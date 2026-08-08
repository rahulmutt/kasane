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
            out.push_str(&inlines_to_md(inlines));
            out.push('\n');
        }
        Block::Para(inls) => {
            out.push_str(&inlines_to_md(inls));
            out.push('\n');
        }
        Block::List { ordered, items } => {
            for (i, item) in items.iter().enumerate() {
                if *ordered {
                    out.push_str(&format!("{}. ", i + 1));
                } else {
                    out.push_str("- ");
                }
                // render first block inline, subsequent blocks indented
                let mut inner = String::new();
                for bb in item {
                    render_block(bb, assets, &mut inner, depth + 1);
                }
                out.push_str(inner.trim_end());
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
            out.push_str(&format!(
                "![{}](_assets/{})\n",
                inlines_to_md(caption),
                fname
            ));
            if let Some(n) = number {
                out.push_str(&format!("*Figure {}: {}*\n", n, inlines_to_md(caption)));
            }
        }
        Block::CodeBlock { lang, text } => {
            out.push_str(&format!(
                "```{}\n{}\n```\n",
                lang.clone().unwrap_or_default(),
                text
            ));
        }
        Block::MathBlock(s) => out.push_str(&format!("$$\n{}\n$$\n", s)),
        Block::Footnote { id, blocks } => {
            let body = blocks_to_markdown_at(blocks, assets, depth + 1);
            out.push_str(&format!("[^{}]: {}\n", id.0, body.trim()));
        }
        Block::Raw { note } => out.push_str(&format!("<!-- {} -->\n", note)),
    }
}

fn render_table(t: &Table, out: &mut String) {
    if t.has_merged {
        out.push_str("<table>\n");
        // header + rows as HTML (merged cells not modeled per-cell here; emit flat)
        let esc = |c: &Vec<Inline>| inlines_to_md(c);
        out.push_str("<tr>");
        for c in &t.header {
            out.push_str(&format!("<th>{}</th>", esc(c)));
        }
        out.push_str("</tr>\n");
        for r in &t.rows {
            out.push_str("<tr>");
            for c in r {
                out.push_str(&format!("<td>{}</td>", esc(c)));
            }
            out.push_str("</tr>\n");
        }
        out.push_str("</table>\n");
        return;
    }
    let cells = |row: &Vec<Vec<Inline>>| {
        let joined: Vec<String> = row.iter().map(|c| inlines_to_md(c)).collect();
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

pub(crate) fn inlines_to_md(inls: &[Inline]) -> String {
    inlines_to_md_at(inls, 0)
}

fn inlines_to_md_at(inls: &[Inline], depth: usize) -> String {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return String::new();
    }
    let mut s = String::new();
    for i in inls {
        match i {
            Inline::Text(t) => s.push_str(t),
            Inline::Emph(x) => s.push_str(&format!("*{}*", inlines_to_md_at(x, depth + 1))),
            Inline::Strong(x) => s.push_str(&format!("**{}**", inlines_to_md_at(x, depth + 1))),
            Inline::Code(t) => s.push_str(&format!("`{}`", t)),
            Inline::Math(t) => s.push_str(&format!("${}$", t)),
            Inline::Link {
                target: RefTarget::External(u),
                inlines,
            } => s.push_str(&format!(
                "[{}]({})",
                inlines_to_md_at(inlines, depth + 1),
                u
            )),
            Inline::Link { inlines, .. } => s.push_str(&inlines_to_md_at(inlines, depth + 1)), // unresolved -> text
            Inline::FootnoteRef(n) => s.push_str(&format!("[^{}]", n.0)),
        }
    }
    s
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
    /// destination -- space, `(`, `)`, `#`, `?`, `%` -- is outside `\p{Word}`
    /// and is therefore already removed by the slug rule.
    ///
    /// This asserts the set rather than the argument, so widening the rule by
    /// hand fails here instead of silently emitting a broken link.
    #[test]
    fn path_slugs_contain_nothing_that_breaks_a_bare_destination() {
        for title in [
            "Don't Panic",
            "Background & Notes (revised)",
            "50% off #1?",
            "第二章",
            "a/b\\c",
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
}
