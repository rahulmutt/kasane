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
    /// Whether this sentinel was stamped into a `Block::Raw` note *and*
    /// `escape::comment_note` actually alters it -- a `--` run anywhere, or
    /// a trailing `-`, mirroring `comment_note`'s own two triggers exactly.
    ///
    /// Not "every `Block::Raw` payload": `comment_note` is design spec §5's
    /// one documented exception to "escaping must never change what the
    /// Markdown renders to" (an HTML comment has no escape mechanism for a
    /// `-->` run, so it transforms rather than escapes), but that exception
    /// is narrow -- of `HOSTILE`'s 26 fragments, only `-->` triggers it; the
    /// other 25 round-trip through a comment verbatim like anything else.
    /// Scoping the skip to the one transformation, not the whole shape,
    /// keeps P7 checking everything `comment_note` does not have to touch,
    /// and would fail loudly if `comment_note` ever grew a transformation
    /// this predicate doesn't know about.
    pub is_comment: bool,
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
/// parenthesis characters are deliberately absent -- not because they are
/// unsafe (`links_in` is a real parser now, and `[bracket]`/`]close` are
/// already in `HOSTILE`, drawn into this same filler text), but so `WORDS`
/// stays "boring": when a shrunk case fails, a bracket or paren in it
/// unambiguously came from the deliberately hostile channel, not from the
/// filler pool meant to be free of anything under test.
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
    // A trailing `#` run preceded by a space: an ATX *closing* sequence, which
    // the parser strips from a heading before inline parsing unless the writer
    // disarms it (2026-08-14 spec §4.2). `"#hash"` above is the line-START
    // case and does not reach this one.
    "tail ###",
    "- bullet",
    "1. ordered",
    "> quote",
    "= setext",
    "line\nbreak",
    "!bang[",
    // A newline RUN, in both spellings. These are the two fragments P2 needed
    // to catch the heading fold disagreeing with `anchor_slug`: the body
    // heading path collapsed a blank line to one space (`## a b`, GitHub id
    // `a-b`) while `anchor_fold` did not (`#a--b`), so the emitted
    // cross-reference was dead. P2 recomputes the anchor from parsed heading
    // text, so it fails on exactly this shape -- but only if the shape is
    // drawn.
    "a\n\nb",
    "a\r\nb",
    // A run that *ends* one inline and one that *begins* the next. Together
    // they let the main tier draw the boundary shape P9 covers directly
    // (residuals spec §5.3) -- which matters for shrinking, not for coverage:
    // the payload must end with the first fragment (1 in 26), the decoration
    // must be an `Inline::Code` (1 in 13), and that code's filler must draw
    // the second as its first word (1 in 100). About 1 in 32,500 per shape,
    // so roughly one default run in seven sees it at all. P9 and the unit
    // tests are what hold this line; these fragments only make a hit useful.
    "trailing\r",
    "\nleading",
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

/// An external `href`, hostile in the ways a real one is: a space and a `)`
/// end a bare destination, a `%` must survive `dest_url` unencoded, and a
/// fragment and a query must stay literal.
///
/// P2 skips a destination starting with `http`, so these do not have to
/// resolve to a file in the tree -- what they exercise is `escape::dest_url`
/// on a real `href` and the `[label](dest)` composition around it, neither of
/// which had any property-tier coverage while the generator emitted no
/// author-supplied `RefTarget::External` at all (design spec §6.4).
const HREFS: &[&str] = &[
    "https://e.com/a b(1)",
    "https://e.com/a%20b",
    "https://e.com/p?q=1#f",
    "https://e.com/x<y>\"z\"",
];

/// One inline run, nested at most `depth` levels. Boxed at every level because
/// the strategy is recursive and its type would otherwise be infinite. The leaf
/// is built twice rather than cloned, so this compiles without requiring the
/// mapped strategy to be `Clone`.
///
/// `Inline::Code` and an external `Inline::Link` are drawn here rather than
/// only as block shapes, because that is where the writer composes them:
/// `escape::code_span`, `escape::dest_url` on an author-supplied `href`, and
/// `inlines_to_html`'s `<a href>` in the merged-table path all sit on this
/// walk and had no property-tier coverage until they did (design spec §6.4).
fn inlines(depth: u32) -> BoxedStrategy<Vec<Inline>> {
    if depth == 0 {
        return filler().prop_map(|s| vec![Inline::Text(s)]).boxed();
    }
    prop_oneof![
        8 => filler().prop_map(|s| vec![Inline::Text(s)]),
        1 => inlines(depth - 1).prop_map(|x| vec![Inline::Emph(x)]),
        1 => inlines(depth - 1).prop_map(|x| vec![Inline::Strong(x)]),
        1 => filler().prop_map(|s| vec![Inline::Code(s)]),
        // `pptx/slide.rs` pushes `Inline::Math` into a table cell and into a
        // paragraph, so this is an adapter-realistic inline, not only a
        // hand-built one -- and it is the only way the tier reaches
        // `escape::math_span`, whose `Ctx::Cell` pipe rule is what a real PPTX
        // equation needs. `Block::MathBlock` covers the display form; nothing
        // covered the inline form until this draw.
        1 => filler().prop_map(|s| vec![Inline::Math(s)]),
        1 => (filler(), proptest::sample::select(HREFS)).prop_map(|(s, u)| vec![Inline::Link {
            target: RefTarget::External(u.to_string()),
            inlines: vec![Inline::Text(s)],
        }]),
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

/// Whether `escape::comment_note` would actually alter `s`: a `--` run
/// anywhere, or a trailing `-` -- the function's own two triggers, mirrored
/// exactly rather than approximated. Not a call into `kasane-writer`'s
/// `escape` module, which is private to that crate; a deliberate, narrow
/// mirror, the same convention `math_safe` uses for `kasane-adapters`'s
/// `sanitize`. Used to scope `Sentinel::is_comment` to the one payload in
/// `HOSTILE` (`-->`) this actually applies to, rather than every
/// `Shape::Raw` draw.
fn comment_note_alters(s: &str) -> bool {
    s.contains("--") || s.ends_with('-')
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
            // `Shape::Math` draws the raw fragment like every other shape.
            // It used to get a `math_safe` draw that pre-neutralized `$`,
            // modelling the adapter's math contract rather than testing it --
            // and the contract had a hole exactly there (`MathNode::Ident`/`Op`
            // reached the writer unneutralized, which is all of PowerPoint's
            // equation text). With the contract closed in `kasane-adapters`,
            // the generator asserts it instead of assuming it.
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
            let is_comment = matches!(sh, Shape::Raw) && comment_note_alters(&payload);
            sentinels.push(Sentinel {
                token,
                payload,
                expect,
                is_comment,
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
