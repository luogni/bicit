use crate::BicitApp;
use egui::{Align, Layout, ScrollArea, Vec2};

const NARROW_BREAKPOINT: f32 = 700.0;

impl eframe::App for BicitApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let screen_width = ctx.input(|i| i.viewport_rect().width());
        let is_narrow = screen_width < NARROW_BREAKPOINT;

        self.poll_map_job(ctx);

        #[cfg(target_arch = "wasm32")]
        {
            self.poll_gpx_pick(ctx);
        }

        // Top panel: header
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Open GPX...").clicked() {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("GPX", &["gpx"])
                            .pick_file()
                            && let Err(e) = self.load_gpx(path)
                        {
                            self.status_message = Some(format!("Error: {e}"));
                        }
                    }

                    #[cfg(target_arch = "wasm32")]
                    {
                        self.start_gpx_pick(ctx.clone());
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.hyperlink_to("GitHub", "https://github.com/luogni/bicit/");
                    ui.heading("Bicit");
                });
            });
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
                    self.export(ctx);
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
