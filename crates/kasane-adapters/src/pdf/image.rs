use crate::guard::MAX_TOTAL_BYTES;
use kasane_ir::{AssetBag, AssetItem, AssetRef, Block};
use lopdf::{Document, Object, ObjectId};

/// Result of scanning one page for images.
pub struct PageImages {
    pub figures: Vec<Block>,
    pub had_image: bool,
    /// Filter names of images we recognized but could not decode.
    pub skipped: Vec<String>,
}

/// Extract a page's `/XObject` images. Supported: FlateDecode DeviceGray/RGB
/// 8-bit (re-encoded to PNG) and DCTDecode (JPEG passthrough). Others are
/// reported in `skipped` for the caller to note. Bomb-guarded per image.
pub fn extract_page_images(doc: &Document, page_id: ObjectId, assets: &mut AssetBag) -> PageImages {
    let mut figures = Vec::new();
    let mut skipped = Vec::new();
    let mut had_image = false;

    let xobjects = match page_xobject_ids(doc, page_id) {
        Some(x) => x,
        None => {
            return PageImages {
                figures,
                had_image,
                skipped,
            }
        }
    };

    for id in xobjects {
        let Ok(obj) = doc.get_object(id) else {
            continue;
        };
        let Ok(stream) = obj.as_stream() else {
            continue;
        };
        let dict = &stream.dict;
        if dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) != Some(b"Image") {
            continue;
        }
        had_image = true;

        let filter = last_filter(dict);
        let key = format!("pdf-image-{}-{}", id.0, id.1);
        let idx = assets.items.len();

        match filter.as_deref() {
            Some(b"DCTDecode") => {
                let bytes = stream.content.clone();
                if bytes.len() as u64 > MAX_TOTAL_BYTES {
                    skipped.push("DCTDecode(too large)".into());
                    continue;
                }
                push_asset(assets, &mut figures, &key, idx, format!("{key}.jpg"), bytes);
            }
            Some(b"FlateDecode") => match flate_to_png(doc, stream) {
                Ok(png) => push_asset(assets, &mut figures, &key, idx, format!("{key}.png"), png),
                Err(reason) => skipped.push(reason),
            },
            other => {
                let name = other
                    .map(|b| String::from_utf8_lossy(b).into_owned())
                    .unwrap_or_else(|| "unknown".into());
                skipped.push(name);
            }
        }
    }

    PageImages {
        figures,
        had_image,
        skipped,
    }
}

fn push_asset(
    assets: &mut AssetBag,
    figures: &mut Vec<Block>,
    key: &str,
    idx: usize,
    filename: String,
    bytes: Vec<u8>,
) {
    assets.items.push(AssetItem {
        key: key.to_string(),
        filename,
        bytes,
    });
    figures.push(Block::Figure {
        image: AssetRef {
            key: key.to_string(),
            bytes_ref: idx,
        },
        caption: vec![],
        number: None,
    });
}

/// Page `/Resources /XObject` entries, resolved to object ids.
fn page_xobject_ids(doc: &Document, page_id: ObjectId) -> Option<Vec<ObjectId>> {
    let (resources, _) = doc.get_page_resources(page_id).ok()?;
    let resources = resources?;
    let xobj = resources.get(b"XObject").ok()?;
    let dict = match xobj {
        Object::Reference(r) => doc.get_dictionary(*r).ok()?,
        Object::Dictionary(d) => d,
        _ => return None,
    };
    Some(
        dict.iter()
            .filter_map(|(_, v)| v.as_reference().ok())
            .collect(),
    )
}

/// The last filter in a stream dict's `/Filter` (Name or Array of Names).
fn last_filter(dict: &lopdf::Dictionary) -> Option<Vec<u8>> {
    match dict.get(b"Filter").ok()? {
        Object::Name(n) => Some(n.clone()),
        Object::Array(a) => a.last().and_then(|o| o.as_name().ok()).map(|n| n.to_vec()),
        _ => None,
    }
}

/// Re-encode a FlateDecode DeviceGray/DeviceRGB 8-bit image as PNG.
/// Returns Err(reason) for unsupported colorspaces/depths.
fn flate_to_png(doc: &Document, stream: &lopdf::Stream) -> Result<Vec<u8>, String> {
    let dict = &stream.dict;
    let width = dict
        .get(b"Width")
        .and_then(|o| o.as_i64())
        .map_err(|_| "no width".to_string())? as u32;
    let height = dict
        .get(b"Height")
        .and_then(|o| o.as_i64())
        .map_err(|_| "no height".to_string())? as u32;
    let bpc = dict
        .get(b"BitsPerComponent")
        .and_then(|o| o.as_i64())
        .unwrap_or(8);
    if bpc != 8 {
        return Err(format!("FlateDecode({bpc}bpc)"));
    }
    let color = match colorspace_name(doc, dict) {
        Some(b"DeviceRGB") => png::ColorType::Rgb,
        Some(b"DeviceGray") => png::ColorType::Grayscale,
        _ => return Err("FlateDecode(colorspace)".to_string()),
    };
    let raw = stream
        .decompressed_content_with_limit(MAX_TOTAL_BYTES as usize)
        .map_err(|_| "FlateDecode(decompress)".to_string())?;
    let expected = (width as u64)
        .checked_mul(height as u64)
        .and_then(|n| n.checked_mul(color.samples() as u64))
        .filter(|&n| n <= MAX_TOTAL_BYTES)
        .ok_or_else(|| "FlateDecode(dims)".to_string())? as usize;
    if raw.len() < expected {
        return Err("FlateDecode(short)".to_string());
    }

    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| format!("png header: {e}"))?;
        writer
            .write_image_data(&raw[..expected])
            .map_err(|e| format!("png data: {e}"))?;
    }
    Ok(out)
}

fn colorspace_name<'a>(doc: &'a Document, dict: &'a lopdf::Dictionary) -> Option<&'a [u8]> {
    match dict.get(b"ColorSpace").ok()? {
        Object::Name(n) => Some(n),
        Object::Reference(r) => doc.get_object(*r).ok()?.as_name().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::doc::{open, pages};
    use kasane_ir::AssetBag;

    fn extract(name: &str) -> (PageImages, AssetBag) {
        let doc =
            open(&std::fs::read(format!("../../tests/fixtures/pdf/{name}.pdf")).unwrap()).unwrap();
        let (_, page1) = pages(&doc)[0];
        let mut assets = AssetBag::default();
        let pi = extract_page_images(&doc, page1, &mut assets);
        (pi, assets)
    }

    #[test]
    fn extracts_flate_rgb_image_as_png() {
        let (pi, assets) = extract("image");
        assert!(pi.had_image);
        assert_eq!(pi.figures.len(), 1);
        assert_eq!(assets.items.len(), 1);
        // FlateDecode RGB is re-encoded to PNG.
        assert!(assets.items[0].filename.ends_with(".png"));
        assert!(assets.items[0].bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn scanned_page_has_image_but_no_text() {
        let (pi, assets) = extract("scanned");
        assert!(pi.had_image);
        assert_eq!(assets.items.len(), 1);
    }

    /// A corrupt/malicious PDF can declare near-u32::MAX `/Width` and
    /// `/Height`. `width * height * samples` must not overflow (or silently
    /// wrap) the `usize` multiply used for the short-buffer guard: it must be
    /// rejected with an `Err`, not panic and not sail past the guard.
    #[test]
    fn flate_to_png_rejects_overflowing_dimensions_instead_of_panicking() {
        let doc = Document::new();

        let mut dict = lopdf::Dictionary::new();
        dict.set("Type", "XObject");
        dict.set("Subtype", "Image");
        dict.set("Width", u32::MAX as i64);
        dict.set("Height", u32::MAX as i64);
        dict.set("BitsPerComponent", 8_i64);
        dict.set("ColorSpace", "DeviceRGB");
        dict.set("Filter", "FlateDecode");

        // A valid (tiny) zlib stream that decompresses to zero bytes, so the
        // decompress step succeeds and we actually reach the dimension guard
        // rather than short-circuiting on a decompress error.
        let content = vec![0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01];
        let stream = lopdf::Stream::new(dict, content);

        let result = flate_to_png(&doc, &stream);
        assert_eq!(result, Err("FlateDecode(dims)".to_string()));
    }
}
