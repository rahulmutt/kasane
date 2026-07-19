use kasane_ir::{AssetBag, Block, Inline, RefTarget, Table};

pub fn blocks_to_markdown(blocks: &[Block], assets: &AssetBag) -> String {
    let mut out = String::new();
    for b in blocks {
        render_block(b, assets, &mut out);
        out.push('\n');
    }
    out
}

fn render_block(b: &Block, assets: &AssetBag, out: &mut String) {
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
                    render_block(bb, assets, &mut inner);
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
            let body = blocks_to_markdown(blocks, assets);
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
    let mut s = String::new();
    for i in inls {
        match i {
            Inline::Text(t) => s.push_str(t),
            Inline::Emph(x) => s.push_str(&format!("*{}*", inlines_to_md(x))),
            Inline::Strong(x) => s.push_str(&format!("**{}**", inlines_to_md(x))),
            Inline::Code(t) => s.push_str(&format!("`{}`", t)),
            Inline::Math(t) => s.push_str(&format!("${}$", t)),
            Inline::Link {
                target: RefTarget::External(u),
                inlines,
            } => s.push_str(&format!("[{}]({})", inlines_to_md(inlines), u)),
            Inline::Link { inlines, .. } => s.push_str(&inlines_to_md(inlines)), // unresolved -> text
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
}
