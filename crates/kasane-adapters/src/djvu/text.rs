//! Pure: DjVu text-layer zones -> IR blocks. No `djvu-rs`, no files.
//! `page_lines` flattens a page's zone tree into reading-order lines, honoring
//! the hierarchy's own column/region/paragraph order so multi-column text comes
//! out correctly without geometric re-sorting. `page_blocks` groups those lines
//! into paragraphs and, when no NAVM outline exists for the document, infers
//! headings from lines that are taller than the modal body-line height.

use super::doc::{Zone, ZoneKind, MAX_ZONE_DEPTH};
use kasane_ir::{Block, BlockId, Inline};

/// One visual line of recovered text plus a font-size proxy (zone height) and
/// whether it opens a paragraph (first line under a Para/Region/Column zone).
#[derive(Clone, Debug)]
pub struct Line {
    pub text: String,
    pub height: f32,
    pub para_start: bool,
}

/// Flatten a page's zone tree into lines in document (reading) order. The zone
/// hierarchy already encodes columns/regions, so honoring its order yields
/// correct multi-column reading order without geometric re-sorting.
///
/// Not every producer emits `Line` zones: a minimal `djvused '(page ... "text")'`
/// text layer bottoms out at `Page`/`Para`/`Word`. Since every DjVu zone carries
/// the text its subtree covers, such a page still holds all of its text — so when
/// the `Line`-based walk finds nothing we fall back to the deepest text-bearing
/// zones rather than reporting an empty page.
pub fn page_lines(root: &Zone) -> Vec<Line> {
    let mut lines = Vec::new();
    walk(root, 0, &mut true, &mut lines);
    if lines.is_empty() {
        fallback_walk(root, 0, &mut true, &mut lines);
    }
    lines
}

/// Fallback for zone trees with no `Line` zones. Emits one line per *deepest*
/// text-bearing zone: a zone's own text is used only when no descendant produced
/// a line, so a parent's covering text is never counted alongside its children's.
/// Returns whether this subtree emitted anything.
fn fallback_walk(
    z: &Zone,
    depth: usize,
    pending_para_start: &mut bool,
    out: &mut Vec<Line>,
) -> bool {
    if depth > MAX_ZONE_DEPTH {
        return false;
    }
    if matches!(z.kind, ZoneKind::Para | ZoneKind::Region | ZoneKind::Column) {
        *pending_para_start = true;
    }
    // A zone whose children are all Word/Char is itself the line-like unit; its
    // words are joined rather than emitted one line apiece.
    if !is_line_like(z) {
        let mut emitted = false;
        for c in &z.children {
            emitted |= fallback_walk(c, depth + 1, pending_para_start, out);
        }
        if emitted {
            return true;
        }
    }
    // Only a line-like zone may draw on its children's text; a container whose
    // subtree produced nothing (because it was cut off by the depth cap, say)
    // falls back to its OWN covering text. That is what stops a container at the
    // cap from resurrecting text the cap just rejected. A line-like zone at the
    // cap does still join its Word/Char children one level past it — the same as
    // `walk` does for a `Line`, and safe because `line_text` is a flat loop.
    let text = if is_line_like(z) {
        line_text(z)
    } else {
        z.text.trim().to_string()
    };
    if text.is_empty() {
        return false;
    }
    out.push(Line {
        text,
        height: z.bbox.height(),
        para_start: std::mem::replace(pending_para_start, false),
    });
    true
}

/// `true` when `z` has no children, or only Word/Char leaves — i.e. it plays the
/// role a `Line` zone would in a fully-structured text layer.
fn is_line_like(z: &Zone) -> bool {
    z.children
        .iter()
        .all(|c| matches!(c.kind, ZoneKind::Word | ZoneKind::Char))
}

/// `pending_para_start` is set when we cross into a new paragraph container and
/// consumed by the next line emitted.
fn walk(z: &Zone, depth: usize, pending_para_start: &mut bool, out: &mut Vec<Line>) {
    if depth > MAX_ZONE_DEPTH {
        return;
    }
    match z.kind {
        ZoneKind::Line => {
            let text = line_text(z);
            if !text.is_empty() {
                out.push(Line {
                    text,
                    height: z.bbox.height(),
                    para_start: std::mem::replace(pending_para_start, false),
                });
            }
        }
        ZoneKind::Para | ZoneKind::Region | ZoneKind::Column => {
            *pending_para_start = true;
            for c in &z.children {
                walk(c, depth + 1, pending_para_start, out);
            }
        }
        _ => {
            for c in &z.children {
                walk(c, depth + 1, pending_para_start, out);
            }
        }
    }
}

/// Line text: direct text if present, else Word/Char children joined by spaces.
fn line_text(line: &Zone) -> String {
    let direct = line.text.trim();
    if !direct.is_empty() {
        return direct.to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    for w in &line.children {
        let t = w.text.trim();
        if !t.is_empty() {
            parts.push(t.to_string());
        }
    }
    parts.join(" ")
}

const HEADING_RATIO: f32 = 1.15;

/// Most common rounded line height across all pages — the document body height.
pub fn modal_body_height(pages: &[Vec<Line>]) -> f32 {
    use std::collections::HashMap;
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for page in pages {
        for l in page {
            *counts.entry(l.height.round() as i32).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(h, _)| h as f32)
        .unwrap_or(0.0)
}

/// Build blocks for one page. When `infer_headings`, a line ≥15% taller than the
/// body height becomes a heading (level bucketed 1–3); otherwise every line is
/// body text. Consecutive body lines merge into a paragraph, split on
/// `para_start`.
pub fn page_blocks(
    lines: &[Line],
    next_id: &mut u32,
    body_height: f32,
    infer_headings: bool,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut para: Vec<String> = Vec::new();

    let flush = |blocks: &mut Vec<Block>, para: &mut Vec<String>| {
        if !para.is_empty() {
            blocks.push(Block::Para(vec![Inline::Text(para.join(" "))]));
            para.clear();
        }
    };

    for l in lines {
        let is_heading =
            infer_headings && body_height > 0.0 && l.height >= body_height * HEADING_RATIO;
        if is_heading {
            flush(&mut blocks, &mut para);
            let id = BlockId(*next_id);
            *next_id += 1;
            blocks.push(Block::Heading {
                level: heading_level(l.height, body_height),
                id,
                inlines: vec![Inline::Text(l.text.clone())],
            });
        } else {
            if l.para_start {
                flush(&mut blocks, &mut para);
            }
            para.push(l.text.clone());
        }
    }
    flush(&mut blocks, &mut para);
    blocks
}

/// Bucket a heading height into levels 1–3 by how far it exceeds the body.
fn heading_level(height: f32, body: f32) -> u8 {
    let ratio = if body > 0.0 { height / body } else { 1.0 };
    if ratio >= 1.8 {
        1
    } else if ratio >= 1.4 {
        2
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::djvu::doc::{BBox, Zone, ZoneKind};

    fn z(kind: ZoneKind, h: f32, text: &str, children: Vec<Zone>) -> Zone {
        Zone {
            kind,
            bbox: BBox {
                x0: 0.0,
                y0: 0.0,
                x1: 10.0,
                y1: h,
            },
            text: text.into(),
            children,
        }
    }
    fn word(t: &str, h: f32) -> Zone {
        z(ZoneKind::Word, h, t, vec![])
    }
    fn line(h: f32, words: &[&str]) -> Zone {
        z(
            ZoneKind::Line,
            h,
            "",
            words.iter().map(|w| word(w, h)).collect(),
        )
    }

    #[test]
    fn concatenates_words_into_line_text_with_height() {
        let page = z(
            ZoneKind::Page,
            0.0,
            "",
            vec![z(
                ZoneKind::Para,
                0.0,
                "",
                vec![line(12.0, &["Hello", "world"])],
            )],
        );
        let lines = page_lines(&page);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Hello world");
        assert!((lines[0].height - 12.0).abs() < 0.01);
        assert!(lines[0].para_start);
    }

    #[test]
    fn first_line_of_each_paragraph_marks_para_start() {
        let page = z(
            ZoneKind::Page,
            0.0,
            "",
            vec![
                z(
                    ZoneKind::Para,
                    0.0,
                    "",
                    vec![line(12.0, &["a"]), line(12.0, &["b"])],
                ),
                z(ZoneKind::Para, 0.0, "", vec![line(12.0, &["c"])]),
            ],
        );
        let starts: Vec<bool> = page_lines(&page).iter().map(|l| l.para_start).collect();
        assert_eq!(starts, vec![true, false, true]);
    }

    #[test]
    fn columns_are_read_in_hierarchy_order() {
        // Two columns; hierarchy order (col1 then col2) is the reading order.
        let col = |t: &str| {
            z(
                ZoneKind::Column,
                0.0,
                "",
                vec![z(ZoneKind::Para, 0.0, "", vec![line(12.0, &[t])])],
            )
        };
        let page = z(ZoneKind::Page, 0.0, "", vec![col("left"), col("right")]);
        let texts: Vec<String> = page_lines(&page).into_iter().map(|l| l.text).collect();
        assert_eq!(texts, vec!["left".to_string(), "right".to_string()]);
    }

    #[test]
    fn line_zone_with_direct_text_and_no_word_children_is_used() {
        // Some encoders put text directly on the Line zone.
        let page = z(
            ZoneKind::Page,
            0.0,
            "",
            vec![z(ZoneKind::Line, 14.0, "Direct", vec![])],
        );
        let lines = page_lines(&page);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Direct");
    }

    #[test]
    fn line_at_max_zone_depth_boundary_is_emitted() {
        // Build a chain to place the line at exactly MAX_ZONE_DEPTH.
        // We need MAX_ZONE_DEPTH - 1 Region zones wrapping the Line,
        // since Page is at depth 0, first Region at depth 1, etc.
        let mut zone = z(ZoneKind::Line, 12.0, "at_boundary", vec![]);

        for _ in 0..(MAX_ZONE_DEPTH - 1) {
            zone = z(ZoneKind::Region, 0.0, "", vec![zone]);
        }

        let page = z(ZoneKind::Page, 0.0, "", vec![zone]);
        let lines = page_lines(&page);
        assert_eq!(lines.len(), 1, "line at MAX_ZONE_DEPTH should be emitted");
        assert_eq!(lines[0].text, "at_boundary");
    }

    #[test]
    fn line_beyond_max_zone_depth_is_not_emitted() {
        // Build a chain to place the line at depth MAX_ZONE_DEPTH + 1.
        // We need MAX_ZONE_DEPTH Region zones wrapping the Line.
        // The depth check is `if depth > MAX_ZONE_DEPTH { return; }`,
        // so depth 65 is rejected while depth 64 is accepted.
        let mut zone = z(ZoneKind::Line, 12.0, "beyond", vec![]);

        for _ in 0..MAX_ZONE_DEPTH {
            zone = z(ZoneKind::Region, 0.0, "", vec![zone]);
        }

        let page = z(ZoneKind::Page, 0.0, "", vec![zone]);
        let lines = page_lines(&page);
        assert_eq!(
            lines.len(),
            0,
            "line beyond MAX_ZONE_DEPTH should not be emitted"
        );
    }

    #[test]
    fn empty_and_whitespace_lines_are_skipped() {
        // Test that a paragraph containing [real line, blank line, real line]
        // emits exactly 2 lines (the blank is skipped) in order, with correct para_start flags.
        let page = z(
            ZoneKind::Page,
            0.0,
            "",
            vec![z(
                ZoneKind::Para,
                0.0,
                "",
                vec![
                    line(12.0, &["first"]),
                    // Line with only whitespace: direct text is spaces,
                    // and word child is also only spaces.
                    z(ZoneKind::Line, 12.0, "   ", vec![word("   ", 12.0)]),
                    line(12.0, &["third"]),
                ],
            )],
        );

        let lines = page_lines(&page);
        assert_eq!(lines.len(), 2, "only non-empty lines should be emitted");
        assert_eq!(lines[0].text, "first");
        assert_eq!(lines[1].text, "third");

        // The first emitted line (at paragraph start) should have para_start=true,
        // the second should have para_start=false (even though a blank was skipped
        // in between, the flag was not consumed).
        assert!(lines[0].para_start);
        assert!(!lines[1].para_start);
    }

    #[test]
    fn para_zones_without_line_zones_still_yield_their_text() {
        // A minimal `djvused '(page ... "text")'` layer bottoms out at Para:
        // there is no Line zone anywhere, but the text is fully present.
        let page = z(
            ZoneKind::Page,
            0.0,
            "First para. Second para.",
            vec![
                z(ZoneKind::Para, 12.0, "First para.", vec![]),
                z(ZoneKind::Para, 12.0, "Second para.", vec![]),
            ],
        );
        let lines = page_lines(&page);
        assert_eq!(lines.len(), 2, "lines: {lines:?}");
        assert_eq!(lines[0].text, "First para.");
        assert_eq!(lines[1].text, "Second para.");
        assert!((lines[0].height - 12.0).abs() < 0.01);
        // Each Para opens a paragraph.
        assert!(lines[0].para_start && lines[1].para_start);
    }

    #[test]
    fn page_with_only_its_own_text_yields_one_line() {
        let page = z(ZoneKind::Page, 20.0, "Just the page text.", vec![]);
        let lines = page_lines(&page);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Just the page text.");
        assert!((lines[0].height - 20.0).abs() < 0.01);
        assert!(lines[0].para_start);
    }

    #[test]
    fn word_zones_without_line_zones_join_into_one_line() {
        // Page -> Para -> Word*: the Para is the line-like unit, so its words
        // join into a single line rather than one line per word.
        let page = z(
            ZoneKind::Page,
            0.0,
            "",
            vec![z(
                ZoneKind::Para,
                14.0,
                "",
                vec![word("Hello", 14.0), word("world", 14.0)],
            )],
        );
        let lines = page_lines(&page);
        assert_eq!(lines.len(), 1, "lines: {lines:?}");
        assert_eq!(lines[0].text, "Hello world");
        assert!((lines[0].height - 14.0).abs() < 0.01);
    }

    #[test]
    fn covering_parent_text_is_never_emitted_alongside_descendants() {
        // Every DjVu zone carries the text its subtree covers. The fallback must
        // emit only the deepest text-bearing zones, never the parent as well.
        let page = z(
            ZoneKind::Page,
            30.0,
            "one two",
            vec![z(
                ZoneKind::Region,
                20.0,
                "one two",
                vec![
                    z(ZoneKind::Para, 12.0, "one", vec![]),
                    z(ZoneKind::Para, 12.0, "two", vec![]),
                ],
            )],
        );
        let texts: Vec<String> = page_lines(&page).into_iter().map(|l| l.text).collect();
        assert_eq!(texts, vec!["one".to_string(), "two".to_string()]);
    }

    #[test]
    fn fallback_does_not_run_when_line_zones_exist() {
        // A Page carrying covering text AND real Line zones: only the Lines are
        // emitted; the parents' covering text must not be duplicated.
        let page = z(
            ZoneKind::Page,
            30.0,
            "one two",
            vec![z(
                ZoneKind::Para,
                20.0,
                "one two",
                vec![line(12.0, &["one"]), line(12.0, &["two"])],
            )],
        );
        let texts: Vec<String> = page_lines(&page).into_iter().map(|l| l.text).collect();
        assert_eq!(texts, vec!["one".to_string(), "two".to_string()]);
    }

    #[test]
    fn fallback_respects_the_depth_cap() {
        // Text-bearing zone one level past the cap: dropped, not rescued.
        let mut zone = z(ZoneKind::Para, 12.0, "too deep", vec![]);
        for _ in 0..MAX_ZONE_DEPTH {
            zone = z(ZoneKind::Region, 0.0, "", vec![zone]);
        }
        let page = z(ZoneKind::Page, 0.0, "", vec![zone]);
        assert!(page_lines(&page).is_empty());

        // One level shallower it is recovered.
        let mut zone = z(ZoneKind::Para, 12.0, "at boundary", vec![]);
        for _ in 0..(MAX_ZONE_DEPTH - 1) {
            zone = z(ZoneKind::Region, 0.0, "", vec![zone]);
        }
        let page = z(ZoneKind::Page, 0.0, "", vec![zone]);
        let lines = page_lines(&page);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "at boundary");
    }

    #[test]
    fn empty_zone_tree_still_yields_no_lines() {
        // The fallback must not manufacture lines: an empty layer stays empty so
        // `mod.rs` can still emit its "present but empty" note.
        let page = z(ZoneKind::Page, 0.0, "", vec![]);
        assert!(page_lines(&page).is_empty());
        let page = z(
            ZoneKind::Page,
            0.0,
            "   ",
            vec![z(ZoneKind::Para, 0.0, "  ", vec![])],
        );
        assert!(page_lines(&page).is_empty());
    }

    fn body_line(t: &str) -> Line {
        Line {
            text: t.into(),
            height: 12.0,
            para_start: false,
        }
    }

    #[test]
    fn modal_body_height_is_the_commonest_rounded_height() {
        let pages = vec![vec![
            Line {
                text: "h".into(),
                height: 24.0,
                para_start: true,
            },
            body_line("a"),
            body_line("b"),
        ]];
        assert!((modal_body_height(&pages) - 12.0).abs() < 0.01);
    }

    #[test]
    fn modal_body_height_empty_returns_zero() {
        let pages: Vec<Vec<Line>> = vec![];
        assert_eq!(modal_body_height(&pages), 0.0);
    }

    #[test]
    fn modal_body_height_counts_across_all_pages() {
        let pages = vec![
            vec![body_line("page1_a"), body_line("page1_b")],
            vec![
                Line {
                    text: "page2_h".into(),
                    height: 16.0,
                    para_start: true,
                },
                body_line("page2_c"),
            ],
        ];
        // 4 lines at height 12.0, 1 line at height 16.0 -> 12.0 is modal
        assert!((modal_body_height(&pages) - 12.0).abs() < 0.01);
    }

    #[test]
    fn tall_line_becomes_heading_and_body_lines_merge() {
        let lines = vec![
            Line {
                text: "Big Title".into(),
                height: 24.0,
                para_start: true,
            },
            Line {
                text: "Body one.".into(),
                height: 12.0,
                para_start: true,
            },
            Line {
                text: "Body two.".into(),
                height: 12.0,
                para_start: false,
            },
        ];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 12.0, true);
        match &blocks[0] {
            Block::Heading { level, inlines, .. } => {
                assert_eq!(level, &1);
                assert_eq!(inline_text(inlines), "Big Title");
            }
            other => panic!("expected heading, got {other:?}"),
        }
        assert_eq!(
            para_text(&blocks[1]).as_deref(),
            Some("Body one. Body two.")
        );
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn paragraph_boundary_splits_on_para_start() {
        let lines = vec![
            Line {
                text: "one".into(),
                height: 12.0,
                para_start: true,
            },
            Line {
                text: "two".into(),
                height: 12.0,
                para_start: true,
            },
        ];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 12.0, true);
        assert_eq!(blocks.len(), 2);
        assert_eq!(para_text(&blocks[0]).as_deref(), Some("one"));
        assert_eq!(para_text(&blocks[1]).as_deref(), Some("two"));
    }

    #[test]
    fn infer_headings_false_keeps_tall_lines_as_paragraphs() {
        let lines = vec![Line {
            text: "Big".into(),
            height: 24.0,
            para_start: true,
        }];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 12.0, false);
        assert!(matches!(blocks[0], Block::Para(_)));
    }

    #[test]
    fn heading_ratio_threshold_just_below_stays_paragraph() {
        // body_height = 12.0, HEADING_RATIO = 1.15, so threshold is 13.8
        // A line at 13.7 should stay as paragraph
        let lines = vec![Line {
            text: "Almost tall".into(),
            height: 13.7,
            para_start: true,
        }];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 12.0, true);
        assert!(
            matches!(blocks[0], Block::Para(_)),
            "line just below threshold should be paragraph"
        );
    }

    #[test]
    fn heading_ratio_threshold_at_exactly_ratio_becomes_heading() {
        // body_height = 12.0, HEADING_RATIO = 1.15, so threshold is 13.8
        // A line at 13.8 (exactly at threshold) should become a heading
        let lines = vec![Line {
            text: "Heading".into(),
            height: 13.8,
            para_start: true,
        }];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 12.0, true);
        assert!(
            matches!(blocks[0], Block::Heading { .. }),
            "line at exactly threshold should be heading"
        );
    }

    #[test]
    fn heading_level_1_at_1_8_ratio() {
        // body = 12.0, height = 21.6 (ratio 1.8) should be level 1
        let lines = vec![Line {
            text: "H1".into(),
            height: 21.6,
            para_start: true,
        }];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 12.0, true);
        match &blocks[0] {
            Block::Heading { level, .. } => assert_eq!(level, &1),
            _ => panic!("expected heading"),
        }
    }

    #[test]
    fn heading_level_2_at_1_4_ratio() {
        // body = 12.0, height = 16.8 (ratio 1.4) should be level 2
        let lines = vec![Line {
            text: "H2".into(),
            height: 16.8,
            para_start: true,
        }];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 12.0, true);
        match &blocks[0] {
            Block::Heading { level, .. } => assert_eq!(level, &2),
            _ => panic!("expected heading"),
        }
    }

    #[test]
    fn heading_level_3_below_1_4_ratio() {
        // body = 12.0, height = 14.0 (ratio 1.167) should be level 3
        let lines = vec![Line {
            text: "H3".into(),
            height: 14.0,
            para_start: true,
        }];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 12.0, true);
        match &blocks[0] {
            Block::Heading { level, .. } => assert_eq!(*level, 3),
            _ => panic!("expected heading"),
        }
    }

    #[test]
    fn heading_level_2_between_1_4_and_1_8() {
        // body = 10.0, height = 17.9 (ratio 1.79)
        // Strictly between 1.4 and 1.8, should be level 2.
        // Height 17.9 >= 10.0 * 1.15 = 11.5, qualifies as heading.
        // This test fails if the 1.8 threshold is mutated downward below 1.79.
        let lines = vec![Line {
            text: "H2-between".into(),
            height: 17.9,
            para_start: true,
        }];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 10.0, true);
        match &blocks[0] {
            Block::Heading { level, .. } => assert_eq!(level, &2),
            _ => panic!("expected heading"),
        }
    }

    #[test]
    fn heading_level_3_between_1_15_and_1_4() {
        // body = 10.0, height = 13.9 (ratio 1.39)
        // Strictly between 1.15 and 1.4, should be level 3.
        // Height 13.9 >= 10.0 * 1.15 = 11.5, qualifies as heading.
        // This test fails if the 1.4 threshold is mutated downward below 1.39.
        let lines = vec![Line {
            text: "H3-between".into(),
            height: 13.9,
            para_start: true,
        }];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 10.0, true);
        match &blocks[0] {
            Block::Heading { level, .. } => assert_eq!(*level, 3),
            _ => panic!("expected heading"),
        }
    }

    #[test]
    fn next_id_increments_per_heading() {
        let lines = vec![
            Line {
                text: "H1".into(),
                height: 24.0,
                para_start: true,
            },
            Line {
                text: "Body".into(),
                height: 12.0,
                para_start: true,
            },
            Line {
                text: "H2".into(),
                height: 24.0,
                para_start: true,
            },
        ];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 12.0, true);

        // Should have 3 blocks: heading, para, heading
        assert_eq!(blocks.len(), 3);

        // Extract heading IDs
        let h1_id = match &blocks[0] {
            Block::Heading { id, .. } => id.0,
            _ => panic!("expected first heading"),
        };
        let h2_id = match &blocks[2] {
            Block::Heading { id, .. } => id.0,
            _ => panic!("expected second heading"),
        };

        // IDs should be distinct and increasing
        assert_eq!(h1_id, 0);
        assert_eq!(h2_id, 1);
        // next_id should be left at 2 (one more than the last assigned ID)
        assert_eq!(id, 2);
    }

    #[test]
    fn infer_headings_with_zero_body_height_ignores_heading_inference() {
        let lines = vec![Line {
            text: "Big".into(),
            height: 24.0,
            para_start: true,
        }];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 0.0, true);
        // Even with infer_headings=true, body_height=0.0 prevents heading detection
        assert!(matches!(blocks[0], Block::Para(_)));
    }

    fn inline_text(inls: &[Inline]) -> String {
        inls.iter()
            .map(|i| match i {
                Inline::Text(t) => t.clone(),
                _ => String::new(),
            })
            .collect()
    }

    fn para_text(b: &Block) -> Option<String> {
        if let Block::Para(inls) = b {
            Some(inline_text(inls))
        } else {
            None
        }
    }
}
