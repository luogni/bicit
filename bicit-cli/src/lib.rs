use anyhow::Result;
use bicit::render::render_svg_to_png_bytes;
use bicit::{Context, Template};
use std::fs;
use std::path::PathBuf;

/// Export a GPX file to PNG using a template.
pub fn export_to_file(template: &Template, context: &Context, outfile: &str) -> Result<()> {
    let svg = template.apply_context(context)?;

    let outfile = PathBuf::from(outfile);
    let outbase = if outfile.extension().is_some() {
        outfile.with_extension("")
    } else {
        outfile
    };

    let outpng = outbase.with_extension("png");

    let data = render_svg_to_png_bytes(&svg, 1.0)?;
    fs::write(outpng, data)?;

    context.cleanup_temp_files();

    Ok(())
}
