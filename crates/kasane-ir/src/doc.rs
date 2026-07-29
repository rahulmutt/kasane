use crate::block::Block;
use crate::inline::Inline;

#[derive(Clone, Debug)]
pub struct Document {
    pub meta: DocMeta,
    pub nodes: Vec<Node>,
}

#[derive(Clone, Debug)]
pub struct DocMeta {
    pub title: String,
    pub authors: Vec<String>,
    pub language: Option<String>,
    pub source_format: String,
    pub source_path: String,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub block: Block,
    pub prov: Provenance,
}

#[derive(Clone, Debug, Default)]
pub struct Provenance {
    pub source_pages: Option<(u32, u32)>,
    pub source_href: Option<String>,
}

/// Tear `doc` down with an explicit worklist rather than letting the
/// compiler-derived `Drop` on `Block`/`Inline` recurse on block or inline
/// nesting depth. Each `Vec` here plays the role of an explicit call stack:
/// popping and matching one value moves its owned children onto the same
/// `Vec` instead of the runtime recursing into their drop glue, so no single
/// `drop` ever costs more than one level regardless of how deep the input
/// was.
///
/// Neither block nesting (`Block::List`/`Block::Footnote`) nor inline nesting
/// past whatever bound (if any) produced it is limited by this type itself:
/// a `Document` handed to `kasane_core::structure()` from an external caller,
/// or held by a fuzz target that has already checked its own depth
/// invariants and just needs to free the value, can be arbitrarily deep. This
/// lives beside `Block`/`Inline` rather than in either of those two callers
/// because it is fundamentally a property of the recursive IR type, not of
/// any one consumer of it — and because a plain `impl Drop for Inline` is not
/// an option: it would forbid moving out of `Inline`, which every
/// depth-bounded walk anywhere in this workspace does via `match inl {
/// Inline::Emph(x) => ... }`.
///
/// `#[doc(hidden)]` because this is a test-seam / internal-safety-valve
/// export for exactly its two callers (`kasane_core::nav::structure` and
/// `kasane_adapters`'s fuzz seam), not a general-purpose public API.
#[doc(hidden)]
pub fn teardown_document(doc: Document) {
    let mut blocks: Vec<Block> = doc.nodes.into_iter().map(|n| n.block).collect();
    while let Some(b) = blocks.pop() {
        match b {
            Block::Heading { inlines, .. } | Block::Para(inlines) => teardown_inlines(inlines),
            Block::List { items, .. } => {
                for item in items {
                    blocks.extend(item);
                }
            }
            Block::Table(t) => {
                for c in t.header {
                    teardown_inlines(c);
                }
                for r in t.rows {
                    for c in r {
                        teardown_inlines(c);
                    }
                }
            }
            Block::Figure { caption, .. } => teardown_inlines(caption),
            Block::Footnote { blocks: inner, .. } => blocks.extend(inner),
            Block::CodeBlock { .. } | Block::MathBlock(_) | Block::Raw { .. } => {}
        }
    }
}

fn teardown_inlines(inls: Vec<Inline>) {
    let mut stack = inls;
    while let Some(i) = stack.pop() {
        match i {
            Inline::Emph(x) | Inline::Strong(x) => stack.extend(x),
            Inline::Link { inlines, .. } => stack.extend(inlines),
            Inline::Text(_) | Inline::Code(_) | Inline::Math(_) | Inline::FootnoteRef(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoteId;

    /// Regression coverage for the move from `kasane_core::nav` /
    /// `kasane_adapters::fuzz_entry` into this crate: both call sites relied
    /// on this not aborting on deep block AND deep inline nesting.
    #[test]
    fn teardown_document_survives_deep_block_and_inline_nesting() {
        const DEPTH: usize = 100_000;
        let mut inline = Inline::Text("bottom".into());
        for _ in 0..DEPTH {
            inline = Inline::Emph(vec![inline]);
        }
        let mut blocks = vec![Block::Para(vec![inline])];
        for _ in 0..DEPTH {
            blocks = vec![Block::List {
                ordered: false,
                items: vec![blocks],
            }];
        }
        // A footnote wrapping the same chain, so both recursive Block arms
        // this function walks are exercised, not just List.
        blocks = vec![Block::Footnote {
            id: NoteId(1),
            blocks,
        }];
        let doc = Document {
            meta: DocMeta {
                title: "T".into(),
                authors: vec![],
                language: None,
                source_format: "test".into(),
                source_path: "test".into(),
            },
            nodes: blocks
                .into_iter()
                .map(|block| Node {
                    block,
                    prov: Provenance::default(),
                })
                .collect(),
        };
        teardown_document(doc); // must return normally, not abort
    }

    #[test]
    fn teardown_document_handles_an_empty_document() {
        teardown_document(Document {
            meta: DocMeta {
                title: "T".into(),
                authors: vec![],
                language: None,
                source_format: "test".into(),
                source_path: "test".into(),
            },
            nodes: vec![],
        });
    }
}
