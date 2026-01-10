use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder};

use anyhow::{Result, anyhow};

pub fn parse_svg_tree(svg_content: &str) -> Result<usvg::Tree> {
    let mut fontdb = usvg::fontdb::Database::new();
    #[cfg(not(target_arch = "wasm32"))]
    fontdb.load_system_fonts();

    // On wasm we can't query system fonts, so ship a couple.
    #[cfg(target_arch = "wasm32")]
    {
        fontdb.load_font_data(include_bytes!("../assets/fonts/DejaVuSans.ttf").to_vec());
        fontdb.load_font_data(include_bytes!("../assets/fonts/DejaVuSans-Oblique.ttf").to_vec());
        fontdb.load_font_data(include_bytes!("../assets/fonts/DejaVuSansMono.ttf").to_vec());
    }

    fontdb.set_sans_serif_family("DejaVu Sans");
    fontdb.set_serif_family("DejaVu Serif");
    fontdb.set_monospace_family("DejaVu Sans Mono");

    let options = usvg::Options {
        fontdb: std::sync::Arc::new(fontdb),
        ..Default::default()
    };

    usvg::Tree::from_str(svg_content, &options).map_err(|e| anyhow!("Failed to parse SVG: {e}"))
}

pub fn render_svg_to_png_bytes(svg_content: &str, scale: f32) -> Result<Vec<u8>> {
    let tree = parse_svg_tree(svg_content)?;

    let original_size = tree.size();
    let width = (original_size.width() * scale).round().max(1.0) as u32;
    let height = (original_size.height() * scale).round().max(1.0) as u32;

    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| anyhow!("Failed to create pixmap {width}x{height}"))?;

    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(pixmap.data(), width, height, ColorType::Rgba8.into())
        .map_err(|e| anyhow!("Failed to encode PNG: {e}"))?;

    Ok(png)
}
