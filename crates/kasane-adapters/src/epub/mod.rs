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
            for b in xhtml::xhtml_to_blocks(&xml, &file_dir, &mut next_id) {
                nodes.push(Node {
                    block: b,
                    prov: Provenance {
                        source_pages: None,
                        source_href: Some(name.clone()),
                    },
                });
            }
        }

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
}
