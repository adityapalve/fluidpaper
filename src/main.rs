use anyhow::Result;
use clap::{Parser, ValueEnum};
use std::path::PathBuf;

use fluidpaper::{parse_pdf, render_text};

#[derive(Parser)]
#[command(name = "fluidpaper", version, about = "Extract semantic reading models from PDFs")]
struct Args {
    pdf: PathBuf,
    #[arg(long, default_value = "semantic-json")]
    format: OutputFormat,
    #[arg(long)]
    max_pages: Option<usize>,
    #[arg(long)]
    pdfium_path: Option<PathBuf>,
}

#[derive(Clone, ValueEnum)]
enum OutputFormat {
    #[value(name = "semantic-json")]
    SemanticJson,
    Text,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let document = parse_pdf(&args.pdf, args.pdfium_path.as_deref(), args.max_pages)?;

    match args.format {
        OutputFormat::SemanticJson => println!("{}", serde_json::to_string_pretty(&document)?),
        OutputFormat::Text => println!("{}", render_text(&document)),
    }

    Ok(())
}
