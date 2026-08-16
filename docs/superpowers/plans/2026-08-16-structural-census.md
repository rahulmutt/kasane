# Structural Census Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the census a second assertion that compares emphasis *structure*, not only recovered text, so a loss that leaves the text byte-identical can no longer ship silently.

**Architecture:** For every character of recovered text, compare the stack of enclosing emphasis containers on the IR side against the parsed side. The IR walk mirrors `kasane_gfm::rendered_text` arm for arm and proves it does so every run by re-deriving `rendered_text` from its own output — so the test cannot drift into checking a different projection. The comparison is gated on the text assertion already passing, because per-character alignment presupposes equal strings. Results land in two ratchet files: a queue that should reach zero, and a computed permanent set for shapes Markdown cannot express at all.

**Tech Stack:** Rust (stable 1.97.1, pinned in `mise.toml`), `pulldown-cmark` 0.13 as the test oracle (`Tag`/`TagEnd` API), `kasane-gfm` as the text-projection oracle. Every change is in `crates/kasane-writer/tests/` plus one `AGENTS.md` edit. No production code changes.

**Spec:** `docs/superpowers/specs/2026-08-16-structural-census-design.md`

## Global Constraints

- Create branch `structural-census` off `main` before Task 1. Do not commit to `main`.
- Per-task checks are `mise run lint` and `mise run test`. `lint` is `cargo fmt --all -- --check` plus `cargo clippy --workspace --all-targets -- -D warnings`; a plain `cargo clippy` is **not** enough, because every line this plan adds lives in a test target.
- `-D warnings` means an unused item fails the build. Every item this plan adds is consumed in the same task that adds it; do not stage an interface ahead of its consumer.
- Run `cargo fmt --all` before every commit rather than hand-aligning; style comes from `rustfmt.toml`.
- Commit messages follow the repo's existing form: `test(writer): …`, `docs: …`.
- **This item fixes nothing.** The queue starts at 2,812 and stays there. A task that "improves" a number has changed the writer, which is out of scope — revert it and record it instead (spec §7).
- **Do not touch production code.** `crates/kasane-writer/src/**`, `kasane-gfm`, `kasane-core` and `kasane-ir` are all out of scope. If an assertion fails, the finding is recorded, not fixed.
- **Do not modify `census-known-corrupt.txt`.** The text allowlist stays at exactly 32 entries throughout. Item #1 drains it and item #3 guards it; a change here would collide with both.
- The census runs in ~0.02s in release over 7,239 shapes. If a change makes it slow, something is quadratic — stop and look, do not shrink the alphabet.
- **Expected counts, from the spec's §5 measurement** — these are the item's evidence, so a task that produces different numbers has found something and must stop and report rather than bless over it:

  | Quantity | Value |
  |---|---|
  | total shapes | 7,239 |
  | text corrupt (gated out) | 32 |
  | structurally corrupt | 4,048 |
  | └─ queue (`census-known-structure-corrupt.txt`) | 2,812 |
  | └─ permanent (`census-inexpressible.txt`) | 1,236 |
  | self-check / alignment / balance failures | 0 |

## File Structure

| File | Responsibility |
|---|---|
| `crates/kasane-writer/tests/census.rs` (modify) | Gains the context walks, the classifier, and the structural assertion beside the existing text one. Shares `alphabet()` and `shapes()` — that sharing is the point, so a shape cannot be text-checked but not structure-checked. |
| `crates/kasane-writer/tests/census-known-structure-corrupt.txt` (create) | The queue. Target zero. 2,812 lines. |
| `crates/kasane-writer/tests/census-inexpressible.txt` (create) | Permanent, computed, header-documented. 1,236 entries. |
| `AGENTS.md` (modify, `:304-310`) | The ratchet paragraph currently describes one file; it must describe three and explain the split. |

One file rather than a new test file: the value of the census is that its alphabet is exhaustive over a chosen set, and two test files drawing from two alphabets would let a shape fall through the gap this item exists to close.

---

### Task 1: The IR-side context walk, and the guard that it cannot drift

The comparison is only meaningful if the IR walk produces exactly the projection the text assertion compares — `kasane_gfm::rendered_text`. Rather than assert that in a comment, this task makes it checkable: the walk emits characters, and the test re-assembles them and requires the result to equal `rendered_text` on all 7,239 shapes. If anyone ever edits one walk and not the other, the build fails.

**Files:**
- Modify: `crates/kasane-writer/tests/census.rs` (add after `shapes()`, currently ending at `:110`)

**Interfaces:**
- Consumes: `shapes()` and `alphabet()` (already in the file), `kasane_gfm::rendered_text`, `kasane_ir::MAX_INLINE_DEPTH`, `kasane_ir::Inline`.
- Produces: `enum Emphasis { Em, St }`; `fn ir_context(&[Inline], usize, &mut Vec<Emphasis>, &mut Vec<(char, Vec<Emphasis>)>)`; `fn context_text(&[(char, Vec<Emphasis>)]) -> String`. Tasks 2, 3 and 4 all use `Emphasis` and `ir_context`.

- [ ] **Step 1: Write the failing test**

Add to `crates/kasane-writer/tests/census.rs`:

```rust
/// One emphasis container, as it appears on the stack enclosing a character.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Emphasis {
    Em,
    St,
}

/// Every character `rendered_text` contributes, paired with the stack of
/// emphasis containers enclosing it.
///
/// Mirrors `kasane_gfm::rendered_text`'s own walk arm for arm — its
/// `MAX_INLINE_DEPTH` cutoff, and its `[^n]` spelling for a footnote reference
/// — because the whole comparison is meaningless if this walks a different
/// projection from the one the text assertion uses. That mirroring is not
/// asserted in prose: `the_context_walk_reproduces_rendered_text_for_every_short_sequence`
/// re-derives `rendered_text` from this walk's own output every run.
///
/// `Link` pushes nothing. `flatten_into` (`markdown.rs:237-238`) splices every
/// non-`External` target away before the emit loop ever sees it, so a
/// transparent link is not a structural level in the output and must not be
/// one here (design spec §2).
fn ir_context(
    inlines: &[Inline],
    depth: usize,
    stack: &mut Vec<Emphasis>,
    out: &mut Vec<(char, Vec<Emphasis>)>,
) {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return;
    }
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => {
                for c in t.chars() {
                    out.push((c, stack.clone()));
                }
            }
            Inline::Emph(x) => {
                stack.push(Emphasis::Em);
                ir_context(x, depth + 1, stack, out);
                stack.pop();
            }
            Inline::Strong(x) => {
                stack.push(Emphasis::St);
                ir_context(x, depth + 1, stack, out);
                stack.pop();
            }
            Inline::Link { inlines, .. } => ir_context(inlines, depth + 1, stack, out),
            Inline::FootnoteRef(n) => {
                for c in format!("[^{}]", n.0).chars() {
                    out.push((c, stack.clone()));
                }
            }
        }
    }
}

/// The characters of a context walk, in order.
fn context_text(v: &[(char, Vec<Emphasis>)]) -> String {
    v.iter().map(|(c, _)| *c).collect()
}

/// The first of the three guards (design spec §3): a hard failure, not a skip.
///
/// If this fires, the instrument is broken rather than the writer — someone has
/// edited `ir_context` or `rendered_text` without the other, and every
/// structural verdict downstream is being computed against the wrong
/// projection.
#[test]
fn the_context_walk_reproduces_rendered_text_for_every_short_sequence() {
    for seq in shapes() {
        let mut ctx = Vec::new();
        ir_context(&seq, 0, &mut Vec::new(), &mut ctx);
        assert_eq!(
            context_text(&ctx),
            kasane_gfm::rendered_text(&seq),
            "the context walk has drifted from `rendered_text` on {seq:?}"
        );
    }
}
```

- [ ] **Step 2: Run the test and verify it passes**

Run: `cargo test -p kasane-writer --test census the_context_walk_reproduces --release`
Expected: PASS.

- [ ] **Step 3: Prove the guard is not vacuous**

A guard that passes is worthless until you have seen it fail. Temporarily delete the `Inline::FootnoteRef(n) => { … }` arm from `ir_context`, replacing it with `Inline::FootnoteRef(_) => {}`.

Run: `cargo test -p kasane-writer --test census the_context_walk_reproduces --release`
Expected: FAIL, with `the context walk has drifted from `rendered_text` on [FootnoteRef(NoteId(1))]` (or another footnote-bearing shape — order is not guaranteed).

Now restore the arm exactly as written in Step 1 and re-run.
Expected: PASS.

- [ ] **Step 4: Lint and test**

Run: `mise run lint && mise run test`
Expected: both green. `census-known-corrupt.txt` unchanged at 32 lines — check with `wc -l crates/kasane-writer/tests/census-known-corrupt.txt`.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-writer/tests/census.rs
git commit -m "test(writer): walk the IR carrying each character's emphasis stack

The structural assertion needs the same projection the text assertion
compares. Rather than assert the mirroring in prose, the walk re-derives
rendered_text from its own output on all 7,239 shapes, so editing one
walk and not the other fails the build."
```

---

### Task 2: The parsed-side walk, and the alignment and balance guards

The other half of the comparison, plus the two remaining §3 guards. Both are hard failures for the same reason as Task 1's: the probe recorded zero of each, so an occurrence means the instrument is broken, not the writer.

`parsed_text` currently builds its `Options` inline. This task lifts them into a shared helper so the two parser walks cannot drift onto different option sets — which would be its own silent-divergence bug.

**Files:**
- Modify: `crates/kasane-writer/tests/census.rs` (`parsed_text` and its doc comment at `:78-95`; add after it)

**Interfaces:**
- Consumes: `Emphasis`, `ir_context`, `context_text` (Task 1); `shapes()`, `parsed_text`; `kasane_writer::blocks_to_markdown`.
- Produces: `fn parser_options() -> Options`; `fn parsed_context(&str) -> Vec<(char, Vec<Emphasis>)>`; `fn trim_whitespace(&[(char, Vec<Emphasis>)]) -> &[(char, Vec<Emphasis>)]`. Tasks 3 and 4 use `parsed_context` and `trim_whitespace`.

- [ ] **Step 1: Extract the parser options**

Replace `parsed_text` and its doc comment (`census.rs:78-95`) with:

```rust
/// The oracle's options. Shared so the two parser walks cannot drift onto
/// different option sets — `ENABLE_MATH` in one and not the other would move
/// characters between `Event::Text` and `Event::InlineMath` and silently
/// change what each walk counts.
fn parser_options() -> Options {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_MATH);
    opts
}

/// The text a real parser recovers from `md`.
fn parsed_text(md: &str) -> String {
    let mut out = String::new();
    for ev in Parser::new_ext(md, parser_options()) {
        match ev {
            Event::Text(t) | Event::Code(t) | Event::InlineMath(t) | Event::DisplayMath(t) => {
                out.push_str(&t)
            }
            _ => {}
        }
    }
    out
}
```

Update the import at `census.rs:37` to bring in the two tag types:

```rust
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
```

- [ ] **Step 2: Run the existing tests to verify the extraction changed nothing**

Run: `cargo test -p kasane-writer --test census --release`
Expected: PASS. The text assertion is the check here — if `parser_options` dropped a flag, the allowlist would no longer match and the test would name the newly-corrupt shapes.

- [ ] **Step 3: Write the parsed walk and the alignment test**

Add after `parsed_text`:

```rust
/// Every character a real parser recovers, paired with the stack of emphasis
/// containers enclosing it.
///
/// The third guard (design spec §3) is the `assert!` at the end: an unbalanced
/// event stream means the comparison below it is meaningless, so it fails
/// rather than returning a half-built vector.
fn parsed_context(md: &str) -> Vec<(char, Vec<Emphasis>)> {
    let mut stack: Vec<Emphasis> = Vec::new();
    let mut out = Vec::new();
    for ev in Parser::new_ext(md, parser_options()) {
        match ev {
            Event::Start(Tag::Emphasis) => stack.push(Emphasis::Em),
            Event::Start(Tag::Strong) => stack.push(Emphasis::St),
            Event::End(TagEnd::Emphasis | TagEnd::Strong) => {
                stack.pop();
            }
            Event::Text(t) | Event::Code(t) | Event::InlineMath(t) | Event::DisplayMath(t) => {
                for c in t.chars() {
                    out.push((c, stack.clone()));
                }
            }
            _ => {}
        }
    }
    assert!(
        stack.is_empty(),
        "unbalanced emphasis events parsing {md:?} — the structural \
         comparison for this shape would be meaningless"
    );
    out
}

/// The slice with leading and trailing whitespace dropped, matching the
/// `.trim()` the text assertion compares under. Without this the two vectors
/// would be off by however much whitespace the writer added or the parser ate.
fn trim_whitespace(v: &[(char, Vec<Emphasis>)]) -> &[(char, Vec<Emphasis>)] {
    let start = v
        .iter()
        .position(|(c, _)| !c.is_whitespace())
        .unwrap_or(v.len());
    let end = v
        .iter()
        .rposition(|(c, _)| !c.is_whitespace())
        .map_or(start, |i| i + 1);
    &v[start..end]
}

/// The second of the three guards (design spec §3).
///
/// Where the text already matches, the two walks must produce the same
/// characters in the same order — that is what makes a positional comparison of
/// their stacks meaningful. This cannot fail by construction, which is exactly
/// why it is asserted: if it ever does, some character is reaching one walk and
/// not the other and every structural verdict is suspect.
#[test]
fn the_two_context_walks_align_character_for_character() {
    for seq in shapes() {
        let md =
            kasane_writer::blocks_to_markdown(&[Block::Para(seq.clone())], &AssetBag::default());
        let expected = kasane_gfm::rendered_text(&seq);
        if parsed_text(&md).trim() != expected.trim() {
            // Text already corrupt: named by the text assertion, and structure
            // is not evaluated here (design spec §2, "Gate").
            continue;
        }

        let mut ir = Vec::new();
        ir_context(&seq, 0, &mut Vec::new(), &mut ir);
        let ir = trim_whitespace(&ir);
        let got = parsed_context(&md);
        let got = trim_whitespace(&got);

        assert_eq!(
            context_text(ir),
            context_text(got),
            "the two walks disagree on characters for {seq:?}, so their \
             stacks cannot be compared positionally"
        );
    }
}
```

- [ ] **Step 4: Run the test and verify it passes**

Run: `cargo test -p kasane-writer --test census the_two_context_walks_align --release`
Expected: PASS.

- [ ] **Step 5: Prove the guard is not vacuous**

Temporarily delete `Event::Code(t) |` from `parsed_context`'s text arm, so code-span characters stop reaching the parsed walk.

Run: `cargo test -p kasane-writer --test census the_two_context_walks_align --release`
Expected: FAIL, with `the two walks disagree on characters for [Code("x")]` or another code-bearing shape.

Restore the arm exactly as written in Step 3 and re-run.
Expected: PASS.

- [ ] **Step 6: Lint and test**

Run: `mise run lint && mise run test`
Expected: both green, `census-known-corrupt.txt` still 32 lines.

- [ ] **Step 7: Commit**

```bash
git add crates/kasane-writer/tests/census.rs
git commit -m "test(writer): walk parsed events carrying the same emphasis stack

Adds the parsed half of the comparison and the two remaining guards:
the walks must agree character-for-character wherever the text already
matches, and an unbalanced event stream fails rather than producing a
half-built vector. Parser options are shared so the two walks cannot
drift onto different option sets."
```

---

### Task 3: The relation, the computed split, and both ratchet files

The comparison itself, plus the classifier that decides which of the two files a corrupt shape belongs in. The classifier's two conditions are the substance of design spec §4, and condition 1 is the one that keeps the permanent file from becoming a place where real regressions go to hide.

**Files:**
- Modify: `crates/kasane-writer/tests/census.rs`
- Create: `crates/kasane-writer/tests/census-known-structure-corrupt.txt` (by bless)
- Create: `crates/kasane-writer/tests/census-inexpressible.txt` (by bless)

**Interfaces:**
- Consumes: everything from Tasks 1 and 2.
- Produces: `enum Structure { Clean, Corrupt, Inexpressible }`; `fn classify(&[Inline]) -> Structure`; `fn nests_same_class_directly(&[Inline]) -> bool`; `fn differs_only_by_collapse(&[(char, Vec<Emphasis>)], &[(char, Vec<Emphasis>)]) -> bool`; `fn ratchet(&str, &BTreeSet<String>, &str, Option<&str>)`. Task 4 uses `classify`, `Structure` and `ratchet`.

- [ ] **Step 1: Write the classifier**

Add to `census.rs`:

```rust
const STRUCTURE_ALLOWLIST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-known-structure-corrupt.txt"
);

const INEXPRESSIBLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-inexpressible.txt"
);

/// How one shape's structure survived rendering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Structure {
    /// Structure preserved — or text already corrupt, in which case structure
    /// is not evaluated (design spec §2, "Gate").
    Clean,
    /// A real, fixable loss. Belongs in the queue.
    Corrupt,
    /// Markdown cannot express this shape at any level. Permanent.
    Inexpressible,
}

/// Whether `seq` holds a container whose **sole** child is a container of the
/// same class — `Emph[Emph[…]]` or `Strong[Strong[…]]`.
///
/// Condition 1 of the inexpressible split (design spec §4), and the one that
/// does the work. `<em><em>x</em></em>` has no CommonMark spelling: `**x**` is
/// strong, not nested emphasis. Only *direct* nesting is inexpressible —
/// `Emph[a, Emph[b], c]` round-trips correctly today, so filing it as permanent
/// on the strength of condition 2 alone would bury a real regression if it ever
/// broke.
fn nests_same_class_directly(seq: &[Inline]) -> bool {
    seq.iter().any(|i| match i {
        Inline::Emph(x) => {
            matches!(x.as_slice(), [Inline::Emph(_)]) || nests_same_class_directly(x)
        }
        Inline::Strong(x) => {
            matches!(x.as_slice(), [Inline::Strong(_)]) || nests_same_class_directly(x)
        }
        Inline::Link { inlines, .. } => nests_same_class_directly(inlines),
        _ => false,
    })
}

/// Whether every difference between the two walks disappears once adjacent
/// identical classes are collapsed. Condition 2 of the split (design spec §4).
fn differs_only_by_collapse(
    ir: &[(char, Vec<Emphasis>)],
    got: &[(char, Vec<Emphasis>)],
) -> bool {
    fn collapse(v: &[Emphasis]) -> Vec<Emphasis> {
        let mut out: Vec<Emphasis> = Vec::new();
        for &e in v {
            if out.last() != Some(&e) {
                out.push(e);
            }
        }
        out
    }
    ir.iter()
        .zip(got)
        .all(|(x, y)| collapse(&x.1) == collapse(&y.1))
}

/// The relation, for one shape (design spec §2).
fn classify(seq: &[Inline]) -> Structure {
    let md = kasane_writer::blocks_to_markdown(&[Block::Para(seq.to_vec())], &AssetBag::default());
    let expected = kasane_gfm::rendered_text(seq);
    if parsed_text(&md).trim() != expected.trim() {
        return Structure::Clean;
    }

    let mut ir = Vec::new();
    ir_context(seq, 0, &mut Vec::new(), &mut ir);
    let ir = trim_whitespace(&ir);
    let got = parsed_context(&md);
    let got = trim_whitespace(&got);

    if ir.iter().zip(got).all(|(x, y)| x.1 == y.1) {
        return Structure::Clean;
    }
    if nests_same_class_directly(seq) && differs_only_by_collapse(ir, got) {
        return Structure::Inexpressible;
    }
    Structure::Corrupt
}
```

- [ ] **Step 2: Write the ratchet helper and the structural assertion**

Add to `census.rs`:

```rust
/// Bless or check one ratchet file, two-directionally: a shape that is in
/// `found` but not the file fails, and a shape in the file but not `found`
/// fails too, so the file can neither grow silently nor rot into stale
/// excuses.
///
/// `#`-prefixed lines are comments, which is how the permanent file carries its
/// header. The text allowlist keeps its own copy of this logic for now — it is
/// the instrument two other items depend on, and re-pointing it is Task 4's
/// step, gated separately.
fn ratchet(path: &str, found: &BTreeSet<String>, noun: &str, header: Option<&str>) {
    if std::env::var_os("KASANE_CENSUS_BLESS").is_some() {
        let mut body = header.unwrap_or("").to_string();
        body.extend(found.iter().map(|l| format!("{l}\n")));
        std::fs::write(path, body).expect("writing the allowlist");
        return;
    }

    let known: BTreeSet<String> = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("{path} must exist -- bless it with KASANE_CENSUS_BLESS=1"))
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();

    let new: Vec<&String> = found.difference(&known).collect();
    let gone: Vec<&String> = known.difference(found).collect();

    assert!(
        new.is_empty(),
        "{} shape(s) newly {noun}:\n{}",
        new.len(),
        new.iter().take(10).map(|s| format!("  {s}\n")).collect::<String>()
    );
    assert!(
        gone.is_empty(),
        "{} listed shape(s) are no longer {noun} -- delete them from {path} \
         (KASANE_CENSUS_BLESS=1 does it for you):\n{}",
        gone.len(),
        gone.iter().take(10).map(|s| format!("  {s}\n")).collect::<String>()
    );
}

const INEXPRESSIBLE_HEADER: &str = "\
# Shapes whose structure Markdown cannot express, at any level.
#
# `<em><em>x</em></em>` has no CommonMark spelling: `**x**` is strong, not
# nested emphasis. No writer change can close these, which is why they are not
# in the queue (`census-known-structure-corrupt.txt`) -- 1,236 unclosable
# entries there would make \"shrink to zero\" meaningless.
#
# COMPUTED, never hand-edited. A shape lands here only if it BOTH contains a
# container whose sole child is a same-class container AND differs from the IR
# only by collapsing adjacent identical classes. Stop satisfying either and it
# moves back to the queue on the next bless. See
# `docs/superpowers/specs/2026-08-16-structural-census-design.md` §4.
#
# Regenerate: KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census
";

/// The structural tier: does the emphasis structure a parser recovers match the
/// structure the IR held?
///
/// Runs only where the text assertion already passes — structure is meaningless
/// where the text is scrambled, and per-character alignment presupposes equal
/// strings. As the text allowlist drains, its shapes graduate into this check
/// without anyone editing this test.
#[test]
fn inline_structure_survives_rendering_for_every_short_sequence() {
    let mut corrupt = BTreeSet::new();
    let mut inexpressible = BTreeSet::new();

    for seq in shapes() {
        match classify(&seq) {
            Structure::Clean => {}
            Structure::Corrupt => {
                corrupt.insert(format!("{seq:?}"));
            }
            Structure::Inexpressible => {
                inexpressible.insert(format!("{seq:?}"));
            }
        }
    }

    ratchet(STRUCTURE_ALLOWLIST, &corrupt, "structurally corrupt", None);
    ratchet(
        INEXPRESSIBLE,
        &inexpressible,
        "inexpressible",
        Some(INEXPRESSIBLE_HEADER),
    );
}
```

- [ ] **Step 3: Run without the files to verify the ratchet demands a bless**

Run: `cargo test -p kasane-writer --test census inline_structure_survives --release`
Expected: FAIL with `…/census-known-structure-corrupt.txt must exist -- bless it with KASANE_CENSUS_BLESS=1`.

- [ ] **Step 4: Bless**

Run: `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census --release`
Expected: PASS.

- [ ] **Step 5: Verify the counts against the spec**

Run:

```bash
wc -l crates/kasane-writer/tests/census-known-structure-corrupt.txt
grep -vc '^#' crates/kasane-writer/tests/census-inexpressible.txt
wc -l crates/kasane-writer/tests/census-known-corrupt.txt
```

Expected: **2812**, **1236**, **32**, exactly. These are the item's evidence (design spec §5).

**If any number differs, stop.** Do not re-bless and do not adjust the classifier to reach the target. A different number means either the writer changed under you or the classifier does not implement §4 — report which, with the actual counts, and wait.

Then spot-check that the split landed correctly:

```bash
grep -c 'Emph(\[Emph(' crates/kasane-writer/tests/census-inexpressible.txt
grep -c 'Emph(\[Strong(' crates/kasane-writer/tests/census-known-structure-corrupt.txt
```

Expected: a non-zero count in each. Direct same-class nesting is permanent; mixed-class nesting is a queued defect (design spec §6). If `Emph([Strong(` appears in the *inexpressible* file, condition 1 is miswired — that family is fixable via `***a***` and must stay in the queue.

- [ ] **Step 6: Run the full check**

Run: `cargo test -p kasane-writer --test census --release`
Expected: PASS, all four tests.

- [ ] **Step 7: Lint and test**

Run: `mise run lint && mise run test`
Expected: both green.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-writer/tests/census.rs \
        crates/kasane-writer/tests/census-known-structure-corrupt.txt \
        crates/kasane-writer/tests/census-inexpressible.txt
git commit -m "test(writer): ratchet the structural census

4,048 of 7,239 shapes lose emphasis structure while rendering text a
parser recovers byte-identically, so nothing in the suite could see
them. 2,812 are a queue; 1,236 are shapes Markdown cannot express at
all (<em><em>x</em></em> has no spelling) and are split into their own
file by a computed predicate, never by hand.

Fixes none of it. The queue's largest family, 2,002 shapes, is the
cross-class edge splice recorded in the design spec's SS6."
```

---

### Task 4: Pin the relation's edges, and point the text allowlist at the shared ratchet

Two hazards remain. A bless-driven test can go vacuous — if `classify` returned `Clean` for everything, the files would bless to empty and every run would pass. And the ratchet logic now exists twice in one file. This task closes both, then updates the codebase map.

**Files:**
- Modify: `crates/kasane-writer/tests/census.rs`
- Modify: `AGENTS.md:304-310`

**Interfaces:**
- Consumes: `classify`, `Structure`, `ratchet`, `ALLOWLIST` (existing, `:40-43`).
- Produces: nothing later tasks depend on. This is the last task.

- [ ] **Step 1: Write the two pinning tests**

Add to `census.rs`:

```rust
/// The relation catches a class substitution — one of the two losses this tier
/// exists for.
///
/// `[Emph("a"), Strong("b")]` prints `*ab*`: the run fuse merges the `Strong`
/// into the `Em` run, and `b` comes back inside an `<em>` it was never in. The
/// text is byte-identical either way, which is why the text assertion cannot
/// see it.
#[test]
fn the_structural_relation_catches_a_class_substitution() {
    let seq = vec![
        Inline::Emph(vec![Inline::Text("a".into())]),
        Inline::Strong(vec![Inline::Text("b".into())]),
    ];
    assert_eq!(classify(&seq), Structure::Corrupt);
}

/// The relation stays silent on intentional fusion.
///
/// `[Emph("a"), Emph("b")]` prints `*ab*` too — one `<em>` over both — but
/// every character keeps the class it had, so nothing was lost. Adjacent-run
/// fusion is deliberate (`2026-08-15-adjacent-inline-fusion-design.md`); a
/// check that flagged it would be unusable, and would have buried the shape
/// above in thousands of false positives.
#[test]
fn the_structural_relation_ignores_intentional_run_fusion() {
    let seq = vec![
        Inline::Emph(vec![Inline::Text("a".into())]),
        Inline::Emph(vec![Inline::Text("b".into())]),
    ];
    assert_eq!(classify(&seq), Structure::Clean);
}
```

- [ ] **Step 2: Run them**

Run: `cargo test -p kasane-writer --test census the_structural_relation --release`
Expected: both PASS.

- [ ] **Step 3: Prove they are not vacuous**

Temporarily change `classify`'s final line from `Structure::Corrupt` to `Structure::Clean`.

Run: `cargo test -p kasane-writer --test census the_structural_relation --release`
Expected: `the_structural_relation_catches_a_class_substitution` FAILS with `left: Clean, right: Corrupt`; the fusion test still passes.

Restore `Structure::Corrupt` and re-run.
Expected: both PASS.

- [ ] **Step 4: Point the text assertion at the shared ratchet**

Replace the bless-and-compare body of `inline_text_survives_rendering_for_every_short_sequence` (`census.rs:125-162`, everything after the `for seq in shapes()` loop) with:

```rust
    ratchet(ALLOWLIST, &corrupt, "corrupt", None);
}
```

This is deliberately a separate step from Task 3: the text assertion is the instrument items #1 and #3 both depend on, and re-pointing it in the same commit that adds a new tier would have doubled the blast radius of a single review.

- [ ] **Step 5: Verify the text assertion is unchanged in behaviour**

Run: `cargo test -p kasane-writer --test census inline_text_survives --release`
Expected: PASS.

Run: `git diff --stat crates/kasane-writer/tests/census-known-corrupt.txt`
Expected: **no output** — the file is untouched. If the ratchet helper changed what gets written, this file would have moved, and it must not.

- [ ] **Step 6: Update the codebase map**

Replace `AGENTS.md:304-310` with:

```markdown
- `crates/kasane-writer/tests/census-known-corrupt.txt` is a ratchet, not a
  todo list: `census.rs` fails the build if a shape is corrupt and unlisted,
  *and* if a listed shape is no longer corrupt, so the file cannot grow
  silently or rot into stale excuses. Regenerate it with
  `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census` and read
  the diff — that diff is the exact evidence a reviewer wants, of what a
  change fixed or broke.
- The census has two tiers, and three files. The text tier above compares what
  a parser recovers against `kasane_gfm::rendered_text`. The **structural**
  tier compares, for each character, the stack of emphasis containers enclosing
  it on both sides — a loss that leaves the text byte-identical (a `<strong>`
  coming back as an `<em>`, a nesting level dropped) is invisible to the first
  tier and caught by the second. It runs only where the text tier already
  passes, since per-character alignment presupposes equal strings.
  `census-known-structure-corrupt.txt` is its queue, target zero;
  `census-inexpressible.txt` is permanent, holding shapes Markdown cannot
  express at any level (`<em><em>x</em></em>` has no spelling — `**x**` is
  strong). The split between those two files is **computed on every bless,
  never hand-edited**: a shape is permanent only if it both nests a same-class
  container directly and differs only by collapsing adjacent identical classes.
  One bless command rewrites all three. Design spec
  `2026-08-16-structural-census-design.md`; its §6 records the largest queued
  family, 2,002 shapes in which `Emph[Strong[x]]` loses its `<strong>` because
  `splice_children`'s edge rule keys on the delimiter character and
  `Delim::ch()` maps both classes to `*`.
```

- [ ] **Step 7: Lint and test**

Run: `mise run lint && mise run test`
Expected: both green.

Final count check:

```bash
wc -l crates/kasane-writer/tests/census-known-corrupt.txt
wc -l crates/kasane-writer/tests/census-known-structure-corrupt.txt
grep -vc '^#' crates/kasane-writer/tests/census-inexpressible.txt
```

Expected: **32**, **2812**, **1236**.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-writer/tests/census.rs AGENTS.md
git commit -m "test(writer): pin the structural relation's edges

A bless-driven test can go vacuous: a classify() that returned Clean for
everything would bless empty files and pass forever. Two unit tests pin
both edges independently of the bless output -- a class substitution is
caught, and intentional run fusion is not.

Also points the text assertion at the shared ratchet helper, kept out of
the previous commit so the instrument two other items depend on had its
own review gate, and documents the second tier in the codebase map."
```

---

## Verification

After Task 4, the branch should show:

- `mise run lint && mise run test` green.
- Four tests in `census.rs`: the two guards, the text tier, the structural tier — plus the two pinning tests.
- `census-known-corrupt.txt` byte-identical to its state on `main`.
- `census-known-structure-corrupt.txt` at 2,812 entries; `census-inexpressible.txt` at 1,236 plus header.
- No change to any file under `crates/*/src/`.

The last one is worth checking explicitly, since the temptation this item creates is to fix what it finds:

```bash
git diff --stat main -- 'crates/*/src/'
```

Expected: no output.
