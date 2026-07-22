# DjVu adapter — Design Spec

**Date:** 2026-07-22
**Status:** Approved (design), pending implementation plan
**Repo:** kasane — document-to-Markdown converter
**Predecessors:** [kasane core pipeline](2026-07-19-kasane-document-to-markdown-design.md),
EPUB, PPTX, MOBI/AZW3, PDF adapters (all shipped). Closest precedent:
[PDF adapter](2026-07-21-pdf-adapter-design.md).

## 1. Purpose & Scope

Add a pure-Rust `DjvuAdapter` that converts a **bundled** DjVu document into the
same IR every other adapter produces, wired into the existing hexagonal pipeline
(`detect → adapter → IR → structure() → write_tree`). Detection already flags the
`AT&T…` magic (and the `.djvu`/`.djv` extension hints) as `Format::Djvu`; today
only `adapter_for` rejects it. This item makes it real.

DjVu is a scanned-document image format, but — unlike a scanned PDF — a DjVu
usually carries a **hidden OCR text layer** (a zone hierarchy of
page/column/region/paragraph/line/word, each with a bounding box) and often a
**NAVM outline** (bookmarks). That is where the recoverable *meaning* lives. So
this adapter maps cleanly onto the PDF adapter's "outline-driven + line grouping"
pattern, with the text-layer zones standing in for PDF's glyph runs. We recover
genuine structured text and a heading tree **without running OCR ourselves**.

### Confirmed decisions

| Decision | Choice |
|---|---|
| Parsing foundation | **`djvu-rs` (MIT, pure-Rust, pinned), minimal surface.** Use it for the IFF container, page enumeration, the text layer (`TXTa`/`TXTz`), and the NAVM outline. The JB2 (bilevel mask) and IW44 (wavelet background) image decoders are **not used** in this cut — they are the youngest, heaviest, most bomb/panic-exposed code paths in a brand-new crate, and page rasters are pictures of pages, not content. |
| Structure ambition | **Outline-driven + line grouping.** The NAVM outline *is* the heading tree when present. When absent, infer headings by **line-box height** (the OCR line bbox height is a proxy for font size), mirroring the PDF adapter's font-size fallback. |
| Reading order | **Follow the text-layer zone hierarchy** (page → column → region → paragraph → line). This respects **multi-column** layout natively, because the OCR engine already resolved column order — a step up from PDF's single-column assumption. |
| Paragraphs | Paragraph zones map directly to `Block::Para`; no gap-based reconstruction guesswork. |
| Encryption | **None.** DjVu has no mainstream encryption/DRM concept, so there is no detect-or-reject path. |
| Provenance | Every node carries page-native `source_pages: (n, n)`. |

### Non-goals (this item)

- **Page-image rendering** (JB2 mask + IW44 background → raster). Deferred to a
  later `-F` feature; a no-text page degrades to a placeholder note (§4).
- **OCR** of no-text pages — the text layer *is* pre-computed OCR; running OCR
  where there is none is the same later `-F` seam as scanned PDF.
- **Indirect (multi-file) DjVu** documents — bundled documents only (§4).
- **Tables** — tabular text falls through as paragraphs.
- **Math** — DjVu carries no source markup to recover LaTeX from.
- **Annotations / hyperlinks** (`ANTa`/`ANTz`) beyond the NAVM outline.

## 2. Module layout

New module `crates/kasane-adapters/src/djvu/`, mirroring `pdf/` but simpler —
there is no content-stream interpreter, because `djvu-rs` hands us structured
text zones directly:

| File | Responsibility |
|---|---|
| `mod.rs` | `DjvuAdapter`; orchestrates open → per-page (text zones → blocks) → outline merge → `Document`. Owns `impl Adapter`. |
| `doc.rs` | Seam over `djvu-rs`: open bytes, bundled-vs-indirect check, page enumeration, size/bomb guards, and **`catch_unwind`** around crate calls (a young crate may panic on malformed input; contain it and degrade to `ParseError::Malformed`). |
| `text.rs` | Text-layer zone hierarchy → lines (each with dominant bbox height) → paragraphs (`Block::Para`) + heading candidates. Owns the **line-height heading inference** for the no-outline case (analog of `pdf/layout.rs`). |
| `outline.rs` | NAVM bookmarks → per-page heading skeleton (level = outline depth), spliced into the page's block stream (mirrors `pdf/outline.rs`). |

`text.rs` and `outline.rs` are **pure functions over already-extracted `djvu-rs`
objects** (`TextZone` trees, `DjVuBookmark` trees) — unit-testable with synthetic
inputs and no real files, exactly how `kasane-core` and the PDF pure modules are
tested.

## 3. Extraction pipeline (per page)

**1. Text zones (`doc.rs` → `text.rs`).** Pull the page's `TextLayer` and walk
its zone hierarchy in document order. The hierarchy already encodes columns and
regions, so honoring its order yields correct multi-column reading order for free.

**2. Lines & paragraphs.** Line zones become line text plus a dominant bbox
height; consecutive lines within a paragraph zone become one `Block::Para`.
Column/region grouping is preserved from the hierarchy rather than re-derived
geometrically.

**3. Headings — two sources, outline wins.**

- **Primary (`outline.rs`):** when a NAVM outline exists, it *is* the chapter
  structure. Each bookmark resolves to a destination page; splice a
  `Block::Heading { level }` (level = outline depth) at that page's position in
  the block stream.
- **Fallback (`text.rs`):** with no outline, infer headings by line-box height —
  compute the modal body line-height across the document; a line taller by a
  margin becomes a heading, level bucketed by how many distinct larger heights
  exist (capped at ~3 → H1–H3). Suppress inference when an outline is present
  (same `has_outline` gate as the PDF adapter).

**4. No-text page.** A page with zero text zones emits a single
`Block::Raw { note: "no text layer; OCR not enabled" }` — the documented seam
where a future `-F` rendering/OCR feature plugs in. Never a hard error.

**Provenance.** Every node carries `source_pages: (n, n)` for its page.

## 4. Degradation, security, edge cases

**Indirect (multi-file) DjVu.** A DjVu document may be *bundled* (all pages in
one file) or *indirect* (a small stub file referencing external per-page files).
The adapter receives only one file's bytes, so an indirect stub cannot be
resolved → `ParseError::Malformed("indirect multi-file DjVu not supported; \
provide the bundled document")`, which maps to **exit 1**. (Exit 2 is reserved
for whole-category refusals — DRM/encrypted — and DjVu has none; indirect is a
structural limitation better surfaced with a descriptive message.)

**No encryption/DRM path.** DjVu has no mainstream encryption, so there is
nothing to detect or reject — simpler than the PDF adapter.

**Untrusted-input boundary (same rigor as the zip and PDF adapters).**

- **Bomb guards:** the BZZ-compressed chunks (`TXTz`, `NAVM`) can expand
  substantially; cap decoded chunk size and total extracted bytes against
  `MAX_TOTAL_BYTES` / `MAX_RATIO` via `guard.rs::check_expansion`; overflow →
  `ParseError::Bomb`.
- **Bounded recursion:** traverse the NAVM outline tree and the text-layer zone
  hierarchy with a depth cap and a node budget to defeat maliciously deep or
  cyclic structures.
- **Panic containment:** wrap `djvu-rs` calls at the `doc.rs` seam in
  `catch_unwind`; a panic on malformed input becomes `ParseError::Malformed`, not
  a process crash.
- **Degrade, don't die:** a corrupt page or zone becomes a `Raw` note and parsing
  continues; only unrecoverable top-level failures (not a DjVu, indirect, bomb)
  abort.

## 5. Wiring

- `adapter_for(Format::Djvu) => Ok(Box::new(DjvuAdapter))` — remove from the
  `Unsupported` arm.
- `lib.rs`: `mod djvu;` + `pub use djvu::DjvuAdapter;`.
- `Cargo.toml` (adapters crate): add `djvu-rs` (and `djvu-bzz` if it is not
  re-exported), version-pinned. As `Cargo.toml` dependencies they are watched by
  Dependabot automatically (unlike the `mise.toml` pins), so no manual-bump note
  is needed in AGENTS.md.
- CLI: no new exit code (no encryption path); update the `--help` input blurb.
- README: move "DjVu" from "coming" to supported; add the honest limitations
  (§7). AGENTS.md: add `djvu/` to the codebase map.

## 6. Testing

Most coverage lives in **unit tests**, because the structuring modules are pure
functions over `djvu-rs` objects — no real files needed:

- **Unit (`text.rs`):** zone → line → paragraph grouping and line-height heading
  inference, on synthetic `TextZone` trees (multi-column ordering; body vs.
  heading height buckets; no-text page → `Raw` note).
- **Unit (`outline.rs`):** NAVM → heading splice and depth→level mapping, on
  synthetic `DjVuBookmark` trees (including a deep/cyclic tree hitting the
  recursion cap).

**Hermetic end-to-end fixture.** A dependency-free Python generator emits a
minimal bundled DjVu using **uncompressed `TXTa` text chunks** (so no BZZ has to
be produced by hand) plus the required `INFO`/form chunks — matching the
`make_*.py` convention used by the MOBI and PDF fixtures. It drives
`detect → parse → structure → write_tree`, asserting `index.md` exists with the
expected headings/paragraphs.

**Outline end-to-end (the one convention break).** NAVM is *always* BZZ-compressed,
so an outline end-to-end fixture cannot be hand-emitted trivially. Outline
parsing is therefore covered at the **unit level** (synthetic `DjVuBookmark`
trees). If an outline end-to-end assertion is later wanted, commit **one tiny
real `.djvu`** generated out-of-band and documented, rather than hand-rolling BZZ
in Python. This is the single place DjVu forces a small break from the
pure-hermetic-generator convention; it is called out explicitly rather than
hidden.

**Plan-time verification.** Confirm `djvu-rs` actually parses uncompressed `TXTa`
chunks (not only `TXTz`) and exposes bbox coordinates on `TextZone`; if not, fall
back to a committed real `.djvu` for the end-to-end text fixture too.

Green under `mise run lint && mise run test`.

## 7. Honest limitations (README + AGENTS.md)

- Page images (the scanned raster) are not rendered; a page with no text layer
  becomes a placeholder note until a later `-F` rendering/OCR feature lands.
- Only bundled DjVu documents are supported; indirect multi-file documents are
  rejected with a clear message.
- Text fidelity equals the quality of the file's embedded OCR text layer; a DjVu
  with no text layer yields only per-page notes.
- Tables become paragraphs; DjVu carries no math markup to recover.
