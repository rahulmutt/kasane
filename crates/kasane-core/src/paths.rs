use crate::section::{SectionNode, SectionTree};
use kasane_gfm::{path_slug, rendered_text, title_text, AnchorCounter};
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

/// `doc_title` is what `file_to_markdown` renders as `index.md`'s heading --
/// the root `SectionNode`'s own title is empty, and `nav::walk` substitutes
/// the document title there. The anchor counter has to see the text the file
/// actually renders or the root's duplicate suffixes are off by one.
pub fn assign_paths(tree: SectionTree, doc_title: &str) -> PlaceResult {
    let mut anchors = HashMap::new();
    let root = place(tree.root, "index.md", "", doc_title, true, &mut anchors);
    PlaceResult { root, anchors }
}

// self_path: this node's markdown file path. dir: directory children live in.
// doc_title: only meaningful when is_root; see `assign_paths`.
fn place(
    mut node: SectionNode,
    self_path: &str,
    dir: &str,
    doc_title: &str,
    is_root: bool,
    anchors: &mut HashMap<BlockId, String>,
) -> Placed {
    // One counter per file, fed in the order `file_to_markdown` renders: the
    // title heading it prepends, then the body.
    let mut counter = AnchorCounter::new();

    // Every file renders a title heading, so every file consumes this slot --
    // including `index.md`, whose heading is the document title rather than
    // the (empty) root node title. `nav::walk` pins the substitution on
    // `id.is_none() && trail.is_empty()`, which is true for the ROOT call only
    // -- every recursive `walk` call has already pushed a title onto the
    // breadcrumb. `is_root` is the same thing made explicit rather than
    // inferred: `node.id.is_none() && dir.is_empty()` looked equivalent but
    // isn't -- `dir` is empty for every top-level LEAF child of the root too
    // (a leaf inherits its parent's `dir` unchanged), and `id` is `None` for
    // every synthetic `Part N` node `balance.rs` splits off an oversized body
    // (root's included). A root-level `Part N` leaf would have matched both
    // clauses and had its counter seeded from the document title instead of
    // its own -- the same `id.is_none()`-alone conflation `nav.rs` already
    // hit once for the TOC (see its comment on the synthetic-parts fix).
    // A file's title heading prints `Frontmatter::title`, which `nav::walk`
    // builds with `title_text` — so the anchor is computed from that same
    // string, not from the inlines behind it. They differ whenever the title
    // carries a footnote reference: the printed line has no `[^1]` in it, and
    // an anchor that predicted one would point at an id no renderer assigns.
    let title_anchor = if is_root {
        counter.next(doc_title)
    } else {
        counter.next(&title_text(&node.title))
    };
    if let Some(id) = node.id {
        anchors.insert(id, format!("{}#{}", self_path, title_anchor));
    }

    // A merged subsection's heading lives in its parent's body (balance.rs
    // demotes it there), and nothing else would give it an anchor. Only
    // top-level body blocks are ANCHORED: a heading nested inside a list item
    // was never folded into a section either, and giving it an anchor would
    // invent structure the engine does not model. Every rendered heading is
    // still COUNTED, nested ones included, because GitHub assigns them ids and
    // they therefore consume duplicate-suffix slots.
    count_headings(&node.body, 0, true, self_path, &mut counter, anchors);

    let children = std::mem::take(&mut node.children);
    let mut placed = Vec::new();
    for (i, child) in children.into_iter().enumerate() {
        let n = i + 1;
        let child_slug = path_slug(&child.title);
        if child.children.is_empty() {
            let p = join(dir, &format!("{:02}-{}.md", n, child_slug));
            placed.push(place(child, &p, dir, doc_title, false, anchors));
        } else {
            let cdir = join(dir, &format!("{:02}-{}", n, child_slug));
            let p = format!("{}/index.md", cdir);
            placed.push(place(child, &p, &cdir, doc_title, false, anchors));
        }
    }
    Placed {
        path: self_path.to_string(),
        node,
        children: placed,
    }
}

/// Walks a file's blocks in render order, feeding every heading to the
/// counter and anchoring only the top-level ones.
///
/// Recursive on block nesting, so it carries `kasane_ir::MAX_BLOCK_DEPTH` like
/// every other block walk in this crate. Past the bound the subtree renders as
/// a truncation note with no headings in it, so stopping here costs nothing.
fn count_headings(
    blocks: &[Block],
    depth: usize,
    top_level: bool,
    self_path: &str,
    counter: &mut AnchorCounter,
    anchors: &mut HashMap<BlockId, String>,
) {
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        return;
    }
    for b in blocks {
        match b {
            Block::Heading { id, inlines, .. } => {
                let a = counter.next(&rendered_text(inlines));
                if top_level {
                    anchors.insert(*id, format!("{}#{}", self_path, a));
                }
            }
            Block::List { items, .. } => {
                for item in items {
                    count_headings(item, depth + 1, false, self_path, counter, anchors);
                }
            }
            // Counted in SOURCE position, which is a known narrow divergence
            // from GFM rather than the render order this walk otherwise
            // follows: GFM relocates every footnote definition into a trailing
            // `<section data-footnotes>`, so a heading inside one is assigned
            // its id after every body heading, not where the definition
            // appears. Reaching it needs a heading inside a footnote AND a
            // colliding base elsewhere in the same file, so the walk is left
            // as it is; restructuring it into two passes would complicate the
            // common case for that.
            Block::Footnote { blocks, .. } => {
                count_headings(blocks, depth + 1, false, self_path, counter, anchors);
            }
            _ => {}
        }
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
        let placed = assign_paths(tree, "B");
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
        let placed = assign_paths(tree, "");
        assert_eq!(placed.anchors[&BlockId(9)], "index.md#merged-bit");
    }

    #[test]
    fn duplicate_titles_get_github_style_suffixes() {
        // Two body headings with the same title. GitHub gives the first the
        // bare id and the second `-1`; before this, both got `#notes` and one
        // cross-reference silently landed on the other's heading.
        let tree = SectionTree {
            root: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body: vec![
                    Block::Heading {
                        level: 2,
                        id: BlockId(1),
                        inlines: vec![Inline::Text("Notes".into())],
                    },
                    Block::Heading {
                        level: 2,
                        id: BlockId(2),
                        inlines: vec![Inline::Text("Notes".into())],
                    },
                ],
                children: vec![],
                pages: None,
            },
        };
        let placed = assign_paths(tree, "Book");
        assert_eq!(placed.anchors[&BlockId(1)], "index.md#notes");
        assert_eq!(placed.anchors[&BlockId(2)], "index.md#notes-1");
    }

    #[test]
    fn the_files_own_title_consumes_the_first_slot() {
        // `file_to_markdown` prepends the file's title as a heading, so a body
        // heading repeating that title is the SECOND occurrence on the page.
        //
        // Built as a literal `SectionTree`, like `body_headings_get_anchors_too`
        // above: `fold_sections` never folds a heading into a parent's `body`
        // (only `balance` does, by demoting a merged subsection's heading
        // there), so a `Block::Heading` node run through `fold_sections` would
        // become a genuine nested child section -- its own file -- rather than
        // a body heading sharing this file with "Notes".
        let tree = SectionTree {
            root: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body: vec![],
                children: vec![SectionNode {
                    id: Some(BlockId(0)),
                    level: 1,
                    title: vec![Inline::Text("Notes".into())],
                    body: vec![Block::Heading {
                        level: 3,
                        id: BlockId(5),
                        inlines: vec![Inline::Text("Notes".into())],
                    }],
                    children: vec![],
                    pages: None,
                }],
                pages: None,
            },
        };
        let placed = assign_paths(tree, "Book");
        assert_eq!(placed.anchors[&BlockId(0)], "01-notes.md#notes");
        assert_eq!(placed.anchors[&BlockId(5)], "01-notes.md#notes-1");
    }

    #[test]
    fn the_root_file_counts_the_document_title() {
        // index.md renders the DOCUMENT title as its heading -- the root
        // node's own `title` is empty. Counting `node.title` here would slug
        // `section` and leave the body heading unsuffixed against a page that
        // really does have two `#book` ids.
        let tree = SectionTree {
            root: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body: vec![Block::Heading {
                    level: 2,
                    id: BlockId(3),
                    inlines: vec![Inline::Text("Book".into())],
                }],
                children: vec![],
                pages: None,
            },
        };
        let placed = assign_paths(tree, "Book");
        assert_eq!(placed.anchors[&BlockId(3)], "index.md#book-1");
    }

    #[test]
    fn a_root_level_leaf_with_no_id_seeds_its_counter_from_its_own_title() {
        // Shaped like a synthetic `Part N` node `balance.rs` splits off an
        // oversized ROOT body: `id: None`, no children, so it's a top-level
        // LEAF child of the root -- which means it inherits the root's own
        // (empty) `dir`, exactly as the root's own call has an empty `dir`.
        // `node.id.is_none() && dir.is_empty()` therefore fires for both, and
        // before making root-ness explicit (`is_root`), this seeded the
        // counter from the DOCUMENT title instead of "Part 1", silently
        // mis-suffixing the body heading below against the wrong page.
        let tree = SectionTree {
            root: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body: vec![],
                children: vec![SectionNode {
                    id: None,
                    level: 1,
                    title: vec![Inline::Text("Part 1".into())],
                    body: vec![Block::Heading {
                        level: 2,
                        id: BlockId(7),
                        inlines: vec![Inline::Text("Part 1".into())],
                    }],
                    children: vec![],
                    pages: None,
                }],
                pages: None,
            },
        };
        let placed = assign_paths(tree, "Book");
        assert_eq!(placed.anchors[&BlockId(7)], "01-part-1.md#part-1-1");
    }

    #[test]
    fn a_nested_heading_consumes_a_slot_without_getting_an_anchor() {
        // A heading inside a list item was never folded into a section, so it
        // gets no anchor -- but GitHub still gives it an id when it renders,
        // so it takes `notes-1` and pushes the next top-level heading to
        // `notes-2`. Counting only top-level headings would put `-1` on the
        // wrong heading.
        let tree = SectionTree {
            root: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body: vec![
                    Block::Heading {
                        level: 2,
                        id: BlockId(1),
                        inlines: vec![Inline::Text("Notes".into())],
                    },
                    Block::List {
                        ordered: false,
                        items: vec![vec![Block::Heading {
                            level: 3,
                            id: BlockId(2),
                            inlines: vec![Inline::Text("Notes".into())],
                        }]],
                    },
                    Block::Heading {
                        level: 2,
                        id: BlockId(3),
                        inlines: vec![Inline::Text("Notes".into())],
                    },
                ],
                children: vec![],
                pages: None,
            },
        };
        let placed = assign_paths(tree, "Book");
        assert_eq!(placed.anchors[&BlockId(1)], "index.md#notes");
        assert!(
            !placed.anchors.contains_key(&BlockId(2)),
            "a nested heading must not gain an anchor"
        );
        assert_eq!(placed.anchors[&BlockId(3)], "index.md#notes-2");
    }

    /// Pins both sides of the bound, matching how the other core walks are
    /// tested: nesting past `MAX_BLOCK_DEPTH` must return rather than
    /// recurse, and a heading just under the bound must still be reached.
    ///
    /// The negative half alone (a heading past the bound has no anchor)
    /// can't tell "stopped at the bound" from "never descended into
    /// `Block::List` at all" -- a nested heading gets no anchor of its own
    /// even when it IS reached, so absence of an anchor is not evidence of
    /// anything. The only observable proof that a nested heading was reached
    /// is that it consumed a counter slot: nest to `MAX_BLOCK_DEPTH - 2`
    /// (comfortably under the bound) and check that a *following* top-level
    /// heading with the same base gets suffixed `-1`, which only happens if
    /// the nested one was counted first.
    #[test]
    fn the_counting_walk_is_bounded() {
        // POSITIVE: reached, so it consumes a slot and pushes the next
        // same-titled heading to `-1`.
        let mut inner = vec![Block::Heading {
            level: 3,
            id: BlockId(1),
            inlines: vec![Inline::Text("Notes".into())],
        }];
        for _ in 0..(kasane_ir::MAX_BLOCK_DEPTH - 2) {
            inner = vec![Block::List {
                ordered: false,
                items: vec![inner],
            }];
        }
        let mut body = inner;
        body.push(Block::Heading {
            level: 2,
            id: BlockId(2),
            inlines: vec![Inline::Text("Notes".into())],
        });
        let tree = SectionTree {
            root: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body,
                children: vec![],
                pages: None,
            },
        };
        let placed = assign_paths(tree, "Book");
        assert!(
            !placed.anchors.contains_key(&BlockId(1)),
            "a nested heading must not gain an anchor"
        );
        assert_eq!(
            placed.anchors[&BlockId(2)],
            "index.md#notes-1",
            "a heading at MAX_BLOCK_DEPTH - 2 must still be reached and consume a slot"
        );

        // NEGATIVE: past the bound, unreachable -- no anchor, no suffix
        // pushed onto anything after it.
        let mut deep = vec![Block::Heading {
            level: 3,
            id: BlockId(99),
            inlines: vec![Inline::Text("Deep".into())],
        }];
        for _ in 0..(kasane_ir::MAX_BLOCK_DEPTH + 2) {
            deep = vec![Block::List {
                ordered: false,
                items: vec![deep],
            }];
        }
        let tree = SectionTree {
            root: SectionNode {
                id: None,
                level: 0,
                title: vec![],
                body: deep,
                children: vec![],
                pages: None,
            },
        };
        let placed = assign_paths(tree, "Book");
        assert!(!placed.anchors.contains_key(&BlockId(99)));
    }

    /// A section title carrying a footnote reference anchors on the text the
    /// title heading PRINTS, which `nav::walk` builds with `title_text` — the
    /// reference is not in it. Anchoring the inlines instead would predict
    /// `notes1` for a line that renders `Notes`.
    #[test]
    fn a_title_anchor_follows_the_printed_title_not_the_inlines() {
        let tree = fold_sections(&doc(vec![
            h(1, 0, "Top"),
            Node {
                block: Block::Heading {
                    level: 2,
                    id: BlockId(1),
                    inlines: vec![Inline::Text("Notes".into()), Inline::FootnoteRef(NoteId(7))],
                },
                prov: Provenance::default(),
            },
        ]));
        let placed = assign_paths(tree, "Book");
        let anchor = placed
            .anchors
            .get(&BlockId(1))
            .expect("the subsection has an anchor");
        assert!(
            anchor.ends_with("#notes"),
            "expected the printed-title anchor, got {anchor}"
        );
    }

    #[test]
    fn a_body_heading_with_an_empty_code_span_anchors_the_space_the_line_prints() {
        // `escape::code_span` prints an empty span as `` ` ` `` -- a real
        // space in the rendered line -- so GitHub ids `## a` `b` as `a-b`.
        // `rendered_text` took `Inline::Code("")` verbatim and this rule
        // computed `ab`: a cross-reference dead against GitHub's own render.
        // Design spec 2026-08-14-empty-code-span-anchor-design.md §2.3.
        //
        // Built through `fold_sections` and `balance` rather than as a literal
        // `SectionTree` like `body_headings_get_anchors_too` above: the
        // canonicalization lives in `fold_sections`' bounded clone, so a test
        // that hand-builds the tree would skip the code it is checking.
        let doc = doc(vec![
            h(1, 0, "Parent"),
            Node {
                block: Block::Heading {
                    level: 2,
                    id: BlockId(1),
                    inlines: vec![
                        Inline::Text("a".into()),
                        Inline::Code(String::new()),
                        Inline::Text("b".into()),
                    ],
                },
                prov: Provenance::default(),
            },
        ]);
        let mut tree = fold_sections(&doc);
        // The H2 is childless with an empty body, so MERGE demotes its heading
        // into the H1's body, where `count_headings` anchors it from
        // `rendered_text`.
        crate::balance(&mut tree, &crate::Options::default());
        let placed = assign_paths(tree, "B");
        assert_eq!(placed.anchors[&BlockId(1)], "01-parent.md#a-b");
    }
}
