use kasane_ir::{Block, BlockId, Inline, RefTarget};
use quick_xml::events::Event;
use quick_xml::Reader;

// Returns blocks; `next_id` is a running BlockId counter for headings.
pub fn xhtml_to_blocks(xml: &str, next_id: &mut u32) -> Vec<Block> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut blocks = vec![];
    let mut buf = Vec::new();
    // inline accumulation stack
    let mut inline_stack: Vec<Vec<Inline>> = vec![];
    let mut cur_block: Option<u8> = None; // heading level, or 0 for para
    let mut link_href: Option<String> = None;

    macro_rules! push_text {
        ($t:expr) => {
            if let Some(top) = inline_stack.last_mut() {
                crate::xmltext::push_inline(top, Inline::Text($t));
            }
        };
    }

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6" => {
                    cur_block = Some(e.local_name().as_ref()[1] - b'0');
                    inline_stack.push(vec![]);
                }
                b"p" => {
                    cur_block = Some(0);
                    inline_stack.push(vec![]);
                }
                b"strong" | b"b" => inline_stack.push(vec![]),
                b"em" | b"i" => inline_stack.push(vec![]),
                b"a" => {
                    link_href = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == b"href")
                        .map(|a| String::from_utf8_lossy(&a.value).into_owned());
                    inline_stack.push(vec![]);
                }
                _ => {}
            },
            Ok(Event::Text(t)) => {
                let s = t
                    .decode()
                    .ok()
                    .and_then(|d| quick_xml::escape::unescape(&d).ok().map(|s| s.into_owned()))
                    .unwrap_or_default();
                if !s.trim().is_empty() && !inline_stack.is_empty() {
                    push_text!(s);
                }
            }
            // quick-xml 0.41 emits entity/character references in text content as
            // their own event instead of folding them into Event::Text.
            Ok(Event::GeneralRef(r)) => {
                let s = crate::xmltext::resolve_general_ref(&r);
                // No whitespace guard here, unlike Event::Text. That guard drops
                // the indentation between tags, which is markup, not content. A
                // reference is always authored deliberately, so `&#160;` or
                // `&#32;` is content and must survive.
                if !s.is_empty() && !inline_stack.is_empty() {
                    push_text!(s);
                }
            }
            Ok(Event::End(e)) => {
                match e.local_name().as_ref() {
                    b"strong" | b"b" => {
                        let x = inline_stack.pop().unwrap_or_default();
                        if let Some(top) = inline_stack.last_mut() {
                            top.push(Inline::Strong(x));
                        }
                    }
                    b"em" | b"i" => {
                        let x = inline_stack.pop().unwrap_or_default();
                        if let Some(top) = inline_stack.last_mut() {
                            top.push(Inline::Emph(x));
                        }
                    }
                    b"a" => {
                        let x = inline_stack.pop().unwrap_or_default();
                        let target = match link_href.take() {
                            // EPUB internal links (both same-file `#frag` and cross-file
                            // `file.xhtml#frag` forms) currently pass through unresolved as
                            // `External`. Mapping them to `RefTarget::Internal(BlockId)` is
                            // deferred to Plan 2's XHTML-fidelity task.
                            Some(h) => RefTarget::External(h),
                            None => RefTarget::External(String::new()),
                        };
                        if let Some(top) = inline_stack.last_mut() {
                            top.push(Inline::Link { target, inlines: x });
                        }
                    }
                    b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6" => {
                        let inls = inline_stack.pop().unwrap_or_default();
                        let level = cur_block.take().unwrap_or(1);
                        let id = BlockId(*next_id);
                        *next_id += 1;
                        blocks.push(Block::Heading {
                            level,
                            id,
                            inlines: inls,
                        });
                    }
                    b"p" => {
                        let inls = inline_stack.pop().unwrap_or_default();
                        cur_block = None;
                        if !inls.is_empty() {
                            blocks.push(Block::Para(inls));
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(inls: &[Inline]) -> String {
        inls.iter()
            .map(|i| match i {
                Inline::Text(t) => t.clone(),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn unescapes_paragraph_text_entities() {
        // Exercises the decode() + quick_xml::escape::unescape chain that
        // replaced t.unescape() in the 0.41 migration: an entity in <p> text
        // must come out decoded, not literal.
        let xml = "<p>a &lt; b</p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(text_of(para), "a < b");
    }

    #[test]
    fn resolves_numeric_and_boundary_references_without_fragmenting() {
        // A reference at the leading and trailing edge of the text, plus decimal
        // and hex character references. Under quick-xml 0.41 the leading `&lt;`
        // is the paragraph's first event, arriving before any Event::Text.
        let xml = "<p>&lt;caf&#233;&#xE9;&gt;</p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(text_of(para), "<caféé>");
        // The four fragments coalesce back into the single text node 0.36 built.
        assert_eq!(para.len(), 1);
    }

    #[test]
    fn keeps_unresolvable_entity_as_source_text() {
        // &nbsp; has no XML predefined mapping. Preserving the reference is
        // lossless; the pre-fix behavior dropped it entirely.
        let xml = "<p>a&nbsp;b</p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(text_of(para), "a&nbsp;b");
    }
}
