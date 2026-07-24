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
