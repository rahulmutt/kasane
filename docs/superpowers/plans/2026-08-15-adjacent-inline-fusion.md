# Adjacent Inline Fusion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a run of adjacent inlines that print with the same delimiter render as one span, so the text a parser recovers from kasane's Markdown is the text the IR held.

**Architecture:** `inlines_to_md_at` (`crates/kasane-writer/src/markdown.rs`) becomes a scan over *runs* rather than a loop over items. A run is a maximal group of neighbouring inlines that print with the same delimiter — backtick, `*`, or `**` — decided by a new `escape::delim`, which keys on what is printed rather than on the `Inline` variant so that a `Math` degrading to a code span joins the backtick class. Inlines that print nothing are stepped over instead of ending a run. No `Inline` is cloned or rewritten; `escape::code_span`, `escape::math_span` and `emphasize` keep their rules.

**Tech Stack:** Rust (stable 1.97.1, pinned in `mise.toml`), `proptest` 1.11, `pulldown-cmark` 0.13 as the test oracle. Tasks run in the `kasane-writer` crate only.

**Spec:** `docs/superpowers/specs/2026-08-15-adjacent-inline-fusion-design.md`

## Global Constraints

- Branch `adjacent-inline-fusion` already exists and holds the spec commit. Work on it; do not branch again and do not commit to `main`.
- Per-task checks are `mise run lint` and `mise run test`. `lint` is `cargo fmt --all -- --check` plus `cargo clippy --workspace --all-targets -- -D warnings`; a plain `cargo clippy` is **not** enough, because the test targets are where most new code in this plan lives.
- `-D warnings` means an unused item fails the build. Every item this plan adds is consumed in the same task that adds it; do not stage an interface ahead of its consumer.
- Rust edition and style come from `rustfmt.toml`; run `cargo fmt --all` before every commit rather than hand-aligning.
- Commit messages follow the repo's existing form: `fix(writer): …`, `test(writer): …`, `docs: …`.
- Do not change `kasane-gfm`, `kasane-core`, or `inlines_to_html`. The spec's §3 says why each is out of scope.
- The standing repo rule applies: a defect found *beside* the one being fixed is closed in this branch, not deferred. Task 4 is the task most likely to surface one.

---

### Task 1: Name the math degradation rule

`escape::math_span` decides whether math content is safe to print as `$…$` or must degrade to a code span. Task 2's delimiter class needs the same decision, and a copy of the expression in a second place is exactly the drift hazard this repo keeps retiring. Extract it under a name, with both callers going through it.

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs:531-539` (`math_span`)
- Test: `crates/kasane-writer/src/escape.rs` (`#[cfg(test)] mod tests` at the end of the file)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub(crate) fn math_degrades(s: &str) -> bool` in `escape.rs` — true when `math_span` would print `s` as a code span rather than as `$…$`.

- [ ] **Step 1: Write the failing test**

Add to `escape.rs`'s `#[cfg(test)] mod tests`:

```rust
/// `math_degrades` is `math_span`'s own branch condition, extracted so
/// `delim` can ask the same question (design spec §2.1). The two must agree
/// forever: this test asserts the predicate against `math_span`'s observable
/// output rather than against a second copy of the expression, so an edit to
/// either one that does not move the other fails here.
#[test]
fn math_degrades_agrees_with_what_math_span_prints() {
    for s in ["a$b", "$", "a\nb", "a\rb"] {
        assert!(math_degrades(s), "{s:?} should degrade");
        assert_eq!(math_span(s, Ctx::Flow), code_span(s, Ctx::Flow), "{s:?}");
    }
    for s in ["x", "\\frac{1}{2}", "a b"] {
        assert!(!math_degrades(s), "{s:?} should not degrade");
        assert_eq!(math_span(s, Ctx::Flow), format!("${s}$"), "{s:?}");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p kasane-writer math_degrades_agrees -- --nocapture`

Expected: FAIL to compile — `cannot find function `math_degrades` in this scope`.

- [ ] **Step 3: Extract the predicate**

In `escape.rs`, immediately above `math_span`, add:

```rust
/// Whether [`math_span`] will degrade this content to a code span rather than
/// print it as `$…$`.
///
/// Named rather than inlined into the branch below because
/// [`delim`] has to ask the same question: a degrading `Inline::Math` prints
/// with backticks, so it collides with a neighbouring code span exactly as a
/// second `Inline::Code` would (design spec §2.1). With the rule in one place,
/// widening what math degrades widens the delimiter class in the same edit and
/// cannot silently fail to.
pub(crate) fn math_degrades(s: &str) -> bool {
    s.contains('$') || s.contains('\n') || s.contains('\r')
}
```

Then change `math_span`'s first branch to call it:

```rust
pub(crate) fn math_span(s: &str, ctx: Ctx) -> String {
    if math_degrades(s) {
        code_span(s, ctx)
    } else if ctx == Ctx::Cell {
        format!("${}$", s.replace('|', "\\|"))
    } else {
        format!("${s}$")
    }
}
```

Leave `math_span`'s doc comment as it is — every word of it still describes what the function does.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p kasane-writer math_degrades_agrees`

Expected: PASS.

- [ ] **Step 5: Run the full checks**

Run: `mise run lint && mise run test`

Expected: both green. Nothing else in the crate changed behaviour — `math_span` prints exactly what it printed before.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-writer/src/escape.rs
git commit -m "refactor(writer): name math_span's degradation rule"
```

---

### Task 2: Fuse adjacent same-delimiter runs

The defect and its fix. `inlines_to_md_at` renders each inline independently, so two neighbours that print with the same delimiter meet with nothing between them and a parser reads one span where the IR held two.

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs` (add `Delim` and `delim`, after `math_degrades`)
- Modify: `crates/kasane-writer/src/markdown.rs:211-266` (`inlines_to_md_at`), plus three new private helpers below it
- Test: `crates/kasane-writer/src/markdown.rs` (`#[cfg(test)] mod tests`)
- Modify: `crates/kasane-writer/tests/properties.rs:798-878` (the pinned divergence test, which this task turns from true to false and must be rewritten in the same commit)

**Interfaces:**
- Consumes: `escape::math_degrades(&str) -> bool` from Task 1.
- Produces:
  - `pub(crate) enum Delim { Backtick, Emph, Strong }` (derives `Clone, Copy, PartialEq, Eq, Debug`) in `escape.rs`
  - `pub(crate) fn delim(i: &Inline) -> Option<Delim>` in `escape.rs`
  - `fn renders_empty(i: &Inline, depth: usize) -> bool` (private, `markdown.rs`)
  - `fn run_end(inls: &[Inline], start: usize, depth: usize) -> usize` (private, `markdown.rs`)
  - `fn backtick_run_content(members: &[Inline]) -> String` (private, `markdown.rs`)
  - `fn emphasis_run(members: &[Inline], depth: usize, ctx: Ctx, pos: Pos, markup: &str) -> String` (private, `markdown.rs`)

- [ ] **Step 1: Write the failing tests**

Add to `markdown.rs`'s `#[cfg(test)] mod tests`. These are the whole battery; write them all before touching the implementation.

```rust
/// Render one paragraph and return its line, without the trailing newline.
fn para(inls: Vec<Inline>) -> String {
    let md = blocks_to_markdown(&[Block::Para(inls)], &AssetBag::default());
    md.trim_end().to_string()
}

/// Adjacent code spans render as one span over their concatenation.
///
/// CommonMark cannot express two code spans in a row: the closing fence of
/// the first and the opening fence of the second form a single backtick run,
/// so `` `x` `` beside `` `y` `` came back as one span reading ``` x``y ```
/// — visible content corruption (design spec §1).
#[test]
fn adjacent_code_spans_render_as_one_span() {
    assert_eq!(
        para(vec![Inline::Code("x".into()), Inline::Code("y".into())]),
        "`xy`"
    );
    assert_eq!(
        para(vec![
            Inline::Code("x".into()),
            Inline::Code("y".into()),
            Inline::Code("z".into()),
        ]),
        "`xyz`"
    );
}

/// The fence is computed from the concatenation, not from either member, so a
/// run whose members each carry a backtick still gets a fence that closes it.
#[test]
fn a_fused_code_run_gets_a_fence_long_enough_for_the_concatenation() {
    assert_eq!(
        para(vec![Inline::Code("a`".into()), Inline::Code("`b".into())]),
        "``` a``b ```"
    );
}

/// Adjacent emphasis renders as one span. Undocumented before this item and
/// worse than the code case: the collided delimiters came back as literal
/// asterisks in the visible text (`*a**b*` parses to one `<em>` reading
/// `a**b`).
#[test]
fn adjacent_emphasis_renders_as_one_span() {
    let em = |s: &str| Inline::Emph(vec![Inline::Text(s.into())]);
    assert_eq!(para(vec![em("a"), em("b")]), "*ab*");

    let st = |s: &str| Inline::Strong(vec![Inline::Text(s.into())]);
    assert_eq!(para(vec![st("a"), st("b")]), "**ab**");
    assert_eq!(para(vec![st("a"), st("b"), st("c")]), "**abc**");
}

/// An inline that prints nothing does not break a run. It cannot: it puts no
/// character between the two delimiters, so the collision happens anyway
/// (design spec §2.3).
#[test]
fn an_inline_that_prints_nothing_does_not_break_a_run() {
    assert_eq!(
        para(vec![
            Inline::Code("x".into()),
            Inline::Text(String::new()),
            Inline::Code("y".into()),
        ]),
        "`xy`"
    );
    assert_eq!(
        para(vec![
            Inline::Emph(vec![Inline::Text("a".into())]),
            Inline::Emph(vec![]),
            Inline::Emph(vec![Inline::Text("b".into())]),
        ]),
        "*ab*"
    );
}

/// A whitespace-only inline is *not* vacuous. `emphasize` prints it as a bare
/// space, which genuinely separates the two code spans, and fusing across it
/// would delete a character a reader can see.
#[test]
fn a_whitespace_only_inline_separates_a_run() {
    assert_eq!(
        para(vec![
            Inline::Code("x".into()),
            Inline::Emph(vec![Inline::Text(" ".into())]),
            Inline::Code("y".into()),
        ]),
        "`x` `y`"
    );
}

/// A `Math` inline whose content forces `math_span` to degrade prints with
/// backticks, so it joins the backtick class. Keying the class on
/// `Inline::Code` alone would leave every shape here broken (design spec
/// §2.1).
#[test]
fn a_degrading_math_span_joins_the_backtick_class() {
    assert_eq!(
        para(vec![Inline::Code("x".into()), Inline::Math("a$b".into())]),
        "`xa$b`"
    );
    assert_eq!(
        para(vec![Inline::Math("a$b".into()), Inline::Code("y".into())]),
        "`a$by`"
    );
    assert_eq!(
        para(vec![Inline::Math("$".into()), Inline::Math("$".into())]),
        "`$$`"
    );
}

/// The run scan reaches every nesting level, because every inline sequence in
/// the crate goes through `inlines_to_md_at`.
#[test]
fn a_run_nested_inside_emphasis_fuses_too() {
    assert_eq!(
        para(vec![Inline::Emph(vec![
            Inline::Code("x".into()),
            Inline::Code("y".into()),
        ])]),
        "*`xy`*"
    );
}

/// `Ctx` is threaded unchanged, so a cell's `|` escaping applies across the
/// concatenation rather than per member.
#[test]
fn a_fused_run_in_a_table_cell_escapes_pipes_across_the_concatenation() {
    let t = Table {
        header: vec![vec![Inline::Text("H".into())]],
        rows: vec![vec![vec![
            Inline::Code("a|b".into()),
            Inline::Code("c".into()),
        ]]],
        has_merged: false,
    };
    let md = blocks_to_markdown(&[Block::Table(t)], &AssetBag::default());
    assert!(md.contains(r"| `a\|bc` |"), "{md}");
}

/// The shapes that must NOT fuse, so a later change that over-fuses fails
/// something. Each was measured against `pulldown-cmark` and recovers its
/// text intact today (design spec §1, "Confirmed").
#[test]
fn inlines_with_different_delimiters_are_left_alone() {
    let em = |s: &str| Inline::Emph(vec![Inline::Text(s.into())]);
    let st = |s: &str| Inline::Strong(vec![Inline::Text(s.into())]);

    assert_eq!(para(vec![em("a"), st("b")]), "*a***b**");
    assert_eq!(
        para(vec![Inline::Code("x".into()), Inline::Math("y".into())]),
        "`x`$y$"
    );
    assert_eq!(
        para(vec![Inline::Math("x".into()), Inline::Math("y".into())]),
        "$x$$y$"
    );
}

/// The one output change this item makes to a shape that was already correct.
///
/// `emphasize` hoists a trailing space outside the delimiters, so this pair
/// printed `*a* *b*` and parsed as two `<em>`s. The rule is uniform, so it
/// now prints one span. Same text, one element where there were two; the
/// alternative was a second copy of `emphasize`'s hoisting rule living in the
/// run scan (design spec §2.4).
#[test]
fn a_whitespace_separated_emphasis_pair_fuses_too() {
    assert_eq!(
        para(vec![
            Inline::Emph(vec![Inline::Text("a ".into())]),
            Inline::Emph(vec![Inline::Text("b".into())]),
        ]),
        "*a b*"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-writer --lib 2>&1 | tail -40`

Expected: FAIL. `adjacent_code_spans_render_as_one_span` reports `` `x``y` `` against `` `xy` ``; `adjacent_emphasis_renders_as_one_span` reports `*a**b*` against `*ab*`; the degrading-math, vacuous-inline, nested and cell tests fail the same way. `inlines_with_different_delimiters_are_left_alone` and `a_whitespace_only_inline_separates_a_run` PASS already — they pin behaviour that is correct today and must survive.

- [ ] **Step 3: Add the delimiter class to `escape.rs`**

Directly below `math_degrades`:

```rust
/// The delimiter an inline prints with, where two neighbours printing the same
/// one would collide.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Delim {
    /// A code span: `` `…` ``, at whatever fence length its content forces.
    Backtick,
    /// `*…*`.
    Emph,
    /// `**…**`.
    Strong,
}

/// Which delimiter this inline prints with, or `None` if it prints none that
/// can collide with a neighbour's.
///
/// Keyed on what is **printed**, not on the `Inline` variant, and that is the
/// whole reason this function exists rather than a `matches!` at the call
/// site: [`math_span`] degrades unsafe content to a code span, so
/// `[Code("x"), Math("a$b")]` prints two backtick spans and fuses exactly as
/// two `Code` inlines would. A rule matching `Inline::Code` alone would look
/// complete and leave that shape broken (design spec §2.1).
///
/// `Inline::Math` that does not degrade is `None` on purpose: `$x$$y$` is read
/// as two inline maths, and the two spans could not be merged even if they did
/// collide — `$xy$` states a different equation (design spec § Non-goals).
pub(crate) fn delim(i: &Inline) -> Option<Delim> {
    match i {
        Inline::Code(_) => Some(Delim::Backtick),
        Inline::Math(t) if math_degrades(t) => Some(Delim::Backtick),
        Inline::Emph(_) => Some(Delim::Emph),
        Inline::Strong(_) => Some(Delim::Strong),
        _ => None,
    }
}
```

- [ ] **Step 4: Add the three helpers to `markdown.rs`**

Insert directly below `inlines_to_md_at` (above `emphasize`):

```rust
/// Whether this inline prints nothing at all.
///
/// Exact rather than conservative, and it has to be both ways: [`run_end`]
/// steps over these, so a false positive drops content a reader can see and a
/// false negative leaves a fused pair behind. Each arm mirrors the renderer
/// above. `escape::text` never deletes, so a `Text` prints nothing exactly
/// when it is empty. `emphasize` returns its inner string unchanged when that
/// string is blank, so a container prints nothing exactly when every child
/// does. And `inlines_to_md_at` returns the empty string at
/// `MAX_INLINE_DEPTH`, so a container whose children sit at the bound really
/// does print nothing — which is why this takes the caller's absolute `depth`
/// rather than counting from zero.
///
/// Everything else is non-vacuous by construction: `Code("")` prints
/// `` ` ` ``, `Math("")` prints `$$`, a `Link` prints its brackets, a
/// `FootnoteRef` prints `[^n]`.
fn renders_empty(i: &Inline, depth: usize) -> bool {
    match i {
        Inline::Text(t) => t.is_empty(),
        Inline::Emph(x) | Inline::Strong(x) => {
            depth + 1 >= kasane_ir::MAX_INLINE_DEPTH
                || x.iter().all(|c| renders_empty(c, depth + 1))
        }
        _ => false,
    }
}

/// The exclusive end of the run of same-delimiter inlines starting at `start`.
///
/// A vacuous inline is stepped over rather than ending the run: it puts no
/// character between the two delimiters, so the collision happens across it
/// anyway. One inside a run is swallowed and never rendered, which is
/// equivalent, because the only thing it would have contributed is the empty
/// string. One *after* the last member is left for the outer loop, which
/// renders it as the no-op it is.
fn run_end(inls: &[Inline], start: usize, depth: usize) -> usize {
    let Some(d) = escape::delim(&inls[start]) else {
        return start + 1;
    };
    let mut end = start + 1;
    let mut k = start + 1;
    while k < inls.len() {
        if renders_empty(&inls[k], depth) {
            k += 1;
        } else if escape::delim(&inls[k]) == Some(d) {
            k += 1;
            end = k;
        } else {
            break;
        }
    }
    end
}

/// The content one code span carries for a whole backtick run.
///
/// A degrading `Inline::Math` contributes its raw LaTeX, which is exactly what
/// `math_span` would have handed `code_span` on its own. Anything else in the
/// slice is a vacuous inline `run_end` swallowed, and contributes nothing by
/// definition.
fn backtick_run_content(members: &[Inline]) -> String {
    let mut content = String::new();
    for m in members {
        if let Inline::Code(t) | Inline::Math(t) = m {
            content.push_str(t);
        }
    }
    content
}

/// Render a run of adjacent `Emph` (or `Strong`) inlines as one emphasized
/// span over the concatenation of their children.
///
/// `pos` is recomputed between members by the same rules the outer loop uses,
/// so a member sees where its own first character lands rather than where the
/// run opened. The first member still sees the run's opening `pos`, which is
/// what keeps a run of one byte-identical to what it printed before.
fn emphasis_run(members: &[Inline], depth: usize, ctx: Ctx, pos: Pos, markup: &str) -> String {
    let mut inner = String::new();
    let mut pos = pos;
    for m in members {
        let (Inline::Emph(x) | Inline::Strong(x)) = m else {
            continue;
        };
        let len_before = inner.len();
        inner.push_str(&inlines_to_md_at(x, depth + 1, ctx, pos));
        if inner.len() != len_before {
            pos = if inner.ends_with('\n') {
                Pos::LineStart
            } else {
                Pos::Mid
            };
        }
    }
    emphasize(&inner, markup)
}
```

- [ ] **Step 5: Turn `inlines_to_md_at`'s loop into a run scan**

Replace the body of `inlines_to_md_at` (`markdown.rs:211-266`) with:

```rust
fn inlines_to_md_at(inls: &[Inline], depth: usize, ctx: Ctx, pos: Pos) -> String {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return String::new();
    }
    let mut s = String::new();
    let mut pos = pos;
    let mut i = 0;
    while i < inls.len() {
        let before = pos;
        let len_before = s.len();
        let end = run_end(inls, i, depth);
        let members = &inls[i..end];
        match escape::delim(&inls[i]) {
            Some(escape::Delim::Backtick) => {
                s.push_str(&escape::code_span(&backtick_run_content(members), ctx))
            }
            Some(escape::Delim::Emph) => {
                s.push_str(&emphasis_run(members, depth, ctx, pos, "*"))
            }
            Some(escape::Delim::Strong) => {
                s.push_str(&emphasis_run(members, depth, ctx, pos, "**"))
            }
            // `delim` said this inline prints no delimiter that can collide,
            // so the run is this inline alone and it renders as it always has.
            None => match &inls[i] {
                // The only call to `escape::text` in the crate. Every other
                // arm here and above emits markup the writer chose, which must
                // not be escaped.
                Inline::Text(t) => s.push_str(&escape::text(t, ctx, pos)),
                Inline::Math(t) => s.push_str(&escape::math_span(t, ctx)),
                Inline::Link {
                    target: RefTarget::External(u),
                    inlines,
                } => s.push_str(&format!(
                    "[{}]({})",
                    kasane_gfm::fold_newlines(&inlines_to_md_at(
                        &escape::fold_inline_newlines(inlines),
                        depth + 1,
                        ctx,
                        pos
                    )),
                    escape::dest_url(u)
                )),
                // unresolved -> text
                Inline::Link { inlines, .. } => {
                    s.push_str(&inlines_to_md_at(inlines, depth + 1, ctx, pos))
                }
                Inline::FootnoteRef(n) => s.push_str(&format!("[^{}]", n.0)),
                // Unreachable: `delim` returns `Some` for all three.
                Inline::Code(_) | Inline::Emph(_) | Inline::Strong(_) => {}
            },
        }
        // Four rules (§2). An arm that appended nothing leaves the position
        // alone, so an empty text run between a reference and its colon does
        // not reset it. `Inline::FootnoteRef` always appends, so rule 3 is
        // never blocked by the length check. A run is one position step, not
        // one per member: only the run's own output has landed.
        if s.len() != len_before {
            pos = if s.ends_with('\n') {
                Pos::LineStart
            } else if matches!(&inls[i], Inline::FootnoteRef(_)) && before == Pos::LineStart {
                Pos::AfterFootnoteRef
            } else {
                Pos::Mid
            };
        }
        i = end;
    }
    s
}
```

Keep the existing doc comment above the function and append this paragraph to it:

```rust
/// The loop walks *runs*, not items: a maximal group of neighbouring inlines
/// that print with the same delimiter renders as one span over their
/// concatenated contents, because CommonMark cannot express two such spans in
/// a row and the writer's two delimiter pairs would otherwise fuse into one
/// span in the rendered line (design spec
/// `2026-08-15-adjacent-inline-fusion-design.md` §2).
```

- [ ] **Step 6: Run the unit tests to verify they pass**

Run: `cargo fmt --all && cargo test -p kasane-writer --lib`

Expected: PASS, all of them.

- [ ] **Step 7: Run the whole suite and watch the pinned test go red**

Run: `cargo test -p kasane-writer 2>&1 | tail -30`

Expected: exactly one failure —
`adjacent_empty_code_spans_diverge_from_the_line_they_print`, asserting
`` "## a` `` `b" `` against the ``## a`  `b`` this task now prints. **This is
the intended outcome, not a regression.** That test asserts a known-open bug's
values on purpose, and its doc comment nominates itself: "this test is what
should fail when that lands." Step 8 rewrites it.

- [ ] **Step 8: Flip the pinned test to an agreement test**

In `crates/kasane-writer/tests/properties.rs`, replace the whole item at
`:798-878` — doc comment, `#[test] fn adjacent_empty_code_spans_diverge_from_the_line_they_print`, and body — with:

```rust
/// Two *adjacent* empty code spans in a heading agree with the line they
/// print. Closed 2026-08-15 by
/// `2026-08-15-adjacent-inline-fusion-design.md`; this test recorded the
/// divergence before that, and its old body asserted `a--b` against a rendered
/// `ab` on purpose.
///
/// What closed it is not an anchor change. CommonMark cannot express two code
/// spans in a row: `` `x` `` beside `` `y` `` fuses into one span whose content
/// is the pair of backticks that should have closed the first and opened the
/// second. The writer now renders a run of adjacent code spans as one span
/// over their concatenation, so the printed line moved onto the anchor rather
/// than the other way round — `kasane-gfm` did not change.
///
/// The empty-code-span canonicalization
/// (`kasane_core::section::clone_inlines_at`) is what makes the two spaces
/// real: it reaches the writer as `[Code(" "), Code(" ")]`, which fuses to one
/// span over `"  "`, and `code_span`'s Rule 2 leaves an all-spaces content
/// unpadded, so the line prints both spaces and ids `a--b`.
///
/// The paragraph half is kept deliberately. The fusion was never only an
/// anchor bug: `Code("x")` beside `Code("y")` in an ordinary paragraph came
/// back as one span reading ``` x``y ```, content corruption with no empty
/// span and no heading anywhere near it.
#[test]
fn adjacent_empty_code_spans_agree_with_the_line_they_print() {
    let inlines = canonicalize_inlines(&[
        Inline::Text("a".into()),
        Inline::Code(String::new()),
        Inline::Code(String::new()),
        Inline::Text("b".into()),
    ]);
    let blocks = vec![Block::Heading {
        level: 2,
        id: BlockId(0),
        inlines: inlines.clone(),
    }];
    let md = kasane_writer::blocks_to_markdown(&blocks, &AssetBag::default());

    // One span over both padding spaces, not two spans that fuse.
    assert_eq!(md.trim_end(), "## a`  `b");
    let parsed = parse_events(&md);
    assert_eq!(parsed.headings, vec!["a  b".to_string()]);

    // The id a renderer computes from the printed line, and the anchor kasane
    // embeds in every cross-reference to it. Compared to each other, which is
    // the point: they used to be held apart.
    assert_eq!(
        anchors_for_headings(&parsed.headings)
            .first()
            .map(String::as_str),
        Some(anchor_slug_of(&inlines).as_str()),
    );
    assert_eq!(anchor_slug_of(&inlines), "a--b");

    // The content half, with no empty span and no heading involved: two
    // ordinary code spans side by side come back as the text they carried.
    let para = kasane_writer::blocks_to_markdown(
        &[Block::Para(vec![
            Inline::Code("x".into()),
            Inline::Code("y".into()),
        ])],
        &AssetBag::default(),
    );
    assert_eq!(para.trim_end(), "`xy`");
    assert_eq!(parse_events(&para).text.trim(), "xy");
}
```

If the old body ends past `:878`, delete through the closing brace of the old
`#[test]` function — the replacement is self-contained and nothing else in the
file refers to the old name. Check with
`grep -rn adjacent_empty_code_spans crates/ docs/` afterwards; the design specs
mention the old name in prose and are updated in Task 5, not here.

- [ ] **Step 9: Run the full checks**

Run: `mise run lint && mise run test`

Expected: both green.

- [ ] **Step 10: Verify the fixture tree is unchanged**

Run:

```bash
cargo run -q -p kasane-cli -- tests/fixtures/epub/rich.epub -o /tmp/kasane-after
git stash && cargo run -q -p kasane-cli -- tests/fixtures/epub/rich.epub -o /tmp/kasane-before; git stash pop
diff -r /tmp/kasane-before /tmp/kasane-after && echo IDENTICAL
```

Expected: `IDENTICAL`. The fixture's XHTML holds no adjacent inline pair, so
any diff means a run was grouped where no run exists. If the CLI's flags differ
from the above, run `cargo run -p kasane-cli -- --help` and adapt; the check is
"same input, same tree, before and after".

- [ ] **Step 11: Commit**

```bash
git add crates/kasane-writer/src/escape.rs crates/kasane-writer/src/markdown.rs crates/kasane-writer/tests/properties.rs
git commit -m "fix(writer): render adjacent same-delimiter inlines as one span"
```

---

### Task 3: Pin it with a text-survival property

The property tier has six whole-pipeline properties and none of them could have caught this. The anchor properties cannot: backticks and asterisks are outside `is_word`, so a fused heading's parsed line and its IR agree on the same slug over corrupt text. P7 cannot either: it checks that each *sentinel payload* survives, and the payload is always the leading `Text` run, never the inlines beside it. This task adds the property that reads the inlines.

**Files:**
- Modify: `crates/kasane-writer/tests/properties.rs` (new constant and strategy above the `proptest!` block at `:337`, new property inside it)

**Interfaces:**
- Consumes: `kasane_gfm::rendered_text` (already exported, `kasane-gfm/src/lib.rs:20`), `parse_events` (already in this file).
- Produces: nothing other tasks depend on.

- [ ] **Step 1: Write the property**

Above the `proptest! {` block (after `P12_TEXTS`), add:

```rust
/// P13's inline alphabet: adjacency, not hostility.
///
/// Every other property in this file draws hostile text on purpose. This one
/// deliberately does not. It asserts an *equality* between the text a real
/// parser recovers and `rendered_text`, and the newline fold, the escapes and
/// the character references each legitimately move bytes between those two
/// sides — with hostile text drawn, the property would fail on correct
/// behaviour. Restricting the alphabet to plain words is what makes the
/// equality exact; it is the move P12 made with `P12_TEXTS`, for the same
/// reason. Widen it and you will get a mystery failure from the fold, not a
/// bug.
///
/// `Inline::Code("")` is excluded specifically. `code_span`'s Rule 1 prints an
/// empty span as `` ` ` `` — an acknowledged round-trip divergence — while
/// `rendered_text` reads the IR's empty string. That divergence is unreachable
/// for IR that went through `structure`, which canonicalizes it to a space,
/// and is not what this property is about. An empty `Inline::Text` *is* drawn:
/// it prints nothing, and an inline that prints nothing sitting between two
/// spans is exactly the shape the run scan has to see through.
const P13_WORDS: &[&str] = &["a", "bc", "xyz"];

fn p13_inline() -> impl Strategy<Value = Inline> {
    let word = || proptest::sample::select(P13_WORDS).prop_map(|w| w.to_string());
    prop_oneof![
        word().prop_map(Inline::Text),
        word().prop_map(Inline::Code),
        word().prop_map(|w| Inline::Emph(vec![Inline::Text(w)])),
        word().prop_map(|w| Inline::Strong(vec![Inline::Text(w)])),
        Just(Inline::Text(String::new())),
    ]
}
```

Inside the `proptest! { … }` block, after P12:

```rust
/// P13 — inline text survives rendering, across inline boundaries.
///
/// The invariant the escaping spec's §5 states — escaping must never change
/// what the Markdown renders to — held within one inline and not between two.
/// Two neighbours printing the same delimiter met with nothing between them,
/// and a parser read one span where the IR held two: the delimiters that
/// should have separated them became visible characters in the middle of the
/// text (design spec §1).
///
/// Drawn as a flat sequence rather than through `generator::case()` because
/// that generator appends decoration after a payload run and this property
/// needs neighbours of the same kind next to each other, at a length the run
/// scan actually has to walk.
#[test]
fn p13_inline_text_survives_rendering(
    inlines in proptest::collection::vec(p13_inline(), 1..6),
) {
    let md = kasane_writer::blocks_to_markdown(
        &[Block::Para(inlines.clone())],
        &AssetBag::default(),
    );
    let recovered = parse_events(&md).text;
    let expected = kasane_gfm::rendered_text(&inlines);
    prop_assert_eq!(
        recovered.trim(),
        expected.trim(),
        "inline text changed under rendering:\n{}",
        md
    );
}
```

Add `rendered_text` to the file's `kasane_gfm` import:

```rust
use kasane_gfm::{anchor_slug_of, anchors_for_headings, rendered_text};
```

and call it unqualified (`rendered_text(&inlines)`) to match the file's style.

- [ ] **Step 2: Run it — expect PASS, then prove it can fail**

Run: `cargo test -p kasane-writer --test properties p13_`

Expected: PASS. **This property is green on arrival**, because Task 2 already
closed the defect; its red-first evidence is Task 2's unit battery. A
regression test that has never been seen to fail is worth little, so prove it:

```bash
# Temporarily neuter the run scan.
#   in markdown.rs, make run_end's body just: start + 1
cargo test -p kasane-writer --test properties p13_
```

Expected while neutered: FAIL, with a shrunk counterexample of two adjacent
same-kind inlines and a message showing the fused line. Then restore `run_end`
and re-run to confirm PASS. Do not commit the neutered version.

- [ ] **Step 3: Run the full checks**

Run: `mise run lint && mise run test`

Expected: both green.

- [ ] **Step 4: Commit**

```bash
git add crates/kasane-writer/tests/properties.rs
git commit -m "test(writer): P13 pins inline text against the line it prints"
```

---

### Task 4: Let the generator draw adjacency

P13 covers flat sequences the property constructs. The main tier still cannot
draw a single adjacent pair, because `build` composes every block's inlines as
`[Text(payload)] ++ deco` and `deco` is one inline. Widening it puts neighbours
into every block shape the tier generates — headings, cells, list items,
captions — and runs them through the whole engine rather than through
`blocks_to_markdown` alone.

**Files:**
- Modify: `crates/kasane-writer/tests/generator/mod.rs:377` (the `shapes` draw in `case()`) and `:255-259` (the `deco` doc comment on `build`)

**Interfaces:**
- Consumes: nothing new.
- Produces: nothing other tasks depend on.

- [ ] **Step 1: Widen the draw**

In `case()`, replace the `shapes` strategy:

```rust
    let shapes = proptest::collection::vec(
        (
            shape(),
            proptest::collection::vec(inlines(3), 1..=3).prop_map(|v| v.concat()),
            proptest::sample::select(HOSTILE),
        ),
        1..40,
    );
```

`inlines(3)` yields a one-element `Vec<Inline>`, so the collection is a
`Vec<Vec<Inline>>` and `concat()` flattens it into the `Vec<Inline>` `build`
already takes. No signature changes.

- [ ] **Step 2: Say why in the comment**

Update the comment directly above the `shapes` draw:

```rust
    // Each entry pairs a block shape with generated nested inline markup, so
    // nesting depth up to 3 is present throughout rather than only in flat
    // runs -- and one to three of them in a row, so neighbouring inlines exist
    // at all. A single draw could never put two inlines of the same kind side
    // by side, which is why six properties over this tier missed the
    // adjacent-delimiter fusion entirely
    // (`2026-08-15-adjacent-inline-fusion-design.md` §5.2).
```

and extend `build`'s `deco` paragraph (`:255-259`):

```rust
/// `deco` is generated nested inline markup (depth <= 3), one to three inlines
/// appended after the sentinel, so the engine's and the writer's inline walks
/// are exercised on real nesting and on real adjacency rather than only on
/// flat text. It is appended, never wrapped around the payload, so the payload
/// itself always renders as a bare run and the occurrence count stays exact --
/// which is why widening it from one inline to three needed no change to
/// `Expect`.
```

- [ ] **Step 3: Run the property tier**

Run: `cargo test -p kasane-writer --test properties 2>&1 | tail -30`

Expected: PASS, all thirteen properties.

**If something fails, read it before assuming it is this task's fault.** A
widened generator draws shapes the tier has never drawn, so a failure here is
most likely a real latent defect that adjacency exposed — which the standing
repo rule says to close in this branch, not defer. Two things to check first:
`proptest-regressions/properties.txt` will hold the seed, and
`cargo test -p kasane-writer --test properties <name> -- --nocapture` prints the
rendered Markdown the failure came from. Commit the regressions file with the
fix if one is written.

- [ ] **Step 4: Run the full checks**

Run: `mise run lint && mise run test`

Expected: both green. `mise run test` also replays the fuzz corpus, which is
unaffected by a generator change but is the run that proves it.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-writer/tests/generator/mod.rs
git commit -m "test(writer): let the generator draw adjacent inlines"
```

---

### Task 5: Correct the record

Two divergences are written down as surviving across five documents, and this
branch takes the count to one — `EMPTY_FALLBACK`, the deliberate choice. Two
code comments name the fusion as well. Every one of them is now wrong.

**Files:**
- Modify: `crates/kasane-gfm/src/slug.rs:50-90` (module doc)
- Modify: `crates/kasane-writer/src/escape.rs:454-479` (`code_span`'s Rule 1 comment)
- Modify: `crates/kasane-core/src/section.rs:160-171` (`clone_inlines_at`'s canonicalization comment)
- Modify: `AGENTS.md:19-40` and `AGENTS.md:98-104`
- Modify: `README.md:146` and `README.md:164-177`
- Modify: `docs/superpowers/specs/2026-08-09-markdown-escaping-design.md:491-517` and `:537`
- Modify: `docs/superpowers/specs/2026-08-14-empty-code-span-anchor-design.md` (Status block, § Non-goals)
- Modify: `docs/superpowers/specs/2026-08-15-adjacent-inline-fusion-design.md` (Status block)

**Interfaces:**
- Consumes: the behaviour Tasks 2-4 shipped.
- Produces: nothing.

- [ ] **Step 1: `kasane_gfm::slug`'s module doc**

At `:68`, "Two divergences are left, one a choice and one a defect:" becomes:

```rust
//! One divergence is left, and it is a choice rather than a construction
//! defect:
```

Delete the whole **Adjacent empty code spans** bullet (`:78-90`, through the
sentence ending "The anchor divergence is downstream of that fusion, not beside
it." and the paragraph after it that records the trade). Keep the **empty id**
bullet exactly as it is.

In the paragraph above (`:52-67`), the sentence beginning "A heading containing
**one** empty inline code span no longer diverges either" ends "…two or more
empty spans *next to each other* are a different shape and still diverge; see
the second bullet." Replace that clause with:

```rust
//! Two or more empty spans next to each other were a different shape and did
//! diverge until 2026-08-15, when `kasane-writer` began rendering a run of
//! adjacent same-delimiter inlines as one span
//! (`2026-08-15-adjacent-inline-fusion-design.md`). That is a fourth
//! mechanism, and the only one of the four that lives outside this crate: the
//! rule here was never wrong for that shape, the printed line was.
```

- [ ] **Step 2: `escape::code_span`'s Rule 1 comment**

The comment at `:454-479` is correct about Rule 1 and about the hand-built
caller; only its account of the adjacent case is stale, and that account is
carried by the phrase "Rule 1 and Rule 2 must keep printing the same bytes for
that canonicalization to stay invisible". Leave that. Append one paragraph
before it:

```rust
        // Two adjacent empty spans are no longer a special case either: the
        // run scan in `inlines_to_md_at` renders them as one span over their
        // concatenated content, so `[Code(" "), Code(" ")]` prints `` `  ` ``
        // and Rule 2 leaves both spaces intact. Rule 1 is reached only by a
        // run whose whole concatenation is empty.
```

- [ ] **Step 3: `clone_inlines_at`'s comment**

At `section.rs:160-171` the comment explains the canonicalization. It does not
name the fusion, so check it with
`grep -n "fuse\|adjacent" crates/kasane-core/src/section.rs` — if nothing
matches, this file needs no edit and that is the expected outcome. Record which
it was; do not invent a change.

- [ ] **Step 4: `AGENTS.md`**

At `:19`, "Of the four anchor divergences … three are now closed" becomes five
and four; "Two survive." at `:24` becomes "One survives." Delete the sentences
from "The other, recorded 2026-08-14, is the shape that canonicalization traded
away" through "…and fixing it needs its own item." Replace with:

```
  That shape closed on 2026-08-15: `kasane-writer` now renders a run of
  adjacent same-delimiter inlines as one span, so two empty code spans print
  one span over both padding spaces and the line ids what the anchor says.
```

In the `kasane-writer` entry (find it with `grep -n "kasane-writer" AGENTS.md`),
add one sentence recording the rule itself:

```
  Adjacent inlines that print with the same delimiter -- two code spans, two
  `Emph`, two `Strong` -- render as one span over their concatenation, because
  CommonMark cannot express two such spans in a row and the two delimiter pairs
  would otherwise fuse into one span in the rendered line, leaking the
  delimiters into the text as visible characters.
```

At `:98-104`, "one divergence still survives there on purpose: the empty-id
fallback" is already correct; delete the following sentences from "The second
divergence the `kasane-gfm` entry lists as surviving — adjacent empty code
spans — is deliberately not in this table and cannot be:" through "…so it
cannot catch a misreading." Keep the sentence beginning "That table pins
kasane's *reading* of the algorithm", reflowing so it still reads as a
paragraph.

- [ ] **Step 5: `README.md`**

At `:146`, "with two exceptions" becomes "with one exception". At `:164`, "The
two exceptions:" becomes "The exception:". Delete the second bullet entirely
(`:169-177`, "A heading containing two or more empty pairs of backticks…"). In
the paragraph that follows, "Three anchors that used to diverge no longer do"
becomes four, with the new one described in the same reader-facing register as
its neighbours — something like:

```
  - Two or more empty pairs of backticks next to each other in a heading. They
    now print as a single pair around the spaces they stand for, instead of
    fusing into one span that swallowed the backticks between them.
```

- [ ] **Step 6: `2026-08-09-markdown-escaping-design.md`**

At `:491`, the bullet **Adjacent code spans, which the writer fuses.** gets a
closure note in the shape the two bullets above it already use — what closed
it, by which mechanism, and what the bullet predicted instead:

```markdown
  **Closed 2026-08-15** by `2026-08-15-adjacent-inline-fusion-design.md`. The
  writer now renders a run of adjacent same-delimiter inlines as one span over
  their concatenation, so the printed line moved onto the anchor and
  `kasane-gfm` did not change. Wider than this bullet predicted in one
  direction: the same collision hits two `Inline::Emph` and two
  `Inline::Strong`, which print `*a**b*` and `**a****b**` and leak literal
  asterisks into visible text — neither was recorded anywhere before that item
  measured them. Narrower in another: adjacent `Inline::Math` does not collide
  and is deliberately untouched, since `$xy$` would state a different equation.
```

Also change the intro above the bullets ("Two cases remain open… The three
bullets below…") to match the new count, and add the one new entry the fix
opened, as a question rather than a defect:

```markdown
- **Adjacent `Inline::Math`, unverified.** `$x$$y$` parses as two inline maths
  under `pulldown-cmark`, the oracle the property tier uses. GitHub's math
  extension is a separate implementation and has not been checked. Recorded as
  a question rather than a known defect, and deliberately not "fixed" — fusing
  two equations into one would corrupt content rather than repair it
  (`2026-08-15-adjacent-inline-fusion-design.md` §8).
```

At `:537`, §5's invariant gains one sentence:

```markdown
Until 2026-08-15 this held *within* one inline and not between two: adjacent
code spans, and adjacent emphasis, collided at the boundary and rendered as one
span over text that was not the IR's. The run scan in `inlines_to_md_at` is
what makes the invariant hold across an inline boundary.
```

- [ ] **Step 7: `2026-08-14-empty-code-span-anchor-design.md`**

Its Status block's "Scope correction, same day" paragraph and its § Non-goals
correction both call the adjacent shape open. Append to each:

```markdown
**Closed 2026-08-15** by `2026-08-15-adjacent-inline-fusion-design.md`, in the
writer rather than in this item's canonicalization: a run of adjacent
same-delimiter inlines now renders as one span, so the two padding spaces this
item created are both printed and the line ids `a--b`, which is what this item
already anchored.
```

- [ ] **Step 8: This branch's own spec**

`2026-08-15-adjacent-inline-fusion-design.md`'s Status line reads "Designed
2026-08-15. Not yet implemented." Replace with:

```markdown
**Status:** Implemented 2026-08-15. Adjacent code spans, adjacent `Emph` and
adjacent `Strong` each render as one span; pinned by the unit battery in
`markdown.rs`, by P13, and by
`adjacent_empty_code_spans_agree_with_the_line_they_print`. The external oracle
has not been re-run; §8's note about the adjacent-code-span case and the
adjacent-math question both stand.
```

- [ ] **Step 9: Check nothing still claims two divergences**

Run:

```bash
grep -rn "two exceptions\|Two divergences\|Two survive\|two or more" \
  README.md AGENTS.md crates/ docs/superpowers/specs/ | grep -v "2026-08-15-adjacent"
```

Expected: no hit that asserts a surviving adjacent-span divergence. Hits inside
the historical narrative of a dated spec are fine where they describe what was
true at that date and carry a closure note; hits in `README.md`, `AGENTS.md` or
a module doc are not.

- [ ] **Step 10: Run the full checks**

Run: `mise run lint && mise run test`

Expected: both green. Doc comments are compiled, so a malformed one fails here.

- [ ] **Step 11: Commit**

```bash
git add -A
git commit -m "docs: record the adjacent-inline fusion closure"
```

---

## Self-Review

**1. Spec coverage.**

| Spec section | Task |
|---|---|
| §2.1 delimiter class, `math_degrades` | 1, 2 (Steps 3) |
| §2.2 run grouping at emission, no clone | 2 (Steps 4-5) |
| §2.3 vacuous inlines | 2 (`renders_empty`, `run_end`) |
| §2.4 the deliberate byte change | 2 (`a_whitespace_separated_emphasis_pair_fuses_too`) |
| §3 blast radius (`inlines_to_html` untouched, fixtures unchanged) | 2 (Step 10) |
| §4 anchor agreement | 2 (Step 8) |
| §5.1 unit battery | 2 (Step 1) |
| §5.2 generator widening + P13 | 3, 4 |
| §5.3 pinned test flips | 2 (Step 8) |
| §6 documentation | 5 |
| §8 verification, fixture check, residual risks | 2 (Step 10), 5 (Step 8) |

No gaps.

**2. Placeholder scan.** No TBD/TODO, no "handle edge cases", no "similar to
Task N". Every code step carries the full text to write. Task 5 Step 3 is the
one step whose outcome is conditional, and it names both outcomes and says to
record which one happened rather than to invent an edit.

**3. Type consistency.** `math_degrades(&str) -> bool` is defined in Task 1 and
called in Task 2's `delim`. `Delim`'s three variants are spelled `Backtick`,
`Emph`, `Strong` in both the definition and the `match` in
`inlines_to_md_at`. `run_end(inls, start, depth) -> usize` returns an exclusive
end and is used as `&inls[i..end]` and `i = end`. `emphasis_run` and
`backtick_run_content` take `&[Inline]` and are called with that slice.
`renders_empty(i, depth)` takes the caller's absolute depth in both its
definition and its two call sites.

**4. Red-first honesty.** Task 2's battery is genuinely red before Step 5.
Task 3's P13 is green on arrival — flagged as such, with a step that proves it
can fail. Task 4's properties should be green on arrival too, with instructions
for the case where widening the generator exposes something else.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-08-15-adjacent-inline-fusion.md`. Two execution options:

**1. Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
