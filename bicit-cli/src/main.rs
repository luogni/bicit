use anyhow::Result;
use bicit_cli::Template;
use bicit_cli::context::Context;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(version = "0.1", author = "Luca Ognibene <luca.ognibene@gmail.com>")]
struct Opts {
    #[arg(short, long, default_value = "templates/dev.svg")]
    template: String,
    #[arg(short, long)]
    datafile: String,
    #[arg(short, long, default_value = "out")]
    outfile: String,
}

fn main() -> Result<()> {
    let opts: Opts = Opts::parse();
    println!(
        "Converting {} to {} using {}.",
        opts.template, opts.outfile, opts.datafile
    );

    let mut c = Context::new(&opts.datafile);
    c.load()?;
    let t = Template::new(&opts.template);

    t.apply_context_to_file(&c, opts.outfile)
}
