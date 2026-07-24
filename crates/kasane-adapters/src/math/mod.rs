#![allow(dead_code)] // removed in Task 6 once adapters wire this in

//! Math conversion: MathML (EPUB) and OMML (PPTX) islands → LaTeX.
//! One shared `MathNode` model, two front-ends, one emitter. The island is
//! untrusted input; every path degrades rather than panicking.

pub(crate) mod ast;
mod latex;
mod symbols;

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
