use super::content::TextRun;
use kasane_ir::{Block, BlockId, Inline};

/// A single visual line of text.
#[derive(Clone, Debug)]
pub struct Line {
    pub y: f32,
    pub x: f32,
    pub size: f32,
    pub text: String,
}

/// Group runs into lines (same y-band) in reading order: top→bottom, left→right.
pub fn group_lines(mut runs: Vec<TextRun>) -> Vec<Line> {
    if runs.is_empty() {
        return Vec::new();
    }
    // Sort top→bottom (larger y first), then left→right.
    runs.sort_by(|a, b| {
        b.y.partial_cmp(&a.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut lines: Vec<Line> = Vec::new();
    for r in runs {
        // y-tolerance scales with font size (sub/superscripts stay on the line).
        let tol = (r.size * 0.5).max(2.0);
        match lines.last_mut() {
            Some(last) if (last.y - r.y).abs() <= tol => {
                if !last.text.ends_with(' ') && !r.text.starts_with(' ') {
                    last.text.push(' ');
                }
                last.text.push_str(r.text.trim());
                last.size = last.size.max(r.size);
                last.x = last.x.min(r.x);
            }
            _ => lines.push(Line { y: r.y, x: r.x, size: r.size, text: r.text.trim().to_string() }),
        }
    }
    for l in &mut lines {
        l.text = l.text.trim().to_string();
    }
    lines.retain(|l| !l.text.is_empty());
    lines
}

/// Most common rounded line size across all pages — the document's body size.
pub fn modal_body_size(pages: &[Vec<Line>]) -> f32 {
    use std::collections::HashMap;
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for page in pages {
        for l in page {
            *counts.entry(l.size.round() as i32).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(sz, _)| sz as f32)
        .unwrap_or(0.0)
}

const HEADING_RATIO: f32 = 1.15;

/// Build paragraph/heading blocks for one page, with no outline available.
/// A line ≥15% larger than the body size becomes a heading; consecutive
/// body-size lines merge into a paragraph, split on large vertical gaps.
pub fn page_blocks_no_headings(lines: &[Line], next_id: &mut u32, body_size: f32) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut prev_y: Option<f32> = None;
    let mut prev_size = body_size;

    let flush = |blocks: &mut Vec<Block>, para: &mut Vec<String>| {
        if !para.is_empty() {
            let text = para.join(" ");
            blocks.push(Block::Para(vec![Inline::Text(text)]));
            para.clear();
        }
    };

    for l in lines {
        let is_heading = body_size > 0.0 && l.size >= body_size * HEADING_RATIO;
        if is_heading {
            flush(&mut blocks, &mut para);
            let id = BlockId(*next_id);
            *next_id += 1;
            blocks.push(Block::Heading {
                level: heading_level(l.size, body_size),
                id,
                inlines: vec![Inline::Text(l.text.clone())],
            });
        } else {
            // Paragraph break on a vertical gap larger than 1.5× line height.
            if let Some(py) = prev_y {
                if (py - l.y) > prev_size.max(l.size) * 1.5 {
                    flush(&mut blocks, &mut para);
                }
            }
            para.push(l.text.clone());
        }
        prev_y = Some(l.y);
        prev_size = l.size;
    }
    flush(&mut blocks, &mut para);
    blocks
}

/// Bucket a heading size into levels 1–3 by how far it exceeds the body size.
fn heading_level(size: f32, body: f32) -> u8 {
    let ratio = if body > 0.0 { size / body } else { 1.0 };
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
    use crate::pdf::content::TextRun;
    use kasane_ir::{Block, Inline};

    fn run(x: f32, y: f32, size: f32, t: &str) -> TextRun {
        TextRun { x, y, size, text: t.into() }
    }

    fn heading_text(b: &Block) -> Option<(u8, String)> {
        if let Block::Heading { level, inlines, .. } = b {
            Some((*level, inline_text(inlines)))
        } else {
            None
        }
    }
    fn para_text(b: &Block) -> Option<String> {
        if let Block::Para(inlines) = b { Some(inline_text(inlines)) } else { None }
    }
    fn inline_text(inls: &[Inline]) -> String {
        inls.iter().map(|i| match i { Inline::Text(t) => t.clone(), _ => String::new() }).collect()
    }

    #[test]
    fn groups_runs_into_lines_in_reading_order() {
        // Two runs on the same line (same y), then a lower line.
        let runs = vec![
            run(60.0, 170.0, 12.0, "world"),
            run(20.0, 170.0, 12.0, "hello"),
            run(20.0, 150.0, 12.0, "next"),
        ];
        let lines = group_lines(runs);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "hello world"); // sorted by x within the line
        assert_eq!(lines[1].text, "next");
    }

    #[test]
    fn promotes_large_line_to_heading_and_merges_body() {
        let page = vec![
            run(20.0, 190.0, 24.0, "Big Title"),
            run(20.0, 160.0, 12.0, "Body line one."),
            run(20.0, 146.0, 12.0, "Body line two."),
        ];
        let lines = group_lines(page);
        let body = modal_body_size(&[lines.clone()]);
        assert!((body - 12.0).abs() < 0.01, "body size {body}");
        let mut id = 0u32;
        let blocks = page_blocks_no_headings(&lines, &mut id, body);
        assert_eq!(heading_text(&blocks[0]), Some((1, "Big Title".into())));
        // The two 12pt lines merge into a single paragraph.
        assert_eq!(para_text(&blocks[1]).as_deref(), Some("Body line one. Body line two."));
        assert_eq!(blocks.len(), 2);
    }
}
