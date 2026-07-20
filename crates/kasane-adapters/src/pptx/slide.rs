use crate::pptx::rels::{unescape_attr, SlideRels};
use kasane_ir::{Block, BlockId, Inline};
use quick_xml::events::Event;
use quick_xml::Reader;

pub(crate) struct Paragraph {
    pub level: u8,
    pub inlines: Vec<Inline>,
}

pub(crate) enum Shape {
    Title(Vec<Inline>),
    Body(Vec<Paragraph>),
    Table(kasane_ir::Table),
}

// Run-formatting state carried while inside <a:r>.
#[derive(Default)]
struct RunFmt {
    bold: bool,
    italic: bool,
}

fn attr_bool(e: &quick_xml::events::BytesStart, key: &[u8]) -> bool {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .map(|a| {
            let v = String::from_utf8_lossy(&a.value);
            v == "1" || v == "true"
        })
        .unwrap_or(false)
}

fn attr_str(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .map(unescape_attr)
}

fn styled(text: String, fmt: &RunFmt) -> Inline {
    let mut inl = Inline::Text(text);
    if fmt.bold {
        inl = Inline::Strong(vec![inl]);
    }
    if fmt.italic {
        inl = Inline::Emph(vec![inl]);
    }
    inl
}

pub(crate) fn parse_shapes(xml: &str, _rels: &SlideRels) -> Vec<Shape> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut buf = Vec::new();

    let mut shapes = Vec::new();
    let mut in_sp = false;
    let mut sp_is_title = false;
    let mut paras: Vec<Paragraph> = Vec::new();
    let mut cur_para: Option<Paragraph> = None;
    let mut fmt = RunFmt::default();
    let mut in_run = false;
    let mut in_tbl = false;
    let mut tbl_rows: Vec<Vec<Vec<Inline>>> = Vec::new();
    let mut cur_row: Vec<Vec<Inline>> = Vec::new();
    let mut cur_cell: Vec<Inline> = Vec::new();
    let mut in_cell = false;
    let mut has_merged = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"sp" => {
                    in_sp = true;
                    sp_is_title = false;
                    paras = Vec::new();
                }
                b"ph" => {
                    let t = attr_str(&e, b"type").unwrap_or_default();
                    if t == "title" || t == "ctrTitle" {
                        sp_is_title = true;
                    }
                }
                b"p" if in_sp => {
                    let mut level = 0u8;
                    // <a:pPr lvl="N"> may be the next event; capture inline attr if empty-expanded
                    if let Some(l) = attr_str(&e, b"lvl") {
                        level = l.parse().unwrap_or(0);
                    }
                    cur_para = Some(Paragraph {
                        level,
                        inlines: Vec::new(),
                    });
                }
                b"pPr" => {
                    if let (Some(p), Some(l)) = (cur_para.as_mut(), attr_str(&e, b"lvl")) {
                        p.level = l.parse().unwrap_or(0);
                    }
                }
                b"r" if in_sp => {
                    in_run = true;
                    fmt = RunFmt::default();
                }
                b"rPr" if in_run => {
                    fmt.bold = attr_bool(&e, b"b");
                    fmt.italic = attr_bool(&e, b"i");
                }
                b"tbl" => {
                    in_tbl = true;
                    tbl_rows = Vec::new();
                }
                b"tr" if in_tbl => cur_row = Vec::new(),
                b"tc" if in_tbl => {
                    // gridSpan/hMerge/vMerge/rowSpan => the writer's HTML fallback
                    if attr_str(&e, b"gridSpan").is_some()
                        || attr_str(&e, b"rowSpan").is_some()
                        || attr_bool(&e, b"hMerge")
                        || attr_bool(&e, b"vMerge")
                    {
                        has_merged = true;
                    }
                    in_cell = true;
                    cur_cell = Vec::new();
                }
                b"r" if in_cell => {
                    in_run = true;
                    fmt = RunFmt::default();
                }
                _ => {}
            },
            Ok(Event::Text(t)) if in_run => {
                let s = t.unescape().unwrap_or_default().to_string();
                if !s.is_empty() {
                    if in_cell {
                        cur_cell.push(styled(s, &fmt));
                    } else if let Some(p) = cur_para.as_mut() {
                        p.inlines.push(styled(s, &fmt));
                    }
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"r" => in_run = false,
                b"p" if in_sp => {
                    if let Some(p) = cur_para.take() {
                        paras.push(p);
                    }
                }
                b"sp" => {
                    in_sp = false;
                    let inls: Vec<Inline> = paras.iter().flat_map(|p| p.inlines.clone()).collect();
                    if sp_is_title {
                        shapes.push(Shape::Title(inls));
                    } else if !paras.iter().all(|p| p.inlines.is_empty()) {
                        shapes.push(Shape::Body(std::mem::take(&mut paras)));
                    }
                }
                b"tc" if in_tbl => {
                    in_cell = false;
                    cur_row.push(std::mem::take(&mut cur_cell));
                }
                b"tr" if in_tbl => tbl_rows.push(std::mem::take(&mut cur_row)),
                b"tbl" => {
                    in_tbl = false;
                    let mut rows = std::mem::take(&mut tbl_rows);
                    let header = if rows.is_empty() {
                        Vec::new()
                    } else {
                        rows.remove(0)
                    };
                    shapes.push(Shape::Table(kasane_ir::Table {
                        header,
                        rows,
                        has_merged,
                    }));
                    has_merged = false;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    shapes
}

// Map a body shape's paragraphs to blocks. Extended in Task 5 to build nested lists.
fn body_to_blocks(paras: Vec<Paragraph>, out: &mut Vec<Block>) {
    let non_empty: Vec<Paragraph> = paras
        .into_iter()
        .filter(|p| !p.inlines.is_empty())
        .collect();
    if non_empty.is_empty() {
        return;
    }
    if non_empty.len() == 1 && non_empty[0].level == 0 {
        out.push(Block::Para(non_empty.into_iter().next().unwrap().inlines));
        return;
    }
    out.push(build_list(&non_empty, 0, &mut 0));
}

// Build a bulleted List for paragraphs at `depth`, consuming from index `*i`.
// A paragraph deeper than `depth` becomes a nested List under the previous item.
fn build_list(paras: &[Paragraph], depth: u8, i: &mut usize) -> Block {
    let mut items: Vec<Vec<Block>> = Vec::new();
    while *i < paras.len() {
        let lvl = paras[*i].level;
        if lvl < depth {
            break; // belongs to an ancestor list
        }
        if lvl == depth {
            items.push(vec![Block::Para(paras[*i].inlines.clone())]);
            *i += 1;
        } else {
            // deeper: nest under the most recent item at this depth
            let nested = build_list(paras, depth + 1, i);
            if let Some(last) = items.last_mut() {
                last.push(nested);
            } else {
                // no parent item (malformed jump in levels): promote to this depth
                items.push(vec![nested]);
            }
        }
    }
    Block::List {
        ordered: false,
        items,
    }
}

pub fn slide_to_blocks(xml: &str, next_id: &mut u32, rels: &SlideRels) -> Vec<Block> {
    let shapes = parse_shapes(xml, rels);
    let mut out = Vec::new();

    // Heading first: the title shape's text, or a "Slide N"-style fallback. The
    // caller (Task 8) sets a real "Slide N" title when no Title shape is present;
    // here we emit an empty heading the caller can fill, keeping ids monotonic.
    let title_inls = shapes.iter().find_map(|s| match s {
        Shape::Title(t) if !t.is_empty() => Some(t.clone()),
        _ => None,
    });
    let id = BlockId(*next_id);
    *next_id += 1;
    out.push(Block::Heading {
        level: 1,
        id,
        inlines: title_inls.unwrap_or_default(),
    });

    for s in shapes {
        match s {
            Shape::Title(_) => {}
            Shape::Body(paras) => body_to_blocks(paras, &mut out),
            Shape::Table(t) => out.push(Block::Table(t)),
        }
    }
    out
}

pub fn notes_to_blocks(xml: &str) -> Vec<Block> {
    let mut out = Vec::new();
    for s in parse_shapes(xml, &SlideRels::empty()) {
        if let Shape::Body(paras) = s {
            body_to_blocks(paras, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pptx::rels::SlideRels;
    use kasane_ir::{Block, Inline};

    fn text_of(inls: &[Inline]) -> String {
        inls.iter()
            .map(|i| match i {
                Inline::Text(t) => t.clone(),
                Inline::Strong(x) | Inline::Emph(x) => text_of(x),
                _ => String::new(),
            })
            .collect()
    }

    const SLIDE: &str = r#"<p:sld xmlns:a="a" xmlns:p="p">
      <p:cSld><p:spTree>
        <p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><a:r><a:t>The Title</a:t></a:r></a:p></p:txBody></p:sp>
        <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p>
            <a:r><a:t>plain </a:t></a:r>
            <a:r><a:rPr b="1"/><a:t>bold</a:t></a:r>
          </a:p></p:txBody></p:sp>
      </p:spTree></p:cSld></p:sld>"#;

    #[test]
    fn title_becomes_h1_and_runs_carry_bold() {
        let mut id = 0u32;
        let blocks = slide_to_blocks(SLIDE, &mut id, &SlideRels::empty());
        // first block is the H1 title
        match &blocks[0] {
            Block::Heading { level, inlines, .. } => {
                assert_eq!(*level, 1);
                assert_eq!(text_of(inlines), "The Title");
            }
            _ => panic!("expected heading"),
        }
        // the body paragraph with a bold run
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(text_of(para), "plain bold");
        assert!(para.iter().any(|i| matches!(i, Inline::Strong(_))));
    }

    #[test]
    fn missing_title_falls_back_to_slide_n_via_caller() {
        // A slide with no title placeholder yields no Title shape; slide_to_blocks
        // still returns a heading built by the fallback path.
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><a:r><a:t>body only</a:t></a:r></a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        assert!(matches!(&blocks[0], Block::Heading { level: 1, .. }));
    }

    #[test]
    fn body_with_levels_becomes_nested_list() {
        use kasane_ir::Block;
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody>
            <a:p><a:r><a:t>A</a:t></a:r></a:p>
            <a:p><a:pPr lvl="1"/><a:r><a:t>A1</a:t></a:r></a:p>
            <a:p><a:r><a:t>B</a:t></a:r></a:p>
          </p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        let list = blocks
            .iter()
            .find_map(|b| match b {
                Block::List { items, .. } => Some(items),
                _ => None,
            })
            .expect("a list");
        assert_eq!(list.len(), 2); // top-level items A and B
                                   // A's item contains a nested List holding A1
        let a_has_nested = list[0].iter().any(|b| matches!(b, Block::List { .. }));
        assert!(a_has_nested, "A1 should nest under A");
    }

    #[test]
    fn lone_paragraph_stays_para() {
        use kasane_ir::Block;
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><a:r><a:t>solo</a:t></a:r></a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        assert!(blocks.iter().any(|b| matches!(b, Block::Para(_))));
        assert!(!blocks.iter().any(|b| matches!(b, Block::List { .. })));
    }

    #[test]
    fn graphic_frame_table_becomes_table_block() {
        use kasane_ir::Block;
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:graphicFrame><a:graphic><a:graphicData><a:tbl>
            <a:tr>
              <a:tc><a:txBody><a:p><a:r><a:t>H1</a:t></a:r></a:p></a:txBody></a:tc>
              <a:tc><a:txBody><a:p><a:r><a:t>H2</a:t></a:r></a:p></a:txBody></a:tc>
            </a:tr>
            <a:tr>
              <a:tc><a:txBody><a:p><a:r><a:t>a</a:t></a:r></a:p></a:txBody></a:tc>
              <a:tc><a:txBody><a:p><a:r><a:t>b</a:t></a:r></a:p></a:txBody></a:tc>
            </a:tr>
          </a:tbl></a:graphicData></a:graphic></p:graphicFrame>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &crate::pptx::rels::SlideRels::empty());
        let t = blocks
            .iter()
            .find_map(|b| match b {
                Block::Table(t) => Some(t),
                _ => None,
            })
            .expect("a table");
        assert_eq!(t.header.len(), 2);
        assert_eq!(t.rows.len(), 1);
        assert!(!t.has_merged);
    }
}
