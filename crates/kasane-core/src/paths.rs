use crate::section::{SectionNode, SectionTree};
use crate::slug::{anchor_slug, path_slug};
use kasane_ir::{Block, BlockId};
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
        anchors.insert(id, format!("{}#{}", self_path, anchor_slug(&node.title)));
    }
    // A merged subsection's heading lives in its parent's body (balance.rs
    // demotes it there), and nothing else would give it an anchor. Only
    // top-level body blocks are scanned: a heading nested inside a list item was
    // never folded into a section either, and giving it an anchor would invent
    // structure the engine does not model.
    for b in &node.body {
        if let Block::Heading { id, inlines, .. } = b {
            anchors.insert(*id, format!("{}#{}", self_path, anchor_slug(inlines)));
        }
    }
    let children = std::mem::take(&mut node.children);
    let mut placed = Vec::new();
    for (i, child) in children.into_iter().enumerate() {
        let n = i + 1;
        let child_slug = path_slug(&child.title);
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

    #[test]
    fn body_headings_get_anchors_too() {
        // A merged subsection's heading lives in its parent's body after balance.
        // It must still be reachable by a cross-reference.
        let tree = SectionTree {
            root: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body: vec![Block::Heading {
                    level: 2,
                    id: BlockId(9),
                    inlines: vec![Inline::Text("Merged Bit".into())],
                }],
                children: vec![],
                pages: None,
            },
        };
        let placed = assign_paths(tree);
        assert_eq!(placed.anchors[&BlockId(9)], "index.md#merged-bit");
    }
}
