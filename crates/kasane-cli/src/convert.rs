use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// One document to convert: where it is read from and where its tree goes.
#[derive(Debug)]
pub struct WorkItem {
    /// File to read.
    pub input: PathBuf,
    /// Output root for this document's own tree.
    pub out_dir: PathBuf,
    /// `input` relative to the root it was discovered under, extension kept.
    /// Shown in the run summary and in the library index's failure list.
    pub rel: String,
}

impl WorkItem {
    /// `rel` with its extension dropped: the document's directory beneath the
    /// output root, and the link target used in the library index.
    // Not yet called outside tests: the library index (Task 5) is what uses it.
    #[allow(dead_code)]
    pub fn rel_dir(&self) -> String {
        Path::new(&self.rel)
            .with_extension("")
            .to_string_lossy()
            .into_owned()
    }
}

/// Per-run conversion settings, identical for every document. Plain data, so it
/// is `Send + Sync` and can be shared across workers by reference.
pub struct ConvertOptions {
    pub max_tokens: usize,
    pub min_tokens: usize,
    pub force: bool,
    /// Only read on `-F ocr` builds; a non-ocr build rejects `--ocr` in `main`.
    #[cfg_attr(not(feature = "ocr"), allow(dead_code))]
    pub ocr: bool,
    pub ocr_lang: String,
    pub ocr_no_image: bool,
}

/// What a successful conversion produced, for the summary and library index.
#[derive(Debug)]
pub struct Converted {
    // `title`/`format` are not yet read outside tests: the run summary
    // (Task 4) and the library index (Task 5) are what consume them.
    #[allow(dead_code)]
    pub title: String,
    #[allow(dead_code)]
    pub format: String,
    pub files: usize,
}

/// Convert exactly one document. Returns `Err` rather than exiting, which is
/// what lets a batch run isolate one file's failure from the rest.
pub fn convert_one(item: &WorkItem, opts: &ConvertOptions) -> Result<Converted> {
    let bytes =
        std::fs::read(&item.input).with_context(|| format!("reading {}", item.input.display()))?;
    let ext = item.input.extension().and_then(|s| s.to_str());
    let fmt = kasane_adapters::detect(&bytes, ext).context("unsupported or unrecognized format")?;
    let adapter = kasane_adapters::adapter_for(fmt).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Each call builds its own extractor, so nothing non-`Send` is shared when
    // this runs on a worker thread. `main` has already validated the language.
    #[cfg(feature = "ocr")]
    let extractor = if opts.ocr {
        Some(
            kasane_adapters::ocr::TesseractExtractor::new(&opts.ocr_lang)
                .map_err(|e| anyhow::anyhow!("{e}"))?,
        )
    } else {
        None
    };

    let ocr_opts = kasane_adapters::ocr::OcrOptions {
        lang: opts.ocr_lang.clone(),
        force_text: opts.ocr_no_image,
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
        .parse_with(&bytes, &item.input.to_string_lossy(), &parse_opts)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // `structure` consumes `doc`, so capture the metadata first.
    let title = doc.meta.title.clone();
    let format = doc.meta.source_format.clone();

    let core_opts = kasane_core::Options {
        max_tokens: opts.max_tokens,
        min_tokens: opts.min_tokens,
    };
    let site = kasane_core::structure(doc, &core_opts);
    let files = site.files.len();

    kasane_writer::write_tree(&site, &assets, &item.out_dir, opts.force)?;

    Ok(Converted {
        title,
        format,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(rel)
    }

    fn opts() -> ConvertOptions {
        ConvertOptions {
            max_tokens: 2000,
            min_tokens: 200,
            force: false,
            ocr: false,
            ocr_lang: "eng".into(),
            ocr_no_image: false,
        }
    }

    #[test]
    fn converts_one_document_and_reports_its_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let out_dir = dir.path().join("book");
        let item = WorkItem {
            input: fixture("epub/minimal.epub"),
            out_dir: out_dir.clone(),
            rel: "minimal.epub".into(),
        };

        let done = convert_one(&item, &opts()).unwrap();

        assert_eq!(done.title, "Minimal Book");
        assert_eq!(done.format, "epub");
        assert!(done.files > 0, "expected at least one emitted file");
        assert!(out_dir.join("index.md").exists());
    }

    #[test]
    fn a_drm_document_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let item = WorkItem {
            input: fixture("mobi/minimal-drm.mobi"),
            out_dir: dir.path().join("out"),
            rel: "minimal-drm.mobi".into(),
        };

        let err = convert_one(&item, &opts()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("DRM"), "expected a DRM error, got: {msg}");
    }

    #[test]
    fn rel_dir_drops_the_extension() {
        let item = WorkItem {
            input: "x/a/ch.epub".into(),
            out_dir: "out/a/ch".into(),
            rel: "a/ch.epub".into(),
        };
        assert_eq!(item.rel_dir(), "a/ch");
    }
}
