use anyhow::Result;
use bicit::{Context, Template, get_template_by_name};
use bicit_cli::export_to_file;
use clap::Parser;
use std::fs;
use std::path::Path;

#[derive(Parser, Debug)]
#[command(version = "0.1", author = "Luca Ognibene <luca.ognibene@gmail.com>")]
struct Opts {
    /// Template name (embedded) or path to SVG file
    #[arg(short, long, default_value = "story_split")]
    template: String,
    /// Path to GPX data file
    #[arg(short, long)]
    datafile: String,
    /// Output basename, default value is same name as gpx data file
    #[arg(short, long, default_value = "")]
    outfile: String,
}

fn main() -> Result<()> {
    let opts: Opts = Opts::parse();

    let outfile = if opts.outfile == "" {
        Path::new(&opts.datafile)
            .file_stem()
            .map(|s| format!("{}.png", s.to_string_lossy()))
            .unwrap_or_else(|| "output.png".to_string())
    } else {
        opts.outfile
    };

    // Try to find embedded template first, then fall back to file path
    let template = if let Some(embedded) = get_template_by_name(&opts.template) {
        println!(
            "Using embedded template '{}' for {} -> {}",
            opts.template, opts.datafile, outfile
        );
        Template::new(embedded.content)
    } else {
        // Assume it's a file path
        println!(
            "Using template file '{}' for {} -> {}",
            opts.template, opts.datafile, outfile
        );
        let content = fs::read_to_string(&opts.template)?;
        Template::new(content)
    };

    let mut ctx = Context::new(&opts.datafile);
    ctx.load()?;

    export_to_file(&template, &ctx, &outfile)
}
