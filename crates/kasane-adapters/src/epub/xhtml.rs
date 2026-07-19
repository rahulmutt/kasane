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
                top.push(Inline::Text($t));
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
                let s = t.unescape().unwrap_or_default().to_string();
                if !s.trim().is_empty() && !inline_stack.is_empty() {
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
                            Some(h) if h.starts_with('#') => RefTarget::External(h), // in-file; refined later
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
