# Block-Nesting Depth Bound Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bound block nesting (`Block::List` / `Block::Footnote`) so a deeply nested document degrades instead of aborting the process with a stack overflow.

**Architecture:** Two constants, mirroring the existing `MAX_INLINE_DEPTH` pair. A *fidelity* bound in the EPUB XHTML parser flattens over-deep lists and footnotes while keeping every text run, so adapter IR is always shallow. A *safety* bound in `kasane-ir` stops the six recursive block walks descending, for hand-built `Document`s that reach the published `structure()` from outside the workspace. The fidelity bound is strictly lower than the safety bound, so adapter IR can never reach the latter.

**Tech Stack:** Rust (pinned stable via mise), `quick-xml` streaming parser, `proptest`, `cargo-fuzz` seeds replayed on stable by `cargo test`.

**Spec:** `docs/superpowers/specs/2026-07-29-block-nesting-depth-bound-design.md`

## Global Constraints

- Every change ships green under `mise run lint && mise run test`. `mise run lint` is `cargo fmt --check` plus `clippy --all-targets -D warnings` — plain `cargo clippy` is not enough.
- The ordering invariant `epub::xhtml::MAX_BLOCK_DEPTH < kasane_ir::MAX_BLOCK_DEPTH` must hold and must be stated in both constants' doc comments.
- Constant values are **measured, not guessed**. Every constant's doc comment names the measured abort depth and the thread it was measured on, matching how `MAX_INLINE_DEPTH`'s comment reads today.
- Value constraints (spec §4): safety ≤ ¼ of the measured abort depth on the tightest thread; fidelity ≤ ¼ of safety. Candidates: safety 128, fidelity 32.
- The tightest stack the code runs on is a **rayon worker in batch mode**, not `main`. Measured evidence in the spec: batch survives depth 500 and aborts at 1,000, where single-file mode survives 2,000 and aborts at 4,000 (debug build).
- No document in the repo may still claim block nesting is unbounded once Task 7 lands.
- `KNOWN_OPEN` in `crates/kasane-adapters/tests/fuzz_corpus.rs` stays empty — the bug is fixed in this branch, so the new seed is armed immediately.

---

## File Structure

**Modified:**
- `crates/kasane-ir/src/lib.rs` — adds `MAX_BLOCK_DEPTH` beside `MAX_INLINE_DEPTH`. Both constants and their rationale live together because they are one policy.
- `crates/kasane-adapters/src/epub/xhtml.rs` — adds `MAX_BLOCK_DEPTH` (fidelity) and the frame-suppression logic. The `<ul>`/`<ol>` push site, the `</ul>`/`</ol>` pop site, the `<aside>` push site.
- `crates/kasane-adapters/src/epub/mod.rs` — `fix_block_links` gains a depth parameter.
- `crates/kasane-adapters/src/mobi/mod.rs` — `strip_empty_anchor_links` gains a depth parameter; `normalize.rs`'s `MAX_DEPTH` comment gains a sentence.
- `crates/kasane-core/src/section.rs` — `clone_block` gains a depth parameter.
- `crates/kasane-core/src/balance.rs` — `est_tokens_block` / `est_tokens_blocks` gain a depth parameter.
- `crates/kasane-core/src/refs.rs` — `fix_block` gains a depth parameter.
- `crates/kasane-core/src/nav.rs` — the comment block asserting block nesting is unbounded.
- `crates/kasane-writer/src/markdown.rs` — `render_block` / `blocks_to_markdown` gain a depth parameter.
- `crates/kasane-adapters/src/fuzz_entry.rs` — `max_block_depth` + its assertion; `max_inline_depth`'s doc comment.
- `crates/kasane-writer/tests/generator/` — nested-list generation.
- `README.md`, `AGENTS.md`.

**Created:**
- `fuzz/seeds/epub/deep-blocks.epub` — the 30,000-deep reproducer.
- A CLI end-to-end test asserting batch-mode conversion of a deep document exits 0.

## Task Order and Why

Task 1 measures, because every later task needs the numbers. Task 2 does the fidelity bound, which is what actually fixes the reported reproducer. Tasks 3–5 do the safety bound crate by crate, innermost first, so each crate compiles against an already-landed dependency. Task 6 is the fuzz seam and the seed. Task 7 is the property tier. Task 8 is documentation, last, so it describes what was actually built rather than what was planned.

---

### Task 1: Measure the abort depth on the tightest thread

**Files:**
- Create: `crates/kasane-core/tests/depth_measurement.rs` (temporary — deleted in Step 5)

**Interfaces:**
- Consumes: nothing.
- Produces: two numbers recorded in this task's commit message and used as the doc-comment evidence in Tasks 2 and 3 — `MEASURED_ABORT_DEPTH` (the depth at which a libtest thread aborts) and the confirmed values for `kasane_ir::MAX_BLOCK_DEPTH` and `epub::xhtml::MAX_BLOCK_DEPTH`.

The spec's §1 table brackets all six walks together end-to-end through the CLI. This task narrows that to the binding walk on a libtest thread, which is the thread the test suite itself runs on and is the tightest one the code must survive.

- [ ] **Step 1: Write the measurement harness**

Create `crates/kasane-core/tests/depth_measurement.rs`:

```rust
//! TEMPORARY measurement harness — deleted in this task's final step.
//! Run with: cargo test -p kasane-core --test depth_measurement -- --nocapture --ignored
use kasane_ir::{Block, BlockId, DocMeta, Document, Inline, Node, Provenance};

/// A `Document` whose single node is a `Block::List` nested `depth` deep,
/// with an inline chain at the bottom so the block and inline budgets
/// compose the way they do in a real document (spec §4).
fn deep_doc(depth: usize) -> Document {
    let mut inline = Inline::Text("bottom".into());
    for _ in 0..kasane_ir::MAX_INLINE_DEPTH {
        inline = Inline::Emph(vec![inline]);
    }
    let mut blocks = vec![Block::Para(vec![inline])];
    for _ in 0..depth {
        blocks = vec![Block::List {
            ordered: false,
            items: vec![blocks],
        }];
    }
    let mut nodes = vec![Node {
        block: Block::Heading {
            level: 1,
            id: BlockId(0),
            inlines: vec![Inline::Text("T".into())],
        },
        prov: Provenance::default(),
    }];
    nodes.extend(blocks.into_iter().map(|block| Node {
        block,
        prov: Provenance::default(),
    }));
    Document {
        meta: DocMeta {
            title: "T".into(),
            authors: vec![],
            language: None,
            source_format: "test".into(),
            source_path: "t".into(),
        },
        nodes,
    }
}

#[test]
#[ignore = "measurement harness, not an assertion"]
fn measure() {
    for depth in [64usize, 128, 256, 512, 1024, 2048] {
        eprintln!("trying depth {depth}");
        let site = kasane_core::structure(
            deep_doc(depth),
            &kasane_core::Options {
                max_tokens: 4000,
                min_tokens: 100,
            },
        );
        eprintln!("  depth {depth}: OK, {} files", site.files.len());
    }
}
```

- [ ] **Step 2: Run it and record where it aborts**

Run: `cargo test -p kasane-core --test depth_measurement -- --nocapture --ignored`

Expected: the `trying depth N` lines print in increasing order and the process dies with `fatal runtime error: stack overflow` at some depth. The **last depth that printed an `OK` line** is the deepest survivable; the depth whose `trying` line printed without a matching `OK` is the abort depth.

Record both numbers. If the run completes all the way through 2048 without aborting, extend the array with `4096, 8192` and re-run — do not conclude "no bound needed".

- [ ] **Step 3: Derive the two constants**

Apply the spec's two constraints:

- `kasane_ir::MAX_BLOCK_DEPTH` = the largest power of two ≤ (abort depth ÷ 4).
- `epub::xhtml::MAX_BLOCK_DEPTH` = `kasane_ir::MAX_BLOCK_DEPTH` ÷ 4.

If that arithmetic yields safety 128 / fidelity 32, use the spec's candidates unchanged. If it yields smaller values, use the smaller ones — the constraints win over the candidates.

Write the two derived numbers and the measured abort depth into a scratch note; Tasks 2 and 3 copy them into doc comments verbatim.

- [ ] **Step 4: Confirm the same measurement on a rayon worker**

The libtest thread and a rayon worker may differ. Confirm with the CLI, which is what actually runs on rayon:

```bash
cargo build -p kasane-cli
python3 - <<'EOF'
import zipfile, os
D = 400  # replace with (derived safety bound) * 2
body = '<h1>Deep</h1>' + '<ul><li>' * D + 'x' + '</li></ul>' * D
xhtml = '<?xml version="1.0" encoding="utf-8"?>\n<html xmlns="http://www.w3.org/1999/xhtml"><body>' + body + '</body></html>'
opf = '<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Deep</dc:title><dc:identifier id="id">deep</dc:identifier><dc:language>en</dc:language></metadata><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>'
container = '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>'
os.makedirs('/tmp/deepbatch', exist_ok=True)
z = zipfile.ZipFile('/tmp/deepbatch/deep.epub', 'w', zipfile.ZIP_STORED)
z.writestr('mimetype', 'application/epub+zip')
z.writestr('META-INF/container.xml', container)
z.writestr('OEBPS/content.opf', opf)
z.writestr('OEBPS/c1.xhtml', xhtml)
z.close()
EOF
./target/debug/kasane /tmp/deepbatch -o /tmp/deepbatch-out; echo "exit=$?"
```

Expected: `exit=134` (the bug is still unfixed at this point — that is the confirmation the depth is genuinely dangerous on a rayon worker). If it exits 0, the derived safety bound is too conservative to be justified by this measurement; halve `D` until you find the boundary and re-derive.

- [ ] **Step 5: Delete the harness and commit the numbers**

```bash
rm crates/kasane-core/tests/depth_measurement.rs
git add -A
git commit -m "chore(core): measure the block-depth abort threshold

Libtest thread aborts at depth <N> (deepest survivable <M>).
Rayon worker confirmed to abort at depth <D>.
Derived: kasane_ir::MAX_BLOCK_DEPTH = <S>, epub::xhtml::MAX_BLOCK_DEPTH = <F>."
```

The commit message is the durable record; the harness itself is not kept because it asserts nothing and would abort the suite by design.

---

### Task 2: The fidelity bound in the XHTML parser

**Files:**
- Modify: `crates/kasane-adapters/src/epub/xhtml.rs`

**Interfaces:**
- Consumes: the fidelity value derived in Task 1.
- Produces: `pub(crate) const MAX_BLOCK_DEPTH: usize` in `crates/kasane-adapters/src/epub/xhtml.rs`, and the guarantee that `xhtml_to_blocks` never emits `Block::List`/`Block::Footnote` nested deeper than it. Task 6 asserts this guarantee.

This is the task that fixes the reported reproducer. It covers EPUB and MOBI/AZW3 alike, because `mobi::normalize_html` re-serializes into this same parser.

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `crates/kasane-adapters/src/epub/xhtml.rs`. Place them next to the existing `deeply_nested_*` inline tests so the block and inline cases read together.

```rust
/// Max nesting depth of List/Footnote blocks, via an explicit stack so the
/// checker itself cannot overflow on the very input it is checking — the
/// same reasoning `fuzz_entry::max_inline_depth` records for its traversal.
fn block_depth_of(blocks: &[Block]) -> usize {
    let mut max_depth = 0;
    let mut stack: Vec<(&[Block], usize)> = vec![(blocks, 0)];
    while let Some((slice, depth)) = stack.pop() {
        for b in slice {
            match b {
                Block::List { items, .. } => {
                    max_depth = max_depth.max(depth + 1);
                    for item in items {
                        stack.push((item, depth + 1));
                    }
                }
                Block::Footnote { blocks, .. } => {
                    max_depth = max_depth.max(depth + 1);
                    stack.push((blocks, depth + 1));
                }
                // Leaves enumerated, not wildcarded: a new nesting variant
                // must break this build rather than make the check blind.
                Block::Heading { .. }
                | Block::Para(_)
                | Block::Table(_)
                | Block::Figure { .. }
                | Block::CodeBlock { .. }
                | Block::MathBlock(_)
                | Block::Raw { .. } => {}
            }
        }
    }
    max_depth
}

/// Collect every text run reachable anywhere in `blocks`, so a test can
/// assert flattening kept content rather than dropping it. Iterative on the
/// block side for the same reason `block_depth_of` is; the inline side
/// delegates to this module's existing `inlines_text`, whose recursion is
/// safe because the inline bound already holds.
fn all_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut stack: Vec<&Block> = blocks.iter().rev().collect();
    while let Some(b) = stack.pop() {
        match b {
            Block::Heading { inlines, .. } | Block::Para(inlines) => {
                out.push_str(&inlines_text(inlines))
            }
            Block::List { items, .. } => {
                for item in items {
                    stack.extend(item.iter());
                }
            }
            Block::Footnote { blocks, .. } => stack.extend(blocks.iter()),
            Block::Figure { caption, .. } => out.push_str(&inlines_text(caption)),
            Block::Table(t) => {
                for c in t.header.iter().chain(t.rows.iter().flatten()) {
                    out.push_str(&inlines_text(c));
                }
            }
            Block::CodeBlock { text, .. } => out.push_str(text),
            Block::MathBlock(s) | Block::Raw { note: s } => out.push_str(s),
        }
    }
    out
}

#[test]
fn deeply_nested_lists_flatten_at_the_block_bound() {
    const DEPTH: usize = 3000;
    let mut html = String::from("<body><h1>T</h1>");
    for _ in 0..DEPTH {
        html.push_str("<ul><li>");
    }
    html.push_str("SENTINEL");
    for _ in 0..DEPTH {
        html.push_str("</li></ul>");
    }
    html.push_str("</body>");

    let blocks = parse_blocks(&html);

    assert!(
        block_depth_of(&blocks) <= MAX_BLOCK_DEPTH,
        "produced block depth {} exceeds MAX_BLOCK_DEPTH {}",
        block_depth_of(&blocks),
        MAX_BLOCK_DEPTH
    );
    // The half that separates flattening from truncation: content survives.
    assert!(
        all_text(&blocks).contains("SENTINEL"),
        "the innermost text was dropped -- this bound must flatten, not truncate"
    );
}

#[test]
fn blocks_after_a_deep_list_are_not_corrupted() {
    const DEPTH: usize = 3000;
    let mut html = String::from("<body><h1>T</h1>");
    for _ in 0..DEPTH {
        html.push_str("<ul><li>");
    }
    html.push_str("inner");
    for _ in 0..DEPTH {
        html.push_str("</li></ul>");
    }
    html.push_str("<p>AFTER</p></body>");

    let blocks = parse_blocks(&html);

    // A suppressed </ul> that wrongly popped a real frame would unbalance
    // `frames` and swallow this trailing paragraph into the deep list.
    assert!(
        blocks
            .iter()
            .any(|b| matches!(b, Block::Para(inls) if inls.iter().any(|i| matches!(i, Inline::Text(t) if t == "AFTER")))),
        "trailing paragraph must be a top-level block, not captured by the deep list"
    );
}

#[test]
fn deeply_nested_footnotes_flatten_at_the_block_bound() {
    const DEPTH: usize = 3000;
    let mut html = String::from("<body><h1>T</h1>");
    for _ in 0..DEPTH {
        html.push_str(r#"<aside epub:type="footnote"><p>x</p>"#);
    }
    html.push_str("<p>NOTESENTINEL</p>");
    for _ in 0..DEPTH {
        html.push_str("</aside>");
    }
    html.push_str("</body>");

    let blocks = parse_blocks(&html);

    assert!(
        block_depth_of(&blocks) <= MAX_BLOCK_DEPTH,
        "produced block depth {} exceeds MAX_BLOCK_DEPTH {}",
        block_depth_of(&blocks),
        MAX_BLOCK_DEPTH
    );
    assert!(all_text(&blocks).contains("NOTESENTINEL"));
}
```

`parse_blocks` is this test module's existing helper (`crates/kasane-adapters/src/epub/xhtml.rs:1070`), a thin wrapper over `parse(xml).blocks` — the same one `deep_inline_nesting_is_flattened_not_preserved` uses. `inlines_text` is the module-level helper at line 261. Neither needs to be created.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-adapters --lib xhtml -- deeply_nested_lists blocks_after_a_deep deeply_nested_footnotes`

Expected: compile error, `cannot find value MAX_BLOCK_DEPTH in this scope`. That is the correct first failure — the constant does not exist yet.

- [ ] **Step 3: Add the fidelity constant**

In `crates/kasane-adapters/src/epub/xhtml.rs`, directly below the existing `MAX_INLINE_DEPTH` constant:

```rust
/// Maximum block nesting this parser preserves as nested `Block` values.
///
/// The frame stack that builds blocks is iterative, so parsing an
/// arbitrarily nested `<ul>` never overflows *here* — but it hands the core
/// and the writer a `Block` tree they walk recursively. Bounding the
/// produced depth is what keeps a hostile book from reaching
/// `kasane_ir::MAX_BLOCK_DEPTH`, exactly as `MAX_INLINE_DEPTH` above does
/// for inline nesting.
///
/// This is a *fidelity* bound, not a safety one: past it a list's items
/// become siblings at this level instead of a nested list, and a footnote
/// `<aside>` becomes transparent, so no content is lost — only the nesting
/// relationship. <REPLACE: F> is far past any real book's list nesting,
/// which rarely exceeds a handful of levels.
///
/// One site covers three formats: MOBI/AZW3 re-serializes through this
/// parser (`mobi::normalize_html`), so it inherits this bound. PPTX nests
/// via `slide.rs`'s `build_list`, already capped at 256 because its `level`
/// is a `u8`. PDF and DjVu never nest blocks.
pub(crate) const MAX_BLOCK_DEPTH: usize = <REPLACE: F>;
```

Replace both `<REPLACE: F>` markers with the fidelity value derived in Task 1 Step 3.

- [ ] **Step 4: Add the list-frame tracker**

`frames` alone cannot tell a real close from a suppressed one. Add a tracker mirroring the existing `aside_pushed`.

Find the declaration of `aside_pushed` (around `crates/kasane-adapters/src/epub/xhtml.rs:423`) and add directly beneath it:

```rust
    // Mirrors the nesting of <ul>/<ol> tags so the End handler knows whether
    // a given close corresponds to a List frame it opened. Without this, a
    // </ul> whose open was suppressed at MAX_BLOCK_DEPTH would satisfy the
    // End arm's `matches!(frames.last(), Some(BlockFrame::List { .. }))`
    // guard -- because the *enclosing* frame is also a List -- and pop the
    // parent, unbalancing `frames` for every block that follows.
    let mut list_pushed: Vec<bool> = vec![];
```

- [ ] **Step 5: Suppress the over-deep list open**

Replace the `b"ul" | b"ol"` arm in the Start handler (around line 603):

```rust
                    b"ul" | b"ol" => {
                        if frames.len() >= MAX_BLOCK_DEPTH {
                            // Over the fidelity bound: push no frame, so this
                            // list's <li> items land in the enclosing List
                            // frame as siblings. Content is kept; only the
                            // nesting relationship is dropped.
                            list_pushed.push(false);
                        } else {
                            frames.push(BlockFrame::List {
                                ordered: e.local_name().as_ref() == b"ol",
                                items: vec![],
                            });
                            list_pushed.push(true);
                        }
                    }
```

- [ ] **Step 6: Match the suppression on close**

Replace the `b"ul" | b"ol"` arm in the End handler (around line 956):

```rust
                    b"ul" | b"ol" => {
                        if list_pushed.pop() == Some(true)
                            && matches!(frames.last(), Some(BlockFrame::List { .. }))
                        {
                            let f = frames.pop().expect("checked");
                            finish_frame(f, &mut frames, &mut inline_stack, &mut blocks);
                        }
                    }
```

`pop() == Some(true)` is the same shape the `b"aside"` arm below already uses. A stray `</ul>` with no matching open pops `None`, which is not `Some(true)`, so it is ignored — strictly safer than the previous unconditional guard.

- [ ] **Step 7: Suppress the over-deep footnote open**

In the `b"aside"` Start arm (around line 668), the footnote branch already records a bool. Change the `if epub_type_has(&e, "footnote")` branch so an over-deep footnote is recorded as *not* pushed. Replace the branch body up to and including `aside_pushed.push(true);` with:

```rust
                        if epub_type_has(&e, "footnote") && frames.len() < MAX_BLOCK_DEPTH {
                            let note = NoteId(*next_note);
                            *next_note += 1;
                            if let Some(idv) = e
                                .attributes()
                                .flatten()
                                .find(|a| a.key.as_ref() == b"id")
                                .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                            {
                                footnotes.push((idv, note));
                            }
                            frames.push(BlockFrame::Footnote {
                                note,
                                blocks: vec![],
                            });
                            aside_pushed.push(true);
                        } else {
                            // Not a footnote aside, or past the fidelity
                            // bound: transparent either way, so its blocks
                            // emit into the enclosing frame. The existing
                            // End arm already handles `Some(false)` by
                            // popping nothing.
                            aside_pushed.push(false);
                        }
```

No change is needed in the `b"aside"` End arm — it already tests `aside_pushed.pop() == Some(true)`.

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p kasane-adapters --lib xhtml`

Expected: PASS, including the three new tests and every pre-existing xhtml test. If a pre-existing list test now fails, the tracker is out of sync with the frame stack — check that every path which pushes a `BlockFrame::List` also pushes to `list_pushed`.

- [ ] **Step 9: Confirm the reproducer is fixed end to end**

```bash
cargo build -p kasane-cli
python3 - <<'EOF'
import zipfile, os
D = 30000
body = '<h1>Deep</h1>' + '<ul><li>' * D + 'x' + '</li></ul>' * D
xhtml = '<?xml version="1.0" encoding="utf-8"?>\n<html xmlns="http://www.w3.org/1999/xhtml"><body>' + body + '</body></html>'
opf = '<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Deep Blocks</dc:title><dc:identifier id="id">deep</dc:identifier><dc:language>en</dc:language></metadata><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>'
container = '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>'
os.makedirs('/tmp/deepblocks', exist_ok=True)
z = zipfile.ZipFile('/tmp/deepblocks/deep-blocks.epub', 'w', zipfile.ZIP_STORED)
z.writestr('mimetype', 'application/epub+zip')
z.writestr('META-INF/container.xml', container)
z.writestr('OEBPS/content.opf', opf)
z.writestr('OEBPS/c1.xhtml', xhtml)
z.close()
EOF
./target/debug/kasane /tmp/deepblocks -o /tmp/deepblocks-out; echo "exit=$?"
grep -rl "x" /tmp/deepblocks-out | head
```

Expected: `exit=0` (was 134 before this task), in **batch** mode specifically — that is the tight-stack path. Keep `/tmp/deepblocks/deep-blocks.epub`; Task 6 commits it as the fuzz seed.

- [ ] **Step 10: Lint and commit**

```bash
mise run lint
git add crates/kasane-adapters/src/epub/xhtml.rs
git commit -m "fix(epub): flatten block nesting past a documented fidelity bound"
```

---

### Task 3: The safety constant in kasane-ir

**Files:**
- Modify: `crates/kasane-ir/src/lib.rs`

**Interfaces:**
- Consumes: the safety value derived in Task 1.
- Produces: `pub const MAX_BLOCK_DEPTH: usize` in `kasane_ir`, consumed by Tasks 4 and 5.

- [ ] **Step 1: Add the constant**

In `crates/kasane-ir/src/lib.rs`, directly below `MAX_INLINE_DEPTH` (line 29):

```rust
/// Maximum block nesting the structuring engine and the writer will descend.
///
/// `Block::List` and `Block::Footnote` nest, and every block walk in
/// `kasane-core` and `kasane-writer` recurses on that nesting. Past this
/// bound they stop descending, because the alternative is a stack overflow —
/// which aborts the process outright rather than surfacing as a recoverable
/// error.
///
/// This is a *safety* bound, not a fidelity one. The EPUB parser flattens at
/// a much lower depth without losing content
/// (`epub::xhtml::MAX_BLOCK_DEPTH`, <REPLACE: F>), and MOBI/AZW3 and PPTX
/// both reach the core through bounds of their own, so adapter-produced IR
/// never reaches this value. It exists for hand-built `Document`s from
/// external callers of the published `structure()`.
///
/// Measured, not guessed: on a libtest thread in a debug build, block depth
/// <REPLACE: M> completes and <REPLACE: N> aborts, so <REPLACE: S> keeps at
/// least a 4x margin under the tightest stack the suite runs on. The
/// measurement composed a full-depth inline chain underneath the block
/// chain, because a block walk at depth d can call an inline walk
/// `MAX_INLINE_DEPTH` deep and the two budgets add.
pub const MAX_BLOCK_DEPTH: usize = <REPLACE: S>;
```

Replace every `<REPLACE: …>` marker with the corresponding number from Task 1: `F` fidelity, `S` safety, `M` deepest survivable, `N` abort depth.

- [ ] **Step 2: Assert the ordering invariant holds**

Add to `crates/kasane-ir/src/lib.rs`'s `mod tests`:

```rust
    /// The two-bound design only works if adapter IR cannot reach the safety
    /// bound. `kasane-ir` cannot name the adapter constant (it depends on
    /// nothing), so this asserts the safety side of the contract: the value
    /// is large enough that the documented fidelity bound sits well under it.
    #[test]
    fn block_bound_leaves_room_under_the_inline_bound_convention() {
        assert!(MAX_BLOCK_DEPTH >= 4, "a bound under 4 cannot host a /4 fidelity bound");
        assert!(MAX_BLOCK_DEPTH.is_power_of_two());
    }
```

- [ ] **Step 3: Run and commit**

```bash
cargo test -p kasane-ir
mise run lint
git add crates/kasane-ir/src/lib.rs
git commit -m "feat(ir): add the block-nesting safety bound"
```

Expected: PASS.

---

### Task 4: Bound the two adapter-side walks

**Files:**
- Modify: `crates/kasane-adapters/src/epub/mod.rs:215`
- Modify: `crates/kasane-adapters/src/mobi/mod.rs:347`

**Interfaces:**
- Consumes: `kasane_ir::MAX_BLOCK_DEPTH` (Task 3).
- Produces: `fix_block_links(b, file, map, footnote_map, noteref_keys, depth: usize)` and `strip_empty_anchor_links(blocks: &mut Vec<Block>, depth: usize)` — both gain a trailing `depth` parameter, and every call site passes `0` at the top level.

These two are unreachable in practice after Task 2, because they only ever see this parser's output. They are bounded anyway so the invariant does not depend on which adapter feeds them (spec §3).

- [ ] **Step 1: Bound `fix_block_links`**

In `crates/kasane-adapters/src/epub/mod.rs`, change the signature and add the guard:

```rust
fn fix_block_links(
    b: &mut Block,
    file: &str,
    map: &std::collections::HashMap<(String, String), BlockId>,
    footnote_map: &std::collections::HashMap<(String, String), NoteId>,
    noteref_keys: &std::collections::HashSet<(String, String)>,
    depth: usize,
) {
    // Unreachable via this crate's own parser, which flattens at
    // `xhtml::MAX_BLOCK_DEPTH` -- but this walk runs on every EPUB and MOBI
    // parse, so the guard is what makes its safety independent of who built
    // the IR rather than contingent on it.
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        return;
    }
    match b {
```

Then update the two recursive calls inside the `Block::List` and `Block::Footnote` arms to pass `depth + 1`:

```rust
        Block::List { items, .. } => {
            for item in items {
                for ib in item {
                    fix_block_links(ib, file, map, footnote_map, noteref_keys, depth + 1);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                fix_block_links(ib, file, map, footnote_map, noteref_keys, depth + 1);
            }
        }
```

And the top-level call site (around line 211) passes `0`:

```rust
        fix_block_links(&mut n.block, &file, map, footnote_map, noteref_keys, 0);
```

- [ ] **Step 2: Bound `strip_empty_anchor_links`**

In `crates/kasane-adapters/src/mobi/mod.rs`:

```rust
fn strip_empty_anchor_links(blocks: &mut Vec<Block>, depth: usize) {
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        return;
    }
    for block in blocks.iter_mut() {
```

Update the two recursive calls:

```rust
            Block::List { items, .. } => {
                for item in items.iter_mut() {
                    strip_empty_anchor_links(item, depth + 1);
                }
            }
```

```rust
            Block::Footnote { blocks, .. } => strip_empty_anchor_links(blocks, depth + 1),
```

Find its call sites and pass `0`:

```bash
grep -n "strip_empty_anchor_links(" crates/kasane-adapters/src/mobi/mod.rs
```

Every call that is not one of the two recursive ones above takes `0`.

Leave `any_empty_anchor_link_in_blocks` alone — it is inside `mod tests` and runs only over a committed fixture, so bounding it would imply it is part of the production hazard surface when it is not.

- [ ] **Step 3: Build, test, commit**

```bash
cargo test -p kasane-adapters
mise run lint
git add crates/kasane-adapters/src/epub/mod.rs crates/kasane-adapters/src/mobi/mod.rs
git commit -m "fix(adapters): bound the two recursive block walks"
```

Expected: PASS. No behaviour change is expected for any existing fixture — these guards are unreachable at fixture depths.

---

### Task 5: Bound the core and writer walks

**Files:**
- Modify: `crates/kasane-core/src/section.rs:90`
- Modify: `crates/kasane-core/src/balance.rs:125-129`
- Modify: `crates/kasane-core/src/refs.rs:16`
- Modify: `crates/kasane-writer/src/markdown.rs:3-12`
- Test: `crates/kasane-core/src/nav.rs` (`mod tests`), `crates/kasane-writer/src/markdown.rs` (`mod tests`)

**Interfaces:**
- Consumes: `kasane_ir::MAX_BLOCK_DEPTH` (Task 3).
- Produces: `clone_block(b: &Block, depth: usize) -> Block`, `est_tokens_block(b: &Block, depth: usize) -> usize`, `fix_block(b: &mut Block, from: &str, anchors: &HashMap<BlockId, String>, depth: usize)`, `render_block(b: &Block, assets: &AssetBag, out: &mut String, depth: usize)`. The public entry points `est_tokens`, `blocks_to_markdown` and `resolve_refs` keep their current signatures and start their walks at `0`.

`clone_block` is the load-bearing one: it is the first core walk to touch the IR, so truncation happens there once and the other three see already-shallow blocks. Their guards are defence in depth, not four independent truncations (spec §3).

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/kasane-core/src/nav.rs`:

```rust
    /// The block-nesting analogue of
    /// `kasane_ir`'s `teardown_document_survives_deep_block_and_inline_nesting`:
    /// the drop side was already safe, the walk side was not. Depth 100_000
    /// is far past anything a real document holds -- the point is that the
    /// bound makes depth irrelevant, so an absurd value is the honest test.
    #[test]
    fn structure_survives_deep_block_nesting() {
        const DEPTH: usize = 100_000;
        let mut blocks = vec![Block::Para(vec![Inline::Text("bottom".into())])];
        for _ in 0..DEPTH {
            blocks = vec![Block::List {
                ordered: false,
                items: vec![blocks],
            }];
        }
        blocks = vec![Block::Footnote {
            id: kasane_ir::NoteId(1),
            blocks,
        }];
        let mut nodes = vec![Node {
            block: Block::Heading {
                level: 1,
                id: BlockId(0),
                inlines: vec![Inline::Text("T".into())],
            },
            prov: Provenance::default(),
        }];
        nodes.extend(blocks.into_iter().map(|block| Node {
            block,
            prov: Provenance::default(),
        }));
        let doc = Document {
            meta: DocMeta {
                title: "T".into(),
                authors: vec![],
                language: None,
                source_format: "test".into(),
                source_path: "t".into(),
            },
            nodes,
        };
        // Must return normally, not abort.
        let site = structure(
            doc,
            &Options {
                max_tokens: 4000,
                min_tokens: 100,
            },
        );
        assert!(!site.files.is_empty());
    }
```

Add to `mod tests` in `crates/kasane-writer/src/markdown.rs`:

```rust
    #[test]
    fn rendering_survives_deep_block_nesting() {
        const DEPTH: usize = 100_000;
        let mut blocks = vec![Block::Para(vec![Inline::Text("bottom".into())])];
        for _ in 0..DEPTH {
            blocks = vec![Block::List {
                ordered: false,
                items: vec![blocks],
            }];
        }

        // Must return normally, not abort.
        let md = blocks_to_markdown(&blocks, &kasane_ir::AssetBag { items: vec![] });

        // Order matters, exactly as `fuzz_entry::adapter`'s comment spells
        // out. `blocks` is 100_000 deep and `Block`'s derived `Drop` recurses,
        // so letting it fall out of scope aborts the process on the way out --
        // a second, independent stack overflow that would read as the code
        // under test failing when it had already returned cleanly. Tear it
        // down through the explicit worklist BEFORE the assertion, so nothing
        // owns it when a panic could unwind through this frame.
        kasane_ir::teardown_document(kasane_ir::Document {
            meta: kasane_ir::DocMeta {
                title: "T".into(),
                authors: vec![],
                language: None,
                source_format: "test".into(),
                source_path: "t".into(),
            },
            nodes: blocks
                .into_iter()
                .map(|block| kasane_ir::Node {
                    block,
                    prov: kasane_ir::Provenance::default(),
                })
                .collect(),
        });

        assert!(!md.is_empty());
    }
```

The core test in `nav.rs` needs no such teardown: `structure()` consumes its `Document` and tears it down internally, and the `site` it returns holds only already-truncated, shallow blocks.

Check the imports each `mod tests` already has and add only what is missing — the fully-qualified `kasane_ir::` paths above are deliberate so this test compiles regardless of what the module already imports.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-core --lib structure_survives_deep_block_nesting`

Expected: `fatal runtime error: stack overflow` and the test binary aborts (SIGABRT). This is a process abort, not an assertion failure — the whole test binary dies, so run this test on its own rather than reading it out of a full-suite run.

Run: `cargo test -p kasane-writer --lib rendering_survives_deep_block_nesting`

Expected: the same abort.

- [ ] **Step 3: Bound `clone_block`**

In `crates/kasane-core/src/section.rs`, change the signature and add the guard:

```rust
fn clone_block(b: &Block, depth: usize) -> Block {
    // The load-bearing truncation. This is the first core walk to touch
    // adapter or caller IR, so past this point every later core and writer
    // walk sees already-shallow blocks -- their own guards are defence in
    // depth, not a second truncation stacked on this one.
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        return Block::Raw {
            note: "nesting truncated at the block depth bound".into(),
        };
    }
    match b {
```

Update the two recursive uses inside it:

```rust
        Block::List { ordered, items } => Block::List {
            ordered: *ordered,
            items: items
                .iter()
                .map(|item| item.iter().map(|b| clone_block(b, depth + 1)).collect())
                .collect(),
        },
```

```rust
        Block::Footnote { id, blocks } => Block::Footnote {
            id: *id,
            blocks: blocks.iter().map(|b| clone_block(b, depth + 1)).collect(),
        },
```

And the call site at `crates/kasane-core/src/section.rs:69`:

```rust
                top.body.push(clone_block(other, 0));
```

- [ ] **Step 4: Bound `est_tokens_block`**

In `crates/kasane-core/src/balance.rs`, thread depth through both functions:

```rust
pub(crate) fn est_tokens_blocks(blocks: &[Block]) -> usize {
    est_tokens_blocks_at(blocks, 0)
}

fn est_tokens_blocks_at(blocks: &[Block], depth: usize) -> usize {
    blocks.iter().map(|b| est_tokens_block(b, depth)).sum()
}

fn est_tokens_block(b: &Block, depth: usize) -> usize {
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        // Not zero: a truncated subtree still renders as a Raw note, so the
        // size guard must not believe it is free.
        return 1;
    }
```

Then the two recursive arms inside `est_tokens_block`:

```rust
        Block::List { items, .. } => items
            .iter()
            .flatten()
            .map(|b| est_tokens_block(b, depth + 1))
            .sum(),
```

```rust
        Block::Footnote { blocks, .. } => est_tokens_blocks_at(blocks, depth + 1),
```

Note the `Block::List` arm's value feeds `chars / 4 + 1` at the end of the function while the `Footnote` arm's does too — keep both inside the `chars` match exactly as they are today; only the recursive call changes.

- [ ] **Step 5: Bound `fix_block`**

In `crates/kasane-core/src/refs.rs`:

```rust
fn fix_block(b: &mut Block, from: &str, anchors: &HashMap<BlockId, String>, depth: usize) {
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        return;
    }
    match b {
```

The two recursive arms:

```rust
        Block::List { items, .. } => {
            for it in items {
                for bb in it {
                    fix_block(bb, from, anchors, depth + 1);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for bb in blocks {
                fix_block(bb, from, anchors, depth + 1);
            }
        }
```

And the call site in `resolve_refs` (line 11):

```rust
    for b in &mut placed.node.body {
        fix_block(b, &from, anchors, 0);
    }
```

- [ ] **Step 6: Bound `render_block`**

In `crates/kasane-writer/src/markdown.rs`:

```rust
pub fn blocks_to_markdown(blocks: &[Block], assets: &AssetBag) -> String {
    blocks_to_markdown_at(blocks, assets, 0)
}

fn blocks_to_markdown_at(blocks: &[Block], assets: &AssetBag, depth: usize) -> String {
    let mut out = String::new();
    for b in blocks {
        render_block(b, assets, &mut out, depth);
        out.push('\n');
    }
    out
}

fn render_block(b: &Block, assets: &AssetBag, out: &mut String, depth: usize) {
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        out.push_str("<!-- nesting truncated at the block depth bound -->\n");
        return;
    }
    match b {
```

The two recursive arms:

```rust
        Block::List { ordered, items } => {
            for (i, item) in items.iter().enumerate() {
                if *ordered {
                    out.push_str(&format!("{}. ", i + 1));
                } else {
                    out.push_str("- ");
                }
                // render first block inline, subsequent blocks indented
                let mut inner = String::new();
                for bb in item {
                    render_block(bb, assets, &mut inner, depth + 1);
                }
                out.push_str(inner.trim_end());
                out.push('\n');
            }
        }
```

```rust
        Block::Footnote { id, blocks } => {
            let body = blocks_to_markdown_at(blocks, assets, depth + 1);
            out.push_str(&format!("[^{}]: {}\n", id.0, body.trim()));
        }
```

The emitted note text matches `clone_block`'s `Block::Raw` note, rendered through the same `<!-- … -->` shape `Block::Raw` already uses — so a reader sees one message regardless of which guard fired.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p kasane-core --lib structure_survives_deep_block_nesting`
Expected: PASS (returns normally instead of aborting).

Run: `cargo test -p kasane-writer --lib rendering_survives_deep_block_nesting`
Expected: PASS.

- [ ] **Step 8: Run the full suite and lint**

Run: `mise run lint && mise run test`

Expected: all green. If a `properties.proptest-regressions` file appears under `crates/kasane-writer/tests/`, commit it with this task — it is what replays the failing case from then on.

- [ ] **Step 9: Commit**

```bash
git add crates/kasane-core/src/section.rs crates/kasane-core/src/balance.rs \
        crates/kasane-core/src/refs.rs crates/kasane-core/src/nav.rs \
        crates/kasane-writer/src/markdown.rs
git add crates/kasane-writer/tests/properties.proptest-regressions 2>/dev/null || true
git commit -m "fix(core): bound block recursion so deep nesting cannot abort the process"
```

---

### Task 6: The fuzz assertion, the seed, and the CLI end-to-end test

**Files:**
- Modify: `crates/kasane-adapters/src/fuzz_entry.rs`
- Create: `fuzz/seeds/epub/deep-blocks.epub`
- Create or modify: the kasane-cli integration test file (find it with `ls crates/kasane-cli/tests/`)

**Interfaces:**
- Consumes: `epub::xhtml::MAX_BLOCK_DEPTH` (Task 2), `kasane_ir::MAX_BLOCK_DEPTH` (Task 3).
- Produces: `fn max_block_depth(doc: &Document) -> usize` and `fn assert_block_depth_bounded(depth: usize)` in `fuzz_entry.rs`, called from the shared `adapter()` seam.

- [ ] **Step 1: Add the iterative block-depth checker**

In `crates/kasane-adapters/src/fuzz_entry.rs`, beside `max_inline_depth`:

```rust
/// Design spec `2026-07-29-block-nesting-depth-bound-design.md`: `kasane-core`
/// and `kasane-writer` walk BLOCK nesting (`Block::List`/`Block::Footnote`)
/// recursively, so IR nested past `kasane_ir::MAX_BLOCK_DEPTH` aborts the
/// process on a stack overflow rather than failing recoverably. No adapter
/// may produce it. Checked against the core's safety bound rather than any
/// one adapter's flattening bound, because the core's is the value that
/// decides whether the process survives -- the same rule
/// `assert_depth_bounded` applies to inline nesting.
///
/// The traversal is iterative for the reason `max_inline_depth`'s comment
/// gives at length: assuming the nesting is already shallow before checking
/// whether it's shallow is circular, and a recursive checker overflows its
/// own stack on exactly the input it exists to catch -- which reads as a
/// crash in the test code rather than in the code under test.
fn max_block_depth(doc: &Document) -> usize {
    let mut max_depth = 0;
    let mut stack: Vec<(&Block, usize)> = doc.nodes.iter().map(|n| (&n.block, 0)).collect();
    while let Some((b, depth)) = stack.pop() {
        match b {
            Block::List { items, .. } => {
                max_depth = max_depth.max(depth + 1);
                for item in items {
                    stack.extend(item.iter().map(|bb| (bb, depth + 1)));
                }
            }
            Block::Footnote { blocks, .. } => {
                max_depth = max_depth.max(depth + 1);
                stack.extend(blocks.iter().map(|bb| (bb, depth + 1)));
            }
            // Leaves, enumerated rather than caught by a wildcard: a new
            // nesting variant must break this build, not silently make the
            // depth this function reports blind to it. Same reasoning as
            // `kasane_ir::teardown_document`'s exhaustive match.
            Block::Heading { .. }
            | Block::Para(_)
            | Block::Table(_)
            | Block::Figure { .. }
            | Block::CodeBlock { .. }
            | Block::MathBlock(_)
            | Block::Raw { .. } => {}
        }
    }
    max_depth
}

fn assert_block_depth_bounded(depth: usize) {
    assert!(
        depth <= kasane_ir::MAX_BLOCK_DEPTH,
        "adapter produced block nesting {depth} deep, past kasane_ir::MAX_BLOCK_DEPTH ({}). \
         Every core and writer block walk recurses on this, so the next stage would abort \
         the process on a stack overflow.",
        kasane_ir::MAX_BLOCK_DEPTH
    );
}
```

- [ ] **Step 2: Call it from the shared seam**

In `crates/kasane-adapters/src/fuzz_entry.rs`, update `fn adapter`:

```rust
fn adapter(a: &dyn Adapter, data: &[u8], source_path: &str) {
    if let Ok((doc, assets)) = a.parse(data, source_path) {
        let depth = max_inline_depth(&doc);
        let block_depth = max_block_depth(&doc);
        kasane_ir::teardown_document(doc);
        assert_assets_contained(&assets);
        assert_depth_bounded(depth);
        assert_block_depth_bounded(block_depth);
    }
}
```

Both depths are computed **before** `teardown_document`, for the reason the existing doc comment on this function already spells out: nothing may own `doc` when an assertion can panic, or unwinding drops it recursively and turns a clean assertion failure into a second stack-overflow abort.

- [ ] **Step 3: Correct `max_inline_depth`'s doc comment**

Its second paragraph currently states that block nesting "is **not bounded anywhere in this codebase**" and that this function does not check it. Both halves are now false. Replace that paragraph with:

```rust
/// BLOCK nesting (`Block::List`/`Block::Footnote`) is a different property
/// with its own bound and its own check -- see `max_block_depth` below.
/// Nesting a list contributes nothing to the value computed here; only the
/// inline content reachable through it does.
```

- [ ] **Step 4: Commit the seed**

Copy the reproducer built in Task 2 Step 9:

```bash
cp /tmp/deepblocks/deep-blocks.epub fuzz/seeds/epub/deep-blocks.epub
ls -la fuzz/seeds/epub/
```

Expected: `deep-blocks.epub` sits beside the existing `deep-nesting.epub` (which is the *inline* seed — the two are different properties and both stay).

- [ ] **Step 5: Verify the stable replay runs the new seed**

Run: `cargo test -p kasane-adapters --test fuzz_corpus -- --nocapture`

Expected: PASS, and the output shows the epub target replaying more seeds than before. `KNOWN_OPEN` stays `&[]` — the bug is fixed in this branch, so nothing is quarantined.

- [ ] **Step 6: Add the batch-mode end-to-end test**

Add to `crates/kasane-cli/tests/e2e.rs`, directly after the existing
`converts_a_deeply_nested_epub_without_aborting` (line 24), whose shape this
copies. That one covers the *inline* seed in single-file mode; this one covers
the *block* seed in batch mode. Both stay.

```rust
#[test]
fn batch_mode_converts_a_deeply_block_nested_epub_without_aborting() {
    // fuzz/seeds/epub/deep-blocks.epub nests <ul> 30,000 deep. Batch mode
    // specifically: conversions run on rayon workers, which get a smaller
    // stack than `main` -- measured at roughly a quarter of the survivable
    // depth (design spec 2026-07-29 §1). A single-file version of this test
    // would pass against a bound four times too loose, so the directory
    // argument here is load-bearing, not incidental.
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("books");
    std::fs::create_dir_all(&books).unwrap();
    std::fs::copy(
        "../../fuzz/seeds/epub/deep-blocks.epub",
        books.join("deep-blocks.epub"),
    )
    .unwrap();
    let out_dir = tmp.path().join("out");

    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg(&books)
        .arg("-o")
        .arg(&out_dir)
        .status()
        .unwrap();

    assert!(
        status.success(),
        "batch conversion of a 30,000-deep list must exit 0, not abort with 134: {status:?}"
    );
    // The library index proves batch mode ran, not single-file mode.
    assert!(out_dir.join("index.md").exists(), "library index missing");
    // And the content survived the flattening rather than being truncated:
    // the innermost list item's text is the seed's only body text.
    let all = read_all_md(&out_dir);
    assert!(
        all.contains('x'),
        "innermost list item text missing -- the bound must flatten, not truncate"
    );
}
```

`read_all_md` is the existing helper at `crates/kasane-cli/tests/e2e.rs:149`, and
`tempfile` and `std::process::Command` are already in scope in that file.

- [ ] **Step 7: Run the full suite and lint**

Run: `mise run lint && mise run test`

Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-adapters/src/fuzz_entry.rs fuzz/seeds/epub/deep-blocks.epub \
        crates/kasane-cli/tests/
git commit -m "test(fuzz): assert bounded block depth and seed the deep-list EPUB"
```

---

### Task 7: Extend the property generator to nested lists

**Files:**
- Modify: `crates/kasane-writer/tests/generator/` (read the directory first to find the block-shape generator)

**Interfaces:**
- Consumes: nothing from earlier tasks at compile time.
- Produces: a `Shape::NestedList(bool, u8)` variant in `crates/kasane-writer/tests/generator/mod.rs`, exercised by the six existing invariants.

Today `Shape::List` (line 128) builds exactly one level — `items: vec![vec![Block::Para(text(token))]]` — so conservation, the size guard and link resolution have never covered *nested* lists at all.

The generator is not built from composed proptest strategies per block; it draws a `Shape` enum value (line 90) and `build` (line 112) turns each shape into one concrete `Block` plus its `Expect` count. So the change is a new `Shape` variant, not a `prop_recursive` wrapper.

- [ ] **Step 1: Add the shape variant**

In `crates/kasane-writer/tests/generator/mod.rs`, add to the `Shape` enum beside `List(bool)` (line 81):

```rust
    /// `ordered` plus a nesting depth. Kept well under
    /// `epub::xhtml::MAX_BLOCK_DEPTH` on purpose: this tier is
    /// adapter-realistic by design, and IR deeper than any adapter can
    /// produce is the safety bound's unit tests' job, not this tier's.
    NestedList(bool, u8),
```

- [ ] **Step 2: Draw it**

In `fn shape()` (line 90), add to the `prop_oneof!`, giving it the same weight the flat list has:

```rust
        2 => (any::<bool>(), 2u8..=4).prop_map(|(o, d)| Shape::NestedList(o, d)),
```

- [ ] **Step 3: Build it**

In `fn build` (line 112), add an arm beside `Shape::List`:

```rust
        // The token sits at the bottom of the chain and renders exactly once,
        // so the conservation invariant's arithmetic is unchanged from the
        // flat-list case -- what changes is the depth the walks must survive
        // to reach it.
        Shape::NestedList(ordered, depth) => {
            let mut inner = vec![Block::Para(text(token))];
            for _ in 0..*depth {
                inner = vec![Block::List {
                    ordered: *ordered,
                    items: vec![inner],
                }];
            }
            (inner.pop().expect("depth >= 1 builds one list"), Expect::Exactly(1))
        }
```

`depth` is drawn from `2..=4`, so the loop always runs at least twice and `inner` always ends as a single `Block::List` — the `expect` is unreachable, and stating why in the message is what keeps it honest.

- [ ] **Step 4: Run the property suite**

Run: `cargo test -p kasane-writer --test properties`

Expected: PASS. If an invariant fails, that is a real finding, not a generator bug — read the shrunk counterexample before changing anything. The most likely genuine failure is conservation: `blocks_to_markdown` renders a list item's blocks into a nested buffer and calls `.trim_end()` on it, so a token at the bottom of a deep chain must still survive that trimming exactly once.

- [ ] **Step 5: Commit, including any regressions file**

```bash
mise run lint && mise run test
git add crates/kasane-writer/tests/
git add crates/kasane-writer/tests/properties.proptest-regressions 2>/dev/null || true
git commit -m "test(writer): generate nested lists in the property tier"
```

If `properties.proptest-regressions` was written, it **must** be committed — it is what replays the found case on every subsequent run.

---

### Task 8: Documentation

**Files:**
- Modify: `README.md`
- Modify: `AGENTS.md`
- Modify: `crates/kasane-core/src/nav.rs` (the comment block at lines 12–32)
- Modify: `crates/kasane-adapters/src/mobi/normalize.rs` (the `MAX_DEPTH` comment)

**Interfaces:**
- Consumes: everything built in Tasks 2–7.
- Produces: no code. This task exists because four documents currently assert block nesting is unbounded, and leaving any of them would make the repo contradict itself.

- [ ] **Step 1: Rewrite README's Known limitations entry**

Replace the bullet beginning "Block nesting has no depth bound." with:

```markdown
- Block nesting is bounded, and deep nesting flattens rather than failing.
  Lists and footnotes nested past the EPUB parser's fidelity bound stop
  producing further nesting: an over-deep list's items become siblings at the
  bound's level and an over-deep footnote container becomes transparent. Every
  text run survives — only the nesting relationship past the bound is lost.
  A ~540 KB EPUB holding a 30,000-deep `<ul>` converts normally, where older
  builds aborted with `fatal runtime error: stack overflow` (SIGABRT, shell
  exit 134). The bound applies to EPUB and MOBI/AZW3 alike, since MOBI
  re-serializes through the same parser. A second, higher bound in
  `kasane-ir` protects the structuring engine and writer from hand-built
  `Document`s passed to `structure()` by external callers; past it a
  truncation note is emitted in place of the over-deep subtree.
```

- [ ] **Step 2: Correct AGENTS.md**

In the Conventions section, replace the paragraph beginning "BLOCK nesting (`Block::List`/`Block::Footnote`) is bounded **nowhere**." with:

```markdown
  BLOCK nesting (`Block::List`/`Block::Footnote`) is bounded the same way, and
  by the same two-constant shape: `epub::xhtml::MAX_BLOCK_DEPTH` is the
  fidelity bound that flattens without losing content (and covers MOBI/AZW3
  too, which re-serializes through that parser), while
  `kasane_ir::MAX_BLOCK_DEPTH` is the safety bound in the recursive walks.
  Six production walks recurse on block nesting and all six carry the bound:
  `epub::fix_block_links` and `mobi::strip_empty_anchor_links` in the adapters
  — both of which run during `parse`, before `kasane-core` is reached — plus
  `section::clone_block`, `balance::est_tokens_block`, `refs::fix_block` and
  `kasane_writer::blocks_to_markdown`. `clone_block` is the load-bearing one:
  it is the first core walk to touch the IR, so the later three see
  already-shallow blocks. The drop side is separately safe via
  `kasane_ir::teardown_document`'s explicit worklist.
```

- [ ] **Step 3: Correct `nav.rs`'s comment**

In `crates/kasane-core/src/nav.rs`, the comment beginning "Inline, and only inline. BLOCK nesting …" through "The walk and clone sides are not." asserts the old state throughout. Replace that passage with:

```rust
    // Both kinds of nesting are now bounded on every side. Block nesting has
    // its own pair of constants (`epub::xhtml::MAX_BLOCK_DEPTH` for fidelity,
    // `kasane_ir::MAX_BLOCK_DEPTH` for safety) and every recursive block walk
    // in this crate and the writer carries the safety bound. The drop side
    // was already safe and stays so: `teardown_document` pops blocks from an
    // explicit worklist, which is why this call is still here rather than
    // letting `doc` fall out of scope — a bounded walk protects the walk, not
    // the compiler-derived `Drop` that runs afterwards.
```

- [ ] **Step 4: Add the sentence to MOBI's `MAX_DEPTH`**

In `crates/kasane-adapters/src/mobi/normalize.rs`, append to the comment above `const MAX_DEPTH: usize = 500;`:

```rust
// This bound is about `serialize`'s own mutual recursion and nothing further
// downstream. It is no longer the value that decides whether the process
// survives: the XHTML parser this feeds flattens block nesting at
// `epub::xhtml::MAX_BLOCK_DEPTH`, far tighter than 500, so IR reaching the
// core is bounded by that instead.
```

- [ ] **Step 5: Verify no document still claims the old state**

```bash
grep -rn "bounded \*\*nowhere\*\*\|not bounded anywhere\|no depth bound" \
     README.md AGENTS.md crates/ docs/superpowers/specs/2026-07-29-block-nesting-depth-bound-design.md
```

Expected: the only surviving hits are inside the design spec's own history sections (§1, §3.1, §7), which describe the pre-fix state deliberately. Any hit in `README.md`, `AGENTS.md`, or under `crates/` is a miss — fix it.

- [ ] **Step 6: Verify the docs match the code**

```bash
grep -n "MAX_BLOCK_DEPTH" AGENTS.md crates/kasane-ir/src/lib.rs \
     crates/kasane-adapters/src/epub/xhtml.rs
```

Expected: the constants named in `AGENTS.md` exist in the source with the paths given, and the two values satisfy fidelity < safety.

- [ ] **Step 7: Full verification and commit**

```bash
mise run lint && mise run test
git add README.md AGENTS.md crates/kasane-core/src/nav.rs \
        crates/kasane-adapters/src/mobi/normalize.rs
git commit -m "docs: record the block-nesting bounds and correct the walk inventory"
```

---

## Final verification

- [ ] `mise run lint && mise run test` green from a clean checkout of the branch.
- [ ] The 30,000-deep reproducer converts with exit 0 in **batch** mode: `cargo run -q -p kasane-cli -- <dir containing deep-blocks.epub> -o /tmp/final-check`.
- [ ] `cargo test -p kasane-adapters --test fuzz_corpus` replays the new `deep-blocks.epub` seed, and `KNOWN_OPEN` is still `&[]`.
- [ ] Both constants' doc comments name a measured figure and the thread it was measured on.
- [ ] `epub::xhtml::MAX_BLOCK_DEPTH < kasane_ir::MAX_BLOCK_DEPTH`, with the fidelity bound at most a quarter of the safety bound.
- [ ] Every `properties.proptest-regressions` file produced during implementation is committed.
- [ ] No file outside the design spec claims block nesting is unbounded.
