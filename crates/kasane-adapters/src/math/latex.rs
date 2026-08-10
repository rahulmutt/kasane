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
        MathNode::Number(s) => out.push_str(&sanitize(s)),
        MathNode::Text(s) => {
            out.push_str("\\text{");
            out.push_str(&sanitize(s));
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

/// `<mn>` / `<mtext>` hold literal document text, not LaTeX source, and unlike
/// `Ident`/`Op` they never pass through `map_text`. Neutralize exactly the
/// characters that would corrupt a delimiter this pipeline itself generates:
/// `$` closes the `$…$` / `$$…$$` span `kasane-writer` opened, and `{` / `}`
/// unbalance the `\text{}` braces the emitter opened. A stray `\` would start
/// an arbitrary command, so it is dropped; newlines would break out of an
/// inline span, so they collapse to a space.
///
/// Scoped deliberately: this is *not* general LaTeX escaping. `kasane-writer`
/// has a repo-wide escaping policy (`escape.rs`) for flow text, cells, code
/// spans, HTML and YAML, but `Inline::Math` is deliberately exempt from it —
/// the writer pushes math content verbatim between the `$…$`/`$$…$$`
/// delimiters it opens, so nothing on that side neutralizes these characters.
/// This function is the other half of that decision: only the structural
/// subset that corrupts the delimiters the writer generates is handled here.
fn sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '$' => out.push_str("\\$"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\\' => {}
            '\n' | '\r' => out.push(' '),
            _ => out.push(c),
        }
    }
    out
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
    fn number_and_text_neutralize_delimiter_breaking_characters() {
        // Number and Text bypass map_text, so without sanitizing they push
        // raw document text into the middle of the `$…$` span the writer
        // generates and the `\text{}` braces this emitter generates.
        assert_eq!(to_conversion(&num("1$2")).latex, "1\\$2");
        assert_eq!(
            to_conversion(&MathNode::Text("a}b{c".into())).latex,
            "\\text{a\\}b\\{c}"
        );
        // A stray backslash would start an arbitrary command: dropped.
        assert_eq!(
            to_conversion(&MathNode::Text("a\\b".into())).latex,
            "\\text{ab}"
        );
        // A newline would break out of an inline span.
        assert_eq!(
            to_conversion(&MathNode::Text("a\nb".into())).latex,
            "\\text{a b}"
        );
        // Ordinary content is untouched.
        assert_eq!(to_conversion(&num("3.14")).latex, "3.14");
    }

    #[test]
    fn unsupported_emits_placeholder_and_marks_incomplete() {
        let c = to_conversion(&MathNode::Row(vec![ident("x"), MathNode::Unsupported]));
        assert_eq!(c.latex, "x \\mathord{?}");
        assert!(!c.complete);
    }
}
