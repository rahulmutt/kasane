//! Guards the generator itself. A property suite is only as good as what it
//! generates, and a generator that silently stops producing headings (or
//! duplicate sentinels) would leave every property passing vacuously.

mod generator;

use kasane_ir::{Block, Inline};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;

proptest! {
    #[test]
    fn sentinels_are_unique(case in generator::case()) {
        let mut seen = std::collections::HashSet::new();
        for s in &case.sentinels {
            prop_assert!(seen.insert(s.token.clone()), "duplicate sentinel {}", s.token);
        }
    }

    #[test]
    fn every_block_carries_a_sentinel(case in generator::case()) {
        prop_assert_eq!(case.sentinels.len(), case.doc.nodes.len());
    }

    #[test]
    fn options_are_well_ordered(case in generator::case()) {
        prop_assert!(case.opts.min_tokens < case.opts.max_tokens);
    }
}

/// True if any inline in `inls` is a nested run (`Emph`/`Strong`) rather than
/// a bare leaf. A single `Emph`/`Strong` occurrence already proves the
/// `inlines()` strategy took a nesting branch, so this does not need to
/// recurse further to be conclusive.
fn has_nested_inline(inls: &[Inline]) -> bool {
    inls.iter()
        .any(|i| matches!(i, Inline::Emph(_) | Inline::Strong(_)))
}

/// Not a `proptest!` property: proptest draws each property's cases
/// independently, so no single `case()` draw can be relied on to contain
/// every `Shape` variant (the rarest carry weight 1 of 19) or any nested
/// inline (`deco` only takes a nesting branch with probability 0.2). This
/// draws many cases directly from the same strategy and asserts what is only
/// true in aggregate: that the generator still produces every block shape and
/// still produces nested inline markup. Without this, a regression that
/// collapsed `shape()` to always emit `Shape::Para`, or `inlines()` to always
/// take the flat-leaf branch, would pass all three properties above and leave
/// every property in Task 6 passing vacuously.
#[test]
fn generator_covers_every_shape_and_some_nesting() {
    // 200 draws, each case holding 1..40 blocks (average ~20): roughly 4,000
    // independent shape picks in total. The rarest `Shape` variant has weight
    // 1 of 19, so its expected count here is ~210 and the probability it
    // never appears is (18/19)^4000 -- effectively zero, so this is
    // deterministic in practice -- while still running far fewer total cases
    // than the proptest suite above (256 cases x 3 properties).
    const DRAWS: usize = 200;

    let mut runner = TestRunner::default();
    let strategy = generator::case();

    let mut seen_heading = false;
    let mut seen_para = false;
    let mut seen_list = false;
    let mut seen_table = false;
    let mut seen_figure = false;
    let mut seen_code = false;
    let mut seen_math = false;
    let mut seen_raw = false;
    let mut seen_footnote = false;
    let mut seen_nested_inline = false;

    for _ in 0..DRAWS {
        let case = strategy
            .new_tree(&mut runner)
            .expect("strategy generation must not fail")
            .current();
        for node in &case.doc.nodes {
            match &node.block {
                Block::Heading { inlines, .. } => {
                    seen_heading = true;
                    seen_nested_inline |= has_nested_inline(inlines);
                }
                Block::Para(inls) => {
                    seen_para = true;
                    seen_nested_inline |= has_nested_inline(inls);
                }
                Block::List { items, .. } => {
                    seen_list = true;
                    for item in items {
                        for b in item {
                            if let Block::Para(inls) = b {
                                seen_nested_inline |= has_nested_inline(inls);
                            }
                        }
                    }
                }
                Block::Table(t) => {
                    seen_table = true;
                    for cell in t.header.iter().chain(t.rows.iter().flatten()) {
                        seen_nested_inline |= has_nested_inline(cell);
                    }
                }
                Block::Figure { caption, .. } => {
                    seen_figure = true;
                    seen_nested_inline |= has_nested_inline(caption);
                }
                Block::CodeBlock { .. } => seen_code = true,
                Block::MathBlock(_) => seen_math = true,
                Block::Raw { .. } => seen_raw = true,
                Block::Footnote { blocks, .. } => {
                    seen_footnote = true;
                    for b in blocks {
                        if let Block::Para(inls) = b {
                            seen_nested_inline |= has_nested_inline(inls);
                        }
                    }
                }
            }
        }
    }

    assert!(seen_heading, "generator stopped producing Shape::Heading");
    assert!(seen_para, "generator stopped producing Shape::Para");
    assert!(seen_list, "generator stopped producing Shape::List");
    assert!(seen_table, "generator stopped producing Shape::Table");
    assert!(seen_figure, "generator stopped producing Shape::Figure");
    assert!(seen_code, "generator stopped producing Shape::Code");
    assert!(seen_math, "generator stopped producing Shape::Math");
    assert!(seen_raw, "generator stopped producing Shape::Raw");
    assert!(seen_footnote, "generator stopped producing Shape::Footnote");
    assert!(
        seen_nested_inline,
        "generator stopped producing nested (Emph/Strong) inline markup"
    );
}
