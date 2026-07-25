use crate::math::ast::{AccentKind, MathNode};
use crate::math::latex::to_conversion;
use crate::math::{degraded, island_within_budget, wrap_island, MathConversion, MAX_MATH_DEPTH};
use roxmltree::{Document, Node};

/// Convert a Presentation-MathML `<math>…</math>` island to LaTeX.
pub fn mathml_to_latex(island: &str) -> MathConversion {
    // Size AND nesting must be bounded before `Document::parse`: roxmltree
    // recurses per element level, and overflowing there aborts the process.
    if !island_within_budget(island) {
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
        "→" | "\u{20D7}" => Some(AccentKind::Vec),
        "~" | "˜" | "\u{0303}" => Some(AccentKind::Tilde),
        "." | "˙" | "\u{0307}" => Some(AccentKind::Dot),
        _ => None,
    }
}

/// `<annotation>` / `<annotation-xml>` inside `<semantics>` carry alternate
/// encodings of the same expression (TeX source, Content MathML), not
/// presentation content. Rendering them would emit the equation twice — and in
/// the TeX case would pipe untrusted markup straight through — so they are
/// dropped and the presentation branch is converted instead.
fn is_annotation(n: &Node) -> bool {
    matches!(n.tag_name().name(), "annotation" | "annotation-xml")
}

fn convert(n: Node, depth: usize) -> MathNode {
    if depth > MAX_MATH_DEPTH {
        return MathNode::Unsupported;
    }
    let kids: Vec<Node> = n
        .children()
        .filter(Node::is_element)
        .filter(|c| !is_annotation(c))
        .collect();
    match n.tag_name().name() {
        // `semantics` is the dominant shape in EPUB3: MathJax, LaTeXML and
        // Pandoc all wrap the presentation tree in it alongside an
        // `<annotation>`. It is a pass-through container, like `mrow`.
        "math" | "mrow" | "mstyle" | "mpadded" | "semantics" => row(&kids, depth),
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
    match items.len() {
        // Parity with the OMML front-end: an empty container has no content to
        // render, and emitting "" would make the writer print a bare `$$`.
        0 => MathNode::Unsupported,
        1 => items.into_iter().next().unwrap(),
        _ => MathNode::Row(items),
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
    n.children()
        .filter_map(|c| c.text())
        .collect::<String>()
        .trim()
        .to_string()
}

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
        let c =
            mathml_to_latex("<math xmlns=\"http://www.w3.org/1998/Math/MathML\"><mn>3</mn></math>");
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

    #[test]
    fn deeply_nested_island_degrades_without_aborting() {
        // roxmltree recurses per element level and has no nesting guard, so an
        // island this deep used to abort the process with `fatal runtime
        // error: stack overflow` -- a SIGSEGV-class abort no catch_unwind can
        // rescue. 18,000 levels is past the release-build threshold
        // (12,000-18,000) and ~11x past the debug one (~1,600).
        let levels = 18_000;
        let island = format!(
            "<math>{}<mn>1</mn>{}</math>",
            "<mrow>".repeat(levels),
            "</mrow>".repeat(levels)
        );
        assert!(
            island.len() < crate::math::MAX_ISLAND_BYTES,
            "must trip the NESTING bound, not the byte bound ({} bytes)",
            island.len()
        );
        let c = mathml_to_latex(&island);
        assert_eq!(c.latex, "\\mathord{?}");
        assert!(!c.complete);
    }

    #[test]
    fn mathjax_semantics_wrapper_converts_presentation_branch() {
        // The dominant shape of MathML in EPUB3: MathJax, LaTeXML and Pandoc
        // all wrap the presentation tree in <semantics> next to a TeX
        // <annotation>. Falling through to Unsupported lost the whole equation.
        let c = mathml_to_latex(
            "<math xmlns=\"http://www.w3.org/1998/Math/MathML\" display=\"block\">\
             <semantics><mrow><msup><mi>x</mi><mn>2</mn></msup></mrow>\
             <annotation encoding=\"application/x-tex\">x^2</annotation>\
             </semantics></math>",
        );
        assert_eq!(c.latex, "{x}^{2}");
        assert!(c.complete);
    }

    #[test]
    fn annotation_xml_is_not_rendered_as_content() {
        let c = mathml_to_latex(
            "<math><semantics><mn>1</mn>\
             <annotation-xml encoding=\"MathML-Content\"><apply><ci>x</ci></apply></annotation-xml>\
             </semantics></math>",
        );
        assert_eq!(c.latex, "1");
        assert!(c.complete);
    }

    #[test]
    fn empty_island_degrades_rather_than_emitting_nothing() {
        // Parity with the OMML front-end. An empty latex string made the
        // writer emit a bare `$$` / `$$\n\n$$`.
        let c = mathml_to_latex("<math></math>");
        assert_eq!(c.latex, "\\mathord{?}");
        assert!(!c.complete);
    }

    #[test]
    fn radical_operator_maps_to_surd_not_sqrt() {
        // `\sqrt` with no braced argument grabs the next token, so `\sqrt 25`
        // would typeset as `\sqrt{2}5`.
        let c = mathml_to_latex("<math><mo>√</mo><mn>25</mn></math>");
        assert_eq!(c.latex, "\\surd 25");
        assert!(c.complete);
    }

    #[test]
    fn dollar_in_number_cannot_close_the_writers_math_span() {
        // kasane-writer wraps inline math in `$…$`; a raw `$` here closed the
        // span the writer itself opened.
        let c = mathml_to_latex("<math><mn>1$ <b>x</b> $2</mn></math>");
        assert_eq!(c.latex, "1\\$ x \\$2");
        assert!(c.complete);
    }

    #[test]
    fn brace_in_mtext_cannot_unbalance_the_text_command() {
        let c = mathml_to_latex("<math><mtext>a}b</mtext></math>");
        assert_eq!(c.latex, "\\text{a\\}b}");
        assert!(c.complete);
    }

    #[test]
    fn prefixed_island_parses() {
        // An island written with mml: prefix (not declared on the island itself,
        // as it arrives after capture) must parse thanks to wrap_island binding it.
        let c = mathml_to_latex("<mml:math><mml:mn>3</mml:mn></mml:math>");
        assert_eq!(c.latex, "3");
        assert!(c.complete);
    }
}
