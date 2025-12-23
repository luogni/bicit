use anyhow::Context;
use anyhow::{Result, anyhow};
use bicit::context;
use bicit::map;
use egui::{Align2, Direction, Pos2};
use egui_toast::{Toast, ToastKind, Toasts};
use galileo::MapView;
use galileo::layer::raster_tile_layer::RasterTileLayerBuilder;
use galileo::{Map, MapBuilder};
use galileo_egui::{EguiMap, EguiMapState};
use galileo_types::geo::Crs;
use galileo_types::geo::GeoPoint;
use galileo_types::geo::impls::GeoPoint2d;
use std::path::PathBuf;
use std::time::Duration;

const STORAGE_KEY: &str = "galileo_egui_app_example";

#[derive(serde::Deserialize, serde::Serialize)]
struct AppStorage {
    position: GeoPoint2d,
    resolution: f64,
}

struct EguiMapApp {
    map: EguiMapState,
    position: GeoPoint2d,
    resolution: f64,
}

impl EguiMapApp {
    fn new(egui_map_state: EguiMapState, cc: &eframe::CreationContext<'_>) -> Self {
        // get initial position from map
        let initial_position = egui_map_state
            .map()
            .view()
            .position()
            .expect("invalid map position");
        // get initial resolution from map
        let initial_resolution = egui_map_state.map().view().resolution();

        // Try to get stored values or use initial values
        // let AppStorage {
        // position,
        // resolution,
        // } = cc
        // .storage
        // .and_then(|storage| eframe::get_value(storage, STORAGE_KEY))
        // .unwrap_or(AppStorage {
        // position: initial_position,
        // resolution: initial_resolution,
        // });
        //
        Self {
            map: egui_map_state,
            position: initial_position,
            resolution: initial_resolution,
        }
    }

    fn load_new_layers(&mut self, ctx: &egui::Context, path: PathBuf) -> Result<()> {
        let mut ctx = context::Context::new(path.to_str().ok_or(anyhow!("Can't parse path"))?);
        ctx.load()?;
        let data = ctx.get_data().context("failed getting context")?;

        let layers = map::get_layers(&data.coords, None);

        if self.map.map().layers().len() > 1 {
            self.map.map_mut().layers_mut().remove(1);
            self.map.map_mut().layers_mut().remove(1);
        }
        let extent = layers.outline.extent_projected(&Crs::EPSG3857);

        if let Some(a) = extent {
            let center = a.center();
            // Preserve the current view's size when creating target view
            let current_size = self.map.map().view().size();
            self.map.map_mut().animate_to(
                MapView::new_projected(&center, 7.0).with_size(current_size),
                Duration::new(0, 400_000_000),
            );
        }
        self.map.map_mut().layers_mut().push(layers.outline);
        self.map.map_mut().layers_mut().push(layers.inner);
        Ok(())
    }
}

impl eframe::App for EguiMapApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.resolution < 2.0 {
                self.resolution = 2.0;
            }

            EguiMap::new(&mut self.map)
                .with_position(&mut self.position)
                .with_resolution(&mut self.resolution)
                .show_ui(ui);

            egui::Window::new("Galileo map").show(ctx, |ui| {
                ui.label("Map center position:");
                ui.label(format!(
                    "Lat: {:.4} Lon: {:.4}",
                    self.position.lat(),
                    self.position.lon()
                ));

                ui.separator();
                ui.label("Map resolution:");
                ui.label(format!("{:6}", self.resolution));
                let mut toasts = Toasts::new()
                    .anchor(Align2::LEFT_TOP, Pos2::new(10.0, 10.0))
                    .direction(Direction::TopDown);
                if ui.button("Open file").clicked()
                    && let Some(path) = rfd::FileDialog::new().pick_file()
                {
                    if let Err(e) = self.load_new_layers(ctx, path) {
                        toasts.add(Toast::default().kind(ToastKind::Error).text(e.to_string()));
                    }
                }
                toasts.show(ctx);
            });
        });
    }

    // Called by egui to save state before shutdown.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(
            storage,
            STORAGE_KEY,
            &AppStorage {
                position: self.position,
                resolution: self.resolution,
            },
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let map = create_map();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_fullscreen(false),
        ..Default::default()
    };

    galileo_egui::InitBuilder::new(map)
        .with_app_builder(|egui_map_state, cc| Box::new(EguiMapApp::new(egui_map_state, cc)))
        .with_native_options(options)
        .with_app_name("Bicit-ui")
        .init()
        .expect("failed to initialize");
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn run() {
    let map = create_map();

    galileo_egui::InitBuilder::new(map)
        .with_app_builder(|egui_map_state, cc| Box::new(EguiMapApp::new(egui_map_state, cc)))
        .init()
        .expect("failed to initialize");
}

fn create_map() -> Map {
    let layer = RasterTileLayerBuilder::new_osm()
        .with_file_cache_checked(".tile_cache")
        .build()
        .expect("failed to create layer");

    MapBuilder::default()
        .with_latlon(37.566, 128.9784)
        .with_z_level(8)
        .with_layer(layer)
        .build()
}
