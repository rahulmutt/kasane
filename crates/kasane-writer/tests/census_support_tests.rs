//! Unit tests for `census_support`'s helpers.
//!
//! They live in a binary of their own because `census_support` is a shared
//! oracle included by every census tier -- five binaries after the length-5/6
//! item -- and a `#[test]` placed in it runs once per binary. That module has
//! carried none since it was created; this file is where its tests go instead.
//!
//! Everything here is cheap and runs in the default `mise run test` set. The
//! expensive tiers are `#[ignore]`d in `census_len5.rs` and `census_len6.rs`.

mod census_support;

use census_support::{
    alphabet, classify_with, counts_ratchet, deep_scan, for_each_shape, is_novel, nonclean_bitset,
    text_is_clean, Counts, NonClean, Structure, ALPHABET_LEN,
};
use kasane_writer::Ledger;

/// The shared odometer agrees with the length-4 one it replaced.
///
/// Not redundant with `the_length_four_odometer_visits_every_shape`, which
/// counts. This pins the *order and content*: a carry loop that visits the
/// right number of shapes in the wrong order would still let every
/// classifying test pass, because those tests build sets.
#[test]
fn the_shared_odometer_yields_shapes_in_base_19_digit_order() {
    let a = alphabet();
    let mut seen = 0usize;
    for_each_shape(4, |seq, idx| {
        assert_eq!(idx.len(), 4, "digit slice must have one digit per position");
        let expected: Vec<_> = idx.iter().map(|&k| a[k].clone()).collect();
        assert_eq!(seq, expected.as_slice(), "shape must match its digits");
        let value = idx.iter().fold(0usize, |acc, &d| acc * 19 + d);
        assert_eq!(value, seen, "shapes must arrive in ascending base-19 order");
        seen += 1;
    });
    assert_eq!(seen, 130_321);
}

/// The radix and the alphabet cannot drift apart.
///
/// `ALPHABET_LEN` is a constant because it is arithmetic, not a lookup, and
/// `pow19` would be a function call per digit otherwise. This is the price of
/// that: one test tying the constant to the thing it describes.
#[test]
fn alphabet_len_matches_the_radix() {
    assert_eq!(alphabet().len(), ALPHABET_LEN);
}

/// A bitset round-trips the bits set in it and counts them.
#[test]
fn nonclean_bitset_stores_and_counts_bits() {
    let mut b = NonClean::new(2);
    assert_eq!(b.count(), 0);
    assert!(!b.get(0));
    b.set(0);
    b.set(64); // across a word boundary -- the off-by-one that a 1-word test misses
    b.set(360); // 19^2 - 1, the last valid index
    assert!(b.get(0) && b.get(64) && b.get(360));
    assert!(!b.get(1) && !b.get(63) && !b.get(65));
    assert_eq!(b.count(), 3);
}

/// The bitset built from the writer agrees with a set built the obvious way.
///
/// Length 2 because it is small enough to hold both forms at once; the point
/// is the base-19 indexing, which does not vary with length.
#[test]
fn nonclean_bitset_agrees_with_a_direct_walk_at_length_two() {
    let bits = nonclean_bitset(2, Ledger::LICENSED);
    let mut direct = 0usize;
    for_each_shape(2, |seq, idx| {
        let value = idx.iter().fold(0usize, |acc, &d| acc * ALPHABET_LEN + d);
        let nonclean = classify_with(seq, Ledger::LICENSED) != Structure::Clean;
        assert_eq!(
            bits.get(value),
            nonclean,
            "disagreement at index {value}: {seq:?}"
        );
        if nonclean {
            direct += 1;
        }
    });
    assert_eq!(bits.count(), direct);
}

/// Index of the shape `idx` becomes when position `i` is deleted.
fn deletion_index(idx: &[usize], i: usize) -> usize {
    idx.iter()
        .enumerate()
        .filter(|(p, _)| *p != i)
        .fold(0usize, |acc, (_, &d)| acc * ALPHABET_LEN + d)
}

/// A shape whose every single-deletion sub-shape is clean is novel.
#[test]
fn a_shape_with_no_non_clean_deletion_is_novel() {
    let shorter = NonClean::new(4);
    assert!(is_novel(&[1, 2, 3, 4, 5], &shorter));
}

/// A shape with even one non-clean deletion is not novel.
///
/// The deletion is at the **interior** position 2, which is the case that
/// separates this predicate from a contiguous-substring one. Design spec §2.1:
/// of 1,204,312 non-clean length-5 shapes, all have a non-clean single-deletion
/// sub-shape but only 1,204,044 have a non-clean contiguous one. A substring
/// implementation passes every other test in this file and starts reporting
/// 268 novelties on a clean tree.
#[test]
fn an_interior_deletion_is_enough_to_make_a_shape_derivative() {
    let idx = [1usize, 2, 3, 4, 5];
    let mut shorter = NonClean::new(4);
    shorter.set(deletion_index(&idx, 2));
    assert!(!is_novel(&idx, &shorter));
}

/// Every position is consulted, not just the first or last.
///
/// A predicate that checked only position 0 -- or that broke out of its loop
/// before the end -- would pass both tests above for some inputs. This one
/// fails unless all five deletions are asked about.
#[test]
fn every_deletion_position_is_consulted() {
    let idx = [1usize, 2, 3, 4, 5];
    for i in 0..5 {
        let mut shorter = NonClean::new(4);
        shorter.set(deletion_index(&idx, i));
        assert!(
            !is_novel(&idx, &shorter),
            "a non-clean deletion at position {i} was not consulted"
        );
    }
}

/// `deep_scan` at length 2, where the numbers are checkable by a direct walk.
#[test]
fn deep_scan_agrees_with_direct_walks_at_length_two() {
    let shorter = nonclean_bitset(1, Ledger::LICENSED);
    let scan = deep_scan(2, &shorter, Ledger::LICENSED);

    let (mut queue, mut permanent, mut text_corrupt) = (0usize, 0usize, 0usize);
    for_each_shape(2, |seq, _| {
        if !text_is_clean(seq, Ledger::LICENSED) {
            text_corrupt += 1;
        }
        match classify_with(seq, Ledger::LICENSED) {
            Structure::Clean => {}
            Structure::Corrupt => queue += 1,
            Structure::Inexpressible => permanent += 1,
        }
    });

    assert_eq!(scan.counts.queue, queue);
    assert_eq!(scan.counts.permanent, permanent);
    assert_eq!(scan.counts.union, queue + permanent);
    assert_eq!(scan.text_corrupt.len(), text_corrupt);
}

/// A counts file round-trips through a bless and then checks clean.
#[test]
fn counts_ratchet_blesses_then_verifies() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("counts.txt");
    let path = path.to_str().expect("utf-8 path");
    let found = Counts {
        queue: 7,
        permanent: 3,
        union: 10,
    };

    temp_env_bless(|| counts_ratchet(path, found, "# header\n"));

    let body = std::fs::read_to_string(path).expect("written");
    assert!(body.starts_with("# header\n"), "header must lead the file");
    assert!(body.contains("queue 7"));
    assert!(body.contains("permanent 3"));
    assert!(body.contains("union 10"));

    // Checking against the same numbers passes.
    counts_ratchet(path, found, "# header\n");
}

/// A counts file that disagrees with reality fails.
#[test]
#[should_panic(expected = "census-len")]
fn counts_ratchet_rejects_a_stale_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("census-len9-counts.txt");
    let path = path.to_str().expect("utf-8 path");
    temp_env_bless(|| {
        counts_ratchet(
            path,
            Counts {
                queue: 7,
                permanent: 3,
                union: 10,
            },
            "# h\n",
        )
    });
    counts_ratchet(
        path,
        Counts {
            queue: 8,
            permanent: 3,
            union: 11,
        },
        "# h\n",
    );
}

/// Run `f` with `KASANE_CENSUS_BLESS` set, then restore the environment.
///
/// Serialized behind a mutex: `std::env::set_var` is process-global and libtest
/// runs tests on threads, so two blessing tests in flight at once would see
/// each other's variable.
///
/// No `unsafe` block: this workspace is edition 2021 (`Cargo.toml:8`), where
/// `set_var` is safe. Wrapping it would trip `unused_unsafe` and fail the
/// `-D warnings` lint gate. If the workspace ever moves to edition 2024 this
/// needs an `unsafe` block and a SAFETY comment naming the mutex.
fn temp_env_bless<T>(f: impl FnOnce() -> T) -> T {
    use std::sync::Mutex;
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("KASANE_CENSUS_BLESS", "1");
    let out = f();
    std::env::remove_var("KASANE_CENSUS_BLESS");
    out
}
