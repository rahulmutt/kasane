#![allow(dead_code)] // removed in Task 6 once adapters wire this in

//! Math conversion: MathML (EPUB) and OMML (PPTX) islands → LaTeX.
//! One shared `MathNode` model, two front-ends, one emitter. The island is
//! untrusted input; every path degrades rather than panicking.

pub(crate) mod ast;
mod latex;
mod mathml;
mod symbols;

#[allow(unused_imports)] // used from Task 5's EPUB wiring; allow removed in Task 6
pub(crate) use mathml::mathml_to_latex;

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
