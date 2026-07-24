# Math → LaTeX Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert MathML (EPUB) and OMML (PPTX) equations into LaTeX, populating the IR's existing `Inline::Math` / `Block::MathBlock` slots so math survives conversion instead of being dropped.

**Architecture:** A new shared `math/` module in `kasane-adapters` with a format-agnostic `MathNode` model. Two front-ends (`mathml.rs`, `omml.rs`) map an XML *island* into `MathNode`; one emitter (`latex.rs`) renders `MathNode → LaTeX` and is the single home for the degradation policy. The EPUB and PPTX adapters isolate a math island from their streaming `quick-xml` parse (shared `capture_island` helper), hand it to the matching front-end, and write the resulting LaTeX into the IR they already build. No `kasane-ir` or `kasane-writer` changes.

**Tech Stack:** Rust, `quick-xml` 0.41 (already a dep, streaming parse + island re-serialization), `roxmltree` (new dep, read-only tree for island recursion).

## Global Constraints

- Every change ships green under `mise run lint && mise run test` (fmt check + `clippy --all-targets -D warnings` + all tests).
- No new IR types; no writer changes. LaTeX strings flow into the existing `Inline::Math(String)` and `Block::MathBlock(String)`.
- The math island is untrusted input: every parse/recursion path must degrade (never panic, never abort the document) on malformed, oversized, or too-deep input.
- Degradation is best-effort + note: emit what maps; each unmapped sub-expression renders the in-band token `\mathord{?}` and sets `complete = false`; a partial **display** equation also emits an adjacent `Block::Raw { note: "equation partially converted" }`. Inline partials self-mark via the token only (no inline note type is added).
- Presentation MathML only (no Content MathML). Documented v1 construct subset (see spec §3); anything outside → `MathNode::Unsupported`.
- Follow existing module conventions: `pub(crate)` internals, one responsibility per file.

---

## File Structure

- Create: `crates/kasane-adapters/src/math/mod.rs` — module wiring, `MathConversion`, shared consts, `capture_island` helper.
- Create: `crates/kasane-adapters/src/math/ast.rs` — `MathNode`, `AccentKind`.
- Create: `crates/kasane-adapters/src/math/symbols.rs` — Unicode → LaTeX symbol table + `map_text`.
- Create: `crates/kasane-adapters/src/math/latex.rs` — `MathNode → LaTeX` emitter + degradation.
- Create: `crates/kasane-adapters/src/math/mathml.rs` — `mathml_to_latex` (EPUB front-end).
- Create: `crates/kasane-adapters/src/math/omml.rs` — `omml_to_latex` (PPTX front-end).
- Modify: `crates/kasane-adapters/src/lib.rs` — add `mod math;`.
- Modify: `crates/kasane-adapters/Cargo.toml` — add `roxmltree` dep.
- Modify: `crates/kasane-adapters/src/epub/xhtml.rs` — `<math>` handling.
- Modify: `crates/kasane-adapters/src/pptx/slide.rs` — `m:oMath` / `m:oMathPara` handling.
- Modify: `README.md`, `AGENTS.md` — docs.

---

## Task 1: Math model, symbol table, and LaTeX emitter

**Files:**
- Create: `crates/kasane-adapters/src/math/ast.rs`
- Create: `crates/kasane-adapters/src/math/symbols.rs`
- Create: `crates/kasane-adapters/src/math/latex.rs`
- Create: `crates/kasane-adapters/src/math/mod.rs`
- Modify: `crates/kasane-adapters/src/lib.rs` (add `mod math;`)

**Interfaces:**
- Produces: `math::MathConversion { pub latex: String, pub complete: bool }`; `math::ast::{MathNode, AccentKind}`; `math::latex::to_conversion(&MathNode) -> MathConversion`; consts `math::MAX_ISLAND_BYTES`, `math::MAX_MATH_DEPTH`, `math::PLACEHOLDER`.

- [ ] **Step 1: Create the module skeleton and register it**

Create `crates/kasane-adapters/src/math/mod.rs`:

```rust
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
```

Add to `crates/kasane-adapters/src/lib.rs` after the existing `mod guard;` line (keep alphabetical grouping loose — place it before `mod mobi;`):

```rust
mod math;
```

- [ ] **Step 2: Write the failing emitter tests**

Create `crates/kasane-adapters/src/math/latex.rs` with only a `tests` module first:

```rust
#[cfg(test)]
mod tests {
    use super::to_conversion;
    use crate::math::ast::{AccentKind, MathNode};

    fn ident(s: &str) -> MathNode {
        MathNode::Ident(s.to_string())
    }
    fn num(s: &str) -> MathNode {
        MathNode::Number(s.to_string())
    }

    #[test]
    fn fraction_renders() {
        let n = MathNode::Frac(Box::new(num("1")), Box::new(num("2")));
        let c = to_conversion(&n);
        assert_eq!(c.latex, "\\frac{1}{2}");
        assert!(c.complete);
    }

    #[test]
    fn subsup_renders() {
        let n = MathNode::SubSup(Box::new(ident("x")), Box::new(num("0")), Box::new(num("2")));
        assert_eq!(to_conversion(&n).latex, "{x}_{0}^{2}");
    }

    #[test]
    fn sqrt_and_root_render() {
        assert_eq!(
            to_conversion(&MathNode::Sqrt(Box::new(num("2")))).latex,
            "\\sqrt{2}"
        );
        assert_eq!(
            to_conversion(&MathNode::Root(Box::new(ident("x")), Box::new(num("3")))).latex,
            "\\sqrt[3]{x}"
        );
    }

    #[test]
    fn fenced_uses_left_right() {
        let n = MathNode::Fenced {
            open: "(".into(),
            close: ")".into(),
            body: Box::new(ident("x")),
        };
        assert_eq!(to_conversion(&n).latex, "\\left(x\\right)");
    }

    #[test]
    fn nary_sum_with_limits() {
        let n = MathNode::Nary {
            op: "\\sum".into(),
            sub: Some(Box::new(ident("i"))),
            sup: Some(Box::new(ident("n"))),
            body: Box::new(ident("i")),
        };
        assert_eq!(to_conversion(&n).latex, "\\sum_{i}^{n} i");
    }

    #[test]
    fn matrix_renders_pmatrix() {
        let n = MathNode::Matrix(vec![
            vec![num("1"), num("2")],
            vec![num("3"), num("4")],
        ]);
        assert_eq!(
            to_conversion(&n).latex,
            "\\begin{pmatrix}1 & 2 \\\\ 3 & 4\\end{pmatrix}"
        );
    }

    #[test]
    fn accent_renders() {
        let n = MathNode::Accent {
            kind: AccentKind::Hat,
            base: Box::new(ident("x")),
        };
        assert_eq!(to_conversion(&n).latex, "\\hat{x}");
    }

    #[test]
    fn greek_symbol_maps_via_table() {
        // An identifier carrying a Greek letter maps to its LaTeX command.
        assert_eq!(to_conversion(&ident("α")).latex, "\\alpha");
    }

    #[test]
    fn unsupported_emits_placeholder_and_marks_incomplete() {
        let c = to_conversion(&MathNode::Row(vec![ident("x"), MathNode::Unsupported]));
        assert_eq!(c.latex, "x \\mathord{?}");
        assert!(!c.complete);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p kasane-adapters math::latex`
Expected: FAIL — `to_conversion` / `MathNode` / `AccentKind` not found.

- [ ] **Step 4: Write `ast.rs`**

Create `crates/kasane-adapters/src/math/ast.rs`:

```rust
//! Format-agnostic math model. Both front-ends target this; the emitter
//! consumes it. Constructs outside the v1 subset become `Unsupported`.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AccentKind {
    Hat,
    Bar,
    Vec,
    Tilde,
    Dot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MathNode {
    Row(Vec<MathNode>),
    Ident(String),
    Number(String),
    Op(String),
    Text(String),
    Frac(Box<MathNode>, Box<MathNode>),
    Sup(Box<MathNode>, Box<MathNode>),
    Sub(Box<MathNode>, Box<MathNode>),
    SubSup(Box<MathNode>, Box<MathNode>, Box<MathNode>),
    Sqrt(Box<MathNode>),
    Root(Box<MathNode>, Box<MathNode>),
    Fenced {
        open: String,
        close: String,
        body: Box<MathNode>,
    },
    Nary {
        op: String,
        sub: Option<Box<MathNode>>,
        sup: Option<Box<MathNode>>,
        body: Box<MathNode>,
    },
    Matrix(Vec<Vec<MathNode>>),
    Accent {
        kind: AccentKind,
        base: Box<MathNode>,
    },
    Unsupported,
}
```

- [ ] **Step 5: Write `symbols.rs`**

Create `crates/kasane-adapters/src/math/symbols.rs`:

```rust
//! Unicode → LaTeX symbol mapping, shared by both front-ends via the emitter.

/// LaTeX command for a single Unicode char, if known. Extend as needed.
pub(crate) fn symbol(c: char) -> Option<&'static str> {
    Some(match c {
        // Greek
        'α' => "\\alpha",
        'β' => "\\beta",
        'γ' => "\\gamma",
        'δ' => "\\delta",
        'θ' => "\\theta",
        'λ' => "\\lambda",
        'μ' => "\\mu",
        'π' => "\\pi",
        'σ' => "\\sigma",
        'φ' => "\\phi",
        'ω' => "\\omega",
        // relations / operators
        '≤' => "\\leq",
        '≥' => "\\geq",
        '≠' => "\\neq",
        '≈' => "\\approx",
        '×' => "\\times",
        '÷' => "\\div",
        '±' => "\\pm",
        '⋅' => "\\cdot",
        '∈' => "\\in",
        '∞' => "\\infty",
        '→' => "\\to",
        '∂' => "\\partial",
        // large operators (used as Op text; Nary carries its own op string)
        '∑' => "\\sum",
        '∏' => "\\prod",
        '∫' => "\\int",
        '√' => "\\sqrt",
        _ => return None,
    })
}

/// Render operator/identifier text to LaTeX. Known symbols map to commands
/// (space-separated so `\alpha x` stays two tokens); ASCII passes through;
/// an unmapped non-ASCII char emits the placeholder and marks incomplete.
pub(crate) fn map_text(s: &str, complete: &mut bool) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if let Some(cmd) = symbol(c) {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            out.push_str(cmd);
            out.push(' ');
        } else if c.is_ascii_graphic() || c.is_ascii_whitespace() {
            out.push(c);
        } else {
            out.push_str(super::PLACEHOLDER);
            *complete = false;
        }
    }
    out.trim().to_string()
}
```

- [ ] **Step 6: Write the emitter above the tests in `latex.rs`**

Insert at the top of `crates/kasane-adapters/src/math/latex.rs` (above the `tests` module):

```rust
use crate::math::ast::{AccentKind, MathNode};
use crate::math::symbols::map_text;
use crate::math::{MathConversion, PLACEHOLDER};

/// Render a `MathNode` tree to a `MathConversion`.
pub(crate) fn to_conversion(node: &MathNode) -> MathConversion {
    let mut out = String::new();
    let mut complete = true;
    render(node, &mut out, &mut complete);
    MathConversion {
        latex: out.trim().to_string(),
        complete,
    }
}

fn render(node: &MathNode, out: &mut String, complete: &mut bool) {
    match node {
        MathNode::Row(items) => {
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                render(it, out, complete);
            }
        }
        MathNode::Ident(s) | MathNode::Op(s) => out.push_str(&map_text(s, complete)),
        MathNode::Number(s) => out.push_str(s),
        MathNode::Text(s) => {
            out.push_str("\\text{");
            out.push_str(s);
            out.push('}');
        }
        MathNode::Frac(n, d) => {
            out.push_str("\\frac{");
            render(n, out, complete);
            out.push_str("}{");
            render(d, out, complete);
            out.push('}');
        }
        MathNode::Sup(b, s) => script(out, complete, b, None, Some(s)),
        MathNode::Sub(b, s) => script(out, complete, b, Some(s), None),
        MathNode::SubSup(b, sub, sup) => script(out, complete, b, Some(sub), Some(sup)),
        MathNode::Sqrt(x) => {
            out.push_str("\\sqrt{");
            render(x, out, complete);
            out.push('}');
        }
        MathNode::Root(x, idx) => {
            out.push_str("\\sqrt[");
            render(idx, out, complete);
            out.push_str("]{");
            render(x, out, complete);
            out.push('}');
        }
        MathNode::Fenced { open, close, body } => {
            out.push_str("\\left");
            out.push_str(fence(open));
            render(body, out, complete);
            out.push_str("\\right");
            out.push_str(fence(close));
        }
        MathNode::Nary {
            op,
            sub,
            sup,
            body,
        } => {
            out.push_str(op);
            if let Some(s) = sub {
                out.push_str("_{");
                render(s, out, complete);
                out.push('}');
            }
            if let Some(s) = sup {
                out.push_str("^{");
                render(s, out, complete);
                out.push('}');
            }
            out.push(' ');
            render(body, out, complete);
        }
        MathNode::Matrix(rows) => {
            out.push_str("\\begin{pmatrix}");
            for (r, row) in rows.iter().enumerate() {
                if r > 0 {
                    out.push_str(" \\\\ ");
                }
                for (c, cell) in row.iter().enumerate() {
                    if c > 0 {
                        out.push_str(" & ");
                    }
                    render(cell, out, complete);
                }
            }
            out.push_str("\\end{pmatrix}");
        }
        MathNode::Accent { kind, base } => {
            out.push_str(accent_cmd(kind));
            out.push('{');
            render(base, out, complete);
            out.push('}');
        }
        MathNode::Unsupported => {
            out.push_str(PLACEHOLDER);
            *complete = false;
        }
    }
}

fn script(
    out: &mut String,
    complete: &mut bool,
    base: &MathNode,
    sub: Option<&MathNode>,
    sup: Option<&MathNode>,
) {
    out.push('{');
    render(base, out, complete);
    out.push('}');
    if let Some(s) = sub {
        out.push_str("_{");
        render(s, out, complete);
        out.push('}');
    }
    if let Some(s) = sup {
        out.push_str("^{");
        render(s, out, complete);
        out.push('}');
    }
}

fn fence(s: &str) -> &str {
    match s {
        "" => ".",
        "{" => "\\{",
        "}" => "\\}",
        "⟨" => "\\langle",
        "⟩" => "\\rangle",
        other => other,
    }
}

fn accent_cmd(k: &AccentKind) -> &'static str {
    match k {
        AccentKind::Hat => "\\hat",
        AccentKind::Bar => "\\bar",
        AccentKind::Vec => "\\vec",
        AccentKind::Tilde => "\\tilde",
        AccentKind::Dot => "\\dot",
    }
}
```

- [ ] **Step 7: Run the emitter tests to verify they pass**

Run: `cargo test -p kasane-adapters math::latex`
Expected: PASS (9 tests).

- [ ] **Step 8: Lint and commit**

Run: `mise run lint`
Expected: clean (fmt + clippy, no warnings).

```bash
git add crates/kasane-adapters/src/math/ crates/kasane-adapters/src/lib.rs
git commit -m "feat(math): MathNode model, symbol table, and LaTeX emitter"
```

---

## Task 2: MathML front-end

**Files:**
- Create: `crates/kasane-adapters/src/math/mathml.rs`
- Modify: `crates/kasane-adapters/src/math/mod.rs` (declare module, re-export, add `wrap_island`)
- Modify: `crates/kasane-adapters/Cargo.toml` (add `roxmltree`)

**Interfaces:**
- Consumes: `math::ast::MathNode`, `math::latex::to_conversion`, consts from Task 1.
- Produces: `math::mathml_to_latex(island: &str) -> MathConversion`; `math::wrap_island(island: &str) -> String` (shared by Task 3).

- [ ] **Step 1: Add the `roxmltree` dependency**

In `crates/kasane-adapters/Cargo.toml`, under `[dependencies]`, after the `quick-xml = "0.41"` line:

```toml
roxmltree = "0.20"
```

- [ ] **Step 2: Add the shared island wrapper to `mod.rs`**

Append to `crates/kasane-adapters/src/math/mod.rs`:

```rust
mod mathml;
pub use mathml::mathml_to_latex;

/// Wrap a captured island in a synthetic root that declares both the MathML
/// default namespace and the OMML `m:` prefix, so `roxmltree` can parse islands
/// whose namespace declarations lived on an ancestor we did not capture. The
/// front-ends match elements by local name, so the exact bindings only need to
/// exist, not to be correct per element.
pub(crate) fn wrap_island(island: &str) -> String {
    format!(
        "<kroot xmlns=\"http://www.w3.org/1998/Math/MathML\" \
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
```

- [ ] **Step 3: Write the failing MathML tests**

Create `crates/kasane-adapters/src/math/mathml.rs` with a tests module:

```rust
#[cfg(test)]
mod tests {
    use super::mathml_to_latex;

    #[test]
    fn superscript_power() {
        let c = mathml_to_latex("<math><msup><mi>x</mi><mn>2</mn></msup></math>");
        assert_eq!(c.latex, "{x}^{2}");
        assert!(c.complete);
    }

    #[test]
    fn fraction() {
        let c = mathml_to_latex("<math><mfrac><mn>1</mn><mn>2</mn></mfrac></math>");
        assert_eq!(c.latex, "\\frac{1}{2}");
    }

    #[test]
    fn square_root() {
        let c = mathml_to_latex("<math><msqrt><mn>2</mn></msqrt></math>");
        assert_eq!(c.latex, "\\sqrt{2}");
    }

    #[test]
    fn greek_identifier_maps() {
        let c = mathml_to_latex("<math><mi>α</mi></math>");
        assert_eq!(c.latex, "\\alpha");
    }

    #[test]
    fn default_namespaced_island_parses() {
        // A real EPUB <math> redeclares the MathML default namespace on itself.
        let c = mathml_to_latex(
            "<math xmlns=\"http://www.w3.org/1998/Math/MathML\"><mn>3</mn></math>",
        );
        assert_eq!(c.latex, "3");
        assert!(c.complete);
    }

    #[test]
    fn content_mathml_is_unsupported() {
        let c = mathml_to_latex("<math><apply><ci>x</ci></apply></math>");
        assert_eq!(c.latex, "\\mathord{?}");
        assert!(!c.complete);
    }

    #[test]
    fn malformed_island_degrades_without_panic() {
        let c = mathml_to_latex("<math><mfrac><mn>1</mn></math"); // truncated
        assert!(!c.complete);
    }

    #[test]
    fn oversized_island_degrades() {
        let big = format!("<math><mn>{}</mn></math>", "9".repeat(300_000));
        let c = mathml_to_latex(&big);
        assert_eq!(c.latex, "\\mathord{?}");
        assert!(!c.complete);
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p kasane-adapters math::mathml`
Expected: FAIL — `mathml_to_latex` not found.

- [ ] **Step 5: Write the MathML front-end above the tests**

Insert at the top of `crates/kasane-adapters/src/math/mathml.rs`:

```rust
use crate::math::ast::{AccentKind, MathNode};
use crate::math::latex::to_conversion;
use crate::math::{degraded, wrap_island, MathConversion, MAX_ISLAND_BYTES, MAX_MATH_DEPTH};
use roxmltree::{Document, Node};

/// Convert a Presentation-MathML `<math>…</math>` island to LaTeX.
pub fn mathml_to_latex(island: &str) -> MathConversion {
    if island.len() > MAX_ISLAND_BYTES {
        return degraded();
    }
    let wrapped = wrap_island(island);
    let doc = match Document::parse(&wrapped) {
        Ok(d) => d,
        Err(_) => return degraded(),
    };
    let math = doc.root_element().children().find(Node::is_element);
    let node = match math {
        Some(m) => convert(m, 0),
        None => MathNode::Unsupported,
    };
    to_conversion(&node)
}

/// Accent operator characters that turn `<mover>` into an accent rather than a
/// superscript.
fn accent_for(op: &str) -> Option<AccentKind> {
    match op.trim() {
        "^" | "ˆ" | "\u{0302}" => Some(AccentKind::Hat),
        "¯" | "‾" | "\u{0304}" => Some(AccentKind::Bar),
        "→" | "⃗" | "\u{20D7}" => Some(AccentKind::Vec),
        "~" | "˜" | "\u{0303}" => Some(AccentKind::Tilde),
        "." | "˙" | "\u{0307}" => Some(AccentKind::Dot),
        _ => None,
    }
}

fn convert(n: Node, depth: usize) -> MathNode {
    if depth > MAX_MATH_DEPTH {
        return MathNode::Unsupported;
    }
    let kids: Vec<Node> = n.children().filter(Node::is_element).collect();
    match n.tag_name().name() {
        "math" | "mrow" | "mstyle" | "mpadded" => row(&kids, depth),
        "mi" => MathNode::Ident(text(n)),
        "mn" => MathNode::Number(text(n)),
        "mo" => MathNode::Op(text(n)),
        "mtext" => MathNode::Text(text(n)),
        "mfrac" if kids.len() == 2 => MathNode::Frac(
            Box::new(convert(kids[0], depth + 1)),
            Box::new(convert(kids[1], depth + 1)),
        ),
        "msup" if kids.len() == 2 => MathNode::Sup(
            Box::new(convert(kids[0], depth + 1)),
            Box::new(convert(kids[1], depth + 1)),
        ),
        "msub" if kids.len() == 2 => MathNode::Sub(
            Box::new(convert(kids[0], depth + 1)),
            Box::new(convert(kids[1], depth + 1)),
        ),
        "msubsup" | "munderover" if kids.len() == 3 => MathNode::SubSup(
            Box::new(convert(kids[0], depth + 1)),
            Box::new(convert(kids[1], depth + 1)),
            Box::new(convert(kids[2], depth + 1)),
        ),
        "munder" if kids.len() == 2 => MathNode::Sub(
            Box::new(convert(kids[0], depth + 1)),
            Box::new(convert(kids[1], depth + 1)),
        ),
        "mover" if kids.len() == 2 => {
            let over_op = kids[1].tag_name().name() == "mo";
            match accent_for(&text(kids[1])) {
                Some(kind) if over_op => MathNode::Accent {
                    kind,
                    base: Box::new(convert(kids[0], depth + 1)),
                },
                _ => MathNode::Sup(
                    Box::new(convert(kids[0], depth + 1)),
                    Box::new(convert(kids[1], depth + 1)),
                ),
            }
        }
        "msqrt" => MathNode::Sqrt(Box::new(row(&kids, depth))),
        "mroot" if kids.len() == 2 => MathNode::Root(
            Box::new(convert(kids[0], depth + 1)),
            Box::new(convert(kids[1], depth + 1)),
        ),
        "mfenced" => MathNode::Fenced {
            open: n.attribute("open").unwrap_or("(").to_string(),
            close: n.attribute("close").unwrap_or(")").to_string(),
            body: Box::new(row(&kids, depth)),
        },
        "mtable" => matrix(&kids, depth),
        _ => MathNode::Unsupported,
    }
}

fn row(kids: &[Node], depth: usize) -> MathNode {
    let items: Vec<MathNode> = kids.iter().map(|k| convert(*k, depth + 1)).collect();
    if items.len() == 1 {
        items.into_iter().next().unwrap()
    } else {
        MathNode::Row(items)
    }
}

fn matrix(rows: &[Node], depth: usize) -> MathNode {
    let mut out = Vec::new();
    for tr in rows.iter().filter(|r| r.tag_name().name() == "mtr") {
        let cells: Vec<MathNode> = tr
            .children()
            .filter(Node::is_element)
            .filter(|c| c.tag_name().name() == "mtd")
            .map(|c| {
                let ck: Vec<Node> = c.children().filter(Node::is_element).collect();
                row(&ck, depth + 1)
            })
            .collect();
        out.push(cells);
    }
    if out.is_empty() {
        MathNode::Unsupported
    } else {
        MathNode::Matrix(out)
    }
}

/// Concatenated text content of an element (MathML tokens hold plain text).
fn text(n: Node) -> String {
    n.descendants()
        .filter_map(|d| d.text())
        .collect::<String>()
        .trim()
        .to_string()
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p kasane-adapters math::mathml`
Expected: PASS (8 tests).

- [ ] **Step 7: Lint and commit**

Run: `mise run lint`
Expected: clean.

```bash
git add crates/kasane-adapters/src/math/mathml.rs crates/kasane-adapters/src/math/mod.rs crates/kasane-adapters/Cargo.toml Cargo.lock
git commit -m "feat(math): MathML front-end (EPUB) → MathNode → LaTeX"
```

---

## Task 3: OMML front-end

**Files:**
- Create: `crates/kasane-adapters/src/math/omml.rs`
- Modify: `crates/kasane-adapters/src/math/mod.rs` (declare module, re-export)

**Interfaces:**
- Consumes: `math::ast::MathNode`, `math::latex::to_conversion`, `math::{wrap_island, degraded}`, consts.
- Produces: `math::omml_to_latex(island: &str) -> MathConversion`.

- [ ] **Step 1: Declare the module in `mod.rs`**

Append to `crates/kasane-adapters/src/math/mod.rs`:

```rust
mod omml;
pub use omml::omml_to_latex;
```

- [ ] **Step 2: Write the failing OMML tests (parity with MathML cases)**

Create `crates/kasane-adapters/src/math/omml.rs` with a tests module:

```rust
#[cfg(test)]
mod tests {
    use super::omml_to_latex;

    // Islands use the `m:` prefix as in real PPTX; the wrapper declares it.
    #[test]
    fn superscript_power_parity() {
        let c = omml_to_latex(
            "<m:oMath><m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e>\
             <m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath>",
        );
        assert_eq!(c.latex, "{x}^{2}");
        assert!(c.complete);
    }

    #[test]
    fn fraction_parity() {
        let c = omml_to_latex(
            "<m:oMath><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num>\
             <m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath>",
        );
        assert_eq!(c.latex, "\\frac{1}{2}");
    }

    #[test]
    fn radical_is_sqrt_without_degree() {
        let c = omml_to_latex(
            "<m:oMath><m:rad><m:deg/><m:e><m:r><m:t>2</m:t></m:r></m:e></m:rad></m:oMath>",
        );
        assert_eq!(c.latex, "\\sqrt{2}");
    }

    #[test]
    fn nary_sum_with_limits() {
        let c = omml_to_latex(
            "<m:oMath><m:nary><m:naryPr><m:chr m:val=\"∑\"/></m:naryPr>\
             <m:sub><m:r><m:t>i</m:t></m:r></m:sub>\
             <m:sup><m:r><m:t>n</m:t></m:r></m:sup>\
             <m:e><m:r><m:t>i</m:t></m:r></m:e></m:nary></m:oMath>",
        );
        assert_eq!(c.latex, "\\sum_{i}^{n} i");
    }

    #[test]
    fn delimiter_becomes_fenced() {
        let c = omml_to_latex(
            "<m:oMath><m:d><m:e><m:r><m:t>x</m:t></m:r></m:e></m:d></m:oMath>",
        );
        assert_eq!(c.latex, "\\left(x\\right)");
    }

    #[test]
    fn unknown_element_is_unsupported() {
        let c = omml_to_latex("<m:oMath><m:weird/></m:oMath>");
        assert_eq!(c.latex, "\\mathord{?}");
        assert!(!c.complete);
    }

    #[test]
    fn malformed_island_degrades_without_panic() {
        let c = omml_to_latex("<m:oMath><m:f><m:num></m:oMath"); // truncated
        assert!(!c.complete);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p kasane-adapters math::omml`
Expected: FAIL — `omml_to_latex` not found.

- [ ] **Step 4: Write the OMML front-end above the tests**

Insert at the top of `crates/kasane-adapters/src/math/omml.rs`:

```rust
use crate::math::ast::MathNode;
use crate::math::latex::to_conversion;
use crate::math::{degraded, wrap_island, MathConversion, MAX_ISLAND_BYTES, MAX_MATH_DEPTH};
use roxmltree::{Document, Node};

/// Convert an OMML `<m:oMath>` / `<m:oMathPara>` island to LaTeX.
pub fn omml_to_latex(island: &str) -> MathConversion {
    if island.len() > MAX_ISLAND_BYTES {
        return degraded();
    }
    let wrapped = wrap_island(island);
    let doc = match Document::parse(&wrapped) {
        Ok(d) => d,
        Err(_) => return degraded(),
    };
    let root = doc.root_element().children().find(Node::is_element);
    let node = match root {
        Some(m) => convert(m, 0),
        None => MathNode::Unsupported,
    };
    to_conversion(&node)
}

/// Element children of `n` with local name `name`.
fn child<'a>(n: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    n.children()
        .filter(Node::is_element)
        .find(|c| c.tag_name().name() == name)
}

fn convert(n: Node, depth: usize) -> MathNode {
    if depth > MAX_MATH_DEPTH {
        return MathNode::Unsupported;
    }
    match n.tag_name().name() {
        "oMathPara" | "oMath" | "e" | "num" | "den" | "sub" | "sup" | "deg" => {
            row(n, depth)
        }
        // A run: gather its <m:t> text. Operators and identifiers both arrive
        // as run text; Ident lets the symbol table map Greek/operators.
        "r" => MathNode::Ident(run_text(n)),
        "f" => match (child(n, "num"), child(n, "den")) {
            (Some(num), Some(den)) => MathNode::Frac(
                Box::new(convert(num, depth + 1)),
                Box::new(convert(den, depth + 1)),
            ),
            _ => MathNode::Unsupported,
        },
        "sSup" => match (child(n, "e"), child(n, "sup")) {
            (Some(e), Some(s)) => MathNode::Sup(
                Box::new(convert(e, depth + 1)),
                Box::new(convert(s, depth + 1)),
            ),
            _ => MathNode::Unsupported,
        },
        "sSub" => match (child(n, "e"), child(n, "sub")) {
            (Some(e), Some(s)) => MathNode::Sub(
                Box::new(convert(e, depth + 1)),
                Box::new(convert(s, depth + 1)),
            ),
            _ => MathNode::Unsupported,
        },
        "sSubSup" => match (child(n, "e"), child(n, "sub"), child(n, "sup")) {
            (Some(e), Some(sb), Some(sp)) => MathNode::SubSup(
                Box::new(convert(e, depth + 1)),
                Box::new(convert(sb, depth + 1)),
                Box::new(convert(sp, depth + 1)),
            ),
            _ => MathNode::Unsupported,
        },
        "rad" => {
            let radicand = child(n, "e").map(|e| convert(e, depth + 1));
            let degree = child(n, "deg")
                .filter(|d| d.children().any(|c| c.is_element()))
                .map(|d| convert(d, depth + 1));
            match (radicand, degree) {
                (Some(x), Some(idx)) => MathNode::Root(Box::new(x), Box::new(idx)),
                (Some(x), None) => MathNode::Sqrt(Box::new(x)),
                _ => MathNode::Unsupported,
            }
        }
        "d" => {
            let (open, close) = delim_chars(n);
            MathNode::Fenced {
                open,
                close,
                body: Box::new(child(n, "e").map_or(MathNode::Unsupported, |e| convert(e, depth + 1))),
            }
        }
        "nary" => {
            let op = nary_op(n);
            let sub = nonempty(child(n, "sub")).map(|s| Box::new(convert(s, depth + 1)));
            let sup = nonempty(child(n, "sup")).map(|s| Box::new(convert(s, depth + 1)));
            MathNode::Nary {
                op,
                sub,
                sup,
                body: Box::new(child(n, "e").map_or(MathNode::Unsupported, |e| convert(e, depth + 1))),
            }
        }
        "m" => matrix(n, depth),
        _ => MathNode::Unsupported,
    }
}

/// A `row` for a container: convert each element child; collapse a single child.
fn row(n: Node, depth: usize) -> MathNode {
    let items: Vec<MathNode> = n
        .children()
        .filter(Node::is_element)
        .map(|c| convert(c, depth + 1))
        .collect();
    match items.len() {
        0 => MathNode::Unsupported,
        1 => items.into_iter().next().unwrap(),
        _ => MathNode::Row(items),
    }
}

fn matrix(n: Node, depth: usize) -> MathNode {
    let mut rows = Vec::new();
    for mr in n
        .children()
        .filter(Node::is_element)
        .filter(|c| c.tag_name().name() == "mr")
    {
        let cells: Vec<MathNode> = mr
            .children()
            .filter(Node::is_element)
            .filter(|c| c.tag_name().name() == "e")
            .map(|c| convert(c, depth + 1))
            .collect();
        rows.push(cells);
    }
    if rows.is_empty() {
        MathNode::Unsupported
    } else {
        MathNode::Matrix(rows)
    }
}

/// Concatenated `<m:t>` text under a run.
fn run_text(n: Node) -> String {
    n.descendants()
        .filter(|d| d.is_element() && d.tag_name().name() == "t")
        .flat_map(|t| t.descendants().filter_map(|x| x.text()))
        .collect::<String>()
}

/// `<m:begChr m:val="…"/>` / `<m:endChr>` on `<m:dPr>`, defaulting to `( )`.
fn delim_chars(n: Node) -> (String, String) {
    let pr = child(n, "dPr");
    let get = |name: &str, default: &str| -> String {
        pr.and_then(|p| child(p, name))
            .and_then(|c| attr_val(c))
            .unwrap_or_else(|| default.to_string())
    };
    (get("begChr", "("), get("endChr", ")"))
}

/// `<m:chr m:val="…"/>` on `<m:naryPr>`, mapped through the symbol table by the
/// emitter; default is the integral sign.
fn nary_op(n: Node) -> String {
    let chr = child(n, "naryPr")
        .and_then(|p| child(p, "chr"))
        .and_then(attr_val)
        .unwrap_or_else(|| "∫".to_string());
    // Map the raw operator char to a command up front so the Nary `op` string
    // is already LaTeX (the emitter emits Nary.op verbatim).
    match chr.trim() {
        "∑" => "\\sum".to_string(),
        "∏" => "\\prod".to_string(),
        "∫" => "\\int".to_string(),
        "∮" => "\\oint".to_string(),
        "⋃" => "\\bigcup".to_string(),
        "⋂" => "\\bigcap".to_string(),
        other => other.to_string(),
    }
}

/// The `m:val` attribute (any namespace prefix), matched by local name.
fn attr_val(n: Node) -> Option<String> {
    n.attributes()
        .find(|a| a.name() == "val")
        .map(|a| a.value().to_string())
}

/// `Some(n)` only if the element has at least one element child (OMML uses
/// empty `<m:sub/>` to mean "no limit").
fn nonempty(n: Option<Node>) -> Option<Node> {
    n.filter(|c| c.children().any(|k| k.is_element()))
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p kasane-adapters math::omml`
Expected: PASS (7 tests).

- [ ] **Step 6: Lint and commit**

Run: `mise run lint`
Expected: clean.

```bash
git add crates/kasane-adapters/src/math/omml.rs crates/kasane-adapters/src/math/mod.rs
git commit -m "feat(math): OMML front-end (PPTX) → MathNode → LaTeX"
```

---

## Task 4: Shared island-capture helper

**Files:**
- Modify: `crates/kasane-adapters/src/math/mod.rs` (add `capture_island`)

**Interfaces:**
- Produces: `math::capture_island(reader: &mut quick_xml::Reader<&[u8]>, start: &quick_xml::events::BytesStart) -> String` — re-serializes the element that `start` opens, through its matching close tag, consuming those events from `reader`.

- [ ] **Step 1: Write the failing test**

Add a tests module to `crates/kasane-adapters/src/math/mod.rs`:

```rust
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
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p kasane-adapters math::tests::captures_nested_island`
Expected: FAIL — `capture_island` not found.

- [ ] **Step 3: Implement `capture_island`**

Append to `crates/kasane-adapters/src/math/mod.rs` (above the tests module):

```rust
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

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
            Ok(ev) => ev,
            Err(_) => break,
        };
        let _ = writer.write_event(&ev);
        match &ev {
            Event::Start(e) if e.local_name().as_ref() == local.as_slice() => depth += 1,
            Event::End(e) if e.local_name().as_ref() == local.as_slice() => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    String::from_utf8(writer.into_inner()).unwrap_or_default()
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p kasane-adapters math::tests::captures_nested_island`
Expected: PASS.

- [ ] **Step 5: Lint and commit**

Run: `mise run lint`
Expected: clean.

```bash
git add crates/kasane-adapters/src/math/mod.rs
git commit -m "feat(math): shared quick-xml island-capture helper"
```

---

## Task 5: EPUB wiring (MathML into xhtml)

**Files:**
- Modify: `crates/kasane-adapters/src/epub/xhtml.rs` (is_inline_tag treatment, `<math>` arm, tests)

**Interfaces:**
- Consumes: `math::mathml_to_latex`, `math::capture_island`.
- Produces: `Inline::Math` / `Block::MathBlock` (+ optional `Block::Raw`) in the block stream returned by `xhtml_to_blocks`.

- [ ] **Step 1: Write the failing EPUB tests**

Add to the `tests` module in `crates/kasane-adapters/src/epub/xhtml.rs` (it already has `parse_blocks` and `text_of` helpers):

```rust
    #[test]
    fn inline_math_stays_in_paragraph() {
        let blocks = parse_blocks(
            "<body><p>The value <math><msup><mi>x</mi><mn>2</mn></msup></math> is positive.</p></body>",
        );
        // One paragraph, containing an Inline::Math between the two text runs.
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(i) => Some(i),
                _ => None,
            })
            .expect("a paragraph");
        let math = para.iter().find_map(|i| match i {
            Inline::Math(s) => Some(s.clone()),
            _ => None,
        });
        assert_eq!(math.as_deref(), Some("{x}^{2}"));
        // The paragraph was not split into pieces around the math.
        assert_eq!(blocks.iter().filter(|b| matches!(b, Block::Para(_))).count(), 1);
    }

    #[test]
    fn display_math_becomes_math_block() {
        let blocks = parse_blocks(
            "<body><p>Before.</p><math display=\"block\"><mfrac><mn>1</mn><mn>2</mn></mfrac></math><p>After.</p></body>",
        );
        let mb = blocks.iter().find_map(|b| match b {
            Block::MathBlock(s) => Some(s.clone()),
            _ => None,
        });
        assert_eq!(mb.as_deref(), Some("\\frac{1}{2}"));
    }

    #[test]
    fn partial_display_math_emits_raw_note() {
        // Content MathML is out of subset → placeholder + note.
        let blocks = parse_blocks(
            "<body><math display=\"block\"><apply><ci>x</ci></apply></math></body>",
        );
        assert!(blocks
            .iter()
            .any(|b| matches!(b, Block::MathBlock(s) if s.contains("\\mathord{?}"))));
        assert!(blocks
            .iter()
            .any(|b| matches!(b, Block::Raw { note } if note.contains("partially converted"))));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-adapters -- epub::xhtml::tests::inline_math epub::xhtml::tests::display_math epub::xhtml::tests::partial_display_math`
Expected: FAIL — no `Inline::Math` / `Block::MathBlock` produced (math is currently dropped).

- [ ] **Step 3: Teach the pre-match logic that inline `<math>` is inline**

In `crates/kasane-adapters/src/epub/xhtml.rs`, add a helper next to `is_inline_tag` (after its closing brace, around line 254):

```rust
// A <math> element is inline unless it carries display="block". Attribute
// inspection is why this is separate from is_inline_tag (name-only).
fn math_is_inline(e: &quick_xml::events::BytesStart) -> bool {
    e.local_name().as_ref() == b"math"
        && !e
            .attributes()
            .flatten()
            .any(|a| a.key.as_ref() == b"display" && a.value.as_ref() == b"block")
}
```

Then, in the `Ok(Event::Start(e))` arm, change the two `is_inline_tag(e.local_name().as_ref())` guards (the `close_implicit!()` guard near line 452 and the implicit-paragraph opener near line 465) to also accept inline math. Replace:

```rust
                if !is_inline_tag(e.local_name().as_ref()) {
                    close_implicit!();
                }
```

with:

```rust
                if !is_inline_tag(e.local_name().as_ref()) && !math_is_inline(&e) {
                    close_implicit!();
                }
```

and replace:

```rust
                if is_inline_tag(e.local_name().as_ref())
                    && inline_stack.is_empty()
                    && in_body
                    && cur_block.is_none()
                {
```

with:

```rust
                if (is_inline_tag(e.local_name().as_ref()) || math_is_inline(&e))
                    && inline_stack.is_empty()
                    && in_body
                    && cur_block.is_none()
                {
```

- [ ] **Step 4: Add the `<math>` arm to the Start dispatch**

In the `match e.local_name().as_ref()` block (the one starting near line 490 with `b"h1" | … =>`), add a new arm (place it before the final `_ => {}`):

```rust
                    b"math" => {
                        let inline = math_is_inline(&e);
                        let island = crate::math::capture_island(&mut reader, &e);
                        let conv = crate::math::mathml_to_latex(&island);
                        if inline {
                            if let Some(top) = inline_stack.last_mut() {
                                crate::xmltext::push_inline(top, Inline::Math(conv.latex));
                            }
                        } else {
                            emit_block(
                                &mut frames,
                                &mut inline_stack,
                                &mut out,
                                Block::MathBlock(conv.latex),
                            );
                            if !conv.complete {
                                emit_block(
                                    &mut frames,
                                    &mut inline_stack,
                                    &mut out,
                                    Block::Raw {
                                        note: "equation partially converted".into(),
                                    },
                                );
                            }
                        }
                    }
```

Note: `capture_island` consumes through `</math>`, so no `End(math)` event reaches the loop and no inline frame is pushed for math itself. For the display case the preceding `close_implicit!()` already flushed any open paragraph, so `emit_block` writes a real block.

- [ ] **Step 5: Run the EPUB tests to verify they pass**

Run: `cargo test -p kasane-adapters -- epub::xhtml::tests::inline_math epub::xhtml::tests::display_math epub::xhtml::tests::partial_display_math`
Expected: PASS (3 tests).

- [ ] **Step 6: Add an end-to-end EPUB test (full zip → Document)**

The spec asks that math survive the whole EPUB pipeline, not just the xhtml pass. Add this test to the `tests` module in `crates/kasane-adapters/src/epub/mod.rs`, which already defines the `build_epub` helper:

```rust
    #[test]
    fn math_survives_full_epub_pipeline() {
        let bytes = build_epub(
            "<body><h1>M</h1>\
             <p>Inline <math><msup><mi>x</mi><mn>2</mn></msup></math> here.</p>\
             <math display=\"block\"><mfrac><mn>1</mn><mn>2</mn></mfrac></math></body>",
            &[],
        );
        let (doc, _assets) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        // inline math reached a paragraph
        assert!(doc.nodes.iter().any(|n| matches!(&n.block,
            Block::Para(i) if i.iter().any(|x| matches!(x, Inline::Math(s) if s == "{x}^{2}")))));
        // display math reached a MathBlock
        assert!(doc
            .nodes
            .iter()
            .any(|n| matches!(&n.block, Block::MathBlock(s) if s == "\\frac{1}{2}")));
    }
```

Run: `cargo test -p kasane-adapters -- epub::tests::math_survives_full_epub_pipeline`
Expected: PASS.

- [ ] **Step 7: Run the whole adapters suite to catch regressions**

Run: `cargo test -p kasane-adapters`
Expected: PASS (all existing xhtml/epub tests still green — the inline-math change must not disturb non-math parsing).

- [ ] **Step 8: Lint and commit**

Run: `mise run lint`
Expected: clean.

```bash
git add crates/kasane-adapters/src/epub/xhtml.rs crates/kasane-adapters/src/epub/mod.rs
git commit -m "feat(epub): convert MathML equations to LaTeX inline/display"
```

---

## Task 6: PPTX wiring (OMML into slides)

**Files:**
- Modify: `crates/kasane-adapters/src/pptx/slide.rs` (capture `m:oMath`/`m:oMathPara`, tests)

**Interfaces:**
- Consumes: `math::omml_to_latex`, `math::capture_island`.
- Produces: `Inline::Math` inside a body paragraph, or a `Block::MathBlock` (+ optional `Block::Raw`) among a slide's blocks.

- [ ] **Step 1: Write the failing PPTX tests**

Add to the `tests` module in `crates/kasane-adapters/src/pptx/slide.rs` (it already has `text_of` and `SLIDE` helpers):

```rust
    #[test]
    fn inline_omath_appends_math_inline_to_paragraph() {
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:m="m"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p>
            <a:r><a:t>value </a:t></a:r>
            <m:oMath><m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e>
              <m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath>
          </a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(i) => Some(i),
                _ => None,
            })
            .expect("a paragraph");
        assert!(para
            .iter()
            .any(|i| matches!(i, Inline::Math(s) if s == "{x}^{2}")));
    }

    #[test]
    fn omathpara_becomes_math_block() {
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:m="m"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p>
            <m:oMathPara><m:oMath><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num>
              <m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath></m:oMathPara>
          </a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        assert!(blocks
            .iter()
            .any(|b| matches!(b, Block::MathBlock(s) if s == "\\frac{1}{2}")));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-adapters -- pptx::slide::tests::inline_omath pptx::slide::tests::omathpara`
Expected: FAIL — no `Inline::Math` / `Block::MathBlock` produced.

- [ ] **Step 3: Capture OMML in `parse_shapes`**

OMML lives inside `<a:p>` but outside `<a:r>`: an inline `<m:oMath>` appends to the current paragraph, while a display `<m:oMathPara>` becomes a block flushed after the shape's paragraphs. `Paragraph` is unchanged — display math is collected in a separate per-shape accumulator.

In `parse_shapes`, add one local alongside the other `let mut` bindings:

```rust
    // Display equations (<m:oMathPara>) for the current shape, flushed as
    // MathBlock siblings when the shape closes.
    let mut display_math: Vec<(String, bool)> = Vec::new();
```

Add the capture arms to the `Ok(Event::Start(e))` match, before its `_ => {}`:

```rust
                b"oMathPara" => {
                    let island = crate::math::capture_island(&mut reader, &e);
                    let conv = crate::math::omml_to_latex(&island);
                    display_math.push((conv.latex, conv.complete));
                }
                b"oMath" => {
                    let island = crate::math::capture_island(&mut reader, &e);
                    let conv = crate::math::omml_to_latex(&island);
                    if let Some(p) = cur_para.as_mut() {
                        crate::xmltext::push_inline(&mut p.inlines, Inline::Math(conv.latex));
                    }
                }
```

Because `<m:oMathPara>` contains an `<m:oMath>`, the `b"oMathPara"` arm's `capture_island` consumes the whole island (including the inner `oMath`), so the inner `oMath` Start never reaches the loop. The two arms have distinct local names, so match order does not matter.

- [ ] **Step 4: Emit the display-math blocks from the shape**

`parse_shapes` returns `Vec<Shape>`; display math is per-shape. Add a new `Shape` variant and push it when a shape closes.

Add to the `Shape` enum:

```rust
    Math { latex: String, complete: bool },
```

In the `b"sp" =>` End arm, after the existing body/title push, drain the display math collected for this shape:

```rust
                    for (latex, complete) in std::mem::take(&mut display_math) {
                        shapes.push(Shape::Math { latex, complete });
                    }
```

- [ ] **Step 5: Render the `Math` shape in `slide_to_blocks` and `notes_to_blocks`**

In `slide_to_blocks`, add an arm to the `for s in shapes` match:

```rust
            Shape::Math { latex, complete } => {
                out.push(Block::MathBlock(latex));
                if !complete {
                    out.push(Block::Raw {
                        note: "equation partially converted".into(),
                    });
                }
            }
```

In `notes_to_blocks`, replace the existing body-only loop:

```rust
    for s in shapes {
        if let Shape::Body(paras) = s {
            body_to_blocks(paras, &mut out);
        }
    }
```

with a `match` that also handles math (so notes math is not dropped):

```rust
    for s in shapes {
        match s {
            Shape::Body(paras) => body_to_blocks(paras, &mut out),
            Shape::Math { latex, complete } => {
                out.push(Block::MathBlock(latex));
                if !complete {
                    out.push(Block::Raw {
                        note: "equation partially converted".into(),
                    });
                }
            }
            _ => {}
        }
    }
```

- [ ] **Step 6: Run the PPTX tests to verify they pass**

Run: `cargo test -p kasane-adapters -- pptx::slide::tests::inline_omath pptx::slide::tests::omathpara`
Expected: PASS (2 tests).

- [ ] **Step 7: Run the whole adapters suite to catch regressions**

Run: `cargo test -p kasane-adapters`
Expected: PASS (all existing slide tests still green — the new `Shape::Math` arm and `para_index` bookkeeping must not disturb non-math slides).

- [ ] **Step 8: Lint and commit**

Run: `mise run lint`
Expected: clean.

```bash
git add crates/kasane-adapters/src/pptx/slide.rs
git commit -m "feat(pptx): convert OMML equations to LaTeX inline/display"
```

---

## Task 7: Documentation

**Files:**
- Modify: `README.md`
- Modify: `AGENTS.md`

**Interfaces:** none (docs only).

- [ ] **Step 1: Update the README known-limitation line**

In `README.md`, under "Known limitations (this build)", replace:

```markdown
- MathML (EPUB) and OMML (PPTX) math are not yet converted to LaTeX.
```

with:

```markdown
- Math is recovered as LaTeX: MathML (EPUB) and OMML (PPTX) equations convert to
  GitHub-Flavored `$…$` (inline) / `$$…$$` (display) over a documented construct
  subset — fractions, sub/superscripts, roots, fenced groups, n-ary operators
  with limits, basic matrices, and common accents. A construct outside the
  subset degrades best-effort: the unmapped part becomes `\mathord{?}` and a
  partial display equation is followed by an "equation partially converted"
  note. Content MathML is not converted.
```

- [ ] **Step 2: Update the AGENTS.md codebase map**

In `AGENTS.md`, in the `crates/kasane-adapters` bullet, add a sentence describing the `math/` seam after the OCR seam description:

```markdown
The math seam (`math/`) converts MathML (EPUB) and OMML (PPTX) equations to LaTeX behind two front-ends (`mathml.rs`, `omml.rs`) over a shared `MathNode` model (`ast.rs`) and one emitter (`latex.rs`); adapters isolate a math island from their streaming parse via `capture_island` and parse it with `roxmltree`. Islands are untrusted: size/depth guards degrade to a `\mathord{?}` placeholder with a note rather than panicking. `Inline::Math`/`Block::MathBlock` are the only IR touchpoints.
```

- [ ] **Step 3: Verify docs build/read correctly**

Run: `mise run lint`
Expected: clean (no code changed; confirms nothing else broke).

- [ ] **Step 4: Commit**

```bash
git add README.md AGENTS.md
git commit -m "docs(math): document MathML/OMML → LaTeX conversion and the math seam"
```

- [ ] **Step 5: Final full verification**

Run: `mise run lint && mise run test`
Expected: all green — the whole workspace builds, lints clean, and every test (including the new math tests) passes.

---

## Notes on the untrusted-input boundary (applies to Tasks 2–6)

- Size guard (`MAX_ISLAND_BYTES`) and depth guard (`MAX_MATH_DEPTH`) both live in the front-ends; tripping either returns `degraded()`.
- `capture_island` returns whatever it captured on a reader error/EOF; the front-end's `Document::parse` then fails and degrades. No path panics.
- `roxmltree` resolves namespace prefixes; the synthetic wrapper (`wrap_island`) guarantees the `m:` prefix and the MathML default namespace are bound even when the original declarations lived on an uncaptured ancestor. Front-ends match by **local name**, so default-namespaced and prefixed islands both work.
