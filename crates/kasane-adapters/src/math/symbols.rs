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
        // large operators. These serve both `<mo>`/run text and `MathNode::Nary`'s
        // operator, which `omml::nary_op` now hands over as the raw character
        // rather than as a pre-built LaTeX command -- so every operator string
        // reaching `map_text` is document text, and `map_text` can neutralize a
        // backslash without destroying an operator the emitter chose.
        '∑' => "\\sum",
        '∏' => "\\prod",
        '∫' => "\\int",
        '∮' => "\\oint",
        '⋃' => "\\bigcup",
        '⋂' => "\\bigcap",
        // `\sqrt` with no braced argument swallows the next token, so a bare
        // radical operator (`<mo>√</mo><mn>25</mn>`) would render `\sqrt 25`
        // = `\sqrt{2}5`. `\surd` is the standalone radical glyph.
        '√' => "\\surd",
        _ => return None,
    })
}

/// Render operator/identifier text to LaTeX. Known symbols map to commands
/// (space-separated so `\alpha x` stays two tokens); other ASCII passes
/// through *neutralized*; an unmapped non-ASCII char emits the placeholder and
/// marks incomplete.
///
/// The neutralization is `latex::sanitize`'s, character for character and for
/// the same reason — the two are one policy, not two. `Ident`/`Op` is not a
/// minor corner of that policy but the majority of it: it carries MathML's
/// `<mi>` and `<mo>` *and every OMML run*, which is all of PowerPoint's
/// equation text. Without it, a `$` in document text closes the `$…$`/`$$…$$`
/// span `kasane-writer` opens around this string and everything after it lands
/// in the Markdown grammar as markup — a real link, a real heading — while
/// `{`/`}` unbalance a `\text{}` group the emitter opened, a `\` starts an
/// arbitrary command, and a newline breaks out of an inline span.
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
            match c {
                '$' => out.push_str("\\$"),
                '{' => out.push_str("\\{"),
                '}' => out.push_str("\\}"),
                '\\' => {}
                '\n' | '\r' => out.push(' '),
                _ => out.push(c),
            }
        } else {
            out.push_str(super::PLACEHOLDER);
            *complete = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::map_text;

    /// The pass-through arm neutralizes exactly what `latex::sanitize` does.
    #[test]
    fn pass_through_neutralizes_the_delimiter_breaking_set() {
        let mut complete = true;
        assert_eq!(map_text("a$b", &mut complete), "a\\$b");
        assert_eq!(map_text("a{b}c", &mut complete), "a\\{b\\}c");
        assert_eq!(map_text("a\\b", &mut complete), "ab");
        // A backslash is dropped BEFORE the `$` is escaped, so the two cannot
        // combine into `\\$` -- a LaTeX line break followed by a live `$`.
        assert_eq!(map_text("a\\$b", &mut complete), "a\\$b");
        assert_eq!(map_text("a\nb", &mut complete), "a b");
        // `\r\n` folds to two spaces, not one: `sanitize` maps each newline
        // character independently and this mirrors it exactly. Inside math the
        // difference is invisible, and diverging here would make the two
        // functions two policies again.
        assert_eq!(map_text("a\r\nb", &mut complete), "a  b");
        assert!(complete, "none of these degrade");
        // Ordinary operator text is untouched, and the symbol table still wins.
        assert_eq!(map_text("x + y", &mut complete), "x + y");
        assert_eq!(map_text("α", &mut complete), "\\alpha");
    }
}
