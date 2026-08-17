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
///
/// Parity is load-bearing, not incidental. The loop starts the innermost pair
/// at `n = 0` with `Strong`, so the *outermost* pair (`n = depth - 1`) is
/// `Emph` over a sole `Strong` — the exempted shape — exactly when `depth` is
/// even; an odd `depth` puts `Strong` outermost over a sole `Emph`, which
/// §3.2 still splices. Measured: `nested_alternating(8)` prints `***x***`,
/// `nested_alternating(7)` and `nested_alternating(9)` both print `**x**`. A
/// future edit that moves a call site to an odd depth would silently stop
/// exercising the exemption while every assertion here kept passing.
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
    // Depth 8 is far under the bound and, being even, lands the exempted
    // shape at the outermost pair (see `nested_alternating`'s doc comment):
    // `Emph` wrapping a sole `Strong`, which §3.2 declines to splice and
    // therefore prints `***x***` rather than the `*x*` the old, unconditional
    // edge rule produced. `md.contains('x')` alone cannot tell those apart —
    // both retain the letter — so it passes identically whether the
    // exemption fires, is absent, or is silently reverted, and pins nothing
    // about the behaviour this test is named for. Asserting the full
    // un-spliced spelling does: revert the exemption and this fails, because
    // the printed run collapses to `*x*`.
    let site = structure(doc_with(nested_alternating(8)), &Options::default());
    let md = kasane_writer::blocks_to_markdown(&site.files[0].blocks, &AssetBag::default());
    assert!(
        md.contains("***x***"),
        "the retained Strong wrapper must survive printing as `***x***`, not just its text"
    );
}

/// The §7 risk this file exists to check but, before this test, never did:
/// declining the edge splice keeps a container the old rule flattened, so a
/// cross-class chain now carries one more un-collapsed level than a
/// same-class chain of the same input depth. `deep_cross_class_nesting_does_not_abort`
/// only shows the writer survives 10,000 levels either way; it says nothing
/// about whether the retained container eats into the headroom below
/// `MAX_INLINE_DEPTH` (256, `kasane_ir::MAX_INLINE_DEPTH`) and makes a
/// cross-class chain truncate at a shallower *input* depth than a same-class
/// one would.
///
/// This measures the deepest input depth at which each helper's output still
/// contains the text, and asserts the two are equal. A hard-coded depth here
/// would just pin `MAX_INLINE_DEPTH`'s current value and break on any future
/// change to that constant for reasons unrelated to this risk; comparing the
/// two helpers against each other keeps testing the actual thing at risk —
/// that the exemption costs no extra headroom — independent of where the
/// bound sits.
///
/// `take_while` assumes survival is monotonic in depth (once truncation
/// starts, deeper inputs stay truncated); the scan below covers
/// `1..=MAX_INLINE_DEPTH + 2` on every run, and each helper has exactly one
/// survived→truncated transition in that range. A one-off manual check
/// extended the scan to `MAX_INLINE_DEPTH + 4` and found no further
/// transition, but that wider range is not what this test re-checks.
#[test]
fn cross_class_nesting_truncates_no_earlier_than_same_class() {
    let deepest_surviving = |build: &dyn Fn(usize) -> Inline| -> Option<usize> {
        (1..=kasane_ir::MAX_INLINE_DEPTH + 2)
            .take_while(|&d| {
                let site = structure(doc_with(build(d)), &Options::default());
                kasane_writer::blocks_to_markdown(&site.files[0].blocks, &AssetBag::default())
                    .contains('x')
            })
            .last()
    };

    let same_class = deepest_surviving(&nested);
    let cross_class = deepest_surviving(&nested_alternating);

    assert!(
        same_class.is_some(),
        "the same-class helper survived no depth at all -- the comparison below would pass vacuously"
    );

    // Measured at MAX_INLINE_DEPTH == 256: both are Some(255). The depth
    // guard (`markdown.rs`'s `depth >= MAX_INLINE_DEPTH`) fires on the input
    // IR's structural depth, walked before any splicing decision, so the
    // retained container does not cost the headroom §7 worried it might —
    // but that is exactly the fact this assertion exists to keep true.
    assert_eq!(
        same_class, cross_class,
        "cross-class nesting must not truncate at a shallower input depth than \
         same-class nesting: the retained container from the edge-splice \
         exemption (design spec §3) must not cost headroom against MAX_INLINE_DEPTH"
    );
}
