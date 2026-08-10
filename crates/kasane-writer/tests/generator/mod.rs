//! Adapter-realistic `Document` strategies for the property suite.
//!
//! Design spec `2026-07-29-core-property-tier-design.md` §4. Two ideas carry
//! the whole design:
//!
//! **Sentinels.** Every generated block carries a unique token, so conservation
//! can be checked by counting occurrences in the rendered Markdown rather than
//! by structural comparison. That stays true no matter how `balance()` rewrites
//! a block, and the property never has to encode what the engine synthesizes.
//! Strategies compose without shared state, so uniqueness cannot come from a
//! counter threaded through generation: the strategy builds a skeleton with
//! placeholder text, and one deterministic `prop_map` stamps sequential tokens
//! over the finished skeleton.
//!
//! **Adapter realism.** Heading levels 1..=6, nesting capped at 3, block mix
//! weighted toward paragraphs. A failure is then unambiguously reachable from a
//! real document, with no triage step asking "can any adapter produce this?".

#![allow(dead_code)] // each property uses a different subset of this module

use kasane_core::Options;
use kasane_ir::*;
use proptest::prelude::*;

/// How many times a sentinel must appear across the rendered files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expect {
    /// Non-heading blocks: exactly one render site.
    Exactly(usize),
    /// Headings legitimately recur — the file's own title heading, the parent's
    /// TOC link, a merge lead-in.
    AtLeast(usize),
}

#[derive(Clone, Debug)]
pub struct Sentinel {
    /// The bare `zq####` token. Alphanumeric, so no escape can appear inside
    /// it and P1's raw-text counting is unaffected by the escaping policy.
    pub token: String,
    /// The full text stamped into the block: the token plus a hostile suffix.
    /// P7 counts *this* in the re-parsed text, which is what makes a missed
    /// escape a failure rather than a curiosity.
    pub payload: String,
    pub expect: Expect,
}

#[derive(Clone, Debug)]
pub struct Case {
    pub doc: Document,
    pub opts: Options,
    pub assets: AssetBag,
    pub sentinels: Vec<Sentinel>,
}

/// Words the generator draws filler text from. Deliberately free of the `zq`
/// sentinel prefix, so generated content can never collide with a token.
///
/// The last five exist to exercise the slug rules rather than to be realistic
/// prose: `&` produces the double-hyphen anchor GFM parity requires, `don't`
/// produces the removed-not-replaced apostrophe, `foo_bar` guards the
/// underscore that `heading_anchors` used to strip, and the CJK and Devanagari
/// words put non-Latin text into both filenames and anchors. Bracket and
/// parenthesis characters are deliberately absent: `links_in` would collect a
/// false link and P2 would fail spuriously.
const WORDS: &[&str] = &[
    "alpha",
    "beta",
    "gamma",
    "delta",
    "epsilon",
    "the",
    "and",
    "of",
    "a",
    "chapter",
    "section",
    "&",
    "don't",
    "foo_bar",
    "第二章",
    "हिन्दी",
];

/// Markdown-hostile fragments, drawn into the same text `WORDS` feeds.
///
/// Every one of these renders as markup, or breaks a container, if the writer
/// emits it unescaped: the inline openers, the line-start block openers, a
/// pipe that splits a cell, a fence and a backtick that break out of code, an
/// entity that would decode, an HTML comment closer, and an embedded newline.
/// `zq` is deliberately absent, for the same reason `WORDS` avoids it — a
/// fragment containing the sentinel prefix would corrupt P1's counting.
const HOSTILE: &[&str] = &[
    "*star*",
    "_under_",
    "[bracket]",
    "]close",
    "`tick`",
    "```fence",
    "<html>",
    "&amp;",
    "&raw",
    "$math$",
    "~~strike~~",
    "back\\slash",
    "-->",
    "|pipe|",
    "#hash",
    "- bullet",
    "1. ordered",
    "> quote",
    "= setext",
    "line\nbreak",
    "!bang[",
];

/// Filler text, with hostile fragments mixed in often enough that a case
/// without one is rare.
fn filler() -> impl Strategy<Value = String> {
    let word = prop_oneof![
        3 => proptest::sample::select(WORDS),
        1 => proptest::sample::select(HOSTILE),
    ];
    proptest::collection::vec(word, 1..12).prop_map(|ws| ws.join(" "))
}

/// One inline run, nested at most `depth` levels. Boxed at every level because
/// the strategy is recursive and its type would otherwise be infinite. The leaf
/// is built twice rather than cloned, so this compiles without requiring the
/// mapped strategy to be `Clone`.
fn inlines(depth: u32) -> BoxedStrategy<Vec<Inline>> {
    if depth == 0 {
        return filler().prop_map(|s| vec![Inline::Text(s)]).boxed();
    }
    prop_oneof![
        8 => filler().prop_map(|s| vec![Inline::Text(s)]),
        1 => inlines(depth - 1).prop_map(|x| vec![Inline::Emph(x)]),
        1 => inlines(depth - 1).prop_map(|x| vec![Inline::Strong(x)]),
    ]
    .boxed()
}

/// The block shapes an adapter really produces. `SHAPE_*` picks the variant; the
/// sentinel is stamped in afterwards.
#[derive(Clone, Debug)]
enum Shape {
    Heading(u8),
    Para,
    List(bool),
    /// `ordered` plus a nesting depth. Kept well under
    /// `epub::xhtml::MAX_BLOCK_DEPTH` on purpose: this tier is
    /// adapter-realistic by design, and IR deeper than any adapter can
    /// produce is the safety bound's unit tests' job, not this tier's.
    NestedList(bool, u8),
    Table(bool),
    Figure(bool),
    Code,
    Math,
    Raw,
    Footnote,
}

fn shape() -> impl Strategy<Value = Shape> {
    prop_oneof![
        3 => (1u8..=6).prop_map(Shape::Heading),
        8 => Just(Shape::Para),
        2 => any::<bool>().prop_map(Shape::List),
        2 => (any::<bool>(), 2u8..=4).prop_map(|(o, d)| Shape::NestedList(o, d)),
        1 => any::<bool>().prop_map(Shape::Table),
        1 => any::<bool>().prop_map(Shape::Figure),
        1 => Just(Shape::Code),
        1 => Just(Shape::Math),
        1 => Just(Shape::Raw),
        1 => Just(Shape::Footnote),
    ]
}

/// Builds one block from a shape, stamping `payload` into the single position
/// that renders, and reporting how many times it is expected to appear.
///
/// `deco` is generated nested inline markup (depth <= 3) appended after the
/// sentinel, so the engine's and the writer's inline walks are exercised on real
/// nesting rather than only on flat text. It is appended, never wrapped around
/// the payload, so the payload itself always renders as a bare run and the
/// occurrence count stays exact.
///
/// Two shapes need no special handling despite carrying hostile text:
/// `Shape::Code` puts the payload inside a code block, which is where
/// ` ```fence` and `` `tick` `` do their work against `escape::fenced_block`.
/// `Shape::Raw` puts it inside an HTML comment, which is where `-->` works
/// against `escape::comment_note`.
fn build(shape: &Shape, deco: &[Inline], payload: &str, idx: u32) -> (Block, Expect) {
    let text = |t: &str| {
        let mut v = vec![Inline::Text(t.to_string())];
        v.extend(deco.iter().cloned());
        v
    };
    match shape {
        Shape::Heading(level) => (
            Block::Heading {
                level: *level,
                id: BlockId(idx),
                inlines: text(payload),
            },
            Expect::AtLeast(1),
        ),
        Shape::Para => (Block::Para(text(payload)), Expect::Exactly(1)),
        Shape::List(ordered) => (
            Block::List {
                ordered: *ordered,
                items: vec![vec![Block::Para(text(payload))]],
            },
            Expect::Exactly(1),
        ),
        // The payload sits at the bottom of the chain and renders exactly
        // once, so the conservation invariant's arithmetic is unchanged from
        // the flat-list case -- what changes is the depth the walks must
        // survive to reach it.
        Shape::NestedList(ordered, depth) => {
            let mut inner = vec![Block::Para(text(payload))];
            for _ in 0..*depth {
                inner = vec![Block::List {
                    ordered: *ordered,
                    items: vec![inner],
                }];
            }
            (
                inner.pop().expect("depth >= 1 builds one list"),
                Expect::Exactly(1),
            )
        }
        // markdown.rs:79-106: both render paths call `inlines_to_md` exactly
        // once for the single generated row's single cell -- the merged
        // branch via `<td>{esc(c)}</td>` (line 92), the pipe-table branch via
        // the `cells` closure (lines 99-101) -- so the payload renders exactly
        // once whether or not `has_merged` is set. Generating the flag (not
        // pinning it) is what makes the HTML branch reachable at all.
        Shape::Table(merged) => (
            Block::Table(Table {
                header: vec![text("col")],
                rows: vec![vec![text(payload)]],
                has_merged: *merged,
            }),
            Expect::Exactly(1),
        ),
        // markdown.rs:54-61 renders a numbered figure's caption twice: once as
        // alt text, once in the visible `*Figure N: ...*` line. Deliberate
        // (alt text plus a caption), so the expectation says two, not one.
        Shape::Figure(numbered) => (
            Block::Figure {
                image: AssetRef {
                    key: format!("img{}", idx),
                    bytes_ref: 0,
                },
                caption: text(payload),
                number: numbered.then(|| "1".to_string()),
            },
            Expect::Exactly(if *numbered { 2 } else { 1 }),
        ),
        Shape::Code => (
            Block::CodeBlock {
                lang: Some("rust".into()),
                text: payload.to_string(),
            },
            Expect::Exactly(1),
        ),
        Shape::Math => (Block::MathBlock(payload.to_string()), Expect::Exactly(1)),
        Shape::Raw => (
            Block::Raw {
                note: payload.to_string(),
            },
            Expect::Exactly(1),
        ),
        Shape::Footnote => (
            Block::Footnote {
                id: NoteId(idx),
                blocks: vec![Block::Para(text(payload))],
            },
            Expect::Exactly(1),
        ),
    }
}

/// A generated case: document, options, assets, and the sentinel ledger.
pub fn case() -> impl Strategy<Value = Case> {
    // Each entry pairs a block shape with generated nested inline markup, so
    // nesting depth up to 3 is present throughout rather than only in flat runs.
    let shapes = proptest::collection::vec(
        (shape(), inlines(3), proptest::sample::select(HOSTILE)),
        1..40,
    );
    let opts = (40usize..400, 5usize..40).prop_map(|(max_tokens, min_tokens)| Options {
        max_tokens,
        // min < max by construction, so the engine is never asked to satisfy
        // contradictory thresholds.
        min_tokens: min_tokens.min(max_tokens.saturating_sub(1)),
    });

    (shapes, opts).prop_map(|(shapes, opts)| {
        let mut nodes = Vec::new();
        let mut sentinels = Vec::new();
        let mut assets = AssetBag::default();

        for (i, (sh, deco, hostile)) in shapes.iter().enumerate() {
            let idx = i as u32;
            let token = format!("zq{:04}", idx);
            let payload = format!("{token} {hostile}");
            let (block, expect) = build(sh, deco, &payload, idx);

            // A figure needs a matching asset or the renderer emits "missing".
            if let Shape::Figure(_) = sh {
                assets.items.push(AssetItem {
                    key: format!("img{}", idx),
                    filename: format!("img{}.png", idx),
                    bytes: vec![0x89, b'P', b'N', b'G'],
                });
            }

            nodes.push(Node {
                block,
                prov: Provenance::default(),
            });
            sentinels.push(Sentinel {
                token,
                payload,
                expect,
            });
        }

        Case {
            doc: Document {
                meta: DocMeta {
                    title: "Generated Book".into(),
                    authors: vec![],
                    language: None,
                    source_format: "epub".into(),
                    source_path: "generated.epub".into(),
                },
                nodes,
            },
            opts,
            assets,
            sentinels,
        }
    })
}

/// A case whose paragraphs additionally carry internal cross-references — some
/// pointing at real generated headings, some dangling, so both the resolve path
/// and the strip path (`refs.rs:63-68`) are exercised.
///
/// The target is drawn from *any* generated heading, not the first one. That is
/// load-bearing coverage, not a stylistic preference: `fold_sections` always
/// pushes the first heading in document order as a direct child of root, and
/// `balance_node`'s merge requires `node.level > 0`, so root's children are
/// never merged. Targeting `heading_ids.first()` therefore made the merged-
/// subsection anchor — a heading demoted into its parent's body, anchored by
/// `assign_paths`' body scan — structurally unreachable from this property.
pub fn case_with_links() -> impl Strategy<Value = Case> {
    (case(), any::<bool>())
        .prop_flat_map(|(c, dangle)| {
            let heading_ids: Vec<BlockId> = c
                .doc
                .nodes
                .iter()
                .filter_map(|n| match &n.block {
                    Block::Heading { id, .. } => Some(*id),
                    _ => None,
                })
                .collect();
            let target: BoxedStrategy<BlockId> = if dangle || heading_ids.is_empty() {
                // No heading generated, or deliberately dangling: an id far past
                // anything the generator assigns.
                Just(BlockId(9_999)).boxed()
            } else {
                proptest::sample::select(heading_ids).boxed()
            };
            (Just(c), target)
        })
        .prop_map(|(mut c, target)| {
            for n in c.doc.nodes.iter_mut() {
                if let Block::Para(inls) = &mut n.block {
                    inls.push(Inline::Link {
                        target: RefTarget::Internal(target),
                        inlines: vec![Inline::Text("see".into())],
                    });
                    break;
                }
            }
            c
        })
}
