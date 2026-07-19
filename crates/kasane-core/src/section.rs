use kasane_ir::{Block, BlockId, Document, Inline};

pub struct SectionTree {
    pub root: SectionNode,
}

pub struct SectionNode {
    pub id: Option<BlockId>,
    pub level: u8,
    pub title: Vec<Inline>,
    pub body: Vec<Block>,
    pub children: Vec<SectionNode>,
    pub pages: Option<(u32, u32)>,
}

impl SectionNode {
    fn root() -> Self {
        Self {
            id: None,
            level: 0,
            title: vec![],
            body: vec![],
            children: vec![],
            pages: None,
        }
    }
    fn from_heading(level: u8, id: BlockId, title: Vec<Inline>) -> Self {
        Self {
            id: Some(id),
            level,
            title,
            body: vec![],
            children: vec![],
            pages: None,
        }
    }
    fn merge_pages(&mut self, p: Option<(u32, u32)>) {
        if let Some((s, e)) = p {
            self.pages = Some(match self.pages {
                Some((cs, ce)) => (cs.min(s), ce.max(e)),
                None => (s, e),
            });
        }
    }
}

pub fn fold_sections(doc: &Document) -> SectionTree {
    let mut root = SectionNode::root();
    // stack holds owned nodes being built; index 0 is always the root.
    let mut stack: Vec<SectionNode> = vec![std::mem::replace(&mut root, SectionNode::root())];
    // (root moved into the stack; `root` var is now a throwaway.)

    for node in &doc.nodes {
        match &node.block {
            Block::Heading { level, id, inlines } => {
                // pop until the top has a strictly-lower level than this heading
                while stack.len() > 1 && stack.last().unwrap().level >= *level {
                    let done = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(done);
                }
                stack.push(SectionNode::from_heading(*level, *id, inlines.clone()));
            }
            other => {
                let top = stack.last_mut().unwrap();
                top.body.push(other.clone());
                top.merge_pages(node.prov.source_pages);
            }
        }
    }
    // unwind
    while stack.len() > 1 {
        let done = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(done);
    }
    SectionTree {
        root: stack.pop().unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn folds_nested_headings() {
        // H1 Intro / para / H2 Background / para / H1 Methods
        let doc = Document {
            meta: DocMeta {
                title: "B".into(),
                authors: vec![],
                language: None,
                source_format: "epub".into(),
                source_path: "b".into(),
            },
            nodes: vec![
                h(1, 0, "Intro"),
                p("a"),
                h(2, 1, "Background"),
                p("b"),
                h(1, 2, "Methods"),
            ],
        };
        let tree = fold_sections(&doc);
        assert_eq!(tree.root.children.len(), 2); // two H1s
        let intro = &tree.root.children[0];
        assert_eq!(intro.body.len(), 1); // "a"
        assert_eq!(intro.children.len(), 1); // Background
        assert_eq!(intro.children[0].body.len(), 1); // "b"
        assert_eq!(tree.root.children[1].children.len(), 0); // Methods empty
    }

    #[test]
    fn preamble_before_first_heading_stays_on_root() {
        let doc = Document {
            meta: DocMeta {
                title: "B".into(),
                authors: vec![],
                language: None,
                source_format: "epub".into(),
                source_path: "b".into(),
            },
            nodes: vec![p("preface"), h(1, 0, "One")],
        };
        let tree = fold_sections(&doc);
        assert_eq!(tree.root.body.len(), 1);
        assert_eq!(tree.root.children.len(), 1);
    }
}
