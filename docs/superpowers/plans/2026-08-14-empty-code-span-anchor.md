# Empty Code Span Anchor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A heading containing an empty inline code span embeds the anchor GitHub actually computes from the line kasane prints, instead of a dead cross-reference.

**Architecture:** `kasane-core`'s `section::clone_inlines_at` — the single walk every inline passes through on its way into the structuring engine — rewrites `Inline::Code("")` to `Inline::Code(" ")`. Downstream, `rendered_text` and `title_text` then see the space that `escape::code_span`'s Rule 1 was already printing, so the anchor and the rendered line agree without either side learning the other's rules. `Code("")` and `Code(" ")` render to the same three bytes (`` ` ` ``), so no code span's output moves.

**Tech Stack:** Rust (workspace crates `kasane-ir`, `kasane-gfm`, `kasane-core`, `kasane-writer`), `proptest`, `pulldown-cmark`, `mise` task runner.

**Spec:** `docs/superpowers/specs/2026-08-14-empty-code-span-anchor-design.md`

## Global Constraints

- Every task ends green under `mise run lint && mise run test`. `lint` is `cargo fmt --all -- --check` plus clippy with `--all-targets`; plain `cargo clippy` is not sufficient.
- Work happens on the branch `empty-code-span-anchor`, which already carries the design spec commit. Do not commit to `main`.
- The canonical form is exactly `Inline::Code(" ")` — one ASCII space. Not a tab, not a non-breaking space.
- The trigger is exactly `t.is_empty()` on the raw content, matching `escape::code_span`'s Rule 1 (spec §1, "Confirmed").
- Do not modify `crates/kasane-writer/src/escape.rs`'s `code_span` behaviour. Rule 1 stays exactly as it is; only its comment changes, and only in Task 4.
- Do not touch `crates/kasane-writer/tests/generator/`. The main property tier is deliberately unchanged (spec §5.3).
- New property is named **P12**. P11 is already taken by the trailing-`#` property.

---

### Task 1: Canonicalize the empty code span

**Files:**
- Modify: `crates/kasane-core/src/section.rs:146-164` (`clone_inlines_at`) and its charter comment at `:84-94`
- Test: `crates/kasane-core/src/paths.rs` (the `#[cfg(test)] mod tests` block starting at `:155`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: the canonicalization itself. `crate::section::clone_inlines_at(inls: &[Inline], depth: usize) -> Vec<Inline>` keeps its existing signature and visibility (`pub(crate)`); only its behaviour for `Inline::Code("")` changes. Task 3 exposes it under a public name.

- [ ] **Step 1: Write the failing test**

Add this to the end of the `mod tests` block in `crates/kasane-core/src/paths.rs`, just before its closing `}`. It goes through `fold_sections` → `balance` → `assign_paths` on purpose: the canonicalization lives inside `fold_sections`' clone, and `balance`'s MERGE is the only thing that puts a heading into another section's body, which is the path the bug is on.

```rust
    #[test]
    fn a_body_heading_with_an_empty_code_span_anchors_the_space_the_line_prints() {
        // `escape::code_span` prints an empty span as `` ` ` `` -- a real
        // space in the rendered line -- so GitHub ids `## a` `b` as `a-b`.
        // `rendered_text` took `Inline::Code("")` verbatim and this rule
        // computed `ab`: a cross-reference dead against GitHub's own render.
        // Design spec 2026-08-14-empty-code-span-anchor-design.md §2.3.
        //
        // Built through `fold_sections` and `balance` rather than as a literal
        // `SectionTree` like `body_headings_get_anchors_too` above: the
        // canonicalization lives in `fold_sections`' bounded clone, so a test
        // that hand-builds the tree would skip the code it is checking.
        let doc = doc(vec![
            h(1, 0, "Parent"),
            Node {
                block: Block::Heading {
                    level: 2,
                    id: BlockId(1),
                    inlines: vec![
                        Inline::Text("a".into()),
                        Inline::Code(String::new()),
                        Inline::Text("b".into()),
                    ],
                },
                prov: Provenance::default(),
            },
        ]);
        let mut tree = fold_sections(&doc);
        // The H2 is childless with an empty body, so MERGE demotes its heading
        // into the H1's body, where `count_headings` anchors it from
        // `rendered_text`.
        crate::balance(&mut tree, &crate::Options::default());
        let placed = assign_paths(tree, "B");
        assert_eq!(placed.anchors[&BlockId(1)], "01-parent.md#a-b");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p kasane-core a_body_heading_with_an_empty_code_span -- --nocapture`

Expected: FAIL. The assertion reports the left side as `01-parent.md#ab` against the expected `01-parent.md#a-b` — the dead anchor, reproduced.

- [ ] **Step 3: Add the canonicalizing arm**

In `crates/kasane-core/src/section.rs`, inside `clone_inlines_at`'s `match i` (currently `:151-162`), add the guarded arm immediately **before** the existing `Inline::Code(t) => Inline::Code(t.clone()),` arm:

```rust
            // CommonMark cannot express an empty code span, so
            // `kasane-writer::escape::code_span` prints one as `` ` ` `` --
            // a padding space that is real text in the rendered line GitHub
            // computes a heading id from. Canonicalizing the empty form here,
            // at the single walk every inline passes through, is what lets
            // `rendered_text` and `title_text` see that space without either
            // importing the writer's escaping rules. `Code("")` and
            // `Code(" ")` render to the same three bytes, so no code span's
            // output moves; `escape.rs`'s
            // `code_span_pads_an_empty_span_to_exactly_what_a_single_space_renders`
            // is the test that keeps that true.
            Inline::Code(t) if t.is_empty() => Inline::Code(" ".into()),
            Inline::Code(t) => Inline::Code(t.clone()),
```

(The existing `Inline::Text(t) => ...` and `Inline::Math(t) => ...` arms are untouched. Only the single `Inline::Code` arm gains a guarded sibling above it.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p kasane-core a_body_heading_with_an_empty_code_span`

Expected: PASS.

- [ ] **Step 5: Widen the documented charter**

`clone_inlines_at` now does two things, and only one of them is written down. In the comment block above `clone_block` (`crates/kasane-core/src/section.rs:84-94`), append this paragraph after the existing text:

```rust
// `clone_inlines_at` carries one canonicalization as well as the bound: an
// empty `Inline::Code` becomes a single space. That is not a tidy-up -- it is
// load-bearing for anchors, and it lives here rather than in a pass of its own
// because this walk is the one place every inline is guaranteed to pass
// through exactly once. See the arm's own comment, and
// `docs/superpowers/specs/2026-08-14-empty-code-span-anchor-design.md` §2.
```

- [ ] **Step 6: Run the full core suite**

Run: `cargo test -p kasane-core`

Expected: PASS. If an existing test fails, it is asserting a title or path built from an empty code span — read it against spec §3 before changing it, and report it rather than adjusting the expectation silently.

- [ ] **Step 7: Lint and commit**

```bash
mise run lint
git add crates/kasane-core/src/section.rs crates/kasane-core/src/paths.rs
git commit -m "fix(core): canonicalize an empty inline code span to a space"
```

---

### Task 2: Pin the two facts the fix rests on

**Files:**
- Test: `crates/kasane-core/src/section.rs` (its `mod tests` block, starting at `:174`)
- Test: `crates/kasane-writer/src/escape.rs` (its `mod tests` block; the existing `code_span` tests are around `:865-926`)

**Interfaces:**
- Consumes: the canonicalizing arm from Task 1.
- Produces: no production symbols. Two guards other tasks rely on being present.

- [ ] **Step 1: Write the nesting test**

Add to the `mod tests` block in `crates/kasane-core/src/section.rs`. Note the
assertion style: `kasane_ir::Inline` derives only `Clone` and `Debug`
(`crates/kasane-ir/src/inline.rs:3`), so `assert_eq!` on inlines does not
compile. Assert per element with `matches!`. **Do not derive `PartialEq` on the
IR to get a shorter test** — that is a public-API change to `kasane-ir` for a
test's convenience.

```rust
    #[test]
    fn an_empty_code_span_is_canonicalized_at_every_depth() {
        // The arm is on the recursive walk, so it has to fire inside emphasis
        // and inside a link label too -- a heading's inlines are not always
        // flat. A non-empty span must come through untouched.
        let got = clone_inlines_at(
            &[
                Inline::Code(String::new()),
                Inline::Emph(vec![Inline::Code(String::new())]),
                Inline::Link {
                    target: RefTarget::External("http://x".into()),
                    inlines: vec![Inline::Code(String::new())],
                },
                Inline::Code("kept".into()),
            ],
            0,
        );
        assert_eq!(got.len(), 4);
        assert!(matches!(&got[0], Inline::Code(s) if s == " "));
        assert!(
            matches!(&got[1], Inline::Emph(x) if matches!(&x[0], Inline::Code(s) if s == " "))
        );
        assert!(matches!(
            &got[2],
            Inline::Link { inlines, .. } if matches!(&inlines[0], Inline::Code(s) if s == " ")
        ));
        assert!(matches!(&got[3], Inline::Code(s) if s == "kept"));
    }
```

- [ ] **Step 2: Run it**

Run: `cargo test -p kasane-core an_empty_code_span_is_canonicalized_at_every_depth`

Expected: PASS — Task 1 already added the arm, so this test documents its reach
rather than driving it.

- [ ] **Step 3: Write the byte-identity guard**

Add to the `mod tests` block in `crates/kasane-writer/src/escape.rs`, next to the existing `code_span` tests:

```rust
    #[test]
    fn code_span_pads_an_empty_span_to_exactly_what_a_single_space_renders() {
        // The load-bearing invariant of the empty-code-span anchor fix:
        // `kasane-core` canonicalizes `Inline::Code("")` to `Inline::Code(" ")`
        // BEFORE anchors are assigned, which is only invisible to a reader
        // because Rule 1 and Rule 2 print the same bytes. If they ever stop
        // agreeing, that canonicalization starts silently rewriting documents
        // -- and the symptom is a changed page, not a failing render, so
        // nothing else would catch it.
        // Design spec 2026-08-14-empty-code-span-anchor-design.md §2.2.
        assert_eq!(code_span("", Ctx::Flow), code_span(" ", Ctx::Flow));
        assert_eq!(code_span("", Ctx::Cell), code_span(" ", Ctx::Cell));
        // Spelled out, so a future edit that breaks BOTH sides equally still
        // fails here rather than agreeing on something new.
        assert_eq!(code_span("", Ctx::Flow), "` `");
    }
```

- [ ] **Step 4: Run it**

Run: `cargo test -p kasane-writer code_span_pads_an_empty_span`

Expected: PASS.

- [ ] **Step 5: Lint and commit**

```bash
mise run lint
git add crates/kasane-core/src/section.rs crates/kasane-writer/src/escape.rs
git commit -m "test: pin the nesting and byte-identity facts the canonicalization rests on"
```

---

### Task 3: Test seam and property P12

**Files:**
- Modify: `crates/kasane-core/src/section.rs` (add the seam function after `clone_inlines_at`)
- Modify: `crates/kasane-core/src/lib.rs:14` (the `pub use section::{...}` line)
- Test: `crates/kasane-writer/tests/properties.rs` (inside the `proptest! { ... }` block, after the `p11_...` property)

**Interfaces:**
- Consumes: `crate::section::clone_inlines_at` from Task 1.
- Produces: `kasane_core::canonicalize_inlines(inls: &[Inline]) -> Vec<Inline>` — `#[doc(hidden)] pub`, a test seam rather than API, wrapping `clone_inlines_at(inls, 0)`.

- [ ] **Step 1: Add the seam**

In `crates/kasane-core/src/section.rs`, immediately after `clone_inlines_at`'s closing brace (currently `:164`):

```rust
/// The engine's inline canonicalization, exposed for the property tier.
///
/// `#[doc(hidden)]` because it is a test seam, not API — the same convention
/// `balance::est_tokens` uses, and for the same reason. P12 has to compare an
/// anchor against the line the writer prints for the SAME inlines the engine
/// anchored, and the engine anchors canonicalized ones. A copy of the rule in
/// the test would pass against its own arithmetic while the engine's changed.
#[doc(hidden)]
pub fn canonicalize_inlines(inls: &[Inline]) -> Vec<Inline> {
    clone_inlines_at(inls, 0)
}
```

- [ ] **Step 2: Export it**

In `crates/kasane-core/src/lib.rs`, change line 14 from:

```rust
pub use section::{fold_sections, SectionNode, SectionTree};
```

to:

```rust
pub use section::{canonicalize_inlines, fold_sections, SectionNode, SectionTree};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p kasane-core`

Expected: success. A dead-code warning here would be a lint failure later; `pub use` from the crate root prevents it.

- [ ] **Step 4: Write P12**

In `crates/kasane-writer/tests/properties.rs`, add the import — change

```rust
use kasane_core::{est_tokens, structure, FileNode};
```

to

```rust
use kasane_core::{canonicalize_inlines, est_tokens, structure, FileNode};
```

then add this property inside the `proptest! { ... }` block, immediately after the `p11_a_trailing_hash_run_survives_and_anchors_the_same` property and before the block's closing brace:

```rust
    /// P12 — an empty inline code span in a heading anchors the space the
    /// printed line actually contains (design spec
    /// 2026-08-14-empty-code-span-anchor-design.md §2.3).
    ///
    /// CommonMark cannot express an empty code span, so `code_span` prints one
    /// as `` ` ` ``. That padding space is real text in the rendered line, and
    /// GitHub ids the heading from the rendered line. `rendered_text` read the
    /// span's content verbatim and saw nothing there, so the engine embedded
    /// an anchor one hyphen short — dead against GitHub's own render.
    ///
    /// The inlines go through `canonicalize_inlines` because that is what the
    /// engine anchors: `structure` applies it to every inline before
    /// `assign_paths` runs. Comparing against raw inlines would test a
    /// pipeline that does not exist.
    #[test]
    fn p12_an_empty_code_span_in_a_heading_anchors_the_same(
        lead in "[a-z]{1,4}",
        tail in "[a-z]{1,4}",
    ) {
        let inlines = canonicalize_inlines(&[
            Inline::Text(lead.clone()),
            Inline::Code(String::new()),
            Inline::Text(tail.clone()),
        ]);
        let blocks = vec![Block::Heading {
            level: 2,
            id: BlockId(0),
            inlines: inlines.clone(),
        }];
        let md = kasane_writer::blocks_to_markdown(&blocks, &AssetBag::default());

        let rendered = anchors_for_headings(&parse_events(&md).headings);
        let embedded = anchor_slug_of(&inlines);

        prop_assert_eq!(
            rendered.first().map(String::as_str),
            Some(embedded.as_str()),
            "anchor/render divergence for an empty code span between {:?} and {:?}:\n{}",
            lead, tail, md
        );
    }
```

- [ ] **Step 5: Run P12**

Run: `cargo test -p kasane-writer --test properties p12_an_empty_code_span`

Expected: PASS. Both sides resolve to `<lead>-<tail>`: the parsed heading text is `<lead> <tail>` because `parse_events` folds `Event::Code(" ")` into the heading text, and `anchor_slug_of` sees the same string via `rendered_text`.

- [ ] **Step 6: Confirm P12 is a real guard, not a tautology**

Temporarily revert the arm — in `crates/kasane-core/src/section.rs`, comment out the `Inline::Code(t) if t.is_empty() =>` line — and run the property again.

Run: `cargo test -p kasane-writer --test properties p12_an_empty_code_span`

Expected: FAIL, reporting `<lead><tail>` against `<lead>-<tail>`. **Then restore the arm** and re-run to confirm PASS. If proptest wrote `crates/kasane-writer/tests/properties.proptest-regressions` during the failing run, delete that file — it records a deliberately induced failure, not a found one, and the repo convention to commit regression files is about genuine finds.

- [ ] **Step 7: Run the whole property tier**

Run: `cargo test -p kasane-writer`

Expected: PASS, P1–P12 included.

- [ ] **Step 8: Lint and commit**

```bash
mise run lint
git add crates/kasane-core/src/section.rs crates/kasane-core/src/lib.rs crates/kasane-writer/tests/properties.rs
git commit -m "test(writer): P12 pins the empty-code-span anchor against the printed line"
```

---

### Task 4: Move the divergence count from two to one

**Files:**
- Modify: `crates/kasane-gfm/src/slug.rs:62-83` (module doc)
- Modify: `crates/kasane-writer/src/escape.rs:454-461` (Rule 1 comment)
- Modify: `AGENTS.md:18-26` and `AGENTS.md:86-91`
- Modify: `README.md:163-180`
- Modify: `docs/superpowers/specs/2026-08-09-markdown-escaping-design.md:468-476`
- Modify: `docs/superpowers/specs/2026-08-14-shared-gfm-text-model-design.md` (Status block, and the § Non-goals bullet)
- Modify: `docs/superpowers/specs/2026-08-14-empty-code-span-anchor-design.md` (Status line)

**Interfaces:**
- Consumes: everything from Tasks 1–3. Nothing produced.

This task changes no behaviour. It exists because the divergence count is written down in seven places, and a fix that leaves six of them claiming a live bug is worse than no fix for the next reader.

- [ ] **Step 1: `slug.rs` module doc**

Replace `//! Two divergences are left:` with `//! One divergence is left:`, and delete the whole `//! - **An empty inline code span inside a heading.** …` bullet (through its final `//!   \`escape::code_span\`'s Rule 1 comment for the writer side.` line). Then extend the sentence above the list, which currently ends `…a real parser would strip a bare closing sequence down to.`, with a third closure:

```rust
//! An empty inline code span in a heading no longer diverges either:
//! `kasane-core` canonicalizes `Inline::Code("")` to a single space before any
//! anchor is computed, so this rule sees the padding space
//! `kasane-writer::escape::code_span` was already printing.
```

- [ ] **Step 2: `escape.rs` Rule 1 comment**

Replace the comment body inside `if content.is_empty() {` (`:455-460`) with:

```rust
        // Rule 1: Empty content gets a single space (only acknowledged divergence from round-trip).
        // No longer an anchor divergence: `kasane-core`'s `clone_inlines_at`
        // canonicalizes `Inline::Code("")` to `Inline::Code(" ")` on the way
        // into the engine, so anything that was structured reaches Rule 2 with
        // the space already spelled, and the anchor sees it. Rule 1 still runs
        // for a caller who hand-builds the empty form and calls
        // `blocks_to_markdown` directly -- they bypass `assign_paths` entirely
        // and have no anchor to diverge from.
        // Rule 1 and Rule 2 must keep printing the same bytes for that
        // canonicalization to stay invisible; see
        // `code_span_pads_an_empty_span_to_exactly_what_a_single_space_renders`.
```

- [ ] **Step 3: `AGENTS.md` — the `kasane-gfm` entry**

In the sentence beginning `Of the three anchor divergences`, change `two are now closed` to `all three are now closed`, and replace the trailing clause `— leaving two: the empty-id fallback (\`EMPTY_FALLBACK\`), a deliberate choice rather than a construction defect, and a pre-existing, still-open one where a heading's empty inline code span renders as a space \`rendered_text\` does not model, so the anchor kasane embeds is a dead cross-reference against GitHub's own render.` with:

```
  — a footnote reference's digits via `rendered_text`, a trailing `#` run via
  `kasane-writer::escape::atx_closing` escaping it before GitHub ever sees it,
  and a heading's empty inline code span via `section::clone_inlines_at`
  canonicalizing `Inline::Code("")` to a single space before any anchor is
  computed. What is left is the empty-id fallback (`EMPTY_FALLBACK`), a
  deliberate choice rather than a construction defect.
```

- [ ] **Step 4: `AGENTS.md` — the mirror/case-table paragraph**

Change `two divergences still survive there on purpose: the empty-id fallback, and a pre-existing case where a heading's empty inline code span renders as a padding space (\`escape::code_span\`'s only way to express one) that \`rendered_text\` does not model, so the anchor kasane embeds is a dead cross-reference.` to:

```
  one divergence still survives there on purpose: the empty-id fallback.
```

and change the following sentence `\`rendered_text\` and \`escape::atx_closing\` closed the other two the table used to record.` to:

```
  `rendered_text`, `escape::atx_closing`, and `section::clone_inlines_at`'s
  empty-code-span canonicalization closed the other three the table used to
  record.
```

- [ ] **Step 5: `README.md` — the exception list**

Change `The two exceptions:` to `The one exception:`, and delete the entire second bullet (`- A heading containing an empty inline code span (an empty pair of backticks between two words) gets a cross-reference that does not resolve: …` through `… Pre-existing, and open — see \`kasane_gfm::slug\`'s module doc.`).

Then change the paragraph below it from `Two anchors that used to diverge no longer do, which matters for a tree an older build produced:` to `Three anchors that used to diverge no longer do, which matters for a tree an older build produced:` and append a third item to its list, after the trailing-`#` clause:

```
  and a heading containing an empty pair of backticks now anchors the space
  that pair prints (`#a-b`, not `#ab`) — which also means such a heading's
  file is now named `a-b.md` rather than `ab.md`.
```

- [ ] **Step 6: The escaping spec's open bullet**

In `docs/superpowers/specs/2026-08-09-markdown-escaping-design.md`, the bullet `- **An empty inline code span inside a heading.**` (at `:468`) keeps its text — it is the record of what was open — and gains a closure note in the shape the footnote-ref bullet below it already uses. Append to that bullet:

```markdown
  **Closed 2026-08-14** by `2026-08-14-empty-code-span-anchor-design.md`, and
  not where this bullet predicted. It says the fix is "not fixable by escaping
  either", which is right, and implies the anchor rule must learn about the
  padding — but `kasane-core`'s `section::clone_inlines_at` instead
  canonicalizes `Inline::Code("")` to `Inline::Code(" ")` before any anchor is
  computed, so neither side learned anything about the other and the empty form
  simply stopped existing downstream. Rule 1 is unchanged and still prints
  `` ` ` ``; it is reachable now only by a caller who renders hand-built IR
  without going through `structure`.
```

Also change the sentence introducing the surviving cases, `The other case recorded here as open, a newline run split across an \`Inline::FootnoteRef\`, closed 2026-08-14:`, so it does not imply the empty-code-span case is still open — make it read `The other case recorded here as open, a newline run split across an \`Inline::FootnoteRef\`, closed 2026-08-14 as well:`.

- [ ] **Step 7: The shared-GFM spec**

In `docs/superpowers/specs/2026-08-14-shared-gfm-text-model-design.md`:

- In the **Status** block, the sentence beginning `A second, pre-existing divergence — a heading's empty inline code span …` ends `… Neither is a defect this item introduced.` Append: `The empty-code-span divergence was closed on 2026-08-14 by its own item, \`2026-08-14-empty-code-span-anchor-design.md\`; \`EMPTY_FALLBACK\` remains, by choice.`
- In **§ Non-goals**, the bullet `- **The empty-id divergence.** …` is unchanged. No other bullet there names the code-span case, so nothing else moves; if a grep for `empty inline code` finds a third mention in this file, give it the same one-sentence closure note.

- [ ] **Step 8: This item's own spec Status**

In `docs/superpowers/specs/2026-08-14-empty-code-span-anchor-design.md`, change `**Status:** Designed, not implemented.` to:

```markdown
**Status:** Implemented 2026-08-14. The anchor now matches GitHub's id for a
heading containing an empty code span, pinned by P12 and by
`a_body_heading_with_an_empty_code_span_anchors_the_space_the_line_prints`. The
external oracle has not been re-run; §8's note about adding this case to the
probe stands.
```

- [ ] **Step 9: Check no stale claim survives**

Run: `grep -rniE "empty inline code|empty code span" --include='*.md' --include='*.rs' . | grep -v '^./target'`

Expected: every remaining hit either describes the case as closed, is the design spec for this item, or is the escaping spec's historical record. Any hit still calling it open or "still surviving" is a miss — fix it.

- [ ] **Step 10: Full verification and commit**

```bash
mise run lint && mise run test
git add -A
git commit -m "docs: record the empty-code-span anchor closure"
```

Expected: both green. `mise run test` is the gate that catches a doc edit having broken a doctest or a comment-anchored test name.

---

## Self-Review

**Spec coverage:**

| Spec section | Task |
|---|---|
| §2.1 the arm, §2.2 byte-identity, §2.3 anchor correctness | 1, 2 |
| §3 blast radius (title/path change) | 1 Step 6 surfaces any existing test that asserts the old spelling; §3's consequences are recorded in README by 4 Step 5 |
| §4 Rule 1's changed status | 4 Step 2 |
| §5.1 unit tests | 1 Step 1 (paths), 2 Steps 1 and 3 (section, escape) |
| §5.2 property P12 + `#[doc(hidden)]` seam | 3 |
| §5.3 generator unchanged | Global Constraints (explicit "do not touch") |
| §6 documentation, all seven spots | 4 |
| §8 verification | Global Constraints + 4 Step 10 |

**Naming consistency:** the seam is `canonicalize_inlines` in Task 3 Steps 1, 2 and 4; the property is `p12_an_empty_code_span_in_a_heading_anchors_the_same` in Steps 4, 5 and 6; the escape guard is `code_span_pads_an_empty_span_to_exactly_what_a_single_space_renders` in Task 1 Step 3's comment, Task 2 Step 3, and Task 4 Step 2's comment.

**Verified against the code while writing, not assumed:** `kasane_ir::Inline` derives only `Clone` and `Debug`, so Task 2's assertions use `matches!`; `P11` is already taken by the trailing-`#` property, so the new one is `P12`; `kasane-writer` already depends on `kasane-core`, so Task 3's seam needs no manifest change; and `balance`'s MERGE is the only thing that puts a heading into another section's body, which is why Task 1's test calls it.
