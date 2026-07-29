//! `structure()` takes ownership of the caller's `Document` and drops it on
//! return. `Inline` has no manual `Drop`, so before this fix teardown
//! recursed once per nesting level via the derived `Drop` glue — a second,
//! independent abort path past the bounded walks and the bounded clones,
//! since it fires on the way out rather than during any walk.

use kasane_core::{structure, Options};
use kasane_ir::*;

fn nested(depth: usize) -> Inline {
    let mut i = Inline::Text("x".into());
    for _ in 0..depth {
        i = Inline::Emph(vec![i]);
    }
    i
}

fn doc_with(inline: Inline) -> Document {
    Document {
        meta: DocMeta {
            title: "T".into(),
            authors: vec![],
            language: None,
            source_format: "epub".into(),
            source_path: "t".into(),
        },
        nodes: vec![Node {
            block: Block::Para(vec![inline]),
            prov: Provenance::default(),
        }],
    }
}

#[test]
fn structure_does_not_abort_tearing_down_a_deeply_nested_document() {
    // Depth 40,000: the walks and clones bounded elsewhere in this task
    // finish quickly regardless of depth, but before this fix returning from
    // `structure()` dropped the original, un-bounded `Document` and aborted
    // here.
    let site = structure(doc_with(nested(40_000)), &Options::default());
    assert!(
        !site.files.is_empty(),
        "structuring must produce output, not abort"
    );
}
