//! Render a text-less DjVu page to a PNG asset. Mask-first: a bilevel JB2 mask
//! becomes a small 1-bit PNG; a pure-IW44 page falls back to a full RGBA render
//! re-encoded as RGB. All `djvu-rs` access is via `doc.rs` (the sole seam).

use super::doc::{self, Bitmap, Pixmap, MAX_RENDER_PIXELS};
use kasane_ir::{AssetBag, AssetItem, AssetRef, Block};

/// Render page `page` (1-based) to PNG bytes, without touching the asset bag.
/// `None` when the page has no dimensions or nothing decodes.
pub(super) fn render_page_png(doc: &doc::DjvuDoc, page: u32) -> Option<Vec<u8>> {
    let (nw, nh) = doc::page_dims(doc, page)?;
    if nw == 0 || nh == 0 {
        return None;
    }
    if let Some(mut mask) = doc::page_mask(doc, page) {
        if let Some((tw, th)) = capped_target(mask.width, mask.height) {
            mask = downscale_mask(&mask, tw, th);
        }
        mask_to_png(&mask)
    } else {
        let (tw, th) = capped_target(nw, nh).unwrap_or((nw, nh));
        let px = doc::page_pixmap(doc, page, tw, th)?;
        pixmap_to_png(&px)
    }
}

/// Render page `page` to a PNG asset and return the `Figure` referencing it.
pub fn render_page_image(doc: &doc::DjvuDoc, page: u32, assets: &mut AssetBag) -> Option<Block> {
    let png = render_page_png(doc, page)?;
    Some(push_page_asset(assets, page, png))
}

/// `None` when `(nw, nh)` is within the pixel budget (render at native size);
/// otherwise the largest same-aspect `(w, h)` whose area is within budget.
/// The `.max(1)` floor is safe on both call sites: the pixmap path's `nw, nh`
/// come from a `u16` page size, and the mask path's come from JB2 decode
/// (capped at ~262K per dimension). Either way each source dimension is far
/// below `MAX_RENDER_PIXELS`, so the extreme-aspect-ratio case that could
/// otherwise scale a dimension below 1 is unreachable.
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
        let sy1 =
            ((((oy + 1) as u64 * bm.height as u64 / th as u64) as u32).max(sy0 + 1)).min(bm.height);
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
pub(super) fn push_page_asset(assets: &mut AssetBag, page: u32, bytes: Vec<u8>) -> Block {
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
