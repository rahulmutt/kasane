# Delimiter-Choice Ordering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an emphasis run choose its delimiter character *before* the splice rules consult it, so a nested container is kept alive by spelling the outer run `_` instead of being deleted.

**Architecture:** `Delim` keeps the class; a new `Mark` pairs a class with the character a run has *chosen*, and `Delim::child_ch` states the character a *child* is predicted to print. The splice rules take a `Mark` and compare real characters, so when a run chooses `_` they simply find nothing to splice — no special-case skip. One function, `choose_mark`, is the single decision point, called by both `emphasis_run` and `probe_edges` so the two cannot drift.

**Tech Stack:** Rust, `pulldown-cmark` 0.13 (dev-dependency, oracle only), `mise` task runner.

**Spec:** `docs/superpowers/specs/2026-08-23-delimiter-choice-ordering-design.md`

## Global Constraints

- Branch: `delimiter-choice-ordering`. It already carries the spec and evidence (commit `72dc2f0`).
- Every task ends green on `mise run test` **and** `mise run lint`. `lint` is `cargo fmt --all -- --check` plus `cargo clippy --workspace --all-targets -- -D warnings`; plain `cargo clippy` is not sufficient.
- Tasks 1 and 2 are **behaviour-neutral refactors**. The census shape files must not change and must not be blessed. If a census test fails in Task 1 or 2, the refactor is wrong — do not bless to make it pass.
- Only Task 3 changes rendered output. Its bless is expected to produce exactly: `census-inexpressible.txt` 1,984 → 428, `census-known-structure-corrupt.txt` 1,730 → 1,616, `census-permanent-count.txt` 1984 → 428, `census-known-corrupt.txt` unchanged at 0 entries.
- `may_abut`, `Ledger`, `Site` and `mod cell` are **not modified by any task**.
- All existing doc comments that this change falsifies are corrected in-branch (Tasks 1, 3, 5), never deferred.
- Existing file layout is kept: all writer changes land in `crates/kasane-writer/src/markdown.rs` and `crates/kasane-writer/src/escape.rs`, matching the crate's existing large-module convention.

## File Structure

| file | responsibility | tasks |
|---|---|---|
| `crates/kasane-writer/src/escape.rs` | `Delim` (class), `Delim::child_ch` (predicted child character), new `Mark` (chosen character + markup) | 1 |
| `crates/kasane-writer/src/markdown.rs` | `choose_mark` (the single decision point), splice rules keyed on `Mark`, `parent_ch` threading through `inlines_to_md_flat` / `emphasis_run` / `probe_edges` | 1, 2, 3 |
| `crates/kasane-writer/src/markdown.rs` `mod tests` | regression tests for each of the rule's three conditions | 3 |
| `crates/kasane-writer/tests/census-*.txt`, `census-permanent-count.txt` | re-blessed shape files | 3 |
| `AGENTS.md`, `crates/kasane-writer/tests/census-inexpressible.txt` header, `crates/kasane-writer/tests/census_support/mod.rs` | prose corrections the measurement falsifies | 5 |

---

### Task 1: Make the delimiter character explicit, with no behaviour change

Today `Delim::ch()` answers two different questions with one method: "what character does this run print" and "what character does this child print". They are about to diverge. This task splits them and routes the splice rules through the run's chosen character — still always `'*'`, so nothing renders differently.

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs:527-543` (the `impl Delim` block)
- Modify: `crates/kasane-writer/src/markdown.rs:869-891` (`edge_to_splice`), `912-921` (`same_delim_to_splice`), `985-1006` (`splice_children`), `1478` (`emphasis_run`'s splice call), `1286` (`probe_edges`'s splice call)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `escape::Delim::child_ch(self) -> char` — renamed from `ch`, same body, new meaning.
  - `escape::Mark { class: Delim, ch: char }`, `pub(crate)`, `#[derive(Clone, Copy, PartialEq, Eq, Debug)]`.
  - `escape::Mark::new(class: Delim, ch: char) -> Mark`
  - `escape::Mark::markup(self) -> &'static str`
  - `edge_to_splice(children: &[Flat<'_>], run: escape::Mark, ledger: Ledger) -> Option<usize>`
  - `same_delim_to_splice(children: &[Flat<'_>], run: escape::Mark, ledger: Ledger) -> Option<usize>`
  - `splice_children<'a>(children: Vec<Flat<'a>>, run: escape::Mark, ledger: Ledger) -> Vec<Flat<'a>>`

- [ ] **Step 1: Write the failing test**

Add to `crates/kasane-writer/src/markdown.rs`, inside `mod tests`:

```rust
#[test]
fn a_mark_spells_its_class_with_its_chosen_character() {
    use escape::{Delim, Mark};
    assert_eq!(Mark::new(Delim::Emph, '*').markup(), "*");
    assert_eq!(Mark::new(Delim::Strong, '*').markup(), "**");
    assert_eq!(Mark::new(Delim::Emph, '_').markup(), "_");
    assert_eq!(Mark::new(Delim::Strong, '_').markup(), "__");
    assert_eq!(Mark::new(Delim::Backtick, '`').markup(), "`");
}

/// `Edge` classifies characters, and both emphasis characters classify the
/// same. This is why `probe_edges` and the render can disagree about the
/// *decision* but never about the classes, and why widening the alphabet does
/// not widen `Edge`'s surface (design spec 2026-08-23 §4.4).
#[test]
fn both_emphasis_characters_are_punctuation_to_the_flanking_rules() {
    assert_eq!(class_of('*'), Flank::Punct);
    assert_eq!(class_of('_'), Flank::Punct);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-writer --lib a_mark_spells_its_class`
Expected: FAIL to compile — `no `Mark` in `escape``.

- [ ] **Step 3: Write minimal implementation**

In `crates/kasane-writer/src/escape.rs`, replace the `impl Delim` block's `ch` method and add `Mark` after it:

```rust
impl Delim {
    /// The character a **child** of a run is predicted to print.
    ///
    /// Conservative by design (design spec `2026-08-23-delimiter-choice-ordering-design.md`
    /// §4.2). A run chooses its own character before its children render, so a
    /// child's character is not yet decided when the splice rules need it.
    /// Rather than recurse, they assume `*`: exact when the run chose `_`,
    /// because a child may not take its parent's character; conservative when
    /// the run chose `*`, where a child that could safely have taken `_` is
    /// spliced first instead. The cost is a missed recovery, never a
    /// corruption.
    ///
    /// This is *not* the character a run prints — that is [`Mark::ch`], which
    /// is chosen per run. The two were one method until 2026-08-23 and had to
    /// be separated when the choice became real.
    pub(crate) fn child_ch(self) -> char {
        match self {
            Delim::Backtick => '`',
            Delim::Emph | Delim::Strong => '*',
        }
    }
}

/// A delimiter class together with the character a run has **chosen** to spell
/// it with.
///
/// Two runs collide when they share a character, not when they share a class:
/// `*` and `**` abut into one `***` run a parser splits somewhere the writer
/// did not intend, while a backtick beside a `*` is simply two characters.
/// Keying the splice rules on this value rather than on [`Delim`] is what
/// states that rule as written, instead of leaving it true by the coincidence
/// that this writer once never spelled emphasis with `_`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Mark {
    pub(crate) class: Delim,
    pub(crate) ch: char,
}

impl Mark {
    pub(crate) fn new(class: Delim, ch: char) -> Mark {
        debug_assert!(
            match class {
                Delim::Backtick => ch == '`',
                Delim::Emph | Delim::Strong => ch == '*' || ch == '_',
            },
            "a {class:?} cannot be spelled with {ch:?}"
        );
        Mark { class, ch }
    }

    /// The literal this mark opens and closes with.
    pub(crate) fn markup(self) -> &'static str {
        match (self.class, self.ch) {
            (Delim::Backtick, _) => "`",
            (Delim::Emph, '_') => "_",
            (Delim::Strong, '_') => "__",
            (Delim::Strong, _) => "**",
            (Delim::Emph, _) => "*",
        }
    }
}
```

In `crates/kasane-writer/src/markdown.rs`, change the three splice functions to take a `Mark`. `edge_to_splice`'s head becomes:

```rust
fn edge_to_splice(children: &[Flat<'_>], run: escape::Mark, ledger: Ledger) -> Option<usize> {
    let ch = run.ch;
    let printing = |&(i, d): &Flat<'_>| !renders_empty(i, d);
    let first = children.iter().position(printing);
    let last = children.iter().rposition(printing);
    [first, last].into_iter().flatten().find(|&idx| {
        let Some(inner) = escape::delim(children[idx].0) else {
            return false;
        };
        if inner.child_ch() != ch {
            return false;
        }
        let site = if first == last {
            Site::WholeRun
        } else if Some(idx) == first {
            Site::HeadEdge
        } else {
            Site::TailEdge
        };
        !may_abut(run.class, inner, site, ledger)
    })
}
```

`same_delim_to_splice` becomes:

```rust
fn same_delim_to_splice(
    children: &[Flat<'_>],
    run: escape::Mark,
    ledger: Ledger,
) -> Option<usize> {
    children.iter().position(|&(i, _)| {
        let Some(inner) = escape::delim(i) else {
            return false;
        };
        inner == run.class
            && inner.child_ch() == run.ch
            && !may_abut(run.class, run.class, Site::Interior, ledger)
    })
}
```

`splice_children`'s signature becomes `(mut children: Vec<Flat<'a>>, run: escape::Mark, ledger: Ledger)`, and its two internal calls pass `run` instead of `want`. Its body is otherwise unchanged.

At both call sites, construct the mark with the character the writer uses today:

```rust
// markdown.rs:1478, in `emphasis_run`
let run_mark = escape::Mark::new(want, '*');
let children = splice_children(run_children(members), run_mark, ledger);
```

```rust
// markdown.rs:1286, in `probe_edges`
let inner_mark = escape::Mark::new(want, '*');
let inner_children = splice_children(run_children(members), inner_mark, ledger);
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `mise run test`
Expected: PASS. The census shape files must be **unchanged** — this task alters no output.

- [ ] **Step 5: Verify no output changed**

Run: `git diff --stat crates/kasane-writer/tests/`
Expected: empty. If any `census-*.txt` differs, the refactor changed behaviour; fix it rather than blessing.

- [ ] **Step 6: Lint**

Run: `mise run lint`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/kasane-writer/src/escape.rs crates/kasane-writer/src/markdown.rs
git commit -m "refactor(writer): split a run's chosen delimiter from a child's predicted one

\`Delim::ch\` answered two questions that are about to diverge. It becomes
\`Delim::child_ch\` -- the conservative prediction the splice rules make about
a child -- and a new \`Mark\` carries the character a run has chosen. The splice
rules take a \`Mark\` and compare real characters.

Behaviour-neutral: every mark is built with '*', so no output moves and no
census file changes."
```

---

### Task 2: Route the choice through one function and thread `parent_ch`

`emphasis_run` and `probe_edges` each derive a run's fate independently, and `debug_assert_eq!(edge, Edge::of(&inner))` is the only thing keeping them in step. Adding a second decision to each of them separately would double that debt. This task gives both a single `choose_mark` to call, and threads the one input it needs that neither has today. Still behaviour-neutral: `choose_mark` returns `'*'` unconditionally.

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs` — add `choose_mark`; change `inlines_to_md_flat:299`, `emphasis_run:1467-1478`, `probe_edges:1258-1265` and their call sites.

**Interfaces:**
- Consumes: `escape::Mark`, `escape::Delim::child_ch`, `splice_children(children, run: Mark, ledger)` from Task 1.
- Produces:
  - `choose_mark(want: escape::Delim, raw_children: &[Flat<'_>], before: Flank, after: Flank, parent_ch: char, ledger: Ledger) -> escape::Mark`
  - `inlines_to_md_flat<'a>(items: Vec<Flat<'a>>, ctx: Ctx, pos: Pos, ledger: Ledger, parent_ch: char) -> String`
  - `emphasis_run<'a>(members: &[Flat<'a>], want: escape::Delim, ctx: Ctx, pos: Pos, before: Flank, after: Flank, ledger: Ledger, parent_ch: char) -> RunOut<'a>` — note the `markup: &str` parameter is **removed**; the mark now supplies it.
  - `probe_edges(children: &[Flat<'_>], ctx: Ctx, pos: Pos, ledger: Ledger, outer_before: Flank, outer_after: Flank, parent_ch: char) -> Edge`
  - The sentinel for "no enclosing emphasis run" is `'\0'`, passed by `blocks_to_markdown`'s entry into `inlines_to_md_flat`.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/kasane-writer/src/markdown.rs`:

```rust
/// Task 2 pins the seam, not the rule: `choose_mark` is the only place a
/// character is decided, and until Task 3 it decides `*` every time.
#[test]
fn choose_mark_is_the_single_decision_point_and_still_says_star() {
    use escape::{Delim, Mark};
    let a = Inline::Emph(vec![Inline::Text("a".into())]);
    let kids: Vec<Flat<'_>> = vec![(&a, 1)];
    for (class, before, after, parent) in [
        (Delim::Emph, Flank::Space, Flank::Space, '\0'),
        (Delim::Strong, Flank::Punct, Flank::Punct, '\0'),
        (Delim::Emph, Flank::Other, Flank::Other, '*'),
    ] {
        assert_eq!(
            choose_mark(class, &kids, before, after, parent, Ledger::LICENSED),
            Mark::new(class, '*'),
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-writer --lib choose_mark_is_the_single`
Expected: FAIL to compile — `cannot find function `choose_mark``.

- [ ] **Step 3: Write minimal implementation**

Add to `crates/kasane-writer/src/markdown.rs`, immediately above `splice_children`:

```rust
/// Which character this run spells its delimiter with.
///
/// The single decision point. [`emphasis_run`] and [`probe_edges`] both call
/// it, because they must reach the same answer or
/// `debug_assert_eq!(edge, Edge::of(&inner))` is comparing two different runs.
/// Neither may re-derive this locally.
///
/// Returns `*` unconditionally until the rule lands (design spec
/// `2026-08-23-delimiter-choice-ordering-design.md` §3).
fn choose_mark(
    want: escape::Delim,
    raw_children: &[Flat<'_>],
    before: Flank,
    after: Flank,
    parent_ch: char,
    ledger: Ledger,
) -> escape::Mark {
    let _ = (raw_children, before, after, parent_ch, ledger);
    escape::Mark::new(want, '*')
}
```

Thread `parent_ch` as a trailing parameter on `inlines_to_md_flat`, `emphasis_run` and `probe_edges`. In `emphasis_run`, replace the two lines that built the mark and the markup:

```rust
let raw = run_children(members);
let run_mark = choose_mark(want, &raw, before, after, parent_ch, ledger);
let children = splice_children(raw, run_mark, ledger);
```

and replace `emphasize(&inner, markup)` with `emphasize(&inner, run_mark.markup())`. Delete the `markup` parameter from the signature and the `let markup = if d == escape::Delim::Emph { "*" } else { "**" };` line at its call site (`markdown.rs:380`); `emphasis_run`'s `#[allow(clippy::too_many_arguments)]` already covers the new arity.

Every recursive render inside `emphasis_run` passes the run's own character down:

```rust
let inner = inlines_to_md_flat(children, ctx, pos, ledger, run_mark.ch);
```

and the all-whitespace early return does the same. In `probe_edges`, mirror it exactly:

```rust
let inner_mark = choose_mark(want, &raw_inner, running_before, after_ctx, parent_ch, ledger);
let inner_children = splice_children(raw_inner, inner_mark, ledger);
…
let sub = probe_edges(&inner_children, ctx, pos, ledger, running_before, after_ctx, inner_mark.ch);
```

`running_before` and `after_ctx` must be computed **before** this call, exactly as they already are — they are the same two values `emphasis_run` passes as `before`/`after`, which is what makes both calls to `choose_mark` agree.

Every other caller of `inlines_to_md_flat` and `probe_edges` passes `'\0'`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `mise run test`
Expected: PASS, census files unchanged.

- [ ] **Step 5: Verify no output changed**

Run: `git diff --stat crates/kasane-writer/tests/`
Expected: empty.

- [ ] **Step 6: Lint**

Run: `mise run lint`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/kasane-writer/src/markdown.rs
git commit -m "refactor(writer): one decision point for a run's delimiter character

\`emphasis_run\` and \`probe_edges\` must agree about a run or the
probe/render invariant compares two different things. Both now call
\`choose_mark\`, and \`parent_ch\` is threaded so the rule can see what the
enclosing run chose.

Behaviour-neutral: \`choose_mark\` returns '*' unconditionally."
```

---

### Task 3: Enable the rule

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs` — `choose_mark`'s body, and `same_delim_to_splice`'s doc comment
- Modify: `crates/kasane-writer/src/markdown.rs` `mod tests` — regression tests
- Modify (by bless): `crates/kasane-writer/tests/census-inexpressible.txt`, `census-known-structure-corrupt.txt`, `census-permanent-count.txt`

**Interfaces:**
- Consumes: `choose_mark` and `parent_ch` threading from Task 2.
- Produces: no new signatures. `choose_mark` may now return `'_'`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/kasane-writer/src/markdown.rs`. Each of the rule's three conditions is pinned separately, because none subsumes another.

```rust
fn md(inlines: Vec<Inline>) -> String {
    blocks_to_markdown(&[Block::Para(inlines)], &AssetBag::default())
        .trim_end()
        .to_string()
}

fn em(i: Inline) -> Inline { Inline::Emph(vec![i]) }
fn st(i: Inline) -> Inline { Inline::Strong(vec![i]) }
fn tx(s: &str) -> Inline { Inline::Text(s.into()) }

/// The three spellings `census-inexpressible.txt` advertised for two years and
/// the writer could not reach, because the container was spliced away before a
/// character was chosen (design spec 2026-08-23 §2.3).
#[test]
fn a_nested_container_survives_by_spelling_the_outer_run_with_an_underscore() {
    assert_eq!(md(vec![em(em(tx("a")))]), "_*a*_");
    assert_eq!(md(vec![st(st(tx("a")))]), "__**a**__");
    assert_eq!(md(vec![st(em(tx("a")))]), "__*a*__");
}

/// Condition 2. `_` cannot open or close against a letter, and emitting it
/// anyway loses *text*, not merely structure: `_*a*_a` parses as
/// `_<em>a</em>_a`, with the underscores landing in the prose.
#[test]
fn a_letter_flank_refuses_the_underscore_and_the_splice_stands() {
    assert_eq!(md(vec![tx("a"), em(em(tx("a"))), tx("c")]), "a*a*c");
    assert_eq!(md(vec![em(em(tx("a"))), tx("a")]), "*a*a");
    assert_eq!(md(vec![tx("a"), em(em(tx("a")))]), "a*a*");
}

/// Condition 3. A child taking its parent's character rebuilds the collision
/// one level down: `___a___` parses as `<em><strong>a</strong></em>`, not as
/// three nested emphases.
#[test]
fn a_child_never_takes_the_character_its_parent_took() {
    for shape in [
        vec![em(em(em(tx("a"))))],
        vec![st(st(st(tx("a"))))],
        vec![em(st(em(tx("a"))))],
    ] {
        let out = md(shape.clone());
        assert!(!out.contains("___"), "{shape:?} produced {out:?}");
    }
}

/// The cost of the conservative child prediction (design spec §4.2), pinned as
/// a limit of *that*, not as a representational one: Markdown spells this
/// shape as `_*_a_*_`. Outside the census alphabet, so it costs nothing today.
#[test]
fn the_conservative_child_prediction_still_loses_the_third_level() {
    assert_eq!(md(vec![em(em(em(tx("a"))))]), "_*a*_");
}

/// `same_delim_to_splice` keys on class *and* character. A `Strong` inside an
/// `Emph` shares neither, and must still be left alone.
#[test]
fn a_strong_inside_an_emph_is_still_not_spliced() {
    assert_eq!(
        md(vec![Inline::Emph(vec![tx("a"), st(tx("b")), tx("c")])]),
        "*a**b**c*"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-writer --lib a_nested_container_survives`
Expected: FAIL — `assertion `left == right` failed: left: "*a*", right: "_*a*_"`.

- [ ] **Step 3: Write the implementation**

Replace `choose_mark`'s body in `crates/kasane-writer/src/markdown.rs`:

```rust
fn choose_mark(
    want: escape::Delim,
    raw_children: &[Flat<'_>],
    before: Flank,
    after: Flank,
    parent_ch: char,
    ledger: Ledger,
) -> escape::Mark {
    let star = escape::Mark::new(want, '*');
    // Condition 1: only where a collision would otherwise cost a container.
    // Without this, `_` would rewrite the spelling of documents that already
    // round-trip and buy nothing; it is why the sweep measured 0 broken.
    let collides = edge_to_splice(raw_children, star, ledger).is_some()
        || same_delim_to_splice(raw_children, star, ledger).is_some();
    // Condition 2: CommonMark forbids `_` opening or closing against a letter
    // or digit. Emitting it anyway is *text* loss -- `_*a*_a` parses as
    // `_<em>a</em>_a` -- so this is mandatory, not conservative.
    let flanks_permit = before != Flank::Other && after != Flank::Other;
    // Condition 3: a child taking its parent's character rebuilds the
    // collision one level down. `___a___` is `<em><strong>a</strong></em>`.
    let parent_took_it = parent_ch == '_';
    if collides && flanks_permit && !parent_took_it {
        escape::Mark::new(want, '_')
    } else {
        star
    }
}
```

No change is needed at the splice call sites: `splice_children` is already keyed on the mark, so a `_` run finds nothing to splice on its own.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-writer --lib`
Expected: PASS.

- [ ] **Step 5: Re-bless the census and read the diff**

Run: `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census`

Then confirm the counts are exactly as the spec predicts:

```bash
wc -l crates/kasane-writer/tests/census-known-corrupt.txt \
      crates/kasane-writer/tests/census-known-structure-corrupt.txt \
      crates/kasane-writer/tests/census-inexpressible.txt
cat crates/kasane-writer/tests/census-permanent-count.txt
```

Expected: text file 0 lines; structure queue 1,616 entries; inexpressible 428 entries plus its comment header; permanent count `428`.

A different number means the rule is not the one that was measured — stop and diff the shape sets against `docs/superpowers/evidence/2026-08-23-underscore-alphabet/harnesses/p2-recovered.txt` rather than accepting it.

- [ ] **Step 6: Correct the doc comment this task falsifies**

In `crates/kasane-writer/src/markdown.rs`, `same_delim_to_splice`'s doc opens with a paragraph warning that its position-blind `Site::Interior` query is "a live hazard for later widening". Replace that paragraph with:

```rust
/// The `may_abut` query below is position-blind by construction: it always
/// asks `Site::Interior`, even for a child [`edge_to_splice`] would have
/// asked about with `HeadEdge`/`TailEdge`/`WholeRun` first. This used to be a
/// live hazard for widening -- licensing a same-`Delim` cell at some other
/// site would make `edge_to_splice` defer a child that this function then
/// spliced anyway on the next loop iteration. The 2026-08-23 delimiter-choice
/// item retired it: both rules are now keyed on the run's chosen character and
/// agree before either fires, so a run that keeps a child cannot have that
/// child removed by the other rule.
```

Also update the paragraph in the same file's `splice_children` doc that explains why a same-`Delim` container is spliced "even though same-`Delim` nesting is sometimes expressible". Append to it:

```rust
///   Since 2026-08-23 there is a third option this rule does not have to
///   reason about: a run that spells itself `_` shares no character with its
///   `*` children, so CommonMark cannot pair them with each other at all and
///   there is nothing to splice. That is why the alternative to splicing is
///   choosing a different character, not mirroring the pairing algorithm
///   §7 approach A rejects.
```

- [ ] **Step 7: Run the full suite and lint**

Run: `mise run test && mise run lint`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-writer/src/markdown.rs crates/kasane-writer/tests/
git commit -m "feat(writer): choose a run's delimiter character before the splice

A run whose child would collide now spells itself \`_\` when its flanks permit
and its parent did not, so the child survives instead of being spliced away.

Measured: 1,670 shapes recovered, 0 broken, 0 new text loss. Inexpressible
1,984 -> 428, structure queue 1,730 -> 1,616. Text stays zero at length 4.

The 428 that remain are one class: every one has letter text adjacent to the
nested container, where neither character can flank and emitting anyway would
lose text rather than structure."
```

---

### Task 4: Verify the invariant the probes could not

Both design-phase probes carried `parent_ch` in a global, so `probe_edges` never saw it and the probe/render agreement was never exercised against the real threading. Task 2 fixed the threading; this task is the evidence that it holds.

**Files:**
- Create (temporary, deleted in Step 5): `crates/kasane-writer/tests/zz_len5_debug.rs`

**Interfaces:**
- Consumes: the shipped writer from Task 3. Produces nothing — this task adds no committed code.

- [ ] **Step 1: Write the sweep**

Create `crates/kasane-writer/tests/zz_len5_debug.rs`:

```rust
//! One-off: the length-5 text tier in DEBUG, so `debug_assert_eq!(edge,
//! Edge::of(&inner))` is live. Deleted after it passes; not shipped, for the
//! same reason `census_len4.rs`'s doc gives for lengths 5 and 6 -- it costs
//! minutes, not seconds.
mod census_support;
use census_support::{alphabet, text_is_clean};
use kasane_ir::Inline;
use kasane_writer::Ledger;

#[test]
fn no_shape_of_length_five_loses_text_or_desynchronises_the_probe() {
    let a = alphabet();
    let n = a.len();
    let mut bad: Vec<String> = Vec::new();
    for m in 0..n.pow(5) {
        let mut x = m;
        let mut seq: Vec<Inline> = Vec::with_capacity(5);
        for _ in 0..5 {
            seq.push(a[x % n].clone());
            x /= n;
        }
        if !text_is_clean(&seq, Ledger::LICENSED) {
            bad.push(format!("{seq:?}"));
        }
    }
    assert!(bad.is_empty(), "{} shapes lost text; first 10:\n  {}",
        bad.len(), bad.iter().take(10).cloned().collect::<Vec<_>>().join("\n  "));
}
```

- [ ] **Step 2: Run it in debug**

Run: `cargo test -p kasane-writer --test zz_len5_debug -- --nocapture`
Expected: PASS over 2,476,099 shapes. Takes minutes — do not add `--release`, which compiles the `debug_assert` out and defeats the point of this task.

A panic reading `probe_edges disagreed with the render it stands in for` means Task 2's threading is wrong in `probe_edges` — most likely `running_before`/`after_ctx` computed after the `choose_mark` call instead of before. Fix Task 2's code, do not weaken the assert.

- [ ] **Step 3: Run the cross-revision ratchet**

Run: `mise run census-ratchet`
Expected: PASS. All three files shrank against `main`, and the permanent ceiling was lowered by the bless, so every direction is one the gate permits.

- [ ] **Step 4: Run the full gate**

Run: `mise run test && mise run lint && mise run census-ratchet`
Expected: PASS.

- [ ] **Step 5: Delete the temporary sweep and record the result**

```bash
rm crates/kasane-writer/tests/zz_len5_debug.rs
```

Append to `docs/superpowers/evidence/2026-08-23-underscore-alphabet/README.md`:

```markdown
## Post-implementation verification

The length-5 text tier was re-run **in debug** against the shipped
implementation, where `debug_assert_eq!(edge, Edge::of(&inner))` is live:
0 of 2,476,099 shapes lose text, and the probe/render invariant holds. That
closes the gap this directory records above — both design-phase probes carried
`parent_ch` in a global, so `probe_edges` never saw it and the length-5 figure
was `--release` only.
```

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/evidence/2026-08-23-underscore-alphabet/README.md
git commit -m "test(writer): verify the probe/render invariant at length 5 in debug

Both design-phase probes carried parent_ch in a global, so probe_edges never
saw it and the length-5 sweep ran under --release with the assert compiled
out. Re-run in debug against the real threading: 0 of 2,476,099."
```

---

### Task 5: Correct the prose this measurement falsifies

Four documents assert things the probes disproved. They are wrong *today*, so they close in this branch.

**Files:**
- Modify: `AGENTS.md:354-364`
- Modify: `crates/kasane-writer/tests/census-inexpressible.txt` (header comment block, lines 1-18)
- Modify: `crates/kasane-writer/tests/census_support/mod.rs:272`

**Interfaces:**
- Consumes: the final census counts from Task 3. Produces nothing.

- [ ] **Step 1: Correct `AGENTS.md`**

Replace the sentences beginning "A probe over every `*`/`_` spelling found 1,740 of its 1,984 entries expressible, so read it as the queue for the alphabet-widening item." with:

```markdown
  A 2026-08-17 probe put 1,740 of its then-1,984 entries within reach of `_`
  and named the follow-up "the alphabet-widening item". Both were wrong, and
  `2026-08-23-delimiter-choice-ordering-design.md` §2 is the measurement:
  offering `_` at the delimiter-emission site fixes **zero** shapes, because
  `splice_children` runs first and deletes the colliding child to dodge a
  collision with a character nothing has chosen yet. The alphabet was never
  the constraint; decision order was. Choosing the character before the splice
  recovered 1,670 shapes and took this file to its present 428 — every one of
  which has letter text adjacent to the nested container, where CommonMark's
  flanking rule stops either character opening and emitting anyway would lose
  text rather than structure. Only an HTML tag spells those.
```

Delete the trailing sentence "What is genuinely unspellable is narrower and has a different cause — CommonMark's left-flanking rule, which stops any delimiter opening between a letter and punctuation, so `[Text("a"), Text("a"), Emph([Code("x")])]` cannot emphasize with `*` or `_`." — it is now said, and measured, in the replacement above.

- [ ] **Step 2: Correct `census-inexpressible.txt`'s header**

Replace the paragraph beginning "A probe over every `*`/`_` spelling of every shape in this file found 1,740 of 1,984 expressible" with:

```
# Every entry here is one shape: CommonMark's flanking rule stops either `*` or
# `_` opening or closing against a letter or digit, and the nested container in
# each of these has letter text against it. 156 have the container first, so it
# is the closing delimiter that is blocked; the rest are blocked on the opener.
# Emitting the delimiter anyway loses TEXT, not merely structure -- `_*a*_a`
# parses as `_<em>a</em>_a`, underscores and all. Only an HTML tag spells these.
#
# This file held 1,984 entries until 2026-08-23 and was described as the queue
# for an alphabet-widening item. That framing was measured and destroyed: `_`
# offered at the delimiter-emission site fixes zero shapes, because the
# colliding child is spliced away before any character is chosen. See
# docs/superpowers/specs/2026-08-23-delimiter-choice-ordering-design.md §2.
```

Keep the two-mechanism explanation and the `_*x*_` examples below it — they remain accurate, and they now describe what the writer *does*.

- [ ] **Step 3: Correct the stale doc comment**

In `crates/kasane-writer/tests/census_support/mod.rs`, `Structure::Inexpressible`'s doc currently reads "Markdown cannot express this shape at any level. Permanent." Replace with:

```rust
    /// Not spellable with `*` or `_`, at any nesting level, because
    /// CommonMark's flanking rule stops either character opening against the
    /// letter text beside it. Only an HTML tag spells these.
    ///
    /// This said "Markdown cannot express this shape" until 2026-08-23, which
    /// was false in a way that cost an item its estimate: `_*x*_` is
    /// `<em><em>x</em></em>`. The 2026-08-17 correction reached the `.txt`
    /// headers and `AGENTS.md` and missed this line.
    Inexpressible,
```

- [ ] **Step 4: Verify the counts quoted in prose match the files**

```bash
# total entries
grep -c '^\[' crates/kasane-writer/tests/census-inexpressible.txt
# entries whose FIRST element is itself the nested container, so it is the
# CLOSING delimiter that is letter-flanked
grep -c '^\[\(Emph(\[Emph\|Emph(\[Strong\|Strong(\[Emph\|Strong(\[Strong\)' \
  crates/kasane-writer/tests/census-inexpressible.txt
# and the claim that every entry has letter text against the container: this
# must print 0
grep -vc 'Text("a")\|Text("b")' crates/kasane-writer/tests/census-inexpressible.txt
```

Expected: `428`, `156`, `0`. The narrower second pattern is the one that yields 156 — `'^\[\(Emph\|Strong\)'` matches 184, because it also catches entries that merely *start* with a plain container. If any differs, correct the prose to the file rather than the reverse.

- [ ] **Step 5: Run the full gate**

Run: `mise run test && mise run lint && mise run census-ratchet`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add AGENTS.md crates/kasane-writer/tests/census-inexpressible.txt \
        crates/kasane-writer/tests/census_support/mod.rs
git commit -m "docs: correct the three records the delimiter-choice measurement falsifies

AGENTS.md and census-inexpressible.txt both called this 'the alphabet-widening
item' and sized it at 1,740 shapes, from a probe that measured CommonMark
rather than this pipeline. Structure::Inexpressible still claimed 'Markdown
cannot express this shape at any level', which the 2026-08-17 correction
missed.

All three now say what the file holds: 428 shapes blocked by CommonMark's
flanking rule against adjacent letter text, spellable only as HTML."
```

---

## Self-Review

**Spec coverage.**

| spec section | task |
|---|---|
| §3 conditions 1/2/3 | Task 3 Step 3, tested Step 1 |
| §3.1 splice skipped, not mirrored | Task 3 Step 6 (doc), inherent in Task 1's keying |
| §4.1 `Delim`/`Mark` split | Task 1 |
| §4.2 conservative child prediction | Task 1 (`child_ch` + doc), pinned Task 3 Step 1 |
| §4.3 functions touched; `may_abut` untouched | Task 1; Global Constraints |
| §4.4 threading `parent_ch` | Task 2 |
| §5.3 `probe_edges` invariant in debug | Task 4 Steps 1–2 |
| §5.4 no global in the seam | Task 2 (parameter, not atomic) |
| §6 census + ratchet motion | Task 3 Step 5, Task 4 Step 3 |
| §7 tests 1–7 | Task 3 Step 1 (1–5), Task 3 Step 5 (6), Task 4 (7) |
| §8 doc corrections | Task 3 Step 6 (2 of them), Task 5 (3 of them) |
| §9 non-goals | no task, by design |

**Placeholder scan.** No TBD/TODO; every code step carries the actual code; no "similar to Task N".

**Type consistency.** `escape::Mark::new` / `.markup()` / `.ch` / `.class` used identically in Tasks 1–3. `Delim::child_ch` named the same at all four use sites. `choose_mark`'s six-parameter signature is identical in Task 2's stub, Task 2's test, and Task 3's implementation. `parent_ch: char` with `'\0'` sentinel is consistent across `inlines_to_md_flat`, `emphasis_run`, `probe_edges`.

**One gap found and closed while reviewing:** Task 2's interface block originally omitted that `emphasis_run` *loses* its `markup: &str` parameter. An implementer reading only Task 2 would have added `parent_ch` to a nine-parameter list and left a dead `markup` argument that Task 3 would then silently contradict. It is now stated in the Interfaces block and in Step 3.
