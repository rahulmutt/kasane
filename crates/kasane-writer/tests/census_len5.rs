//! Both census tiers at length 5, plus the novelty check that replaces a file.
//!
//! **Why this tier commits no shape files.** Every non-clean shape at length 5
//! has a non-clean single-deletion sub-shape -- all 1,204,312 of them -- so a
//! per-shape allowlist here would be a 112 MB index of `census_len4.rs` rather
//! than evidence about length 5, rewritten whole on every bless.
//! `2026-08-23-length-4-structural-tier-design.md` §3.1 rejects count-only
//! gates because they are blind to a swap; that argument holds wherever the
//! set's members carry information, and at this length none of them do. The
//! file is rejected on what it would record, not on its size
//! (`2026-08-26-length-5-6-novelty-tier-design.md` §2.2).
//!
//! What replaces it is `no_length_five_shape_is_corrupt_for_a_novel_reason`
//! below -- zero, with no file, on `census_len4.rs`'s text-tier contract: the
//! answer is zero and a file it does not have cannot rot into stale excuses --
//! plus three counts covering the one gap novelty leaves, a change that
//! multiplies corruption inside families already known at length 4.
//!
//! **Why `#[ignore]` and not `test = false`.** These tests need the release
//! profile: length 5 is ~35 s in release and ~3.3 min in debug, and
//! `mise run test` builds debug. Excluding the target in `Cargo.toml` with
//! `test = false` was measured and rejected -- a target so marked is invisible
//! to `cargo clippy --all-targets` too, so a file containing nothing but
//! `this is not valid rust at all !!!` passes both gates with exit 0, and a
//! tier that stopped compiling would stay green until someone ran the deep
//! task by hand. `#[ignore]` keeps this target compiled, linted, and visible
//! as `ignored` in every test run.
//!
//! Run it: `mise run census-len5`.

mod census_support;

use census_support::{counts_ratchet, deep_scan, nonclean_bitset, Counts};
use kasane_writer::Ledger;

/// 19^5.
const LEN5_SHAPES: usize = 2_476_099;

const LEN5_COUNTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/census-len5-counts.txt");

const LEN5_COUNTS_HEADER: &str = "\
# The length-5 census, as three counts.
#
# NOT a per-shape file, and that is a measurement rather than a concession to
# size. Every one of the 1,204,312 non-clean shapes at this length has a
# non-clean single-deletion sub-shape, so a per-shape allowlist here would be a
# 112 MB index of census-len4-known-structure-corrupt.txt -- it would assert
# nothing the length-4 tier does not already assert shape by shape. See
# docs/superpowers/specs/2026-08-26-length-5-6-novelty-tier-design.md §2.2.
#
#   queue      structurally corrupt: a real, fixable loss.
#   permanent  this writer does not express it at any level.
#   union      queue + permanent. THE GATED NUMBER.
#
# `mise run census-ratchet` gates `union` alone and reports the other two, which
# is the length-3/4 logic reproduced: `permanent` growing while `union` is flat
# IS queue -> permanent movement, and `permanent` growing while `union` grows
# fails on `union`. So the permanence ceiling has no length-5 form -- there is
# no set difference to compute `newly permanent` from, and nothing asks for one.
#
# Every number here moving DOWN is normal on a change that improves the writer.
# `union` moving UP is a shape becoming corrupt that was not.
#
# COMPUTED, never hand-edited. Regenerate: mise run census-bless
";

/// The corpus size the odometer must visit.
///
/// Separate from the tiers below for the reason
/// `the_length_four_odometer_visits_every_shape` gives: a truncated enumeration
/// is invisible to the classifying tests whenever the dropped shapes are
/// `Clean`, and the first shape the odometer visits is `Clean`.
#[test]
#[ignore = "release-only deep tier: run with `mise run census-len5`"]
fn the_length_five_odometer_visits_every_shape() {
    let mut n = 0usize;
    census_support::for_each_shape(5, |_, _| n += 1);
    assert_eq!(n, LEN5_SHAPES);
}

/// Both length-5 tiers and the novelty check, from one walk.
///
/// One test rather than three because `deep_scan` is one 35-second walk and
/// three tests would be three walks. The assertions are ordered so the most
/// fundamental failure speaks first: text before structure, because
/// `classify_with` returns `Clean` when the text is corrupt and a structural
/// number computed over corrupt text is meaningless.
#[test]
#[ignore = "release-only deep tier: run with `mise run census-len5`"]
fn the_length_five_census() {
    let shorter = nonclean_bitset(4, Ledger::LICENSED);
    let scan = deep_scan(5, &shorter, Ledger::LICENSED);

    assert!(
        scan.text_corrupt.is_empty(),
        "shape(s) of length 5 lose text. This tier has no allowlist and will \
         not be given one: text is the census's invariant.\n  {}",
        scan.text_corrupt.join("\n  ")
    );

    assert!(
        scan.novel.is_empty(),
        "shape(s) of length 5 are corrupt for a reason NO shorter shape shows.\n\
         \n\
         This is a design event, not a bless. There is deliberately no file to \
         put these in: design spec §2.2's case for this tier committing no \
         per-shape allowlist rests on this count being zero, and adding a file \
         here would be reinstating the 112 MB index that argument rejected.\n\
         \n\
         Read the spec's §3.1 before doing anything else. If these shapes are \
         genuinely unfixable, that is a new design item -- the census has \
         found a corruption family that originates above length 4, which it \
         has never seen.\n  {}",
        scan.novel.join("\n  ")
    );

    counts_ratchet(LEN5_COUNTS, scan.counts, LEN5_COUNTS_HEADER);
}

/// The header's own arithmetic.
///
/// `union` is stored rather than derived (see `Counts`), so nothing else checks
/// that it is the sum. A bless writes whatever `deep_scan` computed; this is
/// what catches a `deep_scan` that stopped adding them up.
#[test]
#[ignore = "release-only deep tier: run with `mise run census-len5`"]
fn the_length_five_union_is_the_sum_of_its_parts() {
    let shorter = nonclean_bitset(4, Ledger::LICENSED);
    let Counts {
        queue,
        permanent,
        union,
    } = deep_scan(5, &shorter, Ledger::LICENSED).counts;
    assert_eq!(union, queue + permanent);
}
