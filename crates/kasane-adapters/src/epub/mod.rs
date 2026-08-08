mod opf;
pub(crate) mod xhtml;

use crate::{Adapter, ParseError};
use kasane_ir::*;

pub struct EpubAdapter;

impl Adapter for EpubAdapter {
    fn parse_with(
        &self,
        bytes: &[u8],
        source_path: &str,
        _opts: &crate::ParseOptions,
    ) -> Result<(Document, AssetBag), ParseError> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| ParseError::Malformed(e.to_string()))?;

        // Aggregate decompressed-byte accumulator: MAX_TOTAL_BYTES is an absolute cap on the
        // whole archive, not a per-entry budget, so every read_entry_string call shares this counter.
        let mut total_read: u64 = 0;

        // locate the OPF via META-INF/container.xml
        let container =
            crate::ziputil::read_entry_string(&mut zip, "META-INF/container.xml", &mut total_read)?;
        let opf_path =
            find_opf_path(&container).ok_or(ParseError::Malformed("no rootfile".into()))?;
        // container.xml's full-path is archive-root-relative, so the base is "";
        // it is a relative IRI reference like every other href in the format, so
        // it is percent-decoded before resolution on the same terms.
        // A leading `/` (full-path="/OEBPS/content.opf") is package-absolute:
        // resolve_rel normalizes it away and accepts the rootfile, where the
        // deleted safe_entry_name rejected it outright. That loosening is
        // deliberate -- the result is still confined to the archive root, the
        // key is only ever a zip lookup and never reaches the filesystem, and
        // it matches how PPTX already resolves package-absolute targets.
        let opf_path = crate::guard::resolve_rel("", &crate::guard::percent_decode(&opf_path))
            .ok_or(ParseError::Malformed("unsafe rootfile path".into()))?;
        let opf_dir = crate::guard::parent_dir(&opf_path);

        let opf_xml = crate::ziputil::read_entry_string(&mut zip, &opf_path, &mut total_read)?;
        let parsed = opf::parse_opf(&opf_xml, &opf_dir);

        let mut nodes = Vec::new();
        let mut next_id = 0u32;
        // 1-based: the writer renders footnote markers as `[^{id.0}]`, so a
        // NoteId of 0 would render `[^0]`.
        let mut next_note = 1u32;
        let mut anchor_map: std::collections::HashMap<(String, String), BlockId> =
            std::collections::HashMap::new();
        let mut footnote_map: std::collections::HashMap<(String, String), NoteId> =
            std::collections::HashMap::new();
        let mut noteref_keys: std::collections::HashSet<(String, String)> = Default::default();
        for href in &parsed.spine_hrefs {
            // parse_opf resolved and confined these, so they are usable as zip
            // keys directly -- re-guarding here is what used to drop legal
            // chapters whose names merely contained `..`. Check the invariant
            // Opf::spine_hrefs declares, at the point that relies on it.
            debug_assert!(
                !href.split('/').any(|seg| seg == ".."),
                "spine href is not resolve_rel output, so this zip lookup is unconfined: {href}"
            );
            let name = href;
            let Ok(xml) = crate::ziputil::read_entry_string(&mut zip, name, &mut total_read) else {
                continue;
            };
            let file_dir = crate::guard::parent_dir(name);
            let fp = xhtml::xhtml_to_blocks(&xml, &file_dir, &mut next_id, &mut next_note);
            for (aid, bid) in &fp.anchors {
                anchor_map.insert((name.clone(), aid.clone()), *bid);
            }
            if let Some(fh) = fp.first_heading {
                anchor_map.insert((name.clone(), String::new()), fh);
            }
            for (fid, nid) in &fp.footnotes {
                footnote_map.insert((name.clone(), fid.clone()), *nid);
            }
            for h in &fp.noteref_hrefs {
                noteref_keys.insert((name.clone(), h.clone()));
            }
            for b in fp.blocks {
                nodes.push(Node {
                    block: b,
                    prov: Provenance {
                        source_pages: None,
                        source_href: Some(name.clone()),
                    },
                });
            }
        }
        fix_links(&mut nodes, &anchor_map, &footnote_map, &noteref_keys);
        nodes = relocate_footnotes(nodes);

        // Extract every referenced image once; remember which keys failed so their
        // Figures can degrade instead of rendering a broken link.
        let mut assets = AssetBag::default();
        let mut seen: std::collections::HashMap<String, bool> = Default::default(); // key -> readable
        for n in &nodes {
            collect_figure_keys(
                &n.block,
                &mut |key: &str| {
                    if seen.contains_key(key) {
                        return;
                    }
                    match crate::ziputil::read_entry_bytes(&mut zip, key, &mut total_read) {
                        Ok(data) => {
                            let filename =
                                crate::guard::safe_media_filename(key, assets.items.len());
                            assets.items.push(AssetItem {
                                key: key.to_string(),
                                filename,
                                bytes: data,
                            });
                            seen.insert(key.to_string(), true);
                        }
                        Err(_) => {
                            eprintln!("warning: image entry unreadable, degrading figure: {key}");
                            seen.insert(key.to_string(), false);
                        }
                    }
                },
                0,
            );
        }
        let failed: std::collections::HashSet<String> = seen
            .into_iter()
            .filter(|(_, ok)| !ok)
            .map(|(k, _)| k)
            .collect();
        if !failed.is_empty() {
            for n in &mut nodes {
                degrade_failed_figures(&mut n.block, &failed, 0);
            }
        }

        let doc = Document {
            meta: DocMeta {
                title: if parsed.title.is_empty() {
                    "Untitled".into()
                } else {
                    parsed.title
                },
                authors: parsed.authors,
                language: parsed.language,
                source_format: "epub".into(),
                source_path: source_path.to_string(),
            },
            nodes,
        };
        Ok((doc, assets))
    }
}

// Figures can sit inside lists/footnotes, so walk recursively.
pub(crate) fn collect_figure_keys(b: &Block, f: &mut impl FnMut(&str), depth: usize) {
    // Unreachable via the EPUB parser, which flattens at `xhtml::MAX_BLOCK_DEPTH` (32),
    // and the MOBI adapter also parses through that same bound. The guard stays anyway
    // to make this function's safety independent of its callers' structure, rather than
    // contingent on today's entry points.
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        return;
    }
    match b {
        Block::Figure { image, .. } => f(&image.key),
        Block::List { items, .. } => {
            for item in items {
                for ib in item {
                    collect_figure_keys(ib, f, depth + 1);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                collect_figure_keys(ib, f, depth + 1);
            }
        }
        _ => {}
    }
}

pub(crate) fn degrade_failed_figures(
    b: &mut Block,
    failed: &std::collections::HashSet<String>,
    depth: usize,
) {
    // Unreachable via the EPUB parser, which flattens at `xhtml::MAX_BLOCK_DEPTH` (32),
    // and the MOBI adapter also parses through that same bound. The guard stays anyway
    // to make this function's safety independent of its callers' structure, rather than
    // contingent on today's entry points.
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        return;
    }
    match b {
        Block::Figure { image, caption, .. } if failed.contains(&image.key) => {
            *b = if caption.is_empty() {
                Block::Raw {
                    note: format!("image unavailable: {}", image.key),
                }
            } else {
                Block::Para(std::mem::take(caption))
            };
        }
        Block::List { items, .. } => {
            for item in items {
                for ib in item {
                    degrade_failed_figures(ib, failed, depth + 1);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                degrade_failed_figures(ib, failed, depth + 1);
            }
        }
        _ => {}
    }
}

// Rewrites every `<a href>` that survived xhtml.rs as `RefTarget::External`
// (Task 6 doesn't yet know a target file's headings) into `Internal(BlockId)`
// once every spine file's anchor map is known. Scheme/empty hrefs are left
// external; a relative href with no matching entry in `map` is stripped to
// plain text with a warning rather than routed through the core's
// dangling-ref degradation -- see the Step 4 comment in the task brief for
// why this is an intentional, output-identical deviation from the spec.
pub(crate) fn fix_links(
    nodes: &mut [Node],
    map: &std::collections::HashMap<(String, String), BlockId>,
    footnote_map: &std::collections::HashMap<(String, String), NoteId>,
    noteref_keys: &std::collections::HashSet<(String, String)>,
) {
    for n in nodes {
        let file = n.prov.source_href.clone().unwrap_or_default();
        fix_block_links(&mut n.block, &file, map, footnote_map, noteref_keys, 0);
    }
}

fn fix_block_links(
    b: &mut Block,
    file: &str,
    map: &std::collections::HashMap<(String, String), BlockId>,
    footnote_map: &std::collections::HashMap<(String, String), NoteId>,
    noteref_keys: &std::collections::HashSet<(String, String)>,
    depth: usize,
) {
    // Unreachable via this crate's own parser, which flattens at
    // `xhtml::MAX_BLOCK_DEPTH` -- but this walk runs on every EPUB and MOBI
    // parse, so the guard is what makes its safety independent of who built
    // the IR rather than contingent on it.
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        return;
    }
    match b {
        Block::Para(inls) | Block::Heading { inlines: inls, .. } => {
            fix_inline_vec(inls, file, map, footnote_map, noteref_keys)
        }
        Block::List { items, .. } => {
            for item in items {
                for ib in item {
                    fix_block_links(ib, file, map, footnote_map, noteref_keys, depth + 1);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                fix_block_links(ib, file, map, footnote_map, noteref_keys, depth + 1);
            }
        }
        Block::Table(t) => {
            for row in std::iter::once(&mut t.header).chain(t.rows.iter_mut()) {
                for cell in row {
                    fix_inline_vec(cell, file, map, footnote_map, noteref_keys);
                }
            }
        }
        Block::Figure { caption, .. } => {
            fix_inline_vec(caption, file, map, footnote_map, noteref_keys)
        }
        _ => {}
    }
}

fn fix_inline_vec(
    inls: &mut Vec<Inline>,
    file: &str,
    map: &std::collections::HashMap<(String, String), BlockId>,
    footnote_map: &std::collections::HashMap<(String, String), NoteId>,
    noteref_keys: &std::collections::HashSet<(String, String)>,
) {
    let old = std::mem::take(inls);
    for i in old {
        match i {
            Inline::Emph(mut x) => {
                fix_inline_vec(&mut x, file, map, footnote_map, noteref_keys);
                inls.push(Inline::Emph(x));
            }
            Inline::Strong(mut x) => {
                fix_inline_vec(&mut x, file, map, footnote_map, noteref_keys);
                inls.push(Inline::Strong(x));
            }
            Inline::Link {
                target: RefTarget::External(h),
                inlines: mut inner,
            } => {
                fix_inline_vec(&mut inner, file, map, footnote_map, noteref_keys);
                let is_noteref = noteref_keys.contains(&(file.to_string(), h.clone()));
                if is_noteref {
                    if let Some(nid) = resolve_footnote(file, &h, footnote_map) {
                        // The link text (the marker digit) is dropped: FootnoteRef
                        // renders its own [^n] marker.
                        inls.push(Inline::FootnoteRef(nid));
                        continue;
                    }
                    // No matching aside: fall through to the ordinary internal-link path.
                }
                if h.is_empty() || crate::guard::has_scheme(&h) {
                    inls.push(Inline::Link {
                        target: RefTarget::External(h),
                        inlines: inner,
                    });
                } else {
                    match resolve_internal(file, &h, map) {
                        Some(bid) => inls.push(Inline::Link {
                            target: RefTarget::Internal(bid),
                            inlines: inner,
                        }),
                        None => {
                            eprintln!("warning: unresolved internal link '{h}' in {file}");
                            inls.extend(inner); // link text survives as plain text
                        }
                    }
                }
            }
            other => inls.push(other),
        }
    }
}

fn resolve_footnote(
    file: &str,
    href: &str,
    map: &std::collections::HashMap<(String, String), NoteId>,
) -> Option<NoteId> {
    let (path, frag) = match href.split_once('#') {
        Some((p, f)) => (p, f),
        None => (href, ""),
    };
    let target_file = if path.is_empty() {
        file.to_string()
    } else {
        // Decode after the fragment split (so a `%23` can never become a
        // delimiter) and before resolve_rel (so a `%2F` is normalized as a
        // separator). The map is keyed by decoded spine keys, so an undecoded
        // path could never match one.
        crate::guard::resolve_rel(
            &crate::guard::parent_dir(file),
            &crate::guard::percent_decode(path),
        )?
    };
    map.get(&(target_file, frag.to_string())).copied()
}

// "ch2.xhtml#s2" / "#frag" / "ch2.xhtml" -> a heading BlockId, if the target
// file is in the spine. Exact fragment first, then the file's first heading.
fn resolve_internal(
    file: &str,
    href: &str,
    map: &std::collections::HashMap<(String, String), BlockId>,
) -> Option<BlockId> {
    let (path, frag) = match href.split_once('#') {
        Some((p, f)) => (p, f),
        None => (href, ""),
    };
    let target_file = if path.is_empty() {
        file.to_string()
    } else {
        // Same ordering as resolve_footnote: fragment split, then decode, then
        // resolve_rel.
        crate::guard::resolve_rel(
            &crate::guard::parent_dir(file),
            &crate::guard::percent_decode(path),
        )?
    };
    map.get(&(target_file.clone(), frag.to_string()))
        .or_else(|| map.get(&(target_file, String::new())))
        .copied()
}

// Move each Footnote node to directly after the node holding its first
// FootnoteRef, so GFM [^n]/definition pairs land in the same emitted file
// (spec §4). Unreferenced footnotes stay where they are. Three phases because
// the common case is ref-before-aside: a single forward walk would reach the
// ref while the aside is still ahead and unparked.
fn relocate_footnotes(nodes: Vec<Node>) -> Vec<Node> {
    use std::collections::{HashMap, HashSet};
    let mut referenced: HashSet<NoteId> = HashSet::new();
    for n in &nodes {
        let mut refs = Vec::new();
        collect_note_refs(&n.block, &mut refs, &mut HashSet::new(), 0);
        referenced.extend(refs);
    }
    // Phase 1: pull out every referenced Footnote node (a Footnote block never
    // contains its own ref, so referenced => movable).
    let mut parked: HashMap<NoteId, Node> = HashMap::new();
    let mut rest: Vec<Node> = Vec::with_capacity(nodes.len());
    for n in nodes {
        match &n.block {
            Block::Footnote { id, .. } if referenced.contains(id) => {
                parked.insert(*id, n);
            }
            _ => rest.push(n),
        }
    }
    // Phase 2: append each parked note right after the node with its first
    // ref, in the order the node references them. `refs_here` is a
    // document-order Vec (deduped via a scratch seen-set), not a HashSet --
    // a node referencing 2+ footnotes must append their definitions in the
    // order they were first referenced, not std's randomized HashSet
    // iteration order (which could flip run to run).
    let mut out: Vec<Node> = Vec::with_capacity(rest.len() + parked.len());
    for n in rest {
        let mut refs_here = Vec::new();
        collect_note_refs(&n.block, &mut refs_here, &mut HashSet::new(), 0);
        out.push(n);
        for id in refs_here {
            if let Some(fnote) = parked.remove(&id) {
                out.push(fnote);
            }
        }
    }
    // Phase 3: safety net (e.g. a ref that only appears inside another parked
    // footnote's body) -- never drop content. Drained in NoteId order for
    // determinism: `parked` is a HashMap, so its `into_values()` iteration
    // order is likewise randomized.
    let mut leftover: Vec<NoteId> = parked.keys().copied().collect();
    leftover.sort_by_key(|id| id.0);
    for id in leftover {
        if let Some(fnote) = parked.remove(&id) {
            out.push(fnote);
        }
    }
    out
}

fn collect_note_refs(
    b: &Block,
    out: &mut Vec<NoteId>,
    seen: &mut std::collections::HashSet<NoteId>,
    depth: usize,
) {
    // Unreachable via the EPUB parser, which flattens at `xhtml::MAX_BLOCK_DEPTH` (32),
    // far below 128. The guard stays anyway -- it is the guard that makes the invariant
    // independent, not the parser's own flattening.
    if depth >= kasane_ir::MAX_BLOCK_DEPTH {
        return;
    }
    match b {
        Block::Para(inls) | Block::Heading { inlines: inls, .. } => inline_refs(inls, out, seen),
        Block::List { items, .. } => {
            for item in items {
                for ib in item {
                    collect_note_refs(ib, out, seen, depth + 1);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                collect_note_refs(ib, out, seen, depth + 1);
            }
        }
        Block::Table(t) => {
            for row in std::iter::once(&t.header).chain(t.rows.iter()) {
                for cell in row {
                    inline_refs(cell, out, seen);
                }
            }
        }
        Block::Figure { caption, .. } => inline_refs(caption, out, seen),
        _ => {}
    }
}

fn inline_refs(
    inls: &[Inline],
    out: &mut Vec<NoteId>,
    seen: &mut std::collections::HashSet<NoteId>,
) {
    for i in inls {
        match i {
            Inline::FootnoteRef(n) => {
                if seen.insert(*n) {
                    out.push(*n);
                }
            }
            Inline::Emph(x) | Inline::Strong(x) | Inline::Link { inlines: x, .. } => {
                inline_refs(x, out, seen)
            }
            _ => {}
        }
    }
}

fn find_opf_path(container_xml: &str) -> Option<String> {
    // crude: find full-path="..."
    let idx = container_xml.find("full-path=")?;
    let rest = &container_xml[idx + 10..];
    let q = rest.chars().next()?;
    // Slice by UTF-8 byte length of the quote char, not a fixed 1-byte offset: container.xml
    // is attacker-controlled, and a multi-byte char (e.g. '€') immediately after `full-path=`
    // would otherwise split a codepoint and panic ("byte index is not a char boundary").
    let rest = &rest[q.len_utf8()..];
    let end = rest.find(q)?; // byte-safe: `find` only ever returns a char-boundary index
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_opf_path_multibyte_quote_char_does_not_panic() {
        // container.xml is fully attacker-controlled. If the byte immediately after
        // `full-path=` starts a multi-byte UTF-8 character (e.g. '€', 3 bytes), the old code
        // sliced at a fixed byte offset of 1 (`&rest[1..]`), landing mid-codepoint and
        // panicking with "byte index 1 is not a char boundary". Use '€' itself as the
        // delimiter (attacker is free to pick any two identical bytes/chars as "quotes") to
        // exercise that path: the call must not panic, and must correctly skip past the whole
        // multi-byte delimiter to find the matching closing delimiter.
        let xml = "<rootfile full-path=\u{20ac}OEBPS/content.opf\u{20ac}/>";
        let result = find_opf_path(xml);
        assert_eq!(result, Some("OEBPS/content.opf".to_string()));
    }

    #[test]
    fn find_opf_path_normal_case_still_works() {
        let xml = r#"<container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
        assert_eq!(find_opf_path(xml), Some("OEBPS/content.opf".to_string()));
    }

    fn add<W: std::io::Write + std::io::Seek>(w: &mut zip::ZipWriter<W>, name: &str, data: &[u8]) {
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        w.start_file(name, opts).unwrap();
        std::io::Write::write_all(w, data).unwrap();
    }

    fn build_epub(chapter_xhtml: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        add(&mut w, "mimetype", b"application/epub+zip");
        add(
            &mut w,
            "META-INF/container.xml",
            br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#,
        );
        add(
            &mut w,
            "OEBPS/content.opf",
            br#"<package><metadata><dc:title>T</dc:title></metadata>
        <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
        <spine><itemref idref="c1"/></spine></package>"#,
        );
        add(&mut w, "OEBPS/ch1.xhtml", chapter_xhtml.as_bytes());
        for (name, data) in extra {
            add(&mut w, name, data);
        }
        w.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn extracts_referenced_image_into_asset_bag() {
        let bytes = build_epub(
            "<body><h1>C</h1><img src=\"images/cat.png\" alt=\"cat\"/></body>",
            &[("OEBPS/images/cat.png", b"\x89PNG\r\n\x1a\nFAKE")],
        );
        let (doc, assets) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        assert_eq!(assets.items.len(), 1);
        assert_eq!(assets.items[0].key, "OEBPS/images/cat.png");
        assert!(assets.items[0].bytes.starts_with(b"\x89PNG"));
        assert!(doc
            .nodes
            .iter()
            .any(|n| matches!(&n.block, Block::Figure { .. })));
    }

    #[test]
    fn math_survives_full_epub_pipeline() {
        let bytes = build_epub(
            "<body><h1>M</h1>\
             <p>Inline <math><msup><mi>x</mi><mn>2</mn></msup></math> here.</p>\
             <math display=\"block\"><mfrac><mn>1</mn><mn>2</mn></mfrac></math></body>",
            &[],
        );
        let (doc, _assets) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        // inline math reached a paragraph
        assert!(doc.nodes.iter().any(|n| matches!(&n.block,
            Block::Para(i) if i.iter().any(|x| matches!(x, Inline::Math(s) if s == "{x}^{2}")))));
        // display math reached a MathBlock
        assert!(doc
            .nodes
            .iter()
            .any(|n| matches!(&n.block, Block::MathBlock(s) if s == "\\frac{1}{2}")));
    }

    #[test]
    fn missing_image_degrades_to_alt_paragraph() {
        let bytes = build_epub(
            "<body><h1>C</h1><img src=\"images/gone.png\" alt=\"lost chart\"/></body>",
            &[],
        );
        let (doc, assets) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        assert!(assets.items.is_empty());
        assert!(!doc
            .nodes
            .iter()
            .any(|n| matches!(&n.block, Block::Figure { .. })));
        assert!(
            doc.nodes.iter().any(|n| matches!(&n.block,
            Block::Para(i) if i.iter().any(|x| matches!(x, Inline::Text(t) if t == "lost chart"))))
        );
    }

    #[test]
    fn same_image_referenced_twice_extracted_once() {
        let xhtml =
            "<body><h1>C</h1><img src=\"i.png\" alt=\"a\"/><img src=\"i.png\" alt=\"b\"/></body>";
        let bytes = build_epub(xhtml, &[("OEBPS/i.png", b"\x89PNG\r\n\x1a\nX")]);
        let (doc, assets) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        assert_eq!(assets.items.len(), 1);
        let figs = doc
            .nodes
            .iter()
            .filter(|n| matches!(&n.block, Block::Figure { .. }))
            .count();
        assert_eq!(figs, 2);
    }

    fn build_epub2(ch1: &str, ch2: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        add(&mut w, "mimetype", b"application/epub+zip");
        add(&mut w, "META-INF/container.xml",
            br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#);
        add(
            &mut w,
            "OEBPS/content.opf",
            br#"<package><metadata><dc:title>T</dc:title></metadata>
            <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
            <item id="c2" href="ch2.xhtml" media-type="application/xhtml+xml"/></manifest>
            <spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#,
        );
        add(&mut w, "OEBPS/ch1.xhtml", ch1.as_bytes());
        add(&mut w, "OEBPS/ch2.xhtml", ch2.as_bytes());
        w.finish().unwrap();
        buf.into_inner()
    }

    /// An EPUB whose spine mixes a filename that merely *contains* `..`, an href
    /// that legitimately walks out of the OPF directory, and one that escapes the
    /// archive root entirely.
    fn build_epub_awkward_hrefs() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        add(&mut w, "mimetype", b"application/epub+zip");
        add(&mut w, "META-INF/container.xml",
            br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#);
        add(
            &mut w,
            "OEBPS/content.opf",
            br#"<package><metadata><dc:title>T</dc:title></metadata>
            <manifest><item id="c1" href="..foo.xhtml" media-type="application/xhtml+xml"/>
            <item id="c2" href="../shared/ch.xhtml" media-type="application/xhtml+xml"/>
            <item id="c3" href="../../outside.xhtml" media-type="application/xhtml+xml"/></manifest>
            <spine><itemref idref="c1"/><itemref idref="c2"/><itemref idref="c3"/></spine></package>"#,
        );
        add(
            &mut w,
            "OEBPS/..foo.xhtml",
            b"<body><p>dotdot chapter</p></body>",
        );
        add(
            &mut w,
            "shared/ch.xhtml",
            b"<body><p>shared chapter</p></body>",
        );
        // If confinement regressed -- either the deleted join_href-style
        // concatenation, or inserting the raw href unresolved -- one of these
        // literal entries is exactly what `../../outside.xhtml` under
        // opf_dir "OEBPS" would resolve to. Without them, a regression would
        // still fail the zip lookup with "missing entry" (same outcome as a
        // properly rejected href), which makes the `outside` assertion below
        // tautological. Their presence is what lets it actually catch a
        // regression instead of just repeating "this key isn't in the
        // archive."
        add(
            &mut w,
            "OEBPS/../../outside.xhtml",
            b"<body><p>outside</p></body>",
        );
        add(
            &mut w,
            "../../outside.xhtml",
            b"<body><p>outside</p></body>",
        );
        w.finish().unwrap();
        let bytes = buf.into_inner();
        assert_literal_names_present(
            &bytes,
            &["OEBPS/../../outside.xhtml", "../../outside.xhtml"],
        );
        bytes
    }

    /// A fixture that plants traversal-named entries to keep an escaping-href
    /// assertion non-tautological only works if the archive really holds them
    /// under those literal names. `zip` 2.4.2's `start_file` stores a name
    /// verbatim (only `start_file_from_path` normalizes), but that is upstream
    /// behavior, not a guarantee we control: a future bump that normalized on
    /// write would collapse both traversal names to `outside.xhtml`, quietly
    /// turning the assertion back into a tautology while the suite stayed
    /// green. Assert the premise so it fails loudly instead.
    fn assert_literal_names_present(bytes: &[u8], expected: &[&str]) {
        let zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let names: Vec<&str> = zip.file_names().collect();
        for want in expected {
            assert!(
                names.contains(want),
                "fixture premise broken: the zip writer did not store the literal \
                 entry name {want:?} (stored names: {names:?}). The escaping-href \
                 assertion is tautological without it."
            );
        }
    }

    fn has_para_text(doc: &Document, needle: &str) -> bool {
        doc.nodes.iter().any(|n| {
            matches!(&n.block,
                Block::Para(i) if i.iter().any(|x| matches!(x, Inline::Text(t) if t == needle)))
        })
    }

    #[test]
    fn awkward_but_legal_spine_hrefs_are_read_and_escaping_ones_are_not() {
        let bytes = build_epub_awkward_hrefs();
        let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        // A filename containing `..` is not a traversal.
        assert!(
            has_para_text(&doc, "dotdot chapter"),
            "chapter at OEBPS/..foo.xhtml was dropped"
        );
        // An href that walks out of the OPF dir but stays inside the archive.
        assert!(
            has_para_text(&doc, "shared chapter"),
            "chapter at shared/ch.xhtml was dropped"
        );
        // Escaping the archive root is still rejected -- silently, not fatally:
        // the other two chapters must still convert.
        assert!(!has_para_text(&doc, "outside"));
    }

    /// An EPUB whose manifest hrefs are percent-encoded IRI references, as an
    /// ordinary authoring tool writes them for filenames containing a space or
    /// a non-ASCII character.
    fn build_epub_percent_encoded_hrefs() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        add(&mut w, "mimetype", b"application/epub+zip");
        add(&mut w, "META-INF/container.xml",
            br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#);
        add(
            &mut w,
            "OEBPS/content.opf",
            br#"<package><metadata><dc:title>T</dc:title></metadata>
            <manifest><item id="c1" href="ch%201.xhtml" media-type="application/xhtml+xml"/>
            <item id="c2" href="caf%C3%A9.xhtml" media-type="application/xhtml+xml"/>
            <item id="c3" href="100%.xhtml" media-type="application/xhtml+xml"/>
            <item id="c4" href="%2E%2E%2F%2E%2E%2Foutside.xhtml" media-type="application/xhtml+xml"/></manifest>
            <spine><itemref idref="c1"/><itemref idref="c2"/>
            <itemref idref="c3"/><itemref idref="c4"/></spine></package>"#,
        );
        add(
            &mut w,
            "OEBPS/ch 1.xhtml",
            b"<body><p>spaced chapter</p></body>",
        );
        add(
            &mut w,
            "OEBPS/café.xhtml",
            b"<body><p>accented chapter</p></body>",
        );
        add(
            &mut w,
            "OEBPS/100%.xhtml",
            b"<body><p>percent chapter</p></body>",
        );
        // Same non-tautology device as build_epub_awkward_hrefs, for the two
        // shapes a decoding regression would produce: decoding AFTER
        // resolve_rel yields "OEBPS/../../outside.xhtml" (the escape is one
        // opaque segment when the normalizer sees it), and skipping
        // confinement entirely yields "../../outside.xhtml".
        add(
            &mut w,
            "OEBPS/../../outside.xhtml",
            b"<body><p>outside</p></body>",
        );
        add(
            &mut w,
            "../../outside.xhtml",
            b"<body><p>outside</p></body>",
        );
        w.finish().unwrap();
        let bytes = buf.into_inner();
        assert_literal_names_present(
            &bytes,
            &["OEBPS/../../outside.xhtml", "../../outside.xhtml"],
        );
        bytes
    }

    #[test]
    fn percent_encoded_spine_hrefs_are_read_and_encoded_escapes_are_not() {
        let bytes = build_epub_percent_encoded_hrefs();
        let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        assert!(
            has_para_text(&doc, "spaced chapter"),
            "chapter at OEBPS/ch 1.xhtml was dropped: href=\"ch%201.xhtml\" was not decoded"
        );
        assert!(
            has_para_text(&doc, "accented chapter"),
            "chapter at OEBPS/café.xhtml was dropped: %C3%A9 did not round-trip as UTF-8"
        );
        assert!(
            has_para_text(&doc, "percent chapter"),
            "chapter at OEBPS/100%.xhtml was dropped: a literal `%` must survive verbatim"
        );
        // Decoding happens before resolve_rel, so an encoded traversal is
        // normalized and confined exactly like its literal form.
        assert!(!has_para_text(&doc, "outside"));
    }

    #[test]
    fn package_absolute_rootfile_path_is_normalized_and_accepted() {
        // A leading `/` makes full-path package-absolute. The deleted
        // safe_entry_name rejected it outright; resolve_rel normalizes it away
        // and accepts the rootfile. Pinning that here so the loosening is a
        // recorded decision rather than an accident of the refactor.
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        add(&mut w, "mimetype", b"application/epub+zip");
        add(&mut w, "META-INF/container.xml",
            br#"<container><rootfiles><rootfile full-path="/OEBPS/content.opf"/></rootfiles></container>"#);
        add(
            &mut w,
            "OEBPS/content.opf",
            br#"<package><metadata><dc:title>T</dc:title></metadata>
            <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
            <spine><itemref idref="c1"/></spine></package>"#,
        );
        add(
            &mut w,
            "OEBPS/ch1.xhtml",
            b"<body><p>absolute rootfile</p></body>",
        );
        w.finish().unwrap();
        let (doc, _) = EpubAdapter.parse(&buf.into_inner(), "b.epub").unwrap();
        assert!(
            has_para_text(&doc, "absolute rootfile"),
            "a package-absolute rootfile path should normalize to OEBPS/content.opf"
        );
    }

    #[test]
    fn package_absolute_rootfile_path_still_cannot_escape_the_root() {
        // The loosening is confined: a leading `/` resolves from the archive
        // root, it does not disable normalization or confinement.
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        add(&mut w, "mimetype", b"application/epub+zip");
        add(&mut w, "META-INF/container.xml",
            br#"<container><rootfiles><rootfile full-path="/../etc/passwd"/></rootfiles></container>"#);
        w.finish().unwrap();
        let err = EpubAdapter.parse(&buf.into_inner(), "b.epub").unwrap_err();
        assert!(
            err.to_string().contains("unsafe rootfile path"),
            "expected the guard to reject an escaping package-absolute rootfile path, got: {err}"
        );
    }

    #[test]
    fn percent_encoded_rootfile_path_locates_the_opf() {
        // OCF full-path is a relative IRI reference like every other href in
        // the format, so a space in the OPF's name is written `%20`. Left
        // undecoded the zip lookup misses and the whole book fails to convert
        // -- a ParseError rather than finding 1's silent chapter loss, but the
        // same defect class.
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        add(&mut w, "mimetype", b"application/epub+zip");
        add(&mut w, "META-INF/container.xml",
            br#"<container><rootfiles><rootfile full-path="OEBPS/my%20book.opf"/></rootfiles></container>"#);
        add(
            &mut w,
            "OEBPS/my book.opf",
            br#"<package><metadata><dc:title>T</dc:title></metadata>
            <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
            <spine><itemref idref="c1"/></spine></package>"#,
        );
        add(
            &mut w,
            "OEBPS/ch1.xhtml",
            b"<body><p>encoded rootfile</p></body>",
        );
        w.finish().unwrap();
        let (doc, _) = EpubAdapter.parse(&buf.into_inner(), "b.epub").unwrap();
        assert!(
            has_para_text(&doc, "encoded rootfile"),
            "full-path=\"OEBPS/my%20book.opf\" did not locate OEBPS/my book.opf"
        );
    }

    #[test]
    fn percent_encoded_escaping_rootfile_path_is_rejected_as_unsafe() {
        // Decoding runs before resolve_rel here too, so an encoded traversal is
        // confined exactly like its literal form rather than becoming an opaque
        // segment that happens to miss the archive.
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        add(&mut w, "mimetype", b"application/epub+zip");
        add(&mut w, "META-INF/container.xml",
            br#"<container><rootfiles><rootfile full-path="%2E%2E%2Fetc%2Fpasswd"/></rootfiles></container>"#);
        w.finish().unwrap();
        let err = EpubAdapter.parse(&buf.into_inner(), "b.epub").unwrap_err();
        assert!(
            err.to_string().contains("unsafe rootfile path"),
            "expected the guard to reject an encoded escaping rootfile path, got: {err}"
        );
    }

    #[test]
    fn empty_rootfile_path_is_rejected_as_unsafe() {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        add(&mut w, "mimetype", b"application/epub+zip");
        add(
            &mut w,
            "META-INF/container.xml",
            br#"<container><rootfiles><rootfile full-path=""/></rootfiles></container>"#,
        );
        w.finish().unwrap();
        let err = EpubAdapter.parse(&buf.into_inner(), "b.epub").unwrap_err();
        assert!(
            err.to_string().contains("unsafe rootfile path"),
            "expected the guard to reject an empty rootfile path, got: {err}"
        );
    }

    fn first_link_target(doc: &Document) -> Option<RefTarget> {
        doc.nodes.iter().find_map(|n| match &n.block {
            Block::Para(inls) => inls.iter().find_map(|i| match i {
                Inline::Link { target, .. } => Some(target.clone()),
                _ => None,
            }),
            _ => None,
        })
    }

    #[test]
    fn cross_file_link_resolves_to_internal_block_id() {
        let bytes = build_epub2(
            "<body><h1>One</h1><p><a href=\"ch2.xhtml#s2\">go</a></p></body>",
            "<body><h1>Two</h1><h2 id=\"s2\">Sect</h2><p>t</p></body>",
        );
        let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        // ch1: h1 -> BlockId(0); ch2: h1 -> 1, h2#s2 -> 2
        assert!(matches!(
            first_link_target(&doc),
            Some(RefTarget::Internal(BlockId(2)))
        ));
    }

    #[test]
    fn cross_file_link_with_percent_encoded_path_resolves_to_internal_block_id() {
        // Coherence with the manifest-side decode: the anchor map is keyed by
        // spine keys, which are now percent-decoded (`OEBPS/ch 2.xhtml`). A
        // link href is the same kind of IRI reference, so it must be decoded
        // on the same terms -- otherwise `ch%202.xhtml#s2` can never match the
        // map and the link silently degrades to plain text.
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        add(&mut w, "mimetype", b"application/epub+zip");
        add(&mut w, "META-INF/container.xml",
            br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#);
        add(
            &mut w,
            "OEBPS/content.opf",
            br#"<package><metadata><dc:title>T</dc:title></metadata>
            <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
            <item id="c2" href="ch%202.xhtml" media-type="application/xhtml+xml"/></manifest>
            <spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#,
        );
        add(
            &mut w,
            "OEBPS/ch1.xhtml",
            b"<body><h1>One</h1><p><a href=\"ch%202.xhtml#s2\">go</a></p></body>",
        );
        add(
            &mut w,
            "OEBPS/ch 2.xhtml",
            b"<body><h1>Two</h1><h2 id=\"s2\">Sect</h2><p>t</p></body>",
        );
        w.finish().unwrap();
        let (doc, _) = EpubAdapter.parse(&buf.into_inner(), "b.epub").unwrap();
        // ch1: h1 -> BlockId(0); ch2: h1 -> 1, h2#s2 -> 2
        assert!(
            matches!(
                first_link_target(&doc),
                Some(RefTarget::Internal(BlockId(2)))
            ),
            "percent-encoded link path did not resolve: got {:?}",
            first_link_target(&doc)
        );
    }

    #[test]
    fn fragmentless_and_unknown_fragment_hrefs_fall_back_to_first_heading() {
        let bytes = build_epub2(
            "<body><h1>One</h1><p><a href=\"ch2.xhtml\">a</a> <a href=\"ch2.xhtml#nope\">b</a></p></body>",
            "<body><h1>Two</h1><p>t</p></body>",
        );
        let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        let links: Vec<RefTarget> = doc
            .nodes
            .iter()
            .flat_map(|n| match &n.block {
                Block::Para(inls) => inls
                    .iter()
                    .filter_map(|i| match i {
                        Inline::Link { target, .. } => Some(target.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => vec![],
            })
            .collect();
        assert!(matches!(links[0], RefTarget::Internal(BlockId(1))));
        assert!(matches!(links[1], RefTarget::Internal(BlockId(1))));
    }

    #[test]
    fn unresolvable_internal_href_strips_to_text() {
        let bytes = build_epub2(
            "<body><h1>One</h1><p><a href=\"missing.xhtml#x\">gone link</a></p></body>",
            "<body><h1>Two</h1><p>t</p></body>",
        );
        let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        assert!(first_link_target(&doc).is_none(), "link must be stripped");
        assert!(doc.nodes.iter().any(|n| matches!(&n.block,
            Block::Para(i) if i.iter().any(|x| matches!(x, Inline::Text(t) if t == "gone link")))));
    }

    #[test]
    fn external_url_links_stay_external() {
        let bytes = build_epub2(
            "<body><h1>One</h1><p><a href=\"https://example.com\">ext</a></p></body>",
            "<body><h1>Two</h1><p>t</p></body>",
        );
        let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        assert!(
            matches!(first_link_target(&doc), Some(RefTarget::External(u)) if u == "https://example.com")
        );
    }

    // ---- EPUB3 semantic footnotes ----

    #[test]
    fn noteref_pairs_with_aside_and_relocates_definition() {
        let bytes = build_epub2(
            "<body><h1>One</h1><p>claim<a epub:type=\"noteref\" href=\"#fn1\">1</a></p>\
             <p>filler paragraph</p>\
             <aside epub:type=\"footnote\" id=\"fn1\"><p>note body</p></aside></body>",
            "<body><h1>Two</h1><p>t</p></body>",
        );
        let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        // NoteIds are 1-based at the adapter level so rendered markers read [^1].
        // The para holds a FootnoteRef, not a Link.
        let ref_idx = doc
            .nodes
            .iter()
            .position(|n| {
                matches!(&n.block,
            Block::Para(i) if i.iter().any(|x| matches!(x, Inline::FootnoteRef(NoteId(1)))))
            })
            .unwrap();
        // The Footnote block was moved to immediately after the referencing para.
        assert!(
            matches!(
                &doc.nodes[ref_idx + 1].block,
                Block::Footnote { id: NoteId(1), .. }
            ),
            "footnote must directly follow its first reference, got {:?}",
            doc.nodes[ref_idx + 1].block
        );
    }

    #[test]
    fn orphan_noteref_falls_back_to_internal_link_path() {
        let bytes = build_epub2(
            "<body><h1>One</h1><p>x<a epub:type=\"noteref\" href=\"#nosuch\">1</a></p></body>",
            "<body><h1>Two</h1><p>t</p></body>",
        );
        let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        assert!(!doc.nodes.iter().any(|n| matches!(&n.block,
            Block::Para(i) if i.iter().any(|x| matches!(x, Inline::FootnoteRef(_))))));
        // "#nosuch" resolves via first-heading fallback -> stays a link, Internal
        assert!(matches!(
            first_link_target(&doc),
            Some(RefTarget::Internal(_))
        ));
    }

    #[test]
    fn unreferenced_aside_stays_in_place() {
        let bytes = build_epub2(
            "<body><h1>One</h1><p>x</p><aside epub:type=\"footnote\" id=\"fn9\"><p>lonely</p></aside></body>",
            "<body><h1>Two</h1><p>t</p></body>",
        );
        let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        assert!(doc
            .nodes
            .iter()
            .any(|n| matches!(&n.block, Block::Footnote { .. })));
    }

    #[test]
    fn two_footnotes_from_one_paragraph_relocate_in_reference_order() {
        // Regression: relocate_footnotes' phase 2 iterated
        // `refs_here: HashSet<NoteId>` to decide the append order when a
        // single node references 2+ footnotes. std's RandomState gives a
        // HashSet no deterministic iteration order, so the two Footnote
        // definitions could land after the paragraph in either order,
        // varying run to run.
        //
        // The <aside> definitions are placed in the SAME order as the
        // paragraph's <a href> references (fn1 first, fn2 second), so that
        // NoteId assignment (by <aside> encounter order while parsing) lines
        // up with reference order: NoteId(1) is fn1's note, NoteId(2) is
        // fn2's. That makes "preserve first-reference order" and "ascending
        // NoteId" the same expectation here, so the assertion below pins a
        // single unambiguous correct order rather than conflating two
        // different notions of "correct".
        let bytes = build_epub2(
            "<body><h1>One</h1>\
             <p>claim<a epub:type=\"noteref\" href=\"#fn1\">1</a> and \
             <a epub:type=\"noteref\" href=\"#fn2\">2</a></p>\
             <aside epub:type=\"footnote\" id=\"fn1\"><p>first</p></aside>\
             <aside epub:type=\"footnote\" id=\"fn2\"><p>second</p></aside></body>",
            "<body><h1>Two</h1><p>t</p></body>",
        );
        let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
        let ref_idx = doc
            .nodes
            .iter()
            .position(|n| {
                matches!(&n.block,
                Block::Para(i) if i.iter().any(|x| matches!(x, Inline::FootnoteRef(NoteId(1)))))
            })
            .expect("a paragraph referencing NoteId(1) must exist");
        assert!(
            matches!(
                &doc.nodes[ref_idx + 1].block,
                Block::Footnote { id: NoteId(1), .. }
            ),
            "expected NoteId(1) directly after the referencing para, got {:?}",
            doc.nodes.get(ref_idx + 1).map(|n| &n.block)
        );
        assert!(
            matches!(
                &doc.nodes[ref_idx + 2].block,
                Block::Footnote { id: NoteId(2), .. }
            ),
            "expected NoteId(2) right after NoteId(1), got {:?}",
            doc.nodes.get(ref_idx + 2).map(|n| &n.block)
        );
    }
}
