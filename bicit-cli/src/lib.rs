use anyhow::Result;
use bicit::{Context, Template};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Export a GPX file to SVG and PNG using a template.
///
/// This function:
/// 1. Applies the template with GPX context data to generate an SVG
/// 2. Writes the SVG to `{outfile}.svg`
/// 3. Calls Inkscape to convert SVG to PNG at `{outfile}.png`
///
/// # Arguments
/// * `template` - The SVG template to use
/// * `context` - The loaded GPX context with track data
/// * `outfile` - Output basename (without extension)
pub fn export_to_file(template: &Template, context: &Context, outfile: &str) -> Result<()> {
    let svg = template.apply_context(context)?;

    let outfile = PathBuf::from(outfile);
    let outbase = if outfile.extension().is_some() {
        outfile.with_extension("")
    } else {
        outfile
    };

    let outpng = outbase.with_extension("png");
    let outsvg = outbase.with_extension("svg");
    fs::write(&outsvg, svg)?;

    let inkscape_result = Command::new("inkscape")
        .arg(format!("--export-filename={}", outpng.display()))
        .arg(&outsvg)
        .output();

    context.cleanup_temp_files();
    inkscape_result?;

    Ok(())
}
