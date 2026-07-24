//! Tesseract-backed `TextExtractor`. The only C-linking code in kasane; compiled
//! only under `-F ocr`. Line-level results are read from Tesseract's TSV output
//! (stable across versions) rather than the result-iterator FFI.

use super::{OcrBBox, OcrError, OcrLine, OcrOptions, TextExtractor};
use leptess::LepTess;

#[derive(Debug)]
pub struct TesseractExtractor {
    lang: String,
}

impl TesseractExtractor {
    /// Validate up front that Tesseract can init with `lang` (i.e. the
    /// traineddata is present), turning a missing pack into a clear error.
    pub fn new(lang: &str) -> Result<Self, OcrError> {
        LepTess::new(None, lang).map_err(|e| OcrError::MissingLanguage(format!("{lang} ({e})")))?;
        Ok(Self {
            lang: lang.to_string(),
        })
    }
}

impl TextExtractor for TesseractExtractor {
    fn extract(&self, image: &[u8], _opts: &OcrOptions) -> Result<Vec<OcrLine>, OcrError> {
        let mut tess = LepTess::new(None, &self.lang)
            .map_err(|e| OcrError::MissingLanguage(format!("{} ({e})", self.lang)))?;
        tess.set_image_from_mem(image)
            .map_err(|e| OcrError::Decode(e.to_string()))?;
        let tsv = tess
            .get_tsv_text(0)
            .map_err(|e| OcrError::Decode(e.to_string()))?;
        Ok(parse_tsv_lines(&tsv))
    }
}

#[derive(Default)]
struct LineAcc {
    text: String,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    conf_sum: f32,
    words: u32,
}

impl LineAcc {
    fn push_word(&mut self, text: &str, left: f32, top: f32, w: f32, h: f32, conf: f32) {
        if self.words == 0 {
            self.x0 = left;
            self.y0 = top;
            self.x1 = left + w;
            self.y1 = top + h;
        } else {
            self.x0 = self.x0.min(left);
            self.y0 = self.y0.min(top);
            self.x1 = self.x1.max(left + w);
            self.y1 = self.y1.max(top + h);
            self.text.push(' ');
        }
        self.text.push_str(text);
        self.conf_sum += conf;
        self.words += 1;
    }

    fn finish(self) -> OcrLine {
        OcrLine {
            text: self.text,
            bbox: OcrBBox {
                x: self.x0,
                y: self.y0,
                w: self.x1 - self.x0,
                h: self.y1 - self.y0,
            },
            confidence: if self.words == 0 {
                0.0
            } else {
                self.conf_sum / self.words as f32
            },
        }
    }
}

/// Aggregate Tesseract TSV word rows (level 5) into one `OcrLine` per text line.
/// Columns: level page block par line word left top width height conf text.
fn parse_tsv_lines(tsv: &str) -> Vec<OcrLine> {
    use std::collections::BTreeMap;
    // (block, par, line) key: BTreeMap ordering preserves reading order.
    let mut lines: BTreeMap<(i32, i32, i32), LineAcc> = BTreeMap::new();
    for row in tsv.lines() {
        let c: Vec<&str> = row.split('\t').collect();
        if c.len() < 12 || c[0] != "5" {
            continue; // level 5 = word
        }
        let block = c[2].parse::<i32>().unwrap_or(0);
        let par = c[3].parse::<i32>().unwrap_or(0);
        let line = c[4].parse::<i32>().unwrap_or(0);
        let left = c[6].parse::<f32>().unwrap_or(0.0);
        let top = c[7].parse::<f32>().unwrap_or(0.0);
        let w = c[8].parse::<f32>().unwrap_or(0.0);
        let h = c[9].parse::<f32>().unwrap_or(0.0);
        let conf = c[10].parse::<f32>().unwrap_or(-1.0);
        let text = c[11].trim();
        if text.is_empty() || conf < 0.0 {
            continue;
        }
        lines
            .entry((block, par, line))
            .or_default()
            .push_word(text, left, top, w, h, conf);
    }
    lines
        .into_values()
        .filter(|a| !a.text.trim().is_empty())
        .map(LineAcc::finish)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_words_group_into_lines_in_order() {
        // Two words on line 0, one on line 1; header row + a level-4 row ignored.
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   4\t1\t1\t1\t1\t0\t10\t10\t80\t20\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t10\t10\t30\t20\t95\tHello\n\
                   5\t1\t1\t1\t1\t2\t45\t10\t35\t20\t85\tworld\n\
                   5\t1\t1\t1\t2\t1\t10\t40\t40\t18\t70\tnext\n";
        let lines = parse_tsv_lines(tsv);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].confidence, 90.0);
        assert_eq!(lines[1].text, "next");
        assert!(lines[0].bbox.w >= 70.0); // spans both words
    }

    #[test]
    fn tsv_skips_negative_conf_and_empty_text() {
        let tsv = "5\t1\t1\t1\t1\t1\t0\t0\t10\t10\t-1\t\n\
                   5\t1\t1\t1\t1\t2\t0\t0\t10\t10\t50\t \n";
        assert!(parse_tsv_lines(tsv).is_empty());
    }

    #[test]
    fn missing_language_is_a_clear_error() {
        // "zzz" has no traineddata; init must fail with MissingLanguage.
        match TesseractExtractor::new("zzz") {
            Err(OcrError::MissingLanguage(_)) => {}
            other => panic!("expected MissingLanguage, got {other:?}"),
        }
    }
}
