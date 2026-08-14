# Shared GFM Text Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the newline fold and both slug rules into a new `kasane-gfm` leaf crate, then close the two heading-anchor divergences that the hand-kept mirror made unfixable.

**Architecture:** `kasane-gfm` depends on `kasane-ir` alone and owns what GFM does to heading text: the newline fold, a `rendered_text` projection of an inline run, a `title_text` projection for nav surfaces, and the anchor and path slug rules. `kasane-core` consumes it to compute anchors at structuring time; `kasane-writer` consumes the fold and is held to the projection by a property that parses its own output. The footnote-reference divergence closes in the projection; the trailing-`#` divergence closes in the writer, by escaping the ATX closing sequence so the rendered line keeps the text the IR holds.

**Tech Stack:** Rust (pinned stable, `rust-version = "1.97"`), `proptest` + `pulldown-cmark` (dev-only) for the property tier, `cargo-fuzz` on the pinned nightly, `mise` tasks.

**Spec:** `docs/superpowers/specs/2026-08-14-shared-gfm-text-model-design.md`

## Global Constraints

- Every change ships green under `mise run lint && mise run test`. `mise run lint` is `cargo fmt --check` plus `clippy --all-targets -D warnings`; plain `cargo clippy` is not the gate.
- Branch is `shared-gfm-text-model`, already created, with the design spec committed on it.
- The new crate is published like the others: `version.workspace = true`, `edition.workspace = true`, `rust-version.workspace = true`, `license.workspace = true`, `repository.workspace = true`, `[lints] workspace = true`.
- Internal crates are declared once in the root `[workspace.dependencies]` with both a path and a literal `version = "0.1.0"`, and consumed as `kasane-gfm.workspace = true`.
- `pulldown-cmark` stays a **dev**-dependency. Nothing in this plan may put a Markdown parser on the production path.
- Commit messages follow the repo's convention: `refactor(gfm):`, `fix(core):`, `fix(writer):`, `test(writer):`, `docs:`.
- Fuzz target names, `fuzz/seeds/**`, `fuzz/artifacts/**` and `KNOWN_OPEN` are not renamed or moved by any task here.
- In this sandbox `mise run fuzz <target>` needs `ASAN_OPTIONS=detect_leaks=0`, or every target false-positives a crash.

---

### Task 1: Relocate the slug rules into `kasane-gfm`

A pure move. No behaviour changes, no renames, no signature changes — those are Tasks 2 and 3. The gate is that the existing suite passes unchanged with the code in its new home, and that the diff is a relocation a reviewer can read as one.

**Files:**
- Create: `crates/kasane-gfm/Cargo.toml`
- Create: `crates/kasane-gfm/src/lib.rs`
- Create: `crates/kasane-gfm/src/slug.rs` (moved from `crates/kasane-core/src/slug.rs`, 642 lines, verbatim except visibility)
- Create: `crates/kasane-gfm/src/fuzz_entry.rs` (moved from `crates/kasane-core/src/fuzz_entry.rs`)
- Delete: `crates/kasane-core/src/slug.rs`, `crates/kasane-core/src/fuzz_entry.rs`
- Modify: `crates/kasane-core/src/lib.rs` (drop `mod slug`, `pub mod fuzz_entry`, and the slug re-exports)
- Modify: `crates/kasane-core/Cargo.toml` (add `kasane-gfm`, drop `unicode-normalization` and `unicode-properties`)
- Modify: `crates/kasane-core/src/paths.rs:2`, `crates/kasane-core/src/nav.rs:6`, `crates/kasane-core/src/refs.rs:116`, `crates/kasane-core/src/balance.rs:251`
- Modify: `crates/kasane-writer/Cargo.toml` (add `kasane-gfm` as a dev-dependency)
- Modify: `crates/kasane-writer/tests/properties.rs:17`
- Modify: `crates/kasane-adapters/Cargo.toml` (swap the `kasane-core` dev-dependency for `kasane-gfm`)
- Modify: `crates/kasane-adapters/tests/fuzz_corpus.rs:36`
- Modify: `fuzz/Cargo.toml`, `fuzz/fuzz_targets/slug.rs`
- Modify: `Cargo.toml` (workspace dependency + the "FIVE lines" comment)

**Interfaces:**
- Consumes: nothing.
- Produces: crate `kasane-gfm` exporting `pub fn inline_text(&[Inline]) -> String`, `pub fn path_slug(&[Inline]) -> String`, `pub struct AnchorCounter` with `pub fn new() -> Self` and `pub fn next(&mut self, inlines: &[Inline]) -> String`, and the three `#[doc(hidden)] pub` test seams `path_slug_of(&[Inline]) -> String`, `anchor_slug_of(&[Inline]) -> String`, `anchors_for_headings(&[String]) -> Vec<String>`. `pub mod fuzz_entry` with `pub fn slug(&[u8])`. `anchor_slug`, `anchor_fold`, `path_fold`, `fold_newlines`, `is_word`, `is_join_control`, `MAX_PATH_SLUG_BYTES` and `EMPTY_FALLBACK` stay crate-private in this task.

- [ ] **Step 1: Create the crate manifest**

`crates/kasane-gfm/Cargo.toml`:

```toml
[package]
name = "kasane-gfm"
description = "GitHub-Flavored Markdown text semantics for kasane: the heading newline fold and the heading-id and filename slug rules"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
kasane-ir.workspace = true
# NFC (for `path_slug` only -- `anchor_slug` must not normalize; see
# `slug.rs`), and the General_Category tables behind Ruby's `\p{Word}`. std
# answers the `Alphabetic` term itself (`char::is_alphabetic()` IS that
# derived property) but neither of the other two: it cannot keep the
# Devanagari virama (U+094D), a separate Mark that NFC does not compose away
# and that is not `Alphabetic` -- `हिन्दी` would slug to `हिनदी` -- nor tell
# `Nd` from `No`, which Ruby's set turns on.
# `default-features = false` drops the emoji tables, which nothing here needs.
unicode-normalization = "0.1"
unicode-properties = { version = "0.1", default-features = false, features = ["general-category"] }

[lints]
workspace = true
```

- [ ] **Step 2: Move `slug.rs` with git so the rename is visible in review**

```bash
git mv crates/kasane-core/src/slug.rs crates/kasane-gfm/src/slug.rs
git mv crates/kasane-core/src/fuzz_entry.rs crates/kasane-gfm/src/fuzz_entry.rs
```

- [ ] **Step 3: Widen visibility on exactly the items other crates need**

In `crates/kasane-gfm/src/slug.rs`, change these four declarations from `pub(crate)` to `pub`, and nothing else:

```rust
pub fn inline_text(inlines: &[Inline]) -> String
pub fn path_slug(inlines: &[Inline]) -> String
pub struct AnchorCounter
impl AnchorCounter {
    pub fn new() -> Self
    pub fn next(&mut self, inlines: &[Inline]) -> String
}
```

`anchor_slug`, `fold_newlines`, `anchor_fold`, `path_fold`, `is_word`, `is_join_control`, `truncate_to`, `trim_tail`, `MAX_PATH_SLUG_BYTES` and `EMPTY_FALLBACK` all stay `pub(crate)` or private exactly as they are — `fuzz_entry` is inside this crate and reaches them. Leave every doc comment untouched; the module doc is load-bearing and moves verbatim.

Because `AnchorCounter::new` is now public, clippy's `new_without_default` will fire. Add the derive-free impl right below the existing one rather than suppressing it:

```rust
impl Default for AnchorCounter {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: Write the crate root**

`crates/kasane-gfm/src/lib.rs`:

```rust
//! GitHub-Flavored Markdown text semantics, shared by `kasane-core` and
//! `kasane-writer`.
//!
//! A leaf crate over `kasane-ir`. It exists because two crates have to agree
//! on one thing — what a heading line renders to — and neither can own it:
//! `kasane-core` computes a heading's anchor at structuring time, and
//! `kasane-writer` emits the line that anchor has to match. Before this crate
//! the agreement was two functions kept in step by hand.
//!
//! `slug` holds both slug rules and the character class they share.

#[doc(hidden)]
pub mod fuzz_entry;
mod slug;

pub use slug::{
    anchor_slug_of, anchors_for_headings, inline_text, path_slug, path_slug_of, AnchorCounter,
};
```

- [ ] **Step 5: Register the crate in the workspace**

In the root `Cargo.toml`, add to `[workspace.dependencies]`, keeping the existing order (ir, gfm, core, writer, adapters):

```toml
kasane-gfm = { path = "crates/kasane-gfm", version = "0.1.0" }
```

and correct the comment above that block: `a release bumps FIVE lines total` becomes `SIX`, and `plus each of the four entries below` becomes `five`.

- [ ] **Step 6: Point `kasane-core` at the new crate**

`crates/kasane-core/Cargo.toml`: delete the `unicode-normalization` and `unicode-properties` lines together with the comment block above them (it moved to `kasane-gfm` in Step 1), and add:

```toml
kasane-gfm.workspace = true
```

`crates/kasane-core/src/lib.rs` becomes:

```rust
mod balance;
mod nav;
mod options;
mod paths;
mod refs;
mod section;
mod sitetree;

pub use balance::{balance, est_tokens};
pub use nav::structure;
pub use options::Options;
pub use paths::{assign_paths, PlaceResult, Placed};
pub use refs::resolve_refs;
pub use section::{fold_sections, SectionNode, SectionTree};
pub use sitetree::{FileNode, Frontmatter, SiteTree};
```

Note that `pub mod fuzz_entry` goes with it: `slug` was its only seam, so `kasane-core` has no fuzz module left.

Then the four call sites:

- `crates/kasane-core/src/paths.rs:2` — `use crate::slug::{path_slug, AnchorCounter};` becomes `use kasane_gfm::{path_slug, AnchorCounter};`
- `crates/kasane-core/src/nav.rs:6` — `use crate::slug::inline_text;` becomes `use kasane_gfm::inline_text;`
- `crates/kasane-core/src/refs.rs:116` — `Inline::Text(crate::slug::inline_text(&inlines))` becomes `Inline::Text(kasane_gfm::inline_text(&inlines))`
- `crates/kasane-core/src/balance.rs:251` — `.map(|c| crate::slug::inline_text(&c.title))` becomes `.map(|c| kasane_gfm::inline_text(&c.title))`

- [ ] **Step 7: Point the writer's property tier at the new crate**

`crates/kasane-writer/Cargo.toml`, under `[dev-dependencies]`:

```toml
kasane-gfm.workspace = true
```

`crates/kasane-writer/tests/properties.rs:17` becomes two lines:

```rust
use kasane_core::{est_tokens, structure, FileNode};
use kasane_gfm::{anchor_slug_of, anchors_for_headings};
```

- [ ] **Step 8: Point the fuzz replay harness and the fuzz workspace at the new crate**

`crates/kasane-adapters/Cargo.toml`, in `[dev-dependencies]`, replace the `kasane-core` entry and correct its comment:

```toml
# For `tests/fuzz_corpus.rs` only: the `slug` target's seam lives in
# kasane-gfm, and this file is the one replay harness for every target.
# Test-scoped, so it does not put gfm in the adapter crate's real dependency
# graph, and acyclic -- gfm depends on kasane-ir alone.
kasane-gfm.workspace = true
kasane-writer.workspace = true
```

`crates/kasane-adapters/tests/fuzz_corpus.rs:36`:

```rust
        "slug" => kasane_gfm::fuzz_entry::slug,
```

`fuzz/Cargo.toml` — replace the `kasane-core` dependency (the `slug` target was its only user):

```toml
kasane-gfm = { path = "../crates/kasane-gfm" }
```

`fuzz/fuzz_targets/slug.rs`:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    kasane_gfm::fuzz_entry::slug(data);
});
```

- [ ] **Step 9: Verify the move compiles and the moved tests run in their new home**

Run: `cargo test -p kasane-gfm`
Expected: PASS, and the summary lists the slug unit tests (`anchor_matches_github`, `nfd_and_nfc_diverge_for_anchors_and_agree_for_paths`, `path_slug_is_a_filename_not_an_anchor`, `path_slug_cannot_emit_a_separator`, `path_slug_caps_at_the_byte_budget`, `path_slug_trims_a_dangling_tail`) — six tests, running against `kasane-gfm` rather than `kasane-core`.

- [ ] **Step 10: Verify nothing else changed**

Run: `mise run lint && mise run test`
Expected: PASS. No test is added, removed or edited in this task — a pure move whose gate is the unchanged suite.

Run: `cargo build --manifest-path fuzz/Cargo.toml 2>&1 | tail -5`
Expected: builds, or fails only because the nightly toolchain is unavailable in this shell. If it builds nothing else needs checking; the target list is unchanged.

- [ ] **Step 11: Commit**

```bash
git add -A
git commit -m "refactor(gfm): move the slug rules into a shared leaf crate

Pure relocation, no behaviour change: kasane-core/src/slug.rs and the
fuzz seam that reaches it become crates/kasane-gfm, which depends on
kasane-ir alone. kasane-core, kasane-writer's property tier, the fuzz
replay harness and the fuzz workspace follow the code.

This is the home the fold and the projection need in the next commits;
moving and changing in one diff would make neither reviewable."
```

---

### Task 2: The rendered-text projection closes the footnote divergence

`## Notes[^1]` anchors `notes` today and `notes1` on GitHub. The property tier cannot see it, because `parse_events` drops `Event::FootnoteReference` exactly as `inline_text` drops `Inline::FootnoteRef` — both sides agree on the wrong answer. Close the tier's blind spot first, watch it fail, then fix the projection.

**Files:**
- Create: `crates/kasane-gfm/src/text.rs`
- Modify: `crates/kasane-gfm/src/lib.rs`, `crates/kasane-gfm/src/slug.rs`, `crates/kasane-gfm/src/fuzz_entry.rs`
- Modify: `crates/kasane-core/src/paths.rs` (imports, `place`, `count_headings`, tests)
- Modify: `crates/kasane-core/src/nav.rs`, `crates/kasane-core/src/refs.rs`, `crates/kasane-core/src/balance.rs` (`inline_text` → `title_text`)
- Test: `crates/kasane-writer/tests/properties.rs` (`parse_events` arm, new P10)
- Test: `crates/kasane-core/src/paths.rs` (title-anchor unit test)

**Interfaces:**
- Consumes: Task 1's `kasane-gfm` crate.
- Produces: `pub fn fold_newlines(s: &str) -> String`, `pub fn title_text(inlines: &[Inline]) -> String`, `pub fn rendered_text(inlines: &[Inline]) -> String`, `pub fn anchor_slug(line: &str) -> String`, and `AnchorCounter::next(&mut self, line: &str) -> String` — note the counter now takes the **printed line text**, not inlines. `anchor_slug_of(&[Inline])` and `anchors_for_headings(&[String])` keep their signatures. `inline_text` no longer exists under that name.

- [ ] **Step 1: Write the failing test — teach the parser what GitHub renders, and assert on it**

In `crates/kasane-writer/tests/properties.rs`, add the arm to `parse_events`' match, directly after the `Event::Text(t) | Event::Code(t)` arm:

```rust
            // GitHub renders a resolved reference as a superscript number and
            // leaves an unresolved one as the literal `[^1]`; its id filter
            // strips `[`, `^` and `]`, so both spellings contribute the same
            // digits to a heading's id. Modelling the reference here is what
            // lets this tier see a divergence it used to share: the parsed
            // side skipped the reference exactly as `inline_text` did.
            //
            // Heading text only. `p.text` feeds P1's sentinel accounting,
            // where a footnote label is not a payload.
            Event::FootnoteReference(label) => {
                if heading_depth > 0 {
                    heading.push_str(&label);
                }
            }
```

Add `NoteId` to the `kasane_ir` import list at the top of the file, then add the property inside the same `proptest!` block that holds `p9_boundary_newline_runs_anchor_the_same`:

```rust
    /// P10 — a footnote reference in a heading anchors the way GitHub ids it
    /// (design spec 2026-08-14 §3).
    ///
    /// The reference is visible text the writer emits and the IR does not
    /// spell, which is the one place `rendered_text` diverges from
    /// `title_text`. Both spellings are drawn: with the definition in the same
    /// file (GitHub renders a superscript number) and without it (the literal
    /// `[^1]` survives), because §3's claim is that the id is the same either
    /// way.
    #[test]
    fn p10_footnote_ref_in_a_heading_anchors_the_same(
        n in 1u32..=20,
        tail in "[a-z]{0,4}",
        with_def in any::<bool>(),
    ) {
        let inlines = vec![
            Inline::Text("Notes".into()),
            Inline::FootnoteRef(NoteId(n)),
            Inline::Text(format!(" {tail}")),
        ];
        let mut blocks = vec![Block::Heading {
            level: 2,
            id: BlockId(0),
            inlines: inlines.clone(),
        }];
        if with_def {
            blocks.push(Block::Footnote {
                id: NoteId(n),
                blocks: vec![Block::Para(vec![Inline::Text("def".into())])],
            });
        }
        let md = kasane_writer::blocks_to_markdown(&blocks, &AssetBag::default());

        let rendered = anchors_for_headings(&parse_events(&md).headings);
        let embedded = anchor_slug_of(&inlines);

        prop_assert_eq!(
            rendered.first().map(String::as_str),
            Some(embedded.as_str()),
            "anchor/render divergence for [^{}] (definition present: {}):\n{}",
            n, with_def, md
        );
    }
```

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p kasane-writer --test properties p10_footnote_ref_in_a_heading_anchors_the_same`
Expected: FAIL. The message shows the rendered id carrying the reference's digits and the embedded one missing them — `Some("notes1-abc")` against `Some("notes-abc")`, or `Some("notes1")` against `Some("notes")` when `tail` shrinks to empty.

- [ ] **Step 3: Write the two projections**

Create `crates/kasane-gfm/src/text.rs`:

```rust
//! The rendered-line vocabulary: the newline fold, and the two projections of
//! an inline run to text.

use kasane_ir::Inline;

/// Fold every newline spelling to a single space, **collapsing runs**, for the
/// contexts where a newline is structurally impossible: heading lines, link
/// labels, image alt text, code spans, YAML scalars.
///
/// One fold for two crates, and the collapse is what makes that possible. The
/// writer's two heading paths reach it from opposite directions —
/// `Block::Heading` escapes first and folds after, `file_to_markdown`'s title
/// heading folds first and escapes after — so without the collapse they
/// disagreed on a blank line (`## A B` against `# A  B`) and the anchor, which
/// can only predict one rendered line, was dead for whichever path it did not
/// predict.
///
/// Literal spaces are a different mechanism and are deliberately *not*
/// collapsed: `Background & Notes` still anchors `background--notes`. Tabs are
/// untouched too — a tab survives into the rendered line, where the anchor
/// filter drops it, since a tab is in neither `\p{Word}`, `-`, nor space.
pub fn fold_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_newline = false;
    for c in s.chars() {
        if c == '\n' || c == '\r' {
            if !last_was_newline {
                out.push(' ');
            }
            last_was_newline = true;
        } else {
            out.push(c);
            last_was_newline = false;
        }
    }
    out
}

/// The visible text of an inline run for a **navigation surface**: a file's
/// frontmatter title, a breadcrumb entry, a TOC link label, a library index
/// row, and the plain-text fallback `refs` leaves where a link was stripped.
///
/// Skips `Inline::FootnoteRef`, and that is the point rather than an
/// approximation of [`rendered_text`]. A `[^1]` in any of those surfaces
/// renders a footnote reference pointing at a definition that, after a
/// `balance` split, is likely in another file — a dangling marker in
/// `index.md` is worse than an absent one in a title.
pub fn title_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    walk(inlines, 0, false, &mut s);
    s
}

/// The text the writer's rendering of this run renders back to.
///
/// This is what a heading's anchor is computed from, because GitHub computes
/// an id from the rendered line. It differs from [`title_text`] in exactly one
/// arm: `Inline::FootnoteRef(n)` contributes `[^n]`, which the writer emits as
/// visible text.
///
/// Correct whether or not the reference resolves. GitHub renders a resolved
/// one as a superscript `1` and leaves an unresolved one as the literal
/// `[^1]`; the id filter removes `[`, `^` and `]`, so both land on the same
/// digits. Nothing here has to know which happened — which matters, because
/// after a `balance` split the definition may be in a different file.
pub fn rendered_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    walk(inlines, 0, true, &mut s);
    s
}

/// One walk for both projections, so the single arm they differ on is visible
/// in one place. Bounded by `MAX_INLINE_DEPTH` like every other recursive walk
/// over the IR.
fn walk(inlines: &[Inline], depth: usize, notes: bool, s: &mut String) {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return;
    }
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => s.push_str(t),
            Inline::Emph(x) | Inline::Strong(x) => walk(x, depth + 1, notes, s),
            Inline::Link { inlines, .. } => walk(inlines, depth + 1, notes, s),
            Inline::FootnoteRef(n) => {
                if notes {
                    s.push_str(&format!("[^{}]", n.0));
                }
            }
        }
    }
}
```

- [ ] **Step 4: Make the anchor rule take the line it will print**

In `crates/kasane-gfm/src/slug.rs`, delete `fold_newlines`, `inline_text` and `inline_text_at` (they now live in `text.rs`), add `use crate::text::{fold_newlines, rendered_text, title_text};`, and change these four items:

```rust
/// The anchor's fold: the printed line, newlines folded to spaces, outer
/// whitespace trimmed, Unicode-lowercased, and **not normalized**.
fn anchor_fold(line: &str) -> String {
    fold_newlines(line)
        .trim()
        .chars()
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn anchor_slug(line: &str) -> String {
    let out: String = anchor_fold(line)
        .chars()
        .filter(|c| is_word(*c) || is_join_control(*c) || *c == '-' || *c == ' ')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect();
    if out.is_empty() {
        EMPTY_FALLBACK.to_string()
    } else {
        out
    }
}
```

```rust
fn path_fold(inlines: &[Inline]) -> String {
    title_text(inlines)
        .trim()
        .nfc()
        .flat_map(char::to_lowercase)
        .collect()
}
```

```rust
impl AnchorCounter {
    /// The anchor for the next heading in render order, computed from the text
    /// that heading's line prints. Every heading the file renders must pass
    /// through here, including ones that get no anchor of their own — they
    /// still consume a slot on the rendered page.
    ///
    /// Taking `&str` rather than `&[Inline]` is the enforcement, not a
    /// convenience: a caller cannot hand it an inline run and receive an
    /// anchor for a line it is not going to print. The two heading paths print
    /// different things — a body heading prints the writer's rendering of its
    /// inlines, a file's title heading prints `Frontmatter::title` verbatim —
    /// and each projects accordingly.
    pub fn next(&mut self, line: &str) -> String {
        let base = anchor_slug(line);
        let n = self.seen.entry(base.clone()).or_insert(0);
        let out = if *n == 0 { base } else { format!("{base}-{n}") };
        *n += 1;
        out
    }
}
```

```rust
/// Test seam for the anchor rule over real inline structure, same rationale as
/// `path_slug_of` and `anchors_for_headings`. Composes the projection with the
/// rule, which is what a body heading does. No counter is threaded because a
/// single heading has no duplicate to suffix.
#[doc(hidden)]
pub fn anchor_slug_of(inlines: &[Inline]) -> String {
    AnchorCounter::new().next(&rendered_text(inlines))
}

/// Anchors for one file's headings, in the order the file renders them, from
/// the text those lines print.
#[doc(hidden)]
pub fn anchors_for_headings(headings: &[String]) -> Vec<String> {
    let mut counter = AnchorCounter::new();
    headings.iter().map(|t| counter.next(t)).collect()
}
```

The module doc's axis 4 (`Newline folding`) now points at `text::fold_newlines` rather than a local `fold_newlines`; update that one link and leave the rest of the doc alone.

`crates/kasane-gfm/src/lib.rs` gains the module and the exports:

```rust
#[doc(hidden)]
pub mod fuzz_entry;
mod slug;
mod text;

pub use slug::{
    anchor_slug, anchor_slug_of, anchors_for_headings, path_slug, path_slug_of, AnchorCounter,
};
pub use text::{fold_newlines, rendered_text, title_text};
```

`crates/kasane-gfm/src/fuzz_entry.rs` — the seam builds an inline run, so it projects before slugging:

```rust
use crate::slug::{anchor_slug, path_slug, MAX_PATH_SLUG_BYTES};
use crate::text::rendered_text;
```

```rust
    // Anchors are uncapped by design, but an empty one is a dead link.
    let anchor = anchor_slug(&rendered_text(&inlines));
```

- [ ] **Step 5: Feed each heading path the text it prints**

`crates/kasane-core/src/paths.rs:2`:

```rust
use kasane_gfm::{path_slug, rendered_text, title_text, AnchorCounter};
```

In `place`, the title-heading slot — note `Inline` may now be an unused import in this file; drop it from the `kasane_ir` use line if clippy says so:

```rust
    // A file's title heading prints `Frontmatter::title`, which `nav::walk`
    // builds with `title_text` — so the anchor is computed from that same
    // string, not from the inlines behind it. They differ whenever the title
    // carries a footnote reference: the printed line has no `[^1]` in it, and
    // an anchor that predicted one would point at an id no renderer assigns.
    let title_anchor = if is_root {
        counter.next(doc_title)
    } else {
        counter.next(&title_text(&node.title))
    };
```

In `count_headings`, the body-heading slot:

```rust
            Block::Heading { id, inlines, .. } => {
                let a = counter.next(&rendered_text(inlines));
                if top_level {
                    anchors.insert(*id, format!("{}#{}", self_path, a));
                }
            }
```

Then rename the three remaining consumers, which keep the nav-surface behaviour:

- `crates/kasane-core/src/nav.rs:6` — `use kasane_gfm::title_text;`, and lines 73 and 111 call `title_text(...)`
- `crates/kasane-core/src/refs.rs:116` — `Inline::Text(kasane_gfm::title_text(&inlines))`
- `crates/kasane-core/src/balance.rs:251` — `.map(|c| kasane_gfm::title_text(&c.title))`

- [ ] **Step 6: Run the property to verify it passes**

Run: `cargo test -p kasane-writer --test properties p10_footnote_ref_in_a_heading_anchors_the_same`
Expected: PASS.

- [ ] **Step 7: Pin the title path with a unit test**

In `crates/kasane-core/src/paths.rs`'s test module, add:

```rust
    /// A section title carrying a footnote reference anchors on the text the
    /// title heading PRINTS, which `nav::walk` builds with `title_text` — the
    /// reference is not in it. Anchoring the inlines instead would predict
    /// `notes1` for a line that renders `Notes`.
    #[test]
    fn a_title_anchor_follows_the_printed_title_not_the_inlines() {
        let mut tree = fold_sections(&doc(vec![
            h(1, 0, "Top"),
            Node {
                block: Block::Heading {
                    level: 2,
                    id: BlockId(1),
                    inlines: vec![
                        Inline::Text("Notes".into()),
                        Inline::FootnoteRef(NoteId(7)),
                    ],
                },
                prov: Provenance::default(),
                children: vec![],
            },
        ]));
        let result = assign_paths(std::mem::take(&mut tree), "Book");
        let anchor = result
            .anchors
            .get(&BlockId(1))
            .expect("the subsection has an anchor");
        assert!(
            anchor.ends_with("#notes"),
            "expected the printed-title anchor, got {anchor}"
        );
    }
```

If `doc`/`h`/`Node` in that module do not compose the way this test assumes, match the shape of the neighbouring tests in the same file rather than inventing a new fixture helper — the assertion is what matters: `BlockId(1)`'s anchor ends `#notes`, not `#notes7`.

- [ ] **Step 8: Run the full suite**

Run: `mise run lint && mise run test`
Expected: PASS, including P2, P9 and the six moved slug unit tests. If P2 fails on a generated case, read it before changing anything: a real regression here means a heading path was left projecting the wrong text.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "fix(core): anchor a heading on the text its line prints

`rendered_text` contributes `[^n]` for a footnote reference, which is
visible text the writer emits and `inline_text` skipped, so `## Notes[^1]`
anchored `notes` where GitHub ids `notes1` and the cross-reference was
dead. `title_text` keeps skipping it, because a nav surface must not
carry a reference to a definition in another file, and the file title
heading anchors on the string it actually prints.

The tier shared the defect: `parse_events` dropped
`Event::FootnoteReference`, so both sides agreed on the wrong answer.
P10 draws the shape with and without the definition present."
```

---

### Task 3: Escape the ATX closing sequence

`## Intro ###` renders as the text `Intro` on GitHub — the trailing run is a *closing sequence*, stripped at block level — so its id is `intro` while kasane anchors `intro-`. The fix is in the writer: escape the run so the line renders the text the IR holds. The anchor rule is not taught about ATX, because that would concede that a rendered heading may silently drop document text.

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs` (new `atx_closing`, `one_line` deleted, tests)
- Modify: `crates/kasane-writer/src/markdown.rs:28-40`
- Modify: `crates/kasane-writer/src/lib.rs:28-58`
- Modify: `crates/kasane-writer/Cargo.toml` (promote `kasane-gfm` to a real dependency)
- Modify: `crates/kasane-writer/src/fuzz_entry.rs`
- Modify: `crates/kasane-writer/tests/generator/mod.rs` (`HOSTILE` gains the shape)
- Test: `crates/kasane-writer/tests/properties.rs` (new P11)

**Interfaces:**
- Consumes: Task 2's `kasane_gfm::fold_newlines`.
- Produces: `pub(crate) fn atx_closing(escaped: &str) -> String` in `escape.rs`, applied by both heading paths as their last step. `escape::one_line` no longer exists.

- [ ] **Step 1: Write the failing test**

In `crates/kasane-writer/tests/properties.rs`, in the same `proptest!` block:

```rust
    /// P11 — a heading ending in a `#` run renders that run, and anchors the
    /// same way GitHub ids the line (design spec 2026-08-14 §4.2).
    ///
    /// CommonMark strips a trailing `#` run preceded by a space as an ATX
    /// *closing sequence*, at block level, before inline parsing. Unescaped,
    /// the writer therefore emitted a line whose rendered text was missing
    /// document text — and an id computed from the shorter text.
    #[test]
    fn p11_a_trailing_hash_run_survives_and_anchors_the_same(
        mid in "[a-z ]{0,6}",
        hashes in 1usize..=4,
        spaced in any::<bool>(),
    ) {
        let text = format!(
            "Intro {mid}{}{}",
            if spaced { " " } else { "" },
            "#".repeat(hashes)
        );
        let inlines = vec![Inline::Text(text.clone())];
        let blocks = vec![Block::Heading {
            level: 2,
            id: BlockId(0),
            inlines: inlines.clone(),
        }];
        let md = kasane_writer::blocks_to_markdown(&blocks, &AssetBag::default());
        let parsed = parse_events(&md);

        prop_assert_eq!(
            parsed.headings.first().map(String::as_str),
            Some(text.trim()),
            "the rendered heading lost text:\n{}", md
        );

        let rendered = anchors_for_headings(&parsed.headings);
        let embedded = anchor_slug_of(&inlines);
        prop_assert_eq!(
            rendered.first().map(String::as_str),
            Some(embedded.as_str()),
            "anchor/render divergence:\n{}", md
        );
    }
```

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p kasane-writer --test properties p11_a_trailing_hash_run_survives_and_anchors_the_same`
Expected: FAIL on the first assertion, with the parsed heading missing its trailing run — `Some("Intro")` against `Some("Intro ###")`.

- [ ] **Step 3: Write the guard**

In `crates/kasane-writer/src/escape.rs`, replace `one_line` (delete it) with:

```rust
/// Disarm an ATX closing sequence at the end of a heading line (design spec
/// 2026-08-14 §4.2).
///
/// CommonMark strips a trailing run of `#` from an ATX heading — at **block**
/// level, from raw text, before inline parsing — when the run is preceded by a
/// space or tab, or is the whole content. `## Intro ###` therefore renders the
/// text `Intro`, losing document text and, with it, the id kasane computed
/// from the text the IR holds.
///
/// Escaping the first `#` of the run fixes both at once: the block-level scan
/// sees a `\` before the run and does not strip it, then inline parsing turns
/// `\#` back into a literal. `## Intro###` needs nothing, because a run with
/// no space before it was never a closing sequence.
///
/// This is a writer fix rather than an anchor-rule fix on purpose. Teaching
/// `anchor_slug` about closing sequences would buy parity by agreeing that the
/// rendered heading may drop text the document had — the escaping spec's §5
/// invariant, conceded rather than upheld.
pub(crate) fn atx_closing(escaped: &str) -> String {
    // Trailing blanks are dropped by the parser before it looks for the run,
    // so they do not protect it.
    let end = escaped.trim_end_matches([' ', '\t']).len();
    let run_start = escaped[..end].trim_end_matches('#').len();
    if run_start == end {
        return escaped.to_string();
    }
    let closes = escaped[..run_start]
        .chars()
        .next_back()
        .is_none_or(|c| c == ' ' || c == '\t');
    if !closes {
        return escaped.to_string();
    }
    let mut out = String::with_capacity(escaped.len() + 1);
    out.push_str(&escaped[..run_start]);
    out.push('\\');
    out.push_str(&escaped[run_start..]);
    out
}
```

- [ ] **Step 4: Apply it on both heading paths, and take the fold from `kasane-gfm`**

`crates/kasane-writer/Cargo.toml` — move `kasane-gfm.workspace = true` out of `[dev-dependencies]` and into `[dependencies]` (dev builds still see it).

`crates/kasane-writer/src/markdown.rs:28-40`:

```rust
        Block::Heading { level, inlines, .. } => {
            for _ in 0..(*level).min(6) {
                out.push('#');
            }
            out.push(' ');
            let inlines = escape::fold_inline_newlines(inlines);
            out.push_str(&escape::atx_closing(&kasane_gfm::fold_newlines(
                &inlines_to_md(&inlines, Ctx::Flow, Pos::Mid),
            )));
            out.push('\n');
        }
```

`crates/kasane-writer/src/lib.rs`, in `file_to_markdown` — the fold stays where it is (before escaping), because moving it would change which whitespace becomes a character reference:

```rust
    out.push_str(&escape::atx_closing(&escape::text(
        &kasane_gfm::fold_newlines(&file.frontmatter.title),
        escape::Ctx::Flow,
        escape::Pos::Mid,
    )));
```

Update the two comments that name `escape::one_line` in these files to name `kasane_gfm::fold_newlines`, and delete the sentence in `lib.rs`'s comment about the fold being kept in step with `slug::fold_newlines` by hand — it is the same function now. `escape::code_span` (`escape.rs:448`) is the third caller and takes the same substitution.

- [ ] **Step 5: Run the property to verify it passes**

Run: `cargo test -p kasane-writer --test properties p11_a_trailing_hash_run_survives_and_anchors_the_same`
Expected: PASS.

- [ ] **Step 6: Add the deterministic unit tests**

In `crates/kasane-writer/src/escape.rs`'s test module:

```rust
    #[test]
    fn atx_closing_disarms_only_a_real_closing_sequence() {
        assert_eq!(atx_closing("Intro ###"), "Intro \\###");
        assert_eq!(atx_closing("Intro\t###"), "Intro\t\\###");
        assert_eq!(atx_closing("Intro ### "), "Intro \\### ");
        assert_eq!(atx_closing("###"), "\\###");
        // No space before the run: never a closing sequence.
        assert_eq!(atx_closing("Intro###"), "Intro###");
        assert_eq!(atx_closing("Intro"), "Intro");
        assert_eq!(atx_closing(""), "");
        // Idempotent: after the guard there is nothing left to disarm.
        assert_eq!(atx_closing(&atx_closing("Intro ###")), "Intro \\###");
    }
```

In `crates/kasane-writer/src/lib.rs`'s test module, next to `the_title_heading_renders_to_exactly_the_trimmed_title`, add a parser-verified test for the title path, following that test's construction of a `FileNode` exactly and asserting the heading text:

```rust
    /// The title path needs the same guard as `Block::Heading`: this line is
    /// built here, not by `markdown.rs`.
    #[test]
    fn a_title_ending_in_hashes_keeps_them_in_the_rendered_heading() {
        use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

        let file = FileNode {
            path: "index.md".into(),
            frontmatter: Frontmatter {
                title: "Intro ###".into(),
                breadcrumb: vec!["Book".into()],
                parent: None,
                prev: None,
                next: None,
                children: vec![],
                source_pages: None,
            },
            blocks: vec![],
        };
        let md = file_to_markdown(&file, &AssetBag::default());

        let mut in_heading = false;
        let mut text = String::new();
        for ev in Parser::new_ext(&md, Options::empty()) {
            match ev {
                Event::Start(Tag::Heading { .. }) => in_heading = true,
                Event::End(TagEnd::Heading(_)) => in_heading = false,
                Event::Text(t) if in_heading => text.push_str(&t),
                _ => {}
            }
        }
        assert_eq!(text.trim(), "Intro ###", "rendered:\n{md}");
    }
```

- [ ] **Step 7: Add the fuzz postcondition and the generator shape**

In `crates/kasane-writer/src/fuzz_entry.rs`, after the `code_span` block:

```rust
    // A heading line's closing-sequence guard must leave nothing to disarm:
    // re-running it is a no-op precisely when the first pass removed the
    // condition it detects.
    let heading = escape::atx_closing(&escape::text(text, Ctx::Flow, Pos::Mid));
    assert_eq!(
        escape::atx_closing(&heading),
        heading,
        "atx_closing left a closing sequence: {heading:?} from {text:?}"
    );
```

In `crates/kasane-writer/tests/generator/mod.rs`, add to `HOSTILE` after `"#hash"`:

```rust
    // A trailing `#` run preceded by a space: an ATX *closing* sequence, which
    // the parser strips from a heading before inline parsing unless the writer
    // disarms it (2026-08-14 spec §4.2). `"#hash"` above is the line-START
    // case and does not reach this one.
    "tail ###",
```

`is_comment`'s doc comment says `HOSTILE` has 25 fragments; make it 26, and `the other 24` becomes `the other 25`. The claim it supports — that only `-->` triggers `comment_note`'s transformation — is still true of the new fragment.

- [ ] **Step 8: Run the full suite**

Run: `mise run lint && mise run test`
Expected: PASS. P1, P2 and P7 now draw the new fragment; a failure in P7's round trip on a heading means the guard is missing on a path, not that the fragment is wrong.

Run: `ASAN_OPTIONS=detect_leaks=0 mise run fuzz escape -- -max_total_time=60`
Expected: no crash. If the toolchain is unavailable in this environment, say so in the commit body rather than claiming the run happened; `cargo test -p kasane-adapters --test fuzz_corpus` replays the committed corpus on stable either way and must pass.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "fix(writer): escape a heading's ATX closing sequence

`## Intro ###` renders as the text `Intro`: CommonMark strips a trailing
`#` run preceded by a space at block level, before inline parsing. The
line therefore dropped document text, and GitHub ids it `intro` where
kasane anchors `intro-`.

Escaping the first `#` of the run keeps the text and makes today's
anchor correct. Fixing it here rather than in `anchor_slug` is the
point: teaching the anchor about closing sequences would buy parity by
conceding that a rendered heading may lose text.

`escape::one_line` is gone with it -- both heading paths and `code_span`
now fold through `kasane_gfm::fold_newlines`, so the hand-kept mirror
this branch set out to end no longer exists."
```

---

### Task 4: Re-run the external parity check

The case table pins kasane's *reading* of GitHub's algorithm, and this branch edits that reading twice. Only an external render can catch a misreading. Design spec §8.1 records the method; §8.2 records two rows that have been owed a run since 2026-08-10, and they come along.

**Files:**
- Create: `/tmp/claude-1000/-workspace/9ac9ccad-c2c7-4bba-b8fb-163ce6f6ce80/scratchpad/anchor-probe.md` (throwaway)
- Create: `/tmp/claude-1000/-workspace/9ac9ccad-c2c7-4bba-b8fb-163ce6f6ce80/scratchpad/anchor-probe-predicted.txt` (throwaway)
- Modify: nothing in the repo unless the check finds a divergence.

**Interfaces:**
- Consumes: `kasane_gfm::anchors_for_headings` from Task 2.
- Produces: a result — 
  either "N of N ids identical" or a list of divergences — for Task 5's spec and README text.

- [ ] **Step 1: Build the probe document**

One `##` heading per case, in this order: the eight rows already in design spec §8.1's table (`Fig 1½ and ①`, `می‌رود`, NFD `Café`, `Ⓐ Notes`, `⒜ Notes`, `Ⅷ Part`, `Background & Notes`, `Notes` twice), the two §8.2 newline rows (a heading with an embedded `\n` and one with `\r\n`), and this branch's four: `Notes[^1]` with its definition in the same file, `Notes[^1]` with no definition, `Intro ###`, and `###`.

Write it to the scratchpad path above. Keep one heading per case and nothing else in the file — the ids are read positionally.

- [ ] **Step 2: Compute kasane's predicted ids**

Write a throwaway test in `crates/kasane-gfm/src/slug.rs`'s test module that prints `anchors_for_headings` over the same titles in the same order, run it with `cargo test -p kasane-gfm -- --nocapture <name>`, and save the output to the predicted-ids path. Delete the test before Task 5 — it is a measurement, not a regression guard, and the case table is where the cases live.

- [ ] **Step 3: Attempt the render from here**

Follow §8.1's method: push the probe to a throwaway branch of this repo and fetch the rendered blob page, reading ids out of the rendered anchors. Do not use the `/markdown` REST endpoint — §8.1 records that it does not run the anchor filter and emits no ids.

```bash
git checkout -b throwaway-anchor-probe
cp <scratchpad>/anchor-probe.md anchor-probe.md
git add anchor-probe.md && git commit -m "chore: anchor probe (throwaway)"
git -c credential.helper= push -u origin throwaway-anchor-probe
```

Then fetch the blob page with WebFetch and read the `id=` attributes off the rendered headings. If the push or the fetch is blocked in this environment, stop and go to Step 5 — do not substitute a different oracle, and do not report a check that did not run.

- [ ] **Step 4: Diff the two lists mechanically**

Compare the fetched ids against the predicted list positionally, not by eye. Expected: every row identical except `###`, which GitHub gives no id and kasane gives `section` — the documented `EMPTY_FALLBACK` divergence. The two footnote rows must produce the *same* id as each other; that equality is §3's claim.

Any other divergence stops the branch: it means the reading is wrong, and the fix belongs in `kasane-gfm` before Task 5 writes anything down.

- [ ] **Step 5: Clean up, and record what actually happened**

```bash
git checkout shared-gfm-text-model
git branch -D throwaway-anchor-probe
git -c credential.helper= push origin --delete throwaway-anchor-probe
```

Write down, for Task 5 to use verbatim: whether the run happened, the date, the counts, and any divergence. If it did not happen, the deliverable is the probe file and the predicted ids handed to the user with a one-line request to run §8.1's method — and Task 5 records it as owed, in the same shape §8.2 uses, rather than as done.

- [ ] **Step 6: Commit (only if the repo changed)**

This task normally leaves the repo untouched. If Step 4 found a divergence and it was fixed, commit the fix with the case added to `anchor_matches_github`:

```bash
git add -A
git commit -m "fix(gfm): correct the anchor rule where the github.com render disagreed"
```

---

### Task 5: Documentation

Six documents describe a mirror that no longer exists, two divergences that are now closed, and a five-line release. All of it is now wrong.

**Files:**
- Modify: `AGENTS.md`
- Modify: `README.md`
- Modify: `crates/kasane-gfm/src/slug.rs` (module doc)
- Modify: `crates/kasane-writer/src/escape.rs` (`fold_inline_newlines`' doc)
- Modify: `docs/superpowers/specs/2026-08-09-markdown-escaping-design.md` (§4.5, §8)
- Modify: `docs/superpowers/specs/2026-08-13-escaping-residuals-design.md` (§7 pointer)
- Modify: `docs/superpowers/specs/2026-07-19-kasane-document-to-markdown-design.md` (§10, §11)
- Modify: `docs/superpowers/specs/2026-08-08-slug-widening-design.md` (§8.1/§8.2, from Task 4)
- Modify: `docs/superpowers/specs/2026-08-14-shared-gfm-text-model-design.md` (status line)

**Interfaces:**
- Consumes: Task 4's result.
- Produces: nothing code depends on.

- [ ] **Step 1: AGENTS.md**

Add a `crates/kasane-gfm` bullet to the codebase map, placed after `kasane-ir` (dependency order): what it owns — the newline fold, `title_text`/`rendered_text`, both slug rules — and why it exists, in the terms §1 of the spec uses.

In the `kasane-core` entry, **cut** the passage about the two hand-kept folds: the sentences from "They also diverge on normalization…" through "…reopens exactly the anchor mismatch this pairing closed." keep only what is still true. Specifically, the standing-hazard sentence about the two folds living in different crates with no shared function must go — the hazard is gone, and a warning about a mirror that no longer exists sends the next reader hunting for code that is not there. Replace it with one sentence: the fold and both slug rules live in `kasane-gfm`, a heading's anchor is computed from the text its line prints (`rendered_text` for a body heading, the printed title for a file's title heading), and P9/P10/P11 in `kasane-writer/tests/properties.rs` are what check the writer against that claim by parsing its own output.

In the `kasane-writer` entry, replace `escape::one_line` with `kasane_gfm::fold_newlines` and add the closing-sequence rule, including why it is a writer fix.

Also update the `kasane-core` sentence naming `slug.rs`'s divergences: two of the three "survive on purpose" cases are closed.

- [ ] **Step 2: README**

"Heading anchors match GitHub's rule, with three exceptions" becomes "with one exception". Keep `EMPTY_FALLBACK`'s bullet. The two removed bullets do not simply vanish — a reader converting a tree an older build produced needs to know their anchors changed, so add one sentence to that section: a heading carrying a footnote reference now anchors the way GitHub ids it (`#notes1`, not `#notes`), and a heading ending in a `#` run now renders the run, which is what makes its existing anchor correct.

Update the case-table path from `crates/kasane-core/src/slug.rs` to `crates/kasane-gfm/src/slug.rs`, and the oracle sentence with Task 4's result — the date and count if it ran, or the §8.2-shaped "not yet checked against a real render" if it did not.

- [ ] **Step 3: The module doc**

In `crates/kasane-gfm/src/slug.rs`, the "Known divergences that survive on purpose" section loses its footnote-reference and trailing-`#` entries and keeps the empty-id one. The paragraph introducing them ("The anchor is computed from the IR's inline text, not from what a Markdown parser gets back out of the line the writer emits") is now false as written and is replaced by the true version: the anchor is computed from `rendered_text`, the projection of what the writer emits, and the one divergence left is a choice rather than a construction defect.

Axis 4 of the four-axis table points at `text::fold_newlines`. The rest of the doc — the class derivation, the NFC argument, the drift warning — is unchanged.

In `crates/kasane-writer/src/escape.rs`, `fold_inline_newlines`' doc loses its last paragraph (the footnote residual). Replace it with: `Inline::FootnoteRef` is opaque here and that is correct — the reference is visible text between two real separators, and `rendered_text` now agrees with the fold about that.

- [ ] **Step 4: The specs**

`2026-08-09-markdown-escaping-design.md` §4.5: "Two cases remain open" becomes one. The footnote-reference bullet is rewritten as closed, naming this item and the mechanism (the projection, not a fold change — which is what that bullet predicted). The merged-table bullet stays open, unchanged. §8's approach (iii) is marked taken, with the date and the spec path.

`2026-08-13-escaping-residuals-design.md` §7: approach (iii)'s "It deserves its own item" gains one sentence — the item, its date and its spec path. Nothing else in that spec changes; it is a record of a decision at its date.

`2026-07-19-kasane-document-to-markdown-design.md`: §10's layout listing gains `kasane-gfm/`. §11 gains it too, and gets the correction it has been owed: the claim that `kasane-core` and `kasane-writer` depend on `kasane-ir` "and nothing on each other" has been untrue since the writer began taking `FileNode` and `SiteTree`. State the real graph — ir ← gfm ← core ← writer ← cli, with adapters depending on ir alone — and keep the point it was making, which is that the domain core cannot reach into an adapter.

`2026-08-08-slug-widening-design.md` §8.1/§8.2: add Task 4's run in the same shape §8.1 uses (method, result, table of the new rows). If the run did not happen, add the rows to §8.2's pending list instead, and say plainly that they are unverified against a real render.

`2026-08-14-shared-gfm-text-model-design.md`: status line becomes `Implemented`, with the oracle's outcome named.

- [ ] **Step 5: Verify the docs describe the code**

Run: `rg -n "one_line|crate::slug|kasane-core/src/slug|inline_text" --glob '!target' .`
Expected: no hit in `AGENTS.md`, `README.md`, or any source file. Hits inside the older spec documents are correct and stay — they are records of what was true then. `title_text`/`rendered_text` hits in source are the new names and are fine.

Run: `mise run lint && mise run test`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "docs: record the shared GFM crate and the two closed divergences

AGENTS.md loses the hand-kept-mirror warning rather than softening it:
the two folds are one function now, and a warning about a mirror that no
longer exists sends the next reader looking for code that is not there.
README drops two of the three anchor exceptions and says what changed
for a tree an older build produced. The escaping spec's §4.5 goes from
two open cases to one, and its §8 approach (iii) is marked taken."
```

---

## Self-Review

**Spec coverage.** §2 (the crate) → Task 1. §3 (projection, `title_text`, the two heading paths, `AnchorCounter::next(&str)`) → Task 2. §4.1 (one fold) → Task 3 Step 4; §4.2/§4.3 (closing sequence) → Task 3 Steps 3-4. §4.4 (the writer imports no projection) → held by construction: no task adds `rendered_text` to the writer's sources. §5.1 (parser blind spot first) → Task 2 Steps 1-2. §5.2 (property extension) → Tasks 2 and 3 add P10 and P11 alongside P9; note the deviation from the spec's wording, which said P9's `kind` enumeration would grow — P9's shape is `[Text, second]` with the newline run split across the boundary, and neither new case fits inside it, so they are siblings rather than arms. §5.3 (generator) → Task 3 Step 7, with footnote coverage deliberately left to P10 rather than teaching the generator `FootnoteRef` decorations, which would change P1's accounting model to reach a shape P10 hits every run. §5.4 (unit tests) → Task 2 Step 7, Task 3 Step 6. §5.5 (fuzz) → Task 1 Step 8, Task 3 Step 7. §6 (oracle) → Task 4. §8 (documentation) → Task 5. §9's sequencing risk (relocate, then change) → the Task 1/2 split.

**Placeholder scan.** No TBDs. Every code step carries the code. Task 2 Step 7 tells the implementer to match neighbouring fixtures if `doc`/`h` do not compose as written, and pins the assertion that must hold either way — a bounded instruction, not a deferred decision. Task 4 Steps 3-5 branch on whether the network is reachable, and both branches have a stated deliverable.

**Type consistency.** `fold_newlines(&str) -> String`, `title_text(&[Inline]) -> String`, `rendered_text(&[Inline]) -> String`, `anchor_slug(&str) -> String`, `AnchorCounter::next(&mut self, &str) -> String`, `path_slug(&[Inline]) -> String`, `atx_closing(&str) -> String` — used consistently in Tasks 2, 3 and 5. `NoteId(pub u32)` and `Block::Footnote { id: NoteId, blocks: Vec<Block> }` match `kasane-ir`. Task 1 exports `inline_text`; Task 2 renames it to `title_text` and updates all four call sites in the same task, so no task ends with a dangling name.
