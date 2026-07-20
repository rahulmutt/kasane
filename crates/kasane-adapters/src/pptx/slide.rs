use crate::pptx::rels::{attr_local, unescape_attr, RelTarget, SlideRels};
use kasane_ir::{AssetRef, Block, BlockId, Inline, RefTarget};
use quick_xml::events::Event;
use quick_xml::Reader;

pub(crate) struct Paragraph {
    // `build_list` recurses once per distinct level as it nests deeper
    // paragraphs under their ancestors, so this type's width is a hard,
    // untrusted-input-facing bound on recursion depth (max 256): it caps how
    // deep an adversarial bullet-level jump can drive the call stack.
    pub level: u8,
    pub inlines: Vec<Inline>,
}

pub(crate) enum Shape {
    Title(Vec<Inline>),
    Body(Vec<Paragraph>),
    Table(kasane_ir::Table),
    Picture { key: String, alt: String },
}

// Run-formatting state carried while inside <a:r>.
#[derive(Default)]
struct RunFmt {
    bold: bool,
    italic: bool,
    link: Option<String>,
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
    match &fmt.link {
        Some(url) => Inline::Link {
            target: RefTarget::External(url.clone()),
            inlines: vec![inl],
        },
        None => inl,
    }
}

/// Parses `<p:sld>`/`<p:notes>` body XML into shapes. Returns the shapes
/// accumulated so far together with a `truncated` flag: `true` when the XML
/// was malformed and the reader bailed out mid-parse (as opposed to a clean
/// EOF), so callers can surface that the slide's content may be incomplete
/// instead of silently dropping the tail.
pub(crate) fn parse_shapes(xml: &str, rels: &SlideRels) -> (Vec<Shape>, bool) {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    // A bare `&` in an <a:t> run would raise IllFormedError under quick-xml
    // 0.41 (0.36 passed it through), and this loop treats a reader error as
    // truncation -- abandoning every later shape on the slide. Recover the
    // `&` as literal text instead.
    reader.config_mut().allow_dangling_amp = true;
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
    let mut in_pic = false;
    let mut pic_alt = String::new();
    let mut pic_key: Option<String> = None;

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
                b"pic" => {
                    in_pic = true;
                    pic_alt = String::new();
                    pic_key = None;
                }
                b"cNvPr" if in_pic => {
                    pic_alt = attr_str(&e, b"descr").unwrap_or_default();
                }
                b"blip" if in_pic => {
                    if let Some(rid) = attr_local(&e, b"embed") {
                        if let Some(RelTarget::Internal(p)) = rels.get(&rid) {
                            pic_key = Some(p.clone());
                        }
                    }
                }
                b"hlinkClick" if in_run => {
                    if let Some(rid) = attr_local(&e, b"id") {
                        if let Some(RelTarget::External(u)) = rels.get(&rid) {
                            fmt.link = Some(u.clone());
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Text(t)) if in_run => {
                // No unescape() here: the reader splits text at every reference,
                // so an Event::Text can never contain a `&...;`. With
                // allow_dangling_amp it would also turn a recovered `& Jerry`
                // into "" via Err(UnterminatedEntity).
                let s = t.decode().map(|d| d.into_owned()).unwrap_or_default();
                if !s.is_empty() {
                    if in_cell {
                        crate::xmltext::push_inline(&mut cur_cell, styled(s, &fmt));
                    } else if let Some(p) = cur_para.as_mut() {
                        crate::xmltext::push_inline(&mut p.inlines, styled(s, &fmt));
                    }
                }
            }
            // quick-xml 0.41 emits entity/character references in text content as
            // their own event instead of folding them into Event::Text. Same
            // in_run guard, same styling and destination as Event::Text above.
            Ok(Event::GeneralRef(r)) if in_run => {
                let s = crate::xmltext::resolve_general_ref(&r);
                if !s.is_empty() {
                    if in_cell {
                        crate::xmltext::push_inline(&mut cur_cell, styled(s, &fmt));
                    } else if let Some(p) = cur_para.as_mut() {
                        crate::xmltext::push_inline(&mut p.inlines, styled(s, &fmt));
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
                b"pic" => {
                    in_pic = false;
                    if let Some(key) = pic_key.take() {
                        shapes.push(Shape::Picture {
                            key,
                            alt: std::mem::take(&mut pic_alt),
                        });
                    }
                }
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
            Err(_) => return (shapes, true),
            _ => {}
        }
        buf.clear();
    }
    (shapes, false)
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
    let (shapes, truncated) = parse_shapes(xml, rels);
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
            Shape::Picture { key, alt } => out.push(Block::Figure {
                image: AssetRef { key, bytes_ref: 0 },
                caption: if alt.is_empty() {
                    Vec::new()
                } else {
                    vec![Inline::Text(alt)]
                },
                number: None,
            }),
        }
    }
    if truncated {
        out.push(Block::Raw {
            note: "slide truncated: malformed XML".into(),
        });
    }
    out
}

pub fn notes_to_blocks(xml: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let (shapes, truncated) = parse_shapes(xml, &SlideRels::empty());
    for s in shapes {
        if let Shape::Body(paras) = s {
            body_to_blocks(paras, &mut out);
        }
    }
    if truncated {
        out.push(Block::Raw {
            note: "notes truncated: malformed XML".into(),
        });
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
    fn unescapes_run_text_entities() {
        // `x &amp; y` puts the reference between two Text fragments, so under
        // quick-xml 0.41 -- which splits text at every reference -- this
        // exercises resolve_general_ref's unescape() call. Event::Text can
        // never contain a `&...;` once the reader splits on it, so that arm
        // only decodes and deliberately does not unescape. Existing tests cover
        // plain runs, never the reference-resolution half.
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
            <p:txBody><a:p><a:r><a:t>x &amp; y</a:t></a:r></a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(text_of(para), "x & y");
    }

    #[test]
    fn bare_ampersand_in_run_does_not_truncate_later_shapes() {
        // A dangling `&` raised IllFormedError under quick-xml 0.41, which this
        // loop treats as truncation -- dropping every later shape on the slide.
        // The regression is the SECOND shape, not just the `&`.
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
            <p:txBody><a:p><a:r><a:t>Tom & Jerry</a:t></a:r></a:p></p:txBody></p:sp>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
            <p:txBody><a:p><a:r><a:t>SECOND</a:t></a:r></a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        let paras: Vec<String> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Para(inls) => Some(text_of(inls)),
                _ => None,
            })
            .collect();
        assert_eq!(paras, vec!["Tom & Jerry".to_string(), "SECOND".to_string()]);
    }

    #[test]
    fn resolves_numeric_and_boundary_references_in_runs_without_fragmenting() {
        // Numeric character references and references at the very start/end of
        // a run: quick-xml 0.41 emits each as its own GeneralRef event, so a
        // leading one arrives with no preceding Text and a trailing one with
        // no following Text.
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
            <p:txBody><a:p><a:r><a:t>&amp;caf&#233;&#xE9;&gt;</a:t></a:r></a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(text_of(para), "&caféé>");
        // The four fragments coalesce back into the single text node 0.36 built.
        assert_eq!(para.len(), 1);
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
    fn hyperlink_run_and_picture_resolve_via_rels() {
        use crate::pptx::rels::{RelTarget as RT, SlideRels};
        use kasane_ir::{Block, Inline, RefTarget};
        use std::collections::HashMap;

        let mut m = HashMap::new();
        m.insert(
            "rId2".to_string(),
            RT::External("https://example.com".into()),
        );
        m.insert(
            "rId3".to_string(),
            RT::Internal("ppt/media/image1.png".into()),
        );
        let rels = SlideRels(m);

        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:r="r"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p>
            <a:r><a:rPr><a:hlinkClick r:id="rId2"/></a:rPr><a:t>link text</a:t></a:r>
          </a:p></p:txBody></p:sp>
          <p:pic><p:nvPicPr><p:cNvPr id="5" name="Pic" descr="a cat"/></p:nvPicPr>
            <p:blipFill><a:blip r:embed="rId3"/></p:blipFill></p:pic>
        </p:spTree></p:cSld></p:sld>"#;

        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &rels);

        // hyperlink
        let has_link = blocks.iter().any(|b| {
            matches!(b, Block::Para(inls)
            if inls.iter().any(|i| matches!(i,
                Inline::Link { target: RefTarget::External(u), .. } if u == "https://example.com")))
        });
        assert!(has_link, "hyperlink run should become an external link");

        // figure
        let fig = blocks
            .iter()
            .find_map(|b| match b {
                Block::Figure { image, caption, .. } => Some((image.key.clone(), caption.clone())),
                _ => None,
            })
            .expect("a figure");
        assert_eq!(fig.0, "ppt/media/image1.png");
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

    #[test]
    fn malformed_xml_mid_body_still_emits_heading_and_raw_note() {
        // A good title followed by a body with a stray, unmatched close tag.
        // The XML reader bails out mid-parse; the heading must still surface
        // and the truncation must be signaled via a Block::Raw, not silently
        // dropped (Fix 2).
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
            <p:txBody><a:p><a:r><a:t>The Title</a:t></a:r></a:p></p:txBody></p:sp>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
            <p:txBody><a:p><a:r><a:t>body text</a:t></a:r></a:p></a:wrong></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());

        match &blocks[0] {
            Block::Heading { level, inlines, .. } => {
                assert_eq!(*level, 1);
                assert_eq!(text_of(inlines), "The Title");
            }
            _ => panic!("expected heading"),
        }
        let has_raw_note = blocks
            .iter()
            .any(|b| matches!(b, Block::Raw { note } if note.contains("truncated")));
        assert!(
            has_raw_note,
            "expected a truncation Block::Raw, got {blocks:?}"
        );
    }

    // ---- Adversarial-input probes (accepted current behavior, not fixes) ----

    #[test]
    fn nested_table_emits_spurious_empty_outer_table() {
        // A <a:tbl> nested inside another <a:tbl> is not valid OOXML, but the
        // flat state-machine parser doesn't guard against it: the inner tbl's
        // Start/End resets and consumes the shared `tbl_rows`/`in_tbl` state,
        // so the outer tbl's own row is lost and its End produces a second,
        // empty Table shape. This is ACCEPTED v1 behavior (no crash, no data
        // corruption beyond the dropped outer row) — this test pins it rather
        // than "fixing" it.
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:graphicFrame><a:graphic><a:graphicData><a:tbl>
            <a:tr><a:tc><a:txBody><a:p><a:r><a:t>outer</a:t></a:r></a:p></a:txBody></a:tc></a:tr>
            <a:tbl>
              <a:tr><a:tc><a:txBody><a:p><a:r><a:t>inner</a:t></a:r></a:p></a:txBody></a:tc></a:tr>
            </a:tbl>
          </a:tbl></a:graphicData></a:graphic></p:graphicFrame>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());

        let tables: Vec<&kasane_ir::Table> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Table(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(
            tables.len(),
            2,
            "expected inner table + spurious empty outer table"
        );
        assert_eq!(text_of(&tables[0].header[0]), "inner");
        assert!(
            tables[1].header.is_empty() && tables[1].rows.is_empty(),
            "outer close should emit an empty table, got {:?}",
            tables[1]
        );
        // No slide-truncation marker: the XML is well-formed, just semantically odd.
        assert!(!blocks
            .iter()
            .any(|b| matches!(b, Block::Raw { note } if note.contains("truncated"))));
    }

    #[test]
    fn deeply_nested_bullet_levels_do_not_overflow_the_stack() {
        // ~300 paragraphs each one level deeper than the last. `Paragraph.level`
        // is a u8, so any "lvl" value that doesn't fit in u8 (>255) fails to
        // parse and falls back to 0, which bounds `build_list`'s recursion
        // depth to at most 256 regardless of how deep an adversarial document
        // claims to nest. This must not stack-overflow.
        let mut body = String::new();
        for lvl in 0..300u32 {
            body.push_str(&format!(
                r#"<a:p><a:pPr lvl="{lvl}"/><a:r><a:t>L{lvl}</a:t></a:r></a:p>"#
            ));
        }
        let xml = format!(
            r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
              <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
                <p:txBody>{body}</p:txBody></p:sp>
            </p:spTree></p:cSld></p:sld>"#
        );
        let mut id = 0u32;
        let blocks = slide_to_blocks(&xml, &mut id, &SlideRels::empty());
        // Reaching this line without a stack overflow is the assertion; also
        // sanity-check some content made it through.
        assert!(blocks
            .iter()
            .any(|b| matches!(b, Block::List { .. } | Block::Para(_))));
    }
}
