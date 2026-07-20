mod opf;
mod xhtml;

use crate::guard::safe_entry_name;
use crate::{Adapter, ParseError};
use kasane_ir::*;

pub struct EpubAdapter;

impl Adapter for EpubAdapter {
    fn parse(&self, bytes: &[u8], source_path: &str) -> Result<(Document, AssetBag), ParseError> {
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
        let opf_path = crate::guard::safe_entry_name(&opf_path)
            .ok_or(ParseError::Malformed("unsafe rootfile path".into()))?;
        let opf_dir = opf_path
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();

        let opf_xml = crate::ziputil::read_entry_string(&mut zip, &opf_path, &mut total_read)?;
        let parsed = opf::parse_opf(&opf_xml, &opf_dir);

        let mut nodes = Vec::new();
        let mut next_id = 0u32;
        let mut anchor_map: std::collections::HashMap<(String, String), BlockId> =
            std::collections::HashMap::new();
        for href in &parsed.spine_hrefs {
            let Some(name) = safe_entry_name(href) else {
                continue;
            };
            let Ok(xml) = crate::ziputil::read_entry_string(&mut zip, &name, &mut total_read)
            else {
                continue;
            };
            let file_dir = name
                .rsplit_once('/')
                .map(|(d, _)| d.to_string())
                .unwrap_or_default();
            let fp = xhtml::xhtml_to_blocks(&xml, &file_dir, &mut next_id);
            for (aid, bid) in &fp.anchors {
                anchor_map.insert((name.clone(), aid.clone()), *bid);
            }
            if let Some(fh) = fp.first_heading {
                anchor_map.insert((name.clone(), String::new()), fh);
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
        fix_links(&mut nodes, &anchor_map);

        // Extract every referenced image once; remember which keys failed so their
        // Figures can degrade instead of rendering a broken link.
        let mut assets = AssetBag::default();
        let mut seen: std::collections::HashMap<String, bool> = Default::default(); // key -> readable
        for n in &nodes {
            collect_figure_keys(&n.block, &mut |key: &str| {
                if seen.contains_key(key) {
                    return;
                }
                match crate::ziputil::read_entry_bytes(&mut zip, key, &mut total_read) {
                    Ok(data) => {
                        let filename = crate::guard::safe_media_filename(key, assets.items.len());
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
            });
        }
        let failed: std::collections::HashSet<String> = seen
            .into_iter()
            .filter(|(_, ok)| !ok)
            .map(|(k, _)| k)
            .collect();
        if !failed.is_empty() {
            for n in &mut nodes {
                degrade_failed_figures(&mut n.block, &failed);
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
fn collect_figure_keys(b: &Block, f: &mut impl FnMut(&str)) {
    match b {
        Block::Figure { image, .. } => f(&image.key),
        Block::List { items, .. } => {
            for item in items {
                for ib in item {
                    collect_figure_keys(ib, f);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                collect_figure_keys(ib, f);
            }
        }
        _ => {}
    }
}

fn degrade_failed_figures(b: &mut Block, failed: &std::collections::HashSet<String>) {
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
                    degrade_failed_figures(ib, failed);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                degrade_failed_figures(ib, failed);
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
fn fix_links(nodes: &mut [Node], map: &std::collections::HashMap<(String, String), BlockId>) {
    for n in nodes {
        let file = n.prov.source_href.clone().unwrap_or_default();
        fix_block_links(&mut n.block, &file, map);
    }
}

fn fix_block_links(
    b: &mut Block,
    file: &str,
    map: &std::collections::HashMap<(String, String), BlockId>,
) {
    match b {
        Block::Para(inls) | Block::Heading { inlines: inls, .. } => fix_inline_vec(inls, file, map),
        Block::List { items, .. } => {
            for item in items {
                for ib in item {
                    fix_block_links(ib, file, map);
                }
            }
        }
        Block::Footnote { blocks, .. } => {
            for ib in blocks {
                fix_block_links(ib, file, map);
            }
        }
        Block::Table(t) => {
            for row in std::iter::once(&mut t.header).chain(t.rows.iter_mut()) {
                for cell in row {
                    fix_inline_vec(cell, file, map);
                }
            }
        }
        Block::Figure { caption, .. } => fix_inline_vec(caption, file, map),
        _ => {}
    }
}

fn fix_inline_vec(
    inls: &mut Vec<Inline>,
    file: &str,
    map: &std::collections::HashMap<(String, String), BlockId>,
) {
    let old = std::mem::take(inls);
    for i in old {
        match i {
            Inline::Emph(mut x) => {
                fix_inline_vec(&mut x, file, map);
                inls.push(Inline::Emph(x));
            }
            Inline::Strong(mut x) => {
                fix_inline_vec(&mut x, file, map);
                inls.push(Inline::Strong(x));
            }
            Inline::Link {
                target: RefTarget::External(h),
                inlines: mut inner,
            } => {
                fix_inline_vec(&mut inner, file, map);
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
        crate::guard::resolve_rel(&crate::guard::parent_dir(file), path)?
    };
    map.get(&(target_file.clone(), frag.to_string()))
        .or_else(|| map.get(&(target_file, String::new())))
        .copied()
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
}
