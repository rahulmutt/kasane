//! Fuzz seams for `kasane-writer`.
//!
//! A test seam, not API — the same convention and the same rationale as
//! `kasane-core`'s module of this name: it lives inside the crate so it can
//! reach `pub(crate)` internals (`escape::*`) that the separate `fuzz/`
//! workspace cannot.
//!
//! Each function takes `&[u8]` and either returns or panics. A panic **is**
//! the finding.
//!
//! This target asserts **postconditions**, not a round trip through a parser,
//! and that is deliberate: the round trip is P7's job, and a parser here would
//! mean a production dependency on `pulldown-cmark` for every kasane build.
//! The postconditions are the same kind of argument `kasane-core`'s `slug`
//! target makes — untrusted text entering a closed output alphabet.

use crate::escape::{self, Ctx};

pub fn escape(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    for ctx in [Ctx::Flow, Ctx::Cell] {
        for at_line_start in [true, false] {
            let out = escape::text(text, ctx, at_line_start);
            assert!(
                !out.contains('\r'),
                "escape::text kept a CR: {out:?} from {text:?}"
            );
            assert!(
                !out.contains("\n\n"),
                "escape::text kept a blank line: {out:?} from {text:?}"
            );
            if ctx == Ctx::Cell {
                assert!(
                    !out.contains('\n'),
                    "a cell cannot carry a newline: {out:?} from {text:?}"
                );
            }
            assert_unescaped_specials_are_absent(&out, text);
        }
    }

    // An HTML context must leave no bare `<` or `>`, except as part of the
    // writer's own literal `<br>` (Task 2, design spec §3.2: `html_text`
    // renders a newline as `<br>`, unconditionally, same as `Ctx::Cell`
    // does with a backslash escape everywhere else).
    let html = escape::text(text, Ctx::Html, false);
    assert_html_bare_lt_gt_only_in_br(&html, text);

    // A code span's delimiter run must not appear inside its content.
    let span = escape::code_span(text);
    let ticks = span.chars().take_while(|c| *c == '`').count();
    assert!(ticks >= 1, "code_span emitted no delimiter: {span:?}");
    let inner = &span[ticks..span.len() - ticks];
    assert!(
        !inner.contains(&"`".repeat(ticks)),
        "code_span content contains its own delimiter: {span:?} from {text:?}"
    );
    assert!(
        !inner.contains('\n'),
        "code_span kept a newline: {span:?} from {text:?}"
    );

    // A fenced block's fence must not appear at the start of a body line.
    let block = escape::fenced_block(text, Some(text));
    let fence_len = block.chars().take_while(|c| *c == '`').count();
    assert!(
        fence_len >= 3,
        "fence is too short: {fence_len} for {text:?}"
    );
    let fence = "`".repeat(fence_len);
    assert!(
        block.trim_end().ends_with(&fence),
        "fenced_block is not closed: {block:?}"
    );
    let body_start = block.find('\n').map(|i| i + 1).unwrap_or(block.len());
    let body_end = block.trim_end().len() - fence_len;
    if body_start < body_end {
        for line in block[body_start..body_end].lines() {
            assert!(
                !line.starts_with(&fence),
                "a body line reopens the fence: {line:?} from {text:?}"
            );
        }
    }

    // Destinations carry nothing that ends or nests a bare destination.
    for dest in [escape::dest_path(text), escape::dest_url(text)] {
        for bad in [' ', '(', ')', '<', '>', '"', '\\'] {
            assert!(
                !dest.contains(bad),
                "destination contains {bad:?}: {dest:?} from {text:?}"
            );
        }
        assert!(
            !dest.chars().any(|c| c.is_ascii_control()),
            "destination contains a control character: {dest:?} from {text:?}"
        );
    }

    // A YAML scalar is one quoted line with no unescaped interior quote.
    let yaml = escape::yaml_scalar(text);
    assert!(
        yaml.starts_with('"') && yaml.ends_with('"') && yaml.len() >= 2,
        "yaml_scalar is not quoted: {yaml:?} from {text:?}"
    );
    assert!(
        !yaml.contains('\n') && !yaml.chars().any(|c| c.is_control()),
        "yaml_scalar is not one line: {yaml:?} from {text:?}"
    );
    let body = &yaml[1..yaml.len() - 1];
    assert_no_unescaped(body, '"', text);
    assert_backslash_only_input_escapes_to_an_even_count(body, text);
}

/// Every character that can open an inline construct must carry a backslash,
/// except `<` when it is the writer's own literal `<br>` (see
/// `assert_lt_escaped_or_br`).
fn assert_unescaped_specials_are_absent(out: &str, from: &str) {
    for c in ['`', '*', '_', '[', ']', '~', '$'] {
        assert_no_unescaped(out, c, from);
    }
    assert_lt_escaped_or_br(out, from);
}

/// `<` never appears in `s` except immediately after an odd-length run of
/// backslashes, or as the first character of the exact substring `<br>`.
///
/// `Ctx::Cell` renders a newline as a literal `<br>`, not `\n` (a cell cannot
/// carry a newline at all — Task 2, design spec §3.2), so a bare `<` that
/// opens exactly that substring is not a defect. Checked per occurrence, not
/// as a whole-string carve-out: a stray unescaped `<` anywhere else in the
/// same output still panics, even when the output elsewhere legitimately
/// contains `<br>`.
fn assert_lt_escaped_or_br(s: &str, from: &str) {
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'<' {
            continue;
        }
        if bytes[i..].starts_with(b"<br>") {
            continue;
        }
        let mut backslashes = 0;
        let mut j = i;
        while j > 0 && bytes[j - 1] == b'\\' {
            backslashes += 1;
            j -= 1;
        }
        assert!(
            backslashes % 2 == 1,
            "unescaped '<' at byte {i} in {s:?} from {from:?} (not the start of a literal <br>)"
        );
    }
}

/// `Ctx::Html` (`html_text`) entity-encodes every literal `<`/`>`/`&`/`"`, so
/// the only way a bare `<` or `>` can survive into its output is the same
/// `<br>` it substitutes for a newline (same rule as `Ctx::Cell`, mechanism
/// differs — entities, not backslashes). A bare `<` must open that exact
/// substring; a bare `>` must close it. Checked per occurrence, so a stray
/// bare `<`/`>` elsewhere still panics.
fn assert_html_bare_lt_gt_only_in_br(html: &str, from: &str) {
    let bytes = html.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'<' => assert!(
                bytes[i..].starts_with(b"<br>"),
                "html escaping left a bare '<' not part of <br>: byte {i} in {html:?} from {from:?}"
            ),
            b'>' => assert!(
                i >= 3 && bytes[i - 3..].starts_with(b"<br>"),
                "html escaping left a bare '>' not part of <br>: byte {i} in {html:?} from {from:?}"
            ),
            _ => {}
        }
    }
}

/// `assert_no_unescaped`'s odd-length-run rule mis-reads `\` itself: a lone
/// input backslash escapes to `\\`, an even-length run, which the rule scores
/// as *unescaped* (empirically confirmed here — replaying
/// `fuzz/seeds/escape/inline_openers.txt`, which ends `...$j\k`, panics this
/// exact rule on the yaml body; see task-14 report for the trace).
///
/// The property that actually holds is different: doubling every literal
/// backslash always yields an even count. It holds unconditionally only when
/// the input carries no `"`, because a quote contributes one *extra*
/// backslash (`"` becomes `\"`) per occurrence and can make the total odd —
/// so this is checked only for a backslash-only input, not every input;
/// asserting it generally would just trade one wrong assertion for another.
/// The other eight `ALWAYS` characters keep the odd-length rule unchanged.
fn assert_backslash_only_input_escapes_to_an_even_count(body: &str, text: &str) {
    if !text.is_empty() && text.chars().all(|c| c == '\\') {
        let count = body.chars().filter(|&c| c == '\\').count();
        assert!(
            count % 2 == 0,
            "yaml_scalar produced an odd backslash count for backslash-only input: \
             {count} in {body:?} from {text:?}"
        );
    }
}

/// `c` never appears in `s` except immediately after an odd-length run of
/// backslashes — i.e. it is always escaped.
fn assert_no_unescaped(s: &str, c: char, from: &str) {
    let chars: Vec<char> = s.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        if *ch != c {
            continue;
        }
        let mut backslashes = 0;
        let mut j = i;
        while j > 0 && chars[j - 1] == '\\' {
            backslashes += 1;
            j -= 1;
        }
        assert!(
            backslashes % 2 == 1,
            "unescaped {c:?} at {i} in {s:?} from {from:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These pin the per-occurrence shape of the `<br>` exemptions: a
    // legitimate `<br>` must not turn off checking for the rest of the
    // string, unlike the brief's original `!out.contains(bad) ||
    // out.contains("<br>")` shape (Override 1).

    #[test]
    fn lt_exemption_allows_the_writers_own_br() {
        assert_lt_escaped_or_br("a<br>b", "n/a");
    }

    #[test]
    fn lt_exemption_allows_an_escaped_lt_alongside_a_real_br() {
        assert_lt_escaped_or_br("a\\<b<br>c", "n/a");
    }

    #[test]
    #[should_panic(expected = "unescaped '<'")]
    fn lt_exemption_still_catches_a_stray_unescaped_lt_next_to_a_real_br() {
        assert_lt_escaped_or_br("<br><x", "n/a");
    }

    #[test]
    fn html_lt_gt_exemption_allows_the_writers_own_br() {
        assert_html_bare_lt_gt_only_in_br("a<br>b<br>c", "n/a");
    }

    #[test]
    #[should_panic(expected = "bare '<'")]
    fn html_lt_exemption_still_catches_a_stray_bare_lt_next_to_a_real_br() {
        assert_html_bare_lt_gt_only_in_br("<br>x<y", "n/a");
    }

    #[test]
    #[should_panic(expected = "bare '>'")]
    fn html_gt_exemption_still_catches_a_stray_bare_gt() {
        assert_html_bare_lt_gt_only_in_br("a>b", "n/a");
    }

    // Pins the Override 2 fallback: the even-backslash-count property that
    // replaced the odd-length rule for backslash-only input.

    #[test]
    fn backslash_only_input_doubles_to_an_even_count() {
        assert_backslash_only_input_escapes_to_an_even_count("\\\\", "\\");
        assert_backslash_only_input_escapes_to_an_even_count("\\\\\\\\", "\\\\");
    }

    #[test]
    fn backslash_only_check_is_a_no_op_off_its_narrow_case() {
        // Empty input and non-backslash-only input are out of scope for this
        // fallback (see its doc comment) -- it must not panic on them.
        assert_backslash_only_input_escapes_to_an_even_count("", "");
        assert_backslash_only_input_escapes_to_an_even_count("x", "x");
    }

    #[test]
    fn escape_replays_the_backslash_seed_without_panicking() {
        // The exact input that first tripped Override 2's finding.
        escape(b"a*b_c[d]e`f<g>h~i$j\\k");
    }
}
