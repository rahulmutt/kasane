//! Shared bodies for the fuzz targets in `fuzz/`.
//!
//! **This is a test seam, not public API.** It is `pub` only so the `fuzz/`
//! crate — a separate cargo workspace — can call it, and it lives *inside*
//! this crate so it can reach `pub(crate)` internals such as
//! `math::capture_island` and `mobi::palmdoc::decompress` without widening the
//! real public surface.
//!
//! Every function here has the same shape: `fn(&[u8])`. It takes arbitrary
//! bytes and either returns normally or panics. A panic **is** the finding —
//! these functions never return an error to report one. That uniformity is
//! what lets `tests/fuzz_corpus.rs` dispatch by directory name and keeps every
//! libFuzzer wrapper identical.

use crate::{Adapter, DjvuAdapter, EpubAdapter, MobiAdapter, PdfAdapter, PptxAdapter};
use kasane_ir::AssetBag;

pub fn epub(data: &[u8]) {
    adapter(&EpubAdapter, data, "fuzz.epub");
}

pub fn pptx(data: &[u8]) {
    adapter(&PptxAdapter, data, "fuzz.pptx");
}

/// Covers MOBI and AZW3/KF8 alike — they are one adapter.
pub fn mobi(data: &[u8]) {
    adapter(&MobiAdapter, data, "fuzz.mobi");
}

pub fn pdf(data: &[u8]) {
    adapter(&PdfAdapter, data, "fuzz.pdf");
}

pub fn djvu(data: &[u8]) {
    adapter(&DjvuAdapter, data, "fuzz.djvu");
}

/// A rejected parse is a perfectly good outcome — most fuzzer inputs are not
/// valid documents. Only a *successful* parse has assets worth checking.
fn adapter(a: &dyn Adapter, data: &[u8], source_path: &str) {
    if let Ok((_doc, assets)) = a.parse(data, source_path) {
        assert_assets_contained(&assets);
    }
}

/// AGENTS.md: "No path traversal: sanitize archive entry names and asset
/// filenames, confine writes to `_assets/`." `AssetItem::filename` is what the
/// writer actually creates inside `_assets/`, and `guard::safe_media_filename`
/// is supposed to reduce it to a bare basename. Nothing checked that until now.
fn assert_assets_contained(assets: &AssetBag) {
    for item in &assets.items {
        let f = item.filename.as_str();
        assert!(!f.is_empty(), "empty asset filename");
        assert!(
            !f.contains('/') && !f.contains('\\'),
            "asset filename contains a path separator: {f:?}"
        );
        assert!(
            f != "." && f != "..",
            "asset filename is a directory traversal: {f:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kasane_ir::{AssetBag, AssetItem};

    fn bag(filename: &str) -> AssetBag {
        AssetBag {
            items: vec![AssetItem {
                key: "k".into(),
                filename: filename.into(),
                bytes: vec![],
            }],
        }
    }

    #[test]
    fn assets_containment_accepts_a_sanitized_basename() {
        assert_assets_contained(&bag("001-image.png"));
    }

    #[test]
    #[should_panic(expected = "path separator")]
    fn assets_containment_rejects_a_separator() {
        assert_assets_contained(&bag("../../etc/passwd"));
    }

    #[test]
    #[should_panic(expected = "traversal")]
    fn assets_containment_rejects_dotdot() {
        assert_assets_contained(&bag(".."));
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn every_fixture_survives_its_adapter() {
        let cases: &[(&str, fn(&[u8]))] = &[
            ("epub/minimal.epub", epub as fn(&[u8])),
            ("epub/rich.epub", epub),
            ("pptx/minimal.pptx", pptx),
            ("mobi/minimal.mobi", mobi),
            ("mobi/minimal-drm.mobi", mobi),
            ("azw3/minimal.azw3", mobi),
            ("azw3/lying-skel.azw3", mobi),
            ("pdf/minimal.pdf", pdf),
            ("pdf/no-outline.pdf", pdf),
            ("pdf/image.pdf", pdf),
            ("pdf/scanned.pdf", pdf),
            ("djvu/sample.djvu", djvu),
            ("djvu/scanned.djvu", djvu),
        ];
        for (rel, f) in cases {
            let path = format!("../../tests/fixtures/{rel}");
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            f(&bytes);
        }
    }

    #[test]
    fn truncated_and_empty_inputs_survive() {
        let bytes = std::fs::read("../../tests/fixtures/epub/rich.epub").unwrap();
        for f in [epub as fn(&[u8]), pptx, mobi, pdf, djvu] {
            f(&[]);
            f(&bytes[..bytes.len() / 2]);
            f(&bytes[..1]);
        }
    }
}
