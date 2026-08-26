//! Both census tiers at length 6, plus the novelty check. **No files.**
//!
//! Read `census_len5.rs`'s module doc first: the argument for asserting zero
//! rather than committing a per-shape allowlist is the same one, and it is
//! written out there.
//!
//! **Why this tier commits nothing at all, not even counts.** Counts go stale
//! on every writer improvement. This tier's venue is a weekly workflow
//! (`.github/workflows/census-deep.yml`) and its walk costs ~10 min, so its
//! counts would be found stale ON MAIN, up to a week late -- and a weekly job
//! that is routinely red stops being read, which is worse than no counts.
//! Zero-assertions do not have that property: zero stays zero under
//! improvement, so this tier either passes or has found something real
//! (design spec §3.2).
//!
//! That also disposes of a wrinkle. `mise run census-ratchet` resolves its base
//! to HEAD on a push to main, so its git-diff half is inert there. With no
//! length-6 files there is nothing for it to compare, and this tier's guard is
//! entirely the assertion below -- which works identically on main and on a
//! branch.
//!
//! **What this leaves uncovered, stated rather than hidden.** A change that
//! multiplies corruption inside families already known at length 4 moves no
//! novel shape and is not caught here. At length 5 the counts cover that; here
//! it is the price of a guard with nothing to go stale (design spec §3.3).
//!
//! **The weekly cadence** means a novel-at-6 regression surfaces on main up to
//! a week late. That is the bargain `.github/workflows/fuzz.yml` already makes
//! in this repo, and it is written here so nobody reads this tier as a PR gate.
//!
//! Run it: `mise run census-len6`. ~10 min.

mod census_support;

use census_support::{deep_scan, nonclean_bitset};
use kasane_writer::Ledger;

/// 19^6.
const LEN6_SHAPES: usize = 47_045_881;

/// The corpus size the odometer must visit.
#[test]
#[ignore = "release-only deep tier, ~10 min: run with `mise run census-len6`"]
fn the_length_six_odometer_visits_every_shape() {
    let mut n = 0usize;
    census_support::for_each_shape(6, |_, _| n += 1);
    assert_eq!(n, LEN6_SHAPES);
}

/// Both length-6 tiers and the novelty check, from one walk.
///
/// The union is **reported, not asserted**: 26,501,436 as of 2026-08-25, and
/// deliberately not pinned to a number, because pinning it is the counts file
/// this tier refuses to have. It is printed so a human reading a weekly run
/// sees the tier did real work rather than passing vacuously.
#[test]
#[ignore = "release-only deep tier, ~10 min: run with `mise run census-len6`"]
fn the_length_six_census() {
    let shorter = nonclean_bitset(5, Ledger::LICENSED);
    let scan = deep_scan(6, &shorter, Ledger::LICENSED);

    println!(
        "length 6: {} shapes, union {} (queue {}, permanent {})",
        LEN6_SHAPES, scan.counts.union, scan.counts.queue, scan.counts.permanent
    );

    assert!(
        scan.text_corrupt.is_empty(),
        "shape(s) of length 6 lose text. This tier has no allowlist and will \
         not be given one: text is the census's invariant.\n  {}",
        scan.text_corrupt.join("\n  ")
    );

    assert!(
        scan.novel.is_empty(),
        "shape(s) of length 6 are corrupt for a reason NO shorter shape shows.\n\
         \n\
         This is a design event, not a bless, and there is deliberately no file \
         to put them in -- see census_len5.rs's failure text and design spec \
         §3.1. The census has found a corruption family that originates above \
         length 5, which it has never seen.\n  {}",
        scan.novel.join("\n  ")
    );
}
