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
    pub token: String,
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
const WORDS: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "epsilon", "the", "and", "of", "a", "chapter", "section",
];

fn filler() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::sample::select(WORDS), 1..12).prop_map(|ws| ws.join(" "))
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
        1 => any::<bool>().prop_map(Shape::Table),
        1 => any::<bool>().prop_map(Shape::Figure),
        1 => Just(Shape::Code),
        1 => Just(Shape::Math),
        1 => Just(Shape::Raw),
        1 => Just(Shape::Footnote),
    ]
}

/// Builds one block from a shape, stamping `token` into the single position
/// that renders, and reporting how many times it is expected to appear.
///
/// `deco` is generated nested inline markup (depth <= 3) appended after the
/// sentinel, so the engine's and the writer's inline walks are exercised on real
/// nesting rather than only on flat text. It is appended, never wrapped around
/// the token, so the token itself always renders as a bare run and the
/// occurrence count stays exact.
fn build(shape: &Shape, deco: &[Inline], token: &str, idx: u32) -> (Block, Expect) {
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
                inlines: text(token),
            },
            Expect::AtLeast(1),
        ),
        Shape::Para => (Block::Para(text(token)), Expect::Exactly(1)),
        Shape::List(ordered) => (
            Block::List {
                ordered: *ordered,
                items: vec![vec![Block::Para(text(token))]],
            },
            Expect::Exactly(1),
        ),
        // markdown.rs:79-106: both render paths call `inlines_to_md` exactly
        // once for the single generated row's single cell -- the merged
        // branch via `<td>{esc(c)}</td>` (line 92), the pipe-table branch via
        // the `cells` closure (lines 99-101) -- so the token renders exactly
        // once whether or not `has_merged` is set. Generating the flag (not
        // pinning it) is what makes the HTML branch reachable at all.
        Shape::Table(merged) => (
            Block::Table(Table {
                header: vec![text("col")],
                rows: vec![vec![text(token)]],
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
                caption: text(token),
                number: numbered.then(|| "1".to_string()),
            },
            Expect::Exactly(if *numbered { 2 } else { 1 }),
        ),
        Shape::Code => (
            Block::CodeBlock {
                lang: Some("rust".into()),
                text: token.to_string(),
            },
            Expect::Exactly(1),
        ),
        Shape::Math => (Block::MathBlock(token.to_string()), Expect::Exactly(1)),
        Shape::Raw => (
            Block::Raw {
                note: token.to_string(),
            },
            Expect::Exactly(1),
        ),
        Shape::Footnote => (
            Block::Footnote {
                id: NoteId(idx),
                blocks: vec![Block::Para(text(token))],
            },
            Expect::Exactly(1),
        ),
    }
}

/// A generated case: document, options, assets, and the sentinel ledger.
pub fn case() -> impl Strategy<Value = Case> {
    // Each entry pairs a block shape with generated nested inline markup, so
    // nesting depth up to 3 is present throughout rather than only in flat runs.
    let shapes = proptest::collection::vec((shape(), inlines(3)), 1..40);
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

        for (i, (sh, deco)) in shapes.iter().enumerate() {
            let idx = i as u32;
            let token = format!("zq{:04}", idx);
            let (block, expect) = build(sh, deco, &token, idx);

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
            sentinels.push(Sentinel { token, expect });
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
pub fn case_with_links() -> impl Strategy<Value = Case> {
    (case(), any::<bool>()).prop_map(|(mut c, dangle)| {
        let heading_ids: Vec<BlockId> = c
            .doc
            .nodes
            .iter()
            .filter_map(|n| match &n.block {
                Block::Heading { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        let target = match (heading_ids.first(), dangle) {
            (Some(id), false) => *id,
            // No heading generated, or deliberately dangling: an id far past
            // anything the generator assigns.
            _ => BlockId(9_999),
        };
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
