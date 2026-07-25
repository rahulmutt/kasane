mod convert;
mod discover;

use anyhow::{bail, Result};
use clap::Parser;
use convert::{convert_one, ConvertOptions, WorkItem};
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

/// Construct and drop one extractor up front so a bad `--ocr-lang` fails before
/// any document is converted, instead of failing once inside every worker.
#[cfg(feature = "ocr")]
fn validate_ocr_lang(ocr_requested: bool, lang: &str) -> Result<()> {
    if ocr_requested {
        kasane_adapters::ocr::TesseractExtractor::new(lang).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    Ok(())
}

#[cfg(not(feature = "ocr"))]
fn validate_ocr_lang(_ocr_requested: bool, _lang: &str) -> Result<()> {
    Ok(())
}

fn run() -> Result<()> {
    let args = Args::parse();
    ensure_ocr_available(args.ocr)?;
    validate_ocr_lang(args.ocr, &args.ocr_lang)?;

    let out = args.out.clone().unwrap_or_else(|| {
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

    let item = WorkItem {
        rel: args
            .input
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        input: args.input.clone(),
        out_dir: out.clone(),
    };
    let opts = ConvertOptions {
        max_tokens: args.max_tokens,
        min_tokens: args.min_tokens,
        force: args.force,
        ocr: args.ocr,
        ocr_lang: args.ocr_lang.clone(),
        ocr_no_image: args.ocr_no_image,
    };

    let done = convert_one(&item, &opts)?;
    eprintln!("wrote {} files to {}", done.files, out.display());
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
