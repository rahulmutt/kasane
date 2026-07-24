#![allow(dead_code)] // removed in Task 6 once adapters wire this in

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

#[allow(unused_imports)] // used from Task 5's EPUB wiring; allow removed in Task 6
pub(crate) use mathml::mathml_to_latex;
#[allow(unused_imports)] // used from Task 6's PPTX wiring; allow removed in Task 6
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
        let ev = match reader.read_event_into(&mut buf) {
            Ok(ev) => ev.into_owned(),
            Err(_) => break,
        };
        match &ev {
            Event::Start(e) if e.local_name().as_ref() == local.as_slice() => {
                depth += 1;
                let _ = writer.write_event(ev.clone());
            }
            Event::End(e) if e.local_name().as_ref() == local.as_slice() => {
                depth -= 1;
                let _ = writer.write_event(ev.clone());
                if depth == 0 {
                    break;
                }
            }
            Event::Eof => break,
            _ => {
                let _ = writer.write_event(ev.clone());
            }
        }
    }
    String::from_utf8(writer.into_inner()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::capture_island;
    use quick_xml::events::Event;
    use quick_xml::Reader;

    fn capture_first_math(xml: &str) -> String {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().expand_empty_elements = true;
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) if e.local_name().as_ref() == b"math" => {
                    return capture_island(&mut reader, &e);
                }
                Ok(Event::Eof) => return String::new(),
                _ => {}
            }
            buf.clear();
        }
    }

    #[test]
    fn captures_nested_island_only() {
        // Two <math> at flow level; capture must stop at the FIRST close, and a
        // nested same-named element must not end capture early.
        let xml = "<p>before<math><mrow><mn>1</mn></mrow></math>after<math><mn>2</mn></math></p>";
        let island = capture_first_math(xml);
        assert!(island.contains("<mn>1</mn>"));
        assert!(!island.contains("<mn>2</mn>"));
        assert!(island.trim_start().starts_with("<math"));
        assert!(island.trim_end().ends_with("</math>"));
    }
}
