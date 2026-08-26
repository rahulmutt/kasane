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

use census_support::{alphabet, for_each_shape, ALPHABET_LEN};

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
