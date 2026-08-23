//! Every rule for turning document text into Markdown that renders as that
//! text. Design spec `2026-08-09-markdown-escaping-design.md` §3.
//!
//! One function per output context, because the contexts do not share a
//! mechanism: a backslash escapes in flow text, is a literal character inside
//! a code span, is inert inside an HTML block, and means something else again
//! inside a YAML double-quoted scalar.

use kasane_ir::Inline;

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

/// The numeric character reference for a space or a tab; `None` for anything
/// else.
///
/// One function to carry the property both call sites below depend on: these
/// two characters, and no others, are what GFM's block scanner (leading
/// whitespace at a line start, §3) and its cell trimmer (a cell's own edges,
/// §3.3) act on. Stated once here rather than duplicated in each site's own
/// match, so a future third whitespace character GFM treats specially has one
/// place to be added instead of two call sites a reviewer has to notice are
/// supposed to agree.
fn ws_reference(c: char) -> Option<&'static str> {
    match c {
        ' ' => Some("&#32;"),
        '\t' => Some("&#9;"),
        _ => None,
    }
}

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
                Ctx::Cell => {
                    out.push_str("<br>");
                    // A `<br>` is inline markup inside the cell, not a fresh
                    // line -- GFM only trims a cell's own outer edges, never
                    // the run right after an internal `<br>`. Clearing this
                    // keeps `line_start` honest: without it, `Text("\n  x")`
                    // claimed the ` ` right after `<br>` was at column 0 and
                    // spent a character reference on it for no reason. That
                    // reference rendered as the space it names either way
                    // (verified against the parser), so this is a position
                    // fix, not a behaviour fix.
                    line_start = false;
                }
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
            if let Some(reference) = ws_reference(c) {
                out.push_str(reference);
                i += 1;
                line_start = false;
                continue;
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
/// 1–9 ASCII digits followed by `.` or `)`. `None` otherwise.
///
/// The 9-digit cap is CommonMark's own, not a defensive guess: verified
/// against the parser, `123456789. x` (9 digits) opens a real ordered list
/// and `1234567890. x` (10) does not, parsing as a plain paragraph. Without
/// the cap a 10-or-more-digit run got a needless backslash on its delimiter
/// — over-escaping, harmless to the rendered text, but not exact, and this
/// function's own doc used to claim "one or more" with no upper bound.
fn ordered_marker_delimiter(chars: &[char], i: usize) -> Option<usize> {
    if !chars.get(i)?.is_ascii_digit() {
        return None;
    }
    let mut j = i;
    while j - i < 9 && chars.get(j).is_some_and(char::is_ascii_digit) {
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

/// Disarm an ATX closing sequence at the end of a heading line (design spec
/// 2026-08-14 §4.2).
///
/// CommonMark strips a trailing run of `#` from an ATX heading — at **block**
/// level, from raw text, before inline parsing — when the run is preceded by a
/// space or tab, or is the whole content. `## Intro ###` therefore renders the
/// text `Intro`, losing document text and, with it, the id kasane computed
/// from the text the IR holds.
///
/// Escaping the first `#` of the run fixes both at once: the block-level scan
/// sees a `\` before the run and does not strip it, then inline parsing turns
/// `\#` back into a literal. `## Intro###` needs nothing, because a run with
/// no space before it was never a closing sequence.
///
/// This is a writer fix rather than an anchor-rule fix on purpose. Teaching
/// `anchor_slug` about closing sequences would buy parity by agreeing that the
/// rendered heading may drop text the document had — the escaping spec's §5
/// invariant, conceded rather than upheld.
pub(crate) fn atx_closing(escaped: &str) -> String {
    // Trailing blanks are dropped by the parser before it looks for the run,
    // so they do not protect it.
    let end = escaped.trim_end_matches([' ', '\t']).len();
    let run_start = escaped[..end].trim_end_matches('#').len();
    if run_start == end {
        return escaped.to_string();
    }
    let closes = escaped[..run_start]
        .chars()
        .next_back()
        .is_none_or(|c| c == ' ' || c == '\t');
    if !closes {
        return escaped.to_string();
    }
    let mut out = String::with_capacity(escaped.len() + 1);
    out.push_str(&escaped[..run_start]);
    out.push('\\');
    out.push_str(&escaped[run_start..]);
    out
}

/// Collapse a newline run that spans an inline boundary, for the one-line
/// contexts (§4).
///
/// `normalize_newlines` already collapses a run inside one `Inline::Text`, and
/// `kasane_gfm::fold_newlines` collapses one inside a single rendered string
/// — but neither can see two runs that meet across a boundary, because each
/// inline is rendered independently and `code_span` folds its own content to
/// a space before the outer fold ever runs. `anchor_fold` computes over the
/// concatenated `rendered_text`, where the two runs *are* adjacent, so it
/// predicts one separator where the renderer emitted two, and the
/// cross-reference it embeds is dead.
///
/// The fix lands here rather than in `kasane-gfm`'s fold on purpose: the
/// writer already has the inline-boundary information for free as it walks
/// the tree, where teaching the anchor side about it would mean handing
/// `code_span`'s padding rules to a function that only ever sees `&[Inline]`.
///
/// `Inline::FootnoteRef` is opaque here and that is correct — the reference is
/// visible text between two real separators, and `rendered_text` now agrees
/// with the fold about that.
pub(crate) fn fold_inline_newlines(inls: &[Inline]) -> Vec<Inline> {
    let mut pending = false;
    fold_seq(inls, 0, &mut pending)
}

fn fold_seq(inls: &[Inline], depth: usize, pending: &mut bool) -> Vec<Inline> {
    // This runs before `inlines_to_md_at`'s guard, so it carries its own:
    // `blocks_to_markdown` is public API over a public IR, and a hand-built
    // tree deeper than the bound would otherwise overflow the stack here
    // rather than being truncated there.
    //
    // Returns empty rather than `inls.to_vec()`: `Inline`'s derived `Clone`
    // is itself recursive, so cloning the remainder would only relabel the
    // recursion this guard exists to stop, as deep as whatever is left below
    // it. Empty is safe to return because it is unobservable — the only
    // consumer, `inlines_to_md_at`, has its own `MAX_INLINE_DEPTH` guard on a
    // fresh counter and discards everything past that depth before it is
    // ever read, so nothing this deep survives to be rendered either way.
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return Vec::new();
    }
    inls.iter()
        .map(|i| match i {
            Inline::Text(t) => Inline::Text(fold_leaf(t, pending)),
            Inline::Code(t) => Inline::Code(fold_leaf(t, pending)),
            Inline::Math(t) => Inline::Math(fold_leaf(t, pending)),
            Inline::Emph(x) => Inline::Emph(fold_seq(x, depth + 1, pending)),
            Inline::Strong(x) => Inline::Strong(fold_seq(x, depth + 1, pending)),
            Inline::Link { target, inlines } => Inline::Link {
                target: target.clone(),
                inlines: fold_seq(inlines, depth + 1, pending),
            },
            Inline::FootnoteRef(n) => {
                *pending = false;
                Inline::FootnoteRef(*n)
            }
        })
        .collect()
}

/// Fold one leaf's content, carrying `pending` in and out so a run that ends
/// this leaf and begins the next collapses to a single `\n` — which
/// `kasane_gfm::fold_newlines` then turns into the one space `anchor_fold`
/// predicted.
///
/// A third newline-run collapse loop, deliberately not unified with its two
/// siblings: `normalize_newlines` collapses a run *within* one string before
/// any escaping happens, `kasane_gfm::fold_newlines` collapses one within a
/// single already-rendered string, and this one collapses one *across* an
/// inline boundary, carrying state between calls via `pending` rather than
/// being a pure function of one string. That statefulness is exactly what
/// the other two cannot express and do not need — merging this into either
/// would give a context-free loop a cross-call side channel, reintroducing
/// the hazard `fold_inline_newlines` exists to fix in the first place.
fn fold_leaf(s: &str, pending: &mut bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '\n' || c == '\r' {
            if !*pending {
                out.push('\n');
            }
            *pending = true;
        } else {
            out.push(c);
            *pending = false;
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
    kasane_gfm::fold_newlines(&text(s, Ctx::Flow, Pos::Mid))
}

/// Restore whitespace at a rendered cell's trailing edge (§3.3).
///
/// GFM trims a cell's content before parsing it, so a trailing space or tab
/// is dropped outright — document text lost, the same defect the leading edge
/// had. The leading edge is covered by `Pos::LineStart`, because
/// `render_table` renders every cell at a line start; this edge is not a
/// positional question and cannot be.
///
/// Only the last character needs the reference: everything before it is no
/// longer at the trimmed edge. Symmetric with the leading rule, which likewise
/// only converts the first character of the run.
pub(crate) fn cell_edges(rendered: &str) -> String {
    let mut out = rendered.to_string();
    if let Some(reference) = out.chars().next_back().and_then(ws_reference) {
        out.truncate(out.len() - 1);
        out.push_str(reference);
    }
    out
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
    let content = kasane_gfm::fold_newlines(s);
    let content = if ctx == Ctx::Cell {
        content.replace('|', "\\|")
    } else {
        content
    };
    let ticks = "`".repeat(longest_backtick_run(&content) + 1);
    if content.is_empty() {
        // Rule 1: Empty content gets a single space (only acknowledged divergence from round-trip).
        // No longer an anchor divergence for anything the engine structured:
        // `kasane-core`'s `clone_inlines_at` canonicalizes `Inline::Code("")`
        // to `Inline::Code(" ")` on the way into the engine, so structured IR
        // reaches Rule 2 with the space already spelled, and the anchor sees
        // it. `fold_sections` is the only entry point that establishes that
        // invariant.
        //
        // Rule 1 therefore still runs for a caller who renders hand-built IR
        // through `blocks_to_markdown` -- no `assign_paths`, so no anchor to
        // diverge from -- and for one who assembles a `SectionTree` themselves
        // instead of going through `fold_sections`. That second caller *does*
        // get anchors: `SectionTree`/`SectionNode` have all-`pub` fields, no
        // `#[non_exhaustive]` and no private constructor, and `balance` and
        // `assign_paths` are exported, so un-canonicalized inlines can reach
        // the anchor rule that way (`balance`'s merge path clones through
        // `clone_inlines_at` and so canonicalizes the titles it demotes, but an
        // unmerged section title or a hand-placed body heading stays raw).
        // Nothing in this repo takes that path, so it is not a shipped bug --
        // but the API does not forbid it, and this comment does not claim it
        // does.
        //
        // Two adjacent empty spans are no longer a special case either: the
        // run scan in `inlines_to_md_at` renders them as one span over their
        // concatenated content, so `[Code(" "), Code(" ")]` prints `` `  ` ``
        // and Rule 2 leaves both spaces intact. Rule 1 is reached only by a
        // run whose whole concatenation is empty.
        //
        // Rule 1 and Rule 2 must keep printing the same bytes for that
        // canonicalization to stay invisible; see
        // `code_span_pads_an_empty_span_to_exactly_what_a_single_space_renders`.
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

/// Whether [`math_span`] will degrade this content to a code span rather than
/// print it as `$…$`.
///
/// Named rather than inlined into the branch below because
/// [`delim`] has to ask the same question: a degrading `Inline::Math` prints
/// with backticks, so it collides with a neighbouring code span exactly as a
/// second `Inline::Code` would (design spec §2.1). With the rule in one place,
/// widening what math degrades widens the delimiter class in the same edit and
/// cannot silently fail to.
pub(crate) fn math_degrades(s: &str) -> bool {
    s.contains('$') || s.contains('\n') || s.contains('\r')
}

/// The delimiter an inline prints with, where two neighbours printing the same
/// one would collide.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Delim {
    /// A code span: `` `…` ``, at whatever fence length its content forces.
    Backtick,
    /// `*…*`.
    Emph,
    /// `**…**`.
    Strong,
}

impl Delim {
    /// The character a **child** of a run is predicted to print.
    ///
    /// Conservative by design (design spec `2026-08-23-delimiter-choice-ordering-design.md`
    /// §4.2). A run chooses its own character before its children render, so a
    /// child's character is not yet decided when the splice rules need it.
    /// Rather than recurse, they assume `*`: exact when the run chose `_`,
    /// because a child may not take its parent's character; conservative when
    /// the run chose `*`, where a child that could safely have taken `_` is
    /// spliced first instead. The cost is a missed recovery, never a
    /// corruption.
    ///
    /// This is *not* the character a run prints — that is [`Mark::ch`], which
    /// is chosen per run. The two were one method until 2026-08-23 and had to
    /// be separated when the choice became real.
    pub(crate) fn child_ch(self) -> char {
        match self {
            Delim::Backtick => '`',
            Delim::Emph | Delim::Strong => '*',
        }
    }
}

/// A delimiter class together with the character a run has **chosen** to spell
/// it with.
///
/// Two runs collide when they share a character, not when they share a class:
/// `*` and `**` abut into one `***` run a parser splits somewhere the writer
/// did not intend, while a backtick beside a `*` is simply two characters.
/// Keying the splice rules on this value rather than on [`Delim`] is what
/// states that rule as written, instead of leaving it true by the coincidence
/// that this writer once never spelled emphasis with `_`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Mark {
    pub(crate) class: Delim,
    pub(crate) ch: char,
}

impl Mark {
    pub(crate) fn new(class: Delim, ch: char) -> Mark {
        debug_assert!(
            match class {
                Delim::Backtick => ch == '`',
                Delim::Emph | Delim::Strong => ch == '*' || ch == '_',
            },
            "a {class:?} cannot be spelled with {ch:?}"
        );
        Mark { class, ch }
    }

    /// The literal this mark opens and closes with.
    pub(crate) fn markup(self) -> &'static str {
        match (self.class, self.ch) {
            (Delim::Backtick, _) => "`",
            (Delim::Emph, '_') => "_",
            (Delim::Strong, '_') => "__",
            (Delim::Strong, _) => "**",
            (Delim::Emph, _) => "*",
        }
    }
}

/// Which delimiter this inline prints with, or `None` if it prints none that
/// can collide with a neighbour's.
///
/// Keyed on what is **printed**, not on the `Inline` variant, and that is the
/// whole reason this function exists rather than a `matches!` at the call
/// site: [`math_span`] degrades unsafe content to a code span, so
/// `[Code("x"), Math("a$b")]` prints two backtick spans and fuses exactly as
/// two `Code` inlines would. A rule matching `Inline::Code` alone would look
/// complete and leave that shape broken (design spec §2.1).
///
/// `Inline::Math` that does not degrade is `None` on purpose: `$x$$y$` is read
/// as two inline maths, and the two spans could not be merged even if they did
/// collide — `$xy$` states a different equation (design spec § Non-goals).
pub(crate) fn delim(i: &Inline) -> Option<Delim> {
    match i {
        Inline::Code(_) => Some(Delim::Backtick),
        Inline::Math(t) if math_degrades(t) => Some(Delim::Backtick),
        Inline::Emph(_) => Some(Delim::Emph),
        Inline::Strong(_) => Some(Delim::Strong),
        _ => None,
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
    if math_degrades(s) {
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
/// one folds to a space — the same treatment `kasane_gfm::fold_newlines`
/// already gives a newline, and for the same reason: folding, not dropping,
/// is what keeps two words from silently fusing into one.
pub(crate) fn yaml_scalar(s: &str) -> String {
    let flat = kasane_gfm::fold_newlines(s);
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
    for c in kasane_gfm::fold_newlines(s).chars() {
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

    /// CommonMark caps an ordered-list marker at 9 digits (verified against
    /// the parser: `123456789.` opens a real list, `1234567890.` does not,
    /// parsing as plain text). A 10th digit is therefore not part of any
    /// marker a real parser would recognize, so it needs no escape.
    #[test]
    fn flow_does_not_escape_a_marker_past_the_nine_digit_cap() {
        assert_eq!(
            text("123456789. one", Ctx::Flow, Pos::LineStart),
            "123456789\\. one"
        );
        assert_eq!(
            text("1234567890. one", Ctx::Flow, Pos::LineStart),
            "1234567890. one"
        );
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

    /// A `<br>` is inline markup inside the cell, not a fresh line: GFM's
    /// cell-trim rule only ever looks at the cell's own outer edges, never at
    /// what follows an interior `<br>`. `line_start` must not survive one, or
    /// `Text("\n  x")` claims the space right after `<br>` is at column 0 and
    /// spends a character reference on it for no reason.
    #[test]
    fn line_start_does_not_survive_a_br() {
        assert_eq!(text("\n  x", Ctx::Cell, Pos::LineStart), "<br>  x");
        assert_eq!(text("\n\tx", Ctx::Cell, Pos::LineStart), "<br>\tx");
    }

    /// The reference and the literal space it replaces must render
    /// identically once a real GFM cell-trim rule is in play: only the last
    /// character of the whole cell is at a trimmed edge, so the run right
    /// after an interior `<br>` was never going to be trimmed either way.
    #[test]
    fn the_position_fix_after_a_br_does_not_change_what_renders() {
        use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

        fn parse_cell_body(md: &str) -> String {
            let mut opts = Options::empty();
            opts.insert(Options::ENABLE_TABLES);
            let (mut in_cell, mut seen_header_end, mut out) = (false, false, String::new());
            for ev in Parser::new_ext(md, opts) {
                match ev {
                    Event::Start(Tag::TableCell) => in_cell = true,
                    Event::End(TagEnd::TableHead) => seen_header_end = true,
                    Event::Html(h) | Event::InlineHtml(h) if in_cell && seen_header_end => {
                        if h.trim() == "<br>" {
                            out.push('\n');
                        }
                    }
                    Event::Text(t) if in_cell && seen_header_end => out.push_str(&t),
                    _ => {}
                }
            }
            out
        }

        for input in ["\n  x", "\n\tx", "a\n  b", "\n  "] {
            let rendered = cell_edges(&text(input, Ctx::Cell, Pos::LineStart));
            let md = format!("| h |\n| --- |\n| {rendered} |\n");
            assert_eq!(
                parse_cell_body(&md),
                input,
                "input {input:?}: got cell {rendered:?} from {md:?}"
            );
        }
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
    fn atx_closing_disarms_only_a_real_closing_sequence() {
        assert_eq!(atx_closing("Intro ###"), "Intro \\###");
        assert_eq!(atx_closing("Intro\t###"), "Intro\t\\###");
        assert_eq!(atx_closing("Intro ### "), "Intro \\### ");
        assert_eq!(atx_closing("###"), "\\###");
        // No space before the run: never a closing sequence.
        assert_eq!(atx_closing("Intro###"), "Intro###");
        assert_eq!(atx_closing("Intro"), "Intro");
        assert_eq!(atx_closing(""), "");
        // Idempotent: after the guard there is nothing left to disarm.
        assert_eq!(atx_closing(&atx_closing("Intro ###")), "Intro \\###");
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
        // `rendered_text` feeds `Inline::Code`'s text into the line `anchor_slug`
        // slugs like any other text, so a code span in a heading has to fold
        // newlines the way `anchor_fold` does or the emitted fragment misses
        // by a hyphen. P2
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

    #[test]
    fn code_span_pads_an_empty_span_to_exactly_what_a_single_space_renders() {
        // The load-bearing invariant of the empty-code-span anchor fix:
        // `kasane-core` canonicalizes `Inline::Code("")` to `Inline::Code(" ")`
        // BEFORE anchors are assigned, which is only invisible to a reader
        // because Rule 1 and Rule 2 print the same bytes. If they ever stop
        // agreeing, that canonicalization starts silently rewriting documents
        // -- and the symptom is a changed page, not a failing render, so
        // nothing else would catch it.
        // Design spec 2026-08-14-empty-code-span-anchor-design.md §2.2.
        assert_eq!(code_span("", Ctx::Flow), code_span(" ", Ctx::Flow));
        assert_eq!(code_span("", Ctx::Cell), code_span(" ", Ctx::Cell));
        // Spelled out, so a future edit that breaks BOTH sides equally still
        // fails here rather than agreeing on something new.
        assert_eq!(code_span("", Ctx::Flow), "` `");
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
    fn ws_reference_covers_exactly_space_and_tab() {
        assert_eq!(ws_reference(' '), Some("&#32;"));
        assert_eq!(ws_reference('\t'), Some("&#9;"));
        // Not GFM's block-scanner alphabet: a non-breaking space, a newline,
        // and an ordinary letter all pass through untouched.
        for c in ['\u{a0}', '\n', 'x'] {
            assert_eq!(ws_reference(c), None, "{c:?} must not get a reference");
        }
    }

    /// GFM trims both ends of a cell. `Pos::LineStart` covers the leading
    /// edge (§3.2); the trailing edge is not a positional question, so it is
    /// fixed on the rendered cell. Only the last character needs the
    /// reference — everything before it is no longer at the trimmed edge (§3.3).
    #[test]
    fn cell_edges_restores_trailing_whitespace() {
        assert_eq!(cell_edges("x "), "x&#32;");
        assert_eq!(cell_edges("x\t"), "x&#9;");
        assert_eq!(cell_edges("x  "), "x &#32;");
        // Nothing to restore.
        assert_eq!(cell_edges("x"), "x");
        assert_eq!(cell_edges(""), "");
        // An all-whitespace cell: the leading rule (`Pos::LineStart`) converts
        // the first character, `cell_edges` converts the last, and the two
        // references meet with nothing literal between them for a two-space
        // run. Pins that this still parses back as the two spaces it names,
        // not as an empty or collapsed cell.
        assert_eq!(
            cell_edges(&text("  ", Ctx::Cell, Pos::LineStart)),
            "&#32;&#32;"
        );
    }

    /// A comparable view of a folded run: one `(kind, content)` pair per
    /// inline. `Inline` has no `PartialEq`, and widening a public IR type's
    /// derives for a writer test is not worth it — the leaf contents are the
    /// whole of what the fold changes.
    fn shape(inls: &[Inline]) -> Vec<(&'static str, String)> {
        inls.iter()
            .map(|i| match i {
                Inline::Text(t) => ("text", t.clone()),
                Inline::Code(t) => ("code", t.clone()),
                Inline::Math(t) => ("math", t.clone()),
                Inline::FootnoteRef(n) => ("ref", n.0.to_string()),
                Inline::Emph(_) => ("emph", String::new()),
                Inline::Strong(_) => ("strong", String::new()),
                Inline::Link { .. } => ("link", String::new()),
            })
            .collect()
    }

    /// The residual §4 exists for. `normalize_newlines` collapses a run inside
    /// one `Inline::Text` and `kasane_gfm::fold_newlines` collapses one inside
    /// a single rendered string, but neither sees two runs meeting across a
    /// boundary: each inline renders independently, and `code_span` folds its
    /// own content to a space before the outer fold ever runs. `anchor_fold`
    /// computes over the concatenated `rendered_text`, where the two runs
    /// *are* adjacent, so it predicted one separator and the renderer emitted
    /// two.
    #[test]
    fn a_newline_run_collapses_across_an_inline_boundary() {
        let got = fold_inline_newlines(&[Inline::Text("A\r".into()), Inline::Code("\nB".into())]);
        assert_eq!(
            shape(&got),
            vec![("text", "A\n".to_string()), ("code", "B".to_string())]
        );
    }

    #[test]
    fn an_empty_run_does_not_break_the_collapse() {
        let got = fold_inline_newlines(&[
            Inline::Text("A\r".into()),
            Inline::Text(String::new()),
            Inline::Text("\nB".into()),
        ]);
        assert_eq!(
            shape(&got),
            vec![
                ("text", "A\n".to_string()),
                ("text", String::new()),
                ("text", "B".to_string()),
            ]
        );
    }

    /// `Inline::FootnoteRef` renders as visible `[^1]` text, so a run must not
    /// collapse across it — doing so would drop a space GitHub really renders.
    /// This is not a residual any more: `rendered_text` renders the reference
    /// the same way, so the anchor and the fold agree about it.
    #[test]
    fn a_footnote_reference_is_opaque_to_the_fold() {
        use kasane_ir::NoteId;

        let got = fold_inline_newlines(&[
            Inline::Text("a\n".into()),
            Inline::FootnoteRef(NoteId(1)),
            Inline::Text("\nb".into()),
        ]);
        assert_eq!(
            shape(&got),
            vec![
                ("text", "a\n".to_string()),
                ("ref", "1".to_string()),
                ("text", "\nb".to_string()),
            ]
        );
    }

    /// §4.2, and the half of it that is easy to get backwards. The fold
    /// collapses *runs* and normalizes `\r`; it never turns a newline into a
    /// space, because that is `kasane_gfm::fold_newlines`, which runs long
    /// after `math_span` has picked its delimiter. So a lone newline inside
    /// one leaf survives and math still degrades to a code span — only the cross-boundary
    /// duplicate is dropped.
    #[test]
    fn a_lone_newline_inside_one_leaf_survives_the_fold() {
        let got = fold_inline_newlines(&[Inline::Math("a\nb".into())]);
        assert_eq!(shape(&got), vec![("math", "a\nb".to_string())]);

        let across =
            fold_inline_newlines(&[Inline::Text("A\r".into()), Inline::Math("\nB".into())]);
        assert_eq!(
            shape(&across),
            vec![("text", "A\n".to_string()), ("math", "B".to_string())]
        );
    }

    /// The fold runs *before* `inlines_to_md_at`'s depth guard, so it needs
    /// its own — a hand-built inline tree deeper than the bound would
    /// otherwise overflow the stack here instead of being truncated there.
    ///
    /// `10_000` matches the depth `tests/inline_depth.rs`'s
    /// `deep_inline_nesting_does_not_abort` uses for the sibling depth guard
    /// in `inlines_to_md_at`, not `MAX_INLINE_DEPTH` (256) plus a small
    /// margin: the guard caps *actual* recursion at exactly
    /// `MAX_INLINE_DEPTH` regardless of how deep the input nominally is, so a
    /// nominal depth only slightly past the bound cannot tell "guard
    /// present" apart from "guard absent, but that depth happens not to
    /// overflow anyway" — it takes a depth in `inline_depth.rs`'s magnitude
    /// to genuinely risk a stack-overflow abort if the guard were missing or
    /// broken. No `RUST_MIN_STACK` override is needed here, matching that
    /// sibling test's convention, because the guard returns rather than
    /// cloning the remainder (see `fold_seq`).
    #[test]
    fn the_fold_stops_at_the_inline_depth_bound() {
        let mut deep = vec![Inline::Text("x".into())];
        for _ in 0..10_000 {
            deep = vec![Inline::Emph(deep)];
        }
        // Must return rather than recurse to exhaustion / abort the process.
        let got = fold_inline_newlines(&deep);
        assert!(!got.is_empty(), "must return rather than abort");
    }

    /// The depth-bound test above only proves the guard fires; it makes no
    /// claim about what comes back. Pin the other half, the way
    /// `inline_depth.rs`'s `nesting_within_the_bound_is_preserved` does for
    /// the sibling guard: at a depth far under the bound, nothing truncates,
    /// so a newline run split across a boundary inside the nested `Emph`s
    /// must still collapse to exactly the shape
    /// `a_newline_run_collapses_across_an_inline_boundary` pins at depth
    /// zero. This is the check that would fail if `fold_seq`/`fold_leaf`
    /// dropped or corrupted content while threading `pending` through the
    /// recursion — an empty or non-empty `Vec` alone cannot show that.
    #[test]
    fn folding_within_the_bound_still_collapses_correctly() {
        const DEPTH: usize = 8;
        let mut deep = vec![Inline::Text("A\r".into()), Inline::Code("\nB".into())];
        for _ in 0..DEPTH {
            deep = vec![Inline::Emph(deep)];
        }

        let got = fold_inline_newlines(&deep);

        // Unwrap the DEPTH layers of Emph the fold must have preserved
        // faithfully, down to the leaves the boundary case landed on.
        let mut cur = got.as_slice();
        for _ in 0..DEPTH {
            cur = match cur {
                [Inline::Emph(inner)] => inner.as_slice(),
                other => panic!("expected a single Emph wrapper, got {other:?}"),
            };
        }
        assert_eq!(
            shape(cur),
            vec![("text", "A\n".to_string()), ("code", "B".to_string())]
        );
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

    /// `math_degrades` is `math_span`'s own branch condition, extracted so
    /// `delim` can ask the same question (design spec §2.1). The two must agree
    /// forever: this test asserts the predicate against `math_span`'s observable
    /// output rather than against a second copy of the expression, so an edit to
    /// either one that does not move the other fails here.
    #[test]
    fn math_degrades_agrees_with_what_math_span_prints() {
        for s in ["a$b", "$", "a\nb", "a\rb"] {
            assert!(math_degrades(s), "{s:?} should degrade");
            assert_eq!(math_span(s, Ctx::Flow), code_span(s, Ctx::Flow), "{s:?}");
        }
        for s in ["x", "\\frac{1}{2}", "a b"] {
            assert!(!math_degrades(s), "{s:?} should not degrade");
            assert_eq!(math_span(s, Ctx::Flow), format!("${s}$"), "{s:?}");
        }
    }
}
