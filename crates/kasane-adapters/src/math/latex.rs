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
        MathNode::Nary { op, sub, sup, body } => {
            out.push_str(&map_text(op, complete));
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
    fn nary_unmapped_operator_degrades() {
        let n = MathNode::Nary {
            op: "⨁".into(),
            sub: None,
            sup: None,
            body: Box::new(ident("x")),
        };
        let c = to_conversion(&n);
        assert_eq!(c.latex, "\\mathord{?} x");
        assert!(!c.complete);
    }

    #[test]
    fn matrix_renders_pmatrix() {
        let n = MathNode::Matrix(vec![vec![num("1"), num("2")], vec![num("3"), num("4")]]);
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
