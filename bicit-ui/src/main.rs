pub(crate) mod export;
pub(crate) mod ui;

use anyhow::{Result, anyhow};
use bicit::render::parse_svg_tree;
use bicit::template::{AssetProvider, MapImageRequest, TRANSPARENT_PNG_DATA_URL};
use bicit::{Context, EmbeddedTemplate, Template, get_templates, map};
use eframe::wgpu::{Device as WgpuDevice, Queue as WgpuQueue};
use egui::{ColorImage, TextureHandle, TextureOptions, Vec2};
use galileo::layer::raster_tile_layer::RasterTileLayerBuilder;
use galileo::{Map, MapBuilder, MapView};
use galileo_egui::{EguiMap, EguiMapState};
use galileo_types::cartesian::Size as CartesianSize;
use galileo_types::geo::Crs;
use galileo_types::geo::impls::GeoPoint2d;
use std::path::PathBuf;
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;

#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;

/// Render SVG content to an egui ColorImage, scaling to fit within max_width x max_height
fn render_svg_to_color_image(
    svg_content: &str,
    max_width: u32,
    max_height: u32,
) -> Result<ColorImage> {
    let tree = parse_svg_tree(svg_content)?;

    let original_size = tree.size();
    let scale_x = max_width as f32 / original_size.width();
    let scale_y = max_height as f32 / original_size.height();
    let scale = scale_x.min(scale_y);

    let width = (original_size.width() * scale).round().max(1.0) as u32;
    let height = (original_size.height() * scale).round().max(1.0) as u32;

    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| anyhow!("Failed to create pixmap {width}x{height}"))?;

    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    Ok(ColorImage::from_rgba_premultiplied(
        [width as usize, height as usize],
        pixmap.data(),
    ))
}

#[derive(Debug)]
enum MapJobKind {
    Preview,

    #[cfg(target_arch = "wasm32")]
    ExportWasm {
        filename: String,
        template_svg: &'static str,
    },
}

struct MapJobInFlight {
    kind: MapJobKind,
    request: MapImageRequest,

    #[cfg(target_arch = "wasm32")]
    result: Rc<RefCell<Option<anyhow::Result<String>>>>,

    #[cfg(not(target_arch = "wasm32"))]
    rx: mpsc::Receiver<anyhow::Result<String>>,
}

struct ImageMapAssetProvider {
    map_href: Option<String>,
}

impl AssetProvider for ImageMapAssetProvider {
    fn get_image(
        &self,
        id: &str,
        _w_px: u32,
        _h_px: u32,
        _track_color: Option<galileo::Color>,
    ) -> Option<String> {
        if id != "image_map" {
            return None;
        }

        Some(
            self.map_href
                .clone()
                .unwrap_or_else(|| TRANSPARENT_PNG_DATA_URL.to_string()),
        )
    }
}

#[cfg(target_arch = "wasm32")]
struct GpxPickInFlight {
    result: Rc<RefCell<Option<anyhow::Result<(String, Vec<u8>)>>>>,
}

struct BicitApp {
    // Map state
    map: EguiMapState,
    wgpu_device: WgpuDevice,
    wgpu_queue: WgpuQueue,
    position: GeoPoint2d,
    resolution: f64,

    // Template state
    templates: &'static [EmbeddedTemplate],
    selected_template_idx: usize,

    // GPX state
    gpx_path: Option<PathBuf>,
    gpx_context: Option<Context>,
    #[cfg(target_arch = "wasm32")]
    gpx_pick: Option<GpxPickInFlight>,
    // One shared map render job at a time
    map_job: Option<MapJobInFlight>,

    // Cached rendered map href for preview
    preview_map: Option<(MapImageRequest, String)>,

    // Preview state
    preview_texture: Option<TextureHandle>,
    preview_dirty: bool,

    // Status message
    status_message: Option<String>,
}

impl BicitApp {
    fn new(egui_map_state: EguiMapState, cc: &eframe::CreationContext<'_>) -> Self {
        let render_state = cc
            .wgpu_render_state
            .clone()
            .expect("eframe wgpu render state unavailable");
        let wgpu_device = render_state.device.clone();
        let wgpu_queue = render_state.queue.clone();

        let initial_position = egui_map_state
            .map()
            .view()
            .position()
            .expect("invalid map position");
        let initial_resolution = egui_map_state.map().view().resolution();

        Self {
            map: egui_map_state,
            wgpu_device,
            wgpu_queue,
            position: initial_position,
            resolution: initial_resolution,
            templates: get_templates(),
            selected_template_idx: 0,
            gpx_path: None,
            gpx_context: None,
            #[cfg(target_arch = "wasm32")]
            gpx_pick: None,
            map_job: None,
            preview_map: None,
            preview_texture: None,
            preview_dirty: true,
            status_message: None,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn load_gpx(&mut self, path: PathBuf) -> Result<()> {
        let filename = path.to_str().ok_or(anyhow!("Invalid path"))?;
        let mut ctx = Context::new(filename);
        ctx.load()?;
        let res = self.load_from_context(ctx);
        self.gpx_path = Some(path);
        res
    }

    fn load_from_context(&mut self, ctx: Context) -> Result<()> {
        let coords = ctx
            .coords()
            .ok_or_else(|| anyhow!("Failed to get GPX coordinates"))?;
        let layers = map::get_layers(coords, None);

        // Remove old track layers if any
        while self.map.map().layers().len() > 1 {
            self.map.map_mut().layers_mut().remove(1);
        }

        // Center map on track
        if let Some(extent) = layers.outline.extent_projected(&Crs::EPSG3857) {
            let center = extent.center();
            let current_size = self.map.map().view().size();
            self.map.map_mut().animate_to(
                MapView::new_projected(&center, 7.0).with_size(current_size),
                Duration::from_millis(400),
            );
        }

        // Add track layers
        self.map.map_mut().layers_mut().push(layers.outline);
        self.map.map_mut().layers_mut().push(layers.inner);

        self.gpx_context = Some(ctx);
        self.map_job = None;
        self.preview_map = None;
        self.preview_texture = None;
        self.preview_dirty = true;
        self.status_message = Some("GPX loaded successfully".to_string());

        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    fn load_gpx_from_bytes(&mut self, filename: String, bytes: Vec<u8>) -> Result<()> {
        let mut ctx = Context::new(&filename);
        ctx.load_from_bytes(&bytes)?;

        self.load_from_context(ctx)
    }

    #[cfg(target_arch = "wasm32")]
    fn start_gpx_pick(&mut self, egui_ctx: egui::Context) {
        if self.gpx_pick.is_some() {
            return;
        }

        let cell: Rc<RefCell<Option<anyhow::Result<(String, Vec<u8>)>>>> =
            Rc::new(RefCell::new(None));
        let cell2 = cell.clone();
        spawn_local(async move {
            let picked = rfd::AsyncFileDialog::new()
                .add_filter("gpx", &["gpx"])
                .pick_file()
                .await;
            let result = match picked {
                Some(handle) => {
                    let name = handle.file_name();
                    let bytes = handle.read().await;
                    Ok((name, bytes))
                }
                None => Err(anyhow!("No file selected")),
            };

            *cell2.borrow_mut() = Some(result);
            egui_ctx.request_repaint();
        });

        self.gpx_pick = Some(GpxPickInFlight { result: cell });
    }

    #[cfg(target_arch = "wasm32")]
    fn poll_gpx_pick(&mut self, egui_ctx: &egui::Context) {
        let Some(pick) = self.gpx_pick.take() else {
            return;
        };

        let completed = pick.result.borrow_mut().take();
        let Some(result) = completed else {
            self.gpx_pick = Some(pick);
            return;
        };

        match result {
            Ok((name, bytes)) => {
                if let Err(e) = self.load_gpx_from_bytes(name, bytes) {
                    self.status_message = Some(format!("Error: {e}"));
                }
            }
            Err(e) => {
                self.status_message = Some(format!("Error: {e}"));
            }
        }

        egui_ctx.request_repaint();
    }

    fn start_map_job(
        &mut self,
        kind: MapJobKind,
        request: MapImageRequest,
        coords: Vec<geo_types::Point<f64>>,
        egui_ctx: egui::Context,
    ) {
        if self.map_job.is_some() {
            return;
        }

        #[cfg(target_arch = "wasm32")]
        {
            let device = self.wgpu_device.clone();
            let queue = self.wgpu_queue.clone();

            let result = Rc::new(RefCell::new(None));
            let result_cell = result.clone();
            let egui_ctx = egui_ctx.clone();
            spawn_local(async move {
                let res = bicit::map::render_track_map_href_with_wgpu_async(
                    device,
                    queue,
                    &coords,
                    CartesianSize::<u32>::new(request.w_px, request.h_px),
                    request.track_color,
                )
                .await;
                *result_cell.borrow_mut() = Some(res);
                egui_ctx.request_repaint();
            });

            self.map_job = Some(MapJobInFlight {
                kind,
                request,
                result,
            });
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = egui_ctx;

            let device = self.wgpu_device.clone();
            let queue = self.wgpu_queue.clone();

            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let res = bicit::map::render_track_map_href_with_wgpu(
                    device,
                    queue,
                    &coords,
                    CartesianSize::<u32>::new(request.w_px, request.h_px),
                    request.track_color,
                );
                let _ = tx.send(res);
            });

            self.map_job = Some(MapJobInFlight { kind, request, rx });
        }
    }

    fn poll_map_job(&mut self, egui_ctx: &egui::Context) {
        let Some(in_flight) = self.map_job.take() else {
            return;
        };

        #[cfg(target_arch = "wasm32")]
        let completed = in_flight.result.borrow_mut().take();

        #[cfg(not(target_arch = "wasm32"))]
        let completed = match in_flight.rx.try_recv() {
            Ok(res) => Some(res),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(anyhow!("map render worker disconnected")))
            }
        };

        let Some(result) = completed else {
            self.map_job = Some(in_flight);
            return;
        };

        match result {
            Ok(href) => match in_flight.kind {
                MapJobKind::Preview => {
                    self.preview_map = Some((in_flight.request, href));
                    self.preview_dirty = true;
                    egui_ctx.request_repaint();
                }

                #[cfg(target_arch = "wasm32")]
                MapJobKind::ExportWasm {
                    filename,
                    template_svg,
                } => {
                    self.export_wasm_with_map_href(&filename, template_svg, Some(href), egui_ctx);
                }
            },
            Err(e) => {
                self.status_message = Some(format!("Map render error: {e}"));
                egui_ctx.request_repaint();
            }
        }
    }

    fn regenerate_preview(&mut self, ctx: &egui::Context) {
        let Some(gpx_ctx) = self.gpx_context.as_ref() else {
            self.preview_texture = None;
            self.preview_dirty = false;
            return;
        };

        let template = &self.templates[self.selected_template_idx];
        let bicit_template = Template::new(template.content);

        let request = bicit_template.desired_map_image_request();
        let map_href = match request {
            None => None,
            Some(req) => {
                if let Some((cached_req, href)) = &self.preview_map {
                    if *cached_req == req {
                        Some(href.clone())
                    } else {
                        if self.map_job.is_none() {
                            let Some(coords_slice) = gpx_ctx.coords() else {
                                self.status_message = Some("No GPX coordinates".to_string());
                                self.preview_texture = None;
                                self.preview_dirty = false;
                                return;
                            };

                            let coords = coords_slice.to_vec();
                            if coords.is_empty() {
                                self.status_message = Some("No GPX coordinates".to_string());
                                self.preview_texture = None;
                                self.preview_dirty = false;
                                return;
                            }

                            self.start_map_job(MapJobKind::Preview, req, coords, ctx.clone());
                        }

                        // Template wants a map but we don't have it yet.
                        self.preview_texture = None;
                        self.preview_dirty = false;
                        return;
                    }
                } else {
                    if self.map_job.is_none() {
                        let Some(coords_slice) = gpx_ctx.coords() else {
                            self.status_message = Some("No GPX coordinates".to_string());
                            self.preview_texture = None;
                            self.preview_dirty = false;
                            return;
                        };

                        let coords = coords_slice.to_vec();
                        if coords.is_empty() {
                            self.status_message = Some("No GPX coordinates".to_string());
                            self.preview_texture = None;
                            self.preview_dirty = false;
                            return;
                        }

                        self.start_map_job(MapJobKind::Preview, req, coords, ctx.clone());
                    }

                    self.preview_texture = None;
                    self.preview_dirty = false;
                    return;
                }
            }
        };

        let assets = ImageMapAssetProvider { map_href };
        let svg_content = match bicit_template.apply_with(gpx_ctx, &assets) {
            Ok(svg) => svg,
            Err(e) => {
                self.status_message = Some(format!("Preview error: {e}"));
                return;
            }
        };

        match render_svg_to_color_image(&svg_content, 540, 960) {
            Ok(image) => {
                self.preview_texture =
                    Some(ctx.load_texture("preview", image, TextureOptions::LINEAR));
                self.preview_dirty = false;
            }
            Err(e) => {
                self.status_message = Some(format!("SVG render error: {e}"));
            }
        }
    }

    fn show_map(&mut self, ui: &mut egui::Ui) {
        EguiMap::new(&mut self.map).show_ui(ui);

        let view = self.map.map().view();
        self.resolution = view.resolution();
        if let Some(pos) = view.position() {
            self.position = pos;
        }
    }

    fn show_preview(&mut self, ui: &mut egui::Ui) {
        self.poll_map_job(ui.ctx());

        if self.preview_dirty {
            self.regenerate_preview(ui.ctx());
        }

        if let Some(ref texture) = self.preview_texture {
            let available = ui.available_size();
            // Fit 1080x1920 (9:16 aspect) preview into available space
            let aspect = 1080.0 / 1920.0;
            let size = if available.x / available.y > aspect {
                Vec2::new(available.y * aspect, available.y)
            } else {
                Vec2::new(available.x, available.x / aspect)
            };

            ui.centered_and_justified(|ui| {
                ui.image((texture.id(), size));
            });
        } else {
            let msg = if self.gpx_context.is_none() {
                "Load a GPX to preview"
            } else if self.map_job.is_some() {
                "Rendering map..."
            } else {
                "No preview available"
            };

            ui.centered_and_justified(|ui| {
                ui.label(msg);
            });
        }
    }
}

fn create_map() -> Map {
    #[cfg(target_arch = "wasm32")]
    let layer_builder = RasterTileLayerBuilder::new_osm();

    #[cfg(not(target_arch = "wasm32"))]
    let layer_builder = RasterTileLayerBuilder::new_osm().with_file_cache_checked(".tile_cache");

    let layer = layer_builder.build().expect("failed to create layer");

    MapBuilder::default()
        .with_latlon(45.0, 10.0) // Default to Italy
        .with_z_level(6)
        .with_layer(layer)
        .build()
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let map = create_map();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([400.0, 600.0]),
        ..Default::default()
    };

    galileo_egui::InitBuilder::new(map)
        .with_app_builder(|egui_map_state, cc| Box::new(BicitApp::new(egui_map_state, cc)))
        .with_native_options(options)
        .with_app_name("Bicit")
        .init()
        .expect("failed to initialize");
}

#[cfg(target_arch = "wasm32")]
pub fn run() {
    let map = create_map();

    // Firefox often doesn't have WebGPU enabled.
    // Force wgpu to use the WebGL backend for compatibility.
    let mut web_options = eframe::WebOptions::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create_new) =
        &mut web_options.wgpu_options.wgpu_setup
    {
        create_new.instance_descriptor.backends = eframe::wgpu::Backends::GL;
    }

    galileo_egui::InitBuilder::new(map)
        .with_web_options(web_options)
        .with_app_builder(|egui_map_state, cc| Box::new(BicitApp::new(egui_map_state, cc)))
        .init()
        .expect("failed to initialize");
}

/// Entry-point used by Trunk/wasm-bindgen.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn wasm_start() {
    console_error_panic_hook::set_once();
    run();
}

// Allow `cargo build --target wasm32-unknown-unknown` for this package.
#[cfg(target_arch = "wasm32")]
fn main() {}
