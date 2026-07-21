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
        let fmt = if h.kf8.is_some() { "azw3" } else { "mobi" };
        let meta = doc_meta(bytes, rec0, source_path, fmt);
        let raw = raw_text(bytes, &db, &h)?;
        if h.kf8.is_some() {
            // Replaced by the KF8 pipeline in the KF8 task.
            return Err(ParseError::Malformed(
                "KF8/AZW3 parsing not yet wired".into(),
            ));
        }
        parse_mobi6(raw, &db, &h, meta)
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
    let fp = crate::epub::xhtml::xhtml_to_blocks(&xhtml, "", &mut next_id, &mut next_note);

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
}
