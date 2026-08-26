//! Both census tiers at length 4.
//!
//! The length 1-3 census (`census.rs`) carries three allowlist files because
//! its answer is not zero. This file's text tier carries none, and cannot rot
//! into stale excuses, because it has no file to rot into. Its structural
//! tier carries three of its own -- see below.
//!
//! **Why length 4 specifically.** `2026-08-18-abutment-ledger-design.md` §2b.5
//! is that branch's most transferable finding: its structural counter read 0 in
//! every row of every table while text losses ran into the thousands, because
//! the census stops at length 3 and the losses lived at length >= 4. A guard at
//! 4 is the smallest one that would have spoken. Lengths 5 and 6 were swept
//! too (`2026-08-21-declined-run-rescan-design.md` §2.2, also zero) and are not
//! shipped: they cost minutes, not seconds.
//!
//! **What this does not cover.** The alphabet is the census's own 19 elements.
//! Zero here says nothing about text outside it, and the property tier
//! (`properties.rs`) remains the only guard there. That scope statement is
//! load-bearing: `census-inexpressible.txt` spent months asserting "Markdown
//! cannot express" when it meant "this writer does not express", and ~78% of
//! it was wrong (1,984 entries down to 433 on 2026-08-23).
//!
//! This line said "this alphabet cannot express" and "88%" until 2026-08-23.
//! Both came from a probe that measured what CommonMark can spell rather than
//! what this pipeline can emit; the alphabet was never the constraint, decision
//! order was, and the corrected figure is in `permanence_ceiling`'s doc in
//! `census.rs`. The verdict survived its reason.
//!
//! **This file carried a text tier only until 2026-08-23, and that was its own
//! gap.** `census.rs`'s structural tier stops at length 3, so no shipped gate
//! priced structure above it — which let a 135-shape structural regression
//! through at length 4 and 3,134 at length 5 on the delimiter-choice branch,
//! caught only because the family happened to have a length-3 member. The
//! structural tier below closes that gap;
//! `2026-08-23-delimiter-choice-ordering-design.md` §6.1 named it and
//! `2026-08-23-length-4-structural-tier-design.md` is its design. Lengths 5
//! and 6 remain unpriced for structure as well as text, for the same reason:
//! minutes, not seconds.

mod census_support;

use census_support::{
    blessing, classify_with, for_each_shape, permanence_ceiling, ratchet, text_is_clean, Structure,
};
use kasane_ir::Inline;
use kasane_writer::Ledger;
use std::collections::BTreeSet;

/// Every sequence of length 4 over the census alphabet, handed to `f` one at a
/// time.
///
/// A thin wrapper over `census_support::for_each_shape`, which is the census's
/// one carry loop. This function kept its own copy until lengths 5 and 6
/// needed the same loop; two copies in two files is the drift `census_support`
/// exists to prevent, and three would have been worse.
fn for_each_length_four_shape(mut f: impl FnMut(&[Inline])) {
    for_each_shape(4, |seq, _| f(seq));
}

/// The corpus size the odometer must visit.
///
/// `AGENTS.md`, the design spec (§2, §8) and `for_each_length_four_shape`'s
/// own doc all claim `19^4 = 130,321`. Nothing asserted that number before
/// this test existed: the two classifying tests below only notice a
/// truncated enumeration when the dropped shape happens to be `Corrupt` or
/// `Inexpressible` -- both sets would come out identical either way, and only
/// the total shape count ever moved, if the dropped shapes were `Clean`. The
/// very first shape the odometer visits is `Clean`, so a truncation that
/// drops a prefix or a suffix of `Clean` shapes is exactly the failure mode
/// those tests cannot see. This is a separate, minimal test rather than an
/// assertion folded into one of them, so a truncation reports under its own
/// name instead of surfacing as a confusing mismatch inside a classification
/// test.
#[test]
fn the_length_four_odometer_visits_every_shape() {
    let mut n = 0usize;
    for_each_length_four_shape(|_| n += 1);
    assert_eq!(
        n, 130_321,
        "for_each_length_four_shape visited {n} shapes, not 19^4 = 130,321. \
         That would silently pass the two classifying tests below if the \
         dropped shapes were all `Clean` -- which the first shape visited is \
         -- so this is the only gate on the corpus size itself."
    );
}

/// Every sequence of length 4 over the census alphabet round-trips its text.
///
/// The corrupt list is capped when reported so a regression prints a readable
/// failure instead of 130k lines.
#[test]
fn no_shape_of_length_four_loses_text() {
    let mut corrupt: Vec<String> = Vec::new();
    for_each_length_four_shape(|seq| {
        if !text_is_clean(seq, Ledger::LICENSED) {
            corrupt.push(format!("{seq:?}"));
        }
    });
    assert!(
        corrupt.is_empty(),
        "{} shape(s) of length 4 lose text; first 20:\n  {}",
        corrupt.len(),
        corrupt
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

const LEN4_STRUCTURE_ALLOWLIST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-len4-known-structure-corrupt.txt"
);

const LEN4_INEXPRESSIBLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-len4-inexpressible.txt"
);

/// The ceiling on `census-len4-inexpressible.txt`.
///
/// Same mechanism as the length-3 ceiling and same reason — see
/// `PERMANENT_CEILING`'s doc in `census.rs` for the history that built it, and
/// `2026-08-23-length-4-structural-tier-design.md` §3.3 for why ten thousand
/// mechanically-decided claims still deserve a visible number.
const LEN4_PERMANENT_CEILING: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-len4-permanent-count.txt"
);

const LEN4_INEXPRESSIBLE_HEADER: &str = "\
# Length-4 shapes whose structure THIS WRITER DOES NOT EXPRESS.
#
# Not `Markdown cannot express`. The length-3 file's header
# (census-inexpressible.txt) carries the full argument -- the two mechanisms,
# why alternating `*` and `_` closes neither of them here, and the measurement
# that destroyed the alphabet framing on 2026-08-23. This file is the same
# relation at length 4, computed by the same `classify_with`, and nothing about
# it is a separate claim.
#
# 10,153 entries, split by the two conditions `classify_with` files them under:
#
#   7,585  nest a same-class container directly -- `<em><em>x</em></em>` or
#          `<strong><strong>x</strong></strong>`, which collapse onto one
#          container wherever run and child must share a delimiter character.
#   2,568  do not, and are here on the other condition alone: a `<strong>`
#          whose sole child is an `<em>`, where `***x***` is the only
#          single-character run that could carry both levels and CommonMark's
#          tie-break always resolves it em-outermost.
#
# Every entry satisfies at least one and none satisfies neither. That is not a
# coincidence to be maintained but a property of `classify_with`, and
# `the_length_four_permanent_file_splits_by_its_two_permanence_conditions`
# asserts all three numbers.
#
# This is NOT the length-3 file's split. That one separates 428 flanking-wall
# entries from 5 deliberate condition-4 refusals -- a distinction by CAUSE that
# no grep over ten thousand entries can make. 375 entries here nest a `Strong`
# directly inside an `Emph`; how many of those are refusals rather than wall is
# UNMEASURED, and neither this header nor its test claims a number for it.
#
# COMPUTED, never hand-edited. A shape lands here only if it BOTH nests,
# directly, a same-class container or a `<strong>` whose sole child is an
# `<em>`, AND differs from the IR only by collapsing adjacent identical classes
# and dropping an emphasis directly inside a strong.
#
# THIS HEADER IS GENERATED. It is the `LEN4_INEXPRESSIBLE_HEADER` constant in
# census_len4.rs, written out ahead of the entries on every bless. The checker
# filters `#` lines, so a hand-edit here passes the gate and is then silently
# reverted by the next bless. Edit the constant and re-bless.
#
# Regenerate: mise run census-bless
";

/// Every sequence of length 4 over the census alphabet, classified.
///
/// Walks the same shapes as `no_shape_of_length_four_loses_text`, via
/// `for_each_length_four_shape`. Returns only the two non-`Clean` sets,
/// because those are the two the ratchet gates.
fn classify_every_length_four_shape() -> (BTreeSet<String>, BTreeSet<String>) {
    let mut corrupt = BTreeSet::new();
    let mut inexpressible = BTreeSet::new();
    for_each_length_four_shape(|seq| match classify_with(seq, Ledger::LICENSED) {
        Structure::Clean => {}
        Structure::Corrupt => {
            corrupt.insert(format!("{seq:?}"));
        }
        Structure::Inexpressible => {
            inexpressible.insert(format!("{seq:?}"));
        }
    });
    (corrupt, inexpressible)
}

/// The structural tier at length 4 — the gate that was missing.
///
/// `census.rs`'s structural tier stops at length 3 and the text tier above is
/// text-only, so until this test existed **no shipped gate priced structure
/// above length 3**. That was not theoretical: the delimiter-choice branch
/// moved 135 length-4 shapes and 3,134 length-5 shapes from `Inexpressible` to
/// `Corrupt` — a `<strong>` coming back as an `<em>`, text migrating across a
/// structural boundary — with every text tier and a 2.48M-shape length-5 text
/// sweep reading zero throughout, because the text was byte-identical either
/// way. It was caught only because that family happened to also have a
/// length-3 member (`2026-08-23-delimiter-choice-ordering-design.md` §1.1). A
/// family starting at length 4 would have shipped silently.
///
/// Runs only where the text tier already passes — `classify_with` returns
/// `Clean` when the text is corrupt, and per-character alignment presupposes
/// equal strings. The text tier above asserts that set is empty at this length,
/// so today every shape here is genuinely classified.
///
/// Unlike the text tier, this one carries allowlists, because its answer is not
/// zero: 41,443 corrupt and 10,153 permanent as of 2026-08-23. Those files are
/// ratchets, not acceptances — checked in both directions, so neither a
/// regression nor a stale excuse survives.
#[test]
fn inline_structure_survives_rendering_for_every_shape_of_length_four() {
    let (corrupt, inexpressible) = classify_every_length_four_shape();

    ratchet(
        LEN4_STRUCTURE_ALLOWLIST,
        &corrupt,
        "structurally corrupt",
        None,
    );
    ratchet(
        LEN4_INEXPRESSIBLE,
        &inexpressible,
        "inexpressible",
        Some(LEN4_INEXPRESSIBLE_HEADER),
    );

    // The permanence gate. Asserted after both ratchets so a shape that is
    // merely unlisted is reported by the specific error rather than by this
    // one, and so `inexpressible.len()` is the size the file actually has once
    // a passing run is done with it.
    let ceiling = permanence_ceiling(LEN4_PERMANENT_CEILING);
    let ceiling = if blessing() && inexpressible.len() < ceiling {
        std::fs::write(LEN4_PERMANENT_CEILING, format!("{}\n", inexpressible.len()))
            .expect("lowering the permanence ceiling");
        inexpressible.len()
    } else {
        ceiling
    };
    assert!(
        inexpressible.len() <= ceiling,
        "the length-4 permanent file would grow to {} entries, over its \
         ceiling of {ceiling}.\n\
         \n\
         {} shape(s) are newly claimed inexpressible. A bless cannot make that \
         claim for you: raise the number in {LEN4_PERMANENT_CEILING} to {} in \
         this same commit, so the claim appears in the diff and a reviewer \
         sees it.\n\
         \n\
         Before you do -- is it true? A shape is only permanent if NO writer \
         change can express it. `_*x*_` spells `<em><em>x</em></em>`, which the \
         length-3 file's header called unspellable until 2026-08-17.",
        inexpressible.len(),
        inexpressible.len() - ceiling,
        inexpressible.len(),
    );
}

/// Formats a count the way the census headers do: thousands separated by
/// commas (`7585` -> `"7,585"`). Used to search [`LEN4_INEXPRESSIBLE_HEADER`]
/// for the constants below, so the header's prose and this file's constants
/// stay tied together instead of only mirroring each other by coincidence.
fn with_thousands(n: usize) -> String {
    let digits = n.to_string();
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The one claim in [`LEN4_INEXPRESSIBLE_HEADER`] that nothing else gates.
///
/// `permanence_ceiling` gates the 10,153 total. The sentence splitting it into
/// same-class nestings and strong-over-emph-only entries is hand-maintained
/// prose inside a *generated* file: `ratchet` filters `#` lines, so a hand-edit
/// there passes the checker and is silently reverted by the next bless. The
/// length-3 file has the same hazard and
/// `the_permanent_file_holds_exactly_the_five_condition_four_refusals` is its
/// answer; this is that test at length 4.
///
/// The split is `classify_with`'s own two permanence conditions —
/// `nests_same_class_directly` and `nests_strong_over_emph_directly` — and
/// **not** the length-3 file's split, which separates a flanking wall from five
/// deliberate condition-4 refusals. That is a distinction by cause, and no grep
/// over ten thousand entries can make it. This test claims nothing about it.
///
/// The `neither` assertion is the one that would catch a broken bless rather
/// than stale prose: every entry must satisfy at least one condition, because
/// `classify_with` files a shape here only if it does. It cannot fail while the
/// relation and the file agree, which is exactly why it is worth asserting.
///
/// The three string predicates below (`same_class`, `strong_over_emph`, and
/// the `Emph([Strong(` count) are a *proxy* for `classify_with`'s own
/// sole-child predicates, `nests_same_class_directly` and
/// `nests_strong_over_emph_directly` (`census_support/mod.rs`). The proxy is
/// exact only while every container in the census alphabet is single-child
/// and neither `Link` variant wraps emphasis -- true today, verified; widen
/// the alphabet and these string checks stop meaning what they say.
///
/// This test also ties [`LEN4_INEXPRESSIBLE_HEADER`]'s prose to the
/// `SAME_CLASS`/`STRONG_OVER_EMPH_ONLY` constants and to a third figure the
/// header states but nothing else gates: "375 entries here nest a `Strong`
/// directly inside an `Emph`" (an entry containing only `Emph([Strong(`
/// satisfies neither permanence condition, so it is counted inside whichever
/// of `n_same`/`n_soe_only` it also happens to land in and could migrate
/// between them while both totals hold steady). Without the containment
/// checks, editing the header's prose alone -- leaving the constants and the
/// file untouched -- leaves every assertion here green and ships a lying
/// file.
#[test]
fn the_length_four_permanent_file_splits_by_its_two_permanence_conditions() {
    const SAME_CLASS: usize = 7_585;
    const STRONG_OVER_EMPH_ONLY: usize = 2_568;
    const NESTED_STRONG_IN_EMPH: usize = 375;

    let body = std::fs::read_to_string(LEN4_INEXPRESSIBLE)
        .unwrap_or_else(|e| panic!("{LEN4_INEXPRESSIBLE} must exist and be readable: {e}"));
    let entries: Vec<&str> = body
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    let same_class = |l: &&str| l.contains("Emph([Emph(") || l.contains("Strong([Strong(");
    let strong_over_emph = |l: &&str| l.contains("Strong([Emph(");
    let nested_strong_in_emph = |l: &&str| l.contains("Emph([Strong(");

    let n_same = entries.iter().filter(|l| same_class(l)).count();
    let n_soe_only = entries
        .iter()
        .filter(|l| !same_class(l) && strong_over_emph(l))
        .count();
    let n_neither = entries
        .iter()
        .filter(|l| !same_class(l) && !strong_over_emph(l))
        .count();
    let n_nested_strong_in_emph = entries.iter().filter(|l| nested_strong_in_emph(l)).count();

    assert_eq!(
        n_neither, 0,
        "{n_neither} entr(ies) in {LEN4_INEXPRESSIBLE} satisfy neither \
         permanence condition. `classify_with` files a shape here only if it \
         nests a same-class container directly or a `<strong>` whose sole child \
         is an `<em>`, so this cannot happen while the relation and the file \
         agree -- the bless is broken, or the relation changed and the file was \
         not re-blessed."
    );
    assert_eq!(
        n_same, SAME_CLASS,
        "{LEN4_INEXPRESSIBLE} holds {n_same} entries that nest a same-class \
         container, but `LEN4_INEXPRESSIBLE_HEADER` says {SAME_CLASS}.\n\
         \n\
         That header is GENERATED from the constant in this file, and its split \
         is hand-written prose no bless recomputes. Update the constant's text \
         to match what the file now holds, re-bless, and update this test -- in \
         the same commit, so the claim and the entries move together."
    );
    assert_eq!(
        n_soe_only, STRONG_OVER_EMPH_ONLY,
        "{LEN4_INEXPRESSIBLE} holds {n_soe_only} entries that are here on the \
         strong-over-emph condition alone, but `LEN4_INEXPRESSIBLE_HEADER` says \
         {STRONG_OVER_EMPH_ONLY}. Update the constant's text, re-bless, and \
         update this test in the same commit."
    );
    assert_eq!(
        n_nested_strong_in_emph, NESTED_STRONG_IN_EMPH,
        "{LEN4_INEXPRESSIBLE} holds {n_nested_strong_in_emph} entries that nest \
         a `Strong` directly inside an `Emph` (`Emph([Strong(`), but \
         `LEN4_INEXPRESSIBLE_HEADER` says {NESTED_STRONG_IN_EMPH}. Unlike \
         `n_same`/`n_soe_only`, nothing else in this test gates that figure --\
         these entries satisfy neither permanence condition on their own, so \
         they can migrate between the same-class and strong-over-emph-only \
         counts while both totals hold steady and this one silently goes \
         stale. Update the header's prose, re-bless, and update this test in \
         the same commit."
    );

    // Ties the header's own prose to the constants above, in both directions:
    // editing the header's numbers without updating the constants fails here
    // even though nothing above reads the header string itself, and editing a
    // constant without re-blessing the file still fails the count assertions
    // above.
    let same_class_claim = format!(
        "{}  nest a same-class container directly",
        with_thousands(SAME_CLASS)
    );
    assert!(
        LEN4_INEXPRESSIBLE_HEADER.contains(&same_class_claim),
        "LEN4_INEXPRESSIBLE_HEADER does not contain {same_class_claim:?}. Its \
         prose and the SAME_CLASS constant have drifted apart -- update the \
         constant's text to match what the file now holds, re-bless, and \
         update this test in the same commit."
    );
    let soe_only_claim = format!(
        "{}  do not, and are here on the other condition alone",
        with_thousands(STRONG_OVER_EMPH_ONLY)
    );
    assert!(
        LEN4_INEXPRESSIBLE_HEADER.contains(&soe_only_claim),
        "LEN4_INEXPRESSIBLE_HEADER does not contain {soe_only_claim:?}. Its \
         prose and the STRONG_OVER_EMPH_ONLY constant have drifted apart -- \
         update the constant's text, re-bless, and update this test in the \
         same commit."
    );
    // The header wraps this sentence after `` `Strong` `` (the continuation
    // "directly inside an `Emph`" lands on the next line behind its own `#`),
    // so only the first clause is a contiguous substring of the constant.
    let nested_claim = format!("{NESTED_STRONG_IN_EMPH} entries here nest a `Strong`");
    assert!(
        LEN4_INEXPRESSIBLE_HEADER.contains(&nested_claim),
        "LEN4_INEXPRESSIBLE_HEADER does not contain {nested_claim:?}. Its \
         prose and the NESTED_STRONG_IN_EMPH constant have drifted apart -- \
         update the header's prose, re-bless, and update this test in the \
         same commit."
    );
}
