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
        "→" | "\u{20D7}" => Some(AccentKind::Vec),
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
    fn prefixed_island_parses() {
        // An island written with mml: prefix (not declared on the island itself,
        // as it arrives after capture) must parse thanks to wrap_island binding it.
        let c = mathml_to_latex("<mml:math><mml:mn>3</mml:mn></mml:math>");
        assert_eq!(c.latex, "3");
        assert!(c.complete);
    }
}
