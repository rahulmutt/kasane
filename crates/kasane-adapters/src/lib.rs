mod detect;
mod epub;
mod guard;
#[allow(dead_code)] // consumed in later tasks
mod pptx;
mod ziputil;

pub use detect::{detect, Format};
pub use epub::EpubAdapter;

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
