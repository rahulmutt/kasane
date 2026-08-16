//! Deeply nested inlines must not abort the process. See design spec
//! `2026-07-29-core-property-tier-design.md` §2.2: every inline walk in the
//! core and the writer recurses on nesting depth, and an unbounded walk
//! overflows the stack — which aborts, and in batch mode takes every other
//! worker's document down with it.

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
fn deep_inline_nesting_does_not_abort() {
    let site = structure(doc_with(nested(10_000)), &Options::default());
    let md = kasane_writer::blocks_to_markdown(&site.files[0].blocks, &AssetBag::default());
    assert!(!md.is_empty(), "rendering must produce output, not abort");
}

#[test]
fn nesting_within_the_bound_is_preserved() {
    // Depth 8 is far under the bound: the text at the bottom must survive.
    let site = structure(doc_with(nested(8)), &Options::default());
    let md = kasane_writer::blocks_to_markdown(&site.files[0].blocks, &AssetBag::default());
    assert!(
        md.contains('x'),
        "content within the bound must not be dropped"
    );
}

/// Alternating classes, which the splice rules no longer flatten all the way.
///
/// `nested` above builds same-class nesting, every level of which
/// `same_delim_to_splice` removes — so it cannot see the depth this writer
/// actually recurses to. Since
/// `2026-08-16-cross-class-edge-splice-design.md` §3, an `Emph` wrapping only
/// a `Strong` survives, so an alternating chain keeps levels a same-class
/// chain never did.
fn nested_alternating(depth: usize) -> Inline {
    let mut i = Inline::Text("x".into());
    for n in 0..depth {
        i = if n % 2 == 0 {
            Inline::Strong(vec![i])
        } else {
            Inline::Emph(vec![i])
        };
    }
    i
}

#[test]
fn deep_cross_class_nesting_does_not_abort() {
    let site = structure(doc_with(nested_alternating(10_000)), &Options::default());
    let md = kasane_writer::blocks_to_markdown(&site.files[0].blocks, &AssetBag::default());
    assert!(!md.is_empty(), "rendering must produce output, not abort");
}

#[test]
fn cross_class_nesting_within_the_bound_is_preserved() {
    // Depth 8 is far under the bound: the text at the bottom must survive.
    let site = structure(doc_with(nested_alternating(8)), &Options::default());
    let md = kasane_writer::blocks_to_markdown(&site.files[0].blocks, &AssetBag::default());
    assert!(
        md.contains('x'),
        "content within the bound must not be dropped"
    );
}
