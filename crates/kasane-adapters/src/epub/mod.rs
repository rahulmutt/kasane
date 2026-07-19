mod opf;
mod xhtml;

use crate::guard::safe_entry_name;
use crate::{Adapter, ParseError};
use kasane_ir::*;
use std::io::Read;

pub struct EpubAdapter;

impl Adapter for EpubAdapter {
    fn parse(&self, bytes: &[u8], source_path: &str) -> Result<(Document, AssetBag), ParseError> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| ParseError::Malformed(e.to_string()))?;

        // Aggregate decompressed-byte accumulator: MAX_TOTAL_BYTES is an absolute cap on the
        // whole archive, not a per-entry budget, so every read_entry call shares this counter.
        let mut total_read: u64 = 0;

        // locate the OPF via META-INF/container.xml
        let container = read_entry(&mut zip, "META-INF/container.xml", &mut total_read)?;
        let opf_path =
            find_opf_path(&container).ok_or(ParseError::Malformed("no rootfile".into()))?;
        let opf_path = crate::guard::safe_entry_name(&opf_path)
            .ok_or(ParseError::Malformed("unsafe rootfile path".into()))?;
        let opf_dir = opf_path
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();

        let opf_xml = read_entry(&mut zip, &opf_path, &mut total_read)?;
        let parsed = opf::parse_opf(&opf_xml, &opf_dir);

        let mut nodes = Vec::new();
        let mut next_id = 0u32;
        for href in &parsed.spine_hrefs {
            let Some(name) = safe_entry_name(href) else {
                continue;
            };
            let Ok(xml) = read_entry(&mut zip, &name, &mut total_read) else {
                continue;
            };
            for b in xhtml::xhtml_to_blocks(&xml, &mut next_id) {
                nodes.push(Node {
                    block: b,
                    prov: Provenance {
                        source_pages: None,
                        source_href: Some(name.clone()),
                    },
                });
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
        Ok((doc, AssetBag::default()))
    }
}

fn read_entry(
    zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    name: &str,
    total_read: &mut u64,
) -> Result<String, ParseError> {
    let f = zip
        .by_name(name)
        .map_err(|_| ParseError::Malformed(format!("missing entry: {name}")))?;
    // Reject on the declared metadata first (cheap), then bound the ACTUAL read so a
    // lying/small declared size cannot lead to an unbounded decompression.
    if !crate::guard::check_expansion(f.compressed_size(), f.size()) {
        return Err(ParseError::Bomb);
    }
    let cap = crate::guard::MAX_TOTAL_BYTES;
    let mut buf = Vec::new();
    f.take(cap + 1)
        .read_to_end(&mut buf)
        .map_err(|e| ParseError::Malformed(e.to_string()))?;
    if buf.len() as u64 > cap {
        return Err(ParseError::Bomb);
    }
    // MAX_TOTAL_BYTES is documented as an absolute cap on the whole archive's decompressed
    // output, not a per-entry budget: accumulate across every read_entry call and stop once
    // the running total would exceed it, on top of the existing per-entry bound above.
    *total_read += buf.len() as u64;
    if *total_read > crate::guard::MAX_TOTAL_BYTES {
        return Err(ParseError::Bomb);
    }
    String::from_utf8(buf).map_err(|e| ParseError::Malformed(e.to_string()))
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

    /// Build a tiny in-memory zip with a single small stored entry, for exercising
    /// `read_entry`'s aggregate-cap bookkeeping without fabricating a >512 MiB fixture.
    fn tiny_zip_with_entry(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        w.start_file(name, opts).unwrap();
        std::io::Write::write_all(&mut w, contents).unwrap();
        w.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn read_entry_accumulates_total_read_across_calls() {
        let bytes = tiny_zip_with_entry("a.txt", b"hello");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut total_read: u64 = 0;

        let out = read_entry(&mut zip, "a.txt", &mut total_read).unwrap();
        assert_eq!(out, "hello");
        // Running total reflects this entry's decompressed size.
        assert_eq!(total_read, 5);
    }

    #[test]
    fn read_entry_rejects_once_aggregate_cap_is_exceeded() {
        // Even a tiny entry must be rejected once the running total (seeded here to simulate
        // prior entries already having consumed almost the whole 512 MiB absolute budget) plus
        // this entry's size would cross MAX_TOTAL_BYTES. This proves the cap is a single
        // cumulative budget shared across the whole archive, not a fresh per-entry allowance.
        let bytes = tiny_zip_with_entry("b.txt", b"hello");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut total_read: u64 = crate::guard::MAX_TOTAL_BYTES - 2; // only 2 bytes of budget left

        let result = read_entry(&mut zip, "b.txt", &mut total_read);
        assert!(matches!(result, Err(ParseError::Bomb)));
    }
}
