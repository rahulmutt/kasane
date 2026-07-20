use kasane_ir::{Block, BlockId, Inline, RefTarget};
use quick_xml::events::Event;
use quick_xml::Reader;

// Open block containers. Finished blocks land in the top frame instead of the
// output; closing the container folds the frame into its parent. This is what
// makes nesting (list items holding paragraphs, lists holding lists)
// representable in a single streaming pass.
enum BlockFrame {
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
}

fn emit_block(frames: &mut [BlockFrame], out: &mut Vec<Block>, b: Block) {
    match frames.last_mut() {
        None => out.push(b),
        Some(BlockFrame::List { items, .. }) => {
            // A block arriving before any <li> (malformed) opens an item
            // rather than being dropped.
            if items.is_empty() {
                items.push(Vec::new());
            }
            items.last_mut().expect("non-empty").push(b);
        }
    }
}

fn finish_frame(f: BlockFrame, frames: &mut [BlockFrame], out: &mut Vec<Block>) {
    match f {
        BlockFrame::List { ordered, items } => {
            if !items.is_empty() {
                emit_block(frames, out, Block::List { ordered, items });
            }
        }
    }
}

// Returns blocks; `next_id` is a running BlockId counter for headings.
pub fn xhtml_to_blocks(xml: &str, next_id: &mut u32) -> Vec<Block> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    // Real-world XHTML routinely contains a bare `&` (e.g. `Tom & Jerry`).
    // quick-xml 0.41 raises IllFormedError on a dangling ampersand where 0.36
    // passed it through, and this loop's `Err(_) => break` would abandon the
    // rest of the document -- total silent data loss at exit 0. With this set
    // the `&` is delivered as literal Text and the document survives.
    reader.config_mut().allow_dangling_amp = true;
    let mut blocks = vec![];
    let mut buf = Vec::new();
    // inline accumulation stack
    let mut inline_stack: Vec<Vec<Inline>> = vec![];
    let mut frames: Vec<BlockFrame> = vec![];
    let mut cur_block: Option<u8> = None; // heading level, or 0 for para
    let mut link_href: Option<String> = None;
    // A whitespace-only Text fragment is ambiguous until we see what comes
    // next: quick-xml 0.41 splits text at every reference, so `a &lt; &gt; b`
    // puts a lone `" "` fragment between the two GeneralRef events. That
    // space is real content and must survive, but the same-looking `" "`
    // (or `"\n  "`) between two tags in pretty-printed XHTML is formatting
    // and must still be dropped. `pending_ws` holds the undecided fragment;
    // it is kept if a GeneralRef precedes or follows it, and discarded at
    // any tag boundary. `prev_was_ref` records the immediately-preceding
    // case. When kept, the fragment is normalized to a single `" "` rather
    // than pushed verbatim: XHTML collapses whitespace runs anyway, so a
    // pretty-printed `"\n  "` adjacent to a reference must render the same
    // as a literal `" "` would.
    let mut pending_ws: Option<String> = None;
    let mut prev_was_ref = false;

    macro_rules! push_text {
        ($t:expr) => {
            if let Some(top) = inline_stack.last_mut() {
                crate::xmltext::push_inline(top, Inline::Text($t));
            }
        };
    }

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                // A tag boundary resolves any undecided whitespace fragment
                // as formatting, not reference-adjacent content.
                pending_ws = None;
                prev_was_ref = false;
                match e.local_name().as_ref() {
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
                    b"ul" | b"ol" => {
                        frames.push(BlockFrame::List {
                            ordered: e.local_name().as_ref() == b"ol",
                            items: vec![],
                        });
                    }
                    b"li" => {
                        if let Some(BlockFrame::List { items, .. }) = frames.last_mut() {
                            items.push(Vec::new());
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                // No unescape() here: the reader splits text at every reference,
                // so an Event::Text can never contain a `&...;`. Worse, with
                // allow_dangling_amp a recovered fragment like `& Jerry` makes
                // unescape() return Err(UnterminatedEntity), which the
                // unwrap_or_default() would turn into "" -- silently deleting
                // the text run we just rescued.
                let s = t.decode().map(|d| d.into_owned()).unwrap_or_default();
                if s.trim().is_empty() {
                    if prev_was_ref {
                        // Adjacent to the reference that just preceded it:
                        // real content, not inter-tag formatting. Normalized
                        // to a single space -- the run's whitespace content,
                        // not its literal pretty-printed layout.
                        if !inline_stack.is_empty() {
                            push_text!(" ".to_string());
                        }
                    } else {
                        // Undecided: keep it only if a GeneralRef follows.
                        pending_ws = Some(s);
                    }
                } else {
                    pending_ws = None;
                    if !inline_stack.is_empty() {
                        push_text!(s);
                    }
                }
                prev_was_ref = false;
            }
            // quick-xml 0.41 emits entity/character references in text content as
            // their own event instead of folding them into Event::Text.
            Ok(Event::GeneralRef(r)) => {
                // A pending whitespace-only Text fragment sitting right before
                // this reference is reference-adjacent content; flush it.
                if pending_ws.take().is_some() {
                    // Normalized to a single space, same as the prev_was_ref
                    // keep above -- see the comment on `pending_ws` at the
                    // top of this function. Additionally suppress only at
                    // the start of the *block* frame (depth 1: `p`/`h1..h6`
                    // push it and nothing else does -- `strong`/`em`/`a`
                    // push nested inline frames deeper than that). Per HTML
                    // whitespace processing, leading whitespace at the start
                    // of a block is stripped (e.g. `<p>\n  &amp;X` ->
                    // "&X"), but leading whitespace at the start of an
                    // *inline* element is real content and must survive
                    // (`A<em> &amp;B</em>` -> `A` then a space then
                    // emphasized `&B`). Gating on frame depth, not just
                    // "was this frame just pushed empty", is what tells the
                    // two cases apart.
                    let suppress = inline_stack.len() == 1
                        && inline_stack.last().is_some_and(|top| top.is_empty());
                    if !suppress && !inline_stack.is_empty() {
                        push_text!(" ".to_string());
                    }
                }
                let s = crate::xmltext::resolve_general_ref(&r);
                // No trim guard here, unlike Event::Text. That guard drops
                // the indentation between tags, which is markup, not content. A
                // reference is always authored deliberately, so `&#160;` or
                // `&#32;` is content and must survive.
                if !s.is_empty() && !inline_stack.is_empty() {
                    push_text!(s);
                }
                prev_was_ref = true;
            }
            Ok(Event::End(e)) => {
                pending_ws = None;
                prev_was_ref = false;
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
                        emit_block(
                            &mut frames,
                            &mut blocks,
                            Block::Heading {
                                level,
                                id,
                                inlines: inls,
                            },
                        );
                    }
                    b"p" => {
                        let inls = inline_stack.pop().unwrap_or_default();
                        cur_block = None;
                        if !inls.is_empty() {
                            emit_block(&mut frames, &mut blocks, Block::Para(inls));
                        }
                    }
                    b"ul" | b"ol" => {
                        if let Some(f) = frames.pop() {
                            finish_frame(f, &mut frames, &mut blocks);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => {
                while let Some(f) = frames.pop() {
                    finish_frame(f, &mut frames, &mut blocks);
                }
                break;
            }
            Err(_) => break,
            _ => {
                // Other events (comments, PI, etc.) also break adjacency.
                pending_ws = None;
                prev_was_ref = false;
            }
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
        // `&lt;` is the paragraph's entire text run, so under quick-xml 0.41
        // it arrives as a lone GeneralRef with no adjacent Event::Text --
        // Event::Text can never contain a `&...;` once the reader splits at
        // every reference. Reference resolution therefore lives entirely in
        // resolve_general_ref's unescape() call; the Event::Text arm only
        // decodes, and deliberately does not unescape.
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
    fn bare_ampersand_does_not_truncate_the_rest_of_the_document() {
        // A dangling `&` is the most common way real EPUB XHTML departs from
        // well-formedness. quick-xml 0.41 raises IllFormedError on it, and the
        // parse loop's `Err(_) => break` turned that into loss of EVERY later
        // block, at exit 0. `allow_dangling_amp` recovers it as literal text.
        // The regression is the SECOND paragraph, not just the `&`.
        let xml = "<p>Tom & Jerry</p><p>SECOND</p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
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

    // ---- Finding 1: whitespace adjacent to a reference must survive ----

    #[test]
    fn space_between_two_references_survives() {
        // Repro case from review: under 0.41 this is Text("a ") GeneralRef(lt)
        // Text(" ") GeneralRef(gt) Text(" b"). The middle `" "` sits between
        // two references and is real content, not inter-tag formatting; the
        // 0.36-era trim guard used to discard it, dropping a real space.
        let xml = "<p>a &lt; &gt; b</p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(text_of(para), "a < > b");
    }

    #[test]
    fn space_between_closing_tag_and_reference_survives() {
        // Repro case from review: the space between `</strong>` and `&amp;`
        // is a Text(" ") event immediately followed by GeneralRef(amp), not
        // preceded by one -- the "follows a reference" half of the fix.
        let xml = "<p><strong>A</strong> &amp; B</p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(para.len(), 2, "expected Strong(\"A\") + Text(\" & B\")");
        match &para[0] {
            Inline::Strong(x) => assert_eq!(text_of(x), "A"),
            other => panic!("expected Strong, got {other:?}"),
        }
        match &para[1] {
            Inline::Text(t) => assert_eq!(t, " & B"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn formatting_whitespace_between_tags_is_still_dropped() {
        // Guards the original intent of the trim guard: whitespace that is
        // NOT adjacent to any reference -- ordinary pretty-printed
        // indentation between tags -- must still be discarded, or every
        // formatted XHTML document would sprout stray whitespace inlines.
        let xml = "<p>\n  <strong>A</strong>\n  <em>B</em>\n</p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(
            para.len(),
            2,
            "no stray whitespace Text inline between Strong and Emph, got {para:?}"
        );
        assert!(matches!(para[0], Inline::Strong(_)));
        assert!(matches!(para[1], Inline::Emph(_)));
    }

    // ---- Finding 2: a whitespace-only character reference is content ----

    #[test]
    fn whitespace_only_character_reference_survives() {
        // The GeneralRef arm deliberately has no trim guard: &#160; (nbsp)
        // and &#32; (space) are always authored deliberately and must not be
        // treated as discardable inter-tag formatting the way a whitespace
        // Text fragment can be.
        let mut next_id = 0u32;

        let blocks = xhtml_to_blocks("<p>&#160;</p>", &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph for &#160;");
        assert_eq!(text_of(para), "\u{a0}");

        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks("<p>&#32;</p>", &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph for &#32;");
        assert_eq!(text_of(para), " ");
    }

    // ---- Finding 3: merging must stop at a formatting boundary ----

    #[test]
    fn merge_does_not_absorb_a_styled_inline_across_a_reference() {
        // push_inline only merges adjacent Inline::Text; a Strong pushed just
        // before a reference must remain its own inline, not be flattened
        // into the following text. This is the case that would catch an
        // over-eager merge.
        let xml = "<p><strong>A</strong>&amp;B</p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(para.len(), 2, "Strong must not merge into the text");
        match &para[0] {
            Inline::Strong(x) => assert_eq!(text_of(x), "A"),
            other => panic!("expected Strong, got {other:?}"),
        }
        match &para[1] {
            Inline::Text(t) => assert_eq!(t, "&B"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    // ---- Finding 5: kept whitespace must be normalized, not verbatim ----
    //
    // Every case above uses a literal single-space fragment, so pushing `s`
    // verbatim and pushing a normalized `" "` are indistinguishable. In
    // pretty-printed XHTML the fragment is multi-character (`"\n  "`), which
    // is where the previous fix's verbatim push actually leaked hard
    // newlines and indentation into paragraph text.

    #[test]
    fn pending_ws_flush_before_reference_is_normalized_not_verbatim() {
        // pending_ws = Some("\n  ") -> GeneralRef: the flush site at the top
        // of the GeneralRef arm. Pre-normalization this pushed "\n  " itself
        // between Strong("A") and "& B", splitting the paragraph onto a
        // second line.
        let xml = "<p>\n  <strong>A</strong>\n  &amp; B</p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(para.len(), 2, "expected Strong(\"A\") + Text(\" & B\")");
        match &para[0] {
            Inline::Strong(x) => assert_eq!(text_of(x), "A"),
            other => panic!("expected Strong, got {other:?}"),
        }
        match &para[1] {
            Inline::Text(t) => assert_eq!(t, " & B", "whitespace must collapse to one space"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn prev_was_ref_keep_of_multichar_whitespace_is_normalized() {
        // prev_was_ref = true -> Text("\n  "): the keep site inside the
        // Event::Text handler, mirroring the flush site above. Pre-fix this
        // pushed "\n  " verbatim between "&" and Emph("Q").
        let xml = "<p>P &amp;\n  <em>Q</em></p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(para.len(), 2, "expected Text(\"P & \") + Emph(\"Q\")");
        match &para[0] {
            Inline::Text(t) => assert_eq!(t, "P & ", "whitespace must collapse to one space"),
            other => panic!("expected Text, got {other:?}"),
        }
        match &para[1] {
            Inline::Emph(x) => assert_eq!(text_of(x), "Q"),
            other => panic!("expected Emph, got {other:?}"),
        }
    }

    // ---- Two additional transitions named in review, not covered above ----

    #[test]
    fn prev_was_ref_keep_of_trailing_space_before_end_is_retained() {
        // prev_was_ref = true -> Text(" ") -> End: `<p>a &amp; </p>`.
        // Pre-7d3163e this trailing space was dropped outright (the old
        // guard discarded every whitespace-only fragment unconditionally).
        // Under the reference-adjacency design, a Text(" ") immediately
        // following a reference is, by the same rule that keeps a leading
        // separator space, real content -- there is nothing in the state
        // machine that distinguishes "adjacent to a reference, then more
        // content follows" from "adjacent to a reference, then the block
        // ends". Retaining it is the consistent choice; pin it.
        let xml = "<p>a &amp; </p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(
            text_of(para),
            "a & ",
            "trailing reference-adjacent space is kept"
        );
    }

    #[test]
    fn pending_ws_dropped_at_eof_without_a_flushing_event() {
        // pending_ws = Some(_) -> Eof: reached only via malformed/truncated
        // input (a `<p>` never closed), since on well-formed input every
        // pending_ws is resolved by a Start, End, GeneralRef, or the `_` arm
        // before Eof is reached. Nothing ever flushes it to inline_stack in
        // this path -- it is dropped by the same fallthrough as any other
        // unresolved pending_ws, and the unclosed `<p>` never reaches the
        // Event::End(b"p") handler that would emit a Block::Para. Pinning
        // this: no panic, no block, no leaked fragment.
        let xml = "<p>\n  ";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        assert!(
            blocks.is_empty(),
            "unclosed paragraph must not emit a block, got {blocks:?}"
        );
    }

    // ---- Leading whitespace at the start of an inline frame is suppressed ----

    #[test]
    fn pending_ws_flush_at_block_start_suppresses_leading_space() {
        // pending_ws = Some("\n  ") -> GeneralRef, but with nothing pushed
        // into the top inline frame yet: `<p>` just pushed a fresh empty
        // vec, so the "\n  " before `&amp;X` is leading indentation, not a
        // separator between content. Regression: this used to render as
        // " &X" with a leading space.
        let xml = "<p>\n  &amp;X\n</p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        // trim_end(), not a bare equality: the trailing "\n" between "X" and
        // "</p>" arrives as part of the *non-whitespace* Text("X\n") event,
        // which the plain `else` branch of the Text handler has always
        // pushed verbatim -- a separate, pre-existing gap with no leading/
        // trailing trim at all, present since before this fix series
        // (verified against a20db75 and a31e854) and out of scope for the
        // GeneralRef-flush-site fix this test pins. It is harmless in the
        // rendered Markdown (the byte is absorbed as the paragraph's own
        // line terminator, producing an extra blank line rather than
        // corrupting the visible "&X" line -- confirmed against the CLI).
        assert_eq!(
            text_of(para).trim_end(),
            "&X",
            "no leading space at block start"
        );
        assert!(
            !text_of(para).starts_with(' '),
            "must not have a leading space, the regression under test"
        );
    }

    #[test]
    fn pending_ws_flush_at_nested_inline_start_keeps_leading_space_per_frame() {
        // Proves the suppression check is block-vs-inline (frame depth), not
        // just "was this frame pushed empty": `<em>` opens its own fresh
        // empty inline vec, but it is a *nested inline* frame (depth 2, not
        // the block frame at depth 1), so the whitespace immediately inside
        // it is real content and must survive, same as the paragraph's own
        // top-level content ("A ") before the `<em>` starts.
        //
        // Renamed and re-asserted from
        // `pending_ws_flush_at_nested_inline_start_suppresses_leading_space_per_frame`,
        // which pinned the regression this task fixes: it asserted
        // Emph("&B") (space dropped) for this same input. Per HTML
        // whitespace processing, only block-start whitespace is stripped;
        // inline-start whitespace is authored content. Old (wrong)
        // expectation: Text("A ") + Emph("&B"). New (correct) expectation:
        // Text("A ") + Emph(" &B").
        let xml = "<p>A <em>\n  &amp;B</em></p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(para.len(), 2, "expected Text(\"A \") + Emph(\" &B\")");
        match &para[0] {
            Inline::Text(t) => assert_eq!(t, "A ", "text before <em> is unaffected"),
            other => panic!("expected Text, got {other:?}"),
        }
        match &para[1] {
            Inline::Emph(x) => assert_eq!(
                text_of(x),
                " &B",
                "leading space inside a nested inline frame is authored content, not stripped"
            ),
            other => panic!("expected Emph, got {other:?}"),
        }
    }

    // ---- Regression: suppression must be block-vs-inline, not per-frame ----
    //
    // d2ee737 added the empty-top-frame check above to suppress a leading
    // space at block start (`<p>\n  &amp;X` -> "&X"). It was too blunt: it
    // also suppressed at the start of any nested inline frame (`<em>`,
    // `<strong>`, `<a>`), deleting an authored space and joining words. Per
    // HTML whitespace processing, leading whitespace at the start of a
    // *block* is stripped, but whitespace at the start of an *inline*
    // element is not -- `A<em> &amp;B</em>` means `A` then a space then
    // emphasized `&B`. These pin the corrected block/inline distinction.

    #[test]
    fn leading_space_survives_at_start_of_em() {
        let xml = "<p>A<em> &amp;B</em></p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(para.len(), 2, "expected Text(\"A\") + Emph(\" &B\")");
        match &para[0] {
            Inline::Text(t) => assert_eq!(t, "A"),
            other => panic!("expected Text, got {other:?}"),
        }
        match &para[1] {
            Inline::Emph(x) => assert_eq!(text_of(x), " &B"),
            other => panic!("expected Emph, got {other:?}"),
        }
    }

    #[test]
    fn leading_space_survives_at_start_of_strong() {
        let xml = "<p>A<strong> &amp;B</strong> C</p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(
            para.len(),
            3,
            "expected Text(\"A\") + Strong(\" &B\") + Text(\" C\")"
        );
        match &para[0] {
            Inline::Text(t) => assert_eq!(t, "A"),
            other => panic!("expected Text, got {other:?}"),
        }
        match &para[1] {
            Inline::Strong(x) => assert_eq!(text_of(x), " &B"),
            other => panic!("expected Strong, got {other:?}"),
        }
        match &para[2] {
            Inline::Text(t) => assert_eq!(t, " C"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn leading_space_survives_at_start_of_doubly_nested_inline() {
        // `<em><strong>` nests two inline frames (depth 2 and 3), neither of
        // which is the block frame (depth 1). The suppression must not fire
        // at either.
        let xml = "<p>A <em><strong> &amp;B</strong></em></p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(
            para.len(),
            2,
            "expected Text(\"A \") + Emph([Strong(\" &B\")])"
        );
        match &para[0] {
            Inline::Text(t) => assert_eq!(t, "A "),
            other => panic!("expected Text, got {other:?}"),
        }
        match &para[1] {
            Inline::Emph(x) => {
                assert_eq!(x.len(), 1, "expected a single Strong inside the Emph");
                match &x[0] {
                    Inline::Strong(y) => assert_eq!(text_of(y), " &B"),
                    other => panic!("expected Strong, got {other:?}"),
                }
            }
            other => panic!("expected Emph, got {other:?}"),
        }
    }

    #[test]
    fn control_leading_space_survives_at_start_of_em_without_a_reference() {
        // Control proving the loss was reference-specific: the non-whitespace
        // Text(" B") fragment always took the plain `else` branch of the
        // Event::Text handler (line ~89), which has no suppression logic at
        // all, so this case was never broken. Kept here alongside the
        // GeneralRef-triggered cases above for contrast.
        let xml = "<p>A<em> B</em></p>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph");
        assert_eq!(para.len(), 2, "expected Text(\"A\") + Emph(\" B\")");
        match &para[0] {
            Inline::Text(t) => assert_eq!(t, "A"),
            other => panic!("expected Text, got {other:?}"),
        }
        match &para[1] {
            Inline::Emph(x) => assert_eq!(text_of(x), " B"),
            other => panic!("expected Emph, got {other:?}"),
        }
    }

    #[test]
    fn block_start_suppression_still_applies_inside_a_heading() {
        // The block/inline fix must not regress the case d2ee737 fixed for
        // headings, not just paragraphs: h1..h6 push the block frame the
        // same way `p` does (xhtml.rs Event::Start, `h1"..="h6"` arm).
        let xml = "<h2>\n  &amp;T\n</h2>";
        let mut next_id = 0u32;
        let blocks = xhtml_to_blocks(xml, &mut next_id);
        let heading = blocks
            .iter()
            .find_map(|b| match b {
                Block::Heading { inlines, .. } => Some(inlines),
                _ => None,
            })
            .expect("a heading");
        assert_eq!(
            text_of(heading).trim_end(),
            "&T",
            "no leading space at block start of a heading"
        );
        assert!(
            !text_of(heading).starts_with(' '),
            "must not have a leading space, the case d2ee737 fixed"
        );
    }

    // Note: the mid-content keep case (whitespace between `</strong>` and
    // `&amp;` inside `<p>\n  <strong>A</strong>\n  &amp; B\n</p>`) is already
    // covered by `pending_ws_flush_before_reference_is_normalized_not_verbatim`
    // above -- by the time that whitespace flushes, `</strong>` has already
    // pushed `Inline::Strong` into the paragraph's top frame, so the frame is
    // non-empty and the space survives. Not duplicated here.

    // ---- Block-frame stack: nested lists ----

    #[test]
    fn parses_flat_unordered_list() {
        let mut id = 0;
        let blocks = xhtml_to_blocks(
            "<body><ul><li><p>one</p></li><li><p>two</p></li></ul></body>",
            &mut id,
        );
        assert_eq!(blocks.len(), 1);
        let Block::List { ordered, items } = &blocks[0] else {
            panic!("expected List, got {:?}", blocks[0]);
        };
        assert!(!ordered);
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0][0], Block::Para(i) if text_of(i) == "one"));
        assert!(matches!(&items[1][0], Block::Para(i) if text_of(i) == "two"));
    }

    #[test]
    fn ordered_list_sets_ordered_flag() {
        let mut id = 0;
        let blocks = xhtml_to_blocks("<ol><li><p>a</p></li></ol>", &mut id);
        assert!(matches!(&blocks[0], Block::List { ordered: true, .. }));
    }

    #[test]
    fn nested_list_folds_into_parent_item() {
        let mut id = 0;
        let blocks = xhtml_to_blocks(
            "<ul><li><p>A</p><ul><li><p>A1</p></li></ul></li><li><p>B</p></li></ul>",
            &mut id,
        );
        assert_eq!(
            blocks.len(),
            1,
            "nested list must not become a sibling block"
        );
        let Block::List { items, .. } = &blocks[0] else {
            panic!()
        };
        assert_eq!(items.len(), 2);
        // item A holds its Para plus the nested List
        assert!(matches!(&items[0][0], Block::Para(i) if text_of(i) == "A"));
        let Block::List { items: sub, .. } = &items[0][1] else {
            panic!("expected nested List inside item A, got {:?}", items[0])
        };
        assert!(matches!(&sub[0][0], Block::Para(i) if text_of(i) == "A1"));
    }

    #[test]
    fn heading_inside_list_item_stays_in_item() {
        let mut id = 0;
        let blocks = xhtml_to_blocks("<ul><li><h3>t</h3></li></ul>", &mut id);
        let Block::List { items, .. } = &blocks[0] else {
            panic!()
        };
        assert!(matches!(&items[0][0], Block::Heading { level: 3, .. }));
    }

    #[test]
    fn unclosed_list_at_eof_is_flushed_not_dropped() {
        let mut id = 0;
        let blocks = xhtml_to_blocks("<ul><li><p>orphan</p>", &mut id);
        let Block::List { items, .. } = &blocks[0] else {
            panic!("unclosed list must still be emitted")
        };
        assert!(matches!(&items[0][0], Block::Para(i) if text_of(i) == "orphan"));
    }
}
