# Length-5/6 Novelty Tier Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Price lengths 5 and 6 of the emphasis census and ship a guard at both — four zero-assertions (text and novelty at each length) plus three ratcheted counts at length 5.

**Architecture:** All length-generic machinery lands in `census_support` (one base-19 odometer, one bitset, one novelty predicate, one combined scan). Two thin test binaries, `census_len5.rs` and `census_len6.rs`, hold `#[ignore]`d deep tests run by release-profile `mise` tasks. Length 5 runs in PR CI and commits three counts; length 6 runs in a weekly workflow and commits nothing.

**Tech Stack:** Rust (stable 1.97.1, pinned in `mise.toml`), `pulldown-cmark` for the parse-back oracle, bash + `git` for the ratchet, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-08-26-length-5-6-novelty-tier-design.md`

## Global Constraints

- **Branch:** `length-5-6-novelty-tier`, already created, spec already committed at `a7e3ed0`.
- **Lint gate is `mise run lint`** — `cargo fmt --all -- --check` plus `cargo clippy --workspace --all-targets -- -D warnings`. Plain `cargo clippy` is not sufficient and will miss test targets.
- **Never use `test = false`** in `Cargo.toml` for these targets. Verified during design: a `test = false` target containing `this is not valid rust at all !!!` passes both `cargo clippy --all-targets` and `cargo test` with exit 0. Use `#[ignore]`.
- **Deep tiers are release-profile.** Debug is ~8x slower and makes length 5 cost ~3.3 min instead of ~35 s.
- **The census alphabet is 19 elements** (`census_support::alphabet()`). Every index in this plan is base-19.
- **Ledger is always `Ledger::LICENSED`** for these tiers, matching `census.rs` and `census_len4.rs`.
- **Expected measured values** (release, at `699b471`): length 5 — text-corrupt 0, queue 983,694, permanent 220,618, union 1,204,312, novel 0. Length 6 — text-corrupt 0, union 26,501,436, novel 0. Length 4 novelty against length 3 — 0. If any shipped figure disagrees, **stop**: spec §10 says the item halts until the disagreement is resolved.
- **Edition is 2021** (`Cargo.toml:8`). `std::env::set_var` is **safe** here; an `unsafe` block around it trips `unused_unsafe` and fails `-D warnings`.
- **`census_support/mod.rs` carries no `#[test]` functions and must not gain any.** It is a shared oracle included by `census.rs`, `census_probe.rs`, `census_len4.rs` and, after this plan, `census_len5.rs` and `census_len6.rs` — a test placed there runs once per binary. Unit tests for its helpers go in `census_support_tests.rs` (Task 1).
- **Commit messages** end with `Claude-Session: https://claude.ai/code/session_01NePS9A8HjvQYciaKsSfM2k`.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/kasane-writer/tests/census_support/mod.rs` | **Modify.** Gains the generic odometer, `NonClean` bitset, `is_novel`, `Counts`, `deep_scan`, `counts_ratchet`. |
| `crates/kasane-writer/tests/census_len4.rs` | **Modify.** Its private odometer becomes a wrapper over the shared one. |
| `crates/kasane-writer/tests/census_support_tests.rs` | **Create.** The only home for unit tests of `census_support`'s helpers — that module must stay test-free (see Global Constraints). |
| `crates/kasane-writer/tests/census_len5.rs` | **Create.** Length-5 tiers, `#[ignore]`d, plus the counts ratchet. |
| `crates/kasane-writer/tests/census-len5-counts.txt` | **Create (blessed).** Three labelled counts. |
| `crates/kasane-writer/tests/census_len6.rs` | **Create.** Length-6 tiers, `#[ignore]`d. No files. |
| `crates/kasane-writer/tests/ratchet_gate_cases.sh` | **Modify.** Direction 8; `eight` → `nine`. |
| `mise.toml` | **Modify.** `census-len5`, `census-len6` tasks; `census-bless` third line; `census-ratchet` count rows. |
| `.github/workflows/ci.yml` | **Modify.** "Census length 5" step before "Census ratchet". |
| `.github/workflows/census-deep.yml` | **Create.** Weekly + dispatch, runs `census-len6`. |
| `AGENTS.md` | **Modify.** §7 of the spec. |

---

### Task 1: The shared odometer

Removes the second copy of the base-19 carry loop rather than adding a third. `census_support`'s module doc already states this is what the module exists for.

**Files:**
- Modify: `crates/kasane-writer/tests/census_support/mod.rs` (add after `shapes()`, which ends at line 201)
- Modify: `crates/kasane-writer/tests/census_len4.rs:56-78` (`for_each_length_four_shape`)
- Create: `crates/kasane-writer/tests/census_support_tests.rs`

**Interfaces:**
- Consumes: `census_support::alphabet()`
- Produces: `pub const ALPHABET_LEN: usize = 19;`, `pub fn pow19(k: usize) -> usize`, `pub fn for_each_shape(len: usize, f: impl FnMut(&[Inline], &[usize]))` — `f` receives the shape and its base-19 digit slice, in odometer order.

- [ ] **Step 1: Write the failing test**

Create `crates/kasane-writer/tests/census_support_tests.rs`:

```rust
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
    alphabet, classify_with, counts_ratchet, deep_scan, for_each_shape, is_novel,
    nonclean_bitset, Counts, NonClean, Structure, ALPHABET_LEN,
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
```

- [ ] **Step 2: Run it and watch it fail**

```bash
cargo test -p kasane-writer --test census_support_tests the_shared_odometer -- --nocapture
```

Expected: FAIL to compile — `no function or associated item named 'for_each_shape' found`.

- [ ] **Step 3: Add the shared odometer**

Append to `crates/kasane-writer/tests/census_support/mod.rs`:

```rust
/// The census alphabet's size, and the radix every shape index is written in.
///
/// A shape of length `n` is a base-`ALPHABET_LEN` numeral with `n` digits, most
/// significant first, and that numeral is its index everywhere in this module.
/// `nonclean_bitset` keys on it and `is_novel` does deletion arithmetic with
/// it, both of which would be wrong rather than merely slow if this drifted
/// from `alphabet().len()`. `alphabet_len_matches_the_radix` pins them
/// together.
pub const ALPHABET_LEN: usize = 19;

/// `ALPHABET_LEN.pow(k)`, as a `usize`.
///
/// Written as a fold rather than `pow` so an overflow at length 7 and up
/// panics in debug on the multiply rather than wrapping silently: 19^7 fits a
/// `usize`, but nothing here bounds what a future caller passes.
pub fn pow19(k: usize) -> usize {
    (0..k).fold(1usize, |a, _| a * ALPHABET_LEN)
}

/// Every sequence of `len` elements over the census alphabet, in ascending
/// base-`ALPHABET_LEN` order, handed to `f` one at a time.
///
/// Streamed rather than materialized: a `Vec` of 19^5 shapes held at once is a
/// cost the odometer does not pay, and at 19^6 it is not payable at all.
///
/// `f` receives the digit slice as well as the shape, because the deep tiers
/// need the shape's index to do `is_novel`'s deletion arithmetic and
/// recomputing it from the shape would mean a reverse lookup per element.
/// Callers that do not need it take `|seq, _|`.
///
/// This is the **only** carry loop in the census. It lived in `census_len4.rs`
/// as `for_each_length_four_shape` until lengths 5 and 6 needed one too, and a
/// second copy there would have been exactly the drift this module exists to
/// prevent -- the same argument `blessing()`'s doc makes about itself.
pub fn for_each_shape(len: usize, mut f: impl FnMut(&[Inline], &[usize])) {
    let a = alphabet();
    assert_eq!(a.len(), ALPHABET_LEN);
    let mut idx = vec![0usize; len];
    loop {
        let seq: Vec<Inline> = idx.iter().map(|&k| a[k].clone()).collect();
        f(&seq, &idx);
        let mut k = len;
        loop {
            if k == 0 {
                return;
            }
            k -= 1;
            idx[k] += 1;
            if idx[k] < ALPHABET_LEN {
                break;
            }
            idx[k] = 0;
        }
    }
}
```

Then append to `crates/kasane-writer/tests/census_support_tests.rs`:

```rust
/// The radix and the alphabet cannot drift apart.
///
/// `ALPHABET_LEN` is a constant because it is arithmetic, not a lookup, and
/// `pow19` would be a function call per digit otherwise. This is the price of
/// that: one test tying the constant to the thing it describes.
#[test]
fn alphabet_len_matches_the_radix() {
    assert_eq!(alphabet().len(), ALPHABET_LEN);
}
```

- [ ] **Step 4: Rewrite `census_len4.rs`'s odometer as a wrapper**

Replace the body of `for_each_length_four_shape` (`crates/kasane-writer/tests/census_len4.rs:56-78`) with:

```rust
/// Every sequence of length 4 over the census alphabet, handed to `f` one at a
/// time.
///
/// A thin wrapper over `census_support::for_each_shape`, which is the census's
/// one carry loop. This function kept its own copy until lengths 5 and 6
/// needed the same loop; two copies in two files is the drift `census_support`
/// exists to prevent, and three would have been worse.
fn for_each_length_four_shape(mut f: impl FnMut(&[Inline])) {
    census_support::for_each_shape(4, |seq, _| f(seq));
}
```

Add `for_each_shape` to the `use census_support::{...}` list at the top of the file.

- [ ] **Step 5: Run the length-4 tier and confirm nothing moved**

```bash
cargo test -p kasane-writer --release --test census_len4
```

Expected: PASS, 5 tests (the 4 existing plus the new odometer test). If `inline_structure_survives_rendering_for_every_shape_of_length_four` fails, the wrapper changed enumeration order or content — fix the wrapper, do **not** bless.

- [ ] **Step 6: Lint and commit**

```bash
mise run lint
git add crates/kasane-writer/tests/census_support/mod.rs crates/kasane-writer/tests/census_len4.rs crates/kasane-writer/tests/census_support_tests.rs
git commit -m "refactor(census): one base-19 odometer, shared

for_each_length_four_shape held the census's only carry loop. Lengths 5
and 6 need the same loop, so it moves to census_support and length 4
becomes a wrapper -- deleting the second copy rather than adding a third.

The shared form also yields each shape's base-19 digits, which the deep
tiers need for is_novel's deletion arithmetic.

Claude-Session: https://claude.ai/code/session_01NePS9A8HjvQYciaKsSfM2k"
```

---

### Task 2: The `NonClean` bitset

**Files:**
- Modify: `crates/kasane-writer/tests/census_support/mod.rs`

**Interfaces:**
- Consumes: `for_each_shape`, `pow19`, `ALPHABET_LEN`, `classify_with`, `Structure`
- Produces: `pub struct NonClean`, `NonClean::new(len) -> NonClean`, `NonClean::set(&mut self, i: usize)`, `NonClean::get(&self, i: usize) -> bool`, `NonClean::count(&self) -> usize`, `pub fn nonclean_bitset(len: usize, ledger: Ledger) -> NonClean`

- [ ] **Step 1: Write the failing test**

Append to `crates/kasane-writer/tests/census_support_tests.rs`:

```rust
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
        assert_eq!(bits.get(value), nonclean, "disagreement at index {value}: {seq:?}");
        if nonclean {
            direct += 1;
        }
    });
    assert_eq!(bits.count(), direct);
}
```

- [ ] **Step 2: Run and watch it fail**

```bash
cargo test -p kasane-writer --test census_support_tests nonclean_bitset
```

Expected: FAIL to compile — `cannot find type 'NonClean' in this scope`.

- [ ] **Step 3: Implement**

Append to `crates/kasane-writer/tests/census_support/mod.rs`:

```rust
/// Non-clean shapes of one length, as a bitset keyed by base-`ALPHABET_LEN`
/// index.
///
/// A bitset rather than a `BTreeSet<String>` of `format!("{seq:?}")`, which is
/// what the ratchet files use and what the design probes used first. At length
/// 5 that set is ~100 MB and materially slower to build and query, and the
/// length-6 novelty check needs the length-5 set **resident** while it walks
/// 47 million shapes. 19^5 bits is 310 KB.
///
/// This is an in-memory index, never a committed file. Nothing blesses it and
/// nothing reads it across revisions -- design spec §2.2 is why lengths 5 and
/// 6 commit no per-shape files at all.
pub struct NonClean {
    bits: Vec<u64>,
    len: usize,
}

impl NonClean {
    /// An empty bitset with room for every shape of `len`.
    pub fn new(len: usize) -> Self {
        NonClean {
            bits: vec![0u64; pow19(len) / 64 + 1],
            len,
        }
    }

    /// The shape length this bitset indexes.
    pub fn shape_len(&self) -> usize {
        self.len
    }

    pub fn set(&mut self, i: usize) {
        self.bits[i / 64] |= 1 << (i % 64);
    }

    pub fn get(&self, i: usize) -> bool {
        self.bits[i / 64] >> (i % 64) & 1 == 1
    }

    /// How many bits are set.
    pub fn count(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }
}

/// Classify every shape of `len` and record the non-clean ones.
///
/// "Non-clean" is `Structure::Corrupt` **or** `Structure::Inexpressible`, i.e.
/// the union the ratchet gates -- not the queue alone. `is_novel` asks whether
/// a shape's family is already visible to a shipped tier, and a shape filed as
/// permanent is just as visible as one in the queue.
pub fn nonclean_bitset(len: usize, ledger: Ledger) -> NonClean {
    let mut bits = NonClean::new(len);
    let mut value = 0usize;
    for_each_shape(len, |seq, _| {
        if classify_with(seq, ledger) != Structure::Clean {
            bits.set(value);
        }
        value += 1;
    });
    bits
}
```

- [ ] **Step 4: Run and watch it pass**

```bash
cargo test -p kasane-writer --release --test census_support_tests nonclean_bitset -- --nocapture
```

Expected: PASS, 2 tests.

- [ ] **Step 5: Lint and commit**

```bash
mise run lint
git add crates/kasane-writer/tests/census_support/mod.rs crates/kasane-writer/tests/census_support_tests.rs
git commit -m "feat(census): NonClean, a base-19 bitset of non-clean shapes

The length-6 novelty check needs the length-5 non-clean set resident
while it walks 47 million shapes. As a BTreeSet<String> of debug
spellings -- the form the ratchet files use -- that is ~100MB; as a
bitset keyed by base-19 index it is 310KB.

Non-clean means the gated union, Corrupt or Inexpressible, not the queue
alone: a shape filed as permanent is just as visible to a shipped tier.

Claude-Session: https://claude.ai/code/session_01NePS9A8HjvQYciaKsSfM2k"
```

---

### Task 3: `is_novel` — the predicate, proven to bite

This is the task spec §8 calls out: a tier asserting zero that has never been *seen* to fail is indistinguishable from one that always passes. The writer cannot be made to emit novel corruption on demand, so the demonstration is on the predicate over hand-built bitsets.

**Files:**
- Modify: `crates/kasane-writer/tests/census_support/mod.rs`

**Interfaces:**
- Consumes: `NonClean`, `ALPHABET_LEN`
- Produces: `pub fn is_novel(idx: &[usize], shorter: &NonClean) -> bool`

- [ ] **Step 1: Write the failing tests**

Append to `crates/kasane-writer/tests/census_support_tests.rs`:

```rust
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
```

- [ ] **Step 2: Run and watch them fail**

```bash
cargo test -p kasane-writer --test census_support_tests novel -- --nocapture
```

Expected: FAIL to compile — `cannot find function 'is_novel' in this scope`.

- [ ] **Step 3: Implement**

Append to `crates/kasane-writer/tests/census_support/mod.rs`:

```rust
/// Whether a shape is **novel**: non-clean for a reason no shorter shape shows.
///
/// `idx` is the shape's base-`ALPHABET_LEN` digits; `shorter` is the non-clean
/// bitset one length down. The shape is novel when **all** of its
/// single-deletion sub-shapes are clean. The caller has already established
/// that the shape itself is non-clean -- this function does not re-classify it.
///
/// **Deletion, not contiguous substring**, and that is measured rather than
/// chosen: of the 1,204,312 non-clean length-5 shapes, all 1,204,312 have a
/// non-clean single-deletion sub-shape but only 1,204,044 have a non-clean
/// contiguous one. A substring relation reports 268 false novelties on a clean
/// tree. `an_interior_deletion_is_enough_to_make_a_shape_derivative` is what
/// stops someone "simplifying" this into one.
///
/// Novelty is **zero at every length measured** -- 4 against <=3, 5 against
/// <=4, 6 against <=5 -- which is why lengths 5 and 6 assert zero and commit no
/// per-shape files (design spec §2.2). That zero is a property of this writer
/// today, not a theorem.
pub fn is_novel(idx: &[usize], shorter: &NonClean) -> bool {
    debug_assert_eq!(idx.len(), shorter.shape_len() + 1);
    for i in 0..idx.len() {
        let mut sub = 0usize;
        for (p, &d) in idx.iter().enumerate() {
            if p != i {
                sub = sub * ALPHABET_LEN + d;
            }
        }
        if shorter.get(sub) {
            return false;
        }
    }
    true
}
```

- [ ] **Step 4: Run and watch them pass**

```bash
cargo test -p kasane-writer --test census_support_tests novel -- --nocapture
```

Expected: PASS, 3 tests.

- [ ] **Step 5: Confirm novelty is zero at length 4, the length already guarded**

This is the cheapest reproduction of the design's central measurement, and it runs in ~2 s. Append to `crates/kasane-writer/tests/census_len4.rs`:

```rust
/// Length-4 corruption is entirely inherited from lengths <= 3.
///
/// The cheap end of the measurement lengths 5 and 6 rest on (design spec §2.1):
/// novelty is zero at every length measured. This one is affordable in the
/// default test run, so it is the tripwire that fires first if the novelty
/// relation ever stops holding -- long before the weekly length-6 job speaks.
#[test]
fn no_length_four_shape_is_corrupt_for_a_reason_length_three_does_not_show() {
    let shorter = census_support::nonclean_bitset(3, Ledger::LICENSED); // census_len4.rs imports by path
    let mut novel = 0usize;
    let mut first: Vec<String> = Vec::new();
    for_each_shape(4, |seq, idx| {
        if classify_with(seq, Ledger::LICENSED) != Structure::Clean
            && census_support::is_novel(idx, &shorter)
        {
            novel += 1;
            if first.len() < 10 {
                first.push(format!("{seq:?}"));
            }
        }
    });
    assert_eq!(
        novel, 0,
        "{novel} length-4 shape(s) are corrupt for a reason no length-3 shape \
         shows. Design spec §2.2's case for lengths 5 and 6 committing no \
         per-shape files rests on this being zero -- if it is not, that \
         argument is void and the deep tiers need re-designing, not \
         re-blessing.\nFirst {}:\n  {}",
        first.len(),
        first.join("\n  ")
    );
}
```

- [ ] **Step 6: Run it**

```bash
cargo test -p kasane-writer --release --test census_len4 no_length_four_shape_is_corrupt -- --nocapture
```

Expected: PASS. If it reports a non-zero count, **stop and report** — spec §10.

- [ ] **Step 7: Lint and commit**

```bash
mise run lint
git add crates/kasane-writer/tests/census_support/mod.rs crates/kasane-writer/tests/census_support_tests.rs crates/kasane-writer/tests/census_len4.rs
git commit -m "feat(census): is_novel, and the length-4 tripwire for it

A shape is novel when it is non-clean and all of its single-deletion
sub-shapes are clean. Lengths 5 and 6 assert this is never true, which
is what lets them commit no per-shape files.

An assertion of zero that has never been seen to fail cannot be told
from one that always passes, and the writer cannot be made to emit
novel corruption to order -- so the predicate is pinned over hand-built
bitsets instead, including the interior-deletion case that separates it
from a contiguous-substring relation.

The length-4 check runs in the default test set at ~2s, so it fires
long before the weekly length-6 job would.

Claude-Session: https://claude.ai/code/session_01NePS9A8HjvQYciaKsSfM2k"
```

---

### Task 4: `deep_scan` and the counts ratchet

**Files:**
- Modify: `crates/kasane-writer/tests/census_support/mod.rs`

**Interfaces:**
- Consumes: `for_each_shape`, `NonClean`, `is_novel`, `classify_with`, `text_is_clean`, `blessing`
- Produces: `pub struct Counts { pub queue: usize, pub permanent: usize, pub union: usize }`, `pub struct DeepScan { pub text_corrupt: Vec<String>, pub counts: Counts, pub novel: Vec<String> }`, `pub fn deep_scan(len: usize, shorter: &NonClean, ledger: Ledger) -> DeepScan`, `pub fn counts_ratchet(path: &str, found: Counts, header: &str)`

- [ ] **Step 1: Write the failing test**

Append to `crates/kasane-writer/tests/census_support_tests.rs`:

```rust
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
    let found = Counts { queue: 7, permanent: 3, union: 10 };

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
        counts_ratchet(path, Counts { queue: 7, permanent: 3, union: 10 }, "# h\n")
    });
    counts_ratchet(path, Counts { queue: 8, permanent: 3, union: 11 }, "# h\n");
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
```

- [ ] **Step 2: Run and watch them fail**

```bash
cargo test -p kasane-writer --test census_support_tests deep_scan counts_ratchet
```

Expected: FAIL to compile — `cannot find function 'deep_scan'`.

- [ ] **Step 3: Implement**

Append to `crates/kasane-writer/tests/census_support/mod.rs`:

```rust
/// The three census counts at one length.
///
/// `union` is `queue + permanent` and is stored rather than derived, because
/// it is the number the ratchet **gates** and a reader of the committed file
/// should not have to add two numbers to find the gated one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Counts {
    pub queue: usize,
    pub permanent: usize,
    pub union: usize,
}

/// Everything one walk over a length yields.
///
/// The two zero-assertions carry their offending shapes rather than only
/// counts, because a failure of either is a design event -- see
/// `census_len5.rs`'s failure text -- and the first thing anyone will want is
/// an example.
pub struct DeepScan {
    pub text_corrupt: Vec<String>,
    pub counts: Counts,
    pub novel: Vec<String>,
}

/// How many offending shapes a failing assertion reports.
const DEEP_SCAN_SAMPLE: usize = 10;

/// One walk over every shape of `len`: both tiers and the novelty check.
///
/// `shorter` must be the non-clean bitset for `len - 1`.
///
/// **This renders each shape twice** -- `text_is_clean` and `classify_with`
/// each call `render` -- and the measured costs in design spec §2 already
/// include that. Folding them into one render means restructuring
/// `classify_with`, which is shared with the length-1-3 and length-4 tiers and
/// is not worth the risk to save minutes on a weekly job. §10 records that the
/// figure is reported rather than hidden.
pub fn deep_scan(len: usize, shorter: &NonClean, ledger: Ledger) -> DeepScan {
    let mut text_corrupt: Vec<String> = Vec::new();
    let mut novel: Vec<String> = Vec::new();
    let (mut queue, mut permanent) = (0usize, 0usize);
    let mut n_text = 0usize;
    let mut n_novel = 0usize;

    for_each_shape(len, |seq, idx| {
        if !text_is_clean(seq, ledger) {
            n_text += 1;
            if text_corrupt.len() < DEEP_SCAN_SAMPLE {
                text_corrupt.push(format!("{seq:?}"));
            }
        }
        match classify_with(seq, ledger) {
            Structure::Clean => return,
            Structure::Corrupt => queue += 1,
            Structure::Inexpressible => permanent += 1,
        }
        if is_novel(idx, shorter) {
            n_novel += 1;
            if novel.len() < DEEP_SCAN_SAMPLE {
                novel.push(format!("{seq:?}"));
            }
        }
    });

    // The samples are capped; the counts are not. A caller asserting zero needs
    // the true total in its message, so the capped vectors are padded with a
    // tail marker rather than silently under-reporting.
    if n_text > text_corrupt.len() {
        text_corrupt.push(format!("... and {} more", n_text - text_corrupt.len()));
    }
    if n_novel > novel.len() {
        novel.push(format!("... and {} more", n_novel - novel.len()));
    }

    DeepScan {
        text_corrupt,
        counts: Counts { queue, permanent, union: queue + permanent },
        novel,
    }
}

/// Bless or check one counts file.
///
/// The counts analogue of [`ratchet`], and deliberately **not** a ratchet: it
/// asserts equality in both directions, exactly as `ratchet` does for a shape
/// file. Whether a count may only shrink is `mise run census-ratchet`'s
/// question, asked across revisions; this one asks only whether the committed
/// file still describes the writer. Design spec §5 is why the two must not be
/// merged: the ratchet takes this file's accuracy on trust, which is only
/// earned once this assertion has run.
pub fn counts_ratchet(path: &str, found: Counts, header: &str) {
    let body = format!(
        "{header}queue {}\npermanent {}\nunion {}\n",
        found.queue, found.permanent, found.union
    );
    if blessing() {
        std::fs::write(path, body).expect("writing the counts file");
        return;
    }

    let known = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("{path} must exist -- bless it with KASANE_CENSUS_BLESS=1"));
    let strip = |s: &str| -> String {
        s.lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| format!("{l}\n"))
            .collect()
    };
    assert_eq!(
        strip(&known),
        strip(&body),
        "{path} no longer describes this writer.\n\
         \n\
         Committed above, measured below. Every one of these numbers moving is \
         normal on a change that improves the writer -- re-bless with \
         `mise run census-bless`. What is NOT normal is `union` going UP: that \
         is a shape becoming corrupt that was not, and \
         `mise run census-ratchet` will refuse it against main."
    );
}
```

- [ ] **Step 4: Run and watch them pass**

```bash
cargo test -p kasane-writer --release --test census_support_tests deep_scan counts_ratchet -- --nocapture
```

Expected: PASS, 3 tests.

- [ ] **Step 5: Lint and commit**

```bash
mise run lint
git add crates/kasane-writer/tests/census_support/mod.rs crates/kasane-writer/tests/census_support_tests.rs
git commit -m "feat(census): deep_scan and counts_ratchet

One walk per length yielding both tiers and the novelty check, and the
counts analogue of ratchet().

counts_ratchet asserts equality in both directions, like ratchet() does
for a shape file -- it asks whether the committed file still describes
the writer, not whether a count may only shrink. The second question is
census-ratchet's, asked across revisions, and it takes this assertion's
answer on trust.

deep_scan renders each shape twice; the measured costs in the spec
already include that, and folding the two renders together would mean
restructuring classify_with, which three tiers share.

Claude-Session: https://claude.ai/code/session_01NePS9A8HjvQYciaKsSfM2k"
```

---

### Task 5: `census_len5.rs`

**Files:**
- Create: `crates/kasane-writer/tests/census_len5.rs`
- Create (blessed, not hand-written): `crates/kasane-writer/tests/census-len5-counts.txt`
- Modify: `mise.toml` (`census-len5` task, `census-bless` third line)

**Interfaces:**
- Consumes: `deep_scan`, `nonclean_bitset`, `counts_ratchet`, `Counts`, `for_each_shape`
- Produces: `mise run census-len5`; the committed `census-len5-counts.txt`

- [ ] **Step 1: Create the tier**

Create `crates/kasane-writer/tests/census_len5.rs`:

```rust
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

use census_support::{
    counts_ratchet, deep_scan, nonclean_bitset, Counts,
};
use kasane_writer::Ledger;

/// 19^5.
const LEN5_SHAPES: usize = 2_476_099;

const LEN5_COUNTS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-len5-counts.txt"
);

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
    let Counts { queue, permanent, union } = deep_scan(5, &shorter, Ledger::LICENSED).counts;
    assert_eq!(union, queue + permanent);
}
```

- [ ] **Step 2: Add the mise tasks**

In `mise.toml`, add after the `census-bless` task:

```toml
# The length-5 tier, release profile. `#[ignore]`d in the source so
# `mise run test` (debug) skips it -- 35s here is 3.3 min there, for the same
# answer.
#
# `--include-ignored` rather than `--ignored`: the latter runs ONLY ignored
# tests, and this binary is meant to run whole.
[tasks.census-len5]
description = "Both census tiers plus the novelty check at length 5"
run = "cargo test -p kasane-writer --release --test census_len5 -- --include-ignored"
```

And add a third line to `census-bless`'s `run` array:

```toml
  "KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --release --test census_len5 -- --include-ignored",
```

Update `census-bless`'s doc comment: it says "Regenerate every census ratchet file, both tiers". Change the description to `"Regenerate every census ratchet file, every tier"` and add to the comment above it:

```toml
# Length 5 is here and RELEASE, unlike the two above: a debug bless of that
# tier costs ~3.3 min for the same file. Length 6 is NOT here and has nothing
# to bless -- it commits no files at all, because counts on a weekly cadence
# go stale on main (design spec §3.2).
```

- [ ] **Step 3: Bless the counts file**

```bash
mise run census-bless
cat crates/kasane-writer/tests/census-len5-counts.txt
```

Expected content (after the header):

```
queue 983694
permanent 220618
union 1204312
```

**If any number differs from these, STOP and report it.** Spec §10: the probe is corrected, not the tier — but the design's own numbers must be reconciled before the item proceeds.

- [ ] **Step 4: Run the tier clean**

```bash
mise run census-len5
```

Expected: PASS, 3 tests, ~35 s. Record the actual wall-clock — spec §10 wants the observed figure, not the projected one.

- [ ] **Step 5: Confirm the default test run skips it**

```bash
cargo test -p kasane-writer --test census_len5 2>&1 | tail -5
```

Expected: `3 ignored`, and each line naming the reason `release-only deep tier`.

- [ ] **Step 6: Lint and commit**

```bash
mise run lint
git add crates/kasane-writer/tests/census_len5.rs crates/kasane-writer/tests/census-len5-counts.txt mise.toml
git commit -m "feat(census): the length-5 tier

Text and novelty both assert zero with no allowlist; three counts cover
the gap novelty leaves, a change that multiplies corruption inside
families already known at length 4.

No per-shape file, and that is measured rather than conceded: all
1,204,312 non-clean shapes at this length have a non-clean
single-deletion sub-shape, so the file would be a 112MB index of the
length-4 tier and would assert nothing it does not already assert.

#[ignore] rather than test = false in Cargo.toml -- a target marked
test = false is invisible to clippy --all-targets as well, so a tier
that stopped compiling would stay green until someone ran the deep task
by hand.

Claude-Session: https://claude.ai/code/session_01NePS9A8HjvQYciaKsSfM2k"
```

---

### Task 6: `census_len6.rs`

**Files:**
- Create: `crates/kasane-writer/tests/census_len6.rs`
- Modify: `mise.toml` (`census-len6` task)

**Interfaces:**
- Consumes: `deep_scan`, `nonclean_bitset`
- Produces: `mise run census-len6`

- [ ] **Step 1: Create the tier**

Create `crates/kasane-writer/tests/census_len6.rs`:

```rust
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
```

- [ ] **Step 2: Add the mise task**

In `mise.toml`, after `census-len5`:

```toml
# The length-6 tier. ~10 min, so it lives in the weekly census-deep workflow
# rather than PR CI -- .github/workflows/census-deep.yml.
#
# It commits no files, so there is no census-bless line for it and
# `mise run census-ratchet` has nothing here to compare. Its guard is the
# tier's own assertion, which works the same on main as on a branch
# (design spec §3.2).
[tasks.census-len6]
description = "Both census tiers plus the novelty check at length 6 (~10 min)"
run = "cargo test -p kasane-writer --release --test census_len6 -- --include-ignored"
```

- [ ] **Step 3: Run it**

```bash
time mise run census-len6
```

Expected: PASS, 2 tests. The printed line should read `union 26501436`. Record the wall-clock.

**If `union` differs materially from 26,501,436, or either assertion fails, STOP and report** — spec §10.

- [ ] **Step 4: Confirm the default test run skips it**

```bash
cargo test -p kasane-writer --test census_len6 2>&1 | tail -4
```

Expected: `2 ignored`.

- [ ] **Step 5: Lint and commit**

```bash
mise run lint
git add crates/kasane-writer/tests/census_len6.rs mise.toml
git commit -m "feat(census): the length-6 tier, committing no files

Text and novelty assert zero. No counts, and that is the design: counts
go stale on every writer improvement, and on a weekly cadence they would
be found stale on main up to a week late -- a weekly job that is
routinely red stops being read. Zero stays zero under improvement.

With no committed files, census-ratchet has nothing here to compare,
which is fine: its git-diff half is inert on a push to main anyway, and
this tier's guard is its own assertion.

Uncovered and named: a change that multiplies corruption inside families
already known at length 4. Length 5's counts cover that; here it is the
price of a guard with nothing to rot.

Claude-Session: https://claude.ai/code/session_01NePS9A8HjvQYciaKsSfM2k"
```

---

### Task 7: The `union5` gate in `census-ratchet`

**Files:**
- Modify: `mise.toml`, `[tasks.census-ratchet]`

**Interfaces:**
- Consumes: `crates/kasane-writer/tests/census-len5-counts.txt`
- Produces: table rows `queue5`, `perm5`, `union5`; `union5` gates.

- [ ] **Step 1: Add the file path and the count reader**

In `[tasks.census-ratchet]`, beside the other path variables (after `ceil4=`):

```bash
counts5="$dir/census-len5-counts.txt"
```

After the `at_head()` definition, add:

```bash
# One labelled count out of a counts file, at a revision or at HEAD. Prints
# nothing and returns 1 when the file or the label is absent, which is how the
# skip below is detected.
count_at() { # label path revision-or-empty
  local body
  if [ -n "$3" ]; then
    git cat-file -e "$3:$2" 2>/dev/null || return 1
    body="$(git show "$3:$2")"
  else
    [ -f "$2" ] || return 1
    body="$(cat "$2")"
  fi
  printf '%s\n' "$body" | awk -v k="$1" '$1 == k { print $2; found = 1 }
                                          END { exit !found }'
}
```

- [ ] **Step 2: Add the three rows**

After the `check union4 ...` line, add:

```bash
# The length-5 counts. Three rows, one gate.
#
# `union5` is gated and the other two are reported, which is the length-3/4
# logic reproduced rather than a new rule: there, "the union is what makes the
# move safe to allow". `perm5` growing while `union5` is flat IS queue ->
# permanent movement; `perm5` growing while `union5` grows fails on `union5`.
# So the permanence ceiling has no length-5 form -- there is no set difference
# to compute "newly permanent" from, and none is needed.
#
# Accuracy is inherited from `mise run census-len5`, which asserts this file
# still describes the writer. That step runs BEFORE this one in ci.yml for the
# same reason the Test step already does: run first, this could pass on a file
# the tier was about to reject. A hand-edited `union 1204311` sails through
# here -- the union shrank -- and only that tier catches the lie.
count_check() { # label key gate
  local b h verdict
  if ! b="$(count_at "$2" "$counts5" "$base")"; then
    printf '%-8s %8s %8s %8s   %s\n' "$1" "" "" "" "skipped (no baseline)"
    return 0
  fi
  if ! h="$(count_at "$2" "$counts5" "")"; then
    echo "FAIL $1: $counts5 is missing its '$2' line at HEAD." >&2
    fail=1
    return 0
  fi
  verdict="ok"
  if [ "$h" -gt "$b" ]; then
    if [ "$3" = gate ]; then
      verdict="FAIL -- $((h - b)) added"
      fail=1
    else
      verdict="$((h - b)) moved in (not gated)"
    fi
  fi
  printf '%-8s %8s %8s %+8d   %s\n' "$1" "$b" "$h" "$((h - b))" "$verdict"
}
count_check queue5 queue     report
count_check perm5  permanent report
count_check union5 union     gate
```

- [ ] **Step 3: Verify the rows appear and pass**

```bash
mise run census-ratchet
```

Expected: the table gains `queue5`, `perm5`, `union5`. On this branch the counts file is absent at the merge base, so all three should read `skipped (no baseline)` — that is correct and matches how `union4` behaved on the branch that introduced it.

- [ ] **Step 4: Verify the gate bites, by hand, before automating it**

```bash
cp crates/kasane-writer/tests/census-len5-counts.txt /tmp/c5.bak
sed -i 's/^union 1204312$/union 1204313/' crates/kasane-writer/tests/census-len5-counts.txt
KASANE_RATCHET_BASE=HEAD mise run census-ratchet; echo "exit=$?"
cp /tmp/c5.bak crates/kasane-writer/tests/census-len5-counts.txt
git diff --stat  # must be empty
```

Expected: `union5` row reads `FAIL -- 1 added`, task exits 1. With `KASANE_RATCHET_BASE=HEAD` the file exists at the base, so the skip does not fire.

- [ ] **Step 5: Commit**

```bash
mise run lint
git add mise.toml
git commit -m "feat(census): gate union5 in the ratchet

Three rows from census-len5-counts.txt; union5 gates and the other two
report. That is the length-3/4 logic reproduced, not a new rule: perm
growing while the union is flat IS queue -> permanent movement, and perm
growing while the union grows fails on the union. The permanence ceiling
therefore has no length-5 form, and needs none.

Claude-Session: https://claude.ai/code/session_01NePS9A8HjvQYciaKsSfM2k"
```

---

### Task 8: Direction 8 in `ratchet_gate_cases.sh`

Memory of this repo: every gate the ratchet table prints has a negative direction, and each asserts the **row**, never the exit status.

**Files:**
- Modify: `crates/kasane-writer/tests/ratchet_gate_cases.sh`

**Interfaces:**
- Consumes: `census-len5-counts.txt`, `row_says`, `back_up`, `ratchet`
- Produces: direction 8

- [ ] **Step 1: Add the file to the backup set**

Beside the other path variables near the top of the script:

```bash
counts5=crates/kasane-writer/tests/census-len5-counts.txt
```

- [ ] **Step 2: Add the passing-direction coherence check**

In direction 1, after the `union` block and before the ceiling loop:

```bash
# The same argument for union5, which direction 8 targets. Like union4 it CAN
# skip -- its counts file is absent at any base predating it -- so it has the
# same two failure modes: a stale branch, or a gate that was removed.
if row_says union5 'skipped \(no baseline\)' "$tmp/out.1"; then
  echo "FAIL: union5 skipped -- no length-5 counts at this base, so direction 8" >&2
  echo "      cannot be exercised. Rebase onto a commit that carries" >&2
  echo "      census-len5-counts.txt." >&2
  exit 1
fi
if ! row_says union5 '[[:space:]]ok$' "$tmp/out.1"; then
  echo "FAIL: no passing 'union5 ... ok' row; the length-5 union gate did not run." >&2
  exit 1
fi
```

- [ ] **Step 3: Add direction 8 at the end, before the success message**

```bash
echo
echo "== direction 8: a length-5 union growth must FAIL =="
back_up "$counts5"
# +1 on the gated number only. queue5 and perm5 are report-only, so leaving
# them alone is what proves union5 spoke rather than a neighbour: this file
# cannot trip any other row.
awk '$1 == "union" { print $1, $2 + 1; next } { print }' "$counts5" > "$tmp/counts5.new"
cat "$tmp/counts5.new" > "$counts5"
if ratchet "$tmp/out.8"; then
  echo "FAIL: the gate accepted a length-5 union growth" >&2
  exit 1
fi
if ! row_says union5 'FAIL' "$tmp/out.8"; then
  echo "FAIL: census-ratchet failed, but not on the union5 row -- so this" >&2
  echo "      direction proves nothing about the gate under test." >&2
  exit 1
fi
echo "  ok: union5 refused the growth"
restore_all
```

Match the surrounding directions' restore idiom exactly — read directions 2 and 3 in the file and copy whichever form they use (`restore_all` or an inline restore). Do not invent a third.

- [ ] **Step 4: Update the "eight reasons" comments**

Adding a gated row makes it nine. Two places say eight:

- `crates/kasane-writer/tests/ratchet_gate_cases.sh`, in `row_says`'s doc: *"fails as a whole for any of eight reasons"* → **nine**.
- `crates/kasane-writer/tests/ratchet_gate_cases.sh`, in `line_says`'s doc: *"the ceiling is one of eight things that fail this task"* → **nine**.
- `mise.toml`, `[tasks.census-ratchet-cases]`'s comment: *"because eight rows can fail this task"* → **nine**.

Also extend the script's header comment block with a `direction 8` entry matching the style of 2–7, and update the closing sentence *"Every gate the ratchet table prints now has a direction here"* — it stays true, but name `union5` in the list.

- [ ] **Step 5: Run it**

```bash
mise run census-ratchet-cases; echo "exit=$?"
git status --short   # MUST be empty -- every mutated file restored
```

Expected: every direction reports ok, exit 0, clean tree. On this branch direction 1's new `union5` coherence check will **fail** with "no length-5 counts at this base" until the branch merges — that is the check working. Verify with `KASANE_RATCHET_BASE=HEAD mise run census-ratchet-cases` instead, and note the constraint in the commit message.

- [ ] **Step 6: Commit**

```bash
mise run lint
git add crates/kasane-writer/tests/ratchet_gate_cases.sh mise.toml
git commit -m "test(census): direction 8, the length-5 union gate

Bumps union in census-len5-counts.txt and asserts the union5 ROW reports
FAIL. Asserting the row rather than the exit status is the recorded
trap: census-ratchet fails as a whole for any of nine rows now, and an
unrelated gate speaking first is indistinguishable from the one under
test.

queue5 and perm5 get no direction because they do not gate, consistent
with perm, queue4 and perm4. No positive direction either: union5's
predicate has one term, and the ceiling was the only two-term gate.

Direction 1 gains the union5 coherence check, so a permanently-skipping
gate cannot report itself proven.

Claude-Session: https://claude.ai/code/session_01NePS9A8HjvQYciaKsSfM2k"
```

---

### Task 9: CI wiring

**Files:**
- Modify: `.github/workflows/ci.yml`
- Create: `.github/workflows/census-deep.yml`

- [ ] **Step 1: Add the length-5 step to `ci.yml`**

Between the `Test` step and the `Census ratchet` step:

```yaml
      # Before "Census ratchet", not after, and for the same reason that step
      # already runs after Test: the ratchet compares census-len5-counts.txt
      # across revisions and takes its ACCURACY on trust. A hand-edited
      # `union 1204311` sails through the ratchet -- the union shrank -- and
      # only this tier's own assertion catches the lie.
      #
      # Release profile, ~35s. In debug the same three tests take ~3.3 min for
      # the same answer, which is why they are #[ignore]d out of `mise run test`.
      - name: Census length 5
        run: mise run census-len5
```

- [ ] **Step 2: Create `.github/workflows/census-deep.yml`**

```yaml
name: census-deep

# The length-6 census tier. ~10 minutes, so it does not belong in PR CI.
#
# It commits no files, so unlike `mise run census-ratchet` there is nothing
# here that needs a merge base -- which is why this job wants no `fetch-depth`
# at all. The guard is the tier's own assertion, and that works the same on
# main as on a branch (design spec §3.2).
#
# Weekly means a novel-at-6 regression surfaces on main up to a week late. That
# is the same bargain fuzz.yml makes, and it is why length 5 -- not length 6 --
# is the one in PR CI.
on:
  workflow_dispatch:
  # Offset an hour from fuzz.yml (08:00 Monday), which is itself offset from
  # audit.yml (07:00), so the three do not contend.
  schedule:
    - cron: '0 9 * * 1'

permissions:
  contents: read

concurrency:
  group: census-deep-${{ github.ref }}
  cancel-in-progress: true

jobs:
  census-len6:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@93cb6efe18208431cddfb8368fd83d5badbf9bfd # v5

      - uses: jdx/mise-action@dad1bfd3df957f44999b559dd69dc1671cb4e9ea # v4.2.1

      - uses: Swatinem/rust-cache@c19371144df3bb44fab255c43d04cbc2ab54d1c4 # v2.9.1

      # --include-ignored is inside the task; see mise.toml.
      - name: Census length 6
        run: mise run census-len6
```

Verify the two action SHAs against `.github/workflows/ci.yml` before committing — they must match exactly, since this repo pins by SHA.

- [ ] **Step 3: Validate the workflow parses**

```bash
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/census-deep.yml')); print('parses')"
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml')); print('parses')"
```

Expected: `parses` twice.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml .github/workflows/census-deep.yml
git commit -m "ci: length 5 in PR CI, length 6 weekly

The length-5 step precedes Census ratchet, which is load-bearing: the
ratchet takes census-len5-counts.txt's accuracy on trust, so a
hand-edited union that shrank would sail through it and only the tier's
own assertion catches that.

census-deep.yml needs no fetch-depth, unlike the main job -- with no
committed length-6 files there is no merge base to resolve.

Claude-Session: https://claude.ai/code/session_01NePS9A8HjvQYciaKsSfM2k"
```

---

### Task 10: The documentation this item falsifies

Spec §7. Two statements become false, and two design specs get status lines rather than rewrites.

**Files:**
- Modify: `AGENTS.md` (the census entry, around line 428)
- Modify: `crates/kasane-writer/tests/census_len4.rs` (module doc, lines 12-14 and 35-38)
- Modify: `docs/superpowers/specs/2026-08-23-length-4-structural-tier-design.md` (§9)
- Modify: `docs/superpowers/specs/2026-08-21-declined-run-rescan-design.md` (§2.2)
- Modify: `docs/superpowers/specs/2026-08-26-length-5-6-novelty-tier-design.md` (status line)

- [ ] **Step 1: `AGENTS.md`**

Replace:

> Lengths 5 and 6 stay unpriced for structure as well as text: minutes, not seconds.

with:

```markdown
  Lengths 5 and 6 are priced and guarded, since 2026-08-26, and **neither
  commits a per-shape file**. Every non-clean shape at 4, 5 and 6 has a
  non-clean single-deletion sub-shape — corruption in this alphabet never
  *originates* above length 3 — so a length-5 allowlist would be a 112 MB
  index of the length-4 tier rather than evidence about length 5. What ships
  instead is a **novelty** assertion at each length (zero, no file, on the
  length-4 text tier's contract) plus three counts at length 5:
  `census-len5-counts.txt`, queue 983,694, permanent 220,618, union 1,204,312,
  of which `mise run census-ratchet` gates `union` alone. Length 6 commits
  nothing at all — counts on a weekly cadence go stale on main, and zero stays
  zero under improvement. `mise run census-len5` runs in PR CI at ~35 s;
  `mise run census-len6` is ~10 min and runs weekly via
  `.github/workflows/census-deep.yml`. The old "minutes, not seconds" reason
  for leaving both unpriced was measured against the **debug** profile: the
  length-4 binary is 5.64 s debug and 0.72 s release
  (`2026-08-26-length-5-6-novelty-tier-design.md`).
```

- [ ] **Step 2: `census_len4.rs`'s module doc**

Replace *"Lengths 5 and 6 were swept too (`2026-08-21-declined-run-rescan-design.md` §2.2, also zero) and are not shipped: they cost minutes, not seconds."* with:

```rust
//! Lengths 5 and 6 were swept too (`2026-08-21-declined-run-rescan-design.md`
//! §2.2, also zero) and **now ship**, as `census_len5.rs` and `census_len6.rs`
//! -- neither with a per-shape file. The old reason for holding them back,
//! "minutes, not seconds", was measured against the debug profile; this binary
//! is 5.64 s in debug and 0.72 s in release
//! (`2026-08-26-length-5-6-novelty-tier-design.md` §1).
```

And replace the closing *"Lengths 5 and 6 remain unpriced for structure as well as text, for the same reason: minutes, not seconds."* with:

```rust
//! Lengths 5 and 6 are priced and guarded since 2026-08-26. They assert
//! **novelty** -- that no shape is corrupt for a reason a shorter shape does
//! not already show -- rather than carrying allowlists of their own, because
//! every non-clean shape at 5 and 6 has a non-clean single-deletion sub-shape
//! and a file there would index this one.
```

- [ ] **Step 3: Status lines, not rewrites**

This is the convention `2026-08-23-delimiter-choice-ordering-design.md` §6.1 established: the sentence stays true of *that* branch and must not be edited to pretend this work was in scope there.

In `2026-08-23-length-4-structural-tier-design.md` §9, append to the "Length 5 or 6" bullet:

```markdown
  *(Landed 2026-08-26 as `census_len5.rs` and `census_len6.rs` —
  `2026-08-26-length-5-6-novelty-tier-design.md`. The sentence above stays as
  written: it was true of this branch, and "minutes, not seconds" was the
  honest reading of a debug-profile measurement at the time.)*
```

In `2026-08-21-declined-run-rescan-design.md` §2.2, append the same style of status line naming where the sweeps landed.

- [ ] **Step 4: Update this item's own spec status line**

In `2026-08-26-length-5-6-novelty-tier-design.md`, change:

```markdown
**Status:** designed 2026-08-26; not yet implemented.
```

to record implementation, the branch, and — per §10 — any figure the shipped tiers measured differently from the probes.

- [ ] **Step 5: Full verification**

```bash
mise run lint
mise run test
mise run census-len5
KASANE_RATCHET_BASE=HEAD mise run census-ratchet
KASANE_RATCHET_BASE=HEAD mise run census-ratchet-cases
git status --short   # must be empty
```

Expected: all pass, tree clean. `mise run census-len6` was already run in Task 6; re-run it only if `census_support` changed since.

- [ ] **Step 6: Commit**

```bash
git add AGENTS.md crates/kasane-writer/tests/census_len4.rs docs/superpowers/specs/
git commit -m "docs: lengths 5 and 6 are priced and guarded

AGENTS.md and census_len4.rs both said they stay unpriced because they
cost minutes not seconds. Both halves were wrong: they are priced now,
and the cost claim was a debug-profile artifact -- the length-4 binary
is 5.64s debug and 0.72s release.

The two older specs get status lines rather than rewrites, per the
convention delimiter-choice-ordering §6.1 set: those sentences were true
of those branches and must not be edited to pretend this work was in
scope there.

Claude-Session: https://claude.ai/code/session_01NePS9A8HjvQYciaKsSfM2k"
```

---

## Self-Review

**Spec coverage.** §1 cost reframing → Task 10 (docs) and Tasks 5/6 (release tasks). §2 numbers → Tasks 5, 6 verification steps. §2.1 novelty relation → Task 3. §2.2 no per-shape file → Tasks 5, 6 module docs. §3 what each length asserts → Tasks 5, 6. §3.1 no allowlist / failure text → Task 5 Step 1, Task 6 Step 1. §3.2 no length-6 counts → Task 6. §3.3 the gap counts close → Task 5 header, Task 6 module doc. §4.1 shared machinery → Tasks 1-4. §4.2 `#[ignore]` not `test = false` → Tasks 5, 6 + Global Constraints. §4.3 counts file → Task 5. §5 venues and ordering → Task 9. §6 gates → Task 7. §6.1 direction 8 → Task 8. §7 falsified docs → Task 10. §8 tests → Tasks 1, 3, 5, 6, 8. §10 stop-on-disagreement → verification steps in Tasks 3, 5, 6.

**Known gap, deliberate.** §9's non-goals need no task; they are scope statements.

**Type consistency.** `for_each_shape(len, |seq, idx|)` — two arguments everywhere (Tasks 1, 2, 3, 5, 6). `NonClean` methods `new`/`set`/`get`/`count`/`shape_len` — `shape_len` is used only by `is_novel`'s `debug_assert` (Task 3), defined in Task 2. `nonclean_bitset(len, ledger)` takes a `Ledger` in every call site. `deep_scan(len, &shorter, ledger) -> DeepScan` with fields `text_corrupt`/`counts`/`novel`; `counts` is a `Counts` with `queue`/`permanent`/`union`. `counts_ratchet(path, found, header)` takes `&str` header, not `Option<&str>` — unlike `ratchet`, because a counts file always has one.
