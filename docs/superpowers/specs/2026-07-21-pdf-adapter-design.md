# PDF adapter — Design Spec

**Date:** 2026-07-21
**Status:** Approved (design), pending implementation plan
**Repo:** kasane — document-to-Markdown converter
**Predecessors:** [kasane core pipeline](2026-07-19-kasane-document-to-markdown-design.md),
EPUB, PPTX, MOBI/AZW3 adapters (all shipped).

## 1. Purpose & Scope

Add a pure-Rust `PdfAdapter` that converts a **born-digital** PDF into the same
IR every other adapter produces, wired into the existing hexagonal pipeline
(`detect → adapter → IR → structure() → write_tree`). Detection already flags
`%PDF` as `Format::Pdf`; today only `adapter_for` rejects it. This item makes it
real.

PDF has no logical structure — a page is positioned glyphs and images, not
headings and paragraphs. The defining work is *inferring* structure. The agreed
ambition for this first cut is **outline-driven headings + line grouping**:
faithful to the document's own table of contents when it has one, robust
line/paragraph reconstruction always, and honest degradation everywhere else.

### Confirmed decisions

| Decision | Choice |
|---|---|
| Structure ambition | **Outline-driven + line grouping.** PDF `/Outlines` is the heading tree when present; font-size clustering is the fallback when it is absent. Text lines grouped into paragraphs. |
| Parsing foundation | **`lopdf` + our own text-operator pass.** lopdf provides the object model, content streams, outline, and decryption; we interpret text-showing operators to recover glyph positions + font size. |
| Encryption | **Decrypt the empty-user-password case** (RC4/AES) transparently; a real user password → `ParseError::Encrypted` (exit 2). Never breaks DRM, never cracks a password. |
| Reading order | **Single-column, top-to-bottom.** Multi-column detection is an explicit non-goal. |
| Provenance | Every node carries `source_pages: (n, n)` — PDF is page-native. |

### Non-goals (this item)

- **Multi-column** layout reconstruction (interleaves as single-column order).
- **Table** reconstruction from ruling lines — tabular text falls through as paragraphs.
- **Math** recovery — PDF carries no source markup to recover LaTeX from.
- **OCR** — scanned pages degrade to an image + placeholder note; `-F ocr` is a later item.
- **`--password`** CLI flag — YAGNI until someone asks.

## 2. Module layout

New module `crates/kasane-adapters/src/pdf/`, mirroring `mobi/`:

| File | Responsibility |
|---|---|
| `mod.rs` | `PdfAdapter`; orchestrates open → decrypt → per-page extract → outline merge → `Document`. Owns `impl Adapter`. |
| `doc.rs` | Wrapper over `lopdf::Document`: open bytes, empty-password decrypt, page iteration, size/bomb guards. |
| `content.rs` | Content-stream interpreter: `BT/ET, Tf, Tm/Td/TD/T*, Tj/TJ` → flat list of positioned glyph-runs `{ x, y, font_size, text }` per page. |
| `layout.rs` | Positioned runs → lines (y-band grouping) → paragraphs (line-gap grouping) → `Block::Para` and heading candidates. Single-column reading order. |
| `outline.rs` | Parse `/Outlines` tree → destination page/position per entry → heading skeleton spliced into the block stream. |
| `image.rs` | `/XObject` subtype `/Image` → decode → `AssetBag`; emit `Block::Figure`. Reuses `safe_media_filename` + bomb guards. |

`content.rs` / `layout.rs` / `outline.rs` are pure functions over
already-extracted lopdf objects — unit-testable with synthetic inputs and no
real files, matching how `kasane-core` is tested.

## 3. Extraction pipeline (per page)

**1. Glyph runs (`content.rs`).** Interpret the content stream's text operators,
tracking the text matrix. Each `Tj`/`TJ` emits `{ x, y, font_size, text }`, where
`font_size` is the current `Tf` size scaled by the text matrix. Decode text bytes
to Unicode via the font's `/ToUnicode` CMap when present; otherwise assume
WinAnsi/Standard encoding and note reduced fidelity. No full font-metrics engine —
we need positions and sizes, not exact glyph widths.

**2. Lines (`layout.rs`).** Sort runs top-to-bottom, left-to-right; group runs
sharing a y-band (tolerance derived from font size) into a line. Concatenate runs
into line text, inserting a space when the inter-run x-gap exceeds a threshold. A
line records its dominant font size.

**3. Paragraphs.** Group consecutive lines into `Block::Para` while the vertical
gap stays near body leading; a larger gap ends the paragraph.

**4. Headings — two sources, outline wins:**

- **Primary (`outline.rs`):** when a `/Outlines` tree exists, it *is* the chapter
  structure. Each entry resolves to a destination page + y; splice a
  `Block::Heading { level }` (level = outline depth) at that position in the page's
  block stream.
- **Fallback:** with no outline, infer headings by font size — compute the modal
  body size across the document; a line exceeding it by a margin becomes a heading,
  level bucketed by how many distinct larger sizes exist (capped at ~3 → H1–H3).

**Provenance.** Every node carries `source_pages: (n, n)` for its page.

## 4. Images, scanned pages, encryption, degradation

**Embedded images (`image.rs`).** Iterate each page's `/Resources /XObject`
entries of subtype `/Image`. Decode by filter: `DCTDecode` → emit JPEG bytes
as-is; `FlateDecode` raster → wrap to PNG; unsupported filters (`JPXDecode`,
JBIG2) → skip with a `Raw` note rather than fail. Each kept image → `Block::Figure`
(empty caption — PDF has no reliable caption source), flushed through `AssetBag`
with `safe_media_filename`.

**Scanned / no-text pages.** A scanned page is typically one full-page image with
zero glyph runs. It falls out naturally: the page image is extracted as a `Figure`,
plus `Block::Raw { note: "scanned page: no text layer; OCR not enabled" }` — the
documented seam where a future `-F ocr` feature plugs in. A page with neither text
nor image → single `Raw` note, never a hard error.

**Encryption.** On open, if an `/Encrypt` dict is present, attempt standard
empty-user-password decryption (RC4/AES via lopdf). Success → convert normally. A
required real user password → `ParseError::Encrypted`.

**Exit codes.** `Encrypted` maps to exit 2. The CLI today routes only
`"unsupported"`/`"DRM"` substrings to code 2; extend that to also match
`"encrypted"`. Malformed/corrupt PDFs stay exit 1.

**Security — untrusted-input boundary (same rigor as the zip adapters):**

- **Bomb guards:** cap decoded stream and total extracted bytes against
  `MAX_TOTAL_BYTES` / `MAX_RATIO` via `check_expansion`; overflow → `ParseError::Bomb`.
- **Bounded recursion:** traverse the `/Outlines` and page trees with a depth cap
  and a visited-object set to defeat maliciously deep or cyclic references.
- **Degrade, don't die:** a corrupt page, undecodable stream, or bad object becomes
  a `Raw` note and parsing continues; only unrecoverable top-level failures (not a
  PDF, encrypted, bomb) abort.

## 5. Wiring

- `adapter_for(Format::Pdf) => Ok(Box::new(PdfAdapter))` — remove from the
  `_ => Unsupported` arm.
- `lib.rs`: `mod pdf;` + `pub use pdf::PdfAdapter;`.
- CLI: extend the exit-2 substring check to include `"encrypted"`; update the
  `--help` input blurb; move "PDF" from "coming" to supported in the README.
- `Cargo.toml` (adapters crate): add `lopdf`. As a `Cargo.toml` dependency it is
  watched by Dependabot automatically (unlike the `mise.toml` pins), so it needs
  no special manual-bump note in AGENTS.md.

## 6. Testing

Hermetic fixtures, dependency-free Python emitting **raw PDF bytes** (objects +
xref), matching the `make_minimal_mobi.py` convention — no external libraries.

- `tests/fixtures/pdf/make_*.py` producing:
  - `minimal.pdf` — two pages, `/Outlines` (H1 + nested H2), body paragraphs, one
    `DCTDecode` image.
  - `no-outline.pdf` — exercises the font-size heading fallback.
  - `scanned.pdf` — one full-page image, no text → Figure + `Raw` note.
  - `encrypted-empty.pdf` — empty-user-password RC4 → decrypts and converts.
- **Unit tests** (no files): `content.rs` operator interpreter, `layout.rs`
  line/paragraph grouping, `outline.rs` splice — on synthetic run lists.
- **Adapter tests:** outline → heading tree/levels; page-native `source_pages`;
  empty-password decrypt; real-password stub → `Encrypted`.
- **End-to-end** (mirrors the PPTX test in `lib.rs`): `detect → parse → structure
  → write_tree`, asserting `index.md` + a flushed asset.
- Green under `mise run lint && mise run test`.

## 7. Honest limitations (README + AGENTS.md)

- Multi-column layout interleaves as single-column reading order.
- Tables are not reconstructed — tabular text becomes paragraphs.
- Math is not recovered (no source markup in PDF).
- Scanned pages become an image + placeholder note until `-F ocr` lands.
- Fonts without `/ToUnicode` fall back to WinAnsi/Standard encoding; unusual
  encodings may mis-map characters.
