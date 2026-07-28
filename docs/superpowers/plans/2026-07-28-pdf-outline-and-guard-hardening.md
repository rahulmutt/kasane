# PDF Outline and Path-Guard Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the two fuzz findings the adapter-fuzzing item left quarantined (#21, #22) plus a third defect found during diagnosis, and get the weekly fuzz CI run green.

**Architecture:** #21's stack overflow is not in kasane's code — it is unbounded recursion inside `lopdf::Document::get_toc` on a cyclic `/Outlines` graph. The fix proves the outline graph finite in `pdf/outline.rs` *before* `get_toc` is ever called, degrading to the adapter's existing "no outline" path when it isn't. #22 is a one-loop rewrite of `guard::resolve_rel` so `base_dir`'s segments are normalized like `target`'s already were.

**Tech Stack:** Rust (edition 2021, stable pin 1.97), `lopdf` 0.44.0, `cargo-fuzz` on a pinned nightly, `mise` task runner.

**Spec:** `docs/superpowers/specs/2026-07-28-pdf-outline-and-guard-hardening-design.md` (commit `a9004e3`)

**Branch:** `outline-and-guard-hardening` (already created, spec already committed)

## Global Constraints

- Every change ships green under `mise run lint && mise run test`. `mise run lint` is `cargo fmt --all -- --check` plus `cargo clippy --workspace --all-targets -- -D warnings` — `--all-targets` matters, plain `cargo clippy` misses test and example code.
- **Edition 2021.** Let-chains (`if let Some(x) = y && cond`) are edition-2024 syntax and will NOT compile here. Use nested `if let`. All code in this plan is already written that way and has been verified against `cargo clippy --all-targets -- -D warnings`.
- A `KNOWN_OPEN` entry is removed **as part of the fix that closes it**, never before and never in a separate commit. That removal is what re-arms the reproducer as a permanent regression test.
- Commit messages follow the repo's conventional-commit style with an area scope: `fix(pdf):`, `fix(guard):`, `test(fuzz):`, `docs(...)`.
- `fuzz/seeds/**` and `fuzz/artifacts/**` are marked binary in `.gitattributes`. Do not add `-text` attributes; they are already there.
- Do not change any dependency version. `lopdf` stays at 0.44.0; no fork, no `[patch.crates-io]`.

---

### Task 1: `resolve_rel` confinement (closes #22)

**Files:**
- Modify: `crates/kasane-adapters/src/guard.rs:22-42` (the `resolve_rel` body) and its `#[cfg(test)] mod tests`
- Modify: `crates/kasane-adapters/tests/fuzz_corpus.rs:55-66` (remove the `guards` quarantine entry)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `pub fn resolve_rel(base_dir: &str, target: &str) -> Option<String>` — unchanged signature. Behavior change only: a `base_dir` containing a `..` that escapes the root now yields `None` instead of a path with a `..` in it.

- [ ] **Step 1: Write the failing tests**

Add these two tests to the existing `mod tests` in `crates/kasane-adapters/src/guard.rs`, next to `resolve_rel_normalizes_and_confines`:

```rust
    #[test]
    fn resolve_rel_rejects_escaping_base_dir() {
        // The #22 reproducer's shape: `..` in base_dir must pop like it does in
        // target, not pass through into the result.
        assert_eq!(resolve_rel("../a", "x"), None);
        assert_eq!(resolve_rel("..", "x"), None);
        assert_eq!(resolve_rel("a/../../b", "x"), None);
    }

    #[test]
    fn resolve_rel_normalizes_interior_base_dir() {
        // An interior `..` in base_dir normalizes rather than being emitted.
        assert_eq!(resolve_rel("a/../b", "x").as_deref(), Some("b/x"));
        assert_eq!(resolve_rel("a/./b", "x").as_deref(), Some("a/b/x"));
        // An empty base_dir still resolves against the archive root.
        assert_eq!(resolve_rel("", "a/b.xml").as_deref(), Some("a/b.xml"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-adapters --lib guard::tests -- --nocapture`

Expected: `resolve_rel_rejects_escaping_base_dir` FAILS with
`assertion \`left == right\` failed: left: Some("../a/x"), right: None`.
`resolve_rel_normalizes_interior_base_dir` FAILS on the `a/../b` case with
`left: Some("a/../b/x")`. The four pre-existing tests still pass.

- [ ] **Step 3: Write the implementation**

Replace the body of `resolve_rel` in `crates/kasane-adapters/src/guard.rs`. Keep the existing doc comment above it unchanged — it already describes this behavior; that is the point of the fix.

```rust
pub fn resolve_rel(base_dir: &str, target: &str) -> Option<String> {
    // A package-absolute target resolves from the archive root, so base_dir is
    // not consulted at all.
    let base = if target.starts_with('/') { "" } else { base_dir };
    let mut parts: Vec<&str> = Vec::new();
    // Both sources run through the SAME loop. Splitting base_dir raw was the
    // bug: its segments never saw the `..` arm, so a `..` passed straight
    // through into the result and defeated the confinement contract above.
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

Note what disappears: the `base_dir.is_empty()` special case (an empty string splits to one empty segment, which the `""` arm already skips), the two-branch initializer, and the `.filter(|s| !s.is_empty())`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kasane-adapters --lib guard::tests -- --nocapture`

Expected: PASS, all six tests in the module — the four pre-existing ones and the two new ones.

- [ ] **Step 5: Un-quarantine the `guards` reproducer**

In `crates/kasane-adapters/tests/fuzz_corpus.rs`, delete these four lines from the `KNOWN_OPEN` array (leave the `pdf` entry above them alone — Task 2 removes that one):

```rust
    // guards: `resolve_rel` leaks a ".." component from an unnormalized
    // `base_dir` -- it normalizes ".." in `target` but splits `base_dir` raw
    // (guard.rs:26).
    // Tracked in https://github.com/rahulmutt/kasane/issues/22.
    ("guards", "crash-135a36f60489a1f6461ea75e10caf336b27ec0df"),
```

Leave the `KNOWN_OPEN` declaration and its doc comment in place. It documents the quarantine policy and will be needed the next time the fuzzer finds something.

- [ ] **Step 6: Verify the reproducer now replays green**

Run: `cargo test -p kasane-adapters --test fuzz_corpus -- --nocapture`

Expected: PASS. The output must NOT contain
`SKIPPING quarantined reproducer fuzz/artifacts/guards/crash-135a...` any more —
that line's absence is the proof the reproducer is actually being replayed
rather than skipped. It should still print the `pdf` skip line.

- [ ] **Step 7: Run the full gate**

Run: `mise run lint && mise run test`

Expected: both green.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-adapters/src/guard.rs crates/kasane-adapters/tests/fuzz_corpus.rs
git commit -m "fix(guard): normalize base_dir segments in resolve_rel

resolve_rel normalized '..' in its target argument but built its initial
parts by splitting base_dir raw, so a '..' among base_dir's segments never
saw the match loop and passed straight through into the result. That
defeated the confinement contract in the function's own doc comment.

Feed both sources through the one loop. An escaping base_dir is now
rejected rather than clamped, and an interior '..' normalizes.

No production call site changes behavior: every base_dir in the tree is
the literal \"ppt\" or a parent_dir of a path that already cleared
safe_entry_name or an earlier resolve_rel. The fix makes that invariant
self-sustaining.

Un-quarantines the reproducer, re-arming it as a regression test.

Closes #22"
```

---

### Task 2: Outline traversability guard (closes #21, and the `/Next` hang)

**Files:**
- Modify: `crates/kasane-adapters/src/pdf/outline.rs` (imports, two consts, two new fns, one early return in `outline_by_page`, four new tests)
- Modify: `crates/kasane-adapters/tests/fuzz_corpus.rs` (remove the `pdf` quarantine entry)

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: `fn outline_is_traversable(doc: &lopdf::Document) -> bool` and `fn edge<'a>(doc: &'a Document, node: &'a Dictionary, key: &[u8]) -> Option<(Option<ObjectId>, &'a Dictionary)>`, both private to `pdf::outline`. `pub fn outline_by_page(doc: &Document) -> BTreeMap<u32, Vec<OutlineHeading>>` keeps its signature; it now returns an empty map for a cyclic or oversized outline as well as for an absent one.

**Background the implementer needs:** `lopdf::Document::get_outlines` walks the outline tree with neither a visited set nor a depth bound. `/First` is followed by *recursion*, so a `/First` cycle overflows the stack and aborts the process — uncatchable, no `Result` recovers from it. `/Next` is followed by an *iterative* `loop` that pushes onto a `Vec` each pass, so a `/Next` cycle hangs and grows memory instead. One mechanism — cycle detection keyed on `ObjectId` — covers both. A depth bound alone would fix only `/First`.

**Test ordering — read before starting.** The three cycle tests are black-box: they call the existing `outline_by_page` and go red against today's code. The two cap tests are white-box: they name `MAX_OUTLINE_DEPTH` and `outline_is_traversable`, which do not exist yet, so writing them now would be a *compile* error — and a compile error takes down the whole test module, including the three tests you need to watch go red first. So the cycle tests land in Step 1 and the cap tests in Step 4, after the implementation. Their red state is the compile error; there is no way to make them fail behaviorally first, because a deep *acyclic* chain does not crash lopdf at all.

- [ ] **Step 1: Write the three failing cycle tests**

Add to the `#[cfg(test)] mod tests` at the bottom of `crates/kasane-adapters/src/pdf/outline.rs`. The existing module already has `use super::*;`, `use crate::pdf::doc::open;`, and a `doc(name)` helper — keep those. Add this import at the top of the test module:

```rust
    use lopdf::{dictionary, Object};
```

Then add these helpers and three tests:

```rust
    /// An outline root whose /First points at itself — the shape of the #21
    /// reproducer. Drives lopdf's recursive arm.
    fn first_self_cycle() -> lopdf::Document {
        let mut doc = lopdf::Document::with_version("1.5");
        let root = doc.new_object_id();
        doc.objects.insert(
            root,
            Object::Dictionary(dictionary! { "Type" => "Outlines", "First" => root }),
        );
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Outlines" => root });
        doc.trailer.set("Root", cat);
        doc
    }

    /// A single outline item that is its own /Next sibling. Drives lopdf's
    /// iterative arm, which hangs rather than overflowing.
    fn next_self_cycle() -> lopdf::Document {
        let mut doc = lopdf::Document::with_version("1.5");
        let (root, item) = (doc.new_object_id(), doc.new_object_id());
        doc.objects.insert(
            item,
            Object::Dictionary(dictionary! {
                "Title" => Object::string_literal("Loop"),
                "Parent" => root,
                "Next" => item,
            }),
        );
        doc.objects.insert(
            root,
            Object::Dictionary(dictionary! { "Type" => "Outlines", "First" => item }),
        );
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Outlines" => root });
        doc.trailer.set("Root", cat);
        doc
    }

    /// Two nodes whose /First edges point at each other. Proves the visited set
    /// catches more than the degenerate self-edge.
    fn mutual_first_cycle() -> lopdf::Document {
        let mut doc = lopdf::Document::with_version("1.5");
        let (root, a, b) = (doc.new_object_id(), doc.new_object_id(), doc.new_object_id());
        doc.objects.insert(
            a,
            Object::Dictionary(dictionary! { "Title" => Object::string_literal("A"), "First" => b }),
        );
        doc.objects.insert(
            b,
            Object::Dictionary(dictionary! { "Title" => Object::string_literal("B"), "First" => a }),
        );
        doc.objects.insert(
            root,
            Object::Dictionary(dictionary! { "Type" => "Outlines", "First" => a }),
        );
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Outlines" => root });
        doc.trailer.set("Root", cat);
        doc
    }

    #[test]
    fn rejects_first_self_cycle() {
        assert!(outline_by_page(&first_self_cycle()).is_empty());
    }

    #[test]
    fn rejects_next_self_cycle() {
        assert!(outline_by_page(&next_self_cycle()).is_empty());
    }

    #[test]
    fn rejects_mutual_first_cycle() {
        assert!(outline_by_page(&mutual_first_cycle()).is_empty());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

**These tests do not fail cleanly, and that is the bug, not a mistake in the plan.** Run each one separately, because two of them take the whole test process down with them:

Run: `cargo test -p kasane-adapters --lib pdf::outline::tests::rejects_first_self_cycle`
Expected: the process ABORTS. stderr shows
`thread '...' has overflowed its stack` / `fatal runtime error: stack overflow, aborting`,
and the harness reports no test results at all. Exit code 134.

Run: `timeout 30 cargo test -p kasane-adapters --lib pdf::outline::tests::rejects_next_self_cycle`
Expected: HANGS until the timeout kills it. Exit code 124. No output after the test name.

Run: `timeout 30 cargo test -p kasane-adapters --lib pdf::outline::tests::rejects_mutual_first_cycle`
Expected: aborts with a stack overflow (exit 134) — mutual `/First` recurses the same way.

Do not proceed until you have observed all three. An abort and a hang are what
"red" looks like for this bug class; if any of the three passes cleanly, the
test is not reproducing the defect and the guard would be unverifiable.

- [ ] **Step 3: Write the implementation**

In `crates/kasane-adapters/src/pdf/outline.rs`, replace the two existing import lines

```rust
use lopdf::Document;
use std::collections::BTreeMap;
```

with

```rust
use lopdf::{Dictionary, Document, ObjectId};
use std::collections::{BTreeMap, HashSet};
```

Then add the constants and both functions above `outline_by_page`:

```rust
/// Caps on the `/Outlines` graph we are willing to hand to `get_toc`. Depth
/// bounds `/First` descent; the node count bounds the `/Next` sibling walk,
/// which is where lopdf grows a Vec one entry per iteration.
const MAX_OUTLINE_DEPTH: usize = 64;
const MAX_OUTLINE_NODES: usize = 10_000;

/// Resolve `key` in `node` to a dictionary, keeping the `ObjectId` when the
/// edge went through a reference. `Document::get_dict_in_dict` would resolve
/// the same edge but discards the id, and the id is exactly what cycle
/// detection needs. An inline dictionary yields `None` for the id — it cannot
/// refer to itself, so it cannot close a cycle.
fn edge<'a>(
    doc: &'a Document,
    node: &'a Dictionary,
    key: &[u8],
) -> Option<(Option<ObjectId>, &'a Dictionary)> {
    let obj = node.get(key).ok()?;
    let (id, resolved) = doc.dereference(obj).ok()?;
    Some((id, resolved.as_dict().ok()?))
}

/// True when the `/Outlines` graph is finite and small enough to hand to
/// `get_toc`. lopdf walks `/First` recursively and `/Next` iteratively with
/// neither a visited set nor a depth bound, so a cyclic outline either
/// overflows the stack (`/First`) or spins forever while growing a Vec
/// (`/Next`). The overflow aborts the process and cannot be caught, so the
/// graph must be proven finite *before* `get_toc` is called at all.
///
/// The walk uses an explicit stack, so fixing a recursion bug does not
/// introduce recursion of its own. It follows the same edges lopdf follows,
/// including lopdf's reassignment of the start node to the root's `/First`
/// when the root has one.
///
/// `visited` is global to the walk rather than per-path, so a node reachable
/// by two routes is rejected even though it is acyclic. That is deliberate:
/// an outline item has one `/Parent`, sharing is malformed, and the cost of
/// the stricter rule is a fallback to font-size headings.
fn outline_is_traversable(doc: &Document) -> bool {
    // No catalog or no /Outlines: get_toc fails harmlessly on its own.
    let Ok(catalog) = doc.catalog() else {
        return true;
    };
    let Some((root_id, root)) = edge(doc, catalog, b"Outlines") else {
        return true;
    };
    let start = edge(doc, root, b"First").unwrap_or((root_id, root));

    let mut visited: HashSet<ObjectId> = HashSet::new();
    let mut nodes = 0usize;
    let mut stack: Vec<((Option<ObjectId>, &Dictionary), usize)> = vec![(start, 1)];

    while let Some(((id, node), depth)) = stack.pop() {
        if depth > MAX_OUTLINE_DEPTH {
            return false;
        }
        nodes += 1;
        if nodes > MAX_OUTLINE_NODES {
            return false;
        }
        if let Some(id) = id {
            if !visited.insert(id) {
                return false; // already seen: the graph is cyclic
            }
        }
        if let Some(first) = edge(doc, node, b"First") {
            stack.push((first, depth + 1));
        }
        if let Some(next) = edge(doc, node, b"Next") {
            stack.push((next, depth));
        }
    }
    true
}
```

Then add the early return as the first statement in `outline_by_page`, before the
existing `let Ok(toc) = doc.get_toc() else { ... };`:

```rust
pub fn outline_by_page(doc: &Document) -> BTreeMap<u32, Vec<OutlineHeading>> {
    let mut map: BTreeMap<u32, Vec<OutlineHeading>> = BTreeMap::new();
    // A hostile outline is dropped whole. The empty map is the same signal
    // this function already produces for a get_toc error, and pdf/mod.rs
    // reads it as "no outline" and falls back to font-size inference.
    if !outline_is_traversable(doc) {
        return map;
    }
    let Ok(toc) = doc.get_toc() else {
        return map; // Error::NoOutline (or any error) -> no outline headings
    };
    // ...everything from `for entry in toc.toc {` to the end of the function
    // stays exactly as it is. The only edit is the four inserted lines above.
}
```

- [ ] **Step 4: Run the three cycle tests to verify they now pass**

Run: `timeout 120 cargo test -p kasane-adapters --lib pdf::outline`

Expected: PASS, five tests — the three new ones plus the pre-existing
`maps_outline_entries_to_pages` and `empty_when_no_outline`. Those last two are
the counterweight: a guard that rejected every outline would pass the three new
tests and fail them. No abort, no hang.

- [ ] **Step 5: Add the white-box cap tests**

`MAX_OUTLINE_DEPTH` and `MAX_OUTLINE_NODES` are otherwise untested — the node
cap in particular is the bound that stops the `/Next` memory growth, so leaving
it uncovered would be a real gap. Add this helper and test to the same test
module:

```rust
    /// An acyclic chain of `n` items linked by `key`: "First" makes it deep
    /// (exercising the depth cap), "Next" makes it wide (the node cap).
    fn chain(n: usize, key: &str) -> lopdf::Document {
        let mut doc = lopdf::Document::with_version("1.5");
        let ids: Vec<lopdf::ObjectId> = (0..n).map(|_| doc.new_object_id()).collect();
        for (i, id) in ids.iter().enumerate() {
            let mut d = dictionary! { "Title" => Object::string_literal("N") };
            if let Some(next) = ids.get(i + 1) {
                d.set(key, *next);
            }
            doc.objects.insert(*id, Object::Dictionary(d));
        }
        let root = doc.add_object(dictionary! { "Type" => "Outlines", "First" => ids[0] });
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Outlines" => root });
        doc.trailer.set("Root", cat);
        doc
    }

    #[test]
    fn caps_bound_acyclic_but_oversized_outlines() {
        // Depth: a /First chain one past the cap is rejected, one comfortably
        // inside it is not -- so the cap is a boundary, not a blanket refusal.
        assert!(!outline_is_traversable(&chain(MAX_OUTLINE_DEPTH + 1, "First")));
        assert!(outline_is_traversable(&chain(MAX_OUTLINE_DEPTH - 4, "First")));
        // Nodes: a /Next chain stays at depth 1, so only the node cap can fire.
        // This is the bound that stops lopdf growing a Vec per sibling.
        assert!(!outline_is_traversable(&chain(MAX_OUTLINE_NODES + 1, "Next")));
        assert!(outline_is_traversable(&chain(MAX_OUTLINE_NODES, "Next")));
    }
```

- [ ] **Step 6: Run the whole module**

Run: `timeout 300 cargo test -p kasane-adapters --lib pdf::outline`

Expected: PASS, six tests. The cap test builds a 10,001-object document twice,
so it is the slowest in the module — a few seconds in a debug build is normal.

- [ ] **Step 7: Un-quarantine the `pdf` reproducer**

In `crates/kasane-adapters/tests/fuzz_corpus.rs`, delete these five lines, leaving `KNOWN_OPEN` as an empty array `&[]`:

```rust
    // pdf: stack overflow in the PDF adapter (unbounded recursion).
    // Uncatchable -- aborts the whole test process rather than failing one
    // test, so it must be skipped rather than merely allowed to fail.
    // Tracked in https://github.com/rahulmutt/kasane/issues/21.
    ("pdf", "crash-bf187532d0e5d3bae0e505fca2044d82067e55fd"),
```

Keep the declaration and its doc comment. Nothing else in the file needs
changing: no test asserts a replay count, and
`known_open_entries_have_a_reproducer_on_disk` iterates an empty slice
harmlessly.

- [ ] **Step 8: Verify the reproducer now replays green**

Run: `timeout 120 cargo test -p kasane-adapters --test fuzz_corpus -- --nocapture`

Expected: PASS, and the output contains NO `SKIPPING quarantined reproducer`
lines at all — both are gone now. If the process aborts with a stack overflow
here, the guard is not catching the reproducer's shape; do not proceed.

- [ ] **Step 9: Run the full gate**

Run: `mise run lint && mise run test`

Expected: both green.

- [ ] **Step 10: Commit**

```bash
git add crates/kasane-adapters/src/pdf/outline.rs crates/kasane-adapters/tests/fuzz_corpus.rs
git commit -m "fix(pdf): bound the outline graph before handing it to get_toc

The stack overflow was not in kasane's code. lopdf's get_outlines walks
/First recursively and /Next iteratively with neither a visited set nor a
depth bound, so a self-referential outline root overflows the stack and
aborts the process -- uncatchable, which is why the reproducer had to be
skipped rather than merely allowed to fail.

Prove the graph finite before calling get_toc, using an explicit stack so
fixing a recursion bug adds no recursion. Cycle detection keyed on
ObjectId covers both edges; a depth bound alone would fix only /First.

A rejected outline yields the empty map this function already returns for
a get_toc error, which pdf/mod.rs reads as 'no outline' and degrades to
font-size heading inference -- an existing, tested path.

Diagnosis also found that a /Next cycle hangs and grows a Vec rather than
overflowing. Same guard, same commit; seed for it lands next.

Un-quarantines the reproducer, re-arming it as a regression test.

Closes #21"
```

---

### Task 3: Commit a fuzz seed for the `/Next` cycle

**Files:**
- Create: `fuzz/seeds/pdf/outline-next-cycle.pdf` (541 bytes, binary)

**Interfaces:**
- Consumes: the guard from Task 2. Without it this seed hangs the replay test.
- Produces: nothing consumed by later tasks.

**Why `seeds/` and not `artifacts/`:** the fuzzer did not produce this input — it was found by reading lopdf's source. In this repo `artifacts/` holds reproducers the fuzzer produced and `seeds/` holds hand-written starting inputs. No plumbing is needed: `mise.toml:90` already copies `fuzz/seeds/$target/*` into the corpus for any target that has such a directory, and `tests/fuzz_corpus.rs` already replays all of `fuzz/seeds/**` on stable. This creates `fuzz/seeds/pdf/`, the first seed directory for a whole-format target.

- [ ] **Step 1: Write the generator to a scratch path**

This script is a one-off — it is NOT committed, and it must be written OUTSIDE the repo so it does not end up in `git status`. Write it to `/tmp/make_next_cycle.py`. It lives here in the plan so the seed's provenance is recorded:

```python
#!/usr/bin/env python3
"""Emit fuzz/seeds/pdf/outline-next-cycle.pdf: a valid one-page PDF whose sole
outline item sets /Next to its own object id. lopdf's get_outlines walks /Next
in a loop with no visited set, so this input spins forever (and grows a Vec)
rather than overflowing the stack -- the sibling-edge twin of the /First
recursion in fuzz/artifacts/pdf/crash-bf18*. Stdlib only; mirrors the object/
xref bookkeeping in tests/fixtures/pdf/make_pdf_fixtures.py."""
import os, sys

objects, order = {}, []
def add(num, body):
    objects[num] = body; order.append(num)

add(1, b"<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>")
add(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
add(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>")
add(4, b"<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count 1 >>")
# /Next 5 0 R -- the item is its own sibling.
add(5, b"<< /Title (Loop) /Parent 4 0 R /Next 5 0 R /Dest [3 0 R /Fit] >>")

out = bytearray(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n")
offsets = {}
for num in order:
    offsets[num] = len(out)
    out += b"%d 0 obj\n" % num + objects[num] + b"\nendobj\n"
xref_pos = len(out)
max_num = max(order)
out += b"xref\n0 %d\n" % (max_num + 1) + b"0000000000 65535 f \n"
for num in range(1, max_num + 1):
    out += (b"%010d 00000 n \n" % offsets[num]) if num in offsets else b"0000000000 65535 f \n"
out += b"trailer\n<< /Size %d /Root 1 0 R >>\n" % (max_num + 1)
out += b"startxref\n%d\n%%%%EOF" % xref_pos

dest = sys.argv[1]
os.makedirs(os.path.dirname(dest), exist_ok=True)
with open(dest, "wb") as f:
    f.write(bytes(out))
print("wrote %s (%d bytes)" % (dest, len(out)))
```

- [ ] **Step 2: Generate the seed**

Run, from the repo root:

```bash
python3 /tmp/make_next_cycle.py fuzz/seeds/pdf/outline-next-cycle.pdf
```

Expected: `wrote fuzz/seeds/pdf/outline-next-cycle.pdf (541 bytes)`. If the byte
count differs from 541, the script was transcribed incorrectly — do not commit it.

- [ ] **Step 3: Verify the seed is well-formed and replays fast**

Run: `timeout 120 cargo test -p kasane-adapters --test fuzz_corpus -- --nocapture`

Expected: PASS, promptly. `replays_committed_seeds` now includes the new file.
A hang here means Task 2's guard is not covering the `/Next` edge.

- [ ] **Step 4: Confirm git treats the seed as binary**

Run: `git check-attr -a fuzz/seeds/pdf/outline-next-cycle.pdf`

Expected: output includes `text: unset` and `diff: unset`, from the existing
`fuzz/seeds/** -text -diff` rule in `.gitattributes`. No change to that file is
needed; this step only confirms the rule covers the new directory.

- [ ] **Step 5: Commit**

```bash
git add fuzz/seeds/pdf/outline-next-cycle.pdf
git commit -m "test(fuzz): seed the /Next outline cycle that hangs get_toc

lopdf's get_outlines walks /First recursively but /Next iteratively, so a
/Next cycle spins forever and grows a Vec instead of overflowing the
stack. The outline guard covers both edges; this pins the second one.

seeds/ rather than artifacts/: the fuzzer did not produce this input, it
came from reading lopdf's source. A hand-written starting input is what
seeds/ is for. mise.toml already copies fuzz/seeds/<target>/ into the
corpus and fuzz_corpus.rs already replays it on stable, so no plumbing
changes. This is the first seed directory for a whole-format target."
```

---

### Task 4: Documentation

**Files:**
- Modify: `README.md:71-79` (delete the open-findings paragraph) and the PDF entry under *Known limitations*
- Modify: `AGENTS.md` (the PDF `outline.rs` clause in the codebase map)

**Interfaces:**
- Consumes: Tasks 1, 2, and 3 must all be committed — the README paragraph describes both findings as open, so it cannot go until both are fixed.
- Produces: nothing.

- [ ] **Step 1: Delete the open-findings paragraph from the README**

Delete this entire paragraph (README.md lines 71-79, including the blank line after it). It ends by instructing exactly this:

```markdown
Two findings are open this way today: a stack overflow in the `pdf` adapter
([#21](https://github.com/rahulmutt/kasane/issues/21)), and a
path-confinement leak in `guards` ([#22](https://github.com/rahulmutt/kasane/issues/22),
`resolve_rel` normalizes `..` in its `target` argument but not in `base_dir`).
Their reproducers live under `fuzz/artifacts/{pdf,guards}/`. The quarantine
above only protects the stable `cargo test` run — `mise run fuzz`/`mise run
fuzz-all` still reproduce both crashes, so expect those two targets to fail
immediately, and expect the weekly fuzz CI run to be red on them, until
they're fixed. Delete this paragraph once both issues are closed.
```

Leave the paragraph directly above it (the one ending "…removing it from that
list is what re-arms the regression test once the bug is fixed.") in place — it
describes the quarantine mechanism in general, not these two findings.

- [ ] **Step 2: Add the outline fallback to the PDF known-limitation entry**

In `README.md`, find the bullet under *Known limitations (this build)* beginning
"PDF conversion is for born-digital PDFs." Append one sentence to it:

```markdown
An outline that is cyclic or implausibly large is ignored entirely and headings
fall back to font-size inference, because the underlying PDF library walks the
outline tree unbounded.
```

- [ ] **Step 3: Update the codebase map in AGENTS.md**

`AGENTS.md` contains two `outline.rs` clauses — one for the PDF adapter and one
for DjVu. Edit only the PDF one, identifiable by the `/Outlines` TOC wording.
Replace:

```markdown
`outline.rs` maps the `/Outlines` TOC to per-page headings
```

with:

```markdown
`outline.rs` maps the `/Outlines` TOC to per-page headings, but first proves the outline graph finite (`outline_is_traversable`: visited-set on `ObjectId`, plus depth and node caps) — `lopdf`'s own walk follows `/First` recursively and `/Next` iteratively with no bound, so a cyclic outline would abort the process or hang; a rejected outline degrades to font-size inference
```

- [ ] **Step 4: Verify nothing else references the closed findings**

Run: `grep -rn "KNOWN_OPEN\|issues/21\|issues/22" README.md AGENTS.md crates/ fuzz/ --include="*.md" --include="*.rs"`

Expected: the only remaining hits are the `KNOWN_OPEN` declaration, its doc
comment, and the `known_open_entries_have_a_reproducer_on_disk` test in
`crates/kasane-adapters/tests/fuzz_corpus.rs`. No hit should still describe
either finding as open. AGENTS.md's general fuzzing convention paragraph
(which mentions `KNOWN_OPEN` as a mechanism) stays.

- [ ] **Step 5: Run the full gate**

Run: `mise run lint && mise run test`

Expected: both green.

- [ ] **Step 6: Commit**

```bash
git add README.md AGENTS.md
git commit -m "docs(hardening): drop the open-findings notice, document the outline guard

Both quarantined findings are fixed and un-quarantined, so the README
paragraph announcing them goes -- it said to delete it once both issues
closed. The general quarantine-policy paragraph above it stays.

Record the user-visible consequence in the PDF known-limitation entry (a
cyclic or oversized outline falls back to font-size headings) and the
guard itself in the codebase map, alongside how the map already documents
ziputil.rs, guard.rs, capture_island and MAX_RENDER_PIXELS."
```

---

### Task 5: Verify on nightly and close the issues

**Files:** none — this task produces evidence, not code.

**Interfaces:**
- Consumes: Tasks 1-4, all committed.
- Produces: the fuzz-run evidence that justifies closing #21 and #22.

**Why this is a separate task:** `mise run lint && mise run test` proves only that the *stable replay* is green. It re-runs two fixed inputs. It does not prove the fuzzer stops finding these crashes, and the weekly `fuzz.yml` job runs on nightly, not stable. Both issues were "verified red on CI" before the fix; symmetry demands verifying green the same way.

- [ ] **Step 1: Confirm the pinned nightly toolchain is installed**

Run: `mise install && echo "$KASANE_FUZZ_TOOLCHAIN"`

Expected: prints the pinned nightly (`nightly-2026-07-01`). If `cargo +<toolchain> fuzz` is unavailable, stop and report — do not skip this task or substitute a different toolchain. The pin is a manual bump per AGENTS.md.

- [ ] **Step 2: Fuzz the `guards` target**

Run: `mise run fuzz guards -- -max_total_time=120`

Expected: no crash. This target previously reproduced #22 deterministically
around run #3, so surviving a couple of minutes and many thousands of
executions is a meaningful signal rather than a vacuous one.

- [ ] **Step 3: Fuzz the `pdf` target**

Run: `mise run fuzz pdf -- -max_total_time=120`

Expected: no crash, no `ERROR: libFuzzer: out-of-memory` and no timeout report.
The corpus is seeded from `tests/fixtures/pdf/*.pdf` plus the new
`fuzz/seeds/pdf/outline-next-cycle.pdf`, so the `/Next` input is in the starting
corpus and gets exercised on the first pass.

- [ ] **Step 4: Record the evidence and close the issues**

Substitute the real commit SHAs from Tasks 1 and 2 (`git log --oneline`) for
`<task-2-sha>` and `<task-1-sha>`:

```bash
gh issue close 21 --comment "Fixed in <task-2-sha>.

Root cause was not in kasane: lopdf's \`get_outlines\` walks \`/First\`
recursively with no visited set or depth bound, so the reproducer's
self-referential outline root (\`8 0 obj << /Type /Outlines /First 8 0 R >>\`)
overflowed the stack inside \`doc.get_toc()\`. \`outline.rs\` now proves the
outline graph finite before calling \`get_toc\` at all; a rejected outline
degrades to font-size heading inference.

Removed from KNOWN_OPEN, so the reproducer is a live regression test again.

Verified on the pinned nightly: \`mise run fuzz pdf -- -max_total_time=120\`
runs clean."

gh issue close 22 --comment "Fixed in <task-1-sha>.

\`resolve_rel\` now feeds \`base_dir\` and \`target\` through the same
normalization loop, so a \`..\` in \`base_dir\` pops instead of passing
through. An escaping \`base_dir\` is rejected rather than clamped.

As the issue noted, this was a violated postcondition rather than a live
traversal — no production call site changes behavior. The fix does make the
invariant self-sustaining: PPTX slide dirs are \`parent_dir\` of an earlier
\`resolve_rel\` output, so once the function cannot emit a \`..\`, no derived
\`base_dir\` can carry one.

Removed from KNOWN_OPEN, so the reproducer is a live regression test again.

Verified on the pinned nightly: \`mise run fuzz guards -- -max_total_time=120\`
runs clean (this target previously crashed around run #3)."
```

- [ ] **Step 5: Open the PR**

```bash
git push -u origin outline-and-guard-hardening
gh pr create --fill
```

Then follow the repo's merge convention: merge once CI is green, and delete the
branch only after the commits are confirmed on `main`.

---

## Follow-ups — require explicit approval, NOT part of this plan

Neither of these is covered by approval of this plan. Ask before doing either.

1. **Report the bug upstream to `lopdf`.** It affects every caller of `get_toc` on 0.44.0, the current release. One issue covering both edges, with the two minimal reproducers. This is an outward-facing action on a third party's repository, so it needs a specific go-ahead. Offering an upstream PR is a separate question again.
2. **The `safe_entry_name("")` lead.** `safe_entry_name("")` returns `Some("")`, and the `guards` fuzz target asserts `!name.is_empty()` for `resolve_rel`'s output but not for `safe_entry_name`'s. Issue #22 predicts adding that assertion may surface a third finding. Scoped out deliberately; see spec §7.
