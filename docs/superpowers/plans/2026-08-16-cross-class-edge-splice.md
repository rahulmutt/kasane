# Cross-Class Edge Splice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop `splice_children`'s edge rule from destroying an `Emph` run whose entire content is a `Strong`, and file the mirror-image shape — which `*` alone cannot spell — as permanently inexpressible rather than queued.

**Architecture:** `edge_to_splice` is handed the run's `Delim` instead of its bare character, so it can tell the one safe configuration from the unsafe ones, and declines the splice when the candidate is the run's sole printing child, is a different class, and the run is an `Emph`. That makes `Emph[Strong[x]]` print `***x***`, which round-trips. `Strong[Emph[x]]` keeps splicing because `***x***` always resolves em-outermost, so the census relation gains a second, directional inexpressibility condition that moves it to the permanent file.

**Tech Stack:** Rust (stable 1.97.1, pinned in `mise.toml`), `pulldown-cmark` 0.13 as the census oracle and test parser, `mise` as the task runner.

**Spec:** `docs/superpowers/specs/2026-08-16-cross-class-edge-splice-design.md`

## Global Constraints

- Per-task checks are `mise run lint && mise run test`. `lint` is `cargo fmt --all -- --check` plus `cargo clippy --workspace --all-targets -- -D warnings`. Plain `cargo clippy` is not sufficient — `--all-targets` is what reaches test code.
- Regenerate all three census files with one command: `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census`. Never hand-edit `census-known-structure-corrupt.txt` or `census-inexpressible.txt`.
- **The text allowlist `census-known-corrupt.txt` must stay at exactly 32 lines, or shrink.** If it grows by even one line, stop — the approach is wrong, not the allowlist. This is checked before the structural numbers in every task that blesses.
- No `Inline` may be cloned in the writer's splice path; `flatten_into` borrows and `Vec::splice` shuffles `Flat` pairs (`2026-08-15-emphasis-seam-design.md` §2.2).
- Branch is `cross-class-edge-splice`, already created, with the design spec committed at `cf6158d`.

---

### Task 1: Narrow the edge rule

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs:508-516` (`edge_to_splice`), `:586-604` (`splice_children`), `:531-585` (its doc comment), and the `#[cfg(test)]` module near `:1593`
- Modify: `AGENTS.md:325-329`
- Bless: `crates/kasane-writer/tests/census-known-structure-corrupt.txt`

**Interfaces:**
- Consumes: `escape::Delim` and `escape::Delim::ch` (`escape.rs:517-543`), `escape::delim` (`escape.rs:558`), `renders_empty(i: &Inline, depth: usize) -> bool` (`markdown.rs:404`), `type Flat<'a> = (&'a Inline, usize)` (`markdown.rs:210`)
- Produces: `fn edge_to_splice(children: &[Flat<'_>], want: escape::Delim) -> Option<usize>` (signature changed from `ch: char`), `fn sole_child_nests_canonically(children: &[Flat<'_>], idx: usize, want: escape::Delim) -> bool`, and a test helper `fn recovered_html(inls: Vec<Inline>) -> String` that Task 3 does not use but a later reader will

- [ ] **Step 1: Add the `recovered_html` test helper**

`recovered` (`markdown.rs:1606`) returns only text, so it cannot see the `<strong>` this task restores. Add this beside it, inside the same `#[cfg(test)] mod tests`:

```rust
    /// The HTML a real parser builds from what the writer printed.
    ///
    /// `recovered` concatenates `Event::Text` and `Event::Code` and so is blind
    /// to structure by construction — which is the whole defect this task
    /// closes. The `<p>` wrapper is trimmed so a case reads as the inline
    /// shape it is about.
    fn recovered_html(inls: Vec<Inline>) -> String {
        use pulldown_cmark::{html, Options, Parser};

        let md = blocks_to_markdown(&[Block::Para(inls)], &AssetBag::default());
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_MATH);
        let mut out = String::new();
        html::push_html(&mut out, Parser::new_ext(&md, opts));
        out.trim()
            .trim_start_matches("<p>")
            .trim_end_matches("</p>")
            .to_string()
    }
```

- [ ] **Step 2: Write the four tests**

Add to the same test module:

```rust
    /// The defect this task closes. `Emph` wrapping nothing but a `Strong`
    /// prints `***a***`, and a parser splitting that run resolves it
    /// em-outermost — exactly what the IR meant. The old edge rule spliced the
    /// `Strong` away because `Delim::ch()` maps both classes to `*`, and the
    /// `<strong>` vanished from a document converter's output
    /// (`2026-08-16-cross-class-edge-splice-design.md` §1).
    #[test]
    fn an_emph_run_wrapping_only_a_strong_keeps_its_strong() {
        let inls = vec![Inline::Emph(vec![Inline::Strong(vec![Inline::Text(
            "a".into(),
        )])])];
        assert_eq!(para(inls.clone()), "***a***");
        assert_eq!(recovered_html(inls.clone()), "<em><strong>a</strong></em>");
        assert_eq!(recovered(inls), "a");
    }

    /// The cost, pinned so a later reader meets it as a decision. The converse
    /// shape prints the same `***a***`, and the tie-break resolves it
    /// em-outermost *against* the IR, so there is no `*`-only spelling of
    /// `<strong><em>a</em></strong>` — it keeps splicing and Task 2 files it
    /// permanent (design spec §2).
    #[test]
    fn a_strong_run_wrapping_only_an_emph_still_loses_its_emph() {
        let inls = vec![Inline::Strong(vec![Inline::Emph(vec![Inline::Text(
            "a".into(),
        )])])];
        assert_eq!(para(inls.clone()), "**a**");
        assert_eq!(recovered_html(inls.clone()), "<strong>a</strong>");
        assert_eq!(recovered(inls), "a");
    }

    /// The control for `sole_child_nests_canonically`'s second condition. A
    /// same-class sole child is *not* the canonical nesting and must keep
    /// splicing; `same_delim_to_splice` would catch it anyway, which is
    /// precisely why the predicate states the condition rather than relying on
    /// the other rule's ordering (design spec §3.2).
    #[test]
    fn an_emph_run_wrapping_only_an_emph_is_still_spliced() {
        let inls = vec![Inline::Emph(vec![Inline::Emph(vec![Inline::Text(
            "a".into(),
        )])])];
        assert_eq!(para(inls.clone()), "*a*");
        assert_eq!(recovered(inls), "a");
    }

    /// The exemption composes under nesting without a special case, and the
    /// census alphabet cannot reach this shape — every alphabet container is
    /// single-child to a depth of two, so `Emph[Strong[Emph[b]]]` is tested
    /// here or nowhere (design spec §3.3).
    ///
    /// The outer `Emph` run declines and keeps its `Strong`; the inner
    /// `Strong` run does not qualify — condition 3 fails — so it splices its
    /// `Emph` and prints `**b**`; the whole prints `***b***`. The innermost
    /// `Emph` is lost, but that is the `Strong`-outer limit reappearing one
    /// level down, not a new corruption, and Task 2 files the shape permanent
    /// on the strength of the `Strong[Emph[…]]` it contains. A `****b****`
    /// here would mean the exemption is recursing where it must not.
    #[test]
    fn the_exemption_composes_one_level_down() {
        let inls = vec![Inline::Emph(vec![Inline::Strong(vec![Inline::Emph(
            vec![Inline::Text("b".into())],
        )])])];
        assert_eq!(para(inls.clone()), "***b***");
        assert_eq!(recovered_html(inls.clone()), "<em><strong>b</strong></em>");
        assert_eq!(recovered(inls), "b");
    }
```

- [ ] **Step 3: Run the tests to verify the right two fail**

Run: `cargo test -p kasane-writer --lib`

Expected, precisely:

| test | now | why |
|---|---|---|
| `an_emph_run_wrapping_only_a_strong_keeps_its_strong` | **FAIL** | `para` returns `"*a*"`, not `"***a***"` |
| `the_exemption_composes_one_level_down` | **FAIL** | `para` returns `"*b*"`, not `"***b***"` |
| `a_strong_run_wrapping_only_an_emph_still_loses_its_emph` | PASS | pins behaviour that must not change |
| `an_emph_run_wrapping_only_an_emph_is_still_spliced` | PASS | pins behaviour that must not change |

If either of the two PASS rows fails now, stop: the baseline is not what this plan assumes.

- [ ] **Step 4: Give `edge_to_splice` the `Delim`**

Replace `markdown.rs:508-516` with:

```rust
fn edge_to_splice(children: &[Flat<'_>], want: escape::Delim) -> Option<usize> {
    let ch = want.ch();
    let printing = |&(i, d): &Flat<'_>| !renders_empty(i, d);
    let first = children.iter().position(printing);
    let last = children.iter().rposition(printing);
    [first, last].into_iter().flatten().find(|&idx| {
        escape::delim(children[idx].0).map(escape::Delim::ch) == Some(ch)
            && !sole_child_nests_canonically(children, idx, want)
    })
}
```

Replace its doc comment (`markdown.rs:500-507`) with:

```rust
/// The index of a leading or trailing printing element that collides at an
/// edge, sharing a character with the run's own delimiter, or `None` when
/// neither edge does — or when the one edge that would collide is the
/// canonical nesting [`sole_child_nests_canonically`] exempts.
///
/// Takes the run's `Delim` rather than its bare character even though the
/// collision *test* is still on the character. The exemption is the reason:
/// `*` and `**` abut identically, and only the class tells the shape that
/// round-trips from the one that does not.
///
/// One of two sources [`splice_children`] draws splice candidates from — see
/// its doc for why this one tests the character and stops at the edges, while
/// [`same_delim_to_splice`] is keyed on the `Delim` and looks everywhere.
```

- [ ] **Step 5: Add the exception predicate**

Insert directly after `edge_to_splice`:

```rust
/// Whether the edge candidate at `idx` is the run's entire content and nests
/// the one way `*` alone can spell.
///
/// `Emph` wrapping nothing but a `Strong` prints `***x***`, and a parser
/// splitting that run resolves it em-outermost — which is what the IR meant, so
/// splicing would destroy a shape that round-trips. The converse does not hold:
/// `Strong` wrapping nothing but an `Emph` prints the same `***x***` and
/// resolves the same way, *against* the IR, so it keeps splicing and the census
/// files it inexpressible.
///
/// All three conditions are load-bearing. Without the class check this would
/// also decline for `Emph[Emph[x]]`, where the behaviour would stay correct
/// only because `same_delim_to_splice` catches that shape a moment later —
/// true by the ordering of two other rules rather than by construction. The
/// single-printing-child check is what keeps this to the configuration the
/// census can prove: the wider single-edge cases (`*a**b***`) also round-trip,
/// but most of what that would license is unreachable by the census alphabet,
/// so it is deliberately left out (design spec §3.2 and §3.4).
fn sole_child_nests_canonically(
    children: &[Flat<'_>],
    idx: usize,
    want: escape::Delim,
) -> bool {
    if want != escape::Delim::Emph || escape::delim(children[idx].0) == Some(want) {
        return false;
    }
    let printing = |&(i, d): &Flat<'_>| !renders_empty(i, d);
    children
        .iter()
        .enumerate()
        .all(|(i, c)| i == idx || !printing(c))
}
```

- [ ] **Step 6: Update the caller**

In `splice_children` (`markdown.rs:586-590`), delete the `let ch = want.ch();` line and pass `want`:

```rust
fn splice_children<'a>(mut children: Vec<Flat<'a>>, want: escape::Delim) -> Vec<Flat<'a>> {
    while let Some(idx) =
        edge_to_splice(&children, want).or_else(|| same_delim_to_splice(&children, want))
    {
```

The rest of the loop body is unchanged.

- [ ] **Step 7: Extend `splice_children`'s doc comment**

In the first bullet of the doc comment (`markdown.rs:535-542`), after the sentence ending "which is why this rule only ever looks at the first and last printing element (design spec § Confirmed).", append:

```
///   One configuration is exempt: an `Emph` run whose *entire* content is a
///   single `Strong` prints `***x***`, which is the canonical spelling of
///   that nesting and round-trips — the merged run's tie-break resolves
///   em-outermost, which is what the IR meant. See
///   [`sole_child_nests_canonically`], and
///   `2026-08-16-cross-class-edge-splice-design.md` §2 for the measurements
///   that draw the boundary. The exemption is directional: `Strong` over
///   `Emph` prints the same bytes and the same tie-break destroys it, so it
///   keeps splicing.
```

Then, in the "These are two rules and not one rule with an inconsistent key" paragraph (`markdown.rs:566-573`), after the sentence ending "and dropping the `Delim` rule would leave this task's shapes unclosed.", append:

```
/// The edge rule now takes a `Delim` too, which does not merge the two: it
/// still *tests* the character, and takes the class only to recognise the one
/// shape where an abutting `*` and `**` are the correct spelling rather than a
/// collision.
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p kasane-writer --lib`

Expected: PASS, including both `an_emph_run_wrapping_only_a_strong_keeps_its_strong` and `the_exemption_composes_one_level_down`. Also expected to still pass, unchanged: `fusing_nested_emphasis_does_not_leak_its_delimiters` (every case has two printing children, or `want == Strong`) and `a_same_class_container_mid_buffer_is_spliced` (three printing children after fusion). **If either of those two moves, stop — that is evidence the predicate is wrong, not a test to re-bless** (design spec §5.4).

- [ ] **Step 9: Check the text tier before anything else**

Run: `cargo test -p kasane-writer --test census inline_text_survives_rendering_for_every_short_sequence`

Expected: PASS with `census-known-corrupt.txt` untouched. If it fails, the fix scrambled characters — stop and report; do not bless.

- [ ] **Step 10: Bless the structural queue and read the diff**

Run:

```bash
KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census
git diff --stat crates/kasane-writer/tests/
wc -l crates/kasane-writer/tests/census-*.txt
```

Expected: `census-known-corrupt.txt` still 32 lines. `census-known-structure-corrupt.txt` drops from 2,812 to roughly 1,866 — about 946 `Emph`-outer shapes are now clean. `census-inexpressible.txt` unchanged at 1,250 lines (1,236 entries plus its 14-line header); it does not move until Task 2.

Read the removed lines. Every one should contain `Emph([Strong(`. If a removed line contains only `Strong([Emph(`, the predicate's direction is inverted — stop.

- [ ] **Step 11: Update AGENTS.md**

`AGENTS.md:325-329` currently ends the census bullet with a sentence describing the 2,002-shape family as the largest *queued* one. Replace that sentence with:

```
  Design spec `2026-08-16-structural-census-design.md`; its §6 recorded the
  largest queued family, 2,002 shapes losing a level because
  `splice_children`'s edge rule keys on the delimiter character and
  `Delim::ch()` maps both classes to `*`. That family is closed:
  `2026-08-16-cross-class-edge-splice-design.md` narrows the edge rule so
  `Emph[Strong[x]]` prints `***x***`, and files the mirror shape permanent
  because `***x***` always resolves em-outermost.
```

- [ ] **Step 12: Run the full check**

Run: `mise run lint && mise run test`

Expected: both green.

- [ ] **Step 13: Commit**

```bash
git add crates/kasane-writer/src/markdown.rs \
        crates/kasane-writer/tests/census-known-structure-corrupt.txt \
        AGENTS.md
git commit -m "fix(writer): keep the strong inside an emph run that wraps only it

edge_to_splice keyed on the delimiter character, and Delim::ch() maps both
Emph and Strong to '*', so a Strong spanning an Emph run's whole content was
spliced away and *a* lost its <strong>. It now takes the run's Delim and
declines that one configuration, printing ***a*** -- the spelling whose
tie-break resolves em-outermost, which is what the IR meant.

Directional: Strong over Emph prints the same bytes and the same tie-break
destroys it, so it keeps splicing. Task 2 files it permanent.

Structural queue 2812 -> ~1866; text allowlist unchanged at 32."
```

---

### Task 2: File `Strong` over `Emph` as inexpressible

**Files:**
- Modify: `crates/kasane-writer/tests/census.rs:392-450` (condition 1, condition 2, `classify`), `:499-514` (`INEXPRESSIBLE_HEADER`), and the pinned-edge tests near `:362`
- Modify: `AGENTS.md:322-325`
- Bless: `crates/kasane-writer/tests/census-known-structure-corrupt.txt`, `crates/kasane-writer/tests/census-inexpressible.txt`

**Interfaces:**
- Consumes: `enum Structure { Clean, Corrupt, Inexpressible }` (`census.rs:382`), `enum Emphasis { Em, St }` (`census.rs:229`), `fn classify(seq: &[Inline]) -> Structure` (`census.rs:438`), `fn nests_same_class_directly(seq: &[Inline]) -> bool` (`census.rs:407`), and Task 1's narrowed edge rule — `[Emph([Strong([Text("a")])])]` must already be `Structure::Clean` before this task starts
- Produces: `fn nests_strong_over_emph_directly(seq: &[Inline]) -> bool`, `fn differs_only_by_erasure(ir: &[(char, Vec<Emphasis>)], got: &[(char, Vec<Emphasis>)]) -> bool` (replaces `differs_only_by_collapse`, which is deleted)

- [ ] **Step 1: Write the two failing pinned-edge tests**

Add after `the_structural_relation_marks_direct_same_class_nesting_inexpressible` (`census.rs:362-368`):

```rust
/// The second permanent mechanism, and the one this item adds.
///
/// `<strong><em>a</em></strong>` has no `*`-only spelling: `***a***` is the
/// only run that could carry both levels, and CommonMark's tie-break always
/// resolves it em-outermost. Spelling it needs `**_a_**`, and alternating `*`
/// with `_` is rejected by three specs. Permanent, not queued.
#[test]
fn the_structural_relation_marks_strong_over_emph_inexpressible() {
    let seq = vec![Inline::Strong(vec![Inline::Emph(vec![Inline::Text(
        "a".into(),
    )])])];
    assert_eq!(classify(&seq), Structure::Inexpressible);
}

/// The guard that matters most.
///
/// The converse shape *is* spellable — `***a***` — and is fixed in the writer
/// (`2026-08-16-cross-class-edge-splice-design.md` §3). This asserts it is
/// neither corrupt nor permanent, so it fails loudly if the fix regresses
/// *and* if condition 1 ever loses its direction and starts laundering the
/// fixed family into the permanent file.
#[test]
fn the_structural_relation_keeps_emph_over_strong_clean() {
    let seq = vec![Inline::Emph(vec![Inline::Strong(vec![Inline::Text(
        "a".into(),
    )])])];
    assert_eq!(classify(&seq), Structure::Clean);
}
```

- [ ] **Step 2: Run them to verify one fails and one passes**

Run: `cargo test -p kasane-writer --test census the_structural_relation_`

Expected: `the_structural_relation_marks_strong_over_emph_inexpressible` FAILS with `assertion \`left == right\` failed: left: Corrupt, right: Inexpressible`. `the_structural_relation_keeps_emph_over_strong_clean` PASSES — Task 1 already made it clean. The three pre-existing `the_structural_relation_*` tests pass.

- [ ] **Step 3: Add condition 1's second predicate**

Insert after `nests_same_class_directly` (`census.rs:418`):

```rust
/// Whether `seq` holds a `Strong` whose **sole** child is an `Emph`.
///
/// The second disjunct of condition 1 (design spec §4). Directional on
/// purpose: `***x***` always resolves em-outermost, so
/// `<strong><em>x</em></strong>` has no `*`-only spelling, while
/// `<em><strong>x</strong></em>` has one and the writer now prints it.
/// Matching both orders here would let a regression of the fixed family
/// launder itself into the permanent file, which is the one failure this
/// split must not have.
///
/// Scoped to the whole shape for the same reason its sibling is, and carrying
/// the same residual risk — see `2026-08-16-structural-census-design.md` §8,
/// and §7 of this item's spec for what a second predicate costs the
/// per-position conversion.
fn nests_strong_over_emph_directly(seq: &[Inline]) -> bool {
    seq.iter().any(|i| match i {
        Inline::Strong(x) => {
            matches!(x.as_slice(), [Inline::Emph(_)]) || nests_strong_over_emph_directly(x)
        }
        Inline::Emph(x) => nests_strong_over_emph_directly(x),
        Inline::Link { inlines, .. } => nests_strong_over_emph_directly(inlines),
        _ => false,
    })
}
```

- [ ] **Step 4: Replace condition 2**

Delete `differs_only_by_collapse` entirely (`census.rs:420-435`) — leaving it would be dead code and `clippy --all-targets -D warnings` will reject it — and put this in its place:

```rust
/// Whether every difference between the two walks disappears under the two
/// erasures `*` alone forces. Condition 2 of the split (design spec §4).
///
/// Two normalizations, not one: adjacent identical classes collapse
/// (`<em><em>x</em></em>` has no spelling), and an `Em` directly inside a `St`
/// is dropped (`<strong><em>x</em></strong>` has none either). Applied to both
/// walks and iterated to a fixpoint, because a stack can need both — `[St, Em,
/// Em]` collapses to `[St, Em]` and only then drops to `[St]`, and doing the
/// two steps once in the other order leaves `[St, Em]` and files a genuinely
/// unspellable shape corrupt.
///
/// A **drop**, not a reorder. The writer leaves `Strong[Emph[x]]` spliced, so
/// it prints `**x**` and a parser recovers `[St]` against an IR of `[St, Em]`
/// — the level is deleted, not swapped. Nothing prints `***x***` for a
/// `Strong`-outer shape, so a reorder normalization would never fire.
///
/// The drop's direction is half the laundering guard. If the writer regresses
/// and `Emph[Strong[x]]` loses its `<strong>`, the stacks are `[Em, St]`
/// against `[Em]`; this drops an `Em` that follows a `St`, not a `St` that
/// follows an `Em`, so the walks stay unequal and the shape lands in the
/// queue where the ratchet fails the build.
fn differs_only_by_erasure(ir: &[(char, Vec<Emphasis>)], got: &[(char, Vec<Emphasis>)]) -> bool {
    fn normalize(v: &[Emphasis]) -> Vec<Emphasis> {
        let mut cur = v.to_vec();
        loop {
            let mut out: Vec<Emphasis> = Vec::new();
            for &e in &cur {
                match out.last() {
                    Some(&last) if last == e => {}
                    Some(&Emphasis::St) if e == Emphasis::Em => {}
                    _ => out.push(e),
                }
            }
            if out == cur {
                return out;
            }
            cur = out;
        }
    }
    ir.iter()
        .zip(got)
        .all(|(x, y)| normalize(&x.1) == normalize(&y.1))
}
```

- [ ] **Step 5: Widen `classify`**

In `classify` (`census.rs:446-448`), replace the inexpressible arm:

```rust
    if (nests_same_class_directly(seq) || nests_strong_over_emph_directly(seq))
        && differs_only_by_erasure(&ir, &got)
    {
        return Structure::Inexpressible;
    }
```

- [ ] **Step 6: Run the pinned edges to verify they pass**

Run: `cargo test -p kasane-writer --test census the_structural_relation_`

Expected: all five PASS.

- [ ] **Step 7: Rewrite `INEXPRESSIBLE_HEADER`**

Replace `census.rs:499-514` with:

```rust
const INEXPRESSIBLE_HEADER: &str = "\
# Shapes whose structure Markdown cannot express, at any level.
#
# Two mechanisms, both forced by spelling emphasis with `*` alone:
#
#   `<em><em>x</em></em>`         -- `**x**` is strong, not nested emphasis.
#   `<strong><em>x</em></strong>` -- `***x***` is the only run that could carry
#                                    both levels, and CommonMark's tie-break
#                                    always resolves it em-outermost.
#
# The converse of the second, `<em><strong>x</strong></em>`, IS spellable and
# is not here -- the writer prints `***x***` for it. That asymmetry is what
# keeps a regression of the fixed family out of this file.
#
# No writer change can close these, which is why they are not in the queue
# (`census-known-structure-corrupt.txt`).
#
# COMPUTED, never hand-edited. A shape lands here only if it BOTH nests one of
# the two shapes above directly AND differs from the IR only by collapsing
# adjacent identical classes and dropping an emphasis directly inside a strong.
# Stop satisfying either and it moves back to the queue on the next bless. See
# `docs/superpowers/specs/2026-08-16-cross-class-edge-splice-design.md` §4.
#
# Regenerate: KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census
";
```

- [ ] **Step 8: Check the text tier, then bless**

Run:

```bash
cargo test -p kasane-writer --test census inline_text_survives_rendering_for_every_short_sequence
KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census
wc -l crates/kasane-writer/tests/census-*.txt
```

Expected: `census-known-corrupt.txt` still 32. `census-known-structure-corrupt.txt` drops from ~1,866 to roughly 810. `census-inexpressible.txt` grows from 1,236 entries to roughly 2,292.

If the permanent file lands materially above ~2,292, the whole-shape scoping of condition 1 is the first place to look (design spec §7, final bullet) — report the number rather than accepting it silently.

- [ ] **Step 9: Verify no queue entry was laundered**

Run:

```bash
grep -c 'Emph(\[Strong(' crates/kasane-writer/tests/census-inexpressible.txt
```

Expected: some non-zero count is fine — a shape can hold both orders. What must hold is that every such line **also** contains `Strong([Emph(`. Verify:

```bash
grep 'Emph(\[Strong(' crates/kasane-writer/tests/census-inexpressible.txt \
  | grep -vc 'Strong(\[Emph('
```

Expected: `0`. A non-zero count means an `Emph`-outer shape reached the permanent file without a `Strong`-outer shape to justify it — stop and report.

- [ ] **Step 10: Update AGENTS.md**

In `AGENTS.md:322-325`, the sentence beginning "The split between those two files is **computed on every bless, never hand-edited**" describes only the same-class rule. Replace its trailing clause with:

```
  a shape is permanent only if it both nests one of the two unspellable shapes
  directly — `<em><em>x</em></em>` or `<strong><em>x</em></strong>` — and
  differs from the IR only by collapsing adjacent identical classes and
  dropping an emphasis directly inside a strong. The asymmetry is deliberate:
  `<em><strong>x</strong></em>` IS spellable, and keeping it out of the
  permanent file is what stops a regression laundering itself as a
  representational limit.
```

- [ ] **Step 11: Run the full check**

Run: `mise run lint && mise run test`

Expected: both green.

- [ ] **Step 12: Commit**

```bash
git add crates/kasane-writer/tests/census.rs \
        crates/kasane-writer/tests/census-known-structure-corrupt.txt \
        crates/kasane-writer/tests/census-inexpressible.txt \
        AGENTS.md
git commit -m "test(writer): file strong-over-emph as inexpressible

***x*** is the only run that could carry both levels and CommonMark's
tie-break always resolves it em-outermost, so <strong><em>x</em></strong>
has no *-only spelling. It was sitting in a target-zero queue it can never
leave.

Condition 1 gains a directional predicate and condition 2 gains a drop --
an Em directly inside a St -- iterated to a fixpoint alongside the existing
collapse. The direction is the laundering guard: a regression of the fixed
Emph[Strong[x]] family differs by a deletion neither normalization touches,
so it lands in the queue and fails the build.

Queue ~1866 -> ~810; permanent 1236 -> ~2292; text allowlist unchanged at 32."
```

---

### Task 3: Close the inline-depth risk

**Files:**
- Modify: `crates/kasane-writer/tests/inline_depth.rs`

**Interfaces:**
- Consumes: `fn doc_with(inline: Inline) -> Document` (`inline_depth.rs:18`), `kasane_core::structure`, `kasane_writer::blocks_to_markdown`
- Produces: `fn nested_alternating(depth: usize) -> Inline`

Declining a splice keeps a container the old rule flattened, so cross-class chains now render one level deeper than before. `inline_depth.rs`'s existing `nested` helper builds `Emph[Emph[Emph[…]]]` — same-class only, entirely spliced by `same_delim_to_splice`, and therefore blind to this change. The risk the spec records (§7, second bullet) has no test.

- [ ] **Step 1: Write the failing tests**

Add to `inline_depth.rs`:

```rust
/// Alternating classes, which the splice rules no longer flatten all the way.
///
/// `nested` above builds same-class nesting, every level of which
/// `same_delim_to_splice` removes — so it cannot see the depth this writer
/// actually recurses to. Since
/// `2026-08-16-cross-class-edge-splice-design.md` §3, an `Emph` wrapping only
/// a `Strong` survives, so an alternating chain keeps levels a same-class
/// chain never did.
fn nested_alternating(depth: usize) -> Inline {
    let mut i = Inline::Text("x".into());
    for n in 0..depth {
        i = if n % 2 == 0 {
            Inline::Strong(vec![i])
        } else {
            Inline::Emph(vec![i])
        };
    }
    i
}

#[test]
fn deep_cross_class_nesting_does_not_abort() {
    let site = structure(doc_with(nested_alternating(10_000)), &Options::default());
    let md = kasane_writer::blocks_to_markdown(&site.files[0].blocks, &AssetBag::default());
    assert!(!md.is_empty(), "rendering must produce output, not abort");
}

#[test]
fn cross_class_nesting_within_the_bound_is_preserved() {
    // Depth 8 is far under the bound: the text at the bottom must survive.
    let site = structure(doc_with(nested_alternating(8)), &Options::default());
    let md = kasane_writer::blocks_to_markdown(&site.files[0].blocks, &AssetBag::default());
    assert!(
        md.contains('x'),
        "content within the bound must not be dropped"
    );
}
```

- [ ] **Step 2: Run them**

Run: `cargo test -p kasane-writer --test inline_depth`

Expected: PASS. These are characterization tests for a risk, not a red-green cycle — there is no production change in this task. If `deep_cross_class_nesting_does_not_abort` **aborts or overflows the stack**, that is the §7 depth risk realized: stop, report, and treat it as a defect in Task 1's predicate rather than adjusting the bound.

- [ ] **Step 3: Run the full check**

Run: `mise run lint && mise run test`

Expected: both green.

- [ ] **Step 4: Commit**

```bash
git add crates/kasane-writer/tests/inline_depth.rs
git commit -m "test(writer): pin depth on cross-class inline nesting

The existing helper nests one class, every level of which
same_delim_to_splice removes, so it never exercised the depth the writer
recurses to. Declining the edge splice keeps a container that used to be
flattened, so an alternating chain is now genuinely deeper -- which is the
residual risk the design spec's section 7 records, and had no test."
```

---

### Task 4: Record the measured result in the spec

**Files:**
- Modify: `docs/superpowers/specs/2026-08-16-cross-class-edge-splice-design.md` (§5.1 table, §7 opening, and the `**Status:**` line)

Every prior item in this program records what the instrument actually measured back into its spec — `2026-08-15-emphasis-seam-design.md` §8 carries a result note, and the 2a spec's §5 is titled "What the probe measured". A spec whose predicted numbers are left standing after the bless is a spec that quietly diverges from the repo.

- [ ] **Step 1: Read the real numbers**

Run:

```bash
wc -l crates/kasane-writer/tests/census-*.txt
grep -c . crates/kasane-writer/tests/census-known-structure-corrupt.txt
grep -vc '^#' crates/kasane-writer/tests/census-inexpressible.txt
```

- [ ] **Step 2: Replace §5.1's predicted table with the measured one**

Change the sentence "Predicted movement, **confirmed at bless and not asserted in advance**:" to "Measured at bless:", and replace the `~` figures in the table's "after" column with the exact counts from Step 1. Add one line below the table stating the number of shapes that went clean, computed as `2,812 − (queue after) − (permanent after − 1,236)`.

- [ ] **Step 3: Resolve §7's uncertainty bullet**

§7's final bullet ends "If §5.1's permanent count comes back above ~2,292, this is the first place to look." Replace that sentence with what actually happened — either that the count landed at the predicted figure and the hazard stayed unreached, or the exact overshoot and what Task 2 Step 9's grep showed about it.

- [ ] **Step 4: Update the status line**

Change `**Status:** Designed, not implemented.` to `**Status:** Implemented on branch \`cross-class-edge-splice\`.`

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-08-16-cross-class-edge-splice-design.md
git commit -m "docs: record what the cross-class edge splice fix measured"
```

---

## Verification Before Opening a PR

- [ ] `mise run lint && mise run test` green from a clean tree.
- [ ] `crates/kasane-writer/tests/census-known-corrupt.txt` is byte-identical to its state at `8b9d05e`: `git diff 8b9d05e -- crates/kasane-writer/tests/census-known-corrupt.txt` prints nothing.
- [ ] All five `the_structural_relation_*` tests pass.
- [ ] `git log --oneline main..HEAD` shows five commits: the design spec, then Tasks 1–4.
- [ ] The plan's own residuals — the single-edge configurations (§3.4), the ~810 undiagnosed queue tail, and the CI ratchet the spec's §7 argues should land next — are named in the PR description rather than left implicit.
