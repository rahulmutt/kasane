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
use kasane_core::{est_tokens, slug_of, structure, FileNode};
use kasane_ir::Block;
use proptest::prelude::*;
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

/// Every `[text](target)` in a rendered file.
///
/// A `](` inside a fenced code block would be collected as a link too — and
/// unlike `heading_slugs`' analogous imprecision, this one runs the *unsafe*
/// direction: an extra "link" is one more target P2 demands resolve, so a
/// false positive here is a false test failure, not merely a permissive check.
/// It is safe today only because the generator's `Shape::Code` body is a bare
/// sentinel token with no brackets. A generator that ever puts arbitrary text
/// in a code block needs a real fence-skipping pass here first.
fn links_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == ']' && i + 1 < bytes.len() && bytes[i + 1] == '(' {
            let mut j = i + 2;
            let mut target = String::new();
            while j < bytes.len() && bytes[j] != ')' {
                target.push(bytes[j]);
                j += 1;
            }
            if j < bytes.len() {
                out.push(target);
            }
            i = j;
        }
        i += 1;
    }
    out
}

/// Every heading line's slug, as the engine would compute it.
///
/// A `#`-prefixed line inside a fenced code block would be counted too. That
/// only makes P2 more permissive, never less, so it is not worth a Markdown
/// parser here.
///
/// The emphasis markers *are* stripped first, and that one is not optional.
/// The engine anchors a heading at `slug(inlines)`, which reduces through
/// `inline_text` and therefore never sees a marker character; the rendered line
/// comes from `inlines_to_md`, which writes `*`/`**` around `Emph`/`Strong` and
/// backticks around `Code`. A demoted heading rendered as `## Chapter*One*`
/// would otherwise slug to `chapter-one` here against the engine's `chapterone`
/// — a false failure of P2, not a real one. `_` is stripped for the same reason
/// even though `inlines_to_md` never emits it, so a writer that switches
/// emphasis markers does not silently reintroduce the mismatch.
fn heading_slugs(text: &str) -> HashSet<String> {
    text.lines()
        .filter_map(|l| l.strip_prefix('#'))
        .map(|l| l.trim_start_matches('#').trim())
        .map(|t| t.replace(['*', '_', '`'], ""))
        .map(|t| slug_of(&[kasane_ir::Inline::Text(t)]))
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
                        heading_slugs(body.unwrap()).contains(anchor),
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
        heading_slugs(parent).contains(anchor),
        "anchor #{anchor} must be a rendered heading in {path}, got:\n{parent}"
    );
    // It is the *demoted* heading, not the file's own title heading, that
    // carries this anchor — the two must not be the same slug, or the test
    // would pass without the merge path working at all.
    assert_ne!(anchor, "parent-chapter");
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
