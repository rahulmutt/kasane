use crate::pptx::rels::{attr_local, unescape_attr, RelTarget, SlideRels};
use kasane_ir::{AssetRef, Block, BlockId, Inline, RefTarget};
use quick_xml::events::Event;
use quick_xml::Reader;

/// Fidelity bound for PPTX bullet nesting — the analogue of
/// `epub::xhtml::MAX_BLOCK_DEPTH`. `build_list` recurses once per level step,
/// so this is what keeps PPTX IR under `kasane_ir::MAX_BLOCK_DEPTH`.
/// 0-based, so 31 means 32 nesting levels.
///
/// Past it, a deeper bullet becomes a sibling at this level instead of
/// nesting further — the same flatten-not-truncate contract the EPUB parser
/// keeps, so no text is lost.
pub(crate) const MAX_LIST_LEVEL: u8 = 31;

const _: () = assert!(
    (MAX_LIST_LEVEL as usize + 1) * 4 <= kasane_ir::MAX_BLOCK_DEPTH,
    "pptx::slide::MAX_LIST_LEVEL must stay at most a quarter of kasane_ir::MAX_BLOCK_DEPTH"
);

pub(crate) struct Paragraph {
    // `build_list` recurses once per distinct level as it nests deeper
    // paragraphs under their ancestors, so this type's width would otherwise
    // be a hard, untrusted-input-facing bound on recursion depth (up to 256,
    // since `level` is a `u8`). Both parse sites below clamp to
    // `MAX_LIST_LEVEL`, so the real bound on `build_list`'s recursion depth
    // is `MAX_LIST_LEVEL + 1` (32), not 256.
    pub level: u8,
    pub inlines: Vec<Inline>,
}

pub(crate) enum Shape {
    Title(Vec<Inline>),
    Body(Vec<Paragraph>),
    Table(kasane_ir::Table),
    Picture {
        key: String,
        alt: String,
    },
    Math {
        latex: String,
        complete: bool,
    },
    /// A document-level malformation note, emitted at the point in the shape
    /// order where an equation island failed to capture.
    Note(&'static str),
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
    // Display equations (<m:oMathPara>) for the current shape, flushed as
    // MathBlock siblings when the shape closes.
    let mut display_math: Vec<(String, bool)> = Vec::new();

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
                        level = l.parse().unwrap_or(0).min(MAX_LIST_LEVEL);
                    }
                    cur_para = Some(Paragraph {
                        level,
                        inlines: Vec::new(),
                    });
                }
                b"pPr" => {
                    if let (Some(p), Some(l)) = (cur_para.as_mut(), attr_str(&e, b"lvl")) {
                        p.level = l.parse().unwrap_or(0).min(MAX_LIST_LEVEL);
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
                // OMML lives inside <a:p> but outside <a:r>. <m:oMathPara>
                // wraps its own <m:oMath>; capture_island consumes the whole
                // island (including that inner oMath), so the inner oMath
                // Start never reaches this loop and only one arm fires per
                // equation.
                b"oMathPara" => {
                    let conv = match crate::math::capture_island(&mut reader, &e) {
                        Ok(island) => crate::math::omml_to_latex(&island),
                        Err(err) => {
                            // capture_island rewound the reader, so this
                            // island's children (including its inner
                            // <m:oMath>) are handed back to this loop as
                            // ordinary content instead of vanishing. Record
                            // the malformation where it happened.
                            shapes.push(Shape::Note(err.note()));
                            crate::math::degraded()
                        }
                    };
                    // Tables are parsed via a sibling p:graphicFrame, not
                    // nested inside p:sp, so a display equation captured
                    // while in_cell has no p:sp End to flush display_math
                    // against: it would be dropped entirely (if no p:sp
                    // follows) or drained into an unrelated shape (if one
                    // does). Fold it to inline instead, mirroring the oMath
                    // arm below and the EPUB side's identical fold. The
                    // `complete` flag is deliberately discarded here: a
                    // folded display equation is inline, and per the plan a
                    // folded/inline partial self-marks via the in-band
                    // `\mathord{?}` token only -- no "equation partially
                    // converted" note is emitted for it.
                    if in_cell {
                        crate::xmltext::push_inline(&mut cur_cell, Inline::Math(conv.latex));
                    } else {
                        display_math.push((conv.latex, conv.complete));
                    }
                }
                b"oMath" => {
                    let conv = match crate::math::capture_island(&mut reader, &e) {
                        Ok(island) => crate::math::omml_to_latex(&island),
                        Err(err) => {
                            shapes.push(Shape::Note(err.note()));
                            crate::math::degraded()
                        }
                    };
                    // Mirrors the in_cell/cur_para destination split used for
                    // run text above: a table cell paragraph (in_tbl/in_cell)
                    // never sets cur_para (the `b"p"` arm above is gated on
                    // `in_sp`, and tables are not nested inside shapes), so
                    // without this branch inline math inside a table cell
                    // would vanish silently instead of degrading visibly.
                    if in_cell {
                        crate::xmltext::push_inline(&mut cur_cell, Inline::Math(conv.latex));
                    } else if let Some(p) = cur_para.as_mut() {
                        crate::xmltext::push_inline(&mut p.inlines, Inline::Math(conv.latex));
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
                    for (latex, complete) in std::mem::take(&mut display_math) {
                        shapes.push(Shape::Math { latex, complete });
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
            Err(_) => {
                // Same leftover-queue hazard as the Eof case below, reached
                // via a different exit: an oMathPara captured before the XML
                // goes malformed has no p:sp End left to flush against, so
                // without this it would be silently dropped alongside the
                // truncation. See flush_leftover_display_math_after_truncation.
                flush_display_math(&mut shapes, &mut display_math);
                return (shapes, true);
            }
            _ => {}
        }
        buf.clear();
    }
    // Normally every oMathPara is drained by its enclosing p:sp End. This is
    // a safety net for schema-invalid-but-well-formed XML where an oMathPara
    // sits outside any p:sp/table cell (e.g. a stray direct child of
    // spTree): such input reaches Eof with no p:sp End ever having fired to
    // drain it. See flush_leftover_display_math_when_never_inside_a_shape.
    flush_display_math(&mut shapes, &mut display_math);
    (shapes, false)
}

// Drains any display-math entries that never reached a p:sp End into the
// shape list directly, so a leftover queue at parse_shapes's return (Eof or
// truncation) doesn't silently lose an equation.
fn flush_display_math(shapes: &mut Vec<Shape>, display_math: &mut Vec<(String, bool)>) {
    for (latex, complete) in std::mem::take(display_math) {
        shapes.push(Shape::Math { latex, complete });
    }
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
            Shape::Math { latex, complete } => {
                out.push(Block::MathBlock(latex));
                if !complete {
                    out.push(Block::Raw {
                        note: "equation partially converted".into(),
                    });
                }
            }
            Shape::Note(note) => out.push(Block::Raw { note: note.into() }),
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
        match s {
            Shape::Title(_) => {}
            Shape::Body(paras) => body_to_blocks(paras, &mut out),
            Shape::Table(_) => {}
            Shape::Picture { .. } => {}
            Shape::Math { latex, complete } => {
                out.push(Block::MathBlock(latex));
                if !complete {
                    out.push(Block::Raw {
                        note: "equation partially converted".into(),
                    });
                }
            }
            Shape::Note(note) => out.push(Block::Raw { note: note.into() }),
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
    fn inline_omath_appends_math_inline_to_paragraph() {
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:m="m"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p>
            <a:r><a:t>value </a:t></a:r>
            <m:oMath><m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e>
              <m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath>
          </a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(i) => Some(i),
                _ => None,
            })
            .expect("a paragraph");
        assert!(para
            .iter()
            .any(|i| matches!(i, Inline::Math(s) if s == "{x}^{2}")));
    }

    #[test]
    fn inline_omath_inside_table_cell_reaches_cell_inline_content() {
        // Regression for the table-cell inline-math fix (the b"oMath" arm's
        // `if in_cell` branch): tables are parsed via a sibling p:graphicFrame,
        // never nested inside a p:sp, so this exercises the in_cell/cur_cell
        // destination directly rather than the in_sp/cur_para one already
        // covered by inline_omath_appends_math_inline_to_paragraph.
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:m="m"><p:cSld><p:spTree>
          <p:graphicFrame><a:graphic><a:graphicData><a:tbl>
            <a:tr><a:tc><a:txBody><a:p>
              <a:r><a:t>value </a:t></a:r>
              <m:oMath><m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e>
                <m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath>
            </a:p></a:txBody></a:tc></a:tr>
          </a:tbl></a:graphicData></a:graphic></p:graphicFrame>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        let t = blocks
            .iter()
            .find_map(|b| match b {
                Block::Table(t) => Some(t),
                _ => None,
            })
            .expect("a table");
        let cell = &t.header[0];
        assert!(
            cell.iter()
                .any(|i| matches!(i, Inline::Math(s) if s == "{x}^{2}")),
            "expected the inline equation in the cell, got {cell:?}"
        );
    }

    #[test]
    fn display_omathpara_inside_table_cell_folds_to_inline_without_note() {
        // Regression for Fix 1: a display oMathPara captured while in_cell
        // has no p:sp End to flush display_math against, so it must fold to
        // an inline Inline::Math in the cell instead of being lost or
        // misattributed to an unrelated shape. Uses <m:acc>, which is not in
        // omml::convert's match arms, so the equation degrades to the
        // placeholder token and `complete` is false -- this pins that the
        // "equation partially converted" note is deliberately NOT emitted
        // for a folded display equation (the plan's inline-partial rule is
        // the in-band token only).
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:m="m"><p:cSld><p:spTree>
          <p:graphicFrame><a:graphic><a:graphicData><a:tbl>
            <a:tr><a:tc><a:txBody><a:p>
              <m:oMathPara><m:oMath><m:acc><m:e><m:r><m:t>x</m:t></m:r></m:e></m:acc></m:oMath></m:oMathPara>
            </a:p></a:txBody></a:tc></a:tr>
          </a:tbl></a:graphicData></a:graphic></p:graphicFrame>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        let t = blocks
            .iter()
            .find_map(|b| match b {
                Block::Table(t) => Some(t),
                _ => None,
            })
            .expect("a table");
        let cell = &t.header[0];
        assert!(
            cell.iter()
                .any(|i| matches!(i, Inline::Math(s) if s.contains("\\mathord{?}"))),
            "expected the folded equation's placeholder token in the cell, got {cell:?}"
        );
        assert!(
            !blocks
                .iter()
                .any(|b| matches!(b, Block::Raw { note } if note.contains("partially converted"))),
            "the partial-conversion note must not appear anywhere when folded, got {blocks:?}"
        );
    }

    #[test]
    fn flush_leftover_display_math_after_truncation() {
        // A well-formed oMathPara followed by malformed XML before the
        // enclosing p:sp closes. The island itself closes cleanly, so it is
        // captured fine; the outer loop then hits the malformed tail and
        // returns Err(_), bypassing the
        // p:sp End flush entirely (Fix 3). Without the end-of-parse safety
        // net this equation would vanish alongside the truncation.
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:m="m"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p>
            <m:oMathPara><m:oMath><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num>
              <m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath></m:oMathPara>
          </a:p></a:wrong></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::MathBlock(s) if s == "\\frac{1}{2}")),
            "the display equation must survive truncation, got {blocks:?}"
        );
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::Raw { note } if note.contains("truncated"))),
            "truncation must still be signaled, got {blocks:?}"
        );
    }

    #[test]
    fn flush_leftover_display_math_when_never_inside_a_shape() {
        // Schema-invalid but well-formed XML: an oMathPara as a direct
        // child of spTree, never inside any p:sp or table cell. quick-xml
        // only checks tag matching, not OOXML schema, so this reaches Eof
        // cleanly with display_math still populated and no p:sp End ever
        // having fired to drain it (Fix 3).
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:m="m"><p:cSld><p:spTree>
            <m:oMathPara><m:oMath><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num>
              <m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath></m:oMathPara>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::MathBlock(s) if s == "\\frac{1}{2}")),
            "the display equation must survive an oMathPara outside any shape, got {blocks:?}"
        );
    }

    #[test]
    fn over_deep_omath_degrades_and_notes_without_aborting() {
        // Whole-adapter cover for the stack-overflow abort. capture_island
        // refuses the island on its nesting bound before roxmltree can see it,
        // rewinds the reader, and records the malformation; the <m:e> nest is
        // then re-read as flow content (no arm matches it) and the run after
        // the equation still reaches the slide.
        let levels = 18_000;
        let xml = format!(
            r#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:m="m"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p>
            <m:oMath>{}{}</m:oMath>
            <a:r><a:t>AFTER</a:t></a:r>
          </a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#,
            "<m:e>".repeat(levels),
            "</m:e>".repeat(levels)
        );
        let mut id = 0u32;
        let blocks = slide_to_blocks(&xml, &mut id, &SlideRels::empty());
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::Raw { note } if note.contains("too large"))),
            "the over-budget island must be noted, got {blocks:?}"
        );
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::Para(i) if text_of(i).contains("AFTER"))),
            "the run after the over-deep island must survive, got {blocks:?}"
        );
    }

    #[test]
    fn unclosed_omath_notes_the_malformation_instead_of_swallowing_the_shape() {
        // An <m:oMath> that never closes used to make capture_island consume
        // the rest of the part with no trace. It now fails, rewinds, and is
        // recorded. The shape *containing* the unclosed island is still lost
        // to the pre-existing slide-truncation path (shapes flush at </p:sp>,
        // and an unclosed tag makes the reader bail before that) -- but that
        // loss is now announced twice over rather than silent, and earlier
        // shapes are unaffected.
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:m="m"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><a:r><a:t>BEFORE</a:t></a:r></a:p></p:txBody></p:sp>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><m:oMath><m:r><m:t>x</m:t></m:r></a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::Raw { note } if note.contains("equation markup"))),
            "the malformation must be noted, not silent, got {blocks:?}"
        );
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::Para(i) if text_of(i).contains("BEFORE"))),
            "content before the unclosed island must survive, got {blocks:?}"
        );
    }

    #[test]
    fn omathpara_becomes_math_block() {
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:m="m"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p>
            <m:oMathPara><m:oMath><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num>
              <m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath></m:oMathPara>
          </a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        assert!(blocks
            .iter()
            .any(|b| matches!(b, Block::MathBlock(s) if s == "\\frac{1}{2}")));
        // capture_island consumes the whole oMathPara island, including its
        // inner <m:oMath>, so that inner Start never reaches the loop and
        // the b"oMath" arm never fires for it. Pin that no stray
        // Inline::Math for this equation leaked into a paragraph, which
        // would indicate a double-fire regression.
        assert!(
            !blocks.iter().any(|b| matches!(b, Block::Para(inls)
                if inls.iter().any(|i| matches!(i, Inline::Math(s) if s == "\\frac{1}{2}"))
            )),
            "the oMath inside oMathPara must not also fire the inline arm, got {blocks:?}"
        );
    }

    #[test]
    fn notes_math_becomes_math_block() {
        // notes_to_blocks's Shape::Math arm was added alongside slide_to_blocks's
        // but had no direct test; notes bodies share parse_shapes with slides,
        // so an oMathPara in a notes p:sp must surface the same way.
        let xml = r#"<p:notes xmlns:a="a" xmlns:p="p" xmlns:m="m"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p>
            <m:oMathPara><m:oMath><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num>
              <m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath></m:oMathPara>
          </a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:notes>"#;
        let blocks = notes_to_blocks(xml);
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::MathBlock(s) if s == "\\frac{1}{2}")),
            "expected a MathBlock in notes output, got {blocks:?}"
        );
    }

    #[test]
    fn deeply_nested_bullet_levels_do_not_overflow_the_stack() {
        // ~300 paragraphs each one level deeper than the last. Both `lvl`
        // parse sites clamp to `MAX_LIST_LEVEL` (31), which bounds
        // `build_list`'s recursion depth to at most 32 regardless of how deep
        // an adversarial document claims to nest -- well under the old
        // (pre-clamp) 256 ceiling that came from `level` merely being a
        // `u8`. This must not stack-overflow.
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

    // Iterative (explicit worklist, not function-call recursion) max block
    // nesting depth over a `&[Block]` tree -- mirrors
    // `fuzz_entry::max_block_depth`'s reasoning: this is exactly the value a
    // hostile/adversarial `lvl` jump could blow past, so a recursive checker
    // could itself overflow on the very input it is meant to be checking.
    fn max_block_depth(blocks: &[Block]) -> usize {
        let mut max_depth = 0;
        let mut stack: Vec<(&Block, usize)> = blocks.iter().map(|b| (b, 0)).collect();
        while let Some((b, depth)) = stack.pop() {
            match b {
                Block::List { items, .. } => {
                    max_depth = max_depth.max(depth + 1);
                    for item in items {
                        stack.extend(item.iter().map(|bb| (bb, depth + 1)));
                    }
                }
                Block::Footnote { blocks, .. } => {
                    max_depth = max_depth.max(depth + 1);
                    stack.extend(blocks.iter().map(|bb| (bb, depth + 1)));
                }
                Block::Heading { .. }
                | Block::Para(_)
                | Block::Table(_)
                | Block::Figure { .. }
                | Block::CodeBlock { .. }
                | Block::MathBlock(_)
                | Block::Raw { .. } => {}
            }
        }
        max_depth
    }

    // Same reasoning as `max_block_depth` above: an explicit worklist rather
    // than recursion, since this walks the same untrusted-shape tree.
    fn contains_text(blocks: &[Block], needle: &str) -> bool {
        fn inline_has(inls: &[Inline], needle: &str) -> bool {
            let mut stack: Vec<&Inline> = inls.iter().collect();
            while let Some(i) = stack.pop() {
                match i {
                    Inline::Text(s) | Inline::Code(s) | Inline::Math(s) => {
                        if s == needle {
                            return true;
                        }
                    }
                    Inline::Emph(v) | Inline::Strong(v) => stack.extend(v.iter()),
                    Inline::Link { inlines, .. } => stack.extend(inlines.iter()),
                    Inline::FootnoteRef(_) => {}
                }
            }
            false
        }
        let mut stack: Vec<&Block> = blocks.iter().collect();
        while let Some(b) = stack.pop() {
            match b {
                Block::Heading { inlines, .. } | Block::Para(inlines) => {
                    if inline_has(inlines, needle) {
                        return true;
                    }
                }
                Block::List { items, .. } => {
                    for item in items {
                        stack.extend(item.iter());
                    }
                }
                Block::Footnote { blocks, .. } => stack.extend(blocks.iter()),
                Block::Table(_)
                | Block::Figure { .. }
                | Block::CodeBlock { .. }
                | Block::MathBlock(_)
                | Block::Raw { .. } => {}
            }
        }
        false
    }

    #[test]
    fn a_pptx_lvl_beyond_max_list_level_flattens_instead_of_truncating() {
        // A slide whose bullets jump from lvl="0" straight to lvl="200" --
        // well past both `u8::MAX` headroom and `MAX_LIST_LEVEL` (31). Before
        // the clamp landed, this produced 201 nested `Block::List`s, past
        // `kasane_ir::MAX_BLOCK_DEPTH` (128), so the core's block walk
        // truncated the tree and silently dropped the deep paragraph's text
        // (finding 1 of the whole-branch review). The clamp must flatten
        // instead: the deep paragraph's text survives, landing as a sibling
        // at level `MAX_LIST_LEVEL`, and the produced nesting never exceeds
        // `MAX_LIST_LEVEL + 1` (32).
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
            <p:txBody>
              <a:p><a:pPr lvl="0"/><a:r><a:t>SHALLOW</a:t></a:r></a:p>
              <a:p><a:pPr lvl="200"/><a:r><a:t>DEEPMARKER</a:t></a:r></a:p>
              <a:p><a:pPr lvl="250"/><a:r><a:t>SIBLING1</a:t></a:r></a:p>
              <a:p><a:pPr lvl="45"/><a:r><a:t>SIBLING2</a:t></a:r></a:p>
            </p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());

        assert!(
            contains_text(&blocks, "DEEPMARKER"),
            "deep paragraph's text must survive flattening, not be truncated: {blocks:?}"
        );
        assert!(
            contains_text(&blocks, "SHALLOW"),
            "shallow paragraph's text must also survive: {blocks:?}"
        );
        assert!(
            contains_text(&blocks, "SIBLING1") && contains_text(&blocks, "SIBLING2"),
            "the other over-deep paragraphs must also flatten to siblings, not be dropped: {blocks:?}"
        );

        let depth = max_block_depth(&blocks);
        assert!(
            depth <= 32,
            "expected block nesting clamped to at most 32 (MAX_LIST_LEVEL + 1), got {depth}"
        );
    }
}
