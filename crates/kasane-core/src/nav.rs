use crate::balance::balance;
use crate::paths::{assign_paths, Placed};
use crate::refs::resolve_refs;
use crate::section::fold_sections;
use crate::sitetree::{FileNode, Frontmatter, SiteTree};
use crate::slug::inline_text;
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
    // though every INLINE walk over it is now bounded.
    //
    // Both kinds of nesting are now bounded on every side. Block nesting has
    // its own pair of constants (`epub::xhtml::MAX_BLOCK_DEPTH` for fidelity,
    // `kasane_ir::MAX_BLOCK_DEPTH` for safety) and every recursive block walk
    // in this crate and the writer carries the safety bound. The drop side
    // was already safe and stays so: `teardown_document` pops blocks from an
    // explicit worklist, which is why this call is still here rather than
    // letting `doc` fall out of scope — a bounded walk protects the walk, not
    // the compiler-derived `Drop` that runs afterwards.
    //
    // `kasane_ir::teardown_document` (shared with `kasane-adapters`'s fuzz
    // seam, which has the identical hazard for the identical reason) lives
    // beside `Block`/`Inline` rather than here so the exhaustive match inside
    // it stays a single copy the compiler checks once, not two copies that can
    // silently drift apart.
    kasane_ir::teardown_document(doc);
    balance(&mut tree, opts);
    let mut result = assign_paths(tree, &root_title);
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

    /// The block-nesting analogue of
    /// `kasane_ir`'s `teardown_document_survives_deep_block_and_inline_nesting`:
    /// the drop side was already safe, the walk side was not. Depth 100_000
    /// is far past anything a real document holds -- the point is that the
    /// bound makes depth irrelevant, so an absurd value is the honest test.
    #[test]
    fn structure_survives_deep_block_nesting() {
        const DEPTH: usize = 100_000;
        let mut blocks = vec![Block::Para(vec![Inline::Text("bottom".into())])];
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
        let mut nodes = vec![Node {
            block: Block::Heading {
                level: 1,
                id: BlockId(0),
                inlines: vec![Inline::Text("T".into())],
            },
            prov: Provenance::default(),
        }];
        nodes.extend(blocks.into_iter().map(|block| Node {
            block,
            prov: Provenance::default(),
        }));
        let doc = Document {
            meta: DocMeta {
                title: "T".into(),
                authors: vec![],
                language: None,
                source_format: "test".into(),
                source_path: "t".into(),
            },
            nodes,
        };
        // Must return normally, not abort.
        let site = structure(
            doc,
            &Options {
                max_tokens: 4000,
                min_tokens: 100,
            },
        );
        assert!(!site.files.is_empty());
    }

    /// The core-side companion to `kasane-writer`'s
    /// `rendering_preserves_content_well_under_the_block_bound`:
    /// `structure_survives_deep_block_nesting` above only pins that
    /// `structure()` returns normally at an absurd depth, and `structure()`
    /// returns a non-empty `site.files` for any non-empty `Document` -- even
    /// if `clone_block`'s guard fired at depth 1 and silently truncated
    /// almost everything. That would pass `!site.files.is_empty()` without
    /// pinning where the bound actually sits. DEPTH = 10 is well under
    /// `kasane_ir::MAX_BLOCK_DEPTH` (128): the innermost payload text must
    /// reach `site.files` intact, and no truncation note may appear anywhere
    /// in the output.
    #[test]
    fn structure_preserves_content_well_under_the_block_bound() {
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
        let mut nodes = vec![Node {
            block: Block::Heading {
                level: 1,
                id: BlockId(0),
                inlines: vec![Inline::Text("T".into())],
            },
            prov: Provenance::default(),
        }];
        nodes.extend(blocks.into_iter().map(|block| Node {
            block,
            prov: Provenance::default(),
        }));
        let doc = Document {
            meta: DocMeta {
                title: "T".into(),
                authors: vec![],
                language: None,
                source_format: "test".into(),
                source_path: "t".into(),
            },
            nodes,
        };
        let site = structure(
            doc,
            &Options {
                max_tokens: 4000,
                min_tokens: 100,
            },
        );

        assert!(
            site.files
                .iter()
                .any(|f| blocks_contain_text(&f.blocks, "innermost payload")),
            "payload text must survive this far under the bound: {:?}",
            site.files.iter().map(|f| &f.blocks).collect::<Vec<_>>()
        );
        assert!(
            !site
                .files
                .iter()
                .any(|f| blocks_contain_text(&f.blocks, "nesting truncated")),
            "the guard must not fire this shallow: {:?}",
            site.files.iter().map(|f| &f.blocks).collect::<Vec<_>>()
        );
    }

    fn blocks_contain_text(blocks: &[Block], needle: &str) -> bool {
        blocks.iter().any(|b| block_contains_text(b, needle))
    }

    fn block_contains_text(b: &Block, needle: &str) -> bool {
        match b {
            Block::Heading { inlines, .. } | Block::Para(inlines) => {
                inlines.iter().any(|i| inline_contains_text(i, needle))
            }
            Block::List { items, .. } => items.iter().any(|item| blocks_contain_text(item, needle)),
            Block::Footnote { blocks, .. } => blocks_contain_text(blocks, needle),
            Block::Table(t) => t
                .header
                .iter()
                .chain(t.rows.iter().flatten())
                .any(|cell| cell.iter().any(|i| inline_contains_text(i, needle))),
            Block::Figure { caption, .. } => {
                caption.iter().any(|i| inline_contains_text(i, needle))
            }
            Block::CodeBlock { text, .. } | Block::MathBlock(text) => text.contains(needle),
            Block::Raw { note } => note.contains(needle),
        }
    }

    fn inline_contains_text(i: &Inline, needle: &str) -> bool {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => t.contains(needle),
            Inline::Emph(x) | Inline::Strong(x) => {
                x.iter().any(|i| inline_contains_text(i, needle))
            }
            Inline::Link { inlines, .. } => inlines.iter().any(|i| inline_contains_text(i, needle)),
            Inline::FootnoteRef(_) => false,
        }
    }
}
