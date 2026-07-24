//! The OCR seam: a `TextExtractor` behind the PDF and DjVu adapters. The trait
//! and its data types compile on every build; only the Tesseract implementation
//! (`tesseract.rs`) is gated behind the `ocr` feature and links C.

#[cfg(feature = "ocr")]
mod tesseract;
#[cfg(feature = "ocr")]
pub use tesseract::TesseractExtractor;

/// A recovered line's page-space box (Tesseract pixel coords, top-left origin).
/// `h` doubles as the font-size proxy the adapters' heading inference keys on.
#[derive(Clone, Debug, PartialEq)]
pub struct OcrBBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// One OCR'd line: text, box, and Tesseract's mean word confidence (0–100).
#[derive(Clone, Debug, PartialEq)]
pub struct OcrLine {
    pub text: String,
    pub bbox: OcrBBox,
    pub confidence: f32,
}

/// Conservative default: below this mean confidence we prefer the page image.
pub const DEFAULT_MIN_CONFIDENCE: f32 = 60.0;
/// Fewer recovered characters than this reads as "OCR found nothing".
pub const MIN_OCR_CHARS: usize = 8;

/// Tuning for a single OCR run.
#[derive(Clone, Debug)]
pub struct OcrOptions {
    /// Tesseract language string, e.g. "eng" or "eng+deu".
    pub lang: String,
    /// Minimum mean line confidence (0–100) to accept OCR text over the image.
    pub min_confidence: f32,
    /// `--ocr-no-image`: emit OCR text even below `min_confidence`, never an image.
    pub force_text: bool,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            lang: "eng".into(),
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            force_text: false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("OCR engine init failed: {0}")]
    Init(String),
    #[error(
        "OCR language data not found for '{0}'; set TESSDATA_PREFIX or install the traineddata"
    )]
    MissingLanguage(String),
    #[error("OCR failed: {0}")]
    Decode(String),
}

/// Runs an OCR engine over an encoded page image (PNG/JPEG bytes).
pub trait TextExtractor {
    /// Recovered lines in reading order. An empty vec means "no text".
    fn extract(&self, image: &[u8], opts: &OcrOptions) -> Result<Vec<OcrLine>, OcrError>;
}

/// What to do with a text-less page after an OCR attempt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OcrOutcome {
    /// Emit the recovered text as blocks; drop the page image.
    Text,
    /// OCR was not usable: keep the page image as a fallback (with a note).
    ImageFallback,
    /// `--ocr-no-image` and no usable text: emit only the note, no image.
    NoteOnly,
}

/// Decide how to render a text-less page given the OCR result.
pub fn decide(lines: &[OcrLine], opts: &OcrOptions) -> OcrOutcome {
    let chars: usize = lines.iter().map(|l| l.text.trim().chars().count()).sum();
    let has_text = chars >= MIN_OCR_CHARS;
    if opts.force_text {
        return if has_text {
            OcrOutcome::Text
        } else {
            OcrOutcome::NoteOnly
        };
    }
    if !has_text {
        return OcrOutcome::ImageFallback;
    }
    let confs: Vec<f32> = lines
        .iter()
        .filter(|l| !l.text.trim().is_empty())
        .map(|l| l.confidence)
        .collect();
    let mean = if confs.is_empty() {
        0.0
    } else {
        confs.iter().sum::<f32>() / confs.len() as f32
    };
    if mean >= opts.min_confidence {
        OcrOutcome::Text
    } else {
        OcrOutcome::ImageFallback
    }
}

/// Guarded OCR call: an engine panic or error degrades to an empty result, so a
/// bad page falls back rather than crashing the whole conversion.
pub fn extract_guarded(ex: &dyn TextExtractor, image: &[u8], opts: &OcrOptions) -> Vec<OcrLine> {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    match catch_unwind(AssertUnwindSafe(|| ex.extract(image, opts))) {
        Ok(Ok(lines)) => lines,
        _ => Vec::new(),
    }
}

#[cfg(test)]
pub(crate) struct StubExtractor {
    lines: Vec<OcrLine>,
    panic: bool,
}

#[cfg(test)]
impl StubExtractor {
    pub(crate) fn new(lines: Vec<OcrLine>) -> Self {
        Self {
            lines,
            panic: false,
        }
    }
    pub(crate) fn panicking() -> Self {
        Self {
            lines: vec![],
            panic: true,
        }
    }
}

#[cfg(test)]
impl TextExtractor for StubExtractor {
    fn extract(&self, _image: &[u8], _opts: &OcrOptions) -> Result<Vec<OcrLine>, OcrError> {
        if self.panic {
            panic!("stub OCR panic");
        }
        Ok(self.lines.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, h: f32, conf: f32) -> OcrLine {
        OcrLine {
            text: text.into(),
            bbox: OcrBBox {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h,
            },
            confidence: conf,
        }
    }

    #[test]
    fn confident_text_is_accepted() {
        let lines = vec![line("Hello world", 10.0, 88.0)];
        assert_eq!(decide(&lines, &OcrOptions::default()), OcrOutcome::Text);
    }

    #[test]
    fn low_confidence_falls_back_to_image() {
        let lines = vec![line("Hello world", 10.0, 20.0)];
        assert_eq!(
            decide(&lines, &OcrOptions::default()),
            OcrOutcome::ImageFallback
        );
    }

    #[test]
    fn too_little_text_falls_back_to_image() {
        let lines = vec![line("hi", 10.0, 99.0)];
        assert_eq!(
            decide(&lines, &OcrOptions::default()),
            OcrOutcome::ImageFallback
        );
    }

    #[test]
    fn force_text_emits_below_threshold_but_note_only_when_empty() {
        let opts = OcrOptions {
            force_text: true,
            ..Default::default()
        };
        assert_eq!(
            decide(&[line("Hello world", 10.0, 5.0)], &opts),
            OcrOutcome::Text
        );
        assert_eq!(decide(&[], &opts), OcrOutcome::NoteOnly);
    }

    #[test]
    fn guarded_extract_swallows_panic() {
        let stub = StubExtractor::panicking();
        let out = extract_guarded(&stub, b"", &OcrOptions::default());
        assert!(out.is_empty());
    }

    #[test]
    fn guarded_extract_returns_lines() {
        let stub = StubExtractor::new(vec![line("Hello world", 10.0, 90.0)]);
        let out = extract_guarded(&stub, b"", &OcrOptions::default());
        assert_eq!(out.len(), 1);
    }
}
