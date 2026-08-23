# Length-4 Structural Tier Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a structural census tier at length 4, so emphasis-structure regressions above length 3 fail the build instead of shipping silently.

**Architecture:** A second `#[test]` in the existing `crates/kasane-writer/tests/census_len4.rs`, classifying all 130,321 length-4 shapes with `census_support::classify_with` and gating them against two committed per-shape ratchet files plus a hand-raised permanence ceiling — the same contract `census.rs` uses at length 3. The ratchet helpers move from `census.rs` into `census_support/mod.rs` so both tiers share one implementation rather than two that can drift.

**Tech Stack:** Rust 1.97.1 (pinned in `mise.toml`), `cargo test`, `pulldown-cmark` (via `census_support`), bash + `git worktree` for the verification, `mise` tasks for CI entry points.

**Spec:** `docs/superpowers/specs/2026-08-23-length-4-structural-tier-design.md`

## Global Constraints

- **Branch:** `len4-structural-tier`. The spec is already committed there as `f6d1886`.
- **Per-task checks:** every task ends with `mise run lint` (which is `cargo fmt --all -- --check` **and** `cargo clippy --workspace --all-targets -- -D warnings`) and the relevant `cargo test`. Plain `cargo clippy` is not sufficient — `--all-targets` is what reaches test binaries, and every file in this plan is a test binary.
- **No writer changes.** This item ships a guard only. If any step tempts you to edit `crates/kasane-writer/src/`, stop — that is spec §9's first non-goal, and a guard that also changes the writer cannot be trusted to have measured the writer.
- **Measured baseline**, from spec §2, at `97b2604` (main), debug profile. Every count below must be reproduced by the shipped tier, not assumed:
  - 130,321 shapes; 78,725 `Clean`; **41,443 `Corrupt`**; **10,153 `Inexpressible`**.
  - Length-4 permanent file splits 7,585 same-class-nesting / 2,568 strong-over-emph-only / **0 neither**.
  - Structural pass ≈ 5.6s debug; existing text tier ≈ 3.75s.
- **If the shipped tier disagrees with any of those numbers, the probe was wrong and the spec's numbers get corrected — not the tier's.** Spec §10.
- **File naming:** `census-len4-known-structure-corrupt.txt`, `census-len4-inexpressible.txt`, `census-len4-permanent-count.txt`, all in `crates/kasane-writer/tests/`.

---

### Task 1: Move the ratchet helpers into `census_support`

**Files:**
- Modify: `crates/kasane-writer/tests/census_support/mod.rs` (add imports + three `pub fn`)
- Modify: `crates/kasane-writer/tests/census.rs:283-372` (delete three `fn`, import them instead, pass a path to `permanence_ceiling`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces, all `pub` in `census_support`:
  - `fn blessing() -> bool`
  - `fn permanence_ceiling(path: &str) -> usize`
  - `fn ratchet(path: &str, found: &BTreeSet<String>, noun: &str, header: Option<&str>)`

**Why this is a refactor with no new test.** Nothing here changes behavior — the same functions run over the same files. The test for this task is that `census.rs`'s existing tier still passes unchanged, which is a stronger gate than a new unit test would be: it exercises the real ratchet against real 1,611- and 433-entry files in both directions. Do not invent a test for the move.

- [ ] **Step 1: Run the length-3 tier and record that it is green before you touch anything**

Run: `cargo test -p kasane-writer --test census`
Expected: PASS. Note the test count; it must be identical at Step 6.

- [ ] **Step 2: Add `BTreeSet` to `census_support`'s imports**

In `crates/kasane-writer/tests/census_support/mod.rs`, after the existing `use` block:

```rust
use std::collections::BTreeSet;
```

- [ ] **Step 3: Append the three functions to `census_support/mod.rs`**

Add at the end of the file:

```rust
/// Whether this run is regenerating the ratchet files rather than checking
/// them.
///
/// Spelled once and shared by every tier, because two readers disagreeing
/// about what a bless is would let one of them write while the other asserts
/// against the file it just changed. It lived in `census.rs` until the
/// length-4 structural tier needed it too; a second copy there would have been
/// that same hazard, one file further away.
pub fn blessing() -> bool {
    std::env::var_os("KASANE_CENSUS_BLESS").is_some()
}

/// The most entries a permanent file may hold.
///
/// A **ceiling**, not a count: a permanent file shrinking is always an
/// improvement, so this is only ever compared as an upper bound and a shrink
/// needs no edit. A bless *lowers* it to match — safe, since lowering only
/// tightens the gate — and never raises it. Raising it is a hand edit, and
/// that asymmetry is the entire point; see `PERMANENT_CEILING`'s doc in
/// `census.rs` for what went wrong when the claim was made at scale without
/// one.
///
/// Takes its path rather than closing over one, because there is a permanent
/// file per length and a helper that could only read the length-3 one would be
/// copied rather than reused.
pub fn permanence_ceiling(path: &str) -> usize {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path} must exist and be readable: {e}"));
    raw.trim()
        .parse()
        .unwrap_or_else(|e| panic!("{path} must hold a single integer: {e}"))
}

/// Bless or check one ratchet file, two-directionally: a shape that is in
/// `found` but not the file fails, and a shape in the file but not `found`
/// fails too, so the file can neither grow silently nor rot into stale
/// excuses.
///
/// `#`-prefixed lines are comments, which is how a permanent file carries its
/// generated header.
///
/// Note that the first assertion **short-circuits the second**: a run that
/// finds newly-corrupt shapes panics before it can report shapes that are no
/// longer corrupt. A caller that needs both directions of one file in one
/// look must bless and diff, not read a single failure.
pub fn ratchet(path: &str, found: &BTreeSet<String>, noun: &str, header: Option<&str>) {
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
```

- [ ] **Step 4: Delete the three originals from `census.rs` and import them**

In `crates/kasane-writer/tests/census.rs`, delete the whole `fn blessing`, `fn permanence_ceiling`, and `fn ratchet` definitions **including their doc comments**, with two exceptions you move rather than delete:

- `permanence_ceiling`'s long history — the 2026-08-17 probe, the 748-shape bless, the 1,984 → 433 correction, and the 428 → 433 raise on the delimiter branch — is **length-3-specific**. Move that prose onto the `PERMANENT_CEILING` constant, so it stays attached to the file it is about:

```rust
/// The ceiling on `census-inexpressible.txt`, read by
/// `census_support::permanence_ceiling`.
///
/// Raising it is a hand edit, and that asymmetry is the entire point. Moving a
/// shape into the permanent file asserts that *no writer change can ever fix
/// it*, which is the one claim in this census that nothing downstream
/// re-examines: the queue is worked down item by item, but permanence is read
/// as settled. `KASANE_CENSUS_BLESS=1` must therefore not be able to make that
/// claim on its own — it writes the three shape files and stops, so a growing
/// permanent file leaves the test failing until a human raises the number in
/// the same commit. That is a deliberately visible one-line diff.
///
/// The gate exists because the claim went wrong at scale once already. A probe
/// on 2026-08-17 searched every `*`/`_` spelling of each shape in this file and
/// found 1,740 of 1,984 expressible; 748 of those entries had been moved in by
/// a single bless (`2026-08-16-cross-class-edge-splice-design.md` §4).
///
/// That probe's *reason* was itself wrong — it measured what CommonMark can
/// spell, not what this pipeline can emit, and offering `_` at the emission
/// site turned out to fix zero shapes
/// (`2026-08-23-delimiter-choice-ordering-design.md` §2). Its *verdict* stood
/// anyway: reordering the delimiter choice ahead of the splice took the file
/// from 1,984 to 433 on 2026-08-23. The permanence claim was ~78% wrong, for a
/// cause nobody had named.
///
/// And once, on that branch, the gate spoke in the other direction. The feature
/// commit's bless lowered the ceiling to 428 while five shapes were sitting in
/// the queue that belonged here; fixing that had to *raise* it back to 433 by
/// hand, in the commit that needed it. Lowering is the cheap direction and it
/// is also the direction that can quietly spend headroom a later fix needs.
const PERMANENT_CEILING: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-permanent-count.txt"
);
```

Then extend the `use census_support::{...}` block to:

```rust
use census_support::{
    blessing, classify_with, context_text, context_walks_with, ir_context, parsed_text,
    permanence_ceiling, ratchet, render, shapes, Structure,
};
```

- [ ] **Step 5: Pass the path at the one call site**

In `inline_structure_survives_rendering_for_every_short_sequence`, change:

```rust
    let ceiling = permanence_ceiling();
```

to:

```rust
    let ceiling = permanence_ceiling(PERMANENT_CEILING);
```

- [ ] **Step 6: Run the length-3 tier and confirm it is unchanged**

Run: `cargo test -p kasane-writer --test census`
Expected: PASS, with the same number of tests as Step 1. A behavior change here means the move was not a move.

- [ ] **Step 7: Lint**

Run: `mise run lint`
Expected: clean. `#![allow(dead_code)]` at the top of `census_support/mod.rs` already covers the fact that `census_probe.rs` uses none of the three.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-writer/tests/census_support/mod.rs crates/kasane-writer/tests/census.rs
git commit -m "refactor(census): share the ratchet helpers from census_support

The length-4 structural tier needs blessing/ratchet/permanence_ceiling
too, and a second copy in a second test binary is the drift census_support
exists to prevent -- the same argument context_walks_with's doc already
makes for sharing the render/gate/walk setup. permanence_ceiling takes its
path now, because there is one permanent file per length.

The length-3 history stays behind on PERMANENT_CEILING, where it belongs:
it is about that file, not about the mechanism."
```

---

### Task 2: The structural tier and its baseline files

**Files:**
- Modify: `crates/kasane-writer/tests/census_len4.rs` (module doc, imports, three path constants, a header constant, an enumeration helper, the test)
- Create: `crates/kasane-writer/tests/census-len4-known-structure-corrupt.txt` (generated by bless)
- Create: `crates/kasane-writer/tests/census-len4-inexpressible.txt` (generated by bless)
- Create: `crates/kasane-writer/tests/census-len4-permanent-count.txt` (hand-written integer)
- Modify: `mise.toml` (add `[tasks.census-bless]`)
- Modify: `crates/kasane-writer/tests/census.rs:35` (module doc: "two tiers, and three files")

**Interfaces:**
- Consumes: `census_support::{blessing, permanence_ceiling, ratchet}` from Task 1; `census_support::{alphabet, classify_with, Structure}` which already exist.
- Produces: `LEN4_INEXPRESSIBLE` and `LEN4_INEXPRESSIBLE_HEADER` in `census_len4.rs`, both used by Task 3.

- [ ] **Step 1: Write the failing test**

In `crates/kasane-writer/tests/census_len4.rs`, extend the imports:

```rust
use census_support::{
    alphabet, blessing, classify_with, permanence_ceiling, ratchet, text_is_clean, Structure,
};
use kasane_ir::Inline;
use kasane_writer::Ledger;
use std::collections::BTreeSet;
```

Add the constants and the header. The header deliberately claims **no class split yet** — Task 3 measures it and adds it, in the same commit as the test that gates it:

```rust
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
```

Add the enumeration helper and the test. The enumeration is shared with the
pre-existing text tier in this file (`no_shape_of_length_four_loses_text`),
which also walks all 130,321 length-4 shapes by the same odometer: that
function is rewritten in this step to call the shared helper instead of
carrying its own copy of the carry loop, so the two tiers in this file share
one implementation rather than drifting as two:

```rust
/// Every sequence of length 4 over the census alphabet, handed to `f` one at a
/// time.
///
/// Both tiers in this file walk the same 19^4 = 130,321 shapes, by odometer
/// rather than by `shapes()`, which is fixed at lengths 1-3 — and streamed
/// rather than materialized, since a `Vec` of 130,321 shapes held at once is a
/// cost the odometer does not pay. One carry loop, called twice: two copies in
/// one file is the drift `census_support` exists to prevent, one file closer
/// in.
fn for_each_length_four_shape(mut f: impl FnMut(&[Inline])) {
    let a = alphabet();
    let n = a.len();
    let mut idx = [0usize; 4];
    loop {
        let seq: Vec<Inline> = idx.iter().map(|&k| a[k].clone()).collect();
        f(&seq);
        let mut k = 4;
        loop {
            if k == 0 {
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
        std::fs::write(
            LEN4_PERMANENT_CEILING,
            format!("{}\n", inexpressible.len()),
        )
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
```

- [ ] **Step 2: Seed the ceiling file with zero, so the tier reports the real count rather than you asserting it**

```bash
printf '0\n' > crates/kasane-writer/tests/census-len4-permanent-count.txt
```

This is deliberate. Spec §10 requires every figure be reproduced by the shipped tier; seeding the ceiling at `0` makes the tier's own failure message tell you the count, instead of you copying `10153` out of the design doc and never checking it.

- [ ] **Step 3: Run the test to verify it fails on the missing allowlist**

Run: `cargo test -p kasane-writer --test census_len4`
Expected: FAIL, from `ratchet`'s `unwrap_or_else`, with:
`census-len4-known-structure-corrupt.txt must exist -- bless it with KASANE_CENSUS_BLESS=1`

- [ ] **Step 4: Bless, and read the count out of the failure**

Run: `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census_len4`
Expected: FAIL — but *after* both shape files are written. `ratchet` returns early when blessing, so both files land, and then the ceiling assert fires:
`the length-4 permanent file would grow to 10153 entries, over its ceiling of 0.`

Record the number it printed. If it is not `10153`, the spec's §2 table is wrong and must be corrected before you continue — do not adjust the tier to match the doc.

- [ ] **Step 5: Write the measured ceiling and confirm the tier passes**

```bash
printf '10153\n' > crates/kasane-writer/tests/census-len4-permanent-count.txt
cargo test -p kasane-writer --test census_len4
```

Expected: PASS, 2 tests.

- [ ] **Step 6: Confirm the generated files match the spec's measured counts**

```bash
wc -l crates/kasane-writer/tests/census-len4-known-structure-corrupt.txt
grep -vc '^#' crates/kasane-writer/tests/census-len4-inexpressible.txt
```

Expected: `41443` and `10153`. Any disagreement is a spec correction, per the Global Constraints.

- [ ] **Step 7: Confirm the wall-clock claim, and record what you actually saw**

Run: `cargo test -p kasane-writer --test census_len4 2>&1 | tail -3`
Expected: the binary reports a single `finished in` figure for both tests. Spec §4.2 predicts ≈5.7s (the two tests run on parallel threads), not ≈9.3s.

If it lands near 9.3s instead, the runner had no second free core. **Do not change the design** — the tier ships either way; note the real figure, and it is what Task 6 writes into `AGENTS.md`.

- [ ] **Step 8: Correct the two module docs this tier falsifies**

In `crates/kasane-writer/tests/census_len4.rs`, the module doc's final paragraph currently reads "**And this tier is text-only, which is its own gap.** … A structural length-4 tier is the named follow-up; `2026-08-23-delimiter-choice-ordering-design.md` §6.1 is the record." Replace it with:

```rust
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
```

In `crates/kasane-writer/tests/census.rs`, the module doc says "There are two tiers, and three files." Replace that sentence with:

```rust
//! There are two tiers here, and three files — and since 2026-08-23 both tiers
//! run again at length 4 in `census_len4.rs`, where the text tier carries no
//! file because its answer is zero and the structural tier carries three of its
//! own.
```

- [ ] **Step 9: Add the bless task, so nobody blesses one tier and leaves the other stale**

In `mise.toml`, immediately before `[tasks.census-ratchet]`:

```toml
# One command for both tiers. Blessing was a single `cargo test --test census`
# until the length-4 structural tier arrived; with two binaries to bless, a
# human who remembers only one leaves the other's files stale. That failure is
# loud -- CI fails on the un-blessed tier -- but this removes the trap rather
# than relying on the alarm, and gives AGENTS.md one command to name.
#
# It does NOT raise either permanence ceiling: a bless writes the shape files
# and stops, so a growing permanent file still leaves the test failing until a
# human raises the number in the same commit.
[tasks.census-bless]
description = "Regenerate every census ratchet file, both tiers"
run = [
  "KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census",
  "KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census_len4",
]
```

Verify it: `mise run census-bless && git diff --stat` — expected: no changes, because both tiers are already blessed.

- [ ] **Step 10: Run the whole suite and lint**

Run: `mise run test && mise run lint`
Expected: PASS, clean. `mise run test` now carries the new tier; confirm the workspace run still completes and note the delta against the Global Constraints' figures.

- [ ] **Step 11: Commit**

```bash
git add crates/kasane-writer/tests/census_len4.rs \
        crates/kasane-writer/tests/census-len4-known-structure-corrupt.txt \
        crates/kasane-writer/tests/census-len4-inexpressible.txt \
        crates/kasane-writer/tests/census-len4-permanent-count.txt \
        crates/kasane-writer/tests/census.rs \
        mise.toml
git commit -m "test(census): price structure at length 4

No shipped gate priced emphasis structure above length 3: census.rs's
structural tier stops there and this file was text-only. That let 135
length-4 and 3,134 length-5 structural regressions through on the
delimiter-choice branch, invisible to every text tier because the text
was byte-identical, caught only because the family happened to have a
length-3 member.

41,443 corrupt and 10,153 permanent at length 4, on the same contract
length 3 uses: per-shape ratchets checked in both directions, and a
ceiling a bless can lower but only a human can raise. Both counts come
from the shipped tier, not from the design doc -- the ceiling was seeded
at 0 so the tier had to report them.

mise run census-bless blesses both tiers, because two binaries is one
more than a human reliably remembers."
```

---

### Task 3: The permanent header's class split, and the test that gates it

**Files:**
- Modify: `crates/kasane-writer/tests/census_len4.rs` (extend `LEN4_INEXPRESSIBLE_HEADER`, add one test)
- Modify: `crates/kasane-writer/tests/census-len4-inexpressible.txt` (re-blessed header)

**Interfaces:**
- Consumes: `LEN4_INEXPRESSIBLE`, `LEN4_INEXPRESSIBLE_HEADER` from Task 2.
- Produces: nothing later tasks depend on.

**Why this is its own task.** `census.rs` carries `the_permanent_file_holds_exactly_the_five_condition_four_refusals` for a specific reason: the header's class split is hand-written prose living inside a *generated* file, `ratchet` filters `#` lines, so a hand-edit there passes the checker and is silently reverted by the next bless. Nothing compared the numbers against the entries until that test existed. The length-4 header has the identical hazard, and a reviewer should be able to approve the split independently of the tier.

**The split is by cause, and it is not the length-3 split.** Length 3 separates a 428-entry flanking wall from 5 deliberate condition-4 refusals — a distinction by *why the writer declined*, which no grep over 10,153 entries can make. The length-4 split is `classify_with`'s own two permanence conditions instead, which is mechanically checkable and honest. 375 length-4 entries nest a `Strong` directly inside an `Emph`; how many of those are refusals rather than wall is **unmeasured**, and neither the header nor the test claims a number for it.

- [ ] **Step 1: Write the failing test**

Add to `crates/kasane-writer/tests/census_len4.rs`:

```rust
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
#[test]
fn the_length_four_permanent_file_splits_by_its_two_permanence_conditions() {
    const SAME_CLASS: usize = 7_585;
    const STRONG_OVER_EMPH_ONLY: usize = 2_568;

    let body = std::fs::read_to_string(LEN4_INEXPRESSIBLE)
        .unwrap_or_else(|e| panic!("{LEN4_INEXPRESSIBLE} must exist and be readable: {e}"));
    let entries: Vec<&str> = body
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    let same_class = |l: &&str| l.contains("Emph([Emph(") || l.contains("Strong([Strong(");
    let strong_over_emph = |l: &&str| l.contains("Strong([Emph(");

    let n_same = entries.iter().filter(same_class).count();
    let n_soe_only = entries
        .iter()
        .filter(|l| !same_class(l) && strong_over_emph(l))
        .count();
    let n_neither = entries
        .iter()
        .filter(|l| !same_class(l) && !strong_over_emph(l))
        .count();

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
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p kasane-writer --test census_len4 splits_by_its_two_permanence_conditions`
Expected: **PASS**, not fail.

This test is a *consistency check between prose and entries*, and it passes the moment the numbers are right — there is no red phase to manufacture. What must be verified instead is that it **can** fail, which Step 3 does. Do not skip Step 3 on the grounds that Step 2 was green.

- [ ] **Step 3: Prove the test can fail, then restore**

```bash
sed -i 's/const SAME_CLASS: usize = 7_585;/const SAME_CLASS: usize = 7_586;/' \
  crates/kasane-writer/tests/census_len4.rs
cargo test -p kasane-writer --test census_len4 splits_by_its_two_permanence_conditions
```

Expected: FAIL with `holds 7585 entries that nest a same-class container, but LEN4_INEXPRESSIBLE_HEADER says 7586`.

Restore:

```bash
sed -i 's/const SAME_CLASS: usize = 7_586;/const SAME_CLASS: usize = 7_585;/' \
  crates/kasane-writer/tests/census_len4.rs
cargo test -p kasane-writer --test census_len4 splits_by_its_two_permanence_conditions
```

Expected: PASS. Confirm with `git diff --stat crates/kasane-writer/tests/census_len4.rs` that only your intended additions remain.

- [ ] **Step 4: Add the split to the header constant**

In `LEN4_INEXPRESSIBLE_HEADER`, insert after the `# it is a separate claim.` paragraph and before `# COMPUTED, never hand-edited.`:

```
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
```

- [ ] **Step 5: Re-bless so the file carries the new header, and confirm only the header moved**

```bash
mise run census-bless
git diff --stat crates/kasane-writer/tests/census-len4-inexpressible.txt
```

Expected: the file changes, and `git diff` shows **only** added `#` lines — no entry added or removed. If an entry moved, something other than the header changed and you must find out what before continuing.

- [ ] **Step 6: Run the tier and lint**

Run: `cargo test -p kasane-writer --test census_len4 && mise run lint`
Expected: PASS, 3 tests, clean.

- [ ] **Step 7: Commit**

```bash
git add crates/kasane-writer/tests/census_len4.rs \
        crates/kasane-writer/tests/census-len4-inexpressible.txt
git commit -m "test(census): gate the length-4 permanent file's class split

The header's split is hand-written prose inside a generated file, and
ratchet filters '#' lines -- so a hand-edit there passes the checker and
is silently reverted by the next bless. census.rs carries the same test
for the same reason.

The split is classify_with's own two permanence conditions, 7,585 /
2,568 / 0-neither, not the length-3 file's flanking-wall-vs-refusal
split. That one is a distinction by cause, and no grep over ten thousand
entries can make it: 375 entries here nest a Strong inside an Emph and
how many are refusals rather than wall is unmeasured. The header says so
rather than guessing."
```

---

### Task 4: Teach `census-ratchet` the length-4 files

**Files:**
- Modify: `mise.toml`, `[tasks.census-ratchet]` (new file vars, a second union, a shared ceiling check)

**Interfaces:**
- Consumes: the three files created in Task 2.
- Produces: nothing later tasks depend on.

**What this gate does and does not do.** `census.rs`/`census_len4.rs` prove the committed files match what the writer actually does. This task compares committed file to committed file *across revisions* and proves they only ever got better; it never runs a census, which is what lets it stay pure git+text. `Inexpressible → Corrupt` is **not** caught here — the shape was already in the union — it is caught by the tier's own per-shape ratchet, which is the mechanism that spoke at length 3 (spec §5.2).

- [ ] **Step 1: Add the three length-4 paths beside the existing ones**

In `[tasks.census-ratchet]`, after the `ceil=` line:

```bash
q4="$dir/census-len4-known-structure-corrupt.txt"
p4="$dir/census-len4-inexpressible.txt"
ceil4="$dir/census-len4-permanent-count.txt"
```

Extend the collection loop to cover them:

```bash
for f in "$text" "$queue" "$perm" "$q4" "$p4"; do
```

- [ ] **Step 2: Build the second union**

After the existing `for side in base head; do … union; done` block:

```bash
# The length-4 union. Two files, not three: there is no length-4 text file,
# because that tier asserts zero and carries no allowlist by design. So the
# promotion rule below -- structure queue may grow only where the text queue
# shrank -- has no length-4 form either. It exists to PERMIT a growth the union
# would otherwise forbid, and at length 4 there is no such growth to permit.
tq4="$(basename "$q4")"; tp4="$(basename "$p4")"
for side in base head; do
  LC_ALL=C sort -u "$tmp/$side.$tq4" "$tmp/$side.$tp4" > "$tmp/$side.union4"
done
```

- [ ] **Step 3: Extract the ceiling check into a function, so it is written once**

Replace the existing `if git cat-file -e "$base:$ceil"` block with a function plus two calls:

```bash
# A raise is how a human states "these shapes can never be fixed", so it must
# come with shapes that actually moved in. Without this, the number could drift
# upward and pre-authorise a later bless -- precisely the reviewable moment the
# ceiling exists to create. Written once and called per length, because two
# copies would drift the way the census's own helpers were about to.
ceiling_check() { # label ceiling-path base-perm head-perm
  local label="$1" cpath="$2" bperm="$3" hperm="$4" cb ch grew
  if ! git cat-file -e "$base:$cpath" 2>/dev/null; then
    echo "ceiling($label): absent at the base -- introduced by this change, nothing to compare"
    return 0
  fi
  cb="$(git show "$base:$cpath" | tr -d '[:space:]')"
  ch="$(tr -d '[:space:]' < "$cpath")"
  grew="$(comm -13 "$bperm" "$hperm" | wc -l | tr -d ' ')"
  if [ "$ch" -gt "$cb" ] && [ "$grew" -eq 0 ]; then
    echo "FAIL ceiling($label) raised $cb -> $ch with no shape newly claimed inexpressible." >&2
    echo "     A raise is a permanence claim; make it in the commit that needs it." >&2
    fail=1
    return 0
  fi
  echo "ceiling($label): $cb -> $ch ($grew shape(s) newly permanent)"
}
```

- [ ] **Step 4: Add the length-4 checks and both ceiling calls**

After `check union "$tmp/base.union" "$tmp/head.union" gate ""`:

```bash
# The length-4 union skips while either of its files is absent at the base.
# Unlike the length-3 files, these were introduced by the branch that added the
# length-4 structural tier -- so on that branch, and on any branch whose merge
# base predates it, the whole file reads as added and an ungated skip-less check
# would fail on its own introduction. The marker disappears on the first merge
# base that has both files, after which this gates like any other.
if [ -f "$tmp/skip.$tq4" ] || [ -f "$tmp/skip.$tp4" ]; then
  touch "$tmp/skip.union4"
fi
check queue4 "$tmp/base.$tq4"   "$tmp/head.$tq4"   report "$tmp/skip.$tq4"
check perm4  "$tmp/base.$tp4"   "$tmp/head.$tp4"   report "$tmp/skip.$tp4"
check union4 "$tmp/base.union4" "$tmp/head.union4" gate   "$tmp/skip.union4"
```

And replace the old single ceiling block's remnants with:

```bash
echo
ceiling_check len3 "$ceil"  "$tmp/base.$tp"  "$tmp/head.$tp"
ceiling_check len4 "$ceil4" "$tmp/base.$tp4" "$tmp/head.$tp4"
```

- [ ] **Step 5: Run it on this branch and confirm the length-4 sets skip rather than fail**

Run: `mise run census-ratchet`
Expected: PASS. The table shows `queue4`, `perm4` and `union4` as `skipped (no baseline)`, and `ceiling(len4): absent at the base`. The length-3 rows are unchanged and `ceiling(len3)` reports `433 -> 433 (0 shape(s) newly permanent)`.

If `union4` reads `FAIL -- 51596 added`, the skip marker is not reaching it — Step 4's `if` is the bug.

- [ ] **Step 6: Prove the length-4 union gate actually bites, against a base that has the files**

The skip path is not the gate. Exercise the gate by pointing the base at this branch's own tier commit, where the files exist:

```bash
tier="$(git rev-parse HEAD)"
q4=crates/kasane-writer/tests/census-len4-known-structure-corrupt.txt
cp "$q4" /tmp/q4.orig
printf '%s\n' '[Text("zzz"), Text("zzz"), Text("zzz"), Text("zzz")]' >> "$q4"
LC_ALL=C sort -u -o "$q4" "$q4"
KASANE_RATCHET_BASE="$tier" mise run census-ratchet; echo "exit=$?"
cp -f /tmp/q4.orig "$q4"
```

Expected: `exit=1`, with `union4 … FAIL -- 1 added` and the injected shape printed beneath it.

Then confirm the restore worked and the passing direction still passes:

```bash
git diff --stat "$q4"          # expected: no output
KASANE_RATCHET_BASE="$tier" mise run census-ratchet; echo "exit=$?"
```

Expected: `exit=0`, every set `+0 ok`.

Record both runs' output — Task 5 files them as evidence. **Note the scope of this check honestly:** it is a one-off, run by hand, not committed. `ratchet_gate_cases.sh` is deliberately not extended (spec §6.1), so nothing keeps re-proving the length-4 union bites after today.

- [ ] **Step 7: Full suite and lint**

Run: `mise run test && mise run lint && mise run census-ratchet && mise run census-ratchet-cases`
Expected: all PASS. `census-ratchet-cases` still exercises the length-3 queue gate's negative direction and must be unaffected.

- [ ] **Step 8: Commit**

```bash
git add mise.toml
git commit -m "ci(census): ratchet the length-4 files across revisions

A second union, queue4 + perm4, gated the way the length-3 union is. Two
files rather than three because there is no length-4 text file -- that
tier asserts zero and carries no allowlist -- which is also why the
promotion rule has no length-4 form: it exists to permit a growth the
union would otherwise forbid, and there is none to permit.

The union skips while either file is absent at the base, since this
branch introduces them and an ungated check would fail on its own
introduction. Exercised against a base that has them: an injected shape
fails with 'union4 FAIL -- 1 added'.

The ceiling check is a function now, called per length. Two copies of it
would drift the way the census helpers were about to."
```

---

### Task 5: Prove the tier bites, against `05bb516`

**Files:**
- Create: `docs/superpowers/evidence/2026-08-23-len4-structural-tier/README.md`
- Create: `docs/superpowers/evidence/2026-08-23-len4-structural-tier/queue-direction.txt`
- Create: `docs/superpowers/evidence/2026-08-23-len4-structural-tier/both-directions.diff`
- Create: `docs/superpowers/evidence/2026-08-23-len4-structural-tier/union4-gate.txt`
- Modify: `docs/superpowers/specs/2026-08-23-length-4-structural-tier-design.md` §6 (a factual correction — see Step 5)

**Interfaces:**
- Consumes: everything from Tasks 1–4.
- Produces: nothing later tasks depend on.

**Why.** A clean run of a new guard proves nothing unless the guard can be shown to produce a failure. `05bb516` is delimiter-choice-before-splice *without* condition 4 — the commit whose 135 length-4 `Inexpressible → Corrupt` regressions this tier exists to catch. `census_support/mod.rs`, `kasane-ir` and `kasane-writer`'s public surface are byte-identical between `05bb516` and `97b2604`; only `choose_mark`'s body differs. So the current instrument measures the older writer without contamination.

- [ ] **Step 1: Stand up the worktree with this branch's instrument and baselines**

```bash
EV=docs/superpowers/evidence/2026-08-23-len4-structural-tier
mkdir -p "$EV"
WT="$(mktemp -d)/wt-05bb516"
git worktree add --quiet --detach "$WT" 05bb516
cp crates/kasane-writer/tests/census_len4.rs          "$WT/crates/kasane-writer/tests/"
cp crates/kasane-writer/tests/census_support/mod.rs   "$WT/crates/kasane-writer/tests/census_support/"
cp crates/kasane-writer/tests/census-len4-*.txt       "$WT/crates/kasane-writer/tests/"
```

`census.rs` is **not** copied. At `05bb516` it still defines its own `ratchet`/`blessing`/`permanence_ceiling` privately, which does not collide with the now-`pub` copies in `census_support` — and `cargo test --test census_len4` builds only the one target anyway.

- [ ] **Step 2: Run the tier and capture the queue direction**

```bash
(cd "$WT" && cargo test -p kasane-writer --test census_len4 \
  inline_structure_survives_rendering_for_every_shape_of_length_four 2>&1) \
  | tee "$EV/queue-direction.txt" | tail -20
```

Expected: FAIL with `135 shape(s) newly structurally corrupt -- bless them into …census-len4-known-structure-corrupt.txt`, followed by the first ten shapes.

If the count is not 135, **stop.** Spec §10: either the tier does not measure what §1.1 measured or the archived sweep was wrong, and the item does not proceed until that is resolved.

- [ ] **Step 3: Bless inside the worktree and diff, to get both directions with exact shapes**

`ratchet`'s first assertion short-circuits the second, so one run can never show both directions. Blessing and diffing does:

```bash
(cd "$WT" && KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census_len4 \
  inline_structure_survives_rendering_for_every_shape_of_length_four 2>&1 | tail -5)
diff -u crates/kasane-writer/tests/census-len4-known-structure-corrupt.txt \
        "$WT/crates/kasane-writer/tests/census-len4-known-structure-corrupt.txt" \
  > "$EV/both-directions.diff" || true
diff -u crates/kasane-writer/tests/census-len4-inexpressible.txt \
        "$WT/crates/kasane-writer/tests/census-len4-inexpressible.txt" \
  >> "$EV/both-directions.diff" || true
grep -c '^+\[' "$EV/both-directions.diff"
grep -c '^-\[' "$EV/both-directions.diff"
```

Expected: 135 added to the queue and 135 removed from the permanent file — the same 135 shapes, moving `Inexpressible → Corrupt`, which is exactly §1.1's defect.

The bless run itself passes rather than fails: at `05bb516` the permanent file holds 10,018 entries, which is under the 10,153 ceiling, so the ceiling lowers instead of asserting. That is the ceiling behaving correctly, not a missed catch — the queue ratchet is what catches this, and Step 2 is where it spoke.

- [ ] **Step 4: Tear down**

```bash
git worktree remove --force "$WT"
git worktree list          # expected: only the main worktree
git status --short         # expected: only the new evidence files
```

- [ ] **Step 5: Correct spec §6 — it claims something `ratchet` cannot do**

§6 currently says "**Expected: it fails in both directions at once.** ~135 shapes newly structurally corrupt (absent from the queue file), and the same ~135 gone from the permanent file. Both halves of the two-directional ratchet fire, on the same shapes."

That is wrong, and Step 3 is why: `ratchet`'s `assert!(new.is_empty(), …)` panics before the second assertion and before the second `ratchet` call, so a single run reports one direction only. Replace that paragraph with:

```markdown
**Expected: the queue ratchet fails, naming ~135 shapes.** `ratchet`'s first
assertion panics before its second, and before the permanent file's `ratchet`
call runs at all — so one run reports one direction, never both. The permanent
side is confirmed by blessing inside the worktree and diffing the two revisions'
files: ~135 lines added to the queue and the same ~135 removed from the
permanent file, which is `Inexpressible → Corrupt` shown shape by shape.

That short-circuit is now recorded on `ratchet`'s own doc, so the next reader
does not design a verification around a failure mode it cannot produce.
```

- [ ] **Step 6: Write the evidence README**

Create `docs/superpowers/evidence/2026-08-23-len4-structural-tier/README.md`:

```markdown
# Length-4 structural tier — evidence

Design: `docs/superpowers/specs/2026-08-23-length-4-structural-tier-design.md`.

## Does the tier bite?

`05bb516` is delimiter-choice-before-splice without condition 4 — the commit
whose 135 length-4 `Inexpressible → Corrupt` regressions this tier exists to
catch, and which every shipped gate missed at the time.

Method: `git worktree` at `05bb516`, with this branch's `census_len4.rs`,
`census_support/mod.rs` and three `census-len4-*.txt` baselines copied in.
`census_support`, `kasane-ir` and `kasane-writer`'s public surface are
byte-identical across the two revisions; only `choose_mark`'s body differs, so
the instrument measures the older writer without contamination.

- `queue-direction.txt` — the tier failing: `135 shape(s) newly structurally
  corrupt`.
- `both-directions.diff` — the same run blessed inside the worktree and diffed
  against this branch's files: 135 lines into the queue, 135 out of the
  permanent file. One run cannot show both directions, because `ratchet`'s first
  assertion short-circuits its second.

## Does the cross-revision gate bite?

- `union4-gate.txt` — `mise run census-ratchet` with `KASANE_RATCHET_BASE` set
  to the tier commit, once with a shape injected into the length-4 queue
  (`union4 FAIL -- 1 added`, exit 1) and once clean (exit 0).

**Scope, stated rather than implied.** That union check is a one-off, run by
hand on 2026-08-23. `ratchet_gate_cases.sh` is deliberately not extended
(design §6.1), so in CI the length-4 union gate only ever runs in its passing
direction — the silent-gate failure mode this repo has recorded twice. Closing
it is a one-case extension of that script and remains available.
```

- [ ] **Step 7: File the union-gate output from Task 4 Step 6**

Write both runs' output — the failing injected run and the clean run — into
`docs/superpowers/evidence/2026-08-23-len4-structural-tier/union4-gate.txt`.

If you no longer have that output, re-run Task 4 Step 6, piping each `mise run
census-ratchet` through `tee -a` into that file.

- [ ] **Step 8: Record the short-circuit on `ratchet`'s doc**

Task 1 Step 3 already added this paragraph to `census_support::ratchet`. Confirm it is present and reads:

```rust
/// Note that the first assertion **short-circuits the second**: a run that
/// finds newly-corrupt shapes panics before it can report shapes that are no
/// longer corrupt. A caller that needs both directions of one file in one
/// look must bless and diff, not read a single failure.
```

If Task 1 was implemented without it, add it now — it is the finding this task's Step 5 rests on.

- [ ] **Step 9: Lint and commit**

```bash
mise run lint
git add docs/superpowers/evidence/2026-08-23-len4-structural-tier \
        docs/superpowers/specs/2026-08-23-length-4-structural-tier-design.md \
        crates/kasane-writer/tests/census_support/mod.rs
git commit -m "docs(evidence): the length-4 tier catches what shipped past every gate

Run against 05bb516 -- delimiter choice before the splice, condition 4
not yet landed -- the tier fails naming 135 newly structurally corrupt
shapes, and blessing there and diffing shows the same 135 leaving the
permanent file. That is exactly the Inexpressible -> Corrupt regression
no shipped gate priced at the time.

Corrects the design's SS6, which expected both directions from one run.
ratchet's first assertion short-circuits its second, so a single failure
reports one direction; the doc now says so."
```

---

### Task 6: Correct `AGENTS.md` and the delimiter spec

**Files:**
- Modify: `AGENTS.md` (Workflows list ~line 308; census entry ~lines 346-412)
- Modify: `docs/superpowers/specs/2026-08-23-delimiter-choice-ordering-design.md` §6.1

**Interfaces:**
- Consumes: the final counts and the wall-clock figure measured in Task 2 Step 7.
- Produces: nothing.

**Why last.** `AGENTS.md`'s census entry describes tiers, files, gates and commands as one coherent block. It can only be written correctly once all of those are final, and writing it early guarantees writing it twice.

- [ ] **Step 1: Add the two new commands to the Workflows list**

In `AGENTS.md`, after the `mise run census-ratchet-cases` line:

```markdown
- `mise run census-bless` — regenerate every census ratchet file, both tiers
```

- [ ] **Step 2: Correct "two tiers, and four files"**

Find `- The census has two tiers, and four files.` and change the opening to:

```markdown
- The census has two tiers and runs them at two lengths, over seven files. The
  text tier compares what
```

- [ ] **Step 3: Rewrite the length-4 sentence**

Find `A third tier joins the two above: crates/kasane-writer/tests/census_len4.rs, the text tier at length 4, asserting zero, with no allowlist because the answer is zero.` and replace with:

```markdown
  `crates/kasane-writer/tests/census_len4.rs` runs **both** tiers again at
  length 4, over all 130,321 shapes. Its text tier asserts zero and carries no
  allowlist, because the answer is zero and a file it does not have cannot rot
  into stale excuses. Its **structural** tier is the one that was missing until
  2026-08-23: `census.rs` stops at length 3, so nothing priced structure above
  it, and 135 length-4 and 3,134 length-5 regressions shipped through every gate
  on the delimiter-choice branch — invisible because the text was byte-identical
  — caught only because that family happened to have a length-3 member. It
  carries three files of its own on the same contract as length 3:
  `census-len4-known-structure-corrupt.txt` (**41,443**, the queue),
  `census-len4-inexpressible.txt` (**10,153**, permanent), and
  `census-len4-permanent-count.txt` (its ceiling). The permanent file's header
  splits by `classify_with`'s two conditions — 7,585 same-class nestings, 2,568
  strong-over-emph-only, none satisfying neither — and **not** by the length-3
  file's flanking-wall-vs-refusal split, which is a distinction by cause that no
  grep over ten thousand entries can make. Lengths 5 and 6 stay unpriced for
  structure as well as text: minutes, not seconds.
  (`2026-08-23-length-4-structural-tier-design.md`.)
```

- [ ] **Step 4: Correct the bless command and the file count**

Find `One bless command rewrites all three shape files` and replace with:

```markdown
  `mise run census-bless` rewrites all five shape files across both tiers — but
  **neither** ceiling
```

Then, in the same sentence, change `census-permanent-count.txt, a one-integer ceiling on how many entries census-inexpressible.txt may hold` to:

```markdown
  `census-permanent-count.txt` and `census-len4-permanent-count.txt`, one-integer
  ceilings on how many entries each permanent file may hold
```

- [ ] **Step 5: Add the length-4 union to the gate description**

After the sentence ending `would read a text fix as a fresh corruption (2026-08-21-declined-run-rescan-design.md §5)`, add:

```markdown
  Length 4 gets a **second union**, its queue plus its permanent file, gated the
  same way. Two files rather than three, because there is no length-4 text file
  — which is also why the promotion rule has no length-4 form: that rule exists
  to *permit* a growth the union would otherwise forbid, and at length 4 there
  is none to permit. The union skips while either file is absent at the merge
  base, so the branch that introduced them did not fail on its own introduction.
  `Inexpressible → Corrupt` is not caught by any union — the shape was already
  in it — but by the tiers' own per-shape ratchets, which is the mechanism that
  spoke at length 3.
```

- [ ] **Step 6: State the gap, where a reader will meet it**

After the existing sentence `The ratchet task on its own only ever exercises the gate where it passes, which is indistinguishable from a gate that always passes.`, add:

```markdown
  That is still true of the **length-4** union: `ratchet_gate_cases.sh` covers
  the length-3 queue gate only, and extending it was deliberately left out of
  scope (`2026-08-23-length-4-structural-tier-design.md` §6.1). The length-4
  union was exercised against an injected growth once, by hand, and the output
  is in `docs/superpowers/evidence/2026-08-23-len4-structural-tier/`. Nothing
  re-proves it.
```

- [ ] **Step 7: Add the status line to the delimiter spec's §6.1**

In `docs/superpowers/specs/2026-08-23-delimiter-choice-ordering-design.md`, at the end of §6.1, append:

```markdown
**Landed 2026-08-23.** The structural length-4 tier this section names shipped
as `census_len4.rs`'s second tier —
`2026-08-23-length-4-structural-tier-design.md`. The sentence above stays as
written: it was true of *this* branch, which measured the gap with an archived
probe rather than assuming it away, and rewriting it to pretend the tier was in
scope here would falsify the record it exists to keep.
```

- [ ] **Step 8: Verify every claim you just wrote is still true**

```bash
wc -l < crates/kasane-writer/tests/census-len4-known-structure-corrupt.txt   # 41443
grep -vc '^#' crates/kasane-writer/tests/census-len4-inexpressible.txt       # 10153
cat crates/kasane-writer/tests/census-len4-permanent-count.txt              # 10153
grep -c 'Emph(\[Emph(\|Strong(\[Strong(' crates/kasane-writer/tests/census-len4-inexpressible.txt  # 7585
ls crates/kasane-writer/tests/census-*.txt | wc -l                          # 7
mise run census-bless && git diff --stat                                    # no changes
```

Every number in Steps 2–7 must match. `AGENTS.md` has been wrong about this census before — "Markdown cannot express" for months, "88%" until it was measured at 78% — and each time it was a claim nobody re-checked.

- [ ] **Step 9: Full gate run**

Run: `mise run test && mise run lint && mise run census-ratchet && mise run census-ratchet-cases`
Expected: all PASS.

- [ ] **Step 10: Commit**

```bash
git add AGENTS.md docs/superpowers/specs/2026-08-23-delimiter-choice-ordering-design.md
git commit -m "docs: record the length-4 structural tier in the codebase map

Two tiers at two lengths over seven files, not two tiers and four. Adds
the length-4 structural tier, its three files and their counts, the
second union and why it has no promotion rule, and mise run census-bless.

States the gap rather than implying it is covered: ratchet_gate_cases.sh
does not exercise the length-4 union's failing direction, so in CI that
gate only ever runs where it passes. The delimiter spec's SS6.1 gets a
status line and keeps its original sentence -- it was true of that
branch, and rewriting it would falsify the record."
```

---

## Self-Review

**Spec coverage.** Every section maps to a task: §1 → Task 2; §2's counts → Task 2 Steps 4-7 (reproduced, not copied); §3 files/contract → Task 2; §3.3 ceiling → Task 2 Steps 2, 5; §4.1 helper move → Task 1; §4.2 placement and wall clock → Task 2 Steps 1, 7; §4.3 no duplicated instrument guards → honored by omission and stated in Task 2's test doc; §5 cross-revision → Task 4; §5.1 no promotion rule → Task 4 Step 2 comment + Task 6 Step 5; §5.2 → Task 4 preamble; §5.3 bless task → Task 2 Step 9; §6 verification → Task 5; §6.1 stated gap → Task 4 Step 6, Task 5 Step 6, Task 6 Step 6; §7 doc corrections → Tasks 2, 6; §8 tests → Tasks 2, 3; §9 non-goals → Global Constraints; §10 risks → Task 2 Steps 4-7, Task 5 Step 2.

**One spec defect found while planning, and corrected in Task 5 Step 5.** §6 expected the verification run to "fail in both directions at once". `ratchet`'s first `assert!` panics before the second and before the permanent file's `ratchet` call, so a single run reports one direction. The plan gets both directions by blessing in the worktree and diffing, and records the short-circuit on `ratchet`'s doc.

**One design question the spec left open, now closed with a measurement.** §10 flagged that the length-4 permanent split might not be the length-3 split. It is not. The length-3 split is by *cause* (flanking wall vs. condition-4 refusal) and cannot be reproduced by grep at 10,153 entries; the length-4 header splits by `classify_with`'s two permanence conditions instead — 7,585 / 2,568 / 0 — which is mechanically checkable. The 375 entries nesting `Strong` inside `Emph` are reported as unmeasured rather than assumed to be refusals.

**Placeholder scan.** No TBD/TODO; every code step carries the code, every command carries its expected output, and the one number that could not be known in advance (the permanent count) is produced by the tier itself in Task 2 Step 4 rather than asserted from the doc.

**Type consistency.** `blessing() -> bool`, `permanence_ceiling(path: &str) -> usize`, `ratchet(path: &str, found: &BTreeSet<String>, noun: &str, header: Option<&str>)` are defined in Task 1 and used with those exact signatures in Tasks 2 and 3. `classify_every_length_four_shape() -> (BTreeSet<String>, BTreeSet<String>)` is defined and used in Task 2 only. `LEN4_INEXPRESSIBLE` and `LEN4_INEXPRESSIBLE_HEADER` are defined in Task 2 and consumed in Task 3. Shell variables `q4`/`p4`/`ceil4`/`tq4`/`tp4` are introduced in Task 4 Steps 1-2 before use in Steps 3-4.
