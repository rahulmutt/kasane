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
            out.push_str(&escape::one_line(&inlines_to_md(
                inlines,
                Ctx::Flow,
                Pos::Mid,
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
            let alt = escape::one_line(&inlines_to_md(caption, Ctx::Flow, Pos::Mid));
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
            .map(|c| inlines_to_md(c, Ctx::Cell, Pos::LineStart))
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
/// then recomputed after every arm from whether the accumulated output ends
/// with `\n` -- that single rule covers both a run that opens on a fresh line
/// and one that re-arms after an interior newline (`[Text("a\n"), Text("-
/// b")]`), with no per-arm special case, because none of the writer's own
/// markup (`*`, `**`, backticks, `[`, `$`, `[^1]`) ever ends in a newline.
fn inlines_to_md_at(inls: &[Inline], depth: usize, ctx: Ctx, pos: Pos) -> String {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return String::new();
    }
    let mut s = String::new();
    let mut pos = pos;
    for i in inls {
        match i {
            // The only call to `escape::text` in the crate. Every other arm
            // below emits markup the writer chose, which must not be escaped.
            Inline::Text(t) => s.push_str(&escape::text(t, ctx, pos)),
            Inline::Emph(x) => {
                s.push_str(&emphasize(&inlines_to_md_at(x, depth + 1, ctx, pos), "*"))
            }
            Inline::Strong(x) => {
                s.push_str(&emphasize(&inlines_to_md_at(x, depth + 1, ctx, pos), "**"))
            }
            Inline::Code(t) => s.push_str(&escape::code_span(t, ctx)),
            Inline::Math(t) => s.push_str(&escape::math_span(t, ctx)),
            Inline::Link {
                target: RefTarget::External(u),
                inlines,
            } => s.push_str(&format!(
                "[{}]({})",
                escape::one_line(&inlines_to_md_at(inlines, depth + 1, ctx, pos)),
                escape::dest_url(u)
            )),
            // unresolved -> text
            Inline::Link { inlines, .. } => {
                s.push_str(&inlines_to_md_at(inlines, depth + 1, ctx, pos))
            }
            Inline::FootnoteRef(n) => s.push_str(&format!("[^{}]", n.0)),
        }
        pos = if s.ends_with('\n') {
            Pos::LineStart
        } else {
            Pos::Mid
        };
    }
    s
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
    /// in `kasane-core`'s `slug.rs` for that coverage instead).
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
            let slug = kasane_core::path_slug_of(&[Inline::Text(title.into())]);
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
            let anchor = kasane_core::anchors_for_headings(&[title.to_string()])
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

    /// Emphasis whose content begins or ends with whitespace moves that
    /// whitespace outside the delimiters.
    ///
    /// At column 0 the old output was `* x*`, which GFM reads as a bullet
    /// list rather than a paragraph; the second half of the same bug is that
    /// CommonMark's flanking rules mean `* x*` is not emphasis anywhere, so
    /// the markup was silently lost mid-line too.
    #[test]
    fn emphasis_moves_edge_whitespace_outside_its_delimiters() {
        let blocks = vec![Block::Para(vec![Inline::Emph(vec![Inline::Text(
            " x".into(),
        )])])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.starts_with(" *x*"), "a bullet list at column 0: {md}");

        // Mid-line, the emphasis must survive rather than be dropped by the
        // flanking rules.
        let blocks = vec![Block::Para(vec![
            Inline::Text("a".into()),
            Inline::Strong(vec![Inline::Text(" x ".into())]),
            Inline::Text("b".into()),
        ])];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("a **x** b"), "got: {md}");

        // Nothing to emphasize: no delimiters at all, so `**` cannot sit at
        // column 0 as markup for its own sake.
        let blocks = vec![Block::Para(vec![Inline::Strong(vec![Inline::Text(
            "  ".into(),
        )])])];
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
}
