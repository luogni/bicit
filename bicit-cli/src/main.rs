use anyhow::Result;
use bicit::{get_template_by_name, Context, Template};
use bicit_cli::export_to_file;
use clap::Parser;
use std::fs;

#[derive(Parser, Debug)]
#[command(version = "0.1", author = "Luca Ognibene <luca.ognibene@gmail.com>")]
struct Opts {
    /// Template name (embedded) or path to SVG file
    #[arg(short, long, default_value = "dev")]
    template: String,
    /// Path to GPX data file
    #[arg(short, long)]
    datafile: String,
    /// Output basename (without extension)
    #[arg(short, long, default_value = "out")]
    outfile: String,
}

fn main() -> Result<()> {
    let opts: Opts = Opts::parse();

    // Try to find embedded template first, then fall back to file path
    let template = if let Some(embedded) = get_template_by_name(&opts.template) {
        println!(
            "Using embedded template '{}' for {} -> {}",
            opts.template, opts.datafile, opts.outfile
        );
        Template::new(embedded.content)
    } else {
        // Assume it's a file path
        println!(
            "Using template file '{}' for {} -> {}",
            opts.template, opts.datafile, opts.outfile
        );
        let content = fs::read_to_string(&opts.template)?;
        Template::new(content)
    };

    let mut ctx = Context::new(&opts.datafile);
    ctx.load()?;

    export_to_file(&template, &ctx, &opts.outfile)
}
