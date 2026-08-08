mod assets;
mod block;
mod doc;
mod ids;
mod inline;

pub use assets::{AssetBag, AssetItem};
pub use block::{AssetRef, Block, Table};
pub use doc::{teardown_document, DocMeta, Document, Node, Provenance};
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

/// Maximum block nesting the structuring engine and the writer will descend.
///
/// `Block::List` and `Block::Footnote` nest, and every block walk in
/// `kasane-core` and `kasane-writer` recurses on that nesting. Past this
/// bound they stop descending, because the alternative is a stack overflow —
/// which aborts the process outright rather than surfacing as a recoverable
/// error.
///
/// This is a *safety* bound, not a fidelity one. The EPUB parser flattens at
/// a much lower depth without losing content (`epub::xhtml::MAX_BLOCK_DEPTH`,
/// 32); MOBI/AZW3 inherits that same bound by re-serializing through the
/// same parser; and PPTX carries its own fidelity bound,
/// `pptx::slide::MAX_LIST_LEVEL`. Each of these fidelity bounds is pinned to
/// this constant by its own compile-time assertion, so adapter-produced IR
/// never reaches this value. It exists for hand-built `Document`s from
/// external callers of the published `structure()`. The ordering invariant
/// the design depends on is `epub::xhtml::MAX_BLOCK_DEPTH` (32) <
/// `kasane_ir::MAX_BLOCK_DEPTH` (128) — and, separately,
/// `pptx::slide::MAX_LIST_LEVEL` (31, 0-based) kept to the same quarter
/// margin.
///
/// Measured, not guessed: in a debug build, a rayon worker in batch mode —
/// the tightest stack the code actually runs on — completes block depth 750
/// and aborts at 875, while a libtest thread completes 512 and aborts at
/// 1024. 128 is the largest power of two under a quarter of 875, so it keeps
/// at least a 4x margin under the binding thread. The measurement composed a
/// full-depth inline chain underneath the block chain, because a block walk
/// at depth d can call an inline walk `MAX_INLINE_DEPTH` deep and the two
/// budgets add.
pub const MAX_BLOCK_DEPTH: usize = 128;

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

    /// The two-bound design only works if adapter IR cannot reach the safety
    /// bound. `kasane-ir` cannot name the adapter constant (it depends on
    /// nothing), so this asserts the safety side of the contract: the value
    /// is large enough that the documented fidelity bound sits well under it.
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn block_bound_leaves_room_under_the_inline_bound_convention() {
        assert!(
            MAX_BLOCK_DEPTH >= 4,
            "a bound under 4 cannot host a /4 fidelity bound"
        );
        assert!(MAX_BLOCK_DEPTH.is_power_of_two());
    }
}
