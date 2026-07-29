use crate::balance::balance;
use crate::paths::{assign_paths, inline_text, Placed};
use crate::refs::resolve_refs;
use crate::section::fold_sections;
use crate::sitetree::{FileNode, Frontmatter, SiteTree};
use crate::Options;
use kasane_ir::{Block, Document, Inline, RefTarget};

pub fn structure(doc: Document, opts: &Options) -> SiteTree {
    let root_title = doc.meta.title.clone();
    let mut tree = fold_sections(&doc);
    // `doc` is fully cloned into `tree` above; nothing after this point reads
    // it. Tear it down explicitly instead of letting it fall out of scope:
    // `Inline` has no manual `Drop`, so the ordinary, compiler-derived one
    // recurses on block/inline nesting depth exactly like the walks and
    // clones bounded elsewhere in this module — an externally supplied
    // `Document` can be arbitrarily deep, and dropping it normally would
    // abort the process on the way out of this published entry point even
    // though every walk over it is now bounded. `kasane_ir::teardown_document`
    // (shared with `kasane-adapters`'s fuzz seam, which has the identical
    // hazard for the identical reason) lives beside `Block`/`Inline` rather
    // than here so the exhaustive match inside it stays a single copy the
    // compiler checks once, not two copies that can silently drift apart.
    kasane_ir::teardown_document(doc);
    balance(&mut tree, opts);
    let mut result = assign_paths(tree);
    resolve_refs(&mut result.root, &result.anchors);

    // Flatten in reading order (pre-order), carrying breadcrumb trail.
    let mut files = Vec::new();
    let mut order = Vec::new(); // paths in reading order for prev/next
    collect_order(&result.root, &mut order);

    walk(&result.root, &root_title, &[], None, &order, &mut files);
    // Fix root title (root node has empty heading title).
    if let Some(root_file) = files.iter_mut().find(|f| f.path == "index.md") {
        root_file.frontmatter.title = root_title.clone();
        root_file.frontmatter.breadcrumb = vec![root_title];
    }
    SiteTree { files }
}

fn collect_order(p: &Placed, out: &mut Vec<String>) {
    out.push(p.path.clone());
    for c in &p.children {
        collect_order(c, out);
    }
}

fn walk(
    p: &Placed,
    doc_title: &str,
    trail: &[String],
    parent: Option<&str>,
    order: &[String],
    files: &mut Vec<FileNode>,
) {
    let title = if p.node.id.is_none() && trail.is_empty() {
        doc_title.to_string()
    } else {
        inline_text(&p.node.title)
    };
    let mut breadcrumb = trail.to_vec();
    breadcrumb.push(title.clone());

    let idx = order.iter().position(|x| x == &p.path).unwrap();
    let prev = if idx > 0 {
        Some(order[idx - 1].clone())
    } else {
        None
    };
    let next = order.get(idx + 1).cloned();

    let child_paths: Vec<String> = p.children.iter().map(|c| c.path.clone()).collect();

    // Body: for a directory node with children, prepend an auto TOC.
    // Plain `.clone()` is safe here only because everything downstream of
    // `fold_sections`'s bounded clone (this `walk` included) never sees
    // inline nesting past `kasane_ir::MAX_INLINE_DEPTH`; don't reintroduce a
    // hand-built, unbounded `Placed` into this path without re-checking that.
    let mut blocks = p.node.body.clone();
    if !p.children.is_empty() {
        let toc = Block::List {
            ordered: false,
            items: p
                .children
                .iter()
                .map(|c| {
                    vec![Block::Para(vec![Inline::Link {
                        target: RefTarget::External(crate::refs::relativize(&p.path, &c.path)),
                        // A child is never the root, so it always has a title
                        // of its own: a real heading's inlines, or `Part N` for
                        // a synthetic split part. The `title` binding above
                        // substitutes the document title only for the root, and
                        // only because `trail.is_empty()` pins it there; the
                        // TOC used to key off `id.is_none()` alone, which is
                        // also true of every synthetic part, so a split body
                        // produced a TOC of N entries all named after the book.
                        inlines: vec![Inline::Text(inline_text(&c.node.title))],
                    }])]
                })
                .collect(),
        };
        blocks.insert(0, toc);
    }

    files.push(FileNode {
        path: p.path.clone(),
        frontmatter: Frontmatter {
            title,
            breadcrumb: breadcrumb.clone(),
            parent: parent.map(|s| relparent(&p.path, s)),
            prev: prev.map(|s| crate::refs::relativize(&p.path, &s)),
            next: next.map(|s| crate::refs::relativize(&p.path, &s)),
            children: child_paths,
            source_pages: p.node.pages,
        },
        blocks,
    });

    for c in &p.children {
        walk(c, doc_title, &breadcrumb, Some(&p.path), order, files);
    }
}

fn relparent(from: &str, parent_abs: &str) -> String {
    crate::refs::relativize(from, parent_abs)
}

#[cfg(test)]
mod tests {
    use crate::{structure, Options};
    use kasane_ir::*;

    fn h(level: u8, id: u32, t: &str) -> Node {
        Node {
            block: Block::Heading {
                level,
                id: BlockId(id),
                inlines: vec![Inline::Text(t.into())],
            },
            prov: Provenance::default(),
        }
    }
    fn p(t: &str) -> Node {
        Node {
            block: Block::Para(vec![Inline::Text(t.into())]),
            prov: Provenance::default(),
        }
    }

    #[test]
    fn builds_navigation_chain() {
        let doc = Document {
            meta: DocMeta {
                title: "My Book".into(),
                authors: vec![],
                language: None,
                source_format: "epub".into(),
                source_path: "b.epub".into(),
            },
            nodes: vec![h(1, 0, "Intro"), p("hi"), h(1, 1, "Methods"), p("mm")],
        };
        let site = structure(doc, &Options::default());
        let paths: Vec<_> = site.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"index.md"));
        assert!(paths.contains(&"01-intro.md"));
        assert!(paths.contains(&"02-methods.md"));

        let intro = site.files.iter().find(|f| f.path == "01-intro.md").unwrap();
        assert_eq!(intro.frontmatter.title, "Intro");
        assert_eq!(intro.frontmatter.parent.as_deref(), Some("index.md"));
        assert_eq!(intro.frontmatter.next.as_deref(), Some("02-methods.md"));
        assert_eq!(intro.frontmatter.breadcrumb, vec!["My Book", "Intro"]);

        let root = site.files.iter().find(|f| f.path == "index.md").unwrap();
        assert_eq!(root.frontmatter.title, "My Book");
        assert_eq!(
            root.frontmatter.children,
            vec!["01-intro.md", "02-methods.md"]
        );
    }

    #[test]
    fn toc_names_synthetic_parts_by_their_own_title() {
        // A split body becomes `Part N` children. Labelling a TOC entry with
        // the document title whenever the child has no `BlockId` made every
        // one of them read "My Book".
        let doc = Document {
            meta: DocMeta {
                title: "My Book".into(),
                authors: vec![],
                language: None,
                source_format: "epub".into(),
                source_path: "b.epub".into(),
            },
            nodes: vec![
                Node {
                    block: Block::Para(vec![Inline::Text("x".repeat(1200))]),
                    prov: Provenance::default(),
                },
                Node {
                    block: Block::Para(vec![Inline::Text("x".repeat(1200))]),
                    prov: Provenance::default(),
                },
            ],
        };
        let site = structure(
            doc,
            &Options {
                max_tokens: 200,
                min_tokens: 10,
            },
        );
        let root = site.files.iter().find(|f| f.path == "index.md").unwrap();
        let toc = match &root.blocks[0] {
            Block::List { items, .. } => items
                .iter()
                .map(|it| match &it[0] {
                    Block::Para(inls) => match &inls[0] {
                        Inline::Link { inlines, .. } => match &inlines[0] {
                            Inline::Text(t) => t.clone(),
                            _ => panic!("expected link text"),
                        },
                        _ => panic!("expected link"),
                    },
                    _ => panic!("expected para"),
                })
                .collect::<Vec<_>>(),
            _ => panic!("expected a TOC list"),
        };
        assert_eq!(toc, vec!["Part 1", "Part 2"]);
    }
}
