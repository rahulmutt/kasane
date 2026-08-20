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
//! There are two tiers, and three files. The text tier above compares what a
//! parser recovers against `kasane_gfm::rendered_text`. The **structural**
//! tier compares, for each character, the stack of emphasis containers
//! enclosing it on both sides, and runs only where the text tier already
//! passes. `census-known-structure-corrupt.txt` is its queue, target zero;
//! `census-inexpressible.txt` holds the shapes *this writer's `*`-only
//! alphabet* cannot express — not, as this line said until 2026-08-17, shapes
//! Markdown cannot express: `_*x*_` spells `<em><em>x</em></em>`. The split
//! between those two files is computed on every bless, never hand-edited, and
//! growth of the permanent file is gated by `census-permanent-count.txt`
//! (see `permanence_ceiling`).
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

mod census_support;

use census_support::{
    classify_with, context_text, context_walks_with, ir_context, parsed_text, render, shapes,
    Structure,
};
use kasane_ir::Inline;
use kasane_writer::Ledger;
use std::collections::BTreeSet;

const ALLOWLIST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-known-corrupt.txt"
);

/// The second of the three guards (design spec §3).
///
/// Where the text already matches, the two walks must produce the same
/// characters in the same order — that is what makes a positional comparison of
/// their stacks meaningful. This cannot fail by construction, which is exactly
/// why it is asserted: if it ever does, some character is reaching one walk and
/// not the other and every structural verdict is suspect.
#[test]
fn the_two_context_walks_align_character_for_character() {
    for seq in shapes() {
        let Some((ir, got)) = context_walks_with(&seq, Ledger::LICENSED) else {
            // Text already corrupt: named by the text assertion, and structure
            // is not evaluated here (design spec §2, "Gate").
            continue;
        };

        assert_eq!(
            context_text(&ir),
            context_text(&got),
            "the two walks disagree on characters for {seq:?}, so their \
             stacks cannot be compared positionally"
        );
    }
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
        let md = render(&seq, Ledger::LICENSED);
        let recovered = parsed_text(&md);
        let expected = kasane_gfm::rendered_text(&seq);
        if recovered.trim() != expected.trim() {
            corrupt.insert(format!("{seq:?}"));
        }
    }

    ratchet(ALLOWLIST, &corrupt, "corrupt", None);
}

/// The relation catches a class substitution — one of the two losses this tier
/// exists for.
///
/// `[Emph("a"), Strong("b")]` prints `*ab*`: the run fuse merges the `Strong`
/// into the `Em` run, and `b` comes back inside an `<em>` it was never in. The
/// text is byte-identical either way, which is why the text assertion cannot
/// see it.
#[test]
fn the_structural_relation_catches_a_class_substitution() {
    let seq = vec![
        Inline::Emph(vec![Inline::Text("a".into())]),
        Inline::Strong(vec![Inline::Text("b".into())]),
    ];
    assert_eq!(classify_with(&seq, Ledger::LICENSED), Structure::Corrupt);
}

/// The relation stays silent on intentional fusion.
///
/// `[Emph("a"), Emph("b")]` prints `*ab*` too — one `<em>` over both — but
/// every character keeps the class it had, so nothing was lost. Adjacent-run
/// fusion is deliberate (`2026-08-15-adjacent-inline-fusion-design.md`); a
/// check that flagged it would be unusable, and would have buried the shape
/// above in thousands of false positives.
#[test]
fn the_structural_relation_ignores_intentional_run_fusion() {
    let seq = vec![
        Inline::Emph(vec![Inline::Text("a".into())]),
        Inline::Emph(vec![Inline::Text("b".into())]),
    ];
    assert_eq!(classify_with(&seq, Ledger::LICENSED), Structure::Clean);
}

/// The relation names the third state, not only the other two.
///
/// `<em><em>x</em></em>` has no CommonMark spelling — `**x**` is strong, not
/// nested emphasis — so this verdict is permanent rather than a queued
/// defect. Pinning it here keeps the third state honest if the bless path
/// ever breaks.
#[test]
fn the_structural_relation_marks_direct_same_class_nesting_inexpressible() {
    let seq = vec![Inline::Emph(vec![Inline::Emph(vec![Inline::Text(
        "a".into(),
    )])])];
    assert_eq!(
        classify_with(&seq, Ledger::LICENSED),
        Structure::Inexpressible
    );
}

/// The second permanent mechanism, and the one this item adds.
///
/// `<strong><em>a</em></strong>` has no `*`-only spelling: `***a***` is the
/// only run that could carry both levels, and CommonMark's tie-break always
/// resolves it em-outermost. Spelling it needs `**_a_**`, and alternating `*`
/// with `_` is rejected by three specs. Permanent, not queued.
#[test]
fn the_structural_relation_marks_strong_over_emph_inexpressible() {
    let seq = vec![Inline::Strong(vec![Inline::Emph(vec![Inline::Text(
        "a".into(),
    )])])];
    assert_eq!(
        classify_with(&seq, Ledger::LICENSED),
        Structure::Inexpressible
    );
}

/// The guard that matters most.
///
/// The converse shape *is* spellable — `***a***` — and is fixed in the writer
/// (`2026-08-16-cross-class-edge-splice-design.md` §3). This asserts it is
/// neither corrupt nor permanent, so it fails loudly if the fix regresses
/// *and* if condition 1 ever loses its direction and starts laundering the
/// fixed family into the permanent file.
#[test]
fn the_structural_relation_keeps_emph_over_strong_clean() {
    let seq = vec![Inline::Emph(vec![Inline::Strong(vec![Inline::Text(
        "a".into(),
    )])])];
    assert_eq!(classify_with(&seq, Ledger::LICENSED), Structure::Clean);
}

const STRUCTURE_ALLOWLIST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-known-structure-corrupt.txt"
);

const INEXPRESSIBLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-inexpressible.txt"
);

const PERMANENT_CEILING: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-permanent-count.txt"
);

/// Whether this run is regenerating the ratchet files rather than checking
/// them.
///
/// Spelled once and shared, because [`ratchet`] and [`permanence_ceiling`]
/// disagreeing about what a bless is would let one of them write while the
/// other asserts against the file it just changed.
fn blessing() -> bool {
    std::env::var_os("KASANE_CENSUS_BLESS").is_some()
}

/// The most entries `census-inexpressible.txt` may hold.
///
/// A **ceiling**, not a count: the permanent file shrinking is always an
/// improvement, so this is only ever compared as an upper bound and a shrink
/// needs no edit. A bless *lowers* it to match — safe, since lowering only
/// tightens the gate — and never raises it.
///
/// Raising it is a hand edit, and that asymmetry is the entire point. Moving a
/// shape into the permanent file asserts that *no writer change can ever fix
/// it*, which is the one claim in this census that nothing downstream
/// re-examines: the queue is worked down item by item, but permanence is read
/// as settled. `KASANE_CENSUS_BLESS=1` must therefore not be able to make that
/// claim on its own — it writes the three shape files and stops here, so a
/// growing permanent file leaves this test failing until a human raises the
/// number in the same commit. That is a deliberately visible one-line diff.
///
/// The gate exists because the claim went wrong at scale once already. A probe
/// on 2026-08-17 searched every `*`/`_` spelling of each shape in this file and
/// found 1,740 of 1,984 expressible — the file was 88% wrong, and 748 of those
/// entries had been moved in by a single bless
/// (`2026-08-16-cross-class-edge-splice-design.md` §4).
fn permanence_ceiling() -> usize {
    let raw = std::fs::read_to_string(PERMANENT_CEILING)
        .unwrap_or_else(|e| panic!("{PERMANENT_CEILING} must exist and be readable: {e}"));
    raw.trim()
        .parse()
        .unwrap_or_else(|e| panic!("{PERMANENT_CEILING} must hold a single integer: {e}"))
}

/// Bless or check one ratchet file, two-directionally: a shape that is in
/// `found` but not the file fails, and a shape in the file but not `found`
/// fails too, so the file can neither grow silently nor rot into stale
/// excuses.
///
/// `#`-prefixed lines are comments, which is how the permanent file carries its
/// header.
fn ratchet(path: &str, found: &BTreeSet<String>, noun: &str, header: Option<&str>) {
    if blessing() {
        let mut body = header.unwrap_or("").to_string();
        body.extend(found.iter().map(|l| format!("{l}\n")));
        std::fs::write(path, body).expect("writing the allowlist");
        return;
    }

    let known: BTreeSet<String> = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("{path} must exist -- bless it with KASANE_CENSUS_BLESS=1"))
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();

    let new: Vec<&String> = found.difference(&known).collect();
    let gone: Vec<&String> = known.difference(found).collect();

    assert!(
        new.is_empty(),
        "{} shape(s) newly {noun} -- bless them into {path} \
         (KASANE_CENSUS_BLESS=1 does it for you):\n{}",
        new.len(),
        new.iter()
            .take(10)
            .map(|s| format!("  {s}\n"))
            .collect::<String>()
    );
    assert!(
        gone.is_empty(),
        "{} listed shape(s) are no longer {noun} -- delete them from {path} \
         (KASANE_CENSUS_BLESS=1 does it for you):\n{}",
        gone.len(),
        gone.iter()
            .take(10)
            .map(|s| format!("  {s}\n"))
            .collect::<String>()
    );
}

const INEXPRESSIBLE_HEADER: &str = "\
# Shapes whose structure THIS WRITER'S ALPHABET cannot express.
#
# Not `Markdown cannot express`, which is what this line claimed until
# 2026-08-17 and is false. Every mechanism below is forced by spelling emphasis
# with `*` alone, and CommonMark also has `_`. Alternating the two spells all
# three of these:
#
#   `_*x*_`     is `<em><em>x</em></em>`
#   `__**x**__` is `<strong><strong>x</strong></strong>`
#   `__*x*__`   is `<strong><em>x</em></strong>`
#
# A probe over every `*`/`_` spelling of every shape in this file found 1,740 of
# 1,984 expressible -- so read this file as the queue for the item that widens
# the alphabet, not as a statement about Markdown. What is genuinely
# unspellable is narrower and has a different cause: CommonMark's left-flanking
# rule, which stops any delimiter opening between a letter and punctuation, so
# `aa*` + a code span + `*` cannot emphasize at all. `census-permanent-count.txt`
# gates growth here for exactly this reason.
#
# Two mechanisms, both forced by spelling emphasis with `*` alone:
#
#   same-class nesting            -- a container whose sole child is the same
#                                     class collapses: `<em><em>x</em></em>`
#                                     prints `*x*`, not nested emphasis, and
#                                     `<strong><strong>x</strong></strong>`
#                                     prints `**x**`, not doubly strong --
#                                     `****x****` is never what the writer
#                                     prints, because the nested container is
#                                     merged away before printing.
#   `<strong><em>x</em></strong>` -- `***x***` is the only run that could carry
#                                    both levels, and CommonMark's tie-break
#                                    always resolves it em-outermost.
#
# The converse of the second, `<em><strong>x</strong></em>`, IS spellable and
# is not here -- the writer prints `***x***` for it. That asymmetry is what
# keeps a regression of the fixed family out of this file.
#
# No writer change can close these WITHOUT WIDENING THE ALPHABET, which is why
# they are not in the queue (`census-known-structure-corrupt.txt`) -- the queue
# is what the current alphabet can still reach.
#
# COMPUTED, never hand-edited. A shape lands here only if it BOTH nests,
# directly, a same-class container or a `<strong>` whose sole child is an
# `<em>`, AND differs from the IR only by collapsing adjacent identical
# classes and dropping an emphasis directly inside a strong. Stop satisfying
# either and it moves back to the queue on the next bless. See
# `docs/superpowers/specs/2026-08-16-cross-class-edge-splice-design.md` §4.
#
# Regenerate: KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census
";

/// The structural tier: does the emphasis structure a parser recovers match the
/// structure the IR held?
///
/// Runs only where the text assertion already passes — structure is meaningless
/// where the text is scrambled, and per-character alignment presupposes equal
/// strings. As the text allowlist drains, its shapes graduate into this check
/// without anyone editing this test.
#[test]
fn inline_structure_survives_rendering_for_every_short_sequence() {
    let mut corrupt = BTreeSet::new();
    let mut inexpressible = BTreeSet::new();

    for seq in shapes() {
        match classify_with(&seq, Ledger::LICENSED) {
            Structure::Clean => {}
            Structure::Corrupt => {
                corrupt.insert(format!("{seq:?}"));
            }
            Structure::Inexpressible => {
                inexpressible.insert(format!("{seq:?}"));
            }
        }
    }

    ratchet(STRUCTURE_ALLOWLIST, &corrupt, "structurally corrupt", None);
    ratchet(
        INEXPRESSIBLE,
        &inexpressible,
        "inexpressible",
        Some(INEXPRESSIBLE_HEADER),
    );

    // The permanence gate (see `permanence_ceiling`). Asserted after both
    // ratchets so a shape that is merely unlisted is reported by the specific
    // error rather than by this one, and so `inexpressible.len()` is the size
    // the file actually has once a passing run is done with it.
    let ceiling = permanence_ceiling();
    let ceiling = if blessing() && inexpressible.len() < ceiling {
        std::fs::write(PERMANENT_CEILING, format!("{}\n", inexpressible.len()))
            .expect("lowering the permanence ceiling");
        inexpressible.len()
    } else {
        ceiling
    };
    assert!(
        inexpressible.len() <= ceiling,
        "the permanent file would grow to {} entries, over its ceiling of {ceiling}.\n\
         \n\
         {} shape(s) are newly claimed inexpressible. A bless cannot make that \
         claim for you: raise the number in {PERMANENT_CEILING} to {} in this \
         same commit, so the claim appears in the diff and a reviewer sees it.\n\
         \n\
         Before you do -- is it true? A shape is only permanent if NO writer \
         change can express it. `_*x*_` spells `<em><em>x</em></em>`, which this \
         file's header called unspellable until 2026-08-17.",
        inexpressible.len(),
        inexpressible.len() - ceiling,
        inexpressible.len(),
    );
}
