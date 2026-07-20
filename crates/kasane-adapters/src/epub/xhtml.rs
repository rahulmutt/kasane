use kasane_ir::{AssetRef, Block, BlockId, Inline, NoteId, RefTarget};
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
    Table {
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
        has_merged: bool,
        in_thead: bool,
        cur_row: Vec<Vec<Inline>>,
        row_has_td: bool,
    },
    Figure {
        image: Option<AssetRef>,
        alt: Vec<Inline>,
        caption: Vec<Inline>,
        // Stray content emitted directly under <figure> outside <figcaption>
        // (e.g. a rogue <p>). Kept separate from `caption` so the
        // figcaption End handler's unconditional `*caption = x` cannot
        // clobber it -- see emit_block's Figure arm and finish_frame.
        extra: Vec<Inline>,
    },
    Footnote {
        note: NoteId,
        blocks: Vec<Block>,
    },
}

// A block finishing while an inline collection is open (a table cell, later a
// figcaption) flattens into it instead of escaping the container.
fn emit_block(
    frames: &mut [BlockFrame],
    inline_stack: &mut [Vec<Inline>],
    out: &mut Vec<Block>,
    b: Block,
) {
    if let Some(top) = inline_stack.last_mut() {
        if !top.is_empty() {
            crate::xmltext::push_inline(top, Inline::Text(" ".into()));
        }
        flatten_block_inlines(&b, top);
        return;
    }
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
        // A block emitted directly under <table> (stray content between rows)
        // has nowhere to go; degrade by dropping structure, keeping nothing --
        // real content inside cells is caught by the inline_stack branch above.
        Some(BlockFrame::Table { .. }) => {}
        // A block emitted directly under <figure> outside of <figcaption>
        // (e.g. a stray <p>) has no structural home either, but its text is
        // not thrown away -- it flattens into `extra`, kept separate from
        // `caption` because the figcaption End handler unconditionally
        // overwrites `caption` and would otherwise clobber it regardless of
        // document order. `extra` is merged into the caption in
        // finish_frame, after that overwrite has already happened.
        Some(BlockFrame::Figure { extra, .. }) => {
            if !extra.is_empty() {
                crate::xmltext::push_inline(extra, Inline::Text(" ".into()));
            }
            flatten_block_inlines(&b, extra);
        }
        Some(BlockFrame::Footnote { blocks, .. }) => blocks.push(b),
    }
}

// Extracts a block's text content as inlines -- used when block markup
// appears where only inlines fit (inside a table cell). Structure is lost,
// text is not.
fn flatten_block_inlines(b: &Block, dst: &mut Vec<Inline>) {
    let sep = |dst: &mut Vec<Inline>| {
        if !dst.is_empty() {
            crate::xmltext::push_inline(dst, Inline::Text(" ".into()));
        }
    };
    match b {
        Block::Para(inls) | Block::Heading { inlines: inls, .. } => {
            dst.extend(inls.iter().cloned())
        }
        Block::List { items, .. } => {
            for item in items {
                for ib in item {
                    sep(dst);
                    flatten_block_inlines(ib, dst);
                }
            }
        }
        Block::Table(t) => {
            for row in std::iter::once(&t.header).chain(t.rows.iter()) {
                for cell in row {
                    sep(dst);
                    dst.extend(cell.iter().cloned());
                }
            }
        }
        Block::Figure { caption, .. } => dst.extend(caption.iter().cloned()),
        Block::CodeBlock { text, .. } => dst.push(Inline::Code(text.clone())),
        Block::MathBlock(s) => dst.push(Inline::Math(s.clone())),
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                sep(dst);
                flatten_block_inlines(ib, dst);
            }
        }
        Block::Raw { .. } => {}
    }
}

fn finish_frame(
    f: BlockFrame,
    frames: &mut [BlockFrame],
    inline_stack: &mut [Vec<Inline>],
    out: &mut Vec<Block>,
) {
    match f {
        BlockFrame::List { ordered, items } => {
            if !items.is_empty() {
                emit_block(frames, inline_stack, out, Block::List { ordered, items });
            }
        }
        BlockFrame::Table {
            mut header,
            mut rows,
            has_merged,
            ..
        } => {
            if header.is_empty() && !rows.is_empty() {
                header = rows.remove(0); // GFM requires a header row
            }
            let width = std::iter::once(header.len())
                .chain(rows.iter().map(Vec::len))
                .max()
                .unwrap_or(0);
            if width == 0 {
                return;
            }
            header.resize(width, Vec::new());
            for r in &mut rows {
                r.resize(width, Vec::new());
            }
            emit_block(
                frames,
                inline_stack,
                out,
                Block::Table(kasane_ir::Table {
                    header,
                    rows,
                    has_merged,
                }),
            );
        }
        BlockFrame::Figure {
            image,
            alt,
            caption,
            extra,
        } => {
            let mut caption = if caption.is_empty() { alt } else { caption };
            if !extra.is_empty() {
                if !caption.is_empty() {
                    crate::xmltext::push_inline(&mut caption, Inline::Text(" ".into()));
                }
                caption.extend(extra);
            }
            match image {
                Some(image) => emit_block(
                    frames,
                    inline_stack,
                    out,
                    Block::Figure {
                        image,
                        caption,
                        number: None,
                    },
                ),
                None if !caption.is_empty() => {
                    emit_block(frames, inline_stack, out, Block::Para(caption)) // never drop
                }
                None => {}
            }
        }
        BlockFrame::Footnote {
            note,
            blocks: fblocks,
        } => {
            if !fblocks.is_empty() {
                emit_block(
                    frames,
                    inline_stack,
                    out,
                    Block::Footnote {
                        id: note,
                        blocks: fblocks,
                    },
                );
            }
        }
    }
}

// Inline code is a flat string in the IR; nested markup inside <code> keeps
// its text only.
fn inlines_text(inls: &[Inline]) -> String {
    let mut s = String::new();
    for i in inls {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => s.push_str(t),
            Inline::Emph(x) | Inline::Strong(x) => s.push_str(&inlines_text(x)),
            Inline::Link { inlines, .. } => s.push_str(&inlines_text(inlines)),
            Inline::FootnoteRef(_) => {}
        }
    }
    s
}

// Inline-level tags do NOT terminate an implicit paragraph; everything else
// (including unknown tags) is treated as a block boundary.
fn is_inline_tag(name: &[u8]) -> bool {
    matches!(
        name,
        b"strong"
            | b"b"
            | b"em"
            | b"i"
            | b"a"
            | b"code"
            | b"span"
            | b"sub"
            | b"sup"
            | b"small"
            | b"u"
            | b"s"
            | b"br"
    )
}

// epub:type is a space-separated token list, e.g. "footnote" or "rearnote footnote".
fn epub_type_has(e: &quick_xml::events::BytesStart, token: &str) -> bool {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == b"epub:type")
        .map(|a| {
            String::from_utf8_lossy(&a.value)
                .split_whitespace()
                .any(|t| t == token)
        })
        .unwrap_or(false)
}

// A single spine file's parse result: its blocks, plus enough to resolve
// same-file and cross-file `<a href>` fragments against headings once every
// spine file has been parsed (see `epub::mod::fix_links`).
pub struct FileParse {
    pub blocks: Vec<Block>,
    // id attr -> nearest preceding heading's BlockId.
    pub anchors: Vec<(String, BlockId)>,
    pub first_heading: Option<BlockId>,
    // <aside epub:type="footnote" id="..."> id attr -> the NoteId it became.
    pub footnotes: Vec<(String, NoteId)>,
    // href of every <a epub:type="noteref" href="...">, in document order.
    pub noteref_hrefs: Vec<String>,
}

// Returns blocks (plus the anchor map -- see `FileParse`); `next_id` is a
// running BlockId counter for headings. `base_dir` is the XHTML file's
// parent directory inside the zip (e.g. "OEBPS"), used to resolve `img`
// `src` attributes to zip-entry keys. `next_note` is a running NoteId
// counter for `<aside epub:type="footnote">` elements.
pub fn xhtml_to_blocks(
    xml: &str,
    base_dir: &str,
    next_id: &mut u32,
    next_note: &mut u32,
) -> FileParse {
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
    let mut in_body = false;
    let mut implicit_para = false;
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
    // Verbatim accumulation while inside <pre>: (language, accumulated text).
    // Text/whitespace inside <pre> is content, not formatting, so it bypasses
    // the pending_ws/prev_was_ref machinery above entirely -- see the
    // interception block at the top of the loop below.
    let mut pre: Option<(Option<String>, String)> = None;
    // Anchor tracking: every `id` attribute in the file maps to the nearest
    // preceding heading's BlockId, so a same-file or cross-file `<a href>`
    // fragment can resolve to a heading even when the id itself sits on a
    // <p> or <span>, not the heading.
    let mut anchors: Vec<(String, BlockId)> = vec![];
    let mut pending_anchor_ids: Vec<String> = vec![]; // ids seen before the first heading
    let mut first_heading: Option<BlockId> = None;
    let mut last_heading: Option<BlockId> = None;
    let mut heading_own_id: Option<String> = None; // id attr on the open h1..h6 itself
                                                   // Footnote tracking: aside id -> the NoteId it became, and every noteref
                                                   // href seen, in document order. `aside_pushed` mirrors the nesting of
                                                   // <aside> tags so the End handler knows whether a given close corresponds
                                                   // to a Footnote frame it opened (a non-footnote <aside> stays transparent).
    let mut footnotes: Vec<(String, NoteId)> = vec![];
    let mut noteref_hrefs: Vec<String> = vec![];
    let mut aside_pushed: Vec<bool> = vec![];

    macro_rules! push_text {
        ($t:expr) => {
            if let Some(top) = inline_stack.last_mut() {
                crate::xmltext::push_inline(top, Inline::Text($t));
            }
        };
    }

    // Closes an open implicit paragraph (bare flow-level text) at any block
    // boundary, emitting what it collected. See spec §2 "flatten, never drop".
    macro_rules! close_implicit {
        () => {
            if implicit_para {
                implicit_para = false;
                let inls = inline_stack.pop().unwrap_or_default();
                if !inls.is_empty() {
                    emit_block(
                        &mut frames,
                        &mut inline_stack,
                        &mut blocks,
                        Block::Para(inls),
                    );
                }
            }
        };
    }

    loop {
        let ev = reader.read_event_into(&mut buf);
        // Interception: while inside <pre>, text is verbatim (no trim, no
        // pending_ws -- whitespace IS the content), so this bypasses the
        // main match's whitespace machinery entirely rather than threading
        // a "verbatim mode" flag through every arm of it.
        if let Some((lang, text)) = pre.as_mut() {
            match &ev {
                Ok(Event::Text(t)) => {
                    text.push_str(&t.decode().map(|d| d.into_owned()).unwrap_or_default());
                }
                Ok(Event::GeneralRef(r)) => {
                    text.push_str(&crate::xmltext::resolve_general_ref(r));
                }
                Ok(Event::Start(e)) if e.local_name().as_ref() == b"code" && lang.is_none() => {
                    *lang = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == b"class")
                        .and_then(|a| {
                            String::from_utf8_lossy(&a.value)
                                .split_whitespace()
                                .find_map(|c| c.strip_prefix("language-").map(str::to_string))
                        });
                }
                Ok(Event::End(e)) if e.local_name().as_ref() == b"pre" => {
                    let (lang, text) = pre.take().expect("in pre");
                    let text = text.trim_matches('\n').to_string();
                    emit_block(
                        &mut frames,
                        &mut inline_stack,
                        &mut blocks,
                        Block::CodeBlock { lang, text },
                    );
                }
                Ok(Event::Eof) => {
                    let (lang, text) = pre.take().expect("in pre");
                    emit_block(
                        &mut frames,
                        &mut inline_stack,
                        &mut blocks,
                        Block::CodeBlock {
                            lang,
                            text: text.trim_matches('\n').to_string(),
                        },
                    );
                    break;
                }
                _ => {} // other markup inside <pre> is ignored, its text still arrives as Text events
            }
            buf.clear();
            continue;
        }
        match ev {
            Ok(Event::Start(e)) => {
                // A tag boundary resolves any undecided whitespace fragment
                // as formatting, not reference-adjacent content.
                pending_ws = None;
                prev_was_ref = false;
                if !is_inline_tag(e.local_name().as_ref()) {
                    close_implicit!();
                }
                if e.local_name().as_ref() == b"body" {
                    in_body = true;
                }
                // An inline tag (e.g. `strong`, `em`, `a`) that is the FIRST
                // flow-level content -- no preceding bare text opened the
                // implicit paragraph -- must open it itself. Otherwise this
                // Start pushes its own inline frame, and the matching End
                // pops it and finds inline_stack empty when it tries to
                // attach the result, silently discarding the content. Mirrors
                // the Text-arm opener above.
                if is_inline_tag(e.local_name().as_ref())
                    && inline_stack.is_empty()
                    && in_body
                    && cur_block.is_none()
                {
                    inline_stack.push(vec![]);
                    implicit_para = true;
                }
                let id_attr = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref() == b"id")
                    .map(|a| String::from_utf8_lossy(&a.value).into_owned());
                if let Some(idv) = id_attr {
                    if matches!(
                        e.local_name().as_ref(),
                        b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6"
                    ) {
                        heading_own_id = Some(idv); // resolved to the heading's own BlockId at End
                    } else if let Some(h) = last_heading {
                        anchors.push((idv, h));
                    } else {
                        pending_anchor_ids.push(idv);
                    }
                }
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
                        if epub_type_has(&e, "noteref") {
                            if let Some(h) = &link_href {
                                noteref_hrefs.push(h.clone());
                            }
                        }
                        inline_stack.push(vec![]);
                    }
                    // close_implicit! already ran above: `pre` is not an
                    // inline tag, so any bare flow-level text preceding it
                    // was flushed as its own paragraph.
                    b"pre" => pre = Some((None, String::new())),
                    b"code" => {
                        // The generic inline-tag opener above already opens
                        // an implicit paragraph for flow-level <code> (it is
                        // listed in is_inline_tag), so only the tag's own
                        // frame is pushed here -- pushing again here would
                        // double-open.
                        inline_stack.push(vec![]);
                    }
                    b"br" => {
                        if let Some(top) = inline_stack.last_mut() {
                            if !top.is_empty() {
                                crate::xmltext::push_inline(top, Inline::Text(" ".into()));
                            }
                        }
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
                    b"table" => frames.push(BlockFrame::Table {
                        header: vec![],
                        rows: vec![],
                        has_merged: false,
                        in_thead: false,
                        cur_row: vec![],
                        row_has_td: false,
                    }),
                    b"thead" => {
                        if let Some(BlockFrame::Table { in_thead, .. }) = frames.last_mut() {
                            *in_thead = true;
                        }
                    }
                    b"tr" => {
                        if let Some(BlockFrame::Table {
                            cur_row,
                            row_has_td,
                            ..
                        }) = frames.last_mut()
                        {
                            cur_row.clear();
                            *row_has_td = false;
                        }
                    }
                    b"th" | b"td" => {
                        if let Some(BlockFrame::Table {
                            has_merged,
                            row_has_td,
                            ..
                        }) = frames.last_mut()
                        {
                            let merged = e.attributes().flatten().any(|a| {
                                matches!(a.key.as_ref(), b"colspan" | b"rowspan")
                                    && a.value.as_ref() != b"1"
                            });
                            *has_merged |= merged;
                            *row_has_td |= e.local_name().as_ref() == b"td";
                            inline_stack.push(vec![]);
                        }
                    }
                    b"figure" => frames.push(BlockFrame::Figure {
                        image: None,
                        alt: vec![],
                        caption: vec![],
                        extra: vec![],
                    }),
                    b"aside" => {
                        if epub_type_has(&e, "footnote") {
                            let note = NoteId(*next_note);
                            *next_note += 1;
                            if let Some(idv) = e
                                .attributes()
                                .flatten()
                                .find(|a| a.key.as_ref() == b"id")
                                .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                            {
                                footnotes.push((idv, note));
                            }
                            frames.push(BlockFrame::Footnote {
                                note,
                                blocks: vec![],
                            });
                            aside_pushed.push(true);
                        } else {
                            aside_pushed.push(false); // transparent aside
                        }
                    }
                    b"figcaption" => inline_stack.push(vec![]),
                    b"img" => {
                        let attr = |k: &[u8]| {
                            e.attributes()
                                .flatten()
                                .find(|a| a.key.as_ref() == k)
                                .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                        };
                        let src = attr(b"src").unwrap_or_default();
                        let alt = attr(b"alt").unwrap_or_default();
                        let key = if src.is_empty() || crate::guard::has_scheme(&src) {
                            None
                        } else {
                            crate::guard::resolve_rel(base_dir, &src)
                        };
                        match key {
                            Some(key) => {
                                let aref = AssetRef { key, bytes_ref: 0 };
                                let alt_inls = if alt.is_empty() {
                                    vec![]
                                } else {
                                    vec![Inline::Text(alt)]
                                };
                                if let Some(BlockFrame::Figure {
                                    image, alt: falt, ..
                                }) = frames.last_mut()
                                {
                                    if image.is_none() {
                                        *image = Some(aref);
                                        *falt = alt_inls;
                                    }
                                } else {
                                    emit_block(
                                        &mut frames,
                                        &mut inline_stack,
                                        &mut blocks,
                                        Block::Figure {
                                            image: aref,
                                            caption: alt_inls,
                                            number: None,
                                        },
                                    );
                                }
                            }
                            None => {
                                eprintln!("warning: skipping image with unusable src '{src}'");
                                let b = if alt.is_empty() {
                                    Block::Raw {
                                        note: format!("image unavailable: {src}"),
                                    }
                                } else {
                                    Block::Para(vec![Inline::Text(alt)])
                                };
                                emit_block(&mut frames, &mut inline_stack, &mut blocks, b);
                            }
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
                    if inline_stack.is_empty() && in_body && cur_block.is_none() {
                        inline_stack.push(vec![]);
                        implicit_para = true;
                    }
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
                if inline_stack.is_empty() && in_body && cur_block.is_none() {
                    inline_stack.push(vec![]);
                    implicit_para = true;
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
                if !is_inline_tag(e.local_name().as_ref()) {
                    close_implicit!();
                }
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
                    b"code" => {
                        // Inline code is a flat string in the IR; nested
                        // markup inside <code> (rare, but legal XHTML) keeps
                        // its text only, via inlines_text.
                        let x = inline_stack.pop().unwrap_or_default();
                        if let Some(top) = inline_stack.last_mut() {
                            top.push(Inline::Code(inlines_text(&x)));
                        }
                    }
                    b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6" => {
                        let inls = inline_stack.pop().unwrap_or_default();
                        let level = cur_block.take().unwrap_or(1);
                        let id = BlockId(*next_id);
                        *next_id += 1;
                        emit_block(
                            &mut frames,
                            &mut inline_stack,
                            &mut blocks,
                            Block::Heading {
                                level,
                                id,
                                inlines: inls,
                            },
                        );
                        last_heading = Some(id);
                        if first_heading.is_none() {
                            first_heading = Some(id);
                            for a in pending_anchor_ids.drain(..) {
                                anchors.push((a, id));
                            }
                        }
                        if let Some(own) = heading_own_id.take() {
                            anchors.push((own, id));
                        }
                    }
                    b"p" => {
                        let inls = inline_stack.pop().unwrap_or_default();
                        cur_block = None;
                        if !inls.is_empty() {
                            emit_block(
                                &mut frames,
                                &mut inline_stack,
                                &mut blocks,
                                Block::Para(inls),
                            );
                        }
                    }
                    b"ul" | b"ol" => {
                        if let Some(f) = frames.pop() {
                            finish_frame(f, &mut frames, &mut inline_stack, &mut blocks);
                        }
                    }
                    b"thead" => {
                        if let Some(BlockFrame::Table { in_thead, .. }) = frames.last_mut() {
                            *in_thead = false;
                        }
                    }
                    b"th" | b"td" => {
                        if matches!(frames.last(), Some(BlockFrame::Table { .. })) {
                            let cell = inline_stack.pop().unwrap_or_default();
                            if let Some(BlockFrame::Table { cur_row, .. }) = frames.last_mut() {
                                cur_row.push(cell);
                            }
                        }
                    }
                    b"tr" => {
                        if let Some(BlockFrame::Table {
                            header,
                            rows,
                            in_thead,
                            cur_row,
                            row_has_td,
                            ..
                        }) = frames.last_mut()
                        {
                            let row = std::mem::take(cur_row);
                            if row.is_empty() {
                            } else if header.is_empty()
                                && rows.is_empty()
                                && (*in_thead || !*row_has_td)
                            {
                                *header = row; // thead row, or an all-<th> first row
                            } else {
                                rows.push(row);
                            }
                        }
                    }
                    b"table" => {
                        if matches!(frames.last(), Some(BlockFrame::Table { .. })) {
                            let f = frames.pop().expect("checked");
                            finish_frame(f, &mut frames, &mut inline_stack, &mut blocks);
                        }
                    }
                    b"figcaption" => {
                        let x = inline_stack.pop().unwrap_or_default();
                        if let Some(BlockFrame::Figure { caption, .. }) = frames.last_mut() {
                            *caption = x;
                        } else if let Some(top) = inline_stack.last_mut() {
                            top.extend(x);
                        } else if !x.is_empty() {
                            emit_block(&mut frames, &mut inline_stack, &mut blocks, Block::Para(x));
                        }
                    }
                    b"figure" => {
                        if matches!(frames.last(), Some(BlockFrame::Figure { .. })) {
                            let f = frames.pop().expect("checked");
                            finish_frame(f, &mut frames, &mut inline_stack, &mut blocks);
                        }
                    }
                    b"aside"
                        if aside_pushed.pop() == Some(true)
                            && matches!(frames.last(), Some(BlockFrame::Footnote { .. })) =>
                    {
                        let f = frames.pop().expect("checked");
                        finish_frame(f, &mut frames, &mut inline_stack, &mut blocks);
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => {
                // The final `implicit_para = false` inside the macro here is a
                // dead store (the loop breaks right after), unlike its other
                // call sites where the next iteration reads it back; silence
                // the false-positive rather than weaken the lint elsewhere.
                #[allow(unused_assignments)]
                {
                    close_implicit!();
                }
                while let Some(f) = frames.pop() {
                    finish_frame(f, &mut frames, &mut inline_stack, &mut blocks);
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
    FileParse {
        blocks,
        anchors,
        first_heading,
        footnotes,
        noteref_hrefs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(xml: &str) -> FileParse {
        let mut id = 0;
        xhtml_to_blocks(xml, "OEBPS", &mut id, &mut 0)
    }

    fn parse_blocks(xml: &str) -> Vec<Block> {
        parse(xml).blocks
    }

    #[test]
    fn anchors_map_ids_to_nearest_preceding_heading() {
        let fp = parse(
            "<body><h1 id=\"top\">A</h1><p id=\"p1\">x</p><h2 id=\"s2\">B</h2><p id=\"p2\">y</p></body>",
        );
        assert_eq!(fp.first_heading, Some(BlockId(0)));
        let get = |k: &str| fp.anchors.iter().find(|(a, _)| a == k).map(|(_, b)| *b);
        assert_eq!(get("top"), Some(BlockId(0))); // id on the heading -> the heading itself
        assert_eq!(get("p1"), Some(BlockId(0)));
        assert_eq!(get("s2"), Some(BlockId(1)));
        assert_eq!(get("p2"), Some(BlockId(1)));
    }

    #[test]
    fn pre_heading_ids_resolve_to_first_heading() {
        let fp = parse("<body><p id=\"intro\">x</p><h1>A</h1></body>");
        assert_eq!(
            fp.anchors
                .iter()
                .find(|(a, _)| a == "intro")
                .map(|(_, b)| *b),
            Some(BlockId(0))
        );
    }

    #[test]
    fn headingless_file_records_no_anchors() {
        let fp = parse("<body><p id=\"x\">y</p></body>");
        assert!(fp.anchors.is_empty());
        assert!(fp.first_heading.is_none());
    }

    fn text_of(inls: &[Inline]) -> String {
        inls.iter()
            .map(|i| match i {
                Inline::Text(t) => t.clone(),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn figure_with_img_and_figcaption() {
        let blocks = parse_blocks(
            "<body><figure><img src=\"../images/cat.png\" alt=\"a cat\"/>\
             <figcaption>Feline <em>friend</em></figcaption></figure></body>",
        );
        let Block::Figure {
            image,
            caption,
            number,
        } = &blocks[0]
        else {
            panic!("expected Figure, got {:?}", blocks[0])
        };
        assert_eq!(image.key, "images/cat.png"); // resolved against OEBPS, ../ normalized
        assert_eq!(text_of(caption), "Feline ");
        assert!(matches!(&caption[1], Inline::Emph(_)));
        assert!(number.is_none());
    }

    #[test]
    fn bare_img_uses_alt_as_caption() {
        let blocks = parse_blocks("<body><img src=\"pic.png\" alt=\"desc\"/></body>");
        let Block::Figure { image, caption, .. } = &blocks[0] else {
            panic!()
        };
        assert_eq!(image.key, "OEBPS/pic.png");
        assert_eq!(text_of(caption), "desc");
    }

    #[test]
    fn figure_img_alt_used_when_no_figcaption() {
        let blocks =
            parse_blocks("<body><figure><img src=\"p.png\" alt=\"fallback\"/></figure></body>");
        let Block::Figure { caption, .. } = &blocks[0] else {
            panic!()
        };
        assert_eq!(text_of(caption), "fallback");
    }

    #[test]
    fn remote_img_degrades_to_alt_paragraph() {
        let blocks =
            parse_blocks("<body><img src=\"http://evil/x.png\" alt=\"chart of results\"/></body>");
        assert!(matches!(&blocks[0], Block::Para(i) if text_of(i) == "chart of results"));
    }

    #[test]
    fn remote_img_without_alt_degrades_to_raw_note() {
        let blocks = parse_blocks("<body><img src=\"data:image/png;base64,AA\"/></body>");
        assert!(matches!(&blocks[0], Block::Raw { .. }));
    }

    #[test]
    fn traversal_img_src_degrades() {
        let blocks = parse_blocks("<body><img src=\"../../../etc/passwd\" alt=\"x\"/></body>");
        assert!(matches!(&blocks[0], Block::Para(_)));
    }

    #[test]
    fn stray_content_before_figcaption_is_not_clobbered_by_figcaption_overwrite() {
        // Regression: emit_block used to flatten a stray block (a <p> before
        // <figcaption>) directly into the frame's `caption` field, but the
        // figcaption End handler does an unconditional `*caption = x` --
        // silently dropping the stray content when figcaption is processed
        // afterward.
        let blocks = parse_blocks(
            "<body><figure><p>Photo by Jane</p><img src=\"a.png\" alt=\"x\"/>\
             <figcaption>The cat</figcaption></figure></body>",
        );
        let Block::Figure { caption, .. } = &blocks[0] else {
            panic!("expected Figure, got {:?}", blocks[0])
        };
        let text = text_of(caption);
        assert!(
            text.contains("The cat"),
            "caption missing figcaption text: {text:?}"
        );
        assert!(
            text.contains("Photo by Jane"),
            "caption missing stray content: {text:?}"
        );
    }

    #[test]
    fn stray_content_after_figcaption_is_preserved() {
        let blocks = parse_blocks(
            "<body><figure><img src=\"a.png\" alt=\"x\"/>\
             <figcaption>The cat</figcaption><p>after</p></figure></body>",
        );
        let Block::Figure { caption, .. } = &blocks[0] else {
            panic!("expected Figure, got {:?}", blocks[0])
        };
        let text = text_of(caption);
        assert!(
            text.contains("The cat"),
            "caption missing figcaption text: {text:?}"
        );
        assert!(
            text.contains("after"),
            "caption missing stray content: {text:?}"
        );
    }

    #[test]
    fn figure_without_img_flattens_caption_to_para() {
        let blocks =
            parse_blocks("<body><figure><figcaption>orphan caption</figcaption></figure></body>");
        assert!(matches!(&blocks[0], Block::Para(i) if text_of(i) == "orphan caption"));
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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

        let blocks = parse_blocks("<p>&#160;</p>");
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(inls) => Some(inls),
                _ => None,
            })
            .expect("a paragraph for &#160;");
        assert_eq!(text_of(para), "\u{a0}");

        let blocks = parse_blocks("<p>&#32;</p>");
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks(xml);
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
        let blocks = parse_blocks("<body><ul><li><p>one</p></li><li><p>two</p></li></ul></body>");
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
        let blocks = parse_blocks("<ol><li><p>a</p></li></ol>");
        assert!(matches!(&blocks[0], Block::List { ordered: true, .. }));
    }

    #[test]
    fn nested_list_folds_into_parent_item() {
        let blocks =
            parse_blocks("<ul><li><p>A</p><ul><li><p>A1</p></li></ul></li><li><p>B</p></li></ul>");
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
        let blocks = parse_blocks("<ul><li><h3>t</h3></li></ul>");
        let Block::List { items, .. } = &blocks[0] else {
            panic!()
        };
        assert!(matches!(&items[0][0], Block::Heading { level: 3, .. }));
    }

    #[test]
    fn unclosed_list_at_eof_is_flushed_not_dropped() {
        let blocks = parse_blocks("<ul><li><p>orphan</p>");
        let Block::List { items, .. } = &blocks[0] else {
            panic!("unclosed list must still be emitted")
        };
        assert!(matches!(&items[0][0], Block::Para(i) if text_of(i) == "orphan"));
    }

    // ---- Implicit paragraphs, transparent containers, head/body gating ----

    #[test]
    fn blockquote_bare_text_becomes_paragraph() {
        let blocks =
            parse_blocks("<body><blockquote>quoted <em>words</em> here</blockquote></body>");
        assert_eq!(blocks.len(), 1);
        let Block::Para(inls) = &blocks[0] else {
            panic!("expected Para")
        };
        assert!(matches!(&inls[0], Inline::Text(t) if t == "quoted "));
        assert!(matches!(&inls[1], Inline::Emph(_)));
    }

    #[test]
    fn dl_definition_text_is_flattened_not_dropped() {
        let blocks = parse_blocks("<body><dl><dt>term</dt><dd>meaning</dd></dl></body>");
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], Block::Para(i) if text_of(i) == "term"));
        assert!(matches!(&blocks[1], Block::Para(i) if text_of(i) == "meaning"));
    }

    #[test]
    fn bare_li_text_becomes_item_paragraph() {
        let blocks = parse_blocks("<body><ul><li>one</li><li>two</li></ul></body>");
        let Block::List { items, .. } = &blocks[0] else {
            panic!()
        };
        assert!(matches!(&items[0][0], Block::Para(i) if text_of(i) == "one"));
        assert!(matches!(&items[1][0], Block::Para(i) if text_of(i) == "two"));
    }

    #[test]
    fn head_title_text_stays_out_of_output() {
        let blocks = parse_blocks(
            "<html><head><title>Skip Me</title></head><body><p>keep</p></body></html>",
        );
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], Block::Para(i) if text_of(i) == "keep"));
    }

    #[test]
    fn implicit_paragraph_splits_at_block_boundary() {
        let blocks = parse_blocks("<body><div>before<p>inside</p>after</div></body>");
        assert_eq!(blocks.len(), 3);
        assert!(matches!(&blocks[0], Block::Para(i) if text_of(i) == "before"));
        assert!(matches!(&blocks[1], Block::Para(i) if text_of(i) == "inside"));
        assert!(matches!(&blocks[2], Block::Para(i) if text_of(i) == "after"));
    }

    // ---- Regression: a leading inline tag at flow level must open the
    // implicit paragraph itself, not rely on preceding bare text ----

    #[test]
    fn leading_inline_tag_at_flow_level_opens_implicit_paragraph() {
        // Repro from review: <strong> is the FIRST flow-level content inside
        // <body>, with no preceding bare text to open the implicit-paragraph
        // wrapper. Pre-fix, Start(strong) pushed its own inline frame, then
        // End(strong) popped it and tried inline_stack.last_mut() to attach
        // the result -- found the stack empty, and silently discarded
        // "Warning" entirely. Only Para([" ok"]) survived.
        let blocks = parse_blocks("<body><strong>Warning</strong> ok</body>");
        assert_eq!(blocks.len(), 1, "expected a single Para, got {blocks:?}");
        let Block::Para(inls) = &blocks[0] else {
            panic!("expected Para, got {:?}", blocks[0]);
        };
        assert_eq!(
            inls.len(),
            2,
            "expected Strong(\"Warning\") + Text(\" ok\"), got {inls:?}"
        );
        match &inls[0] {
            Inline::Strong(x) => assert_eq!(text_of(x), "Warning"),
            other => panic!("expected Strong, got {other:?}"),
        }
        match &inls[1] {
            Inline::Text(t) => assert_eq!(t, " ok"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn leading_inline_tag_inside_list_item_opens_implicit_paragraph() {
        // Same regression, but inside a <li>: the first flow-level content
        // of the item is an inline tag with no preceding bare text.
        let blocks = parse_blocks("<body><ul><li><em>x</em></li></ul></body>");
        let Block::List { items, .. } = &blocks[0] else {
            panic!("expected List, got {:?}", blocks[0]);
        };
        let Block::Para(inls) = &items[0][0] else {
            panic!("expected Para inside item, got {:?}", items[0]);
        };
        assert_eq!(inls.len(), 1, "expected a single Emph, got {inls:?}");
        match &inls[0] {
            Inline::Emph(x) => assert_eq!(text_of(x), "x"),
            other => panic!("expected Emph, got {other:?}"),
        }
    }

    // ---- Tables ----

    #[test]
    fn parses_table_with_thead() {
        let blocks = parse_blocks(
            "<body><table><thead><tr><th>H1</th><th>H2</th></tr></thead>\
             <tbody><tr><td>a</td><td><em>b</em></td></tr></tbody></table></body>",
        );
        let Block::Table(t) = &blocks[0] else {
            panic!("expected Table")
        };
        assert!(!t.has_merged);
        assert_eq!(text_of(&t.header[0]), "H1");
        assert_eq!(text_of(&t.header[1]), "H2");
        assert_eq!(t.rows.len(), 1);
        assert_eq!(text_of(&t.rows[0][0]), "a");
        assert!(matches!(&t.rows[0][1][0], Inline::Emph(_)));
    }

    #[test]
    fn th_only_first_row_without_thead_becomes_header() {
        let blocks =
            parse_blocks("<body><table><tr><th>H</th></tr><tr><td>v</td></tr></table></body>");
        let Block::Table(t) = &blocks[0] else {
            panic!()
        };
        assert_eq!(text_of(&t.header[0]), "H");
        assert_eq!(t.rows.len(), 1);
    }

    #[test]
    fn headerless_table_promotes_first_row() {
        let blocks =
            parse_blocks("<body><table><tr><td>a</td></tr><tr><td>b</td></tr></table></body>");
        let Block::Table(t) = &blocks[0] else {
            panic!()
        };
        assert_eq!(text_of(&t.header[0]), "a"); // GFM requires a header row
        assert_eq!(t.rows.len(), 1);
    }

    #[test]
    fn colspan_sets_merged_flag() {
        let blocks = parse_blocks("<body><table><tr><td colspan=\"2\">wide</td></tr><tr><td>a</td><td>b</td></tr></table></body>");
        let Block::Table(t) = &blocks[0] else {
            panic!()
        };
        assert!(t.has_merged);
    }

    #[test]
    fn short_row_is_padded_to_table_width() {
        let blocks = parse_blocks(
            "<body><table><tr><th>A</th><th>B</th></tr><tr><td>only</td></tr></table></body>",
        );
        let Block::Table(t) = &blocks[0] else {
            panic!()
        };
        assert_eq!(t.rows[0].len(), 2, "short row padded with an empty cell");
        assert!(t.rows[0][1].is_empty());
    }

    #[test]
    fn paragraph_inside_cell_flattens_to_cell_inlines() {
        let blocks = parse_blocks("<body><table><tr><td><p>x</p><p>y</p></td></tr></table></body>");
        let Block::Table(t) = &blocks[0] else {
            panic!()
        };
        assert_eq!(text_of(&t.header[0]), "x y"); // promoted headerless row; paras space-joined
    }

    // ---- Code blocks, inline code, <br> ----

    #[test]
    fn pre_becomes_code_block_with_verbatim_whitespace() {
        let blocks = parse_blocks("<body><pre><code class=\"language-rust\">fn main() {\n    let x = 1 &amp; 2;\n}</code></pre></body>");
        let Block::CodeBlock { lang, text } = &blocks[0] else {
            panic!("expected CodeBlock, got {:?}", blocks[0])
        };
        assert_eq!(lang.as_deref(), Some("rust"));
        assert_eq!(text, "fn main() {\n    let x = 1 & 2;\n}");
    }

    #[test]
    fn pre_without_code_child_still_works() {
        let blocks = parse_blocks("<body><pre>plain  spaced</pre></body>");
        assert!(
            matches!(&blocks[0], Block::CodeBlock { lang: None, text } if text == "plain  spaced")
        );
    }

    #[test]
    fn inline_code_survives_in_paragraph() {
        let blocks = parse_blocks("<body><p>call <code>foo()</code> now</p></body>");
        let Block::Para(inls) = &blocks[0] else {
            panic!()
        };
        assert!(inls
            .iter()
            .any(|i| matches!(i, Inline::Code(t) if t == "foo()")));
    }

    #[test]
    fn br_becomes_single_space() {
        let blocks = parse_blocks("<body><p>line one<br/>line two</p></body>");
        let Block::Para(inls) = &blocks[0] else {
            panic!()
        };
        assert_eq!(text_of(inls), "line one line two");
    }

    // ---- EPUB3 semantic footnotes ----

    #[test]
    fn semantic_aside_becomes_footnote_block() {
        let fp = parse(
            "<body><h1>C</h1><p>claim<a epub:type=\"noteref\" href=\"#fn1\">1</a></p>\
             <aside epub:type=\"footnote\" id=\"fn1\"><p>the details</p></aside></body>",
        );
        let Some(Block::Footnote { id, blocks }) = fp
            .blocks
            .iter()
            .find(|b| matches!(b, Block::Footnote { .. }))
        else {
            panic!("expected Footnote block")
        };
        assert_eq!(*id, NoteId(0));
        assert!(matches!(&blocks[0], Block::Para(i) if text_of(i) == "the details"));
        assert_eq!(fp.footnotes, vec![("fn1".to_string(), NoteId(0))]);
        assert_eq!(fp.noteref_hrefs, vec!["#fn1".to_string()]);
    }

    #[test]
    fn non_footnote_aside_stays_transparent() {
        let fp = parse("<body><h1>C</h1><aside><p>sidebar</p></aside></body>");
        assert!(!fp
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Footnote { .. })));
        assert!(fp
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Para(i) if text_of(i) == "sidebar")));
    }
}
