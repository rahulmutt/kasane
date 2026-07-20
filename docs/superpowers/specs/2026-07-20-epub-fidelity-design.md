# EPUB fidelity — Design Spec

**Date:** 2026-07-20
**Status:** Approved (design), pending implementation plan
**Parent spec:** `2026-07-19-kasane-document-to-markdown-design.md` (deferred "full EPUB fidelity" item)

## 1. Purpose & scope

The EPUB adapter currently converts only headings, paragraphs, emphasis, and links; it
silently drops lists, tables, images, footnotes, and code, and passes internal links
through unresolved as `External`. This work makes the flagship format faithful.

**In scope:** lists, tables, figures + asset extraction, EPUB3 semantic footnotes,
internal-link resolution, code blocks (block and inline), flatten-never-drop handling of
unmapped block elements.

**Out of scope:** MathML→LaTeX (its own backlog item, matching PPTX's deferred OMML
math); the `--no-assets` CLI flag (part of the batch-mode/CLI backlog item);
`insta`/`proptest`/fuzz (the test-hardening backlog item); heuristic footnote detection
for books without EPUB3 semantic markup (their note links resolve as ordinary internal
links, which is correct and non-lossy).

The IR, writer, and core already fully support every block type this work emits — the
writer renders `List`/`Table`/`Figure`/`CodeBlock`/`Footnote`, and the core resolves
`RefTarget::Internal` with dangling-ref degradation. This is adapter-side work in
`crates/kasane-adapters/src/epub/` only.

## 2. Parser architecture: block-frame stack

Chosen over (B) building a DOM then walking it — which would re-port the hard-won
whitespace/dangling-ampersand recovery into a new layer and hold chapters in memory —
and (C) ad-hoc per-element flags, which grow an implicit, buggier stack. One streaming
pass is kept; nesting becomes explicit.

`xhtml_to_blocks` gains a `Vec<BlockFrame>` alongside the existing inline stack.
Completed blocks push into the top frame, or into the output when the stack is empty.
Closing a container pops its frame and folds it into the parent frame (or output).

```rust
enum BlockFrame {
    List { ordered: bool, items: Vec<Vec<Block>> },      // <ul>/<ol>; <li> starts an item
    Table { header: Vec<Vec<Inline>>, rows: Vec<Vec<Vec<Inline>>>,
            has_merged: bool, in_header: bool, cur_row: Vec<Vec<Inline>> },
    Figure { image: Option<AssetRef>, caption: Vec<Inline> },
    Footnote { note_id: NoteId, key: (String, String) }, // <aside epub:type="footnote">
}
```

- **Lists** — a nested `<ul>`/`<ol>` folds into the current `<li>` item, the same shape
  PPTX's `slide.rs` builds.
- **Tables** — `<th>`/`<td>` collect **inlines** (IR cells are `Vec<Inline>`; block
  markup inside a cell flattens to inlines). Any `colspan`/`rowspan` attribute sets
  `has_merged = true`; the writer already emits an HTML fallback for merged tables.
  `<thead>` rows (or `<th>`-only first row) populate `header`.
- **Figures** — `<figure>` opens the frame; `<img>` supplies the asset, `<figcaption>`
  the caption. A bare `<img>` outside `<figure>` becomes a `Figure` directly with its
  `alt` text as caption.
- **Code** — `<pre>` (optional inner `<code class="language-x">`) → `CodeBlock` with
  the language parsed from the class; text inside `<pre>` is collected verbatim
  (whitespace preserved, bypassing the inline whitespace normalization). Inline
  `<code>` outside `<pre>` → `Inline::Code` (currently dropped).
- **`<br>`** → a single space. Accepted v1 limitation: poetry line breaks collapse; the
  IR has no hard-break inline.

**Flatten, never drop.** `blockquote`, `dl`/`dt`/`dd`, `div`, `section`, and other
unmapped block elements are *transparent*: inner `<p>`s emit normally; semantics
(indent, definition structure) are lost but no text is. Bare text at flow level (e.g.
`<blockquote>text</blockquote>`) opens an **implicit paragraph**, closed at the next
tag boundary. Implicit paragraphs activate only inside `<body>` (tracked with a flag)
so `<head><title>` text stays out of the output.

**Unchanged:** the `pending_ws` whitespace machinery, `GeneralRef` entity handling, and
`allow_dangling_amp` recovery are untouched. Frames change where finished blocks and
inlines *land*, not how text is decoded.

## 3. Asset extraction & security

- `<img src>` resolves relative to the current XHTML file's zip-internal path (the same
  normalization `opf.rs` applies to spine hrefs). Every read goes through
  `ziputil.rs`/`guard.rs` — the decompression-ratio and size caps apply to images
  exactly as to XHTML, preserving the AGENTS.md invariant that every guarded zip read
  goes through `ziputil.rs`. No new security surface.
- **AssetBag keys:** the zip-internal path slugified to a safe filename
  (`images/fig 1.png` → `fig-1.png`), deduplicated with a numeric suffix on collision.
  The same image referenced twice gets one bag entry and two `AssetRef`s. The writer
  already confines the flush to `_assets/`.
- **Degradation:** a `src` missing from the zip, failing the guard, or carrying a URL
  scheme (`http:`, `data:` — never fetched; conversion stays offline) becomes a `Para`
  with the alt text, or a `Raw` note when there is no alt, plus a logged warning.
  Never a broken image link.
- SVG files in the zip are copied byte-for-byte to `_assets/`.

## 4. Internal links & footnotes

Two phases: an **anchor map** built during the spine parse, then a **fixup pass** over
the assembled `Document` in `epub/mod.rs`.

**Anchor map.** Only headings carry `BlockId`s, so anchors resolve to headings: any
element with an `id` attribute records `(file, id) → BlockId` of the nearest preceding
heading in that file, falling back to the file's first heading; a file with no headings
records nothing. Each file also records `(file, "")` → its first heading's `BlockId`
for fragment-less hrefs.

**Link fixup.** Each `Link` whose href points into a spine file (`chap2.xhtml#sec3`,
`#frag`, `chap2.xhtml`) is rewritten to `RefTarget::Internal(block_id)` via the anchor
map; when the exact fragment is unknown, it falls back to that file's first-heading
entry. An internal-shaped href with no entry at all (target file has no headings) is
left for the core's dangling-ref degradation — link text survives as plain text, with a
warning. Hrefs with a URL scheme stay `External`.

**Footnotes** (EPUB3 semantic markup only — decided against heuristics, whose
misfires would relocate non-footnote content):

1. `<a epub:type="noteref" href="...">` parses as a *pending* noteref (a placeholder
   link carrying its target `(file, frag)`).
2. `<aside epub:type="footnote" id="...">` produces `Block::Footnote` with a fresh
   sequential `NoteId`, keyed by `(file, id)`.
3. Fixup rewrites each pending noteref with a matching aside into
   `Inline::FootnoteRef(note_id)` and **relocates the `Footnote` block to immediately
   after the block containing its first reference**, so GFM `[^n]`/definition pairs
   land in the same emitted file.
4. A noteref with no matching aside falls back to the ordinary internal-link path; an
   unreferenced aside stays in place as a normal `Footnote` block.

**Accepted limitation:** the size-guard may split between a reference and its relocated
definition at a paragraph boundary, leaving `[^n]` whose definition sits in the
adjacent `part-NN.md`. Rare, non-lossy, and the fix belongs in the structuring engine.

## 5. Error handling

No new error variants. Everything inside a spine file degrades rather than aborts:

- Table row with a cell count differing from the header → padded with empty cells.
- Unclosed element at EOF → its frame is popped and folded, never discarded.
- `<li>` outside a list, `<figcaption>` outside `<figure>` → transparent; text flattens.
- Warnings use the existing CLI logging path.

## 6. Testing

- **Unit, `xhtml.rs`** (existing inline-XML style): nested lists; table with/without
  `thead`; merged-cell flag; figure + figcaption; bare img with alt; `<pre>`/`<code>`
  with language class and preserved whitespace; inline code; blockquote flattening;
  implicit paragraph; `<head>` exclusion; br-to-space.
- **Unit, fixup pass in `mod.rs`**: cross-file link → `Internal`; fragment-less href;
  unresolvable href degradation; noteref→`FootnoteRef` pairing; footnote relocation;
  orphan noteref; unreferenced aside.
- **Integration:** new fixture `tests/fixtures/epub/rich.epub` — a tiny two-chapter
  book exercising every element (list, table, image, footnote, cross-chapter link,
  code block) — run through the full pipeline in the existing integration style,
  asserting the image lands in `_assets/`, the cross-ref resolves to a real relative
  path, and the footnote renders as `[^1]` with its definition in the same file.
- **Gate:** `mise run lint && mise run test` green.
