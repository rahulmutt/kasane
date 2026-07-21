pub(crate) mod indx;
pub(crate) mod kf8;
pub(crate) mod normalize;
pub(crate) mod palmdb;
pub(crate) mod palmdoc;
pub(crate) mod splice;

use crate::{Adapter, ParseError};
use kasane_ir::*;
use palmdb::{MobiHeader, PalmDb};

// The single MOBI 6 "file" name used in anchor maps: MOBI 6 has no spine,
// the whole book is one stream.
const FILE_KEY: &str = "book";

pub struct MobiAdapter;

impl Adapter for MobiAdapter {
    fn parse(&self, bytes: &[u8], source_path: &str) -> Result<(Document, AssetBag), ParseError> {
        let db = PalmDb::parse(bytes)?;
        let rec0 = db
            .record(0)
            .ok_or_else(|| ParseError::Malformed("empty palm database".into()))?;
        let h = palmdb::parse_header(rec0)?;
        if h.encryption != 0 {
            return Err(ParseError::Drm);
        }
        // Bomb guard on the DECLARED size, before any decompression work.
        if !crate::guard::check_expansion(bytes.len() as u64, h.text_length as u64) {
            return Err(ParseError::Bomb);
        }
        let fmt = if h.kf8.is_some() { "azw3" } else { "mobi" };
        let meta = doc_meta(bytes, rec0, source_path, fmt);
        let raw = raw_text(bytes, &db, &h)?;
        // Belt-and-suspenders: re-check ACTUAL output (covers lying headers
        // and the HUFF fallback, whose decompression we don't control).
        if !crate::guard::check_expansion(bytes.len() as u64, raw.0.len() as u64) {
            return Err(ParseError::Bomb);
        }
        match &h.kf8 {
            Some(k) => parse_kf8(raw, &db, &h, k, meta),
            None => parse_mobi6(raw, &db, &h, meta),
        }
    }
}

/// Decompressed text stream with trailing entries stripped — byte offsets are
/// exact for compression 1 (none) and 2 (PalmDoc). HUFF/CDIC (17480) falls
/// back to the mobi crate's whole-book decoder, whose output is already
/// UTF-8; filepos offsets can drift slightly there, which the splice snap
/// tolerates (degrade, don't die).
fn raw_text(bytes: &[u8], db: &PalmDb, h: &MobiHeader) -> Result<(Vec<u8>, bool), ParseError> {
    match h.compression {
        1 | 2 => {
            let mut out = Vec::new();
            for i in 1..=h.text_records as usize {
                let rec = db
                    .record(i)
                    .ok_or_else(|| ParseError::Malformed("text record missing".into()))?;
                let body = palmdb::strip_trailing(rec, h.extra_flags);
                if h.compression == 1 {
                    out.extend_from_slice(body);
                } else {
                    out.extend(palmdoc::decompress(body));
                }
                if out.len() as u64 > crate::guard::MAX_TOTAL_BYTES {
                    return Err(ParseError::Bomb);
                }
            }
            // Declared length trims zero padding in the final record.
            out.truncate((h.text_length as usize).min(out.len()));
            Ok((out, false))
        }
        17480 => {
            // mobi 0.8's `Mobi::new` takes `B: AsRef<Vec<u8>>`, not `&[u8]`
            // (the brief's reference assumed the latter) -- a copy is
            // unavoidable here.
            let m = mobi::Mobi::new(bytes.to_vec())
                .map_err(|e| ParseError::Malformed(e.to_string()))?;
            Ok((m.content_as_string_lossy().into_bytes(), true))
        }
        other => Err(ParseError::Malformed(format!(
            "unknown compression {other}"
        ))),
    }
}

fn decode_text(bytes: Vec<u8>, h: &MobiHeader, already_utf8: bool) -> String {
    if !already_utf8 && h.encoding == 1252 {
        palmdoc::cp1252_to_string(&bytes)
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

fn parse_mobi6(
    (mut raw, already_utf8): (Vec<u8>, bool),
    db: &PalmDb,
    h: &MobiHeader,
    meta: DocMeta,
) -> Result<(Document, AssetBag), ParseError> {
    // Markers first, on raw bytes: filepos targets are byte offsets into this
    // stream, and both decoding and normalization shift offsets.
    let targets = collect_filepos(&raw);
    splice::splice_markers(&mut raw, &targets, "kasane-fp-");
    let text = decode_text(raw, h, already_utf8);

    let hook = |tag: &str, attrs: &mut Vec<(String, String)>| {
        if tag == "a" {
            if let Some(i) = attrs.iter().position(|(k, _)| k == "filepos") {
                if let Some(n) = parse_digits(&attrs[i].1) {
                    attrs.retain(|(k, _)| k != "filepos" && k != "href");
                    attrs.push(("href".into(), format!("#kasane-fp-{n}")));
                }
            }
        } else if tag == "img" {
            if let Some(i) = attrs.iter().position(|(k, _)| k == "recindex") {
                if let Some(n) = parse_digits(&attrs[i].1) {
                    attrs.retain(|(k, _)| k != "recindex" && k != "src");
                    attrs.push(("src".into(), format!("kasane-rec-{n}")));
                }
            }
        }
    };
    let xhtml = normalize::normalize_html(&text, &hook);
    // Mobipocket routinely wraps a standalone image in its own paragraph
    // (`<p><img recindex="..."/></p>`), unlike the EPUB fixtures the shared
    // `epub::xhtml::xhtml_to_blocks` was tested against (always a bare
    // `<img>` under `<body>` or inside `<figure>`). That parser treats any
    // block emitted while a `<p>`'s inline collection is still open as
    // stray content to flatten into the paragraph's text (its rule for
    // content stranded inside an open inline container, e.g. a table
    // cell) -- so a lone `<img>` inside a `<p>` silently loses its
    // AssetRef and degrades to bare alt text instead of becoming a
    // `Block::Figure`. `epub::xhtml` is shared, already-tested EPUB code
    // out of scope for this task to change, so the fix is MOBI-local:
    // promote a paragraph whose *entire* content is exactly one
    // self-closing `<img.../>` to a standalone sibling, matching how the
    // parser already treats an `<img>` that is a direct child of `<body>`.
    let xhtml = unwrap_solo_images(&xhtml);

    let mut next_id = 0u32;
    let mut next_note = 1u32;
    let mut fp = crate::epub::xhtml::xhtml_to_blocks(&xhtml, "", &mut next_id, &mut next_note);
    strip_empty_anchor_links(&mut fp.blocks);

    let mut anchor_map = std::collections::HashMap::new();
    for (aid, bid) in &fp.anchors {
        anchor_map.insert((FILE_KEY.to_string(), aid.clone()), *bid);
    }
    if let Some(fh) = fp.first_heading {
        anchor_map.insert((FILE_KEY.to_string(), String::new()), fh);
    }
    let mut nodes: Vec<Node> = fp
        .blocks
        .into_iter()
        .map(|b| Node {
            block: b,
            prov: Provenance {
                source_pages: None,
                source_href: Some(FILE_KEY.to_string()),
            },
        })
        .collect();
    let no_footnotes: std::collections::HashMap<(String, String), NoteId> = Default::default();
    let no_norefs: std::collections::HashSet<(String, String)> = Default::default();
    crate::epub::fix_links(&mut nodes, &anchor_map, &no_footnotes, &no_norefs);
    let assets = collect_assets(db, h, &mut nodes);
    Ok((Document { meta, nodes }, assets))
}

/// KF8/AZW3 pipeline: reassemble skeleton+fragment parts, resolve every
/// kindle:pos href to a (part, offset) target, splice anchor markers at
/// those exact byte offsets, then parse each part through the same
/// xhtml_to_blocks + fix_links machinery the EPUB adapter uses for its
/// spine. Mirrors parse_mobi6's shape, but per-part instead of whole-book.
fn parse_kf8(
    (raw, _already_utf8): (Vec<u8>, bool), // KF8 text is always UTF-8
    db: &PalmDb,
    h: &MobiHeader,
    k: &palmdb::Kf8Indices,
    meta: DocMeta,
) -> Result<(Document, AssetBag), ParseError> {
    // An unreadable index is an unreadable container (per spec); a *lying*
    // index inside a readable one degrades per part in assemble().
    let skels = indx::skel_entries(&indx::read_index(db, k.skel_index as usize)?);
    let frags = indx::frag_entries(&indx::read_index(db, k.frag_index as usize)?);
    if skels.is_empty() {
        return Err(ParseError::Malformed("kf8: empty skeleton table".into()));
    }
    let mut asm = kf8::assemble(&raw, &skels, &frags);

    // Resolve every kindle:pos href, then splice target markers per part.
    // Marker ids embed the part-local offset, so hrefs rewrite to
    // "partNNNN.xhtml#kasane-kp-{off}" and meet their markers through the
    // ordinary (file, id) anchor map.
    let mut per_part: Vec<Vec<u64>> = vec![vec![]; asm.parts.len()];
    let mut href_map: std::collections::HashMap<String, String> = Default::default();
    for href in kf8::collect_kindle_pos(&asm) {
        if let Some((pi, off)) = kf8::resolve_kindle_pos(&href, &asm) {
            per_part[pi].push(off as u64);
            href_map.insert(
                href.clone(),
                format!("{}#kasane-kp-{off}", asm.parts[pi].name),
            );
        }
    }
    for (pi, offs) in per_part.iter().enumerate() {
        splice::splice_markers(&mut asm.parts[pi].html, offs, "kasane-kp-");
    }

    // Per-part parse: mirrors the EPUB spine loop in epub/mod.rs.
    let mut nodes: Vec<Node> = vec![];
    let mut anchor_map = std::collections::HashMap::new();
    let mut next_id = 0u32;
    let mut next_note = 1u32;
    for part in &asm.parts {
        let hook = |tag: &str, attrs: &mut Vec<(String, String)>| {
            if tag == "a" {
                if let Some(i) = attrs.iter().position(|(kk, _)| kk == "href") {
                    let v = attrs[i].1.clone();
                    if v.starts_with("kindle:") {
                        // Unresolvable kindle: URIs must not leak as external
                        // links; a fragment that misses the anchor map strips
                        // to plain text with a warning in fix_links.
                        attrs[i].1 = href_map
                            .get(&v)
                            .cloned()
                            .unwrap_or_else(|| "#kasane-unresolved".into());
                    }
                }
            } else if tag == "img" {
                if let Some(i) = attrs.iter().position(|(kk, _)| kk == "src") {
                    if let Some(n) = parse_kindle_embed(&attrs[i].1) {
                        attrs[i].1 = format!("kasane-rec-{n}");
                    }
                }
            }
        };
        let text = String::from_utf8_lossy(&part.html).into_owned();
        let xhtml = normalize::normalize_html(&text, &hook);
        // Same Mobipocket/KF8-ism as parse_mobi6: a solo `<img>` inside its
        // own `<p>` must be promoted to a standalone sibling, or the shared
        // xhtml_to_blocks parser flattens it into plain text and drops its
        // AssetRef. See unwrap_solo_images's doc comment for the full story.
        let xhtml = unwrap_solo_images(&xhtml);
        let mut fp = crate::epub::xhtml::xhtml_to_blocks(&xhtml, "", &mut next_id, &mut next_note);
        strip_empty_anchor_links(&mut fp.blocks);
        for (aid, bid) in &fp.anchors {
            anchor_map.insert((part.name.clone(), aid.clone()), *bid);
        }
        if let Some(fh) = fp.first_heading {
            anchor_map.insert((part.name.clone(), String::new()), fh);
        }
        for b in fp.blocks {
            nodes.push(Node {
                block: b,
                prov: Provenance {
                    source_pages: None,
                    source_href: Some(part.name.clone()),
                },
            });
        }
    }
    let no_footnotes: std::collections::HashMap<(String, String), NoteId> = Default::default();
    let no_norefs: std::collections::HashSet<(String, String)> = Default::default();
    crate::epub::fix_links(&mut nodes, &anchor_map, &no_footnotes, &no_norefs);
    let assets = collect_assets(db, h, &mut nodes);
    Ok((Document { meta, nodes }, assets))
}

/// "kindle:embed:XXXX?mime=..." -> 1-based resource number (base 32).
fn parse_kindle_embed(src: &str) -> Option<u64> {
    let rest = src.strip_prefix("kindle:embed:")?;
    let idx: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    indx::base32(&idx).filter(|&n| n > 0)
}

/// Rewrites `<p ...><img .../></p>` -- a `<p>` whose entire content is
/// exactly one self-closing `<img>` -- into a bare `<img .../>`, dropping
/// the paragraph wrapper. Safe to do with plain substring search because
/// `normalize_html`'s output always double-quotes attributes and escapes
/// `<`/`>` inside both text and attribute values, so a literal `</p>` or
/// `<img` can only ever appear at an actual tag boundary, never inside
/// escaped content. See the call site in `parse_mobi6` for why this exists.
fn unwrap_solo_images(xhtml: &str) -> String {
    let mut out = String::with_capacity(xhtml.len());
    let mut rest = xhtml;
    loop {
        let Some(p_pos) = rest.find("<p") else {
            out.push_str(rest);
            break;
        };
        let after_tag_name = &rest[p_pos + 2..];
        // Avoid matching "<pre" or similar; a real <p> tag is followed by
        // '>' (no attrs) or whitespace (attrs follow).
        let is_p_tag = after_tag_name.starts_with('>') || after_tag_name.starts_with(' ');
        if !is_p_tag {
            out.push_str(&rest[..p_pos + 2]);
            rest = after_tag_name;
            continue;
        }
        let Some(gt_rel) = after_tag_name.find('>') else {
            out.push_str(rest);
            break;
        };
        let open_tag_end = p_pos + 2 + gt_rel + 1;
        out.push_str(&rest[..p_pos]); // copy everything before this <p>
        let body = &rest[open_tag_end..];
        if let Some(img_end_rel) = body.strip_prefix("<img").and_then(|b| b.find("/>")) {
            let img_end = "<img".len() + img_end_rel + "/>".len();
            if body[img_end..].starts_with("</p>") {
                out.push_str(&body[..img_end]); // just the <img .../>
                rest = &body[img_end + "</p>".len()..];
                continue;
            }
        }
        // Not a solo-image paragraph: keep the open tag and move past it.
        out.push_str(&rest[p_pos..open_tag_end]);
        rest = body;
    }
    out
}

/// `splice::splice_markers` inserts bare `<a id="kasane-fp-N"></a>` /
/// `<a id="kasane-kp-N"></a>` anchor markers so filepos/kindle:pos targets
/// land in `fp.anchors`. The shared `epub::xhtml::xhtml_to_blocks` parser
/// has no way to distinguish those synthetic markers from a real (if
/// pointless) empty anchor tag, so it also emits each one as an
/// `Inline::Link { target: External(""), inlines: [] }` -- which the
/// writer renders as a bare `[]()`. That's loss-free to drop: the anchor
/// id the marker exists for is already captured in `fp.anchors`, not in
/// the inline itself. Recurses through every nested block/inline shape
/// that can carry inlines (list items, table cells, figure captions,
/// footnote bodies, emphasis/strong/link nesting) rather than
/// special-casing the top-level-or-after-heading positions the splice
/// actually uses, since that's cheap and future-proofs against the
/// marker position changing.
fn strip_empty_anchor_links(blocks: &mut Vec<Block>) {
    for block in blocks.iter_mut() {
        match block {
            Block::Heading { inlines, .. } | Block::Para(inlines) => {
                strip_empty_anchor_links_in_inlines(inlines);
            }
            Block::List { items, .. } => {
                for item in items.iter_mut() {
                    strip_empty_anchor_links(item);
                }
            }
            Block::Table(t) => {
                for cell in t.header.iter_mut() {
                    strip_empty_anchor_links_in_inlines(cell);
                }
                for row in t.rows.iter_mut() {
                    for cell in row.iter_mut() {
                        strip_empty_anchor_links_in_inlines(cell);
                    }
                }
            }
            Block::Figure { caption, .. } => strip_empty_anchor_links_in_inlines(caption),
            Block::Footnote { blocks, .. } => strip_empty_anchor_links(blocks),
            Block::CodeBlock { .. } | Block::MathBlock(_) | Block::Raw { .. } => {}
        }
    }
    // A paragraph whose only content was the marker link is now empty;
    // drop it rather than emit a blank block.
    blocks.retain(|b| !matches!(b, Block::Para(inls) if inls.is_empty()));
}

fn strip_empty_anchor_links_in_inlines(inlines: &mut Vec<Inline>) {
    for inline in inlines.iter_mut() {
        match inline {
            Inline::Emph(x) | Inline::Strong(x) => strip_empty_anchor_links_in_inlines(x),
            Inline::Link { inlines: x, .. } => strip_empty_anchor_links_in_inlines(x),
            Inline::Text(_) | Inline::Code(_) | Inline::Math(_) | Inline::FootnoteRef(_) => {}
        }
    }
    inlines.retain(|i| {
        !matches!(
            i,
            Inline::Link { target: RefTarget::External(t), inlines } if t.is_empty() && inlines.is_empty()
        )
    });
}

fn parse_digits(s: &str) -> Option<u64> {
    let t = s.trim();
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse().ok()
}

/// Every `filepos=` value in the raw stream, quoted or bare.
fn collect_filepos(raw: &[u8]) -> Vec<u64> {
    const NEEDLE: &[u8] = b"filepos=";
    let mut out = vec![];
    let mut i = 0;
    while let Some(p) = raw[i..]
        .windows(NEEDLE.len())
        .position(|w| w == NEEDLE)
        .map(|p| i + p)
    {
        let mut j = p + NEEDLE.len();
        if matches!(raw.get(j), Some(b'"') | Some(b'\'')) {
            j += 1;
        }
        let s = j;
        while raw.get(j).is_some_and(|b| b.is_ascii_digit()) {
            j += 1;
        }
        if j > s {
            if let Some(v) = std::str::from_utf8(&raw[s..j])
                .ok()
                .and_then(|d| d.parse().ok())
            {
                out.push(v);
            }
        }
        i = p + NEEDLE.len();
    }
    out
}

/// Extract each referenced `kasane-rec-{n}` image record into the AssetBag;
/// unreadable or unrecognized records degrade their Figures, mirroring the
/// EPUB adapter.
fn collect_assets(db: &PalmDb, h: &MobiHeader, nodes: &mut [Node]) -> AssetBag {
    use std::collections::HashSet;
    let mut keys = vec![];
    for n in nodes.iter() {
        crate::epub::collect_figure_keys(&n.block, &mut |k: &str| keys.push(k.to_string()));
    }
    let mut assets = AssetBag::default();
    let mut seen = HashSet::new();
    let mut failed = HashSet::new();
    for key in keys {
        if !seen.insert(key.clone()) {
            continue;
        }
        let extracted = (|| {
            let n: u32 = key.strip_prefix("kasane-rec-")?.parse().ok()?;
            let first = h.first_image_rec?;
            // recindex / kindle:embed are 1-based from the first image record
            let rec = db.record(first.checked_add(n.checked_sub(1)?)? as usize)?;
            let ext = sniff_ext(rec)?;
            let filename =
                crate::guard::safe_media_filename(&format!("img{n}.{ext}"), assets.items.len());
            assets.items.push(AssetItem {
                key: key.clone(),
                filename,
                bytes: rec.to_vec(),
            });
            Some(())
        })()
        .is_some();
        if !extracted {
            eprintln!("warning: image record unreadable, degrading figure: {key}");
            failed.insert(key);
        }
    }
    if !failed.is_empty() {
        for n in nodes.iter_mut() {
            crate::epub::degrade_failed_figures(&mut n.block, &failed);
        }
    }
    assets
}

// Only formats the writer can link as images; anything else degrades.
fn sniff_ext(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG") {
        Some("png")
    } else if b.starts_with(b"\xFF\xD8") {
        Some("jpg")
    } else if b.starts_with(b"GIF8") {
        Some("gif")
    } else if b.starts_with(b"BM") {
        Some("bmp")
    } else {
        None
    }
}

// mobi 0.8's `language()` returns a `Language` enum, not `Option<String>` as
// the brief's reference code assumed; `Neutral`/`Unknown` carry no real
// language info and degrade to `None`, everything else formats via `Debug`
// (the crate exposes no ISO-639 conversion).
fn mobi_language_to_string(lang: mobi::headers::Language) -> Option<String> {
    use mobi::headers::Language;
    match lang {
        Language::Neutral | Language::Unknown => None,
        other => Some(format!("{other:?}")),
    }
}

fn doc_meta(bytes: &[u8], rec0: &[u8], source_path: &str, fmt: &str) -> DocMeta {
    let stem = std::path::Path::new(source_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();
    // The mobi crate is consulted only for EXTH metadata; any failure
    // degrades through the chain: EXTH title -> MOBI full-name field -> stem.
    let m = mobi::Mobi::new(bytes.to_vec()).ok();
    let title = m
        .as_ref()
        .map(|m| m.title())
        .filter(|t| !t.trim().is_empty())
        .or_else(|| palmdb::full_name(rec0))
        .unwrap_or(stem);
    DocMeta {
        title,
        authors: m.as_ref().and_then(|m| m.author()).into_iter().collect(),
        language: m
            .as_ref()
            .and_then(|m| mobi_language_to_string(m.language())),
        source_format: fmt.into(),
        source_path: source_path.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kasane_ir::{Block, Inline, RefTarget};

    fn text_of(inls: &[Inline]) -> String {
        inls.iter()
            .map(|i| match i {
                Inline::Text(t) => t.clone(),
                Inline::Emph(x) | Inline::Strong(x) => text_of(x),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn minimal_mobi_full_fidelity() {
        let bytes = std::fs::read("../../tests/fixtures/mobi/minimal.mobi").unwrap();
        let (doc, assets) = MobiAdapter.parse(&bytes, "minimal.mobi").unwrap();
        assert_eq!(doc.meta.title, "Minimal Mobi");
        assert_eq!(doc.meta.source_format, "mobi");

        let heads: Vec<String> = doc
            .nodes
            .iter()
            .filter_map(|n| match &n.block {
                Block::Heading { inlines, .. } => Some(text_of(inlines)),
                _ => None,
            })
            .collect();
        assert_eq!(heads, vec!["Chapter One", "Chapter Two"]);

        // nested list: outer ul has 2 items, second item contains the inner ul
        assert!(doc.nodes.iter().any(|n| matches!(
            &n.block,
            Block::List { ordered: false, items }
                if items.len() == 2
                    && items[1].iter().any(|b| matches!(b, Block::List { .. }))
        )));

        // image extracted into the AssetBag with a sniffed png name
        assert_eq!(assets.items.len(), 1);
        assert!(assets.items[0].bytes.starts_with(b"\x89PNG"));
        assert!(assets.items[0].filename.ends_with(".png"));
        assert!(doc
            .nodes
            .iter()
            .any(|n| matches!(&n.block, Block::Figure { .. })));

        // the filepos link resolved to Chapter Two's heading BlockId
        let ch2 = doc
            .nodes
            .iter()
            .find_map(|n| match &n.block {
                Block::Heading { id, inlines, .. } if text_of(inlines) == "Chapter Two" => {
                    Some(*id)
                }
                _ => None,
            })
            .unwrap();
        let link = doc
            .nodes
            .iter()
            .find_map(|n| match &n.block {
                Block::Para(inls) => inls.iter().find_map(|i| match i {
                    Inline::Link { target, .. } => Some(target.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .expect("the filepos link must survive as a Link");
        assert!(
            matches!(link, RefTarget::Internal(b) if b == ch2),
            "filepos link must resolve to Chapter Two, got {link:?}"
        );
    }

    #[test]
    fn drm_mobi_is_rejected() {
        let bytes = std::fs::read("../../tests/fixtures/mobi/minimal-drm.mobi").unwrap();
        assert!(matches!(
            MobiAdapter.parse(&bytes, "d.mobi"),
            Err(ParseError::Drm)
        ));
    }

    #[test]
    fn adapter_for_routes_both_formats_here() {
        assert!(crate::adapter_for(crate::Format::Mobi).is_ok());
        assert!(crate::adapter_for(crate::Format::Azw3).is_ok());
    }

    #[test]
    fn unwrap_solo_images_drops_the_paragraph_wrapper() {
        assert_eq!(
            unwrap_solo_images("<p><img src=\"a.png\"/></p>"),
            "<img src=\"a.png\"/>"
        );
        assert_eq!(
            unwrap_solo_images("<p height=\"1em\"><img src=\"a.png\"/></p>"),
            "<img src=\"a.png\"/>"
        );
    }

    #[test]
    fn unwrap_solo_images_leaves_other_paragraphs_untouched() {
        assert_eq!(unwrap_solo_images("<p>text</p>"), "<p>text</p>");
        assert_eq!(
            unwrap_solo_images("<p>text <img src=\"a.png\"/></p>"),
            "<p>text <img src=\"a.png\"/></p>"
        );
        assert_eq!(unwrap_solo_images("<pre>code</pre>"), "<pre>code</pre>");
    }

    #[test]
    fn unwrap_solo_images_handles_multiple_and_adjacent_occurrences() {
        assert_eq!(
            unwrap_solo_images("a<p><img src=\"1.png\"/></p>b<p><img src=\"2.png\"/></p>c"),
            "a<img src=\"1.png\"/>b<img src=\"2.png\"/>c"
        );
    }

    #[test]
    fn lying_declared_text_length_is_a_bomb() {
        let mut bytes = std::fs::read("../../tests/fixtures/mobi/minimal.mobi").unwrap();
        // record 0 starts after the 78-byte header, 3 table entries, 2 pad bytes
        let rec0 = 78 + 3 * 8 + 2;
        bytes[rec0 + 4..rec0 + 8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            MobiAdapter.parse(&bytes, "x.mobi"),
            Err(ParseError::Bomb)
        ));
    }

    #[test]
    fn minimal_azw3_full_fidelity() {
        let bytes = std::fs::read("../../tests/fixtures/azw3/minimal.azw3").unwrap();
        let (doc, assets) = MobiAdapter.parse(&bytes, "minimal.azw3").unwrap();
        assert_eq!(doc.meta.title, "KF8 Minimal");
        assert_eq!(doc.meta.source_format, "azw3");

        let heads: Vec<String> = doc
            .nodes
            .iter()
            .filter_map(|n| match &n.block {
                Block::Heading { inlines, .. } => Some(text_of(inlines)),
                _ => None,
            })
            .collect();
        assert_eq!(heads, vec!["Part One", "Part Two"]);

        // table with detected header row
        assert!(doc.nodes.iter().any(|n| matches!(
            &n.block,
            Block::Table(t) if text_of(&t.header[0]) == "Name" && t.rows.len() == 1
        )));

        // code block with language
        assert!(doc.nodes.iter().any(|n| matches!(
            &n.block,
            Block::CodeBlock { lang: Some(l), text } if l == "rust" && text.contains("fn main")
        )));

        // kindle:embed image extracted
        assert_eq!(assets.items.len(), 1);
        assert!(assets.items[0].bytes.starts_with(b"\x89PNG"));

        // the kindle:pos link resolved to Part Two's heading, across parts
        let ch2 = doc
            .nodes
            .iter()
            .find_map(|n| match &n.block {
                Block::Heading { id, inlines, .. } if text_of(inlines) == "Part Two" => Some(*id),
                _ => None,
            })
            .unwrap();
        let link = doc
            .nodes
            .iter()
            .find_map(|n| match &n.block {
                Block::Para(inls) => inls.iter().find_map(|i| match i {
                    Inline::Link { target, .. } => Some(target.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .expect("kindle:pos link must survive as a Link");
        assert!(
            matches!(link, RefTarget::Internal(b) if b == ch2),
            "kindle:pos link must resolve to Part Two, got {link:?}"
        );
    }

    #[test]
    fn lying_skel_azw3_degrades_not_dies() {
        let bytes = std::fs::read("../../tests/fixtures/azw3/lying-skel.azw3").unwrap();
        let (doc, _assets) = MobiAdapter.parse(&bytes, "lying.azw3").unwrap();
        // part 0 degraded to a Raw-ish note; part 1 still parsed fully
        assert!(doc.nodes.iter().any(
            |n| matches!(&n.block, Block::Heading { inlines, .. } if text_of(inlines) == "Part Two")
        ));
    }

    // An empty-text, empty-target `Inline::Link` is exactly what the shared
    // XHTML parser emits for a bare `<a id="..."></a>` anchor marker -- the
    // writer renders it as a stray `[]()`. The anchor id itself is captured
    // separately in `fp.anchors`, so no such link should ever survive into
    // the emitted document. Walks every nested shape that can carry inlines.
    fn any_empty_anchor_link_in_inlines(inls: &[Inline]) -> bool {
        inls.iter().any(|i| match i {
            Inline::Link { target, inlines } => {
                (matches!(target, RefTarget::External(t) if t.is_empty()) && inlines.is_empty())
                    || any_empty_anchor_link_in_inlines(inlines)
            }
            Inline::Emph(x) | Inline::Strong(x) => any_empty_anchor_link_in_inlines(x),
            Inline::Text(_) | Inline::Code(_) | Inline::Math(_) | Inline::FootnoteRef(_) => false,
        })
    }

    fn any_empty_anchor_link_in_blocks(blocks: &[Block]) -> bool {
        blocks.iter().any(|b| match b {
            Block::Heading { inlines, .. } | Block::Para(inlines) => {
                any_empty_anchor_link_in_inlines(inlines)
            }
            Block::List { items, .. } => items.iter().any(|i| any_empty_anchor_link_in_blocks(i)),
            Block::Table(t) => {
                t.header.iter().any(|c| any_empty_anchor_link_in_inlines(c))
                    || t.rows
                        .iter()
                        .any(|r| r.iter().any(|c| any_empty_anchor_link_in_inlines(c)))
            }
            Block::Figure { caption, .. } => any_empty_anchor_link_in_inlines(caption),
            Block::Footnote { blocks, .. } => any_empty_anchor_link_in_blocks(blocks),
            Block::CodeBlock { .. } | Block::MathBlock(_) | Block::Raw { .. } => false,
        })
    }

    #[test]
    fn minimal_mobi_has_no_stray_anchor_marker_links() {
        let bytes = std::fs::read("../../tests/fixtures/mobi/minimal.mobi").unwrap();
        let (doc, _assets) = MobiAdapter.parse(&bytes, "minimal.mobi").unwrap();
        let blocks: Vec<Block> = doc.nodes.into_iter().map(|n| n.block).collect();
        assert!(
            !any_empty_anchor_link_in_blocks(&blocks),
            "no block should contain an empty-text empty-target anchor-marker link"
        );
    }

    #[test]
    fn minimal_azw3_has_no_stray_anchor_marker_links() {
        let bytes = std::fs::read("../../tests/fixtures/azw3/minimal.azw3").unwrap();
        let (doc, _assets) = MobiAdapter.parse(&bytes, "minimal.azw3").unwrap();
        let blocks: Vec<Block> = doc.nodes.into_iter().map(|n| n.block).collect();
        assert!(
            !any_empty_anchor_link_in_blocks(&blocks),
            "no block should contain an empty-text empty-target anchor-marker link"
        );
    }
}
