mod assets;
mod block;
mod doc;
mod ids;
mod inline;

pub use assets::{AssetBag, AssetItem};
pub use block::{AssetRef, Block, Table};
pub use doc::{DocMeta, Document, Node, Provenance};
pub use ids::{BlockId, NoteId};
pub use inline::{Inline, RefTarget};

/// Maximum inline nesting depth the structuring engine and the writer will
/// descend.
///
/// Every inline walk in `kasane-core` and `kasane-writer` recurses on nesting
/// depth. Past this bound they stop descending and contribute nothing, because
/// the alternative is a stack overflow — which aborts the process outright
/// rather than surfacing as a recoverable error.
///
/// This is a *safety* bound, not a fidelity one. The EPUB adapter flattens at a
/// much lower depth without losing content (`epub::xhtml::MAX_INLINE_DEPTH`), so
/// adapter-produced IR never reaches this value; it exists for hand-built
/// `Document`s from external callers of the published `structure()`.
///
/// Measured, not guessed: in a debug build on a libtest thread, depth 256 and
/// 1024 both complete and 4096 aborts, so 256 keeps at least a 4x margin under
/// the tightest stack the suite runs on.
pub const MAX_INLINE_DEPTH: usize = 256;

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn build_minimal_document() {
        let doc = Document {
            meta: DocMeta {
                title: "T".into(),
                authors: vec![],
                language: None,
                source_format: "epub".into(),
                source_path: "t.epub".into(),
            },
            nodes: vec![Node {
                block: Block::Heading {
                    level: 1,
                    id: BlockId(0),
                    inlines: vec![Inline::Text("Hi".into())],
                },
                prov: Provenance {
                    source_pages: None,
                    source_href: Some("ch1.xhtml".into()),
                },
            }],
        };
        assert_eq!(doc.nodes.len(), 1);
        assert!(matches!(
            doc.nodes[0].block,
            Block::Heading { level: 1, .. }
        ));
    }
}
