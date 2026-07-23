//! Pure: DjVu text-layer zones -> IR blocks. No `djvu-rs`, no files.
//! Functions are added in Tasks 5 (page_lines) and 6 (page_blocks).

use super::doc::{Zone, ZoneKind};

/// One visual line of recovered text plus a font-size proxy (zone height) and
/// whether it opens a paragraph (first line under a Para/Region/Column zone).
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Line {
    pub text: String,
    pub height: f32,
    pub para_start: bool,
}

const MAX_ZONE_DEPTH: usize = 64;

/// Flatten a page's zone tree into lines in document (reading) order. The zone
/// hierarchy already encodes columns/regions, so honoring its order yields
/// correct multi-column reading order without geometric re-sorting.
#[allow(dead_code)]
pub fn page_lines(root: &Zone) -> Vec<Line> {
    let mut lines = Vec::new();
    walk(root, 0, &mut true, &mut lines);
    lines
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
}
