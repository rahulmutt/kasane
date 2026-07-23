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
    /// Input document (EPUB, PPTX, MOBI, AZW3, PDF, DjVu supported in this build)
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

/// Map an error message to an exit code: 2 for unsupported/DRM/encrypted, else 1.
fn exit_code_for(msg: &str) -> u8 {
    if msg.contains("unsupported") || msg.contains("DRM") || msg.contains("encrypted") {
        2
    } else {
        1
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(exit_code_for(&format!("{e:#}")))
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

#[cfg(test)]
mod tests {
    use super::exit_code_for;

    #[test]
    fn encrypted_maps_to_exit_two() {
        assert_eq!(exit_code_for("encrypted content"), 2);
        assert_eq!(exit_code_for("DRM-protected content is not supported"), 2);
        assert_eq!(exit_code_for("malformed input: bad xref"), 1);
    }
}
