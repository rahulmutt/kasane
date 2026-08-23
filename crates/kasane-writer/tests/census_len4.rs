//! The text tier at length 4, asserting zero.
//!
//! The length 1-3 census (`census.rs`) carries three allowlist files because
//! its answer is not zero. This one carries none, and cannot rot into stale
//! excuses, because it has no file to rot into.
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
    alphabet, blessing, classify_with, permanence_ceiling, ratchet, text_is_clean, Structure,
};
use kasane_ir::Inline;
use kasane_writer::Ledger;
use std::collections::BTreeSet;

/// Every sequence of length 4 over the census alphabet round-trips its text.
///
/// Shapes are built by odometer rather than by `shapes()`, which is fixed at
/// lengths 1-3. The corrupt list is capped when reported so a regression prints
/// a readable failure instead of 130k lines.
#[test]
fn no_shape_of_length_four_loses_text() {
    let a = alphabet();
    let n = a.len();
    let mut corrupt: Vec<String> = Vec::new();
    let mut idx = [0usize; 4];
    loop {
        let seq: Vec<Inline> = idx.iter().map(|&k| a[k].clone()).collect();
        if !text_is_clean(&seq, Ledger::LICENSED) {
            corrupt.push(format!("{seq:?}"));
        }
        let mut k = 4;
        loop {
            if k == 0 {
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
                return;
            }
            k -= 1;
            idx[k] += 1;
            if idx[k] < n {
                break;
            }
            idx[k] = 0;
        }
    }
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
/// Built by odometer rather than by `shapes()`, which is fixed at lengths 1-3,
/// and streamed rather than materialized: a `Vec` of 130,321 shapes held at
/// once is a cost the odometer does not pay. Returns only the two non-`Clean`
/// sets, because those are the two the ratchet gates.
fn classify_every_length_four_shape() -> (BTreeSet<String>, BTreeSet<String>) {
    let a = alphabet();
    let n = a.len();
    let mut corrupt = BTreeSet::new();
    let mut inexpressible = BTreeSet::new();
    let mut idx = [0usize; 4];
    loop {
        let seq: Vec<Inline> = idx.iter().map(|&k| a[k].clone()).collect();
        match classify_with(&seq, Ledger::LICENSED) {
            Structure::Clean => {}
            Structure::Corrupt => {
                corrupt.insert(format!("{seq:?}"));
            }
            Structure::Inexpressible => {
                inexpressible.insert(format!("{seq:?}"));
            }
        }
        let mut k = 4;
        loop {
            if k == 0 {
                return (corrupt, inexpressible);
            }
            k -= 1;
            idx[k] += 1;
            if idx[k] < n {
                break;
            }
            idx[k] = 0;
        }
    }
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
