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

        // locate the OPF via META-INF/container.xml
        let container = read_entry(&mut zip, "META-INF/container.xml")?;
        let opf_path =
            find_opf_path(&container).ok_or(ParseError::Malformed("no rootfile".into()))?;
        let opf_path = crate::guard::safe_entry_name(&opf_path)
            .ok_or(ParseError::Malformed("unsafe rootfile path".into()))?;
        let opf_dir = opf_path
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();

        let opf_xml = read_entry(&mut zip, &opf_path)?;
        let parsed = opf::parse_opf(&opf_xml, &opf_dir);

        let mut nodes = Vec::new();
        let mut next_id = 0u32;
        for href in &parsed.spine_hrefs {
            let Some(name) = safe_entry_name(href) else {
                continue;
            };
            let xml = match read_entry(&mut zip, &name) {
                Ok(x) => x,
                Err(_) => continue,
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
    String::from_utf8(buf).map_err(|e| ParseError::Malformed(e.to_string()))
}

fn find_opf_path(container_xml: &str) -> Option<String> {
    // crude: find full-path="..."
    let idx = container_xml.find("full-path=")?;
    let rest = &container_xml[idx + 10..];
    let q = rest.chars().next()?;
    let rest = &rest[1..];
    let end = rest.find(q)?;
    Some(rest[..end].to_string())
}
