use crate::section::{SectionNode, SectionTree};
use kasane_ir::{BlockId, Inline};
use std::collections::HashMap;

pub struct Placed {
    pub path: String,
    pub node: SectionNode,
    pub children: Vec<Placed>,
}

pub struct PlaceResult {
    pub root: Placed,
    pub anchors: HashMap<BlockId, String>,
}

pub fn assign_paths(tree: SectionTree) -> PlaceResult {
    let mut anchors = HashMap::new();
    let root = place(tree.root, "index.md", "", &mut anchors);
    PlaceResult { root, anchors }
}

// self_path: this node's markdown file path. dir: directory children live in.
fn place(
    mut node: SectionNode,
    self_path: &str,
    dir: &str,
    anchors: &mut HashMap<BlockId, String>,
) -> Placed {
    if let Some(id) = node.id {
        anchors.insert(id, format!("{}#{}", self_path, slug(&node.title)));
    }
    let children = std::mem::take(&mut node.children);
    let mut placed = Vec::new();
    for (i, child) in children.into_iter().enumerate() {
        let n = i + 1;
        let child_slug = slug(&child.title);
        if child.children.is_empty() {
            let p = join(dir, &format!("{:02}-{}.md", n, child_slug));
            placed.push(place(child, &p, dir, anchors));
        } else {
            let cdir = join(dir, &format!("{:02}-{}", n, child_slug));
            let p = format!("{}/index.md", cdir);
            placed.push(place(child, &p, &cdir, anchors));
        }
    }
    Placed {
        path: self_path.to_string(),
        node,
        children: placed,
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", dir, name)
    }
}

pub(crate) fn slug(inlines: &[Inline]) -> String {
    let text = inline_text(inlines);
    let mut out = String::new();
    let mut prev_dash = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "section".to_string()
    } else {
        out
    }
}

pub(crate) fn inline_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => s.push_str(t),
            Inline::Emph(x) | Inline::Strong(x) => s.push_str(&inline_text(x)),
            Inline::Link { inlines, .. } => s.push_str(&inline_text(inlines)),
            Inline::FootnoteRef(_) => {}
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::section::fold_sections;
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
    fn doc(nodes: Vec<Node>) -> Document {
        Document {
            meta: DocMeta {
                title: "B".into(),
                authors: vec![],
                language: None,
                source_format: "epub".into(),
                source_path: "b".into(),
            },
            nodes,
        }
    }

    #[test]
    fn assigns_index_and_leaf_paths() {
        // H1 Intro (has H2 child) ; H1 Methods (leaf)
        let tree = fold_sections(&doc(vec![
            h(1, 0, "Intro"),
            h(2, 1, "Background & Notes"),
            h(1, 2, "Methods"),
        ]));
        let placed = assign_paths(tree);
        assert_eq!(placed.root.path, "index.md");
        let intro = &placed.root.children[0];
        assert_eq!(intro.path, "01-intro/index.md"); // has a child -> dir
        assert_eq!(intro.children[0].path, "01-intro/01-background-notes.md");
        assert_eq!(placed.root.children[1].path, "02-methods.md"); // leaf -> file
                                                                   // anchor map points at the file+slug
        assert_eq!(placed.anchors[&BlockId(2)], "02-methods.md#methods");
    }
}
