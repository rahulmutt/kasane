//! Math conversion: MathML (EPUB) and OMML (PPTX) islands → LaTeX.
//! One shared `MathNode` model, two front-ends, one emitter. The island is
//! untrusted input; every path degrades rather than panicking.

pub(crate) mod ast;
mod latex;
mod mathml;
mod omml;
mod symbols;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

pub(crate) use mathml::mathml_to_latex;
pub(crate) use omml::omml_to_latex;

/// Result of converting one math island to LaTeX.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MathConversion {
    /// Best-effort LaTeX, NOT wrapped in `$` / `$$`.
    pub latex: String,
    /// False if at least one sub-expression degraded to the placeholder.
    pub complete: bool,
}

/// Hard cap on island size handed to the tree parser (untrusted-input bound).
pub(crate) const MAX_ISLAND_BYTES: usize = 256 * 1024;
/// Hard cap on math tree recursion depth (untrusted-input bound).
pub(crate) const MAX_MATH_DEPTH: usize = 64;
/// Hard cap on raw XML element nesting inside an island (untrusted-input bound).
///
/// `roxmltree` recurses once per element nesting level while parsing and its
/// only depth guard is for entity references, so nesting must be bounded
/// BEFORE `Document::parse` sees the island: an over-deep island aborts the
/// process with a stack overflow, which is a SIGSEGV-class abort no
/// `catch_unwind` can rescue. `MAX_ISLAND_BYTES` does not bound this —
/// `<mrow></mrow>` is 13 bytes per level, so 256 KB admits ~20,000 levels.
///
/// Deliberately larger than `MAX_MATH_DEPTH` (OMML spends 2-3 XML levels per
/// math level, so equal caps would reject equations `convert` handles today)
/// and far below the overflow threshold: measured at ~5.5 KB of stack per
/// level in a debug build, 128 levels is ~700 KB, comfortable inside a 2 MiB
/// test thread and two orders of magnitude below the release threshold.
pub(crate) const MAX_ISLAND_NESTING: usize = 128;
/// In-band token emitted for any unmapped sub-expression or symbol.
pub(crate) const PLACEHOLDER: &str = "\\mathord{?}";

/// Wrap a captured island in a synthetic root that declares the MathML default
/// namespace, the `mml:` prefix (for prefixed MathML islands), and the OMML `m:`
/// prefix, so `roxmltree` can parse islands whose namespace declarations lived on
/// an ancestor we did not capture. The front-ends match elements by local name, so
/// the exact bindings only need to exist, not to be correct per element.
pub(crate) fn wrap_island(island: &str) -> String {
    format!(
        "<kroot xmlns=\"http://www.w3.org/1998/Math/MathML\" \
         xmlns:mml=\"http://www.w3.org/1998/Math/MathML\" \
         xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\">\
         {island}</kroot>"
    )
}

/// The degraded outcome: just the placeholder, marked incomplete.
pub(crate) fn degraded() -> MathConversion {
    MathConversion {
        latex: PLACEHOLDER.to_string(),
        complete: false,
    }
}

/// Why `capture_island` gave up before reaching the island's matching end tag.
///
/// Every variant means the same thing to a caller — degrade the equation and
/// record a document-level note — but they carry distinct note text because
/// they say different things about the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureError {
    /// Input ended before the island closed (the adapters read with
    /// `check_end_names = false`, so an unclosed `<math>` is not a parse error).
    Unclosed,
    /// The XML reader rejected the input inside the island.
    Reader,
    /// The island exceeded `MAX_ISLAND_BYTES` or `MAX_ISLAND_NESTING`.
    OverBudget,
}

impl CaptureError {
    /// Text for the `Block::Raw` note an adapter emits at the failure site.
    pub(crate) fn note(self) -> &'static str {
        match self {
            CaptureError::Unclosed => "unclosed equation markup",
            CaptureError::Reader => "malformed equation markup",
            CaptureError::OverBudget => "equation too large to convert",
        }
    }
}

/// Re-serialize the element opened by `start` (already read from `reader`),
/// through its matching end tag, and return it as an XML string. Depth-counts
/// same-named nested elements so an inner `<mrow>` inside `<mrow>` (or nested
/// `<m:e>`) does not end capture early, and bounds both the captured byte count
/// and the raw element nesting so the island is within budget *before* the tree
/// parser ever sees it.
///
/// On any abnormal outcome the reader is **rewound** to the position it held on
/// entry — just past `start` — and `Err` is returned. That is deliberate. The
/// alternative, consuming to EOF and returning the partial capture, silently
/// swallows everything after an unclosed `<math>`: the rest of the chapter
/// disappears with no trace, which is exactly the data loss the adapters'
/// lenient readers exist to avoid. Rewinding instead hands the island's own
/// children back to the outer loop, which re-reads them as ordinary document
/// content: the equation is lost (it degrades to `PLACEHOLDER`) and the markup
/// around it degrades to text, but nothing vanishes. Callers pair the degraded
/// equation with `CaptureError::note`, so the outcome is never silent.
///
/// Rewinding cannot loop: the `start` event was already consumed by the caller
/// before this function was entered, so the outer loop always resumes strictly
/// past it and makes progress even when a nested island also fails.
pub(crate) fn capture_island(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart,
) -> Result<String, CaptureError> {
    // `Reader<&[u8]>` is a cheap value (a slice plus parser state), so this
    // clone is the rewind point, not a copy of the document.
    let rewind = reader.clone();
    let local = start.local_name().as_ref().to_vec();
    let mut writer = Writer::new(Vec::new());
    let _ = writer.write_event(Event::Start(start.borrow()));
    // Open elements carrying the island's own local name (end-tag matching)…
    let mut same_name = 1usize;
    // …and open elements of any name (the nesting bound roxmltree needs).
    let mut nesting = 1usize;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let ev = match reader.read_event_into(&mut buf) {
            Ok(ev) => ev,
            Err(_) => {
                *reader = rewind;
                return Err(CaptureError::Reader);
            }
        };
        match &ev {
            Event::Eof => {
                *reader = rewind;
                return Err(CaptureError::Unclosed);
            }
            Event::Start(e) => {
                nesting += 1;
                if nesting > MAX_ISLAND_NESTING {
                    *reader = rewind;
                    return Err(CaptureError::OverBudget);
                }
                if e.local_name().as_ref() == local.as_slice() {
                    same_name += 1;
                }
                let _ = writer.write_event(ev.borrow());
            }
            Event::End(e) => {
                nesting = nesting.saturating_sub(1);
                let _ = writer.write_event(ev.borrow());
                if e.local_name().as_ref() == local.as_slice() {
                    same_name -= 1;
                    if same_name == 0 {
                        break;
                    }
                }
            }
            _ => {
                let _ = writer.write_event(ev.borrow());
            }
        }
        // Capped *during* capture: checking `island.len()` afterwards would
        // already have let one `<math>` force an allocation the size of the
        // whole guarded document.
        if writer.get_ref().len() > MAX_ISLAND_BYTES {
            *reader = rewind;
            return Err(CaptureError::OverBudget);
        }
    }
    match String::from_utf8(writer.into_inner()) {
        Ok(s) => Ok(s),
        Err(_) => {
            *reader = rewind;
            Err(CaptureError::Reader)
        }
    }
}

/// Cheap size/nesting bound for an island handed straight to a front-end.
///
/// `capture_island` enforces the same budget while streaming, but the
/// front-ends are entry points in their own right and must not hand an
/// unbounded island to `roxmltree` — see `MAX_ISLAND_NESTING` for why that is
/// fatal rather than merely slow. Fails closed: input this scanner cannot read
/// is treated as over budget, since a scan that stopped early has not bounded
/// what follows it.
pub(crate) fn island_within_budget(island: &str) -> bool {
    if island.len() > MAX_ISLAND_BYTES {
        return false;
    }
    let mut reader = Reader::from_str(island);
    reader.config_mut().check_end_names = false;
    let mut depth = 0usize;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(_)) => {
                depth += 1;
                if depth > MAX_ISLAND_NESTING {
                    return false;
                }
            }
            Ok(Event::End(_)) => depth = depth.saturating_sub(1),
            Ok(Event::Eof) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{capture_island, island_within_budget, CaptureError, MAX_ISLAND_NESTING};
    use quick_xml::events::Event;
    use quick_xml::Reader;

    /// Capture the first `local_name` element, then drain what the *outer*
    /// loop would see next. The tail is what proves whether the reader was
    /// rewound: on an abnormal capture it must contain the island's own
    /// children plus everything after them, not nothing.
    fn capture_with_tail(
        xml: &str,
        local_name: &[u8],
    ) -> (Result<String, CaptureError>, Vec<String>) {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().expand_empty_elements = true;
        reader.config_mut().check_end_names = false;
        let mut buf = Vec::new();
        let captured = loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) if e.local_name().as_ref() == local_name => {
                    break capture_island(&mut reader, &e);
                }
                Ok(Event::Eof) | Err(_) => return (Ok(String::new()), Vec::new()),
                _ => {}
            }
            buf.clear();
        };
        let mut tail = Vec::new();
        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    tail.push(String::from_utf8_lossy(e.local_name().as_ref()).into_owned());
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        (captured, tail)
    }

    fn capture_first_element(xml: &str, local_name: &[u8]) -> String {
        capture_with_tail(xml, local_name)
            .0
            .expect("island should close normally")
    }

    fn capture_first_math(xml: &str) -> String {
        capture_first_element(xml, b"math")
    }

    #[test]
    fn captures_first_sibling_only() {
        // Capture stops at the first close tag of the opened element;
        // later sibling elements with the same name are not swallowed.
        let xml = "<p>before<math><mrow><mn>1</mn></mrow></math>after<math><mn>2</mn></math></p>";
        let island = capture_first_math(xml);
        assert!(island.contains("<mn>1</mn>"));
        assert!(!island.contains("<mn>2</mn>"));
        assert!(island.trim_start().starts_with("<math"));
        assert!(island.trim_end().ends_with("</math>"));
    }

    #[test]
    fn captures_nested_same_name_element() {
        // Depth counting allows same-named nested elements: an inner <math>
        // inside <math> does not end capture early.
        let xml = "<math><mrow><math><mn>1</mn></math></mrow></math>";
        let island = capture_first_math(xml);
        assert!(island.contains("<math><mn>1</mn></math>"));
        assert!(island.ends_with("</math>"));
        // Verify the outer close tag is present and not truncated at the inner close.
        let close_count = island.matches("</math>").count();
        assert_eq!(close_count, 2, "should have both inner and outer </math>");
    }

    #[test]
    fn captures_nested_mrow() {
        // Nested <mrow> inside <mrow> with depth counting.
        let xml = "<math><mrow><mrow><mn>1</mn></mrow></mrow></math>";
        let island = capture_first_element(xml, b"mrow");
        assert!(island.contains("<mrow><mn>1</mn></mrow>"));
        assert!(island.trim_end().ends_with("</mrow>"));
        // Verify both closes are present.
        let close_count = island.matches("</mrow>").count();
        assert_eq!(close_count, 2, "should have both inner and outer </mrow>");
    }

    #[test]
    fn captures_nested_omml_element() {
        // OMML prefixed names (e.g., <m:e> inside <m:e>) use local-name matching,
        // so depth counting works across the prefix boundary.
        let xml = "<m:e><m:e><m:t>text</m:t></m:e></m:e>";
        let island = capture_first_element(xml, b"e");
        assert!(island.contains("<m:t>text</m:t>"));
        assert!(island.ends_with("</m:e>"));
        let close_count = island.matches("</m:e>").count();
        assert_eq!(close_count, 2, "should have both inner and outer </m:e>");
    }

    #[test]
    fn unclosed_island_reports_unclosed_and_rewinds() {
        // EOF before the close tag: capture fails rather than handing back a
        // partial island, and the reader is rewound so the content the island
        // swallowed is still there for the outer loop.
        let xml = "<math><mrow><mn>1</mn></mrow><p>AFTER</p>";
        let (res, tail) = capture_with_tail(xml, b"math");
        assert_eq!(res, Err(CaptureError::Unclosed));
        assert_eq!(tail, vec!["mrow", "mn", "p"]);
    }

    #[test]
    fn reader_error_reports_reader_and_rewinds() {
        // An unterminated comment is a syntax error, not merely a missing
        // close tag, and must be reported as such.
        let xml = "<math><mrow/><!-- oops";
        let (res, tail) = capture_with_tail(xml, b"math");
        assert_eq!(res, Err(CaptureError::Reader));
        assert_eq!(tail, vec!["mrow"]);
    }

    #[test]
    fn oversized_island_trips_during_capture_and_rewinds() {
        // The cap is enforced while streaming, so this never materializes as
        // one allocation the size of the document.
        let filler = "<mn>9</mn>".repeat(40_000); // ~400 KB, past MAX_ISLAND_BYTES
        let xml = format!("<math>{filler}</math><p>AFTER</p>");
        let (res, tail) = capture_with_tail(&xml, b"math");
        assert_eq!(res, Err(CaptureError::OverBudget));
        assert_eq!(tail.last().map(String::as_str), Some("p"));
    }

    #[test]
    fn overnested_island_trips_during_capture_and_rewinds() {
        let deep = MAX_ISLAND_NESTING + 50;
        let xml = format!(
            "<math>{}<mn>1</mn>{}</math><p>AFTER</p>",
            "<mrow>".repeat(deep),
            "</mrow>".repeat(deep)
        );
        let (res, tail) = capture_with_tail(&xml, b"math");
        assert_eq!(res, Err(CaptureError::OverBudget));
        assert_eq!(tail.last().map(String::as_str), Some("p"));
    }

    #[test]
    fn budget_scan_rejects_deep_and_large_islands() {
        let deep = MAX_ISLAND_NESTING + 50;
        let island = format!(
            "<math>{}<mn>1</mn>{}</math>",
            "<mrow>".repeat(deep),
            "</mrow>".repeat(deep)
        );
        assert!(!island_within_budget(&island));
        assert!(!island_within_budget(&format!(
            "<math><mn>{}</mn></math>",
            "9".repeat(300_000)
        )));
        assert!(island_within_budget("<math><mn>1</mn></math>"));
    }
}
