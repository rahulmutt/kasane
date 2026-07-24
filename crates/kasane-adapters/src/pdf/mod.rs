mod content;
pub(crate) mod doc;
mod image;
mod layout;
mod outline;

use crate::{Adapter, ParseError};
use content::page_text_runs;
use image::extract_page_images;
use kasane_ir::*;
use layout::{group_lines, modal_body_size, page_blocks_no_headings, Line};
use outline::outline_by_page;

pub struct PdfAdapter;

impl Adapter for PdfAdapter {
    fn parse_with(
        &self,
        bytes: &[u8],
        source_path: &str,
        opts: &crate::ParseOptions,
    ) -> Result<(Document, AssetBag), ParseError> {
        let _ = opts;
        let pdf = doc::open(bytes)?;
        let page_list = doc::pages(&pdf);
        let outline = outline_by_page(&pdf);

        // First pass: group each page's text into lines (needed for the doc-wide body size).
        let page_lines: Vec<(u32, Line0)> = page_list
            .iter()
            .map(|&(num, id)| {
                (
                    num,
                    Line0 {
                        id,
                        lines: group_lines(page_text_runs(&pdf, id)),
                    },
                )
            })
            .collect();
        let all_lines: Vec<Vec<Line>> = page_lines.iter().map(|(_, p)| p.lines.clone()).collect();
        let body_size = modal_body_size(&all_lines);
        let has_outline = !outline.is_empty();

        let mut nodes = Vec::new();
        let mut next_id = 0u32;
        let mut assets = AssetBag::default();

        for (num, page) in &page_lines {
            let prov = Provenance {
                source_pages: Some((*num, *num)),
                source_href: None,
            };

            if let Some(hs) = outline.get(num) {
                for h in hs {
                    let id = BlockId(next_id);
                    next_id += 1;
                    nodes.push(Node {
                        block: Block::Heading {
                            level: h.level,
                            id,
                            inlines: vec![Inline::Text(h.title.clone())],
                        },
                        prov: prov.clone(),
                    });
                }
            }

            let effective_body = if has_outline { f32::MAX } else { body_size };
            let text_blocks = page_blocks_no_headings(&page.lines, &mut next_id, effective_body);
            let has_text = !text_blocks.is_empty();
            for b in text_blocks {
                nodes.push(Node {
                    block: b,
                    prov: prov.clone(),
                });
            }

            // Images, and a scanned-page note for text-less image pages.
            let imgs = extract_page_images(&pdf, page.id, &mut assets);
            for f in imgs.figures {
                nodes.push(Node {
                    block: f,
                    prov: prov.clone(),
                });
            }
            if imgs.had_image && !has_text {
                nodes.push(Node {
                    block: Block::Raw {
                        note: "scanned page: no text layer; OCR not enabled".into(),
                    },
                    prov: prov.clone(),
                });
            }
            for filter in imgs.skipped {
                nodes.push(Node {
                    block: Block::Raw {
                        note: format!("image not extracted (filter: {filter})"),
                    },
                    prov: prov.clone(),
                });
            }

            // Fully empty page (no heading, text, or image) still gets represented.
            let page_has_heading = outline.contains_key(num);
            if !has_text && !imgs.had_image && !page_has_heading {
                nodes.push(Node {
                    block: Block::Raw {
                        note: raw_empty_note(*num),
                    },
                    prov: prov.clone(),
                });
            }
        }

        let doc_out = Document {
            meta: DocMeta {
                title: derive_title(&pdf, source_path),
                authors: pdf_authors(&pdf),
                language: None,
                source_format: "pdf".into(),
                source_path: source_path.to_string(),
            },
            nodes,
        };
        Ok((doc_out, assets))
    }
}

/// Per-page grouped lines plus the page object id.
struct Line0 {
    id: lopdf::ObjectId,
    lines: Vec<Line>,
}

fn raw_empty_note(page: u32) -> String {
    format!("page {page}: no extractable text")
}

/// Title from the document Info dictionary, falling back to the file stem.
fn derive_title(pdf: &lopdf::Document, source_path: &str) -> String {
    if let Some(t) = info_string(pdf, b"Title") {
        if !t.trim().is_empty() {
            return t;
        }
    }
    source_path
        .rsplit(['/', '\\'])
        .next()
        .and_then(|f| f.strip_suffix(".pdf").or(Some(f)))
        .unwrap_or("document")
        .to_string()
}

fn pdf_authors(pdf: &lopdf::Document) -> Vec<String> {
    match info_string(pdf, b"Author") {
        Some(a) if !a.trim().is_empty() => vec![a],
        _ => vec![],
    }
}

/// Read a UTF-8/PDFDocEncoded string from the trailer's /Info dictionary.
fn info_string(pdf: &lopdf::Document, key: &[u8]) -> Option<String> {
    let info_ref = pdf.trailer.get(b"Info").ok()?.as_reference().ok()?;
    let dict = pdf.get_dictionary(info_ref).ok()?;
    let obj = dict.get(key).ok()?;
    let bytes = obj.as_str().ok()?;
    Some(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kasane_ir::{Block, Inline};

    fn parse(name: &str) -> Document {
        let bytes = std::fs::read(format!("../../tests/fixtures/pdf/{name}.pdf")).unwrap();
        PdfAdapter.parse(&bytes, &format!("{name}.pdf")).unwrap().0
    }
    fn text(inls: &[Inline]) -> String {
        inls.iter()
            .map(|i| match i {
                Inline::Text(t) => t.clone(),
                _ => String::new(),
            })
            .collect()
    }
    fn headings(doc: &Document) -> Vec<(u8, String)> {
        doc.nodes
            .iter()
            .filter_map(|n| match &n.block {
                Block::Heading { level, inlines, .. } => Some((*level, text(inlines))),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn outline_headings_in_order_with_page_provenance() {
        let doc = parse("minimal");
        assert_eq!(doc.meta.source_format, "pdf");
        assert_eq!(
            headings(&doc),
            vec![(1, "Chapter One".into()), (1, "Section Two".into())]
        );
        // Every node carries a source page.
        assert!(doc.nodes.iter().all(|n| n.prov.source_pages.is_some()));
        // "Section Two" heading is provenanced to page 2.
        let sec = doc
            .nodes
            .iter()
            .find(|n| {
                matches!(&n.block,
            Block::Heading { inlines, .. } if text(inlines) == "Section Two")
            })
            .unwrap();
        assert_eq!(sec.prov.source_pages, Some((2, 2)));
    }

    #[test]
    fn font_size_fallback_when_no_outline() {
        let doc = parse("no-outline");
        assert_eq!(headings(&doc), vec![(1, "Big Title".into())]);
    }

    #[test]
    fn scanned_page_yields_figure_and_note() {
        let bytes = std::fs::read("../../tests/fixtures/pdf/scanned.pdf").unwrap();
        let (doc, assets) = PdfAdapter.parse(&bytes, "scanned.pdf").unwrap();
        assert_eq!(assets.items.len(), 1);
        assert!(doc
            .nodes
            .iter()
            .any(|n| matches!(&n.block, Block::Figure { .. })));
        assert!(doc.nodes.iter().any(|n| matches!(&n.block,
            Block::Raw { note } if note.contains("scanned page"))));
    }
}
