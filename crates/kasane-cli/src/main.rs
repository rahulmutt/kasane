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
    /// Run OCR on text-less pages (requires a build compiled with `-F ocr`)
    #[arg(long)]
    ocr: bool,
    /// OCR language(s), e.g. "eng" or "eng+deu" (used with --ocr)
    #[arg(long, default_value = "eng")]
    ocr_lang: String,
    /// With --ocr, emit OCR text even at low confidence and never a page image
    #[arg(long)]
    ocr_no_image: bool,
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

/// On a build without the `ocr` feature, reject `--ocr` with a clear, exit-2
/// error (the message contains "unsupported" so `exit_code_for` maps it to 2).
#[cfg(not(feature = "ocr"))]
fn ensure_ocr_available(ocr_requested: bool) -> Result<()> {
    if ocr_requested {
        bail!("OCR is unsupported in this build; rebuild with `-F ocr` (requires Tesseract + Leptonica)");
    }
    Ok(())
}

#[cfg(feature = "ocr")]
fn ensure_ocr_available(_ocr_requested: bool) -> Result<()> {
    Ok(())
}

fn run() -> Result<()> {
    let args = Args::parse();
    let bytes =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let ext = args.input.extension().and_then(|s| s.to_str());
    let fmt = kasane_adapters::detect(&bytes, ext).context("unsupported or unrecognized format")?;
    let adapter = kasane_adapters::adapter_for(fmt).map_err(|e| anyhow::anyhow!("{e}"))?;

    ensure_ocr_available(args.ocr)?;

    #[cfg(feature = "ocr")]
    let extractor = if args.ocr {
        Some(
            kasane_adapters::ocr::TesseractExtractor::new(&args.ocr_lang)
                .map_err(|e| anyhow::anyhow!("{e}"))?,
        )
    } else {
        None
    };

    let ocr_opts = kasane_adapters::ocr::OcrOptions {
        lang: args.ocr_lang.clone(),
        force_text: args.ocr_no_image,
        ..Default::default()
    };

    #[cfg(feature = "ocr")]
    let parse_opts = kasane_adapters::ParseOptions {
        ocr: extractor
            .as_ref()
            .map(|e| e as &dyn kasane_adapters::ocr::TextExtractor),
        ocr_opts,
    };
    #[cfg(not(feature = "ocr"))]
    let parse_opts = kasane_adapters::ParseOptions {
        ocr: None,
        ocr_opts,
    };

    let (doc, assets) = adapter
        .parse_with(&bytes, &args.input.to_string_lossy(), &parse_opts)
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
    use super::*;

    #[test]
    fn encrypted_maps_to_exit_two() {
        assert_eq!(exit_code_for("encrypted content"), 2);
        assert_eq!(exit_code_for("DRM-protected content is not supported"), 2);
        assert_eq!(exit_code_for("malformed input: bad xref"), 1);
    }

    #[cfg(not(feature = "ocr"))]
    #[test]
    fn ocr_flag_rejected_without_feature() {
        let err = ensure_ocr_available(true).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("unsupported"), "msg was: {msg}");
        assert_eq!(exit_code_for(&msg), 2);
    }

    #[cfg(not(feature = "ocr"))]
    #[test]
    fn no_ocr_flag_is_fine_without_feature() {
        assert!(ensure_ocr_available(false).is_ok());
    }
}
