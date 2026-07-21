use super::doc::MAX_CONTENT_BYTES;
use lopdf::content::Content;
use lopdf::{Document, Encoding, Object, ObjectId};
use std::collections::BTreeMap;

/// One text-showing operation, positioned in device space.
#[derive(Clone, Debug)]
pub struct TextRun {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub text: String,
}

/// 2×3 affine matrix [a b c d e f] using the PDF row-vector convention
/// (a point [x y 1] is transformed as [x y 1] · M).
type Mat = [f32; 6];
const IDENT: Mat = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// m · n (apply m first, then n).
fn mul(m: Mat, n: Mat) -> Mat {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

fn translate(tx: f32, ty: f32) -> Mat {
    [1.0, 0.0, 0.0, 1.0, tx, ty]
}

fn nums(operands: &[Object]) -> Vec<f32> {
    operands.iter().map(|o| o.as_float().unwrap_or(0.0)).collect()
}

/// Interpret a page's content stream into positioned, Unicode-decoded text runs.
/// Never panics; returns an empty vec if the page has no readable content.
pub fn page_text_runs(doc: &Document, page_id: ObjectId) -> Vec<TextRun> {
    let fonts = doc.get_page_fonts(page_id).unwrap_or_default();
    let encodings: BTreeMap<Vec<u8>, Encoding<'_>> = fonts
        .into_iter()
        .filter_map(|(name, font)| font.get_font_encoding(doc).ok().map(|enc| (name, enc)))
        .collect();

    let bytes = match doc.get_page_content_with_limit(page_id, MAX_CONTENT_BYTES) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let content = match Content::decode(&bytes) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut runs = Vec::new();
    let mut ctm_stack: Vec<Mat> = Vec::new();
    let mut ctm = IDENT;
    let mut tm = IDENT; // text matrix
    let mut tlm = IDENT; // text line matrix
    let mut font_size = 0.0f32;
    let mut leading = 0.0f32;
    let mut encoding: Option<&Encoding<'_>> = None;

    for op in &content.operations {
        match op.operator.as_str() {
            "q" => ctm_stack.push(ctm),
            "Q" => {
                if let Some(m) = ctm_stack.pop() {
                    ctm = m;
                }
            }
            "cm" => {
                let n = nums(&op.operands);
                if n.len() == 6 {
                    ctm = mul([n[0], n[1], n[2], n[3], n[4], n[5]], ctm);
                }
            }
            "BT" => {
                tm = IDENT;
                tlm = IDENT;
            }
            "Tf" => {
                if let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                    encoding = encodings.get(name);
                }
                font_size = op.operands.get(1).and_then(|o| o.as_float().ok()).unwrap_or(font_size);
            }
            "TL" => leading = op.operands.first().and_then(|o| o.as_float().ok()).unwrap_or(leading),
            "Td" => {
                let n = nums(&op.operands);
                if n.len() == 2 {
                    tlm = mul(translate(n[0], n[1]), tlm);
                    tm = tlm;
                }
            }
            "TD" => {
                let n = nums(&op.operands);
                if n.len() == 2 {
                    leading = -n[1];
                    tlm = mul(translate(n[0], n[1]), tlm);
                    tm = tlm;
                }
            }
            "Tm" => {
                let n = nums(&op.operands);
                if n.len() == 6 {
                    tlm = [n[0], n[1], n[2], n[3], n[4], n[5]];
                    tm = tlm;
                }
            }
            "T*" => {
                tlm = mul(translate(0.0, -leading), tlm);
                tm = tlm;
            }
            "Tj" | "TJ" => {
                if let Some(enc) = encoding {
                    if let Some(run) = show(&op.operands, enc, tm, ctm, font_size) {
                        runs.push(run);
                    }
                }
            }
            "'" => {
                tlm = mul(translate(0.0, -leading), tlm);
                tm = tlm;
                if let Some(enc) = encoding {
                    if let Some(run) = show(&op.operands, enc, tm, ctm, font_size) {
                        runs.push(run);
                    }
                }
            }
            "\"" => {
                tlm = mul(translate(0.0, -leading), tlm);
                tm = tlm;
                if let (Some(enc), Some(s)) = (encoding, op.operands.get(2)) {
                    if let Some(run) = show(std::slice::from_ref(s), enc, tm, ctm, font_size) {
                        runs.push(run);
                    }
                }
            }
            _ => {}
        }
    }
    runs
}

/// Build a TextRun from a show operator's operands at the current matrices.
fn show(operands: &[Object], enc: &Encoding<'_>, tm: Mat, ctm: Mat, font_size: f32) -> Option<TextRun> {
    let mut text = String::new();
    decode_into(operands, enc, &mut text);
    if text.trim().is_empty() {
        return None;
    }
    let trm = mul(tm, ctm); // text rendering matrix (translation + scale, ignoring rise)
    // vertical scale magnitude of the composed matrix
    let yscale = (trm[1] * trm[1] + trm[3] * trm[3]).sqrt();
    Some(TextRun {
        x: trm[4],
        y: trm[5],
        size: font_size * if yscale.is_finite() && yscale > 0.0 { yscale } else { 1.0 },
        text,
    })
}

/// Append decoded text from Tj (string) / TJ (array of strings + kerning numbers).
/// A large negative kerning advance is rendered as a space.
fn decode_into(operands: &[Object], enc: &Encoding<'_>, out: &mut String) {
    for op in operands {
        match op {
            Object::String(bytes, _) => {
                if let Ok(s) = enc.bytes_to_string(bytes) {
                    out.push_str(&s);
                }
            }
            Object::Array(arr) => {
                for el in arr {
                    match el {
                        Object::String(bytes, _) => {
                            if let Ok(s) = enc.bytes_to_string(bytes) {
                                out.push_str(&s);
                            }
                        }
                        Object::Real(n) if *n <= -100.0 => out.push(' '),
                        Object::Integer(n) if *n <= -100 => out.push(' '),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::doc::{open, pages};

    fn runs(name: &str) -> Vec<TextRun> {
        let bytes = std::fs::read(format!("../../tests/fixtures/pdf/{name}.pdf")).unwrap();
        let doc = open(&bytes).unwrap();
        let (_, page1) = pages(&doc)[0];
        page_text_runs(&doc, page1)
    }

    #[test]
    fn extracts_positioned_text_from_page_one() {
        let r = runs("minimal");
        let texts: Vec<&str> = r.iter().map(|t| t.text.as_str()).collect();
        assert!(texts.contains(&"Chapter One"), "got {texts:?}");
        assert!(texts.contains(&"First body line."), "got {texts:?}");
        // Tm placed "Chapter One" at y=170, size 12.
        let title = r.iter().find(|t| t.text == "Chapter One").unwrap();
        assert!((title.y - 170.0).abs() < 1.0, "y was {}", title.y);
        assert!((title.size - 12.0).abs() < 0.5, "size was {}", title.size);
        assert!((title.x - 20.0).abs() < 1.0, "x was {}", title.x);
    }

    #[test]
    fn font_size_survives_for_large_heading() {
        let r = runs("no-outline");
        let big = r.iter().find(|t| t.text == "Big Title").unwrap();
        assert!(big.size > 20.0, "expected large size, got {}", big.size);
    }
}
