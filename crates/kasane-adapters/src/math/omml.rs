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
        "oMathPara" | "oMath" | "e" | "num" | "den" | "sub" | "sup" | "deg" => row(n, depth),
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
                body: Box::new(
                    child(n, "e").map_or(MathNode::Unsupported, |e| convert(e, depth + 1)),
                ),
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
                body: Box::new(
                    child(n, "e").map_or(MathNode::Unsupported, |e| convert(e, depth + 1)),
                ),
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
        .flat_map(|t| t.children().filter_map(|x| x.text()))
        .collect::<String>()
}

/// `<m:begChr m:val="…"/>` / `<m:endChr>` on `<m:dPr>`, defaulting to `( )`.
fn delim_chars(n: Node) -> (String, String) {
    let pr = child(n, "dPr");
    let get = |name: &str, default: &str| -> String {
        pr.and_then(|p| child(p, name))
            .and_then(attr_val)
            .unwrap_or_else(|| default.to_string())
    };
    (get("begChr", "("), get("endChr", ")"))
}

/// `<m:chr m:val="…"/>` on `<m:naryPr>`, mapped to LaTeX commands where known;
/// default is the integral sign. Unknown operators are passed through for the
/// emitter's symbol table to degrade via placeholder.
fn nary_op(n: Node) -> String {
    let chr = child(n, "naryPr")
        .and_then(|p| child(p, "chr"))
        .and_then(attr_val)
        .unwrap_or_else(|| "∫".to_string());
    // Map known operator chars to LaTeX commands; unknown chars pass through
    // for the emitter to degrade to placeholder.
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
fn nonempty<'a>(n: Option<Node<'a, 'a>>) -> Option<Node<'a, 'a>> {
    n.filter(|c| c.children().any(|k| k.is_element()))
}

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
    fn nary_unmapped_operator_degrades() {
        let c = omml_to_latex(
            "<m:oMath><m:nary><m:naryPr><m:chr m:val=\"⨁\"/></m:naryPr>\
             <m:e><m:r><m:t>x</m:t></m:r></m:e></m:nary></m:oMath>",
        );
        assert_eq!(c.latex, "\\mathord{?} x");
        assert!(!c.complete);
    }

    #[test]
    fn delimiter_becomes_fenced() {
        let c = omml_to_latex("<m:oMath><m:d><m:e><m:r><m:t>x</m:t></m:r></m:e></m:d></m:oMath>");
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

    #[test]
    fn oversized_island_degrades() {
        let big = format!(
            "<m:oMath><m:r><m:t>{}</m:t></m:r></m:oMath>",
            "9".repeat(300_000)
        );
        let c = omml_to_latex(&big);
        assert_eq!(c.latex, "\\mathord{?}");
        assert!(!c.complete);
    }
}
