//! An exhaustive differential census of short inline sequences.
//!
//! Renders every sequence of length 1-3 over the alphabet below, parses the
//! result, and compares the recovered text against `kasane_gfm::rendered_text`
//! of the same inlines — the same equality `p13_inline_text_survives_rendering`
//! asserts, exhaustive instead of sampled.
//!
//! This is the instrument that found the emphasis-seam defects
//! (`2026-08-15-emphasis-seam-design.md` §1). Six whole-pipeline properties and
//! three review rounds missed shapes it finds in one pass, because a property
//! draws from an alphabet someone chose and a census draws from all of it.
//!
//! # The allowlist is a ratchet, not an acceptance
//!
//! `census-known-corrupt.txt` names the shapes that are corrupt today. A
//! corrupt shape *absent* from it fails, so a regression cannot ship quietly. A
//! listed shape that is *no longer* corrupt also fails, so the file cannot rot
//! into a set of stale excuses — fixing a family means deleting lines from it.
//!
//! Regenerate with `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test
//! census`, and read the diff: it is the exact list of shapes your change
//! fixed or broke, which is the evidence a reviewer wants.
//!
//! # Why this alphabet
//!
//! Nineteen elements, chosen to put every delimiter class next to every other:
//! plain text, text that is itself a delimiter character, both code-span
//! classes (`Code`, and a `Math` that degrades to backticks), a `Math` that
//! does not degrade, each emphasis class alone and wrapping each of the
//! others, a transparent link both empty and delimiter-bearing, and a footnote
//! reference. `Inline::Code("")` is excluded: `code_span`'s Rule 1 prints
//! `` ` ` `` against `rendered_text`'s empty string, an acknowledged
//! divergence unreachable after `structure` and not what this census is about
//! — the same exclusion `P13_WORDS` documents.

use kasane_ir::{AssetBag, Block, BlockId, Inline, NoteId, RefTarget};
use pulldown_cmark::{Event, Options, Parser};
use std::collections::BTreeSet;

const ALLOWLIST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-known-corrupt.txt"
);

fn alphabet() -> Vec<Inline> {
    let t = |s: &str| Inline::Text(s.to_string());
    let em = |i: Inline| Inline::Emph(vec![i]);
    let st = |i: Inline| Inline::Strong(vec![i]);
    vec![
        t("a"),
        t("b"),
        t(" "),
        t("*"),
        Inline::Code("x".into()),
        Inline::Code("y".into()),
        Inline::Math("m".into()),
        Inline::Math("a$b".into()),
        em(t("a")),
        st(t("a")),
        em(Inline::Code("x".into())),
        st(Inline::Code("x".into())),
        em(em(t("a"))),
        st(em(t("a"))),
        em(st(t("a"))),
        st(st(t("a"))),
        Inline::Link {
            target: RefTarget::Internal(BlockId(0)),
            inlines: vec![Inline::Code("x".into())],
        },
        Inline::Link {
            target: RefTarget::Internal(BlockId(0)),
            inlines: vec![],
        },
        Inline::FootnoteRef(NoteId(1)),
    ]
}

/// The text a real parser recovers from `md`.
fn parsed_text(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_MATH);
    let mut out = String::new();
    for ev in Parser::new_ext(md, opts) {
        match ev {
            Event::Text(t) | Event::Code(t) | Event::InlineMath(t) | Event::DisplayMath(t) => {
                out.push_str(&t)
            }
            _ => {}
        }
    }
    out
}

/// Every sequence of length 1-3 over the alphabet.
fn shapes() -> Vec<Vec<Inline>> {
    let a = alphabet();
    let mut out: Vec<Vec<Inline>> = a.iter().map(|i| vec![i.clone()]).collect();
    for i in &a {
        for j in &a {
            out.push(vec![i.clone(), j.clone()]);
            for k in &a {
                out.push(vec![i.clone(), j.clone(), k.clone()]);
            }
        }
    }
    out
}

/// One emphasis container, as it appears on the stack enclosing a character.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Emphasis {
    Em,
    St,
}

/// Every character `rendered_text` contributes, paired with the stack of
/// emphasis containers enclosing it.
///
/// Mirrors `kasane_gfm::rendered_text`'s own walk arm for arm — its
/// `MAX_INLINE_DEPTH` cutoff, and its `[^n]` spelling for a footnote reference
/// — because the whole comparison is meaningless if this walks a different
/// projection from the one the text assertion uses. That mirroring is not
/// asserted in prose: `the_context_walk_reproduces_rendered_text_for_every_short_sequence`
/// re-derives `rendered_text` from this walk's own output every run.
///
/// `Link` pushes nothing. `flatten_into` (`markdown.rs:237-238`) splices every
/// non-`External` target away before the emit loop ever sees it, so a
/// transparent link is not a structural level in the output and must not be
/// one here (design spec §2).
fn ir_context(
    inlines: &[Inline],
    depth: usize,
    stack: &mut Vec<Emphasis>,
    out: &mut Vec<(char, Vec<Emphasis>)>,
) {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return;
    }
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => {
                for c in t.chars() {
                    out.push((c, stack.clone()));
                }
            }
            Inline::Emph(x) => {
                stack.push(Emphasis::Em);
                ir_context(x, depth + 1, stack, out);
                stack.pop();
            }
            Inline::Strong(x) => {
                stack.push(Emphasis::St);
                ir_context(x, depth + 1, stack, out);
                stack.pop();
            }
            Inline::Link { inlines, .. } => ir_context(inlines, depth + 1, stack, out),
            Inline::FootnoteRef(n) => {
                for c in format!("[^{}]", n.0).chars() {
                    out.push((c, stack.clone()));
                }
            }
        }
    }
}

/// The characters of a context walk, in order.
fn context_text(v: &[(char, Vec<Emphasis>)]) -> String {
    v.iter().map(|(c, _)| *c).collect()
}

/// The first of the three guards (design spec §3): a hard failure, not a skip.
///
/// If this fires, the instrument is broken rather than the writer — someone has
/// edited `ir_context` or `rendered_text` without the other, and every
/// structural verdict downstream is being computed against the wrong
/// projection.
#[test]
fn the_context_walk_reproduces_rendered_text_for_every_short_sequence() {
    for seq in shapes() {
        let mut ctx = Vec::new();
        ir_context(&seq, 0, &mut Vec::new(), &mut ctx);
        assert_eq!(
            context_text(&ctx),
            kasane_gfm::rendered_text(&seq),
            "the context walk has drifted from `rendered_text` on {seq:?}"
        );
    }
}

#[test]
fn inline_text_survives_rendering_for_every_short_sequence() {
    let mut corrupt = BTreeSet::new();
    for seq in shapes() {
        let md =
            kasane_writer::blocks_to_markdown(&[Block::Para(seq.clone())], &AssetBag::default());
        let recovered = parsed_text(&md);
        let expected = kasane_gfm::rendered_text(&seq);
        if recovered.trim() != expected.trim() {
            corrupt.insert(format!("{seq:?}"));
        }
    }

    if std::env::var_os("KASANE_CENSUS_BLESS").is_some() {
        let body: String = corrupt.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(ALLOWLIST, body).expect("writing the allowlist");
        return;
    }

    let known: BTreeSet<String> = std::fs::read_to_string(ALLOWLIST)
        .expect("tests/census-known-corrupt.txt must exist -- bless it with KASANE_CENSUS_BLESS=1")
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();

    let new: Vec<&String> = corrupt.difference(&known).collect();
    let fixed: Vec<&String> = known.difference(&corrupt).collect();

    assert!(
        new.is_empty(),
        "{} shape(s) newly corrupt. Each of these renders to text a parser \
         reads differently from `rendered_text`:\n{}",
        new.len(),
        new.iter()
            .take(10)
            .map(|s| format!("  {s}\n"))
            .collect::<String>()
    );
    assert!(
        fixed.is_empty(),
        "{} allowlisted shape(s) are no longer corrupt -- delete them from \
         tests/census-known-corrupt.txt (KASANE_CENSUS_BLESS=1 does it for \
         you):\n{}",
        fixed.len(),
        fixed
            .iter()
            .take(10)
            .map(|s| format!("  {s}\n"))
            .collect::<String>()
    );
}
