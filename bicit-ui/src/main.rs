use anyhow::{Context as AnyhowContext, Result, anyhow};
use bicit::{Context, EmbeddedTemplate, Template, get_templates, map};
use egui::{Align, ColorImage, Layout, ScrollArea, TextureHandle, TextureOptions, Vec2};
use galileo::layer::raster_tile_layer::RasterTileLayerBuilder;
use galileo::{Map, MapBuilder, MapView};
use galileo_egui::{EguiMap, EguiMapState};
use galileo_types::geo::Crs;
use galileo_types::geo::impls::GeoPoint2d;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const NARROW_BREAKPOINT: f32 = 700.0;

/// Render SVG content to an egui ColorImage, scaling to fit within max_width x max_height
fn render_svg_to_color_image(
    svg_content: &str,
    max_width: u32,
    max_height: u32,
) -> Result<ColorImage> {
    // Load system fonts for text rendering
    let mut fontdb = usvg::fontdb::Database::new();
    fontdb.load_system_fonts();

    // Set fallback font families
    fontdb.set_sans_serif_family("DejaVu Sans");
    fontdb.set_serif_family("DejaVu Serif");
    fontdb.set_monospace_family("DejaVu Sans Mono");

    let options = usvg::Options {
        fontdb: std::sync::Arc::new(fontdb),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(svg_content, &options)
        .map_err(|e| anyhow!("Failed to parse SVG: {}", e))?;

    let original_size = tree.size();
    let scale_x = max_width as f32 / original_size.width();
    let scale_y = max_height as f32 / original_size.height();
    let scale = scale_x.min(scale_y);

    let width = (original_size.width() * scale).round() as u32;
    let height = (original_size.height() * scale).round() as u32;

    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| anyhow!("Failed to create pixmap {}x{}", width, height))?;

    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    Ok(ColorImage::from_rgba_premultiplied(
        [width as usize, height as usize],
        pixmap.data(),
    ))
}

struct BicitApp {
    // Map state
    map: EguiMapState,
    position: GeoPoint2d,
    resolution: f64,

    // Template state
    templates: &'static [EmbeddedTemplate],
    selected_template_idx: usize,

    // GPX state
    gpx_path: Option<PathBuf>,
    gpx_context: Option<Context>,

    // Preview state
    preview_texture: Option<TextureHandle>,
    preview_dirty: bool,

    // Status message
    status_message: Option<String>,
}

impl BicitApp {
    fn new(egui_map_state: EguiMapState, _cc: &eframe::CreationContext<'_>) -> Self {
        let initial_position = egui_map_state
            .map()
            .view()
            .position()
            .expect("invalid map position");
        let initial_resolution = egui_map_state.map().view().resolution();

        Self {
            map: egui_map_state,
            position: initial_position,
            resolution: initial_resolution,
            templates: get_templates(),
            selected_template_idx: 0,
            gpx_path: None,
            gpx_context: None,
            preview_texture: None,
            preview_dirty: true,
            status_message: None,
        }
    }

    fn load_gpx(&mut self, path: PathBuf) -> Result<()> {
        let mut ctx = Context::new(path.to_str().ok_or(anyhow!("Invalid path"))?);
        ctx.load()?;

        // Update map view to show the track
        let data = ctx.get_data().context("Failed to get GPX data")?;
        let layers = map::get_layers(&data.coords, None);

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

        // Reload context (get_data consumes it)
        let mut ctx = Context::new(path.to_str().unwrap());
        ctx.load()?;

        self.gpx_path = Some(path);
        self.gpx_context = Some(ctx);
        self.preview_dirty = true;
        self.status_message = Some("GPX loaded successfully".to_string());

        Ok(())
    }

    fn regenerate_preview(&mut self, ctx: &egui::Context) {
        let template = &self.templates[self.selected_template_idx];
        let bicit_template = Template::new(template.content);

        let svg_content = if let Some(ref gpx_ctx) = self.gpx_context {
            match bicit_template.apply_context(gpx_ctx) {
                Ok(svg) => svg,
                Err(e) => {
                    self.status_message = Some(format!("Preview error: {}", e));
                    return;
                }
            }
        } else {
            // No GPX loaded - show template with placeholder values
            template.content.to_string()
        };

        // Render SVG to texture using resvg via usvg
        match render_svg_to_color_image(&svg_content, 540, 960) {
            Ok(image) => {
                self.preview_texture =
                    Some(ctx.load_texture("preview", image, TextureOptions::LINEAR));
                self.preview_dirty = false;
            }
            Err(e) => {
                self.status_message = Some(format!("SVG render error: {}", e));
            }
        }
    }

    fn export(&mut self) {
        let Some(ref gpx_path) = self.gpx_path else {
            self.status_message = Some("No GPX file loaded".to_string());
            return;
        };

        let Some(ref gpx_ctx) = self.gpx_context else {
            self.status_message = Some("No GPX data available".to_string());
            return;
        };

        // Derive default filename from GPX
        let default_name = gpx_path
            .file_stem()
            .map(|s| format!("{}.png", s.to_string_lossy()))
            .unwrap_or_else(|| "output.png".to_string());

        let Some(out_path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("PNG", &["png"])
            .save_file()
        else {
            return;
        };

        let template = &self.templates[self.selected_template_idx];
        let bicit_template = Template::new(template.content);

        match bicit_template.apply_context(gpx_ctx) {
            Ok(svg) => {
                let outbase = out_path.with_extension("");
                let outsvg = outbase.with_extension("svg");
                let outpng = outbase.with_extension("png");

                if let Err(e) = std::fs::write(&outsvg, &svg) {
                    self.status_message = Some(format!("Failed to write SVG: {}", e));
                    return;
                }

                match Command::new("inkscape")
                    .arg(format!("--export-filename={}", outpng.display()))
                    .arg(&outsvg)
                    .output()
                {
                    Ok(output) => {
                        if output.status.success() {
                            self.status_message =
                                Some(format!("Exported to {}", outpng.display()));
                        } else {
                            self.status_message = Some(format!(
                                "Inkscape error: {}",
                                String::from_utf8_lossy(&output.stderr)
                            ));
                        }
                    }
                    Err(e) => {
                        self.status_message =
                            Some(format!("Failed to run Inkscape: {}", e));
                    }
                }

                gpx_ctx.cleanup_temp_files();
            }
            Err(e) => {
                self.status_message = Some(format!("Template error: {}", e));
            }
        }
    }

    fn show_map(&mut self, ui: &mut egui::Ui) {
        if self.resolution < 2.0 {
            self.resolution = 2.0;
        }

        EguiMap::new(&mut self.map)
            .with_position(&mut self.position)
            .with_resolution(&mut self.resolution)
            .show_ui(ui);
    }

    fn show_preview(&mut self, ui: &mut egui::Ui) {
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
            ui.centered_and_justified(|ui| {
                ui.label("No preview available");
            });
        }
    }
}

impl eframe::App for BicitApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let screen_width = ctx.input(|i| i.viewport_rect().width());
        let is_narrow = screen_width < NARROW_BREAKPOINT;

        // Top panel: header
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open GPX...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("GPX", &["gpx"])
                        .pick_file()
                    {
                        if let Err(e) = self.load_gpx(path) {
                            self.status_message = Some(format!("Error: {}", e));
                        }
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.heading("Bicit");
                });
            });
        });

        // Bottom panel: template selector + export + status
        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.add_space(4.0);

            // Template selector row
            ui.horizontal(|ui| {
                ui.label("Template:");
                ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (i, template) in self.templates.iter().enumerate() {
                            if ui
                                .selectable_label(i == self.selected_template_idx, template.name)
                                .clicked()
                            {
                                self.selected_template_idx = i;
                                self.preview_dirty = true;
                            }
                        }
                    });
                });
            });

            ui.add_space(4.0);

            // Export row
            ui.horizontal(|ui| {
                let export_enabled = self.gpx_context.is_some();
                if ui
                    .add_enabled(export_enabled, egui::Button::new("Export PNG"))
                    .clicked()
                {
                    self.export();
                }

                if let Some(ref msg) = self.status_message {
                    ui.label(msg);
                }
            });

            ui.add_space(4.0);
        });

        // Main content: responsive layout
        egui::CentralPanel::default().show(ctx, |ui| {
            if is_narrow {
                // Stacked layout: map on top (smaller), preview below
                let available_height = ui.available_height();
                let map_height = (available_height * 0.3).max(150.0);

                // Map section
                ui.allocate_ui(Vec2::new(ui.available_width(), map_height), |ui| {
                    ui.group(|ui| {
                        self.show_map(ui);
                    });
                });

                ui.add_space(4.0);

                // Preview section (remaining space)
                ui.group(|ui| {
                    self.show_preview(ui);
                });
            } else {
                // Side-by-side layout: map left (~35%), preview right (~65%)
                ui.columns(2, |cols| {
                    cols[0].group(|ui| {
                        self.show_map(ui);
                    });
                    cols[1].group(|ui| {
                        self.show_preview(ui);
                    });
                });
            }
        });
    }
}

fn create_map() -> Map {
    let layer = RasterTileLayerBuilder::new_osm()
        .with_file_cache_checked(".tile_cache")
        .build()
        .expect("failed to create layer");

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

    galileo_egui::InitBuilder::new(map)
        .with_app_builder(|egui_map_state, cc| Box::new(BicitApp::new(egui_map_state, cc)))
        .init()
        .expect("failed to initialize");
}
