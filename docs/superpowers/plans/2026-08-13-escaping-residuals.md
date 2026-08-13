# Escaping Residuals Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the three unfixed shapes recorded in §4.5 of the Markdown escaping spec — a footnote reference followed by a colon at column 0, leading whitespace at a line start, and a newline run split across an inline boundary — plus the table-cell whitespace defect in the same class.

**Architecture:** Three rules, all owned by `crates/kasane-writer/src/escape.rs`. `escape::text`'s `at_line_start: bool` widens to a three-state `Pos` so the renderer keeps reporting position while `escape.rs` keeps deciding what to escape. Whitespace at a line start becomes a numeric character reference rather than a backslash, because the backslash form deletes the whitespace it was meant to protect. A pre-render pass over `&[Inline]` collapses newline runs across inline boundaries so the rendered heading line matches what `kasane-core`'s `anchor_fold` predicts, leaving that hand-kept mirror untouched.

**Tech Stack:** Rust 2021, Cargo workspace, `proptest`, `pulldown-cmark` 0.13 (dev-dependency, test-only), `mise` task runner.

**Spec:** `docs/superpowers/specs/2026-08-13-escaping-residuals-design.md`. Section references below (§2, §3.2, §4.1, …) point into it.

## Global Constraints

- **Branch:** `escaping-residuals`, already created and holding the spec commit.
- **Every rule lives in `escape.rs`.** §2 of the escaping spec (2026-08-09) states `escape.rs` owns every escaping rule. `markdown.rs` may compute *where* text lands; it must never decide *what* to escape.
- **Escaping must never change what the Markdown renders to.** This is §5 of the escaping spec and the reason anchors still resolve. A fix that suppresses a construct by deleting characters is not a fix.
- **Assert against `pulldown-cmark`, not against a reading of CommonMark.** Every mechanism here was chosen because a real parser confirmed it, and §1 of the spec records three readings that were wrong before it did.
- **Per-task gate:** `mise run test` and `mise run lint` must both pass before each commit. `mise run lint` is `cargo fmt --all -- --check` plus `cargo clippy --workspace --all-targets -- -D warnings` — the `--all-targets` matters, since plain `cargo clippy` skips test code.
- **Never leave a `KNOWN_OPEN` entry behind.** `crates/kasane-adapters/tests/fuzz_corpus.rs`'s `KNOWN_OPEN` is `&[]` today and must still be `&[]` at the end.

---

### Task 1: `Pos`, the escaping position

Mechanical widening with **no behaviour change**. Rules A and C need positional state that a `bool` cannot carry; this task creates the vocabulary and migrates every call site, so the rule tasks that follow are small and reviewable on their own.

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs` (add `Pos`; `text` signature; `label`; test module)
- Modify: `crates/kasane-writer/src/markdown.rs` (`inlines_to_md`, `inlines_to_md_at`, every call; test module)
- Modify: `crates/kasane-writer/src/lib.rs:39-43` (`file_to_markdown`)
- Modify: `crates/kasane-writer/src/fuzz_entry.rs` (the `escape` target's loop)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub(crate) enum Pos { LineStart, AfterFootnoteRef, Mid }` in `escape.rs`; `pub(crate) fn text(s: &str, ctx: Ctx, pos: Pos) -> String`; `pub(crate) fn inlines_to_md(inls: &[Inline], ctx: Ctx, pos: Pos) -> String` in `markdown.rs`.

- [ ] **Step 1: Add the `Pos` enum**

In `crates/kasane-writer/src/escape.rs`, directly below the `Ctx` enum:

```rust
/// Where the next character emitted lands. The rules that depend on position
/// need three states, not two: `escape::text` has to distinguish "at column 0"
/// from "directly after a footnote reference that opened the line", because
/// only the latter makes a following `:` a footnote *definition* delimiter
/// (residuals spec §2).
///
/// `markdown.rs` computes this and passes it in; it never decides what to
/// escape. That division is escaping spec §2 — `escape.rs` owns every rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Pos {
    /// The next character lands at the start of a line.
    LineStart,
    /// The next character lands directly after a `[^n]` that itself opened
    /// the line.
    AfterFootnoteRef,
    /// Anywhere else.
    Mid,
}
```

- [ ] **Step 2: Change `text`'s signature**

In `escape.rs`, change the signature and the one line that reads the flag. Nothing else in the function body changes:

```rust
pub(crate) fn text(s: &str, ctx: Ctx, pos: Pos) -> String {
    if ctx == Ctx::Html {
        return html_text(s);
    }
    let chars: Vec<char> = normalize_newlines(s).chars().collect();
    let mut out = String::with_capacity(chars.len() + 8);
    let mut line_start = pos == Pos::LineStart;
    let mut i = 0;
```

`Pos::AfterFootnoteRef` therefore behaves exactly like `Pos::Mid` until Task 2. That is intentional: this task must not change any output.

- [ ] **Step 3: Update every call site**

`escape.rs` — `label`:

```rust
pub(crate) fn label(s: &str) -> String {
    one_line(&text(s, Ctx::Flow, Pos::Mid))
}
```

`markdown.rs` — change both signatures:

```rust
pub(crate) fn inlines_to_md(inls: &[Inline], ctx: Ctx, pos: Pos) -> String {
    inlines_to_md_at(inls, 0, ctx, pos)
}

fn inlines_to_md_at(inls: &[Inline], depth: usize, ctx: Ctx, pos: Pos) -> String {
```

and inside `inlines_to_md_at` rename the local, leaving the recomputation rule as it is for now:

```rust
    let mut pos = pos;
    for i in inls {
        match i {
            Inline::Text(t) => s.push_str(&escape::text(t, ctx, pos)),
            // ...every other arm passes `pos` where it passed `line_start`...
        }
        pos = if s.ends_with('\n') { Pos::LineStart } else { Pos::Mid };
    }
```

Also update the import at the top of `markdown.rs`:

```rust
use crate::escape::{self, Ctx, Pos};
```

Then translate the literal arguments — `true` becomes `Pos::LineStart`, `false` becomes `Pos::Mid` — at each of these:

| File | Site | New argument |
|---|---|---|
| `markdown.rs` | `Block::Heading` | `Pos::Mid` |
| `markdown.rs` | `Block::Para` | `Pos::LineStart` |
| `markdown.rs` | `Block::Figure` alt text | `Pos::Mid` |
| `markdown.rs` | `Block::Figure` `number` | `Pos::Mid` |
| `markdown.rs` | `render_table`'s `cells` closure | `Pos::LineStart` |
| `markdown.rs` | all four `Ctx::Html` calls in `inlines_to_html` | `Pos::Mid` |
| `lib.rs` | `file_to_markdown`'s title heading | `Pos::LineStart` |
| `fuzz_entry.rs` | the `Ctx::Html` call | `Pos::Mid` |

- [ ] **Step 4: Widen the fuzz target's loop to three states**

In `crates/kasane-writer/src/fuzz_entry.rs`, the `escape` function currently loops `for at_line_start in [true, false]`. Replace it:

```rust
    for ctx in [Ctx::Flow, Ctx::Cell] {
        for pos in [Pos::LineStart, Pos::AfterFootnoteRef, Pos::Mid] {
            let out = escape::text(text, ctx, pos);
```

and import `Pos`:

```rust
use crate::escape::{self, Ctx, Pos};
```

No assertion in that file needs relaxing, now or after Tasks 2–4.
`assert_unescaped_specials_are_absent` checks `` ` ``, `*`, `_`, `[`, `]`, `~`
and `$`; `&` is deliberately not on that list, so the `&#32;` and `&#9;`
references Task 3 introduces pass it unchanged. `#` is unchecked too, and
correctly — it is a `LINE_START` character, not an inline opener.

- [ ] **Step 5: Update the existing tests**

In the `#[cfg(test)]` modules of `escape.rs` and `markdown.rs`, translate every `text(...)`/`inlines_to_md(...)` argument the same way: `true` → `Pos::LineStart`, `false` → `Pos::Mid`. Assertions and expected values do not change — if any expected string needs editing, the migration was not mechanical and something is wrong.

- [ ] **Step 6: Run the full suite**

Run: `mise run test`
Expected: PASS, with no expected-value edits in step 5. Any failure here means the translation changed behaviour.

- [ ] **Step 7: Lint**

Run: `mise run lint`
Expected: PASS. If clippy flags `Pos` as never constructed for `AfterFootnoteRef`, that is expected only until Task 2 — add nothing to silence it; the variant is constructed in step 4's fuzz loop, so it should not fire.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-writer/src/
git commit -m "refactor(writer): widen the escaping position from a bool to Pos

Rules A and C in the residuals spec need positional state a bool cannot
carry. No behaviour change: Pos::AfterFootnoteRef behaves as Pos::Mid until
the next commit, and no expected value in the test suite moved."
```

---

### Task 2: Rule A — the footnote-reference colon

`Para([FootnoteRef(1), Text(": note")])` renders `[^1]: note` at column 0, which GFM parses as a footnote *definition*. The paragraph is lost.

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs` (the rule, plus tests)
- Modify: `crates/kasane-writer/src/markdown.rs` (`inlines_to_md_at`'s position recomputation, plus tests)

**Interfaces:**
- Consumes: `Pos` and the migrated signatures from Task 1.
- Produces: no new symbols. `escape::text` gains behaviour at `Pos::AfterFootnoteRef` in `Ctx::Flow`.

- [ ] **Step 1: Write the failing escape.rs tests**

Add to `escape.rs`'s test module:

```rust
    /// A footnote reference that opened the line makes a following `:` the
    /// delimiter of a footnote *definition*, which swallows the paragraph.
    /// The `:` belongs to the next inline, which is why the position has to
    /// carry the fact across the boundary (§2).
    #[test]
    fn flow_escapes_a_colon_directly_after_a_footnote_reference() {
        assert_eq!(text(": note", Ctx::Flow, Pos::AfterFootnoteRef), "\\: note");
        // Only the *leading* colon: nothing later on the line can be a
        // definition delimiter.
        assert_eq!(text("x: y", Ctx::Flow, Pos::AfterFootnoteRef), "x: y");
        // Not at that position, not escaped.
        assert_eq!(text(": note", Ctx::Flow, Pos::LineStart), ": note");
        assert_eq!(text(": note", Ctx::Flow, Pos::Mid), ": note");
    }

    /// A cell is inline context, where `[^1]:` is never a definition, so the
    /// backslash would render as nothing and exist for no reason (§2).
    #[test]
    fn a_cell_does_not_escape_the_footnote_colon() {
        assert_eq!(text(": note", Ctx::Cell, Pos::AfterFootnoteRef), ": note");
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p kasane-writer --lib escape::tests::flow_escapes_a_colon_directly_after_a_footnote_reference`
Expected: FAIL — `assertion \`left == right\` failed: left: ": note", right: "\\: note"`.

- [ ] **Step 3: Implement the rule**

In `escape.rs`'s `text`, between the `line_start` initialization and the `while` loop:

```rust
    let mut line_start = pos == Pos::LineStart;
    let mut i = 0;

    // A `[^n]` that opened the line makes a leading `:` the delimiter of a
    // footnote *definition*, which swallows the paragraph (§2). `\:` is a
    // valid CommonMark escape — `:` is ASCII punctuation — and it leaves the
    // reference itself intact.
    //
    // Gated to `Ctx::Flow`: `render_table` renders every cell at a line start
    // so this position arises in a cell too, but a cell is inline context
    // where `[^1]:` is never a definition.
    if ctx == Ctx::Flow && pos == Pos::AfterFootnoteRef && chars.first() == Some(&':') {
        out.push('\\');
        out.push(':');
        i = 1;
    }

    while i < chars.len() {
```

- [ ] **Step 4: Run them to verify they pass**

Run: `cargo test -p kasane-writer --lib escape::tests`
Expected: PASS, all tests in the module.

- [ ] **Step 5: Write the failing markdown.rs threading tests**

The rule cannot fire until `inlines_to_md_at` produces `Pos::AfterFootnoteRef`. Add to `markdown.rs`'s test module:

```rust
    /// The end-to-end shape §1's table calls A. A matching definition has to
    /// be present or *no* footnote reference parses at all — without one,
    /// `[^1]` decomposes into bare text and the test measures the fixture
    /// rather than the fix.
    #[test]
    fn a_footnote_reference_at_column_zero_does_not_open_a_definition() {
        use pulldown_cmark::{Event, Options, Parser};

        let blocks = vec![
            Block::Para(vec![
                Inline::FootnoteRef(NoteId(1)),
                Inline::Text(": note".into()),
            ]),
            Block::Footnote {
                id: NoteId(1),
                blocks: vec![Block::Para(vec![Inline::Text("the definition".into())])],
            },
        ];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("[^1]\\: note"), "got:\n{md}");

        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_FOOTNOTES);
        let (mut refs, mut defs) = (0, 0);
        for ev in Parser::new_ext(&md, opts) {
            match ev {
                Event::FootnoteReference(_) => refs += 1,
                Event::Start(pulldown_cmark::Tag::FootnoteDefinition(_)) => defs += 1,
                _ => {}
            }
        }
        assert_eq!(refs, 1, "the reference must survive the escape:\n{md}");
        assert_eq!(defs, 1, "only the real Block::Footnote is a definition:\n{md}");
    }

    /// An empty run between the reference and the colon must not reset the
    /// position — the IR permits it and the colon is still a delimiter (§2,
    /// rule 1).
    #[test]
    fn an_empty_run_does_not_reset_the_footnote_position() {
        let blocks = vec![Block::Para(vec![
            Inline::FootnoteRef(NoteId(1)),
            Inline::Text(String::new()),
            Inline::Text(": note".into()),
        ])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("[^1]\\: note"), "got:\n{md}");
    }

    /// A wrapped reference renders `*[^1]*: x`, which begins with `*` and was
    /// never a definition, so the `Emph` arm must yield `Pos::Mid` (§2, rule 3).
    #[test]
    fn a_wrapped_footnote_reference_leaves_the_colon_alone() {
        let blocks = vec![Block::Para(vec![
            Inline::Emph(vec![Inline::FootnoteRef(NoteId(1))]),
            Inline::Text(": x".into()),
        ])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("*[^1]*: x"), "got:\n{md}");
        assert!(!md.contains("\\:"), "no escape is needed here:\n{md}");
    }
```

If `NoteId` is not already imported in that test module, add it to the existing `use kasane_ir::*;`.

- [ ] **Step 6: Run them to verify they fail**

Run: `cargo test -p kasane-writer --lib markdown::tests::a_footnote_reference_at_column_zero_does_not_open_a_definition`
Expected: FAIL — the `md.contains("[^1]\\: note")` assertion, because nothing yet produces `Pos::AfterFootnoteRef`.

- [ ] **Step 7: Implement the position recomputation**

Replace the single recomputation line at the end of `inlines_to_md_at`'s loop body:

```rust
    let mut pos = pos;
    for i in inls {
        let before = pos;
        let len_before = s.len();
        match i {
            // ...arms unchanged...
        }
        // Four rules (§2). An arm that appended nothing leaves the position
        // alone, so an empty text run between a reference and its colon does
        // not reset it. `Inline::FootnoteRef` always appends, so rule 3 is
        // never blocked by the length check.
        if s.len() != len_before {
            pos = if s.ends_with('\n') {
                Pos::LineStart
            } else if matches!(i, Inline::FootnoteRef(_)) && before == Pos::LineStart {
                Pos::AfterFootnoteRef
            } else {
                Pos::Mid
            };
        }
    }
```

- [ ] **Step 8: Run the full suite**

Run: `mise run test`
Expected: PASS. Watch `properties.rs` in particular — P7's round-trip counts payload occurrences, and a stray backslash inside a payload would fail it.

- [ ] **Step 9: Lint and commit**

```bash
mise run lint
git add crates/kasane-writer/src/
git commit -m "fix(writer): escape the colon after a footnote ref at column 0

Para([FootnoteRef(1), Text(\": note\")]) rendered [^1]: note at column 0,
which GFM parses as a footnote definition — the paragraph was lost. The colon
belongs to the next inline, so the position carries the fact across the
boundary and escape.rs still owns the rule.

Gated to Ctx::Flow: render_table renders cells at a line start, so the
position arises there too, but a cell is inline context where [^1]: is never
a definition."
```

---

### Task 3: Rule B — whitespace at a line start

`escape::text` clears `line_start` on any leading space, so `Text("  # h")` at column 0 emits an unescaped ATX heading, and four or more spaces open an indented code block that no marker escaping can reach.

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs` (the rule, plus tests)
- Modify: `crates/kasane-writer/src/markdown.rs` (test only — the `emphasize` interaction, §3.5)

**Interfaces:**
- Consumes: `Pos` from Task 1.
- Produces: no new symbols. `escape::text` gains behaviour at `Pos::LineStart` for `' '` and `'\t'` in `Ctx::Flow` and `Ctx::Cell`.

- [ ] **Step 1: Write the failing tests**

Add to `escape.rs`'s test module:

```rust
    /// A character reference, not a backslash. `   \# h` does suppress the
    /// heading, but the parser then strips the three leading spaces — the
    /// text is gone, which is an escaping-spec §5 violation presenting as a
    /// fix. A reference renders as the character and is not whitespace to the
    /// block scanner, so one at the head of the run disarms all of it (§3).
    #[test]
    fn a_line_start_whitespace_run_becomes_a_character_reference() {
        // The first character carries it; the rest of the run is literal,
        // and the `#` needs no backslash because it is no longer at column 0.
        assert_eq!(text("  # h", Ctx::Flow, Pos::LineStart), "&#32; # h");
        // Four spaces would open an indented code block, which has no marker
        // to escape.
        assert_eq!(text("    x", Ctx::Flow, Pos::LineStart), "&#32;   x");
        // A tab indents just as far.
        assert_eq!(text("\tx", Ctx::Flow, Pos::LineStart), "&#9;x");
        // Mid-line whitespace is ordinary text.
        assert_eq!(text("  x", Ctx::Flow, Pos::Mid), "  x");
    }

    #[test]
    fn line_start_whitespace_re_arms_after_an_interior_newline() {
        assert_eq!(
            text("intro\n  # not a heading", Ctx::Flow, Pos::Mid),
            "intro\n&#32; # not a heading"
        );
    }

    /// GFM trims a cell before parsing it, so a leading run is dropped
    /// outright — document text lost. `render_table` renders every cell at a
    /// line start, so this rule reaches the leading edge with no new
    /// plumbing (§3.2).
    #[test]
    fn a_cell_keeps_its_leading_whitespace() {
        assert_eq!(text("  x", Ctx::Cell, Pos::LineStart), "&#32; x");
        assert_eq!(text("\tx", Ctx::Cell, Pos::LineStart), "&#9;x");
    }

    /// The reference has to survive as one, which means the `&` must not pick
    /// up a backslash from the entity rule on the way out.
    #[test]
    fn the_emitted_reference_parses_back_to_the_whitespace() {
        use pulldown_cmark::{Event, Options, Parser};

        let md = format!("{}\n", text("    x", Ctx::Flow, Pos::LineStart));
        let mut got = String::new();
        let mut is_code_block = false;
        for ev in Parser::new_ext(&md, Options::empty()) {
            match ev {
                Event::Text(t) => got.push_str(&t),
                Event::Start(pulldown_cmark::Tag::CodeBlock(_)) => is_code_block = true,
                _ => {}
            }
        }
        assert!(!is_code_block, "four spaces still opened a code block: {md:?}");
        assert_eq!(got, "    x", "the whitespace must render back: {md:?}");
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p kasane-writer --lib escape::tests::a_line_start_whitespace_run_becomes_a_character_reference`
Expected: FAIL — `left: "  # h", right: "&#32; # h"`.

- [ ] **Step 3: Implement the rule**

In `escape.rs`'s `text`, as the **first** branch inside the existing `if line_start { … }` block, above the ordered-marker check:

```rust
        if line_start {
            // GFM reinterprets or discards whitespace at a line start: up to
            // three spaces still open a heading or a list, four open an
            // indented code block, and a cell's leading run is trimmed away
            // entirely. None of that is reachable by escaping a marker — the
            // code-block form has no marker, and `   \# h` suppresses the
            // heading only by losing the spaces.
            //
            // A character reference renders as the character it names but is
            // not whitespace to the block scanner, so one at the head of the
            // run disarms the whole run: everything after it is no longer at
            // column 0 (§3).
            if c == ' ' || c == '\t' {
                out.push_str(if c == ' ' { "&#32;" } else { "&#9;" });
                i += 1;
                line_start = false;
                continue;
            }
            if let Some(after_digits) = ordered_marker_delimiter(&chars, i) {
```

- [ ] **Step 4: Run them to verify they pass**

Run: `cargo test -p kasane-writer --lib escape::tests`
Expected: PASS.

- [ ] **Step 5: Pin the `emphasize` interaction**

§3.5. This looks like double-handling of the same whitespace and is not — the
inner render replaces the space before `emphasize` ever sees it, so its `trim`
finds nothing to extract. Add to `markdown.rs`'s test module:

```rust
    /// §3.5. At column 0 this rendered `* x*`, which GFM reads as a bullet
    /// list — the paragraph was lost, and the emphasis was silently dropped
    /// too, because a `*` with adjacent whitespace is never a delimiter.
    /// `emphasize` moves edge whitespace outside the delimiters, but at a line
    /// start the reference has already replaced it, so `*&` is left-flanking
    /// and the space stays *inside* the emphasis where the IR put it.
    #[test]
    fn emphasis_at_column_zero_keeps_its_leading_space_inside() {
        use pulldown_cmark::{Event, Options, Parser};

        let blocks = vec![Block::Para(vec![Inline::Emph(vec![Inline::Text(
            " x".into(),
        )])])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.starts_with("*&#32;x*"), "got:\n{md}");

        let mut in_em = false;
        let mut emphasized = String::new();
        let mut is_list = false;
        for ev in Parser::new_ext(&md, Options::empty()) {
            match ev {
                Event::Start(pulldown_cmark::Tag::Emphasis) => in_em = true,
                Event::End(pulldown_cmark::TagEnd::Emphasis) => in_em = false,
                Event::Start(pulldown_cmark::Tag::List(_)) => is_list = true,
                Event::Text(t) if in_em => emphasized.push_str(&t),
                _ => {}
            }
        }
        assert!(!is_list, "a paragraph became a bullet list:\n{md}");
        assert_eq!(emphasized, " x", "the emphasis must apply and keep its space:\n{md}");
    }
```

Run: `cargo test -p kasane-writer --lib markdown::tests::emphasis_at_column_zero_keeps_its_leading_space_inside`
Expected: PASS — Task 3's rule already produces this. If it fails on the `starts_with`, the rule fired in the wrong order relative to `emphasize`; read the rendered `md` before changing either.

- [ ] **Step 6: Run the full suite**

Run: `mise run test`
Expected: PASS. `properties.rs` P7 whitespace-normalizes before counting payload occurrences, so it neither confirms nor contradicts this rule — but a reference that failed to parse would corrupt a payload and fail it.

- [ ] **Step 7: Lint and commit**

```bash
mise run lint
git add crates/kasane-writer/src/escape.rs
git commit -m "fix(writer): keep whitespace at a line start with a character reference

escape::text cleared line_start on any leading space, so Text(\"  # h\") at
column 0 emitted a real heading, and four spaces opened an indented code
block no marker escaping can reach.

The backslash form the escaping spec §4.5 assumed does not work: `   \\# h`
suppresses the heading but the parser strips the spaces, losing the text.
&#32; and &#9; render as the character, are not whitespace to the block
scanner, and one at the head of a run disarms all of it.

Also fixes a live defect in Ctx::Cell, which reaches this position because
render_table renders every cell at a line start: GFM trims a cell before
parsing it, so `|  x |` rendered `x`."
```

---

### Task 4: The cell's trailing edge

GFM trims both ends of a cell. Task 3 fixed the leading edge for free; the trailing edge is not a positional question and needs its own rule (§3.3).

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs` (`cell_edges`, plus tests)
- Modify: `crates/kasane-writer/src/markdown.rs` (`render_table`'s `cells` closure, plus a test)

**Interfaces:**
- Consumes: nothing from Tasks 2–3.
- Produces: `pub(crate) fn cell_edges(rendered: &str) -> String` in `escape.rs`.

- [ ] **Step 1: Write the failing escape.rs test**

```rust
    /// GFM trims both ends of a cell. `Pos::LineStart` covers the leading
    /// edge (§3.2); the trailing edge is not a positional question, so it is
    /// fixed on the rendered cell. Only the last character needs the
    /// reference — everything before it is no longer at the trimmed edge (§3.3).
    #[test]
    fn cell_edges_restores_trailing_whitespace() {
        assert_eq!(cell_edges("x "), "x&#32;");
        assert_eq!(cell_edges("x\t"), "x&#9;");
        assert_eq!(cell_edges("x  "), "x &#32;");
        // Nothing to restore.
        assert_eq!(cell_edges("x"), "x");
        assert_eq!(cell_edges(""), "");
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p kasane-writer --lib escape::tests::cell_edges_restores_trailing_whitespace`
Expected: FAIL to compile — `cannot find function 'cell_edges' in this scope`.

- [ ] **Step 3: Implement `cell_edges`**

Add to `escape.rs`, next to `label`:

```rust
/// Restore whitespace at a rendered cell's trailing edge (§3.3).
///
/// GFM trims a cell's content before parsing it, so a trailing space or tab
/// is dropped outright — document text lost, the same defect the leading edge
/// had. The leading edge is covered by `Pos::LineStart`, because
/// `render_table` renders every cell at a line start; this edge is not a
/// positional question and cannot be.
///
/// Only the last character needs the reference: everything before it is no
/// longer at the trimmed edge. Symmetric with the leading rule, which likewise
/// only converts the first character of the run.
pub(crate) fn cell_edges(rendered: &str) -> String {
    let mut out = rendered.to_string();
    if out.ends_with(' ') {
        out.truncate(out.len() - 1);
        out.push_str("&#32;");
    } else if out.ends_with('\t') {
        out.truncate(out.len() - 1);
        out.push_str("&#9;");
    }
    out
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p kasane-writer --lib escape::tests::cell_edges_restores_trailing_whitespace`
Expected: PASS.

- [ ] **Step 5: Write the failing wiring test**

Add to `markdown.rs`'s test module:

```rust
    /// Both edges, through the real table renderer and a real parser.
    #[test]
    fn a_table_cell_keeps_the_whitespace_at_both_its_edges() {
        use pulldown_cmark::{Event, Options, Parser};

        let cell = |s: &str| vec![Inline::Text(s.to_string())];
        let blocks = vec![Block::Table(Table {
            header: vec![cell("h")],
            rows: vec![vec![cell("  x  ")]],
            has_merged: false,
        })];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());

        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        let mut in_cell = false;
        let mut body = String::new();
        let mut seen_header = false;
        for ev in Parser::new_ext(&md, opts) {
            match ev {
                Event::Start(pulldown_cmark::Tag::TableCell) => in_cell = true,
                Event::End(pulldown_cmark::TagEnd::TableHead) => seen_header = true,
                Event::Text(t) if in_cell && seen_header => body.push_str(&t),
                _ => {}
            }
        }
        assert_eq!(body, "  x  ", "both edges must survive:\n{md}");
    }
```

If `Table` is not already in scope in that module, the existing `use kasane_ir::*;` covers it.

- [ ] **Step 6: Run it to verify it fails**

Run: `cargo test -p kasane-writer --lib markdown::tests::a_table_cell_keeps_the_whitespace_at_both_its_edges`
Expected: FAIL — `body` is `"  x"`, missing the two trailing spaces (the leading pair already survives, from Task 3).

- [ ] **Step 7: Wire it into `render_table`**

In `markdown.rs`'s `render_table`, wrap the cell render:

```rust
    let cells = |row: &Vec<Vec<Inline>>| {
        let joined: Vec<String> = row
            .iter()
            .map(|c| escape::cell_edges(&inlines_to_md(c, Ctx::Cell, Pos::LineStart)))
            .collect();
        format!("| {} |", joined.join(" | "))
    };
```

The surrounding `| ` / ` |` padding is unaffected: a cell rendering to `x&#9;` becomes `| x&#9; |`, whose content is `x&#9;` once the literal padding spaces are trimmed.

- [ ] **Step 8: Run it to verify it passes**

Run: `cargo test -p kasane-writer --lib markdown::tests::a_table_cell_keeps_the_whitespace_at_both_its_edges`
Expected: PASS.

- [ ] **Step 9: Run the full suite, lint, commit**

```bash
mise run test
mise run lint
git add crates/kasane-writer/src/
git commit -m "fix(writer): restore a table cell's trailing whitespace

GFM trims both ends of a cell before parsing it. Task 3 covered the leading
edge through Pos::LineStart; the trailing edge is not a positional question,
so it is fixed on the rendered cell. Fixing one edge and not the other would
leave escaping spec §5 half-true inside Ctx::Cell."
```

---

### Task 5: The pre-render inline fold

A heading `[Text("A\r"), Code("\nB")]` anchors `a-b` while GitHub computes `a--b`, because each inline's newlines are folded independently and `code_span` folds its content to a space before the outer `one_line` can see two runs meet.

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs` (`fold_inline_newlines` and helpers, plus tests)
- Modify: `crates/kasane-writer/src/markdown.rs` (`Block::Heading`, `Block::Figure` alt, the external-link arm, plus a test)

**Interfaces:**
- Consumes: `Pos` from Task 1.
- Produces: `pub(crate) fn fold_inline_newlines(inls: &[Inline]) -> Vec<Inline>` in `escape.rs`.

- [ ] **Step 1: Write the failing escape.rs tests**

`kasane_ir::Inline` derives only `Clone` and `Debug`, so these compare a
flattened view rather than the values directly. Add the helper inside
`escape.rs`'s test module:

```rust
    /// A comparable view of a folded run: one `(kind, content)` pair per
    /// inline. `Inline` has no `PartialEq`, and widening a public IR type's
    /// derives for a writer test is not worth it — the leaf contents are the
    /// whole of what the fold changes.
    fn shape(inls: &[Inline]) -> Vec<(&'static str, String)> {
        inls.iter()
            .map(|i| match i {
                Inline::Text(t) => ("text", t.clone()),
                Inline::Code(t) => ("code", t.clone()),
                Inline::Math(t) => ("math", t.clone()),
                Inline::FootnoteRef(n) => ("ref", n.0.to_string()),
                Inline::Emph(_) => ("emph", String::new()),
                Inline::Strong(_) => ("strong", String::new()),
                Inline::Link { .. } => ("link", String::new()),
            })
            .collect()
    }

    /// The residual §4 exists for. `normalize_newlines` collapses a run inside
    /// one `Inline::Text` and `one_line` collapses one inside a single
    /// rendered string, but neither sees two runs meeting across a boundary:
    /// each inline renders independently, and `code_span` folds its own
    /// content to a space before the outer `one_line` ever runs. `anchor_fold`
    /// computes over the concatenated `inline_text`, where the two runs *are*
    /// adjacent, so it predicted one separator and the renderer emitted two.
    #[test]
    fn a_newline_run_collapses_across_an_inline_boundary() {
        let got = fold_inline_newlines(&[
            Inline::Text("A\r".into()),
            Inline::Code("\nB".into()),
        ]);
        assert_eq!(
            shape(&got),
            vec![("text", "A\n".to_string()), ("code", "B".to_string())]
        );
    }

    #[test]
    fn an_empty_run_does_not_break_the_collapse() {
        let got = fold_inline_newlines(&[
            Inline::Text("A\r".into()),
            Inline::Text(String::new()),
            Inline::Text("\nB".into()),
        ]);
        assert_eq!(
            shape(&got),
            vec![
                ("text", "A\n".to_string()),
                ("text", String::new()),
                ("text", "B".to_string()),
            ]
        );
    }

    /// `Inline::FootnoteRef` renders as visible `[^1]` text, so a run must not
    /// collapse across it — doing so would drop a space GitHub really renders.
    /// The residual that leaves open is the already-documented one (§4.1).
    #[test]
    fn a_footnote_reference_is_opaque_to_the_fold() {
        use kasane_ir::NoteId;

        let got = fold_inline_newlines(&[
            Inline::Text("a\n".into()),
            Inline::FootnoteRef(NoteId(1)),
            Inline::Text("\nb".into()),
        ]);
        assert_eq!(
            shape(&got),
            vec![
                ("text", "a\n".to_string()),
                ("ref", "1".to_string()),
                ("text", "\nb".to_string()),
            ]
        );
    }

    /// §4.2, and the half of it that is easy to get backwards. The fold
    /// collapses *runs* and normalizes `\r`; it never turns a newline into a
    /// space, because that is `one_line`, which runs long after `math_span`
    /// has picked its delimiter. So a lone newline inside one leaf survives
    /// and math still degrades to a code span — only the cross-boundary
    /// duplicate is dropped.
    #[test]
    fn a_lone_newline_inside_one_leaf_survives_the_fold() {
        let got = fold_inline_newlines(&[Inline::Math("a\nb".into())]);
        assert_eq!(shape(&got), vec![("math", "a\nb".to_string())]);

        let across = fold_inline_newlines(&[
            Inline::Text("A\r".into()),
            Inline::Math("\nB".into()),
        ]);
        assert_eq!(
            shape(&across),
            vec![("text", "A\n".to_string()), ("math", "B".to_string())]
        );
    }

    /// The fold runs *before* `inlines_to_md_at`'s depth guard, so it needs
    /// its own or a hand-built inline tree deeper than the bound overflows the
    /// stack here instead of being truncated there.
    #[test]
    fn the_fold_stops_at_the_inline_depth_bound() {
        let mut deep = vec![Inline::Text("x".into())];
        for _ in 0..(kasane_ir::MAX_INLINE_DEPTH + 50) {
            deep = vec![Inline::Emph(deep)];
        }
        // Must return rather than recurse to exhaustion.
        let _ = fold_inline_newlines(&deep);
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p kasane-writer --lib escape::tests::a_newline_run_collapses_across_an_inline_boundary`
Expected: FAIL to compile — `cannot find function 'fold_inline_newlines' in this scope`.

- [ ] **Step 3: Implement the fold**

Add `use kasane_ir::Inline;` to the top of `escape.rs`, then:

```rust
/// Collapse a newline run that spans an inline boundary, for the one-line
/// contexts (§4).
///
/// `normalize_newlines` already collapses a run inside one `Inline::Text`, and
/// `one_line` collapses one inside a single rendered string — but neither can
/// see two runs that meet across a boundary, because each inline is rendered
/// independently and `code_span` folds its own content to a space before the
/// outer `one_line` ever runs. `anchor_fold` computes over the concatenated
/// `inline_text`, where the two runs *are* adjacent, so it predicts one
/// separator where the renderer emitted two, and the cross-reference it
/// embeds is dead.
///
/// The fix lands here rather than in `kasane-core`'s mirror on purpose: the
/// two folds are kept in step by hand (see `one_line`, and AGENTS.md), and
/// teaching the anchor side about inline boundaries would add `code_span`'s
/// padding rules to what that hand-kept correspondence has to track.
///
/// `Inline::FootnoteRef` is opaque: it renders as visible `[^1]` text, so a
/// run must not collapse across it. The residual that leaves is the
/// footnote-reference divergence `kasane-core::slug` already documents (§4.1).
pub(crate) fn fold_inline_newlines(inls: &[Inline]) -> Vec<Inline> {
    let mut pending = false;
    fold_seq(inls, 0, &mut pending)
}

fn fold_seq(inls: &[Inline], depth: usize, pending: &mut bool) -> Vec<Inline> {
    // This runs before `inlines_to_md_at`'s guard, so it carries its own:
    // `blocks_to_markdown` is public API over a public IR, and a hand-built
    // tree deeper than the bound would otherwise overflow the stack here
    // rather than being truncated there.
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return inls.to_vec();
    }
    inls.iter()
        .map(|i| match i {
            Inline::Text(t) => Inline::Text(fold_leaf(t, pending)),
            Inline::Code(t) => Inline::Code(fold_leaf(t, pending)),
            Inline::Math(t) => Inline::Math(fold_leaf(t, pending)),
            Inline::Emph(x) => Inline::Emph(fold_seq(x, depth + 1, pending)),
            Inline::Strong(x) => Inline::Strong(fold_seq(x, depth + 1, pending)),
            Inline::Link { target, inlines } => Inline::Link {
                target: target.clone(),
                inlines: fold_seq(inlines, depth + 1, pending),
            },
            Inline::FootnoteRef(n) => {
                *pending = false;
                Inline::FootnoteRef(*n)
            }
        })
        .collect()
}

/// Fold one leaf's content, carrying `pending` in and out so a run that ends
/// this leaf and begins the next collapses to a single `\n` — which
/// `one_line` then turns into the one space `anchor_fold` predicted.
fn fold_leaf(s: &str, pending: &mut bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '\n' || c == '\r' {
            if !*pending {
                out.push('\n');
            }
            *pending = true;
        } else {
            out.push(c);
            *pending = false;
        }
    }
    out
}
```

- [ ] **Step 4: Run them to verify they pass**

Run: `cargo test -p kasane-writer --lib escape::tests`
Expected: PASS.

- [ ] **Step 5: Write the failing wiring test**

Add to `markdown.rs`'s test module:

```rust
    /// The anchor kasane embeds must equal the id GitHub computes from the
    /// rendered heading line. Before the fold this shape rendered `A  B`
    /// (two spaces — one from each run's independent fold) against an
    /// embedded `a-b`.
    #[test]
    fn a_newline_run_split_by_a_code_span_yields_one_separator() {
        use pulldown_cmark::{Event, Options, Parser};

        let blocks = vec![Block::Heading {
            level: 2,
            id: BlockId(0),
            inlines: vec![Inline::Text("A\r".into()), Inline::Code("\nB".into())],
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());

        let mut heading = String::new();
        let mut depth = 0;
        for ev in Parser::new_ext(&md, Options::empty()) {
            match ev {
                Event::Start(pulldown_cmark::Tag::Heading { .. }) => depth += 1,
                Event::End(pulldown_cmark::TagEnd::Heading(_)) => depth -= 1,
                Event::Text(t) | Event::Code(t) if depth > 0 => heading.push_str(&t),
                _ => {}
            }
        }
        assert_eq!(heading, "A B", "one separator, not two:\n{md}");
    }
```

- [ ] **Step 6: Run it to verify it fails**

Run: `cargo test -p kasane-writer --lib markdown::tests::a_newline_run_split_by_a_code_span_yields_one_separator`
Expected: FAIL — `left: "A  B", right: "A B"`.

- [ ] **Step 7: Wire the fold into the three one-line contexts**

`Block::Heading`:

```rust
        Block::Heading { level, inlines, .. } => {
            for _ in 0..(*level).min(6) {
                out.push('#');
            }
            out.push(' ');
            let inlines = escape::fold_inline_newlines(inlines);
            out.push_str(&escape::one_line(&inlines_to_md(&inlines, Ctx::Flow, Pos::Mid)));
            out.push('\n');
        }
```

`Block::Figure`'s alt text:

```rust
            let caption = escape::fold_inline_newlines(caption);
            let alt = escape::one_line(&inlines_to_md(&caption, Ctx::Flow, Pos::Mid));
```

The external-link arm in `inlines_to_md_at`:

```rust
            Inline::Link {
                target: RefTarget::External(u),
                inlines,
            } => s.push_str(&format!(
                "[{}]({})",
                escape::one_line(&inlines_to_md_at(
                    &escape::fold_inline_newlines(inlines),
                    depth + 1,
                    ctx,
                    pos
                )),
                escape::dest_url(u)
            )),
```

Only the heading feeds an anchor. All three get the fold because `one_line` is the writer's marker for "this is one line by grammar", and letting the fold mean different things in different one-line contexts is how the divergence got in.

- [ ] **Step 8: Run it to verify it passes**

Run: `cargo test -p kasane-writer --lib markdown::tests::a_newline_run_split_by_a_code_span_yields_one_separator`
Expected: PASS.

- [ ] **Step 9: Run the full suite**

Run: `mise run test`
Expected: PASS. P2 in `properties.rs` is the one to watch: it recomputes anchors from parsed heading text and compares them against what the engine embedded, so a fold that overshoots fails it.

- [ ] **Step 10: Lint and commit**

```bash
mise run lint
git add crates/kasane-writer/src/
git commit -m "fix(writer): collapse a newline run across an inline boundary

A heading [Text(\"A\\r\"), Code(\"\\nB\")] anchored a-b while GitHub computed
a--b: each inline's newlines fold independently, and code_span folds its
content to a space before the outer one_line can see the two runs meet. The
emitted cross-reference pointed at an id no render assigns.

Fixed on the writer side rather than in kasane-core's anchor_fold, so the
hand-kept mirror between the two folds stays exactly as wide as it is —
teaching the anchor side about inline boundaries would add code_span's
padding rules to what it has to track.

Inline::FootnoteRef is opaque to the fold: it renders as visible text, so a
run must not collapse across it. That leaves the footnote-reference
divergence slug.rs already documents, which needs a different fix."
```

---

### Task 6: Property coverage for the fold's class

Task 5's unit test pins one shape. This task covers the class, and adds the seam it needs.

**Files:**
- Modify: `crates/kasane-core/src/slug.rs` (add `anchor_slug_of` test seam)
- Modify: `crates/kasane-core/src/lib.rs:19` (export it)
- Modify: `crates/kasane-writer/tests/properties.rs` (the focused property)
- Modify: `crates/kasane-writer/tests/generator/mod.rs` (two `HOSTILE` fragments; the stale count in `is_comment`'s doc comment)

**Interfaces:**
- Consumes: `fold_inline_newlines`'s behaviour from Task 5 (through the writer, not directly).
- Produces: `#[doc(hidden)] pub fn anchor_slug_of(inlines: &[Inline]) -> String` in `kasane-core`.

- [ ] **Step 1: Add the `anchor_slug_of` seam**

`anchors_for_headings` takes `&[String]`, so it cannot express "the anchor for *these inlines*" — which is exactly what the property has to compare against. Add to `crates/kasane-core/src/slug.rs`, next to `path_slug_of`:

```rust
/// Test seam for the anchor rule over real inline structure, same rationale
/// as `path_slug_of` and `anchors_for_headings`.
///
/// `anchors_for_headings` takes rendered heading *strings* and re-wraps each
/// as a single `Inline::Text`, which is right for the parsed side of a
/// comparison and wrong for the IR side: the residuals spec §5.2 property
/// needs the engine's anchor for an inline run whose *structure* is the thing
/// under test. No counter is threaded because a single heading has no
/// duplicate to suffix.
#[doc(hidden)]
pub fn anchor_slug_of(inlines: &[Inline]) -> String {
    AnchorCounter::new().next(inlines)
}
```

Export it in `crates/kasane-core/src/lib.rs`:

```rust
pub use slug::{anchor_slug_of, anchors_for_headings, path_slug_of};
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p kasane-core`
Expected: PASS. If `AnchorCounter::new()` or `.next()` has a different name or arity, read `anchors_for_headings` directly above and mirror whatever it calls.

- [ ] **Step 3: Write the focused property**

Add to `crates/kasane-writer/tests/properties.rs`, inside the existing `proptest!` block.

The file currently imports only `use kasane_ir::Block;` at file scope — the IR
types below are pulled in per-test further down. A `proptest!` body cannot
carry its own `use`, so widen both import lines at the top of the file:

```rust
use kasane_core::{anchor_slug_of, anchors_for_headings, est_tokens, structure, FileNode};
use kasane_ir::{AssetBag, Block, BlockId, Inline, RefTarget};
```

If that makes an existing per-test `use kasane_ir::{…}` redundant, delete it —
`-D warnings` fails the build on an unused import.

Then the property:

```rust
    /// P9 — the anchor/render agreement, on the shape the main tier cannot
    /// reach (residuals spec §5.2).
    ///
    /// A newline run split across an inline boundary must produce exactly one
    /// separator in the rendered heading line, so the id GitHub computes from
    /// that line equals the anchor the engine embedded. The main tier can draw
    /// this shape but lands it about one run in seven (§5.3), which is why it
    /// gets a generator narrow enough to hit it every time.
    #[test]
    fn p9_boundary_newline_runs_anchor_the_same(
        nl1 in prop_oneof![Just("\n"), Just("\r"), Just("\r\n")],
        nl2 in prop_oneof![Just("\n"), Just("\r"), Just("\r\n")],
        kind in 0usize..5,
    ) {
        let head = format!("{nl2}b");
        let second = match kind {
            0 => Inline::Code(head),
            1 => Inline::Math(head),
            2 => Inline::Emph(vec![Inline::Text(head)]),
            3 => Inline::Strong(vec![Inline::Text(head)]),
            _ => Inline::Link {
                target: RefTarget::External("http://example.invalid/".into()),
                inlines: vec![Inline::Text(head)],
            },
        };
        let inlines = vec![Inline::Text(format!("a{nl1}")), second];

        let blocks = vec![Block::Heading {
            level: 2,
            id: BlockId(0),
            inlines: inlines.clone(),
        }];
        let md = kasane_writer::blocks_to_markdown(&blocks, &AssetBag::default());

        // The id a renderer computes from the line the writer emitted.
        let rendered = anchors_for_headings(&parse_events(&md).headings);
        // The anchor the engine embeds in every cross-reference to it.
        let embedded = anchor_slug_of(&inlines);

        prop_assert_eq!(
            rendered.first().map(String::as_str),
            Some(embedded.as_str()),
            "anchor/render divergence for kind {} nl1 {:?} nl2 {:?}:\n{}",
            kind, nl1, nl2, md
        );
    }
```

- [ ] **Step 4: Run it**

Run: `cargo test -p kasane-writer --test properties p9_boundary_newline_runs_anchor_the_same`
Expected: PASS — Task 5 fixed the defect this covers. If it fails, the fold is wrong for one of the five inline kinds; read the printed `md` and the two anchors before changing anything.

- [ ] **Step 5: Confirm the property actually catches the bug**

Temporarily comment out the `escape::fold_inline_newlines` call in `markdown.rs`'s `Block::Heading` arm and re-run step 4.
Expected: FAIL, naming the divergence. Restore the line and re-run.
Expected: PASS.

A property that passes both with and without the fix tests nothing — this step is what distinguishes the two.

- [ ] **Step 6: Add the two `HOSTILE` fragments**

In `crates/kasane-writer/tests/generator/mod.rs`, at the end of `HOSTILE`:

```rust
    // A run that *ends* one inline and one that *begins* the next. Together
    // they let the main tier draw the boundary shape P9 covers directly
    // (residuals spec §5.3) -- which matters for shrinking, not for coverage:
    // the payload must end with the first fragment (1 in 25), the decoration
    // must be an `Inline::Code` (1 in 13), and that code's filler must draw
    // the second as its first word (1 in 100). About 1 in 32,500 per shape,
    // so roughly one default run in seven sees it at all. P9 and the unit
    // tests are what hold this line; these fragments only make a hit useful.
    "trailing\r",
    "\nleading",
```

- [ ] **Step 7: Fix the stale fragment count**

The `is_comment` doc comment in the same file says "of `HOSTILE`'s 21 fragments, only `-->` triggers it; the other 20 round-trip". The list held 23 before step 6 — the two newline-run fragments were added without updating the count — and holds 25 after. Correct both numbers:

```rust
    /// is narrow -- of `HOSTILE`'s 25 fragments, only `-->` triggers it; the
    /// other 24 round-trip through a comment verbatim like anything else.
```

- [ ] **Step 8: Run the full suite**

Run: `mise run test`
Expected: PASS. The two new fragments widen what every property draws, so a failure here is a real find — read the shrunk case rather than reverting the fragments.

- [ ] **Step 9: Lint and commit**

```bash
mise run lint
git add crates/kasane-core/src/ crates/kasane-writer/tests/
git commit -m "test(writer): cover the boundary fold's class, and reach it from the tier

P9 asserts the anchor/render agreement over the shape §4.5 left uncovered:
a newline run split across an inline boundary, for each of the five inline
kinds that can carry the second half. Verified to fail with the fold removed.

anchor_slug_of joins path_slug_of and anchors_for_headings as a doc(hidden)
seam. anchors_for_headings re-wraps rendered strings as one Inline::Text,
which is right for the parsed side of the comparison and wrong for the IR
side, where the structure is the thing under test.

Two HOSTILE fragments let the main tier draw the shape too. That is for
shrinking, not coverage -- about 1 in 32,500 per shape. Also corrects
is_comment's fragment count, stale at 21 against a list of 23."
```

---

### Task 7: Documentation

**Files:**
- Modify: `docs/superpowers/specs/2026-08-09-markdown-escaping-design.md` (§4.5, §8)
- Modify: `README.md` (Known limitations)
- Modify: `AGENTS.md` (`kasane-writer` entry)

**Interfaces:**
- Consumes: everything above. No code changes.

- [ ] **Step 1: Rewrite §4.5's residual list**

In `docs/superpowers/specs/2026-08-09-markdown-escaping-design.md`, replace the passage running from "Three sibling cases are **not** fixed and are recorded here rather than in a ledger." through "None of these has property-tier coverage, and neither does the emphasis fix itself — see §6.4 for why the generator cannot reach it." with:

```markdown
All three sibling cases recorded here as unfixed were closed by the escaping
residuals item (2026-08-13); see that spec for the mechanisms. Two of the
three were smaller than this section assumed, and for the same reason: this
section reasoned from the CommonMark grammar where a parser was available.
`   \# h` does suppress the heading, but the parser then strips the three
leading spaces, so the backslash form loses the text it was meant to protect —
a §5 violation presenting as a fix. A numeric character reference preserves it
and disarms the whole run, which made the "changes the line-start rules for
every context at once" characterization an artifact of the mechanism rather
than of the defect.

Two cases remain open, both narrower than what was closed:

- **A newline run split across an `Inline::FootnoteRef`.** The reference
  renders as visible `[^1]` text that `inline_text` skips, so the fold cannot
  collapse across it without dropping a space GitHub renders. Same root cause
  as the `## Notes[^1]` divergence `kasane-core::slug` documents as surviving
  on purpose; it needs §8's approach (iii), not a fold change.
- **Whitespace inside the merged-table HTML fallback.** Not reachable by
  escaping: an HTML renderer collapses whitespace runs whether they arrived
  literally or as `&#32;`. One more cost of that path, alongside §3.3's.

The emphasis fix in this section still has no property-tier coverage — see
§6.4 for why the generator cannot reach it.
```

- [ ] **Step 2: Add approach (iii) to §8**

Append to §8 ("Approaches considered"):

```markdown
### 8.2 A shared crate for the anchor rule

Not taken, here or by the residuals item (2026-08-13), and recorded so the
next anchor divergence finds the argument rather than re-deriving it.

`anchor_slug` predicts the rendered heading line from IR inlines, and
`escape::one_line` produces that line — two hand-kept mirrors in crates that
cannot depend on each other. Moving both onto a shared crate would end the
mirror and close the whole divergence class at once, including the
footnote-reference case (§4.5) and the trailing-`#` case, both of which are
currently documented as surviving on purpose.

It was not taken because `assign_paths` needs the anchor at structuring time,
before anything is rendered, so the shared model has to be a *prediction* of
the rendered line either way — the crate boundary moves, the prediction
remains. That makes it an architecture change with a real ripple and a
smaller payoff than it first appears, and it deserves its own item rather
than riding along with a fix.
```

- [ ] **Step 3: Update README's Known limitations**

In the "Text that looks like Markdown is preserved as text" bullet, after the sentence ending "`a\|b` inside a table cell.", add:

```markdown
  Leading whitespace on a line is carried by a character reference — `&#32;`
  or `&#9;` — rather than a backslash, because a backslash would suppress the
  construct by losing the whitespace. Table cells now keep the whitespace at
  both their edges, which GFM would otherwise trim away: text that earlier
  builds dropped silently now survives.
```

- [ ] **Step 4: Update AGENTS.md**

In the `crates/kasane-writer` entry, after the sentence about `file_to_markdown` opening every file with its frontmatter title, add:

```markdown
  `escape.rs`'s `Pos` is the writer's escaping-position vocabulary: `LineStart`,
  `AfterFootnoteRef`, `Mid`. It has three states rather than two because a
  `[^n]` that opened the line makes a following `:` a footnote *definition*
  delimiter, and the `:` belongs to the *next* inline — `markdown.rs` computes
  the position, `escape.rs` still owns every rule. Whitespace at `LineStart`
  becomes `&#32;`/`&#9;`, not a backslash: the backslash form suppresses the
  construct only by losing the whitespace, and it cannot reach the
  four-spaces-is-an-indented-code-block case at all.
  `fold_inline_newlines` collapses a newline run spanning an inline boundary
  before the one-line contexts render, which is what keeps the rendered
  heading line matching `kasane-core`'s `anchor_fold` without widening the
  hand-kept mirror described below.
```

The `kasane-core` entry and `slug.rs`'s module docs are deliberately **not**
touched: the passage describing the two hand-kept folds stays accurate word
for word, which is the payoff of fixing this on the writer side.

- [ ] **Step 5: Verify the docs match the code**

Run: `mise run test && mise run lint`
Expected: PASS.

Then re-read §4.5's rewritten text against `escape.rs` and `markdown.rs` as they now stand. Every mechanism named must exist under that name. Fix any that drifted during Tasks 1–6.

- [ ] **Step 6: Commit**

```bash
git add docs/ README.md AGENTS.md
git commit -m "docs: record the three closed residuals and the two that remain

§4.5 stops being a deferred list. Names what stayed open and why each needs a
different fix than this item had: the footnote-ref-adjacent fold shares a root
cause with a divergence slug.rs already documents, and whitespace in the
merged-table fallback is not reachable by escaping at all.

§8.2 writes up the shared-crate option that would end the core/writer mirror,
including the reason it is smaller a win than it looks: assign_paths needs the
anchor before anything renders, so the shared model stays a prediction either
way."
```

---

## Verification

- [ ] **Full gate**

```bash
mise run test
mise run lint
```

Both PASS.

- [ ] **`KNOWN_OPEN` is still empty**

Run: `grep -n 'KNOWN_OPEN: ' crates/kasane-adapters/tests/fuzz_corpus.rs`
Expected: `const KNOWN_OPEN: &[(&str, &str)] = &[];`

- [ ] **The fuzz target builds and runs briefly**

Run: `mise run fuzz escape -- -max_total_time=60`
Expected: no crash. This needs the pinned nightly; if it is unavailable in the environment, say so in the PR rather than skipping silently — the stable replay in `kasane-adapters/tests/fuzz_corpus.rs` runs as part of `mise run test` either way.

- [ ] **A real conversion still works end to end**

```bash
mise run convert tests/fixtures/epub/rich.epub -o /tmp/kasane-check
grep -rn '&#32;\|&#9;\|\\:' /tmp/kasane-check || echo "none — expected"
```

Expected: a normal tree, and the grep finding nothing. `rich.epub` is ordinary
prose, and all three mechanisms here fire only on text that really has
whitespace at a line start or a colon after a footnote reference. A hit means
one of the rules is over-firing.
