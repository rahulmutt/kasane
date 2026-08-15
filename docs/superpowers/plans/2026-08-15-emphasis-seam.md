# Emphasis Seam Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the writer emitting an emphasis delimiter that lands against another delimiter or against content punctuation, so the text a parser recovers is the text the IR held.

**Architecture:** One rule at two seams — a run's flattened children are trimmed at both edges while the outermost printing element is a container whose delimiter shares a character with the run's own, and two adjacent runs sharing a delimiter character are one run. A second, separate rule moves the decision of *whether* a delimiter can be spelled at all into the scan, which is the only place that can see both the character before the run and the one after it. `emphasize` does not change; it is simply not called with delimiters when they cannot be spelled.

**Tech Stack:** Rust (stable 1.97.1, pinned in `mise.toml`), `proptest` 1.11, `pulldown-cmark` 0.13 as the test oracle. Tasks touch `kasane-writer`, plus one test move into `kasane-adapters` and out of `kasane-cli`.

**Spec:** `docs/superpowers/specs/2026-08-15-emphasis-seam-design.md`

## Global Constraints

- Branch `adjacent-inline-fusion` already exists and holds the spec commit. Work on it; do not branch again and do not commit to `main`. This item must land before that branch merges — it closes a regression that branch introduced.
- Per-task checks are `mise run lint` and `mise run test`. `lint` is `cargo fmt --all -- --check` plus `cargo clippy --workspace --all-targets -- -D warnings`; a plain `cargo clippy` is **not** enough, because most new code in this plan lives in test targets.
- `-D warnings` means an unused item fails the build. Every item this plan adds is consumed in the same task that adds it; do not stage an interface ahead of its consumer.
- Rust edition and style come from `rustfmt.toml`; run `cargo fmt --all` before every commit rather than hand-aligning.
- Commit messages follow the repo's existing form: `fix(writer): …`, `test(writer): …`, `test(adapters): …`, `docs: …`.
- **The invariant, stated once:** the text a parser recovers from the writer's Markdown equals `kasane_gfm::rendered_text` of the same inlines. Emphasis *structure* is expendable at a colliding seam and text is not (spec §1, "The trade this item makes").
- Do not change `kasane-gfm` or `kasane-core`. Do not change `emphasize`, `escape::code_span` or `escape::math_span` — spec §3 keeps all three out of scope.
- **No `Inline` is cloned or rewritten anywhere.** Every view this plan builds holds `&Inline` pointers. `Inline`'s derived `Clone` recurses once per nesting level, so a hand-built tree past the bound would overflow the stack inside the clone before the depth guard could discard it (`2026-08-15-adjacent-inline-fusion-design.md` §2.2).
- Task 2's census allowlist is re-blessed by Tasks 3, 4 and 5. **A task that re-blesses must show the allowlist shrinking in its diff** — that shrinkage is the task's evidence, and an unchanged allowlist means the fix did nothing.

---

### Task 1: Move the inline-depth assertion off the writer's bytes

`kasane-cli/tests/e2e.rs` proves the EPUB adapter's inline-flattening bound reached the CLI path by counting `*` characters in writer output. It is an adapter property observed through writer bytes, and it is the only thing forcing the `nests_alone` carve-out Task 3 deletes: under the edge trim, `Emph([Emph([Text("a")])])` prints `*a*`, one asterisk rather than the stack this assertion counts. Both spellings are text-correct, so nothing wrong is pinned today — but 64 stacked `*` are read as 32 nested `<strong>`, a semantic the IR never held (spec §2.4).

The assertion moves to where the property lives. The CLI test keeps everything that is genuinely about the CLI path.

**Files:**
- Modify: `crates/kasane-adapters/src/epub/mod.rs` (add one test to the existing `#[cfg(test)] mod tests` at `:516`)
- Modify: `crates/kasane-cli/tests/e2e.rs:23-66` (`converts_a_deeply_nested_epub_without_aborting`)

**Interfaces:**
- Consumes: `crate::Adapter` (trait, `lib.rs:48`), `super::EpubAdapter`, `super::xhtml::MAX_INLINE_DEPTH` (`pub(crate)`, value 64), `kasane_ir::{Block, Inline}` — all already in scope in that module, which has `use kasane_ir::*;` at the file head.
- Produces: nothing later tasks depend on. Task 3 depends only on this task having *landed*, not on any name it defines.

- [ ] **Step 1: Write the moved test**

Add to `crates/kasane-adapters/src/epub/mod.rs`'s `#[cfg(test)] mod tests`:

```rust
    /// The inline-flattening bound holds on a real EPUB read through the real
    /// adapter, not only on a hand-built XML string.
    ///
    /// `fuzz/seeds/epub/deep-nesting.epub` nests `<em>` 5000 deep. The unit
    /// test above covers `parse_blocks` on a 300-deep string; this one covers
    /// the zip, the OPF, the spine and the XHTML parser together, which is
    /// what the seed exists for.
    ///
    /// This assertion used to live in `kasane-cli/tests/e2e.rs`, where it
    /// counted `*` characters in the *writer's* output. That coupled an
    /// adapter bound to a writer spelling: it forced a carve-out into the
    /// writer's central emphasis rule to keep 64 stacked `*` printing, a
    /// spelling a parser reads as 32 nested `<strong>` — a structure the IR
    /// never held (`2026-08-15-emphasis-seam-design.md` §2.4). Reading the
    /// depth off the parsed IR asserts the property directly and leaves the
    /// writer free to spell it however it likes.
    #[test]
    fn the_inline_depth_bound_holds_on_a_real_epub() {
        fn depth_of(inls: &[Inline]) -> usize {
            inls.iter()
                .map(|i| match i {
                    Inline::Emph(x) | Inline::Strong(x) => 1 + depth_of(x),
                    Inline::Link { inlines, .. } => 1 + depth_of(inlines),
                    _ => 0,
                })
                .max()
                .unwrap_or(0)
        }

        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fuzz/seeds/epub/deep-nesting.epub"
        ))
        .expect("the deep-nesting seed must exist");
        let (doc, _) = EpubAdapter
            .parse(&bytes, "deep-nesting.epub")
            .expect("the seed must parse");

        let inls = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(i) => Some(i),
                _ => None,
            })
            .expect("the chapter must survive as a paragraph, not be dropped");

        let depth = depth_of(inls);
        assert!(
            depth <= xhtml::MAX_INLINE_DEPTH,
            "5000 nested <em> must flatten to the bound, got {depth}"
        );
        // A bound that flattened everything to nothing would also satisfy the
        // assertion above. The seed is 5000 deep, so real nesting must survive.
        assert!(depth > 1, "flattening must keep nesting, got {depth}");
    }
```

If `doc.blocks` is not the field name on `Document`, find it with
`grep -n "pub struct Document" -A 10 crates/kasane-ir/src/*.rs` and use the
field that holds the block list; nothing else in the test changes.

- [ ] **Step 2: Run the moved test**

Run: `cargo test -p kasane-adapters the_inline_depth_bound_holds_on_a_real_epub`

Expected: PASS. It is green on arrival — the bound already works; this task moves where it is asserted, it does not change behaviour.

- [ ] **Step 3: Cut the `*`-counting half out of the CLI test**

In `crates/kasane-cli/tests/e2e.rs`, in `converts_a_deeply_nested_epub_without_aborting`, delete the `stars_before` binding and the `assert!(stars_before >= 64, …)` that follows it, and replace the comment block above the `chapter` binding with:

```rust
    // `title: Deep Nesting` comes from the OPF's <dc:title>, not the chapter
    // -- it is present even in the exact bug this seed exists to catch (the
    // chapter silently dropped by guard.rs's zip-bomb ratio guard, book
    // parsing to zero body nodes), so it alone proves nothing about the
    // chapter surviving. Read the chapter file and check its actual content:
    // "bottom" must be present, which proves the chapter was not dropped.
    //
    // The flattening bound itself is asserted in `kasane-adapters`
    // (`epub::tests::the_inline_depth_bound_holds_on_a_real_epub`), against
    // the parsed IR rather than against a `*` run in this file's output. It
    // is an adapter property, and reading it through writer bytes coupled it
    // to a writer spelling (`2026-08-15-emphasis-seam-design.md` §2.4).
```

Keep everything else: the conversion, the `status.success()` assertion, the
`index.md` title check, and the `chapter.contains("bottom")` assertion.

- [ ] **Step 4: Run the CLI test**

Run: `cargo test -p kasane-cli converts_a_deeply_nested_epub_without_aborting`

Expected: PASS.

- [ ] **Step 5: Run the full checks**

Run: `mise run lint && mise run test`

Expected: both green. No writer behaviour changed in this task.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-adapters/src/epub/mod.rs crates/kasane-cli/tests/e2e.rs
git commit -m "test(adapters): assert the inline-depth bound on the IR, not on writer bytes"
```

---

### Task 2: Commit the census, blessed at today's state

The whole-branch review found every defect in this item by rendering every inline sequence of length 1-3 over a fixed alphabet and comparing the recovered text against `rendered_text`. Six whole-pipeline properties and three review rounds had missed all of them; one census pass found them all. This task turns that throwaway probe into a committed ratchet (spec §5.1).

It is blessed at the **current** state, so it lands green with an allowlist naming every shape that is corrupt today. Tasks 3, 4 and 5 each shrink that file, and the shrinkage is their evidence.

**Files:**
- Create: `crates/kasane-writer/tests/census.rs`
- Create: `crates/kasane-writer/tests/census-known-corrupt.txt` (generated in Step 3, committed)

**Interfaces:**
- Consumes: `kasane_writer::blocks_to_markdown`, `kasane_gfm::rendered_text`, `pulldown_cmark` — all already dependencies of this crate's test targets.
- Produces: `crates/kasane-writer/tests/census-known-corrupt.txt`, the allowlist Tasks 3-5 shrink. Its format is one `{:?}` line per corrupt shape, sorted.

- [ ] **Step 1: Write the census**

Create `crates/kasane-writer/tests/census.rs`:

```rust
//! An exhaustive differential census of short inline sequences.
//!
//! Renders every sequence of length 1-3 over the alphabet below, parses the
//! result, and compares the recovered text against `kasane_gfm::rendered_text`
//! of the same inlines — the same equality `p13_inline_text_survives_rendering`
//! asserts, exhaustive instead of sampled.
//!
//! This is the instrument that found the emphasis-seam defects
//! (`2026-08-15-emphasis-seam-design.md` §1). Six whole-pipeline properties and
//! three review rounds missed shapes it finds in one pass, because a property
//! draws from an alphabet someone chose and a census draws from all of it.
//!
//! # The allowlist is a ratchet, not an acceptance
//!
//! `census-known-corrupt.txt` names the shapes that are corrupt today. A
//! corrupt shape *absent* from it fails, so a regression cannot ship quietly. A
//! listed shape that is *no longer* corrupt also fails, so the file cannot rot
//! into a set of stale excuses — fixing a family means deleting lines from it.
//!
//! Regenerate with `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test
//! census`, and read the diff: it is the exact list of shapes your change
//! fixed or broke, which is the evidence a reviewer wants.
//!
//! # Why this alphabet
//!
//! Eighteen elements, chosen to put every delimiter class next to every other:
//! plain text, text that is itself a delimiter character, both code-span
//! classes (`Code`, and a `Math` that degrades to backticks), a `Math` that
//! does not degrade, each emphasis class alone and wrapping each of the
//! others, a transparent link both empty and delimiter-bearing, and a footnote
//! reference. `Inline::Code("")` is excluded: `code_span`'s Rule 1 prints
//! `` ` ` `` against `rendered_text`'s empty string, an acknowledged
//! divergence unreachable after `structure` and not what this census is about
//! — the same exclusion `P13_WORDS` documents.

use kasane_ir::{AssetBag, Block, BlockId, Inline, NoteId, RefTarget};
use pulldown_cmark::{Event, Options, Parser};
use std::collections::BTreeSet;

const ALLOWLIST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/census-known-corrupt.txt");

fn alphabet() -> Vec<Inline> {
    let t = |s: &str| Inline::Text(s.to_string());
    let em = |i: Inline| Inline::Emph(vec![i]);
    let st = |i: Inline| Inline::Strong(vec![i]);
    vec![
        t("a"),
        t("b"),
        t(" "),
        t("*"),
        Inline::Code("x".into()),
        Inline::Code("y".into()),
        Inline::Math("m".into()),
        Inline::Math("a$b".into()),
        em(t("a")),
        st(t("a")),
        em(Inline::Code("x".into())),
        st(Inline::Code("x".into())),
        em(em(t("a"))),
        st(em(t("a"))),
        em(st(t("a"))),
        Inline::Link {
            target: RefTarget::Internal(BlockId(0)),
            inlines: vec![Inline::Code("x".into())],
        },
        Inline::Link {
            target: RefTarget::Internal(BlockId(0)),
            inlines: vec![],
        },
        Inline::FootnoteRef(NoteId(1)),
    ]
}

/// The text a real parser recovers from `md`.
fn parsed_text(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_MATH);
    let mut out = String::new();
    for ev in Parser::new_ext(md, opts) {
        match ev {
            Event::Text(t) | Event::Code(t) | Event::InlineMath(t) | Event::DisplayMath(t) => {
                out.push_str(&t)
            }
            _ => {}
        }
    }
    out
}

/// Every sequence of length 1-3 over the alphabet.
fn shapes() -> Vec<Vec<Inline>> {
    let a = alphabet();
    let mut out: Vec<Vec<Inline>> = a.iter().map(|i| vec![i.clone()]).collect();
    for i in &a {
        for j in &a {
            out.push(vec![i.clone(), j.clone()]);
            for k in &a {
                out.push(vec![i.clone(), j.clone(), k.clone()]);
            }
        }
    }
    out
}

#[test]
fn inline_text_survives_rendering_for_every_short_sequence() {
    let mut corrupt = BTreeSet::new();
    for seq in shapes() {
        let md = kasane_writer::blocks_to_markdown(
            &[Block::Para(seq.clone())],
            &AssetBag::default(),
        );
        let recovered = parsed_text(&md);
        let expected = kasane_gfm::rendered_text(&seq);
        if recovered.trim() != expected.trim() {
            corrupt.insert(format!("{seq:?}"));
        }
    }

    if std::env::var_os("KASANE_CENSUS_BLESS").is_some() {
        let body: String = corrupt.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(ALLOWLIST, body).expect("writing the allowlist");
        return;
    }

    let known: BTreeSet<String> = std::fs::read_to_string(ALLOWLIST)
        .expect("tests/census-known-corrupt.txt must exist -- bless it with KASANE_CENSUS_BLESS=1")
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();

    let new: Vec<&String> = corrupt.difference(&known).collect();
    let fixed: Vec<&String> = known.difference(&corrupt).collect();

    assert!(
        new.is_empty(),
        "{} shape(s) newly corrupt. Each of these renders to text a parser \
         reads differently from `rendered_text`:\n{}",
        new.len(),
        new.iter().take(10).map(|s| format!("  {s}\n")).collect::<String>()
    );
    assert!(
        fixed.is_empty(),
        "{} allowlisted shape(s) are no longer corrupt -- delete them from \
         tests/census-known-corrupt.txt (KASANE_CENSUS_BLESS=1 does it for \
         you):\n{}",
        fixed.len(),
        fixed.iter().take(10).map(|s| format!("  {s}\n")).collect::<String>()
    );
}
```

If `parse_events` in `tests/properties.rs` already builds its `Options` set
differently, match that set here rather than the one above — the two must agree
about what the oracle is, or the census and P13 disagree about what "recovered"
means. Check with `grep -n "Options::" crates/kasane-writer/tests/properties.rs`.

- [ ] **Step 2: Run it and watch it fail for the stated reason**

Run: `cargo test -p kasane-writer --test census`

Expected: FAIL, panicking on the missing allowlist with
`tests/census-known-corrupt.txt must exist`. This is the file not existing yet,
not a defect.

- [ ] **Step 3: Bless the allowlist**

Run:

```bash
KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census
wc -l crates/kasane-writer/tests/census-known-corrupt.txt
```

Expected: the test passes (it returns early after writing), and the file holds
several hundred lines. Record the exact count in your report — Tasks 3, 4 and 5
are measured against it.

- [ ] **Step 4: Run it again, unblessed**

Run: `cargo test -p kasane-writer --test census`

Expected: PASS. The allowlist now matches reality exactly.

- [ ] **Step 5: Prove the ratchet bites in both directions**

The census is worthless if it cannot fail. Prove both halves:

```bash
# A shape that regresses must fail: delete a line and re-run.
head -1 crates/kasane-writer/tests/census-known-corrupt.txt
sed -i '1d' crates/kasane-writer/tests/census-known-corrupt.txt
cargo test -p kasane-writer --test census   # expect: "1 shape(s) newly corrupt"

# A shape that gets fixed must also fail: add one that is not corrupt.
git checkout crates/kasane-writer/tests/census-known-corrupt.txt
echo '[Text("a")]' >> crates/kasane-writer/tests/census-known-corrupt.txt
cargo test -p kasane-writer --test census   # expect: "1 allowlisted shape(s) are no longer corrupt"

git checkout crates/kasane-writer/tests/census-known-corrupt.txt
cargo test -p kasane-writer --test census   # expect: PASS
```

Put both failure messages in your report. Do not commit either edit.

- [ ] **Step 6: Run the full checks**

Run: `mise run lint && mise run test`

Expected: both green.

- [ ] **Step 7: Commit**

```bash
git add crates/kasane-writer/tests/census.rs crates/kasane-writer/tests/census-known-corrupt.txt
git commit -m "test(writer): census every short inline sequence against a ratcheting allowlist"
```

---

### Task 3: Trim a run's edges, and delete the blanket splice

Seam one (spec §2.1). A run's children are trimmed at both edges while the outermost printing element is a container whose delimiter shares a character with the run's own. This closes the 18-shape regression family, and it lets `flatten_into`'s `also` parameter and the whole of `nests_alone` go — the blanket splice was doing edge work everywhere and then apologising for it mid-buffer (spec §2.2).

**Files:**
- Modify: `crates/kasane-writer/src/escape.rs` (add `Delim::ch`, directly below the `Delim` enum)
- Modify: `crates/kasane-writer/src/markdown.rs` (`flatten_into`, `inlines_to_md_at`, `emphasis_run`, `run_children`; delete `nests_alone`; add `trim_edges` and `edge_to_splice`)
- Test: `crates/kasane-writer/src/markdown.rs` (`#[cfg(test)] mod tests`)
- Modify: `crates/kasane-writer/tests/census-known-corrupt.txt` (re-blessed, must shrink)

**Interfaces:**
- Consumes: `Flat<'a> = (&'a Inline, usize)` and `renders_empty(&Inline, usize) -> bool`, both already in `markdown.rs`.
- Produces:
  - `pub(crate) fn Delim::ch(self) -> char` in `escape.rs`
  - `fn trim_edges<'a>(children: Vec<Flat<'a>>, ch: char) -> Vec<Flat<'a>>` (private, `markdown.rs`)
  - `fn edge_to_splice(children: &[Flat<'_>], ch: char) -> Option<usize>` (private, `markdown.rs`)
  - `fn flatten_into<'a>(inls: &'a [Inline], depth: usize, out: &mut Vec<Flat<'a>>)` — the same function with its `also` parameter **removed**; Task 4 and Task 5 call it with this signature.

- [ ] **Step 1: Write the failing tests**

Add to `markdown.rs`'s `#[cfg(test)] mod tests`. The module already has the
`para` helper from the fusion item; use it.

```rust
    /// A container at the *tail* of a run's children carries a delimiter that
    /// abuts the one `emphasize` is about to append, so the two merge into a
    /// longer run and the parser splits it in the wrong place. This shape
    /// recovered `abc` before the fusion item and `abc**` after it — the
    /// regression this task closes (design spec §1).
    #[test]
    fn a_container_at_a_runs_tail_is_trimmed_into_it() {
        let em = |s: &str| Inline::Emph(vec![Inline::Text(s.into())]);
        assert_eq!(
            para(vec![
                em("a"),
                Inline::Strong(vec![Inline::Text("b".into())]),
                Inline::Strong(vec![em("c")]),
            ]),
            "*a***bc**"
        );
    }

    /// The same at the head of the run.
    #[test]
    fn a_container_at_a_runs_head_is_trimmed_into_it() {
        assert_eq!(
            para(vec![
                Inline::Emph(vec![Inline::Emph(vec![Inline::Text("a".into())])]),
                Inline::Emph(vec![Inline::Text("bc".into())]),
            ]),
            "*abc*"
        );
    }

    /// Splicing exposes a new edge, so the trim repeats. Three levels collapse
    /// to one.
    #[test]
    fn the_trim_repeats_until_neither_edge_is_a_container() {
        let inner = Inline::Emph(vec![Inline::Emph(vec![Inline::Emph(vec![
            Inline::Text("a".into()),
        ])])]);
        assert_eq!(para(vec![inner]), "*a*");
    }

    /// A container *between* other content contributes its delimiters with
    /// content on both sides, so nothing abuts and nothing is trimmed. This is
    /// the control: over-trimming here would flatten structure that is correct
    /// today, for no text gain (design spec § Confirmed).
    #[test]
    fn a_container_mid_buffer_is_left_alone() {
        assert_eq!(
            para(vec![Inline::Emph(vec![
                Inline::Text("a".into()),
                Inline::Strong(vec![Inline::Text("b".into())]),
                Inline::Text("c".into()),
            ])]),
            "*a**b**c*"
        );
    }

    /// A backtick at an edge does not share a character with `*`, so it does
    /// not collide and is not trimmed.
    #[test]
    fn a_code_span_at_an_edge_is_not_trimmed() {
        assert_eq!(
            para(vec![Inline::Emph(vec![Inline::Code("x".into())])]),
            "*`x`*"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-writer --lib 2>&1 | tail -40`

Expected: FAIL. `a_container_at_a_runs_tail_is_trimmed_into_it` reports
`*a***b*c***`; `the_trim_repeats_until_neither_edge_is_a_container` reports
`**a**`. `a_container_at_a_runs_head_is_trimmed_into_it`,
`a_container_mid_buffer_is_left_alone` and `a_code_span_at_an_edge_is_not_trimmed`
PASS already — the first because the fusion item's blanket splice happens to
cover it, the other two because they pin behaviour that is correct today and
must survive.

- [ ] **Step 3: Add `Delim::ch` to `escape.rs`**

Directly below the `Delim` enum:

```rust
impl Delim {
    /// The character this delimiter is spelled with.
    ///
    /// Two delimiter runs collide when they share a character, not when they
    /// are the same `Delim`: `*` and `**` are different classes that abut into
    /// one `***` run a parser splits somewhere the writer did not intend, while
    /// a backtick beside a `*` is simply two characters. Keying the rule on the
    /// character is what states it as written rather than leaving it true by
    /// the coincidence that this writer never spells emphasis with `_`
    /// (design spec `2026-08-15-emphasis-seam-design.md` §2.1).
    pub(crate) fn ch(self) -> char {
        match self {
            Delim::Backtick => '`',
            Delim::Emph | Delim::Strong => '*',
        }
    }
}
```

- [ ] **Step 4: Drop `also` from `flatten_into`**

Replace `flatten_into` with:

```rust
fn flatten_into<'a>(inls: &'a [Inline], depth: usize, out: &mut Vec<Flat<'a>>) {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return;
    }
    for i in inls {
        match i {
            Inline::Link { target, inlines } if !matches!(target, RefTarget::External(_)) => {
                flatten_into(inlines, depth + 1, out)
            }
            _ => out.push((i, depth)),
        }
    }
}
```

Delete the `also` paragraph from its doc comment — the one beginning "`also`
names a *second* transparency" — and leave every other paragraph, which still
describes what the function does. Update the call in `inlines_to_md_at` to
`flatten_into(inls, depth, &mut view)`.

- [ ] **Step 5: Add the trim, and rewrite `emphasis_run`**

Replace `run_children`, `nests_alone` and `emphasis_run` with:

```rust
/// The flattened view of every member's children, as one sequence.
fn run_children<'a>(members: &[Flat<'a>]) -> Vec<Flat<'a>> {
    let mut out = Vec::new();
    for &(m, depth) in members {
        if let Inline::Emph(x) | Inline::Strong(x) = m {
            flatten_into(x, depth + 1, &mut out);
        }
    }
    out
}

/// The index of a leading or trailing printing element that must be spliced,
/// or `None` when neither edge collides.
fn edge_to_splice(children: &[Flat<'_>], ch: char) -> Option<usize> {
    let printing = |&&(i, d): &&Flat<'_>| !renders_empty(i, d);
    let first = children.iter().position(|c| printing(&c));
    let last = children.iter().rposition(|c| printing(&c));
    [first, last].into_iter().flatten().find(|&idx| {
        escape::delim(children[idx].0).map(escape::Delim::ch) == Some(ch)
    })
}

/// Trim a run's children at both edges while the outermost printing element is
/// a container whose delimiter shares a character with the run's own.
///
/// The edge is where a collision can happen and the only place it can:
/// `emphasize` appends its delimiter immediately outside these children, so an
/// edge container's own delimiter abuts it and the two merge into a longer run
/// the parser splits somewhere else. A container *between* other content has
/// content on both sides and collides with nothing, which is why this trims
/// rather than splicing everywhere — over-trimming would flatten structure
/// that is correct today for no text gain (design spec § Confirmed).
///
/// Splicing replaces a container with its own children, which may put another
/// container at the edge, so the loop repeats. It terminates because each
/// splice yields elements strictly deeper than the one it replaced and
/// `flatten_into` yields nothing at `MAX_INLINE_DEPTH`.
///
/// Only pointers move: `flatten_into` borrows, and `Vec::splice` shuffles
/// `Flat` pairs. No `Inline` is cloned (design spec §2.2's constraint).
fn trim_edges<'a>(mut children: Vec<Flat<'a>>, ch: char) -> Vec<Flat<'a>> {
    while let Some(idx) = edge_to_splice(&children, ch) {
        let (inline, depth) = children[idx];
        let (Inline::Emph(x) | Inline::Strong(x)) = inline else {
            // `edge_to_splice` only ever names a container: `escape::delim`
            // returns `Emph`/`Strong` for those alone, and `Backtick` cannot
            // match an emphasis run's `ch`.
            debug_assert!(false, "edge_to_splice named a non-container");
            return children;
        };
        let mut spliced = Vec::new();
        flatten_into(x, depth + 1, &mut spliced);
        children.splice(idx..idx + 1, spliced);
    }
    children
}

/// Render a run of adjacent `Emph` (or `Strong`) elements as one emphasized
/// span over the concatenation of their children, with both edges trimmed.
///
/// The members' children are flattened into **one** view and scanned once, so a
/// delimiter-bearing inline at the end of one member's children and one at the
/// start of the next member's are neighbours to the scan exactly as they are to
/// a parser.
///
/// There is no per-member `pos` bookkeeping: the scan below owns the four `Pos`
/// rules and applies them once per run member exactly as the outer loop does
/// for any other neighbour.
fn emphasis_run<'a>(
    members: &[Flat<'a>],
    want: escape::Delim,
    ctx: Ctx,
    pos: Pos,
    markup: &str,
) -> String {
    let children = trim_edges(run_children(members), want.ch());
    emphasize(&inlines_to_md_flat(&children, ctx, pos), markup)
}
```

Delete `nests_alone` entirely, and delete the two bullet paragraphs in
`emphasis_run`'s old doc comment that described it — the ones beginning
"**Alone**," and "**Beside anything else**," and the sentence after them
beginning "The second case is a regression".

- [ ] **Step 6: Run the unit tests**

Run: `cargo fmt --all && cargo test -p kasane-writer --lib`

Expected: PASS, all of them, including the three controls.

- [ ] **Step 7: Re-bless the census and read the diff**

Run:

```bash
cargo test -p kasane-writer --test census 2>&1 | tail -20
KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census
git diff --stat crates/kasane-writer/tests/census-known-corrupt.txt
```

Expected: the unblessed run fails with "allowlisted shape(s) are no longer
corrupt", and the diff shows the file **shrinking** — lines removed, none
added. Put the removed-line count in your report; that number is this task's
evidence. **If any line was added, stop and report it**: this task must not
make a shape corrupt that was not, and an added line means it did.

- [ ] **Step 8: Run the full checks**

Run: `mise run lint && mise run test`

Expected: both green.

- [ ] **Step 9: Commit**

```bash
git add crates/kasane-writer/src/escape.rs crates/kasane-writer/src/markdown.rs \
        crates/kasane-writer/tests/census-known-corrupt.txt
git commit -m "fix(writer): trim a run's edges where a delimiter would abut its own character"
```

---

### Task 4: Fuse adjacent runs that share a delimiter character

Seam two (spec §2.1). `[Emph([Text("a")]), Strong([Code("bc")])]` prints
`` *a***`bc`** ``: the two runs' delimiters abut into a three-`*` run and
CommonMark's multiple-of-three rule splits it in the wrong place. Adjacent runs
sharing a delimiter character become one run, the first member's class winning.

This is where the item's stated structural cost is paid: `[Emph(a), Strong(b)]`
renders as one `<em>` where it renders as both today. Recovered text is
identical (spec §1, "The trade this item makes").

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs` (`run_end`)
- Test: `crates/kasane-writer/src/markdown.rs` (`#[cfg(test)] mod tests`)
- Modify: `crates/kasane-writer/tests/census-known-corrupt.txt` (re-blessed, must shrink)

**Interfaces:**
- Consumes: `escape::Delim::ch` from Task 3.
- Produces: nothing later tasks depend on by name.

- [ ] **Step 1: Write the failing tests**

```rust
    /// Two runs whose delimiters share a character are one run. Their
    /// delimiters would otherwise abut into a longer run, and whether that
    /// parses as intended depends on CommonMark's multiple-of-three rule —
    /// `[Emph(a), Strong(b)]` survives it and `[Emph(a), Strong([Code])]` does
    /// not, differing only in the character after the second opening
    /// delimiter. Telling those apart means mirroring the parser's delimiter
    /// matching, which this repo has refused three times (design spec §7 A).
    #[test]
    fn adjacent_runs_sharing_a_delimiter_character_fuse() {
        assert_eq!(
            para(vec![
                Inline::Emph(vec![Inline::Text("a".into())]),
                Inline::Strong(vec![Inline::Code("bc".into())]),
            ]),
            "*a`bc`*"
        );
    }

    /// The cost this item pays, pinned so a later reader meets it as a
    /// decision rather than a surprise. `[Emph(a), Strong(b)]` renders as one
    /// `<em>` where it used to render as an `<em>` and a `<strong>`. The
    /// recovered text is identical; a boundary is lost on a shape that is not
    /// broken (design spec §1, "The trade this item makes").
    #[test]
    fn fusing_adjacent_runs_costs_a_structural_boundary() {
        assert_eq!(
            para(vec![
                Inline::Emph(vec![Inline::Text("a".into())]),
                Inline::Strong(vec![Inline::Text("b".into())]),
            ]),
            "*ab*"
        );
    }

    /// A backtick run beside an emphasis run shares no character, so the two
    /// stay separate. The control against over-fusing.
    #[test]
    fn runs_with_different_delimiter_characters_do_not_fuse() {
        assert_eq!(
            para(vec![
                Inline::Code("x".into()),
                Inline::Emph(vec![Inline::Text("a".into())]),
            ]),
            "`x`*a*"
        );
    }
```

`fusing_adjacent_runs_costs_a_structural_boundary` replaces the fusion item's
`inlines_with_different_delimiters_are_left_alone` assertion for that shape.
Find that test with
`grep -n "inlines_with_different_delimiters_are_left_alone" -A 15 crates/kasane-writer/src/markdown.rs`
and delete only its `assert_eq!(para(vec![em("a"), st("b")]), "*a***b**");`
line, leaving the code-span and math assertions and the doc comment, which are
still true.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-writer --lib 2>&1 | tail -30`

Expected: FAIL. `adjacent_runs_sharing_a_delimiter_character_fuse` reports
`` *a***`bc`** ``; `fusing_adjacent_runs_costs_a_structural_boundary` reports
`*a***b**`. `runs_with_different_delimiter_characters_do_not_fuse` PASSES
already and must keep passing.

- [ ] **Step 3: Group runs by character**

In `run_end`, replace the `Delim` equality with a character equality. The
function becomes:

```rust
fn run_end(items: &[Flat<'_>], start: usize) -> usize {
    let Some(d) = escape::delim(items[start].0) else {
        return start + 1;
    };
    let ch = d.ch();
    let mut k = start + 1;
    while k < items.len()
        && (renders_empty(items[k].0, items[k].1)
            || escape::delim(items[k].0).map(escape::Delim::ch) == Some(ch))
    {
        k += 1;
    }
    k
}
```

Append to its doc comment:

```rust
/// A run is grouped by the delimiter's **character**, not by its `Delim`: `*`
/// and `**` abut into one `***` run that a parser splits somewhere the writer
/// did not intend, so two adjacent emphasis runs of different classes are one
/// run. The first member's class wins, which is what the emit loop already
/// reads from `items[i]` (design spec §2.1, seam two).
```

The emit loop needs no change: it already selects markup from
`escape::delim(inline)` where `inline` is the run's first element.

- [ ] **Step 4: Run the unit tests**

Run: `cargo fmt --all && cargo test -p kasane-writer --lib`

Expected: PASS.

- [ ] **Step 5: Re-bless the census and read the diff**

Run:

```bash
KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census
git diff --stat crates/kasane-writer/tests/census-known-corrupt.txt
```

Expected: the file shrinks again, lines removed and none added. Report the
count. **An added line means this task corrupted a shape that was fine — stop
and report it.**

- [ ] **Step 6: Run the full checks**

Run: `mise run lint && mise run test`

Expected: both green.

- [ ] **Step 7: Commit**

```bash
git add crates/kasane-writer/src/markdown.rs crates/kasane-writer/tests/census-known-corrupt.txt
git commit -m "fix(writer): fuse adjacent runs that share a delimiter character"
```

---

### Task 5: Decline to spell a delimiter that cannot flank

Spec §2.3. A delimiter can also meet the *content's* own punctuation, and then
there is nothing to splice or fuse with. `[Text("a"), Emph([Code("a")])]` prints
`` a*`a`* ``: the opening `*` is preceded by a letter and followed by a
backtick, so it is neither left- nor right-flanking and CommonMark leaves it as
a literal asterisk. The scan decides, because the closing delimiter's flanking
depends on the character *after* it — which `emphasize` never sees.

**Files:**
- Modify: `crates/kasane-writer/src/markdown.rs` (add `Flank`, `class_of`, `next_class`; `emphasis_run` gains two arguments; the emit loop passes them)
- Test: `crates/kasane-writer/src/markdown.rs` (`#[cfg(test)] mod tests`)
- Modify: `crates/kasane-writer/tests/census-known-corrupt.txt` (re-blessed, must shrink)

**Interfaces:**
- Consumes: `Flat`, `renders_empty`, `escape::delim`, `escape::math_degrades` — all present.
- Produces: nothing later tasks depend on by name.

- [ ] **Step 1: Write the failing tests**

```rust
    /// An opening delimiter preceded by a word character and followed by the
    /// content's own punctuation flanks on neither side, so CommonMark leaves
    /// it as a literal asterisk: `` a*`a`* `` reads `a*a*`, with both
    /// asterisks visible in the prose. Nothing here can be fused with — the
    /// collision is with content, not with markup — so the run renders its
    /// children bare (design spec §2.3).
    #[test]
    fn an_unspellable_opening_delimiter_is_not_emitted() {
        assert_eq!(
            para(vec![
                Inline::Text("a".into()),
                Inline::Emph(vec![Inline::Code("a".into())]),
            ]),
            "a`a`"
        );
    }

    /// The same failure on the closing side, which is why the decision cannot
    /// live in `emphasize`: the character after the closing delimiter is the
    /// next element in the stream, which `emphasize` never sees.
    #[test]
    fn an_unspellable_closing_delimiter_is_not_emitted() {
        assert_eq!(
            para(vec![
                Inline::Emph(vec![Inline::Code("a".into())]),
                Inline::Text("a".into()),
            ]),
            "`a`a"
        );
    }

    /// The control: the same emphasis with a word character inside flanks on
    /// both sides and keeps its delimiters.
    #[test]
    fn a_spellable_delimiter_is_still_emitted() {
        assert_eq!(
            para(vec![
                Inline::Text("a".into()),
                Inline::Emph(vec![Inline::Text("b".into())]),
            ]),
            "a*b*"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-writer --lib 2>&1 | tail -30`

Expected: FAIL. The first reports `` a*`a`* ``, the second `` *`a`*a ``.
`a_spellable_delimiter_is_still_emitted` PASSES already and must keep passing.

- [ ] **Step 3: Add the flanking classes**

Insert directly above `emphasis_run`:

```rust
/// CommonMark's three character classes for the flanking rules.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flank {
    Space,
    Punct,
    Other,
}

/// Which class a character falls in.
///
/// Everything that is neither whitespace nor alphanumeric counts as
/// punctuation, which is a superset of Unicode's `P*` categories and matches
/// what CommonMark 0.30 and later fold in as symbols. The classes are used
/// only to decide whether the writer's *own* delimiter can flank, so erring
/// wide costs at most an emphasis span the writer declines to spell, never
/// corrupt text.
fn class_of(c: char) -> Flank {
    if c.is_whitespace() {
        Flank::Space
    } else if c.is_alphanumeric() {
        Flank::Other
    } else {
        Flank::Punct
    }
}

/// The class of the first character the rest of the view will print.
///
/// Computed without rendering anything twice: every element except `Text`
/// begins with markup the writer chose, and every one of those characters is
/// punctuation — a backtick for a code span or a degrading `Math`, `$` for one
/// that does not degrade, `*` for a container, `[` for a footnote reference or
/// an external link. A `Text` begins with its own first character, and stays
/// punctuation if `escape::text` prefixes a backslash, since a backslash is
/// punctuation and so is anything it escapes. An exhausted view is the end of
/// the line, which CommonMark counts as whitespace.
fn next_class(rest: &[Flat<'_>]) -> Flank {
    for &(i, d) in rest {
        if renders_empty(i, d) {
            continue;
        }
        return match i {
            Inline::Text(t) => t.chars().next().map_or(Flank::Space, class_of),
            _ => Flank::Punct,
        };
    }
    Flank::Space
}

/// Whether a delimiter run can open emphasis here, by CommonMark's
/// left-flanking rule: not followed by whitespace, and either not followed by
/// punctuation or preceded by whitespace or punctuation.
fn can_open(before: Flank, after: Flank) -> bool {
    after != Flank::Space && (after != Flank::Punct || before != Flank::Other)
}

/// Whether a delimiter run can close emphasis here — the right-flanking rule,
/// which is the mirror of [`can_open`].
fn can_close(before: Flank, after: Flank) -> bool {
    before != Flank::Space && (before != Flank::Punct || after != Flank::Other)
}
```

- [ ] **Step 4: Let the scan make the decision**

Give `emphasis_run` the two classes it cannot compute itself, and have it
decline rather than lie:

```rust
fn emphasis_run<'a>(
    members: &[Flat<'a>],
    want: escape::Delim,
    ctx: Ctx,
    pos: Pos,
    markup: &str,
    before: Flank,
    after: Flank,
) -> String {
    let children = trim_edges(run_children(members), want.ch());
    let inner = inlines_to_md_flat(&children, ctx, pos);
    let core = inner.trim();
    // An all-whitespace or empty inner buffer gets no delimiters anyway --
    // `emphasize` says so itself -- and has no first or last character to
    // classify, so the flanking question does not arise.
    if core.is_empty() {
        return inner;
    }
    let opens = can_open(before, class_of(core.chars().next().unwrap()));
    let closes = can_close(class_of(core.chars().next_back().unwrap()), after);
    if opens && closes {
        emphasize(&inner, markup)
    } else {
        // The delimiter would not flank where it lands, so a parser would read
        // it as a literal asterisk in the middle of the prose. The text is the
        // invariant and the span is not: render the children bare (design spec
        // §2.3).
        inner
    }
}
```

In `inlines_to_md_flat`, compute the two classes and pass them. Replace the
`Some(escape::Delim::Emph)` and `Some(escape::Delim::Strong)` arms with:

```rust
            Some(d @ (escape::Delim::Emph | escape::Delim::Strong)) => {
                let before = s.chars().next_back().map_or(Flank::Space, class_of);
                let after = next_class(&items[end..]);
                let markup = if d == escape::Delim::Emph { "*" } else { "**" };
                s.push_str(&emphasis_run(members, d, ctx, pos, markup, before, after))
            }
```

`before` reads the buffer emitted so far, and an empty buffer is the start of
the line, which counts as whitespace — the same rule `Pos::LineStart` encodes.

- [ ] **Step 5: Run the unit tests**

Run: `cargo fmt --all && cargo test -p kasane-writer --lib`

Expected: PASS, all of them. The Task 3 and Task 4 tests must still pass: none
of their shapes has an unflankable delimiter.

- [ ] **Step 6: Re-bless the census and read the diff**

Run:

```bash
KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census
git diff --stat crates/kasane-writer/tests/census-known-corrupt.txt
```

Expected: shrinks again, no lines added. Report the count and the running
total across Tasks 3-5. **A line added means this task corrupted a shape that
was fine — stop and report it.**

- [ ] **Step 7: Run the full checks**

Run: `mise run lint && mise run test`

Expected: both green.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-writer/src/markdown.rs crates/kasane-writer/tests/census-known-corrupt.txt
git commit -m "fix(writer): do not emit an emphasis delimiter that cannot flank where it lands"
```

---

### Task 6: Widen P13's alphabet

Spec §5.3. Three widenings are recorded as blocked in `P13_WORDS`'s doc
comment; Tasks 3-5 unblock all three. The property that could not draw a
delimiter-bearing emphasis child is exactly why the seam defects shipped.

**Files:**
- Modify: `crates/kasane-writer/tests/properties.rs` (`p13_inline`, and `P13_WORDS`'s doc comment)

**Interfaces:**
- Consumes: the behaviour Tasks 3-5 shipped.
- Produces: nothing.

- [ ] **Step 1: Widen the strategy**

In `p13_inline()`, add three arms to the `prop_oneof!`:

```rust
        word().prop_map(|w| Inline::Emph(vec![Inline::Code(w)])),
        word().prop_map(|w| Inline::Emph(vec![Inline::Emph(vec![Inline::Text(w)])])),
        word().prop_map(|w| Inline::Strong(vec![Inline::Emph(vec![Inline::Text(w)])])),
```

- [ ] **Step 2: Run it**

Run: `cargo test -p kasane-writer --test properties p13_`

Expected: PASS. If it fails, the shrunk counterexample is a shape Tasks 3-5
did not close — **read it before assuming this task is at fault**, and report
it rather than narrowing the alphabet back. Narrowing would hide exactly what
this task exists to expose.

- [ ] **Step 3: Correct the doc comment**

In `P13_WORDS`'s doc comment, delete the two numbered blocking entries and the
paragraph beginning "Widen the alphabet with `Emph(vec![Code(w)])`", and the
paragraph beginning "`Strong(vec![Emph(vec![Text(w)])])` is blocked by neither
of those two". Replace all of it with:

```rust
/// The alphabet draws delimiter-bearing children — `Emph([Code])`,
/// `Emph([Emph])`, `Strong([Emph])` — because the seam between one member's
/// last child and the next member's first is where the run scan had to be
/// taught to look, and only such a child reaches it. Those three shapes were
/// blocked until 2026-08-15 by two defects at `emphasize`'s own seam, both
/// closed by `2026-08-15-emphasis-seam-design.md`; the census
/// (`tests/census.rs`) is what watches the shapes this alphabet still cannot
/// reach.
```

Keep every paragraph above it: the restricted-alphabet argument and the
`Inline::Code("")` exclusion are unrelated to this item and still true.

- [ ] **Step 4: Run the full checks**

Run: `mise run lint && mise run test`

Expected: both green.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-writer/tests/properties.rs
git commit -m "test(writer): draw delimiter-bearing emphasis children in P13"
```

---

### Task 7: Correct the record

Spec §6. Three open bullets close, two spec claims are already false at this
branch's head, and `AGENTS.md` describes a narrower rule than the writer now
has.

**Files:**
- Modify: `docs/superpowers/specs/2026-08-09-markdown-escaping-design.md` (the last three bullets of § "Recorded as open")
- Modify: `docs/superpowers/specs/2026-08-15-adjacent-inline-fusion-design.md` (§2.1, §2.2, §2.4)
- Modify: `docs/superpowers/specs/2026-08-15-emphasis-seam-design.md` (Status block)
- Modify: `AGENTS.md` (the `kasane-writer` entry)
- Check: `README.md`

**Interfaces:**
- Consumes: the behaviour Tasks 1-6 shipped.
- Produces: nothing.

- [ ] **Step 1: Close the three escaping-spec bullets**

Each of the last three bullets in § "Recorded as open" — the edge-punctuation
one, the lone-nested-emphasis one, and the wrap-seam regression one — gets a
closure note in the shape the bullets above them already use: what closed it,
by which mechanism, and what the bullet predicted instead. Locate them with
`grep -n "emphasis delimiter flush\|lone nested emphasis\|wrap seam" docs/superpowers/specs/2026-08-09-markdown-escaping-design.md`.

The lone-nested bullet predicted a `(character, length)` delimiter class; say
that what closed it is the edge trim, which needed no such class — only the
observation that the collision is at the edge. The wrap-seam bullet predicted
that the three seams should close together; say that they did.

- [ ] **Step 2: Correct the fusion spec's two false claims**

In `2026-08-15-adjacent-inline-fusion-design.md`:

- §2.1's "A run of one is what happens today, so a document containing no
  adjacent pair renders byte-identically" is already false at that item's head
  (`Emph([Emph([Text("a")]), Text("bc")])` moved from `**a*bc*` to `*abc*`) and
  is falser now. Replace the clause with a note that the emphasis-seam item
  supersedes it: a document with no *colliding* seam renders byte-identically,
  and a colliding one is what both items exist to change.
- §2.2's description of `flatten_into`'s `also` parameter goes with the
  parameter — replace it with a pointer to the emphasis-seam spec's §2.1 trim.
- §2.4's cost statement gains one sentence recording that
  `2026-08-15-emphasis-seam-design.md` extended the same trade to abutting
  runs of different classes.

Leave that spec's Status block alone.

- [ ] **Step 3: This item's own Status block**

`2026-08-15-emphasis-seam-design.md`'s Status line reads "Designed 2026-08-15.
Not yet implemented." Replace with:

```markdown
**Status:** Implemented 2026-08-15. The edge trim, the run fuse and the
flanking decline each landed with unit coverage; the census
(`kasane-writer/tests/census.rs`) is committed with its ratcheting allowlist,
and P13 now draws delimiter-bearing emphasis children. The allowlist still
names the shapes §8 records as out of this item's scope.
```

- [ ] **Step 4: `AGENTS.md`**

Find the fusion sentence with `grep -n "same delimiter" AGENTS.md` and replace
it with the general rule:

```
  Delimiter runs that share a character never abut in the printed line: a
  container at the edge of an emphasis run is spliced into it, two adjacent
  runs spelled with the same character are one run, and a delimiter that would
  flank on neither side where it lands is not emitted at all. CommonMark cannot
  express those arrangements, so the writer trades the span boundary for the
  text -- which is the invariant -- and `kasane-writer/tests/census.rs` is the
  exhaustive check that it does.
```

- [ ] **Step 5: Check `README.md` rather than assume**

Run: `grep -n "backtick\|emphasis\|exception" README.md`

Expected: no user-visible anchor change from this item (spec §4), so the Known
limitations list needs nothing. If a hit describes emphasis output in a way
this item falsifies, correct it; record either outcome in your report and do
not invent an edit.

- [ ] **Step 6: Run the full checks**

Run: `mise run lint && mise run test`

Expected: both green. Doc comments are compiled, so a malformed one fails here.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "docs: record the emphasis seam closure"
```

---

## Self-Review

**1. Spec coverage.**

| Spec section | Task |
|---|---|
| §2.1 `Delim::ch`, the edge trim (seam one) | 3 |
| §2.1 the run fuse (seam two), and its ordering after the fuse | 4 |
| §2.2 delete `also` and `nests_alone` | 3 (Steps 4-5) |
| §2.3 the flanking decline, in the scan not in `emphasize` | 5 |
| §2.4 move the inline-depth assertion | 1 |
| §3 blast radius (`emphasize` untouched, fixtures unchanged) | 3-5 (census), 8 below |
| §4 anchors unaffected | no task needed — nothing to change |
| §5.1 the census and its allowlist | 2, re-blessed by 3, 4, 5 |
| §5.2 unit coverage, one shape per family plus controls | 3, 4, 5 |
| §5.3 P13 widening | 6 |
| §6 documentation | 7 |
| §8 verification, residual risks | 2 (the allowlist is the record), 7 (Step 3) |

One deliberate omission: spec §8 asks that `tests/fixtures/epub/rich.epub`
convert to an identical tree. That is `mise run test`'s job — the fixture tree
is asserted by the existing CLI tests, which every task runs. No task adds a
bespoke check for it.

**2. Placeholder scan.** No TBD/TODO, no "handle edge cases", no "similar to
Task N". Every code step carries the full text to write. Three steps are
conditional and each names both outcomes and says to record which happened
rather than invent an edit: Task 1 Step 1's `Document` field check, Task 2 Step
1's `Options` set check, and Task 7 Step 5's README grep.

**3. Type consistency.** `Delim::ch(self) -> char` is defined in Task 3 and
called in Tasks 3 and 4. `flatten_into` loses its `also` parameter in Task 3
and is called with the three-argument form in `inlines_to_md_at`,
`run_children` and `trim_edges`. `trim_edges(Vec<Flat<'a>>, char) ->
Vec<Flat<'a>>` is called with `want.ch()`. `run_end(items, start) -> usize`
keeps its signature and returns the scan cursor. `emphasis_run` grows two
`Flank` arguments in Task 5 and its only call site is updated in the same step.
`Flank`, `class_of`, `next_class`, `can_open` and `can_close` are all defined
and consumed within Task 5, so nothing is staged ahead of its consumer.

**4. Red-first honesty.** Tasks 3, 4 and 5 each have a genuinely red battery
before their implementation step, with the exact wrong output named. Task 1 is
green on arrival and says so — it moves an assertion rather than changing
behaviour. Task 2's census is green after Step 3 by construction, which is why
Step 5 proves it can fail in both directions before it is committed. Task 6 is
green on arrival, and its red evidence is the census diff Tasks 3-5 produced.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-08-15-emphasis-seam.md`. Two execution options:

**1. Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
