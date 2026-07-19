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
