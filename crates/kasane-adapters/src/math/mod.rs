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

/// Re-serialize the element opened by `start` (already read from `reader`),
/// through its matching end tag, and return it as an XML string. Depth-counts
/// same-named nested elements so an inner `<mrow>` inside `<mrow>` (or nested
/// `<m:e>`) does not end capture early. On a reader error or EOF, returns what
/// was captured so far — the front-end then degrades on the malformed island.
pub(crate) fn capture_island(reader: &mut Reader<&[u8]>, start: &BytesStart) -> String {
    let local = start.local_name().as_ref().to_vec();
    let mut writer = Writer::new(Vec::new());
    let _ = writer.write_event(Event::Start(start.borrow()));
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(ev) => match &ev {
                Event::Start(e) if e.local_name().as_ref() == local.as_slice() => {
                    depth += 1;
                    let _ = writer.write_event(ev.borrow());
                }
                Event::End(e) if e.local_name().as_ref() == local.as_slice() => {
                    depth -= 1;
                    let _ = writer.write_event(ev.borrow());
                    if depth == 0 {
                        break;
                    }
                }
                Event::Eof => break,
                _ => {
                    let _ = writer.write_event(ev.borrow());
                }
            },
            Err(_) => break,
        }
    }
    String::from_utf8(writer.into_inner()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::capture_island;
    use quick_xml::events::Event;
    use quick_xml::Reader;

    fn capture_first_element(xml: &str, local_name: &[u8]) -> String {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().expand_empty_elements = true;
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) if e.local_name().as_ref() == local_name => {
                    return capture_island(&mut reader, &e);
                }
                Ok(Event::Eof) => return String::new(),
                _ => {}
            }
            buf.clear();
        }
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
    fn handles_truncated_input() {
        // Unclosed element (EOF before close tag) should return partial capture
        // without panicking.
        let xml = "<math><mrow><mn>1</mn></mrow>";
        let island = capture_first_math(xml);
        // Should have captured what was available.
        assert!(island.contains("<mrow>"));
        assert!(island.contains("<mn>1</mn>"));
        // The partial capture does not panic and does not hang.
    }

    #[test]
    fn handles_malformed_close() {
        // Mismatched close tag should degrade gracefully.
        let xml = "<math><mrow><mn>1</mn></mrow></wrong>";
        let island = capture_first_math(xml);
        // Should have captured the mrow and its contents before the mismatched close.
        assert!(island.contains("<mrow>"));
        assert!(island.contains("<mn>1</mn>"));
    }
}
