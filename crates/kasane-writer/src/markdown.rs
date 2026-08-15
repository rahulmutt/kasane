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
/// The loop walks *runs*, not items: a maximal group of neighbouring inlines
/// that print with the same delimiter renders as one span over their
/// concatenated contents, because CommonMark cannot express two such spans in
/// a row and the writer's two delimiter pairs would otherwise fuse into one
/// span in the rendered line (design spec
/// `2026-08-15-adjacent-inline-fusion-design.md` §2).
fn inlines_to_md_at(inls: &[Inline], depth: usize, ctx: Ctx, pos: Pos) -> String {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return String::new();
    }
    let mut s = String::new();
    let mut pos = pos;
    let mut i = 0;
    while i < inls.len() {
        let before = pos;
        let len_before = s.len();
        let end = run_end(inls, i, depth);
        let members = &inls[i..end];
        match escape::delim(&inls[i]) {
            Some(escape::Delim::Backtick) => {
                s.push_str(&escape::code_span(&backtick_run_content(members), ctx))
            }
            Some(escape::Delim::Emph) => s.push_str(&emphasis_run(members, depth, ctx, pos, "*")),
            Some(escape::Delim::Strong) => {
                s.push_str(&emphasis_run(members, depth, ctx, pos, "**"))
            }
            // `delim` said this inline prints no delimiter that can collide,
            // so the run is this inline alone and it renders as it always has.
            None => match &inls[i] {
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
                // unresolved -> text
                Inline::Link { inlines, .. } => {
                    s.push_str(&inlines_to_md_at(inlines, depth + 1, ctx, pos))
                }
                Inline::FootnoteRef(n) => s.push_str(&format!("[^{}]", n.0)),
                // Unreachable: `delim` returns `Some` for all three.
                Inline::Code(_) | Inline::Emph(_) | Inline::Strong(_) => {}
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
            } else if matches!(&inls[i], Inline::FootnoteRef(_)) && before == Pos::LineStart {
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
/// Everything else is non-vacuous by construction: `Code("")` prints
/// `` ` ` ``, `Math("")` prints `$$`, a `Link` prints its brackets, a
/// `FootnoteRef` prints `[^n]`.
fn renders_empty(i: &Inline, depth: usize) -> bool {
    match i {
        Inline::Text(t) => t.is_empty(),
        Inline::Emph(x) | Inline::Strong(x) => {
            depth + 1 >= kasane_ir::MAX_INLINE_DEPTH
                || x.iter().all(|c| renders_empty(c, depth + 1))
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
/// string. One *after* the last member is left for the outer loop, which
/// renders it as the no-op it is.
fn run_end(inls: &[Inline], start: usize, depth: usize) -> usize {
    let Some(d) = escape::delim(&inls[start]) else {
        return start + 1;
    };
    let mut end = start + 1;
    let mut k = start + 1;
    while k < inls.len() {
        if renders_empty(&inls[k], depth) {
            k += 1;
        } else if escape::delim(&inls[k]) == Some(d) {
            k += 1;
            end = k;
        } else {
            break;
        }
    }
    end
}

/// The content one code span carries for a whole backtick run.
///
/// A degrading `Inline::Math` contributes its raw LaTeX, which is exactly what
/// `math_span` would have handed `code_span` on its own. Anything else in the
/// slice is a vacuous inline `run_end` swallowed, and contributes nothing by
/// definition.
fn backtick_run_content(members: &[Inline]) -> String {
    let mut content = String::new();
    for m in members {
        if let Inline::Code(t) | Inline::Math(t) = m {
            content.push_str(t);
        }
    }
    content
}

/// Render a run of adjacent `Emph` (or `Strong`) inlines as one emphasized
/// span over the concatenation of their children.
///
/// `pos` is recomputed between members by the same rules the outer loop uses,
/// so a member sees where its own first character lands rather than where the
/// run opened. The first member still sees the run's opening `pos`, which is
/// what keeps a run of one byte-identical to what it printed before.
fn emphasis_run(members: &[Inline], depth: usize, ctx: Ctx, pos: Pos, markup: &str) -> String {
    let mut inner = String::new();
    let mut pos = pos;
    for m in members {
        let (Inline::Emph(x) | Inline::Strong(x)) = m else {
            continue;
        };
        let len_before = inner.len();
        inner.push_str(&inlines_to_md_at(x, depth + 1, ctx, pos));
        if inner.len() != len_before {
            pos = if inner.ends_with('\n') {
                Pos::LineStart
            } else {
                Pos::Mid
            };
        }
    }
    emphasize(&inner, markup)
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
}
