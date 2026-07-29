# Structuring-Engine Property Tier Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build design spec §9's `proptest` property tier over `kasane-core` + `kasane-writer`, and close the two defects that designing its invariants uncovered.

**Architecture:** One new test target in `kasane-writer` (the only crate already depending on both `kasane-ir` and `kasane-core`) generates adapter-realistic `Document`s, runs `structure()`, renders each file in memory, and asserts six invariants against the resulting Markdown. Two defect fixes land first, because the link invariant cannot pass until the writer emits a title heading, and because the suite must not be written against known-broken behavior.

**Tech Stack:** Rust 2021, `proptest` 1.11 (dev-dependency only), the existing pinned stable toolchain, `mise` tasks.

**Spec:** `docs/superpowers/specs/2026-07-29-core-property-tier-design.md`

## Global Constraints

- Every change ships green under `mise run lint && mise run test`.
- `mise run lint` is `cargo fmt --all -- --check` **plus** `cargo clippy --workspace --all-targets -- -D warnings`. `--all-targets` matters: test code is linted too.
- `proptest` is a **dev-dependency** of `kasane-writer` only. No published crate gains a runtime dependency.
- No new workflow file, no new pinned tool in `mise.toml`, no `PROPTEST_CASES` environment variable.
- Adapters must never trust input. Guards at the untrusted boundary follow the existing `guard.rs` style: a named `const`, a doc comment saying what it bounds and why.
- Internals a test needs are exposed as `#[doc(hidden)] pub`, matching the `kasane_adapters::fuzz_entry` precedent. Never widen to plain `pub`.
- A crash reproducer or seed that proves a bug is committed under `fuzz/seeds/<target>/`; `proptest-regressions/` files are committed for the same reason.
- Depth bounds are fixed values from the spec: `kasane_ir::MAX_INLINE_DEPTH = 256` (core/writer safety bound), `epub::xhtml::MAX_INLINE_DEPTH = 64` (adapter fidelity bound).

## File Structure

| File | Responsibility |
|---|---|
| `crates/kasane-ir/src/lib.rs` | **Modify** — export `MAX_INLINE_DEPTH` |
| `crates/kasane-core/src/paths.rs` | **Modify** — depth-bound `inline_text`; body-heading anchors in `place`; `slug_of` seam |
| `crates/kasane-core/src/balance.rs` | **Modify** — depth-bound `est_tokens_block`; saturating level; merge demotion; `est_tokens` seam |
| `crates/kasane-core/src/refs.rs` | **Modify** — depth-bound `fix_inlines` |
| `crates/kasane-core/src/lib.rs` | **Modify** — re-export the two seams |
| `crates/kasane-writer/src/markdown.rs` | **Modify** — depth-bound `inlines_to_md` |
| `crates/kasane-writer/src/lib.rs` | **Modify** — add `file_to_markdown`, use it in `write_tree_contents` |
| `crates/kasane-adapters/src/epub/xhtml.rs` | **Modify** — `MAX_INLINE_DEPTH` flattening guard |
| `crates/kasane-adapters/src/fuzz_entry.rs` | **Modify** — `assert_inline_depth_bounded` in the shared `adapter()` helper |
| `tests/fixtures/epub/make_deep_nesting_epub.py` | **Create** — generates the deep-nesting fixture and fuzz seed |
| `fuzz/seeds/epub/deep-nesting.epub` | **Create** — committed seed |
| `crates/kasane-writer/tests/generator/mod.rs` | **Create** — adapter-realistic `Document` strategies + sentinel stamping |
| `crates/kasane-writer/tests/properties.rs` | **Create** — P1–P6 |
| `AGENTS.md`, `README.md` | **Modify** — codebase map, seams, testing section |

Task order is load-bearing: Task 6 (the properties) cannot pass before Task 4 (title heading), and Task 3's fuzz assertion cannot pass before Tasks 1–2 bound the depth.

---

### Task 1: Depth-bound the inline recursion in core and writer

Closes the core half of spec §2.2. `inline_text`, `est_tokens_block`, `fix_inlines` and `inlines_to_md` all recurse on inline nesting with no bound; a deeply nested `Document` aborts the process.

**Files:**
- Modify: `crates/kasane-ir/src/lib.rs`
- Modify: `crates/kasane-core/src/paths.rs:84-95`
- Modify: `crates/kasane-core/src/balance.rs:74-95`
- Modify: `crates/kasane-core/src/refs.rs:48-88`
- Modify: `crates/kasane-writer/src/markdown.rs:113-131`
- Test: `crates/kasane-writer/tests/inline_depth.rs` (create)

**Interfaces:**
- Produces: `kasane_ir::MAX_INLINE_DEPTH: usize` (= 256), used by `kasane-core` and `kasane-writer`.

- [ ] **Step 1: Write the failing test**

Create `crates/kasane-writer/tests/inline_depth.rs`:

```rust
//! Deeply nested inlines must not abort the process. See design spec
//! `2026-07-29-core-property-tier-design.md` §2.2: every inline walk in the
//! core and the writer recurses on nesting depth, and an unbounded walk
//! overflows the stack — which aborts, and in batch mode takes every other
//! worker's document down with it.

use kasane_core::{structure, Options};
use kasane_ir::*;

fn nested(depth: usize) -> Inline {
    let mut i = Inline::Text("x".into());
    for _ in 0..depth {
        i = Inline::Emph(vec![i]);
    }
    i
}

fn doc_with(inline: Inline) -> Document {
    Document {
        meta: DocMeta {
            title: "T".into(),
            authors: vec![],
            language: None,
            source_format: "epub".into(),
            source_path: "t".into(),
        },
        nodes: vec![Node {
            block: Block::Para(vec![inline]),
            prov: Provenance::default(),
        }],
    }
}

#[test]
fn deep_inline_nesting_does_not_abort() {
    let site = structure(doc_with(nested(10_000)), &Options::default());
    let md = kasane_writer::blocks_to_markdown(&site.files[0].blocks, &AssetBag::default());
    assert!(!md.is_empty(), "rendering must produce output, not abort");
}

#[test]
fn nesting_within_the_bound_is_preserved() {
    // Depth 8 is far under the bound: the text at the bottom must survive.
    let site = structure(doc_with(nested(8)), &Options::default());
    let md = kasane_writer::blocks_to_markdown(&site.files[0].blocks, &AssetBag::default());
    assert!(md.contains('x'), "content within the bound must not be dropped");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p kasane-writer --test inline_depth`

Expected: the process **aborts** — `fatal runtime error: stack overflow, aborting` and `signal: 6, SIGABRT`. This is the red state; a stack overflow is not a catchable failure, so the whole test binary dies rather than reporting one failed test.

- [ ] **Step 3: Add the shared constant**

In `crates/kasane-ir/src/lib.rs`, after the `pub use` block:

```rust
/// Maximum inline nesting depth the structuring engine and the writer will
/// descend.
///
/// Every inline walk in `kasane-core` and `kasane-writer` recurses on nesting
/// depth. Past this bound they stop descending and contribute nothing, because
/// the alternative is a stack overflow — which aborts the process outright
/// rather than surfacing as a recoverable error.
///
/// This is a *safety* bound, not a fidelity one. The EPUB adapter flattens at a
/// much lower depth without losing content (`epub::xhtml::MAX_INLINE_DEPTH`), so
/// adapter-produced IR never reaches this value; it exists for hand-built
/// `Document`s from external callers of the published `structure()`.
///
/// Measured, not guessed: in a debug build on a libtest thread, depth 256 and
/// 1024 both complete and 4096 aborts, so 256 keeps at least a 4x margin under
/// the tightest stack the suite runs on.
pub const MAX_INLINE_DEPTH: usize = 256;
```

- [ ] **Step 4: Bound `inline_text` and add the `slug_of` seam's dependency**

In `crates/kasane-core/src/paths.rs`, replace the whole `inline_text` function (lines 84-95):

```rust
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

- [ ] **Step 5: Bound `est_tokens_block`**

In `crates/kasane-core/src/balance.rs`, replace the nested `inl` helper inside `est_tokens_block` (lines 75-84):

```rust
fn est_tokens_block(b: &Block) -> usize {
    fn inl_at(is: &[Inline], depth: usize) -> usize {
        if depth >= kasane_ir::MAX_INLINE_DEPTH {
            return 0;
        }
        is.iter()
            .map(|i| match i {
                Inline::Text(s) | Inline::Code(s) | Inline::Math(s) => s.len(),
                Inline::Emph(x) | Inline::Strong(x) => inl_at(x, depth + 1),
                Inline::Link { inlines, .. } => inl_at(inlines, depth + 1),
                Inline::FootnoteRef(_) => 4,
            })
            .sum()
    }
    fn inl(is: &[Inline]) -> usize {
        inl_at(is, 0)
    }
    let chars = match b {
```

The rest of `est_tokens_block` (the `match b { ... }` body and `chars / 4 + 1`) is unchanged — `inl` keeps the same call signature, so every existing call site still compiles.

- [ ] **Step 6: Bound `fix_inlines`**

In `crates/kasane-core/src/refs.rs`, replace `fix_inlines` and `fix_inline` (lines 48-88):

```rust
fn fix_inlines(inls: &mut Vec<Inline>, from: &str, anchors: &HashMap<BlockId, String>) {
    fix_inlines_at(inls, from, anchors, 0);
}

fn fix_inlines_at(
    inls: &mut Vec<Inline>,
    from: &str,
    anchors: &HashMap<BlockId, String>,
    depth: usize,
) {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return;
    }
    let mut out = Vec::with_capacity(inls.len());
    for inl in std::mem::take(inls) {
        out.push(fix_inline(inl, from, anchors, depth));
    }
    *inls = out;
}

fn fix_inline(
    inl: Inline,
    from: &str,
    anchors: &HashMap<BlockId, String>,
    depth: usize,
) -> Inline {
    match inl {
        Inline::Link {
            target: RefTarget::Internal(id),
            mut inlines,
        } => {
            fix_inlines_at(&mut inlines, from, anchors, depth + 1);
            match anchors.get(&id) {
                Some(target) => Inline::Link {
                    target: RefTarget::External(relativize(from, target)),
                    inlines,
                },
                None => Inline::Emph(vec![]).replace_with_text(inlines), // strip: keep child text
            }
        }
        Inline::Link {
            target,
            mut inlines,
        } => {
            fix_inlines_at(&mut inlines, from, anchors, depth + 1);
            Inline::Link { target, inlines }
        }
        Inline::Emph(mut x) => {
            fix_inlines_at(&mut x, from, anchors, depth + 1);
            Inline::Emph(x)
        }
        Inline::Strong(mut x) => {
            fix_inlines_at(&mut x, from, anchors, depth + 1);
            Inline::Strong(x)
        }
        other => other,
    }
}
```

- [ ] **Step 7: Bound `inlines_to_md`**

In `crates/kasane-writer/src/markdown.rs`, replace `inlines_to_md` (lines 113-131):

```rust
pub(crate) fn inlines_to_md(inls: &[Inline]) -> String {
    inlines_to_md_at(inls, 0)
}

fn inlines_to_md_at(inls: &[Inline], depth: usize) -> String {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return String::new();
    }
    let mut s = String::new();
    for i in inls {
        match i {
            Inline::Text(t) => s.push_str(t),
            Inline::Emph(x) => s.push_str(&format!("*{}*", inlines_to_md_at(x, depth + 1))),
            Inline::Strong(x) => s.push_str(&format!("**{}**", inlines_to_md_at(x, depth + 1))),
            Inline::Code(t) => s.push_str(&format!("`{}`", t)),
            Inline::Math(t) => s.push_str(&format!("${}$", t)),
            Inline::Link {
                target: RefTarget::External(u),
                inlines,
            } => s.push_str(&format!("[{}]({})", inlines_to_md_at(inlines, depth + 1), u)),
            Inline::Link { inlines, .. } => s.push_str(&inlines_to_md_at(inlines, depth + 1)),
            Inline::FootnoteRef(n) => s.push_str(&format!("[^{}]", n.0)),
        }
    }
    s
}
```

- [ ] **Step 8: Run the test to verify it passes**

Run: `cargo test -p kasane-writer --test inline_depth`

Expected: PASS, 2 tests, no abort.

- [ ] **Step 9: Run the full suite and lint**

Run: `mise run lint && mise run test`

Expected: all green. If `clippy` flags `format!` inside `push_str`, keep the existing style — it matches the surrounding code and the lint config already tolerates it in this file.

- [ ] **Step 10: Commit**

```bash
git add crates/kasane-ir/src/lib.rs crates/kasane-core/src/paths.rs \
        crates/kasane-core/src/balance.rs crates/kasane-core/src/refs.rs \
        crates/kasane-writer/src/markdown.rs crates/kasane-writer/tests/inline_depth.rs
git commit -m "fix(core): bound inline recursion so deep nesting cannot abort the process"
```

---

### Task 2: Flatten deep inline nesting in the EPUB XHTML parser

Closes the adapter half of spec §2.2. The parser's frame stack applies no depth limit, so a hostile book produces the IR Task 1 now merely survives. This bound preserves content instead of dropping it.

**Files:**
- Modify: `crates/kasane-adapters/src/epub/xhtml.rs:537-538, 843-857`
- Test: `crates/kasane-adapters/src/epub/xhtml.rs` (its existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing from Task 1 (independent; both bound the same shape from different sides).
- Produces: `epub::xhtml::MAX_INLINE_DEPTH: usize` (= 64), asserted by Task 3.

- [ ] **Step 1: Write the failing test**

Append to the `mod tests` block at the bottom of `crates/kasane-adapters/src/epub/xhtml.rs`:

```rust
#[test]
fn deep_inline_nesting_is_flattened_not_preserved() {
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

    // 300 nested <em>, well past MAX_INLINE_DEPTH.
    let n = 300;
    let xml = format!(
        "<body><p>{}deep{}</p></body>",
        "<em>".repeat(n),
        "</em>".repeat(n)
    );
    let blocks = parse_blocks(&xml);

    let inls = blocks
        .iter()
        .find_map(|b| match b {
            Block::Para(i) => Some(i),
            _ => None,
        })
        .expect("a paragraph");
    assert!(
        depth_of(inls) <= MAX_INLINE_DEPTH,
        "nesting must be flattened to the bound, got {}",
        depth_of(inls)
    );
    // Flattening preserves content: this is a fidelity bound, not a safety one.
    assert!(
        inlines_text(inls).contains("deep"),
        "flattening must keep the text"
    );
}
```

`parse_blocks(xml: &str) -> Vec<Block>` and `inlines_text` are both already in scope in this `mod tests` (the helper is defined at `xhtml.rs:1031`, and `use super::*` brings in `inlines_text`).

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p kasane-adapters deep_inline_nesting_is_flattened -- --nocapture`

Expected: FAIL — either `cannot find value MAX_INLINE_DEPTH in this scope` (compile error, since the constant does not exist yet), or once the constant exists, an assertion failure reporting depth 300.

- [ ] **Step 3: Add the constant**

Near the top of `crates/kasane-adapters/src/epub/xhtml.rs`, beside the other module-level items:

```rust
/// Maximum inline nesting this parser preserves as nested `Inline` values.
///
/// The frame stack that builds inlines is iterative, so parsing arbitrarily
/// nested `<em>`/`<strong>`/`<a>` never overflows here — but it hands the core
/// and the writer an `Inline` tree they walk recursively. Bounding the produced
/// depth is what keeps a hostile book from reaching
/// `kasane_ir::MAX_INLINE_DEPTH`.
///
/// This is a *fidelity* bound, not a safety one: past it a closing inline tag
/// contributes its text instead of another wrapper, so no content is lost. 64 is
/// far past any real book's `<em><strong><a>` layering.
pub(crate) const MAX_INLINE_DEPTH: usize = 64;
```

- [ ] **Step 4: Add the depth-aware wrapper helper**

In the same file, beside `inlines_text`:

```rust
/// Wraps `x` in `wrap`, unless doing so would push nesting past
/// `MAX_INLINE_DEPTH` — in which case the content is contributed as flat text.
///
/// `depth` is the inline-frame depth *after* the frame being closed was popped,
/// so it is the depth of the frame that will receive the result.
fn wrap_inline(depth: usize, wrap: fn(Vec<Inline>) -> Inline, x: Vec<Inline>) -> Inline {
    if depth > MAX_INLINE_DEPTH {
        Inline::Text(inlines_text(&x))
    } else {
        wrap(x)
    }
}
```

- [ ] **Step 5: Apply the guard at the closing-tag sites**

In the `Event::End` match, replace the `strong`/`b` and `em`/`i` arms (lines 843-852):

```rust
                    b"strong" | b"b" => {
                        let x = inline_stack.pop().unwrap_or_default();
                        let depth = inline_stack.len();
                        if let Some(top) = inline_stack.last_mut() {
                            top.push(wrap_inline(depth, Inline::Strong, x));
                        }
                    }
                    b"em" | b"i" => {
                        let x = inline_stack.pop().unwrap_or_default();
                        let depth = inline_stack.len();
                        if let Some(top) = inline_stack.last_mut() {
                            top.push(wrap_inline(depth, Inline::Emph, x));
                        }
                    }
```

`depth` is read after the pop and before `last_mut()` borrows the stack — computing it inline would not borrow-check. The `a` and `code` arms need no change: `code` already flattens to `Inline::Code(inlines_text(&x))`, and nested `<a>` is illegal XHTML that cannot recur deeply.

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p kasane-adapters deep_inline_nesting_is_flattened -- --nocapture`

Expected: PASS.

- [ ] **Step 7: Run the full suite and lint**

Run: `mise run lint && mise run test`

Expected: all green. The existing `<em><strong>` nesting tests near lines 1582-1690 assert depth-2 and depth-3 shapes, which are far under 64 and must be unaffected — if any of them fail, the guard is firing too early and `depth > MAX_INLINE_DEPTH` has been written as `>=` by mistake.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-adapters/src/epub/xhtml.rs
git commit -m "fix(epub): flatten inline nesting past a documented depth bound"
```

---

### Task 3: Assert bounded inline depth in the fuzz seam and commit the seed

Turns the §2.2 shape into a permanent regression test on stable, following the repo's rule that a bug's reproducer is committed.

**Files:**
- Modify: `crates/kasane-adapters/src/fuzz_entry.rs:50-54`
- Create: `tests/fixtures/epub/make_deep_nesting_epub.py`
- Create: `fuzz/seeds/epub/deep-nesting.epub`

**Interfaces:**
- Consumes: `kasane_ir::MAX_INLINE_DEPTH` (Task 1), `epub::xhtml::MAX_INLINE_DEPTH` (Task 2).

- [ ] **Step 1: Write the seed generator**

Create `tests/fixtures/epub/make_deep_nesting_epub.py`:

```python
#!/usr/bin/env python3
"""Generate fuzz/seeds/epub/deep-nesting.epub.

A minimal EPUB whose single chapter nests <em> 5000 deep. Before the depth
bounds landed (design spec 2026-07-29 SS2.2), converting this aborted the
process with a stack overflow in the core's and writer's recursive inline
walks. It is a seed rather than an artifact because the bug was found by
designing the property tier, not by libFuzzer.
"""
import pathlib
import zipfile

DEPTH = 5000
ROOT = pathlib.Path(__file__).resolve().parents[3]
OUT = ROOT / "fuzz" / "seeds" / "epub" / "deep-nesting.epub"

CONTAINER = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf"
    media-type="application/oebps-package+xml"/></rootfiles>
</container>
"""

OPF = """<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Deep Nesting</dc:title><dc:identifier id="id">deep</dc:identifier>
    <dc:language>en</dc:language>
  </metadata>
  <manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="c1"/></spine>
</package>
"""

CHAPTER = (
    '<?xml version="1.0" encoding="utf-8"?>\n'
    '<html xmlns="http://www.w3.org/1999/xhtml"><body>'
    "<h1>Deep</h1><p>" + "<em>" * DEPTH + "bottom" + "</em>" * DEPTH + "</p>"
    "</body></html>"
)

OUT.parent.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(OUT, "w", zipfile.ZIP_DEFLATED) as z:
    # mimetype must be first and stored, per EPUB OCF.
    z.writestr(zipfile.ZipInfo("mimetype"), "application/epub+zip",
               compress_type=zipfile.ZIP_STORED)
    z.writestr("META-INF/container.xml", CONTAINER)
    z.writestr("OEBPS/content.opf", OPF)
    z.writestr("OEBPS/c1.xhtml", CHAPTER)

print(f"wrote {OUT} ({OUT.stat().st_size} bytes)")
```

- [ ] **Step 2: Generate the seed**

Run: `python3 tests/fixtures/epub/make_deep_nesting_epub.py`

Expected: `wrote /workspace/fuzz/seeds/epub/deep-nesting.epub (N bytes)` — a few KB, since 5000 repeated `<em>` tags compress well.

- [ ] **Step 3: Write the failing assertion**

In `crates/kasane-adapters/src/fuzz_entry.rs`, replace the shared `adapter()` helper (lines 48-54):

```rust
/// A rejected parse is a perfectly good outcome — most fuzzer inputs are not
/// valid documents. Only a *successful* parse has assets worth checking.
fn adapter(a: &dyn Adapter, data: &[u8], source_path: &str) {
    if let Ok((doc, assets)) = a.parse(data, source_path) {
        assert_assets_contained(&assets);
        assert_inline_depth_bounded(&doc);
    }
}

/// Design spec `2026-07-29-core-property-tier-design.md` §2.2: `kasane-core` and
/// `kasane-writer` walk inlines recursively, so IR nested past
/// `kasane_ir::MAX_INLINE_DEPTH` aborts the process on a stack overflow rather
/// than failing recoverably. No adapter may produce it. Asserted against the
/// core's safety bound rather than any one adapter's flattening bound, because
/// the core's is the value that decides whether the process survives.
fn assert_inline_depth_bounded(doc: &Document) {
    fn inline_depth(inls: &[Inline]) -> usize {
        inls.iter()
            .map(|i| match i {
                Inline::Emph(x) | Inline::Strong(x) => 1 + inline_depth(x),
                Inline::Link { inlines, .. } => 1 + inline_depth(inlines),
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }

    fn block_depth(b: &Block) -> usize {
        match b {
            Block::Heading { inlines, .. } | Block::Para(inlines) => inline_depth(inlines),
            Block::Figure { caption, .. } => inline_depth(caption),
            Block::List { items, .. } => items
                .iter()
                .flatten()
                .map(block_depth)
                .max()
                .unwrap_or(0),
            Block::Footnote { blocks, .. } => blocks.iter().map(block_depth).max().unwrap_or(0),
            Block::Table(t) => t
                .header
                .iter()
                .chain(t.rows.iter().flatten())
                .map(|c| inline_depth(c))
                .max()
                .unwrap_or(0),
            _ => 0,
        }
    }

    for node in &doc.nodes {
        let d = block_depth(&node.block);
        assert!(
            d <= kasane_ir::MAX_INLINE_DEPTH,
            "inline nesting depth {} exceeds MAX_INLINE_DEPTH {}",
            d,
            kasane_ir::MAX_INLINE_DEPTH
        );
    }
}
```

Add whatever of `Document`, `Block`, `Inline` is missing to this file's existing `use` statements.

**Note:** `inline_depth` and `block_depth` are themselves recursive over the same nesting. That is safe here only because the assertion runs after the adapter's own bound has flattened the input; if this assertion ever aborts instead of failing, the adapter guard from Task 2 is not doing its job, which is exactly the signal wanted.

- [ ] **Step 4: Run the corpus replay to verify the seed is picked up**

Run: `cargo test -p kasane-adapters --test fuzz_corpus -- --nocapture`

Expected: PASS, and the printed target/file counts include the new `epub` seed directory. If the run reports an unrecognized corpus directory, `target()` in `fuzz_corpus.rs` is missing an `"epub"` entry — it already has one (line ~25), so this should not happen.

- [ ] **Step 5: Verify the seed actually exercised the bug**

Temporarily revert Task 2's guard by changing `epub::xhtml::MAX_INLINE_DEPTH` to `usize::MAX`, then run:

Run: `cargo test -p kasane-adapters --test fuzz_corpus -- --nocapture`

Expected: the assertion **fails** (or the process aborts), proving the seed reaches the shape. Restore `64` afterwards and re-run to confirm PASS. A seed that cannot fail is not a regression test.

- [ ] **Step 6: Add the end-to-end conversion test**

Spec §6.2 asks for the whole-pipeline case, not just the parser unit test and the fuzz replay: the seed must convert through the real CLI path. Append to `crates/kasane-cli/tests/e2e.rs`:

```rust
#[test]
fn converts_a_deeply_nested_epub_without_aborting() {
    // fuzz/seeds/epub/deep-nesting.epub nests <em> 5000 deep. Before the two
    // depth bounds landed (design spec 2026-07-29 §2.2) this aborted the
    // process on a stack overflow in the core's and writer's inline walks --
    // which, in batch mode, would have taken every other worker down with it.
    let out = tempfile::tempdir().unwrap();
    let out_dir = out.path().join("deep");
    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg("../../fuzz/seeds/epub/deep-nesting.epub")
        .arg("-o")
        .arg(&out_dir)
        .status()
        .unwrap();
    assert!(status.success(), "conversion failed: {:?}", status);
    let idx = std::fs::read_to_string(out_dir.join("index.md")).unwrap();
    assert!(idx.contains("title: Deep Nesting"));
}
```

The relative source path and the `Command::new(env!("CARGO_BIN_EXE_kasane"))` shape match the existing tests in this file (`e2e.rs:7-12`); `use std::process::Command;` is already at the top.

- [ ] **Step 7: Run the full suite and lint**

Run: `mise run lint && mise run test`

Expected: all green, including the new e2e test.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-adapters/src/fuzz_entry.rs \
        crates/kasane-cli/tests/e2e.rs \
        tests/fixtures/epub/make_deep_nesting_epub.py \
        fuzz/seeds/epub/deep-nesting.epub
git commit -m "test(fuzz): assert bounded inline depth and seed the deep-nesting EPUB"
```

---

### Task 4: Emit the file title as a heading, and give merged subsections live anchors

Closes spec §2.1. Today every in-book cross-reference resolves to an anchor no rendered file contains, and section files open with no visible title.

**Files:**
- Modify: `crates/kasane-writer/src/lib.rs:64-87`
- Modify: `crates/kasane-writer/src/markdown.rs` (no change to `blocks_to_markdown` itself)
- Modify: `crates/kasane-core/src/balance.rs:17-33`
- Modify: `crates/kasane-core/src/paths.rs:23-51`
- Test: `crates/kasane-writer/src/lib.rs` (existing `mod tests`), `crates/kasane-core/src/paths.rs` (existing `mod tests`)

**Interfaces:**
- Produces: `kasane_writer::file_to_markdown(file: &FileNode, assets: &AssetBag) -> String` — prepends the title heading, then delegates to `blocks_to_markdown`. Consumed by `write_tree_contents` and by Task 6's property suite.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/kasane-writer/src/lib.rs`:

```rust
#[test]
fn file_to_markdown_opens_with_the_title_heading() {
    use crate::file_to_markdown;
    let file = FileNode {
        path: "01-intro/02-background.md".into(),
        frontmatter: Frontmatter {
            title: "Background".into(),
            breadcrumb: vec!["Book".into(), "Intro".into(), "Background".into()],
            parent: Some("index.md".into()),
            prev: None,
            next: None,
            children: vec![],
            source_pages: None,
        },
        blocks: vec![Block::Para(vec![Inline::Text("body".into())])],
    };
    let md = file_to_markdown(&file, &AssetBag::default());
    // breadcrumb depth 3 -> "###"
    assert!(md.starts_with("### Background\n"), "got: {:?}", md);
    assert!(md.contains("body"));
}

#[test]
fn title_heading_level_is_clamped_to_six() {
    use crate::file_to_markdown;
    let file = FileNode {
        path: "a/b/c/d/e/f/g.md".into(),
        frontmatter: Frontmatter {
            title: "Deep".into(),
            breadcrumb: (0..9).map(|i| format!("L{}", i)).collect(),
            parent: None,
            prev: None,
            next: None,
            children: vec![],
            source_pages: None,
        },
        blocks: vec![],
    };
    let md = file_to_markdown(&file, &AssetBag::default());
    assert!(md.starts_with("###### Deep\n"), "got: {:?}", md);
}
```

Append to `mod tests` in `crates/kasane-core/src/paths.rs`:

```rust
#[test]
fn body_headings_get_anchors_too() {
    // A merged subsection's heading lives in its parent's body after balance.
    // It must still be reachable by a cross-reference.
    let tree = SectionTree {
        root: SectionNode {
            id: None,
            level: 0,
            title: vec![],
            body: vec![Block::Heading {
                level: 2,
                id: BlockId(9),
                inlines: vec![Inline::Text("Merged Bit".into())],
            }],
            children: vec![],
            pages: None,
        },
    };
    let placed = assign_paths(tree);
    assert_eq!(placed.anchors[&BlockId(9)], "index.md#merged-bit");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-writer file_to_markdown title_heading_level && cargo test -p kasane-core body_headings_get_anchors`

Expected: FAIL — `cannot find function file_to_markdown` for the writer tests, and a panic on a missing `BlockId(9)` key for the core test.

- [ ] **Step 3: Add `file_to_markdown`**

In `crates/kasane-writer/src/lib.rs`, add below the `pub use` block:

```rust
pub use markdown::blocks_to_markdown;

use kasane_core::FileNode;

/// Renders one file's body: its title as a heading, then its blocks.
///
/// The title heading is what makes cross-references resolvable. `fold_sections`
/// consumes a section's heading into `SectionNode.title` and never re-emits it,
/// while `assign_paths` records the section's anchor as `path#slug(title)` — so
/// without a heading rendered here, every internal link points at an anchor no
/// file contains (design spec `2026-07-29-core-property-tier-design.md` §2.1).
///
/// It lives here rather than in `blocks_to_markdown` because that function takes
/// `&[Block]` and never sees a `Frontmatter`, and rather than in
/// `write_tree_contents` because the property suite renders without touching the
/// filesystem and must see byte-identical output.
pub fn file_to_markdown(file: &FileNode, assets: &AssetBag) -> String {
    let level = file.frontmatter.breadcrumb.len().clamp(1, 6);
    let mut out = String::new();
    for _ in 0..level {
        out.push('#');
    }
    out.push(' ');
    out.push_str(&file.frontmatter.title);
    out.push('\n');
    out.push('\n');
    out.push_str(&blocks_to_markdown(&file.blocks, assets));
    out
}
```

Adjust the existing `use kasane_core::SiteTree;` to `use kasane_core::{FileNode, SiteTree};` and drop the duplicate `use` line if one results.

- [ ] **Step 4: Route `write_tree_contents` through it**

In `crates/kasane-writer/src/lib.rs`, inside `write_tree_contents`, replace:

```rust
        let body = blocks_to_markdown(&file.blocks, assets);
```

with:

```rust
        let body = file_to_markdown(file, assets);
```

- [ ] **Step 5: Record anchors for body headings**

In `crates/kasane-core/src/paths.rs`, inside `place()`, immediately after the existing `if let Some(id) = node.id { ... }` block:

```rust
    // A merged subsection's heading lives in its parent's body (balance.rs
    // demotes it there), and nothing else would give it an anchor. Only
    // top-level body blocks are scanned: a heading nested inside a list item was
    // never folded into a section either, and giving it an anchor would invent
    // structure the engine does not model.
    for b in &node.body {
        if let kasane_ir::Block::Heading { id, inlines, .. } = b {
            anchors.insert(*id, format!("{}#{}", self_path, slug(inlines)));
        }
    }
```

Add `Block` to this file's `use kasane_ir::{...}` line if preferred over the fully-qualified path.

- [ ] **Step 6: Demote merged children to real headings**

In `crates/kasane-core/src/balance.rs`, replace the merge body (lines 23-28):

```rust
        if small {
            // Demote the heading into the parent's body. A real `Block::Heading`
            // carrying the original `BlockId` is what lets `assign_paths` give
            // it an anchor, so a cross-reference into a merged subsection
            // resolves instead of degrading to plain text. A synthetic split
            // part has `id: None` and nothing can link to it, so it keeps the
            // bold lead-in.
            if !child.title.is_empty() {
                match child.id {
                    Some(id) => node.body.push(Block::Heading {
                        level: child.level,
                        id,
                        inlines: child.title.clone(),
                    }),
                    None => node
                        .body
                        .push(Block::Para(vec![Inline::Strong(child.title.clone())])),
                }
            }
            node.body.extend(child.body);
        } else {
```

- [ ] **Step 7: Run the new tests to verify they pass**

Run: `cargo test -p kasane-writer file_to_markdown title_heading_level && cargo test -p kasane-core body_headings_get_anchors`

Expected: PASS, 3 tests.

- [ ] **Step 8: Run the full suite**

Run: `mise run test`

Expected: green. The change is additive to rendered output (a heading line is prepended), and every existing end-to-end assertion is a `contains(...)` on text that survives, so no e2e test should need editing. **If one fails, do not weaken it** — read what it asserts and confirm the new output is correct before adjusting; a broken assertion here is the most likely place a real regression shows up.

- [ ] **Step 9: Verify against a real conversion**

Run:

```bash
cargo run -q -p kasane-cli -- tests/fixtures/epub/rich.epub -o /tmp/rich-check
head -12 /tmp/rich-check/01-chapter-one.md
```

Expected: the frontmatter block, then a blank line, then `## Chapter One` — the heading that was missing in spec §2.1's transcript.

- [ ] **Step 10: Lint and commit**

```bash
mise run lint
git add crates/kasane-writer/src/lib.rs crates/kasane-core/src/paths.rs crates/kasane-core/src/balance.rs
git commit -m "fix(writer): emit the file title as a heading so cross-references resolve"
```

---

### Task 5: The document generator

Builds the adapter-realistic `proptest` strategies and the sentinel scheme from spec §4.

**Files:**
- Modify: `crates/kasane-writer/Cargo.toml`
- Create: `crates/kasane-writer/tests/generator/mod.rs`
- Create: `crates/kasane-writer/tests/generator_smoke.rs`

**Interfaces:**
- Produces, all consumed by Task 6:
  - `generator::Case { doc: Document, opts: Options, assets: AssetBag, sentinels: Vec<Sentinel> }`
  - `generator::Sentinel { token: String, expect: Expect }`
  - `generator::Expect { Exactly(usize), AtLeast(usize) }`
  - `generator::case() -> impl Strategy<Value = Case>`

- [ ] **Step 1: Add the dev-dependency**

In `crates/kasane-writer/Cargo.toml`, under `[dev-dependencies]`:

```toml
[dev-dependencies]
tempfile = "3"
proptest = "1.11.0"
```

- [ ] **Step 2: Write the failing smoke test**

Create `crates/kasane-writer/tests/generator_smoke.rs`:

```rust
//! Guards the generator itself. A property suite is only as good as what it
//! generates, and a generator that silently stops producing headings (or
//! duplicate sentinels) would leave every property passing vacuously.

mod generator;

use proptest::prelude::*;

proptest! {
    #[test]
    fn sentinels_are_unique(case in generator::case()) {
        let mut seen = std::collections::HashSet::new();
        for s in &case.sentinels {
            prop_assert!(seen.insert(s.token.clone()), "duplicate sentinel {}", s.token);
        }
    }

    #[test]
    fn every_block_carries_a_sentinel(case in generator::case()) {
        prop_assert_eq!(case.sentinels.len(), case.doc.nodes.len());
    }

    #[test]
    fn options_are_well_ordered(case in generator::case()) {
        prop_assert!(case.opts.min_tokens < case.opts.max_tokens);
    }
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p kasane-writer --test generator_smoke`

Expected: FAIL — `file not found for module 'generator'`.

- [ ] **Step 4: Write the generator**

Create `crates/kasane-writer/tests/generator/mod.rs`:

```rust
//! Adapter-realistic `Document` strategies for the property suite.
//!
//! Design spec `2026-07-29-core-property-tier-design.md` §4. Two ideas carry
//! the whole design:
//!
//! **Sentinels.** Every generated block carries a unique token, so conservation
//! can be checked by counting occurrences in the rendered Markdown rather than
//! by structural comparison. That stays true no matter how `balance()` rewrites
//! a block, and the property never has to encode what the engine synthesizes.
//! Strategies compose without shared state, so uniqueness cannot come from a
//! counter threaded through generation: the strategy builds a skeleton with
//! placeholder text, and one deterministic `prop_map` stamps sequential tokens
//! over the finished skeleton.
//!
//! **Adapter realism.** Heading levels 1..=6, nesting capped at 3, block mix
//! weighted toward paragraphs. A failure is then unambiguously reachable from a
//! real document, with no triage step asking "can any adapter produce this?".

#![allow(dead_code)] // each property uses a different subset of this module

use kasane_core::Options;
use kasane_ir::*;
use proptest::prelude::*;

/// How many times a sentinel must appear across the rendered files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expect {
    /// Non-heading blocks: exactly one render site.
    Exactly(usize),
    /// Headings legitimately recur — the file's own title heading, the parent's
    /// TOC link, a merge lead-in.
    AtLeast(usize),
}

#[derive(Clone, Debug)]
pub struct Sentinel {
    pub token: String,
    pub expect: Expect,
}

#[derive(Clone, Debug)]
pub struct Case {
    pub doc: Document,
    pub opts: Options,
    pub assets: AssetBag,
    pub sentinels: Vec<Sentinel>,
}

/// Words the generator draws filler text from. Deliberately free of the `zq`
/// sentinel prefix, so generated content can never collide with a token.
const WORDS: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "epsilon", "the", "and", "of", "a", "chapter", "section",
];

fn filler() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::sample::select(WORDS), 1..12)
        .prop_map(|ws| ws.join(" "))
}

/// One inline run, nested at most `depth` levels. Boxed at every level because
/// the strategy is recursive and its type would otherwise be infinite. The leaf
/// is built twice rather than cloned, so this compiles without requiring the
/// mapped strategy to be `Clone`.
fn inlines(depth: u32) -> BoxedStrategy<Vec<Inline>> {
    if depth == 0 {
        return filler().prop_map(|s| vec![Inline::Text(s)]).boxed();
    }
    prop_oneof![
        8 => filler().prop_map(|s| vec![Inline::Text(s)]),
        1 => inlines(depth - 1).prop_map(|x| vec![Inline::Emph(x)]),
        1 => inlines(depth - 1).prop_map(|x| vec![Inline::Strong(x)]),
    ]
    .boxed()
}

/// The block shapes an adapter really produces. `SHAPE_*` picks the variant; the
/// sentinel is stamped in afterwards.
#[derive(Clone, Debug)]
enum Shape {
    Heading(u8),
    Para,
    List(bool),
    Table,
    Figure(bool),
    Code,
    Math,
    Raw,
    Footnote,
}

fn shape() -> impl Strategy<Value = Shape> {
    prop_oneof![
        3 => (1u8..=6).prop_map(Shape::Heading),
        8 => Just(Shape::Para),
        2 => any::<bool>().prop_map(Shape::List),
        1 => Just(Shape::Table),
        1 => any::<bool>().prop_map(Shape::Figure),
        1 => Just(Shape::Code),
        1 => Just(Shape::Math),
        1 => Just(Shape::Raw),
        1 => Just(Shape::Footnote),
    ]
}

/// Builds one block from a shape, stamping `token` into the single position
/// that renders, and reporting how many times it is expected to appear.
///
/// `deco` is generated nested inline markup (depth <= 3) appended after the
/// sentinel, so the engine's and the writer's inline walks are exercised on real
/// nesting rather than only on flat text. It is appended, never wrapped around
/// the token, so the token itself always renders as a bare run and the
/// occurrence count stays exact.
fn build(shape: &Shape, deco: &[Inline], token: &str, idx: u32) -> (Block, Expect) {
    let text = |t: &str| {
        let mut v = vec![Inline::Text(t.to_string())];
        v.extend(deco.iter().cloned());
        v
    };
    match shape {
        Shape::Heading(level) => (
            Block::Heading {
                level: *level,
                id: BlockId(idx),
                inlines: text(token),
            },
            Expect::AtLeast(1),
        ),
        Shape::Para => (Block::Para(text(token)), Expect::Exactly(1)),
        Shape::List(ordered) => (
            Block::List {
                ordered: *ordered,
                items: vec![vec![Block::Para(text(token))]],
            },
            Expect::Exactly(1),
        ),
        Shape::Table => (
            Block::Table(Table {
                header: vec![text("col")],
                rows: vec![vec![text(token)]],
                has_merged: false,
            }),
            Expect::Exactly(1),
        ),
        // markdown.rs:54-61 renders a numbered figure's caption twice: once as
        // alt text, once in the visible `*Figure N: ...*` line. Deliberate
        // (alt text plus a caption), so the expectation says two, not one.
        Shape::Figure(numbered) => (
            Block::Figure {
                image: AssetRef {
                    key: format!("img{}", idx),
                    bytes_ref: 0,
                },
                caption: text(token),
                number: numbered.then(|| "1".to_string()),
            },
            Expect::Exactly(if *numbered { 2 } else { 1 }),
        ),
        Shape::Code => (
            Block::CodeBlock {
                lang: Some("rust".into()),
                text: token.to_string(),
            },
            Expect::Exactly(1),
        ),
        Shape::Math => (Block::MathBlock(token.to_string()), Expect::Exactly(1)),
        Shape::Raw => (
            Block::Raw {
                note: token.to_string(),
            },
            Expect::Exactly(1),
        ),
        Shape::Footnote => (
            Block::Footnote {
                id: NoteId(idx),
                blocks: vec![Block::Para(text(token))],
            },
            Expect::Exactly(1),
        ),
    }
}

/// A generated case: document, options, assets, and the sentinel ledger.
pub fn case() -> impl Strategy<Value = Case> {
    // Each entry pairs a block shape with generated nested inline markup, so
    // nesting depth up to 3 is present throughout rather than only in flat runs.
    let shapes = proptest::collection::vec((shape(), inlines(3)), 1..40);
    let opts = (40usize..400, 5usize..40).prop_map(|(max_tokens, min_tokens)| Options {
        max_tokens,
        // min < max by construction, so the engine is never asked to satisfy
        // contradictory thresholds.
        min_tokens: min_tokens.min(max_tokens.saturating_sub(1)),
    });

    (shapes, opts).prop_map(|(shapes, opts)| {
        let mut nodes = Vec::new();
        let mut sentinels = Vec::new();
        let mut assets = AssetBag::default();

        for (i, (sh, deco)) in shapes.iter().enumerate() {
            let idx = i as u32;
            let token = format!("zq{:04}", idx);
            let (block, expect) = build(sh, deco, &token, idx);

            // A figure needs a matching asset or the renderer emits "missing".
            if let Shape::Figure(_) = sh {
                assets.items.push(AssetItem {
                    key: format!("img{}", idx),
                    filename: format!("img{}.png", idx),
                    bytes: vec![0x89, b'P', b'N', b'G'],
                });
            }

            nodes.push(Node {
                block,
                prov: Provenance::default(),
            });
            sentinels.push(Sentinel { token, expect });
        }

        Case {
            doc: Document {
                meta: DocMeta {
                    title: "Generated Book".into(),
                    authors: vec![],
                    language: None,
                    source_format: "epub".into(),
                    source_path: "generated.epub".into(),
                },
                nodes,
            },
            opts,
            assets,
            sentinels,
        }
    })
}

/// A case whose paragraphs additionally carry internal cross-references — some
/// pointing at real generated headings, some dangling, so both the resolve path
/// and the strip path (`refs.rs:63-68`) are exercised.
pub fn case_with_links() -> impl Strategy<Value = Case> {
    (case(), any::<bool>()).prop_map(|(mut c, dangle)| {
        let heading_ids: Vec<BlockId> = c
            .doc
            .nodes
            .iter()
            .filter_map(|n| match &n.block {
                Block::Heading { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        let target = match (heading_ids.first(), dangle) {
            (Some(id), false) => *id,
            // No heading generated, or deliberately dangling: an id far past
            // anything the generator assigns.
            _ => BlockId(9_999),
        };
        for n in c.doc.nodes.iter_mut() {
            if let Block::Para(inls) = &mut n.block {
                inls.push(Inline::Link {
                    target: RefTarget::Internal(target),
                    inlines: vec![Inline::Text("see".into())],
                });
                break;
            }
        }
        c
    })
}
```

- [ ] **Step 5: Run the smoke test to verify it passes**

Run: `cargo test -p kasane-writer --test generator_smoke`

Expected: PASS, 3 tests, each running 256 cases.

- [ ] **Step 6: Lint**

Run: `mise run lint`

Expected: green. `--all-targets` lints this file; the `#![allow(dead_code)]` at the top is what keeps `case_with_links` from tripping `-D warnings` before Task 6 consumes it.

- [ ] **Step 7: Commit**

```bash
git add crates/kasane-writer/Cargo.toml Cargo.lock \
        crates/kasane-writer/tests/generator/mod.rs \
        crates/kasane-writer/tests/generator_smoke.rs
git commit -m "test(core): add the adapter-realistic Document generator"
```

---

### Task 6: The six properties

Spec §5. This is the deliverable the item exists for.

**Files:**
- Modify: `crates/kasane-core/src/balance.rs` (expose `est_tokens`)
- Modify: `crates/kasane-core/src/paths.rs` (expose `slug_of`)
- Modify: `crates/kasane-core/src/lib.rs` (re-export both)
- Create: `crates/kasane-writer/tests/properties.rs`

**Interfaces:**
- Consumes: `generator::{case, case_with_links, Case, Expect}` (Task 5); `kasane_writer::file_to_markdown` (Task 4).
- Produces: `kasane_core::est_tokens(blocks: &[Block]) -> usize` and `kasane_core::slug_of(inlines: &[Inline]) -> String`, both `#[doc(hidden)] pub`.

- [ ] **Step 1: Expose the two core seams**

In `crates/kasane-core/src/balance.rs`, above the existing `pub(crate) fn est_tokens_blocks`:

```rust
/// Token estimate for a block slice.
///
/// `#[doc(hidden)]` because it is a test seam, not API — the same convention
/// `kasane-adapters` uses for `fuzz_entry`. The property suite's size-guard
/// invariant needs the engine's own estimator; re-implementing it in the test
/// would create a second source of truth that drifts silently, passing against
/// its own arithmetic while the engine's changed.
#[doc(hidden)]
pub fn est_tokens(blocks: &[Block]) -> usize {
    est_tokens_blocks(blocks)
}
```

In `crates/kasane-core/src/paths.rs`, above the existing `pub(crate) fn slug`:

```rust
/// Slug for a heading's inlines, as `assign_paths` computes it.
///
/// `#[doc(hidden)]` test seam, same rationale as `est_tokens`: the link
/// invariant has to compare a rendered heading against the anchor the engine
/// emitted, using the engine's own slug rule rather than a copy of it.
#[doc(hidden)]
pub fn slug_of(inlines: &[Inline]) -> String {
    slug(inlines)
}
```

In `crates/kasane-core/src/lib.rs`, extend the re-exports:

```rust
pub use balance::{balance, est_tokens};
pub use paths::{assign_paths, slug_of, PlaceResult, Placed};
```

- [ ] **Step 2: Write the failing properties**

Create `crates/kasane-writer/tests/properties.rs`:

```rust
//! Design spec §9's property tier, over `kasane-core` + `kasane-writer`.
//!
//! Six invariants that must hold for *any* document, checked against the
//! rendered Markdown rather than against intermediate structures — because
//! §9's link invariant is about a real file and a real anchor, and only the
//! rendered text can answer that.
//!
//! A failure writes `proptest-regressions/properties.txt`. **Commit it.** That
//! file is what turns a bug the search found into a permanent regression test,
//! exactly as `fuzz/artifacts/` reproducers are.

mod generator;

use generator::{Case, Expect};
use kasane_core::{est_tokens, slug_of, structure, FileNode};
use kasane_ir::Block;
use proptest::prelude::*;
use std::collections::{HashMap, HashSet};

/// Runs the pipeline and returns each file with the text a real conversion
/// would write.
fn render(case: &Case) -> Vec<(String, String, FileNode)> {
    let site = structure(case.doc.clone(), &case.opts);
    site.files
        .into_iter()
        .map(|f| {
            let text = kasane_writer::file_to_markdown(&f, &case.assets);
            (f.path.clone(), text, f)
        })
        .collect()
}

/// Resolves a link relative to the file containing it, into a tree path.
/// Mirrors `refs::relativize` in reverse. Returns `None` if it escapes the root.
fn resolve_relative(from_file: &str, rel: &str) -> Option<String> {
    let rel = rel.split('#').next().unwrap_or(rel);
    let mut parts: Vec<&str> = from_file.split('/').collect();
    parts.pop(); // drop the filename
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Every `[text](target)` in a rendered file.
fn links_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == ']' && i + 1 < bytes.len() && bytes[i + 1] == '(' {
            let mut j = i + 2;
            let mut target = String::new();
            while j < bytes.len() && bytes[j] != ')' {
                target.push(bytes[j]);
                j += 1;
            }
            if j < bytes.len() {
                out.push(target);
            }
            i = j;
        }
        i += 1;
    }
    out
}

/// Every heading line's slug, as the engine would compute it.
///
/// A `#`-prefixed line inside a fenced code block would be counted too. That
/// only makes P2 more permissive, never less, so it is not worth a Markdown
/// parser here.
fn heading_slugs(text: &str) -> HashSet<String> {
    text.lines()
        .filter_map(|l| l.strip_prefix('#'))
        .map(|l| l.trim_start_matches('#').trim())
        .map(|t| slug_of(&[kasane_ir::Inline::Text(t.to_string())]))
        .collect()
}

proptest! {
    /// P1 — Conservation. No block lost, none duplicated.
    #[test]
    fn p1_conservation(case in generator::case()) {
        let files = render(&case);
        let all: String = files.iter().map(|(_, t, _)| t.as_str()).collect();
        for s in &case.sentinels {
            let n = all.matches(&s.token).count();
            match s.expect {
                Expect::Exactly(k) => prop_assert_eq!(
                    n, k, "sentinel {} appeared {} times, expected exactly {}", s.token, n, k
                ),
                Expect::AtLeast(k) => prop_assert!(
                    n >= k, "sentinel {} appeared {} times, expected at least {}", s.token, n, k
                ),
            }
        }
    }

    /// P2 — Link resolution, end to end.
    #[test]
    fn p2_links_resolve(case in generator::case_with_links()) {
        let files = render(&case);
        let by_path: HashMap<&str, &str> =
            files.iter().map(|(p, t, _)| (p.as_str(), t.as_str())).collect();

        // No symbolic ref survives into the emitted tree.
        for (_, _, f) in &files {
            for b in &f.blocks {
                prop_assert!(
                    !contains_internal_ref(b),
                    "an unresolved RefTarget::Internal reached the writer"
                );
            }
        }

        for (path, text, _) in &files {
            for target in links_in(text) {
                if target.starts_with("http") || target.starts_with("_assets/") {
                    continue;
                }
                let resolved = resolve_relative(path, &target);
                let resolved = match resolved {
                    Some(r) => r,
                    None => return Err(TestCaseError::fail(
                        format!("link {} from {} escapes the tree root", target, path)
                    )),
                };
                let body = by_path.get(resolved.as_str());
                prop_assert!(
                    body.is_some(),
                    "link {} from {} resolves to {}, which is not a file in the tree",
                    target, path, resolved
                );
                if let Some((_, anchor)) = target.split_once('#') {
                    prop_assert!(
                        heading_slugs(body.unwrap()).contains(anchor),
                        "anchor #{} from {} is not a heading in {}", anchor, path, resolved
                    );
                }
            }
        }
    }

    /// P3 — Size guard.
    #[test]
    fn p3_size_guard(case in generator::case()) {
        let files = render(&case);
        for (path, _, f) in &files {
            let weight = est_tokens(&f.blocks);
            let single_oversized_block = f.blocks.len() == 1
                && est_tokens(&f.blocks[..1]) > case.opts.max_tokens;
            // A container's TOC is inserted by nav *after* balancing sized the
            // node, so it can push a file over. Bounded by the TOC's own
            // weight, which is the first block when children exist.
            let toc_weight = if f.frontmatter.children.is_empty() {
                0
            } else {
                est_tokens(&f.blocks[..1])
            };
            prop_assert!(
                weight <= case.opts.max_tokens + toc_weight || single_oversized_block,
                "{} weighs {} against max_tokens {} (toc {})",
                path, weight, case.opts.max_tokens, toc_weight
            );
        }
    }

    /// P4 — Navigation chain.
    #[test]
    fn p4_nav_chain(case in generator::case()) {
        let files = render(&case);
        let by_path: HashMap<&str, &FileNode> =
            files.iter().map(|(p, _, f)| (p.as_str(), f)).collect();

        let mut cur = "index.md".to_string();
        let mut visited = Vec::new();
        loop {
            prop_assert!(!visited.contains(&cur), "next chain cycles at {}", cur);
            visited.push(cur.clone());
            let f = match by_path.get(cur.as_str()) {
                Some(f) => *f,
                None => return Err(TestCaseError::fail(format!("next led to missing {}", cur))),
            };
            match &f.frontmatter.next {
                None => break,
                Some(rel) => {
                    let nxt = resolve_relative(&cur, rel)
                        .ok_or_else(|| TestCaseError::fail("next escapes root".to_string()))?;
                    // prev of the next file must point back here.
                    let nf = by_path.get(nxt.as_str())
                        .ok_or_else(|| TestCaseError::fail(format!("missing {}", nxt)))?;
                    let back = nf.frontmatter.prev.as_ref()
                        .and_then(|p| resolve_relative(&nxt, p));
                    prop_assert_eq!(
                        back.as_deref(), Some(cur.as_str()),
                        "prev of {} does not point back to {}", nxt, cur
                    );
                    cur = nxt;
                }
            }
        }
        prop_assert_eq!(
            visited.len(), files.len(),
            "next chain visited {} of {} files", visited.len(), files.len()
        );
    }

    /// P5 — Path well-formedness.
    #[test]
    fn p5_paths_well_formed(case in generator::case()) {
        let files = render(&case);
        let paths: HashSet<&str> = files.iter().map(|(p, _, _)| p.as_str()).collect();
        prop_assert_eq!(paths.len(), files.len(), "duplicate file paths");

        for (path, _, f) in &files {
            prop_assert!(!path.starts_with('/'), "{} is absolute", path);
            for seg in path.split('/') {
                prop_assert!(seg != "..", "{} contains a traversal segment", path);
                prop_assert!(!seg.is_empty(), "{} contains an empty segment", path);
            }
            for child in &f.frontmatter.children {
                prop_assert!(
                    paths.contains(child.as_str()),
                    "{} lists child {} which is not a file", path, child
                );
            }
            if let Some(parent) = &f.frontmatter.parent {
                let resolved = resolve_relative(path, parent)
                    .ok_or_else(|| TestCaseError::fail("parent escapes root".to_string()))?;
                prop_assert!(
                    paths.contains(resolved.as_str()),
                    "{}'s parent {} resolves to {}, not a file", path, parent, resolved
                );
            }
        }
    }

    /// P6 — Determinism.
    #[test]
    fn p6_deterministic(case in generator::case()) {
        let a: Vec<_> = render(&case).into_iter().map(|(p, t, _)| (p, t)).collect();
        let b: Vec<_> = render(&case).into_iter().map(|(p, t, _)| (p, t)).collect();
        prop_assert_eq!(a, b, "structure + render is not deterministic");
    }
}

/// Whether any inline anywhere in this block is still a symbolic internal ref.
fn contains_internal_ref(b: &Block) -> bool {
    use kasane_ir::{Inline, RefTarget};

    fn in_inlines(is: &[Inline]) -> bool {
        is.iter().any(|i| match i {
            Inline::Link { target: RefTarget::Internal(_), .. } => true,
            Inline::Link { inlines, .. } | Inline::Emph(inlines) | Inline::Strong(inlines) => {
                in_inlines(inlines)
            }
            _ => false,
        })
    }

    match b {
        Block::Heading { inlines, .. } | Block::Para(inlines) => in_inlines(inlines),
        Block::Figure { caption, .. } => in_inlines(caption),
        Block::List { items, .. } => items.iter().flatten().any(contains_internal_ref),
        Block::Footnote { blocks, .. } => blocks.iter().any(contains_internal_ref),
        Block::Table(t) => {
            t.header.iter().any(|c| in_inlines(c))
                || t.rows.iter().flatten().any(|c| in_inlines(c))
        }
        _ => false,
    }
}
```

- [ ] **Step 3: Run the properties**

Run: `cargo test -p kasane-writer --test properties`

Expected: this is the moment the tier earns its keep. Three outcomes, each with a defined response:

- **All six pass.** Proceed to Step 5.
- **A property fails and the failure is a real defect.** `proptest-regressions/properties.txt` is written. Fix the engine, keep the property as written, and commit the regressions file with the fix. Per the standing rule, an adjacent defect is closed in-branch, not deferred.
- **A property fails because the property is wrong.** Only then adjust the property — and record why in a comment on it, so the next reader does not have to rediscover the reasoning.

Spec §5.2 names the two most likely first failures: a runt final part from `split_blocks` falling under `min_tokens`, and a container file inflated past `max_tokens` by its TOC. P3 above already grants the TOC escape; if the runt case fires, it is a defect in `split_blocks`, which should rebalance its last two parts rather than emit a runt.

- [ ] **Step 4: If anything failed, fix and re-run**

Run: `cargo test -p kasane-writer --test properties`

Expected: PASS, 6 properties × 256 cases.

- [ ] **Step 5: Run the full suite and lint**

Run: `mise run lint && mise run test`

Expected: all green, and the whole workspace suite still finishes in its usual time — the generator caps documents at 40 blocks and strings at ~12 words precisely so this stays cheap.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-core/src/balance.rs crates/kasane-core/src/paths.rs \
        crates/kasane-core/src/lib.rs crates/kasane-writer/tests/properties.rs
# plus, if the run produced one:
git add crates/kasane-writer/tests/proptest-regressions/properties.txt 2>/dev/null || true
git commit -m "test(core): add the structuring-engine property tier"
```

---

### Task 7: The heading-level overflow

Spec §6.1. `balance.rs:41` computes `node.level + 1` when splitting an oversized leaf; a level-255 heading overflow-panics in debug. Unreachable through today's adapters, reachable through the published `structure()`.

**Files:**
- Modify: `crates/kasane-core/src/balance.rs:41`
- Test: `crates/kasane-core/src/balance.rs` (existing `mod tests`)

- [ ] **Step 1: Write the failing test**

Append to `mod tests` in `crates/kasane-core/src/balance.rs`:

```rust
#[test]
fn splitting_a_max_level_heading_does_not_overflow() {
    // Every adapter clamps heading levels to 1..=6, but `Block::Heading.level`
    // is a `u8` and `structure()` is public, so a caller can hand us 255.
    // Splitting it computes level + 1.
    let mut tree = fold_sections(&doc(vec![
        h(255, 0, "Max"),
        big_para(1200),
        big_para(1200),
    ]));
    balance(
        &mut tree,
        &Options {
            max_tokens: 400,
            min_tokens: 10,
        },
    );
    let sec = &tree.root.children[0];
    assert!(sec.children.len() >= 2, "expected split into parts");
    assert_eq!(sec.children[0].level, 255, "level must saturate, not wrap");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p kasane-core splitting_a_max_level_heading`

Expected: FAIL — `attempt to add with overflow` panic at `balance.rs:41`.

- [ ] **Step 3: Saturate the increment**

In `crates/kasane-core/src/balance.rs`, in the `SectionNode` built by the split loop:

```rust
                // Saturating, not wrapping: adapters clamp levels to 1..=6, but
                // `structure()` is public and `level` is a `u8`, so a caller can
                // hand us 255 and a plain `+ 1` panics in debug.
                level: node.level.saturating_add(1),
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p kasane-core splitting_a_max_level_heading`

Expected: PASS.

- [ ] **Step 5: Run the full suite and lint**

Run: `mise run lint && mise run test`

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-core/src/balance.rs
git commit -m "fix(core): saturate the split heading level instead of overflowing"
```

---

### Task 8: Documentation

Spec §8. The discoverability surface is single-sourced, so it has to move with the code.

**Files:**
- Modify: `AGENTS.md`
- Modify: `README.md`

- [ ] **Step 1: Update the codebase map**

In `AGENTS.md`, extend the `crates/kasane-core` line to name the seams, and the `crates/kasane-writer` line to name the property tier. Append to the `kasane-core` entry:

```
  `est_tokens` and `slug_of` are `#[doc(hidden)] pub` test seams, not API — the
  same convention `kasane-adapters` uses for `fuzz_entry`, and for the same
  reason: the property tier needs the engine's own token estimate and slug rule,
  and a copy in the test would drift.
```

Append to the `kasane-writer` entry:

```
  `tests/properties.rs` is design spec §9's property tier: it generates
  adapter-realistic `Document`s (`tests/generator/`), runs `structure()`, renders
  each file with `file_to_markdown`, and asserts six invariants against the
  resulting Markdown — conservation, link resolution, the size guard, the
  prev/next chain, path well-formedness, determinism. It reaches the writer
  rather than stopping at `kasane-core` because §9's link invariant is about a
  real file *and a real anchor*, which only rendered text can answer.
  `file_to_markdown` is what both the property suite and `write_tree_contents`
  render through, so what CI asserts is what a conversion writes.
```

- [ ] **Step 2: Update the conventions section**

In `AGENTS.md`, under `## Conventions`, add beside the fuzz-reproducer rule:

```
- A failing property writes `crates/kasane-writer/tests/proptest-regressions/`.
  Commit it, for the same reason a fuzz reproducer is committed: it is what makes
  the found case a permanent regression test.
- Inline nesting is bounded twice, deliberately. `epub::xhtml::MAX_INLINE_DEPTH`
  (64) is a fidelity bound that flattens without losing content;
  `kasane_ir::MAX_INLINE_DEPTH` (256) is a safety bound in the core and writer's
  recursive walks, which adapter-produced IR can never reach. Unbounded, deep
  nesting aborts the process on a stack overflow.
```

- [ ] **Step 3: Update the README testing section**

In `README.md`, under `## Development`, after the `mise run lint` line:

```markdown
### Property tests

`kasane-core`'s structuring engine is checked with `proptest`: generated
documents run through `structure()` and the Markdown writer, and six invariants
are asserted against the rendered text — every block appears exactly once, every
internal link resolves to a real file and a real anchor, the size guard holds,
`prev`/`next` forms a complete chain, no path escapes the tree, and rendering is
deterministic. They run in `mise run test` with no extra setup.

When a property fails it writes `crates/kasane-writer/tests/proptest-regressions/`.
**Commit that file** — like a fuzz reproducer, it is what replays the failing
case on every subsequent run.
```

- [ ] **Step 4: Correct the known-limitations text**

In `README.md`, check the `## Known limitations (this build)` section for any claim that contradicts Task 4's output change (files now open with a title heading, and in-book cross-references now resolve to a live anchor). Update anything affected. If nothing there mentions cross-references or file headings, make no change rather than inventing an entry.

- [ ] **Step 5: Verify the docs match reality**

Run:

```bash
grep -n "MAX_INLINE_DEPTH" AGENTS.md crates/kasane-ir/src/lib.rs crates/kasane-adapters/src/epub/xhtml.rs
grep -n "proptest-regressions" AGENTS.md README.md
```

Expected: the constants named in `AGENTS.md` match the values in the source, and both docs name the regressions path consistently.

- [ ] **Step 6: Full verification**

Run: `mise run lint && mise run test`

Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add AGENTS.md README.md
git commit -m "docs: record the property tier, the core test seams, and the depth bounds"
```

---

## Final verification

- [ ] `mise run lint && mise run test` green from a clean checkout of the branch.
- [ ] `cargo run -q -p kasane-cli -- tests/fixtures/epub/rich.epub -o /tmp/final-check` produces section files that open with a title heading.
- [ ] `cargo test -p kasane-adapters --test fuzz_corpus -- --nocapture` replays the new `epub` seed.
- [ ] Every `proptest-regressions` file produced during implementation is committed.
