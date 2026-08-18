# Abutment Ledger Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Recover the ~924 inline shapes whose plain `*`-only spelling already round-trips but which `run_end`'s fusion and `splice_children`'s collapse currently discard, by routing both rules through one enumerated ledger of licensed abutments.

**Architecture:** A `Ledger` is a bitset of licensed *cells*, where a cell is an (outer class, inner class, structural site) triple. `may_abut` is set membership — no text is read and no delimiter pairing is computed, so an unlisted triple is `false`, which is today's behaviour. Task 1 wires both call sites with a ledger that reproduces today's output exactly; every later behaviour change is one more bit turned on, plus the evidence that licenses it.

**Tech Stack:** Rust (stable, pinned in `mise.toml`), `pulldown-cmark` 0.13 as the census oracle, `mise` task runner.

**Spec:** `docs/superpowers/specs/2026-08-18-abutment-ledger-design.md` (committed at `e6d4aa2`). Read it before Task 1; §4.2's ordering note and §6's laundering rule are not restated in full here.

**Branch:** `abutment-ledger`, already created off `main` at `7a5b02f`.

## Global Constraints

- Every task ends green under `mise run lint && mise run test`. `mise run lint` is `clippy --all-targets` plus `fmt --check` — plain `cargo clippy` does not cover test targets and will let a broken test compile through.
- `mise run census-ratchet` runs **after** `mise run test`, never instead of it. It takes the census files' accuracy on trust; only the test establishes it.
- Bless the census with `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census`. **Read the diff.** It is the evidence, not a formality.
- Nothing in this plan emits `_`. The alphabet stays `*`-only; widening it is item 2.
- `splice_children` clones no `Inline` — `flatten_into` borrows and `Vec::splice` shuffles `Flat` pairs. Any change that introduces a clone there violates the parent spec's §2.2 constraint.
- `escape::Delim` and `escape::Ctx` are `pub(crate)`. Nothing in this plan makes them public; the test seam is expressed in `u32` bits and `&str` names instead.
- Commit messages follow the repo's Conventional Commits usage (`feat(writer):`, `test(writer):`, `refactor(writer):`, `docs:`).

---

### Task 1: The ledger seam, behaviour-neutral

Introduce `Site`, `Ledger`, and `may_abut`; thread the ledger from `blocks_to_markdown` down to both rules; delete `sole_child_nests_canonically`. `Ledger::LICENSED` starts with exactly one cell — the one that reproduces `sole_child_nests_canonically` — so **output is byte-identical to `main` and the census files do not move.**

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs` (add the seam; thread through `blocks_to_markdown`, `blocks_to_markdown_at`, `render_block`, `render_table`, `inlines_to_md`, `inlines_to_md_at`, `inlines_to_md_flat`, `emphasis_run`, `run_end`, `splice_children`, `edge_to_splice`, `same_delim_to_splice`)
- Modify: `crates/kasane-writer/src/lib.rs` (re-export `Ledger` as a hidden test seam)
- Modify: `docs/superpowers/specs/2026-08-18-abutment-ledger-design.md` (§3.1 and §5.1: the ledger is a bitset, not a two-value enum)
- Test: `crates/kasane-writer/src/markdown.rs`'s existing `mod tests`

**Interfaces:**
- Consumes: nothing (first task).
- Produces:
  - `pub(crate) enum Site { WholeRun, HeadEdge, TailEdge, Interior, RunSeam }`
  - `#[doc(hidden)] pub struct Ledger(u32)` with `Ledger::CONSERVATIVE`, `Ledger::LICENSED`, `Ledger::from_bits(u32) -> Ledger`, `Ledger::bits(self) -> u32`, and `Ledger::CELLS: &'static [(&'static str, u32)]`
  - `fn may_abut(outer: escape::Delim, inner: escape::Delim, site: Site, ledger: Ledger) -> bool`
  - `#[doc(hidden)] pub fn blocks_to_markdown_with_ledger(blocks: &[Block], assets: &AssetBag, ledger: Ledger) -> String`
  - `pub fn blocks_to_markdown(blocks: &[Block], assets: &AssetBag) -> String` — signature unchanged, delegates with `Ledger::LICENSED`

- [ ] **Step 1: Write the failing test for the ledger's shape**

Add to `mod tests` in `crates/kasane-writer/src/markdown.rs`:

```rust
/// The default arm is the whole safety argument: an unlisted triple is
/// `false`, which is the conservative rule this item started from. If a
/// widening ever makes an unlisted triple `true` by accident, this fails.
#[test]
fn an_unlisted_triple_is_refused() {
    use escape::Delim::{Emph, Strong};
    // Same-`Delim` anywhere: `splice_children`'s `Delim` rule, unchanged.
    assert!(!may_abut(Emph, Emph, Site::Interior, Ledger::LICENSED));
    assert!(!may_abut(Strong, Strong, Site::Interior, Ledger::LICENSED));
    assert!(!may_abut(Emph, Emph, Site::WholeRun, Ledger::LICENSED));
    // The tie-break row: `Strong` over `Emph` spanning a whole run prints
    // `***x***`, which always resolves em-outermost, against the IR.
    assert!(!may_abut(Strong, Emph, Site::WholeRun, Ledger::LICENSED));
}

/// The one cell Task 1 licenses, and the only one: it reproduces
/// `sole_child_nests_canonically`, which this task deletes.
#[test]
fn the_conservative_ledger_licenses_nothing_and_licensed_starts_at_one_cell() {
    use escape::Delim::{Emph, Strong};
    assert!(!may_abut(Emph, Strong, Site::WholeRun, Ledger::CONSERVATIVE));
    assert!(may_abut(Emph, Strong, Site::WholeRun, Ledger::LICENSED));
    assert_eq!(Ledger::LICENSED.bits().count_ones(), 1);
}

/// Every cell in `CELLS` must be reachable from some triple, or the probe in
/// Task 3 would measure a bit nothing can ever set.
#[test]
fn every_named_cell_is_reachable_from_a_triple() {
    use escape::Delim::{Emph, Strong};
    let mut seen = 0u32;
    for outer in [Emph, Strong] {
        for inner in [Emph, Strong] {
            for site in [
                Site::WholeRun,
                Site::HeadEdge,
                Site::TailEdge,
                Site::Interior,
                Site::RunSeam,
            ] {
                if let Some(bit) = bit_for(outer, inner, site) {
                    seen |= bit;
                }
            }
        }
    }
    for (name, bit) in Ledger::CELLS {
        assert!(seen & bit != 0, "cell {name} is named but unreachable");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-writer --lib may_abut 2>&1 | tail -20`
Expected: FAIL to compile — `cannot find function 'may_abut'`, `cannot find type 'Site'`, `cannot find type 'Ledger'`.

- [ ] **Step 3: Add the seam to `markdown.rs`**

Insert immediately above `fn run_end` (currently `markdown.rs:454`):

```rust
/// Where in the printed stream an abutment would happen.
///
/// Structural positions, not descriptions of the text: [`may_abut`] must be
/// answerable from this plus two classes and nothing else. See design spec
/// §3.3 — an arm that needs more than its three parameters is the
/// delimiter-pairing mirror re-entering by the back door.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Site {
    /// The inner container is the run's entire printing content.
    WholeRun,
    /// The inner container is the run's first printing child, of several.
    HeadEdge,
    /// The inner container is the run's last printing child, of several.
    TailEdge,
    /// The inner container sits between other printing content.
    Interior,
    /// The boundary between two adjacent runs, which [`run_end`] is deciding
    /// whether to fuse.
    RunSeam,
}

/// One licensed abutment, as a bit in a [`Ledger`].
///
/// Named constants rather than an enum so [`Ledger::CELLS`] can hand a test a
/// `(name, bit)` pair without exposing `escape::Delim`, which is `pub(crate)`.
mod cell {
    pub(super) const EMPH_OVER_STRONG_WHOLE_RUN: u32 = 1 << 0;
    pub(super) const EMPH_OVER_STRONG_HEAD_EDGE: u32 = 1 << 1;
    pub(super) const EMPH_OVER_STRONG_TAIL_EDGE: u32 = 1 << 2;
    pub(super) const STRONG_OVER_EMPH_HEAD_EDGE: u32 = 1 << 3;
    pub(super) const STRONG_OVER_EMPH_TAIL_EDGE: u32 = 1 << 4;
    pub(super) const EMPH_BESIDE_STRONG_RUN_SEAM: u32 = 1 << 5;
    pub(super) const STRONG_BESIDE_EMPH_RUN_SEAM: u32 = 1 << 6;
}

/// The set of abutments the writer is licensed to leave standing.
///
/// A bitset rather than a two-value mode, because design spec §2's probe has
/// to price each cell *separately*: the last probe's finer split was cut for
/// being unreproducible, and a mode that only says "old" or "new" reproduces
/// exactly that failure. `CONSERVATIVE` is the empty set and renders byte for
/// byte what `main` renders.
#[doc(hidden)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ledger(u32);

impl Ledger {
    /// No abutment licensed: every rule collapses, as before this item.
    pub const CONSERVATIVE: Ledger = Ledger(0);

    /// What the writer ships with.
    pub const LICENSED: Ledger = Ledger(cell::EMPH_OVER_STRONG_WHOLE_RUN);

    /// Every cell, named, for a probe that measures them one at a time.
    pub const CELLS: &'static [(&'static str, u32)] = &[
        ("emph_over_strong_whole_run", cell::EMPH_OVER_STRONG_WHOLE_RUN),
        ("emph_over_strong_head_edge", cell::EMPH_OVER_STRONG_HEAD_EDGE),
        ("emph_over_strong_tail_edge", cell::EMPH_OVER_STRONG_TAIL_EDGE),
        ("strong_over_emph_head_edge", cell::STRONG_OVER_EMPH_HEAD_EDGE),
        ("strong_over_emph_tail_edge", cell::STRONG_OVER_EMPH_TAIL_EDGE),
        ("emph_beside_strong_run_seam", cell::EMPH_BESIDE_STRONG_RUN_SEAM),
        ("strong_beside_emph_run_seam", cell::STRONG_BESIDE_EMPH_RUN_SEAM),
    ];

    pub fn from_bits(bits: u32) -> Ledger {
        Ledger(bits)
    }

    pub fn bits(self) -> u32 {
        self.0
    }
}

/// The bit a triple corresponds to, or `None` when the triple is unlisted.
///
/// This `match` is the table. It reads no text, takes no buffer, and computes
/// nothing — every arm is a measured row (design spec §3.2), and the `None`
/// fallthrough is what keeps an unlisted triple conservative by default.
fn bit_for(outer: escape::Delim, inner: escape::Delim, site: Site) -> Option<u32> {
    use escape::Delim::{Emph, Strong};
    Some(match (outer, inner, site) {
        // `*` wrapping nothing but `**` prints `***x***`, and the merged run's
        // tie-break resolves em-outermost, which is what the IR meant.
        (Emph, Strong, Site::WholeRun) => cell::EMPH_OVER_STRONG_WHOLE_RUN,
        (Emph, Strong, Site::HeadEdge) => cell::EMPH_OVER_STRONG_HEAD_EDGE,
        (Emph, Strong, Site::TailEdge) => cell::EMPH_OVER_STRONG_TAIL_EDGE,
        (Strong, Emph, Site::HeadEdge) => cell::STRONG_OVER_EMPH_HEAD_EDGE,
        (Strong, Emph, Site::TailEdge) => cell::STRONG_OVER_EMPH_TAIL_EDGE,
        (Emph, Strong, Site::RunSeam) => cell::EMPH_BESIDE_STRONG_RUN_SEAM,
        (Strong, Emph, Site::RunSeam) => cell::STRONG_BESIDE_EMPH_RUN_SEAM,
        // Everything else, including every same-`Delim` pair at every site and
        // `Strong` over `Emph` spanning a whole run, stays collapsed.
        _ => return None,
    })
}

/// May a run of `inner`'s class stand adjacent to a run of `outer`'s class at
/// this site, or must it be collapsed?
fn may_abut(
    outer: escape::Delim,
    inner: escape::Delim,
    site: Site,
    ledger: Ledger,
) -> bool {
    bit_for(outer, inner, site).is_some_and(|b| ledger.0 & b != 0)
}
```

- [ ] **Step 4: Run the new tests to verify they pass**

Run: `cargo test -p kasane-writer --lib -- an_unlisted_triple_is_refused the_conservative_ledger every_named_cell 2>&1 | tail -20`
Expected: 3 passed. (The rest of the crate still compiles because nothing consumes the seam yet.)

- [ ] **Step 5: Thread the ledger through the render path**

Add `ledger: Ledger` as the **last** parameter of each of these, and pass it down unchanged:

`blocks_to_markdown_at`, `render_block`, `render_table`, `inlines_to_md`, `inlines_to_md_at`, `inlines_to_md_flat`, `emphasis_run`.

Then change the two entry points at the top of `markdown.rs`:

```rust
pub fn blocks_to_markdown(blocks: &[Block], assets: &AssetBag) -> String {
    blocks_to_markdown_with_ledger(blocks, assets, Ledger::LICENSED)
}

/// Render under a chosen ledger. A test seam, not API — the same
/// `#[doc(hidden)] pub` convention as `est_tokens` and `path_slug_of`, and for
/// the same reason: design spec §2's probe and §5's deep census both need
/// today's output *and* the licensed output in one process, and a copy of the
/// writer's rules in a test would drift.
#[doc(hidden)]
pub fn blocks_to_markdown_with_ledger(
    blocks: &[Block],
    assets: &AssetBag,
    ledger: Ledger,
) -> String {
    blocks_to_markdown_at(blocks, assets, 0, ledger)
}
```

`inlines_to_html` (`markdown.rs:164`, the merged-table HTML path) is **not** threaded — it emits HTML tags and reaches none of these rules. It is item 2b's territory.

Re-export from `crates/kasane-writer/src/lib.rs`, beside the existing `blocks_to_markdown` export:

```rust
#[doc(hidden)]
pub use markdown::{blocks_to_markdown_with_ledger, Ledger};
```

- [ ] **Step 6: Route `run_end` through the seam**

Replace the body of `run_end` (`markdown.rs:454-467`) with:

```rust
fn run_end(items: &[Flat<'_>], start: usize, ledger: Ledger) -> usize {
    let Some(d) = escape::delim(items[start].0) else {
        return start + 1;
    };
    let ch = d.ch();
    // The run's class, decided by its first *printing* member — `None` while
    // every member so far is vacuous. This is what makes a class-keyed seam
    // answerable from inside `run_end` without circularity: the walk is left
    // to right, so the first printing member is fixed before any later member
    // is considered (design spec §4.2's ordering note). While it is `None` the
    // seam is not consulted, which is correct — a run of purely vacuous
    // members prints nothing, so there is no abutment to license.
    let mut class_so_far = (!renders_empty(items[start].0, items[start].1)).then_some(d);
    let mut k = start + 1;
    while k < items.len() {
        let (el, depth) = items[k];
        if renders_empty(el, depth) {
            k += 1;
            continue;
        }
        let Some(next) = escape::delim(el) else { break };
        if next.ch() != ch {
            break;
        }
        if class_so_far.is_some_and(|cls| may_abut(cls, next, Site::RunSeam, ledger)) {
            break;
        }
        class_so_far = class_so_far.or(Some(next));
        k += 1;
    }
    k
}
```

Update its doc comment's final paragraph to say the run's class is now tracked here as well as re-derived by the emit loop, and cross-reference design spec §4.2. Update the call in `inlines_to_md_flat` to `run_end(items, i, ledger)`.

- [ ] **Step 7: Route both splice rules through the seam and delete `sole_child_nests_canonically`**

```rust
fn edge_to_splice(
    children: &[Flat<'_>],
    want: escape::Delim,
    ledger: Ledger,
) -> Option<usize> {
    let ch = want.ch();
    let printing = |&(i, d): &Flat<'_>| !renders_empty(i, d);
    let first = children.iter().position(printing);
    let last = children.iter().rposition(printing);
    [first, last].into_iter().flatten().find(|&idx| {
        let Some(inner) = escape::delim(children[idx].0) else {
            return false;
        };
        if inner.ch() != ch {
            return false;
        }
        let site = if first == last {
            Site::WholeRun
        } else if Some(idx) == first {
            Site::HeadEdge
        } else {
            Site::TailEdge
        };
        !may_abut(want, inner, site, ledger)
    })
}

fn same_delim_to_splice(
    children: &[Flat<'_>],
    want: escape::Delim,
    ledger: Ledger,
) -> Option<usize> {
    children.iter().position(|&(i, _)| {
        escape::delim(i) == Some(want) && !may_abut(want, want, Site::Interior, ledger)
    })
}
```

Delete `fn sole_child_nests_canonically` entirely (`markdown.rs:549-558`) and its doc comment. Two notes for the commit message, both real:

- The old caller invariant about `Backtick` is now enforced by construction. `edge_to_splice` binds `inner` and tests `inner.ch() != ch`; a `Backtick`'s character is `` ` ``, never `*`, so it returns `false` before any site is computed. The paragraph of the old doc that asked a reader to hold that invariant in their head can go.
- `sole_child_nests_canonically`'s three load-bearing conditions survive as table distinctions: its class check is the `(outer, inner)` key, its sole-printing-child check is `Site::WholeRun`, and its `want != Emph` guard is the absence of a `(Strong, Emph, WholeRun)` arm.

Update `splice_children`'s signature to take `ledger: Ledger`, pass it to both helpers, and pass it from `emphasis_run`'s `splice_children(run_children(members), want, ledger)` call. Rewrite the two long bullets in `splice_children`'s doc to point at `may_abut` as the single owner of the question, keeping the measured reasoning but not duplicating the table.

- [ ] **Step 8: Verify the whole suite is green and output is unchanged**

Run: `mise run lint && mise run test`
Expected: PASS, all crates.

Run: `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census && git diff --stat crates/kasane-writer/tests/`
Expected: **no diff at all.** This task is behaviour-neutral; any census movement here means the threading changed output and must be found before going further.

Run: `mise run census-ratchet`
Expected: PASS.

- [ ] **Step 9: Amend the spec to match the implementation**

The spec's §3.1 describes `may_abut` as a `match` with a `false` default and §5.1 describes `Ledger` as a two-value enum. The implementation is a bitset whose membership *is* the table, which is strictly more capable (§2's probe needs per-cell measurement) and keeps the same guarantee. Edit both sections to describe the bitset, keeping §3.3's reviewer instruction verbatim, and add one sentence to §5.1 saying why: a two-value mode cannot price cells separately, which is the specific failure that got the last probe's sub-split cut.

- [ ] **Step 10: Commit**

```bash
git add crates/kasane-writer/src/markdown.rs crates/kasane-writer/src/lib.rs \
        docs/superpowers/specs/2026-08-18-abutment-ledger-design.md
git commit -m "refactor(writer): route splice and fuse through one abutment ledger

Behaviour-neutral: LICENSED holds the single cell that reproduces
sole_child_nests_canonically, which this deletes. No census movement."
```

---

### Task 2: Extract the census oracle into a shared test module

Three tiers need the same oracle — the existing census, Task 3's probe, Task 4's deep tier. A second copy of `classify` would drift, which is the same argument `est_tokens` and `section::canonicalize_inlines` are `#[doc(hidden)] pub` for. Pure refactor: no behaviour change, no census movement.

**Files:**
- Create: `crates/kasane-writer/tests/census_support/mod.rs`
- Modify: `crates/kasane-writer/tests/census.rs`

**Interfaces:**
- Consumes: `kasane_writer::{blocks_to_markdown_with_ledger, Ledger}` (Task 1).
- Produces, from `census_support`:
  - `pub enum Emphasis { Em, St }`
  - `pub enum Structure { Clean, Corrupt, Inexpressible }`
  - `pub type ContextWalk = Vec<(char, Vec<Emphasis>)>`
  - `pub fn alphabet() -> Vec<Inline>`
  - `pub fn shapes() -> Vec<Vec<Inline>>`
  - `pub fn render(seq: &[Inline], ledger: Ledger) -> String`
  - `pub fn parser_options() -> Options`
  - `pub fn parsed_text(md: &str) -> String`
  - `pub fn parsed_context(md: &str) -> ContextWalk`
  - `pub fn ir_context(inlines: &[Inline], depth: usize, stack: &mut Vec<Emphasis>, out: &mut ContextWalk)`
  - `pub fn context_text(v: &[(char, Vec<Emphasis>)]) -> String`
  - `pub fn trim_whitespace(v: &[(char, Vec<Emphasis>)]) -> &[(char, Vec<Emphasis>)]`
  - `pub fn context_walks_with(seq: &[Inline], ledger: Ledger) -> Option<(ContextWalk, ContextWalk)>`
  - `pub fn classify_with(seq: &[Inline], ledger: Ledger) -> Structure`
  - `pub fn text_is_clean(seq: &[Inline], ledger: Ledger) -> bool`

- [ ] **Step 1: Create the shared module**

Create `crates/kasane-writer/tests/census_support/mod.rs`. Move these items out of `census.rs` verbatim, changing only visibility to `pub` and the two entry points noted below: `Emphasis`, `Structure`, `ContextWalk`, `alphabet`, `shapes`, `parser_options`, `parsed_text`, `parsed_context`, `trim_whitespace`, `ir_context`, `context_text`, `nests_same_class_directly`, `nests_strong_over_emph_directly`, `differs_only_by_erasure`. Keep every doc comment — they carry the reasoning the census depends on.

Head the file with:

```rust
//! The census oracle, shared by every census tier.
//!
//! Three test binaries render the same shapes through the same parser and ask
//! the same structural question: `census.rs` (lengths 1-3, the ratchet),
//! `census_probe.rs` (design spec §2's re-measurement), and `census_deep.rs`
//! (design spec §5's licensed-spelling tier). A copy of `classify` in any one
//! of them would drift from the others, which is the same reason
//! `section::canonicalize_inlines` is a `#[doc(hidden)] pub` seam rather than
//! a rule re-spelled in a test.
//!
//! Every tier renders through [`render`], which takes an explicit `Ledger`:
//! the probe and the deep tier both need today's output and the licensed
//! output in one process.

// Each tier uses a different subset of this module; Rust warns per test
// binary, not per workspace.
#![allow(dead_code)]
```

Add the imports the two new entry points need, beside those the moved items already carry:

```rust
use kasane_ir::{AssetBag, Block, Inline};
use kasane_writer::Ledger;
```

Add the two parameterized entry points:

```rust
/// Render one shape as a paragraph, under a chosen ledger.
pub fn render(seq: &[Inline], ledger: Ledger) -> String {
    kasane_writer::blocks_to_markdown_with_ledger(
        &[Block::Para(seq.to_vec())],
        &AssetBag::default(),
        ledger,
    )
}

/// Whether the text tier passes for this shape under this ledger.
///
/// Separate from [`classify_with`] on purpose: `classify_with` returns `Clean`
/// when the text is already corrupt, because the structural tier is gated on
/// the text tier and the text assertion names those shapes itself. A caller
/// that is not the ratchet — the deep tier — must ask both questions.
pub fn text_is_clean(seq: &[Inline], ledger: Ledger) -> bool {
    let md = render(seq, ledger);
    parsed_text(&md).trim() == kasane_gfm::rendered_text(seq).trim()
}
```

`context_walks_with` and `classify_with` are the existing `context_walks` and `classify` with a `ledger: Ledger` parameter threaded to `render`. Their bodies are otherwise unchanged.

- [ ] **Step 2: Point `census.rs` at the shared module**

At the top of `census.rs`, after the module doc:

```rust
mod census_support;

use census_support::{
    classify_with, context_text, context_walks_with, ir_context, shapes, trim_whitespace,
    Emphasis, Structure,
};
use kasane_writer::Ledger;
```

Delete the moved items from `census.rs`. It keeps its module doc, the four path constants, `INEXPRESSIBLE_HEADER`, `blessing`, `permanence_ceiling`, `ratchet`, and all seven `#[test]` functions. Replace every `classify(&seq)` with `classify_with(&seq, Ledger::LICENSED)` and every `context_walks(&seq)` with `context_walks_with(&seq, Ledger::LICENSED)`.

- [ ] **Step 3: Verify nothing moved**

Run: `mise run lint && mise run test`
Expected: PASS.

Run: `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census && git diff --stat crates/kasane-writer/tests/*.txt`
Expected: **no diff.** A refactor that moves a census file has changed the oracle.

- [ ] **Step 4: Commit**

```bash
git add crates/kasane-writer/tests/
git commit -m "test(writer): extract the census oracle for reuse across tiers"
```

---

### Task 3: The re-measurement probe

Design spec §2. Price each cell separately against both census files, commit the probe, and write the measured numbers into the spec. **No writer behaviour changes in this task.**

**Files:**
- Create: `crates/kasane-writer/tests/census_probe.rs`
- Modify: `docs/superpowers/specs/2026-08-18-abutment-ledger-design.md` (§2: replace the hypothesis with the measurement)

**Interfaces:**
- Consumes: `census_support::{shapes, classify_with, text_is_clean, Structure}`, `kasane_writer::Ledger`.
- Produces: no code other tasks consume. Its output is the numbers §2 records.

- [ ] **Step 1: Write the probe**

Create `crates/kasane-writer/tests/census_probe.rs`:

```rust
//! Design spec §2's re-measurement, committed rather than archived.
//!
//! The probe this replaces was a throwaway script, and its finer sub-split was
//! cut from `2026-08-16-cross-class-edge-splice-design.md` §6 for being
//! reproducible by nobody but its author. This one lives in the repo, prices
//! every cell separately, and is re-runnable by anyone:
//!
//! ```text
//! cargo test -p kasane-writer --test census_probe -- --ignored --nocapture
//! ```

mod census_support;

use census_support::{classify_with, shapes, text_is_clean, Structure};
use kasane_writer::Ledger;
use std::collections::BTreeSet;

const STRUCTURE_QUEUE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-known-structure-corrupt.txt"
);
const PERMANENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-inexpressible.txt"
);

/// The shape keys listed in one census file.
fn keys(path: &str) -> BTreeSet<String> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("{path} must exist"))
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[test]
#[ignore = "measurement, not an assertion: run with --ignored --nocapture"]
fn price_every_cell_against_both_census_files() {
    let queued = keys(STRUCTURE_QUEUE);
    let permanent = keys(PERMANENT);
    let all = shapes();

    println!("cell,file,newly_clean,newly_corrupt");
    let mut union = Ledger::CONSERVATIVE.bits();
    for (name, bit) in Ledger::CELLS {
        let ledger = Ledger::from_bits(*bit);
        let (mut q_clean, mut p_clean, mut broke) = (0usize, 0usize, 0usize);
        for seq in &all {
            let key = format!("{seq:?}");
            let clean = text_is_clean(seq, ledger)
                && classify_with(seq, ledger) == Structure::Clean;
            let was_clean = text_is_clean(seq, Ledger::CONSERVATIVE)
                && classify_with(seq, Ledger::CONSERVATIVE) == Structure::Clean;
            if clean && !was_clean {
                if queued.contains(&key) {
                    q_clean += 1;
                } else if permanent.contains(&key) {
                    p_clean += 1;
                }
            }
            if was_clean && !clean {
                broke += 1;
            }
        }
        println!("{name},queue,{q_clean},{broke}");
        println!("{name},permanent,{p_clean},{broke}");
        union |= bit;
    }

    // The combined figure, which is what design spec §2 records. It is not the
    // sum of the per-cell rows: one shape can be recovered by more than one
    // cell, and a cell can recover a shape only once another cell has stopped
    // a fusion from swallowing it.
    let ledger = Ledger::from_bits(union);
    let (mut q, mut p, mut broke) = (0usize, 0usize, 0usize);
    for seq in &all {
        let key = format!("{seq:?}");
        let clean =
            text_is_clean(seq, ledger) && classify_with(seq, ledger) == Structure::Clean;
        let was_clean = text_is_clean(seq, Ledger::CONSERVATIVE)
            && classify_with(seq, Ledger::CONSERVATIVE) == Structure::Clean;
        if clean && !was_clean {
            if queued.contains(&key) {
                q += 1;
            } else if permanent.contains(&key) {
                p += 1;
            }
        }
        if was_clean && !clean {
            broke += 1;
        }
    }
    println!("ALL_CELLS,queue,{q},{broke}");
    println!("ALL_CELLS,permanent,{p},{broke}");
    println!("ALL_CELLS,total_recovered,{},{broke}", q + p);
}
```

- [ ] **Step 2: Run the probe**

Run: `cargo test -p kasane-writer --test census_probe -- --ignored --nocapture 2>&1 | tail -25`
Expected: CSV rows, one per cell per file, plus three `ALL_CELLS` rows. Every `newly_corrupt` column should read `0`. **If any cell shows a non-zero `newly_corrupt`, stop** — that cell's table arm is wrong, and Tasks 4 and 5 must not turn it on until you know why.

- [ ] **Step 3: Record the measurement in the spec**

Edit §2 of `docs/superpowers/specs/2026-08-18-abutment-ledger-design.md`. Replace the "hypothesis: 924/390/534" framing with a table of the probe's actual per-cell and combined rows, and add one line naming the command that reproduces it. If the total differs from 924, say so explicitly and note that the archived probe was wrong — per §2, the measurement wins.

Also update §6's three "(hypothesis: N)" parentheticals to the measured numbers.

- [ ] **Step 4: Verify and commit**

Run: `mise run lint && mise run test`
Expected: PASS (the probe is `#[ignore]`d, so it compiles but does not run).

```bash
git add crates/kasane-writer/tests/census_probe.rs \
        docs/superpowers/specs/2026-08-18-abutment-ledger-design.md
git commit -m "test(writer): price every ledger cell against both census files"
```

---

### Task 4: License the splice-site cells, with the deep census that proves them

Turn on the four container-edge cells and build design spec §5's deep tier in the same task, because the tier is what makes the cells reviewable. Fix `splicing_mid_buffer_costs_a_span_that_would_round_trip`, which this task's change is expected to break.

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs` (`Ledger::LICENSED`; the pinned-loss test; new per-cell unit tests)
- Create: `crates/kasane-writer/tests/census_deep.rs`
- Modify: `crates/kasane-writer/tests/census-known-structure-corrupt.txt`, `census-inexpressible.txt`, `census-permanent-count.txt` (by bless)

**Interfaces:**
- Consumes: `census_support::{render, classify_with, text_is_clean, Structure}`, `kasane_writer::Ledger`, `Site`, `may_abut`, `cell::*`.
- Produces: `Ledger::LICENSED` with five cells set.

- [ ] **Step 1: Write the failing unit tests for the four edge cells**

Add to `mod tests` in `markdown.rs`:

```rust
/// `Emph` with a `Strong` at one edge and other content beside it. The
/// un-spliced spelling is the canonical one and round-trips (design spec §3.2,
/// rows 2, 3 and 6 of the edge-splice table).
#[test]
fn a_strong_at_one_edge_of_an_emph_run_keeps_its_delimiters() {
    let md = blocks_to_markdown(
        &[Block::Para(vec![Inline::Emph(vec![
            Inline::Text("a".into()),
            Inline::Strong(vec![Inline::Text("b".into())]),
        ])])],
        &AssetBag::default(),
    );
    assert_eq!(md.trim(), "*a**b***");
}

#[test]
fn a_strong_at_the_head_of_an_emph_run_keeps_its_delimiters() {
    let md = blocks_to_markdown(
        &[Block::Para(vec![Inline::Emph(vec![
            Inline::Strong(vec![Inline::Text("b".into())]),
            Inline::Text("a".into()),
        ])])],
        &AssetBag::default(),
    );
    assert_eq!(md.trim(), "***b**a*");
}

#[test]
fn an_emph_at_one_edge_of_a_strong_run_keeps_its_delimiters() {
    let md = blocks_to_markdown(
        &[Block::Para(vec![Inline::Strong(vec![
            Inline::Text("a".into()),
            Inline::Emph(vec![Inline::Text("b".into())]),
        ])])],
        &AssetBag::default(),
    );
    assert_eq!(md.trim(), "**a*b***");
}

#[test]
fn an_emph_at_the_head_of_a_strong_run_keeps_its_delimiters() {
    let md = blocks_to_markdown(
        &[Block::Para(vec![Inline::Strong(vec![
            Inline::Emph(vec![Inline::Text("b".into())]),
            Inline::Text("a".into()),
        ])])],
        &AssetBag::default(),
    );
    assert_eq!(md.trim(), "***b*a**");
}

/// The refusal that stays, and the reason the exemption is directional: a
/// `Strong` wrapping nothing but an `Emph` prints the same `***x***` and the
/// tie-break resolves it em-outermost, against the IR.
#[test]
fn a_strong_wrapping_only_an_emph_still_splices() {
    let md = blocks_to_markdown(
        &[Block::Para(vec![Inline::Strong(vec![Inline::Emph(vec![
            Inline::Text("b".into()),
        ])])])],
        &AssetBag::default(),
    );
    assert_eq!(md.trim(), "**b**");
}
```

- [ ] **Step 2: Run them to verify the four new ones fail**

Run: `cargo test -p kasane-writer --lib -- _edge_of_ _head_of_ a_strong_wrapping_only 2>&1 | tail -25`
Expected: the four edge tests FAIL with the spliced output (`*ab*`, `*ba*`, `**ab**`, `**ba**`); `a_strong_wrapping_only_an_emph_still_splices` PASSES already.

- [ ] **Step 3: Turn on the four cells**

In `markdown.rs`, change:

```rust
    pub const LICENSED: Ledger = Ledger(
        cell::EMPH_OVER_STRONG_WHOLE_RUN
            | cell::EMPH_OVER_STRONG_HEAD_EDGE
            | cell::EMPH_OVER_STRONG_TAIL_EDGE
            | cell::STRONG_OVER_EMPH_HEAD_EDGE
            | cell::STRONG_OVER_EMPH_TAIL_EDGE,
    );
```

Update `the_conservative_ledger_licenses_nothing_and_licensed_starts_at_one_cell`: rename it to `the_conservative_ledger_licenses_nothing`, drop the `count_ones() == 1` assertion, and keep the two `may_abut` assertions.

- [ ] **Step 4: Run the unit tests to verify they pass**

Run: `cargo test -p kasane-writer --lib 2>&1 | tail -25`
Expected: the four new tests PASS. `splicing_mid_buffer_costs_a_span_that_would_round_trip` (`markdown.rs:2421`) is expected to FAIL — it pins a loss this task recovers. Note its exact failure output; you rewrite it in Step 7.

- [ ] **Step 5: Write the deep census tier**

Create `crates/kasane-writer/tests/census_deep.rs`:

```rust
//! Design spec §5: every spelling this item newly licensed round-trips.
//!
//! A second corpus, seven delimiter-bearing elements at lengths 4 and 5. Each
//! shape renders twice — under `Ledger::LICENSED` and under
//! `Ledger::CONSERVATIVE` — and **only shapes whose renderings differ** are
//! kept and asserted clean. The kept set is therefore exactly the spellings
//! this item licensed, computed by differencing rather than by matching cases
//! someone anticipated. That is deliberate: design spec §4.3's flanking
//! interaction changes the rendering, so it lands in the kept set whether or
//! not anyone predicted it.
//!
//! No allowlist and no queue. A corrupt shape here is a wrong cell in
//! `may_abut`, not a residual.
//!
//! Lengths 4 and 5 rather than 3 because the licensed configurations appear at
//! length 3 but their *consequences* do not: a flanking flip needs a licensed
//! abutment with a neighbour on both sides (four elements), and a second
//! abutment whose flank class the first one's shortened run changed needs five.

mod census_support;

use census_support::{classify_with, render, text_is_clean, Structure};
use kasane_ir::Inline;
use kasane_writer::Ledger;

/// Seven elements, chosen to put both emphasis classes beside each other and
/// beside a non-emphasis delimiter. Smaller than the length-3 census's
/// nineteen because 7^5 is already 16,807 shapes.
fn deep_alphabet() -> Vec<Inline> {
    let t = |s: &str| Inline::Text(s.to_string());
    let em = |i: Inline| Inline::Emph(vec![i]);
    let st = |i: Inline| Inline::Strong(vec![i]);
    vec![
        t("a"),
        t("*"),
        Inline::Code("x".into()),
        em(t("a")),
        st(t("a")),
        em(st(t("a"))),
        st(em(t("a"))),
    ]
}

/// Every sequence of length 4 and 5 over [`deep_alphabet`].
///
/// 7^4 + 7^5 = 2,401 + 16,807 = 19,208 shapes. Each index is one digit of a
/// base-7 counter, which is simply an exhaustive product and is worth keeping
/// obviously correct: a subtly wrong generator would silently shrink the
/// corpus, and this tier's only guard against that is `kept > 0`.
fn deep_shapes() -> Vec<Vec<Inline>> {
    let a = deep_alphabet();
    let n = a.len();
    let mut out = Vec::new();
    for len in 4..=5u32 {
        for code in 0..n.pow(len) {
            let mut code = code;
            let mut seq = Vec::with_capacity(len as usize);
            for _ in 0..len {
                seq.push(a[code % n].clone());
                code /= n;
            }
            out.push(seq);
        }
    }
    out
}

#[test]
fn every_newly_licensed_spelling_round_trips() {
    let mut kept = 0usize;
    let mut bad: Vec<String> = Vec::new();

    for seq in deep_shapes() {
        let licensed = render(&seq, Ledger::LICENSED);
        if licensed == render(&seq, Ledger::CONSERVATIVE) {
            continue;
        }
        kept += 1;

        // Both tiers, and the text tier first: `classify_with` returns `Clean`
        // when the text is already corrupt, because the length-3 census gates
        // structure on text and names text corruption with its own assertion.
        // Nothing here names it, so this must ask both questions.
        if !text_is_clean(&seq, Ledger::LICENSED) {
            bad.push(format!("text: {seq:?} -> {licensed:?}"));
        } else if classify_with(&seq, Ledger::LICENSED) != Structure::Clean {
            bad.push(format!("structure: {seq:?} -> {licensed:?}"));
        }
    }

    // The filter is the tier. If it ever matches nothing -- a cell turned off,
    // a corpus that stopped reaching the licensed configurations -- this test
    // would pass vacuously and go on passing forever.
    assert!(
        kept > 0,
        "the differencing filter matched no shapes, so this tier proved nothing"
    );

    assert!(
        bad.is_empty(),
        "{} newly-licensed spelling(s) do not round-trip -- a cell in \
         `may_abut` is wrong, not a residual to be recorded:\n{}",
        bad.len(),
        bad.iter().take(10).map(|s| format!("  {s}\n")).collect::<String>()
    );
}
```

- [ ] **Step 6: Run the deep tier**

Run: `cargo test -p kasane-writer --test census_deep -- --nocapture 2>&1 | tail -25`
Expected: PASS. If it fails, a licensed cell is wrong — **do not add the shape to any allowlist**; this tier has none by design. Turn the offending cell back off, re-run Task 3's probe restricted to it, and fix the table arm.

Also record the wall-clock: `time cargo test -p kasane-writer --test census_deep`. If it pushes `mise run test` past the repo's PR budget, drop to length 4 only (change `4..=5` to `4..=4`) and **log the drop in the spec's §5.2 and in the PR body** — design spec §5.3 authorizes the fallback but not silently.

- [ ] **Step 7: Rewrite the pinned-loss test**

`splicing_mid_buffer_costs_a_span_that_would_round_trip` exists to pin a deliberate loss that this task recovers. Rewrite it to pin what is still lost — a same-`Delim` container in the middle of a buffer, which `may_abut(want, want, Interior)` still refuses — and add to its doc comment the sentence: **"The cross-class half of this trade is now licensed by `cell::STRONG_OVER_EMPH_HEAD_EDGE` / `TAIL_EDGE`; what remains pinned here is the same-`Delim` case, which no cell covers."**

If you cannot name a cell that covers the case the old assertion made, the test is right and the change is wrong — stop and re-open the table.

- [ ] **Step 8: Bless the census and read the diff**

Run: `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census`
Run: `git diff --stat crates/kasane-writer/tests/*.txt`

Expected: `census-known-structure-corrupt.txt` and `census-inexpressible.txt` both shrink; `census-permanent-count.txt` drops to the new permanent length; `census-known-corrupt.txt` **unchanged**.

Then check the one movement that needs justification:

```bash
git diff -U0 crates/kasane-writer/tests/census-inexpressible.txt | grep '^+' | grep -v '^+++' | grep -v '^+#'
```

Every line here is a shape newly claimed permanent. Per design spec §6, each must be named and justified individually in the PR body. If there are none, say so in the PR body explicitly.

- [ ] **Step 9: Verify green and commit**

Run: `mise run lint && mise run test && mise run census-ratchet`
Expected: all PASS.

```bash
git add crates/kasane-writer/src/markdown.rs crates/kasane-writer/tests/
git commit -m "feat(writer): license a cross-class container at one run edge

Four edge cells, plus the deep census tier that proves every newly
licensed spelling round-trips."
```

---

### Task 5: License the run-seam cells

Turn on the two `RunSeam` cells, so two adjacent runs of different classes stop fusing where the ledger says they may abut. Fix `fusing_adjacent_runs_costs_a_structural_boundary`, which this task's change is expected to break.

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs` (`Ledger::LICENSED`; the pinned-loss test; new unit tests)
- Modify: the three census `.txt` files (by bless)

**Interfaces:**
- Consumes: everything from Tasks 1 and 4.
- Produces: `Ledger::LICENSED` with all seven cells set.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `markdown.rs`:

```rust
/// Two adjacent runs of different classes no longer fuse: the boundary
/// between them survives into the printed line (design spec §4.2).
#[test]
fn adjacent_runs_of_different_classes_stay_separate() {
    let md = blocks_to_markdown(
        &[Block::Para(vec![
            Inline::Emph(vec![Inline::Text("a".into())]),
            Inline::Strong(vec![Inline::Text("b".into())]),
        ])],
        &AssetBag::default(),
    );
    assert_eq!(md.trim(), "*a***b**");
}

/// The `class_so_far` ordering note, directly: a vacuous leading member leaves
/// the class unset, so the seam is not consulted against it and the run
/// extends exactly as before. Pairs with
/// `a_vacuous_leading_member_does_not_downgrade_the_run_class`.
#[test]
fn a_vacuous_leading_member_does_not_decide_a_run_seam() {
    let md = blocks_to_markdown(
        &[Block::Para(vec![
            Inline::Emph(vec![]),
            Inline::Strong(vec![Inline::Text("b".into())]),
        ])],
        &AssetBag::default(),
    );
    assert_eq!(md.trim(), "**b**");
}

/// Same class still fuses: no `RunSeam` cell names a same-`Delim` pair, so
/// `bit_for` returns `None` and the two runs merge as they always have.
#[test]
fn adjacent_runs_of_the_same_class_still_fuse() {
    let md = blocks_to_markdown(
        &[Block::Para(vec![
            Inline::Emph(vec![Inline::Text("a".into())]),
            Inline::Emph(vec![Inline::Text("b".into())]),
        ])],
        &AssetBag::default(),
    );
    assert_eq!(md.trim(), "*ab*");
}
```

- [ ] **Step 2: Run them to verify the first fails**

Run: `cargo test -p kasane-writer --lib -- adjacent_runs a_vacuous_leading_member_does_not_decide 2>&1 | tail -25`
Expected: `adjacent_runs_of_different_classes_stay_separate` FAILS (prints `*ab*`); the other two PASS already.

- [ ] **Step 3: Turn on the two run-seam cells**

```rust
    pub const LICENSED: Ledger = Ledger(
        cell::EMPH_OVER_STRONG_WHOLE_RUN
            | cell::EMPH_OVER_STRONG_HEAD_EDGE
            | cell::EMPH_OVER_STRONG_TAIL_EDGE
            | cell::STRONG_OVER_EMPH_HEAD_EDGE
            | cell::STRONG_OVER_EMPH_TAIL_EDGE
            | cell::EMPH_BESIDE_STRONG_RUN_SEAM
            | cell::STRONG_BESIDE_EMPH_RUN_SEAM,
    );
```

- [ ] **Step 4: Run the unit tests and the deep tier**

Run: `cargo test -p kasane-writer --lib 2>&1 | tail -30`
Expected: the three new tests PASS. `fusing_adjacent_runs_costs_a_structural_boundary` (`markdown.rs:2399`) is expected to FAIL — it pins the loss this task recovers.

Run: `cargo test -p kasane-writer --test census_deep -- --nocapture 2>&1 | tail -20`
Expected: PASS, with a **larger kept set than in Task 4**. This is the run that covers design spec §4.3's flanking risk: shortening a run changes the following flank class, and if that pushes `emphasis_run` down its decline branch and costs text, it fails here. If it does fail, the failing lines prefixed `text:` are the flanking regression — report them rather than allowlisting them.

- [ ] **Step 5: Rewrite the pinned-loss test**

`fusing_adjacent_runs_costs_a_structural_boundary` pins a loss this task recovers. Rewrite it to pin what still fuses — two adjacent runs of the *same* class, which `adjacent_runs_of_the_same_class_still_fuse` already asserts — and add to its doc comment: **"The cross-class half of this trade is now licensed by `cell::EMPH_BESIDE_STRONG_RUN_SEAM` / `STRONG_BESIDE_EMPH_RUN_SEAM`; what remains pinned here is the same-class fuse, which no cell covers."** If the two tests now assert the same thing, delete the older one and say so in the commit message rather than keeping a duplicate.

- [ ] **Step 6: Bless, and read the diff the same way as Task 4**

Run: `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census`
Run: `git diff --stat crates/kasane-writer/tests/*.txt`
Expected: both queues shrink again; `census-known-corrupt.txt` unchanged.

Run: `git diff -U0 crates/kasane-writer/tests/census-inexpressible.txt | grep '^+' | grep -v '^+++' | grep -v '^+#'`
Expected: collect these for the PR body, per design spec §6.

- [ ] **Step 7: Verify green and commit**

Run: `mise run lint && mise run test && mise run census-ratchet`
Expected: all PASS.

```bash
git add crates/kasane-writer/src/markdown.rs crates/kasane-writer/tests/
git commit -m "feat(writer): stop fusing adjacent runs of different classes"
```

---

### Task 6: Documentation and merge readiness

The codebase map is how the next contributor learns this seam exists. It is also where the old rules are described in prose that is now wrong.

**Files:**
- Modify: `AGENTS.md` (the `kasane-writer` bullet and the census convention bullet)
- Modify: `docs/superpowers/specs/2026-08-18-abutment-ledger-design.md` (status line)

**Interfaces:**
- Consumes: the finished implementation.
- Produces: nothing code depends on.

- [ ] **Step 1: Update the `kasane-writer` bullet in `AGENTS.md`**

The paragraph beginning "Delimiter runs that share a character never abut in the printed line, by four rules" is now wrong in its first clause — runs that share a character *do* abut, where a cell licenses it. Rewrite that paragraph to describe the ledger: the four rules still exist, but the first three now ask `may_abut`, which is a table of licensed (outer class, inner class, site) triples with a `false` default, and the fourth (flanking) is unchanged. Keep the existing sentence about what CommonMark can express that the writer gives up, and add that the licensed set is what closed part of that gap without a delimiter-pairing mirror. Point at the design spec by filename.

- [ ] **Step 2: Update the census convention bullet in `AGENTS.md`**

The bullet describing the census's two tiers and four files gains a sentence naming the third tier: `census_deep.rs`, its differencing filter, and the fact that it has no allowlist because a failure there is a wrong ledger cell rather than a residual. Also note `census_support/mod.rs` as the shared oracle.

- [ ] **Step 3: Update the spec's status line**

Change `**Status:** Designed. Not implemented.` to `**Status:** Implemented on branch `abutment-ledger`.` — matching the convention the cross-class edge-splice spec uses.

- [ ] **Step 4: Full verification**

Run: `mise run lint && mise run test && mise run census-ratchet`
Expected: all PASS. Record the actual output; do not claim green without reading it.

Run: `cargo test -p kasane-writer --test census_probe -- --ignored --nocapture 2>&1 | tail -5`
Expected: the `ALL_CELLS` rows should now show `0` newly-clean against the current files — because the shapes they name have already left the files. This is a consistency check that the bless matched the measurement.

- [ ] **Step 5: Commit and open the PR**

```bash
git add AGENTS.md docs/superpowers/specs/2026-08-18-abutment-ledger-design.md
git commit -m "docs: record the abutment ledger in the codebase map"
git push -u origin abutment-ledger
```

The PR body must contain, per design spec §6 and §5.3:
1. The bless diff summary: how many shapes left each file.
2. **Every queue -> permanent movement, named and justified individually.** If there were none, say so explicitly — a reviewer needs to know the question was asked.
3. Whether the deep tier runs at lengths 4-5 or fell back to length 4, and the measured wall-clock.
4. The probe's `ALL_CELLS` numbers against the pre-change files, and whether they matched the archived probe's 924/390/534.

If `git push` fails with `gh: not found`, the stale credential helper is the cause — override it for that one command rather than editing global git config.

---

## Notes for the executor

- **The order is load-bearing.** Task 1 is behaviour-neutral on purpose: it establishes that the threading changed nothing, so every later census movement is attributable to a named cell. If Task 1's bless shows a diff, do not proceed — find out why.
- **Never allowlist a deep-tier failure.** It has no allowlist by design. A failure there means a cell is wrong.
- **The `newly_corrupt` column in Task 3's probe is the early warning** for the whole item. A non-zero value for a cell means turning that cell on will cost something; find out what before Task 4 or 5 turns it on.
