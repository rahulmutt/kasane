# Declined-Run Rescan Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close `census-known-corrupt.txt`'s 32 shapes — the last shapes in this repo that lose *text* — by re-entering a declined emphasis run's children into the outer flat view instead of pushing them into the output buffer.

**Architecture:** `emphasis_run` stops returning a bare `String` and returns `RunOut::{Emitted, Declined}`. `inlines_to_md_flat` takes an owned working view plus a checkpoint stack, so on a decline it splices the children over the run's slot, rolls the output buffer back one run, and re-scans — which lets the exposed edge meet the neighbour that already printed beside it. Two supporting changes ship in the same branch because the code change is not reviewable without them: a repair to `mise run census-ratchet`, whose union excludes the text file and so reads this fix as a regression, and a new length-4 text tier asserting zero.

**Tech Stack:** Rust (stable, pinned in `mise.toml`), `pulldown-cmark` 0.13 as the census oracle's parser, `proptest` for the property tier, `mise` as the task runner, bash for the ratchet.

**Spec:** `docs/superpowers/specs/2026-08-21-declined-run-rescan-design.md`

## Global Constraints

- Branch: `declined-run-rescan`. The spec is already committed there as `eec5295`.
- Every change ships green under `mise run lint && mise run test`. `lint` is `cargo fmt --all -- --check` plus `cargo clippy --workspace --all-targets -- -D warnings`; plain `cargo clippy` is not the gate.
- `mise run census-ratchet` must be green by the end of Task 4, and is expected to FAIL between Tasks 2 and 4. Do not "fix" it by editing census files by hand.
- Census files are regenerated only by `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census`. Never hand-edit `census-known-corrupt.txt`, `census-known-structure-corrupt.txt`, or `census-inexpressible.txt`.
- `census-permanent-count.txt` must not change in this branch. It is 1984 at the base and 1984 at the end. If a bless lowers it, stop — that means a shape reached the permanent file, which this item does not do.
- Do not add a dependency. Everything needed is already in `crates/kasane-writer/Cargo.toml`.
- `cargo test --workspace` auto-discovers `crates/kasane-writer/tests/*.rs`. A new test file needs no manifest entry.
- Measured figures from the spec, for checking your work: text queue 32 → 0; structure queue 1698 → 1730; permanent entries 1984 → 1984; length-4 text-corrupt 1344 → 0.

---

### Task 1: `RunOut` and the forward-only rescan

Closes 16 of the 32. This is the parent spec's own proposal, landed on its own so the review record shows what it does and does not buy — spec §2.3.

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs:299-402` (`inlines_to_md_flat`), `:897-965` (`emphasis_run`)
- Test: `crates/kasane-writer/src/markdown.rs` (the crate's unit tests live in-file, at the bottom of the module)

**Interfaces:**
- Consumes: `Flat<'a> = (&'a Inline, usize)` (Copy), `splice_children(Vec<Flat<'a>>, escape::Delim, Ledger) -> Vec<Flat<'a>>`, `run_children(&[Flat<'a>]) -> Vec<Flat<'a>>`, `run_end(&[Flat<'_>], usize, Ledger) -> usize`, `Pos`.
- Produces: `enum RunOut<'a> { Emitted(String), Declined(Vec<Flat<'a>>) }` — private to `markdown.rs`. `emphasis_run` returns `RunOut<'a>` instead of `String`. Task 2 changes only the consumer side in `inlines_to_md_flat`.

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block at the bottom of `crates/kasane-writer/src/markdown.rs`. Follow the naming style already there (a full sentence, no `test_` prefix), and use the module's existing helpers rather than adding one:

- `para(inls: Vec<Inline>) -> String` (`markdown.rs:1813`) — the rendered paragraph line, no trailing newline.
- `recovered(inls: Vec<Inline>) -> String` (`markdown.rs:1826`) — what `pulldown-cmark` recovers from that line.

Assert on **both**: `recovered` is the invariant this item exists to protect, and `para` pins the spelling that achieves it. A test that checked only the spelling would pass on a line that happens to render wrong.

```rust
/// A declined run's children re-enter the outer view, so the code span it
/// exposed at its tail fuses with the one that follows instead of colliding
/// with it in the buffer.
///
/// `[Text("a"), Emph([Code("x")]), Code("x")]` used to print `` a*`x``x` ``,
/// in which a parser reads one code span over both backtick pairs and recovers
/// `ax``x` — text the IR never held. With the rescan the two spans meet in the
/// view, `run_end` groups them, and the line is `` a`xx` ``.
///
/// This is the tail half of the 32-shape family (design spec §2.3). The head
/// half needs Task 2's rollback and is pinned there.
#[test]
fn a_declined_runs_children_rejoin_the_view_and_fuse_forward() {
    let seq = vec![
        Inline::Text("a".into()),
        Inline::Emph(vec![Inline::Code("x".into())]),
        Inline::Code("x".into()),
    ];
    assert_eq!(para(seq.clone()), "a`xx`");
    assert_eq!(recovered(seq), "axx");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-writer --lib a_declined_runs_children_rejoin_the_view_and_fuse_forward`

Expected: FAIL. `para` returns ``"a*`x``x`"`` — the declined run's rendered children sitting in the buffer beside the following code span — and `recovered` returns `"ax``x"` rather than `"axx"`.

- [ ] **Step 3: Write minimal implementation**

In `markdown.rs`, immediately above the `#[allow(clippy::too_many_arguments)]` at line 897:

```rust
/// What one emphasis run contributed: either printed text, or the children it
/// declined to wrap, handed back for re-entry into the outer view.
///
/// A declined run printed no delimiter, so its children are not "the run's
/// contents" in any sense a parser can see — they are plain neighbours in the
/// printed line. Rendering them into the buffer asserts otherwise, and the
/// 32-shape backtick family is what that assertion cost (design spec §3.1).
enum RunOut<'a> {
    Emitted(String),
    Declined(Vec<Flat<'a>>),
}
```

Keep `#[allow(clippy::too_many_arguments)]` directly attached to `fn emphasis_run` — putting the enum between them detaches it and `mise run lint` fails on 8/7 arguments.

Change `emphasis_run`'s return type to `RunOut<'a>`, then its three exits:

```rust
// line 915
    if core.is_empty() {
        return RunOut::Emitted(inner);
    }
// line 937
    if opens && closes {
        RunOut::Emitted(emphasize(&inner, markup))
    } else {
// line 964 — the decline branch's final expression
        RunOut::Declined(children)
    }
```

`children` is the `Vec<Flat<'a>>` already bound at line 908; move it rather than recomputing. The rendered `inner` is dropped on that path.

In `inlines_to_md_flat`, take ownership of the view and handle the decline:

```rust
    // Owned, because a declined run rewrites it: the run's slot is replaced by
    // the children it would have wrapped, and the loop re-scans them in place.
    let mut items: Vec<Flat<'a>> = items.to_vec();
```

Then `run_end(items, i, ledger)` becomes `run_end(&items, i, ledger)`, and the emphasis arm becomes:

```rust
                match emphasis_run(
                    members, d, ctx, pos, markup, before_class, after_class, ledger,
                ) {
                    RunOut::Emitted(t) => s.push_str(&t),
                    RunOut::Declined(children) => {
                        items.splice(i..end, children);
                        continue;
                    }
                }
```

`continue` without advancing `i` is deliberate: the container is gone from the view, so the next pass groups whatever replaced it.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p kasane-writer --lib`

Expected: PASS, including the new test and every existing unit test in `markdown.rs`.

Then: `cargo test -p kasane-writer --test census`

Expected: FAIL, with `16 listed shape(s) are no longer corrupt` and `16 shape(s) newly structurally corrupt`, naming shapes of the form `[Text(_), Emph|Strong([Code("x")]), <backtick-bearing>]`. This failure is the task working. Do **not** bless here — Task 2 changes the number and one bless at the end is the diff a reviewer reads.

- [ ] **Step 5: Verify the split matches the spec**

Confirm the 16 survivors are the head half. The census failure output lists the shapes that *closed*; every one should start with `[Text(`. If any survivor starts with `[Text(`, or any closed shape does not, stop and re-read spec §2.3 — the mechanism is not what this plan assumes.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-writer/src/markdown.rs
git commit -m "refactor(writer): return a declined run's children instead of its text

\`emphasis_run\` now returns \`RunOut::{Emitted, Declined}\`, and
\`inlines_to_md_flat\` splices a declined run's children over its slot and
re-scans rather than pushing rendered text into the buffer.

This is the parent spec's own proposal (2026-08-15-emphasis-seam-design.md
§8), which predicted it closes all 32 shapes of the backtick family. It closes
16 -- the tail half. The head half needs a buffer rollback, which lands next;
see 2026-08-21-declined-run-rescan-design.md §2.3 for why a forward-only loop
cannot reach it.

The census is left failing deliberately: one bless at the end of the branch is
the diff a reviewer reads."
```

---

### Task 2: The checkpoint stack and rollback

Closes the remaining 16. Spec §3.2 and §3.3.

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs` (`inlines_to_md_flat` only)
- Test: `crates/kasane-writer/src/markdown.rs` (`mod tests`)

**Interfaces:**
- Consumes: `RunOut<'a>` from Task 1; `Pos` (`LineStart`, `AfterFootnoteRef`, `Mid`).
- Produces: no new public or crate-visible names. The checkpoint stack is a local in `inlines_to_md_flat`.

- [ ] **Step 1: Write the failing tests**

Three tests. The first is the head half; the second is the cascade that a single saved slot would mis-handle; the third is spec §10's risk 1, the `Pos` restore.

```rust
/// The head half of the 32-shape family: the collision is with output that has
/// already landed in the buffer, so the rescan has to reach backwards.
///
/// `[Code("x"), Emph([Code("x")]), Text("a")]` printed `` `x``x`a ``, which a
/// parser reads as one code span over `` x``x `` — recovering `x``xa` where the
/// IR said `xxa`. Rolling the buffer back one run lets the two spans meet in
/// the view and print `` `xx`a `` (design spec §2.3).
#[test]
fn a_declined_run_rolls_the_buffer_back_so_the_preceding_span_can_fuse() {
    let seq = vec![
        Inline::Code("x".into()),
        Inline::Emph(vec![Inline::Code("x".into())]),
        Inline::Text("a".into()),
    ];
    assert_eq!(para(seq.clone()), "`xx`a");
    assert_eq!(recovered(seq), "xxa");
}

/// A rollback can cascade: re-rendering the predecessor can make *it* decline,
/// because the view past it has changed. The checkpoint stack pops into that
/// run's own predecessor; a single saved slot handles one level and silently
/// mis-handles two (design spec §3.2).
#[test]
fn a_rollback_cascades_through_a_predecessor_that_then_declines() {
    let seq = vec![
        Inline::Code("x".into()),
        Inline::Emph(vec![Inline::Code("x".into())]),
        Inline::Strong(vec![Inline::Code("y".into())]),
        Inline::Text("a".into()),
    ];
    // The recovered text is the invariant; the spelling is how this writer
    // reaches it. Assert the invariant first -- if the spelling below ever
    // needs updating, this line is what says whether the update is legitimate.
    assert_eq!(recovered(seq.clone()), "xxya");
    assert_eq!(para(seq), "`xxy`a");
}

/// The checkpoint restores `Pos`, not just the buffer length. `Pos` has three
/// states because a `[^n]` that opened the line makes a following `:` a
/// footnote-definition delimiter, and the predecessor a rollback re-renders
/// must escape exactly as it did the first time (design spec §10, risk 1).
///
/// Measured: with the `pos = ppos` line removed and everything else intact,
/// this prints `` [^1]:`x`a ``, whose unescaped `[^1]:` at line start makes a
/// parser read the whole paragraph as a footnote *definition* -- `recovered`
/// comes back empty. Every one of the crate's other 168 unit tests passes in
/// that state. This test is the only thing that catches it.
#[test]
fn a_rollback_restores_the_escaping_position_it_rewound_past() {
    let seq = vec![
        Inline::FootnoteRef(NoteId(1)),
        Inline::Text(":".into()),
        Inline::Emph(vec![Inline::Code("x".into())]),
        Inline::Text("a".into()),
    ];
    // The `Emph` declines (its `*` sits between a `:` and a backtick), so the
    // `Text(":")` before it is re-rendered -- and must still be escaped,
    // because the position it re-renders at is `Pos::AfterFootnoteRef`.
    assert_eq!(para(seq.clone()), "[^1]\\:`x`a");
    assert_eq!(recovered(seq), "[^1]:xa");
}
```

Add `NoteId` to the test module's imports if it is not already there.

If the cascade test's `para` string turns out not to be `` `xxy`a ``, do not adjust the assertion to match the output. The `recovered` assertion above it is the one that must hold: it is `kasane_gfm::rendered_text` of the IR. A different spelling that still recovers `xxya` is a legitimate update to the `para` line; a spelling that recovers anything else is the bug this task exists to fix.

All four expected values above were measured against a working implementation, not derived — including the third test's, whose obvious formulation (`[FootnoteRef, Code, Emph([Code]), Text(":a")]`) does **not** decline at all and so would have passed without any rollback. If you change the shape, re-derive the values; do not assume a `Pos`-flavoured shape exercises this path just because it contains a `FootnoteRef`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-writer --lib a_declined_run_rolls a_rollback`

Expected: the first two FAIL — `a_declined_run_rolls_...` shows ``para`` returning ``"`x``x`a"`` and `recovered` returning `"x``xa"` instead of `"xxa"`; the cascade test fails the same way.

**The third test passes at this point, and that is correct.** `a_rollback_restores_the_escaping_position_it_rewound_past` renders identically before and after this task — on the base there is no rollback to restore anything, and the declined run's rendered children happen to produce the same bytes. It is a *guard on the rollback*, not a driver of it: its red state is the implementation with `pos = ppos` omitted, which Step 4 checks explicitly. Do not "fix" it by hunting for a variant that fails here; that variant does not exist, and the ones that look like it (§ the note below) do not decline at all.

- [ ] **Step 3: Write minimal implementation**

In `inlines_to_md_flat`, beside the existing locals:

```rust
    // One checkpoint per iteration: where the run started, how long the buffer
    // was before it, and the escaping position it opened at. A decline pops
    // its own and its predecessor's, so the exposed edge is re-scanned beside
    // the run that already printed next to it.
    //
    // A stack rather than one saved slot, because a rollback can cascade: the
    // re-rendered predecessor may itself decline, and the stack pops into
    // *its* predecessor (design spec §3.2).
    let mut marks: Vec<(usize, usize, Pos)> = Vec::new();
```

Push at the top of the loop, after `before` and `len_before` are bound and before anything renders:

```rust
        marks.push((i, len_before, before));
```

And in the decline arm from Task 1:

```rust
                    RunOut::Declined(children) => {
                        items.splice(i..end, children);
                        marks.pop();
                        // No predecessor means the run opened the view: there
                        // is nothing behind it to fuse with, so leave the
                        // buffer alone and re-scan from `i`. The splice above
                        // is what makes progress in that case.
                        if let Some((pi, plen, ppos)) = marks.pop() {
                            s.truncate(plen);
                            pos = ppos;
                            i = pi;
                        }
                        continue;
                    }
```

Termination: each decline permanently removes at least one emphasis container from `items`, containers are finite, and between declines `i` advances monotonically (spec §3.3).

- [ ] **Step 4: Run the tests, then verify the guard is not vacuous**

Run: `cargo test -p kasane-writer --lib`

Expected: PASS, all three new tests plus every existing one.

Now confirm the third test earns its place. Temporarily change `pos = ppos;` to `let _ = ppos;` in the decline arm and re-run `cargo test -p kasane-writer --lib`.

Expected: **exactly one failure**, `a_rollback_restores_the_escaping_position_it_rewound_past`, showing `para` as ``"[^1]:`x`a"`` and `recovered` as `""` — the unescaped `[^1]:` at line start turns the whole paragraph into a footnote definition and the text disappears. Every other test in the crate passes in that state. Restore `pos = ppos;` before continuing.

If more than one test fails, or if none does, stop: the guard is not measuring what §10's risk 1 describes.

Then: `cargo test -p kasane-writer --test census`

Expected: FAIL, now with `32 listed shape(s) are no longer corrupt` and `32 shape(s) newly structurally corrupt`. Still do not bless.

- [ ] **Step 5: Confirm the whole suite and the lint gate**

Run: `mise run test`

Expected: everything green except the census target, which fails with the 32/32 message above.

Run: `mise run lint`

Expected: exit 0. If clippy reports `this function has too many arguments (8/7)` on `emphasis_run`, the `#[allow(clippy::too_many_arguments)]` has been detached from it by the `RunOut` enum — move the enum above the attribute.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-writer/src/markdown.rs
git commit -m "fix(writer): roll the buffer back so a declined run's head seam is rescanned

The forward-only rescan closes the tail half of the backtick family and leaves
the head half, because \`inlines_to_md_flat\` walks forward and the element
before a declined run is already a substring of the output buffer.

One checkpoint per emitted run -- index, buffer length, \`Pos\` -- lets a
decline roll back to its predecessor and re-scan it against a view the
declined container has left. \`[Code(\"x\"), Emph([Code(\"x\")]), Text(\"a\")]\`
now prints \`\`\`xx\`a\`\`\` and recovers \`xxa\`.

A stack rather than one slot: a rollback cascades when the re-rendered
predecessor itself declines. Pinned by
a_rollback_cascades_through_a_predecessor_that_then_declines, which a single
saved slot passes every other test in the file but fails.

Design spec 2026-08-21-declined-run-rescan-design.md §3.2-§3.3."
```

---

### Task 3: Bless the census

The bless diff is the evidence a reviewer reads (spec §4).

**Files:**
- Modify: `crates/kasane-writer/tests/census-known-corrupt.txt`, `crates/kasane-writer/tests/census-known-structure-corrupt.txt` (both regenerated, never hand-edited)

**Interfaces:**
- Consumes: the writer behaviour from Task 2.
- Produces: an empty `census-known-corrupt.txt`, and 32 new lines in `census-known-structure-corrupt.txt`. Task 4's gate is written against exactly this move.

- [ ] **Step 1: Bless**

```bash
KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census
```

Expected: PASS, 9 tests.

- [ ] **Step 2: Read the diff, do not skim it**

```bash
git diff --stat crates/kasane-writer/tests/
```

Expected exactly: `census-known-corrupt.txt | 32 ------` and `census-known-structure-corrupt.txt | 32 ++++++`. Two files, nothing else.

If `census-inexpressible.txt` or `census-permanent-count.txt` appears in that diff, **stop and report it**. The permanent file is the one claim nothing downstream re-examines, and this item does not touch it.

- [ ] **Step 3: Verify the two sets are identical**

Task 4's gate rests on every shape entering the queue being a shape that left the text file. Check it rather than assume it:

```bash
export LC_ALL=C
git show origin/main:crates/kasane-writer/tests/census-known-corrupt.txt | awk '!/^#/ && NF' | sort -u > /tmp/t.base
awk '!/^#/ && NF' crates/kasane-writer/tests/census-known-corrupt.txt | sort -u > /tmp/t.head
git show origin/main:crates/kasane-writer/tests/census-known-structure-corrupt.txt | awk '!/^#/ && NF' | sort -u > /tmp/q.base
awk '!/^#/ && NF' crates/kasane-writer/tests/census-known-structure-corrupt.txt | sort -u > /tmp/q.head
comm -23 /tmp/t.base /tmp/t.head > /tmp/text_removed
comm -13 /tmp/q.base /tmp/q.head > /tmp/queue_added
echo "text_removed=$(wc -l < /tmp/text_removed)  queue_added=$(wc -l < /tmp/queue_added)"
echo "queue additions not justified by a text removal: $(comm -23 /tmp/queue_added /tmp/text_removed | wc -l)"
echo "text removals that did not enter the queue:      $(comm -23 /tmp/text_removed /tmp/queue_added | wc -l)"
```

Expected: `text_removed=32  queue_added=32`, and **0** on both of the last two lines. Anything else invalidates Task 4's design — stop and report.

- [ ] **Step 4: Confirm the ratchet fails, and why**

```bash
mise run census-ratchet
```

Expected: FAIL, with `queue 1698 1730 +32` and `union 3682 3714 +32`. This is the blind spot Task 4 repairs, not a problem with the bless. Record the table — it goes in the PR body.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-writer/tests/census-known-corrupt.txt \
        crates/kasane-writer/tests/census-known-structure-corrupt.txt
git commit -m "test(writer): bless the census -- the text queue is now empty

census-known-corrupt.txt 32 -> 0. All 32 move to the structure queue
(1698 -> 1730); the permanent file and its ceiling do not move.

That move is a promotion, not a regression. \`classify_with\` returns \`Clean\`
when the text is corrupt, because per-character structural alignment
presupposes equal strings -- so a text-corrupt shape is structurally
*unclassified*, not clean. Fixing its text is what makes the structural
question answerable, and these 32 answer it badly.

\`mise run census-ratchet\` fails here on the queue and union gates. That is a
blind spot in the ratchet, repaired next; see design spec §5."
```

---

### Task 4: Repair the ratchet

Spec §5. The union excludes the text file, so a text-corrupt shape — the worst state the census records — sits outside the gate entirely.

**Files:**
- Modify: `mise.toml`, `[tasks.census-ratchet]` (the union construction near line 140, and the `check queue` call near line 181)

**Interfaces:**
- Consumes: `$tmp/base.$tt` / `$tmp/head.$tt` (the text file's sorted shape sets), already built by the existing `for f in "$text" "$queue" "$perm"` loop.
- Produces: a `census-ratchet` that passes on this branch and still fails on a queue growth unmatched by a text removal.

- [ ] **Step 1: Write the failing check**

There is no unit-test harness for this bash task, so the test is a scripted assertion of both directions. Create `crates/kasane-writer/tests/ratchet_gate_cases.sh` — a documented, re-runnable check, not a throwaway:

```bash
#!/usr/bin/env bash
# Both directions of the queue gate, run against real history.
#
# The gate admits `queue_added \ text_removed`: a shape may enter the structure
# queue only if the same shape left census-known-corrupt.txt in the same
# commit. This script proves it accepts this branch's promotion and still
# rejects the case design spec §2b.4 of the abutment ledger recorded, where a
# shape entered the queue with the text file unchanged.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

echo "== direction 1: this branch's 32-shape promotion must PASS =="
mise run census-ratchet

echo
echo "== direction 2: a queue growth with no text removal must FAIL =="
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
q=crates/kasane-writer/tests/census-known-structure-corrupt.txt
cp "$q" "$tmp/q.orig"
printf '%s\n' '[Code("x"), Code("x"), Emph([Emph([Text("a")])])]' >> "$q"
LC_ALL=C sort -u -o "$q" "$q"
if mise run census-ratchet; then
  cp "$tmp/q.orig" "$q"
  echo "FAIL: the gate accepted a queue growth with no matching text removal" >&2
  exit 1
fi
cp "$tmp/q.orig" "$q"
echo
echo "both directions behaved correctly"
```

```bash
chmod +x crates/kasane-writer/tests/ratchet_gate_cases.sh
```

- [ ] **Step 2: Run it to verify direction 1 fails**

Run: `./crates/kasane-writer/tests/ratchet_gate_cases.sh`

Expected: FAIL at direction 1, on `queue ... +32` and `union ... +32`.

- [ ] **Step 3: Write minimal implementation**

Two edits in `mise.toml`'s `[tasks.census-ratchet]`.

The union gains the text file. Find:

```bash
  LC_ALL=C sort -u "$tmp/$side.$tq" "$tmp/$side.$tp" > "$tmp/$side.union"
```

Replace with:

```bash
  LC_ALL=C sort -u "$tmp/$side.$tq" "$tmp/$side.$tp" "$tmp/$side.$tt" > "$tmp/$side.union"
```

and update the comment above that loop, which currently states the union's premise as "a shape may move between the queue and the permanent file":

```bash
# The union is the assertion that actually forbids a regression: a shape may
# move between any two of the three files, but no shape may become corrupt
# that was not. Checking the files separately would let a regression hide as a
# reclassification.
#
# The text file is IN the union, and that is load-bearing rather than tidy.
# `classify_with` returns `Clean` when the text is corrupt, so a text-corrupt
# shape is structurally *unclassified* -- it is in the worst state this census
# records and, without this line, in none of the sets that gate. Fixing its
# text would then read as a fresh corruption. Design spec
# 2026-08-21-declined-run-rescan-design.md §5.1.
```

Then the queue gate. Replace:

```bash
check queue "$tmp/base.$tq"    "$tmp/head.$tq"    gate   "$tmp/skip.$tq"
```

with a gate that subtracts the justified moves first:

```bash
# The queue may grow only by shapes that left the text file in the same commit.
# Text-corrupt -> structure-corrupt is a promotion: text is the invariant and
# the span boundary is not, so a shape that stops losing text and starts losing
# only structure has got better. Every other queue growth still fails.
#
# This keeps the one case on record where the queue gate alone spoke (abutment
# ledger spec §2b.4): there the text file was unchanged, so `text_removed` is
# empty, every addition is unjustified, and it still fails. Both directions are
# pinned by crates/kasane-writer/tests/ratchet_gate_cases.sh.
comm -23 "$tmp/base.$tt" "$tmp/head.$tt" > "$tmp/text_removed"
comm -13 "$tmp/base.$tq" "$tmp/head.$tq" > "$tmp/queue_added"
comm -23 "$tmp/queue_added" "$tmp/text_removed" > "$tmp/queue_unjustified"
check queue "$tmp/base.$tq" "$tmp/head.$tq" report "$tmp/skip.$tq"
if [ -s "$tmp/queue_unjustified" ]; then
  fail=1
  printf '%-8s %8s %8s %8s   %s\n' "queue+" "" "" \
    "$(wc -l < "$tmp/queue_unjustified" | tr -d ' ')" \
    "FAIL -- queue growth with no matching text removal"
  sed -n '1,10p' "$tmp/queue_unjustified" | sed 's/^/           /'
fi
```

The `check queue ... report` line keeps the queue's own row in the table — the count is still worth seeing — while the gating moves to the `queue+` row below it.

- [ ] **Step 4: Run it to verify both directions**

Run: `./crates/kasane-writer/tests/ratchet_gate_cases.sh`

Expected: PASS. Direction 1 shows `union 3714 3714 +0` and `queue 1698 1730 +32   32 moved in (not gated)` with no `queue+` row; direction 2 fails with a `queue+` row naming `[Code("x"), Code("x"), Emph([Emph([Text("a")])])]`.

- [ ] **Step 5: Confirm the ordinary gates**

Run: `mise run lint && mise run test && mise run census-ratchet`

Expected: all three exit 0.

- [ ] **Step 6: Commit**

```bash
git add mise.toml crates/kasane-writer/tests/ratchet_gate_cases.sh
git commit -m "test: let the census ratchet see a text fix as a fix

The union was queue + permanent, excluding the text file. But
\`classify_with\` gates structure on text, so a text-corrupt shape is
structurally unclassified: it is in the worst state the census records and in
none of the sets that gate. Fixing its text moved it from invisible to counted
and the union read a strict improvement as +32.

The union now spans all three files (3714 -> 3714, +0), and the queue gate
admits \`queue_added \\ text_removed\` -- a shape may enter the structure queue
only if the same shape left census-known-corrupt.txt in the same commit.

This is the mirror of the abutment branch's most transferable finding: there,
structural gates stayed silent through thousands of text losses; here, a
structural gate cried regression at a text fix. Both follow from the two tiers
being ordered while the gates treated them as peers.

ratchet_gate_cases.sh pins both directions, including §2b.4's case, which the
relaxed gate still rejects.

Design spec §5."
```

---

### Task 5: The length-4 text tier

Spec §6. The guard whose absence the abutment branch calls its most transferable finding.

**Files:**
- Create: `crates/kasane-writer/tests/census_len4.rs`
- Test: itself

**Interfaces:**
- Consumes: `census_support::{alphabet, text_is_clean}`, `kasane_writer::Ledger`, `kasane_ir::Inline`.
- Produces: one `#[test] fn no_shape_of_length_four_loses_text()`. No files, no allowlist, no bless hook.

- [ ] **Step 1: Write the test**

This tier asserts zero, so the test *is* the implementation — there is no separate red step beyond confirming it would have failed before Tasks 1–2. Create `crates/kasane-writer/tests/census_len4.rs`:

```rust
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
//! cannot express" when it meant "this alphabet cannot express", and 88% of it
//! was wrong.

mod census_support;

use census_support::{alphabet, text_is_clean};
use kasane_ir::Inline;
use kasane_writer::Ledger;

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
                    corrupt.iter().take(20).cloned().collect::<Vec<_>>().join("\n  ")
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
```

- [ ] **Step 2: Run it**

Run: `cargo test -p kasane-writer --test census_len4`

Expected: PASS, in roughly 2-3 seconds debug.

- [ ] **Step 3: Verify it would have caught the bug**

A guard that has never been seen red is not evidence. Confirm against the base:

```bash
git stash push -- crates/kasane-writer/src/markdown.rs
cargo test -p kasane-writer --test census_len4
git stash pop
```

Expected: FAIL on the stashed (pre-Task-1) writer, reporting **1344** shapes. Then PASS again after the pop. If the count is not 1344, stop — the tier is not measuring what spec §2.2 measured.

- [ ] **Step 4: Confirm the wall-clock cost**

```bash
mise run test
```

Expected: exit 0. The suite grows by roughly 2.3s against a ~7.9s baseline. Record the actual delta for the PR body; spec §6 commits to +29% and a materially larger number is worth reporting rather than absorbing.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-writer/tests/census_len4.rs
git commit -m "test(writer): gate the text tier at length 4

Asserts zero over the census's own 19-element alphabet -- no allowlist, no
bless, no ratchet interaction, because the measured answer is 0 and the rescan
is what makes it 0.

This is the guard the abutment branch's §2b.5 calls its most transferable
missing finding: that branch's structural counter read 0 in every row of every
table while text losses ran into the thousands, because the census stops at
length 3 and the losses lived at length >= 4.

Verified red before the rescan (1344 shapes) and green after. Costs ~2.3s
against a ~7.9s suite.

Design spec §6."
```

---

### Task 6: Correct the records this branch falsifies

Spec §7. These currently assert the opposite of what ships. Corrected in place, not deleted — the treatment `census-inexpressible.txt`'s header got on 2026-08-17.

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs` (the decline branch's comment, around line 940-963)
- Modify: `docs/superpowers/specs/2026-08-15-emphasis-seam-design.md` (§8's first residual bullet)
- Modify: `AGENTS.md` (the four-rules paragraph in the `kasane-writer` entry; the census bullet under Conventions)

**Interfaces:**
- Consumes: everything above. This task adds no code.
- Produces: nothing further tasks depend on.

- [ ] **Step 1: Rewrite the decline branch's comment**

The existing comment's closing argument is that the exposed seam is left unscanned on a measured claim, that no census shape is corrupt *only* because of this decline, and that "a future shape corrupt only through this seam would not be caught by anything here." All three are now false. Keep its worked example — `[Code("x"), Emph([Code("x")]), Text("a")]` printing `` `x``x`a `` — because it is now the description of what the rescan fixes. Replace the argument with:

```rust
        // The delimiter would not flank where it lands, so a parser would read
        // it as a literal asterisk in the middle of the prose. The text is the
        // invariant and the span is not: hand the children back to the outer
        // view rather than printing them (design spec §2.3).
        //
        // Handing them back rather than rendering them is what closed the
        // 32-shape backtick family. A declined run prints no delimiter, so its
        // children are plain neighbours in the printed line, and rendering them
        // into the buffer asserted otherwise: for
        // `[Code("x"), Emph([Code("x")]), Text("a")]` the buffer held
        // `` `x``x`a ``, in which a parser reads one code span over both
        // backtick pairs and recovers `x``xa` where the IR said `xxa`.
        //
        // `inlines_to_md_flat` re-scans the seam in both directions -- forward
        // by splicing over the run's slot, backward by rolling the buffer back
        // one run -- because a forward-only rescan closes only the half whose
        // collision is with what follows (design spec §2.3, measured at 16 of
        // 32). `census_len4.rs` is the guard: zero text loss over the census
        // alphabet at length 4, where the census's own tiers stop at 3.
```

- [ ] **Step 2: Correct the emphasis-seam spec's §8 residual bullet**

That bullet says 32 shapes remain, that a later item works them down, and that "the shape of that fix is already known: a re-scan … closing the whole residual set". Append a status block in the style §8's own result note uses, rather than editing the prediction away:

```markdown
> **Closed, 2026-08-21, and this bullet's prediction was measured wrong.** The
> re-scan as described above — children re-entering the outer view before run
> detection — closes **16** of the 32, not the whole set. The survivors are the
> head half, `[backtick-bearing, Emph|Strong([Code]), Text]`, because
> `inlines_to_md_flat` walks forward and the element before a declined run is
> already a substring of the output buffer, so restarting the scan can reach
> what follows and never what precedes. Adding a one-run buffer rollback closes
> all 32 and takes text corruption to zero at every length through 6 over the
> census alphabet. See `2026-08-21-declined-run-rescan-design.md` §2.3 for the
> per-length table and §3.2 for the mechanism. `census-known-corrupt.txt` is
> now empty.
```

- [ ] **Step 3: Correct AGENTS.md's four-rules paragraph**

That paragraph ends by describing the fourth rule — the flanking decline — as one that prints children bare. Amend that clause so it describes what ships:

> and a delimiter that would fail to flank on either side where it lands is not
> emitted at all, its children returning to the flattened view rather than
> being printed there. That return is what closed the last text-losing family
> in the census: a declined run prints no delimiter, so its children are plain
> neighbours in the printed line, and `inlines_to_md_flat` re-scans that seam in
> both directions — forward by splicing over the run's slot, backward by rolling
> the output buffer back one run. Forward alone closes only half of it
> (`2026-08-21-declined-run-rescan-design.md` §2.3).

- [ ] **Step 4: Correct AGENTS.md's census bullet**

Two edits under Conventions. The bullet beginning "`crates/kasane-writer/tests/census-known-corrupt.txt` is a ratchet, not a todo list" describes a file with entries in it; note that it is now empty and that the mechanism stands:

> It is **empty** as of 2026-08-21 — the rescan closed the last family — and
> the ratchet still stands: a newly corrupt shape fails the build until someone
> blesses it in, which is now a strictly visible act against an empty file.

Then extend the four-files paragraph where it describes `mise run census-ratchet`, adding after the union sentence:

> The union spans all three shape files including the text queue, and the queue
> gate admits a growth only where the same shapes left the text queue in the
> same commit. Both follow from the tiers being ordered: `classify_with`
> returns `Clean` when the text is corrupt, so a text-corrupt shape is
> structurally *unclassified* — the worst state this census records — and a
> union without it would read a text fix as a fresh corruption
> (`2026-08-21-declined-run-rescan-design.md` §5). Add
> `crates/kasane-writer/tests/census_len4.rs` to the tiers named above: the text
> tier at length 4, asserting zero, with no allowlist because the answer is
> zero.

- [ ] **Step 5: Verify nothing stale survives**

```bash
grep -rn "closing the whole residual set\|would not be caught by anything here\|32 shapes remain" \
  AGENTS.md docs/superpowers/specs crates/kasane-writer/src
```

Expected: hits only inside the new status block in the emphasis-seam spec, where the old prediction is quoted as the thing being corrected. Any other hit is a record still asserting the opposite of what ships.

- [ ] **Step 6: Confirm the gates and commit**

Run: `mise run lint && mise run test && mise run census-ratchet`

Expected: all three exit 0.

```bash
git add AGENTS.md docs/superpowers/specs/2026-08-15-emphasis-seam-design.md \
        crates/kasane-writer/src/markdown.rs
git commit -m "docs: correct the records the rescan falsifies

Three places asserted the opposite of what now ships:

- emphasis_run's decline comment argued the exposed seam is left unscanned on
  a measured claim, and that a shape corrupt only through it would not be
  caught by anything. The seam is scanned and census_len4.rs is the guard. Its
  worked example stays -- it is now the description of what the rescan fixes.
- 2026-08-15-emphasis-seam-design.md §8 predicted the re-scan closes all 32.
  Measured: 16. Corrected in place with a status block, the way that spec's own
  result note is written, rather than editing the prediction away.
- AGENTS.md's four-rules paragraph and census bullet.

Design spec §7."
```

---

## Amendments during execution

Precedent: `2026-08-15-emphasis-seam.md`'s own section of this name. The steps
above are left exactly as written — a plan that is silently rewritten to match
what happened stops being evidence of what was predicted. Recorded here instead
are the three places where execution, or the reviews that followed it, found a
step's own claim false.

1. **Task 2 Step 4's mutation claim is false.** Step 1's doc-comment block and
   Step 4's verification both state that removing `pos = ppos` produces exactly
   one failure, `a_rollback_restores_the_escaping_position_it_rewound_past`,
   printing `` [^1]:`x`a `` and recovering `""`. Instrumenting the decline arm
   on that exact shape counts `declines=1, rollbacks=0, skips=1`: `Text(":")`
   is the declining `Emph`'s predecessor, `Text`'s `run_end` never moves, so
   the guard takes the **skip** path and nothing is re-rendered at all. The
   test would not fail, and the mutation Step 4 asks for is not a red state.
   The live case for the `Pos` restore is the `predecessor_is_emphasis`
   rollback branch instead, pinned by
   `a_rollback_restores_the_escaping_position_on_a_genuine_predecessor_re_render`
   under `Ledger::from_bits(cell::EMPH_BESIDE_STRONG_RUN_SEAM)` — measured
   `declines=2, rollbacks=1, skips=1`, and with `pos = ppos` removed it prints
   `` [^1]:b`y`a `` instead of `` [^1]\:b`y`a ``, which a footnote-enabled
   parser reads as a footnote *definition*. The code comment at the skip
   branch says all of this, and the final review of the branch renamed the two
   tests whose names still asserted the falsified claims:
   `a_rollback_restores_the_escaping_position_it_rewound_past` →
   `a_skipped_rollback_leaves_an_already_escaped_predecessor_alone`, and
   `a_rollback_cascades_through_a_predecessor_that_then_declines` →
   `a_fused_emph_strong_run_declines_once_and_rolls_back_into_its_code_predecessor`.
   Every name this plan quotes for those two therefore names a test that no
   longer exists under that name.

2. **Task 5 Step 3 could not work as written.** It verifies the new tier's red
   state with `git stash push -- crates/kasane-writer/src/markdown.rs`, which
   assumes the writer changes are uncommitted. By Task 5 they are committed
   (Tasks 1 and 2 each end in a commit), so the stash saves nothing, the tier
   runs against the fixed writer and passes — the exact opposite of the step's
   own stop condition, and a step that reports green where it demanded red is
   worse than no step. The red state was verified out-of-tree instead, against
   the merge base `3188c5b`, and gave exactly the 1,344 the spec's §2.2 table
   predicts. Any future step of this shape must name the revision it wants to
   measure rather than assume a dirty tree.

3. **Task 4 Step 1's `ratchet_gate_cases.sh` could leave a tracked census file
   mutated.** As drafted its `trap 'rm -rf "$tmp"' EXIT` deletes the backup
   before either explicit restore can run, so a signal during the mutation or
   during the `mise run` call would leave the injected line in
   `census-known-structure-corrupt.txt` with nothing left to recover it from.
   The shipped script puts the restore inside the trap, ahead of the cleanup,
   so the ordering cannot invert. (An `EXIT` trap does not survive `SIGKILL`;
   what is guaranteed is every exit path the shell still controls.)

---

## Self-Review

**Spec coverage.** §1 → Tasks 1-3. §2 → the plan argues from it; §2.3's measured 16/32 split is pinned by Task 1 Step 5 and recorded in Task 6. §2.4 (approach E) → no task, correctly: it is a record of something not to build. §3.1 → Task 1. §3.2-§3.3 → Task 2, including the no-predecessor case and the cascade. §4 → Task 3. §5.1-§5.2 → Task 4; §5.3 (permanent file and ceiling unchanged) → Task 3 Step 2's stop condition. §6 → Task 5. §7 → Task 6. §8's five tests → Task 1 Step 1, Task 2 Step 1 (three), Task 4 Step 1, Task 5 Step 1. §9 is non-goals. §10's risks → risk 1 is Task 2's third test, risk 3 is Task 5 Step 4, risk 4 is Task 2's cascade test.

**Placeholder scan.** No TBD/TODO. Every code step carries the actual code, and every expected value in every test was measured against a working implementation in a throwaway worktree rather than reasoned. That is not belt-and-braces: it caught two defects in this plan's own first draft — the module's render helpers are `para`/`recovered` and not the one the draft invented, and the draft's `Pos` test used a shape that does not decline, so it would have passed without any rollback and guarded nothing.

**Type consistency.** `RunOut<'a>` with variants `Emitted(String)` / `Declined(Vec<Flat<'a>>)` is defined in Task 1 and consumed under those exact names in Tasks 2 and 6. `marks: Vec<(usize, usize, Pos)>` is introduced and destructured as `(pi, plen, ppos)` consistently. `census_support::{alphabet, text_is_clean}` and `Ledger::LICENSED` match the signatures in `census_support/mod.rs`. The ratchet's `$tmp/base.$tt` / `$tmp/head.$tt` match the existing variable names in `mise.toml`.
