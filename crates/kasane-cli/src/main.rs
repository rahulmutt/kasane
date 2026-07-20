use anyhow::{bail, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "kasane",
    about = "Convert documents to progressive-disclosure Markdown"
)]
struct Args {
    /// Input document (EPUB, PPTX supported in this build)
    input: PathBuf,
    /// Output root directory (default: ./<input-stem>/)
    #[arg(short, long)]
    out: Option<PathBuf>,
    /// Overwrite a non-empty output directory
    #[arg(long)]
    force: bool,
    /// Size-guard split threshold (estimated tokens)
    #[arg(long, default_value_t = 2000)]
    max_tokens: usize,
    /// Size-guard merge threshold (estimated tokens)
    #[arg(long, default_value_t = 200)]
    min_tokens: usize,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            // Distinguish unsupported/DRM for exit code 2.
            let msg = format!("{e:#}");
            if msg.contains("unsupported") || msg.contains("DRM") {
                ExitCode::from(2)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let bytes =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let ext = args.input.extension().and_then(|s| s.to_str());
    let fmt = kasane_adapters::detect(&bytes, ext).context("unsupported or unrecognized format")?;
    let adapter = kasane_adapters::adapter_for(fmt).map_err(|e| anyhow::anyhow!("{e}"))?;
    let (doc, assets) = adapter
        .parse(&bytes, &args.input.to_string_lossy())
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let opts = kasane_core::Options {
        max_tokens: args.max_tokens,
        min_tokens: args.min_tokens,
    };
    let site = kasane_core::structure(doc, &opts);

    let out = args.out.unwrap_or_else(|| {
        PathBuf::from(
            args.input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("out"),
        )
    });
    if out.as_os_str().is_empty() {
        bail!("could not determine output directory");
    }
    kasane_writer::write_tree(&site, &assets, &out, args.force)?;
    eprintln!("wrote {} files to {}", site.files.len(), out.display());
    Ok(())
}
