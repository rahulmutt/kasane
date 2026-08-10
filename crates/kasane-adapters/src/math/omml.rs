use crate::math::ast::MathNode;
use crate::math::latex::to_conversion;
use crate::math::{degraded, island_within_budget, wrap_island, MathConversion, MAX_MATH_DEPTH};
use roxmltree::{Document, Node};

/// Convert an OMML `<m:oMath>` / `<m:oMathPara>` island to LaTeX.
pub fn omml_to_latex(island: &str) -> MathConversion {
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

/// All element children of `n` with local name `name`, in document order.
fn children<'a>(n: Node<'a, 'a>, name: &'a str) -> impl Iterator<Item = Node<'a, 'a>> {
    n.children()
        .filter(Node::is_element)
        .filter(move |c| c.tag_name().name() == name)
}

/// True for an OMML *property* element — `oMathParaPr`, `argPr`, `ctrlPr`,
/// `rPr`, `naryPr`, `dPr`, … — which carries formatting metadata, never
/// content. Real producers emit these constantly (every PowerPoint display
/// equation opens with `<m:oMathParaPr>`), so converting them as content puts a
/// spurious `\mathord{?}` in front of essentially every real equation and
/// falsely marks it incomplete.
///
/// This only excludes them from *content* traversal. `nary_op` and
/// `delim_chars` still reach `naryPr`/`dPr` by name through `child`.
fn is_props(n: &Node) -> bool {
    let name = n.tag_name().name();
    name.len() > 2 && name.ends_with("Pr")
}

/// Element children of `n` that are content rather than properties.
fn content_children<'a>(n: Node<'a, 'a>) -> impl Iterator<Item = Node<'a, 'a>> {
    n.children()
        .filter(Node::is_element)
        .filter(|c| !is_props(c))
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
            let degree = nonempty(child(n, "deg")).map(|d| convert(d, depth + 1));
            match (radicand, degree) {
                (Some(x), Some(idx)) => MathNode::Root(Box::new(x), Box::new(idx)),
                (Some(x), None) => MathNode::Sqrt(Box::new(x)),
                _ => MathNode::Unsupported,
            }
        }
        "d" => {
            let (open, close, sep) = delim_chars(n);
            // `(a, b)` is `<m:d><m:e>a</m:e><m:e>b</m:e></m:d>`: taking only
            // the first `<m:e>` dropped every argument after the first while
            // still reporting `complete`, which is worse than degrading.
            let mut body: Vec<MathNode> = Vec::new();
            for e in children(n, "e") {
                if !body.is_empty() && !sep.is_empty() {
                    body.push(MathNode::Op(sep.clone()));
                }
                body.push(convert(e, depth + 1));
            }
            MathNode::Fenced {
                open,
                close,
                body: Box::new(match body.len() {
                    0 => MathNode::Unsupported,
                    1 => body.into_iter().next().expect("len 1"),
                    _ => MathNode::Row(body),
                }),
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

/// A `row` for a container: convert each content child (properties are not
/// content); collapse a single child.
fn row(n: Node, depth: usize) -> MathNode {
    let items: Vec<MathNode> = content_children(n).map(|c| convert(c, depth + 1)).collect();
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

/// `<m:begChr>` / `<m:endChr>` / `<m:sepChr>` on `<m:dPr>`, defaulting to
/// `(`, `)` and `,`. An explicitly empty `m:val` means "none" and is kept as
/// the empty string.
fn delim_chars(n: Node) -> (String, String, String) {
    let pr = child(n, "dPr");
    let get = |name: &str, default: &str| -> String {
        pr.and_then(|p| child(p, name))
            .and_then(attr_val)
            .unwrap_or_else(|| default.to_string())
    };
    (get("begChr", "("), get("endChr", ")"), get("sepChr", ","))
}

/// `<m:chr m:val="…"/>` on `<m:naryPr>`, as the raw operator character;
/// default is the integral sign.
///
/// The Unicode → LaTeX mapping deliberately lives in `symbols::symbol` rather
/// than here, even though this used to hold its own copy. `m:val` is an
/// untrusted attribute, so the fallback arm always did hand document text to
/// `map_text`; returning a ready-made `\sum` alongside it meant `map_text` saw
/// both emitter-chosen LaTeX and document text on one code path and could not
/// neutralize a backslash in either without destroying the other. With the
/// mapping moved, every string reaching `map_text` is document text.
/// `symbols::symbol` gained `∮`, `⋃` and `⋂` so this table's coverage is
/// unchanged; an operator in neither table still degrades to the placeholder,
/// exactly as the pass-through arm did before.
fn nary_op(n: Node) -> String {
    child(n, "naryPr")
        .and_then(|p| child(p, "chr"))
        .and_then(attr_val)
        .map(|c| c.trim().to_string())
        .unwrap_or_else(|| "∫".to_string())
}

/// The `m:val` attribute (any namespace prefix), matched by local name.
fn attr_val(n: Node) -> Option<String> {
    n.attributes()
        .find(|a| a.name() == "val")
        .map(|a| a.value().to_string())
}

/// `Some(n)` only if the element has at least one *content* element child
/// (OMML uses an empty `<m:sub/>` — or one holding nothing but `<m:ctrlPr>` —
/// to mean "no limit").
fn nonempty<'a>(n: Option<Node<'a, 'a>>) -> Option<Node<'a, 'a>> {
    n.filter(|c| content_children(*c).next().is_some())
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

    /// Every OMML run is `MathNode::Ident`, so `map_text` -- not `sanitize` --
    /// is the only thing between PowerPoint's equation text and the `$…$`
    /// span `kasane-writer` opens around it. This is the whole of PPTX math.
    #[test]
    fn run_text_cannot_escape_the_math_span() {
        let c = omml_to_latex("<m:oMath><m:r><m:t>a$ [x](http://y) $b</m:t></m:r></m:oMath>");
        assert_eq!(c.latex, "a\\$ [x](http://y) \\$b");
    }

    /// `<m:begChr>`/`<m:endChr>` are untrusted attributes reaching a
    /// `\left`/`\right`.
    #[test]
    fn a_delimiter_attribute_cannot_escape_the_math_span() {
        let c = omml_to_latex(
            "<m:oMath><m:d><m:dPr><m:begChr m:val=\"$\"/><m:endChr m:val=\"$\"/></m:dPr>\
             <m:e><m:r><m:t>1</m:t></m:r></m:e></m:d></m:oMath>",
        );
        assert_eq!(c.latex, "\\left\\$1\\right\\$");
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
    fn deeply_nested_island_degrades_without_aborting() {
        // See the MathML twin: roxmltree recurses per element level, and this
        // depth used to abort the process with a stack overflow.
        let levels = 18_000;
        let island = format!(
            "<m:oMath>{}{}</m:oMath>",
            "<m:e>".repeat(levels),
            "</m:e>".repeat(levels)
        );
        assert!(
            island.len() < crate::math::MAX_ISLAND_BYTES,
            "must trip the NESTING bound, not the byte bound ({} bytes)",
            island.len()
        );
        let c = omml_to_latex(&island);
        assert_eq!(c.latex, "\\mathord{?}");
        assert!(!c.complete);
    }

    #[test]
    fn real_powerpoint_display_paragraph_ignores_its_properties() {
        // Exactly what PowerPoint writes for a centred display equation. The
        // <m:oMathParaPr> block is formatting, not content: converting it put
        // a spurious leading `\mathord{?}` on essentially every real display
        // equation and falsely marked it incomplete.
        let c = omml_to_latex(
            "<m:oMathPara><m:oMathParaPr><m:jc m:val=\"centerGroup\"/></m:oMathParaPr>\
             <m:oMath><m:r><m:t>x</m:t></m:r></m:oMath></m:oMathPara>",
        );
        assert_eq!(c.latex, "x");
        assert!(c.complete);
    }

    #[test]
    fn argument_properties_inside_a_fraction_are_not_content() {
        // <m:argPr> appears inside <m:e>/<m:num>/<m:den>/<m:sub>/<m:sup>/<m:deg>
        // and used to render as `\frac{\mathord{?} 1}{2}`.
        let c = omml_to_latex(
            "<m:oMath><m:f><m:fPr><m:ctrlPr/></m:fPr>\
             <m:num><m:argPr><m:argSz m:val=\"-1\"/></m:argPr><m:r><m:t>1</m:t></m:r></m:num>\
             <m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath>",
        );
        assert_eq!(c.latex, "\\frac{1}{2}");
        assert!(c.complete);
    }

    #[test]
    fn delimiter_with_two_arguments_keeps_both() {
        // `(a, b)` is two <m:e> children; taking only the first dropped `b`
        // while still reporting complete -- worse than degrading.
        let c = omml_to_latex(
            "<m:oMath><m:d><m:e><m:r><m:t>a</m:t></m:r></m:e>\
             <m:e><m:r><m:t>b</m:t></m:r></m:e></m:d></m:oMath>",
        );
        assert_eq!(c.latex, "\\left(a , b\\right)");
        assert!(c.complete);
    }

    #[test]
    fn delimiter_separator_char_is_honoured() {
        let c = omml_to_latex(
            "<m:oMath><m:d><m:dPr><m:begChr m:val=\"[\"/><m:endChr m:val=\"]\"/>\
             <m:sepChr m:val=\"|\"/></m:dPr>\
             <m:e><m:r><m:t>a</m:t></m:r></m:e>\
             <m:e><m:r><m:t>b</m:t></m:r></m:e></m:d></m:oMath>",
        );
        assert_eq!(c.latex, "\\left[a | b\\right]");
    }

    #[test]
    fn empty_limit_holding_only_properties_is_still_empty() {
        // OMML writes <m:sub><m:ctrlPr/></m:sub> for "no lower limit".
        let c = omml_to_latex(
            "<m:oMath><m:nary><m:naryPr><m:chr m:val=\"∑\"/></m:naryPr>\
             <m:sub><m:ctrlPr/></m:sub><m:sup><m:ctrlPr/></m:sup>\
             <m:e><m:r><m:t>i</m:t></m:r></m:e></m:nary></m:oMath>",
        );
        assert_eq!(c.latex, "\\sum i");
        assert!(c.complete);
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
