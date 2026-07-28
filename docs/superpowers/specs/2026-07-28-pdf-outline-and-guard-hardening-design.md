# kasane — PDF Outline and Path-Guard Hardening Design Spec

**Date:** 2026-07-28
**Status:** Approved (design), pending implementation plan
**Repo:** kasane
**Closes:** [#21](https://github.com/rahulmutt/kasane/issues/21), [#22](https://github.com/rahulmutt/kasane/issues/22)

## 1. Purpose & scope

The adapter-fuzzing item (spec `2026-07-27-adapter-fuzzing-design.md`) built the
fuzzing tier and scoped fixing what it found as follow-up work. It found two
bugs. Both were committed as reproducers and quarantined in `KNOWN_OPEN`
(`crates/kasane-adapters/tests/fuzz_corpus.rs`) so the stable suite stayed
green, which leaves the weekly `fuzz.yml` job red on both by design. This item
is that follow-up: fix both, un-quarantine both, and get the weekly fuzz run
green.

Diagnosing #21 turned up a third defect of the same class, behind the same call,
which this item also fixes (§2.2).

### Boundary

Two source files change: `crates/kasane-adapters/src/pdf/outline.rs` and
`crates/kasane-adapters/src/guard.rs`. Plus their tests, one new fuzz seed,
the `KNOWN_OPEN` list, README, and AGENTS.md. No IR type, core pass, writer
behavior, CLI surface, or dependency version changes.

### Non-goals

- **`safe_entry_name("")` returns `Some("")`.** Issue #22's closing note: the
  `guards` fuzz target asserts non-emptiness on `resolve_rel`'s output but not
  on `safe_entry_name`'s, and adding that assertion may surface a third
  finding. Deliberately deferred, recorded here so it is not lost (§7).
- **Fixing lopdf.** The upstream bug is real and worth reporting, but this item
  does not block on it and does not fork or patch the dependency (§6).
- **Salvaging a partial outline.** A rejected outline is dropped whole
  (§2.3).

## 2. #21 — stack overflow in the PDF adapter

### 2.1 Root cause

The recursion is not in kasane's code. `crates/kasane-adapters/src/pdf/doc.rs`
loads the reproducer without error; the abort happens entirely inside
`lopdf::Document::get_toc()`, which `outline.rs` calls at its only call site.

`lopdf::Document::get_outlines` (`src/outlines.rs`, lopdf 0.44.0) walks the
outline tree with neither a visited set nor a depth bound:

```rust
loop {
    // ... push this node's outline entry ...
    if let Ok(first) = node.get(b"First") {
        let sub_outlines = self.get_outlines(Some(first.clone()), ...)?;  // recurses
        // ...
    }
    node = match self.get_dict_in_dict(node, b"Next") {   // iterates
        Ok(n) => n,
        Err(_) => break,
    };
}
```

The committed reproducer
`fuzz/artifacts/pdf/crash-bf187532d0e5d3bae0e505fca2044d82067e55fd` contains:

```
8 0 obj
<< /Type /Outlines /First 8 0 R /Last 10 0 R /Count 2 >>
endobj
```

`/First` points at the object that declares it. The recursive arm never
terminates, the stack overflows, and the process aborts via SIGABRT. This is
uncatchable — no `Result` plumbing recovers from it, which is why the
reproducer had to be skipped rather than merely allowed to fail.

### 2.2 The second edge: `/Next`

The sibling walk is a `loop`, not recursion, so a `/Next` cycle does not
overflow — it spins forever, pushing an `Outline` onto a `Vec` every iteration.
A one-page document whose sole outline item sets `/Next` to its own object id
was confirmed to run `get_toc` past a 20-second timeout without returning.

This matters to the design, not just the changelog: **a depth bound alone fixes
only the `/First` edge.** Cycle detection is what covers both. The fuzzer has
not found this one — libFuzzer would report it as a timeout rather than a
crash — and it is not separately tracked upstream or here.

### 2.3 The fix

A new private function in `outline.rs`, called before `get_toc`:

```rust
const MAX_OUTLINE_DEPTH: usize = 64;
const MAX_OUTLINE_NODES: usize = 10_000;

/// True when the `/Outlines` graph is finite and small enough to hand to
/// `get_toc`. lopdf walks `/First` recursively and `/Next` iteratively with
/// neither a visited set nor a depth bound, so a cyclic outline either
/// overflows the stack (`/First`) or spins forever while growing a Vec
/// (`/Next`). The overflow aborts the process and cannot be caught, so the
/// graph must be proven finite *before* `get_toc` is called at all.
fn outline_is_traversable(doc: &Document) -> bool
```

It walks the same edge set lopdf walks — the catalog's `/Outlines`, then that
root's `/First` as the start node when it has one (otherwise the root dict
itself), then per node `/First` (descend) and `/Next` (sibling) — using an
**explicit work stack**, so kasane introduces no recursion
of its own while fixing a recursion bug. It carries:

- a `HashSet<ObjectId>` of every reference it resolves; a repeat visit is a
  cycle,
- a depth counter bounded by `MAX_OUTLINE_DEPTH`,
- a node counter bounded by `MAX_OUTLINE_NODES`.

Any repeat visit or exceeded cap returns `false`.

Cycle detection keys on `ObjectId` because only a reference can close a cycle;
an inline dictionary cannot refer to itself. Inline dictionaries can still
nest, but that nesting is bounded by file size and is covered by
`MAX_OUTLINE_DEPTH`. Both mechanisms are needed.

On the committed reproducer, the root `8 0 R` is marked visited when resolved
as the outline root, its `/First` resolves to `8 0 R` again, and the walk
rejects on the first edge.

The traversal must mirror lopdf's edge set, including the detail that lopdf
reassigns the start node to the root's `/First` when present before entering
its loop. A validator following a different edge set could let a cycle through.
The re-armed fuzz target is what keeps this honest.

`outline_by_page` gains one early return:

```rust
if !outline_is_traversable(doc) {
    return map;   // empty: "no outline"
}
```

### 2.4 Why an empty map is the right degradation

This is the same signal `outline_by_page` already produces for any `get_toc`
failure (`let Ok(toc) = doc.get_toc() else { return map };`). The adapter reads
an empty map as "no outline" at `pdf/mod.rs:50` and falls back to font-size
heading inference — a path that already exists and is already covered by
`font_size_fallback_when_no_outline`. A hostile outline therefore lands on
tested behavior rather than a new one.

Two consequences, stated deliberately:

- **A rejected outline is dropped whole, not truncated.** Salvaging the acyclic
  prefix would require producing headings from kasane's own traversal, which is
  a different design (see §8, approach B). Rejecting entirely keeps lopdf as
  the only thing that ever constructs a heading, so the good case keeps lopdf's
  UTF-16BE/LE title decoding, named-destination resolution, and `/A` GoTo
  handling unchanged.
- **The degradation is silent — no `Block::Raw` note.** This matches how
  `outline_by_page` already swallows `get_toc` errors. Unlike the math seam,
  where a dropped equation leaves a visible hole in the text, a dropped outline
  still yields headings via font-size inference, so a note would be noise.

## 3. #22 — `resolve_rel` leaks a `..` from `base_dir`

`resolve_rel` (`guard.rs`) normalizes `..` in its `target` argument but builds
its initial `parts` by splitting `base_dir` raw, so `base_dir`'s segments never
pass through the match loop and a `..` among them is emitted verbatim. That
defeats the confinement contract in the function's own doc comment.

The fix feeds both segment sources through the one loop:

```rust
pub fn resolve_rel(base_dir: &str, target: &str) -> Option<String> {
    // A package-absolute target resolves from the archive root, so base_dir is
    // not consulted at all.
    let base = if target.starts_with('/') { "" } else { base_dir };
    let mut parts: Vec<&str> = Vec::new();
    for seg in base.split('/').chain(target.split('/')) {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}
```

This is smaller than what it replaces. The `base_dir.is_empty()` special case
disappears — `"".split('/')` yields a single empty segment, which the `""` arm
already skips — and so do the two-branch initializer and the
`.filter(|s| !s.is_empty())`. One loop now governs every segment, which is
precisely the property whose absence was the bug.

Behavior changes, both in the intended direction:

| Call | Before | After |
|---|---|---|
| `resolve_rel("../a", "x")` | `Some("../a/x")` | `None` (the reproducer's case) |
| `resolve_rel("a/../b", "x")` | `Some("a/../b/x")` | `Some("b/x")` |

An escaping `base_dir` is **rejected**, not clamped, matching the doc comment.
All six existing assertions in `resolve_rel_normalizes_and_confines` still
hold, including the package-absolute case and both existing rejections.

No production call site changes behavior. There are seven, and every `base_dir`
reaching them is already confined: the literal `"ppt"` (`pptx/mod.rs:33`), or a
`parent_dir` of a path that itself either cleared `safe_entry_name` (which
rejects `..` as a substring) or was produced by an earlier `resolve_rel` — PPTX
slide paths, for instance, are `resolve_rel("ppt", …)` outputs before
`parent_dir` ever sees them (`pptx/mod.rs:33`, `:55`, `:65`).

The fix makes that invariant self-sustaining: once `resolve_rel` cannot emit a
`..` component, no `base_dir` derived from its output can carry one either. As
#22 says, this closes a violated postcondition in a pure helper, not a live
traversal.

## 4. Tests

### 4.1 Unit tests

**`outline.rs`** — four hostile documents, built in memory with lopdf's
`dictionary!` macro rather than loaded from fixture files, so the cycle is
visible in the test source instead of hidden in a binary blob:

1. `/First` self-cycle (the reproducer's shape),
2. `/Next` self-cycle (§2.2),
3. mutual `/First` cycle across two nodes — proves the visited set works on
   more than the degenerate self-edge,
4. a `/First` chain longer than `MAX_OUTLINE_DEPTH`.

Each asserts `outline_by_page` returns an empty map. The existing
`maps_outline_entries_to_pages` and `empty_when_no_outline` are the
counterweight: a guard that rejected every outline would pass all four new
tests and fail those two.

**`guard.rs`** — added to the existing `resolve_rel_normalizes_and_confines`
neighborhood: a `..`-bearing `base_dir` is rejected, and an interior `..` in
`base_dir` normalizes.

### 4.2 A limitation of the outline tests

**A regression in the `outline.rs` tests does not surface as an assertion
failure.** A `/First` regression aborts the test process — uncatchable, which
is the whole reason #21 needed quarantining rather than allowing a failure —
and a `/Next` regression hangs the suite until CI times out. There is no way to
convert an uncatchable abort into a clean red assertion. CI still goes red; it
goes red loudly rather than tidily. This is inherent to the bug class, not a
gap to close.

### 4.3 Fuzz corpus

The `/First` case already has its committed reproducer under
`fuzz/artifacts/pdf/`; un-quarantining re-arms it (§4.4).

The `/Next` hang gets a new hand-written `fuzz/seeds/pdf/outline-next-cycle.pdf`
— `seeds/`, not `artifacts/`, because the fuzzer did not find it. That
distinction is what the two directories mean in this repo: `artifacts/` holds
reproducers the fuzzer produced, `seeds/` holds hand-written starting inputs.

This creates `fuzz/seeds/pdf/`, the first seed directory for a whole-format
target. No plumbing is needed: `mise.toml:90` already copies
`fuzz/seeds/$target/*` into the corpus for any target that has such a
directory, and `tests/fuzz_corpus.rs` already replays all of `fuzz/seeds/**` on
stable in `mise run test`. `.gitattributes` already marks `fuzz/seeds/**` as
binary, so autocrlf cannot mutate it.

### 4.4 Un-quarantine

Both entries are removed from `KNOWN_OPEN`, which becomes `&[]`. The const and
its policy comment stay — they document the mechanism and will be needed the
next time the fuzzer finds something.

Nothing else in `fuzz_corpus.rs` needs changing: no test asserts a replay count,
and `known_open_entries_have_a_reproducer_on_disk` iterates an empty slice
harmlessly.

Removing these entries is what converts both reproducers back into permanent
regression tests, and per both issues it happens **as part of** the fix, not
before it.

## 5. Documentation

**README.** Delete the "Two findings are open this way today…" paragraph in the
Fuzzing section; it ends with "Delete this paragraph once both issues are
closed." The general quarantine-policy paragraph above it stays — it describes
the mechanism, not these two findings. In the PDF entry under *Known
limitations*, add one clause: an outline that is cyclic or implausibly large is
ignored, and headings fall back to font-size inference.

**AGENTS.md.** Extend the `outline.rs` clause in the codebase map — currently
"maps the `/Outlines` TOC to per-page headings" — to name the traversability
pre-check and why it exists. The map already documents untrusted-input bounds
this way for `ziputil.rs`, `guard.rs`, `capture_island`, and DjVu's
`MAX_RENDER_PIXELS`.

## 6. Upstream

`lopdf` 0.44.0 is the current release; there is no newer version to upgrade to
and no fix to pull in. The bug affects every caller of `get_toc`, not just
kasane.

Reporting it upstream is worthwhile: one issue covering both edges, with the
two minimal reproducers. It is explicitly **not** on this item's critical path
— the §2.3 guard closes #21 regardless of what upstream does — and it is an
outward-facing action on a third party's repository, so it requires the
maintainer's explicit go-ahead at the time rather than being covered by
approval of this spec. The same applies to offering an upstream PR.

Forking or patching lopdf was considered and rejected (§8, approach C).

## 7. Follow-up work recorded, not done

`safe_entry_name("")` returns `Some("")` — an empty name neither starts with
`/` nor contains `..`. The `guards` fuzz target asserts `!name.is_empty()` for
`resolve_rel`'s output but not for `safe_entry_name`'s. Issue #22 predicts that
adding the assertion may surface a third finding. Deliberately out of scope
here; this section exists so it is not lost.

## 8. Approaches considered

**A. Pre-flight validation, then call `get_toc` — chosen.** Self-contained, no
fork, no upstream wait. Preserves lopdf's title decoding, named-destination
resolution, and `/A` GoTo handling for the good case. One mechanism (cycle
detection) covers both the overflow and the hang. It follows the pattern
`math::capture_island` already established in this crate: bound the untrusted
structure *before* handing it to a recursive parser you do not control. Costs a
second traversal (negligible) and carries a mirror-drift risk against lopdf's
edge set, mitigated by the re-armed fuzz target.

**B. Replace `get_toc` with a bounded outline walk of our own.** One traversal,
no mirror-drift, full control. Rejected: it would require reimplementing UTF-16
title decoding, `/Dests` and `/Names` named-destination resolution, `/A` GoTo
actions, and page-id-to-page-number mapping — a substantial fidelity-regression
surface, for a bug about robustness rather than correctness.

**C. Patch lopdf and pin a fork.** Fixes it for every consumer with no
duplicated logic. Rejected as the closing mechanism: it puts a fork in the
dependency graph of a crate AGENTS.md already flags as manual-bump-only with no
automated security PRs, and it blocks on upstream review. Retained as a
non-blocking follow-up (§6).

## 9. Verification

`mise run lint && mise run test` is necessary but not sufficient — it proves
only that the stable replay is green. The proof this item actually needs runs
on the pinned nightly:

- `mise run fuzz guards` — previously crashed deterministically around run #3,
  so a clean run of a few thousand executions is a meaningful signal.
- `mise run fuzz pdf -- -max_total_time=120`.

Both must run clean before this work is called done. Then close #21 and #22.
