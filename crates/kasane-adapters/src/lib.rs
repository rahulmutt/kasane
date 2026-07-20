mod detect;
mod epub;
mod guard;
mod pptx;
mod ziputil;

pub use detect::{detect, Format};
pub use epub::EpubAdapter;
pub use pptx::PptxAdapter;

use kasane_ir::{AssetBag, Document};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("unsupported format")]
    Unsupported,
    #[error("DRM-protected content is not supported")]
    Drm,
    #[error("encrypted content")]
    Encrypted,
    #[error("malformed input: {0}")]
    Malformed(String),
    #[error("input rejected: decompression bomb")]
    Bomb,
}

pub trait Adapter {
    fn parse(&self, bytes: &[u8], source_path: &str) -> Result<(Document, AssetBag), ParseError>;
}

pub fn adapter_for(fmt: Format) -> Result<Box<dyn Adapter>, ParseError> {
    match fmt {
        Format::Epub => Ok(Box::new(EpubAdapter)),
        Format::Pptx => Ok(Box::new(PptxAdapter)),
        _ => Err(ParseError::Unsupported), // other formats land in Plan 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_minimal_epub_to_ir() {
        let bytes = std::fs::read("../../tests/fixtures/epub/minimal.epub").unwrap();
        let (doc, _assets) = EpubAdapter.parse(&bytes, "minimal.epub").unwrap();
        assert_eq!(doc.meta.title, "Minimal Book");
        // headings present in order
        let heads: Vec<_> = doc
            .nodes
            .iter()
            .filter_map(|n| match &n.block {
                kasane_ir::Block::Heading { level, inlines, .. } => {
                    Some((*level, kasane_ir_text(inlines)))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            heads,
            vec![
                (1, "Chapter One".to_string()),
                (2, "Section Two".to_string())
            ]
        );
    }
    #[test]
    fn end_to_end_pptx_fixture_to_sitetree() {
        let bytes = std::fs::read("../../tests/fixtures/pptx/minimal.pptx").unwrap();
        assert!(matches!(detect(&bytes, Some("pptx")), Some(Format::Pptx)));

        let (doc, assets) = PptxAdapter.parse(&bytes, "minimal.pptx").unwrap();
        assert_eq!(doc.meta.source_format, "pptx");
        assert_eq!(doc.meta.title, "minimal");

        // slide order + title fallback: slide1 "Welcome", slide2 "Data"
        let headings: Vec<String> = doc
            .nodes
            .iter()
            .filter_map(|n| match &n.block {
                kasane_ir::Block::Heading { inlines, .. } => Some(kasane_ir_text(inlines)),
                _ => None,
            })
            .collect();
        assert_eq!(headings, vec!["Welcome".to_string(), "Data".to_string()]);

        // media flushed through the whole pipeline
        assert_eq!(assets.items.len(), 1);

        // structuring + writing succeeds end to end
        let site = kasane_core::structure(doc, &kasane_core::Options::default());
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("deck");
        kasane_writer::write_tree(&site, &assets, &out, false).unwrap();
        assert!(out.join("index.md").exists());
        assert!(out.join("_assets").read_dir().unwrap().next().is_some());
    }
    fn kasane_ir_text(inls: &[kasane_ir::Inline]) -> String {
        inls.iter()
            .map(|i| {
                if let kasane_ir::Inline::Text(t) = i {
                    t.clone()
                } else {
                    String::new()
                }
            })
            .collect()
    }
}
