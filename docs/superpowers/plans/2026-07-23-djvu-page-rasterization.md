# DjVu Page Rasterization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render a page image for text-less DjVu pages (currently only a placeholder note), emitted as a `Block::Figure` + `AssetItem`.

**Architecture:** A new `djvu/image.rs` module renders one page to a PNG, mask-first (1-bit JB2 mask → falls back to full RGBA render). New port functions in `djvu/doc.rs` (the sole `djvu-rs` seam) expose page dimensions, the bilevel mask, and a sized RGBA render, each panic-guarded. `djvu/mod.rs` invokes the renderer only for pages that recovered no text, then emits the figure plus a trimmed note.

**Tech Stack:** Rust, `djvu-rs` 0.27 (`Bitmap`, `Pixmap`, `Page::decode_mask`/`render_to_size`), `png` 0.17 (already a dependency).

## Global Constraints

- Every change ships green under `mise run lint && mise run test`. Lint = `cargo fmt --check` + `cargo clippy --all-targets -D warnings`.
- Adapters must never trust input: all `djvu-rs` calls go through `djvu/doc.rs`, wrapped in the existing `guard_panic` (`catch_unwind`). No other module references `djvu_rs`.
- Decoded-pixel budget: `MAX_RENDER_PIXELS = 25_000_000`. Pages above it are downscaled, never rejected.
- Rendered PNG bytes are page images for **text-less pages only** — pages that recovered text or carry an outline heading get no image.
- Asset naming mirrors the PDF adapter: key `djvu-page-{n}`, filename `djvu-page-{n}.png`.

---

### Task 1: `doc.rs` render ports

**Files:**
- Modify: `crates/kasane-adapters/src/djvu/doc.rs`

**Interfaces:**
- Consumes: existing `DjvuDoc` (holds `inner: djvu_rs::DjVuDocument`), existing private `guard_panic<T>(impl FnOnce() -> Result<T, ParseError>) -> Result<T, ParseError>`.
- Produces:
  - `pub(crate) const MAX_RENDER_PIXELS: u64 = 25_000_000;`
  - `pub use djvu_rs::{Bitmap, Pixmap};` (re-export so `image.rs` never names `djvu_rs`)
  - `pub fn page_dims(doc: &DjvuDoc, page: u32) -> Option<(u32, u32)>`
  - `pub fn page_mask(doc: &DjvuDoc, page: u32) -> Option<Bitmap>`
  - `pub fn page_pixmap(doc: &DjvuDoc, page: u32, target_w: u32, target_h: u32) -> Option<Pixmap>`

All three take a 1-based `page`, return `None` for page 0 / missing page / decode failure (degrade, don't die), following the existing `page_text` pattern (`guard_panic(...).ok().flatten()`).

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)] mod tests` in `doc.rs` (which already has a `open`/fixture helper pattern — read the file first). If no doc-open helper exists in that module, use this inline form:

```rust
#[test]
fn page_dims_reports_fixture_page_size() {
    let bytes = std::fs::read("../../tests/fixtures/djvu/sample.djvu").unwrap();
    let doc = open(&bytes).unwrap();
    assert_eq!(page_dims(&doc, 1), Some((64, 64)));
    assert_eq!(page_dims(&doc, 0), None);
    assert_eq!(page_dims(&doc, 99), None);
}

#[test]
fn page_mask_decodes_bilevel_layer() {
    let bytes = std::fs::read("../../tests/fixtures/djvu/sample.djvu").unwrap();
    let doc = open(&bytes).unwrap();
    let mask = page_mask(&doc, 1).expect("fixture page has a JB2 mask");
    assert_eq!((mask.width, mask.height), (64, 64));
}

#[test]
fn page_pixmap_renders_to_requested_size() {
    let bytes = std::fs::read("../../tests/fixtures/djvu/sample.djvu").unwrap();
    let doc = open(&bytes).unwrap();
    let px = page_pixmap(&doc, 1, 64, 64).expect("fixture page renders");
    assert_eq!((px.width, px.height), (64, 64));
    assert_eq!(px.data.len(), 64 * 64 * 4); // RGBA
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters --lib djvu::doc 2>&1 | tail -20`
Expected: FAIL — `cannot find function 'page_dims'` (and `page_mask`, `page_pixmap`).

- [ ] **Step 3: Implement the ports**

Add near the other `pub fn` seams in `doc.rs` (after `page_text`), and the re-export near the top of the file (below the existing `use` lines):

```rust
/// Decoded-pixel budget for a rendered page image. A hostile file can declare
/// enormous dimensions; the input-size guard (`MAX_TOTAL_BYTES`) does not bound
/// decoded pixels. ~25 MP ≈ a 300-dpi tabloid page. Larger pages are downscaled
/// by the caller, not rejected — degrade, don't die.
pub(crate) const MAX_RENDER_PIXELS: u64 = 25_000_000;

/// Re-exported so `image.rs` consumes only port types, never `djvu_rs` directly.
pub use djvu_rs::{Bitmap, Pixmap};

/// Pixel `(width, height)` of a 1-based page; `None` if missing or on panic.
pub fn page_dims(doc: &DjvuDoc, page: u32) -> Option<(u32, u32)> {
    if page == 0 {
        return None;
    }
    guard_panic(|| {
        let Ok(p) = doc.inner.page((page - 1) as usize) else {
            return Ok(None);
        };
        Ok(Some((p.width(), p.height())))
    })
    .ok()
    .flatten()
}

/// The page's bilevel JB2/G4 mask, or `None` for a pure-IW44 (photographic)
/// page, a missing page, or a decode panic.
pub fn page_mask(doc: &DjvuDoc, page: u32) -> Option<Bitmap> {
    if page == 0 {
        return None;
    }
    guard_panic(|| {
        let Ok(p) = doc.inner.page((page - 1) as usize) else {
            return Ok(None);
        };
        Ok(p.decode_mask().unwrap_or(None))
    })
    .ok()
    .flatten()
}

/// A full RGBA render scaled to `target_w x target_h`; `None` on missing page
/// or render failure/panic.
pub fn page_pixmap(doc: &DjvuDoc, page: u32, target_w: u32, target_h: u32) -> Option<Pixmap> {
    if page == 0 {
        return None;
    }
    guard_panic(|| {
        let Ok(p) = doc.inner.page((page - 1) as usize) else {
            return Ok(None);
        };
        Ok(p.render_to_size(target_w, target_h).ok())
    })
    .ok()
    .flatten()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters --lib djvu::doc 2>&1 | tail -20`
Expected: PASS (all three new tests, plus existing `doc` tests unaffected).

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-adapters/src/djvu/doc.rs
git commit -m "feat(djvu): doc.rs ports for page dims, mask, and sized render"
```

---

### Task 2: `image.rs` — render and PNG-encode a page

**Files:**
- Create: `crates/kasane-adapters/src/djvu/image.rs`
- Modify: `crates/kasane-adapters/src/djvu/mod.rs` (add `mod image;`)

**Interfaces:**
- Consumes: `doc::{DjvuDoc, Bitmap, Pixmap, page_dims, page_mask, page_pixmap, MAX_RENDER_PIXELS}`; `kasane_ir::{AssetBag, AssetItem, AssetRef, Block}`.
- Produces: `pub fn render_page_image(doc: &doc::DjvuDoc, page: u32, assets: &mut AssetBag) -> Option<Block>` — appends one PNG `AssetItem` and returns the `Block::Figure` referencing it, or `None` when nothing renders.
- Internal pure helpers (unit-tested): `capped_target(u32, u32) -> Option<(u32, u32)>`, `downscale_mask(&Bitmap, u32, u32) -> Bitmap`.

- [ ] **Step 1: Register the module**

In `crates/kasane-adapters/src/djvu/mod.rs`, add alongside the existing `mod doc; mod outline; mod text;` lines:

```rust
mod image;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/kasane-adapters/src/djvu/image.rs` with only a test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::djvu::doc::{open, Bitmap};
    use kasane_ir::{AssetBag, Block};

    fn sample_doc() -> crate::djvu::doc::DjvuDoc {
        let bytes = std::fs::read("../../tests/fixtures/djvu/sample.djvu").unwrap();
        open(&bytes).unwrap()
    }

    #[test]
    fn renders_fixture_page_to_png_figure() {
        let doc = sample_doc();
        let mut assets = AssetBag::default();
        let block = render_page_image(&doc, 1, &mut assets).expect("fixture renders");
        assert!(matches!(block, Block::Figure { .. }));
        assert_eq!(assets.items.len(), 1);
        assert_eq!(assets.items[0].key, "djvu-page-1");
        assert!(assets.items[0].filename.ends_with(".png"));
        // PNG magic number.
        assert!(assets.items[0].bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn capped_target_downscales_only_over_budget() {
        // Under budget: no downscale.
        assert_eq!(capped_target(1000, 1000), None);
        // Over budget: scaled so width*height <= MAX_RENDER_PIXELS.
        let (w, h) = capped_target(10_000, 10_000).expect("over budget");
        assert!((w as u64) * (h as u64) <= super::super::doc::MAX_RENDER_PIXELS);
        assert!(w < 10_000 && h < 10_000);
    }

    #[test]
    fn downscale_mask_preserves_black_via_any_black_in_block() {
        // 4x4 with a single black pixel at (0,0) -> 2x2: top-left must be black.
        let mut bm = Bitmap::new(4, 4);
        bm.set_black(0, 0);
        let small = downscale_mask(&bm, 2, 2);
        assert_eq!((small.width, small.height), (2, 2));
        assert!(small.get(0, 0), "black pixel must survive downscale");
        assert!(!small.get(1, 1), "empty block must stay white");
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters --lib djvu::image 2>&1 | tail -20`
Expected: FAIL — `cannot find function 'render_page_image'` / `capped_target` / `downscale_mask`.

- [ ] **Step 4: Implement `image.rs`**

Prepend above the test module:

```rust
//! Render a text-less DjVu page to a PNG asset. Mask-first: a bilevel JB2 mask
//! becomes a small 1-bit PNG; a pure-IW44 page falls back to a full RGBA render
//! re-encoded as RGB. All `djvu-rs` access is via `doc.rs` (the sole seam).

use super::doc::{self, Bitmap, Pixmap, MAX_RENDER_PIXELS};
use kasane_ir::{AssetBag, AssetItem, AssetRef, Block};

/// Render page `page` (1-based) to a PNG and return the `Figure` referencing the
/// appended asset. `None` when the page has no dimensions or nothing decodes.
pub fn render_page_image(doc: &doc::DjvuDoc, page: u32, assets: &mut AssetBag) -> Option<Block> {
    let (nw, nh) = doc::page_dims(doc, page)?;
    if nw == 0 || nh == 0 {
        return None;
    }

    let png = if let Some(mut mask) = doc::page_mask(doc, page) {
        if let Some((tw, th)) = capped_target(mask.width, mask.height) {
            mask = downscale_mask(&mask, tw, th);
        }
        mask_to_png(&mask)?
    } else {
        let (tw, th) = capped_target(nw, nh).unwrap_or((nw, nh));
        let px = doc::page_pixmap(doc, page, tw, th)?;
        pixmap_to_png(&px)?
    };

    Some(push_page_asset(assets, page, png))
}

/// `None` when `(nw, nh)` is within the pixel budget (render at native size);
/// otherwise the largest same-aspect `(w, h)` whose area is within budget.
fn capped_target(nw: u32, nh: u32) -> Option<(u32, u32)> {
    let pixels = (nw as u64).saturating_mul(nh as u64);
    if nw == 0 || nh == 0 || pixels <= MAX_RENDER_PIXELS {
        return None;
    }
    let scale = (MAX_RENDER_PIXELS as f64 / pixels as f64).sqrt();
    let tw = ((nw as f64 * scale) as u32).max(1);
    let th = ((nh as f64 * scale) as u32).max(1);
    Some((tw, th))
}

/// Downscale a bilevel bitmap to `tw x th` by "any black in the source block":
/// an output pixel is black if any covered source pixel is black. Preserves thin
/// strokes better than point sampling.
fn downscale_mask(bm: &Bitmap, tw: u32, th: u32) -> Bitmap {
    let mut out = Bitmap::new(tw, th);
    if tw == 0 || th == 0 || bm.width == 0 || bm.height == 0 {
        return out;
    }
    for oy in 0..th {
        let sy0 = (oy as u64 * bm.height as u64 / th as u64) as u32;
        let sy1 = ((((oy + 1) as u64 * bm.height as u64 / th as u64) as u32).max(sy0 + 1))
            .min(bm.height);
        for ox in 0..tw {
            let sx0 = (ox as u64 * bm.width as u64 / tw as u64) as u32;
            let sx1 = ((((ox + 1) as u64 * bm.width as u64 / tw as u64) as u32).max(sx0 + 1))
                .min(bm.width);
            'block: for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    if bm.get(sx, sy) {
                        out.set_black(ox, oy);
                        break 'block;
                    }
                }
            }
        }
    }
    out
}

/// 1-bit grayscale PNG. `Bitmap` uses bit 1 = black; PNG grayscale uses sample
/// 0 = black, so every packed byte is inverted. Padding bits in each row's last
/// byte are ignored by decoders. `None` on a zero dimension or encoder error.
fn mask_to_png(bm: &Bitmap) -> Option<Vec<u8>> {
    if bm.width == 0 || bm.height == 0 {
        return None;
    }
    let inverted: Vec<u8> = bm.data.iter().map(|b| !b).collect();
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, bm.width, bm.height);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::One);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(&inverted).ok()?;
    }
    Some(out)
}

/// 8-bit RGB PNG from an RGBA pixmap (DjVu pages are opaque; drop alpha).
fn pixmap_to_png(px: &Pixmap) -> Option<Vec<u8>> {
    if px.width == 0 || px.height == 0 {
        return None;
    }
    let rgb: Vec<u8> = px
        .data
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, px.width, px.height);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(&rgb).ok()?;
    }
    Some(out)
}

/// Append the PNG as an asset and build the `Figure` referencing it.
fn push_page_asset(assets: &mut AssetBag, page: u32, bytes: Vec<u8>) -> Block {
    let key = format!("djvu-page-{page}");
    let idx = assets.items.len();
    assets.items.push(AssetItem {
        key: key.clone(),
        filename: format!("{key}.png"),
        bytes,
    });
    Block::Figure {
        image: AssetRef {
            key,
            bytes_ref: idx,
        },
        caption: vec![],
        number: None,
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters --lib djvu::image 2>&1 | tail -20`
Expected: PASS (all three tests).

- [ ] **Step 6: Lint (the mask/pixmap helpers use casts clippy scrutinizes)**

Run: `cargo clippy -p kasane-adapters --all-targets 2>&1 | tail -20`
Expected: no warnings. If clippy flags `cast_possible_truncation` on the scale math, the casts are intentional and bounded — add a scoped `#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]` on `capped_target`/`downscale_mask` with a one-line comment, matching how existing adapter code handles pixel math.

- [ ] **Step 7: Commit**

```bash
git add crates/kasane-adapters/src/djvu/image.rs crates/kasane-adapters/src/djvu/mod.rs
git commit -m "feat(djvu): image.rs — mask-first page render to PNG figure"
```

---

### Task 3: Add a text-less DjVu fixture

**Files:**
- Modify: `crates/kasane-adapters/examples/make_djvu_fixture.rs`
- Create (generated, committed): `tests/fixtures/djvu/scanned.djvu`
- Modify: `tests/fixtures/djvu/README.md`

**Interfaces:**
- Produces: `tests/fixtures/djvu/scanned.djvu` — a single bundled page with a JB2 mask, **no text layer**, **no NAVM outline**. This is the input that exercises the text-less-page code path in Task 4.

- [ ] **Step 1: Extend the generator**

In `make_djvu_fixture.rs`, after the block that writes `sample.djvu` (the final `std::fs::write("tests/fixtures/djvu/sample.djvu", ...)`), add a second fixture built from the **same bitmap `bm`** but with no text layer and no bookmark. Insert before the closing `}` of `main`:

```rust
    // --- Second fixture: a text-less scanned page (mask only, no TXTz, no NAVM).
    //     Exercises the page-image code path: no recoverable text, no outline.
    let scanned_page = PageEncoder::from_bitmap(&bm)
        .with_dpi(100)
        .encode()
        .expect("encode text-less FORM:DJVU");
    // A bare single-page FORM:DJVU is bundled by construction (no DIRM), which is
    // what the adapter requires; no DJVM merge or NAVM attachment needed.
    let scanned_parsed = DjVuDocument::parse(&scanned_page).expect("re-parse scanned fixture");
    assert_eq!(scanned_parsed.page_count(), 1, "scanned fixture: one page");
    assert!(
        scanned_parsed
            .page(0)
            .expect("scanned page 0")
            .text_layer()
            .expect("text layer decode ok")
            .is_none(),
        "scanned fixture must have NO text layer"
    );
    std::fs::write("tests/fixtures/djvu/scanned.djvu", &scanned_page)
        .expect("write scanned.djvu");
    println!(
        "wrote tests/fixtures/djvu/scanned.djvu ({} bytes): 1 page, no text layer, no outline",
        scanned_page.len(),
    );
```

- [ ] **Step 2: Regenerate the fixtures**

Run: `cargo run -p kasane-adapters --example make_djvu_fixture`
Expected: prints two `wrote ...` lines; `tests/fixtures/djvu/scanned.djvu` now exists. Confirm:

Run: `ls -l tests/fixtures/djvu/scanned.djvu`
Expected: file present, non-zero size.

- [ ] **Step 3: Document the new fixture**

Append to `tests/fixtures/djvu/README.md` a short paragraph:

```markdown
`scanned.djvu` is generated by the same `make_djvu_fixture` example: a single
bundled page carrying only the JB2 bitmap mask — no hidden text layer and no
NAVM outline. It exercises the text-less page path, where the adapter renders the
page image (see `djvu/image.rs`) instead of recovering text.
```

- [ ] **Step 4: Commit**

```bash
git add crates/kasane-adapters/examples/make_djvu_fixture.rs tests/fixtures/djvu/scanned.djvu tests/fixtures/djvu/README.md
git commit -m "test(djvu): add text-less scanned.djvu fixture (mask only)"
```

---

### Task 4: Wire rendering into `mod.rs`

**Files:**
- Modify: `crates/kasane-adapters/src/djvu/mod.rs`

**Interfaces:**
- Consumes: `image::render_page_image`; existing `text::page_blocks`.
- Produces: text-less pages emit `Block::Figure` + a trimmed `Block::Raw` note. `page_nodes_from_lines` gains a final parameter `page_image: Option<Block>`.

- [ ] **Step 1: Write the failing integration tests**

Add to the `#[cfg(test)] mod tests` in `mod.rs`:

```rust
    #[test]
    fn scanned_page_emits_figure_and_trimmed_note() {
        let bytes = std::fs::read("../../tests/fixtures/djvu/scanned.djvu").unwrap();
        let (doc, assets) = DjvuAdapter.parse(&bytes, "scanned.djvu").unwrap();

        // A page image was emitted as a Figure + asset.
        assert_eq!(assets.items.len(), 1, "one page image asset");
        assert!(doc
            .nodes
            .iter()
            .any(|n| matches!(&n.block, Block::Figure { .. })));

        // The note is the trimmed "page image only" form, not the bare note.
        let notes: Vec<&str> = doc
            .nodes
            .iter()
            .filter_map(|n| match &n.block {
                Block::Raw { note } => Some(note.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            notes.iter().any(|n| n.contains("page image only")),
            "notes: {notes:?}"
        );
        assert!(
            !notes.iter().any(|n| *n == "no text layer; OCR not enabled"),
            "bare note must be replaced when an image is emitted: {notes:?}"
        );
    }

    #[test]
    fn text_page_emits_no_figure() {
        // Regression pin: pages that recovered text never get a page image.
        let doc = sample();
        assert!(
            !doc.nodes
                .iter()
                .any(|n| matches!(&n.block, Block::Figure { .. })),
            "text-bearing fixture must not produce a page image"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters --lib djvu::tests::scanned_page_emits_figure_and_trimmed_note djvu::tests::text_page_emits_no_figure 2>&1 | tail -20`
Expected: FAIL — `scanned_page_emits_figure...` fails (0 assets / bare note present). `text_page_emits_no_figure` passes already but must keep passing.

- [ ] **Step 3: Add the trimmed-note constants**

Near the existing `NO_TEXT_NOTE` / `EMPTY_TEXT_NOTE` consts at the top of `mod.rs`:

```rust
/// Emitted with a rendered page image when the page had no text layer.
const IMG_NO_TEXT_NOTE: &str = "page image only; no text layer, OCR not enabled";
/// Emitted with a rendered page image when the text layer was present but empty.
const IMG_EMPTY_TEXT_NOTE: &str = "page image only; text layer present but empty";
```

- [ ] **Step 4: Extend `page_nodes_from_lines` with the image branch**

Change its signature to add a final parameter and rewrite only the terminal text-less branch. The function's other logic is unchanged. New signature:

```rust
fn page_nodes_from_lines(
    page: u32,
    lines: &[Line],
    headings: &[OutlineHeading],
    next_id: &mut u32,
    body_height: f32,
    infer_headings: bool,
    has_text: bool,
    page_image: Option<Block>,
) -> Vec<Node> {
```

Replace the existing terminal block:

```rust
    // No outline heading, nothing recovered -> honest note. Every such page must
    // leave a trace, but the wording distinguishes "no text layer at all" from
    // "text layer present but empty after filtering".
    if headings.is_empty() && !had_blocks {
        let note = if has_text {
            EMPTY_TEXT_NOTE
        } else {
            NO_TEXT_NOTE
        };
        out.push(Node {
            block: Block::Raw { note: note.into() },
            prov,
        });
    }
    out
```

with:

```rust
    // No outline heading, nothing recovered. If we rendered a page image, emit
    // it and a trimmed note that still records text was not recovered; otherwise
    // fall back to the bare note. The wording keeps "no text layer at all"
    // distinct from "text layer present but empty after filtering".
    if headings.is_empty() && !had_blocks {
        match page_image {
            Some(figure) => {
                out.push(Node {
                    block: figure,
                    prov: prov.clone(),
                });
                let note = if has_text {
                    IMG_EMPTY_TEXT_NOTE
                } else {
                    IMG_NO_TEXT_NOTE
                };
                out.push(Node {
                    block: Block::Raw { note: note.into() },
                    prov,
                });
            }
            None => {
                let note = if has_text {
                    EMPTY_TEXT_NOTE
                } else {
                    NO_TEXT_NOTE
                };
                out.push(Node {
                    block: Block::Raw { note: note.into() },
                    prov,
                });
            }
        }
    }
    out
```

- [ ] **Step 5: Render only for text-less pages in `parse`, and pass `page_image` down**

In `parse`, replace the second per-page loop body. Current:

```rust
        for (p, has_text, lines) in &pages {
            let headings = outline.get(p).unwrap_or(&empty);
            nodes.extend(page_nodes_from_lines(
                *p,
                lines,
                headings,
                &mut next_id,
                body_height,
                !has_outline,
                *has_text,
            ));
        }
```

with:

```rust
        for (p, has_text, lines) in &pages {
            let headings = outline.get(p).unwrap_or(&empty);
            // A page is text-less iff it has no outline heading and its lines
            // yield no blocks. Probe with a throwaway id counter so this trial
            // run allocates no BlockIds (text-less pages produce zero blocks, so
            // nothing is consumed anyway) and does not perturb `next_id`.
            let text_less = headings.is_empty() && {
                let mut probe = next_id;
                text::page_blocks(lines, &mut probe, body_height, !has_outline).is_empty()
            };
            let page_image = if text_less {
                image::render_page_image(&djvu, *p, &mut assets)
            } else {
                None
            };
            nodes.extend(page_nodes_from_lines(
                *p,
                lines,
                headings,
                &mut next_id,
                body_height,
                !has_outline,
                *has_text,
                page_image,
            ));
        }
```

Note: `assets` is currently `AssetBag::default()` returned at the end of `parse` — promote it to a `let mut assets = AssetBag::default();` declared before the loop, and return `assets` (not a fresh default) in the `Ok((out, ...))`. Read the surrounding code and adjust the return accordingly.

- [ ] **Step 6: Update the `page_nodes` test wrapper**

The `#[cfg(test)] fn page_nodes(...)` wrapper calls `page_nodes_from_lines`. Add the new argument as `None`:

```rust
    page_nodes_from_lines(
        page,
        &lines,
        headings,
        next_id,
        body_height,
        infer_headings,
        text_root.is_some(),
        None,
    )
```

- [ ] **Step 7: Run the full adapter test suite**

Run: `cargo test -p kasane-adapters --lib djvu 2>&1 | tail -25`
Expected: PASS — the two new tests plus all existing `djvu` tests (existing text-less unit tests still assert the bare `NO_TEXT_NOTE`/`EMPTY_TEXT_NOTE` because the `page_nodes` wrapper passes `None`).

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-adapters/src/djvu/mod.rs
git commit -m "feat(djvu): emit page image for text-less pages"
```

---

### Task 5: Documentation

**Files:**
- Modify: `README.md`
- Modify: `AGENTS.md`

**Interfaces:** none (docs only).

- [ ] **Step 1: Update README DjVu limitations**

In `README.md`, find the bullet:

```
- Scanned page images (JB2/IW44) are not rendered in this build. A page with no
  text layer at all becomes a placeholder note ("no text layer; OCR not
  enabled"); a page whose text layer is present but decodes to nothing gets a
  different note ("text layer present but empty; no recoverable text").
```

Replace with:

```
- Text-less pages now emit the rendered page image: the bilevel JB2 mask as a
  compact 1-bit PNG, or a full IW44 render (RGB PNG) when the page has no mask.
  A rendered page carries a marker that its text is un-OCR'd — "page image only;
  no text layer, OCR not enabled" when there was no text layer, or "page image
  only; text layer present but empty" when the layer decoded to nothing. If a
  page fails to render, the bare placeholder note is emitted instead. Pages that
  recovered text get no image. Text recovery still depends on the embedded OCR
  text layer; kasane does not run its own OCR (see the `-F ocr` roadmap).
```

- [ ] **Step 2: Update the AGENTS.md codebase map**

In `AGENTS.md`, the `djvu/` map sentence currently reads (in part):

```
Image layers (JB2/IW44) are intentionally not decoded — pages fall back to placeholder notes.
```

Replace that clause with:

```
`image.rs` renders text-less pages to a page image — the JB2 mask as a 1-bit PNG, falling back to a full IW44 render — bounded by a decoded-pixel budget in `doc.rs` (`MAX_RENDER_PIXELS`); text-bearing pages remain text-only.
```

- [ ] **Step 3: Verify the whole gate is green**

Run: `mise run lint && mise run test`
Expected: fmt clean, clippy `--all-targets` no warnings, all tests pass across the workspace.

- [ ] **Step 4: Commit**

```bash
git add README.md AGENTS.md
git commit -m "docs(djvu): document page-image rendering for text-less pages"
```

---

## Self-Review Notes

- **Spec coverage:** §2 module → Task 2; §2 seam ports → Task 1; §3 pixel cap (`MAX_RENDER_PIXELS`, `capped_target`, mask downscale, cumulative byte guard is the pre-existing `accumulate_text_bytes` path, unaffected) → Tasks 1–2; §4 wiring + trimmed notes + `page_image` param + purity via probe → Task 4; §5 tests (mask path, chosen-path fixture, cap, guard-degrades — the guard-degrades case is covered by the existing `guard_panic` `.ok().flatten()` returning `None`, already unit-covered in `doc.rs`) → Tasks 1–4; §6 docs → Task 5; §7 file list matches Tasks 1–5.
- **Deviation from spec §3:** the spec sketched `page_pixmap(doc, page, max_w, max_h)` using `fit_to_box`. This plan uses `Page::render_to_size(target_w, target_h)` with the cap math extracted into the pure, unit-testable `capped_target`, keeping `doc.rs` a thin seam and the scaling logic independently tested. Same behavior and same safety bound.
- **Type consistency:** `render_page_image`, `page_dims`, `page_mask`, `page_pixmap`, `capped_target`, `downscale_mask`, `push_page_asset` names are used identically across tasks; `Block::Figure`/`AssetRef`/`AssetItem` fields match `kasane-ir` (`image`, `caption`, `number`; `key`, `bytes_ref`; `key`, `filename`, `bytes`).
