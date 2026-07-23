//! Orchestration: DjVu bytes -> IR document. Page-native: every node carries the
//! 1-based page it came from.

mod doc;
mod outline;
mod text;

use crate::guard::MAX_TOTAL_BYTES;
use crate::{Adapter, ParseError};
use kasane_ir::{AssetBag, Block, BlockId, DocMeta, Document, Inline, Node, Provenance};
use outline::{outline_by_page, OutlineHeading};
use text::{modal_body_height, page_blocks, page_lines, Line};

/// Emitted for a page that has no text layer at all and nothing else to say.
const NO_TEXT_NOTE: &str = "no text layer; OCR not enabled";
/// Emitted for a page whose text layer is present but yielded no recoverable
/// lines (distinct from `NO_TEXT_NOTE`: the layer exists, it's just empty).
const EMPTY_TEXT_NOTE: &str = "text layer present but empty; no recoverable text";

pub struct DjvuAdapter;

impl Adapter for DjvuAdapter {
    fn parse(&self, bytes: &[u8], source_path: &str) -> Result<(Document, AssetBag), ParseError> {
        let djvu = doc::open(bytes)?;
        let n = doc::page_count(&djvu);
        let outline = outline_by_page(&doc::bookmarks(&djvu));
        // Any outline at all makes the outline the sole heading source.
        let has_outline = !outline.is_empty();

        // First pass: per-page lines (needed for the doc-wide modal body height).
        // The `bool` records whether the page had a text layer *at all*, keeping
        // "no text layer" distinct from "layer present but empty after filtering".
        let mut pages: Vec<(u32, bool, Vec<Line>)> = Vec::with_capacity(n as usize);
        let mut total_text_bytes: u64 = 0;
        for p in 1..=n {
            let root = doc::page_text(&djvu, p);
            let lines = root.as_ref().map(page_lines).unwrap_or_default();
            // Bomb guard over recovered text, cumulative across pages.
            let page_bytes: u64 = lines.iter().map(|l| l.text.len() as u64).sum();
            total_text_bytes = accumulate_text_bytes(total_text_bytes, page_bytes)?;
            pages.push((p, root.is_some(), lines));
        }

        let body_height =
            modal_body_height(&pages.iter().map(|(_, _, l)| l.clone()).collect::<Vec<_>>());

        let mut nodes = Vec::new();
        let mut next_id = 0u32;
        let empty: Vec<OutlineHeading> = Vec::new();
        for (p, has_text, lines) in &pages {
            let headings = outline.get(p).unwrap_or(&empty);
            nodes.extend(page_nodes_from_lines(
                *p,
                lines,
                headings,
                &mut next_id,
                body_height,
                !has_outline,
                *has_text,
            ));
        }

        let out = Document {
            meta: DocMeta {
                title: doc::title(&djvu).unwrap_or_else(|| derive_title(source_path)),
                authors: vec![],
                language: None,
                source_format: "djvu".into(),
                source_path: source_path.to_string(),
            },
            nodes,
        };
        Ok((out, AssetBag::default()))
    }
}

/// Build the nodes for one page from its lines. `has_text` distinguishes a page
/// with an (empty-after-filtering) text layer from one with none.
fn page_nodes_from_lines(
    page: u32,
    lines: &[Line],
    headings: &[OutlineHeading],
    next_id: &mut u32,
    body_height: f32,
    infer_headings: bool,
    has_text: bool,
) -> Vec<Node> {
    let prov = Provenance {
        source_pages: Some((page, page)),
        source_href: None,
    };
    let mut out = Vec::new();

    // Outline headings are spliced ahead of the page's own blocks.
    for h in headings {
        let id = BlockId(*next_id);
        *next_id += 1;
        out.push(Node {
            block: Block::Heading {
                level: h.level,
                id,
                inlines: vec![Inline::Text(h.title.clone())],
            },
            prov: prov.clone(),
        });
    }

    let blocks = page_blocks(lines, next_id, body_height, infer_headings);
    let had_blocks = !blocks.is_empty();
    for b in blocks {
        out.push(Node {
            block: b,
            prov: prov.clone(),
        });
    }

    // No outline heading, nothing recovered -> honest note. Every such page must
    // leave a trace, but the wording distinguishes "no text layer at all" from
    // "text layer present but empty after filtering".
    if headings.is_empty() && !had_blocks {
        let note = if has_text {
            EMPTY_TEXT_NOTE
        } else {
            NO_TEXT_NOTE
        };
        out.push(Node {
            block: Block::Raw { note: note.into() },
            prov,
        });
    }
    out
}

/// Test-facing wrapper: assemble a page from an optional text-layer zone. Shares
/// one code path with `parse`, deriving `has_text` the same way (layer present?).
#[cfg(test)]
fn page_nodes(
    page: u32,
    text_root: Option<&doc::Zone>,
    headings: &[OutlineHeading],
    next_id: &mut u32,
    body_height: f32,
    infer_headings: bool,
) -> Vec<Node> {
    let lines = text_root.map(page_lines).unwrap_or_default();
    page_nodes_from_lines(
        page,
        &lines,
        headings,
        next_id,
        body_height,
        infer_headings,
        text_root.is_some(),
    )
}

/// Cumulative bomb guard over recovered text bytes: adds `page_bytes` to `total`
/// and rejects once the running total exceeds `MAX_TOTAL_BYTES`. Saturating so a
/// pathological `page_bytes` (or an already-huge `total`) cannot wrap around to a
/// small value and slip past the check.
fn accumulate_text_bytes(total: u64, page_bytes: u64) -> Result<u64, ParseError> {
    let new_total = total.saturating_add(page_bytes);
    if new_total > MAX_TOTAL_BYTES {
        return Err(ParseError::Bomb);
    }
    Ok(new_total)
}

/// Title from the source filename stem (DjVu metadata title handled in `doc.rs`).
fn derive_title(source_path: &str) -> String {
    let file = source_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(source_path);
    file.strip_suffix(".djvu")
        .or_else(|| file.strip_suffix(".djv"))
        .unwrap_or(file)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Adapter;
    use doc::{BBox, Zone, ZoneKind};
    use kasane_ir::{Block, Inline};
    use outline::OutlineHeading;

    fn sample() -> kasane_ir::Document {
        let bytes = std::fs::read("../../tests/fixtures/djvu/sample.djvu").unwrap();
        DjvuAdapter.parse(&bytes, "sample.djvu").unwrap().0
    }
    fn text(inls: &[Inline]) -> String {
        inls.iter()
            .map(|i| match i {
                Inline::Text(t) => t.clone(),
                _ => String::new(),
            })
            .collect()
    }
    fn headings_of(nodes: &[kasane_ir::Node]) -> Vec<String> {
        nodes
            .iter()
            .filter_map(|n| match &n.block {
                Block::Heading { inlines, .. } => Some(text(inlines)),
                _ => None,
            })
            .collect()
    }
    fn line_zone(h: f32, t: &str) -> Zone {
        Zone {
            kind: ZoneKind::Line,
            bbox: BBox {
                x0: 0.0,
                y0: 0.0,
                x1: 10.0,
                y1: h,
            },
            text: t.into(),
            children: vec![],
        }
    }
    fn page_zone(children: Vec<Zone>) -> Zone {
        Zone {
            kind: ZoneKind::Page,
            bbox: BBox {
                x0: 0.0,
                y0: 0.0,
                x1: 10.0,
                y1: 0.0,
            },
            text: String::new(),
            children,
        }
    }

    #[test]
    fn end_to_end_outline_heading_and_page_provenance() {
        let doc = sample();
        assert_eq!(doc.meta.source_format, "djvu");
        assert_eq!(doc.meta.source_path, "sample.djvu");
        // No METa title in the fixture -> filename stem.
        assert_eq!(doc.meta.title, "sample");

        let heads = headings_of(&doc.nodes);
        assert_eq!(heads, vec!["Chapter One".to_string()], "heads: {heads:?}");

        // Page-native provenance on every node.
        assert!(doc
            .nodes
            .iter()
            .all(|n| n.prov.source_pages == Some((1, 1))));

        // Body text came through as paragraphs.
        let paras: Vec<String> = doc
            .nodes
            .iter()
            .filter_map(|n| match &n.block {
                Block::Para(p) => Some(text(p)),
                _ => None,
            })
            .collect();
        let all = paras.join(" ");
        assert!(all.contains("First body line."), "paras: {paras:?}");
        assert!(all.contains("Second body line."), "paras: {paras:?}");
    }

    #[test]
    fn no_text_page_emits_a_raw_note_not_an_error() {
        // Pure helper: a page with no text layer and no outline heading.
        let mut id = 0u32;
        let nodes = page_nodes(3, None, &[], &mut id, 0.0, true);
        assert_eq!(nodes.len(), 1);
        assert!(matches!(&nodes[0].block, Block::Raw { note } if note.contains("no text layer")));
        assert_eq!(nodes[0].prov.source_pages, Some((3, 3)));
    }

    #[test]
    fn present_but_empty_text_layer_is_not_a_no_text_page() {
        // A text layer that exists but yields no lines is distinct from `None`:
        // it must NOT claim "no text layer" -- but the page must still be
        // represented in the output, not silently dropped.
        let mut id = 0u32;
        let root = page_zone(vec![]);
        let nodes = page_nodes(2, Some(&root), &[], &mut id, 0.0, true);
        assert_eq!(nodes.len(), 1, "nodes: {nodes:?}");
        match &nodes[0].block {
            Block::Raw { note } => {
                assert_eq!(note, EMPTY_TEXT_NOTE);
                assert!(!note.contains("no text layer"), "note: {note}");
            }
            other => panic!("expected a Raw note, got {other:?}"),
        }
        assert_eq!(nodes[0].prov.source_pages, Some((2, 2)));
    }

    #[test]
    fn page_with_outline_heading_suppresses_height_inference() {
        // A tall line + an outline heading: only the outline heading is a Heading.
        let root = page_zone(vec![line_zone(24.0, "Tall body")]);
        let headings = [OutlineHeading {
            level: 1,
            title: "Real".into(),
        }];
        let mut id = 0u32;
        let nodes = page_nodes(1, Some(&root), &headings, &mut id, 12.0, false);
        assert_eq!(headings_of(&nodes), vec!["Real".to_string()]);
        assert!(nodes
            .iter()
            .any(|n| matches!(&n.block, Block::Para(p) if text(p) == "Tall body")));
        // The outline heading is spliced ahead of the page's blocks.
        assert!(matches!(nodes[0].block, Block::Heading { .. }));
    }

    #[test]
    fn block_ids_are_unique_across_pages() {
        // One counter threaded across two pages: an outline heading on page 1 and
        // a height-inferred heading on page 2 must not collide.
        let mut id = 0u32;
        let p1_root = page_zone(vec![line_zone(12.0, "Body one.")]);
        let p1 = page_nodes(
            1,
            Some(&p1_root),
            &[OutlineHeading {
                level: 1,
                title: "Outline H".into(),
            }],
            &mut id,
            12.0,
            false,
        );
        let p2_root = page_zone(vec![line_zone(24.0, "Inferred H"), line_zone(12.0, "b")]);
        let p2 = page_nodes(2, Some(&p2_root), &[], &mut id, 12.0, true);

        let ids: Vec<u32> = p1
            .iter()
            .chain(p2.iter())
            .filter_map(|n| match &n.block {
                Block::Heading { id, .. } => Some(id.0),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec![0, 1], "ids: {ids:?}");
        assert_eq!(id, 2);
        assert_eq!(headings_of(&p1), vec!["Outline H".to_string()]);
        assert_eq!(headings_of(&p2), vec!["Inferred H".to_string()]);
    }

    #[test]
    fn derive_title_strips_djvu_extensions_and_directories() {
        assert_eq!(derive_title("a/b/sample.djvu"), "sample");
        assert_eq!(derive_title("sample.djv"), "sample");
        assert_eq!(derive_title("noext"), "noext");
    }

    #[test]
    fn accumulate_text_bytes_allows_exactly_at_the_cap() {
        // Landing exactly on MAX_TOTAL_BYTES must NOT trip: the guard is `>`, not `>=`.
        let total = accumulate_text_bytes(0, MAX_TOTAL_BYTES).unwrap();
        assert_eq!(total, MAX_TOTAL_BYTES);
        // Building up to the cap across multiple pages must also not trip.
        let total = accumulate_text_bytes(MAX_TOTAL_BYTES - 1, 1).unwrap();
        assert_eq!(total, MAX_TOTAL_BYTES);
    }

    #[test]
    fn accumulate_text_bytes_trips_one_byte_past_the_cap() {
        let err = accumulate_text_bytes(0, MAX_TOTAL_BYTES + 1).unwrap_err();
        assert!(matches!(err, ParseError::Bomb));
        let err = accumulate_text_bytes(MAX_TOTAL_BYTES, 1).unwrap_err();
        assert!(matches!(err, ParseError::Bomb));
    }

    #[test]
    fn accumulate_text_bytes_saturates_instead_of_wrapping() {
        // A huge running total plus a huge page contribution must saturate to
        // u64::MAX (and thus trip Bomb), never wrap around to a small value that
        // would slip back under the cap.
        let err = accumulate_text_bytes(u64::MAX, u64::MAX).unwrap_err();
        assert!(matches!(err, ParseError::Bomb));
        let err = accumulate_text_bytes(u64::MAX, 1).unwrap_err();
        assert!(matches!(err, ParseError::Bomb));
    }
}
