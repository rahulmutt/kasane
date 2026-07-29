use crate::section::{clone_inlines_at, SectionNode, SectionTree};
use crate::Options;
use kasane_ir::{Block, Inline};

pub fn balance(tree: &mut SectionTree, opts: &Options) {
    balance_node(&mut tree.root, opts);
}

fn balance_node(node: &mut SectionNode, opts: &Options) {
    // depth-first so children are balanced before we consider merging them up
    for child in &mut node.children {
        balance_node(child, opts);
    }

    // MERGE: absorb tiny childless children into this node's body
    // (but preserve top-level sections: don't merge children of root)
    let mut kept = Vec::new();
    for child in std::mem::take(&mut node.children) {
        let small = node.level > 0
            && child.children.is_empty()
            && est_tokens_blocks(&child.body) < opts.min_tokens;
        if small {
            // Demote the heading into the parent's body. A real `Block::Heading`
            // carrying the original `BlockId` is what lets `assign_paths` give
            // it an anchor, so a cross-reference into a merged subsection
            // resolves instead of degrading to plain text. A synthetic split
            // part has `id: None` and nothing can link to it, so it keeps the
            // bold lead-in.
            //
            // `balance` is exported and `SectionTree`/`SectionNode` are
            // all-`pub` fields, so a caller can hand-build a tree with an
            // arbitrarily deep title and call `balance` directly without ever
            // going through `fold_sections`'s bounded clone. Clone through the
            // same bounded helper here so this site can't reintroduce the
            // unbounded-clone abort on that path.
            if !child.title.is_empty() {
                match child.id {
                    Some(id) => node.body.push(Block::Heading {
                        level: child.level,
                        id,
                        inlines: clone_inlines_at(&child.title, 0),
                    }),
                    None => node
                        .body
                        .push(Block::Para(vec![Inline::Strong(clone_inlines_at(
                            &child.title,
                            0,
                        ))])),
                }
            }
            node.body.extend(child.body);
        } else {
            kept.push(child);
        }
    }
    node.children = kept;

    // SPLIT: an oversized body gets synthetic Part sections.
    //
    // This fires for a node that already has children, not only for a leaf. A
    // node's own body is the run of blocks between its heading and its first
    // subheading — for the root, the whole preamble before the first heading
    // (`section.rs`'s `preamble_before_first_heading_stays_on_root`). Gating
    // the split on `children.is_empty()` left that body in the container's own
    // file at any size, so a book with a long preface produced an `index.md`
    // arbitrarily over `max_tokens` with nothing to bound it. The size guard
    // has to hold for every file, not just for leaves.
    //
    // The parts are *prepended* to the existing children: `nav::collect_order`
    // walks pre-order, so a container's own body still reads ahead of its real
    // subsections.
    if est_tokens_blocks(&node.body) > opts.max_tokens {
        let parts = split_blocks(std::mem::take(&mut node.body), opts.max_tokens);
        let mut sections: Vec<SectionNode> = parts
            .into_iter()
            .enumerate()
            .map(|(i, blocks)| SectionNode {
                id: None,
                level: node.level + 1,
                title: vec![Inline::Text(format!("Part {}", i + 1))],
                body: blocks,
                children: vec![],
                pages: node.pages,
            })
            .collect();
        sections.append(&mut node.children);
        node.children = sections;
    }
}

fn split_blocks(blocks: Vec<Block>, max_tokens: usize) -> Vec<Vec<Block>> {
    let mut parts = vec![];
    let mut cur = vec![];
    let mut cur_tokens = 0;
    for b in blocks {
        let t = est_tokens_blocks(std::slice::from_ref(&b));
        if cur_tokens + t > max_tokens && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur));
            cur_tokens = 0;
        }
        cur.push(b);
        cur_tokens += t;
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

/// Token estimate for a block slice.
///
/// `#[doc(hidden)]` because it is a test seam, not API — the same convention
/// `kasane-adapters` uses for `fuzz_entry`. The property suite's size-guard
/// invariant needs the engine's own estimator; re-implementing it in the test
/// would create a second source of truth that drifts silently, passing against
/// its own arithmetic while the engine's changed.
#[doc(hidden)]
pub fn est_tokens(blocks: &[Block]) -> usize {
    est_tokens_blocks(blocks)
}

pub(crate) fn est_tokens_blocks(blocks: &[Block]) -> usize {
    blocks.iter().map(est_tokens_block).sum()
}

fn est_tokens_block(b: &Block) -> usize {
    fn inl_at(is: &[Inline], depth: usize) -> usize {
        if depth >= kasane_ir::MAX_INLINE_DEPTH {
            return 0;
        }
        is.iter()
            .map(|i| match i {
                Inline::Text(s) | Inline::Code(s) | Inline::Math(s) => s.len(),
                Inline::Emph(x) | Inline::Strong(x) => inl_at(x, depth + 1),
                Inline::Link { inlines, .. } => inl_at(inlines, depth + 1),
                Inline::FootnoteRef(_) => 4,
            })
            .sum()
    }
    fn inl(is: &[Inline]) -> usize {
        inl_at(is, 0)
    }
    let chars = match b {
        Block::Heading { inlines, .. } | Block::Para(inlines) => inl(inlines),
        Block::List { items, .. } => items.iter().flatten().map(est_tokens_block).sum(),
        Block::Table(t) => t.rows.iter().flatten().map(|c| inl(c)).sum::<usize>() + 20,
        Block::Figure { caption, .. } => inl(caption) + 16,
        Block::CodeBlock { text, .. } => text.len(),
        Block::MathBlock(s) | Block::Raw { note: s } => s.len(),
        Block::Footnote { blocks, .. } => est_tokens_blocks(blocks),
    };
    chars / 4 + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::section::fold_sections;
    use kasane_ir::*;

    fn big_para(n: usize) -> Node {
        Node {
            block: Block::Para(vec![Inline::Text("x".repeat(n))]),
            prov: Provenance::default(),
        }
    }
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
    fn splits_oversized_leaf() {
        // one H1 with two ~1200-char paras => ~600 tokens, over max_tokens=400
        let mut tree = fold_sections(&doc(vec![h(1, 0, "Big"), big_para(1200), big_para(1200)]));
        balance(
            &mut tree,
            &Options {
                max_tokens: 400,
                min_tokens: 10,
            },
        );
        let sec = &tree.root.children[0];
        assert!(sec.children.len() >= 2, "expected split into parts");
        assert!(sec.body.is_empty(), "body moved into parts");
    }

    #[test]
    fn splits_an_oversized_body_that_also_has_children() {
        // A preamble before the first heading lives in the root's own body
        // while the headings become its children. Splitting only leaves left
        // that body in `index.md` at any size, breaking the size guard for the
        // one file every reader opens first.
        let mut tree = fold_sections(&doc(vec![
            big_para(1200),
            big_para(1200),
            h(1, 0, "Real Chapter"),
        ]));
        balance(
            &mut tree,
            &Options {
                max_tokens: 400,
                min_tokens: 10,
            },
        );
        assert!(tree.root.body.is_empty(), "preamble moved into parts");
        let titles: Vec<String> = tree
            .root
            .children
            .iter()
            .map(|c| crate::paths::inline_text(&c.title))
            .collect();
        // Parts precede the real chapter: pre-order flattening is reading order.
        assert_eq!(titles, vec!["Part 1", "Part 2", "Real Chapter"]);
    }

    #[test]
    fn merges_tiny_leaf_into_parent() {
        // H1 with H2 child holding one tiny para; child under min_tokens should merge up
        let mut tree = fold_sections(&doc(vec![h(1, 0, "Top"), h(2, 1, "Tiny"), big_para(4)]));
        balance(
            &mut tree,
            &Options {
                max_tokens: 2000,
                min_tokens: 100,
            },
        );
        let top = &tree.root.children[0];
        assert!(top.children.is_empty(), "tiny child folded up");
        assert!(!top.body.is_empty(), "child body absorbed into parent");
    }

    #[test]
    fn merging_a_deeply_nested_title_does_not_abort() {
        // `balance` is exported and `SectionTree`/`SectionNode` are all-`pub`,
        // so a caller can hand-build a tree and call `balance` directly
        // without ever going through `fold_sections`'s bounded clone. Before
        // the fix, the merge branch's `child.title.clone()` recursed on the
        // derived `Clone` with no bound and aborted at this depth.
        let mut deep_title = Inline::Text("x".into());
        for _ in 0..10_000 {
            deep_title = Inline::Emph(vec![deep_title]);
        }
        // Merging only fires for a node with `level > 0` (root's own direct
        // children are preserved as top-level sections), so the deep-titled
        // leaf must be a grandchild: root -> level-1 parent -> level-2 leaf.
        let mut tree = SectionTree {
            root: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body: vec![],
                children: vec![SectionNode {
                    id: None,
                    level: 1,
                    title: vec![Inline::Text("Parent".into())],
                    body: vec![],
                    children: vec![SectionNode {
                        id: None,
                        level: 2,
                        title: vec![deep_title],
                        body: vec![Block::Para(vec![Inline::Text("tiny".into())])],
                        children: vec![],
                        pages: None,
                    }],
                    pages: None,
                }],
                pages: None,
            },
        };
        balance(
            &mut tree,
            &Options {
                max_tokens: 10_000,
                min_tokens: 100,
            },
        );
        let parent = &tree.root.children[0];
        assert!(
            parent.children.is_empty(),
            "tiny deep-titled leaf folded up"
        );
        assert_eq!(parent.body.len(), 2, "demoted title + absorbed body");
    }
}
