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
                        inlines: vec![Inline::Text(child_title(c, doc_title))],
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

fn child_title(p: &Placed, doc_title: &str) -> String {
    if p.node.id.is_none() {
        doc_title.to_string()
    } else {
        inline_text(&p.node.title)
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
}
