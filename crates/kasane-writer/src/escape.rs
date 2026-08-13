//! Every rule for turning document text into Markdown that renders as that
//! text. Design spec `2026-08-09-markdown-escaping-design.md` §3.
//!
//! One function per output context, because the contexts do not share a
//! mechanism: a backslash escapes in flow text, is a literal character inside
//! a code span, is inert inside an HTML block, and means something else again
//! inside a YAML double-quoted scalar.

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

/// Where the next character emitted lands. The rules that depend on position
/// need three states, not two: `escape::text` has to distinguish "at column 0"
/// from "directly after a footnote reference that opened the line", because
/// only the latter makes a following `:` a footnote *definition* delimiter
/// (residuals spec §2).
///
/// `markdown.rs` computes this and passes it in; it never decides what to
/// escape. That division is escaping spec §2 — `escape.rs` owns every rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Pos {
    /// The next character lands at the start of a line.
    LineStart,
    /// The next character lands directly after a `[^n]` that itself opened
    /// the line.
    AfterFootnoteRef,
    /// Anywhere else.
    Mid,
}

pub(crate) fn text(s: &str, ctx: Ctx, pos: Pos) -> String {
    if ctx == Ctx::Html {
        return html_text(s);
    }
    let chars: Vec<char> = normalize_newlines(s).chars().collect();
    let mut out = String::with_capacity(chars.len() + 8);
    let mut line_start = pos == Pos::LineStart;
    let mut i = 0;

    // A `[^n]` that opened the line makes a leading `:` the delimiter of a
    // footnote *definition*, which swallows the paragraph (§2). `\:` is a
    // valid CommonMark escape — `:` is ASCII punctuation — and it leaves the
    // reference itself intact.
    //
    // Gated to `Ctx::Flow`: `render_table` renders every cell at a line start
    // so this position arises in a cell too, but a cell is inline context
    // where `[^1]:` is never a definition.
    if ctx == Ctx::Flow && pos == Pos::AfterFootnoteRef && chars.first() == Some(&':') {
        out.push('\\');
        out.push(':');
        i = 1;
    }

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
            // GFM reinterprets or discards whitespace at a line start: up to
            // three spaces still open a heading or a list, four open an
            // indented code block, and a cell's leading run is trimmed away
            // entirely. None of that is reachable by escaping a marker — the
            // code-block form has no marker, and `   \# h` suppresses the
            // heading only by losing the spaces.
            //
            // A character reference renders as the character it names but is
            // not whitespace to the block scanner, so one at the head of the
            // run disarms the whole run: everything after it is no longer at
            // column 0 (§3).
            match c {
                ' ' => {
                    out.push_str("&#32;");
                    i += 1;
                    line_start = false;
                    continue;
                }
                '\t' => {
                    out.push_str("&#9;");
                    i += 1;
                    line_start = false;
                    continue;
                }
                _ => {}
            }
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

/// Fold every newline spelling to a single space, **collapsing runs**, for the
/// contexts where a newline is structurally impossible: heading lines, link
/// labels, image alt text, code spans, YAML scalars (§4.1).
///
/// One fold for the whole crate, and the collapse is what makes that possible.
/// The two heading paths reach this from opposite directions and used to
/// disagree because it did not collapse: `markdown.rs`'s `Block::Heading`
/// escapes first and folds after, so `escape::text`'s own `normalize_newlines`
/// had already collapsed the run by the time it got here and `"A\n\nB"`
/// rendered `## A B`; `lib.rs`'s file-title heading folds first and escapes
/// after, so nothing had collapsed anything and the same title rendered
/// `# A  B`. `code_span` is a third caller with the same split — it folds
/// unescaped content, so nothing collapses ahead of it either.
///
/// That is not cosmetic. `kasane-core::slug`'s `anchor_fold` computes a
/// heading's fragment by predicting the rendered heading line, and it can only
/// predict one rule; whichever paths disagree with it emit a cross-reference
/// pointing at an id GitHub does not assign. `slug::fold_newlines` is this
/// function's hand-kept mirror in `kasane-core`, which cannot depend on this
/// crate — changing one without the other reopens that mismatch, and P2 is
/// what catches it.
///
/// Literal spaces are a different mechanism and are deliberately *not*
/// collapsed, here or in the mirror: `Background & Notes` still anchors
/// `background--notes`.
pub(crate) fn one_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_newline = false;
    for c in s.chars() {
        if c == '\n' || c == '\r' {
            if !last_was_newline {
                out.push(' ');
            }
            last_was_newline = true;
        } else {
            out.push(c);
            last_was_newline = false;
        }
    }
    out
}

/// A link label or image alt text: flow rules, flattened to one line.
///
/// Escaped, never substituted. `library.rs`'s superseded `link_text` replaced
/// `[` with `(`, which changes the rendered text — forbidden by §5, because
/// anchors are computed from unescaped IR text and must still match what the
/// heading renders to.
pub(crate) fn label(s: &str) -> String {
    one_line(&text(s, Ctx::Flow, Pos::Mid))
}

/// Wrap code content in a backtick run the content cannot contain (§3.4).
///
/// No escape exists inside a code span *for the code grammar*, so the
/// delimiter is the only lever there. Newlines fold to spaces because a blank
/// line would end the enclosing paragraph.
///
/// `Ctx` is required for the one thing that is not true of that: GFM's table
/// grammar runs *before* inline parsing, so a `|` splits a row even from
/// inside a code span, and GFM's own answer is a backslash — `` `b \| az` ``
/// renders `b | az` in a cell. That is why this takes a context at all rather
/// than being context-free like `fenced_block`; P8 fails on a row that gained
/// a cell otherwise.
pub(crate) fn code_span(s: &str, ctx: Ctx) -> String {
    let content = one_line(s);
    let content = if ctx == Ctx::Cell {
        content.replace('|', "\\|")
    } else {
        content
    };
    let ticks = "`".repeat(longest_backtick_run(&content) + 1);
    if content.is_empty() {
        // Rule 1: Empty content gets a single space (only acknowledged divergence from round-trip).
        format!("{ticks} {ticks}")
    } else if content.chars().all(|c| c == ' ') {
        // Rule 2: All-spaces content pads not at all; CommonMark's carve-out means the
        // content is not stripped, so no padding is needed to preserve input.
        format!("{ticks}{content}{ticks}")
    } else if content.contains('`') || content.starts_with(' ') || content.ends_with(' ') {
        // Rule 3: Pad if the content contains a backtick, or starts/ends with space.
        // For non-all-spaces content, CommonMark strips exactly one space from each end
        // iff it begins and ends with space, so padding is invisible in the render.
        format!("{ticks} {content} {ticks}")
    } else {
        // Plain content: no padding
        format!("{ticks}{content}{ticks}")
    }
}

/// Inline math: `$…$` around verbatim content, or a code span when that
/// content would break out of the span.
///
/// Math is the one inline the writer escapes nothing inside, on the strength
/// of a contract held in `kasane-adapters` (`math::latex::sanitize` and
/// `math::symbols::map_text` neutralize `$`, `{`, `}`, `\` and newlines in
/// every node kind that carries document text). That contract is real and is
/// tested where it lives — but `blocks_to_markdown` is public API over a
/// public IR, so a caller who builds `Inline::Math("a$ [x](http://y) $b")` by
/// hand reaches this function without ever passing an adapter, and gets a
/// document with a live injected link in it.
///
/// There is no escape available: a `\$` would be corrupted for adapter output
/// that already spells a literal dollar that way, and neutralizing `\`, `{` or
/// `}` here would destroy the `\frac{1}{2}` the adapter legitimately emits.
/// The only lever left is the one `code_span` already uses — pick a delimiter
/// the content cannot contain — and for math there is no wider delimiter to
/// pick. So unsafe content degrades to a code span instead: the LaTeX is still
/// there for a reader, verbatim, and it cannot break out of a code span by
/// construction. Same shape as `render_block`'s depth guard, and reachable for
/// the same reason: a caller who bypasses the adapters.
///
/// A newline is unsafe too, not only `$`: inline math can land in a GFM table
/// cell, where any newline ends the row.
///
/// `Ctx` is required for the same reason `code_span` takes one, and this is
/// **not** a hand-built-IR concern like the rest of this function:
/// `pptx/slide.rs` pushes `Inline::Math` straight into `cur_cell`, and `|`
/// survives `map_text` untouched (it is `ascii_graphic` and not in the symbol
/// table). A PPTX table cell holding `|x|` emitted `$|x|$`, which GFM splits
/// into `$` and `x` and then *drops* the row's real last cell — content loss,
/// or a destroyed table if the row is the header. Both branches escape it:
/// the verbatim one because the span itself sits in the cell, the degrade one
/// by passing `ctx` down. The backslash is consumed by the table grammar
/// before the math renderer sees it, so `$\|x\|$` recovers `InlineMath("|x|")`.
pub(crate) fn math_span(s: &str, ctx: Ctx) -> String {
    if s.contains('$') || s.contains('\n') || s.contains('\r') {
        code_span(s, ctx)
    } else if ctx == Ctx::Cell {
        format!("${}$", s.replace('|', "\\|"))
    } else {
        format!("${s}$")
    }
}

/// Display math: `$$…$$` around verbatim content, or a fenced code block when
/// that content would break out. See [`math_span`] for the argument.
///
/// A blank line is unsafe here rather than any newline: `$$…$$` spans lines by
/// design, but a blank line ends the block it sits in, leaving the closing
/// `$$` stranded as literal text.
///
/// The guard tests the **wrapped** string, not `s`. Testing `s` alone missed
/// content that merely starts or ends with a single newline, because the
/// wrapper's own newlines then complete the blank line the guard exists to
/// prevent: `"a\n"` became `"$$\na\n\n$$\n"`, which a real parser reads as two
/// paragraphs of literal `$` with the closing fence stranded.
pub(crate) fn math_block(s: &str) -> String {
    let wrapped = format!("$$\n{s}\n$$\n");
    if s.contains('$') || has_blank_line(&wrapped) {
        fenced_block(s, None)
    } else {
        wrapped
    }
}

/// Whether `s` contains a blank line, in any newline spelling.
fn has_blank_line(s: &str) -> bool {
    s.replace("\r\n", "\n").replace('\r', "\n").contains("\n\n")
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

/// Percent-encode a destination kasane *constructs* from the filesystem:
/// `_assets/<filename>`, the library index's `rel_dir`, internal links (§3.5).
///
/// This is `library.rs`'s former `link_dest`, rule unchanged. `%` is encoded
/// because a literal `%` in a filename would otherwise read back as an escape;
/// `#` and `?` because they are fragment and query delimiters, so `C# Notes`
/// would otherwise emit a destination parsing as path `C` with a fragment; the
/// rest because they end or nest a bare destination. `/` stays literal — it is
/// the path separator.
pub(crate) fn dest_path(s: &str) -> String {
    encode(s, &['%', '#', '?', ' ', '(', ')', '<', '>', '\\', '"'])
}

/// Percent-encode a destination that arrived as a URL from a source document's
/// `href` (`RefTarget::External`) (§3.5).
///
/// The asymmetry with `dest_path` is the whole reason both exist: this input is
/// *already* percent-encoded, so encoding `%` again would turn every
/// legitimately-encoded link into a broken one. `#` and `?` stay literal too —
/// in a URL they are meaningful delimiters, not text. What is left is exactly
/// what ends or nests a bare Markdown destination.
pub(crate) fn dest_url(s: &str) -> String {
    encode(s, &[' ', '(', ')', '<', '>', '\\', '"'])
}

/// A YAML double-quoted scalar, quotes included (§3.6).
///
/// Always quoted, rather than quoted-when-it-looks-risky. That deletes the
/// question of which characters require quoting — a question the superseded
/// `yaml_str` answered with `:` and `#`, and got wrong for a leading `-`, `[`,
/// `{`, `&`, `*`, `!`, `|`, `>`, `%`, `@`, a quote character, a trailing space,
/// and the bare words `true`, `null` and `~`, each of which YAML reads as
/// something other than a string. The cost is two bytes per line.
///
/// A double-quoted scalar cannot carry a raw control character (§4.1), so each
/// one folds to a space — the same treatment `one_line` already gives a
/// newline, and for the same reason: folding, not dropping, is what keeps two
/// words from silently fusing into one.
pub(crate) fn yaml_scalar(s: &str) -> String {
    let flat = one_line(s);
    let mut out = String::with_capacity(flat.len() + 2);
    out.push('"');
    for c in flat.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Make a note safe inside `<!-- ... -->` (§4.4).
///
/// A note containing `-->` closes the comment early, and one ending in `-`
/// leaves it malformed. This is load-bearing, not precautionary: notes are
/// not always internal fixed strings. `epub/xhtml.rs` and `epub/mod.rs` both
/// build a note with `format!("image unavailable: {src}")`, interpolating an
/// untrusted `<img src>` attribute value straight off the document, so a
/// crafted or merely unusual EPUB can put arbitrary text here. `Block::Raw`
/// is `Ctx::Html`'s comment sibling with one difference the rest of this
/// module doesn't have to face: an HTML comment admits no escape mechanism
/// at all -- no backslash, no entity, nothing that represents a literal
/// `-->` inside `<!-- -->`. `comment_note` therefore cannot escape, only
/// transform, and design spec §5 documents `Block::Raw` as the one place its
/// own invariant ("escaping must never change what the Markdown renders to")
/// does not hold -- forced by the format, not chosen, and harmless only
/// because a comment's content is never rendered for a reader to see the
/// difference.
///
/// A one-shot `str::replace("--", "- -")` is not enough: it matches
/// non-overlapping left-to-right, so an odd-length dash run leaves one dash
/// unreplaced right after a chunk that itself ends in `-`, and the two
/// recombine into a fresh `--` (`"--->"` -> `"- -->"`, which still closes the
/// comment). Walking the characters one at a time and inserting a space
/// whenever the character about to be pushed is `-` and the output so far
/// already ends in `-` considers the *output*, not the input, so a pair
/// created by an earlier insertion is caught too — no run of two or more
/// dashes can ever survive into `out`.
pub(crate) fn comment_note(s: &str) -> String {
    let mut out = String::new();
    for c in one_line(s).chars() {
        if c == '-' && out.ends_with('-') {
            out.push(' ');
        }
        out.push(c);
    }
    if out.ends_with('-') {
        out.push(' ');
    }
    out
}

fn encode(s: &str, extra: &[char]) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if extra.contains(&c) || c.is_ascii_control() {
            for b in c.to_string().as_bytes() {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_escapes_every_always_character() {
        for c in ['\\', '`', '*', '_', '[', ']', '<', '~', '$'] {
            let input = format!("a{c}b");
            let got = text(&input, Ctx::Flow, Pos::Mid);
            assert_eq!(got, format!("a\\{c}b"), "input {input:?}");
        }
    }

    #[test]
    fn flow_leaves_bang_alone_because_bracket_is_escaped() {
        // `!` matters only before `[`, and `[` is always escaped, so `!\[`
        // can never form an image. Every "Wow!" in the corpus stays clean.
        assert_eq!(text("Wow! [see]", Ctx::Flow, Pos::Mid), "Wow! \\[see\\]");
    }

    #[test]
    fn flow_escapes_ampersand_only_when_it_opens_an_entity() {
        assert_eq!(text("Q&A", Ctx::Flow, Pos::Mid), "Q&A");
        assert_eq!(text("Tom & Jerry", Ctx::Flow, Pos::Mid), "Tom & Jerry");
        assert_eq!(text("a&amp;b", Ctx::Flow, Pos::Mid), "a\\&amp;b");
        assert_eq!(text("a&#38;b", Ctx::Flow, Pos::Mid), "a\\&#38;b");
        assert_eq!(text("a&#x26;b", Ctx::Flow, Pos::Mid), "a\\&#x26;b");
        // No terminating semicolon: not an entity, no escape.
        assert_eq!(text("a&amp b", Ctx::Flow, Pos::Mid), "a&amp b");
    }

    #[test]
    fn flow_escapes_line_start_characters_only_at_a_line_start() {
        for c in ['#', '-', '+', '>', '=', '|'] {
            let input = format!("{c}x");
            assert_eq!(
                text(&input, Ctx::Flow, Pos::LineStart),
                format!("\\{c}x"),
                "at line start: {input:?}"
            );
            assert_eq!(
                text(&input, Ctx::Flow, Pos::Mid),
                input,
                "mid-line: {input:?}"
            );
        }
    }

    #[test]
    fn flow_escapes_an_ordered_list_marker_delimiter() {
        assert_eq!(text("1. one", Ctx::Flow, Pos::LineStart), "1\\. one");
        assert_eq!(text("12) two", Ctx::Flow, Pos::LineStart), "12\\) two");
        // Not a marker: no digits, or no delimiter, or not at a line start.
        assert_eq!(text("1x. one", Ctx::Flow, Pos::LineStart), "1x. one");
        assert_eq!(text("1. one", Ctx::Flow, Pos::Mid), "1. one");
    }

    #[test]
    fn flow_re_arms_line_start_after_an_interior_newline() {
        // The second line of a text run can open a block just as the first can.
        assert_eq!(
            text("intro\n# not a heading", Ctx::Flow, Pos::Mid),
            "intro\n\\# not a heading"
        );
    }

    #[test]
    fn flow_collapses_blank_lines_so_one_para_stays_one_para() {
        assert_eq!(text("a\n\n\nb", Ctx::Flow, Pos::Mid), "a\nb");
        assert_eq!(text("a\r\n\r\nb", Ctx::Flow, Pos::Mid), "a\nb");
    }

    #[test]
    fn cell_escapes_pipes_everywhere_and_carries_newlines_as_br() {
        assert_eq!(text("a|b", Ctx::Cell, Pos::Mid), "a\\|b");
        assert_eq!(text("one\ntwo", Ctx::Cell, Pos::Mid), "one<br>two");
        // Flow's rules still apply inside a cell.
        assert_eq!(text("a*b", Ctx::Cell, Pos::Mid), "a\\*b");
    }

    #[test]
    fn html_escapes_entities_not_backslashes() {
        assert_eq!(
            text("a & b < c > d \" e", Ctx::Html, Pos::Mid),
            "a &amp; b &lt; c &gt; d &quot; e"
        );
        // A backslash is a literal character inside an HTML block.
        assert_eq!(text("a\\b", Ctx::Html, Pos::Mid), "a\\b");
        // Every `&` is an entity opener here, unconditionally.
        assert_eq!(text("Q&A", Ctx::Html, Pos::Mid), "Q&amp;A");
        assert_eq!(text("one\ntwo", Ctx::Html, Pos::Mid), "one<br>two");
    }

    #[test]
    fn one_line_folds_every_newline_run_to_a_single_space() {
        assert_eq!(one_line("a\nb\r\nc\rd"), "a b c d");
        // The rows this used to get wrong: a blank line is ONE separator on the
        // rendered line, so it must be one space here and one hyphen in
        // `anchor_slug`.
        assert_eq!(one_line("a\n\nb"), "a b");
        assert_eq!(one_line("a\r\n\r\nb"), "a b");
        assert_eq!(one_line("a\n\r\n\rb"), "a b");
        // Literal spaces are a different mechanism and are NOT collapsed --
        // `Background & Notes` still anchors `background--notes`.
        assert_eq!(one_line("a  b"), "a  b");
    }

    /// GFM splits a row on `|` before it parses any inline, so a code span is
    /// no shelter -- and GFM's own answer is a backslash, which it strips back
    /// out when it renders the cell.
    #[test]
    fn code_span_escapes_a_pipe_in_a_cell_but_not_in_flow() {
        assert_eq!(code_span("a|b", Ctx::Cell), "`a\\|b`");
        assert_eq!(code_span("a|b", Ctx::Flow), "`a|b`");
    }

    #[test]
    fn code_span_folds_a_newline_run_the_same_way_a_heading_does() {
        // `inline_text` feeds `Inline::Code`'s text to `anchor_slug` like any
        // other text, so a code span in a heading has to fold newlines the way
        // `anchor_fold` does or the emitted fragment misses by a hyphen. P2
        // found this the first time the generator drew a newline run.
        assert_eq!(code_span("a\n\nb", Ctx::Flow), "`a b`");
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
        assert_eq!(code_span("plain", Ctx::Flow), "`plain`");
        assert_eq!(code_span("a ` b", Ctx::Flow), "`` a ` b ``");
        assert_eq!(code_span("a ``` b", Ctx::Flow), "```` a ``` b ````");
    }

    #[test]
    fn code_span_pads_when_the_content_touches_a_backtick() {
        // CommonMark strips exactly one space from each end, so the padding is
        // invisible in the rendered output.
        assert_eq!(code_span("`x", Ctx::Flow), "`` `x ``");
        assert_eq!(code_span("x`", Ctx::Flow), "`` x` ``");
    }

    #[test]
    fn code_span_handles_all_spaces_and_empty_content() {
        // All-spaces content receives no padding because CommonMark's carve-out
        // means "consists entirely of space characters" are not stripped, so the
        // input round-trips exactly.
        assert_eq!(code_span("  ", Ctx::Flow), "`  `");
        // CommonMark cannot express an empty code span; a single space is the
        // closest thing, and P7 normalizes whitespace so it round-trips.
        assert_eq!(code_span("", Ctx::Flow), "` `");
    }

    #[test]
    fn code_span_preserves_spaces_at_both_ends() {
        // Content with space at both ends gets padded. CommonMark strips exactly
        // one space from each end (because the content begins and ends with space
        // but does not consist entirely of spaces), so the outer padding is
        // invisible and the input's own spaces survive the render.
        assert_eq!(code_span(" a ", Ctx::Flow), "`  a  `");
    }

    #[test]
    fn code_span_folds_newlines_to_spaces() {
        // A blank line would end the enclosing paragraph.
        assert_eq!(code_span("a\nb", Ctx::Flow), "`a b`");
    }

    /// Adapter-produced math is emitted verbatim; content that would break out
    /// of the delimiter degrades to a construct that cannot.
    ///
    /// Only reachable from a caller who builds `Inline::Math` /
    /// `Block::MathBlock` by hand — `kasane-adapters` neutralizes `$` and
    /// newlines in every node kind that carries document text — but
    /// `blocks_to_markdown` is public API over a public IR, so that caller
    /// exists.
    #[test]
    fn math_degrades_when_its_content_would_close_the_delimiter() {
        assert_eq!(math_span("x^2", Ctx::Flow), "$x^2$");
        assert_eq!(math_span("\\frac{1}{2}", Ctx::Flow), "$\\frac{1}{2}$");
        // A `$` closes the span; a newline ends a table row.
        assert_eq!(
            math_span("a$ [x](http://y) $b", Ctx::Flow),
            "`a$ [x](http://y) $b`"
        );
        assert_eq!(math_span("a\nb", Ctx::Flow), "`a b`");

        assert_eq!(math_block("x^2"), "$$\nx^2\n$$\n");
        // Display math spans lines by design, so a lone newline is fine.
        assert_eq!(math_block("a\nb"), "$$\na\nb\n$$\n");
        // A blank line ends the block, stranding the closing `$$`.
        assert_eq!(math_block("a\n\nb"), "```\na\n\nb\n```\n");
        assert_eq!(math_block("a$b"), "```\na$b\n```\n");
        // Content that merely touches an edge manufactures the blank line
        // together with the wrapper's own newlines, which is why the guard
        // tests the wrapped string.
        assert_eq!(math_block("a\n"), "```\na\n\n```\n");
        assert_eq!(math_block("\na"), "```\n\na\n```\n");
        assert_eq!(math_block("\r\na"), "```\n\r\na\n```\n");
    }

    /// A PPTX table cell really does hold `Inline::Math` (`pptx/slide.rs`
    /// pushes it into `cur_cell`), and `|` survives the adapter's `map_text`,
    /// so this is adapter-reachable rather than hand-built-IR defence. Both
    /// branches must escape it: unescaped, `$|x|$` splits into `$` and `x` and
    /// the row's real last cell is dropped outright.
    #[test]
    fn math_in_a_cell_escapes_a_pipe_on_both_branches() {
        // Verbatim branch.
        assert_eq!(math_span("|x|", Ctx::Cell), "$\\|x\\|$");
        assert_eq!(math_span("|x|", Ctx::Flow), "$|x|$");
        // Degrade branch: the `$` forces a code span, which must still carry
        // the cell's pipe rule down with it.
        assert_eq!(math_span("$|x|", Ctx::Cell), "`$\\|x\\|`");
        assert_eq!(math_span("$|x|", Ctx::Flow), "`$|x|`");
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

    #[test]
    fn dest_path_encodes_percent_because_it_is_a_filename() {
        // A literal `%` in a filename would otherwise read back as an escape:
        // `50%20off` would decode to `50 off`, a directory that does not exist.
        assert_eq!(dest_path("50%20off"), "50%2520off");
        assert_eq!(dest_path("a b"), "a%20b");
        assert_eq!(dest_path("C# Notes"), "C%23%20Notes");
        assert_eq!(dest_path("a(b)c"), "a%28b%29c");
        assert_eq!(dest_path("a\nb"), "a%0Ab");
        // `/` stays literal: it is the path separator, not something to hide.
        assert_eq!(dest_path("a/b/index.md"), "a/b/index.md");
    }

    #[test]
    fn dest_url_leaves_percent_alone_because_it_is_already_a_url() {
        // Encoding `%` again would break every legitimately-encoded href.
        assert_eq!(dest_url("https://e.com/a%20b"), "https://e.com/a%20b");
        assert_eq!(dest_url("https://e.com/a b"), "https://e.com/a%20b");
        assert_eq!(dest_url("https://e.com/x(1)"), "https://e.com/x%281%29");
        assert_eq!(dest_url("https://e.com/a\nb"), "https://e.com/a%0Ab");
        // A fragment and a query are meaningful in a URL and stay literal.
        assert_eq!(dest_url("https://e.com/p?q=1#f"), "https://e.com/p?q=1#f");
    }

    #[test]
    fn yaml_scalar_always_quotes() {
        assert_eq!(yaml_scalar("plain"), "\"plain\"");
        assert_eq!(yaml_scalar("a: b"), "\"a: b\"");
        // Each of these is a shape the old conditional quoting got wrong.
        assert_eq!(yaml_scalar("- dash"), "\"- dash\"");
        assert_eq!(yaml_scalar("[bracket"), "\"[bracket\"");
        assert_eq!(yaml_scalar("&anchor"), "\"&anchor\"");
        assert_eq!(yaml_scalar("*alias"), "\"*alias\"");
        assert_eq!(yaml_scalar("true"), "\"true\"");
        assert_eq!(yaml_scalar("null"), "\"null\"");
        assert_eq!(yaml_scalar("trailing "), "\"trailing \"");
    }

    #[test]
    fn yaml_scalar_escapes_quotes_and_backslashes_and_flattens() {
        assert_eq!(yaml_scalar("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(yaml_scalar("a\\b"), "\"a\\\\b\"");
        assert_eq!(yaml_scalar("a\nb"), "\"a b\"");
        // A double-quoted scalar cannot carry a raw control character, so it
        // folds to a space for the same reason a newline does -- folding
        // rather than dropping is what keeps two words from fusing.
        assert_eq!(yaml_scalar("a\u{7}b"), "\"a b\"");
        assert_eq!(yaml_scalar("cat\tnap"), "\"cat nap\"");
    }

    /// A footnote reference that opened the line makes a following `:` the
    /// delimiter of a footnote *definition*, which swallows the paragraph.
    /// The `:` belongs to the next inline, which is why the position has to
    /// carry the fact across the boundary (§2).
    #[test]
    fn flow_escapes_a_colon_directly_after_a_footnote_reference() {
        assert_eq!(text(": note", Ctx::Flow, Pos::AfterFootnoteRef), "\\: note");
        // Only the *leading* colon: nothing later on the line can be a
        // definition delimiter.
        assert_eq!(text("x: y", Ctx::Flow, Pos::AfterFootnoteRef), "x: y");
        // Not at that position, not escaped.
        assert_eq!(text(": note", Ctx::Flow, Pos::LineStart), ": note");
        assert_eq!(text(": note", Ctx::Flow, Pos::Mid), ": note");
    }

    /// A cell is inline context, where `[^1]:` is never a definition, so the
    /// backslash would render as nothing and exist for no reason (§2).
    #[test]
    fn a_cell_does_not_escape_the_footnote_colon() {
        assert_eq!(text(": note", Ctx::Cell, Pos::AfterFootnoteRef), ": note");
    }

    /// A character reference, not a backslash. `   \# h` does suppress the
    /// heading, but the parser then strips the three leading spaces — the
    /// text is gone, which is an escaping-spec §5 violation presenting as a
    /// fix. A reference renders as the character and is not whitespace to the
    /// block scanner, so one at the head of the run disarms all of it (§3).
    #[test]
    fn a_line_start_whitespace_run_becomes_a_character_reference() {
        // The first character carries it; the rest of the run is literal,
        // and the `#` needs no backslash because it is no longer at column 0.
        assert_eq!(text("  # h", Ctx::Flow, Pos::LineStart), "&#32; # h");
        // Four spaces would open an indented code block, which has no marker
        // to escape.
        assert_eq!(text("    x", Ctx::Flow, Pos::LineStart), "&#32;   x");
        // A tab indents just as far.
        assert_eq!(text("\tx", Ctx::Flow, Pos::LineStart), "&#9;x");
        // A whitespace-only run with no trailing content: only the head
        // converts, same as any other run.
        assert_eq!(text("  ", Ctx::Flow, Pos::LineStart), "&#32; ");
        // Mid-line whitespace is ordinary text.
        assert_eq!(text("  x", Ctx::Flow, Pos::Mid), "  x");
    }

    #[test]
    fn line_start_whitespace_re_arms_after_an_interior_newline() {
        assert_eq!(
            text("intro\n  # not a heading", Ctx::Flow, Pos::Mid),
            "intro\n&#32; # not a heading"
        );
    }

    /// GFM trims a cell before parsing it, so a leading run is dropped
    /// outright — document text lost. `render_table` renders every cell at a
    /// line start, so this rule reaches the leading edge with no new
    /// plumbing (§3.2).
    #[test]
    fn a_cell_keeps_its_leading_whitespace() {
        assert_eq!(text("  x", Ctx::Cell, Pos::LineStart), "&#32; x");
        assert_eq!(text("\tx", Ctx::Cell, Pos::LineStart), "&#9;x");
    }

    /// The reference has to survive as one, which means the `&` must not pick
    /// up a backslash from the entity rule on the way out. Both references
    /// the rule can emit are checked, not just the space -- `&#9;` has to
    /// decode back to a real tab, not to four literal spaces or nothing.
    #[test]
    fn the_emitted_reference_parses_back_to_the_whitespace() {
        use pulldown_cmark::{Event, Options, Parser};

        for input in ["    x", "\tx"] {
            let md = format!("{}\n", text(input, Ctx::Flow, Pos::LineStart));
            let mut got = String::new();
            let mut is_code_block = false;
            for ev in Parser::new_ext(&md, Options::empty()) {
                match ev {
                    Event::Text(t) => got.push_str(&t),
                    Event::Start(pulldown_cmark::Tag::CodeBlock(_)) => is_code_block = true,
                    _ => {}
                }
            }
            assert!(
                !is_code_block,
                "input {input:?} still opened a code block: {md:?}"
            );
            assert_eq!(
                got, input,
                "input {input:?}: the whitespace must render back: {md:?}"
            );
        }
    }

    #[test]
    fn comment_note_cannot_close_the_comment_early() {
        // The property, not a literal output string: wrap the note the same
        // way `markdown.rs`'s `Block::Raw` arm does, then require that the
        // only `-->` in the whole thing is the wrapper's own closer. A
        // one-shot `str::replace("--", "- -")` fails "--->" and "x--->y" --
        // it leaves an odd dash unreplaced right after a chunk ending in
        // `-`, and the two recombine.
        for note in ["--->", "x--->y", "-----", "----"] {
            let wrapped = format!("<!-- {} -->", comment_note(note));
            let body = wrapped
                .strip_suffix("-->")
                .expect("the wrapper always ends in the closer");
            assert!(
                !body.contains("-->"),
                "note {note:?} closed the comment early: {wrapped}"
            );
        }
    }
}
