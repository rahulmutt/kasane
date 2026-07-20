# EPUB Fidelity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the EPUB adapter faithful — lists, tables, figures + asset extraction, EPUB3 semantic footnotes, internal-link resolution, code blocks — instead of silently dropping everything but headings/paragraphs/emphasis/links.

**Architecture:** Extend the streaming quick-xml event loop in `crates/kasane-adapters/src/epub/xhtml.rs` with an explicit **block-frame stack** (spec §2) next to the existing inline stack. Asset extraction and a two-phase link/footnote **fixup pass** live in `crates/kasane-adapters/src/epub/mod.rs`. The IR, core, and writer already support everything emitted — no changes outside `kasane-adapters` except one shared-helper move.

**Tech Stack:** Rust, quick-xml 0.41 (`GeneralRef` events, `allow_dangling_amp`), `zip` crate, existing `guard.rs`/`ziputil.rs` security plumbing.

**Spec:** `docs/superpowers/specs/2026-07-20-epub-fidelity-design.md`

## Global Constraints

- Every change ships green under `mise run lint && mise run test` (clippy runs `-D warnings`: no dead code, no unused params — add fields/variants only in the task that reads them).
- Adapters never trust input: every zip read goes through `crate::ziputil::read_entry_bytes/_string` with the shared `total_read` accumulator; entry names via `crate::guard::safe_entry_name` / `crate::guard::resolve_rel`. Never `zip.by_name(...)` directly.
- Do NOT touch the whitespace machinery (`pending_ws`, `prev_was_ref`, the `GeneralRef` suppress logic) or `allow_dangling_amp` handling — three bugfix commits live there. Frames change where blocks *land*, not how text is decoded.
- Pure Rust, no new dependencies.
- Degrade, don't die: malformed content becomes `Para`/`Raw` + `eprintln!` warning, never an aborted parse, never a broken link/image in output.
- Warnings go to stderr via `eprintln!("warning: ...")` (the CLI's existing stderr convention).

## Current interfaces (read before Task 1)

```rust
// epub/xhtml.rs (private mod inside epub)
pub fn xhtml_to_blocks(xml: &str, next_id: &mut u32) -> Vec<Block>  // next_id: running BlockId counter

// epub/mod.rs call site (inside the spine loop)
for b in xhtml::xhtml_to_blocks(&xml, &mut next_id) { nodes.push(Node { block: b, prov: ... }) }

// IR (kasane_ir) — already complete, do not modify:
Block::{Heading{level,id,inlines}, Para(Vec<Inline>), List{ordered, items: Vec<Vec<Block>>},
        Table(Table), Figure{image: AssetRef, caption: Vec<Inline>, number: Option<String>},
        CodeBlock{lang: Option<String>, text: String}, MathBlock(String),
        Footnote{id: NoteId, blocks: Vec<Block>}, Raw{note: String}}
Table { header: Vec<Vec<Inline>>, rows: Vec<Vec<Vec<Inline>>>, has_merged: bool }
AssetRef { key: String, bytes_ref: usize }          // key = zip-internal path; bytes_ref always 0
AssetBag { items: Vec<AssetItem> }; AssetItem { key, filename, bytes }
Inline::{Text, Emph, Strong, Code(String), Math, Link{target, inlines}, FootnoteRef(NoteId)}
RefTarget::{Internal(BlockId), External(String), Footnote(NoteId)}
BlockId(pub u32); NoteId(pub u32)

// guard.rs
pub fn safe_entry_name(name: &str) -> Option<String>
pub fn resolve_rel(base_dir: &str, target: &str) -> Option<String>  // normalizes ../, confines to root

// ziputil.rs
pub(crate) fn read_entry_bytes(zip: &mut ZipReader, name: &str, total_read: &mut u64) -> Result<Vec<u8>, ParseError>

// Writer behavior you rely on (do not modify):
// Figure -> ![caption](_assets/<filename looked up by AssetRef.key in AssetBag>)
// Footnote -> "[^{id.0}]: body"; FootnoteRef -> "[^{id.0}]"
// Table has_merged=true -> HTML <table> fallback; else GFM pipe table (header row required)
// Link{Internal} resolved by core to relative path; dangling Internal stripped to text by core
```

---

### Task 1: Block-frame stack + lists

**Files:**
- Modify: `crates/kasane-adapters/src/epub/xhtml.rs`

**Interfaces:**
- Consumes: existing `xhtml_to_blocks(xml, next_id) -> Vec<Block>` (signature unchanged in this task).
- Produces: private `enum BlockFrame { List { ordered: bool, items: Vec<Vec<Block>> } }`, `fn emit_block(frames: &mut Vec<BlockFrame>, out: &mut Vec<Block>, b: Block)`, `fn finish_frame(f: BlockFrame, frames: &mut Vec<BlockFrame>, out: &mut Vec<Block>)`. Tasks 2–8 extend both.

- [ ] **Step 1: Write the failing tests** (append to `mod tests` in `xhtml.rs`)

```rust
#[test]
fn parses_flat_unordered_list() {
    let mut id = 0;
    let blocks = xhtml_to_blocks(
        "<body><ul><li><p>one</p></li><li><p>two</p></li></ul></body>",
        &mut id,
    );
    assert_eq!(blocks.len(), 1);
    let Block::List { ordered, items } = &blocks[0] else {
        panic!("expected List, got {:?}", blocks[0]);
    };
    assert!(!ordered);
    assert_eq!(items.len(), 2);
    assert!(matches!(&items[0][0], Block::Para(i) if text_of(i) == "one"));
    assert!(matches!(&items[1][0], Block::Para(i) if text_of(i) == "two"));
}

#[test]
fn ordered_list_sets_ordered_flag() {
    let mut id = 0;
    let blocks = xhtml_to_blocks("<ol><li><p>a</p></li></ol>", &mut id);
    assert!(matches!(&blocks[0], Block::List { ordered: true, .. }));
}

#[test]
fn nested_list_folds_into_parent_item() {
    let mut id = 0;
    let blocks = xhtml_to_blocks(
        "<ul><li><p>A</p><ul><li><p>A1</p></li></ul></li><li><p>B</p></li></ul>",
        &mut id,
    );
    assert_eq!(blocks.len(), 1, "nested list must not become a sibling block");
    let Block::List { items, .. } = &blocks[0] else { panic!() };
    assert_eq!(items.len(), 2);
    // item A holds its Para plus the nested List
    assert!(matches!(&items[0][0], Block::Para(i) if text_of(i) == "A"));
    let Block::List { items: sub, .. } = &items[0][1] else {
        panic!("expected nested List inside item A, got {:?}", items[0])
    };
    assert!(matches!(&sub[0][0], Block::Para(i) if text_of(i) == "A1"));
}

#[test]
fn heading_inside_list_item_stays_in_item() {
    let mut id = 0;
    let blocks = xhtml_to_blocks("<ul><li><h3>t</h3></li></ul>", &mut id);
    let Block::List { items, .. } = &blocks[0] else { panic!() };
    assert!(matches!(&items[0][0], Block::Heading { level: 3, .. }));
}

#[test]
fn unclosed_list_at_eof_is_flushed_not_dropped() {
    let mut id = 0;
    let blocks = xhtml_to_blocks("<ul><li><p>orphan</p>", &mut id);
    let Block::List { items, .. } = &blocks[0] else {
        panic!("unclosed list must still be emitted")
    };
    assert!(matches!(&items[0][0], Block::Para(i) if text_of(i) == "orphan"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters epub::xhtml`
Expected: FAIL — `parses_flat_unordered_list` etc. panic with "expected List" (the current parser emits only the `Para`s, at top level).

- [ ] **Step 3: Implement the frame stack**

In `xhtml.rs`, above `xhtml_to_blocks`:

```rust
// Open block containers. Finished blocks land in the top frame instead of the
// output; closing the container folds the frame into its parent. This is what
// makes nesting (list items holding paragraphs, lists holding lists)
// representable in a single streaming pass.
enum BlockFrame {
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
}

fn emit_block(frames: &mut [BlockFrame], out: &mut Vec<Block>, b: Block) {
    match frames.last_mut() {
        None => out.push(b),
        Some(BlockFrame::List { items, .. }) => {
            // A block arriving before any <li> (malformed) opens an item
            // rather than being dropped.
            if items.is_empty() {
                items.push(Vec::new());
            }
            items.last_mut().expect("non-empty").push(b);
        }
    }
}

fn finish_frame(f: BlockFrame, frames: &mut [BlockFrame], out: &mut Vec<Block>) {
    match f {
        BlockFrame::List { ordered, items } => {
            if !items.is_empty() {
                emit_block(frames, out, Block::List { ordered, items });
            }
        }
    }
}
```

Inside `xhtml_to_blocks`, declare `let mut frames: Vec<BlockFrame> = vec![];` next to `inline_stack`. Then:

1. In the `Ok(Event::Start(e))` arm's element match, add:

```rust
b"ul" | b"ol" => {
    frames.push(BlockFrame::List {
        ordered: e.local_name().as_ref() == b"ol",
        items: vec![],
    });
}
b"li" => {
    if let Some(BlockFrame::List { items, .. }) = frames.last_mut() {
        items.push(Vec::new());
    }
}
```

2. In the `Ok(Event::End(e))` arm, add:

```rust
b"ul" | b"ol" => {
    if let Some(f @ BlockFrame::List { .. }) = frames.pop() {
        finish_frame(f, &mut frames, &mut blocks);
    }
}
```

(Rust note: `if let Some(f @ BlockFrame::List { .. })` needs the binding; with one variant `if let Some(f) = frames.pop()` suffices — use that until Task 3 adds variants, then match.)

3. Replace the two `blocks.push(...)` calls in the `h1..h6` and `p` End handlers with `emit_block(&mut frames, &mut blocks, ...)`.

4. Replace `Ok(Event::Eof) => break,` with a flush so unclosed containers fold instead of vanish (spec §5):

```rust
Ok(Event::Eof) => {
    while let Some(f) = frames.pop() {
        finish_frame(f, &mut frames, &mut blocks);
    }
    break;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters epub::xhtml`
Expected: PASS, including all pre-existing whitespace/entity tests (they must not change).

- [ ] **Step 5: Gate and commit**

```bash
mise run lint && mise run test
git add crates/kasane-adapters/src/epub/xhtml.rs
git commit -m "feat(epub): parse nested lists via a block-frame stack"
```

---

### Task 2: Implicit paragraphs, transparent containers, head/body gating

**Files:**
- Modify: `crates/kasane-adapters/src/epub/xhtml.rs`

**Interfaces:**
- Consumes: Task 1's `frames`/`emit_block`.
- Produces: `fn is_inline_tag(name: &[u8]) -> bool`; loop state `in_body: bool`, `implicit_para: bool`; a `close_implicit!()` macro. Tasks 3–5 call `close_implicit!()` at every new block-level Start handler.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn blockquote_bare_text_becomes_paragraph() {
    let mut id = 0;
    let blocks = xhtml_to_blocks(
        "<body><blockquote>quoted <em>words</em> here</blockquote></body>",
        &mut id,
    );
    assert_eq!(blocks.len(), 1);
    let Block::Para(inls) = &blocks[0] else { panic!("expected Para") };
    assert!(matches!(&inls[0], Inline::Text(t) if t == "quoted "));
    assert!(matches!(&inls[1], Inline::Emph(_)));
}

#[test]
fn dl_definition_text_is_flattened_not_dropped() {
    let mut id = 0;
    let blocks =
        xhtml_to_blocks("<body><dl><dt>term</dt><dd>meaning</dd></dl></body>", &mut id);
    assert_eq!(blocks.len(), 2);
    assert!(matches!(&blocks[0], Block::Para(i) if text_of(i) == "term"));
    assert!(matches!(&blocks[1], Block::Para(i) if text_of(i) == "meaning"));
}

#[test]
fn bare_li_text_becomes_item_paragraph() {
    let mut id = 0;
    let blocks = xhtml_to_blocks("<body><ul><li>one</li><li>two</li></ul></body>", &mut id);
    let Block::List { items, .. } = &blocks[0] else { panic!() };
    assert!(matches!(&items[0][0], Block::Para(i) if text_of(i) == "one"));
    assert!(matches!(&items[1][0], Block::Para(i) if text_of(i) == "two"));
}

#[test]
fn head_title_text_stays_out_of_output() {
    let mut id = 0;
    let blocks = xhtml_to_blocks(
        "<html><head><title>Skip Me</title></head><body><p>keep</p></body></html>",
        &mut id,
    );
    assert_eq!(blocks.len(), 1);
    assert!(matches!(&blocks[0], Block::Para(i) if text_of(i) == "keep"));
}

#[test]
fn implicit_paragraph_splits_at_block_boundary() {
    let mut id = 0;
    let blocks = xhtml_to_blocks(
        "<body><div>before<p>inside</p>after</div></body>",
        &mut id,
    );
    assert_eq!(blocks.len(), 3);
    assert!(matches!(&blocks[0], Block::Para(i) if text_of(i) == "before"));
    assert!(matches!(&blocks[1], Block::Para(i) if text_of(i) == "inside"));
    assert!(matches!(&blocks[2], Block::Para(i) if text_of(i) == "after"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters epub::xhtml`
Expected: FAIL — bare text outside `<p>` is currently dropped (`blocks.len()` is 0 or missing paras).

- [ ] **Step 3: Implement**

Above `xhtml_to_blocks`:

```rust
// Inline-level tags do NOT terminate an implicit paragraph; everything else
// (including unknown tags) is treated as a block boundary.
fn is_inline_tag(name: &[u8]) -> bool {
    matches!(
        name,
        b"strong" | b"b" | b"em" | b"i" | b"a" | b"code" | b"span" | b"sub" | b"sup"
            | b"small" | b"u" | b"s" | b"br"
    )
}
```

Inside `xhtml_to_blocks`, add state next to `cur_block`:

```rust
let mut in_body = false;
let mut implicit_para = false;
```

Add the macro right after the existing `push_text!` macro (it references locals, so it must be a macro, not a fn):

```rust
// Closes an open implicit paragraph (bare flow-level text) at any block
// boundary, emitting what it collected. See spec §2 "flatten, never drop".
macro_rules! close_implicit {
    () => {
        if implicit_para {
            implicit_para = false;
            let inls = inline_stack.pop().unwrap_or_default();
            if !inls.is_empty() {
                emit_block(&mut frames, &mut blocks, Block::Para(inls));
            }
        }
    };
}
```

Wire it in:

1. **Start arm**, immediately after the existing `pending_ws = None; prev_was_ref = false;`:

```rust
if !is_inline_tag(e.local_name().as_ref()) {
    close_implicit!();
}
if e.local_name().as_ref() == b"body" {
    in_body = true;
}
```

2. **End arm**, immediately after its `pending_ws = None; prev_was_ref = false;`:

```rust
if !is_inline_tag(e.local_name().as_ref()) {
    close_implicit!();
}
```

(This is safe for `</p>`/`</h1>`: an implicit para can only be open when *their* frames aren't, because their Start handlers just closed it.)

3. **Text arm** — in the non-whitespace branch, replace

```rust
pending_ws = None;
if !inline_stack.is_empty() {
    push_text!(s);
}
```

with

```rust
pending_ws = None;
if inline_stack.is_empty() && in_body && cur_block.is_none() {
    inline_stack.push(vec![]);
    implicit_para = true;
}
if !inline_stack.is_empty() {
    push_text!(s);
}
```

4. **GeneralRef arm** — before the existing `let s = crate::xmltext::resolve_general_ref(&r);`, add the same opener so `&amp;` at flow level isn't lost:

```rust
if inline_stack.is_empty() && in_body && cur_block.is_none() {
    inline_stack.push(vec![]);
    implicit_para = true;
}
```

5. **Eof arm** — add `close_implicit!();` as the first statement, before the frame-flush loop.

No handlers needed for `blockquote`/`dl`/`dt`/`dd`/`div`/`section`/`hr` — they hit the `_ => {}` arms and are transparent by construction; the implicit-para machinery captures their bare text.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters epub::xhtml`
Expected: PASS, all pre-existing tests still green (the whitespace tests exercise text inside `<p>`, untouched by this change).

- [ ] **Step 5: Gate and commit**

```bash
mise run lint && mise run test
git add crates/kasane-adapters/src/epub/xhtml.rs
git commit -m "feat(epub): flatten transparent containers via implicit paragraphs"
```

---

### Task 3: Tables

**Files:**
- Modify: `crates/kasane-adapters/src/epub/xhtml.rs`

**Interfaces:**
- Consumes: Tasks 1–2 machinery.
- Produces: `BlockFrame::Table` variant; generalized `emit_block(frames, inline_stack, out, b)` signature (gains `inline_stack: &mut Vec<Vec<Inline>>` — **update Task 1's call sites**); `fn flatten_block_inlines(b: &Block, dst: &mut Vec<Inline>)`. Later tasks use the 4-arg `emit_block`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn parses_table_with_thead() {
    let mut id = 0;
    let blocks = xhtml_to_blocks(
        "<body><table><thead><tr><th>H1</th><th>H2</th></tr></thead>\
         <tbody><tr><td>a</td><td><em>b</em></td></tr></tbody></table></body>",
        &mut id,
    );
    let Block::Table(t) = &blocks[0] else { panic!("expected Table") };
    assert!(!t.has_merged);
    assert_eq!(text_of(&t.header[0]), "H1");
    assert_eq!(text_of(&t.header[1]), "H2");
    assert_eq!(t.rows.len(), 1);
    assert_eq!(text_of(&t.rows[0][0]), "a");
    assert!(matches!(&t.rows[0][1][0], Inline::Emph(_)));
}

#[test]
fn th_only_first_row_without_thead_becomes_header() {
    let mut id = 0;
    let blocks = xhtml_to_blocks(
        "<body><table><tr><th>H</th></tr><tr><td>v</td></tr></table></body>",
        &mut id,
    );
    let Block::Table(t) = &blocks[0] else { panic!() };
    assert_eq!(text_of(&t.header[0]), "H");
    assert_eq!(t.rows.len(), 1);
}

#[test]
fn headerless_table_promotes_first_row() {
    let mut id = 0;
    let blocks = xhtml_to_blocks(
        "<body><table><tr><td>a</td></tr><tr><td>b</td></tr></table></body>",
        &mut id,
    );
    let Block::Table(t) = &blocks[0] else { panic!() };
    assert_eq!(text_of(&t.header[0]), "a"); // GFM requires a header row
    assert_eq!(t.rows.len(), 1);
}

#[test]
fn colspan_sets_merged_flag() {
    let mut id = 0;
    let blocks = xhtml_to_blocks(
        "<body><table><tr><td colspan=\"2\">wide</td></tr><tr><td>a</td><td>b</td></tr></table></body>",
        &mut id,
    );
    let Block::Table(t) = &blocks[0] else { panic!() };
    assert!(t.has_merged);
}

#[test]
fn short_row_is_padded_to_table_width() {
    let mut id = 0;
    let blocks = xhtml_to_blocks(
        "<body><table><tr><th>A</th><th>B</th></tr><tr><td>only</td></tr></table></body>",
        &mut id,
    );
    let Block::Table(t) = &blocks[0] else { panic!() };
    assert_eq!(t.rows[0].len(), 2, "short row padded with an empty cell");
    assert!(t.rows[0][1].is_empty());
}

#[test]
fn paragraph_inside_cell_flattens_to_cell_inlines() {
    let mut id = 0;
    let blocks = xhtml_to_blocks(
        "<body><table><tr><td><p>x</p><p>y</p></td></tr></table></body>",
        &mut id,
    );
    let Block::Table(t) = &blocks[0] else { panic!() };
    assert_eq!(text_of(&t.header[0]), "x y"); // promoted headerless row; paras space-joined
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters epub::xhtml`
Expected: FAIL — no `Block::Table` is produced.

- [ ] **Step 3: Implement**

1. Add the frame variant:

```rust
Table {
    header: Vec<Vec<Inline>>,
    rows: Vec<Vec<Vec<Inline>>>,
    has_merged: bool,
    in_thead: bool,
    cur_row: Vec<Vec<Inline>>,
    row_has_td: bool,
},
```

2. Generalize `emit_block` — a block finishing while an inline collection is open (a table cell, later a figcaption) flattens into it instead of escaping the container:

```rust
fn emit_block(
    frames: &mut [BlockFrame],
    inline_stack: &mut [Vec<Inline>],
    out: &mut Vec<Block>,
    b: Block,
) {
    if let Some(top) = inline_stack.last_mut() {
        if !top.is_empty() {
            crate::xmltext::push_inline(top, Inline::Text(" ".into()));
        }
        flatten_block_inlines(&b, top);
        return;
    }
    match frames.last_mut() {
        None => out.push(b),
        Some(BlockFrame::List { items, .. }) => {
            if items.is_empty() {
                items.push(Vec::new());
            }
            items.last_mut().expect("non-empty").push(b);
        }
        // A block emitted directly under <table> (stray content between rows)
        // has nowhere to go; degrade by dropping structure, keeping nothing —
        // real content inside cells is caught by the inline_stack branch above.
        Some(BlockFrame::Table { .. }) => {}
    }
}

// Extracts a block's text content as inlines — used when block markup appears
// where only inlines fit (inside a table cell). Structure is lost, text is not.
fn flatten_block_inlines(b: &Block, dst: &mut Vec<Inline>) {
    let sep = |dst: &mut Vec<Inline>| {
        if !dst.is_empty() {
            crate::xmltext::push_inline(dst, Inline::Text(" ".into()));
        }
    };
    match b {
        Block::Para(inls) | Block::Heading { inlines: inls, .. } => {
            dst.extend(inls.iter().cloned())
        }
        Block::List { items, .. } => {
            for item in items {
                for ib in item {
                    sep(dst);
                    flatten_block_inlines(ib, dst);
                }
            }
        }
        Block::Table(t) => {
            for row in std::iter::once(&t.header).chain(t.rows.iter()) {
                for cell in row {
                    sep(dst);
                    dst.extend(cell.iter().cloned());
                }
            }
        }
        Block::Figure { caption, .. } => dst.extend(caption.iter().cloned()),
        Block::CodeBlock { text, .. } => dst.push(Inline::Code(text.clone())),
        Block::MathBlock(s) => dst.push(Inline::Math(s.clone())),
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                sep(dst);
                flatten_block_inlines(ib, dst);
            }
        }
        Block::Raw { .. } => {}
    }
}
```

Update every existing `emit_block(&mut frames, &mut blocks, ...)` call (Task 1 list handlers, `close_implicit!`, heading/para End handlers, `finish_frame`) to `emit_block(&mut frames, &mut inline_stack, &mut blocks, ...)`. `finish_frame` gains the same parameter and threads it through.

3. Start handlers (in the element match; `close_implicit!()` already ran for these non-inline tags):

```rust
b"table" => frames.push(BlockFrame::Table {
    header: vec![], rows: vec![], has_merged: false,
    in_thead: false, cur_row: vec![], row_has_td: false,
}),
b"thead" => {
    if let Some(BlockFrame::Table { in_thead, .. }) = frames.last_mut() {
        *in_thead = true;
    }
}
b"tr" => {
    if let Some(BlockFrame::Table { cur_row, row_has_td, .. }) = frames.last_mut() {
        cur_row.clear();
        *row_has_td = false;
    }
}
b"th" | b"td" => {
    if let Some(BlockFrame::Table { has_merged, row_has_td, .. }) = frames.last_mut() {
        let merged = e.attributes().flatten().any(|a| {
            matches!(a.key.as_ref(), b"colspan" | b"rowspan")
                && a.value.as_ref() != b"1"
        });
        *has_merged |= merged;
        *row_has_td |= e.local_name().as_ref() == b"td";
        inline_stack.push(vec![]);
    }
}
```

4. End handlers:

```rust
b"thead" => {
    if let Some(BlockFrame::Table { in_thead, .. }) = frames.last_mut() {
        *in_thead = false;
    }
}
b"th" | b"td" => {
    if matches!(frames.last(), Some(BlockFrame::Table { .. })) {
        let cell = inline_stack.pop().unwrap_or_default();
        if let Some(BlockFrame::Table { cur_row, .. }) = frames.last_mut() {
            cur_row.push(cell);
        }
    }
}
b"tr" => {
    if let Some(BlockFrame::Table { header, rows, in_thead, cur_row, row_has_td, .. }) =
        frames.last_mut()
    {
        let row = std::mem::take(cur_row);
        if row.is_empty() {
        } else if header.is_empty() && rows.is_empty() && (*in_thead || !*row_has_td) {
            *header = row; // thead row, or an all-<th> first row
        } else {
            rows.push(row);
        }
    }
}
b"table" => {
    if matches!(frames.last(), Some(BlockFrame::Table { .. })) {
        let f = frames.pop().expect("checked");
        finish_frame(f, &mut frames, &mut inline_stack, &mut blocks);
    }
}
```

5. `finish_frame` gets the Table arm — width normalization then emit:

```rust
BlockFrame::Table { mut header, mut rows, has_merged, .. } => {
    if header.is_empty() && !rows.is_empty() {
        header = rows.remove(0); // GFM requires a header row
    }
    let width = std::iter::once(header.len())
        .chain(rows.iter().map(Vec::len))
        .max()
        .unwrap_or(0);
    if width == 0 {
        return;
    }
    header.resize(width, Vec::new());
    for r in &mut rows {
        r.resize(width, Vec::new());
    }
    emit_block(frames, inline_stack, out, Block::Table(kasane_ir::Table {
        header, rows, has_merged,
    }));
}
```

6. In the **p End handler**, the generalized `emit_block` inline-absorb branch now handles `<p>` inside `<td>` automatically (the popped para goes through `emit_block`, which sees the cell's open inline frame). Verify the handler still reads:

```rust
b"p" => {
    let inls = inline_stack.pop().unwrap_or_default();
    cur_block = None;
    if !inls.is_empty() {
        emit_block(&mut frames, &mut inline_stack, &mut blocks, Block::Para(inls));
    }
}
```

**Caution:** `th`/`td` push an inline frame at depth ≥ 1 even when no `p` is open. The `GeneralRef` suppress heuristic (`inline_stack.len() == 1`) now also fires for a reference at the very start of a *cell* — that is correct (leading whitespace at the start of a table cell is formatting), so leave it.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters epub::xhtml`
Expected: PASS, all pre-existing tests green.

- [ ] **Step 5: Gate and commit**

```bash
mise run lint && mise run test
git add crates/kasane-adapters/src/epub/xhtml.rs
git commit -m "feat(epub): parse tables with header detection, merged-cell flag, row padding"
```

---

### Task 4: Code blocks, inline code, `<br>`

**Files:**
- Modify: `crates/kasane-adapters/src/epub/xhtml.rs`

**Interfaces:**
- Consumes: Tasks 1–3 machinery.
- Produces: loop state `pre: Option<(Option<String>, String)>`; `fn inlines_text(inls: &[Inline]) -> String`. Task 9's fixture exercises these.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn pre_becomes_code_block_with_verbatim_whitespace() {
    let mut id = 0;
    let blocks = xhtml_to_blocks(
        "<body><pre><code class=\"language-rust\">fn main() {\n    let x = 1 &amp; 2;\n}</code></pre></body>",
        &mut id,
    );
    let Block::CodeBlock { lang, text } = &blocks[0] else {
        panic!("expected CodeBlock, got {:?}", blocks[0])
    };
    assert_eq!(lang.as_deref(), Some("rust"));
    assert_eq!(text, "fn main() {\n    let x = 1 & 2;\n}");
}

#[test]
fn pre_without_code_child_still_works() {
    let mut id = 0;
    let blocks = xhtml_to_blocks("<body><pre>plain  spaced</pre></body>", &mut id);
    assert!(matches!(&blocks[0], Block::CodeBlock { lang: None, text } if text == "plain  spaced"));
}

#[test]
fn inline_code_survives_in_paragraph() {
    let mut id = 0;
    let blocks = xhtml_to_blocks("<body><p>call <code>foo()</code> now</p></body>", &mut id);
    let Block::Para(inls) = &blocks[0] else { panic!() };
    assert!(inls.iter().any(|i| matches!(i, Inline::Code(t) if t == "foo()")));
}

#[test]
fn br_becomes_single_space() {
    let mut id = 0;
    let blocks = xhtml_to_blocks("<body><p>line one<br/>line two</p></body>", &mut id);
    let Block::Para(inls) = &blocks[0] else { panic!() };
    assert_eq!(text_of(inls), "line one line two");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters epub::xhtml`
Expected: FAIL — no CodeBlock; inline code text appears as plain `Text` or is dropped; br test shows `"line oneline two"`.

- [ ] **Step 3: Implement**

1. State: `let mut pre: Option<(Option<String>, String)> = None;` — `(lang, accumulated text)`.

2. **Intercept while inside `<pre>`** — at the top of the event loop, before the main `match`, restructure to:

```rust
let ev = reader.read_event_into(&mut buf);
if let Some((lang, text)) = pre.as_mut() {
    match &ev {
        Ok(Event::Text(t)) => {
            // Verbatim: no trim, no pending_ws — whitespace IS the content here.
            text.push_str(&t.decode().map(|d| d.into_owned()).unwrap_or_default());
        }
        Ok(Event::GeneralRef(r)) => {
            text.push_str(&crate::xmltext::resolve_general_ref(r));
        }
        Ok(Event::Start(e)) if e.local_name().as_ref() == b"code" && lang.is_none() => {
            *lang = e.attributes().flatten()
                .find(|a| a.key.as_ref() == b"class")
                .and_then(|a| {
                    String::from_utf8_lossy(&a.value)
                        .split_whitespace()
                        .find_map(|c| c.strip_prefix("language-").map(str::to_string))
                });
        }
        Ok(Event::End(e)) if e.local_name().as_ref() == b"pre" => {
            let (lang, text) = pre.take().expect("in pre");
            let text = text.trim_matches('\n').to_string();
            emit_block(&mut frames, &mut inline_stack, &mut blocks,
                Block::CodeBlock { lang, text });
        }
        Ok(Event::Eof) => {
            let (lang, text) = pre.take().expect("in pre");
            emit_block(&mut frames, &mut inline_stack, &mut blocks,
                Block::CodeBlock { lang, text: text.trim_matches('\n').to_string() });
            break;
        }
        _ => {} // other markup inside <pre> is ignored, its text still arrives as Text events
    }
    buf.clear();
    continue;
}
match ev {
    // ... existing arms unchanged ...
}
```

3. **Start handlers** (main match; `pre`/`code` are handled before `br`):

```rust
b"pre" => pre = Some((None, String::new())),   // close_implicit! already ran (block tag)
b"code" => {
    if inline_stack.is_empty() && in_body && cur_block.is_none() {
        inline_stack.push(vec![]); // host an implicit para for flow-level <code>
        implicit_para = true;
    }
    inline_stack.push(vec![]);
}
b"br" => {
    if let Some(top) = inline_stack.last_mut() {
        if !top.is_empty() {
            crate::xmltext::push_inline(top, Inline::Text(" ".into()));
        }
    }
}
```

`br` must be listed in `is_inline_tag` (it already is from Task 2).

4. **End handler:**

```rust
b"code" => {
    let x = inline_stack.pop().unwrap_or_default();
    if let Some(top) = inline_stack.last_mut() {
        top.push(Inline::Code(inlines_text(&x)));
    }
}
```

5. Helper:

```rust
// Inline code is a flat string in the IR; nested markup inside <code> keeps
// its text only.
fn inlines_text(inls: &[Inline]) -> String {
    let mut s = String::new();
    for i in inls {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => s.push_str(t),
            Inline::Emph(x) | Inline::Strong(x) => s.push_str(&inlines_text(x)),
            Inline::Link { inlines, .. } => s.push_str(&inlines_text(inlines)),
            Inline::FootnoteRef(_) => {}
        }
    }
    s
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters epub::xhtml`
Expected: PASS, pre-existing tests green.

- [ ] **Step 5: Gate and commit**

```bash
mise run lint && mise run test
git add crates/kasane-adapters/src/epub/xhtml.rs
git commit -m "feat(epub): parse pre/code blocks, inline code, and br"
```

---

### Task 5: Figures and images (parse side)

**Files:**
- Modify: `crates/kasane-adapters/src/epub/xhtml.rs`
- Modify: `crates/kasane-adapters/src/epub/mod.rs` (call site)
- Modify: `crates/kasane-adapters/src/guard.rs` (add `has_scheme`)

**Interfaces:**
- Consumes: `crate::guard::resolve_rel(base_dir, target) -> Option<String>`.
- Produces: **signature change** `pub fn xhtml_to_blocks(xml: &str, base_dir: &str, next_id: &mut u32) -> Vec<Block>` — `base_dir` is the XHTML file's parent directory inside the zip (e.g. `"OEBPS"`), used to resolve `img src` to zip-entry keys. Also `pub(crate) fn has_scheme(href: &str) -> bool` in `guard.rs` (Task 7 reuses it). `BlockFrame::Figure` variant. Emitted `Block::Figure.image.key` is the resolved zip-internal path; Task 6 extracts bytes for those keys.

- [ ] **Step 1: Write the failing tests**

In `guard.rs` tests:

```rust
#[test]
fn has_scheme_detects_urls_not_paths() {
    assert!(has_scheme("http://x/a.png"));
    assert!(has_scheme("data:image/png;base64,AA"));
    assert!(has_scheme("mailto:a@b"));
    assert!(!has_scheme("images/a.png"));
    assert!(!has_scheme("../images/a.png"));
    assert!(!has_scheme("a/b:c.png")); // colon after a slash is not a scheme
    assert!(!has_scheme("#frag"));
}
```

In `xhtml.rs` tests — add this helper at the top of `mod tests` and use it in the new tests (existing tests get mechanically updated in Step 3):

```rust
fn parse(xml: &str) -> Vec<Block> {
    let mut id = 0;
    xhtml_to_blocks(xml, "OEBPS", &mut id)
}

#[test]
fn figure_with_img_and_figcaption() {
    let blocks = parse(
        "<body><figure><img src=\"../images/cat.png\" alt=\"a cat\"/>\
         <figcaption>Feline <em>friend</em></figcaption></figure></body>",
    );
    let Block::Figure { image, caption, number } = &blocks[0] else {
        panic!("expected Figure, got {:?}", blocks[0])
    };
    assert_eq!(image.key, "images/cat.png"); // resolved against OEBPS, ../ normalized
    assert_eq!(text_of(caption), "Feline ");
    assert!(matches!(&caption[1], Inline::Emph(_)));
    assert!(number.is_none());
}

#[test]
fn bare_img_uses_alt_as_caption() {
    let blocks = parse("<body><img src=\"pic.png\" alt=\"desc\"/></body>");
    let Block::Figure { image, caption, .. } = &blocks[0] else { panic!() };
    assert_eq!(image.key, "OEBPS/pic.png");
    assert_eq!(text_of(caption), "desc");
}

#[test]
fn figure_img_alt_used_when_no_figcaption() {
    let blocks = parse("<body><figure><img src=\"p.png\" alt=\"fallback\"/></figure></body>");
    let Block::Figure { caption, .. } = &blocks[0] else { panic!() };
    assert_eq!(text_of(caption), "fallback");
}

#[test]
fn remote_img_degrades_to_alt_paragraph() {
    let blocks = parse("<body><img src=\"http://evil/x.png\" alt=\"chart of results\"/></body>");
    assert!(matches!(&blocks[0], Block::Para(i) if text_of(i) == "chart of results"));
}

#[test]
fn remote_img_without_alt_degrades_to_raw_note() {
    let blocks = parse("<body><img src=\"data:image/png;base64,AA\"/></body>");
    assert!(matches!(&blocks[0], Block::Raw { .. }));
}

#[test]
fn traversal_img_src_degrades() {
    let blocks = parse("<body><img src=\"../../../etc/passwd\" alt=\"x\"/></body>");
    assert!(matches!(&blocks[0], Block::Para(_)));
}

#[test]
fn figure_without_img_flattens_caption_to_para() {
    let blocks = parse("<body><figure><figcaption>orphan caption</figcaption></figure></body>");
    assert!(matches!(&blocks[0], Block::Para(i) if text_of(i) == "orphan caption"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters`
Expected: FAIL to compile (`has_scheme` undefined; `xhtml_to_blocks` arity) — that counts as the failing state; fix compile errors as part of Step 3, then behavioral asserts drive the rest.

- [ ] **Step 3: Implement**

1. `guard.rs`:

```rust
/// True when `href` starts with a URL scheme (`http:`, `data:`, `mailto:`, …)
/// rather than being a document-relative path. A colon only counts before the
/// first `/`, `#`, or `?`.
pub(crate) fn has_scheme(href: &str) -> bool {
    href.chars()
        .take_while(|c| !matches!(c, '/' | '#' | '?'))
        .any(|c| c == ':')
}
```

2. `xhtml.rs` — change the signature to `pub fn xhtml_to_blocks(xml: &str, base_dir: &str, next_id: &mut u32) -> Vec<Block>`. Update the call in `epub/mod.rs`'s spine loop:

```rust
let file_dir = name.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
for b in xhtml::xhtml_to_blocks(&xml, &file_dir, &mut next_id) {
```

Mechanically update **every existing test call site** in `xhtml.rs` from `xhtml_to_blocks(X, &mut id)` to the new `parse(X)` helper (or `xhtml_to_blocks(X, "OEBPS", &mut id)` where the test needs its own id counter).

3. Frame variant:

```rust
Figure {
    image: Option<AssetRef>,
    alt: Vec<Inline>,
    caption: Vec<Inline>,
},
```

(`use kasane_ir::AssetRef;` joins the imports.)

4. Start handlers:

```rust
b"figure" => frames.push(BlockFrame::Figure { image: None, alt: vec![], caption: vec![] }),
b"figcaption" => inline_stack.push(vec![]),
b"img" => {
    let attr = |k: &[u8]| e.attributes().flatten()
        .find(|a| a.key.as_ref() == k)
        .map(|a| String::from_utf8_lossy(&a.value).into_owned());
    let src = attr(b"src").unwrap_or_default();
    let alt = attr(b"alt").unwrap_or_default();
    let key = if src.is_empty() || crate::guard::has_scheme(&src) {
        None
    } else {
        crate::guard::resolve_rel(base_dir, &src)
    };
    match key {
        Some(key) => {
            let aref = AssetRef { key, bytes_ref: 0 };
            let alt_inls = if alt.is_empty() { vec![] } else { vec![Inline::Text(alt)] };
            if let Some(BlockFrame::Figure { image, alt: falt, .. }) = frames.last_mut() {
                if image.is_none() {
                    *image = Some(aref);
                    *falt = alt_inls;
                }
            } else {
                emit_block(&mut frames, &mut inline_stack, &mut blocks,
                    Block::Figure { image: aref, caption: alt_inls, number: None });
            }
        }
        None => {
            eprintln!("warning: skipping image with unusable src '{src}'");
            let b = if alt.is_empty() {
                Block::Raw { note: format!("image unavailable: {src}") }
            } else {
                Block::Para(vec![Inline::Text(alt)])
            };
            emit_block(&mut frames, &mut inline_stack, &mut blocks, b);
        }
    }
}
```

(`img`, `figure`, `figcaption` are block-level: `close_implicit!()` already fires for them since they're not in `is_inline_tag`.)

5. End handlers:

```rust
b"figcaption" => {
    let x = inline_stack.pop().unwrap_or_default();
    if let Some(BlockFrame::Figure { caption, .. }) = frames.last_mut() {
        *caption = x;
    } else if let Some(top) = inline_stack.last_mut() {
        top.extend(x);
    } else if !x.is_empty() {
        emit_block(&mut frames, &mut inline_stack, &mut blocks, Block::Para(x));
    }
}
b"figure" => {
    if matches!(frames.last(), Some(BlockFrame::Figure { .. })) {
        let f = frames.pop().expect("checked");
        finish_frame(f, &mut frames, &mut inline_stack, &mut blocks);
    }
}
```

6. `finish_frame` Figure arm:

```rust
BlockFrame::Figure { image, alt, caption } => {
    let caption = if caption.is_empty() { alt } else { caption };
    match image {
        Some(image) => emit_block(frames, inline_stack, out,
            Block::Figure { image, caption, number: None }),
        None if !caption.is_empty() => {
            emit_block(frames, inline_stack, out, Block::Para(caption)) // never drop
        }
        None => {}
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters`
Expected: PASS (whole crate — the mod.rs call-site change is covered by the existing `parses_minimal_epub_to_ir` test).

- [ ] **Step 5: Gate and commit**

```bash
mise run lint && mise run test
git add crates/kasane-adapters/src/epub/xhtml.rs crates/kasane-adapters/src/epub/mod.rs crates/kasane-adapters/src/guard.rs
git commit -m "feat(epub): parse figures and images with guarded src resolution"
```

---

### Task 6: Asset extraction into the AssetBag

**Files:**
- Modify: `crates/kasane-adapters/src/guard.rs` (receive shared helpers)
- Modify: `crates/kasane-adapters/src/pptx/mod.rs` (use shared helpers)
- Modify: `crates/kasane-adapters/src/epub/mod.rs`

**Interfaces:**
- Consumes: Figures with zip-path keys from Task 5; `read_entry_bytes`.
- Produces: `pub(crate) fn safe_media_filename(archive_path: &str, n: usize) -> String` and `pub(crate) fn parent_dir(path: &str) -> String` **moved verbatim** from `pptx/mod.rs` into `guard.rs` (pptx updated to call `crate::guard::…`). EPUB parse now returns a populated `AssetBag`; unreadable figures degrade in-IR.

- [ ] **Step 1: Write the failing test** (in `epub/mod.rs` `mod tests` — build an EPUB in-memory like `pptx/mod.rs::build_pptx` does)

```rust
fn add<W: std::io::Write + std::io::Seek>(w: &mut zip::ZipWriter<W>, name: &str, data: &[u8]) {
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    w.start_file(name, opts).unwrap();
    std::io::Write::write_all(w, data).unwrap();
}

fn build_epub(chapter_xhtml: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    add(&mut w, "mimetype", b"application/epub+zip");
    add(&mut w, "META-INF/container.xml",
        br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#);
    add(&mut w, "OEBPS/content.opf",
        br#"<package><metadata><dc:title>T</dc:title></metadata>
        <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
        <spine><itemref idref="c1"/></spine></package>"#);
    add(&mut w, "OEBPS/ch1.xhtml", chapter_xhtml.as_bytes());
    for (name, data) in extra {
        add(&mut w, name, data);
    }
    w.finish().unwrap();
    buf.into_inner()
}

#[test]
fn extracts_referenced_image_into_asset_bag() {
    let bytes = build_epub(
        "<body><h1>C</h1><img src=\"images/cat.png\" alt=\"cat\"/></body>",
        &[("OEBPS/images/cat.png", b"\x89PNG\r\n\x1a\nFAKE")],
    );
    let (doc, assets) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
    assert_eq!(assets.items.len(), 1);
    assert_eq!(assets.items[0].key, "OEBPS/images/cat.png");
    assert!(assets.items[0].bytes.starts_with(b"\x89PNG"));
    assert!(doc.nodes.iter().any(|n| matches!(&n.block, Block::Figure { .. })));
}

#[test]
fn missing_image_degrades_to_alt_paragraph() {
    let bytes = build_epub(
        "<body><h1>C</h1><img src=\"images/gone.png\" alt=\"lost chart\"/></body>",
        &[],
    );
    let (doc, assets) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
    assert!(assets.items.is_empty());
    assert!(!doc.nodes.iter().any(|n| matches!(&n.block, Block::Figure { .. })));
    assert!(doc.nodes.iter().any(|n| matches!(&n.block,
        Block::Para(i) if i.iter().any(|x| matches!(x, Inline::Text(t) if t == "lost chart")))));
}

#[test]
fn same_image_referenced_twice_extracted_once() {
    let xhtml = "<body><h1>C</h1><img src=\"i.png\" alt=\"a\"/><img src=\"i.png\" alt=\"b\"/></body>";
    let bytes = build_epub(xhtml, &[("OEBPS/i.png", b"\x89PNG\r\n\x1a\nX")]);
    let (doc, assets) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
    assert_eq!(assets.items.len(), 1);
    let figs = doc.nodes.iter()
        .filter(|n| matches!(&n.block, Block::Figure { .. })).count();
    assert_eq!(figs, 2);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters epub::tests`
Expected: FAIL — `assets.items` is empty (adapter returns `AssetBag::default()`); missing-image test finds a lingering `Figure`.

- [ ] **Step 3: Implement**

1. Move `safe_media_filename` and `parent_dir` from `pptx/mod.rs` to `guard.rs` unchanged, marked `pub(crate)`; update pptx call sites to `crate::guard::safe_media_filename(...)` / `crate::guard::parent_dir(...)` and delete the pptx-local copies.

2. In `epub/mod.rs`, after the spine loop and before building `Document`:

```rust
// Extract every referenced image once; remember which keys failed so their
// Figures can degrade instead of rendering a broken link.
let mut assets = AssetBag::default();
let mut seen: std::collections::HashMap<String, bool> = Default::default(); // key -> readable
for n in &nodes {
    collect_figure_keys(&n.block, &mut |key: &str| {
        if seen.contains_key(key) {
            return;
        }
        match crate::ziputil::read_entry_bytes(&mut zip, key, &mut total_read) {
            Ok(data) => {
                let filename = crate::guard::safe_media_filename(key, assets.items.len());
                assets.items.push(AssetItem { key: key.to_string(), filename, bytes: data });
                seen.insert(key.to_string(), true);
            }
            Err(_) => {
                eprintln!("warning: image entry unreadable, degrading figure: {key}");
                seen.insert(key.to_string(), false);
            }
        }
    });
}
let failed: std::collections::HashSet<String> =
    seen.into_iter().filter(|(_, ok)| !ok).map(|(k, _)| k).collect();
if !failed.is_empty() {
    for n in &mut nodes {
        degrade_failed_figures(&mut n.block, &failed);
    }
}
```

and return `Ok((doc, assets))` instead of `Ok((doc, AssetBag::default()))`.

3. Helpers at the bottom of `epub/mod.rs`:

```rust
// Figures can sit inside lists/footnotes, so walk recursively.
fn collect_figure_keys(b: &Block, f: &mut impl FnMut(&str)) {
    match b {
        Block::Figure { image, .. } => f(&image.key),
        Block::List { items, .. } => {
            for item in items {
                for ib in item {
                    collect_figure_keys(ib, f);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                collect_figure_keys(ib, f);
            }
        }
        _ => {}
    }
}

fn degrade_failed_figures(b: &mut Block, failed: &std::collections::HashSet<String>) {
    match b {
        Block::Figure { image, caption, .. } if failed.contains(&image.key) => {
            *b = if caption.is_empty() {
                Block::Raw { note: format!("image unavailable: {}", image.key) }
            } else {
                Block::Para(std::mem::take(caption))
            };
        }
        Block::List { items, .. } => {
            for item in items {
                for ib in item {
                    degrade_failed_figures(ib, failed);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                degrade_failed_figures(ib, failed);
            }
        }
        _ => {}
    }
}
```

(SVG needs no special handling — files in the zip are copied byte-for-byte, spec §3.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters`
Expected: PASS, including pptx tests (helper move is behavior-preserving).

- [ ] **Step 5: Gate and commit**

```bash
mise run lint && mise run test
git add crates/kasane-adapters/src
git commit -m "feat(epub): extract referenced images into the AssetBag with guarded reads"
```

---

### Task 7: Anchor map and internal-link resolution

**Files:**
- Modify: `crates/kasane-adapters/src/epub/xhtml.rs`
- Modify: `crates/kasane-adapters/src/epub/mod.rs`

**Interfaces:**
- Consumes: `crate::guard::{resolve_rel, parent_dir, has_scheme}`.
- Produces: **signature change** — `xhtml_to_blocks` returns
  ```rust
  pub struct FileParse {
      pub blocks: Vec<Block>,
      pub anchors: Vec<(String, BlockId)>,      // id attr -> nearest preceding heading
      pub first_heading: Option<BlockId>,
  }
  ```
  and `epub/mod.rs` gains `fn fix_links(nodes: &mut [Node], map: &HashMap<(String, String), BlockId>)`. Task 8 extends both.

- [ ] **Step 1: Write the failing tests**

In `xhtml.rs` tests (update the `parse` helper to return `FileParse`; keep a `parse_blocks` wrapper returning `.blocks` so existing tests need only a rename):

```rust
fn parse(xml: &str) -> FileParse {
    let mut id = 0;
    xhtml_to_blocks(xml, "OEBPS", &mut id)
}
fn parse_blocks(xml: &str) -> Vec<Block> {
    parse(xml).blocks
}

#[test]
fn anchors_map_ids_to_nearest_preceding_heading() {
    let fp = parse(
        "<body><h1 id=\"top\">A</h1><p id=\"p1\">x</p><h2 id=\"s2\">B</h2><p id=\"p2\">y</p></body>",
    );
    assert_eq!(fp.first_heading, Some(BlockId(0)));
    let get = |k: &str| fp.anchors.iter().find(|(a, _)| a == k).map(|(_, b)| *b);
    assert_eq!(get("top"), Some(BlockId(0))); // id on the heading -> the heading itself
    assert_eq!(get("p1"), Some(BlockId(0)));
    assert_eq!(get("s2"), Some(BlockId(1)));
    assert_eq!(get("p2"), Some(BlockId(1)));
}

#[test]
fn pre_heading_ids_resolve_to_first_heading() {
    let fp = parse("<body><p id=\"intro\">x</p><h1>A</h1></body>");
    assert_eq!(
        fp.anchors.iter().find(|(a, _)| a == "intro").map(|(_, b)| *b),
        Some(BlockId(0))
    );
}

#[test]
fn headingless_file_records_no_anchors() {
    let fp = parse("<body><p id=\"x\">y</p></body>");
    assert!(fp.anchors.is_empty());
    assert!(fp.first_heading.is_none());
}
```

In `epub/mod.rs` tests (uses Task 6's `build_epub`, extended to two chapters):

```rust
fn build_epub2(ch1: &str, ch2: &str) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    add(&mut w, "mimetype", b"application/epub+zip");
    add(&mut w, "META-INF/container.xml",
        br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#);
    add(&mut w, "OEBPS/content.opf",
        br#"<package><metadata><dc:title>T</dc:title></metadata>
        <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
        <item id="c2" href="ch2.xhtml" media-type="application/xhtml+xml"/></manifest>
        <spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#);
    add(&mut w, "OEBPS/ch1.xhtml", ch1.as_bytes());
    add(&mut w, "OEBPS/ch2.xhtml", ch2.as_bytes());
    w.finish().unwrap();
    buf.into_inner()
}

fn first_link_target(doc: &Document) -> Option<RefTarget> {
    doc.nodes.iter().find_map(|n| match &n.block {
        Block::Para(inls) => inls.iter().find_map(|i| match i {
            Inline::Link { target, .. } => Some(target.clone()),
            _ => None,
        }),
        _ => None,
    })
}

#[test]
fn cross_file_link_resolves_to_internal_block_id() {
    let bytes = build_epub2(
        "<body><h1>One</h1><p><a href=\"ch2.xhtml#s2\">go</a></p></body>",
        "<body><h1>Two</h1><h2 id=\"s2\">Sect</h2><p>t</p></body>",
    );
    let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
    // ch1: h1 -> BlockId(0); ch2: h1 -> 1, h2#s2 -> 2
    assert!(matches!(first_link_target(&doc), Some(RefTarget::Internal(BlockId(2)))));
}

#[test]
fn fragmentless_and_unknown_fragment_hrefs_fall_back_to_first_heading() {
    let bytes = build_epub2(
        "<body><h1>One</h1><p><a href=\"ch2.xhtml\">a</a> <a href=\"ch2.xhtml#nope\">b</a></p></body>",
        "<body><h1>Two</h1><p>t</p></body>",
    );
    let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
    let links: Vec<RefTarget> = doc.nodes.iter().flat_map(|n| match &n.block {
        Block::Para(inls) => inls.iter().filter_map(|i| match i {
            Inline::Link { target, .. } => Some(target.clone()),
            _ => None,
        }).collect::<Vec<_>>(),
        _ => vec![],
    }).collect();
    assert!(matches!(links[0], RefTarget::Internal(BlockId(1))));
    assert!(matches!(links[1], RefTarget::Internal(BlockId(1))));
}

#[test]
fn unresolvable_internal_href_strips_to_text() {
    let bytes = build_epub2(
        "<body><h1>One</h1><p><a href=\"missing.xhtml#x\">gone link</a></p></body>",
        "<body><h1>Two</h1><p>t</p></body>",
    );
    let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
    assert!(first_link_target(&doc).is_none(), "link must be stripped");
    assert!(doc.nodes.iter().any(|n| matches!(&n.block,
        Block::Para(i) if i.iter().any(|x| matches!(x, Inline::Text(t) if t == "gone link")))));
}

#[test]
fn external_url_links_stay_external() {
    let bytes = build_epub2(
        "<body><h1>One</h1><p><a href=\"https://example.com\">ext</a></p></body>",
        "<body><h1>Two</h1><p>t</p></body>",
    );
    let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
    assert!(matches!(first_link_target(&doc), Some(RefTarget::External(u)) if u == "https://example.com"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters`
Expected: compile FAIL (`FileParse` undefined), then after scaffolding, assert failures (links still `External("ch2.xhtml#s2")`).

- [ ] **Step 3: Implement — parse side (`xhtml.rs`)**

1. Define `FileParse` (above the function) and change the signature/return. In `xhtml_to_blocks` add state:

```rust
let mut anchors: Vec<(String, BlockId)> = vec![];
let mut pending_anchor_ids: Vec<String> = vec![]; // ids seen before the first heading
let mut first_heading: Option<BlockId> = None;
let mut last_heading: Option<BlockId> = None;
let mut heading_own_id: Option<String> = None; // id attr on the open h1..h6 itself
```

2. **Start arm** — after the `close_implicit!`/body lines, before the element match:

```rust
let id_attr = e.attributes().flatten()
    .find(|a| a.key.as_ref() == b"id")
    .map(|a| String::from_utf8_lossy(&a.value).into_owned());
if let Some(idv) = id_attr {
    if matches!(e.local_name().as_ref(), b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6") {
        heading_own_id = Some(idv); // resolved to the heading's own BlockId at End
    } else if let Some(h) = last_heading {
        anchors.push((idv, h));
    } else {
        pending_anchor_ids.push(idv);
    }
}
```

3. **End h1..h6 handler** — after `blocks.push`/`emit_block` of the heading (where `id` was just assigned):

```rust
last_heading = Some(id);
if first_heading.is_none() {
    first_heading = Some(id);
    for a in pending_anchor_ids.drain(..) {
        anchors.push((a, id));
    }
}
if let Some(own) = heading_own_id.take() {
    anchors.push((own, id));
}
```

**Caution:** when a heading is absorbed into an open inline frame (heading inside a `<td>`, Task 3's `emit_block` absorb branch), it still gets a BlockId here — that is acceptable; the core drops unused ids. Do not special-case it.

4. Return `FileParse { blocks, anchors, first_heading }` (Eof arm falls through to this).

- [ ] **Step 4: Implement — fixup side (`epub/mod.rs`)**

In the spine loop, collect per-file results; after the loop (before asset extraction, order matters for nothing, but keep links first for readability):

```rust
use std::collections::HashMap;
let mut anchor_map: HashMap<(String, String), BlockId> = HashMap::new();
// inside the spine loop, after `let fp = xhtml::xhtml_to_blocks(&xml, &file_dir, &mut next_id);`
for (aid, bid) in &fp.anchors {
    anchor_map.insert((name.clone(), aid.clone()), *bid);
}
if let Some(fh) = fp.first_heading {
    anchor_map.insert((name.clone(), String::new()), fh);
}
// then push fp.blocks into nodes as before
```

After the loop:

```rust
fix_links(&mut nodes, &anchor_map);
```

Helpers at the bottom of `epub/mod.rs`:

```rust
fn fix_links(nodes: &mut [Node], map: &std::collections::HashMap<(String, String), BlockId>) {
    for n in nodes {
        let file = n.prov.source_href.clone().unwrap_or_default();
        fix_block_links(&mut n.block, &file, map);
    }
}

fn fix_block_links(
    b: &mut Block,
    file: &str,
    map: &std::collections::HashMap<(String, String), BlockId>,
) {
    match b {
        Block::Para(inls) | Block::Heading { inlines: inls, .. } => fix_inline_vec(inls, file, map),
        Block::List { items, .. } => {
            for item in items {
                for ib in item {
                    fix_block_links(ib, file, map);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                fix_block_links(ib, file, map);
            }
        }
        Block::Table(t) => {
            for row in std::iter::once(&mut t.header).chain(t.rows.iter_mut()) {
                for cell in row {
                    fix_inline_vec(cell, file, map);
                }
            }
        }
        Block::Figure { caption, .. } => fix_inline_vec(caption, file, map),
        _ => {}
    }
}

fn fix_inline_vec(
    inls: &mut Vec<Inline>,
    file: &str,
    map: &std::collections::HashMap<(String, String), BlockId>,
) {
    let old = std::mem::take(inls);
    for i in old {
        match i {
            Inline::Emph(mut x) => {
                fix_inline_vec(&mut x, file, map);
                inls.push(Inline::Emph(x));
            }
            Inline::Strong(mut x) => {
                fix_inline_vec(&mut x, file, map);
                inls.push(Inline::Strong(x));
            }
            Inline::Link { target: RefTarget::External(h), inlines: mut inner } => {
                fix_inline_vec(&mut inner, file, map);
                if h.is_empty() || crate::guard::has_scheme(&h) {
                    inls.push(Inline::Link { target: RefTarget::External(h), inlines: inner });
                } else {
                    match resolve_internal(file, &h, map) {
                        Some(bid) => inls.push(Inline::Link {
                            target: RefTarget::Internal(bid),
                            inlines: inner,
                        }),
                        None => {
                            eprintln!("warning: unresolved internal link '{h}' in {file}");
                            inls.extend(inner); // link text survives as plain text
                        }
                    }
                }
            }
            other => inls.push(other),
        }
    }
}

// "ch2.xhtml#s2" / "#frag" / "ch2.xhtml" -> a heading BlockId, if the target
// file is in the spine. Exact fragment first, then the file's first heading.
fn resolve_internal(
    file: &str,
    href: &str,
    map: &std::collections::HashMap<(String, String), BlockId>,
) -> Option<BlockId> {
    let (path, frag) = match href.split_once('#') {
        Some((p, f)) => (p, f),
        None => (href, ""),
    };
    let target_file = if path.is_empty() {
        file.to_string()
    } else {
        crate::guard::resolve_rel(&crate::guard::parent_dir(file), path)?
    };
    map.get(&(target_file.clone(), frag.to_string()))
        .or_else(|| map.get(&(target_file, String::new())))
        .copied()
}
```

(Spec deviation, intentional: the spec routes no-entry hrefs through the core's dangling-ref degradation; stripping directly in the fixup produces the identical output — plain text plus a warning — without inventing a sentinel BlockId.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters`
Expected: PASS. Also run `cargo test -p kasane-cli` — the e2e `minimal.epub` test asserts an internal link resolves; it must still pass.

- [ ] **Step 6: Gate and commit**

```bash
mise run lint && mise run test
git add crates/kasane-adapters/src/epub
git commit -m "feat(epub): resolve internal links via a per-file anchor map"
```

---

### Task 8: EPUB3 semantic footnotes

**Files:**
- Modify: `crates/kasane-adapters/src/epub/xhtml.rs`
- Modify: `crates/kasane-adapters/src/epub/mod.rs`

**Interfaces:**
- Consumes: Task 7's `FileParse`, `fix_inline_vec`, `resolve_internal`.
- Produces: **signature change** — `xhtml_to_blocks(xml, base_dir, next_id, next_note: &mut u32) -> FileParse`; `FileParse` gains `pub footnotes: Vec<(String, NoteId)>` (aside id → note) and `pub noteref_hrefs: Vec<String>`; `BlockFrame::Footnote` variant; `epub/mod.rs` gains `fn relocate_footnotes(nodes: Vec<Node>) -> Vec<Node>`.

- [ ] **Step 1: Write the failing tests**

`xhtml.rs` (update `parse` to pass `&mut 0` for `next_note`):

```rust
#[test]
fn semantic_aside_becomes_footnote_block() {
    let fp = parse(
        "<body><h1>C</h1><p>claim<a epub:type=\"noteref\" href=\"#fn1\">1</a></p>\
         <aside epub:type=\"footnote\" id=\"fn1\"><p>the details</p></aside></body>",
    );
    let Some(Block::Footnote { id, blocks }) =
        fp.blocks.iter().find(|b| matches!(b, Block::Footnote { .. }))
    else { panic!("expected Footnote block") };
    assert_eq!(*id, NoteId(0));
    assert!(matches!(&blocks[0], Block::Para(i) if text_of(i) == "the details"));
    assert_eq!(fp.footnotes, vec![("fn1".to_string(), NoteId(0))]);
    assert_eq!(fp.noteref_hrefs, vec!["#fn1".to_string()]);
}

#[test]
fn non_footnote_aside_stays_transparent() {
    let fp = parse("<body><h1>C</h1><aside><p>sidebar</p></aside></body>");
    assert!(!fp.blocks.iter().any(|b| matches!(b, Block::Footnote { .. })));
    assert!(fp.blocks.iter().any(|b| matches!(b, Block::Para(i) if text_of(i) == "sidebar")));
}
```

`epub/mod.rs`:

```rust
#[test]
fn noteref_pairs_with_aside_and_relocates_definition() {
    let bytes = build_epub2(
        "<body><h1>One</h1><p>claim<a epub:type=\"noteref\" href=\"#fn1\">1</a></p>\
         <p>filler paragraph</p>\
         <aside epub:type=\"footnote\" id=\"fn1\"><p>note body</p></aside></body>",
        "<body><h1>Two</h1><p>t</p></body>",
    );
    let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
    // NoteIds are 1-based at the adapter level so rendered markers read [^1].
    // The para holds a FootnoteRef, not a Link.
    let ref_idx = doc.nodes.iter().position(|n| matches!(&n.block,
        Block::Para(i) if i.iter().any(|x| matches!(x, Inline::FootnoteRef(NoteId(1)))))).unwrap();
    // The Footnote block was moved to immediately after the referencing para.
    assert!(matches!(&doc.nodes[ref_idx + 1].block, Block::Footnote { id: NoteId(1), .. }),
        "footnote must directly follow its first reference, got {:?}", doc.nodes[ref_idx + 1].block);
}

#[test]
fn orphan_noteref_falls_back_to_internal_link_path() {
    let bytes = build_epub2(
        "<body><h1>One</h1><p>x<a epub:type=\"noteref\" href=\"#nosuch\">1</a></p></body>",
        "<body><h1>Two</h1><p>t</p></body>",
    );
    let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
    assert!(!doc.nodes.iter().any(|n| matches!(&n.block,
        Block::Para(i) if i.iter().any(|x| matches!(x, Inline::FootnoteRef(_))))));
    // "#nosuch" resolves via first-heading fallback -> stays a link, Internal
    assert!(matches!(first_link_target(&doc), Some(RefTarget::Internal(_))));
}

#[test]
fn unreferenced_aside_stays_in_place() {
    let bytes = build_epub2(
        "<body><h1>One</h1><p>x</p><aside epub:type=\"footnote\" id=\"fn9\"><p>lonely</p></aside></body>",
        "<body><h1>Two</h1><p>t</p></body>",
    );
    let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
    assert!(doc.nodes.iter().any(|n| matches!(&n.block, Block::Footnote { .. })));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters`
Expected: compile FAIL (arity/fields), then behavioral failures.

- [ ] **Step 3: Implement — parse side (`xhtml.rs`)**

1. Signature: `pub fn xhtml_to_blocks(xml: &str, base_dir: &str, next_id: &mut u32, next_note: &mut u32) -> FileParse`. Add `footnotes: Vec<(String, NoteId)>` + `noteref_hrefs: Vec<String>` to `FileParse` and the return. Update `epub/mod.rs` call — `let mut next_note = 1u32;` beside `next_id` (**1-based**, so the writer's `[^{id.0}]` markers read `[^1]`, `[^2]`, … — the xhtml unit tests pass their own `&mut 0` and assert `NoteId(0)`, which is fine) — and the test helpers.

2. Frame variant + aside tracking state:

```rust
Footnote { note: NoteId, blocks: Vec<Block> },
```
```rust
let mut aside_pushed: Vec<bool> = vec![]; // did this <aside> open a Footnote frame?
```

3. Attribute helper (place near `is_inline_tag`):

```rust
// epub:type is a space-separated token list, e.g. "footnote" or "rearnote footnote".
fn epub_type_has(e: &quick_xml::events::BytesStart, token: &str) -> bool {
    e.attributes().flatten()
        .find(|a| a.key.as_ref() == b"epub:type")
        .map(|a| String::from_utf8_lossy(&a.value).split_whitespace().any(|t| t == token))
        .unwrap_or(false)
}
```

4. **Start handlers:**

```rust
b"aside" => {
    if epub_type_has(&e, "footnote") {
        let note = NoteId(*next_note);
        *next_note += 1;
        if let Some(idv) = e.attributes().flatten()
            .find(|a| a.key.as_ref() == b"id")
            .map(|a| String::from_utf8_lossy(&a.value).into_owned())
        {
            footnotes.push((idv, note));
        }
        frames.push(BlockFrame::Footnote { note, blocks: vec![] });
        aside_pushed.push(true);
    } else {
        aside_pushed.push(false); // transparent aside
    }
}
```

and in the existing `b"a"` Start handler, after `link_href` is read:

```rust
if epub_type_has(&e, "noteref") {
    if let Some(h) = &link_href {
        noteref_hrefs.push(h.clone());
    }
}
```

Note: the generic `id`-anchor recording from Task 7 also records the aside's id → nearest heading; that is harmless (the noteref match in the fixup takes priority) — leave it.

5. **End handler:**

```rust
b"aside" => {
    if aside_pushed.pop() == Some(true)
        && matches!(frames.last(), Some(BlockFrame::Footnote { .. }))
    {
        let f = frames.pop().expect("checked");
        finish_frame(f, &mut frames, &mut inline_stack, &mut blocks);
    }
}
```

6. `finish_frame` arm and `emit_block` arm:

```rust
// finish_frame:
BlockFrame::Footnote { note, blocks: fblocks } => {
    if !fblocks.is_empty() {
        emit_block(frames, inline_stack, out, Block::Footnote { id: note, blocks: fblocks });
    }
}
// emit_block match, alongside the List arm:
Some(BlockFrame::Footnote { blocks, .. }) => blocks.push(b),
```

- [ ] **Step 4: Implement — fixup side (`epub/mod.rs`)**

1. Collect per file in the spine loop:

```rust
let mut footnote_map: HashMap<(String, String), NoteId> = HashMap::new();
let mut noteref_keys: std::collections::HashSet<(String, String)> = Default::default();
// in the loop:
for (fid, nid) in &fp.footnotes {
    footnote_map.insert((name.clone(), fid.clone()), *nid);
}
for h in &fp.noteref_hrefs {
    noteref_keys.insert((name.clone(), h.clone()));
}
```

2. Thread both into `fix_links` / `fix_block_links` / `fix_inline_vec` as extra parameters (`footnote_map: &HashMap<(String, String), NoteId>`, `noteref_keys: &HashSet<(String, String)>`), and update Task 7's call site to `fix_links(&mut nodes, &anchor_map, &footnote_map, &noteref_keys);`. Restructure `fix_inline_vec`'s `Link { External(h) }` arm so the noteref check comes first:

```rust
Inline::Link { target: RefTarget::External(h), inlines: mut inner } => {
    fix_inline_vec(&mut inner, file, map, footnote_map, noteref_keys);
    let is_noteref = noteref_keys.contains(&(file.to_string(), h.clone()));
    if is_noteref {
        if let Some(nid) = resolve_footnote(file, &h, footnote_map) {
            // The link text (the marker digit) is dropped: FootnoteRef
            // renders its own [^n] marker.
            inls.push(Inline::FootnoteRef(nid));
            continue;
        }
        // No matching aside: fall through to the ordinary internal-link path.
    }
    if h.is_empty() || crate::guard::has_scheme(&h) {
        inls.push(Inline::Link { target: RefTarget::External(h), inlines: inner });
    } else {
        match resolve_internal(file, &h, map) {
            Some(bid) => inls.push(Inline::Link {
                target: RefTarget::Internal(bid),
                inlines: inner,
            }),
            None => {
                eprintln!("warning: unresolved internal link '{h}' in {file}");
                inls.extend(inner);
            }
        }
    }
}
```

(The `for i in old` loop body allows `continue`.) Helper:

```rust
fn resolve_footnote(
    file: &str,
    href: &str,
    map: &std::collections::HashMap<(String, String), NoteId>,
) -> Option<NoteId> {
    let (path, frag) = match href.split_once('#') {
        Some((p, f)) => (p, f),
        None => (href, ""),
    };
    let target_file = if path.is_empty() {
        file.to_string()
    } else {
        crate::guard::resolve_rel(&crate::guard::parent_dir(file), path)?
    };
    map.get(&(target_file, frag.to_string())).copied()
}
```

3. Relocation — call after `fix_links`, replacing `nodes`:

```rust
nodes = relocate_footnotes(nodes);
```

```rust
// Move each Footnote node to directly after the node holding its first
// FootnoteRef, so GFM [^n]/definition pairs land in the same emitted file
// (spec §4). Unreferenced footnotes stay where they are. Three phases because
// the common case is ref-before-aside: a single forward walk would reach the
// ref while the aside is still ahead and unparked.
fn relocate_footnotes(nodes: Vec<Node>) -> Vec<Node> {
    use std::collections::{HashMap, HashSet};
    let mut referenced: HashSet<NoteId> = HashSet::new();
    for n in &nodes {
        collect_note_refs(&n.block, &mut referenced);
    }
    // Phase 1: pull out every referenced Footnote node (a Footnote block never
    // contains its own ref, so referenced => movable).
    let mut parked: HashMap<NoteId, Node> = HashMap::new();
    let mut rest: Vec<Node> = Vec::with_capacity(nodes.len());
    for n in nodes {
        match &n.block {
            Block::Footnote { id, .. } if referenced.contains(id) => {
                parked.insert(*id, n);
            }
            _ => rest.push(n),
        }
    }
    // Phase 2: append each parked note right after the node with its first ref.
    let mut out: Vec<Node> = Vec::with_capacity(rest.len() + parked.len());
    for n in rest {
        let mut refs_here = HashSet::new();
        collect_note_refs(&n.block, &mut refs_here);
        out.push(n);
        for id in refs_here {
            if let Some(fnote) = parked.remove(&id) {
                out.push(fnote);
            }
        }
    }
    // Phase 3: safety net (e.g. a ref that only appears inside another parked
    // footnote's body) — never drop content.
    out.extend(parked.into_values());
    out
}

fn collect_note_refs(b: &Block, out: &mut std::collections::HashSet<NoteId>) {
    match b {
        Block::Para(inls) | Block::Heading { inlines: inls, .. } => inline_refs(inls, out),
        Block::List { items, .. } => {
            for item in items {
                for ib in item {
                    collect_note_refs(ib, out);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                collect_note_refs(ib, out);
            }
        }
        Block::Table(t) => {
            for row in std::iter::once(&t.header).chain(t.rows.iter()) {
                for cell in row {
                    inline_refs(cell, out);
                }
            }
        }
        Block::Figure { caption, .. } => inline_refs(caption, out),
        _ => {}
    }
}

fn inline_refs(inls: &[Inline], out: &mut std::collections::HashSet<NoteId>) {
    for i in inls {
        match i {
            Inline::FootnoteRef(n) => {
                out.insert(*n);
            }
            Inline::Emph(x) | Inline::Strong(x) | Inline::Link { inlines: x, .. } => {
                inline_refs(x, out)
            }
            _ => {}
        }
    }
}

```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters && cargo test -p kasane-cli`
Expected: PASS.

- [ ] **Step 6: Gate and commit**

```bash
mise run lint && mise run test
git add crates/kasane-adapters/src/epub
git commit -m "feat(epub): pair EPUB3 semantic footnotes and relocate definitions to first use"
```

---

### Task 9: Rich fixture + end-to-end test

**Files:**
- Create: `tests/fixtures/epub/make_rich_epub.py` (generator, committed for regeneration)
- Create: `tests/fixtures/epub/rich.epub` (generated, committed)
- Modify: `crates/kasane-cli/tests/e2e.rs`

**Interfaces:**
- Consumes: the full pipeline via the `kasane` binary.
- Produces: the spec §6 integration fixture — two chapters exercising list, table, image, footnote, cross-chapter link, code block.

- [ ] **Step 1: Write the generator**

`tests/fixtures/epub/make_rich_epub.py`:

```python
#!/usr/bin/env python3
"""Regenerate rich.epub. Run from anywhere: python3 tests/fixtures/epub/make_rich_epub.py"""
import base64, pathlib, zipfile

OUT = pathlib.Path(__file__).parent / "rich.epub"
# 1x1 red PNG
PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842"
    "iQAAAABJRU5ErkJggg=="
)

CONTAINER = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf"
    media-type="application/oebps-package+xml"/></rootfiles>
</container>"""

OPF = """<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="uid">rich-test</dc:identifier>
    <dc:title>Rich Book</dc:title>
    <dc:creator>Fixture Author</dc:creator>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
    <item id="img1" href="images/dot.png" media-type="image/png"/>
  </manifest>
  <spine><itemref idref="c1"/><itemref idref="c2"/></spine>
</package>"""

CH1 = """<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>ch1</title></head>
<body>
  <h1>Chapter One</h1>
  <p>Intro with <em>emphasis</em>, <code>inline_code()</code>, and a
     footnote<a epub:type="noteref" href="#fn1">1</a>.</p>
  <ul><li>alpha</li><li>beta<ul><li>beta-one</li></ul></li></ul>
  <table>
    <thead><tr><th>Name</th><th>Value</th></tr></thead>
    <tbody><tr><td>pi</td><td>3.14</td></tr><tr><td>e</td><td>2.72</td></tr></tbody>
  </table>
  <figure>
    <img src="images/dot.png" alt="a dot"/>
    <figcaption>The red dot</figcaption>
  </figure>
  <pre><code class="language-rust">fn main() {}</code></pre>
  <p>See <a href="ch2.xhtml#sect">the second section</a> for more.</p>
  <aside epub:type="footnote" id="fn1"><p>Footnote body text.</p></aside>
</body></html>"""

CH2 = """<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>ch2</title></head>
<body>
  <h1>Chapter Two</h1>
  <p>Opening paragraph of chapter two.</p>
  <h2 id="sect">Second Section</h2>
  <p>Target of the cross-chapter link.</p>
</body></html>"""

with zipfile.ZipFile(OUT, "w", zipfile.ZIP_STORED) as z:
    z.writestr("mimetype", "application/epub+zip")
    z.writestr("META-INF/container.xml", CONTAINER)
    z.writestr("OEBPS/content.opf", OPF)
    z.writestr("OEBPS/ch1.xhtml", CH1)
    z.writestr("OEBPS/ch2.xhtml", CH2)
    z.writestr("OEBPS/images/dot.png", PNG)
print(f"wrote {OUT}")
```

- [ ] **Step 2: Generate the fixture**

Run: `python3 tests/fixtures/epub/make_rich_epub.py`
Expected: `wrote .../tests/fixtures/epub/rich.epub`

- [ ] **Step 3: Write the failing e2e test** (append to `crates/kasane-cli/tests/e2e.rs`)

```rust
#[test]
fn converts_rich_epub_with_full_fidelity() {
    let out = tempfile::tempdir().unwrap();
    let out_dir = out.path().join("rich");
    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg("../../tests/fixtures/epub/rich.epub")
        .arg("-o")
        .arg(&out_dir)
        // Disable merge/split so section->file mapping is deterministic.
        .arg("--min-tokens").arg("0")
        .arg("--max-tokens").arg("100000")
        .status()
        .unwrap();
    assert!(status.success());

    // Gather every emitted markdown file.
    let mut all = String::new();
    let mut files: Vec<(std::path::PathBuf, String)> = vec![];
    let mut stack = vec![out_dir.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "md") {
                let s = std::fs::read_to_string(&p).unwrap();
                all.push_str(&s);
                files.push((p, s));
            }
        }
    }

    // Lists (nested), table, code — present somewhere in the tree.
    assert!(all.contains("- alpha"), "bullet list missing");
    assert!(all.contains("beta-one"), "nested list item missing");
    assert!(all.contains("| Name | Value |"), "GFM table header missing");
    assert!(all.contains("```rust"), "code block language missing");
    assert!(all.contains("`inline_code()`"), "inline code missing");

    // Image: link in markdown + actual bytes flushed under _assets/.
    assert!(all.contains("![The red dot](_assets/"), "figure link missing");
    let assets: Vec<_> = std::fs::read_dir(out_dir.join("_assets")).unwrap().collect();
    assert_eq!(assets.len(), 1, "exactly one extracted asset");

    // Footnote: ref and definition in the SAME file.
    let fnote_file = files.iter()
        .find(|(_, s)| s.contains("[^1]") && !s.contains("[^1]:"))
        .or_else(|| files.iter().find(|(_, s)| s.contains("[^1]")));
    let (_, s) = fnote_file.expect("no file contains the footnote ref");
    assert!(s.contains("[^1]") && s.contains("[^1]: Footnote body text."),
        "footnote ref and definition must share a file");

    // Cross-chapter link resolved to a real relative .md path.
    let (link_file, link_src) = files.iter()
        .find(|(_, s)| s.contains("](") && s.contains("the second section"))
        .expect("cross-chapter link text missing");
    let target = link_src.split("[the second section](").nth(1)
        .and_then(|r| r.split(')').next())
        .expect("link not in markdown form — was it stripped to text?");
    let target_path = link_file.parent().unwrap()
        .join(target.split('#').next().unwrap());
    assert!(target_path.exists(), "link target {target} does not exist on disk");
}
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p kasane-cli`
Expected: PASS if Tasks 1–8 are complete. If any assert fails, the failing fidelity feature has a bug — fix it in its own crate (this task adds no production code), re-run.

- [ ] **Step 5: Full gate and commit**

```bash
mise run lint && mise run test
git add tests/fixtures/epub/make_rich_epub.py tests/fixtures/epub/rich.epub crates/kasane-cli/tests/e2e.rs
git commit -m "test(epub): rich end-to-end fixture covering full fidelity set"
```

---

## Self-review checklist (run after Task 9)

- Spec §2 lists/tables/figures/code/br/flatten → Tasks 1–5. Spec §3 assets/guards/degradation → Tasks 5–6. Spec §4 anchors/links/footnotes/relocation → Tasks 7–8. Spec §5 degradation cases → Tasks 1 (EOF flush), 3 (row padding), 5–6 (image degradation). Spec §6 tests → per-task units + Task 9 e2e.
- Out of scope confirmed untouched: MathML, `--no-assets`, insta/proptest/fuzz, footnote heuristics.
- `mise run lint && mise run test` green at every commit.
