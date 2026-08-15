use kasane_ir::{Block, BlockId, Document, Inline, RefTarget, Table};

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
                stack.push(SectionNode::from_heading(
                    *level,
                    *id,
                    clone_inlines_at(inlines, 0),
                ));
            }
            other => {
                let top = stack.last_mut().unwrap();
                top.body.push(clone_block(other, 0));
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

// `Block` and `Inline` derive `Clone`, but that derive recurses on nesting
// depth just like the hand-written walks it sits next to — an unbounded
// `Document` clones straight into a stack overflow before any of those walks
// even run. `clone_inlines_at` bounds inline nesting (see
// `kasane_ir::MAX_INLINE_DEPTH`): past the bound it stops descending and
// contributes nothing, rather than recursing into the derived `Clone`.
//
// `clone_block` below is the load-bearing truncation for BLOCK nesting. This
// is the first core walk to touch adapter or caller IR, so past this point
// every later core and writer walk sees already-shallow blocks -- their own
// guards are defence in depth, not a second truncation stacked on this one.
//
// `clone_inlines_at` carries one canonicalization as well as the bound: an
// empty `Inline::Code` becomes a single space. That is not a tidy-up -- it is
// load-bearing for anchors, and it lives here rather than in a pass of its own
// because this walk is the one place every inline is guaranteed to pass
// through exactly once. See the arm's own comment, and
// `docs/superpowers/specs/2026-08-14-empty-code-span-anchor-design.md` §2.
fn clone_block(b: &Block, depth: usize) -> Block {
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        return Block::Raw {
            note: "nesting truncated at the block depth bound".into(),
        };
    }
    match b {
        Block::Heading { level, id, inlines } => Block::Heading {
            level: *level,
            id: *id,
            inlines: clone_inlines_at(inlines, 0),
        },
        Block::Para(inlines) => Block::Para(clone_inlines_at(inlines, 0)),
        Block::List { ordered, items } => Block::List {
            ordered: *ordered,
            items: items
                .iter()
                .map(|item| item.iter().map(|b| clone_block(b, depth + 1)).collect())
                .collect(),
        },
        Block::Table(t) => Block::Table(Table {
            header: t.header.iter().map(|c| clone_inlines_at(c, 0)).collect(),
            rows: t
                .rows
                .iter()
                .map(|r| r.iter().map(|c| clone_inlines_at(c, 0)).collect())
                .collect(),
            has_merged: t.has_merged,
        }),
        Block::Figure {
            image,
            caption,
            number,
        } => Block::Figure {
            image: image.clone(),
            caption: clone_inlines_at(caption, 0),
            number: number.clone(),
        },
        Block::CodeBlock { lang, text } => Block::CodeBlock {
            lang: lang.clone(),
            text: text.clone(),
        },
        Block::MathBlock(s) => Block::MathBlock(s.clone()),
        Block::Footnote { id, blocks } => Block::Footnote {
            id: *id,
            blocks: blocks.iter().map(|b| clone_block(b, depth + 1)).collect(),
        },
        Block::Raw { note } => Block::Raw { note: note.clone() },
    }
}

pub(crate) fn clone_inlines_at(inls: &[Inline], depth: usize) -> Vec<Inline> {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return Vec::new();
    }
    inls.iter()
        .map(|i| match i {
            Inline::Text(t) => Inline::Text(t.clone()),
            // CommonMark cannot express an empty code span, so
            // `kasane-writer::escape::code_span` prints one as `` ` ` `` --
            // a padding space that is real text in the rendered line GitHub
            // computes a heading id from. Canonicalizing the empty form here,
            // at the single walk every inline passes through, is what lets
            // `rendered_text` and `title_text` see that space without either
            // importing the writer's escaping rules. `Code("")` and
            // `Code(" ")` render to the same three bytes, so no code span's
            // output moves; `escape.rs`'s
            // `code_span_pads_an_empty_span_to_exactly_what_a_single_space_renders`
            // is the test that keeps that true.
            Inline::Code(t) if t.is_empty() => Inline::Code(" ".into()),
            Inline::Code(t) => Inline::Code(t.clone()),
            Inline::Math(t) => Inline::Math(t.clone()),
            Inline::Emph(x) => Inline::Emph(clone_inlines_at(x, depth + 1)),
            Inline::Strong(x) => Inline::Strong(clone_inlines_at(x, depth + 1)),
            Inline::Link { target, inlines } => Inline::Link {
                target: clone_ref_target(target),
                inlines: clone_inlines_at(inlines, depth + 1),
            },
            Inline::FootnoteRef(n) => Inline::FootnoteRef(*n),
        })
        .collect()
}

fn clone_ref_target(t: &RefTarget) -> RefTarget {
    match t {
        RefTarget::Internal(id) => RefTarget::Internal(*id),
        RefTarget::External(s) => RefTarget::External(s.clone()),
        RefTarget::Footnote(n) => RefTarget::Footnote(*n),
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

    #[test]
    fn folding_deeply_nested_inlines_does_not_abort() {
        // `fold_sections` clones every non-heading block into the tree; the
        // derived `Clone` on `Inline` recurses on nesting depth exactly like
        // the hand-written inline walks, so an unbounded clone of a deeply
        // nested `Document` would stack-overflow before those walks ever run.
        let mut inline = Inline::Text("x".into());
        for _ in 0..10_000 {
            inline = Inline::Emph(vec![inline]);
        }
        let doc = Document {
            meta: DocMeta {
                title: "B".into(),
                authors: vec![],
                language: None,
                source_format: "epub".into(),
                source_path: "b".into(),
            },
            nodes: vec![Node {
                block: Block::Para(vec![inline]),
                prov: Provenance::default(),
            }],
        };
        let tree = fold_sections(&doc);
        assert_eq!(tree.root.body.len(), 1, "the deep para must still fold in");
    }
}
