use crate::section::{SectionNode, SectionTree};
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
            // demote heading to a bold lead-in para, then append its body
            if !child.title.is_empty() {
                node.body
                    .push(Block::Para(vec![Inline::Strong(child.title.clone())]));
            }
            node.body.extend(child.body);
        } else {
            kept.push(child);
        }
    }
    node.children = kept;

    // SPLIT: an oversized leaf (no children) gets synthetic Part sections
    if node.children.is_empty() && est_tokens_blocks(&node.body) > opts.max_tokens {
        let parts = split_blocks(std::mem::take(&mut node.body), opts.max_tokens);
        for (i, blocks) in parts.into_iter().enumerate() {
            node.children.push(SectionNode {
                id: None,
                level: node.level + 1,
                title: vec![Inline::Text(format!("Part {}", i + 1))],
                body: blocks,
                children: vec![],
                pages: node.pages,
            });
        }
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

pub(crate) fn est_tokens_blocks(blocks: &[Block]) -> usize {
    blocks.iter().map(est_tokens_block).sum()
}

fn est_tokens_block(b: &Block) -> usize {
    fn inl(is: &[Inline]) -> usize {
        is.iter()
            .map(|i| match i {
                Inline::Text(s) | Inline::Code(s) | Inline::Math(s) => s.len(),
                Inline::Emph(x) | Inline::Strong(x) => inl(x),
                Inline::Link { inlines, .. } => inl(inlines),
                Inline::FootnoteRef(_) => 4,
            })
            .sum()
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
}
