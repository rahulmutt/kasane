# Slug Widening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace kasane's single ASCII-only slug rule with two Unicode-aware rules — anchors that mirror GitHub's heading-id algorithm exactly, and path slugs that carry the title in any script while staying a portable filename.

**Architecture:** A new `crates/kasane-core/src/slug.rs` owns both rules over a shared character class (Ruby's `\p{Word}`: Letter, Mark, Number, Connector_Punctuation) and a shared NFC + Unicode-lowercase normalization, diverging only in the tail: `anchor_slug` maps spaces to hyphens with no collapsing and no trimming (GitHub does neither), while `path_slug` collapses separator runs, trims, and caps at 64 bytes. `assign_paths` gains a per-file counter that uniquifies duplicate anchors the way GitHub does, fed in the order `file_to_markdown` renders.

**Tech Stack:** Rust 1.97.1 (pinned via mise), `unicode-normalization` and `unicode-properties` as new direct dependencies of `kasane-core`, `proptest` for the property tier, `cargo-fuzz` on `nightly-2026-07-01` for the new target.

**Spec:** `docs/superpowers/specs/2026-08-08-slug-widening-design.md`

## Global Constraints

- **Lint gate is `mise run lint`, never plain `cargo clippy`.** It runs `cargo fmt --all -- --check` **and** `cargo clippy --workspace --all-targets -- -D warnings`. Plain `cargo clippy -p <crate>` does not compile `#[cfg(test)]` modules and does not check formatting, so test-only lints and rustfmt drift accumulate silently. Run `mise run lint` at the end of every task.
- **Test gate is `mise run test`** (whole workspace).
- **Byte cap for path slugs is exactly 64** (`MAX_PATH_SLUG_BYTES`). Anchors are never capped.
- **Empty-slug fallback is the literal string `section`**, for both rules.
- **Every recursive block walk carries `kasane_ir::MAX_BLOCK_DEPTH`.** AGENTS.md keeps a counted inventory of these walks; a new one moves the count.
- **Commit messages** follow the repo's Conventional Commits style with a crate scope: `feat(core):`, `fix(core):`, `test(writer):`, `docs:`.
- **Dependency versions:** `unicode-normalization = "0.1"`, `unicode-properties = { version = "0.1", default-features = false, features = ["general-category"] }`. Both already resolve in `Cargo.lock` (0.1.25 and 0.1.4) as transitive dependencies, so this adds direct edges, not a new subtree.

---

## File Structure

**Created:**
- `crates/kasane-core/src/slug.rs` — both slug rules, the shared character class, the `AnchorCounter`, and `inline_text` (moved here from `paths.rs`). One responsibility: turning inline runs into strings that are safe in a given position.
- `crates/kasane-core/src/fuzz_entry.rs` — the fuzz seam for `path_slug`, mirroring `kasane-adapters`'s module of the same name.
- `fuzz/fuzz_targets/slug.rs` — the libFuzzer wrapper.
- `fuzz/seeds/slug/*.txt` — hand-written seeds.

**Modified:**
- `crates/kasane-core/Cargo.toml` — two dependencies.
- `crates/kasane-core/src/lib.rs` — `mod slug`, `mod fuzz_entry`, exports.
- `crates/kasane-core/src/paths.rs` — calls the two rules; gains the per-file counter and the bounded render-order body walk; loses `slug` and `inline_text`.
- `crates/kasane-core/src/{nav,refs,balance}.rs` — `inline_text` import path only.
- `crates/kasane-writer/tests/properties.rs` — the ordered-anchor seam, the `_` strip fix.
- `crates/kasane-writer/tests/generator/mod.rs` — non-Latin and punctuation-bearing filler words.
- `crates/kasane-writer/src/markdown.rs` — a test asserting the closed destination character set.
- `crates/kasane-adapters/Cargo.toml` — `kasane-core` as a dev-dependency.
- `crates/kasane-adapters/tests/fuzz_corpus.rs` — dispatch the new target; `TARGET_COUNT` 12 → 13.
- `fuzz/Cargo.toml` — `kasane-core` path dependency and a `[[bin]]`.
- `README.md`, `AGENTS.md` — documentation.

---

## Task 1: The two slug rules

Creates `slug.rs` with both rules and wires `paths.rs` to them. Duplicate suffixing is **not** in this task — Task 2 adds it. Every existing test stays green: `Background & Notes` still yields the path `background-notes`, and `Methods` still yields the anchor `methods`.

**Files:**
- Create: `crates/kasane-core/src/slug.rs`
- Modify: `crates/kasane-core/Cargo.toml`
- Modify: `crates/kasane-core/src/lib.rs`
- Modify: `crates/kasane-core/src/paths.rs` (delete `slug` at 81-104 and `inline_text` at 106-127; update the three call sites at 30, 39, 46)
- Modify: `crates/kasane-core/src/nav.rs:2`, `crates/kasane-core/src/refs.rs:116`, `crates/kasane-core/src/balance.rs:251` (import path only)
- Test: `crates/kasane-core/src/slug.rs` (inline `#[cfg(test)] mod tests`, matching the crate's convention)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `pub(crate) fn anchor_slug(inlines: &[Inline]) -> String`
  - `pub(crate) fn path_slug(inlines: &[Inline]) -> String`
  - `pub(crate) fn inline_text(inlines: &[Inline]) -> String` (moved, unchanged)
  - `pub(crate) const MAX_PATH_SLUG_BYTES: usize = 64`
  - `#[doc(hidden)] pub fn path_slug_of(inlines: &[Inline]) -> String` — test seam, re-exported from `lib.rs`
  - `pub use paths::slug_of` continues to exist and now forwards to `anchor_slug`; Task 3 removes it.

- [ ] **Step 1: Add the two dependencies**

In `crates/kasane-core/Cargo.toml`, replace the `[dependencies]` section:

```toml
[dependencies]
kasane-ir.workspace = true
# NFC, and the General_Category tables behind Ruby's `\p{Word}`. std cannot
# answer the Mark question: `char::is_alphanumeric()` is Alphabetic + Numeric,
# and after NFC the Devanagari virama (U+094D) is still a separate Mark, so
# `हिन्दी` would slug to `हिनदी` without these. `default-features = false`
# drops the emoji tables, which nothing here needs.
unicode-normalization = "0.1"
unicode-properties = { version = "0.1", default-features = false, features = ["general-category"] }
```

- [ ] **Step 2: Write the failing test file**

Create `crates/kasane-core/src/slug.rs` containing **only** the test module for now, so the test compiles against functions that do not exist yet:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kasane_ir::Inline;

    fn t(s: &str) -> Vec<Inline> {
        vec![Inline::Text(s.to_string())]
    }

    /// Each row names the rule it pins. These are derived from GitHub's TOC
    /// filter: downcase, remove everything outside `\p{Word}`/`-`/space, then
    /// map spaces to hyphens. No collapsing and no trimming of interior runs,
    /// which is why some rows look wrong and are not.
    #[test]
    fn anchor_matches_github() {
        // punctuation is REMOVED, not replaced -- the old rule made this
        // `don-t-panic`, which resolved nowhere on GitHub.
        assert_eq!(anchor_slug(&t("Don't Panic")), "dont-panic");
        // `&` is removed and BOTH surviving spaces become hyphens. The double
        // hyphen is correct GFM output, not a bug.
        assert_eq!(anchor_slug(&t("Background & Notes")), "background--notes");
        // `_` is Connector_Punctuation, which is inside `\p{Word}`.
        assert_eq!(anchor_slug(&t("foo_bar")), "foo_bar");
        // CJK passes through untouched; there is nothing to downcase.
        assert_eq!(anchor_slug(&t("第二章")), "第二章");
        // Devanagari matras and the virama are Marks, also inside `\p{Word}`.
        assert_eq!(anchor_slug(&t("हिन्दी")), "हिन्दी");
        // Symbols are outside `\p{Word}`; the two spaces around the emoji
        // survive as two hyphens.
        assert_eq!(anchor_slug(&t("Hello 🎉 World")), "hello--world");
        // Unicode-aware downcasing.
        assert_eq!(anchor_slug(&t("CHAPTER One")), "chapter-one");
        // Outer whitespace is trimmed because a Markdown renderer strips it
        // from the heading's text before computing the id. Interior runs are
        // not trimmed, per the row above.
        assert_eq!(anchor_slug(&t("  Intro  ")), "intro");
        // No Word character at all: GitHub emits an empty id, which would be a
        // dead link here. The documented divergence.
        assert_eq!(anchor_slug(&t("***")), "section");
    }

    /// NFC runs before anything else, so a decomposed title and its composed
    /// twin cannot slug differently. macOS-sourced text is the realistic
    /// source of NFD input.
    #[test]
    fn nfd_and_nfc_agree() {
        let nfc = "Café"; // é = U+00E9
        let nfd = "Cafe\u{0301}"; // e + COMBINING ACUTE
        assert_eq!(anchor_slug(&t(nfc)), anchor_slug(&t(nfd)));
        assert_eq!(anchor_slug(&t(nfc)), "café");
        assert_eq!(path_slug(&t(nfc)), path_slug(&t(nfd)));
    }

    /// Same character class as the anchor, then it diverges where a filename
    /// should: separator runs collapse, the result is trimmed and capped.
    #[test]
    fn path_slug_is_a_filename_not_an_anchor() {
        assert_eq!(path_slug(&t("Don't Panic")), "dont-panic");
        // where the anchor keeps `background--notes`
        assert_eq!(path_slug(&t("Background & Notes")), "background-notes");
        assert_eq!(path_slug(&t("foo_bar")), "foo_bar");
        assert_eq!(path_slug(&t("第二章")), "第二章");
        assert_eq!(path_slug(&t("Hello 🎉 World")), "hello-world");
        assert_eq!(path_slug(&t("***")), "section");
        // A leading separator is never emitted, so nothing needs trimming off
        // the front.
        assert_eq!(path_slug(&t("  Intro  ")), "intro");
    }

    /// Traversal and separator injection are impossible by construction: `/`,
    /// `\`, `.`, NUL, the fullwidth solidus and the RTL override are all
    /// outside `\p{Word}` and are removed rather than mapped to anything.
    #[test]
    fn path_slug_cannot_emit_a_separator() {
        assert_eq!(path_slug(&t("../../etc/passwd")), "etcpasswd");
        assert_eq!(path_slug(&t("a\\b")), "ab");
        assert_eq!(path_slug(&t("..")), "section");
        assert_eq!(path_slug(&t("a\u{FF0F}b")), "ab");
        assert_eq!(path_slug(&t("a\u{202E}b")), "ab");
        assert_eq!(path_slug(&t("a\u{0}b")), "ab");
    }

    /// 64 bytes, cut on a char boundary. A CJK title hits the cap three times
    /// faster than a Latin one: 64/3 = 21 characters, 63 bytes.
    #[test]
    fn path_slug_caps_at_the_byte_budget() {
        let long = "第".repeat(40);
        let out = path_slug(&t(&long));
        assert_eq!(out, "第".repeat(21));
        assert!(out.len() <= MAX_PATH_SLUG_BYTES);
        // Anchors are not filenames and are deliberately uncapped.
        assert_eq!(anchor_slug(&t(&long)).len(), 120);
    }

    /// Truncation must not leave a hyphen or a combining mark with nothing to
    /// attach to. 61 `a`s plus a 3-byte virama is exactly 64 bytes, so nothing
    /// is cut -- the trailing mark is dropped by the tail trim, not the cap.
    #[test]
    fn path_slug_trims_a_dangling_tail() {
        let s = format!("{}{}", "a".repeat(61), "\u{094D}");
        assert_eq!(s.len(), 64);
        assert_eq!(path_slug(&t(&s)), "a".repeat(61));
        assert_eq!(path_slug(&t("Intro -")), "intro");
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p kasane-core --lib slug`
Expected: FAIL — `file not found for module 'slug'` (the module is not declared yet), or once declared, `cannot find function 'anchor_slug' in this scope`.

- [ ] **Step 4: Declare the module**

In `crates/kasane-core/src/lib.rs`, add `mod slug;` to the module list (alphabetical, after `sitetree`) and add to the exports:

```rust
mod balance;
mod nav;
mod options;
mod paths;
mod refs;
mod section;
mod sitetree;
mod slug;

pub use balance::{balance, est_tokens};
pub use nav::structure;
pub use options::Options;
pub use paths::{assign_paths, slug_of, PlaceResult, Placed};
pub use refs::resolve_refs;
pub use section::{fold_sections, SectionNode, SectionTree};
pub use sitetree::{FileNode, Frontmatter, SiteTree};
pub use slug::path_slug_of;
```

- [ ] **Step 5: Write the implementation**

Prepend this to `crates/kasane-core/src/slug.rs`, above the test module:

```rust
//! The two slug rules.
//!
//! `anchor_slug` is a deliberate mirror of GitHub's heading-id algorithm, so
//! an in-book cross-reference resolves when the tree is rendered on GitHub.
//! `path_slug` turns the same text into a portable file or directory name.
//!
//! They share a character class and a normalization step and diverge only in
//! the tail. That is deliberate, not an oversight: an anchor lands in the
//! fragment of a link and a path slug lands in the path portion, so nothing
//! forces them to agree and nothing breaks when they don't.
//!
//! Being a mirror, `anchor_slug` carries drift risk against github.com, the
//! same class the PDF adapter took on mirroring `lopdf`. The case table in
//! this file's tests is where that mirror is written down.

use kasane_ir::Inline;
use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategory, GeneralCategoryGroup, UnicodeGeneralCategory};

/// Byte budget for one path slug.
///
/// Roughly 64 Latin characters or 21 CJK ones -- comfortably a chapter title.
/// With the `NN-` ordinal prefix and the `.md` suffix that `paths.rs` adds, a
/// component stays far inside the 255-byte per-component limit. Anchors are
/// deliberately NOT capped: they are not filenames, and capping them would
/// break GFM parity for no benefit.
pub(crate) const MAX_PATH_SLUG_BYTES: usize = 64;

/// Emitted when a title has no `\p{Word}` character at all (`## ***`, `## —`).
///
/// GitHub gives such a heading an empty id. kasane cannot: an empty anchor is
/// a dead link. This is the one documented divergence from GFM.
const EMPTY_FALLBACK: &str = "section";

/// Ruby's `\p{Word}`, which is exactly what GitHub's TOC filter keeps: Letter,
/// Mark, Number, and Connector_Punctuation (so `_` survives).
///
/// Mark is why this needs a table rather than `char::is_alphanumeric()`.
/// After NFC the Devanagari virama (U+094D) is still a separate Mark, and
/// dropping it would slug `हिन्दी` as `हिनदी`.
fn is_word(c: char) -> bool {
    matches!(
        c.general_category_group(),
        GeneralCategoryGroup::Letter | GeneralCategoryGroup::Mark | GeneralCategoryGroup::Number
    ) || c.general_category() == GeneralCategory::ConnectorPunctuation
}

/// The shared prefix of both rules: the inline text, outer whitespace trimmed,
/// NFC-normalized, Unicode-lowercased.
///
/// The trim mirrors the renderer rather than the filter: a Markdown parser
/// strips a heading's surrounding whitespace before GitHub ever computes an
/// id, so `##   Intro  ` and `## Intro` anchor identically. Interior runs are
/// left alone, which is what produces the double hyphens.
fn normalized(inlines: &[Inline]) -> String {
    inline_text(inlines)
        .trim()
        .nfc()
        .flat_map(char::to_lowercase)
        .collect()
}

/// GitHub's algorithm, in its order: normalize, downcase, remove everything
/// outside `\p{Word}`/`-`/space, then map each remaining space to `-`.
///
/// No run-collapsing and no interior trimming, because GitHub does neither.
/// Exact parity therefore means deliberately emitting anchors that look wrong:
/// `Background & Notes` anchors as `background--notes`, since the `&` is
/// removed and each of the two surviving spaces becomes a hyphen.
pub(crate) fn anchor_slug(inlines: &[Inline]) -> String {
    let out: String = normalized(inlines)
        .chars()
        .filter(|c| is_word(*c) || *c == '-' || *c == ' ')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect();
    if out.is_empty() {
        EMPTY_FALLBACK.to_string()
    } else {
        out
    }
}

/// The same character class and normalization, then it diverges where a
/// filename should: separator runs collapse to a single `-`, the tail is
/// trimmed, and the result is capped at `MAX_PATH_SLUG_BYTES`.
///
/// Everything outside `\p{Word}` is REMOVED, exactly as the anchor rule
/// removes it -- only space and `-` act as separators. That is what makes
/// `Don't Panic` a `dont-panic` file rather than the old `don-t-panic`.
///
/// Truncation can make two sibling slugs identical. That is harmless: every
/// non-root component carries an `NN-` ordinal prefix, which is already what
/// makes sibling collisions impossible -- including the case-insensitive ones
/// macOS and Windows would produce, and the NFC-vs-NFD ones macOS would.
pub(crate) fn path_slug(inlines: &[Inline]) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in normalized(inlines).chars() {
        if is_word(c) {
            out.push(c);
            prev_dash = false;
        } else if (c == ' ' || c == '-') && !prev_dash && !out.is_empty() {
            // `!out.is_empty()` is what makes a leading separator impossible,
            // so there is nothing to trim off the front later.
            out.push('-');
            prev_dash = true;
        }
    }
    truncate_to(&mut out, MAX_PATH_SLUG_BYTES);
    trim_tail(&mut out);
    if out.is_empty() {
        EMPTY_FALLBACK.to_string()
    } else {
        out
    }
}

/// Truncate to at most `max` bytes without splitting a `char`.
fn truncate_to(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

/// Drop trailing `-` and trailing combining marks, so neither the collapse
/// rule nor the cap can leave a dangling hyphen or a mark with nothing to
/// attach to.
fn trim_tail(s: &mut String) {
    while s
        .chars()
        .next_back()
        .is_some_and(|c| c == '-' || c.general_category_group() == GeneralCategoryGroup::Mark)
    {
        s.pop();
    }
}

/// Test seam for `path_slug`, same rationale as `est_tokens` and `slug_of`:
/// the fuzz seam and the property tier need the engine's own rule rather than
/// a copy of it that can drift.
#[doc(hidden)]
pub fn path_slug_of(inlines: &[Inline]) -> String {
    path_slug(inlines)
}

/// The visible text of an inline run, bounded by `MAX_INLINE_DEPTH`.
///
/// Moved here from `paths.rs`: it exists to feed the slug rules, and leaving
/// it there would make `paths` and `slug` mutually dependent.
pub(crate) fn inline_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    inline_text_at(inlines, 0, &mut s);
    s
}

fn inline_text_at(inlines: &[Inline], depth: usize, s: &mut String) {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return;
    }
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => s.push_str(t),
            Inline::Emph(x) | Inline::Strong(x) => inline_text_at(x, depth + 1, s),
            Inline::Link { inlines, .. } => inline_text_at(inlines, depth + 1, s),
            Inline::FootnoteRef(_) => {}
        }
    }
}
```

- [ ] **Step 6: Run the slug tests to verify they pass**

Run: `cargo test -p kasane-core --lib slug`
Expected: PASS — seven tests.

If `unicode_properties`' item names do not resolve, check the crate's actual API before improvising: `GeneralCategoryGroup` has variants `Letter`, `Mark`, `Number`, `Punctuation`, `Symbol`, `Separator`, `Other`, and `GeneralCategory::ConnectorPunctuation` is the `Pc` variant.

- [ ] **Step 7: Wire `paths.rs` to the new rules**

In `crates/kasane-core/src/paths.rs`, **delete** `slug` (lines 81-104), `slug_of` (lines 72-80), and `inline_text` plus `inline_text_at` (lines 106-127). Replace the import line at the top and the three call sites:

```rust
use crate::section::{SectionNode, SectionTree};
use crate::slug::{anchor_slug, path_slug};
use kasane_ir::{Block, BlockId, Inline};
use std::collections::HashMap;
```

At line 30: `anchors.insert(id, format!("{}#{}", self_path, anchor_slug(&node.title)));`

At line 39: `anchors.insert(*id, format!("{}#{}", self_path, anchor_slug(inlines)));`

At line 46: `let child_slug = path_slug(&child.title);`

The `Inline` import stays — `place`'s body-heading match arm still binds `inlines`.

- [ ] **Step 8: Re-home `slug_of` and the three `inline_text` importers**

In `crates/kasane-core/src/slug.rs`, add the forwarding seam next to `path_slug_of` (Task 3 removes it):

```rust
/// Anchor-rule test seam. Kept under its historical name so
/// `kasane-writer`'s property tier keeps compiling; Task 3 replaces it with
/// the ordered form that duplicate suffixing requires.
#[doc(hidden)]
pub fn slug_of(inlines: &[Inline]) -> String {
    anchor_slug(inlines)
}
```

In `crates/kasane-core/src/lib.rs`, move `slug_of` off the `paths` export line:

```rust
pub use paths::{assign_paths, PlaceResult, Placed};
pub use slug::{path_slug_of, slug_of};
```

Update the three importers:
- `crates/kasane-core/src/nav.rs:2` → `use crate::paths::{assign_paths, Placed};` plus `use crate::slug::inline_text;`
- `crates/kasane-core/src/refs.rs:116` → `Inline::Text(crate::slug::inline_text(&inlines))`
- `crates/kasane-core/src/balance.rs:251` → `.map(|c| crate::slug::inline_text(&c.title))`

- [ ] **Step 9: Run the full suite**

Run: `mise run test`
Expected: PASS. `paths.rs`'s own tests are unchanged and must stay green — `Background & Notes` still produces `01-background-notes.md` (the `&` is removed, both spaces collapse to one hyphen) and `Methods` still anchors as `methods`.

- [ ] **Step 10: Lint**

Run: `mise run lint`
Expected: clean. `clippy` may object to `format!` in a `push_str`; follow whatever it says rather than suppressing it.

- [ ] **Step 11: Commit**

```bash
git add crates/kasane-core/Cargo.toml crates/kasane-core/src/slug.rs \
        crates/kasane-core/src/lib.rs crates/kasane-core/src/paths.rs \
        crates/kasane-core/src/nav.rs crates/kasane-core/src/refs.rs \
        crates/kasane-core/src/balance.rs Cargo.lock
git commit -m "feat(core): split the slug rule into a GFM anchor and a portable path slug"
```

---

## Task 2: Duplicate anchor suffixing

GitHub uniquifies heading ids per rendered page in document order: the first occurrence of a base keeps it, the next gets `-1`, then `-2`. Two headings titled `Notes` in one kasane file currently produce the same anchor, so one cross-reference silently lands on the other's heading.

Two things make this more than a counter. First, `file_to_markdown` renders **every** file's title as a heading, and for `index.md` that title is the *document* title, not `node.title` (which is empty for the root) — so `assign_paths` has to be told the document title or the root's count is off by one. Second, a heading nested inside a list item is never given an anchor (deliberately — it was never folded into a section) but GitHub still assigns it an id when it renders, so it **consumes a counter slot**.

**Files:**
- Modify: `crates/kasane-core/src/slug.rs` (add `AnchorCounter`)
- Modify: `crates/kasane-core/src/paths.rs` (`assign_paths` signature, `place`, new bounded walk)
- Modify: `crates/kasane-core/src/nav.rs:37` (pass the document title)
- Test: `crates/kasane-core/src/paths.rs` (inline test module)

**Interfaces:**
- Consumes: `anchor_slug`, `path_slug`, `MAX_PATH_SLUG_BYTES` from Task 1.
- Produces:
  - `pub(crate) struct AnchorCounter` with `pub(crate) fn new() -> Self` and `pub(crate) fn next(&mut self, inlines: &[Inline]) -> String`
  - `pub fn assign_paths(tree: SectionTree, doc_title: &str) -> PlaceResult` — **signature change**, one caller (`nav.rs:37`)

- [ ] **Step 1: Write the failing tests**

Add these to the existing `#[cfg(test)] mod tests` in `crates/kasane-core/src/paths.rs`. The two tests already there (`assigns_index_and_leaf_paths`, `body_headings_get_anchors_too`) need their `assign_paths` calls updated to pass a title — use `"B"` for the first (it builds `doc()` with `title: "B"`) and `""` for the second.

```rust
    #[test]
    fn duplicate_titles_get_github_style_suffixes() {
        // Two body headings with the same title. GitHub gives the first the
        // bare id and the second `-1`; before this, both got `#notes` and one
        // cross-reference silently landed on the other's heading.
        let tree = SectionTree {
            root: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body: vec![
                    Block::Heading {
                        level: 2,
                        id: BlockId(1),
                        inlines: vec![Inline::Text("Notes".into())],
                    },
                    Block::Heading {
                        level: 2,
                        id: BlockId(2),
                        inlines: vec![Inline::Text("Notes".into())],
                    },
                ],
                children: vec![],
                pages: None,
            },
        };
        let placed = assign_paths(tree, "Book");
        assert_eq!(placed.anchors[&BlockId(1)], "index.md#notes");
        assert_eq!(placed.anchors[&BlockId(2)], "index.md#notes-1");
    }

    #[test]
    fn the_files_own_title_consumes_the_first_slot() {
        // `file_to_markdown` prepends the file's title as a heading, so a body
        // heading repeating that title is the SECOND occurrence on the page.
        let tree = fold_sections(&doc(vec![
            h(1, 0, "Notes"),
            Node {
                block: Block::Heading {
                    level: 3,
                    id: BlockId(5),
                    inlines: vec![Inline::Text("Notes".into())],
                },
                prov: Provenance::default(),
            },
        ]));
        let placed = assign_paths(tree, "Book");
        assert_eq!(placed.anchors[&BlockId(0)], "01-notes.md#notes");
        assert_eq!(placed.anchors[&BlockId(5)], "01-notes.md#notes-1");
    }

    #[test]
    fn the_root_file_counts_the_document_title() {
        // index.md renders the DOCUMENT title as its heading -- the root
        // node's own `title` is empty. Counting `node.title` here would slug
        // `section` and leave the body heading unsuffixed against a page that
        // really does have two `#book` ids.
        let tree = SectionTree {
            root: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body: vec![Block::Heading {
                    level: 2,
                    id: BlockId(3),
                    inlines: vec![Inline::Text("Book".into())],
                }],
                children: vec![],
                pages: None,
            },
        };
        let placed = assign_paths(tree, "Book");
        assert_eq!(placed.anchors[&BlockId(3)], "index.md#book-1");
    }

    #[test]
    fn a_nested_heading_consumes_a_slot_without_getting_an_anchor() {
        // A heading inside a list item was never folded into a section, so it
        // gets no anchor -- but GitHub still gives it an id when it renders,
        // so it takes `notes-1` and pushes the next top-level heading to
        // `notes-2`. Counting only top-level headings would put `-1` on the
        // wrong heading.
        let tree = SectionTree {
            root: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body: vec![
                    Block::Heading {
                        level: 2,
                        id: BlockId(1),
                        inlines: vec![Inline::Text("Notes".into())],
                    },
                    Block::List {
                        ordered: false,
                        items: vec![vec![Block::Heading {
                            level: 3,
                            id: BlockId(2),
                            inlines: vec![Inline::Text("Notes".into())],
                        }]],
                    },
                    Block::Heading {
                        level: 2,
                        id: BlockId(3),
                        inlines: vec![Inline::Text("Notes".into())],
                    },
                ],
                children: vec![],
                pages: None,
            },
        };
        let placed = assign_paths(tree, "Book");
        assert_eq!(placed.anchors[&BlockId(1)], "index.md#notes");
        assert!(
            !placed.anchors.contains_key(&BlockId(2)),
            "a nested heading must not gain an anchor"
        );
        assert_eq!(placed.anchors[&BlockId(3)], "index.md#notes-2");
    }

    /// Pins the bound's position, matching how the other core walks are
    /// tested: nesting past `MAX_BLOCK_DEPTH` must return rather than recurse,
    /// and a heading below the bound must simply be unreachable.
    #[test]
    fn the_counting_walk_is_bounded() {
        let mut inner = vec![Block::Heading {
            level: 3,
            id: BlockId(99),
            inlines: vec![Inline::Text("Deep".into())],
        }];
        for _ in 0..(kasane_ir::MAX_BLOCK_DEPTH + 2) {
            inner = vec![Block::List {
                ordered: false,
                items: vec![inner],
            }];
        }
        let tree = SectionTree {
            root: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body: inner,
                children: vec![],
                pages: None,
            },
        };
        let placed = assign_paths(tree, "Book");
        assert!(!placed.anchors.contains_key(&BlockId(99)));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-core --lib paths`
Expected: FAIL — `assign_paths` takes 1 argument, not 2.

- [ ] **Step 3: Add the counter to `slug.rs`**

Append to `crates/kasane-core/src/slug.rs`, above the test module:

```rust
/// Assigns anchors to one file's headings in render order, uniquifying
/// duplicates the way GitHub does.
///
/// One instance per file. The first occurrence of a base keeps it, the next
/// gets `-1`, then `-2`. GitHub does not re-check whether the suffixed form
/// itself collides with an existing id, and neither does this -- mirroring the
/// quirk is the point.
pub(crate) struct AnchorCounter {
    seen: std::collections::HashMap<String, usize>,
}

impl AnchorCounter {
    pub(crate) fn new() -> Self {
        Self {
            seen: std::collections::HashMap::new(),
        }
    }

    /// The anchor for the next heading in render order. Every heading the file
    /// renders must pass through here, including ones that get no anchor of
    /// their own -- they still consume a slot on the rendered page.
    pub(crate) fn next(&mut self, inlines: &[Inline]) -> String {
        let base = anchor_slug(inlines);
        let n = self.seen.entry(base.clone()).or_insert(0);
        let out = if *n == 0 {
            base
        } else {
            format!("{base}-{n}")
        };
        *n += 1;
        out
    }
}
```

- [ ] **Step 4: Rewrite `place` and `assign_paths`**

Replace `assign_paths` and `place` in `crates/kasane-core/src/paths.rs`:

```rust
/// `doc_title` is what `file_to_markdown` renders as `index.md`'s heading --
/// the root `SectionNode`'s own title is empty, and `nav::walk` substitutes
/// the document title there. The anchor counter has to see the text the file
/// actually renders or the root's duplicate suffixes are off by one.
pub fn assign_paths(tree: SectionTree, doc_title: &str) -> PlaceResult {
    let mut anchors = HashMap::new();
    let root = place(tree.root, "index.md", "", doc_title, &mut anchors);
    PlaceResult { root, anchors }
}

// self_path: this node's markdown file path. dir: directory children live in.
// doc_title: only meaningful for the root; see `assign_paths`.
fn place(
    mut node: SectionNode,
    self_path: &str,
    dir: &str,
    doc_title: &str,
    anchors: &mut HashMap<BlockId, String>,
) -> Placed {
    // One counter per file, fed in the order `file_to_markdown` renders: the
    // title heading it prepends, then the body.
    let mut counter = AnchorCounter::new();

    // Every file renders a title heading, so every file consumes this slot --
    // including `index.md`, whose heading is the document title rather than
    // the (empty) root node title. `nav::walk` pins the substitution on
    // `id.is_none() && trail.is_empty()`; `dir.is_empty()` is the same
    // condition here.
    let title_anchor = if node.id.is_none() && dir.is_empty() {
        counter.next(&[Inline::Text(doc_title.to_string())])
    } else {
        counter.next(&node.title)
    };
    if let Some(id) = node.id {
        anchors.insert(id, format!("{}#{}", self_path, title_anchor));
    }

    // A merged subsection's heading lives in its parent's body (balance.rs
    // demotes it there), and nothing else would give it an anchor. Only
    // top-level body blocks are ANCHORED: a heading nested inside a list item
    // was never folded into a section either, and giving it an anchor would
    // invent structure the engine does not model. Every rendered heading is
    // still COUNTED, nested ones included, because GitHub assigns them ids and
    // they therefore consume duplicate-suffix slots.
    count_headings(&node.body, 0, true, self_path, &mut counter, anchors);

    let children = std::mem::take(&mut node.children);
    let mut placed = Vec::new();
    for (i, child) in children.into_iter().enumerate() {
        let n = i + 1;
        let child_slug = path_slug(&child.title);
        if child.children.is_empty() {
            let p = join(dir, &format!("{:02}-{}.md", n, child_slug));
            placed.push(place(child, &p, dir, doc_title, anchors));
        } else {
            let cdir = join(dir, &format!("{:02}-{}", n, child_slug));
            let p = format!("{}/index.md", cdir);
            placed.push(place(child, &p, &cdir, doc_title, anchors));
        }
    }
    Placed {
        path: self_path.to_string(),
        node,
        children: placed,
    }
}

/// Walks a file's blocks in render order, feeding every heading to the
/// counter and anchoring only the top-level ones.
///
/// Recursive on block nesting, so it carries `kasane_ir::MAX_BLOCK_DEPTH` like
/// every other block walk in this crate. Past the bound the subtree renders as
/// a truncation note with no headings in it, so stopping here costs nothing.
fn count_headings(
    blocks: &[Block],
    depth: usize,
    top_level: bool,
    self_path: &str,
    counter: &mut AnchorCounter,
    anchors: &mut HashMap<BlockId, String>,
) {
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        return;
    }
    for b in blocks {
        match b {
            Block::Heading { id, inlines, .. } => {
                let a = counter.next(inlines);
                if top_level {
                    anchors.insert(*id, format!("{}#{}", self_path, a));
                }
            }
            Block::List { items, .. } => {
                for item in items {
                    count_headings(item, depth + 1, false, self_path, counter, anchors);
                }
            }
            Block::Footnote { blocks, .. } => {
                count_headings(blocks, depth + 1, false, self_path, counter, anchors);
            }
            _ => {}
        }
    }
}
```

Update the imports at the top of `paths.rs`:

```rust
use crate::section::{SectionNode, SectionTree};
use crate::slug::{path_slug, AnchorCounter};
use kasane_ir::{Block, BlockId, Inline};
use std::collections::HashMap;
```

`anchor_slug` is no longer called directly from `paths.rs` — every anchor now goes through the counter.

- [ ] **Step 5: Update the one caller**

In `crates/kasane-core/src/nav.rs:37`:

```rust
    let mut result = assign_paths(tree, &root_title);
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p kasane-core --lib paths`
Expected: PASS — seven tests, including the two pre-existing ones.

- [ ] **Step 7: Run the full suite**

Run: `mise run test`
Expected: PASS.

- [ ] **Step 8: Lint**

Run: `mise run lint`
Expected: clean.

- [ ] **Step 9: Commit**

```bash
git add crates/kasane-core/src/slug.rs crates/kasane-core/src/paths.rs crates/kasane-core/src/nav.rs
git commit -m "fix(core): uniquify duplicate heading anchors the way GitHub does"
```

---

## Task 3: Property tier, generator, and the closed destination character set

The property suite's P2 (link resolution) recomputes the engine's slug from each rendered heading line. Duplicate suffixing breaks that: an anchor is no longer a function of one heading's text. The seam has to become ordered.

One existing line in that helper also inverts. `heading_slugs` strips `*`, `_`, and `` ` `` from the rendered line, because `inlines_to_md` writes those around `Emph`/`Strong`/`Code` while the engine slugs `inline_text`, which never sees a marker. `_` was in that set defensively — the writer never emits it — but `_` is now a Word character, so a heading `foo_bar` anchors as `foo_bar` while the helper would compute `foobar`.

**Files:**
- Modify: `crates/kasane-core/src/slug.rs` (replace `slug_of` with the ordered seam)
- Modify: `crates/kasane-core/src/lib.rs` (export)
- Modify: `crates/kasane-writer/tests/properties.rs:99-105` (the helper) and its two call sites at 164 and 366
- Modify: `crates/kasane-writer/tests/generator/mod.rs:51-53` (`WORDS`)
- Test: `crates/kasane-writer/src/markdown.rs` (new unit test for §4's closed character set)

**Interfaces:**
- Consumes: `AnchorCounter`, `anchor_slug`, `path_slug_of` from Tasks 1-2.
- Produces: `#[doc(hidden)] pub fn anchors_for_headings(headings: &[String]) -> Vec<String>`, re-exported from `lib.rs`. `slug_of` is **removed**.

- [ ] **Step 1: Write the failing writer test for the closed character set**

Add to the `#[cfg(test)] mod tests` in `crates/kasane-writer/src/markdown.rs`:

```rust
    /// Design spec §4: link destinations are emitted raw, with no
    /// percent-encoding, and that is safe because the character set of a path
    /// component is closed. Every character that would break a bare Markdown
    /// destination -- space, `(`, `)`, `#`, `?`, `%` -- is outside `\p{Word}`
    /// and is therefore already removed by the slug rule.
    ///
    /// This asserts the set rather than the argument, so widening the rule by
    /// hand fails here instead of silently emitting a broken link.
    #[test]
    fn path_slugs_contain_nothing_that_breaks_a_bare_destination() {
        for title in [
            "Don't Panic",
            "Background & Notes (revised)",
            "50% off #1?",
            "第二章",
            "a/b\\c",
        ] {
            let slug = kasane_core::path_slug_of(&[Inline::Text(title.into())]);
            for c in slug.chars() {
                assert!(
                    c == '-' || (c.is_alphanumeric() || c == '_'),
                    "path slug for {title:?} contains {c:?}, which is outside the closed set"
                );
            }
            for bad in [' ', '(', ')', '#', '?', '%', '/', '\\', '.'] {
                assert!(
                    !slug.contains(bad),
                    "path slug for {title:?} contains {bad:?}"
                );
            }
        }
    }
```

`kasane-writer` already depends on `kasane-core`, so no manifest change is needed. Note the assertion uses `is_alphanumeric()` deliberately — it is a *narrower* set than the rule, which is what makes it a useful check on Latin/CJK inputs; none of these titles contain a combining mark.

- [ ] **Step 2: Run it**

Run: `cargo test -p kasane-writer --lib path_slugs_contain`
Expected: **PASS**. This one is a regression guard, not a red-green driver — Task 1 already made it true, and its job is to fail if someone later widens the character class by hand. If it fails now, the character class in `slug.rs` is wrong and Task 1's tests missed it.

- [ ] **Step 3: Replace `slug_of` with the ordered seam**

In `crates/kasane-core/src/slug.rs`, delete the `slug_of` function added in Task 1 Step 8 and add:

```rust
/// Anchors for one file's headings, in the order the file renders them.
///
/// Test seam for the property tier, and the reason `slug_of` could not stay:
/// with duplicate suffixing an anchor depends on what preceded it on the page,
/// so a per-heading function cannot express the rule. The tier asserts against
/// the engine's own counter rather than a copy of it that can drift.
#[doc(hidden)]
pub fn anchors_for_headings(headings: &[String]) -> Vec<String> {
    let mut counter = AnchorCounter::new();
    headings
        .iter()
        .map(|t| counter.next(&[Inline::Text(t.clone())]))
        .collect()
}
```

In `crates/kasane-core/src/lib.rs`:

```rust
pub use slug::{anchors_for_headings, path_slug_of};
```

- [ ] **Step 4: Update the property helper**

In `crates/kasane-writer/tests/properties.rs`, replace `heading_slugs` (lines 91-105, doc comment included) with:

```rust
/// Every heading line's anchor, as the engine would compute it for this file.
///
/// A `#`-prefixed line inside a fenced code block would be counted too. That
/// only makes P2 more permissive, never less, so it is not worth a Markdown
/// parser here.
///
/// Order matters now, and did not before: duplicate anchors are suffixed per
/// file in render order, so this feeds the whole ordered list to the engine's
/// own counter rather than slugging each line independently.
///
/// The emphasis markers *are* stripped first, and that one is not optional.
/// The engine anchors a heading at `anchor_slug(inlines)`, which reduces
/// through `inline_text` and therefore never sees a marker; the rendered line
/// comes from `inlines_to_md`, which writes `*`/`**` around `Emph`/`Strong`
/// and backticks around `Code`. A demoted heading rendered as
/// `## Chapter*One*` would otherwise anchor to `chapter-one` here against the
/// engine's `chapterone` -- a false failure of P2, not a real one.
///
/// `_` is deliberately NOT stripped, and used to be. It was in the set
/// defensively, against a writer that might switch emphasis markers -- but `_`
/// is Connector_Punctuation, inside `\p{Word}`, so the engine now keeps a
/// literal underscore. Stripping it here would compute `foobar` against the
/// engine's `foo_bar`.
fn heading_anchors(text: &str) -> HashSet<String> {
    let titles: Vec<String> = text
        .lines()
        .map(|l| l.trim_start())
        .filter_map(|l| l.strip_prefix('#'))
        .map(|l| l.trim_start_matches('#').trim())
        .map(|t| t.replace(['*', '`'], ""))
        .collect();
    anchors_for_headings(&titles).into_iter().collect()
}
```

Replace the import at line 17 — `slug_of` no longer exists:

```rust
use kasane_core::{anchors_for_headings, est_tokens, structure, FileNode};
```

Update the two call sites: line 164 `heading_slugs(body.unwrap()).contains(anchor)` → `heading_anchors(body.unwrap()).contains(anchor)`, and line 366 `heading_slugs(parent).contains(anchor)` → `heading_anchors(parent).contains(anchor)`.

- [ ] **Step 5: Run the property suite to verify it still passes**

Run: `cargo test -p kasane-writer --test properties`
Expected: PASS — six properties.

- [ ] **Step 6: Widen the generator**

In `crates/kasane-writer/tests/generator/mod.rs`, replace `WORDS` (lines 49-53):

```rust
/// Words the generator draws filler text from. Deliberately free of the `zq`
/// sentinel prefix, so generated content can never collide with a token.
///
/// The last five exist to exercise the slug rules rather than to be realistic
/// prose: `&` produces the double-hyphen anchor GFM parity requires, `don't`
/// produces the removed-not-replaced apostrophe, `foo_bar` guards the
/// underscore that `heading_anchors` used to strip, and the CJK and Devanagari
/// words put non-Latin text into both filenames and anchors. Bracket and
/// parenthesis characters are deliberately absent: `links_in` would collect a
/// false link and P2 would fail spuriously.
const WORDS: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "epsilon", "the", "and", "of", "a", "chapter", "section",
    "&", "don't", "foo_bar", "第二章", "हिन्दी",
];
```

- [ ] **Step 7: Run the property suite against the widened generator**

Run: `cargo test -p kasane-writer --test properties`
Expected: PASS. If P2 fails, read the shrunk case before changing anything: the likely causes are a rendered heading line the helper's `#` scan misses, or a title whose rendered form differs from `inline_text`. Do **not** narrow the word list to make it pass — that is the bug the widening exists to find. A failure writes `crates/kasane-writer/tests/properties.proptest-regressions`; if one is written and the underlying issue is then fixed, **commit that file** — it is what replays the case from then on.

- [ ] **Step 8: Run the full suite and lint**

Run: `mise run test && mise run lint`
Expected: PASS, clean.

- [ ] **Step 9: Commit**

```bash
git add crates/kasane-core/src/slug.rs crates/kasane-core/src/lib.rs \
        crates/kasane-writer/tests/properties.rs \
        crates/kasane-writer/tests/generator/mod.rs \
        crates/kasane-writer/src/markdown.rs
git commit -m "test(writer): assert GFM anchors over non-Latin generated titles"
```

---

## Task 4: The `slug` fuzz target

This item makes `kasane-core` write filenames derived from untrusted adapter text. The safety argument is by construction (§5.3 of the spec) — repo habit is to pin such an argument with something executable.

It has to be a new target. `fuzz_entry` lives in `kasane-adapters`, which depends on `kasane-ir` alone and cannot reach `kasane-core`. The stable replay stays in one harness: `crates/kasane-adapters/tests/fuzz_corpus.rs` **panics on a corpus directory it does not recognize**, so `fuzz/seeds/slug/` must be registered there regardless. That file is a test target, so `kasane-core` goes in `kasane-adapters`'s `[dev-dependencies]` — acyclic and confined to tests.

**Files:**
- Create: `crates/kasane-core/src/fuzz_entry.rs`
- Create: `fuzz/fuzz_targets/slug.rs`
- Create: `fuzz/seeds/slug/{traversal.txt,marks.txt,long-cjk.txt,empty.txt}`
- Modify: `crates/kasane-core/src/lib.rs` (`pub mod fuzz_entry;`)
- Modify: `fuzz/Cargo.toml`
- Modify: `crates/kasane-adapters/Cargo.toml`
- Modify: `crates/kasane-adapters/tests/fuzz_corpus.rs:22-39`

**Interfaces:**
- Consumes: `path_slug`, `anchor_slug`, `MAX_PATH_SLUG_BYTES` from Task 1.
- Produces: `pub fn slug(data: &[u8])` in `kasane_core::fuzz_entry`, signature `fn(&[u8])` so the shared replay harness can dispatch to it.

- [ ] **Step 1: Write the fuzz seam**

Create `crates/kasane-core/src/fuzz_entry.rs`:

```rust
//! Fuzz seams for `kasane-core`.
//!
//! A test seam, not API — the same convention and the same rationale as
//! `kasane-adapters`'s module of this name: it lives inside the crate so it
//! can reach `pub(crate)` internals (`slug::path_slug`, `slug::anchor_slug`)
//! that the separate `fuzz/` workspace cannot.
//!
//! Each function takes `&[u8]` and either returns or panics. A panic **is**
//! the finding. That uniformity is what lets
//! `kasane-adapters/tests/fuzz_corpus.rs` dispatch by directory name and keeps
//! every libFuzzer wrapper identical.

use crate::slug::{anchor_slug, path_slug, MAX_PATH_SLUG_BYTES};
use kasane_ir::Inline;

/// `path_slug`'s postconditions, which are security-critical because this is
/// where untrusted adapter text becomes a filename.
///
/// The confinement argument is by construction -- `/`, `\`, `.`, NUL, the
/// fullwidth solidus and the RTL override are all outside `\p{Word}` and are
/// removed -- so this target exists to make that argument fail loudly if the
/// character class is ever widened by hand.
pub fn slug(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let inlines = [Inline::Text(text.to_string())];

    let path = path_slug(&inlines);
    assert!(
        !path.contains('/') && !path.contains('\\'),
        "path_slug emitted a separator: {path:?} from {text:?}"
    );
    assert!(
        !path.split('-').any(|s| s == ".." || s == "."),
        "path_slug emitted a traversal component: {path:?} from {text:?}"
    );
    assert!(
        !path.contains('.'),
        "path_slug emitted a dot: {path:?} from {text:?}"
    );
    assert!(
        !path.is_empty(),
        "path_slug emitted an empty name from {text:?}"
    );
    assert!(
        path.len() <= MAX_PATH_SLUG_BYTES,
        "path_slug exceeded the byte cap: {} bytes from {text:?}",
        path.len()
    );

    // Anchors are uncapped by design, but an empty one is a dead link.
    let anchor = anchor_slug(&inlines);
    assert!(
        !anchor.is_empty(),
        "anchor_slug emitted an empty anchor from {text:?}"
    );
}
```

Add to `crates/kasane-core/src/lib.rs`, above the private module list:

```rust
pub mod fuzz_entry;
```

- [ ] **Step 2: Verify the seam compiles and the assertions hold on obvious input**

Run: `cargo build -p kasane-core`
Expected: builds clean.

- [ ] **Step 3: Register the target in the shared replay harness**

In `crates/kasane-adapters/Cargo.toml`, add (or extend) a dev-dependencies section:

```toml
[dev-dependencies]
# For `tests/fuzz_corpus.rs` only: the `slug` target's seam lives in
# kasane-core, and this file is the one replay harness for every target.
# Test-scoped, so it does not put core in the adapter crate's real dependency
# graph, and acyclic -- core depends on kasane-ir alone.
kasane-core.workspace = true
```

If a `[dev-dependencies]` section already exists, add the entry to it rather than creating a second one.

In `crates/kasane-adapters/tests/fuzz_corpus.rs`, add the dispatch arm and bump the count:

```rust
        "guards" => fuzz_entry::guards,
        "xmltext" => fuzz_entry::xmltext,
        "slug" => kasane_core::fuzz_entry::slug,
        _ => return None,
    })
}

const TARGET_COUNT: usize = 13;
```

- [ ] **Step 4: Write the seeds**

Create four files under `fuzz/seeds/slug/`, each a hand-written starting input:

```bash
mkdir -p fuzz/seeds/slug
printf '../../etc/passwd' > fuzz/seeds/slug/traversal.txt
printf 'हिन्दी Ch\xc3\xa1pter & Notes' > fuzz/seeds/slug/marks.txt
printf '第二章第二章第二章第二章第二章第二章第二章第二章第二章第二章' > fuzz/seeds/slug/long-cjk.txt
printf '***' > fuzz/seeds/slug/empty.txt
```

- [ ] **Step 5: Run the stable replay to verify the seeds pass**

Run: `cargo test -p kasane-adapters --test fuzz_corpus`
Expected: PASS. If it panics with "has no matching fuzz target", the dispatch arm in Step 3 is missing or misspelled.

- [ ] **Step 6: Add the libFuzzer wrapper**

Create `fuzz/fuzz_targets/slug.rs`:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    kasane_core::fuzz_entry::slug(data);
});
```

In `fuzz/Cargo.toml`, add the dependency next to the existing one:

```toml
[dependencies]
libfuzzer-sys = "0.4"
kasane-adapters = { path = "../crates/kasane-adapters" }
kasane-core = { path = "../crates/kasane-core" }
```

...and a `[[bin]]` entry alongside the others:

```toml
[[bin]]
name = "slug"
path = "fuzz_targets/slug.rs"
test = false
doc = false
bench = false
```

- [ ] **Step 7: Run the fuzzer on the pinned nightly**

Run: `mise run fuzz slug -- -max_total_time=120`
Expected: no crash, and the seeds are picked up. This needs the nightly toolchain `mise` pins; it is the only step in this plan that does.

If the fuzzer finds a crash, that is a real finding: commit the reproducer from `fuzz/artifacts/slug/`, fix the rule, and keep the reproducer — it becomes the permanent regression test on stable. Do **not** add it to `KNOWN_OPEN` unless the fix is genuinely being deferred, which for a bug this branch introduces it should not be.

- [ ] **Step 8: Run the full suite and lint**

Run: `mise run test && mise run lint`
Expected: PASS, clean.

- [ ] **Step 9: Commit**

```bash
git add crates/kasane-core/src/fuzz_entry.rs crates/kasane-core/src/lib.rs \
        crates/kasane-adapters/Cargo.toml crates/kasane-adapters/tests/fuzz_corpus.rs \
        fuzz/Cargo.toml fuzz/fuzz_targets/slug.rs fuzz/seeds/slug Cargo.lock
git commit -m "test(fuzz): assert path_slug's confinement postconditions"
```

---

## Task 5: Documentation

Four README locations and three AGENTS.md ones. The churn paragraph matters most: this changes output paths for every book with punctuation in a heading, and changes non-Latin books wholesale.

**Files:**
- Modify: `README.md` (Output shape ~line 40; Property tests ~line 70; Fuzzing line 85; Known limitations ~line 129)
- Modify: `AGENTS.md` (the `kasane-core` entry, the `fuzz_entry.rs` note, the block-walk inventory)

**Interfaces:**
- Consumes: everything from Tasks 1-4.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Rewrite the Known limitations bullet**

In `README.md`, replace the first bullet under "Known limitations (this build)" (the one beginning "Heading anchors use kasane's own slug rule") with:

```markdown
- Heading anchors match GitHub's rule, with one exception. Anchors are
  computed the way GitHub computes them — Unicode-aware, punctuation removed
  rather than replaced, `_` kept, duplicates within a file suffixed `-1`,
  `-2` — so `## Don't Panic` anchors as `#dont-panic` and `## 第二章` as
  `#第二章`, both of which resolve on GitHub. Note that exact parity means
  some anchors look wrong and are not: `## Background & Notes` anchors as
  `#background--notes`, because GFM removes the `&` and turns each surviving
  space into a hyphen. The exception is a heading with no letter, digit or
  mark at all (`## ***`): GitHub gives it an empty id, and kasane emits
  `#section` instead, because an empty anchor is a dead link.
  Filenames carry the title in any script, capped at 64 bytes per component.
  What they do not carry is total path length: depth comes from heading
  nesting plus whatever `-o` you pass, so a deep book in a deep output
  directory can still exceed Windows' 260-character default path limit.
```

- [ ] **Step 2: Add the churn paragraph to Output shape**

In `README.md`, append to the "Output shape" section, after the existing `Part N` paragraph:

```markdown
Paths also changed with the slug rule. Punctuation is now removed rather than
turned into a separator, so `01-don-t-panic.md` became `01-dont-panic.md`, and
a heading in any script now keeps its text, so a Japanese book that used to
emit `01-section.md` and `02-section.md` now emits `01-第二章.md` and its
siblings. As with the `Part N` change, `write_tree` replaces the output
directory wholesale so nothing stale is left behind — but anything outside the
tree that referenced the old paths needs updating.
```

- [ ] **Step 3: Correct the Property tests paragraph**

In `README.md`, replace the paragraph beginning "Read the link invariant precisely":

```markdown
Read the link invariant precisely: it holds *against kasane's own slug rule*,
which is now a deliberate mirror of GitHub's — the generator draws non-Latin,
accented and punctuation-bearing titles, so the invariant exercises that rule
rather than an ASCII subset of it. What it still cannot say is whether
github.com computes the same anchor: nothing in CI can ask it. The mirror is
written down as a case table in `crates/kasane-core/src/slug.rs`. Deep
list/footnote nesting remains under Known limitations.
```

- [ ] **Step 4: Correct the fuzz target count**

In `README.md` line 85, change "Twelve targets cover the five format adapters, format detection," to:

```markdown
`cargo-fuzz`. Thirteen targets cover the five format adapters, format detection,
```

...and extend the sentence listing the sub-parsers so it ends "the path guards, XML entity resolution, and the slug rule that turns untrusted title text into a filename."

- [ ] **Step 5: Update AGENTS.md**

Three edits in `AGENTS.md`:

1. In the `crates/kasane-core` entry, add a paragraph after the `est_tokens`/`slug_of` sentence:

```markdown
  `slug.rs` owns two rules, not one. `anchor_slug` is a deliberate mirror of
  GitHub's heading-id algorithm (NFC, Unicode-lowercase, remove everything
  outside Ruby's `\p{Word}`, map spaces to hyphens, no collapsing and no
  interior trimming) so in-book cross-references resolve when the tree is
  rendered on GitHub; `path_slug` shares that character class but collapses
  separator runs, trims, and caps at 64 bytes, because a filename wants
  different things than a fragment. Being a mirror, the anchor rule carries
  drift risk against github.com, and the case table in `slug.rs`'s tests is
  where that mirror is written down. Duplicate anchors are suffixed per file
  in render order, which is why `place` counts headings nested inside list
  items even though it deliberately gives them no anchor: GitHub assigns them
  ids, so they consume a suffix slot. `assign_paths` takes the document title
  because `index.md` renders *that* as its heading, not the (empty) root node
  title. `path_slug_of` and `anchors_for_headings` are `#[doc(hidden)] pub`
  test seams, same convention as `est_tokens`.
```

2. In the `fuzz_entry.rs` note under `crates/kasane-adapters`, add: "`kasane-core` has its own `fuzz_entry.rs` for the same reason, reaching `slug::path_slug`; the stable replay for both lives in `kasane-adapters/tests/fuzz_corpus.rs`, which takes `kasane-core` as a dev-dependency so one harness covers every target."

3. In the block-nesting Conventions bullet, the walk inventory count increases by one for `paths::count_headings`. Read the surrounding prose first — it names how many walks run where — and move both the count and the sentence that explains it.

- [ ] **Step 6: Verify the documented claims against the code**

Run: `mise run test && mise run lint`
Expected: PASS, clean. Then spot-check the two most falsifiable claims by hand:

```bash
ls fuzz/fuzz_targets/ | wc -l                            # must print 13
grep -n "TARGET_COUNT" crates/kasane-adapters/tests/fuzz_corpus.rs   # must be 13
grep -n "Thirteen targets" README.md                     # must match the two above
```

- [ ] **Step 7: Convert a real non-Latin book and inspect the output**

This is the verification the case table cannot do — it tests the derivation from GitHub's algorithm, not this plan's reading of it.

```bash
mise run convert <a real EPUB with non-Latin headings> -o /tmp/slug-check
find /tmp/slug-check -name '*.md' | head -20
grep -rn '](' /tmp/slug-check/index.md | head
```

Confirm filenames carry the titles, then paste one converted file into a GitHub gist or PR preview and check that an in-book cross-reference actually jumps to its heading. If GitHub disagrees with the case table, that is a real finding and the table is what changes.

- [ ] **Step 8: Commit**

```bash
git add README.md AGENTS.md
git commit -m "docs: record the two slug rules and the path churn they cause"
```

---

## Self-Review

**Spec coverage:**

| Spec section | Task |
|---|---|
| §2.1 `anchor_slug` | 1 |
| §2.2 `path_slug` | 1 |
| §2.3 dependencies | 1 (Step 1) |
| §3.1 duplicate suffixing, nested-heading counting, bounded walk | 2 |
| §3.2 byte cap, char-boundary truncation, tail trim | 1 (Steps 2, 5) |
| §3.3 `section` fallback and the divergence | 1 (test rows), 5 (README) |
| §4 raw destinations, closed character set, pinned by a test | 3 (Step 1) |
| §5.1 curated case table | 1 (Step 2) |
| §5.2 ordered seam, generator widening, `_` strip inversion | 3 |
| §5.3 fuzz target, core `fuzz_entry`, dev-dependency, `TARGET_COUNT` | 4 |
| §5.4 end-to-end assertions | 1 (Step 9), 3 (Step 8) — verified green rather than edited; `Chapter One` and `Deep` slug identically under both rules, so `e2e.rs` needs no change. If any assertion does move, fold it into that task's commit. |
| §6 documentation | 5 |
| §8 verification | 4 (Step 7 nightly fuzz), 5 (Step 7 hand check) |

**Placeholder scan:** none. Every code step carries the code; the one judgement call left open (AGENTS.md's walk-inventory count in Task 5 Step 5) is explicitly "read the surrounding prose first" because the number depends on prose this plan should not guess at.

**Type consistency:** `anchor_slug`/`path_slug`/`inline_text`/`AnchorCounter::next` all take `&[Inline]`. `assign_paths` gains `doc_title: &str` in Task 2 and every caller (`nav.rs:37`, five tests in `paths.rs`) is updated in the same task. `slug_of` is introduced as a forwarding seam in Task 1 Step 8 solely so `properties.rs` compiles between tasks, and removed in Task 3 Step 3 — no task references it afterwards. `path_slug_of` is introduced in Task 1 and first consumed in Task 3; `fuzz_entry::slug` in Task 4 uses the `pub(crate)` functions directly, not the seams.
