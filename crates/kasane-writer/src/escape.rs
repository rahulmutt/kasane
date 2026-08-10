//! Every rule for turning document text into Markdown that renders as that
//! text. Design spec `2026-08-09-markdown-escaping-design.md` §3.
//!
//! One function per output context, because the contexts do not share a
//! mechanism: a backslash escapes in flow text, is a literal character inside
//! a code span, is inert inside an HTML block, and means something else again
//! inside a YAML double-quoted scalar.
#![allow(dead_code)] // callers arrive in Task 6; removed there

/// Where a text run lands. The rules differ per context, so the renderers
/// must state which one they are in — `inlines_to_md` takes this as a
/// required argument for exactly that reason (§2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ctx {
    /// Paragraph, heading, list item, footnote body, caption, link label.
    Flow,
    /// A GFM table cell: `Flow` plus `|`, minus newlines.
    Cell,
    /// Inside the `has_merged` `<table>` fallback, where backslashes are inert.
    Html,
}

/// Characters that can open an inline construct at any position (§3.1).
/// `\` is first because every escape below would otherwise be escapable.
const ALWAYS: &[char] = &['\\', '`', '*', '_', '[', ']', '<', '~', '$'];

/// Characters that can only open a *block* construct, and so matter solely at
/// the start of a line (§3.1). `-` and `=` cover setext underlines and `---`
/// thematic breaks as well as bullets.
const LINE_START: &[char] = &['#', '-', '+', '>', '=', '|'];

pub(crate) fn text(s: &str, ctx: Ctx, at_line_start: bool) -> String {
    if ctx == Ctx::Html {
        return html_text(s);
    }
    let chars: Vec<char> = normalize_newlines(s).chars().collect();
    let mut out = String::with_capacity(chars.len() + 8);
    let mut line_start = at_line_start;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];

        if c == '\n' {
            match ctx {
                Ctx::Cell => out.push_str("<br>"),
                _ => {
                    out.push('\n');
                    line_start = true;
                }
            }
            i += 1;
            continue;
        }

        if line_start {
            if let Some(after_digits) = ordered_marker_delimiter(&chars, i) {
                for d in &chars[i..after_digits] {
                    out.push(*d);
                }
                out.push('\\');
                out.push(chars[after_digits]);
                i = after_digits + 1;
                line_start = false;
                continue;
            }
            if LINE_START.contains(&c) {
                out.push('\\');
                out.push(c);
                i += 1;
                line_start = false;
                continue;
            }
        }

        if ALWAYS.contains(&c) || (ctx == Ctx::Cell && c == '|') {
            out.push('\\');
            out.push(c);
        } else if c == '&' && opens_entity(&chars, i) {
            out.push('\\');
            out.push('&');
        } else {
            out.push(c);
        }
        line_start = false;
        i += 1;
    }
    out
}

/// `\r\n` and `\r` become `\n`, then runs of two or more `\n` collapse to one.
///
/// The collapse is what keeps one `Block::Para` one block: a blank line inside
/// a text run would otherwise split it into two paragraphs, which P7's
/// structural half would (correctly) fail on. In practice these are PDF and
/// DjVu line-break artifacts rather than authored breaks.
fn normalize_newlines(s: &str) -> String {
    let unified = s.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(unified.len());
    let mut last_was_newline = false;
    for c in unified.chars() {
        if c == '\n' {
            if !last_was_newline {
                out.push('\n');
            }
            last_was_newline = true;
        } else {
            out.push(c);
            last_was_newline = false;
        }
    }
    out
}

/// `Some(index_of_delimiter)` when `chars[i..]` begins an ordered-list marker:
/// one or more ASCII digits followed by `.` or `)`. `None` otherwise.
fn ordered_marker_delimiter(chars: &[char], i: usize) -> Option<usize> {
    if !chars.get(i)?.is_ascii_digit() {
        return None;
    }
    let mut j = i;
    while chars.get(j).is_some_and(char::is_ascii_digit) {
        j += 1;
    }
    match chars.get(j) {
        Some('.') | Some(')') => Some(j),
        _ => None,
    }
}

/// Whether `chars[i]` (a `&`) begins an entity reference, per CommonMark:
/// `&name;`, `&#decimal;`, or `&#xhex;`.
///
/// The lookahead is exact rather than heuristic, and that is why it is worth
/// having: `&` is common in real titles, and escaping it unconditionally would
/// backslash every "Q&A" and "Tom & Jerry" in the corpus for no parsing
/// benefit (§3.1).
fn opens_entity(chars: &[char], i: usize) -> bool {
    let mut j = i + 1;
    let numeric = chars.get(j) == Some(&'#');
    if numeric {
        j += 1;
        let hex = matches!(chars.get(j), Some('x') | Some('X'));
        if hex {
            j += 1;
        }
        let start = j;
        while chars.get(j).is_some_and(|c| {
            if hex {
                c.is_ascii_hexdigit()
            } else {
                c.is_ascii_digit()
            }
        }) {
            j += 1;
        }
        return j > start && chars.get(j) == Some(&';');
    }
    let start = j;
    while chars.get(j).is_some_and(|c| c.is_ascii_alphanumeric()) {
        j += 1;
    }
    j > start && chars.get(j) == Some(&';')
}

/// HTML-escape, for `Ctx::Html` — the `has_merged` `<table>` fallback.
///
/// Backslash escapes do not apply there: GFM parses an HTML block's content as
/// raw HTML, not as Markdown. `&` is escaped unconditionally here, unlike in
/// flow text, because inside HTML every `&` really is an entity opener.
fn html_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in normalize_newlines(s).chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("<br>"),
            _ => out.push(c),
        }
    }
    out
}

/// Fold every newline spelling to a single space, for the contexts where a
/// newline is structurally impossible: heading lines, link labels, image alt
/// text, YAML scalars (§4.1).
pub(crate) fn one_line(s: &str) -> String {
    s.replace("\r\n", " ").replace(['\n', '\r'], " ")
}

/// A link label or image alt text: flow rules, flattened to one line.
///
/// Escaped, never substituted. `library.rs`'s superseded `link_text` replaced
/// `[` with `(`, which changes the rendered text — forbidden by §5, because
/// anchors are computed from unescaped IR text and must still match what the
/// heading renders to.
pub(crate) fn label(s: &str) -> String {
    one_line(&text(s, Ctx::Flow, false))
}

/// Wrap code content in a backtick run the content cannot contain (§3.4).
///
/// No escape exists inside a code span, so the delimiter is the only lever.
/// Newlines fold to spaces because a blank line would end the enclosing
/// paragraph.
pub(crate) fn code_span(s: &str) -> String {
    let content = one_line(s);
    let ticks = "`".repeat(longest_backtick_run(&content) + 1);
    if content.is_empty() || content.chars().all(|c| c == ' ') {
        // Empty content or all-spaces: pad with a space on the left only
        format!("{ticks} {content}{ticks}")
    } else if content.starts_with('`') || content.ends_with('`') || content.contains('`') {
        // Contains backticks: pad with spaces on both sides
        format!("{ticks} {content} {ticks}")
    } else {
        // Plain content: no padding
        format!("{ticks}{content}{ticks}")
    }
}

/// A whole fenced code block, trailing newline included (§3.4).
pub(crate) fn fenced_block(text: &str, lang: Option<&str>) -> String {
    let ticks = "`".repeat(longest_backtick_run(text).max(2) + 1);
    let info = lang.map(sanitize_info).unwrap_or_default();
    format!("{ticks}{info}\n{text}\n{ticks}\n")
}

fn longest_backtick_run(s: &str) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for c in s.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

/// An info string is a single token by grammar. Take the first whitespace-free
/// run and drop backticks, which would otherwise break the opening fence.
fn sanitize_info(lang: &str) -> String {
    lang.split_whitespace()
        .next()
        .unwrap_or_default()
        .replace('`', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_escapes_every_always_character() {
        for c in ['\\', '`', '*', '_', '[', ']', '<', '~', '$'] {
            let input = format!("a{c}b");
            let got = text(&input, Ctx::Flow, false);
            assert_eq!(got, format!("a\\{c}b"), "input {input:?}");
        }
    }

    #[test]
    fn flow_leaves_bang_alone_because_bracket_is_escaped() {
        // `!` matters only before `[`, and `[` is always escaped, so `!\[`
        // can never form an image. Every "Wow!" in the corpus stays clean.
        assert_eq!(text("Wow! [see]", Ctx::Flow, false), "Wow! \\[see\\]");
    }

    #[test]
    fn flow_escapes_ampersand_only_when_it_opens_an_entity() {
        assert_eq!(text("Q&A", Ctx::Flow, false), "Q&A");
        assert_eq!(text("Tom & Jerry", Ctx::Flow, false), "Tom & Jerry");
        assert_eq!(text("a&amp;b", Ctx::Flow, false), "a\\&amp;b");
        assert_eq!(text("a&#38;b", Ctx::Flow, false), "a\\&#38;b");
        assert_eq!(text("a&#x26;b", Ctx::Flow, false), "a\\&#x26;b");
        // No terminating semicolon: not an entity, no escape.
        assert_eq!(text("a&amp b", Ctx::Flow, false), "a&amp b");
    }

    #[test]
    fn flow_escapes_line_start_characters_only_at_a_line_start() {
        for c in ['#', '-', '+', '>', '=', '|'] {
            let input = format!("{c}x");
            assert_eq!(
                text(&input, Ctx::Flow, true),
                format!("\\{c}x"),
                "at line start: {input:?}"
            );
            assert_eq!(text(&input, Ctx::Flow, false), input, "mid-line: {input:?}");
        }
    }

    #[test]
    fn flow_escapes_an_ordered_list_marker_delimiter() {
        assert_eq!(text("1. one", Ctx::Flow, true), "1\\. one");
        assert_eq!(text("12) two", Ctx::Flow, true), "12\\) two");
        // Not a marker: no digits, or no delimiter, or not at a line start.
        assert_eq!(text("1x. one", Ctx::Flow, true), "1x. one");
        assert_eq!(text("1. one", Ctx::Flow, false), "1. one");
    }

    #[test]
    fn flow_re_arms_line_start_after_an_interior_newline() {
        // The second line of a text run can open a block just as the first can.
        assert_eq!(
            text("intro\n# not a heading", Ctx::Flow, false),
            "intro\n\\# not a heading"
        );
    }

    #[test]
    fn flow_collapses_blank_lines_so_one_para_stays_one_para() {
        assert_eq!(text("a\n\n\nb", Ctx::Flow, false), "a\nb");
        assert_eq!(text("a\r\n\r\nb", Ctx::Flow, false), "a\nb");
    }

    #[test]
    fn cell_escapes_pipes_everywhere_and_carries_newlines_as_br() {
        assert_eq!(text("a|b", Ctx::Cell, false), "a\\|b");
        assert_eq!(text("one\ntwo", Ctx::Cell, false), "one<br>two");
        // Flow's rules still apply inside a cell.
        assert_eq!(text("a*b", Ctx::Cell, false), "a\\*b");
    }

    #[test]
    fn html_escapes_entities_not_backslashes() {
        assert_eq!(
            text("a & b < c > d \" e", Ctx::Html, false),
            "a &amp; b &lt; c &gt; d &quot; e"
        );
        // A backslash is a literal character inside an HTML block.
        assert_eq!(text("a\\b", Ctx::Html, false), "a\\b");
        // Every `&` is an entity opener here, unconditionally.
        assert_eq!(text("Q&A", Ctx::Html, false), "Q&amp;A");
        assert_eq!(text("one\ntwo", Ctx::Html, false), "one<br>two");
    }

    #[test]
    fn one_line_folds_every_newline_spelling_to_a_single_space() {
        assert_eq!(one_line("a\nb\r\nc\rd"), "a b c d");
    }

    #[test]
    fn label_escapes_and_flattens() {
        // A label lives inside `[...]` on one line: brackets escaped, newline
        // folded. Escaped, never substituted -- a substitution would change the
        // rendered text, which section 5 forbids.
        assert_eq!(label("a [b]\nc"), "a \\[b\\] c");
    }

    #[test]
    fn code_span_picks_a_run_longer_than_anything_inside() {
        assert_eq!(code_span("plain"), "`plain`");
        assert_eq!(code_span("a ` b"), "`` a ` b ``");
        assert_eq!(code_span("a ``` b"), "```` a ``` b ````");
    }

    #[test]
    fn code_span_pads_when_the_content_touches_a_backtick() {
        // CommonMark strips exactly one space from each end, so the padding is
        // invisible in the rendered output.
        assert_eq!(code_span("`x"), "`` `x ``");
        assert_eq!(code_span("x`"), "`` x` ``");
    }

    #[test]
    fn code_span_handles_all_spaces_and_empty_content() {
        assert_eq!(code_span("  "), "`   `");
        // CommonMark cannot express an empty code span; a single space is the
        // closest thing, and P7 normalizes whitespace so it round-trips.
        assert_eq!(code_span(""), "` `");
    }

    #[test]
    fn code_span_folds_newlines_to_spaces() {
        // A blank line would end the enclosing paragraph.
        assert_eq!(code_span("a\nb"), "`a b`");
    }

    #[test]
    fn fenced_block_widens_past_any_run_inside() {
        assert_eq!(fenced_block("plain", None), "```\nplain\n```\n");
        assert_eq!(
            fenced_block("outer\n```\ninner", None),
            "````\nouter\n```\ninner\n````\n"
        );
    }

    #[test]
    fn fenced_block_sanitizes_the_info_string() {
        assert_eq!(fenced_block("x", Some("rust")), "```rust\nx\n```\n");
        // An info string is one token by grammar; a backtick in it breaks the
        // opening fence outright.
        assert_eq!(fenced_block("x", Some("rust ignore")), "```rust\nx\n```\n");
        assert_eq!(fenced_block("x", Some("ru`st")), "```rust\nx\n```\n");
        assert_eq!(fenced_block("x", Some("   ")), "```\nx\n```\n");
    }
}
