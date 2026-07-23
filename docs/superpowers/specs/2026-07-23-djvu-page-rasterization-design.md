# kasane — DjVu Page Rasterization Design Spec

**Date:** 2026-07-23
**Status:** Approved (design), pending implementation plan
**Repo:** kasane

## 1. Purpose & scope

Give text-less DjVu pages a rendered page image instead of only a placeholder
note, fixing the README limitation "Scanned page images (JB2/IW44) are not
rendered in this build."

This is **item A** of the OCR work, deliberately split from the OCR seam
(**item B**, `-F ocr` / `TextExtractor` / Tesseract). Item A is a default-build
feature with no OCR in it. It produces exactly the page-image set that item B
will later hand to an OCR engine, so building it first makes the eventual OCR
spec uniform across the PDF and DjVu adapters (both will already emit page
images).

### Boundary

Everything stays inside `crates/kasane-adapters/src/djvu/`. IR, core, writer,
and CLI are untouched — a page image is a `Block::Figure` + `AssetItem`, which
the pipeline already handles for the PDF adapter. Adding this format capability
does not cross a hexagonal boundary.

### Non-goals

- No OCR. Text fidelity is unchanged; this only adds images.
- No images on pages that already recovered text. Only text-less pages
  (the pages that today emit `NO_TEXT_NOTE` / `EMPTY_TEXT_NOTE`) get an image.
- No new CLI flag. Behavior is automatic and scoped to the text-less set, so
  output stays small for text-bearing books.

## 2. New module: `djvu/image.rs`

One new file, mirroring `pdf/image.rs`'s shape. Public surface:

```rust
/// Render one page to a PNG asset, appending it to `assets`, and return the
/// resulting Figure block. None if the page has nothing renderable.
pub fn render_page_image(doc: &DjvuDoc, page: u32, assets: &mut AssetBag) -> Option<Block>
```

### Seam discipline

All `djvu-rs` calls go **through `doc.rs`**, keeping `doc.rs` the sole seam over
the crate (AGENTS.md convention: "the sole seam over the crate"). `doc.rs` grows
two small port functions:

- `page_mask(doc, page) -> Option<Bitmap>` — the page's bilevel JB2 mask.
- `page_pixmap(doc, page, max_w, max_h) -> Option<Pixmap>` — a full RGBA render,
  fit within a pixel box.

Each is wrapped in the existing `run_guarded` (`catch_unwind` +
`AssertUnwindSafe`) so a decoder panic degrades to `None`, never a process
crash. `Bitmap` and `Pixmap` are re-exported from `djvu-rs`; they are simple
`{ width, height, data }` structs and are the only `djvu-rs` types `image.rs`
sees.

`image.rs` itself is pure: bitmap/pixmap → PNG bytes via the `png` crate (already
a dependency), no `djvu-rs` calls of its own.

### Render policy (mask-first, fall back to full)

1. Try `page_mask`. On success, encode as a **1-bit grayscale PNG**
   (`png::BitDepth::One`, `ColorType::Grayscale`). The packed, row-major
   `Bitmap.data` (stride = `width.div_ceil(8)`, bit 1 = black) maps onto PNG
   scanlines directly; PNG's convention is 0 = black, so bits are inverted
   during encoding (or the PLTE/`Grayscale` sense is chosen accordingly). This
   is the common case for a text scan and is tiny on disk.
2. If there is no mask (a photographic, IW44-background-only page), fall back to
   `page_pixmap` → **8-bit RGB PNG** (drop the alpha channel; DjVu pages are
   opaque).
3. If both yield nothing, return `None`.

The asset key/filename follow the PDF convention: `djvu-page-{n}` →
`djvu-page-{n}.png`. The `Figure` has an empty caption and no number, matching
`pdf/image.rs`.

## 3. Pixel cap & the untrusted-input boundary

Rendering is new attack surface. A hostile file can declare enormous page
dimensions, and decoded pixels are **not** bounded by the input-size guard
(`MAX_TOTAL_BYTES`, 512 MB) — a small file can declare a huge page.

- New `const MAX_RENDER_PIXELS` in `doc.rs`, ~25 MP (≈ a 300-dpi tabloid page).
- `page_pixmap` builds `RenderOptions::fit_to_box(page, max_w, max_h)` so the
  **decoder never allocates beyond the cap** — pages under it render at native
  resolution, pages over it are downscaled by `djvu-rs` during rendering.
- `page_mask` checks the declared `width * height` (with `checked_mul`) against
  the cap **before** decoding; if over, it downscales the resulting bitmap by an
  integer factor (nearest-neighbor / any-black-in-block, since it is already
  1-bit) so the emitted PNG stays within budget.
- `djvu-rs`'s own internal 64 MP `Pixmap` backstop (returns an empty pixmap
  rather than OOM) stays as defense-in-depth.
- The rendered PNG's byte length counts against the existing cumulative
  `MAX_TOTAL_BYTES` guard in `mod.rs`, alongside recovered text bytes, so a
  document of many large pages cannot blow the total budget.

## 4. Wiring into `mod.rs`

Only the text-less branch of `page_nodes_from_lines` changes — the branch where
`headings.is_empty() && !had_blocks`, which today pushes a single `Raw` note
(`NO_TEXT_NOTE` or `EMPTY_TEXT_NOTE`).

New behavior for that branch:

1. Render the page image (see below on wiring).
2. **On success:** push the `Figure`, then a **trimmed** `Raw` note that still
   records text was not recovered, preserving the existing no-layer vs
   empty-layer distinction:
   - no text layer → `"page image only; no text layer, OCR not enabled"`
   - empty text layer → `"page image only; text layer present but empty"`
3. **On `None`** (render also failed): push the current full note unchanged
   (`NO_TEXT_NOTE` / `EMPTY_TEXT_NOTE`).

Pages that recovered text or carry an outline heading are untouched: no image,
exactly as today.

### Keeping `page_nodes_from_lines` pure

`page_nodes_from_lines` is currently a pure function (no doc handle), and its
unit tests are fixture-free. To preserve that, `parse` renders the image in the
per-page loop (where it already holds `&djvu`) and passes the resulting
`Option<Block>` **into** `page_nodes_from_lines`, rather than threading the doc
handle down. The signature gains one `page_image: Option<Block>` parameter; the
test-facing `page_nodes` wrapper passes `None`.

## 5. Testing

- **Unit (`image.rs`), mask path:** the committed `sample.djvu` fixture has a
  real JB2 mask → assert `render_page_image` returns a `Figure` whose asset is a
  valid, decodable 1-bit PNG of the page's dimensions.
- **Unit, chosen-path pin:** extend `make_djvu_fixture.rs` with a **no-text-layer**
  fixture variant so the mask path is the *selected* path (not merely present
  alongside text), and assert the text-less page produces an image.
- **Cap:** a unit test with a synthetic oversized page (declared dimensions past
  `MAX_RENDER_PIXELS`) asserts the emitted image is downscaled to ≤ the cap and
  does not OOM.
- **`mod.rs` integration:** a text-less page yields `Figure` + trimmed note (not
  the bare note); a text-bearing page yields **no** figure — a regression pin
  that images never leak onto text pages.
- **Guard:** a panic injected through `run_guarded` returns `None`, and
  conversion of the rest of the document continues.
- All green under `mise run lint && mise run test` (clippy `--all-targets` +
  `fmt --check`, per the repo lint gate).

## 6. Docs

- **README** DjVu limitations: remove "Scanned page images (JB2/IW44) are not
  rendered in this build." Replace with: text-less pages now emit the rendered
  page image (bilevel JB2 mask, or full IW44 render when there is no mask) plus
  a marker that the text is un-OCR'd. Keep the OCR caveat.
- **AGENTS.md** `djvu/` map line: mention `image.rs` and that image layers are
  now rasterized to a page image **for text-less pages** (previously "image
  layers are intentionally not decoded"), while text-bearing pages remain
  text-only.

## 7. File-change summary

- `crates/kasane-adapters/src/djvu/image.rs` — new; render + PNG encode.
- `crates/kasane-adapters/src/djvu/doc.rs` — new port fns `page_mask`,
  `page_pixmap`, `const MAX_RENDER_PIXELS`.
- `crates/kasane-adapters/src/djvu/mod.rs` — text-less branch emits figure +
  trimmed note; `parse` renders per page; `page_nodes_from_lines` gains an
  `Option<Block>` param.
- `crates/kasane-adapters/examples/make_djvu_fixture.rs` — add a no-text-layer
  fixture variant.
- `README.md`, `AGENTS.md` — doc updates above.
