mod content;
pub(crate) mod doc;
mod image;
mod layout;
mod outline;

use crate::ocr::{self, OcrOutcome};
use crate::outline_dup::title_line_mask;
use crate::{Adapter, ParseError};
use content::page_text_runs;
use image::extract_page_images;
use kasane_ir::*;
use layout::{group_lines, modal_body_size, page_blocks_no_headings, Line};
use outline::outline_by_page;

/// Page image kept because OCR ran but produced nothing confident.
const OCR_IMG_NOTE: &str = "page image only; OCR found no confident text";
/// `--ocr-no-image` and OCR recovered nothing: note only, no image.
const OCR_NO_TEXT_NOTE: &str = "no text recovered by OCR";
/// Legacy note for a scanned page on a build/run without OCR.
const SCANNED_NOTE: &str = "scanned page: no text layer; OCR not enabled";

pub struct PdfAdapter;

impl Adapter for PdfAdapter {
    fn parse_with(
        &self,
        bytes: &[u8],
        source_path: &str,
        opts: &crate::ParseOptions,
    ) -> Result<(Document, AssetBag), ParseError> {
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

            let mut titles: Vec<&str> = Vec::new();
            if let Some(hs) = outline.get(num) {
                for h in hs {
                    let id = BlockId(next_id);
                    next_id += 1;
                    titles.push(h.title.as_str());
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
            let page_lines = strip_title_lines(&page.lines, &titles);
            let text_blocks = page_blocks_no_headings(&page_lines, &mut next_id, effective_body);
            // Whether the page *had* recovered text, judged before the dedup: a
            // chapter-opener page whose only line is its own title must not
            // become a "scanned page, no text layer" once that line is dropped.
            let has_text = !page.lines.is_empty();
            for b in text_blocks {
                nodes.push(Node {
                    block: b,
                    prov: prov.clone(),
                });
            }

            // Images, then OCR / scanned-page handling for text-less image pages.
            let asset_mark = assets.items.len();
            let imgs = extract_page_images(&pdf, page.id, &mut assets);
            let had_image = imgs.had_image;
            let skipped = imgs.skipped;

            // OCR only a text-less page that produced a decoded raster.
            let outcome = if !has_text && !imgs.figures.is_empty() {
                opts.ocr.map(|ex| {
                    let mut lines = Vec::new();
                    for f in &imgs.figures {
                        if let Block::Figure { image, .. } = f {
                            let bytes = assets.items[image.bytes_ref].bytes.clone();
                            lines.extend(ocr::extract_guarded(ex, &bytes, &opts.ocr_opts));
                        }
                    }
                    (ocr::decide(&lines, &opts.ocr_opts), lines)
                })
            } else {
                None
            };

            let ocr_fell_back = matches!(&outcome, Some((OcrOutcome::ImageFallback, _)));
            match outcome {
                Some((OcrOutcome::Text, lines)) => {
                    assets.items.truncate(asset_mark); // drop the page images

                    // Same dedup on OCR-recovered text: this page reached OCR
                    // *because* it had no extractable text, which is exactly the
                    // case where its outline heading supplied the only heading.
                    // Strip before sizing, so the title line cannot skew the
                    // modal body size it is about to be measured against.
                    let mapped = strip_title_lines(
                        &lines.iter().map(map_ocr_line).collect::<Vec<Line>>(),
                        &titles,
                    );
                    let bs = modal_body_size(std::slice::from_ref(&mapped));
                    for b in page_blocks_no_headings(&mapped, &mut next_id, bs) {
                        nodes.push(Node {
                            block: b,
                            prov: prov.clone(),
                        });
                    }
                }
                Some((OcrOutcome::NoteOnly, _)) => {
                    assets.items.truncate(asset_mark);
                    nodes.push(Node {
                        block: Block::Raw {
                            note: OCR_NO_TEXT_NOTE.into(),
                        },
                        prov: prov.clone(),
                    });
                }
                _ => {
                    // ImageFallback, or OCR disabled: emit figures + a note.
                    for f in imgs.figures {
                        nodes.push(Node {
                            block: f,
                            prov: prov.clone(),
                        });
                    }
                    if had_image && !has_text {
                        let note = if ocr_fell_back {
                            OCR_IMG_NOTE
                        } else {
                            SCANNED_NOTE
                        };
                        nodes.push(Node {
                            block: Block::Raw { note: note.into() },
                            prov: prov.clone(),
                        });
                    }
                }
            }

            for filter in skipped {
                nodes.push(Node {
                    block: Block::Raw {
                        note: format!("image not extracted (filter: {filter})"),
                    },
                    prov: prov.clone(),
                });
            }

            // Fully empty page (no heading, text, or image) still gets represented.
            let page_has_heading = outline.contains_key(num);
            if !has_text && !had_image && !page_has_heading {
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

/// Map an OCR line into PDF's line type. PDF `Line.y` is bottom-up (larger =
/// higher); OCR `bbox.y` is top-down, so negate it to keep reading order under
/// `page_blocks_no_headings`'s gap logic. `bbox.h` is the heading-size proxy.
fn map_ocr_line(l: &ocr::OcrLine) -> Line {
    Line {
        x: l.bbox.x,
        y: -l.bbox.y,
        size: l.bbox.h,
        text: l.text.clone(),
    }
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

/// Drop the lines that merely reprint one of this page's outline titles, so a
/// bookmarked chapter title does not appear both as the spliced heading and in
/// the body below it. Paragraph breaks survive on their own: they come from the
/// `y` gap between *kept* lines, which only widens across a dropped one.
fn strip_title_lines(lines: &[Line], titles: &[&str]) -> Vec<Line> {
    if titles.is_empty() {
        return lines.to_vec();
    }
    let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    let drop = title_line_mask(&texts, titles);
    lines
        .iter()
        .zip(drop)
        .filter(|(_, d)| !d)
        .map(|(l, _)| l.clone())
        .collect()
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
    use crate::ocr::{OcrBBox, OcrLine, StubExtractor};
    use crate::ParseOptions;
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

    fn paras(doc: &Document) -> Vec<String> {
        doc.nodes
            .iter()
            .filter_map(|n| match &n.block {
                Block::Para(p) => Some(text(p)),
                _ => None,
            })
            .collect()
    }

    /// `minimal.pdf` both bookmarks "Chapter One" and prints it on page 1 at
    /// body size. With the outline suppressing size inference, that line used to
    /// fuse into the following paragraph -- "Chapter One First body line." --
    /// so the title appeared twice in the output.
    #[test]
    fn a_printed_title_is_not_repeated_under_its_outline_heading() {
        let doc = parse("minimal");
        let paras = paras(&doc);
        assert!(
            paras.iter().all(|p| !p.contains("Chapter One")),
            "paras: {paras:?}"
        );
        assert!(
            paras.iter().all(|p| !p.contains("Section Two")),
            "paras: {paras:?}"
        );
        // The body text either side of the dropped line survives intact.
        assert!(
            paras.iter().any(|p| p == "First body line."),
            "paras: {paras:?}"
        );
        assert!(
            paras.iter().any(|p| p == "Second body line."),
            "paras: {paras:?}"
        );
        // Both headings still come from the outline.
        assert_eq!(
            headings(&doc),
            vec![(1, "Chapter One".into()), (1, "Section Two".into())]
        );
    }

    #[test]
    fn font_size_fallback_when_no_outline() {
        let doc = parse("no-outline");
        assert_eq!(headings(&doc), vec![(1, "Big Title".into())]);
    }

    /// The guard-rejection seam, end to end: `cyclic-outline.pdf` carries a
    /// real page with real body text *and* an `/Outlines` graph whose item is
    /// its own `/First`. `outline_by_page` drops the hostile outline whole and
    /// returns an empty map, which this adapter reads as "no outline", so the
    /// headings that come out must be the font-size ones — the same result as
    /// `no-outline.pdf`, reached by a different route.
    #[test]
    fn font_size_fallback_when_the_outline_is_rejected() {
        let doc = parse("cyclic-outline");
        assert_eq!(headings(&doc), vec![(1, "Big Title".into())]);
        // The body text survives; the whole document is not dropped with the
        // outline.
        assert!(doc.nodes.iter().any(|n| matches!(&n.block,
            Block::Para(inlines) if text(inlines).contains("Ordinary paragraph"))));
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

    fn pdf_ocr_line(text: &str, h: f32, conf: f32) -> OcrLine {
        OcrLine {
            text: text.into(),
            bbox: OcrBBox {
                x: 0.0,
                y: 0.0,
                w: 300.0,
                h,
            },
            confidence: conf,
        }
    }

    fn parse_scanned_pdf(
        stub: &StubExtractor,
        force_text: bool,
    ) -> (Document, kasane_ir::AssetBag) {
        let bytes = std::fs::read("../../tests/fixtures/pdf/scanned.pdf").unwrap();
        let opts = ParseOptions {
            ocr: Some(stub),
            ocr_opts: crate::ocr::OcrOptions {
                force_text,
                ..Default::default()
            },
        };
        PdfAdapter.parse_with(&bytes, "scanned.pdf", &opts).unwrap()
    }

    #[test]
    fn pdf_ocr_recovers_text_and_drops_figure() {
        let stub = StubExtractor::new(vec![pdf_ocr_line(
            "recovered scanned paragraph text here",
            12.0,
            91.0,
        )]);
        let (doc, assets) = parse_scanned_pdf(&stub, false);
        assert!(
            !doc.nodes
                .iter()
                .any(|n| matches!(&n.block, Block::Figure { .. })),
            "OCR success must drop the page image"
        );
        assert!(!doc.nodes.iter().any(|n| matches!(&n.block,
            Block::Raw { note } if note.contains("scanned page"))));
        assert!(
            assets.items.is_empty(),
            "the dropped image asset must be truncated"
        );
        assert!(doc.nodes.iter().any(|n| matches!(&n.block, Block::Para(_))));
    }

    /// The dedup on the OCR path. `scanned-outline.pdf` is an image-only page
    /// bookmarked "Chapter One" — a page reaches OCR precisely when the outline
    /// supplied its only heading — and the stub reads that printed title back
    /// off the scan along with the body.
    #[test]
    fn an_ocr_recovered_title_is_not_repeated_under_its_outline_heading() {
        let stub = StubExtractor::new(vec![
            pdf_ocr_line("Chapter One", 12.0, 91.0),
            pdf_ocr_line("recovered scanned paragraph text here", 12.0, 91.0),
        ]);
        let bytes = std::fs::read("../../tests/fixtures/pdf/scanned-outline.pdf").unwrap();
        let opts = ParseOptions {
            ocr: Some(&stub),
            ocr_opts: crate::ocr::OcrOptions::default(),
        };
        let (doc, _) = PdfAdapter
            .parse_with(&bytes, "scanned-outline.pdf", &opts)
            .unwrap();

        assert_eq!(headings(&doc), vec![(1, "Chapter One".into())]);
        let paras = paras(&doc);
        assert!(
            paras.iter().all(|p| !p.contains("Chapter One")),
            "paras: {paras:?}"
        );
        assert!(
            paras
                .iter()
                .any(|p| p.contains("recovered scanned paragraph text here")),
            "paras: {paras:?}"
        );
    }

    #[test]
    fn pdf_ocr_low_confidence_keeps_figure() {
        let stub = StubExtractor::new(vec![pdf_ocr_line("garbled low conf scan line", 12.0, 18.0)]);
        let (doc, assets) = parse_scanned_pdf(&stub, false);
        assert!(doc
            .nodes
            .iter()
            .any(|n| matches!(&n.block, Block::Figure { .. })));
        assert!(doc.nodes.iter().any(|n| matches!(&n.block,
            Block::Raw { note } if note.contains("OCR found no confident text"))));
        assert_eq!(assets.items.len(), 1);
    }

    #[test]
    fn pdf_ocr_never_touches_text_pages() {
        // A born-digital fixture: OCR on must change nothing.
        let bytes = std::fs::read("../../tests/fixtures/pdf/minimal.pdf").unwrap();
        let stub = StubExtractor::new(vec![pdf_ocr_line("SHOULD NOT APPEAR", 40.0, 99.0)]);
        let opts = ParseOptions {
            ocr: Some(&stub),
            ocr_opts: crate::ocr::OcrOptions::default(),
        };
        let with_ocr = PdfAdapter
            .parse_with(&bytes, "minimal.pdf", &opts)
            .unwrap()
            .0;
        let without = PdfAdapter.parse(&bytes, "minimal.pdf").unwrap().0;
        assert_eq!(with_ocr.nodes.len(), without.nodes.len());
        assert!(!with_ocr
            .nodes
            .iter()
            .any(|n| matches!(&n.block, Block::Figure { .. })));
    }
}
