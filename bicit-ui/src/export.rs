use crate::BicitApp;
#[cfg(not(target_arch = "wasm32"))]
use crate::CartesianSize;
use crate::ImageMapAssetProvider;
use crate::Template;
use bicit::render::render_svg_to_png_bytes;

#[cfg(target_arch = "wasm32")]
use crate::MapJobKind;
#[cfg(target_arch = "wasm32")]
use anyhow::{Result, anyhow};
#[cfg(target_arch = "wasm32")]
use js_sys::Uint8Array;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, Url};

#[cfg(target_arch = "wasm32")]
fn download_bytes_as_file(filename: &str, bytes: &[u8], mime: &str) -> Result<()> {
    let array = Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());

    let bag = {
        let bag = BlobPropertyBag::new();
        bag.set_type(mime);
        bag
    };

    let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &bag)
        .map_err(|_| anyhow!("Failed to create download blob"))?;
    let url = Url::create_object_url_with_blob(&blob)
        .map_err(|_| anyhow!("Failed to create download URL"))?;

    let document = web_sys::window()
        .ok_or_else(|| anyhow!("No window"))?
        .document()
        .ok_or_else(|| anyhow!("No document"))?;

    let a = document
        .create_element("a")
        .map_err(|_| anyhow!("Failed to create <a> element"))?
        .dyn_into::<HtmlAnchorElement>()
        .map_err(|_| anyhow!("Failed to cast <a> element"))?;

    a.set_href(&url);
    a.set_download(filename);
    a.click();
    let _ = Url::revoke_object_url(&url);

    Ok(())
}

impl BicitApp {
    #[cfg(target_arch = "wasm32")]
    pub fn export_wasm_with_map_href(
        &mut self,
        filename: &str,
        template_svg: &'static str,
        map_href: Option<String>,
        egui_ctx: &egui::Context,
    ) {
        let Some(ref gpx_ctx) = self.gpx_context else {
            self.status_message = Some("No GPX data available".to_string());
            return;
        };

        let bicit_template = Template::new(template_svg);
        let assets = ImageMapAssetProvider { map_href };

        let svg = match bicit_template.apply_with(gpx_ctx, &assets) {
            Ok(svg) => svg,
            Err(e) => {
                self.status_message = Some(format!("Template error: {e}"));
                return;
            }
        };

        let png = match render_svg_to_png_bytes(&svg, 1.0) {
            Ok(png) => png,
            Err(e) => {
                self.status_message = Some(format!("SVG render error: {e}"));
                return;
            }
        };

        if let Err(e) = download_bytes_as_file(filename, &png, "image/png") {
            self.status_message = Some(format!("Export failed: {e}"));
        } else {
            self.status_message = Some(format!("Downloaded {filename}"));
        }

        gpx_ctx.cleanup_temp_files();
        egui_ctx.request_repaint();
    }

    #[cfg(target_arch = "wasm32")]
    pub fn start_export_wasm(&mut self, egui_ctx: egui::Context) {
        if self.map_job.is_some() {
            self.status_message = Some("Busy (map rendering in progress)".to_string());
            return;
        }

        let Some(ref gpx_ctx) = self.gpx_context else {
            self.status_message = Some("No GPX data available".to_string());
            return;
        };

        let filename = self
            .gpx_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| format!("{}.png", s.to_string_lossy()))
            .unwrap_or_else(|| "output.png".to_string());

        let template = &self.templates[self.selected_template_idx];
        let bicit_template = Template::new(template.content);
        let request = bicit_template.desired_map_image_request();

        // If the template doesn't ask for a map image, export immediately.
        if request.is_none() {
            self.export_wasm_with_map_href(&filename, template.content, None, &egui_ctx);
            return;
        }

        let req = request.expect("request checked");

        if let Some((cached_req, href)) = &self.preview_map {
            if *cached_req == req {
                self.export_wasm_with_map_href(
                    &filename,
                    template.content,
                    Some(href.clone()),
                    &egui_ctx,
                );
                return;
            }
        }

        let Some(coords_slice) = gpx_ctx.coords() else {
            self.status_message = Some("No GPX coordinates".to_string());
            return;
        };

        let coords = coords_slice.to_vec();
        if coords.is_empty() {
            self.status_message = Some("No GPX coordinates".to_string());
            return;
        }

        self.start_map_job(
            MapJobKind::ExportWasm {
                filename,
                template_svg: template.content,
            },
            req,
            coords,
            egui_ctx.clone(),
        );
        self.status_message = Some("Exporting...".to_string());
        egui_ctx.request_repaint();
    }

    pub fn export(&mut self, egui_ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = egui_ctx;

            let Some(ref gpx_path) = self.gpx_path else {
                self.status_message = Some("No GPX file loaded".to_string());
                return;
            };

            let Some(ref gpx_ctx) = self.gpx_context else {
                self.status_message = Some("No GPX data available".to_string());
                return;
            };

            let coords = match gpx_ctx.coords() {
                Some(c) => c,
                None => {
                    self.status_message = Some("No GPX coordinates".to_string());
                    return;
                }
            };

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
            let request = bicit_template.desired_map_image_request();

            let map_href = match request {
                Some(req) => match bicit::map::render_track_map_href_with_wgpu(
                    self.wgpu_device.clone(),
                    self.wgpu_queue.clone(),
                    coords,
                    CartesianSize::<u32>::new(req.w_px, req.h_px),
                    req.track_color,
                ) {
                    Ok(href) => Some(href),
                    Err(e) => {
                        self.status_message = Some(format!("Map render error: {e}"));
                        return;
                    }
                },
                None => None,
            };

            let assets = ImageMapAssetProvider { map_href };

            let svg = match bicit_template.apply_with(gpx_ctx, &assets) {
                Ok(svg) => svg,
                Err(e) => {
                    self.status_message = Some(format!("Template error: {e}"));
                    return;
                }
            };

            let png = match render_svg_to_png_bytes(&svg, 1.0) {
                Ok(png) => png,
                Err(e) => {
                    self.status_message = Some(format!("SVG render error: {e}"));
                    return;
                }
            };

            if let Err(e) = std::fs::write(&out_path, png) {
                self.status_message = Some(format!("Failed to write PNG: {e}"));
                return;
            }

            self.status_message = Some(format!("Exported to {}", out_path.display()));
            gpx_ctx.cleanup_temp_files();
        }

        #[cfg(target_arch = "wasm32")]
        {
            self.start_export_wasm(egui_ctx.clone());
        }
    }
}
