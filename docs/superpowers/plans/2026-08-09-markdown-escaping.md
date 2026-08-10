# Markdown Escaping Policy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `kasane-writer` escape every piece of document text it emits, so a book containing `*`, `|`, `` ` ``, `[`, `&`, `<` or a leading `#` converts to Markdown that renders as that text rather than as markup.

**Architecture:** One new `crates/kasane-writer/src/escape.rs` owns every rule. The inline renderers take a required `Ctx` argument so a call site must name its output context; `Inline::Text` is the only arm allowed to call `escape::text`. Four structural fixes escaping alone cannot make — newline folding, footnote and list-item continuation indents, code-span and fence widening — land in the same branch. Verification is a round-trip property over a real GFM parser plus a fuzz target asserting the escape functions' postconditions.

**Tech Stack:** Rust 1.97.1 (pinned in `mise.toml`), `proptest` 1.11, `pulldown-cmark` 0.13 (new, **dev-dependency only**), `cargo-fuzz` 0.13.2 on `nightly-2026-07-01`.

**Design spec:** `docs/superpowers/specs/2026-08-09-markdown-escaping-design.md`. Section references below (§2, §3.1, …) point at it.

## Global Constraints

- Every task ships green under `mise run lint && mise run test`. `lint` is `cargo fmt --all -- --check` **and** `cargo clippy --workspace --all-targets -- -D warnings` — `--all-targets` includes tests, so a warning in a test file fails the gate.
- **Escaping must never change what the Markdown renders to** (§5). `anchor_slug` computes fragments from unescaped IR text while GitHub computes ids from rendered text; a scheme that substituted characters instead of escaping them would break every in-book cross-reference. This is why `library.rs`'s current `[`→`(` substitution does not survive.
- Paths and anchors are byte-identical before and after this branch. Only file *content* changes.
- `pulldown-cmark` is a **dev-dependency of `kasane-writer` only**. It must not appear in `[dependencies]` of any crate. The fuzz seam (Task 14) therefore asserts postconditions, not round-trips through a parser.
- New `pub(crate)` code that no caller uses yet warns under `-D warnings`. Tasks 1–5 build `escape.rs` before its callers exist, so the module carries `#![allow(dead_code)]` at its top from Task 1 and **Task 6 removes it**. Do not leave it in place past Task 6.
- Commit messages follow the repo's convention: `type(scope): imperative summary`, with a body explaining *why* when the change is not self-evident. Scopes in use: `core`, `writer`, `adapters`, `cli`, `fuzz`, `docs`, `test`.
- Never weaken an assertion to make a test pass. If a test fails in a way the plan does not predict, that is a finding — stop and report it rather than adjusting the expectation.

## File Structure

**Created:**
- `crates/kasane-writer/src/escape.rs` — every escaping rule, one function per output context. The only path from document text to an output buffer.
- `crates/kasane-writer/src/fuzz_entry.rs` — `#[doc(hidden)] pub` fuzz seam, same convention as `kasane-core/src/fuzz_entry.rs`.
- `fuzz/fuzz_targets/escape.rs` — libFuzzer wrapper.
- `fuzz/seeds/escape/` — hand-written starting inputs.

**Modified:**
- `crates/kasane-writer/src/markdown.rs` — every render arm routed through `escape`; tables, footnotes, list items, fences.
- `crates/kasane-writer/src/lib.rs` — module declarations; `file_to_markdown`'s title heading.
- `crates/kasane-writer/src/frontmatter.rs` — `yaml_str` replaced by `escape::yaml_scalar`.
- `crates/kasane-writer/src/library.rs` — local `link_text`/`link_dest` folded into `escape`.
- `crates/kasane-writer/Cargo.toml` — `pulldown-cmark` dev-dependency.
- `crates/kasane-writer/tests/generator/mod.rs` — hostile alphabet, payload ledger.
- `crates/kasane-writer/tests/properties.rs` — helpers rebuilt on the parser; P7 added.
- `crates/kasane-adapters/tests/fuzz_corpus.rs` + `crates/kasane-adapters/Cargo.toml` — replay the new target on stable.
- `fuzz/Cargo.toml` — `kasane-writer` dependency, `escape` bin.
- `mise.toml` — `escape` (and `slug`) added to `fuzz-all`.
- `README.md`, `AGENTS.md` — user-facing and codebase-map documentation.

---

### Task 1: `escape::text` — the `Ctx::Flow` rules

**Files:**
- Create: `crates/kasane-writer/src/escape.rs`
- Modify: `crates/kasane-writer/src/lib.rs:1-3` (add `mod escape;`)
- Test: `crates/kasane-writer/src/escape.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub(crate) enum Ctx { Flow, Cell, Html }` (all three variants declared now, `Cell`/`Html` implemented in Task 2) and `pub(crate) fn text(s: &str, ctx: Ctx, at_line_start: bool) -> String`.

- [ ] **Step 1: Write the failing tests**

Create `crates/kasane-writer/src/escape.rs` containing only this test module plus the module header, so the first run fails to compile on the missing items:

```rust
//! Every rule for turning document text into Markdown that renders as that
//! text. Design spec `2026-08-09-markdown-escaping-design.md` §3.
//!
//! One function per output context, because the contexts do not share a
//! mechanism: a backslash escapes in flow text, is a literal character inside
//! a code span, is inert inside an HTML block, and means something else again
//! inside a YAML double-quoted scalar.
#![allow(dead_code)] // callers arrive in Task 6; removed there

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_escapes_every_always_character() {
        for c in ['\\', '`', '*', '_', '[', ']', '<', '~', '$'] {
            let input = format!("a{c}b");
            let got = text(&input, Ctx::Flow, false);
            assert_eq!(got, format!("a\\{c}b"), "input {input:?}");
        }
    }

    #[test]
    fn flow_leaves_bang_alone_because_bracket_is_escaped() {
        // `!` matters only before `[`, and `[` is always escaped, so `!\[`
        // can never form an image. Every "Wow!" in the corpus stays clean.
        assert_eq!(text("Wow! [see]", Ctx::Flow, false), "Wow! \\[see\\]");
    }

    #[test]
    fn flow_escapes_ampersand_only_when_it_opens_an_entity() {
        assert_eq!(text("Q&A", Ctx::Flow, false), "Q&A");
        assert_eq!(text("Tom & Jerry", Ctx::Flow, false), "Tom & Jerry");
        assert_eq!(text("a&amp;b", Ctx::Flow, false), "a\\&amp;b");
        assert_eq!(text("a&#38;b", Ctx::Flow, false), "a\\&#38;b");
        assert_eq!(text("a&#x26;b", Ctx::Flow, false), "a\\&#x26;b");
        // No terminating semicolon: not an entity, no escape.
        assert_eq!(text("a&amp b", Ctx::Flow, false), "a&amp b");
    }

    #[test]
    fn flow_escapes_line_start_characters_only_at_a_line_start() {
        for c in ['#', '-', '+', '>', '=', '|'] {
            let input = format!("{c}x");
            assert_eq!(
                text(&input, Ctx::Flow, true),
                format!("\\{c}x"),
                "at line start: {input:?}"
            );
            assert_eq!(
                text(&input, Ctx::Flow, false),
                input,
                "mid-line: {input:?}"
            );
        }
    }

    #[test]
    fn flow_escapes_an_ordered_list_marker_delimiter() {
        assert_eq!(text("1. one", Ctx::Flow, true), "1\\. one");
        assert_eq!(text("12) two", Ctx::Flow, true), "12\\) two");
        // Not a marker: no digits, or no delimiter, or not at a line start.
        assert_eq!(text("1x. one", Ctx::Flow, true), "1x. one");
        assert_eq!(text("1. one", Ctx::Flow, false), "1. one");
    }

    #[test]
    fn flow_re_arms_line_start_after_an_interior_newline() {
        // The second line of a text run can open a block just as the first can.
        assert_eq!(
            text("intro\n# not a heading", Ctx::Flow, false),
            "intro\n\\# not a heading"
        );
    }

    #[test]
    fn flow_collapses_blank_lines_so_one_para_stays_one_para() {
        assert_eq!(text("a\n\n\nb", Ctx::Flow, false), "a\nb");
        assert_eq!(text("a\r\n\r\nb", Ctx::Flow, false), "a\nb");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p kasane-writer escape 2>&1 | tail -20
```

Expected: FAIL — compile errors, `cannot find function 'text' in this scope` and `cannot find type 'Ctx'`. (`lib.rs` does not declare the module yet, so also expect the file to be ignored until Step 3 — add `mod escape;` as part of Step 3, then re-run.)

- [ ] **Step 3: Write the implementation**

Add `mod escape;` to `crates/kasane-writer/src/lib.rs` alongside the existing `mod frontmatter; mod library; mod markdown;` declarations (keep them alphabetical: `escape` goes first).

Then prepend to `escape.rs`, above the test module:

```rust
/// Where a text run lands. The rules differ per context, so the renderers
/// must state which one they are in — `inlines_to_md` takes this as a
/// required argument for exactly that reason (§2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ctx {
    /// Paragraph, heading, list item, footnote body, caption, link label.
    Flow,
    /// A GFM table cell: `Flow` plus `|`, minus newlines.
    Cell,
    /// Inside the `has_merged` `<table>` fallback, where backslashes are inert.
    Html,
}

/// Characters that can open an inline construct at any position (§3.1).
/// `\` is first because every escape below would otherwise be escapable.
const ALWAYS: &[char] = &['\\', '`', '*', '_', '[', ']', '<', '~', '$'];

/// Characters that can only open a *block* construct, and so matter solely at
/// the start of a line (§3.1). `-` and `=` cover setext underlines and `---`
/// thematic breaks as well as bullets.
const LINE_START: &[char] = &['#', '-', '+', '>', '=', '|'];

pub(crate) fn text(s: &str, ctx: Ctx, at_line_start: bool) -> String {
    if ctx == Ctx::Html {
        return html_text(s);
    }
    let chars: Vec<char> = normalize_newlines(s).chars().collect();
    let mut out = String::with_capacity(chars.len() + 8);
    let mut line_start = at_line_start;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];

        if c == '\n' {
            match ctx {
                Ctx::Cell => out.push_str("<br>"),
                _ => {
                    out.push('\n');
                    line_start = true;
                }
            }
            i += 1;
            continue;
        }

        if line_start {
            if let Some(after_digits) = ordered_marker_delimiter(&chars, i) {
                for d in &chars[i..after_digits] {
                    out.push(*d);
                }
                out.push('\\');
                out.push(chars[after_digits]);
                i = after_digits + 1;
                line_start = false;
                continue;
            }
            if LINE_START.contains(&c) {
                out.push('\\');
                out.push(c);
                i += 1;
                line_start = false;
                continue;
            }
        }

        if ALWAYS.contains(&c) || (ctx == Ctx::Cell && c == '|') {
            out.push('\\');
            out.push(c);
        } else if c == '&' && opens_entity(&chars, i) {
            out.push('\\');
            out.push('&');
        } else {
            out.push(c);
        }
        line_start = false;
        i += 1;
    }
    out
}

/// `\r\n` and `\r` become `\n`, then runs of two or more `\n` collapse to one.
///
/// The collapse is what keeps one `Block::Para` one block: a blank line inside
/// a text run would otherwise split it into two paragraphs, which P7's
/// structural half would (correctly) fail on. In practice these are PDF and
/// DjVu line-break artifacts rather than authored breaks.
fn normalize_newlines(s: &str) -> String {
    let unified = s.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(unified.len());
    let mut last_was_newline = false;
    for c in unified.chars() {
        if c == '\n' {
            if !last_was_newline {
                out.push('\n');
            }
            last_was_newline = true;
        } else {
            out.push(c);
            last_was_newline = false;
        }
    }
    out
}

/// `Some(index_of_delimiter)` when `chars[i..]` begins an ordered-list marker:
/// one or more ASCII digits followed by `.` or `)`. `None` otherwise.
fn ordered_marker_delimiter(chars: &[char], i: usize) -> Option<usize> {
    if !chars.get(i)?.is_ascii_digit() {
        return None;
    }
    let mut j = i;
    while chars.get(j).is_some_and(char::is_ascii_digit) {
        j += 1;
    }
    match chars.get(j) {
        Some('.') | Some(')') => Some(j),
        _ => None,
    }
}

/// Whether `chars[i]` (a `&`) begins an entity reference, per CommonMark:
/// `&name;`, `&#decimal;`, or `&#xhex;`.
///
/// The lookahead is exact rather than heuristic, and that is why it is worth
/// having: `&` is common in real titles, and escaping it unconditionally would
/// backslash every "Q&A" and "Tom & Jerry" in the corpus for no parsing
/// benefit (§3.1).
fn opens_entity(chars: &[char], i: usize) -> bool {
    let mut j = i + 1;
    let numeric = chars.get(j) == Some(&'#');
    if numeric {
        j += 1;
        let hex = matches!(chars.get(j), Some('x') | Some('X'));
        if hex {
            j += 1;
        }
        let start = j;
        while chars
            .get(j)
            .is_some_and(|c| if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() })
        {
            j += 1;
        }
        return j > start && chars.get(j) == Some(&';');
    }
    let start = j;
    while chars.get(j).is_some_and(|c| c.is_ascii_alphanumeric()) {
        j += 1;
    }
    j > start && chars.get(j) == Some(&';')
}

/// HTML-escape, for `Ctx::Html`. Implemented in Task 2; `text` already routes
/// to it so the routing is written once.
fn html_text(s: &str) -> String {
    s.to_string()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kasane-writer escape 2>&1 | tail -20
```

Expected: PASS, 7 tests.

- [ ] **Step 5: Lint**

```bash
mise run lint
```

Expected: clean. If clippy objects to `is_some_and(char::is_ascii_digit)`, use `is_some_and(|c| c.is_ascii_digit())` — both compile; take whichever clippy accepts.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-writer/src/escape.rs crates/kasane-writer/src/lib.rs
git commit -m "feat(writer): escape.rs and the Ctx::Flow rules

Design spec 2026-08-09 section 3.1. Nine characters that can open an inline
construct anywhere, six that matter only at a line start, plus the ordered-list
marker delimiter. The ampersand is escaped only when what follows would really
parse as an entity reference, because unconditional escaping would backslash
every Q&A in the corpus for no parsing benefit; the exclamation mark is not
escaped at all, since it only matters before a bracket and brackets always are."
```

---

### Task 2: `Ctx::Cell`, `Ctx::Html`, and `escape::label`

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs`
- Test: same file

**Interfaces:**
- Consumes: `Ctx`, `text` (Task 1).
- Produces: working `Ctx::Cell` and `Ctx::Html` behaviour; `pub(crate) fn label(s: &str) -> String`; `pub(crate) fn one_line(s: &str) -> String`.

- [ ] **Step 1: Write the failing tests**

Append to `escape.rs`'s `mod tests`:

```rust
    #[test]
    fn cell_escapes_pipes_everywhere_and_carries_newlines_as_br() {
        assert_eq!(text("a|b", Ctx::Cell, false), "a\\|b");
        assert_eq!(text("one\ntwo", Ctx::Cell, false), "one<br>two");
        // Flow's rules still apply inside a cell.
        assert_eq!(text("a*b", Ctx::Cell, false), "a\\*b");
    }

    #[test]
    fn html_escapes_entities_not_backslashes() {
        assert_eq!(
            text("a & b < c > d \" e", Ctx::Html, false),
            "a &amp; b &lt; c &gt; d &quot; e"
        );
        // A backslash is a literal character inside an HTML block.
        assert_eq!(text("a\\b", Ctx::Html, false), "a\\b");
        // Every `&` is an entity opener here, unconditionally.
        assert_eq!(text("Q&A", Ctx::Html, false), "Q&amp;A");
        assert_eq!(text("one\ntwo", Ctx::Html, false), "one<br>two");
    }

    #[test]
    fn one_line_folds_every_newline_spelling_to_a_single_space() {
        assert_eq!(one_line("a\nb\r\nc\rd"), "a b c d");
    }

    #[test]
    fn label_escapes_and_flattens() {
        // A label lives inside `[...]` on one line: brackets escaped, newline
        // folded. Escaped, never substituted -- a substitution would change the
        // rendered text, which section 5 forbids.
        assert_eq!(label("a [b]\nc"), "a \\[b\\] c");
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kasane-writer escape 2>&1 | tail -20
```

Expected: FAIL — `cell_escapes_pipes_everywhere_and_carries_newlines_as_br` passes already (Task 1 wrote the `Cell` arms), `html_escapes_entities_not_backslashes` fails on `assertion left == right` (`html_text` is the identity stub), and `one_line`/`label` fail to compile as undefined.

- [ ] **Step 3: Write the implementation**

Replace the `html_text` stub in `escape.rs` and add the two new functions:

```rust
/// HTML-escape, for `Ctx::Html` — the `has_merged` `<table>` fallback.
///
/// Backslash escapes do not apply there: GFM parses an HTML block's content as
/// raw HTML, not as Markdown. `&` is escaped unconditionally here, unlike in
/// flow text, because inside HTML every `&` really is an entity opener.
fn html_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in normalize_newlines(s).chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("<br>"),
            _ => out.push(c),
        }
    }
    out
}

/// Fold every newline spelling to a single space, for the contexts where a
/// newline is structurally impossible: heading lines, link labels, image alt
/// text, YAML scalars (§4.1).
pub(crate) fn one_line(s: &str) -> String {
    s.replace("\r\n", " ").replace(['\n', '\r'], " ")
}

/// A link label or image alt text: flow rules, flattened to one line.
///
/// Escaped, never substituted. `library.rs`'s superseded `link_text` replaced
/// `[` with `(`, which changes the rendered text — forbidden by §5, because
/// anchors are computed from unescaped IR text and must still match what the
/// heading renders to.
pub(crate) fn label(s: &str) -> String {
    one_line(&text(s, Ctx::Flow, false))
}
```

- [ ] **Step 4: Run to verify pass**

```bash
cargo test -p kasane-writer escape 2>&1 | tail -20
```

Expected: PASS, 11 tests.

- [ ] **Step 5: Commit**

```bash
mise run lint
git add crates/kasane-writer/src/escape.rs
git commit -m "feat(writer): the Cell and Html contexts, and label flattening

A cell adds the pipe and carries newlines as <br>, GFM's only multi-line-cell
carrier. The HTML context escapes entities instead of using backslashes,
because GFM parses an HTML block's content as raw HTML and a backslash there
is a literal character. label() escapes rather than substitutes: replacing a
bracket with a paren the way library.rs does today changes the rendered text,
which the anchor rule cannot survive."
```

---

### Task 3: Code spans and fences

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs`
- Test: same file

**Interfaces:**
- Consumes: `one_line` (Task 2).
- Produces: `pub(crate) fn code_span(s: &str) -> String` (returns the delimiters *and* content), `pub(crate) fn fenced_block(text: &str, lang: Option<&str>) -> String` (returns the whole block including its trailing newline).

- [ ] **Step 1: Write the failing tests**

Append to `mod tests`:

```rust
    #[test]
    fn code_span_picks_a_run_longer_than_anything_inside() {
        assert_eq!(code_span("plain"), "`plain`");
        assert_eq!(code_span("a ` b"), "`` a ` b ``");
        assert_eq!(code_span("a ``` b"), "```` a ``` b ````");
    }

    #[test]
    fn code_span_pads_when_the_content_touches_a_backtick() {
        // CommonMark strips exactly one space from each end, so the padding is
        // invisible in the rendered output.
        assert_eq!(code_span("`x"), "`` `x ``");
        assert_eq!(code_span("x`"), "`` x` ``");
    }

    #[test]
    fn code_span_handles_all_spaces_and_empty_content() {
        // All-spaces content receives no padding because CommonMark's carve-out
        // means "consists entirely of space characters" are not stripped, so the
        // input round-trips exactly.
        assert_eq!(code_span("  "), "`  `");
        // CommonMark cannot express an empty code span; a single space is the
        // closest thing, and P7 normalizes whitespace so it round-trips.
        assert_eq!(code_span(""), "` `");
    }

    #[test]
    fn code_span_preserves_spaces_at_both_ends() {
        // Content with space at both ends gets padded. CommonMark strips exactly
        // one space from each end (because the content begins and ends with space
        // but does not consist entirely of spaces), so the outer padding is
        // invisible and the input's own spaces survive the render.
        assert_eq!(code_span(" a "), "`  a  `");
    }

    #[test]
    fn code_span_folds_newlines_to_spaces() {
        // A blank line would end the enclosing paragraph.
        assert_eq!(code_span("a\nb"), "`a b`");
    }

    #[test]
    fn fenced_block_widens_past_any_run_inside() {
        assert_eq!(fenced_block("plain", None), "```\nplain\n```\n");
        assert_eq!(
            fenced_block("outer\n```\ninner", None),
            "````\nouter\n```\ninner\n````\n"
        );
    }

    #[test]
    fn fenced_block_sanitizes_the_info_string() {
        assert_eq!(fenced_block("x", Some("rust")), "```rust\nx\n```\n");
        // An info string is one token by grammar; a backtick in it breaks the
        // opening fence outright.
        assert_eq!(fenced_block("x", Some("rust ignore")), "```rust\nx\n```\n");
        assert_eq!(fenced_block("x", Some("ru`st")), "```rust\nx\n```\n");
        assert_eq!(fenced_block("x", Some("   ")), "```\nx\n```\n");
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kasane-writer escape 2>&1 | tail -20
```

Expected: FAIL — `cannot find function 'code_span'` / `'fenced_block'`.

- [ ] **Step 3: Write the implementation**

Append to `escape.rs` (above `mod tests`):

```rust
/// Wrap code content in a backtick run the content cannot contain (§3.4).
///
/// No escape exists inside a code span, so the delimiter is the only lever.
/// Newlines fold to spaces because a blank line would end the enclosing
/// paragraph.
pub(crate) fn code_span(s: &str) -> String {
    let content = one_line(s);
    let ticks = "`".repeat(longest_backtick_run(&content) + 1);
    let pad = if content.is_empty() {
        // Rule 1: Empty content gets a single space (only acknowledged divergence from round-trip).
        true
    } else if content.chars().all(|c| c == ' ') {
        // Rule 2: All-spaces content pads not at all; CommonMark's carve-out means the
        // content is not stripped, so no padding is needed to preserve input.
        false
    } else {
        // Rule 3: Pad if the content contains a backtick, or starts/ends with space.
        // For non-all-spaces content, CommonMark strips exactly one space from each end
        // iff it begins and ends with space, so padding is invisible in the render.
        content.contains('`') || content.starts_with(' ') || content.ends_with(' ')
    };
    if pad {
        format!("{ticks} {content} {ticks}")
    } else {
        format!("{ticks}{content}{ticks}")
    }
}

/// A whole fenced code block, trailing newline included (§3.4).
pub(crate) fn fenced_block(text: &str, lang: Option<&str>) -> String {
    let ticks = "`".repeat(longest_backtick_run(text).max(2) + 1);
    let info = lang.map(sanitize_info).unwrap_or_default();
    format!("{ticks}{info}\n{text}\n{ticks}\n")
}

fn longest_backtick_run(s: &str) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for c in s.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

/// An info string is a single token by grammar. Take the first whitespace-free
/// run and drop backticks, which would otherwise break the opening fence.
fn sanitize_info(lang: &str) -> String {
    lang.split_whitespace()
        .next()
        .unwrap_or_default()
        .replace('`', "")
}
```

- [ ] **Step 4: Run to verify pass**

```bash
cargo test -p kasane-writer escape 2>&1 | tail -20
```

Expected: PASS, 18 tests.

- [ ] **Step 5: Commit**

```bash
mise run lint
git add crates/kasane-writer/src/escape.rs
git commit -m "feat(writer): variable-length code spans and fences

Neither takes a backslash escape, so the delimiter is the only lever: a run one
longer than anything in the content, plus CommonMark's space padding when the
content touches a backtick. The info string is sanitized to one token because a
backtick or a space in it breaks the opening fence."
```

---

### Task 4: Destinations — `dest_path` and `dest_url`

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs`
- Test: same file

**Interfaces:**
- Consumes: nothing new.
- Produces: `pub(crate) fn dest_path(s: &str) -> String`, `pub(crate) fn dest_url(s: &str) -> String`.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests`:

```rust
    #[test]
    fn dest_path_encodes_percent_because_it_is_a_filename() {
        // A literal `%` in a filename would otherwise read back as an escape:
        // `50%20off` would decode to `50 off`, a directory that does not exist.
        assert_eq!(dest_path("50%20off"), "50%2520off");
        assert_eq!(dest_path("a b"), "a%20b");
        assert_eq!(dest_path("C# Notes"), "C%23%20Notes");
        assert_eq!(dest_path("a(b)c"), "a%28b%29c");
        assert_eq!(dest_path("a\nb"), "a%0Ab");
        // `/` stays literal: it is the path separator, not something to hide.
        assert_eq!(dest_path("a/b/index.md"), "a/b/index.md");
    }

    #[test]
    fn dest_url_leaves_percent_alone_because_it_is_already_a_url() {
        // Encoding `%` again would break every legitimately-encoded href.
        assert_eq!(
            dest_url("https://e.com/a%20b"),
            "https://e.com/a%20b"
        );
        assert_eq!(dest_url("https://e.com/a b"), "https://e.com/a%20b");
        assert_eq!(dest_url("https://e.com/x(1)"), "https://e.com/x%281%29");
        assert_eq!(dest_url("https://e.com/a\nb"), "https://e.com/a%0Ab");
        // A fragment and a query are meaningful in a URL and stay literal.
        assert_eq!(dest_url("https://e.com/p?q=1#f"), "https://e.com/p?q=1#f");
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kasane-writer escape 2>&1 | tail -20
```

Expected: FAIL — `cannot find function 'dest_path'` / `'dest_url'`.

- [ ] **Step 3: Write the implementation**

Append to `escape.rs`:

```rust
/// Percent-encode a destination kasane *constructs* from the filesystem:
/// `_assets/<filename>`, the library index's `rel_dir`, internal links (§3.5).
///
/// This is `library.rs`'s former `link_dest`, rule unchanged. `%` is encoded
/// because a literal `%` in a filename would otherwise read back as an escape;
/// `#` and `?` because they are fragment and query delimiters, so `C# Notes`
/// would otherwise emit a destination parsing as path `C` with a fragment; the
/// rest because they end or nest a bare destination. `/` stays literal — it is
/// the path separator.
pub(crate) fn dest_path(s: &str) -> String {
    encode(s, &['%', '#', '?', ' ', '(', ')', '<', '>', '\\', '"'])
}

/// Percent-encode a destination that arrived as a URL from a source document's
/// `href` (`RefTarget::External`) (§3.5).
///
/// The asymmetry with `dest_path` is the whole reason both exist: this input is
/// *already* percent-encoded, so encoding `%` again would turn every
/// legitimately-encoded link into a broken one. `#` and `?` stay literal too —
/// in a URL they are meaningful delimiters, not text. What is left is exactly
/// what ends or nests a bare Markdown destination.
pub(crate) fn dest_url(s: &str) -> String {
    encode(s, &[' ', '(', ')', '<', '>', '\\', '"'])
}

fn encode(s: &str, extra: &[char]) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if extra.contains(&c) || c.is_ascii_control() {
            for b in c.to_string().as_bytes() {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}
```

- [ ] **Step 4: Run to verify pass**

```bash
cargo test -p kasane-writer escape 2>&1 | tail -20
```

Expected: PASS, 19 tests.

- [ ] **Step 5: Commit**

```bash
mise run lint
git add crates/kasane-writer/src/escape.rs
git commit -m "fix(writer): split destination encoding into path and URL rules

library.rs's link_dest encodes % because a literal % in a filename would read
back as an escape. Applying that same rule to an external href would be wrong
in the opposite direction: an href arrives already percent-encoded, so encoding
% again breaks every legitimately-encoded link. Two functions, each naming the
other in its doc comment so a reader meets both at once."
```

---

### Task 5: `yaml_scalar`, and the frontmatter rewired onto it

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs`
- Modify: `crates/kasane-writer/src/frontmatter.rs:1-36` (replace `yaml_str`)
- Test: both files

**Interfaces:**
- Consumes: `one_line` (Task 2).
- Produces: `pub(crate) fn yaml_scalar(s: &str) -> String` — returns the scalar *including* its surrounding quotes.

- [ ] **Step 1: Write the failing tests**

Append to `escape.rs`'s `mod tests`:

```rust
    #[test]
    fn yaml_scalar_always_quotes() {
        assert_eq!(yaml_scalar("plain"), "\"plain\"");
        assert_eq!(yaml_scalar("a: b"), "\"a: b\"");
        // Each of these is a shape the old conditional quoting got wrong.
        assert_eq!(yaml_scalar("- dash"), "\"- dash\"");
        assert_eq!(yaml_scalar("[bracket"), "\"[bracket\"");
        assert_eq!(yaml_scalar("&anchor"), "\"&anchor\"");
        assert_eq!(yaml_scalar("*alias"), "\"*alias\"");
        assert_eq!(yaml_scalar("true"), "\"true\"");
        assert_eq!(yaml_scalar("null"), "\"null\"");
        assert_eq!(yaml_scalar("trailing "), "\"trailing \"");
    }

    #[test]
    fn yaml_scalar_escapes_quotes_and_backslashes_and_flattens() {
        assert_eq!(yaml_scalar("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(yaml_scalar("a\\b"), "\"a\\\\b\"");
        assert_eq!(yaml_scalar("a\nb"), "\"a b\"");
        // A double-quoted scalar cannot carry a raw control character, so it
        // folds to a space for the same reason a newline does -- folding
        // rather than dropping is what keeps two words from fusing.
        assert_eq!(yaml_scalar("a\u{7}b"), "\"a b\"");
        assert_eq!(yaml_scalar("cat\tnap"), "\"cat nap\"");
    }
```

Replace `frontmatter.rs`'s test-free body by adding this test module at the end of `crates/kasane-writer/src/frontmatter.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kasane_core::Frontmatter;

    fn fm(title: &str) -> Frontmatter {
        Frontmatter {
            title: title.to_string(),
            breadcrumb: vec!["Book".into(), title.to_string()],
            parent: Some("../index.md".into()),
            prev: None,
            next: None,
            children: vec!["01-a.md".into()],
            source_pages: Some((1, 2)),
        }
    }

    #[test]
    fn every_string_scalar_is_quoted() {
        let y = frontmatter_yaml(&fm("Notes: a study"));
        assert!(y.contains("title: \"Notes: a study\""), "{y}");
        assert!(y.contains("breadcrumb: \"Book > Notes: a study\""), "{y}");
        assert!(y.contains("parent: \"../index.md\""), "{y}");
        assert!(y.contains("  - \"01-a.md\""), "{y}");
        // Numeric, built by format! from two integers: never text, never quoted.
        assert!(y.contains("source_pages: 1-2"), "{y}");
    }

    #[test]
    fn a_title_that_would_break_the_block_is_contained() {
        let y = frontmatter_yaml(&fm("- \"quoted\"\nsecond"));
        assert!(y.contains("title: \"- \\\"quoted\\\" second\""), "{y}");
        assert_eq!(
            y.lines().filter(|l| l.starts_with("title:")).count(),
            1,
            "the title must not spill onto a second line: {y}"
        );
    }
}
```

Confirm `Frontmatter`'s field names and types before running — if any differ from the literal above, fix the test's constructor, not the assertions:

```bash
grep -n "pub struct Frontmatter" -A 12 crates/kasane-core/src/sitetree.rs
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kasane-writer escape 2>&1 | tail -20
cargo test -p kasane-writer frontmatter 2>&1 | tail -20
```

Expected: FAIL — `cannot find function 'yaml_scalar'`, and the frontmatter tests fail on unquoted scalars.

- [ ] **Step 3: Write the implementation**

Append to `escape.rs`:

```rust
/// A YAML double-quoted scalar, quotes included (§3.6).
///
/// Always quoted, rather than quoted-when-it-looks-risky. That deletes the
/// question of which characters require quoting — a question the superseded
/// `yaml_str` answered with `:` and `#`, and got wrong for a leading `-`, `[`,
/// `{`, `&`, `*`, `!`, `|`, `>`, `%`, `@`, a quote character, a trailing space,
/// and the bare words `true`, `null` and `~`, each of which YAML reads as
/// something other than a string. The cost is two bytes per line.
///
/// A double-quoted scalar cannot carry a raw control character (§4.1), so each
/// one folds to a space — the same treatment `one_line` already gives a
/// newline, and for the same reason: folding, not dropping, is what keeps two
/// words from silently fusing into one.
pub(crate) fn yaml_scalar(s: &str) -> String {
    let flat = one_line(s);
    let mut out = String::with_capacity(flat.len() + 2);
    out.push('"');
    for c in flat.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
```

Rewrite `crates/kasane-writer/src/frontmatter.rs`'s body (keep the test module added in Step 1):

```rust
use crate::escape::yaml_scalar;
use kasane_core::Frontmatter;

pub(crate) fn frontmatter_yaml(fm: &Frontmatter) -> String {
    let mut y = String::new();
    y.push_str(&format!("title: {}\n", yaml_scalar(&fm.title)));
    if !fm.breadcrumb.is_empty() {
        y.push_str(&format!(
            "breadcrumb: {}\n",
            yaml_scalar(&fm.breadcrumb.join(" > "))
        ));
    }
    if let Some(p) = &fm.parent {
        y.push_str(&format!("parent: {}\n", yaml_scalar(p)));
    }
    if let Some(p) = &fm.prev {
        y.push_str(&format!("prev: {}\n", yaml_scalar(p)));
    }
    if let Some(n) = &fm.next {
        y.push_str(&format!("next: {}\n", yaml_scalar(n)));
    }
    if !fm.children.is_empty() {
        y.push_str("children:\n");
        for c in &fm.children {
            y.push_str(&format!("  - {}\n", yaml_scalar(c)));
        }
    }
    if let Some((s, e)) = fm.source_pages {
        y.push_str(&format!("source_pages: {}-{}\n", s, e));
    }
    y
}
```

The path fields draw from the closed slug alphabet and do not need quoting, but a uniform block has one rule instead of five — and the closed-alphabet argument is one more thing that would have to stay true.

- [ ] **Step 4: Run to verify pass**

```bash
cargo test -p kasane-writer 2>&1 | tail -20
```

Expected: PASS. Other tests in the crate may now fail if they assert on unquoted frontmatter — check `crates/kasane-writer/src/lib.rs`'s tests and `crates/kasane-cli`'s integration tests:

```bash
cargo test --workspace 2>&1 | grep -E "^(test |failures:|---- )" | head -30
```

Any failure asserting `title: X` must be updated to `title: "X"`. That is the expected churn, not a regression.

- [ ] **Step 5: Commit**

```bash
mise run lint
git add crates/kasane-writer/src/escape.rs crates/kasane-writer/src/frontmatter.rs
git commit -m "fix(writer): always quote YAML scalars in the frontmatter

yaml_str quoted only on ':' or '#', which leaves a title starting with '-',
'[', '&', '*' or '!', ending in a space, containing a newline, or reading
'true' or 'null' able to parse as something other than its own text. Quoting
unconditionally deletes the question of which characters need it, for two bytes
a line. Every string scalar is covered, not only the title, so the block has one
rule instead of five."
```

---

### Task 6: Route the inline renderers through `escape`

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs:126-155` (`inlines_to_md`, `inlines_to_md_at`)
- Modify: `crates/kasane-writer/src/markdown.rs:27-90` (call sites pass `Ctx`)
- Modify: `crates/kasane-writer/src/escape.rs:5` (remove `#![allow(dead_code)]`)
- Test: `crates/kasane-writer/src/markdown.rs` tests

**Interfaces:**
- Consumes: `escape::{Ctx, text, code_span, dest_url, label, one_line}`.
- Produces: `pub(crate) fn inlines_to_md(inls: &[Inline], ctx: Ctx, at_line_start: bool) -> String` — **signature change**; every caller must name both a context and a line position. Task 8 adds `inlines_to_html`.

- [ ] **Step 1: Write the failing tests**

Append to `markdown.rs`'s `mod tests`:

```rust
    #[test]
    fn text_runs_are_escaped_but_markup_is_not() {
        let blocks = vec![Block::Para(vec![
            Inline::Text("a*b".into()),
            Inline::Strong(vec![Inline::Text("c[d".into())]),
        ])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        // The writer's own `**` survives; the document's `*` and `[` do not.
        assert!(md.contains("a\\*b**c\\[d**"), "got: {md}");
    }

    #[test]
    fn a_code_span_containing_a_backtick_does_not_break_out() {
        let blocks = vec![Block::Para(vec![Inline::Code("a ` b".into())])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("`` a ` b ``"), "got: {md}");
    }

    #[test]
    fn an_external_destination_is_encoded_and_its_label_escaped() {
        let blocks = vec![Block::Para(vec![Inline::Link {
            target: RefTarget::External("https://e.com/a b(1)".into()),
            inlines: vec![Inline::Text("see [this]".into())],
        }])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(
            md.contains("[see \\[this\\]](https://e.com/a%20b%281%29)"),
            "got: {md}"
        );
    }

    #[test]
    fn a_heading_stays_on_one_line() {
        let blocks = vec![Block::Heading {
            level: 2,
            id: BlockId(0),
            inlines: vec![Inline::Text("Title\nspilled".into())],
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("## Title spilled\n"), "got: {md}");
        assert_eq!(md.lines().filter(|l| l.starts_with("##")).count(), 1);
    }

    #[test]
    fn math_is_emitted_verbatim_but_a_dollar_in_prose_is_not() {
        let blocks = vec![Block::Para(vec![
            Inline::Math("x^2".into()),
            Inline::Text(" costs 5$".into()),
        ])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("$x^2$ costs 5\\$"), "got: {md}");
    }

    #[test]
    fn a_paragraph_beginning_with_a_line_start_character_is_escaped() {
        // A paragraph renders at column 0, so a leading LINE_START character
        // would otherwise open a different block (a bullet, a heading, a
        // blockquote, ...) instead of staying prose text.
        for c in ['#', '-', '+', '>', '=', '|'] {
            let blocks = vec![Block::Para(vec![Inline::Text(format!("{c} text"))])];
            let md = blocks_to_markdown(&blocks, &AssetBag::default());
            assert!(md.contains(&format!("\\{c} text")), "char {c:?}, got: {md}");
        }
    }

    #[test]
    fn a_paragraph_beginning_with_an_ordered_marker_is_escaped() {
        let blocks = vec![Block::Para(vec![Inline::Text("1. one".into())])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("1\\. one"), "got: {md}");
    }

    #[test]
    fn a_text_run_re_arms_line_start_after_an_interior_newline() {
        // The first `Inline::Text` ends in a newline, so the second run --
        // even though it is not the first inline in the paragraph -- really
        // does begin a line and must be escaped.
        let blocks = vec![Block::Para(vec![
            Inline::Text("a\n".into()),
            Inline::Text("- b".into()),
        ])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("a\n\\- b"), "got: {md}");
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kasane-writer markdown 2>&1 | tail -30
```

Expected: FAIL — the five original assertions, since nothing is escaped yet. The three line-start tests also fail: `inlines_to_md`'s `Text` arm hardcodes `at_line_start: false`, so a paragraph opening with `-`, `#`, or an ordered marker goes out unescaped and GFM re-parses it as a different block. That is the bug this task's `at_line_start` threading exists to close — see Step 3.

- [ ] **Step 3: Write the implementation**

In `markdown.rs`, replace `inlines_to_md` / `inlines_to_md_at` (lines 126-155) with:

```rust
pub(crate) fn inlines_to_md(inls: &[Inline], ctx: Ctx, at_line_start: bool) -> String {
    inlines_to_md_at(inls, 0, ctx, at_line_start)
}

/// `at_line_start` is threaded, not inferred: `true` iff the next character
/// emitted lands at the start of a line (design spec §2). It starts as
/// whatever the caller passed and is then recomputed after every arm from
/// whether the accumulated output ends with `\n` -- that single rule covers
/// both a run that opens on a fresh line and one that re-arms after an
/// interior newline (`[Text("a\n"), Text("- b")]`), with no per-arm special
/// case, because none of the writer's own markup (`*`, `**`, backticks,
/// `[`, `$`, `[^1]`) ever ends in a newline.
fn inlines_to_md_at(inls: &[Inline], depth: usize, ctx: Ctx, at_line_start: bool) -> String {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return String::new();
    }
    let mut s = String::new();
    let mut line_start = at_line_start;
    for i in inls {
        match i {
            // The only call to `escape::text` in the crate. Every other arm
            // below emits markup the writer chose, which must not be escaped.
            Inline::Text(t) => s.push_str(&escape::text(t, ctx, line_start)),
            Inline::Emph(x) => s.push_str(&format!(
                "*{}*",
                inlines_to_md_at(x, depth + 1, ctx, line_start)
            )),
            Inline::Strong(x) => s.push_str(&format!(
                "**{}**",
                inlines_to_md_at(x, depth + 1, ctx, line_start)
            )),
            Inline::Code(t) => s.push_str(&escape::code_span(t)),
            Inline::Math(t) => s.push_str(&format!("${}$", t)),
            Inline::Link {
                target: RefTarget::External(u),
                inlines,
            } => s.push_str(&format!(
                "[{}]({})",
                escape::one_line(&inlines_to_md_at(inlines, depth + 1, ctx, line_start)),
                escape::dest_url(u)
            )),
            // unresolved -> text
            Inline::Link { inlines, .. } => {
                s.push_str(&inlines_to_md_at(inlines, depth + 1, ctx, line_start))
            }
            Inline::FootnoteRef(n) => s.push_str(&format!("[^{}]", n.0)),
        }
        line_start = s.ends_with('\n');
    }
    s
}
```

Recursion into `Emph`, `Strong` and the `Link` arms passes the current `line_start` down unchanged, because their content is emitted at whatever position the parent had reached. The writer's own opening delimiters (`*`, `**`, `` ` ``) cannot themselves open a block construct at line start — a CommonMark bullet marker requires a following space — so passing the flag through this way is safe rather than over-escaping: at worst it adds a harmless backslash the render did not strictly need.

Add to the top of `markdown.rs`:

```rust
use crate::escape::{self, Ctx};
```

Update the block-level call sites in `render_block` to name both their context and their line position — tables become `Ctx::Cell`/`Ctx::Html` in Task 8, and Task 8's cell sites also start passing `true` for a cell's first character (design spec §3.2); until then everything below is `Ctx::Flow`:

- line 32, heading: `out.push_str(&escape::one_line(&inlines_to_md(inlines, Ctx::Flow, false)));` — `false`, because after `## ` the line has already started and a `-` there cannot open a list.
- line 36, para: `out.push_str(&inlines_to_md(inls, Ctx::Flow, true));` — `true`. This is the line-start fix: `blocks_to_markdown_at` renders a paragraph at column 0, and a list item or footnote body renders its inner blocks into a separate buffer at column 0 too (the marker is prefixed afterward), so a leading `-`, `#`, `>`, `+`, `=`, `|`, or ordered marker really would open a different block if left unescaped.
- lines 69 and 73, figure captions: `inlines_to_md(caption, Ctx::Flow, false)` — `false`; both sit after markup (`![` / `*Figure N: `) the writer already emitted on that line.
- lines 96, 113, table cells: `inlines_to_md(c, Ctx::Flow, false)` (Task 8 replaces these call sites entirely, and passes `true` there instead — see Task 8's Step 3)

One note on the heading arm carries over unchanged: `escape::one_line` is applied to the *rendered* string rather than to the inline text, which can leave a `\#` mid-title where the `#` followed an interior newline — harmless, since `\#` renders as `#`, and cheaper than a fold pass over `Vec<Inline>`.

Finally, remove `#![allow(dead_code)]` from `escape.rs` — the callers exist now. `dest_path`, `label` and `yaml_scalar` are still unused at this point; if clippy flags them, add `#[allow(dead_code)]` on those three items only, with a comment naming the task that consumes each (`dest_path`/`label` → Task 11, already used by `yaml_scalar` → Task 5 consumed it), and delete those attributes in Task 11.

- [ ] **Step 4: Run to verify pass**

```bash
cargo test -p kasane-writer markdown 2>&1 | tail -30
```

Expected: PASS for the eight new tests (the five from Step 1 plus the three line-start tests). The pre-existing `renders_headings_emphasis_and_links` still passes (its text has nothing to escape).

- [ ] **Step 5: Run the whole workspace**

```bash
cargo test --workspace 2>&1 | tail -20
```

Expected: PASS. `crates/kasane-cli`'s integration tests may assert on rendered text; any failure must be a *changed escape*, not lost content — inspect before editing.

- [ ] **Step 6: Commit**

```bash
mise run lint
git add crates/kasane-writer/src/markdown.rs crates/kasane-writer/src/escape.rs
git commit -m "fix(writer): escape inline text, code spans and external destinations

inlines_to_md takes Ctx as a required argument rather than a defaulted field.
That is the enforcement mechanism: a new Inline arm or a new caller cannot
inherit flow rules into a cell by omission, because it does not compile until
it names a context. Inline::Text becomes the only arm that reaches
escape::text; every other arm emits markup the writer chose, which must not be
escaped."
```

---

### Task 7: Block renderers — figures, code blocks, and `Raw` comments

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs:56-88` (figure, code block, raw)
- Modify: `crates/kasane-writer/src/lib.rs:25-37` (`file_to_markdown`'s title heading)
- Modify: `crates/kasane-writer/src/escape.rs` (add `comment_note`)
- Test: `crates/kasane-writer/src/markdown.rs`, `crates/kasane-writer/src/lib.rs`

**Interfaces:**
- Consumes: `escape::{Ctx, text, label, dest_path, fenced_block, one_line}`.
- Produces: `pub(crate) fn comment_note(s: &str) -> String` in `escape.rs`.

- [ ] **Step 1: Write the failing tests**

Append to `markdown.rs`'s `mod tests`:

```rust
    #[test]
    fn figure_alt_text_and_caption_are_escaped_and_the_asset_path_encoded() {
        let assets = AssetBag {
            items: vec![AssetItem {
                key: "k".into(),
                filename: "a b(1).png".into(),
                bytes: vec![],
            }],
        };
        let blocks = vec![Block::Figure {
            image: AssetRef {
                key: "k".into(),
                bytes_ref: 0,
            },
            caption: vec![Inline::Text("fig [1]".into())],
            number: Some("1".into()),
        }];
        let md = blocks_to_markdown(&blocks, &assets);
        assert!(
            md.contains("![fig \\[1\\]](_assets/a%20b%281%29.png)"),
            "got: {md}"
        );
        assert!(md.contains("*Figure 1: fig \\[1\\]*"), "got: {md}");
    }

    #[test]
    fn a_code_block_containing_a_fence_does_not_break_out() {
        let blocks = vec![Block::CodeBlock {
            lang: Some("rust ignore".into()),
            text: "outer\n```\ninner".into(),
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("````rust\nouter\n```\ninner\n````"), "got: {md}");
    }

    #[test]
    fn a_raw_note_cannot_close_its_own_comment() {
        let blocks = vec![Block::Raw {
            note: "a --> b -".into(),
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(!md.contains("--> b"), "the note closed the comment: {md}");
        assert!(md.starts_with("<!-- "), "got: {md}");
        assert!(md.trim_end().ends_with("-->"), "got: {md}");
    }
```

Append to `escape.rs`'s `mod tests`:

```rust
    #[test]
    fn comment_note_cannot_close_the_comment_early() {
        // The property, not a literal output string: wrap the note the same
        // way `markdown.rs`'s `Block::Raw` arm does, then require that the
        // only `-->` in the whole thing is the wrapper's own closer. A
        // one-shot `str::replace("--", "- -")` fails "--->" and "x--->y" --
        // it leaves an odd dash unreplaced right after a chunk ending in
        // `-`, and the two recombine.
        for note in ["--->", "x--->y", "-----", "----"] {
            let wrapped = format!("<!-- {} -->", comment_note(note));
            let body = wrapped
                .strip_suffix("-->")
                .expect("the wrapper always ends in the closer");
            assert!(
                !body.contains("-->"),
                "note {note:?} closed the comment early: {wrapped}"
            );
        }
    }
```

Append to `crates/kasane-writer/src/lib.rs` a test module (the file has none today):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kasane_core::Frontmatter;

    #[test]
    fn the_title_heading_is_escaped_and_kept_on_one_line() {
        let file = FileNode {
            path: "index.md".into(),
            frontmatter: Frontmatter {
                title: "# Notes\nspilled".into(),
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
        assert!(md.starts_with("# \\# Notes spilled\n"), "got: {md}");
    }
}
```

Confirm `FileNode`'s fields before running:

```bash
grep -n "pub struct FileNode" -A 8 crates/kasane-core/src/sitetree.rs
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kasane-writer 2>&1 | tail -30
```

Expected: FAIL on all five new tests.

- [ ] **Step 3: Write the implementation**

Add to `escape.rs`:

```rust
/// Make a note safe inside `<!-- ... -->` (§4.4).
///
/// A note containing `-->` closes the comment early, and one ending in `-`
/// leaves it malformed. Today's notes are internal fixed strings, so this is
/// defence in depth on a surface the rest of this module already covers.
///
/// A one-shot `str::replace("--", "- -")` is not enough: it matches
/// non-overlapping left-to-right, so an odd-length dash run leaves one dash
/// unreplaced right after a chunk that itself ends in `-`, and the two
/// recombine into a fresh `--` (`"--->"` -> `"- -->"`, which still closes the
/// comment). Walking the characters one at a time and inserting a space
/// whenever the character about to be pushed is `-` and the output so far
/// already ends in `-` considers the *output*, not the input, so a pair
/// created by an earlier insertion is caught too -- no run of two or more
/// dashes can ever survive into `out`.
pub(crate) fn comment_note(s: &str) -> String {
    let mut out = String::new();
    for c in one_line(s).chars() {
        if c == '-' && out.ends_with('-') {
            out.push(' ');
        }
        out.push(c);
    }
    if out.ends_with('-') {
        out.push(' ');
    }
    out
}
```

In `markdown.rs`, replace the `Block::Figure`, `Block::CodeBlock` and `Block::Raw` arms:

```rust
        Block::Figure {
            image,
            caption,
            number,
        } => {
            let fname = assets
                .items
                .iter()
                .find(|a| a.key == image.key)
                .map(|a| a.filename.as_str())
                .unwrap_or("missing");
            let alt = escape::one_line(&inlines_to_md(caption, Ctx::Flow, false));
            out.push_str(&format!(
                "![{}](_assets/{})\n",
                alt,
                escape::dest_path(fname)
            ));
            if let Some(n) = number {
                out.push_str(&format!("*Figure {}: {}*\n", n, alt));
            }
        }
        Block::CodeBlock { lang, text } => {
            out.push_str(&escape::fenced_block(text, lang.as_deref()));
        }
```

and

```rust
        Block::Raw { note } => out.push_str(&format!("<!-- {} -->\n", escape::comment_note(note))),
```

In `lib.rs`, `file_to_markdown`'s title heading (line 32) becomes:

```rust
    out.push_str(&escape::text(
        &escape::one_line(&file.frontmatter.title),
        escape::Ctx::Flow,
        true,
    ));
```

Folding *before* escaping here (the opposite order from the `Block::Heading` arm) is possible because the title is a plain `String`, and it avoids the stray `\#` that the other order leaves behind.

`at_line_start` is `true`, which is a deliberate over-escape rather than a necessity: the text follows `# ` on a line the writer already opened, so no block construct can form there and `false` would render identically. `true` is chosen because a leading `#` in a title then comes out as `\#`, which reads unambiguously in the source as *part of the title* rather than as a deeper heading level someone might later "fix". It costs one backslash and renders as `#` either way. The test above pins this spelling.

Add `use crate::escape;` to `lib.rs`'s imports.

- [ ] **Step 4: Run to verify pass**

```bash
cargo test -p kasane-writer 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
mise run lint
cargo test --workspace 2>&1 | tail -5
git add crates/kasane-writer/src/markdown.rs crates/kasane-writer/src/lib.rs crates/kasane-writer/src/escape.rs
git commit -m "fix(writer): escape figures, fences, comments and the title heading

A caption reaches the output twice (alt text and the visible Figure line) and
both were raw; an asset filename lands in a bare destination; a code block used
a fixed three-backtick fence a body containing one breaks out of; a Raw note
could close its own comment. The title heading folds newlines before escaping
rather than after, which the Block::Heading arm cannot do -- it has a String
where the arm has a Vec<Inline>."
```

---

### Task 8: Tables — GFM cells and the HTML fallback

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs:92-124` (`render_table`)
- Test: `crates/kasane-writer/src/markdown.rs`

**Interfaces:**
- Consumes: `escape::{Ctx, text}`, `inlines_to_md`.
- Produces: `fn inlines_to_html(inls: &[Inline], depth: usize) -> String` — private to `markdown.rs`.

- [ ] **Step 1: Write the failing tests**

Append to `markdown.rs`'s `mod tests`:

```rust
    #[test]
    fn a_pipe_in_a_cell_does_not_split_the_row() {
        let t = Table {
            header: vec![vec![Inline::Text("a|b".into())]],
            rows: vec![vec![vec![Inline::Text("c\nd".into())]]],
            has_merged: false,
        };
        let md = blocks_to_markdown(&[Block::Table(t)], &AssetBag::default());
        assert!(md.contains("| a\\|b |"), "got: {md}");
        assert!(md.contains("| c<br>d |"), "got: {md}");
    }

    #[test]
    fn the_merged_path_emits_html_markup_and_html_escaped_text() {
        // GFM parses nothing inside an HTML block, so `**bold**` there renders
        // with its asterisks showing. Emit tags instead.
        let t = Table {
            header: vec![vec![Inline::Text("a & b".into())]],
            rows: vec![vec![vec![
                Inline::Strong(vec![Inline::Text("bold".into())]),
                Inline::Text(" <x>".into()),
                Inline::Code("c<d".into()),
            ]]],
            has_merged: true,
        };
        let md = blocks_to_markdown(&[Block::Table(t)], &AssetBag::default());
        assert!(md.contains("<th>a &amp; b</th>"), "got: {md}");
        assert!(
            md.contains("<td><strong>bold</strong> &lt;x&gt;<code>c&lt;d</code></td>"),
            "got: {md}"
        );
        assert!(!md.contains("**bold**"), "markdown markup leaked: {md}");
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kasane-writer markdown 2>&1 | tail -30
```

Expected: FAIL on both.

- [ ] **Step 3: Write the implementation**

Replace `render_table` in `markdown.rs`:

```rust
fn render_table(t: &Table, out: &mut String) {
    if t.has_merged {
        // An HTML block's content is raw: GFM parses no Markdown inside it, so
        // the inlines must be emitted as HTML tags and the text HTML-escaped.
        // Emitting `**bold**` here (as this branch did) renders with the
        // asterisks showing (§3.3).
        out.push_str("<table>\n");
        out.push_str("<tr>");
        for c in &t.header {
            out.push_str(&format!("<th>{}</th>", inlines_to_html(c, 0)));
        }
        out.push_str("</tr>\n");
        for r in &t.rows {
            out.push_str("<tr>");
            for c in r {
                out.push_str(&format!("<td>{}</td>", inlines_to_html(c, 0)));
            }
            out.push_str("</tr>\n");
        }
        out.push_str("</table>\n");
        return;
    }
    let cells = |row: &Vec<Vec<Inline>>| {
        let joined: Vec<String> = row
            .iter()
            .map(|c| inlines_to_md(c, Ctx::Cell, true))
            .collect();
        format!("| {} |", joined.join(" | "))
    };
    out.push_str(&cells(&t.header));
    out.push('\n');
    let sep: Vec<&str> = t.header.iter().map(|_| "---").collect();
    out.push_str(&format!("| {} |\n", sep.join(" | ")));
    for r in &t.rows {
        out.push_str(&cells(r));
        out.push('\n');
    }
}

/// Render inlines as HTML, for the merged-table fallback only.
///
/// Math is the one inline with no HTML spelling here: `$…$` is not parsed
/// inside an HTML block either, so a merged-cell equation degrades to its
/// literal LaTeX. That renders no worse than today, and the alternative is
/// emitting MathML the IR does not carry (§3.3).
fn inlines_to_html(inls: &[Inline], depth: usize) -> String {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return String::new();
    }
    let mut s = String::new();
    for i in inls {
        match i {
            Inline::Text(t) => s.push_str(&escape::text(t, Ctx::Html, false)),
            Inline::Emph(x) => s.push_str(&format!("<em>{}</em>", inlines_to_html(x, depth + 1))),
            Inline::Strong(x) => {
                s.push_str(&format!("<strong>{}</strong>", inlines_to_html(x, depth + 1)))
            }
            Inline::Code(t) => s.push_str(&format!(
                "<code>{}</code>",
                escape::text(t, Ctx::Html, false)
            )),
            Inline::Math(t) => s.push_str(&format!("${}$", escape::text(t, Ctx::Html, false))),
            Inline::Link {
                target: RefTarget::External(u),
                inlines,
            } => s.push_str(&format!(
                "<a href=\"{}\">{}</a>",
                escape::text(&escape::dest_url(u), Ctx::Html, false),
                inlines_to_html(inlines, depth + 1)
            )),
            Inline::Link { inlines, .. } => s.push_str(&inlines_to_html(inlines, depth + 1)),
            Inline::FootnoteRef(n) => s.push_str(&format!("[^{}]", n.0)),
        }
    }
    s
}
```

- [ ] **Step 4: Run to verify pass**

```bash
cargo test -p kasane-writer markdown 2>&1 | tail -20
```

Expected: PASS, including the pre-existing `renders_gfm_table`.

- [ ] **Step 5: Commit**

```bash
mise run lint
git add crates/kasane-writer/src/markdown.rs
git commit -m "fix(writer): escape table cells and emit real HTML in the merged path

A pipe in a cell split the row and misaligned every row after it. The merged
path had a second, quieter bug the escaping work exposed: GFM parses nothing
inside an HTML block, so the '**bold**' this branch emitted rendered with its
asterisks showing. Cells now carry <br> for newlines and the merged path emits
<strong>/<em>/<code>/<a> with HTML-escaped text."
```

---

### Task 9: Footnote continuation indent

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs:84-87` (`Block::Footnote`)
- Test: `crates/kasane-writer/src/markdown.rs`

**Interfaces:**
- Consumes: `blocks_to_markdown_at`.
- Produces: `fn indent_continuation(body: &str, indent: &str) -> String` — private to `markdown.rs`, reused by Task 10.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_multi_block_footnote_body_stays_inside_its_definition() {
        let blocks = vec![Block::Footnote {
            id: kasane_ir::NoteId(1),
            blocks: vec![
                Block::Para(vec![Inline::Text("first".into())]),
                Block::Para(vec![Inline::Text("second".into())]),
            ],
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        let lines: Vec<&str> = md.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines[0], "[^1]: first", "got: {md}");
        assert_eq!(
            lines[1], "    second",
            "the second block escaped the definition: {md}"
        );
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kasane-writer a_multi_block_footnote 2>&1 | tail -20
```

Expected: FAIL — `lines[1]` is `second`, at column zero, outside the definition.

- [ ] **Step 3: Write the implementation**

Replace the `Block::Footnote` arm:

```rust
        Block::Footnote { id, blocks } => {
            // Four spaces is GFM's footnote continuation indent. Without it a
            // body of more than one line puts its second line at column zero,
            // outside the definition, where it becomes a sibling paragraph
            // (§4.2).
            let body = blocks_to_markdown_at(blocks, assets, depth + 1);
            let body = body.trim();
            out.push_str(&format!("[^{}]: {}\n", id.0, indent_continuation(body, "    ")));
        }
```

and add:

```rust
/// Indent every line after the first by `indent`, leaving blank lines blank so
/// no line carries trailing whitespace.
fn indent_continuation(body: &str, indent: &str) -> String {
    let mut lines = body.lines();
    let mut out = String::new();
    if let Some(first) = lines.next() {
        out.push_str(first);
    }
    for line in lines {
        out.push('\n');
        if !line.trim().is_empty() {
            out.push_str(indent);
            out.push_str(line);
        }
    }
    out
}
```

- [ ] **Step 4: Run to verify pass**

```bash
cargo test -p kasane-writer 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
mise run lint
git add crates/kasane-writer/src/markdown.rs
git commit -m "fix(writer): keep a multi-block footnote body inside its definition

body.trim() put every line after the first at column zero, where GFM reads it
as a sibling paragraph rather than as part of the note. Four spaces is the
continuation indent; blank lines stay blank so no line carries trailing
whitespace."
```

---

### Task 10: List-item continuation indent

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs:39-54` (`Block::List`)
- Test: `crates/kasane-writer/src/markdown.rs`

**Interfaces:**
- Consumes: `indent_continuation` (Task 9).
- Produces: nothing new.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn a_multi_block_list_item_stays_inside_its_item() {
        let blocks = vec![Block::List {
            ordered: false,
            items: vec![vec![
                Block::Para(vec![Inline::Text("first".into())]),
                Block::List {
                    ordered: false,
                    items: vec![vec![Block::Para(vec![Inline::Text("nested".into())])]],
                },
            ]],
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        let lines: Vec<&str> = md.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines[0], "- first", "got: {md}");
        assert_eq!(lines[1], "  - nested", "got: {md}");
    }

    #[test]
    fn an_ordered_item_indents_by_its_own_marker_width() {
        let blocks = vec![Block::List {
            ordered: true,
            items: vec![vec![
                Block::Para(vec![Inline::Text("a".into())]),
                Block::Para(vec![Inline::Text("b".into())]),
            ]],
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        let lines: Vec<&str> = md.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines[0], "1. a", "got: {md}");
        assert_eq!(lines[1], "   b", "got: {md}");
    }

    #[test]
    fn a_heading_leading_a_list_item_still_renders_on_the_marker_line() {
        // properties.rs's strip_list_markers depends on this shape.
        let blocks = vec![Block::List {
            ordered: false,
            items: vec![vec![Block::Heading {
                level: 2,
                id: BlockId(0),
                inlines: vec![Inline::Text("Notes".into())],
            }]],
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("- ## Notes"), "got: {md}");
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kasane-writer list_item 2>&1 | tail -20
```

Expected: FAIL — the nested item renders as `- - nested` on one line today (first test), and the ordered item's second paragraph sits at column zero (second test). The third test passes already and must keep passing.

- [ ] **Step 3: Write the implementation**

Replace the `Block::List` arm:

```rust
        Block::List { ordered, items } => {
            for (i, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}. ", i + 1)
                } else {
                    "- ".to_string()
                };
                let mut inner = String::new();
                for bb in item {
                    render_block(bb, assets, &mut inner, depth + 1);
                }
                // Continuation lines are indented by the marker's own width;
                // without it an item holding a paragraph and a nested list
                // drops to column zero on its second line and leaves the item
                // (§4.3). The first block still renders on the marker's line,
                // which is what keeps `- ## Notes` intact.
                let indent = " ".repeat(marker.chars().count());
                out.push_str(&marker);
                out.push_str(&indent_continuation(inner.trim_end(), &indent));
                out.push('\n');
            }
        }
```

- [ ] **Step 4: Run to verify pass**

```bash
cargo test -p kasane-writer 2>&1 | tail -20
```

Expected: PASS. `rendering_survives_deep_block_nesting` and `rendering_preserves_content_well_under_the_block_bound` must still pass — the second builds a 10-deep nest whose payload now sits under 20 spaces of indent, which does not change `md.contains("innermost payload")`.

- [ ] **Step 5: Commit**

```bash
mise run lint
cargo test --workspace 2>&1 | tail -5
git add crates/kasane-writer/src/markdown.rs
git commit -m "fix(writer): indent list-item continuation lines

Same shape of bug as the footnote body: the item's whole rendered body was
appended after the marker, so an item holding a paragraph and a nested list
dropped to column zero on line two and left the item. The marker line is
unchanged, so a heading leading an item still renders as '- ## Notes' and the
property tier's marker stripping keeps working."
```

---

### Task 11: Fold `library.rs` onto the shared module

**Files:**
- Modify: `crates/kasane-writer/src/library.rs:52-58,78-122` (call sites; delete `link_text`, `link_dest`)
- Modify: `crates/kasane-writer/src/escape.rs` (drop any leftover `#[allow(dead_code)]`)
- Test: `crates/kasane-writer/src/library.rs`

**Interfaces:**
- Consumes: `escape::{label, dest_path, one_line}`.
- Produces: nothing new. `library.rs` keeps only its own `one_line` usage via `escape::one_line`.

- [ ] **Step 1: Write the failing tests**

Append to `library.rs`'s `mod tests`:

```rust
    #[test]
    fn a_title_with_markdown_in_it_is_escaped_not_substituted() {
        // link_text replaced `[` with `(`, which changes the rendered text.
        // Section 5 forbids that: anchors are computed from unescaped IR text.
        let md = write(&[entry("A [b] *c*", "a/x")], &[]);
        assert!(md.contains("- [A \\[b\\] \\*c\\*](a/x/index.md)"), "got: {md}");
    }

    #[test]
    fn a_failure_reason_is_escaped_inside_its_backticks() {
        let md = write(
            &[],
            &[LibraryFailure {
                input: "c/d`e.azw3".into(),
                reason: "bad [thing]\nsecond line".into(),
            }],
        );
        assert!(md.contains("bad \\[thing\\] second line"), "got: {md}");
        assert!(
            md.lines().filter(|l| l.starts_with("- ")).count() == 1,
            "the reason spilled onto another line: {md}"
        );
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kasane-writer library 2>&1 | tail -20
```

Expected: FAIL — the first gets `A (b) *c*` (substituted, unescaped), the second gets an unescaped `[thing]`.

- [ ] **Step 3: Write the implementation**

In `library.rs`: delete `link_text` and `link_dest` entirely, add `use crate::escape;` at the top, and rewire:

```rust
        s.push_str(&format!(
            "- [{}]({}/index.md) — {}, {} files\n",
            escape::label(title),
            escape::dest_path(&e.rel_dir),
            e.format,
            e.files
        ));
```

and the failures loop:

```rust
        for f in failures {
            s.push_str(&format!(
                "- `{}` — {}\n",
                escape::code_span(&f.input).trim_matches('`'),
                escape::label(&f.reason)
            ));
        }
```

That `trim_matches` is wrong — it re-opens exactly the breakout `code_span` exists to close. Write it as the code span itself, with no surrounding backticks in the format string:

```rust
        for f in failures {
            s.push_str(&format!(
                "- {} — {}\n",
                escape::code_span(&f.input),
                escape::label(&f.reason)
            ));
        }
```

Delete `library.rs`'s local `one_line` (its only remaining caller is the failure loop above, which now goes through `escape::label`).

Update `write_library_index`'s doc comment, which currently says titles are neutralized by `link_text`:

```rust
/// Write `<out>/index.md`: the entry point for a batch run.
///
/// Written even when every document failed, so a failed run leaves an on-disk
/// record rather than only a stderr trace. The frontmatter holds no free text —
/// only `kind` and two counts — so no YAML quoting is needed; titles appear
/// solely as link labels, where `escape::label` handles them under the same
/// policy as the rest of the writer.
```

Then remove any `#[allow(dead_code)]` attributes left on `escape.rs` items from Task 6 — every function now has a caller.

- [ ] **Step 4: Run to verify pass**

```bash
cargo test -p kasane-writer 2>&1 | tail -20
```

Expected: PASS. `lists_failures_with_their_reason` asserts `- \`c/drm.azw3\` — DRM-protected, unsupported`; `code_span` renders that input as `` `c/drm.azw3` ``, so the assertion still holds.

- [ ] **Step 5: Commit**

```bash
mise run lint
git add crates/kasane-writer/src/library.rs crates/kasane-writer/src/escape.rs
git commit -m "refactor(writer): fold the library index onto the shared escape module

link_text and link_dest were the only escaping in the crate and its doc comment
called itself a stopgap for this item. link_text also substituted rather than
escaped -- '[' became '(' -- which changes the rendered text and so could never
have been the repo-wide rule. A failing input now goes through code_span, so a
backtick in a filename cannot break out of its own span."
```

---

### Task 12: Widen the generator's alphabet

**Files:**
- Modify: `crates/kasane-writer/tests/generator/mod.rs:41-47,49-80,133-231,233-287`
- Test: `crates/kasane-writer/tests/generator_smoke.rs` (existing smoke test must stay green)

**Interfaces:**
- Consumes: nothing new.
- Produces: `pub struct Sentinel { pub token: String, pub payload: String, pub expect: Expect }` — **field added**; `payload` is the full text stamped into the block (token + hostile suffix), `token` stays the bare `zq####` P1 counts.

- [ ] **Step 1: Write the failing test**

Append to `crates/kasane-writer/tests/generator_smoke.rs`:

```rust
#[test]
fn some_generated_case_carries_markdown_hostile_text() {
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;

    let mut runner = TestRunner::deterministic();
    let mut saw_hostile = false;
    for _ in 0..200 {
        let case = generator::case().new_tree(&mut runner).unwrap().current();
        if case
            .sentinels
            .iter()
            .any(|s| s.payload.contains(['*', '|', '[', '`', '#', '\\']))
        {
            saw_hostile = true;
            break;
        }
    }
    assert!(
        saw_hostile,
        "200 draws produced no Markdown-hostile payload; the widening is not reaching the sentinels"
    );
}
```

Check the existing file's `mod generator;` declaration and imports first:

```bash
cat crates/kasane-writer/tests/generator_smoke.rs
```

If it does not already declare `mod generator;`, add it at the top.

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p kasane-writer --test generator_smoke 2>&1 | tail -20
```

Expected: FAIL — `no field 'payload' on type 'Sentinel'`.

- [ ] **Step 3: Write the implementation**

In `crates/kasane-writer/tests/generator/mod.rs`:

Add the hostile alphabet below `WORDS`:

```rust
/// Markdown-hostile fragments, drawn into the same text `WORDS` feeds.
///
/// Every one of these renders as markup, or breaks a container, if the writer
/// emits it unescaped: the inline openers, the line-start block openers, a
/// pipe that splits a cell, a fence and a backtick that break out of code, an
/// entity that would decode, an HTML comment closer, and an embedded newline.
/// `zq` is deliberately absent, for the same reason `WORDS` avoids it — a
/// fragment containing the sentinel prefix would corrupt P1's counting.
const HOSTILE: &[&str] = &[
    "*star*",
    "_under_",
    "[bracket]",
    "]close",
    "`tick`",
    "```fence",
    "<html>",
    "&amp;",
    "&raw",
    "$math$",
    "~~strike~~",
    "back\\slash",
    "-->",
    "|pipe|",
    "#hash",
    "- bullet",
    "1. ordered",
    "> quote",
    "= setext",
    "line\nbreak",
    "!bang[",
];

/// Filler text, with hostile fragments mixed in often enough that a case
/// without one is rare.
fn filler() -> impl Strategy<Value = String> {
    let word = prop_oneof![
        3 => proptest::sample::select(WORDS),
        1 => proptest::sample::select(HOSTILE),
    ];
    proptest::collection::vec(word, 1..12).prop_map(|ws| ws.join(" "))
}
```

Add `payload` to `Sentinel`:

```rust
#[derive(Clone, Debug)]
pub struct Sentinel {
    /// The bare `zq####` token. Alphanumeric, so no escape can appear inside
    /// it and P1's raw-text counting is unaffected by the escaping policy.
    pub token: String,
    /// The full text stamped into the block: the token plus a hostile suffix.
    /// P7 counts *this* in the re-parsed text, which is what makes a missed
    /// escape a failure rather than a curiosity.
    pub payload: String,
    pub expect: Expect,
}
```

Change `build`'s signature to take the payload and stamp it in place of the bare token — replace every `text(token)` and `token.to_string()` with `text(payload)` / `payload.to_string()`:

```rust
fn build(shape: &Shape, deco: &[Inline], payload: &str, idx: u32) -> (Block, Expect) {
```

(The body is otherwise unchanged: every `token` identifier becomes `payload`.)

In `case()`, build the payload and record both strings:

```rust
        for (i, (sh, deco, hostile)) in shapes.iter().enumerate() {
            let idx = i as u32;
            let token = format!("zq{:04}", idx);
            let payload = format!("{token} {hostile}");
            let (block, expect) = build(sh, deco, &payload, idx);
            ...
            sentinels.push(Sentinel { token, payload, expect });
        }
```

and widen the `shapes` strategy to carry the hostile fragment:

```rust
    let shapes = proptest::collection::vec(
        (shape(), inlines(3), proptest::sample::select(HOSTILE)),
        1..40,
    );
```

Two shapes need care and neither changes here — note them in a comment above `build`:

- `Shape::Code` puts the payload inside a code block, which is where ````fence` and `` `tick` `` do their work against `escape::fenced_block`.
- `Shape::Raw` puts it inside an HTML comment, which is where `-->` works against `escape::comment_note`.

- [ ] **Step 4: Run to verify pass**

```bash
cargo test -p kasane-writer --test generator_smoke 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 5: Run the property suite and expect real failures**

```bash
cargo test -p kasane-writer --test properties 2>&1 | tail -40
```

Expected: **FAIL**, most likely P1 (`links_in`/`heading_anchors` mis-parsing hostile text) and possibly P2. This is the debt those helpers' doc comments predicted, and Task 13 pays it. Do **not** narrow the generator to make this green. If a failure looks like a *writer* bug rather than a helper bug — content genuinely lost, not merely miscounted — stop and report it before proceeding.

- [ ] **Step 6: Commit**

```bash
mise run lint
git add crates/kasane-writer/tests/generator/mod.rs crates/kasane-writer/tests/generator_smoke.rs
git commit -m "test(writer): draw Markdown-hostile text in the generator

Twenty-one fragments that each render as markup, or break a container, if the
writer emits them unescaped, mixed into the same text WORDS feeds -- so titles,
cells, captions, code bodies and Raw notes all carry them. Sentinel gains a
payload field: token stays the bare zq#### P1 counts in raw Markdown, payload
is the full string P7 will count in re-parsed text.

The property suite is expected to fail at this commit: links_in and
heading_anchors are correct only under the old closed alphabet, which their own
doc comments say. The next commit rebuilds them on a real parser."
```

---

### Task 13: Rebuild the helpers on a parser, and add P7

**Files:**
- Modify: `crates/kasane-writer/Cargo.toml:15-17` (dev-dependency)
- Modify: `crates/kasane-writer/tests/properties.rs:53-157` (helpers), and append P7
- Test: `crates/kasane-writer/tests/properties.rs`

**Interfaces:**
- Consumes: `generator::Sentinel { token, payload, expect }` (Task 12).
- Produces: `fn parse_events(md: &str) -> Parsed` where `struct Parsed { text: String, headings: Vec<String>, links: Vec<String>, footnote_defs: usize, table_rows: usize }`.

- [ ] **Step 1: Add the dev-dependency**

```bash
cargo add --package kasane-writer --dev pulldown-cmark@0.13
cargo tree -p kasane-writer --edges normal 2>&1 | grep -c pulldown
```

Expected: `cargo add` succeeds; the `grep -c` prints `0`, confirming `pulldown-cmark` is **not** a normal dependency. If it prints anything else, it landed in the wrong table — move it to `[dev-dependencies]`.

- [ ] **Step 2: Write the failing test**

Append to `properties.rs`, inside the existing `proptest! { ... }` block:

```rust
    /// P7 — Round trip. Every generated payload survives escaping, verbatim,
    /// into the text a real GFM parser recovers from the rendered file.
    ///
    /// This is the check a case table cannot make. The table pins kasane's
    /// reading of CommonMark; this pins the reading against an implementation
    /// of it (design spec §6.2). A missed escape shows up here as a payload
    /// that came back changed, or did not come back at all.
    #[test]
    fn p7_round_trip(case in generator::case()) {
        let files = render(&case);
        let recovered: String = files
            .iter()
            .map(|(_, t, _)| normalize_ws(&parse_events(t).text))
            .collect::<Vec<_>>()
            .join(" ");
        for s in &case.sentinels {
            let needle = normalize_ws(&s.payload);
            let n = recovered.matches(&needle).count();
            match s.expect {
                Expect::Exactly(k) => prop_assert_eq!(
                    n, k,
                    "payload {:?} survived {} times in parsed text, expected exactly {}",
                    s.payload, n, k
                ),
                Expect::AtLeast(k) => prop_assert!(
                    n >= k,
                    "payload {:?} survived {} times in parsed text, expected at least {}",
                    s.payload, n, k
                ),
            }
        }
    }

    /// P8 — Structure. The parsed block structure matches the IR that produced
    /// it: one footnote definition per `Block::Footnote`, one heading per
    /// heading (plus the file's own title), and a full grid per GFM table.
    #[test]
    fn p8_structure_survives(case in generator::case()) {
        for (path, text, f) in render(&case) {
            let parsed = parse_events(&text);
            let want_notes = count_blocks(&f.blocks, |b| matches!(b, Block::Footnote { .. }));
            prop_assert_eq!(
                parsed.footnote_defs, want_notes,
                "{}: {} footnote definitions parsed, {} in the IR",
                path, parsed.footnote_defs, want_notes
            );
            let want_headings =
                count_blocks(&f.blocks, |b| matches!(b, Block::Heading { .. })) + 1; // + the title
            prop_assert_eq!(
                parsed.headings.len(), want_headings,
                "{}: {} headings parsed, {} expected",
                path, parsed.headings.len(), want_headings
            );
            let want_rows: usize = sum_blocks(&f.blocks, |b| match b {
                Block::Table(t) if !t.has_merged => 1 + t.rows.len(),
                _ => 0,
            });
            prop_assert_eq!(
                parsed.table_rows, want_rows,
                "{}: {} table rows parsed, {} expected",
                path, parsed.table_rows, want_rows
            );
        }
    }
```

and, above the `proptest!` block, the helpers:

```rust
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// What a real GFM parser recovers from a rendered file.
struct Parsed {
    /// Concatenated text, code and stripped inline HTML, in document order.
    text: String,
    /// Each heading's text, in render order.
    headings: Vec<String>,
    /// Every link destination.
    links: Vec<String>,
    footnote_defs: usize,
    table_rows: usize,
}

/// Parse with exactly the GFM extensions kasane emits.
///
/// Math stays **off**: kasane emits `$…$` deliberately, and with the extension
/// off it arrives as literal text — which is also what an escaped `\$` in prose
/// arrives as, so both sides of P7 agree without a special case.
fn parse_events(md: &str) -> Parsed {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);

    let mut p = Parsed {
        text: String::new(),
        headings: Vec::new(),
        links: Vec::new(),
        footnote_defs: 0,
        table_rows: 0,
    };
    let mut heading_depth = 0usize;
    let mut heading = String::new();

    for ev in Parser::new_ext(md, opts) {
        match ev {
            Event::Text(t) | Event::Code(t) => {
                p.text.push_str(&t);
                if heading_depth > 0 {
                    heading.push_str(&t);
                }
            }
            // An HTML block or inline tag: keep its text, drop its markup. The
            // merged-table path and `<br>` in cells both arrive this way.
            Event::Html(h) | Event::InlineHtml(h) => {
                let stripped = strip_tags(&h);
                p.text.push(' ');
                p.text.push_str(&stripped);
                p.text.push(' ');
                if heading_depth > 0 {
                    heading.push_str(&stripped);
                }
            }
            Event::SoftBreak | Event::HardBreak => p.text.push(' '),
            Event::Start(Tag::Heading { .. }) => {
                heading_depth += 1;
                heading.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                heading_depth = heading_depth.saturating_sub(1);
                p.headings.push(heading.trim().to_string());
            }
            Event::Start(Tag::Link { dest_url, .. }) => p.links.push(dest_url.to_string()),
            Event::Start(Tag::FootnoteDefinition(_)) => p.footnote_defs += 1,
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => p.table_rows += 1,
            Event::End(TagEnd::Paragraph) => p.text.push(' '),
            _ => {}
        }
    }
    p
}

/// Drop HTML tags and decode the four entities `escape::text(_, Ctx::Html, _)`
/// produces, so an HTML block's text can be compared with the IR's.
fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

/// Collapse whitespace runs and trim, on both sides of every comparison.
///
/// Markdown normalizes whitespace at line boundaries — a soft break is a
/// newline in the source and a space in the render — so an exact comparison
/// would fail on formatting rather than on escaping. The cost is that a lost
/// *space* is invisible to P7; every other character is not.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Count blocks matching `pred`, recursing into lists and footnotes.
fn count_blocks(blocks: &[Block], pred: fn(&Block) -> bool) -> usize {
    sum_blocks(blocks, move |b| usize::from(pred(b)))
}

fn sum_blocks<F: Fn(&Block) -> usize + Copy>(blocks: &[Block], f: F) -> usize {
    let mut total = 0;
    for b in blocks {
        total += f(b);
        match b {
            Block::List { items, .. } => {
                for item in items {
                    total += sum_blocks(item, f);
                }
            }
            Block::Footnote { blocks, .. } => total += sum_blocks(blocks, f),
            _ => {}
        }
    }
    total
}
```

- [ ] **Step 3: Run to verify failure**

```bash
cargo test -p kasane-writer --test properties 2>&1 | tail -40
```

Expected: FAIL — P1/P2 still failing from Task 12, and P7/P8 now compiling and reporting real results.

- [ ] **Step 4: Rebuild `links_in` and `heading_anchors` on the parser**

Delete `links_in`, `strip_list_markers` and `heading_anchors` (`properties.rs:53-157`) and replace with:

```rust
/// Every link destination in a rendered file, as a real parser sees them.
///
/// Superseded a hand-rolled `](`-scanner whose own doc comment named its
/// limit: it collected a false link from a fenced code block, and a false
/// positive here is one more target P2 demands resolve — a false failure, not
/// a lenient check. The generator now draws bracket-bearing text into code
/// blocks, so that limit is reached on the first hostile draw.
fn links_in(text: &str) -> Vec<String> {
    parse_events(text).links
}

/// Every heading's anchor, as the engine would compute it for this file.
///
/// Also superseded a line scanner. It stripped list markers by hand to see a
/// heading leading a list item, and stripped `*` and `` ` `` to undo the
/// writer's own markup — both of which a parser does correctly and neither of
/// which survives contact with hostile text, since a paragraph beginning with
/// `#` looked like a heading to it and consumed a duplicate-suffix slot.
///
/// Order still matters: duplicate anchors are suffixed per file in render
/// order, so the whole ordered list goes to the engine's own counter rather
/// than each line being slugged independently.
fn heading_anchors(text: &str) -> HashSet<String> {
    anchors_for_headings(&parse_events(text).headings)
        .into_iter()
        .collect()
}
```

- [ ] **Step 5: Run the full property suite**

```bash
cargo test -p kasane-writer --test properties 2>&1 | tail -40
```

Expected: PASS, eight properties. A failure writes `crates/kasane-writer/tests/properties.proptest-regressions` — **commit that file** if it appears; it is what turns a found case into a permanent regression test. If a property fails, read the shrunk case before touching anything: P7 failing means a real missed escape, and the fix belongs in `escape.rs`, not in the test.

- [ ] **Step 6: Commit**

```bash
mise run lint
cargo test --workspace 2>&1 | tail -5
git add crates/kasane-writer/Cargo.toml Cargo.lock crates/kasane-writer/tests/properties.rs
git commit -m "test(writer): round-trip property over a real GFM parser

P7 counts each generated payload in the text pulldown-cmark recovers from the
rendered file, against the same ledger P1 counts in raw Markdown; P8 checks the
block structure came back. This is the check a case table cannot make -- the
table pins kasane's reading of CommonMark, this pins that reading against an
implementation of it.

links_in and heading_anchors move onto the parser, which is the debt their own
doc comments recorded: both were correct only under the old closed alphabet,
and each error ran in the unsafe direction, producing a false failure rather
than a lenient check. pulldown-cmark is a dev-dependency only."
```

---

### Task 14: The fuzz seam, target, and stable replay

**Files:**
- Create: `crates/kasane-writer/src/fuzz_entry.rs`
- Create: `fuzz/fuzz_targets/escape.rs`
- Create: `fuzz/seeds/escape/{inline_openers,line_starts,fences,entities,yaml}.txt`
- Modify: `crates/kasane-writer/src/lib.rs` (declare the module)
- Modify: `crates/kasane-writer/src/escape.rs` (make items reachable from the seam)
- Modify: `fuzz/Cargo.toml` (dependency + `[[bin]]`)
- Modify: `crates/kasane-adapters/Cargo.toml` (dev-dependency on `kasane-writer`)
- Modify: `crates/kasane-adapters/tests/fuzz_corpus.rs:22-40` (`target()`, `TARGET_COUNT`)
- Modify: `mise.toml` (`fuzz-all` list)

**Interfaces:**
- Consumes: every `escape` function.
- Produces: `pub fn escape(data: &[u8])` in `kasane_writer::fuzz_entry`.

- [ ] **Step 1: Write the seam**

Create `crates/kasane-writer/src/fuzz_entry.rs`:

```rust
//! Fuzz seams for `kasane-writer`.
//!
//! A test seam, not API — the same convention and the same rationale as
//! `kasane-core`'s module of this name: it lives inside the crate so it can
//! reach `pub(crate)` internals (`escape::*`) that the separate `fuzz/`
//! workspace cannot.
//!
//! Each function takes `&[u8]` and either returns or panics. A panic **is**
//! the finding.
//!
//! This target asserts **postconditions**, not a round trip through a parser,
//! and that is deliberate: the round trip is P7's job, and a parser here would
//! mean a production dependency on `pulldown-cmark` for every kasane build.
//! The postconditions are the same kind of argument `kasane-core`'s `slug`
//! target makes — untrusted text entering a closed output alphabet.

use crate::escape::{self, Ctx};

pub fn escape(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    for ctx in [Ctx::Flow, Ctx::Cell] {
        for at_line_start in [true, false] {
            let out = escape::text(text, ctx, at_line_start);
            assert!(
                !out.contains('\r'),
                "escape::text kept a CR: {out:?} from {text:?}"
            );
            assert!(
                !out.contains("\n\n"),
                "escape::text kept a blank line: {out:?} from {text:?}"
            );
            if ctx == Ctx::Cell {
                assert!(
                    !out.contains('\n'),
                    "a cell cannot carry a newline: {out:?} from {text:?}"
                );
            }
            assert_unescaped_specials_are_absent(&out, text);
        }
    }

    // An HTML context must leave no bare `<`, `>` or `&`.
    let html = escape::text(text, Ctx::Html, false);
    for bad in ['<', '>'] {
        assert!(
            !html.contains(bad) || html.contains("<br>"),
            "html escaping left a bare {bad:?}: {html:?} from {text:?}"
        );
    }

    // A code span's delimiter run must not appear inside its content.
    let span = escape::code_span(text);
    let ticks = span.chars().take_while(|c| *c == '`').count();
    assert!(ticks >= 1, "code_span emitted no delimiter: {span:?}");
    let inner = &span[ticks..span.len() - ticks];
    assert!(
        !inner.contains(&"`".repeat(ticks)),
        "code_span content contains its own delimiter: {span:?} from {text:?}"
    );
    assert!(
        !inner.contains('\n'),
        "code_span kept a newline: {span:?} from {text:?}"
    );

    // A fenced block's fence must not appear at the start of a body line.
    let block = escape::fenced_block(text, Some(text));
    let fence_len = block.chars().take_while(|c| *c == '`').count();
    assert!(fence_len >= 3, "fence is too short: {fence_len} for {text:?}");
    let fence = "`".repeat(fence_len);
    assert!(
        block.trim_end().ends_with(&fence),
        "fenced_block is not closed: {block:?}"
    );
    let body_start = block.find('\n').map(|i| i + 1).unwrap_or(block.len());
    let body_end = block.trim_end().len() - fence_len;
    if body_start < body_end {
        for line in block[body_start..body_end].lines() {
            assert!(
                !line.starts_with(&fence),
                "a body line reopens the fence: {line:?} from {text:?}"
            );
        }
    }

    // Destinations carry nothing that ends or nests a bare destination.
    for dest in [escape::dest_path(text), escape::dest_url(text)] {
        for bad in [' ', '(', ')', '<', '>', '"', '\\'] {
            assert!(
                !dest.contains(bad),
                "destination contains {bad:?}: {dest:?} from {text:?}"
            );
        }
        assert!(
            !dest.chars().any(|c| c.is_ascii_control()),
            "destination contains a control character: {dest:?} from {text:?}"
        );
    }

    // A YAML scalar is one quoted line with no unescaped interior quote.
    let yaml = escape::yaml_scalar(text);
    assert!(
        yaml.starts_with('"') && yaml.ends_with('"') && yaml.len() >= 2,
        "yaml_scalar is not quoted: {yaml:?} from {text:?}"
    );
    assert!(
        !yaml.contains('\n') && !yaml.chars().any(|c| c.is_control()),
        "yaml_scalar is not one line: {yaml:?} from {text:?}"
    );
    let body = &yaml[1..yaml.len() - 1];
    assert_no_unescaped(body, '"', text);
    assert_no_unescaped(body, '\\', text);
}

/// Every character that can open an inline construct must carry a backslash.
fn assert_unescaped_specials_are_absent(out: &str, from: &str) {
    for c in ['`', '*', '_', '[', ']', '<', '~', '$'] {
        assert_no_unescaped(out, c, from);
    }
}

/// `c` never appears in `s` except immediately after an odd-length run of
/// backslashes — i.e. it is always escaped.
fn assert_no_unescaped(s: &str, c: char, from: &str) {
    let chars: Vec<char> = s.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        if *ch != c {
            continue;
        }
        let mut backslashes = 0;
        let mut j = i;
        while j > 0 && chars[j - 1] == '\\' {
            backslashes += 1;
            j -= 1;
        }
        assert!(
            backslashes % 2 == 1,
            "unescaped {c:?} at {i} in {s:?} from {from:?}"
        );
    }
}
```

Note the `\\` case in `assert_no_unescaped`: a lone backslash in the input is escaped to `\\`, an even-length run, which the odd-length rule reads as *escaped* only for the second character. Verify against the real output when the target first runs; if the rule mis-fires on backslash runs, assert instead that the total backslash count is even for a `\`-only input, and leave the odd-rule for the other eight characters. Do not weaken the other eight.

- [ ] **Step 2: Wire it up**

In `crates/kasane-writer/src/lib.rs`, add after `mod escape;`:

```rust
#[doc(hidden)]
pub mod fuzz_entry;
```

Change `escape.rs`'s items from `pub(crate)` to `pub(crate)` — no change needed, the seam is inside the crate. Confirm `mod escape;` is not `pub`.

Create `fuzz/fuzz_targets/escape.rs`:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    kasane_writer::fuzz_entry::escape(data);
});
```

In `fuzz/Cargo.toml`, add to `[dependencies]`:

```toml
kasane-writer = { path = "../crates/kasane-writer" }
```

and append:

```toml
[[bin]]
name = "escape"
path = "fuzz_targets/escape.rs"
test = false
doc = false
bench = false
```

In `crates/kasane-adapters/Cargo.toml`, add `kasane-writer.workspace = true` to `[dev-dependencies]` (matching how `kasane-core` is declared there).

In `crates/kasane-adapters/tests/fuzz_corpus.rs`, add to `target()` after the `slug` arm:

```rust
        "escape" => kasane_writer::fuzz_entry::escape,
```

and bump `const TARGET_COUNT: usize = 13;` to `14`.

In `mise.toml`, `[tasks.fuzz-all]`, extend the loop list — it currently omits `slug` as well, which is an oversight from that branch:

```bash
for t in epub pptx mobi pdf djvu epub_zip pptx_zip detect math_island palmdoc guards xmltext slug escape; do
```

- [ ] **Step 3: Write the seeds**

```bash
mkdir -p fuzz/seeds/escape
printf 'a*b_c[d]e`f<g>h~i$j\\k' > fuzz/seeds/escape/inline_openers.txt
printf '# h\n- b\n1. o\n> q\n= s\n| t\n' > fuzz/seeds/escape/line_starts.txt
printf '```rust\ninner ``` fence\n```' > fuzz/seeds/escape/fences.txt
printf 'Q&A &amp; &#38; &#x26; &notanentity' > fuzz/seeds/escape/entities.txt
printf -- '- "quoted"\ntrue\nnull\ntrailing ' > fuzz/seeds/escape/yaml.txt
```

- [ ] **Step 4: Run the stable replay**

```bash
cargo test -p kasane-adapters --test fuzz_corpus -- --nocapture 2>&1 | tail -20
```

Expected: PASS, and the run reports 14 targets. A panic here is a real finding in `escape.rs` — fix the module, not the assertion.

- [ ] **Step 5: Run the fuzzer**

```bash
ASAN_OPTIONS=detect_leaks=0 mise run fuzz escape -- -max_total_time=120 2>&1 | tail -20
```

Expected: no crash. `ASAN_OPTIONS=detect_leaks=0` is required in this sandbox — LeakSanitizer's atexit scan needs `ptrace`, which is not granted here, and every target otherwise ends with an empty-content artifact that looks like a finding (AGENTS.md, Workflows). If a real crash lands, commit the reproducer from `fuzz/artifacts/escape/` — that is what makes it a permanent regression test on stable — and fix the bug in the same branch.

- [ ] **Step 6: Commit**

```bash
mise run lint
mise run test
git add crates/kasane-writer/src/fuzz_entry.rs crates/kasane-writer/src/lib.rs \
        crates/kasane-adapters/Cargo.toml crates/kasane-adapters/tests/fuzz_corpus.rs \
        fuzz/Cargo.toml fuzz/fuzz_targets/escape.rs fuzz/seeds/escape mise.toml Cargo.lock
git commit -m "test(fuzz): an escape target, and slug restored to fuzz-all

The same argument the slug target makes: untrusted text entering a format with
a grammar. It asserts postconditions rather than a round trip, because a round
trip needs a parser and pulldown-cmark must not become a production dependency
of every kasane build -- P7 owns the round trip, this owns the closure of the
output alphabet.

fuzz-all's loop was also missing slug, which has been unfuzzed by that task
since it landed."
```

---

### Task 15: Documentation, and the path-invariance check

**Files:**
- Modify: `README.md:144` (Known limitations, new entry)
- Modify: `AGENTS.md` (the `crates/kasane-writer` entry)
- Test: a manual before/after conversion, recorded in the commit message

**Interfaces:**
- Consumes: everything.
- Produces: nothing.

- [ ] **Step 1: Run the path-invariance check (design spec §6.6)**

This is §5's first consequence, checked rather than asserted. Convert a fixture with the branch's code and with `main`'s, and diff the *tree shape* — which must be identical — separately from the content, which must not be.

```bash
mise run convert tests/fixtures/epub/sample.epub -o /tmp/claude-1000/-workspace/5b43e256-793e-4af9-b2c7-77e66e5eb4e4/scratchpad/after --force
git stash push --include-untracked
git checkout main -- crates/kasane-writer
cargo build -p kasane-cli 2>&1 | tail -3
mise run convert tests/fixtures/epub/sample.epub -o /tmp/claude-1000/-workspace/5b43e256-793e-4af9-b2c7-77e66e5eb4e4/scratchpad/before --force
git checkout HEAD -- crates/kasane-writer
git stash pop
cd /tmp/claude-1000/-workspace/5b43e256-793e-4af9-b2c7-77e66e5eb4e4/scratchpad
diff <(cd before && find . -type f | sort) <(cd after && find . -type f | sort)
```

Expected: **no output** from the final `diff` — every path identical. Then check the anchors:

```bash
diff <(grep -rho "](.*#[^)]*)" before | sort) <(grep -rho "](.*#[^)]*)" after | sort)
```

Expected: no output. If either diff is non-empty, stop: §5's invariant is broken and something in Tasks 6–11 changed rendered text rather than escaping it.

Pick a real fixture path if `tests/fixtures/epub/sample.epub` does not exist:

```bash
ls tests/fixtures/epub/
```

- [ ] **Step 2: Write the README entry**

Insert into `README.md`'s "Known limitations (this build)" list, after the anchors entry:

```markdown
- Text that looks like Markdown is preserved as text, not as markup. A book
  that literally prints `*`, `|`, `` ` ``, `[`, `&` or a line beginning with
  `#` converts to a file where those characters render as themselves, which
  means the Markdown source contains backslash escapes — `a\*b`, `1\. two`,
  `a\|b` inside a table cell. That is deliberate: the source document is
  content, not syntax. Two consequences worth knowing. A newline inside a
  heading, a table cell, a link label or a frontmatter title is folded — to a
  space, or to `<br>` in a cell — because those places are a single line by
  grammar. And a merged-cell table, which is emitted as raw HTML, carries its
  emphasis as `<strong>`/`<em>` tags and its equations as literal LaTeX, since
  GitHub parses no Markdown inside an HTML block.
```

- [ ] **Step 3: Write the AGENTS.md entry**

In `AGENTS.md`, append to the `crates/kasane-writer` bullet:

```markdown
  `escape.rs` is the only path from document text to an output buffer, and
  `Ctx` is a *required* argument on `inlines_to_md` rather than a defaulted
  one — that is the mechanism, not a convention: a new `Inline` arm or a new
  caller cannot inherit flow rules into a table cell by omission, because it
  does not compile until it names a context. `Inline::Text` is the only arm
  that calls `escape::text`; every other arm emits markup the writer chose,
  which must not be escaped. The governing invariant is that escaping never
  changes what the Markdown *renders* to, because `anchor_slug` computes
  fragments from unescaped IR text while GitHub computes ids from rendered
  text — which is also why `library.rs`'s former `link_text` (it replaced `[`
  with `(`) could not become the shared rule. Two destination encoders exist
  and differ on exactly one character: `dest_path` encodes `%` because a
  literal `%` in a filename would read back as an escape, and `dest_url` must
  not, because an `href` from a source document is already percent-encoded.
  The merged-table path emits HTML tags rather than Markdown markup, since GFM
  parses nothing inside an HTML block. `fuzz_entry.rs` is the `escape` fuzz
  seam, asserting postconditions (P7 in `tests/properties.rs` owns the round
  trip, because it can take `pulldown-cmark` as a dev-dependency and the
  library cannot).
```

- [ ] **Step 4: Verify the full gate**

```bash
mise run lint && mise run test
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add README.md AGENTS.md
git commit -m "docs: record the escaping policy and the invariance it preserves

README states the user-visible semantic -- source text that looks like Markdown
stays text, which is why the output carries backslashes -- plus the two places
that fold a newline and the merged-table path's HTML. AGENTS.md records the
mechanism: Ctx is required rather than defaulted, Inline::Text is the only arm
that escapes, and the two destination encoders differ on '%' alone.

Paths and anchors verified byte-identical against main over
tests/fixtures/epub: both the file list and every '](...#...)' destination
diff clean, which is the check design spec section 6.6 asks for."
```

---

## Self-Review

**Spec coverage.** §2 (module and `Ctx`) → Tasks 1, 6. §3.1 Flow → Task 1. §3.2 Cell → Task 2. §3.3 Html, including the HTML-tag fidelity fix → Tasks 2, 8. §3.4 code spans and fences → Tasks 3, 7. §3.5 destinations → Tasks 4, 6, 7, 11. §3.6 YAML → Task 5. §4.1 newlines → Tasks 1, 2, 7, 8. §4.2 footnotes → Task 9. §4.3 list items → Task 10. §4.4 `Raw` comments → Task 7. §5 the invariant → Task 15's path-invariance check. §6.1 case table → Tasks 1–5. §6.2 P7 → Task 13. §6.3 rebuilt helpers → Task 13. §6.4 generator → Task 12. §6.5 fuzz → Task 14. §6.6 end-to-end → Task 15. §7 documentation → Task 15. The `library.rs` fold named in §2 → Task 11.

**One assertion is flagged as needing verification rather than stated as fact:** `assert_no_unescaped` applied to `\` in Task 14. A lone backslash escapes to `\\`, an even-length run, which the odd-length rule reads differently from the other eight characters. The task says so, gives the narrower assertion to fall back to, and forbids weakening the other eight. Everything else in the plan is stated because it was checked: `pulldown-cmark` 0.13.4's event and tag names, `\*`/`\|` round-tripping, `<br>` arriving as `InlineHtml`, footnote continuation parsing, and fence widening were all run against the real crate before this plan was written.

**Type consistency.** `Ctx` is `{ Flow, Cell, Html }` throughout. `escape::text(&str, Ctx, bool) -> String`, `code_span(&str) -> String` (delimiters included), `fenced_block(&str, Option<&str>) -> String` (trailing newline included), `dest_path`/`dest_url`/`label`/`one_line`/`yaml_scalar`/`comment_note`: `&str -> String`. `inlines_to_md(&[Inline], Ctx, bool) -> String` after Task 6, the third argument being `at_line_start`; `inlines_to_html(&[Inline], usize) -> String` from Task 8. `indent_continuation(&str, &str) -> String` is introduced in Task 9 and consumed in Task 10. `Sentinel` gains `payload` in Task 12 and is read by P7 in Task 13. `parse_events(&str) -> Parsed` is defined once in Task 13 and used by `links_in`, `heading_anchors`, P7 and P8.
