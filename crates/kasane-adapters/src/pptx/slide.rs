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
                _ => {}
            },
            Ok(Event::Text(t)) if in_run => {
                let s = t.unescape().unwrap_or_default().to_string();
                if !s.is_empty() {
                    if let Some(p) = cur_para.as_mut() {
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
    for p in paras {
        if !p.inlines.is_empty() {
            out.push(Block::Para(p.inlines));
        }
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
}
