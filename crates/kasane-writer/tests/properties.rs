//! Design spec §9's property tier, over `kasane-core` + `kasane-writer`.
//!
//! Six invariants that must hold for *any* document, checked against the
//! rendered Markdown rather than against intermediate structures — because
//! §9's link invariant is about a real file and a real anchor, and only the
//! rendered text can answer that.
//!
//! A failure writes `crates/kasane-writer/tests/properties.proptest-regressions`
//! (proptest's `SourceParallel` strategy falls back to `WithSource` here, since
//! there is no `lib.rs`/`main.rs` above a `tests/` file for it to key on).
//! **Commit it.** That file is what turns a bug the search found into a
//! permanent regression test, exactly as `fuzz/artifacts/` reproducers are.

mod generator;

use generator::{Case, Expect};
use kasane_core::{est_tokens, structure, FileNode};
use kasane_gfm::{anchor_slug_of, anchors_for_headings};
use kasane_ir::{AssetBag, Block, BlockId, Inline, RefTarget};
use proptest::prelude::*;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::collections::{HashMap, HashSet};

/// Runs the pipeline and returns each file with the text a real conversion
/// would write.
fn render(case: &Case) -> Vec<(String, String, FileNode)> {
    let site = structure(case.doc.clone(), &case.opts);
    site.files
        .into_iter()
        .map(|f| {
            let text = kasane_writer::file_to_markdown(&f, &case.assets);
            (f.path.clone(), text, f)
        })
        .collect()
}

/// Resolves a link relative to the file containing it, into a tree path.
/// Mirrors `refs::relativize` in reverse. Returns `None` if it escapes the root.
fn resolve_relative(from_file: &str, rel: &str) -> Option<String> {
    let rel = rel.split('#').next().unwrap_or(rel);
    let mut parts: Vec<&str> = from_file.split('/').collect();
    parts.pop(); // drop the filename
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// What a real GFM parser recovers from a rendered file.
struct Parsed {
    /// Concatenated text, code, math and stripped inline HTML, in document
    /// order.
    text: String,
    /// Each heading's text, in render order.
    headings: Vec<String>,
    /// Each heading's level, in the same order.
    heading_levels: Vec<usize>,
    /// Every link destination.
    links: Vec<String>,
    footnote_defs: usize,
    table_rows: usize,
    /// Every table cell, header cells included.
    table_cells: usize,
}

/// Parse with exactly the GFM extensions kasane emits, plus math.
///
/// Math is **on**, unlike an earlier draft of this helper: GitHub renders
/// `$…$`/`$$…$$` as math rather than re-parsing the interior as Markdown, so
/// leaving the extension off tested something GitHub does not do. With it
/// off, `Block::MathBlock`'s content — deliberately unescaped, real LaTeX —
/// was read back through the *inline* Markdown grammar instead of treated as
/// opaque, so a hostile character legitimately present in math (`*`, `_`,
/// `` ` ``, …) came back as real emphasis/code/etc. instead of the literal
/// text kasane wrote and GitHub would actually show.
///
/// The reason cited for keeping math off was that an escaped `\$` in prose
/// must still arrive as literal text, matching a bare, unescaped `$…$`
/// nowhere. That still holds with the extension on: `pulldown-cmark`'s
/// backslash-escape handling runs before its math-delimiter scan, so `\$`
/// never opens a math span, on either side of it, with or without a real
/// math span next to it (checked directly against the parser: `"costs
/// 5\\$"`, `` "\\$math\\$ in prose" `` and `"$x^2$ costs 5\\$"` all decode the
/// escaped `$` to plain `Text`, never `InlineMath`/`DisplayMath`). Prose and
/// math therefore still agree without a special case — the brief's original
/// concern, just satisfied by a fact about the parser rather than by leaving
/// a whole construct untested.
fn parse_events(md: &str) -> Parsed {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_MATH);

    let mut p = Parsed {
        text: String::new(),
        headings: Vec::new(),
        links: Vec::new(),
        heading_levels: Vec::new(),
        footnote_defs: 0,
        table_rows: 0,
        table_cells: 0,
    };
    let mut heading_depth = 0usize;
    let mut heading = String::new();

    for ev in Parser::new_ext(md, opts) {
        match ev {
            Event::Text(t) | Event::Code(t) => {
                p.text.push_str(&t);
                if heading_depth > 0 {
                    heading.push_str(&t);
                }
            }
            // An HTML block or inline tag: keep its text, drop its markup. The
            // merged-table path and `<br>` in cells both arrive this way.
            //
            // Math is padded the same way and for the same reason: `$$…$$`
            // wraps its content in the writer's own newlines, but `$…$`
            // (never generated today, since the property tier only draws
            // hostile text into `Block::MathBlock`) would not, and could
            // otherwise fuse onto neighboring text with no boundary at all.
            Event::Html(h) | Event::InlineHtml(h) => {
                let stripped = strip_tags(&h);
                p.text.push(' ');
                p.text.push_str(&stripped);
                p.text.push(' ');
                if heading_depth > 0 {
                    heading.push_str(&stripped);
                }
            }
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                p.text.push(' ');
                p.text.push_str(&t);
                p.text.push(' ');
                if heading_depth > 0 {
                    heading.push_str(&t);
                }
            }
            Event::SoftBreak | Event::HardBreak => p.text.push(' '),
            Event::Start(Tag::Heading { level, .. }) => {
                heading_depth += 1;
                heading.clear();
                p.heading_levels.push(level as usize);
            }
            Event::End(TagEnd::Heading(_)) => {
                heading_depth = heading_depth.saturating_sub(1);
                p.headings.push(heading.trim().to_string());
            }
            Event::Start(Tag::Link { dest_url, .. }) => p.links.push(dest_url.to_string()),
            Event::Start(Tag::FootnoteDefinition(_)) => p.footnote_defs += 1,
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => p.table_rows += 1,
            Event::Start(Tag::TableCell) => p.table_cells += 1,
            Event::End(TagEnd::Paragraph) => p.text.push(' '),
            _ => {}
        }
    }
    p
}

/// Drop HTML tags and decode the four entities `escape::text(_, Ctx::Html, _)`
/// produces, so an HTML block's text can be compared with the IR's.
///
/// An `escape::comment_note` comment (`Block::Raw`) is handled first and
/// separately from ordinary tags. It arrives as a single `Html` event with
/// exactly one `<` (in `<!--`) and one `>` (in `-->`) bracketing the whole
/// note, so the generic tag-stripping loop below would treat everything
/// between them as "inside a tag" and discard the note entirely — the
/// opposite of what it does for a real tag pair like `<td>text</td>`, where
/// the content sits *between* two separate `<...>` groups. `comment_note`
/// also never HTML-escapes (a `<` or `&` inside a comment does not open
/// anything), so the interior is taken verbatim, with no entity decoding.
fn strip_tags(html: &str) -> String {
    let trimmed = html.trim();
    if let Some(inner) = trimmed
        .strip_prefix("<!--")
        .and_then(|s| s.strip_suffix("-->"))
    {
        return inner.to_string();
    }
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

/// Collapse whitespace runs and trim, on both sides of every comparison.
///
/// Markdown normalizes whitespace at line boundaries — a soft break is a
/// newline in the source and a space in the render — so an exact comparison
/// would fail on formatting rather than on escaping. The cost is that a lost
/// *space* is invisible to P7; every other character is not.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every `Block::Heading`'s rendered level, in render order.
///
/// `render_block` clamps to 6, so this must too, or a generated `level: 7`
/// heading reads as a false failure. The walk order is `render_block`'s:
/// a block, then anything nested inside it.
fn collect_heading_levels(blocks: &[Block], out: &mut Vec<usize>) {
    for b in blocks {
        match b {
            Block::Heading { level, .. } => out.push((*level).min(6) as usize),
            Block::List { items, .. } => {
                for item in items {
                    collect_heading_levels(item, out);
                }
            }
            Block::Footnote { blocks, .. } => collect_heading_levels(blocks, out),
            _ => {}
        }
    }
}

/// Count blocks matching `pred`, recursing into lists and footnotes.
fn count_blocks(blocks: &[Block], pred: fn(&Block) -> bool) -> usize {
    sum_blocks(blocks, move |b| usize::from(pred(b)))
}

fn sum_blocks<F: Fn(&Block) -> usize + Copy>(blocks: &[Block], f: F) -> usize {
    let mut total = 0;
    for b in blocks {
        total += f(b);
        match b {
            Block::List { items, .. } => {
                for item in items {
                    total += sum_blocks(item, f);
                }
            }
            Block::Footnote { blocks, .. } => total += sum_blocks(blocks, f),
            _ => {}
        }
    }
    total
}

/// Every link destination in a rendered file, as a real parser sees them.
///
/// Superseded a hand-rolled `](`-scanner whose own doc comment named its
/// limit: it collected a false link from a fenced code block, and a false
/// positive here is one more target P2 demands resolve — a false failure, not
/// a lenient check. The generator now draws bracket-bearing text into code
/// blocks, so that limit is reached on the first hostile draw.
fn links_in(text: &str) -> Vec<String> {
    parse_events(text).links
}

/// Every heading's anchor, as the engine would compute it for this file.
///
/// Also superseded a line scanner. It stripped list markers by hand to see a
/// heading leading a list item, and stripped `*` and `` ` `` to undo the
/// writer's own markup — both of which a parser does correctly and neither of
/// which survives contact with hostile text, since a paragraph beginning with
/// `#` looked like a heading to it and consumed a duplicate-suffix slot.
///
/// Order still matters: duplicate anchors are suffixed per file in render
/// order, so the whole ordered list goes to the engine's own counter rather
/// than each line being slugged independently.
fn heading_anchors(text: &str) -> HashSet<String> {
    anchors_for_headings(&parse_events(text).headings)
        .into_iter()
        .collect()
}

proptest! {
    /// P1 — Conservation. No block lost, none duplicated.
    #[test]
    fn p1_conservation(case in generator::case()) {
        let files = render(&case);
        let all: String = files.iter().map(|(_, t, _)| t.as_str()).collect();
        for s in &case.sentinels {
            let n = all.matches(&s.token).count();
            match s.expect {
                Expect::Exactly(k) => prop_assert_eq!(
                    n, k, "sentinel {} appeared {} times, expected exactly {}", s.token, n, k
                ),
                Expect::AtLeast(k) => prop_assert!(
                    n >= k, "sentinel {} appeared {} times, expected at least {}", s.token, n, k
                ),
            }
        }
    }

    /// P2 — Link resolution, end to end.
    #[test]
    fn p2_links_resolve(case in generator::case_with_links()) {
        let files = render(&case);
        let by_path: HashMap<&str, &str> =
            files.iter().map(|(p, t, _)| (p.as_str(), t.as_str())).collect();

        // No symbolic ref survives into the emitted tree.
        for (_, _, f) in &files {
            for b in &f.blocks {
                prop_assert!(
                    !contains_internal_ref(b),
                    "an unresolved RefTarget::Internal reached the writer"
                );
            }
        }

        for (path, text, _) in &files {
            for target in links_in(text) {
                if target.starts_with("http") || target.starts_with("_assets/") {
                    continue;
                }
                let resolved = resolve_relative(path, &target);
                let resolved = match resolved {
                    Some(r) => r,
                    None => return Err(TestCaseError::fail(
                        format!("link {} from {} escapes the tree root", target, path)
                    )),
                };
                let body = by_path.get(resolved.as_str());
                prop_assert!(
                    body.is_some(),
                    "link {} from {} resolves to {}, which is not a file in the tree",
                    target, path, resolved
                );
                if let Some((_, anchor)) = target.split_once('#') {
                    prop_assert!(
                        heading_anchors(body.unwrap()).contains(anchor),
                        "anchor #{} from {} is not a heading in {}", anchor, path, resolved
                    );
                }
            }
        }
    }

    /// P3 — Size guard.
    #[test]
    fn p3_size_guard(case in generator::case()) {
        let files = render(&case);
        for (path, _, f) in &files {
            let weight = est_tokens(&f.blocks);
            let single_oversized_block = f.blocks.len() == 1
                && est_tokens(&f.blocks[..1]) > case.opts.max_tokens;
            // A container's TOC is inserted by nav *after* balancing sized the
            // node, so it can push a file over. Bounded by the TOC's own
            // weight, which is the first block when children exist.
            let toc_weight = if f.frontmatter.children.is_empty() {
                0
            } else {
                est_tokens(&f.blocks[..1])
            };
            prop_assert!(
                weight <= case.opts.max_tokens + toc_weight || single_oversized_block,
                "{} weighs {} against max_tokens {} (toc {})",
                path, weight, case.opts.max_tokens, toc_weight
            );
        }
    }

    /// P4 — Navigation chain.
    #[test]
    fn p4_nav_chain(case in generator::case()) {
        let files = render(&case);
        let by_path: HashMap<&str, &FileNode> =
            files.iter().map(|(p, _, f)| (p.as_str(), f)).collect();

        let mut cur = "index.md".to_string();
        let mut visited = Vec::new();
        loop {
            prop_assert!(!visited.contains(&cur), "next chain cycles at {}", cur);
            visited.push(cur.clone());
            let f = match by_path.get(cur.as_str()) {
                Some(f) => *f,
                None => return Err(TestCaseError::fail(format!("next led to missing {}", cur))),
            };
            match &f.frontmatter.next {
                None => break,
                Some(rel) => {
                    let nxt = resolve_relative(&cur, rel)
                        .ok_or_else(|| TestCaseError::fail("next escapes root".to_string()))?;
                    // prev of the next file must point back here.
                    let nf = by_path.get(nxt.as_str())
                        .ok_or_else(|| TestCaseError::fail(format!("missing {}", nxt)))?;
                    let back = nf.frontmatter.prev.as_ref()
                        .and_then(|p| resolve_relative(&nxt, p));
                    prop_assert_eq!(
                        back.as_deref(), Some(cur.as_str()),
                        "prev of {} does not point back to {}", nxt, cur
                    );
                    cur = nxt;
                }
            }
        }
        prop_assert_eq!(
            visited.len(), files.len(),
            "next chain visited {} of {} files", visited.len(), files.len()
        );
    }

    /// P5 — Path well-formedness.
    #[test]
    fn p5_paths_well_formed(case in generator::case()) {
        let files = render(&case);
        let paths: HashSet<&str> = files.iter().map(|(p, _, _)| p.as_str()).collect();
        prop_assert_eq!(paths.len(), files.len(), "duplicate file paths");

        for (path, _, f) in &files {
            prop_assert!(!path.starts_with('/'), "{} is absolute", path);
            for seg in path.split('/') {
                prop_assert!(seg != "..", "{} contains a traversal segment", path);
                prop_assert!(!seg.is_empty(), "{} contains an empty segment", path);
            }
            for child in &f.frontmatter.children {
                prop_assert!(
                    paths.contains(child.as_str()),
                    "{} lists child {} which is not a file", path, child
                );
            }
            if let Some(parent) = &f.frontmatter.parent {
                let resolved = resolve_relative(path, parent)
                    .ok_or_else(|| TestCaseError::fail("parent escapes root".to_string()))?;
                prop_assert!(
                    paths.contains(resolved.as_str()),
                    "{}'s parent {} resolves to {}, not a file", path, parent, resolved
                );
            }
        }
    }

    /// P6 — Determinism.
    #[test]
    fn p6_deterministic(case in generator::case()) {
        let a: Vec<_> = render(&case).into_iter().map(|(p, t, _)| (p, t)).collect();
        let b: Vec<_> = render(&case).into_iter().map(|(p, t, _)| (p, t)).collect();
        prop_assert_eq!(a, b, "structure + render is not deterministic");
    }

    /// P7 — Round trip. Every generated payload survives escaping, verbatim,
    /// into the text a real GFM parser recovers from the rendered file.
    ///
    /// This is the check a case table cannot make. The table pins kasane's
    /// reading of CommonMark; this pins the reading against an implementation
    /// of it (design spec §6.2). A missed escape shows up here as a payload
    /// that came back changed, or did not come back at all.
    ///
    /// `Sentinel::is_comment` payloads are skipped: `Block::Raw`'s note is
    /// design spec §5's one documented exception to this property's own
    /// premise, because an HTML comment has no escape mechanism for a `-->`
    /// run, so `escape::comment_note` transforms rather than escapes a note
    /// containing one, and that payload legitimately does not survive
    /// verbatim. The flag is scoped to exactly the payloads
    /// `comment_note` alters (a `--` run or a trailing `-`), not every
    /// `Block::Raw` draw, so every other note is still checked here like
    /// anything else. Proving a note cannot break *out* of its comment is
    /// the fuzz seam's job (design spec §6.5), not this property's.
    #[test]
    fn p7_round_trip(case in generator::case()) {
        let files = render(&case);
        let recovered: String = files
            .iter()
            .map(|(_, t, _)| normalize_ws(&parse_events(t).text))
            .collect::<Vec<_>>()
            .join(" ");
        for s in case.sentinels.iter().filter(|s| !s.is_comment) {
            let needle = normalize_ws(&s.payload);
            let n = recovered.matches(&needle).count();
            match s.expect {
                Expect::Exactly(k) => prop_assert_eq!(
                    n, k,
                    "payload {:?} survived {} times in parsed text, expected exactly {}",
                    s.payload, n, k
                ),
                Expect::AtLeast(k) => prop_assert!(
                    n >= k,
                    "payload {:?} survived {} times in parsed text, expected at least {}",
                    s.payload, n, k
                ),
            }
        }
    }

    /// P8 — Structure. The parsed block structure matches the IR that produced
    /// it: one footnote definition per `Block::Footnote`, one heading per
    /// heading (plus the file's own title) at the level the IR asked for and in
    /// render order, and a full grid — every row *and* every cell — per GFM
    /// table.
    ///
    /// The cell count cannot fail on its own and is kept only as documentation
    /// of the intended grid. `pulldown-cmark` pads a short row and drops a long
    /// one against the header's column count (`firstpass.rs`), so a recognized
    /// table always reports exactly `header.len() * (1 + rows.len())` — which
    /// is `want_cells` by construction — and an unrecognized one reports 0 for
    /// rows and cells alike, where the row assertion fires first.
    ///
    /// The **row** count is what actually catches an unescaped `|`, and it
    /// catches it through the header: an extra pipe there changes the header's
    /// column count, the delimiter row no longer matches it, and GFM stops
    /// recognizing the table at all. That is how the committed regression
    /// `cc 9ffd40…` presented (its `|pipe|` is in a header cell), and how
    /// reverting `math_span`'s `Ctx::Cell` rule presents today: "0 table rows
    /// parsed, 2 expected".
    ///
    /// List nesting depth is deliberately not checked here. Unlike a row or a
    /// heading it has no event that says "this is the same list the IR built" —
    /// `balance` may have moved a list into another file, and a nested list
    /// that lost its indent shows up as a *sibling* list, so the check would
    /// have to reconstruct the tree. `Expect::Exactly` in P1 plus the
    /// continuation-indent unit tests in `markdown.rs` cover the failure that
    /// motivated it.
    #[test]
    fn p8_structure_survives(case in generator::case()) {
        for (path, text, f) in render(&case) {
            let parsed = parse_events(&text);
            let want_notes = count_blocks(&f.blocks, |b| matches!(b, Block::Footnote { .. }));
            prop_assert_eq!(
                parsed.footnote_defs, want_notes,
                "{}: {} footnote definitions parsed, {} in the IR",
                path, parsed.footnote_defs, want_notes
            );
            // The file's own title heading comes first, at its breadcrumb
            // depth; every `Block::Heading` follows in render order, at the
            // level the renderer clamps it to.
            let mut want_levels = vec![f.frontmatter.breadcrumb.len().clamp(1, 6)];
            collect_heading_levels(&f.blocks, &mut want_levels);
            prop_assert_eq!(
                &parsed.heading_levels, &want_levels,
                "{}: heading levels {:?} parsed, {:?} expected",
                path, parsed.heading_levels, want_levels
            );
            let want_rows: usize = sum_blocks(&f.blocks, |b| match b {
                Block::Table(t) if !t.has_merged => 1 + t.rows.len(),
                _ => 0,
            });
            prop_assert_eq!(
                parsed.table_rows, want_rows,
                "{}: {} table rows parsed, {} expected",
                path, parsed.table_rows, want_rows
            );
            let want_cells: usize = sum_blocks(&f.blocks, |b| match b {
                Block::Table(t) if !t.has_merged => t.header.len() * (1 + t.rows.len()),
                _ => 0,
            });
            prop_assert_eq!(
                parsed.table_cells, want_cells,
                "{}: {} table cells parsed, {} expected",
                path, parsed.table_cells, want_cells
            );
        }
    }

    /// P9 — the anchor/render agreement, on the shape the main tier cannot
    /// reach (residuals spec §5.2).
    ///
    /// A newline run split across an inline boundary must produce exactly one
    /// separator in the rendered heading line, so the id GitHub computes from
    /// that line equals the anchor the engine embedded. The main tier can draw
    /// this shape but lands it about one run in seven (§5.3), which is why it
    /// gets a generator narrow enough to hit it every time.
    #[test]
    fn p9_boundary_newline_runs_anchor_the_same(
        nl1 in prop_oneof![Just("\n"), Just("\r"), Just("\r\n")],
        nl2 in prop_oneof![Just("\n"), Just("\r"), Just("\r\n")],
        kind in 0usize..5,
    ) {
        let head = format!("{nl2}b");
        let second = match kind {
            0 => Inline::Code(head),
            1 => Inline::Math(head),
            2 => Inline::Emph(vec![Inline::Text(head)]),
            3 => Inline::Strong(vec![Inline::Text(head)]),
            _ => Inline::Link {
                target: RefTarget::External("http://example.invalid/".into()),
                inlines: vec![Inline::Text(head)],
            },
        };
        let inlines = vec![Inline::Text(format!("a{nl1}")), second];

        let blocks = vec![Block::Heading {
            level: 2,
            id: BlockId(0),
            inlines: inlines.clone(),
        }];
        let md = kasane_writer::blocks_to_markdown(&blocks, &AssetBag::default());

        // The id a renderer computes from the line the writer emitted.
        let rendered = anchors_for_headings(&parse_events(&md).headings);
        // The anchor the engine embeds in every cross-reference to it.
        let embedded = anchor_slug_of(&inlines);

        prop_assert_eq!(
            rendered.first().map(String::as_str),
            Some(embedded.as_str()),
            "anchor/render divergence for kind {} nl1 {:?} nl2 {:?}:\n{}",
            kind, nl1, nl2, md
        );
    }
}

/// The merged-subsection anchor, pinned end to end and deterministically.
///
/// `balance_node` demotes a tiny childless subsection's heading into its
/// parent's body as a real `Block::Heading` carrying the original `BlockId`;
/// `assign_paths`' body scan is what gives that heading an anchor, so a
/// cross-reference into it resolves instead of degrading to plain text. Those
/// two halves and `file_to_markdown`'s title heading are the pair the design
/// spec calls "only worth making together" — a link needs both a real file and
/// a real anchor.
///
/// P2 covers this shape now that `case_with_links` draws its target from a
/// random heading, but only statistically. This test fixes the exact shape so
/// the coverage cannot silently lapse, and it renders (rather than inspecting
/// `assign_paths` in isolation, which `paths.rs`'s own unit test already does)
/// because the claim is about text a reader's Markdown viewer will follow.
#[test]
fn merged_subsection_anchor_renders_as_a_heading() {
    use kasane_ir::{AssetBag, BlockId, DocMeta, Document, Inline, Node, Provenance, RefTarget};

    fn node(block: Block) -> Node {
        Node {
            block,
            prov: Provenance::default(),
        }
    }
    fn heading(level: u8, id: u32, title: &str) -> Node {
        node(Block::Heading {
            level,
            id: BlockId(id),
            inlines: vec![Inline::Text(title.into())],
        })
    }
    fn para(text: &str) -> Node {
        node(Block::Para(vec![Inline::Text(text.into())]))
    }

    let doc = Document {
        meta: DocMeta {
            title: "Pinned Book".into(),
            authors: vec![],
            language: None,
            source_format: "epub".into(),
            source_path: "pinned.epub".into(),
        },
        nodes: vec![
            heading(1, 0, "Parent Chapter"),
            para("some parent body text that keeps this section from being tiny itself"),
            // Childless and far under `min_tokens`: exactly what the merge
            // branch absorbs into the parent above.
            heading(2, 1, "Tiny Child"),
            para("tiny"),
            // A separate top-level section, so the emitted reference is a real
            // cross-file `path#slug` rather than a bare fragment.
            heading(1, 2, "Other Chapter"),
            node(Block::Para(vec![Inline::Link {
                target: RefTarget::Internal(BlockId(1)),
                inlines: vec![Inline::Text("the tiny bit".into())],
            }])),
        ],
    };

    let opts = kasane_core::Options {
        max_tokens: 2000,
        min_tokens: 100,
    };
    let files: HashMap<String, String> = structure(doc, &opts)
        .files
        .into_iter()
        .map(|f| {
            let text = kasane_writer::file_to_markdown(&f, &AssetBag::default());
            (f.path.clone(), text)
        })
        .collect();

    let other = files
        .get("02-other-chapter.md")
        .expect("the linking section is its own file");
    let targets = links_in(other);
    assert_eq!(
        targets,
        vec!["01-parent-chapter.md#tiny-child".to_string()],
        "the merged subsection must be referenced by file and anchor, not stripped to plain text"
    );

    let (path, anchor) = targets[0].split_once('#').unwrap();
    let parent = files.get(path).expect("the anchor's file must exist");
    assert!(
        parent.contains("## Tiny Child"),
        "the demoted heading must be rendered in {path}, got:\n{parent}"
    );
    assert!(
        heading_anchors(parent).contains(anchor),
        "anchor #{anchor} must be a rendered heading in {path}, got:\n{parent}"
    );
    // It is the *demoted* heading, not the file's own title heading, that
    // carries this anchor — the two must not be the same slug, or the test
    // would pass without the merge path working at all.
    assert_ne!(anchor, "parent-chapter");
}

/// `heading_anchors` must see a heading the engine counts.
///
/// `paths::count_headings` walks into list items and feeds every heading it
/// finds to the anchor counter, because GFM assigns those headings ids and
/// they therefore consume duplicate-suffix slots. The helper above has to
/// agree, or its order diverges from the engine's and P2 fails on a tree that
/// is correct.
///
/// This renders through `blocks_to_markdown` rather than asserting the marker
/// spelling from memory: `render_block` puts a list item's first block on the
/// marker's own line, and it is that real output — not this test's idea of it
/// — that `heading_anchors` (via `parse_events`, a real GFM parser) has to
/// read as a heading. Both marker shapes are covered, since `Shape::List`
/// generates `ordered` either way.
#[test]
fn heading_anchors_sees_a_heading_that_leads_a_list_item() {
    use kasane_ir::{AssetBag, BlockId, Inline};

    for ordered in [false, true] {
        let blocks = vec![
            Block::Heading {
                level: 2,
                id: BlockId(0),
                inlines: vec![Inline::Text("Notes".into())],
            },
            Block::List {
                ordered,
                items: vec![vec![Block::Heading {
                    level: 3,
                    id: BlockId(1),
                    inlines: vec![Inline::Text("Nested".into())],
                }]],
            },
        ];
        let text = kasane_writer::blocks_to_markdown(&blocks, &AssetBag::default());

        // The shape the helper has to cope with, straight from the writer.
        let marker = if ordered { "1. " } else { "- " };
        assert!(
            text.contains(&format!("{marker}### Nested")),
            "render_block's list-item shape changed; heading_anchors' parser \
             must still read a heading off the marker line. Got:\n{text}"
        );

        let anchors = heading_anchors(&text);
        assert!(
            anchors.contains("nested"),
            "a heading leading a list item must be visible here, got {anchors:?}"
        );
        assert!(anchors.contains("notes"), "got {anchors:?}");
    }

    // The ordering half, pinned directly: a nested heading between two
    // same-titled top-level ones takes the middle slot, so the last one is
    // `notes-2`. Missing the nested line would compute `notes-1` here against
    // the engine's `notes-2` -- a false P2 failure.
    let blocks = vec![
        Block::Heading {
            level: 2,
            id: BlockId(0),
            inlines: vec![Inline::Text("Notes".into())],
        },
        Block::List {
            ordered: false,
            items: vec![vec![Block::Heading {
                level: 3,
                id: BlockId(1),
                inlines: vec![Inline::Text("Notes".into())],
            }]],
        },
        Block::Heading {
            level: 2,
            id: BlockId(2),
            inlines: vec![Inline::Text("Notes".into())],
        },
    ];
    let text = kasane_writer::blocks_to_markdown(&blocks, &AssetBag::default());
    let anchors = heading_anchors(&text);
    assert!(
        anchors.contains("notes-2"),
        "the nested heading must consume the middle slot, got {anchors:?}"
    );
}

/// Whether any inline anywhere in this block is still a symbolic internal ref.
fn contains_internal_ref(b: &Block) -> bool {
    use kasane_ir::{Inline, RefTarget};

    fn in_inlines(is: &[Inline]) -> bool {
        is.iter().any(|i| match i {
            Inline::Link {
                target: RefTarget::Internal(_),
                ..
            } => true,
            Inline::Link { inlines, .. } | Inline::Emph(inlines) | Inline::Strong(inlines) => {
                in_inlines(inlines)
            }
            _ => false,
        })
    }

    match b {
        Block::Heading { inlines, .. } | Block::Para(inlines) => in_inlines(inlines),
        Block::Figure { caption, .. } => in_inlines(caption),
        Block::List { items, .. } => items.iter().flatten().any(contains_internal_ref),
        Block::Footnote { blocks, .. } => blocks.iter().any(contains_internal_ref),
        Block::Table(t) => {
            t.header.iter().any(|c| in_inlines(c)) || t.rows.iter().flatten().any(|c| in_inlines(c))
        }
        _ => false,
    }
}
