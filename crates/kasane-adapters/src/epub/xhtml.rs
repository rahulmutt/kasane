use kasane_ir::{AssetRef, Block, BlockId, Inline, NoteId, RefTarget};
use quick_xml::events::Event;
use quick_xml::Reader;

/// Maximum inline nesting this parser preserves as nested `Inline` values.
///
/// The frame stack that builds inlines is iterative, so parsing arbitrarily
/// nested `<em>`/`<strong>`/`<a>` never overflows here — but it hands the core
/// and the writer an `Inline` tree they walk recursively. Bounding the produced
/// depth is what keeps a hostile book from reaching
/// `kasane_ir::MAX_INLINE_DEPTH`.
///
/// This is a *fidelity* bound, not a safety one: past it a closing inline tag
/// contributes its text instead of another wrapper, so no content is lost. 64 is
/// far past any real book's `<em><strong><a>` layering.
pub(crate) const MAX_INLINE_DEPTH: usize = 64;

/// Maximum block nesting this parser preserves as nested `Block` values.
///
/// The frame stack that builds blocks is iterative, so parsing an
/// arbitrarily nested `<ul>` never overflows *here* — but it hands the core
/// and the writer a `Block` tree they walk recursively. Bounding the
/// produced depth is what keeps a hostile book from reaching
/// `kasane_ir::MAX_BLOCK_DEPTH`, exactly as `MAX_INLINE_DEPTH` above does
/// for inline nesting. The ordering invariant this depends on is
/// `epub::xhtml::MAX_BLOCK_DEPTH` (32) < `kasane_ir::MAX_BLOCK_DEPTH` (128).
///
/// This is a *fidelity* bound, not a safety one: past it a list's items
/// become siblings at this level instead of a nested list, and a footnote
/// `<aside>` becomes transparent, so no content is lost — only the nesting
/// relationship. 32 is far past any real book's list nesting, which rarely
/// exceeds a handful of levels.
///
/// Measured, not guessed: in a debug build, a rayon worker thread in batch
/// mode (Task 1, commit 289ce85) aborts on block nesting at depth 875 (the
/// largest surviving depth was 750), and a libtest thread aborts at 1024. 32
/// is a quarter of the 128 safety bound (`kasane_ir::MAX_BLOCK_DEPTH`), which
/// is itself the largest power of two under a quarter of 875.
///
/// One site covers three formats: MOBI/AZW3 re-serializes through this
/// parser (`mobi::normalize_html`), so it inherits this bound. PPTX nests
/// via `slide.rs`'s `build_list`, which carries its own fidelity bound,
/// `pptx::slide::MAX_LIST_LEVEL`, with its own compile-time assertion
/// against `kasane_ir::MAX_BLOCK_DEPTH`. PDF and DjVu never nest blocks.
///
/// The bound is enforced against `frames.len()`, the depth of the whole
/// open-container stack (`List`, `Table`, `Figure`, `Footnote` alike), not
/// against List/Footnote nesting counted on its own. So a `<ul>` sitting
/// inside 30 nested `<table>`s can flatten well before 32 levels of list
/// nesting are seen. This is always conservative -- produced List/Footnote
/// depth is bounded above by `frames.len()`, so it never exceeds this
/// constant -- but a reader should not expect exactly 32 nested `<ul>` tags
/// to be the only way to trigger flattening.
///
/// A footnote `<aside>` past this bound is transparent (its content is kept,
/// unnested), but it also stops being tracked as a footnote: no `NoteId` is
/// allocated and its `id` is not recorded, so a `noteref` pointing at it
/// resolves to nothing. This is consistent with the flattening contract --
/// no text is lost -- but the cross-reference link is.
pub(crate) const MAX_BLOCK_DEPTH: usize = 32;

// The two-bound design is only sound while this holds: adapter IR must not
// be able to reach the safety bound that the core and writer walks stop at.
// A `const` assertion rather than a `#[test]` on purpose -- this is the one
// invariant that spans both crates, and a bad edit should fail the BUILD
// rather than wait for someone to run the suite.
const _: () = assert!(
    MAX_BLOCK_DEPTH * 4 <= kasane_ir::MAX_BLOCK_DEPTH,
    "epub::xhtml::MAX_BLOCK_DEPTH must stay at most a quarter of kasane_ir::MAX_BLOCK_DEPTH"
);

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
        flatten_block_inlines(&b, top, 0);
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
            flatten_block_inlines(&b, extra, 0);
        }
        Some(BlockFrame::Footnote { blocks, .. }) => blocks.push(b),
    }
}

// A malformed-markup note is a statement about the DOCUMENT -- the markup
// after this point was mis-nested and got re-read as flow content -- not about
// one equation. Unlike "equation partially converted", whose in-band
// `\mathord{?}` token already self-marks an inline partial, it has no in-band
// counterpart, so routing it through emit_block would let an open inline
// context swallow it (flatten_block_inlines drops Block::Raw) and put us right
// back at the silent data loss this note exists to report. This is why the
// malformed note is emitted BEFORE its degraded equation in the XML source
// order, opposite to the partially-converted note which follows its equation.
// Use the normal block path when there is one; otherwise put it straight into
// the block flow.
fn emit_malformed_note(
    frames: &mut [BlockFrame],
    inline_stack: &mut [Vec<Inline>],
    out: &mut Vec<Block>,
    note: &str,
) {
    let b = Block::Raw { note: note.into() };
    if inline_stack.is_empty() {
        emit_block(frames, inline_stack, out, b);
    } else {
        // Pushing directly to out also bypasses the frame stack, placing the
        // note at the top level outside any list, table, or figure — acceptable
        // because the note documents document-level malformation, not local structure loss.
        out.push(b);
    }
}

// Extracts a block's text content as inlines -- used when block markup
// appears where only inlines fit (inside a table cell). Structure is lost,
// text is not.
fn flatten_block_inlines(b: &Block, dst: &mut Vec<Inline>, depth: usize) {
    // Unreachable via this parser, which flattens at `xhtml::MAX_BLOCK_DEPTH` (32),
    // far below 128. The guard stays anyway -- it is the guard that makes the invariant
    // independent, not the parser's own flattening.
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        return;
    }
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
                    flatten_block_inlines(ib, dst, depth + 1);
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
                flatten_block_inlines(ib, dst, depth + 1);
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

/// Wraps `x` in `wrap`, unless doing so would push nesting past
/// `MAX_INLINE_DEPTH` — in which case the content is contributed as flat text.
///
/// `depth` is the inline-frame depth *after* the frame being closed was popped,
/// so it is the depth of the frame that will receive the result — that is, the
/// nesting level the wrapper this call returns would *occupy*, not the number of
/// levels already descended. Hence `>` here where `kasane-core` and
/// `kasane-writer` write `depth >= kasane_ir::MAX_INLINE_DEPTH`: with `depth`
/// counting the resulting level, `>` admits exactly `MAX_INLINE_DEPTH` levels of
/// nesting, the same count `>=` admits over there. The two idioms differ only in
/// what `depth` names; both bounds are inclusive of `MAX_INLINE_DEPTH` levels.
fn wrap_inline(depth: usize, wrap: fn(Vec<Inline>) -> Inline, x: Vec<Inline>) -> Inline {
    if depth > MAX_INLINE_DEPTH {
        Inline::Text(inlines_text(&x))
    } else {
        wrap(x)
    }
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

// A <math> element is inline unless it carries display="block". Attribute
// inspection is why this is separate from is_inline_tag (name-only).
fn math_is_inline(e: &quick_xml::events::BytesStart) -> bool {
    e.local_name().as_ref() == b"math"
        && !e
            .attributes()
            .flatten()
            .any(|a| a.key.as_ref() == b"display" && a.value.as_ref() == b"block")
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
    // Same rationale as allow_dangling_amp: quick-xml's default
    // MismatchedEndTag check raises IllFormedError the moment a closing tag's
    // name doesn't match the innermost still-open one, and this loop's
    // `Err(_) => break` would then abandon the rest of the document on a
    // single bad closing tag anywhere in real-world EPUB content. Disabling
    // the check delivers the End event as written instead, so a stray/
    // mismatched close degrades locally -- e.g. via the BlockFrame guards
    // below (`matches!(frames.last(), Some(BlockFrame::List { .. }))` and
    // friends) -- rather than truncating everything after it.
    reader.config_mut().check_end_names = false;
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
    // Mirrors the nesting of <ul>/<ol> tags so the End handler knows whether
    // a given close corresponds to a List frame it opened. Without this, a
    // </ul> whose open was suppressed at MAX_BLOCK_DEPTH would satisfy the
    // End arm's `matches!(frames.last(), Some(BlockFrame::List { .. }))`
    // guard -- because the *enclosing* frame is also a List -- and pop the
    // parent, unbalancing `frames` for every block that follows.
    let mut list_pushed: Vec<bool> = vec![];

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
                    // Mirror the main Eof arm's frame drain: EOF mid-<pre>
                    // can still leave containers (a <ul>/<table>/... that
                    // opened before the <pre>) on the stack, and they must
                    // fold into `blocks` the same as any other truncated
                    // document, not be silently discarded. `implicit_para`
                    // is already false here -- <pre> is not an inline tag,
                    // so the Start(b"pre") arm's close_implicit!() already
                    // flushed any bare text that preceded it.
                    while let Some(f) = frames.pop() {
                        finish_frame(f, &mut frames, &mut inline_stack, &mut blocks);
                    }
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
                if !is_inline_tag(e.local_name().as_ref()) && !math_is_inline(&e) {
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
                if (is_inline_tag(e.local_name().as_ref()) || math_is_inline(&e))
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
                        if frames.len() >= MAX_BLOCK_DEPTH {
                            // Over the fidelity bound: push no frame, so this
                            // list's <li> items land in the enclosing List
                            // frame as siblings. Content is kept; only the
                            // nesting relationship is dropped.
                            list_pushed.push(false);
                        } else {
                            frames.push(BlockFrame::List {
                                ordered: e.local_name().as_ref() == b"ol",
                                items: vec![],
                            });
                            list_pushed.push(true);
                        }
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
                        if epub_type_has(&e, "footnote") && frames.len() < MAX_BLOCK_DEPTH {
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
                            // Not a footnote aside, or past the fidelity
                            // bound: transparent either way, so its blocks
                            // emit into the enclosing frame. The existing
                            // End arm already handles `Some(false)` by
                            // popping nothing.
                            aside_pushed.push(false);
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
                            crate::guard::resolve_rel(base_dir, &crate::guard::percent_decode(&src))
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
                    b"math" => {
                        let inline = math_is_inline(&e);
                        let conv = match crate::math::capture_island(&mut reader, &e) {
                            Ok(island) => crate::math::mathml_to_latex(&island),
                            Err(err) => {
                                // capture_island rewound the reader, so the
                                // markup this island swallowed is about to be
                                // re-read as ordinary content by this very
                                // loop -- nothing is lost, but the document
                                // was malformed here and that must be visible.
                                emit_malformed_note(
                                    &mut frames,
                                    &mut inline_stack,
                                    &mut blocks,
                                    err.note(),
                                );
                                crate::math::degraded()
                            }
                        };
                        if inline {
                            if let Some(top) = inline_stack.last_mut() {
                                crate::xmltext::push_inline(top, Inline::Math(conv.latex));
                            }
                        } else {
                            emit_block(
                                &mut frames,
                                &mut inline_stack,
                                &mut blocks,
                                Block::MathBlock(conv.latex),
                            );
                            if !conv.complete {
                                // If this display equation lands where an inline
                                // collection is already open (a table cell, a
                                // figcaption -- see emit_block's inline-context
                                // branch), the MathBlock above folds to
                                // Inline::Math carrying the in-band `\mathord{?}`
                                // token, and this Raw note is intentionally
                                // dropped by flatten_block_inlines's
                                // `Block::Raw { .. } => {}` arm: a folded display
                                // equation is inline, and the plan's degradation
                                // rule for inline partials is to self-mark via
                                // the token only (no inline note type is added).
                                // Pinned by
                                // partial_display_math_in_folding_context_drops_note_but_keeps_token.
                                emit_block(
                                    &mut frames,
                                    &mut inline_stack,
                                    &mut blocks,
                                    Block::Raw {
                                        note: "equation partially converted".into(),
                                    },
                                );
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
                        let depth = inline_stack.len();
                        if let Some(top) = inline_stack.last_mut() {
                            top.push(wrap_inline(depth, Inline::Strong, x));
                        }
                    }
                    b"em" | b"i" => {
                        let x = inline_stack.pop().unwrap_or_default();
                        let depth = inline_stack.len();
                        if let Some(top) = inline_stack.last_mut() {
                            top.push(wrap_inline(depth, Inline::Emph, x));
                        }
                    }
                    b"a" => {
                        let x = inline_stack.pop().unwrap_or_default();
                        let depth = inline_stack.len();
                        let target = match link_href.take() {
                            // EPUB internal links (both same-file `#frag` and cross-file
                            // `file.xhtml#frag` forms) currently pass through unresolved as
                            // `External`. Mapping them to `RefTarget::Internal(BlockId)` is
                            // deferred to Plan 2's XHTML-fidelity task.
                            Some(h) => RefTarget::External(h),
                            None => RefTarget::External(String::new()),
                        };
                        if let Some(top) = inline_stack.last_mut() {
                            if depth > MAX_INLINE_DEPTH {
                                top.push(Inline::Text(inlines_text(&x)));
                            } else {
                                top.push(Inline::Link { target, inlines: x });
                            }
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
                        // The frame guard is checked before popping the
                        // tracker, not after: a mismatched close (top frame
                        // is not a List, e.g. `<ul><li><table></ul>`) must
                        // leave `list_pushed` untouched, or it would consume
                        // the tracker entry belonging to a real, still-open
                        // list and leave that list's frame stuck open until
                        // EOF. With the guard first, a stray `</ul>` with no
                        // matching open still finds no List frame and is
                        // ignored; a mismatched close is ignored the same
                        // way and does not disturb the tracker; only a close
                        // that actually faces a List frame consults
                        // `list_pushed` to decide whether that frame is one
                        // this tag opened.
                        if matches!(frames.last(), Some(BlockFrame::List { .. }))
                            && list_pushed.pop() == Some(true)
                        {
                            let f = frames.pop().expect("checked");
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
    fn img_src_is_percent_decoded_before_resolution() {
        // `src` is an IRI reference, the zip entry name is literal bytes.
        let blocks = parse_blocks("<body><img src=\"my%20pic.png\" alt=\"d\"/></body>");
        let Block::Figure { image, .. } = &blocks[0] else {
            panic!("expected Figure, got {:?}", blocks[0])
        };
        assert_eq!(image.key, "OEBPS/my pic.png");

        // A multi-byte UTF-8 sequence spans two escapes.
        let blocks = parse_blocks("<body><img src=\"caf%C3%A9.png\" alt=\"d\"/></body>");
        let Block::Figure { image, .. } = &blocks[0] else {
            panic!("expected Figure, got {:?}", blocks[0])
        };
        assert_eq!(image.key, "OEBPS/café.png");

        // A literal `%` and a truncated escape survive verbatim.
        let blocks = parse_blocks("<body><img src=\"100%.png\" alt=\"d\"/></body>");
        let Block::Figure { image, .. } = &blocks[0] else {
            panic!("expected Figure, got {:?}", blocks[0])
        };
        assert_eq!(image.key, "OEBPS/100%.png");

        let blocks = parse_blocks("<body><img src=\"a%2.png\" alt=\"d\"/></body>");
        let Block::Figure { image, .. } = &blocks[0] else {
            panic!("expected Figure, got {:?}", blocks[0])
        };
        assert_eq!(image.key, "OEBPS/a%2.png");

        // An encoded separator is normalized by resolve_rel, not smuggled
        // into the key as an opaque segment.
        let blocks = parse_blocks("<body><img src=\"sub%2Fpic.png\" alt=\"d\"/></body>");
        let Block::Figure { image, .. } = &blocks[0] else {
            panic!("expected Figure, got {:?}", blocks[0])
        };
        assert_eq!(image.key, "OEBPS/sub/pic.png");
    }

    #[test]
    fn percent_encoded_traversal_img_src_degrades() {
        // The security case: decoding happens before resolve_rel, so an
        // encoded traversal is confined exactly like its literal form.
        let blocks =
            parse_blocks("<body><img src=\"%2E%2E%2F%2E%2E%2Fetc%2Fpasswd\" alt=\"x\"/></body>");
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

    #[test]
    fn eof_inside_pre_still_flushes_open_frames() {
        // Regression: the <pre> intercept's own Eof arm used to emit the
        // accumulated CodeBlock and `break` directly, bypassing the main
        // Eof arm's `while let Some(f) = frames.pop() { finish_frame(...) }`
        // drain. EOF mid-<pre> with an open <ul><li> left the List frame on
        // the stack forever -- it never folded into `blocks`, silently
        // dropping the list (and the CodeBlock landed inside it) entirely.
        let blocks = parse_blocks("<body><ul><li><p>one</p><pre>orphan code");
        assert_eq!(
            blocks.len(),
            1,
            "the list must survive EOF mid-<pre>, got {blocks:?}"
        );
        let Block::List { items, .. } = &blocks[0] else {
            panic!("expected List, got {:?}", blocks[0]);
        };
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].len(),
            2,
            "expected Para(\"one\") + CodeBlock(\"orphan code\") in the open item"
        );
        assert!(matches!(&items[0][0], Block::Para(i) if text_of(i) == "one"));
        assert!(
            matches!(&items[0][1], Block::CodeBlock { text, .. } if text == "orphan code"),
            "expected CodeBlock inside the still-open list item, got {:?}",
            items[0][1]
        );
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

    #[test]
    fn stray_ul_close_inside_table_does_not_pop_the_table_frame() {
        // Regression: `</ul>`/`</ol>` still used Task 1's untyped
        // `if let Some(f) = frames.pop()`, so a stray `</ul>` with no
        // matching `<ul>` popped whatever frame happened to be on top --
        // here the open <table> -- and finished it prematurely (before its
        // still-buffered <tr> flushed the cell into `rows`), losing the
        // table's content entirely.
        let blocks = parse_blocks("<body><table><tr><td>x</td></ul></tr></table></body>");
        assert_eq!(
            blocks.len(),
            1,
            "table must survive the stray </ul>, got {blocks:?}"
        );
        let Block::Table(t) = &blocks[0] else {
            panic!("expected Table, got {:?}", blocks[0]);
        };
        assert_eq!(text_of(&t.header[0]), "x");
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

    #[test]
    fn inline_math_stays_in_paragraph() {
        let blocks = parse_blocks(
            "<body><p>The value <math><msup><mi>x</mi><mn>2</mn></msup></math> is positive.</p></body>",
        );
        // One paragraph, containing an Inline::Math between the two text runs.
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(i) => Some(i),
                _ => None,
            })
            .expect("a paragraph");
        let math = para.iter().find_map(|i| match i {
            Inline::Math(s) => Some(s.clone()),
            _ => None,
        });
        assert_eq!(math.as_deref(), Some("{x}^{2}"));
        // The paragraph was not split into pieces around the math.
        assert_eq!(
            blocks
                .iter()
                .filter(|b| matches!(b, Block::Para(_)))
                .count(),
            1
        );
    }

    #[test]
    fn inline_math_after_leading_text_stays_in_the_same_implicit_paragraph() {
        // No wrapping <p>: inline math directly under <body>, preceded by
        // bare text, must land in the same implicit paragraph the leading
        // text opened rather than starting a new one or being dropped.
        //
        // Renamed from `inline_math_at_body_level_opens_implicit_paragraph`:
        // that name overstated what this input exercises. The leading text
        // "Value " already runs through the pre-existing Text arm's opener
        // (xhtml.rs's Event::Text handler) and pushes an inline frame before
        // <math> is ever seen, so by the time the Start(math) guard's
        // `|| math_is_inline(&e)` term is evaluated, `inline_stack.is_empty()`
        // is already false and that term is never the deciding factor. See
        // `leading_math_at_empty_inline_stack_opens_implicit_paragraph` below
        // for a case where the term actually decides.
        let blocks = parse_blocks("<body>Value <math><mn>1</mn></math> end.</body>");
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(i) => Some(i),
                _ => None,
            })
            .expect("an implicit paragraph");
        assert!(para.iter().any(|i| matches!(i, Inline::Math(_))));
    }

    #[test]
    fn leading_math_at_empty_inline_stack_opens_implicit_paragraph() {
        // Repro for the guard this task added support for
        // (`(is_inline_tag(...) || math_is_inline(&e)) && inline_stack.is_empty()
        // && in_body && cur_block.is_none()` in the Start(math) handler,
        // xhtml.rs around line 475): a non-display <math> is the FIRST thing
        // under <body>, with an empty inline_stack and no open block. Without
        // the `|| math_is_inline(&e)` disjunct, this guard would not fire,
        // inline_stack would still be empty when the `<math>` arm runs, and
        // its `if let Some(top) = inline_stack.last_mut()` would find nothing
        // to push into -- the equation silently vanishes. This test is the
        // one that actually exercises that term; see the check documented
        // alongside it for confirmation that removing the term makes this
        // test fail.
        let blocks = parse_blocks("<body><math><mn>1</mn></math></body>");
        let para = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(i) => Some(i),
                _ => None,
            })
            .expect("an implicit paragraph opened by the leading <math>, got no Para");
        let math = para.iter().find_map(|i| match i {
            Inline::Math(s) => Some(s.clone()),
            _ => None,
        });
        assert_eq!(
            math.as_deref(),
            Some("1"),
            "the equation must reach the output, not be silently dropped"
        );
    }

    #[test]
    fn leading_math_in_list_item_opens_implicit_paragraph() {
        // Cheap variant of the case above in a different empty-inline-stack
        // context: <li> (unlike <td>) does not push its own inline_stack
        // frame on Start -- see the b"li" arm, which only pushes onto the
        // List frame's `items` -- so inline_stack is still empty when <math>
        // is the item's first and only content.
        let blocks = parse_blocks("<body><ul><li><math><mn>2</mn></math></li></ul></body>");
        let Block::List { items, .. } = &blocks[0] else {
            panic!("expected List, got {:?}", blocks[0]);
        };
        let Block::Para(inls) = &items[0][0] else {
            panic!("expected Para inside item, got {:?}", items[0]);
        };
        let math = inls.iter().find_map(|i| match i {
            Inline::Math(s) => Some(s.clone()),
            _ => None,
        });
        assert_eq!(
            math.as_deref(),
            Some("2"),
            "the equation must reach the output, not be silently dropped"
        );
    }

    #[test]
    fn display_math_becomes_math_block() {
        let blocks = parse_blocks(
            "<body><p>Before.</p><math display=\"block\"><mfrac><mn>1</mn><mn>2</mn></mfrac></math><p>After.</p></body>",
        );
        let mb = blocks.iter().find_map(|b| match b {
            Block::MathBlock(s) => Some(s.clone()),
            _ => None,
        });
        assert_eq!(mb.as_deref(), Some("\\frac{1}{2}"));
    }

    #[test]
    fn partial_display_math_emits_raw_note() {
        // Content MathML is out of subset → placeholder + note.
        let blocks =
            parse_blocks("<body><math display=\"block\"><apply><ci>x</ci></apply></math></body>");
        let mb_idx = blocks
            .iter()
            .position(|b| matches!(b, Block::MathBlock(s) if s.contains("\\mathord{?}")))
            .expect("expected a MathBlock carrying the placeholder token");
        let note_idx = blocks
            .iter()
            .position(|b| matches!(b, Block::Raw { note } if note.contains("partially converted")))
            .expect("expected a Raw partial-conversion note");
        // "Adjacent" per the plan means the note immediately follows its
        // MathBlock, not merely that both exist somewhere in the document.
        assert_eq!(
            note_idx,
            mb_idx + 1,
            "the partial-conversion note must immediately follow its MathBlock, got blocks {blocks:?}"
        );
    }

    #[test]
    fn partial_display_math_in_folding_context_drops_note_but_keeps_token() {
        // Adjudicated as correct behavior, not a bug: when a MathBlock lands
        // where an inline collection is already open, emit_block's
        // inline-context branch (`if let Some(top) = inline_stack.last_mut()`)
        // flattens it to Inline::Math via flatten_block_inlines, and the
        // Block::MathBlock arm there keeps the in-band `\mathord{?}` token.
        // The following Block::Raw note hits the same fold and is dropped by
        // flatten_block_inlines's `Block::Raw { .. } => {}` arm: a folded
        // display equation is inline, and the plan's degradation rule for
        // inline partials is "self-mark via the token only (no inline note
        // type is added)". This pins that as deliberate so a future change
        // to the fold logic can't silently alter it.
        //
        // <td> is confirmed (not assumed) to fold here: the b"th" | b"td"
        // Start arm pushes its own inline_stack frame directly (xhtml.rs,
        // the `th | td` arm around line 591), so inline_stack is non-empty
        // for the whole cell, and emit_block's inline-context branch is
        // checked before the frames-based Table branch.
        let blocks = parse_blocks(
            "<body><table><tr><td><math display=\"block\"><apply><ci>x</ci></apply></math></td></tr></table></body>",
        );
        let Block::Table(t) = &blocks[0] else {
            panic!("expected Table, got {:?}", blocks[0]);
        };
        let cell = &t.header[0]; // headerless table promotes the first row
        assert!(
            cell.iter()
                .any(|i| matches!(i, Inline::Math(s) if s.contains("\\mathord{?}"))),
            "expected the folded equation's placeholder token in the cell, got {cell:?}"
        );
        assert!(
            !blocks
                .iter()
                .any(|b| matches!(b, Block::Raw { note } if note.contains("partially converted"))),
            "the partial-conversion note must not appear anywhere in the document when folded, got {blocks:?}"
        );
    }

    #[test]
    fn unclosed_math_keeps_the_rest_of_the_chapter_and_notes_the_malformation() {
        // The reader runs with check_end_names = false precisely so malformed
        // markup does not lose content. An unclosed <math> used to make
        // capture_island consume to EOF, so everything after it vanished with
        // no trace. capture_island now rewinds instead: the equation degrades,
        // the island's children come back as ordinary flow content, and the
        // malformation is recorded.
        let blocks = parse_blocks(
            "<body><p>before</p><math><mn>1</mn><p>AFTER-ONE</p><h2>AFTER-TWO</h2></body>",
        );
        let all_text: String = blocks
            .iter()
            .flat_map(|b| {
                let mut inls = Vec::new();
                super::flatten_block_inlines(b, &mut inls, 0);
                inls
            })
            .map(|i| match i {
                Inline::Text(t) => t,
                _ => String::new(),
            })
            .collect();
        assert!(
            all_text.contains("before"),
            "content before the island must survive, got {blocks:?}"
        );
        assert!(
            all_text.contains("AFTER-ONE"),
            "content after an unclosed <math> must survive, got {blocks:?}"
        );
        assert!(
            all_text.contains("AFTER-TWO"),
            "content after an unclosed <math> must survive, got {blocks:?}"
        );
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::Raw { note } if note.contains("unclosed equation"))),
            "the malformation must be noted, not silent, got {blocks:?}"
        );
    }

    #[test]
    fn over_deep_math_island_degrades_and_notes_without_aborting() {
        // Whole-adapter cover for the stack-overflow abort: capture_island
        // refuses the island on its nesting bound, so roxmltree never sees it,
        // the reader rewinds, and the (meaningless) <mrow> nest is re-read as
        // flow content -- which the parser ignores -- leaving the trailing
        // paragraph intact.
        let levels = 18_000;
        let xml = format!(
            "<body><math>{}<mn>1</mn>{}</math><p>AFTER</p></body>",
            "<mrow>".repeat(levels),
            "</mrow>".repeat(levels)
        );
        let blocks = parse_blocks(&xml);
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::Raw { note } if note.contains("too large"))),
            "the over-budget island must be noted, got {blocks:?}"
        );
        assert!(
            blocks.iter().any(|b| matches!(b, Block::Para(i)
                if i.iter().any(|x| matches!(x, Inline::Text(t) if t.contains("AFTER"))))),
            "content after the over-deep island must survive, got {blocks:?}"
        );
    }

    #[test]
    fn deep_inline_nesting_is_flattened_not_preserved() {
        fn depth_of(inls: &[Inline]) -> usize {
            inls.iter()
                .map(|i| match i {
                    Inline::Emph(x) | Inline::Strong(x) => 1 + depth_of(x),
                    Inline::Link { inlines, .. } => 1 + depth_of(inlines),
                    _ => 0,
                })
                .max()
                .unwrap_or(0)
        }

        // 300 nested <em>, well past MAX_INLINE_DEPTH.
        let n = 300;
        let xml = format!(
            "<body><p>{}deep{}</p></body>",
            "<em>".repeat(n),
            "</em>".repeat(n)
        );
        let blocks = parse_blocks(&xml);

        let inls = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(i) => Some(i),
                _ => None,
            })
            .expect("a paragraph");
        assert!(
            depth_of(inls) <= MAX_INLINE_DEPTH,
            "nesting must be flattened to the bound, got {}",
            depth_of(inls)
        );
        // Flattening preserves content: this is a fidelity bound, not a safety one.
        assert!(
            inlines_text(inls).contains("deep"),
            "flattening must keep the text"
        );
    }

    #[test]
    fn deep_anchor_nesting_is_flattened_not_preserved() {
        fn depth_of(inls: &[Inline]) -> usize {
            inls.iter()
                .map(|i| match i {
                    Inline::Emph(x) | Inline::Strong(x) => 1 + depth_of(x),
                    Inline::Link { inlines, .. } => 1 + depth_of(inlines),
                    _ => 0,
                })
                .max()
                .unwrap_or(0)
        }

        // 300 nested <a>, well past MAX_INLINE_DEPTH.
        // Anchors are invalid XHTML but quick_xml is non-validating and emits Start/End
        // for whatever the byte stream contains.
        let n = 300;
        let xml = format!(
            "<body><p>{}<a href=\"#\">text</a>{}</p></body>",
            "<a href=\"#\">".repeat(n),
            "</a>".repeat(n)
        );
        let blocks = parse_blocks(&xml);

        let inls = blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(i) => Some(i),
                _ => None,
            })
            .expect("a paragraph");
        assert!(
            depth_of(inls) <= MAX_INLINE_DEPTH,
            "anchor nesting must be flattened to the bound, got {}",
            depth_of(inls)
        );
        // Flattening preserves content: the anchor text survives.
        assert!(
            inlines_text(inls).contains("text"),
            "flattening must keep the text, got {}",
            inlines_text(inls)
        );
    }

    /// Max nesting depth of List/Footnote blocks, via an explicit stack so the
    /// checker itself cannot overflow on the very input it is checking — the
    /// same reasoning `fuzz_entry::max_inline_depth` records for its traversal.
    fn block_depth_of(blocks: &[Block]) -> usize {
        let mut max_depth = 0;
        let mut stack: Vec<(&[Block], usize)> = vec![(blocks, 0)];
        while let Some((slice, depth)) = stack.pop() {
            for b in slice {
                match b {
                    Block::List { items, .. } => {
                        max_depth = max_depth.max(depth + 1);
                        for item in items {
                            stack.push((item, depth + 1));
                        }
                    }
                    Block::Footnote { blocks, .. } => {
                        max_depth = max_depth.max(depth + 1);
                        stack.push((blocks, depth + 1));
                    }
                    // Leaves enumerated, not wildcarded: a new nesting variant
                    // must break this build rather than make the check blind.
                    Block::Heading { .. }
                    | Block::Para(_)
                    | Block::Table(_)
                    | Block::Figure { .. }
                    | Block::CodeBlock { .. }
                    | Block::MathBlock(_)
                    | Block::Raw { .. } => {}
                }
            }
        }
        max_depth
    }

    /// Collect every text run reachable anywhere in `blocks`, so a test can
    /// assert flattening kept content rather than dropping it. Iterative on the
    /// block side for the same reason `block_depth_of` is; the inline side
    /// delegates to this module's existing `inlines_text`, whose recursion is
    /// safe because the inline bound already holds.
    fn all_text(blocks: &[Block]) -> String {
        let mut out = String::new();
        let mut stack: Vec<&Block> = blocks.iter().rev().collect();
        while let Some(b) = stack.pop() {
            match b {
                Block::Heading { inlines, .. } | Block::Para(inlines) => {
                    out.push_str(&inlines_text(inlines))
                }
                Block::List { items, .. } => {
                    for item in items {
                        stack.extend(item.iter());
                    }
                }
                Block::Footnote { blocks, .. } => stack.extend(blocks.iter()),
                Block::Figure { caption, .. } => out.push_str(&inlines_text(caption)),
                Block::Table(t) => {
                    for c in t.header.iter().chain(t.rows.iter().flatten()) {
                        out.push_str(&inlines_text(c));
                    }
                }
                Block::CodeBlock { text, .. } => out.push_str(text),
                Block::MathBlock(s) | Block::Raw { note: s } => out.push_str(s),
            }
        }
        out
    }

    #[test]
    fn deeply_nested_lists_flatten_at_the_block_bound() {
        const DEPTH: usize = 3000;
        let mut html = String::from("<body><h1>T</h1>");
        for _ in 0..DEPTH {
            html.push_str("<ul><li>");
        }
        html.push_str("SENTINEL");
        for _ in 0..DEPTH {
            html.push_str("</li></ul>");
        }
        html.push_str("</body>");

        let blocks = parse_blocks(&html);

        // Pinned to equality, not just `<=`: a bound-suppressing bug that
        // stops early (e.g. an off-by-one between `>` and `>=` at the
        // suppression site, or a check that fires at depth 0) would still
        // satisfy `<= MAX_BLOCK_DEPTH`. Equality asserts the bound is
        // actually reached, not merely respected.
        assert_eq!(
            block_depth_of(&blocks),
            MAX_BLOCK_DEPTH,
            "produced block depth does not equal MAX_BLOCK_DEPTH {}",
            MAX_BLOCK_DEPTH
        );
        // The half that separates flattening from truncation: content survives.
        assert!(
            all_text(&blocks).contains("SENTINEL"),
            "the innermost text was dropped -- this bound must flatten, not truncate"
        );
    }

    #[test]
    fn blocks_after_a_deep_list_are_not_corrupted() {
        const DEPTH: usize = 3000;
        let mut html = String::from("<body><h1>T</h1>");
        for _ in 0..DEPTH {
            html.push_str("<ul><li>");
        }
        html.push_str("inner");
        for _ in 0..DEPTH {
            html.push_str("</li></ul>");
        }
        html.push_str("<p>AFTER</p></body>");

        let blocks = parse_blocks(&html);

        // A suppressed </ul> that wrongly popped a real frame would unbalance
        // `frames` and swallow this trailing paragraph into the deep list.
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::Para(inls) if inls.iter().any(|i| matches!(i, Inline::Text(t) if t == "AFTER")))),
            "trailing paragraph must be a top-level block, not captured by the deep list"
        );
    }

    #[test]
    fn an_over_deep_sublist_does_not_close_its_enclosing_list() {
        // Pairs with `blocks_after_a_deep_list_are_not_corrupted` above,
        // which catches an *under*-pop: a suppressed `</ul>` that wrongly
        // fails to no-op, draining `frames` early and swallowing trailing
        // content. That test cannot catch the opposite defect -- an
        // *over*-pop, where a suppressed `</ul>` wrongly satisfies the End
        // arm's `matches!(frames.last(), Some(BlockFrame::List { .. }))`
        // guard (true because the *enclosing* frame is also a List) and
        // pops a real frame that must survive. Over-popping a pure,
        // fully-closed deep nest is invisible to that test: an over-popping
        // End arm and a correct one drain the same real frames in the same
        // order and reach the same result (frames empty, list already
        // emitted) by the time trailing content arrives. The defect only
        // becomes observable when a real List frame must outlive a deep
        // sub-nest closed underneath it and receive a further sibling
        // afterward -- which is what this test builds: everything but the
        // outermost `<ul><li>` closes, then one more `<li>` arrives for that
        // still-open outer list.
        const DEPTH: usize = 3000;
        let mut html = String::from("<body><h1>T</h1>");
        for _ in 0..DEPTH {
            html.push_str("<ul><li>");
        }
        html.push_str("inner");
        for _ in 0..(DEPTH - 1) {
            html.push_str("</li></ul>");
        }
        html.push_str("<li>SIBLING</li></ul>");
        html.push_str("<p>AFTER</p></body>");

        let blocks = parse_blocks(&html);

        // Under an over-popping End arm, the outer list's frame is consumed
        // (and emitted) while closing the deep sub-nest beneath it, so
        // `<li>SIBLING</li>` finds no open List frame and SIBLING escapes to
        // a top-level Para instead of becoming a sibling item.
        assert!(
            !blocks.iter().any(|b| matches!(b, Block::Para(inls)
                if inls.iter().any(|i| matches!(i, Inline::Text(t) if t == "SIBLING")))),
            "SIBLING must not be a top-level Para -- that means the enclosing list closed early"
        );
        assert!(
            blocks.iter().any(|b| matches!(b, Block::List { .. })
                && all_text(std::slice::from_ref(b)).contains("SIBLING")),
            "SIBLING must be nested inside the top-level list, got {blocks:?}"
        );
        assert!(
            blocks.iter().any(|b| matches!(b, Block::Para(inls)
                if inls.iter().any(|i| matches!(i, Inline::Text(t) if t == "AFTER")))),
            "trailing paragraph must still be a top-level block"
        );
    }

    #[test]
    fn mismatched_close_inside_a_list_item_does_not_consume_the_list_tracker() {
        // A `</ul>` that does not face a List frame (here, a `<table>`
        // opened inside the `<li>` sees the stray close before its own
        // `</table>`) must not consume `list_pushed`. If it did, it would
        // steal the tracker entry belonging to the real, still-open outer
        // `<ul>`; that `<ul>`'s own (correctly matched) `</ul>` would then
        // find the tracker empty, fail to pop, and stay open until EOF --
        // mis-parenting everything parsed afterward. Checking the frame
        // guard before popping the tracker (rather than after) is what
        // makes a mismatched close leave the tracker alone.
        let blocks = parse_blocks(
            "<body><ul><li><table><tr><td>CELL</td></tr></ul></table></ul><p>AFTER</p></body>",
        );

        assert!(
            blocks.iter().any(|b| matches!(b, Block::Para(inls)
                if inls.iter().any(|i| matches!(i, Inline::Text(t) if t == "AFTER")))),
            "trailing paragraph must be top-level, not captured by a list left open by a stolen tracker entry, got {blocks:?}"
        );
        assert!(
            blocks.iter().any(|b| matches!(b, Block::List { .. })),
            "the real list must still close and emit its (mismatched-but-recovered) content, got {blocks:?}"
        );
    }

    #[test]
    fn deeply_nested_footnotes_flatten_at_the_block_bound() {
        const DEPTH: usize = 3000;
        let mut html = String::from("<body><h1>T</h1>");
        for _ in 0..DEPTH {
            html.push_str(r#"<aside epub:type="footnote"><p>x</p>"#);
        }
        html.push_str("<p>NOTESENTINEL</p>");
        for _ in 0..DEPTH {
            html.push_str("</aside>");
        }
        html.push_str("</body>");

        let blocks = parse_blocks(&html);

        // See the equality rationale in deeply_nested_lists_flatten_at_the_block_bound.
        assert_eq!(
            block_depth_of(&blocks),
            MAX_BLOCK_DEPTH,
            "produced block depth does not equal MAX_BLOCK_DEPTH {}",
            MAX_BLOCK_DEPTH
        );
        assert!(all_text(&blocks).contains("NOTESENTINEL"));
    }
}
