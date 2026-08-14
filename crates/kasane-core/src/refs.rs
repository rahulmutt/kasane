use crate::paths::Placed;
use kasane_ir::{Block, BlockId, Inline, RefTarget};
use std::collections::HashMap;

pub fn resolve_refs(placed: &mut Placed, anchors: &HashMap<BlockId, String>) {
    let from = placed.path.clone();
    fix_inlines(&mut placed.node.title, &from, anchors);
    for b in &mut placed.node.body {
        fix_block(b, &from, anchors, 0);
    }
    for child in &mut placed.children {
        resolve_refs(child, anchors);
    }
}

fn fix_block(b: &mut Block, from: &str, anchors: &HashMap<BlockId, String>, depth: usize) {
    // Defence in depth: section::clone_block already truncated anything from
    // adapter or caller IR into a shallow Block::Raw, so this guard is not a
    // second truncation stacked on that one -- it covers hand-built `Placed`
    // input that reaches `resolve_refs` without ever going through
    // `fold_sections`'s bounded clone.
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        return;
    }
    match b {
        Block::Para(inls) | Block::Heading { inlines: inls, .. } => {
            fix_inlines(inls, from, anchors)
        }
        Block::List { items, .. } => {
            for it in items {
                for bb in it {
                    fix_block(bb, from, anchors, depth + 1);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for bb in blocks {
                fix_block(bb, from, anchors, depth + 1);
            }
        }
        Block::Figure { caption, .. } => fix_inlines(caption, from, anchors),
        Block::Table(t) => {
            for c in &mut t.header {
                fix_inlines(c, from, anchors);
            }
            for r in &mut t.rows {
                for c in r {
                    fix_inlines(c, from, anchors);
                }
            }
        }
        _ => {}
    }
}

fn fix_inlines(inls: &mut Vec<Inline>, from: &str, anchors: &HashMap<BlockId, String>) {
    fix_inlines_at(inls, from, anchors, 0);
}

fn fix_inlines_at(
    inls: &mut Vec<Inline>,
    from: &str,
    anchors: &HashMap<BlockId, String>,
    depth: usize,
) {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return;
    }
    let mut out = Vec::with_capacity(inls.len());
    for inl in std::mem::take(inls) {
        out.push(fix_inline(inl, from, anchors, depth));
    }
    *inls = out;
}

fn fix_inline(inl: Inline, from: &str, anchors: &HashMap<BlockId, String>, depth: usize) -> Inline {
    match inl {
        Inline::Link {
            target: RefTarget::Internal(id),
            mut inlines,
        } => {
            fix_inlines_at(&mut inlines, from, anchors, depth + 1);
            match anchors.get(&id) {
                Some(target) => Inline::Link {
                    target: RefTarget::External(relativize(from, target)),
                    inlines,
                },
                None => Inline::Emph(vec![]).replace_with_text(inlines), // strip: keep child text
            }
        }
        Inline::Link {
            target,
            mut inlines,
        } => {
            fix_inlines_at(&mut inlines, from, anchors, depth + 1);
            Inline::Link { target, inlines }
        }
        Inline::Emph(mut x) => {
            fix_inlines_at(&mut x, from, anchors, depth + 1);
            Inline::Emph(x)
        }
        Inline::Strong(mut x) => {
            fix_inlines_at(&mut x, from, anchors, depth + 1);
            Inline::Strong(x)
        }
        other => other,
    }
}

// Helper: flatten stripped link children into a single Text run.
trait ReplaceWithText {
    fn replace_with_text(self, inlines: Vec<Inline>) -> Inline;
}
impl ReplaceWithText for Inline {
    fn replace_with_text(self, inlines: Vec<Inline>) -> Inline {
        Inline::Text(kasane_gfm::inline_text(&inlines))
    }
}

pub(crate) fn relativize(from_file: &str, to_target: &str) -> String {
    let (to_path, anchor) = match to_target.split_once('#') {
        Some((p, a)) => (p, Some(a)),
        None => (to_target, None),
    };
    let from_dirs: Vec<&str> = from_file.split('/').collect();
    let from_dirs = &from_dirs[..from_dirs.len().saturating_sub(1)]; // drop filename
    let to_parts: Vec<&str> = to_path.split('/').collect();

    // common prefix of directories
    let mut i = 0;
    while i < from_dirs.len() && i + 1 < to_parts.len() && from_dirs[i] == to_parts[i] {
        i += 1;
    }
    let ups = from_dirs.len() - i;
    let mut rel = String::new();
    for _ in 0..ups {
        rel.push_str("../");
    }
    rel.push_str(&to_parts[i..].join("/"));
    match anchor {
        Some(a) => format!("{}#{}", rel, a),
        None => rel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relativizes_sibling_and_nested() {
        assert_eq!(
            relativize("02-methods.md", "index.md#intro"),
            "index.md#intro"
        );
        assert_eq!(
            relativize("01-intro/index.md", "02-methods.md#m"),
            "../02-methods.md#m"
        );
        assert_eq!(
            relativize("01-intro/01-a.md", "01-intro/02-b.md#x"),
            "02-b.md#x"
        );
        assert_eq!(relativize("a/b/c.md", "a/x.md#y"), "../x.md#y");
    }

    #[test]
    fn resolves_internal_link_and_strips_dangling() {
        use crate::paths::Placed;
        use crate::section::SectionNode;
        use kasane_ir::*;
        use std::collections::HashMap;

        let mut anchors = HashMap::new();
        anchors.insert(BlockId(7), "02-methods.md#methods".to_string());

        let blocks = vec![Block::Para(vec![
            Inline::Link {
                target: RefTarget::Internal(BlockId(7)),
                inlines: vec![Inline::Text("see methods".into())],
            },
            Inline::Link {
                target: RefTarget::Internal(BlockId(99)), // dangling
                inlines: vec![Inline::Text("gone".into())],
            },
        ])];
        let mut placed = Placed {
            path: "01-intro/index.md".into(),
            node: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body: blocks,
                children: vec![],
                pages: None,
            },
            children: vec![],
        };
        resolve_refs(&mut placed, &anchors);

        if let Block::Para(inls) = &placed.node.body[0] {
            match &inls[0] {
                Inline::Link {
                    target: RefTarget::External(u),
                    ..
                } => assert_eq!(u, "../02-methods.md#methods"),
                _ => panic!("first should be external link"),
            }
            assert!(
                matches!(inls[1], Inline::Text(_)),
                "dangling link stripped to text"
            );
        } else {
            panic!()
        }
    }

    #[test]
    fn resolves_link_in_section_title() {
        use crate::paths::Placed;
        use crate::section::SectionNode;
        use kasane_ir::*;
        use std::collections::HashMap;

        let mut anchors = HashMap::new();
        anchors.insert(BlockId(7), "02-methods.md#methods".to_string());

        let title = vec![
            Inline::Link {
                target: RefTarget::Internal(BlockId(7)),
                inlines: vec![Inline::Text("see".into())],
            },
            Inline::Link {
                target: RefTarget::Internal(BlockId(99)), // dangling
                inlines: vec![Inline::Text("gone".into())],
            },
        ];
        let mut placed = Placed {
            path: "01-intro/index.md".into(),
            node: SectionNode {
                id: None,
                level: 0,
                title,
                body: vec![],
                children: vec![],
                pages: None,
            },
            children: vec![],
        };
        resolve_refs(&mut placed, &anchors);

        match &placed.node.title[0] {
            Inline::Link {
                target: RefTarget::External(u),
                ..
            } => assert_eq!(u, "../02-methods.md#methods"),
            _ => panic!("first should be external link"),
        }
        assert!(
            matches!(placed.node.title[1], Inline::Text(_)),
            "dangling link in title stripped to text"
        );
    }
}
