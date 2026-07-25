mod batch;
mod convert;
mod discover;

use anyhow::{bail, Context, Result};
use batch::run_batch;
use clap::Parser;
use convert::{convert_one, ConvertOptions, WorkItem};
use discover::discover;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "kasane",
    about = "Convert documents to progressive-disclosure Markdown"
)]
struct Args {
    /// Input documents and/or directories to convert
    #[arg(required = true, num_args = 1..)]
    inputs: Vec<PathBuf>,
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
    /// Parallel workers for batch mode (default: available parallelism)
    #[arg(short = 'j', long)]
    jobs: Option<std::num::NonZeroUsize>,
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

/// A single positional argument that is not a directory means single-file mode.
/// Keying on the argument rather than on what a walk finds keeps the output
/// shape predictable from the command line alone — and a nonexistent path stays
/// in single-file mode, so it still reports "reading <path>" as it does today.
fn is_dir(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

fn run() -> Result<()> {
    let args = Args::parse();
    ensure_ocr_available(args.ocr)?;
    validate_ocr_lang(args.ocr, &args.ocr_lang)?;

    let opts = ConvertOptions {
        max_tokens: args.max_tokens,
        min_tokens: args.min_tokens,
        force: args.force,
        ocr: args.ocr,
        ocr_lang: args.ocr_lang.clone(),
        ocr_no_image: args.ocr_no_image,
    };

    if args.inputs.len() == 1 && !is_dir(&args.inputs[0]) {
        return run_single(&args.inputs[0], &args, &opts);
    }
    run_many(&args, &opts)
}

fn run_single(input: &Path, args: &Args, opts: &ConvertOptions) -> Result<()> {
    let out = args.out.clone().unwrap_or_else(|| {
        PathBuf::from(input.file_stem().and_then(|s| s.to_str()).unwrap_or("out"))
    });
    if out.as_os_str().is_empty() {
        bail!("could not determine output directory");
    }

    let item = WorkItem {
        rel: input
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        input: input.to_path_buf(),
        out_dir: out.clone(),
    };
    let done = convert_one(&item, opts)?;
    eprintln!("wrote {} files to {}", done.files, out.display());
    Ok(())
}

fn run_many(args: &Args, opts: &ConvertOptions) -> Result<()> {
    let Some(out) = args.out.clone() else {
        bail!("converting more than one document requires an output root: add `-o <DIR>`");
    };

    // Spec §4: the non-empty check applies once to the output root, up front.
    // Per-document directories are created fresh below, and `force` is still
    // passed through to write_tree so a re-run behaves as it does today.
    if !args.force && out.exists() {
        let non_empty = out
            .read_dir()
            .with_context(|| format!("inspect output directory {}", out.display()))?
            .next()
            .is_some();
        if non_empty {
            bail!(
                "output directory {} is not empty (use --force)",
                out.display()
            );
        }
    }

    let items = discover(&args.inputs, &out)?;
    if items.is_empty() {
        bail!("no supported documents found");
    }

    let jobs = args.jobs.map(|n| n.get()).unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });

    let outcomes = run_batch(items, jobs, opts)?;
    let failures = outcomes.iter().filter(|o| o.result.is_err()).count();
    if failures > 0 {
        bail!("{failures} of {} documents failed", outcomes.len());
    }
    eprintln!(
        "converted {} documents to {}",
        outcomes.len(),
        out.display()
    );
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
