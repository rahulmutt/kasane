# kasane — PPTX Adapter Design Spec

**Date:** 2026-07-19
**Status:** Approved (design), pending implementation plan
**Repo:** kasane — document → progressive-disclosure Markdown
**Depends on:** the shipped core pipeline (`2026-07-19-kasane-document-to-markdown-design.md`, `2026-07-19-kasane-core-pipeline.md`)

## 1. Purpose & scope

Add the **PPTX** input adapter, the second format after EPUB. Each slide becomes one
`Heading{level:1}` section; text frames become paragraphs and nested lists; media
becomes figures; speaker notes are appended per slide. The adapter plugs into the
existing hexagonal pipeline — `detect()` → `adapter_for()` → `parse()` → `structure()`
→ `write_tree()` — so the core engine, writer, and CLI are essentially untouched.

This is the first adapter to exercise the writer's `_assets/` flush path: v1 extracts
slide images into the `AssetBag` (the shipped EPUB adapter returns an empty bag).

### In scope (v1)

- Correct slide **display order** (via `presentation.xml`, not filename numbers).
- Title placeholder → slide heading (`Slide N` fallback).
- Paragraphs, runs, bold/italic, bullet lists with **nesting** by `<a:pPr lvl>`.
- **Images** → `Figure` + `AssetBag` entries.
- **Tables** (`<a:tbl>`) → IR `Table` (merged cells → writer's HTML fallback).
- **Hyperlinks** (`<a:hlinkClick>`) → external links.
- **Speaker notes** appended per slide.

### Non-goals (v1)

- Cross-slide internal links (degrade to plain text; no `RefTarget::Internal` mapping).
- Slide masters / layout inheritance, themes, animations, transitions, charts, SmartArt
  (charts/SmartArt degrade to a `Raw` note where encountered).
- Math (PPTX OMML → LaTeX) — deferred.

## 2. Approach

Three parser strategies were considered:

- **A — Streaming `quick-xml` state machine (chosen).** One event-loop pass per slide
  with a small context stack — the exact idiom `epub/xhtml.rs` already uses. No new
  dependencies; the security boundary stays ours; DrawingML text nesting is shallow and
  regular (`txBody > a:p > a:r > a:t`, tables one level deeper).
- **B — DOM parse (`roxmltree`/`minidom`).** Cleaner for nested tables but adds a
  dependency and cuts against the "pure Rust, minimal deps" grain. Rejected.
- **C — Existing PPTX crate.** No mature pure-Rust option; would bypass our
  bomb/traversal guards. Rejected by the untrusted-boundary principle.

## 3. Module layout

```
crates/kasane-adapters/src/
  ziputil.rs        NEW: shared bomb-guarded zip read + aggregate byte counter
  guard.rs          + resolve_rel(base_dir, target) — normalize `..`, confine to root
  pptx/
    mod.rs          PptxAdapter: zip → slide order → per-slide parse → media → notes
    rels.rs         parse presentation.xml <sldIdLst> and *.rels (rId → target)
    slide.rs        DrawingML slide XML → Vec<Block> (the state machine)
  lib.rs            adapter_for(Format::Pptx) => PptxAdapter
```

### Refactor: shared zip-read helper

`read_entry` (bomb-guarded read + shared `total_read` aggregate counter) is currently
private in `epub/mod.rs`. PPTX needs it identically, so it moves to `ziputil.rs` and the
EPUB adapter calls the shared version. No behavior change — pure de-duplication, in the
spirit of improving code we're working in.

## 4. Slide ordering & relationship resolution

Slide **filename numbers are not display order**; PowerPoint stores order in
`presentation.xml`. The adapter follows the same relationship chain the format uses:

1. `ppt/presentation.xml` → `<p:sldIdLst>` yields slide `r:id`s in display order.
2. `ppt/_rels/presentation.xml.rels` resolves each `r:id` → `ppt/slides/slideN.xml`.
3. Per slide, `ppt/slides/_rels/slideN.xml.rels` resolves:
   - image `r:embed` → `ppt/media/*`,
   - hyperlink `r:id` (`TargetMode="External"`) → URL,
   - the notes rel (`.../notesSlide`) → `ppt/notesSlides/notesSlideN.xml`.

This mirrors how the EPUB adapter follows the OPF spine.

## 5. DrawingML → IR mapping

| Slide element | IR |
|---|---|
| Each slide | `Heading{level:1}` from the **title placeholder** (`<p:ph type="title"/ctrTitle"/>`); fallback `Slide N` |
| `<a:p>` / `<a:r>` / `<a:t>` | `Para` of `Text` runs |
| `<a:rPr b="1">` / `i="1"` | `Strong` / `Emph` |
| consecutive `<a:p>` with `<a:pPr lvl="N">` | nested `List{ordered:false}` (lvl → depth) |
| `<p:pic>` → `<a:blip r:embed>` | `Figure` + `AssetBag` entry; caption from `<p:cNvPr descr>` |
| `<a:graphicFrame>` → `<a:tbl>` | `Table` (row 1 → header; merged cells → `has_merged`) |
| run with `<a:hlinkClick r:id>` | `Inline::Link{ External(url) }` (cross-slide → plain text) |
| notes slide text | appended after slide body under a `**Notes**` `Para` lead-in |

Every slide's blocks carry `Provenance{ source_href: Some("ppt/slides/slideN.xml") }`.
Heading `BlockId`s use the same running counter pattern as the EPUB adapter.

## 6. Security (untrusted input boundary)

- All entry reads go through the shared bomb-guarded helper: **200:1** max expansion
  ratio and **512 MiB** absolute aggregate cap across the whole archive.
- **New guard — `resolve_rel(base_dir, target)`:** rels targets legitimately contain
  `../media/…`, which `safe_entry_name` rejects outright. `resolve_rel` normalizes `..`
  against the base directory and **confines the result to the archive root**, rejecting
  any escape. This is the one genuinely new guard PPTX requires.
- Media filenames are sanitized before landing in `_assets/`.
- `quick-xml` is used with no entity/DTD expansion (same as EPUB) → XXE-safe.

## 7. Degrade, don't die

- A slide that fails to parse still emits its `Heading` (title or `Slide N`) plus a
  `Raw{ note: "unparsable slide" }`.
- A missing or broken rel drops that single image/link, never the slide.
- A missing title falls back to `Slide N`.
- Unsupported shapes (charts, SmartArt) become a `Raw` note where encountered.
- The presentation is never aborted on one bad slide.

## 8. Testing

- **Unit (`slide.rs`):** DrawingML snippets → blocks — title→H1, bold run→`Strong`,
  `lvl` bullets→nested list, `a:tbl`→`Table`, `hlinkClick`→`Link`.
- **Unit (`rels.rs`):** `sldIdLst` reorders slides out of filename order; target
  resolution plus `..` confinement (escape rejected).
- **Golden:** a tiny hand-built `tests/fixtures/pptx/minimal.pptx` (2 slides, 1 image,
  1 table, notes) → assert the resulting `Document`, mirroring the checked-in
  `minimal.epub`.
- **End-to-end:** `kasane minimal.pptx -o out` produces the tree (works the moment
  `adapter_for` handles `Pptx`).

## 9. CLI

One dispatch arm — `Format::Pptx => Ok(Box::new(PptxAdapter))` — plus updating the input
doc-comment / help text from "EPUB supported in this build" to "EPUB, PPTX". No other CLI
change: `detect()` already recognizes PPTX and `write_tree` already flushes the
`AssetBag`.

## 10. Crate dependency direction (unchanged)

The adapter lives entirely in `kasane-adapters`, which already depends only on
`kasane-ir`. No new crate dependencies; no change to the hexagonal boundaries.
