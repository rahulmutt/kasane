use crate::escape::{self, Ctx, Pos};
use kasane_ir::{AssetBag, Block, Inline, RefTarget, Table};

pub fn blocks_to_markdown(blocks: &[Block], assets: &AssetBag) -> String {
    blocks_to_markdown_at(blocks, assets, 0)
}

fn blocks_to_markdown_at(blocks: &[Block], assets: &AssetBag, depth: usize) -> String {
    let mut out = String::new();
    for b in blocks {
        render_block(b, assets, &mut out, depth);
        out.push('\n');
    }
    out
}

fn render_block(b: &Block, assets: &AssetBag, out: &mut String, depth: usize) {
    // Defence in depth: section::clone_block already truncated anything that
    // reached the engine through an adapter or a caller into a shallow
    // Block::Raw, so this guard is not a second truncation stacked on that
    // one -- it covers a caller who builds a `Vec<Block>` by hand and calls
    // `blocks_to_markdown` directly, bypassing `structure()` entirely.
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        out.push_str("<!-- nesting truncated at the block depth bound -->\n");
        return;
    }
    match b {
        Block::Heading { level, inlines, .. } => {
            for _ in 0..(*level).min(6) {
                out.push('#');
            }
            out.push(' ');
            let inlines = escape::fold_inline_newlines(inlines);
            out.push_str(&escape::atx_closing(&kasane_gfm::fold_newlines(
                &inlines_to_md(&inlines, Ctx::Flow, Pos::Mid),
            )));
            out.push('\n');
        }
        Block::Para(inls) => {
            out.push_str(&inlines_to_md(inls, Ctx::Flow, Pos::LineStart));
            out.push('\n');
        }
        Block::List { ordered, items } => {
            for (i, item) in items.iter().enumerate() {
                let marker = if *ordered {
                    format!("{}. ", i + 1)
                } else {
                    "- ".to_string()
                };
                let mut inner = String::new();
                for bb in item {
                    render_block(bb, assets, &mut inner, depth + 1);
                }
                // Continuation lines are indented by the marker's own width;
                // without it an item holding a paragraph and a nested list
                // drops to column zero on its second line and leaves the item
                // (§4.3). The first block still renders on the marker's line,
                // which is what keeps `- ## Notes` intact.
                let indent = " ".repeat(marker.chars().count());
                out.push_str(&marker);
                out.push_str(&indent_continuation(inner.trim_end(), &indent));
                out.push('\n');
            }
        }
        Block::Table(t) => render_table(t, out),
        Block::Figure {
            image,
            caption,
            number,
        } => {
            let fname = assets
                .items
                .iter()
                .find(|a| a.key == image.key)
                .map(|a| a.filename.as_str())
                .unwrap_or("missing");
            let caption = escape::fold_inline_newlines(caption);
            let alt = kasane_gfm::fold_newlines(&inlines_to_md(&caption, Ctx::Flow, Pos::Mid));
            out.push_str(&format!(
                "![{}](_assets/{})\n",
                alt,
                escape::dest_path(fname)
            ));
            if let Some(n) = number {
                // Escaped like any other document text, even though every
                // adapter sets `None` today: `blocks_to_markdown` is public
                // API over a public IR, and "escape.rs is the only path from
                // document text to an output buffer" is stated without
                // qualification. `Ctx::Flow` with `Pos::Mid`, because the `*`
                // before it is already on the line.
                out.push_str(&format!(
                    "*Figure {}: {}*\n",
                    escape::text(n, Ctx::Flow, Pos::Mid),
                    alt
                ));
            }
        }
        Block::CodeBlock { lang, text } => {
            out.push_str(&escape::fenced_block(text, lang.as_deref()));
        }
        Block::MathBlock(s) => out.push_str(&escape::math_block(s)),
        Block::Footnote { id, blocks } => {
            // Four spaces is GFM's footnote continuation indent. Without it a
            // body of more than one line puts its second line at column zero,
            // outside the definition, where it becomes a sibling paragraph
            // (§4.2).
            let body = blocks_to_markdown_at(blocks, assets, depth + 1);
            let body = body.trim();
            out.push_str(&format!(
                "[^{}]: {}\n",
                id.0,
                indent_continuation(body, "    ")
            ));
        }
        Block::Raw { note } => out.push_str(&format!("<!-- {} -->\n", escape::comment_note(note))),
    }
}

fn render_table(t: &Table, out: &mut String) {
    if t.has_merged {
        // An HTML block's content is raw: GFM parses no Markdown inside it, so
        // the inlines must be emitted as HTML tags and the text HTML-escaped.
        // Emitting `**bold**` here (as this branch did) renders with the
        // asterisks showing (§3.3).
        out.push_str("<table>\n");
        out.push_str("<tr>");
        for c in &t.header {
            out.push_str(&format!("<th>{}</th>", inlines_to_html(c, 0)));
        }
        out.push_str("</tr>\n");
        for r in &t.rows {
            out.push_str("<tr>");
            for c in r {
                out.push_str(&format!("<td>{}</td>", inlines_to_html(c, 0)));
            }
            out.push_str("</tr>\n");
        }
        out.push_str("</table>\n");
        return;
    }
    let cells = |row: &Vec<Vec<Inline>>| {
        let joined: Vec<String> = row
            .iter()
            .map(|c| escape::cell_edges(&inlines_to_md(c, Ctx::Cell, Pos::LineStart)))
            .collect();
        format!("| {} |", joined.join(" | "))
    };
    out.push_str(&cells(&t.header));
    out.push('\n');
    let sep: Vec<&str> = t.header.iter().map(|_| "---").collect();
    out.push_str(&format!("| {} |\n", sep.join(" | ")));
    for r in &t.rows {
        out.push_str(&cells(r));
        out.push('\n');
    }
}

/// Render inlines as HTML, for the merged-table fallback only.
///
/// Math is the one inline with no HTML spelling here: `$…$` is not parsed
/// inside an HTML block either, so a merged-cell equation degrades to its
/// literal LaTeX. That renders no worse than today, and the alternative is
/// emitting MathML the IR does not carry (§3.3).
fn inlines_to_html(inls: &[Inline], depth: usize) -> String {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return String::new();
    }
    let mut s = String::new();
    for i in inls {
        match i {
            Inline::Text(t) => s.push_str(&escape::text(t, Ctx::Html, Pos::Mid)),
            Inline::Emph(x) => s.push_str(&format!("<em>{}</em>", inlines_to_html(x, depth + 1))),
            Inline::Strong(x) => s.push_str(&format!(
                "<strong>{}</strong>",
                inlines_to_html(x, depth + 1)
            )),
            Inline::Code(t) => s.push_str(&format!(
                "<code>{}</code>",
                escape::text(t, Ctx::Html, Pos::Mid)
            )),
            Inline::Math(t) => s.push_str(&format!("${}$", escape::text(t, Ctx::Html, Pos::Mid))),
            Inline::Link {
                target: RefTarget::External(u),
                inlines,
            } => s.push_str(&format!(
                "<a href=\"{}\">{}</a>",
                escape::text(&escape::dest_url(u), Ctx::Html, Pos::Mid),
                inlines_to_html(inlines, depth + 1)
            )),
            Inline::Link { inlines, .. } => s.push_str(&inlines_to_html(inlines, depth + 1)),
            Inline::FootnoteRef(n) => s.push_str(&format!("[^{}]", n.0)),
        }
    }
    s
}

pub(crate) fn inlines_to_md(inls: &[Inline], ctx: Ctx, pos: Pos) -> String {
    inlines_to_md_at(inls, 0, ctx, pos)
}

/// One element of the flattened view the run scan walks: an inline, and the
/// depth at which it renders.
///
/// The depth travels *with* the element rather than being a single number for
/// the whole slice, because the elements of one view do not all sit at the same
/// level: a transparent link's children render one level below the link, and
/// `kasane_ir::MAX_INLINE_DEPTH` has to keep falling exactly where it fell for
/// them before. Flattening the depth away would silently move that boundary for
/// everything nested through a link.
type Flat<'a> = (&'a Inline, usize);

/// Build the flattened view of an inline sequence: the stream a *parser* sees,
/// rather than the IR's own sibling list.
///
/// An unresolved (non-`External`) `Link` prints as just its children, with no
/// brackets at all, so in the printed line those children stand beside whatever
/// stood beside the link. The run scan has to see them there or the collision
/// this item exists to close reopens through a link:
/// `[Code("x"), Link { Internal, [Code("y")] }]` otherwise prints two backtick
/// spans in a row and comes back as one span reading ``` x``y ```.
/// [`renders_empty`] already encoded this insight for the *empty* case; this is
/// the same insight at full strength, since the property that matters at a run
/// boundary is transparency and not vacuity (design spec §2.3).
///
/// `also` names a *second* transparency, used only from [`emphasis_run`]: inside
/// a run printing that delimiter, a container printing the same one emits no
/// delimiter a parser can tell apart from the run's own. `None` — every other
/// caller — matches nothing, since [`escape::delim`] returns `Some` for both
/// `Emph` and `Strong`.
///
/// **Only pointers are copied.** No `Inline` is cloned or rewritten, which is
/// the constraint design spec §2.2 imposes: `Inline`'s derived `Clone` recurses
/// once per nesting level, so a hand-built tree past the bound would overflow
/// the stack inside the clone *before* the depth guard could discard it. That
/// guard is the one below, applied while the view is built rather than after —
/// children at or past `MAX_INLINE_DEPTH` print nothing, so they enter no view.
fn flatten_into<'a>(
    inls: &'a [Inline],
    depth: usize,
    also: Option<escape::Delim>,
    out: &mut Vec<Flat<'a>>,
) {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return;
    }
    for i in inls {
        match i {
            Inline::Link { target, inlines } if !matches!(target, RefTarget::External(_)) => {
                flatten_into(inlines, depth + 1, also, out)
            }
            Inline::Emph(x) | Inline::Strong(x) if escape::delim(i) == also => {
                flatten_into(x, depth + 1, also, out)
            }
            _ => out.push((i, depth)),
        }
    }
}

/// Render an inline sequence, building the flattened view [`inlines_to_md_flat`]
/// scans. The depth guard lives in [`flatten_into`], which yields an empty view
/// at or past the bound.
fn inlines_to_md_at(inls: &[Inline], depth: usize, ctx: Ctx, pos: Pos) -> String {
    let mut view = Vec::new();
    flatten_into(inls, depth, None, &mut view);
    inlines_to_md_flat(&view, ctx, pos)
}

/// `pos` is threaded, not inferred: it names where the next character emitted
/// lands (design spec §2). It starts as whatever the caller passed and is
/// then recomputed after every arm, by four rules: an arm that appended
/// nothing leaves `pos` alone; output ending in `\n` re-arms `Pos::LineStart`
/// (covering both a run that opens on a fresh line and one that re-arms after
/// an interior newline, e.g. `[Text("a\n"), Text("- b")]`, since none of the
/// writer's own markup -- `*`, `**`, backticks, `[`, `$`, `[^1]` -- ever ends
/// in a newline); a `FootnoteRef` that fired at `Pos::LineStart` yields
/// `Pos::AfterFootnoteRef`, so `escape::text` can tell a following `:` apart
/// from an ordinary one (residuals spec §2); anything else yields `Pos::Mid`.
///
/// The loop walks *runs*, not items: a maximal group of neighbouring elements
/// that print with the same delimiter renders as one span over their
/// concatenated contents, because CommonMark cannot express two such spans in
/// a row and the writer's two delimiter pairs would otherwise fuse into one
/// span in the rendered line (design spec
/// `2026-08-15-adjacent-inline-fusion-design.md` §2).
///
/// It walks the *flattened* view rather than the IR's own siblings because a
/// parser sees the printed stream, in which a transparent link's children and a
/// fused emphasis run's members' children are siblings too. Scanning IR
/// siblings alone left the defect open one level down, at every container seam
/// (`[Emph([Code("x")]), Emph([Code("y")])]` printed `` *`x``y`* ``).
fn inlines_to_md_flat<'a>(items: &[Flat<'a>], ctx: Ctx, pos: Pos) -> String {
    let mut s = String::new();
    let mut pos = pos;
    let mut i = 0;
    while i < items.len() {
        let (inline, depth) = items[i];
        let before = pos;
        let len_before = s.len();
        let end = run_end(items, i);
        let members = &items[i..end];
        match escape::delim(inline) {
            Some(escape::Delim::Backtick) => {
                s.push_str(&escape::code_span(&backtick_run_content(members), ctx))
            }
            Some(escape::Delim::Emph) => {
                s.push_str(&emphasis_run(members, escape::Delim::Emph, ctx, pos, "*"))
            }
            Some(escape::Delim::Strong) => s.push_str(&emphasis_run(
                members,
                escape::Delim::Strong,
                ctx,
                pos,
                "**",
            )),
            // `delim` said this inline prints no delimiter that can collide,
            // so the run is this inline alone and it renders as it always has.
            None => match inline {
                // The only call to `escape::text` in the crate. Every other
                // arm here and above emits markup the writer chose, which must
                // not be escaped.
                Inline::Text(t) => s.push_str(&escape::text(t, ctx, pos)),
                Inline::Math(t) => s.push_str(&escape::math_span(t, ctx)),
                Inline::Link {
                    target: RefTarget::External(u),
                    inlines,
                } => s.push_str(&format!(
                    "[{}]({})",
                    kasane_gfm::fold_newlines(&inlines_to_md_at(
                        &escape::fold_inline_newlines(inlines),
                        depth + 1,
                        ctx,
                        pos
                    )),
                    escape::dest_url(u)
                )),
                Inline::FootnoteRef(n) => s.push_str(&format!("[^{}]", n.0)),
                // Unreachable: `delim` returns `Some` for all three, so they
                // are handled above. Asserted rather than merely commented --
                // if `delim` ever narrows, the content would otherwise vanish
                // in silence.
                Inline::Code(_) | Inline::Emph(_) | Inline::Strong(_) => debug_assert!(
                    escape::delim(inline).is_some(),
                    "escape::delim narrowed; this inline's content would be dropped"
                ),
                // Unreachable: `flatten_into` splices a transparent link's
                // children into the view in the link's own place, so no such
                // link is ever an element. Rendered rather than dropped all
                // the same, so that if the splice ever narrowed the output
                // would degrade to what it was before the splice existed
                // instead of losing the link's text outright.
                Inline::Link { inlines, .. } => {
                    debug_assert!(
                        false,
                        "a transparent link reached the emit loop; flatten_into must splice it"
                    );
                    s.push_str(&inlines_to_md_at(inlines, depth + 1, ctx, pos))
                }
            },
        }
        // Four rules (§2). An arm that appended nothing leaves the position
        // alone, so an empty text run between a reference and its colon does
        // not reset it. `Inline::FootnoteRef` always appends, so rule 3 is
        // never blocked by the length check. A run is one position step, not
        // one per member: only the run's own output has landed.
        if s.len() != len_before {
            pos = if s.ends_with('\n') {
                Pos::LineStart
            } else if matches!(inline, Inline::FootnoteRef(_)) && before == Pos::LineStart {
                Pos::AfterFootnoteRef
            } else {
                Pos::Mid
            };
        }
        i = end;
    }
    s
}

/// Whether this inline prints nothing at all.
///
/// Exact rather than conservative, and it has to be both ways: [`run_end`]
/// steps over these, so a false positive drops content a reader can see and a
/// false negative leaves a fused pair behind. Each arm mirrors the renderer
/// above. `escape::text` never deletes, so a `Text` prints nothing exactly
/// when it is empty. `emphasize` returns its inner string unchanged when that
/// string is blank, so a container prints nothing exactly when every child
/// does. And `inlines_to_md_at` returns the empty string at
/// `MAX_INLINE_DEPTH`, so a container whose children sit at the bound really
/// does print nothing — which is why this takes the caller's absolute `depth`
/// rather than counting from zero.
///
/// `Inline::Link` splits by target the same way the renderer does. An
/// `External` link always emits its `[...](...)` brackets
/// (`inlines_to_md_flat`'s own arm for it), so it is never vacuous even with
/// empty `inlines`. Any other target is transparent: [`flatten_into`] splices
/// its children into the view in its place, so no such link is ever an element
/// of a view and this arm answers only for one nested inside a container — an
/// `Emph` whose sole child is an empty internal link prints nothing, and the
/// scan has to know that.
///
/// Everything else is non-vacuous by construction: `Code("")` prints
/// `` ` ` ``, `Math("")` prints `$$`, a `FootnoteRef` prints `[^n]`.
fn renders_empty(i: &Inline, depth: usize) -> bool {
    match i {
        Inline::Text(t) => t.is_empty(),
        Inline::Emph(x) | Inline::Strong(x) => {
            depth + 1 >= kasane_ir::MAX_INLINE_DEPTH
                || x.iter().all(|c| renders_empty(c, depth + 1))
        }
        Inline::Link { target, inlines } if !matches!(target, RefTarget::External(_)) => {
            depth + 1 >= kasane_ir::MAX_INLINE_DEPTH
                || inlines.iter().all(|c| renders_empty(c, depth + 1))
        }
        _ => false,
    }
}

/// The exclusive end of the run of same-delimiter inlines starting at `start`.
///
/// A vacuous inline is stepped over rather than ending the run: it puts no
/// character between the two delimiters, so the collision happens across it
/// anyway. One inside a run is swallowed and never rendered, which is
/// equivalent, because the only thing it would have contributed is the empty
/// string.
///
/// A *trailing* vacuous tail — past the last member that actually matched the
/// delimiter — is swallowed too, rather than left at `k` for the outer loop to
/// walk again next iteration. That is output-identical, not merely
/// convenient: every member in that tail failed the delimiter-match branch
/// and was consumed by the vacuity branch instead, so `backtick_run_content`
/// (which only ever appends a `Code`/`Math`) and `emphasis_run` (which only
/// ever appends an `Emph`/`Strong`, and contributes nothing for a vacuous one)
/// already treat it as contributing nothing, and `pos` does not
/// move for a member that appends nothing either way. Returning the shorter
/// "last real match" bound instead would leave that tail for the *next* call
/// to re-walk from its own start — quadratic in the length of a long vacuous
/// run (e.g. `[Emph(vec![]); m]`, where every element is both
/// delimiter-bearing and vacuous) where the loop this replaced was linear.
fn run_end(items: &[Flat<'_>], start: usize) -> usize {
    let Some(want) = escape::delim(items[start].0) else {
        return start + 1;
    };
    let mut k = start + 1;
    while k < items.len()
        && (renders_empty(items[k].0, items[k].1) || escape::delim(items[k].0) == Some(want))
    {
        k += 1;
    }
    k
}

/// The content one code span carries for a whole backtick run.
///
/// A degrading `Inline::Math` contributes its raw LaTeX, which is exactly what
/// `math_span` would have handed `code_span` on its own — and the guard is
/// spelled here rather than left to `escape::delim`'s, so the invariant is
/// local to this function instead of an argument spanning two. Anything else in
/// the slice is a vacuous element `run_end` swallowed, and contributes nothing
/// by definition.
fn backtick_run_content(members: &[Flat<'_>]) -> String {
    let mut content = String::new();
    for &(m, _) in members {
        match m {
            Inline::Code(t) => content.push_str(t),
            Inline::Math(t) if escape::math_degrades(t) => content.push_str(t),
            _ => {}
        }
    }
    content
}

/// Render a run of adjacent `Emph` (or `Strong`) elements as one emphasized
/// span over the concatenation of their children.
///
/// The members' children are flattened into **one** view and scanned once, so a
/// delimiter-bearing inline at the end of one member's children and one at the
/// start of the next member's are neighbours to the scan exactly as they are to
/// a parser. Rendering each member's children through its own
/// `inlines_to_md_at` call — which is what design spec §2.2 originally asked
/// for — reopened this item's own §1 defect at every member seam, one level
/// down: `[Emph([Code("x")]), Emph([Code("y")])]` printed `` *`x``y`* ``.
///
/// A child that prints the run's *own* delimiter is transparent here too,
/// unless it is the run's whole printing content. The two cases really are
/// different, and both were measured against `pulldown-cmark`:
///
/// - **Alone**, the two delimiter pairs stack with nothing between them on
///   either side, and CommonMark reads the longer run as one nested span, so
///   the text survives: `Emph([Emph([Text("a")])])` prints `**a**` and recovers
///   `a`. Left exactly as it was — this is also the shape
///   `kasane-cli/tests/e2e.rs` reads the EPUB adapter's inline-flattening bound
///   through, one `*` per surviving `<em>` level.
/// - **Beside anything else**, the stack is broken up on one side, the parser
///   splits the run at the wrong place and the surplus delimiters leak into the
///   visible text: `[Emph([Emph([Text("a")])]), Emph([Text("bc")])]` fuses to an
///   inner buffer of `*a*bc`, prints `**a*bc*` and recovers `**abc`. Splicing
///   the inner container into the run's view removes the collision at its
///   source, and loses no structure that was ever expressible — CommonMark has
///   no spelling for `Emph` directly inside `Emph`.
///
/// The second case is a regression this item introduced and this closes: at its
/// base the pair printed `**a***bc*` and recovered `abc` intact.
///
/// There is no per-member `pos` bookkeeping any more: the scan below owns the
/// four `Pos` rules, and one scan over one view applies them once per run
/// member exactly as the outer loop does for any other neighbour.
fn emphasis_run<'a>(
    members: &[Flat<'a>],
    want: escape::Delim,
    ctx: Ctx,
    pos: Pos,
    markup: &str,
) -> String {
    let children = run_children(members, None);
    let children = if nests_alone(&children, want) {
        children
    } else {
        run_children(members, Some(want))
    };
    emphasize(&inlines_to_md_flat(&children, ctx, pos), markup)
}

/// The flattened view of every member's children, as one sequence.
fn run_children<'a>(members: &[Flat<'a>], also: Option<escape::Delim>) -> Vec<Flat<'a>> {
    let mut out = Vec::new();
    for &(m, depth) in members {
        if let Inline::Emph(x) | Inline::Strong(x) = m {
            flatten_into(x, depth + 1, also, &mut out);
        }
    }
    out
}

/// Whether a run's whole printing content is a single container that prints the
/// run's own delimiter — the one arrangement in which the two delimiter pairs
/// stack cleanly instead of leaking. See [`emphasis_run`].
fn nests_alone(children: &[Flat<'_>], want: escape::Delim) -> bool {
    let mut printing = children.iter().filter(|&&(c, d)| !renders_empty(c, d));
    match (printing.next(), printing.next()) {
        (Some(&(only, _)), None) => escape::delim(only) == Some(want),
        _ => false,
    }
}

/// Wrap already-rendered inner content in an emphasis delimiter, with any
/// whitespace at its edges moved *outside* the delimiters.
///
/// Two problems, one fix. `pos` guards `Inline::Text` but not the
/// markup the writer itself emits at column 0, so `Block::Para([Emph([Text("
/// x")])])` rendered `* x*` — which a GFM parser reads as a bullet list, not a
/// paragraph, losing the paragraph outright. Reachable from `<p><em>
/// Note:</em> …</p>`. And CommonMark's flanking rules mean a `*` with an
/// adjacent space is never an emphasis delimiter *anywhere*, so the same
/// output silently dropped the emphasis mid-line too.
///
/// Moving the whitespace out fixes both at once, and is what CommonMark's own
/// "emphasis cannot begin or end with whitespace" rule asks for: the rendered
/// text is unchanged (§5's invariant), the emphasis now actually applies, and
/// the first character on the line is a space rather than a bullet marker.
/// Content that is *entirely* whitespace (or empty) gets no delimiters at all,
/// since there is nothing left to emphasize and `**` at column 0 is markup for
/// its own sake.
fn emphasize(inner: &str, delim: &str) -> String {
    let core = inner.trim();
    if core.is_empty() {
        return inner.to_string();
    }
    let lead = &inner[..inner.len() - inner.trim_start().len()];
    let trail = &inner[inner.trim_end().len()..];
    format!("{lead}{delim}{core}{delim}{trail}")
}

/// Indent every line after the first by `indent`, leaving blank lines blank so
/// no line carries trailing whitespace.
fn indent_continuation(body: &str, indent: &str) -> String {
    let mut lines = body.lines();
    let mut out = String::new();
    if let Some(first) = lines.next() {
        out.push_str(first);
    }
    for line in lines {
        out.push('\n');
        if !line.trim().is_empty() {
            out.push_str(indent);
            out.push_str(line);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kasane_ir::*;

    #[test]
    fn renders_headings_emphasis_and_links() {
        let blocks = vec![
            Block::Heading {
                level: 2,
                id: BlockId(0),
                inlines: vec![Inline::Text("Title".into())],
            },
            Block::Para(vec![
                Inline::Strong(vec![Inline::Text("bold".into())]),
                Inline::Text(" and ".into()),
                Inline::Link {
                    target: RefTarget::External("../m.md#x".into()),
                    inlines: vec![Inline::Text("link".into())],
                },
            ]),
        ];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("## Title"));
        assert!(md.contains("**bold** and [link](../m.md#x)"));
    }

    #[test]
    fn renders_gfm_table() {
        let t = Table {
            header: vec![
                vec![Inline::Text("A".into())],
                vec![Inline::Text("B".into())],
            ],
            rows: vec![vec![
                vec![Inline::Text("1".into())],
                vec![Inline::Text("2".into())],
            ]],
            has_merged: false,
        };
        let md = blocks_to_markdown(&[Block::Table(t)], &AssetBag::default());
        assert!(md.contains("| A | B |"));
        assert!(md.contains("| --- | --- |"));
        assert!(md.contains("| 1 | 2 |"));
    }

    #[test]
    fn rendering_survives_deep_block_nesting() {
        const DEPTH: usize = 100_000;
        let mut blocks = vec![Block::Para(vec![Inline::Text("bottom".into())])];
        for _ in 0..DEPTH {
            blocks = vec![Block::List {
                ordered: false,
                items: vec![blocks],
            }];
        }
        // Wrap in a Footnote so this test also exercises the
        // `Block::Footnote` arm, which routes back through
        // `blocks_to_markdown_at` at `depth + 1` -- the site where an
        // increment applied in both `render_block` and
        // `blocks_to_markdown_at` would silently halve the effective bound.
        // Without this, nothing deep ever reached that arm.
        blocks = vec![Block::Footnote {
            id: kasane_ir::NoteId(1),
            blocks,
        }];

        // Must return normally, not abort.
        let md = blocks_to_markdown(&blocks, &kasane_ir::AssetBag { items: vec![] });

        // Order matters, exactly as `fuzz_entry::adapter`'s comment spells
        // out. `blocks` is 100_000 deep and `Block`'s derived `Drop` recurses,
        // so letting it fall out of scope aborts the process on the way out --
        // a second, independent stack overflow that would read as the code
        // under test failing when it had already returned cleanly. Tear it
        // down through the explicit worklist BEFORE the assertion, so nothing
        // owns it when a panic could unwind through this frame.
        kasane_ir::teardown_document(kasane_ir::Document {
            meta: kasane_ir::DocMeta {
                title: "T".into(),
                authors: vec![],
                language: None,
                source_format: "test".into(),
                source_path: "t".into(),
            },
            nodes: blocks
                .into_iter()
                .map(|block| kasane_ir::Node {
                    block,
                    prov: kasane_ir::Provenance::default(),
                })
                .collect(),
        });

        assert!(!md.is_empty());
    }

    /// Design spec §4: link destinations are emitted raw, with no
    /// percent-encoding, and that is safe because the character set of a path
    /// component is closed. Every character that would break a bare Markdown
    /// destination -- space, `(`, `)`, `#`, `?`, `%`, `.` -- is outside
    /// `\p{Word}` and is therefore already removed by the slug rule.
    ///
    /// This asserts the set rather than the argument, so widening the rule by
    /// hand fails here instead of silently emitting a broken link. Each bad
    /// character below must actually occur in at least one title, or its
    /// assertion is vacuously true and the widening it is meant to catch
    /// would pass silently: space/`don't` in "Don't Panic"; `(`/`)` in
    /// "Background & Notes (revised)"; `#`/`?`/`%` in "50% off #1?"; `/`/`\`
    /// in "a/b\c"; `.` in "v1.2 Final.".
    ///
    /// The first loop's `c.is_alphanumeric()` is deliberately narrower than
    /// the slug rule's own `\p{Word}` (it rejects combining marks, which
    /// `\p{Word}` keeps), which is exactly what makes it a useful check on
    /// these Latin/CJK titles rather than a tautology. A title containing a
    /// combining mark -- `हिन्दी`, for instance -- does NOT belong in this
    /// array: it would fail this test spuriously even though the engine's
    /// rule is correct for it (see `path_slug_is_a_filename_not_an_anchor`
    /// in `kasane-gfm`'s `slug.rs` for that coverage instead).
    #[test]
    fn path_slugs_contain_nothing_that_breaks_a_bare_destination() {
        for title in [
            "Don't Panic",
            "Background & Notes (revised)",
            "50% off #1?",
            "第二章",
            "a/b\\c",
            "v1.2 Final.",
        ] {
            let slug = kasane_gfm::path_slug_of(&[Inline::Text(title.into())]);
            for c in slug.chars() {
                assert!(
                    c == '-' || (c.is_alphanumeric() || c == '_'),
                    "path slug for {title:?} contains {c:?}, which is outside the closed set"
                );
            }
            for bad in [' ', '(', ')', '#', '?', '%', '/', '\\', '.'] {
                assert!(
                    !slug.contains(bad),
                    "path slug for {title:?} contains {bad:?}"
                );
            }
        }
    }

    /// The other half of design spec §4, which used to be dismissed rather
    /// than shown: `refs::relativize` emits `format!("{}#{}", rel, a)`, so the
    /// ANCHOR lands in a bare `[text](...)` destination too, and its character
    /// set is part of §4's argument.
    ///
    /// It is a different set from the path slug's -- wider by ZWJ/ZWNJ, and it
    /// keeps GFM's double hyphens and untrimmed tails -- so the sibling test
    /// above cannot cover it, and its `is_alphanumeric()` check would reject a
    /// combining mark and a zero-width joiner that both belong here. What
    /// actually has to hold is narrower and is what is asserted: no character
    /// that ends a bare destination or splits a link.
    ///
    /// The zero-width non-joiner is the interesting row. It is `Cf`, which is
    /// neither `char::is_whitespace()` nor `char::is_control()`, and
    /// CommonMark ends an unbracketed destination on ASCII whitespace or an
    /// unbalanced `)` -- so it rides through a raw destination intact, which
    /// is exactly what GFM parity requires of it.
    #[test]
    fn anchors_contain_nothing_that_breaks_a_bare_destination() {
        for title in [
            "Don't Panic",
            "Background & Notes (revised)",
            "50% off #1?",
            "第二章",
            "a/b\\c",
            "v1.2 Final.",
            "हिन्दी",
            "می\u{200C}رود",
            "Ⓐ Notes",
        ] {
            let anchor = kasane_gfm::anchors_for_headings(&[title.to_string()])
                .pop()
                .expect("one title in, one anchor out");
            for c in anchor.chars() {
                assert!(
                    !c.is_whitespace() && !c.is_control(),
                    "anchor for {title:?} contains {c:?}, which a bare destination cannot carry"
                );
            }
            for bad in [' ', '(', ')', '#', '?', '%', '/', '\\', '.'] {
                assert!(
                    !anchor.contains(bad),
                    "anchor for {title:?} contains {bad:?}"
                );
            }
        }
    }

    /// `rendering_survives_deep_block_nesting` only proves the guard fires
    /// -- `assert!(!md.is_empty())` would still pass at an effective bound of
    /// 1, since a lone truncation comment is non-empty too. Pin the other
    /// half: at a depth far under `kasane_ir::MAX_BLOCK_DEPTH` (128), nothing
    /// truncates, so the innermost payload text must survive verbatim into
    /// the rendered output.
    #[test]
    fn rendering_preserves_content_well_under_the_block_bound() {
        const DEPTH: usize = 10;
        let mut blocks = vec![Block::Para(vec![Inline::Text("innermost payload".into())])];
        for _ in 0..DEPTH {
            blocks = vec![Block::List {
                ordered: false,
                items: vec![blocks],
            }];
        }
        blocks = vec![Block::Footnote {
            id: kasane_ir::NoteId(1),
            blocks,
        }];

        let md = blocks_to_markdown(&blocks, &kasane_ir::AssetBag { items: vec![] });

        assert!(
            md.contains("innermost payload"),
            "payload text must survive this far under the bound: {md}"
        );
        assert!(
            !md.contains("nesting truncated"),
            "the guard must not fire this shallow: {md}"
        );
    }

    #[test]
    fn text_runs_are_escaped_but_markup_is_not() {
        let blocks = vec![Block::Para(vec![
            Inline::Text("a*b".into()),
            Inline::Strong(vec![Inline::Text("c[d".into())]),
        ])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        // The writer's own `**` survives; the document's `*` and `[` do not.
        assert!(md.contains("a\\*b**c\\[d**"), "got: {md}");
    }

    #[test]
    fn a_code_span_containing_a_backtick_does_not_break_out() {
        let blocks = vec![Block::Para(vec![Inline::Code("a ` b".into())])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("`` a ` b ``"), "got: {md}");
    }

    #[test]
    fn an_external_destination_is_encoded_and_its_label_escaped() {
        let blocks = vec![Block::Para(vec![Inline::Link {
            target: RefTarget::External("https://e.com/a b(1)".into()),
            inlines: vec![Inline::Text("see [this]".into())],
        }])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(
            md.contains("[see \\[this\\]](https://e.com/a%20b%281%29)"),
            "got: {md}"
        );
    }

    #[test]
    fn a_heading_stays_on_one_line() {
        let blocks = vec![Block::Heading {
            level: 2,
            id: BlockId(0),
            inlines: vec![Inline::Text("Title\nspilled".into())],
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("## Title spilled\n"), "got: {md}");
        assert_eq!(md.lines().filter(|l| l.starts_with("##")).count(), 1);
    }

    /// The two heading paths now fold newlines identically, and the fold
    /// collapses runs. Before, `Block::Heading` collapsed (via
    /// `escape::text`'s `normalize_newlines`) and `file_to_markdown`'s title
    /// heading did not, so `"A\n\nB"` rendered `## A B` in one and `# A  B` in
    /// the other -- and `anchor_slug`, which can only predict one rendered
    /// line, emitted `#a--b` against a heading GitHub ids as `a-b`.
    #[test]
    fn a_blank_line_in_a_heading_is_one_separator() {
        for (input, want) in [
            ("A\n\nB", "## A B\n"),
            ("A\r\n\r\nB", "## A B\n"),
            ("A\nB", "## A B\n"),
            // Literal spaces are a different mechanism and stay.
            ("A  B", "## A  B\n"),
        ] {
            let blocks = vec![Block::Heading {
                level: 2,
                id: BlockId(0),
                inlines: vec![Inline::Text(input.into())],
            }];
            let md = blocks_to_markdown(&blocks, &AssetBag::default());
            assert!(md.contains(want), "input {input:?} got: {md}");
        }
    }

    /// The anchor kasane embeds must equal the id GitHub computes from the
    /// rendered heading line. Before the fold this shape rendered `A  B`
    /// (two spaces — one from each run's independent fold) against an
    /// embedded `a-b`.
    #[test]
    fn a_newline_run_split_by_a_code_span_yields_one_separator() {
        use pulldown_cmark::{Event, Options, Parser};

        let blocks = vec![Block::Heading {
            level: 2,
            id: BlockId(0),
            inlines: vec![Inline::Text("A\r".into()), Inline::Code("\nB".into())],
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());

        let mut heading = String::new();
        let mut depth = 0;
        for ev in Parser::new_ext(&md, Options::empty()) {
            match ev {
                Event::Start(pulldown_cmark::Tag::Heading { .. }) => depth += 1,
                Event::End(pulldown_cmark::TagEnd::Heading(_)) => depth -= 1,
                Event::Text(t) | Event::Code(t) if depth > 0 => heading.push_str(&t),
                _ => {}
            }
        }
        assert_eq!(heading, "A B", "one separator, not two:\n{md}");
    }

    /// Emphasis whose content begins or ends with whitespace moves that
    /// whitespace outside the delimiters.
    ///
    /// The column-0 case this test used to pin here -- `* x*` read by GFM as
    /// a bullet list, dropping both the paragraph and the emphasis -- is
    /// superseded by Task 3's line-start rule: `escape::text` now replaces
    /// the leading space with a character reference before `emphasize` ever
    /// runs, so there is no leading whitespace left for `trim` to move
    /// outside the delimiters. See
    /// `emphasis_at_column_zero_keeps_its_leading_space_inside` for that case
    /// pinned end-to-end. What is left here is `emphasize`'s own behaviour
    /// away from a line start, where CommonMark's flanking rules are still
    /// the only thing standing between edge whitespace and dropped emphasis.
    #[test]
    fn emphasis_moves_edge_whitespace_outside_its_delimiters() {
        // Mid-line, the emphasis must survive rather than be dropped by the
        // flanking rules.
        let blocks = vec![Block::Para(vec![
            Inline::Text("a".into()),
            Inline::Strong(vec![Inline::Text(" x ".into())]),
            Inline::Text("b".into()),
        ])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("a **x** b"), "got: {md}");

        // Nothing to emphasize, mid-line: no delimiters at all, so `**`
        // cannot sit in the output as markup for its own sake. Kept off a
        // line start deliberately -- at column 0 the line-start rule always
        // converts the leading character, so whitespace-only content no
        // longer trims to empty there; see
        // `strong_of_pure_whitespace_at_column_zero_survives_as_a_real_paragraph`
        // for that case pinned instead.
        let blocks = vec![Block::Para(vec![
            Inline::Text("a ".into()),
            Inline::Strong(vec![Inline::Text("  ".into())]),
        ])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(!md.contains('*'), "got: {md}");
    }

    #[test]
    fn math_is_emitted_verbatim_but_a_dollar_in_prose_is_not() {
        let blocks = vec![Block::Para(vec![
            Inline::Math("x^2".into()),
            Inline::Text(" costs 5$".into()),
        ])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("$x^2$ costs 5\\$"), "got: {md}");
    }

    #[test]
    fn a_paragraph_beginning_with_a_line_start_character_is_escaped() {
        // A paragraph renders at column 0, so a leading LINE_START character
        // would otherwise open a different block (a bullet, a heading, a
        // blockquote, ...) instead of staying prose text.
        for c in ['#', '-', '+', '>', '=', '|'] {
            let blocks = vec![Block::Para(vec![Inline::Text(format!("{c} text"))])];
            let md = blocks_to_markdown(&blocks, &AssetBag::default());
            assert!(md.contains(&format!("\\{c} text")), "char {c:?}, got: {md}");
        }
    }

    #[test]
    fn a_paragraph_beginning_with_an_ordered_marker_is_escaped() {
        let blocks = vec![Block::Para(vec![Inline::Text("1. one".into())])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("1\\. one"), "got: {md}");
    }

    #[test]
    fn a_text_run_re_arms_line_start_after_an_interior_newline() {
        // The first `Inline::Text` ends in a newline, so the second run --
        // even though it is not the first inline in the paragraph -- really
        // does begin a line and must be escaped.
        let blocks = vec![Block::Para(vec![
            Inline::Text("a\n".into()),
            Inline::Text("- b".into()),
        ])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("a\n\\- b"), "got: {md}");
    }

    #[test]
    fn figure_alt_text_and_caption_are_escaped_and_the_asset_path_encoded() {
        let assets = AssetBag {
            items: vec![AssetItem {
                key: "k".into(),
                filename: "a b(1).png".into(),
                bytes: vec![],
            }],
        };
        let blocks = vec![Block::Figure {
            image: AssetRef {
                key: "k".into(),
                bytes_ref: 0,
            },
            caption: vec![Inline::Text("fig [1]".into())],
            // Deliberately a number with markup in it. `Some("1")` -- what
            // this test used to pass -- exercises no rule at all, so reverting
            // the `escape::text` on `number` left the suite green.
            number: Some("*3*".into()),
        }];
        let md = blocks_to_markdown(&blocks, &assets);
        assert!(
            md.contains("![fig \\[1\\]](_assets/a%20b%281%29.png)"),
            "got: {md}"
        );
        assert!(md.contains("*Figure \\*3\\*: fig \\[1\\]*"), "got: {md}");
    }

    #[test]
    fn a_code_block_containing_a_fence_does_not_break_out() {
        let blocks = vec![Block::CodeBlock {
            lang: Some("rust ignore".into()),
            text: "outer\n```\ninner".into(),
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(
            md.contains("````rust\nouter\n```\ninner\n````"),
            "got: {md}"
        );
    }

    #[test]
    fn a_raw_note_cannot_close_its_own_comment() {
        let blocks = vec![Block::Raw {
            note: "a --> b -".into(),
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(!md.contains("--> b"), "the note closed the comment: {md}");
        assert!(md.starts_with("<!-- "), "got: {md}");
        assert!(md.trim_end().ends_with("-->"), "got: {md}");
    }

    /// Both edges, through the real table renderer and a real parser.
    #[test]
    fn a_table_cell_keeps_the_whitespace_at_both_its_edges() {
        use pulldown_cmark::{Event, Options, Parser};

        let cell = |s: &str| vec![Inline::Text(s.to_string())];
        let blocks = vec![Block::Table(Table {
            header: vec![cell("h")],
            rows: vec![vec![cell("  x  ")]],
            has_merged: false,
        })];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());

        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        let mut in_cell = false;
        let mut body = String::new();
        let mut seen_header = false;
        for ev in Parser::new_ext(&md, opts) {
            match ev {
                Event::Start(pulldown_cmark::Tag::TableCell) => in_cell = true,
                Event::End(pulldown_cmark::TagEnd::TableHead) => seen_header = true,
                Event::Text(t) if in_cell && seen_header => body.push_str(&t),
                _ => {}
            }
        }
        assert_eq!(body, "  x  ", "both edges must survive:\n{md}");
    }

    /// A cell with no non-whitespace content at all: the leading rule
    /// (`Pos::LineStart`) converts the first character and `cell_edges`
    /// converts the last, so a two-space cell round-trips as two references
    /// with nothing literal between them (`escape::cell_edges_restores_trailing_whitespace`
    /// pins the string; this pins it through the real table renderer and a
    /// real parser).
    #[test]
    fn an_all_whitespace_table_cell_round_trips() {
        use pulldown_cmark::{Event, Options, Parser};

        let cell = |s: &str| vec![Inline::Text(s.to_string())];
        let blocks = vec![Block::Table(Table {
            header: vec![cell("h")],
            rows: vec![vec![cell("  ")]],
            has_merged: false,
        })];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());

        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        let mut in_cell = false;
        let mut body = String::new();
        let mut seen_header = false;
        for ev in Parser::new_ext(&md, opts) {
            match ev {
                Event::Start(pulldown_cmark::Tag::TableCell) => in_cell = true,
                Event::End(pulldown_cmark::TagEnd::TableHead) => seen_header = true,
                Event::Text(t) if in_cell && seen_header => body.push_str(&t),
                _ => {}
            }
        }
        assert_eq!(
            body, "  ",
            "an all-whitespace cell must not collapse:\n{md}"
        );
    }

    #[test]
    fn a_pipe_in_a_cell_does_not_split_the_row() {
        let t = Table {
            header: vec![vec![Inline::Text("a|b".into())]],
            rows: vec![vec![vec![Inline::Text("c\nd".into())]]],
            has_merged: false,
        };
        let md = blocks_to_markdown(&[Block::Table(t)], &AssetBag::default());
        assert!(md.contains("| a\\|b |"), "got: {md}");
        assert!(md.contains("| c<br>d |"), "got: {md}");
    }

    /// `pptx/slide.rs` pushes `Inline::Math` straight into a table cell, and
    /// `|` passes through the adapter's `map_text` untouched, so this reaches
    /// the writer from a real PPTX rather than only from hand-built IR.
    /// Unescaped, `$|x|$` splits into `$` and `x` and GFM drops the row's real
    /// last cell.
    #[test]
    fn math_in_a_cell_does_not_split_the_row_on_either_branch() {
        let t = Table {
            header: vec![
                vec![Inline::Text("h".into())],
                vec![Inline::Text("i".into())],
            ],
            rows: vec![vec![
                // Verbatim branch.
                vec![Inline::Math("|x|".into())],
                // Degrade branch: the `$` forces the code span.
                vec![Inline::Math("$|y|".into())],
            ]],
            has_merged: false,
        };
        let md = blocks_to_markdown(&[Block::Table(t)], &AssetBag::default());
        assert!(md.contains("| $\\|x\\|$ | `$\\|y\\|` |"), "got: {md}");
    }

    #[test]
    fn the_merged_path_emits_html_markup_and_html_escaped_text() {
        // GFM parses nothing inside an HTML block, so `**bold**` there renders
        // with its asterisks showing. Emit tags instead.
        let t = Table {
            header: vec![vec![Inline::Text("a & b".into())]],
            rows: vec![vec![vec![
                Inline::Strong(vec![Inline::Text("bold".into())]),
                Inline::Text(" <x>".into()),
                Inline::Code("c<d".into()),
            ]]],
            has_merged: true,
        };
        let md = blocks_to_markdown(&[Block::Table(t)], &AssetBag::default());
        assert!(md.contains("<th>a &amp; b</th>"), "got: {md}");
        assert!(
            md.contains("<td><strong>bold</strong> &lt;x&gt;<code>c&lt;d</code></td>"),
            "got: {md}"
        );
        assert!(!md.contains("**bold**"), "markdown markup leaked: {md}");
    }

    #[test]
    fn a_multi_block_footnote_body_stays_inside_its_definition() {
        let blocks = vec![Block::Footnote {
            id: kasane_ir::NoteId(1),
            blocks: vec![
                Block::Para(vec![Inline::Text("first".into())]),
                Block::Para(vec![Inline::Text("second".into())]),
            ],
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        let lines: Vec<&str> = md.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines[0], "[^1]: first", "got: {md}");
        assert_eq!(
            lines[1], "    second",
            "the second block escaped the definition: {md}"
        );
    }

    #[test]
    fn a_multi_block_list_item_stays_inside_its_item() {
        let blocks = vec![Block::List {
            ordered: false,
            items: vec![vec![
                Block::Para(vec![Inline::Text("first".into())]),
                Block::List {
                    ordered: false,
                    items: vec![vec![Block::Para(vec![Inline::Text("nested".into())])]],
                },
            ]],
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        let lines: Vec<&str> = md.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines[0], "- first", "got: {md}");
        assert_eq!(lines[1], "  - nested", "got: {md}");
    }

    #[test]
    fn an_ordered_item_indents_by_its_own_marker_width() {
        let blocks = vec![Block::List {
            ordered: true,
            items: vec![vec![
                Block::Para(vec![Inline::Text("a".into())]),
                Block::Para(vec![Inline::Text("b".into())]),
            ]],
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        let lines: Vec<&str> = md.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines[0], "1. a", "got: {md}");
        assert_eq!(lines[1], "   b", "got: {md}");
    }

    /// The end-to-end shape §1's table calls A. A matching definition has to
    /// be present or *no* footnote reference parses at all — without one,
    /// `[^1]` decomposes into bare text and the test measures the fixture
    /// rather than the fix.
    #[test]
    fn a_footnote_reference_at_column_zero_does_not_open_a_definition() {
        use pulldown_cmark::{Event, Options, Parser};

        let blocks = vec![
            Block::Para(vec![
                Inline::FootnoteRef(NoteId(1)),
                Inline::Text(": note".into()),
            ]),
            Block::Footnote {
                id: NoteId(1),
                blocks: vec![Block::Para(vec![Inline::Text("the definition".into())])],
            },
        ];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("[^1]\\: note"), "got:\n{md}");

        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_FOOTNOTES);
        let (mut refs, mut defs) = (0, 0);
        for ev in Parser::new_ext(&md, opts) {
            match ev {
                Event::FootnoteReference(_) => refs += 1,
                Event::Start(pulldown_cmark::Tag::FootnoteDefinition(_)) => defs += 1,
                _ => {}
            }
        }
        assert_eq!(refs, 1, "the reference must survive the escape:\n{md}");
        assert_eq!(
            defs, 1,
            "only the real Block::Footnote is a definition:\n{md}"
        );
    }

    /// An empty run between the reference and the colon must not reset the
    /// position — the IR permits it and the colon is still a delimiter (§2,
    /// rule 1).
    #[test]
    fn an_empty_run_does_not_reset_the_footnote_position() {
        let blocks = vec![Block::Para(vec![
            Inline::FootnoteRef(NoteId(1)),
            Inline::Text(String::new()),
            Inline::Text(": note".into()),
        ])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("[^1]\\: note"), "got:\n{md}");
    }

    /// A wrapped reference renders `*[^1]*: x`, which begins with `*` and was
    /// never a definition, so the `Emph` arm must yield `Pos::Mid` (§2, rule 3).
    #[test]
    fn a_wrapped_footnote_reference_leaves_the_colon_alone() {
        let blocks = vec![Block::Para(vec![
            Inline::Emph(vec![Inline::FootnoteRef(NoteId(1))]),
            Inline::Text(": x".into()),
        ])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("*[^1]*: x"), "got:\n{md}");
        assert!(!md.contains("\\:"), "no escape is needed here:\n{md}");
    }

    #[test]
    fn a_heading_leading_a_list_item_still_renders_on_the_marker_line() {
        // properties.rs's heading_anchors (parser-based, via parse_events)
        // depends on a real GFM parser reading this shape as a heading.
        let blocks = vec![Block::List {
            ordered: false,
            items: vec![vec![Block::Heading {
                level: 2,
                id: BlockId(0),
                inlines: vec![Inline::Text("Notes".into())],
            }]],
        }];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("- ## Notes"), "got: {md}");
    }

    /// §3.5. At column 0 this rendered `* x*`, which GFM reads as a bullet
    /// list — the paragraph was lost, and the emphasis was silently dropped
    /// too, because a `*` with adjacent whitespace is never a delimiter.
    /// `emphasize` moves edge whitespace outside the delimiters, but at a line
    /// start the reference has already replaced it, so `*&` is left-flanking
    /// and the space stays *inside* the emphasis where the IR put it.
    #[test]
    fn emphasis_at_column_zero_keeps_its_leading_space_inside() {
        use pulldown_cmark::{Event, Options, Parser};

        let blocks = vec![Block::Para(vec![Inline::Emph(vec![Inline::Text(
            " x".into(),
        )])])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.starts_with("*&#32;x*"), "got:\n{md}");

        let mut in_em = false;
        let mut emphasized = String::new();
        let mut is_list = false;
        for ev in Parser::new_ext(&md, Options::empty()) {
            match ev {
                Event::Start(pulldown_cmark::Tag::Emphasis) => in_em = true,
                Event::End(pulldown_cmark::TagEnd::Emphasis) => in_em = false,
                Event::Start(pulldown_cmark::Tag::List(_)) => is_list = true,
                Event::Text(t) if in_em => emphasized.push_str(&t),
                _ => {}
            }
        }
        assert!(!is_list, "a paragraph became a bullet list:\n{md}");
        assert_eq!(
            emphasized, " x",
            "the emphasis must apply and keep its space:\n{md}"
        );
    }

    /// §3.5, whitespace-only content. Before Task 3, `escape::text("  ",
    /// Flow, LineStart)` and `emphasize` both left the two spaces untouched,
    /// and a line of two bare spaces is a *blank* line to GFM — the whole
    /// paragraph, `Strong` and all, vanished from the rendered document
    /// instead of rendering as anything. That is a harder form of the same
    /// §5 violation `emphasis_at_column_zero_keeps_its_leading_space_inside`
    /// pins: content silently dropped, not merely misrendered.
    ///
    /// The line-start rule now converts the leading space to a reference
    /// before `emphasize` ever sees it, so the line is no longer blank: a
    /// real paragraph survives, with a `<strong>` materializing around one
    /// preserved space where before there was nothing at all. That is the
    /// correct direction under §5 -- preserved content that renders as
    /// (still-invisible) whitespace beats a block that disappears outright.
    #[test]
    fn strong_of_pure_whitespace_at_column_zero_survives_as_a_real_paragraph() {
        use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

        let blocks = vec![Block::Para(vec![Inline::Strong(vec![Inline::Text(
            "  ".into(),
        )])])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.starts_with("**&#32;** "), "got:\n{md}");

        let mut saw_paragraph = false;
        let mut saw_strong = false;
        let mut in_strong = false;
        let mut strong_text = String::new();
        let mut is_list = false;
        for ev in Parser::new_ext(&md, Options::empty()) {
            match ev {
                Event::Start(Tag::Paragraph) => saw_paragraph = true,
                Event::Start(Tag::Strong) => {
                    saw_strong = true;
                    in_strong = true;
                }
                Event::End(TagEnd::Strong) => in_strong = false,
                Event::Start(Tag::List(_)) => is_list = true,
                Event::Text(t) if in_strong => strong_text.push_str(&t),
                _ => {}
            }
        }
        assert!(saw_paragraph, "the paragraph must not vanish:\n{md}");
        assert!(saw_strong, "the strong element must not vanish:\n{md}");
        assert!(!is_list, "a paragraph became a bullet list:\n{md}");
        assert_eq!(
            strong_text, " ",
            "the preserved whitespace must round-trip:\n{md}"
        );
    }

    /// Render one paragraph and return its line, without the trailing newline.
    fn para(inls: Vec<Inline>) -> String {
        let md = blocks_to_markdown(&[Block::Para(inls)], &AssetBag::default());
        md.trim_end().to_string()
    }

    /// The text a real parser recovers from that same paragraph.
    ///
    /// The printed bytes are what these tests pin first, but bytes alone cannot
    /// tell a fused span from a collided one — `` *`x``y`* `` and `` *`xy`* ``
    /// differ by two characters and by the whole defect. This is the other half:
    /// what a reader actually sees. It must equal `kasane_gfm::rendered_text` of
    /// the same inlines, which is the equality P13 asserts over generated
    /// sequences.
    fn recovered(inls: Vec<Inline>) -> String {
        use pulldown_cmark::{Event, Options, Parser};

        let md = blocks_to_markdown(&[Block::Para(inls)], &AssetBag::default());
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_MATH);
        let mut out = String::new();
        for ev in Parser::new_ext(&md, opts) {
            if let Event::Text(t) | Event::Code(t) = ev {
                out.push_str(&t);
            }
        }
        out
    }

    /// Adjacent code spans render as one span over their concatenation.
    ///
    /// CommonMark cannot express two code spans in a row: the closing fence of
    /// the first and the opening fence of the second form a single backtick run,
    /// so `` `x` `` beside `` `y` `` came back as one span reading ``` x``y ```
    /// — visible content corruption (design spec §1).
    #[test]
    fn adjacent_code_spans_render_as_one_span() {
        assert_eq!(
            para(vec![Inline::Code("x".into()), Inline::Code("y".into())]),
            "`xy`"
        );
        assert_eq!(
            para(vec![
                Inline::Code("x".into()),
                Inline::Code("y".into()),
                Inline::Code("z".into()),
            ]),
            "`xyz`"
        );
    }

    /// The fence is computed from the concatenation, not from either member, so a
    /// run whose members each carry a backtick still gets a fence that closes it.
    #[test]
    fn a_fused_code_run_gets_a_fence_long_enough_for_the_concatenation() {
        assert_eq!(
            para(vec![Inline::Code("a`".into()), Inline::Code("`b".into())]),
            "``` a``b ```"
        );
    }

    /// Adjacent emphasis renders as one span. Undocumented before this item and
    /// worse than the code case: the collided delimiters came back as literal
    /// asterisks in the visible text (`*a**b*` parses to one `<em>` reading
    /// `a**b`).
    #[test]
    fn adjacent_emphasis_renders_as_one_span() {
        let em = |s: &str| Inline::Emph(vec![Inline::Text(s.into())]);
        assert_eq!(para(vec![em("a"), em("b")]), "*ab*");

        let st = |s: &str| Inline::Strong(vec![Inline::Text(s.into())]);
        assert_eq!(para(vec![st("a"), st("b")]), "**ab**");
        assert_eq!(para(vec![st("a"), st("b"), st("c")]), "**abc**");
    }

    /// An inline that prints nothing does not break a run. It cannot: it puts no
    /// character between the two delimiters, so the collision happens anyway
    /// (design spec §2.3).
    #[test]
    fn an_inline_that_prints_nothing_does_not_break_a_run() {
        assert_eq!(
            para(vec![
                Inline::Code("x".into()),
                Inline::Text(String::new()),
                Inline::Code("y".into()),
            ]),
            "`xy`"
        );
        assert_eq!(
            para(vec![
                Inline::Emph(vec![Inline::Text("a".into())]),
                Inline::Emph(vec![]),
                Inline::Emph(vec![Inline::Text("b".into())]),
            ]),
            "*ab*"
        );
    }

    /// A whitespace-only inline is *not* vacuous. `emphasize` prints it as a bare
    /// space, which genuinely separates the two code spans, and fusing across it
    /// would delete a character a reader can see.
    #[test]
    fn a_whitespace_only_inline_separates_a_run() {
        assert_eq!(
            para(vec![
                Inline::Code("x".into()),
                Inline::Emph(vec![Inline::Text(" ".into())]),
                Inline::Code("y".into()),
            ]),
            "`x` `y`"
        );
    }

    /// A `Math` inline whose content forces `math_span` to degrade prints with
    /// backticks, so it joins the backtick class. Keying the class on
    /// `Inline::Code` alone would leave every shape here broken (design spec
    /// §2.1).
    #[test]
    fn a_degrading_math_span_joins_the_backtick_class() {
        assert_eq!(
            para(vec![Inline::Code("x".into()), Inline::Math("a$b".into())]),
            "`xa$b`"
        );
        assert_eq!(
            para(vec![Inline::Math("a$b".into()), Inline::Code("y".into())]),
            "`a$by`"
        );
        assert_eq!(
            para(vec![Inline::Math("$".into()), Inline::Math("$".into())]),
            "`$$`"
        );
    }

    /// The run scan reaches every nesting level, because every inline sequence in
    /// the crate goes through `inlines_to_md_at`.
    #[test]
    fn a_run_nested_inside_emphasis_fuses_too() {
        assert_eq!(
            para(vec![Inline::Emph(vec![
                Inline::Code("x".into()),
                Inline::Code("y".into()),
            ])]),
            "*`xy`*"
        );
    }

    /// `Ctx` is threaded unchanged, so a cell's `|` escaping applies across the
    /// concatenation rather than per member.
    #[test]
    fn a_fused_run_in_a_table_cell_escapes_pipes_across_the_concatenation() {
        let t = Table {
            header: vec![vec![Inline::Text("H".into())]],
            rows: vec![vec![vec![
                Inline::Code("a|b".into()),
                Inline::Code("c".into()),
            ]]],
            has_merged: false,
        };
        let md = blocks_to_markdown(&[Block::Table(t)], &AssetBag::default());
        assert!(md.contains(r"| `a\|bc` |"), "{md}");
    }

    /// The shapes that must NOT fuse, so a later change that over-fuses fails
    /// something. Each was measured against `pulldown-cmark` and recovers its
    /// text intact today (design spec §1, "Confirmed").
    #[test]
    fn inlines_with_different_delimiters_are_left_alone() {
        let em = |s: &str| Inline::Emph(vec![Inline::Text(s.into())]);
        let st = |s: &str| Inline::Strong(vec![Inline::Text(s.into())]);

        assert_eq!(para(vec![em("a"), st("b")]), "*a***b**");
        assert_eq!(
            para(vec![Inline::Code("x".into()), Inline::Math("y".into())]),
            "`x`$y$"
        );
        assert_eq!(
            para(vec![Inline::Math("x".into()), Inline::Math("y".into())]),
            "$x$$y$"
        );
    }

    /// The one output change this item makes to a shape that was already correct.
    ///
    /// `emphasize` hoists a trailing space outside the delimiters, so this pair
    /// printed `*a* *b*` and parsed as two `<em>`s. The rule is uniform, so it
    /// now prints one span. Same text, one element where there were two; the
    /// alternative was a second copy of `emphasize`'s hoisting rule living in the
    /// run scan (design spec §2.4).
    #[test]
    fn a_whitespace_separated_emphasis_pair_fuses_too() {
        assert_eq!(
            para(vec![
                Inline::Emph(vec![Inline::Text("a ".into())]),
                Inline::Emph(vec![Inline::Text("b".into())]),
            ]),
            "*a b*"
        );
    }

    /// An unresolved link (any `RefTarget` other than `External`) with no
    /// printing children renders through the "unresolved -> text" arm as
    /// nothing at all — no brackets, no text — so it is vacuous like an empty
    /// `Emph`, and must not break a run. Before this test's fix, `renders_empty`
    /// reported `false` for every `Link`, so this exact shape still printed
    /// `` `x``y` `` — the fusion bug reopened through a different inline.
    #[test]
    fn an_unresolved_link_with_no_printing_children_does_not_break_a_run() {
        assert_eq!(
            para(vec![
                Inline::Code("x".into()),
                Inline::Link {
                    target: RefTarget::Internal(BlockId(0)),
                    inlines: vec![],
                },
                Inline::Code("y".into()),
            ]),
            "`xy`"
        );
        assert_eq!(
            para(vec![
                Inline::Code("x".into()),
                Inline::Link {
                    target: RefTarget::Footnote(NoteId(1)),
                    inlines: vec![],
                },
                Inline::Code("y".into()),
            ]),
            "`xy`"
        );
    }

    /// The other direction: an unresolved link that DOES print something (its
    /// children are non-vacuous) is not vacuous itself, and genuinely
    /// separates two code spans — fusing across it would delete the text it
    /// carries.
    #[test]
    fn an_unresolved_link_with_printing_children_separates_a_run() {
        assert_eq!(
            para(vec![
                Inline::Code("x".into()),
                Inline::Link {
                    target: RefTarget::Internal(BlockId(0)),
                    inlines: vec![Inline::Text("mid".into())],
                },
                Inline::Code("y".into()),
            ]),
            "`x`mid`y`"
        );
    }

    /// The run scan sees across a *member* seam, not only across an IR sibling
    /// boundary.
    ///
    /// Rendering each member's children through its own `inlines_to_md_at` call
    /// — which is what design spec §2.2 originally asked for — left this item's
    /// own §1 defect open one level down: the last child of member *k* and the
    /// first child of member *k+1* met with nothing between them, exactly as
    /// the top-level pair did. `[Emph([Code("x")]), Emph([Code("y")])]` printed
    /// `` *`x``y`* `` and came back reading ``x``y``, in ordinary paragraph
    /// text. One flattened view over all the members' children closes it
    /// (review finding).
    #[test]
    fn a_run_fuses_across_its_members_children_too() {
        let cases = [
            (
                vec![
                    Inline::Emph(vec![Inline::Code("x".into())]),
                    Inline::Emph(vec![Inline::Code("y".into())]),
                ],
                "*`xy`*",
                "xy",
            ),
            (
                vec![
                    Inline::Emph(vec![Inline::Text("a".into()), Inline::Code("x".into())]),
                    Inline::Emph(vec![Inline::Code("y".into()), Inline::Text("b".into())]),
                ],
                "*a`xy`b*",
                "axyb",
            ),
            (
                vec![
                    Inline::Strong(vec![Inline::Code("x".into())]),
                    Inline::Strong(vec![Inline::Code("y".into())]),
                ],
                "**`xy`**",
                "xy",
            ),
        ];
        for (inls, bytes, text) in cases {
            assert_eq!(para(inls.clone()), bytes);
            assert_eq!(recovered(inls.clone()), text);
            assert_eq!(kasane_gfm::rendered_text(&inls), text);
        }
    }

    /// The same seam with an emphasis child rather than a code span, which is
    /// where this item regressed text that had been intact before it.
    ///
    /// `[Emph([Strong(a)]), Emph([Strong(b)])]` and
    /// `[Strong([Emph(a)]), Strong([Emph(b)])]` both recovered `ab` at this
    /// item's base — two independently rendered spans whose delimiters
    /// reassociated but kept every character — and recovered `ab**` once the
    /// members' children were concatenated as strings. The flattened view fuses
    /// the *inner* runs too, so one pair of delimiters is emitted where two
    /// collided.
    #[test]
    fn fusing_nested_emphasis_does_not_leak_its_delimiters() {
        let em = |x: Vec<Inline>| Inline::Emph(x);
        let st = |x: Vec<Inline>| Inline::Strong(x);
        let t = |s: &str| Inline::Text(s.into());

        let cases = [
            (
                vec![em(vec![em(vec![t("a")])]), em(vec![em(vec![t("b")])])],
                "*ab*",
            ),
            (
                vec![em(vec![st(vec![t("a")])]), em(vec![st(vec![t("b")])])],
                "***ab***",
            ),
            (
                vec![st(vec![em(vec![t("a")])]), st(vec![em(vec![t("b")])])],
                "***ab***",
            ),
        ];
        for (inls, bytes) in cases {
            assert_eq!(para(inls.clone()), bytes);
            assert_eq!(recovered(inls.clone()), "ab", "printed {bytes}");
            assert_eq!(kasane_gfm::rendered_text(&inls), "ab");
        }
    }

    /// A container printing the run's *own* delimiter is transparent inside it
    /// — unless it is the run's whole printing content, where the two delimiter
    /// pairs stack cleanly instead of leaking.
    ///
    /// The leaking case is a regression this item introduced:
    /// `[Emph([Emph(a)]), Emph([Text("bc")])]` printed `**a***bc*` at base and
    /// recovered `abc` intact, then printed `**a*bc*` once the members' children
    /// were concatenated and recovered `**abc`. The flat cases below it were
    /// broken at base too, in the same shape and for the same reason, and close
    /// with it.
    #[test]
    fn a_nested_emphasis_beside_other_content_joins_its_run() {
        let em = |x: Vec<Inline>| Inline::Emph(x);
        let t = |s: &str| Inline::Text(s.into());

        for inls in [
            vec![em(vec![em(vec![t("a")])]), em(vec![t("bc")])],
            vec![em(vec![t("a")]), em(vec![em(vec![t("bc")])])],
            vec![em(vec![em(vec![t("a")]), t("bc")])],
            vec![em(vec![t("a"), em(vec![t("b")]), t("c")])],
        ] {
            assert_eq!(para(inls.clone()), "*abc*");
            assert_eq!(recovered(inls.clone()), "abc");
            assert_eq!(kasane_gfm::rendered_text(&inls), "abc");
        }
    }

    /// The other side of that rule, pinned because something else depends on
    /// it: a nested emphasis that is the run's *entire* content keeps its own
    /// delimiters, one `*` per level. `kasane-cli/tests/e2e.rs` reads the EPUB
    /// adapter's inline-flattening bound through exactly this — 5000 nested
    /// `<em>` in, at most 64 `*` out — so splicing here uniformly would have
    /// collapsed a 64-deep chain to a single pair and broken a check that has
    /// nothing to do with fusion.
    #[test]
    fn a_lone_nested_emphasis_keeps_its_own_delimiters() {
        let em = |x: Vec<Inline>| Inline::Emph(x);
        let t = |s: &str| Inline::Text(s.into());

        assert_eq!(para(vec![em(vec![em(vec![t("a")])])]), "**a**");
        assert_eq!(para(vec![em(vec![em(vec![em(vec![t("a")])])])]), "***a***");
        // Vacuous company is no company: the nested span is still alone.
        assert_eq!(
            para(vec![em(vec![
                em(vec![t("a")]),
                Inline::Text(String::new())
            ])]),
            "**a**"
        );
    }

    /// An unresolved link renders as *just its children*, with no brackets, so
    /// a parser sees those children standing beside the link's own neighbours.
    /// The run scan has to see them there too: `renders_empty` already encoded
    /// that insight for an empty link, and treating a non-empty one as an
    /// opaque run-breaker left the collision open through it (review finding).
    ///
    /// Reachable from EPUB input like `<code>x</code><a
    /// href="#foo"><code>y</code></a>`, and identical at this item's base — a
    /// standing defect rather than a regression.
    #[test]
    fn a_transparent_link_does_not_hide_a_collision_from_the_run_scan() {
        let link = |x: Vec<Inline>| Inline::Link {
            target: RefTarget::Internal(BlockId(0)),
            inlines: x,
        };
        let code = |s: &str| Inline::Code(s.into());

        for inls in [
            vec![code("x"), link(vec![code("y")])],
            vec![link(vec![code("x")]), code("y")],
            vec![link(vec![code("x")]), link(vec![code("y")])],
        ] {
            assert_eq!(para(inls.clone()), "`xy`");
            assert_eq!(recovered(inls.clone()), "xy");
            assert_eq!(kasane_gfm::rendered_text(&inls), "xy");
        }

        // The link's children carry the link's depth, not the run's, so a
        // fused span made of them still renders one level down from where the
        // link sat.
        assert_eq!(
            para(vec![Inline::Emph(vec![code("x"), link(vec![code("y")])])]),
            "*`xy`*"
        );
    }

    /// `run_end` swallows a trailing run of vacuous inlines past the last real
    /// match rather than leaving them for the next iteration (review finding:
    /// leaving them made the scan quadratic on a long vacuous tail). Pinned
    /// here as an output-identity claim: a code-span run followed by trailing
    /// vacuous inlines must render exactly as the code-span run alone, with
    /// nothing appended by the swallowed tail.
    #[test]
    fn a_trailing_vacuous_tail_after_a_run_renders_as_the_run_alone() {
        let baseline = para(vec![Inline::Code("x".into()), Inline::Code("y".into())]);
        let with_tail = para(vec![
            Inline::Code("x".into()),
            Inline::Code("y".into()),
            Inline::Emph(vec![]),
            Inline::Text(String::new()),
            Inline::Link {
                target: RefTarget::Internal(BlockId(0)),
                inlines: vec![],
            },
        ]);
        assert_eq!(with_tail, baseline);
        assert_eq!(with_tail, "`xy`");
    }
}
