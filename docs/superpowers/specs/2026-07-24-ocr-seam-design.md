# kasane — OCR Seam Design Spec

**Date:** 2026-07-24
**Status:** Approved (design), pending implementation plan
**Repo:** kasane

## 1. Purpose & scope

This is **item B** of the OCR work — the seam item A was deliberately split
from. Item A (DjVu page rasterization, merged) made both the PDF and DjVu
adapters emit a rendered page image + a "OCR not enabled" note for text-less
pages. Item B turns that page image into recovered text: it introduces a
`TextExtractor` trait, wires a Tesseract implementation behind an opt-in
`-F ocr` Cargo feature, and routes the page images through OCR — uniformly
across **both** the PDF and DjVu adapters.

OCR is the one deliberate exception to kasane's "pure Rust on the default build"
promise: only `-F ocr` links a C library (Tesseract + Leptonica). The default
build is untouched — no new C dependency, identical behavior to today.

### Boundary

The seam lives entirely inside `crates/kasane-adapters/`: a new shared `ocr/`
module (trait + types + the feature-gated Tesseract impl) and OCR wiring in the
existing text-less branches of `pdf/` and `djvu/`. `kasane-ir`, `kasane-core`,
and `kasane-writer` are untouched — OCR output is ordinary `Block`s the pipeline
already handles. `kasane-cli` gains three flags and the code that constructs the
extractor. `Adapter::parse` gains one `&ParseOptions` parameter (§3).

### Non-goals

- **No new pipeline stage.** OCR text becomes the same `Block::Paragraph` /
  `Block::Heading` nodes born-digital text already produces. Core, IR, and
  writer are agnostic to whether text came from a layer or from OCR.
- **No always-on OCR.** OCR links C, is slow (seconds/page), and can be noisy.
  It requires both a build feature (`-F ocr`) and a runtime flag (`--ocr`).
- **No PDF page rasterizer and no new image-codec decoders.** PDF OCR runs only
  on pages whose raster the adapter already decodes (JPEG/Flate). CCITT G4,
  JBIG2, and JPXDecode scans stay un-OCR'd — a documented, bounded gap (§5).
- **No language auto-detection.** Language is `eng` by default, overridable
  (§6). No Tesseract OSD script guessing.

## 2. The seam: `crates/kasane-adapters/src/ocr/`

A new module. The trait and its data types compile **always** (default build
included); only the Tesseract implementation is feature-gated.

```rust
// ocr/mod.rs — always compiled, no C dependency
pub struct OcrBBox { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }

/// One recovered line of text with its page-space box and Tesseract's
/// confidence (0–100). `bbox.h` is the font-size proxy for heading inference.
pub struct OcrLine { pub text: String, pub bbox: OcrBBox, pub confidence: f32 }

pub struct OcrOptions { pub lang: String, pub min_confidence: f32 }

#[derive(Debug, thiserror::Error)]
pub enum OcrError { /* EngineInit, MissingLanguage(String), Decode(String), ... */ }

/// Behind PDF and DjVu. Default builds have no implementor; `-F ocr` adds one.
pub trait TextExtractor {
    /// OCR an encoded page image (the PNG/JPEG bytes the adapter already
    /// produced for its fallback figure). Returns lines in reading order.
    fn extract(&self, image: &[u8], opts: &OcrOptions) -> Result<Vec<OcrLine>, OcrError>;
}
```

```rust
// ocr/tesseract.rs — #[cfg(feature = "ocr")] only; the sole C-linking code
pub struct TesseractExtractor { /* leptess handle + configured lang */ }

impl TesseractExtractor {
    /// Fails with a clear `OcrError::MissingLanguage` if the traineddata pack
    /// for `lang` is not found (names the pack and where Tesseract looked).
    pub fn new(lang: &str) -> Result<Self, OcrError> { /* ... */ }
}

impl TextExtractor for TesseractExtractor {
    fn extract(&self, image: &[u8], opts: &OcrOptions) -> Result<Vec<OcrLine>, OcrError> {
        // leptess: set_image_from_mem(image) -> Leptonica decodes PNG/JPEG.
        // Read the TSV / result iterator at line granularity -> OcrLine
        // { text, bbox, confidence }, filtering blank lines.
    }
}
```

### Design rationale

- **Encoded bytes as input.** The trait takes `&[u8]` (PNG/JPEG), not a raster
  struct, because both adapters already produce encoded bytes for their fallback
  figure (DjVu `render_page_image` → PNG; PDF `extract_page_images` →
  PNG/JPEG). Leptonica decodes those directly via `set_image_from_mem`, so no
  adapter has to expose its internal raster type across the seam.
- **Engine crate: `leptess`** (links `libtesseract` + `libleptonica`). This is
  the "only component that links C" the 2026-07-19 architecture reserved.
  (`rusty-tesseract`, which shells out to the `tesseract` binary via temp files,
  was considered and rejected: it trades C *linking* for a subprocess + a
  runtime binary dependency + temp-file I/O, without removing the external
  Tesseract requirement.)
- **Guarded.** Every `extract` call is wrapped in the adapters' existing
  `catch_unwind` guard, so a decoder panic degrades to the image fallback rather
  than crashing the process — the same discipline `doc.rs` uses for `djvu-rs`.
- **`ocr/mod.rs` stays pure.** Types, trait, and (in `#[cfg(test)]`) a stub
  implementation — no C, no `leptess` — so the wiring is fully testable on the
  default build (§7).

## 3. Threading OCR into adapters

`Adapter::parse` gains one parameter carrying the (optional) extractor:

```rust
pub struct ParseOptions<'a> {
    pub ocr: Option<&'a dyn TextExtractor>,
    pub ocr_opts: OcrOptions,          // ignored when `ocr` is None
}
impl Default for ParseOptions<'_> { /* ocr: None, eng, default threshold */ }

pub trait Adapter {
    fn parse(&self, bytes: &[u8], source_path: &str, opts: &ParseOptions)
        -> Result<(Document, AssetBag), ParseError>;
}
```

- `ParseOptions::default()` = **no OCR**. EPUB, PPTX, and MOBI ignore `opts`
  entirely; every existing call site and test passes `&ParseOptions::default()`.
- Only PDF and DjVu read `opts.ocr`, and only in their existing text-less
  branch. When it is `None`, both adapters behave exactly as they do today.

This is a signature change across the `Adapter` trait — mechanical and contained
(add one ignored parameter to five adapters; thread it from the CLI). It keeps
OCR configuration flowing through one typed channel rather than a global or a
per-adapter setter.

## 4. Data flow — OCR in the text-less branch

Both adapters already have the branch: a page with no recoverable text where
item A emits `page image + note`. OCR slots into that branch. When the page is
text-less **and** `opts.ocr` is `Some`:

1. Obtain the page raster the adapter would have emitted as its fallback image
   (the encoded PNG/JPEG bytes).
2. `let lines = opts.ocr.unwrap().extract(bytes, &opts.ocr_opts)` — guarded;
   an `Err` or panic is treated as "no text" (step 5 fallback).
3. Map each `OcrLine` into the adapter's **own** line type and run the adapter's
   **existing** block/heading builder — no new structuring code:
   - **DjVu** → `text::Line { text, height: bbox.h }`, then `text::page_blocks`
     with a modal body height computed over the OCR lines. Tall lines become
     inferred headings exactly as text-layer lines do.
   - **PDF** → `layout::Line { x: bbox.x, y: bbox.y, size: bbox.h, text }`, then
     `layout::page_blocks_no_headings` with `modal_body_size` over the OCR
     lines. (OCR already returns grouped lines, so `group_lines` is skipped.)
   Reading order and multi-column handling come from Tesseract's own line order.
4. **Confidence gate (Decision A).** If the recovered text clears the
   threshold — mean line confidence ≥ `min_confidence` **and** a minimum total
   character count — emit those blocks and **drop** the page image. The page
   then looks like any born-digital page.
5. **Fallback.** If OCR errored, panicked, or fell below the gate, keep today's
   behavior: the page image plus a note recording that text was not recovered
   (e.g. "page image only; OCR found no recoverable text"), preserving the
   existing no-layer vs empty-layer distinction where the adapter tracks it.
6. **`--ocr-no-image` (Decision C as a flag).** When set, step 4 emits the OCR
   text unconditionally (even below the gate) and never falls back to an image —
   smallest output, user accepts the risk of a poor page. A hard OCR *error*
   (not merely low confidence) still degrades to the note.

Pages that already recovered text, or that carry no decodable raster, are
untouched — no OCR attempt, exactly as today.

## 5. PDF scanned-page coverage (bounded — B1)

DjVu has a real page rasterizer (`djvu-rs`), so **every** text-less DjVu page
has an image to OCR. **PDF does not** — `pdf/image.rs` extracts image XObjects
and only decodes `FlateDecode` and `DCTDecode`; `CCITTFax`, `JBIG2`, and
`JPXDecode` are reported in `skipped`, not decoded. The most common bilevel scan
encodings (CCITT G4, JBIG2) therefore have **no raster to OCR**.

This item takes the **bounded** approach:

- PDF OCR runs only on text-less pages that produced a decoded raster
  (`imgs.had_image` — a JPEG or Flate image). Those pages route through §4.
- CCITT/JBIG2/JPX scans remain as today: a `skipped`/note entry, now documented
  as "not OCR'd — image encoding not decoded in this build." No new image-codec
  decoders and no full-page PDF rasterizer enter item B.

Broadening PDF raster coverage — most cheaply by adding pure-Rust CCITT G4
decoding (the `fax` crate), which would both feed OCR and close the "noted but
not extracted" gap for those pages — is deferred to its own fast-follow item.
A full PDF page rasterizer (which would need a C renderer or an immature
pure-Rust one) is explicitly out of scope.

## 6. CLI, features, and build

### Flags (always parse)

- `--ocr` — turn OCR on. On a build **without** `-F ocr`, this prints a clear
  error and exits **2** ("OCR requested but this build lacks the `ocr` feature;
  rebuild with `-F ocr`"). Both PDF and DjVu conversions honor it.
- `--ocr-lang <LANG>` — Tesseract language(s), default `eng`; `eng+deu` style
  multi-language strings pass through. A missing traineddata pack is a hard,
  fail-fast error (before parsing) naming the pack and `TESSDATA_PREFIX`.
- `--ocr-no-image` — emit OCR text unconditionally, never the fallback image
  (§4 step 6).

Flags are defined unconditionally so `kasane --help` and argument parsing are
identical across builds; the feature check happens in `run()`.

### Features

- `kasane-adapters`: `[features] ocr = ["dep:leptess"]`, with
  `leptess = { version = "...", optional = true }`.
- `kasane-cli`: `[features] ocr = ["kasane-adapters/ocr"]`.
- Default build pulls in neither; the pure-Rust guarantee holds.

### Build / CI

- `mise run test` and `mise run lint` stay pure-Rust and unchanged.
- Add an `ocr` mise task and a CI job that provision Tesseract + Leptonica +
  the `eng` traineddata (via mise `[tools]` / devenv) and run the `-F ocr`
  tests. The AGENTS.md caveat applies: mise pins are manual bumps Dependabot
  cannot see, so the Tesseract/Leptonica versions there are hand-maintained.

## 7. Testing

- **`StubExtractor`** (`#[cfg(test)]`, returns canned `OcrLine`s, no C) makes the
  entire seam and both adapters' wiring testable on the **default build** — no
  Tesseract required for the wiring tests:
  - DjVu text-less page + stub → text blocks; a tall `OcrLine` becomes an
    inferred heading; **no** figure.
  - DjVu low-confidence stub → figure + fallback note (image kept).
  - `--ocr-no-image` path → text emitted even below the confidence gate.
  - PDF decoded-raster text-less page + stub → text blocks, figure dropped.
  - PDF CCITT/JBIG2 page + stub → **unchanged** (no raster, no OCR) — a
    regression pin for the §5 boundary.
  - Injected panic in `extract` → page falls back to image + note, and the rest
    of the document still converts.
- **CLI:** `--ocr` on a non-`ocr` build → clear error, exit 2; flag parsing
  identical across builds.
- **Tesseract impl** (`#[cfg(feature = "ocr")]`, OCR CI job only): a tiny
  fixture image of rendered text OCRs to approximately the expected string;
  `TesseractExtractor::new("zzz")` returns a clear `MissingLanguage` error.
- All green under `mise run lint && mise run test` (clippy `--all-targets` +
  `fmt --check`, per the repo lint gate).

## 8. Docs

- **README** — replace the PDF and DjVu "OCR not enabled / `-F ocr` roadmap"
  caveats with: OCR is available in `-F ocr` builds via `--ocr`
  (`--ocr-lang`, `--ocr-no-image`); text-less pages recover text when OCR is
  confident, else keep the page image. State the PDF §5 bound plainly
  (CCITT/JBIG2/JPX scans are not OCR'd). Note the Tesseract/Leptonica build
  requirement for the feature.
- **AGENTS.md** — add the `ocr/` module to the adapters map (trait + types
  always compiled; Tesseract impl behind `-F ocr`, the sole C dependency); note
  `Adapter::parse` now takes `&ParseOptions`.

## 9. File-change summary

- `crates/kasane-adapters/src/ocr/mod.rs` — new; trait, `OcrLine`/`OcrBBox`/
  `OcrOptions`/`OcrError`, and a `#[cfg(test)]` `StubExtractor`.
- `crates/kasane-adapters/src/ocr/tesseract.rs` — new; `#[cfg(feature = "ocr")]`
  `TesseractExtractor` over `leptess`.
- `crates/kasane-adapters/src/lib.rs` — `ParseOptions`; `Adapter::parse` gains
  the parameter; re-export the `ocr` module surface.
- `crates/kasane-adapters/src/djvu/mod.rs` — text-less branch calls OCR, maps
  `OcrLine → text::Line`, gates on confidence, drops image on success.
- `crates/kasane-adapters/src/pdf/mod.rs` — text-less (decoded-raster) branch
  calls OCR, maps `OcrLine → layout::Line`, gates, drops figure on success.
- `crates/kasane-adapters/src/{epub,pptx,mobi}/mod.rs` — accept and ignore the
  new `&ParseOptions` parameter.
- `crates/kasane-adapters/Cargo.toml` — `ocr` feature + optional `leptess`.
- `crates/kasane-cli/Cargo.toml` — `ocr` feature forwarding.
- `crates/kasane-cli/src/main.rs` — `--ocr` / `--ocr-lang` / `--ocr-no-image`;
  construct the extractor under `-F ocr`; clear error otherwise; build and pass
  `ParseOptions`.
- `mise.toml` — `ocr` task + Tesseract/Leptonica/eng provisioning for the OCR
  job.
- `README.md`, `AGENTS.md` — doc updates above.
