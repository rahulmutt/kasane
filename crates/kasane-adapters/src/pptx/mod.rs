mod rels;
mod slide;

use crate::guard::resolve_rel;
use crate::ziputil::{read_entry_bytes, read_entry_string};
use crate::{Adapter, ParseError};
use kasane_ir::*;
use rels::{parse_rels, parse_slide_order, SlideRels};
use std::collections::HashMap;

pub struct PptxAdapter;

impl Adapter for PptxAdapter {
    fn parse(&self, bytes: &[u8], source_path: &str) -> Result<(Document, AssetBag), ParseError> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| ParseError::Malformed(e.to_string()))?;
        let mut total = 0u64;

        // 1. Slide display order via presentation.xml + its rels.
        let pres = read_entry_string(&mut zip, "ppt/presentation.xml", &mut total)?;
        let order = parse_slide_order(&pres);
        let pres_rels_xml =
            read_entry_string(&mut zip, "ppt/_rels/presentation.xml.rels", &mut total)
                .unwrap_or_default();
        let mut rid_to_slide: HashMap<String, String> = HashMap::new();
        for r in parse_rels(&pres_rels_xml) {
            if r.ty.ends_with("slide") && !r.external {
                if let Some(p) = resolve_rel("ppt", &r.target) {
                    rid_to_slide.insert(r.id, p);
                }
            }
        }
        let slide_paths: Vec<String> = order
            .iter()
            .filter_map(|rid| rid_to_slide.get(rid).cloned())
            .collect();

        // 2. Each slide -> blocks (+ media + notes).
        let mut nodes = Vec::new();
        let mut assets = AssetBag::default();
        let mut seen_media: HashMap<String, String> = HashMap::new(); // archive path -> filename
        let mut next_id = 0u32;

        for (idx, spath) in slide_paths.iter().enumerate() {
            let Ok(sxml) = read_entry_string(&mut zip, spath, &mut total) else {
                // Degrade: unreadable slide still gets a heading + raw note.
                push_slide_fallback(&mut nodes, &mut next_id, idx, spath);
                continue;
            };
            let sdir = parent_dir(spath);
            let srels_path = rels_path_for(spath);
            let srels_xml =
                read_entry_string(&mut zip, &srels_path, &mut total).unwrap_or_default();
            let parsed_rels = parse_rels(&srels_xml);

            // Notes target (internal, .../notesSlide) before building SlideRels (which consumes rels).
            let notes_target = parsed_rels
                .iter()
                .find(|r| r.ty.ends_with("notesSlide") && !r.external)
                .and_then(|r| resolve_rel(&sdir, &r.target));

            let slide_rels = SlideRels::from_rels(parsed_rels, &sdir);
            let mut blocks = slide::slide_to_blocks(&sxml, &mut next_id, &slide_rels);

            // Fill an empty title heading with "Slide N".
            if let Some(Block::Heading { inlines, .. }) = blocks.first_mut() {
                if inlines.is_empty() {
                    *inlines = vec![Inline::Text(format!("Slide {}", idx + 1))];
                }
            }

            // Extract referenced media into the AssetBag (once per archive path).
            for b in &blocks {
                if let Block::Figure { image, .. } = b {
                    if !seen_media.contains_key(&image.key) {
                        if let Ok(data) = read_entry_bytes(&mut zip, &image.key, &mut total) {
                            let filename = safe_media_filename(&image.key, seen_media.len());
                            seen_media.insert(image.key.clone(), filename.clone());
                            assets.items.push(AssetItem {
                                key: image.key.clone(),
                                filename,
                                bytes: data,
                            });
                        }
                    }
                }
            }

            // Speaker notes appended under a bold "Notes" lead-in.
            if let Some(nt) = notes_target {
                if let Ok(nxml) = read_entry_string(&mut zip, &nt, &mut total) {
                    let note_blocks = slide::notes_to_blocks(&nxml);
                    if !note_blocks.is_empty() {
                        blocks.push(Block::Para(vec![Inline::Strong(vec![Inline::Text(
                            "Notes".into(),
                        )])]));
                        blocks.extend(note_blocks);
                    }
                }
            }

            for b in blocks {
                nodes.push(Node {
                    block: b,
                    prov: Provenance {
                        source_pages: None,
                        source_href: Some(spath.clone()),
                    },
                });
            }
        }

        let doc = Document {
            meta: DocMeta {
                title: derive_title(source_path),
                authors: vec![],
                language: None,
                source_format: "pptx".into(),
                source_path: source_path.to_string(),
            },
            nodes,
        };
        Ok((doc, assets))
    }
}

fn push_slide_fallback(nodes: &mut Vec<Node>, next_id: &mut u32, idx: usize, spath: &str) {
    let id = BlockId(*next_id);
    *next_id += 1;
    let prov = Provenance {
        source_pages: None,
        source_href: Some(spath.to_string()),
    };
    nodes.push(Node {
        block: Block::Heading {
            level: 1,
            id,
            inlines: vec![Inline::Text(format!("Slide {}", idx + 1))],
        },
        prov: prov.clone(),
    });
    nodes.push(Node {
        block: Block::Raw {
            note: "unparsable slide".into(),
        },
        prov,
    });
}

fn parent_dir(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(d, _)| d.to_string())
        .unwrap_or_default()
}

// "ppt/slides/slide1.xml" -> "ppt/slides/_rels/slide1.xml.rels"
fn rels_path_for(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, file)) => format!("{}/_rels/{}.rels", dir, file),
        None => format!("_rels/{}.rels", path),
    }
}

fn safe_media_filename(archive_path: &str, n: usize) -> String {
    let base = archive_path.rsplit('/').next().unwrap_or("image");
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Prefix an index to guarantee uniqueness even if basenames collide across dirs.
    format!(
        "{:03}-{}",
        n,
        if cleaned.is_empty() {
            "image".into()
        } else {
            cleaned
        }
    )
}

fn derive_title(source_path: &str) -> String {
    let stem = source_path
        .rsplit('/')
        .next()
        .and_then(|f| f.rsplit_once('.').map(|(s, _)| s).or(Some(f)))
        .unwrap_or("Untitled");
    if stem.is_empty() {
        "Untitled".into()
    } else {
        stem.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Adapter;

    fn add<W: std::io::Write + std::io::Seek>(w: &mut zip::ZipWriter<W>, name: &str, data: &[u8]) {
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        w.start_file(name, opts).unwrap();
        std::io::Write::write_all(w, data).unwrap();
    }

    fn build_pptx() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        add(&mut w, "[Content_Types].xml", b"<Types/>");
        // presentation lists slide rId3 THEN rId2 -> display order is slide2 then slide1
        add(
            &mut w,
            "ppt/presentation.xml",
            br#"<p:presentation xmlns:r="r">
          <p:sldIdLst><p:sldId r:id="rId3"/><p:sldId r:id="rId2"/></p:sldIdLst>
          </p:presentation>"#,
        );
        add(
            &mut w,
            "ppt/_rels/presentation.xml.rels",
            br#"<Relationships>
          <Relationship Id="rId2" Type="x/slide" Target="slides/slide1.xml"/>
          <Relationship Id="rId3" Type="x/slide" Target="slides/slide2.xml"/>
          </Relationships>"#,
        );
        add(
            &mut w,
            "ppt/slides/slide1.xml",
            br#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><a:r><a:t>First</a:t></a:r></a:p></p:txBody></p:sp>
          <p:pic><p:nvPicPr><p:cNvPr id="5" name="P" descr="pic"/></p:nvPicPr>
          <p:blipFill><a:blip r:embed="rId9"/></p:blipFill></p:pic>
          </p:spTree></p:cSld></p:sld>"#,
        );
        add(
            &mut w,
            "ppt/slides/_rels/slide1.xml.rels",
            br#"<Relationships>
          <Relationship Id="rId9" Type="x/image" Target="../media/image1.png"/>
          <Relationship Id="rId8" Type="x/notesSlide" Target="../notesSlides/notesSlide1.xml"/>
          </Relationships>"#,
        );
        add(
            &mut w,
            "ppt/slides/slide2.xml",
            br#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><a:r><a:t>no title here</a:t></a:r></a:p></p:txBody></p:sp>
          </p:spTree></p:cSld></p:sld>"#,
        );
        add(
            &mut w,
            "ppt/notesSlides/notesSlide1.xml",
            br#"<p:notes xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><a:r><a:t>speaker note</a:t></a:r></a:p></p:txBody></p:sp>
          </p:spTree></p:cSld></p:notes>"#,
        );
        add(&mut w, "ppt/media/image1.png", b"\x89PNG\r\n\x1a\nFAKE");
        w.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn parses_pptx_in_display_order_with_media_and_notes() {
        use kasane_ir::Block;
        let bytes = build_pptx();
        let (doc, assets) = PptxAdapter.parse(&bytes, "deck.pptx").unwrap();
        assert_eq!(doc.meta.source_format, "pptx");

        // Display order: slide2 (rId3) comes before slide1 (rId2). slide2 has no title
        // -> "Slide 1" fallback; slide1's title is "First".
        let headings: Vec<String> = doc
            .nodes
            .iter()
            .filter_map(|n| match &n.block {
                Block::Heading { inlines, .. } => Some(
                    inlines
                        .iter()
                        .map(|i| {
                            if let kasane_ir::Inline::Text(t) = i {
                                t.clone()
                            } else {
                                String::new()
                            }
                        })
                        .collect(),
                ),
                _ => None,
            })
            .collect();
        assert_eq!(headings, vec!["Slide 1".to_string(), "First".to_string()]);

        // media extracted into the AssetBag
        assert_eq!(assets.items.len(), 1);
        assert_eq!(assets.items[0].key, "ppt/media/image1.png");
        assert!(assets.items[0].bytes.starts_with(b"\x89PNG"));

        // speaker note appended under a bold "Notes" lead-in
        let has_notes_leadin = doc.nodes.iter().any(|n| {
            matches!(&n.block,
            Block::Para(inls) if inls.iter().any(|i| matches!(i, kasane_ir::Inline::Strong(x)
                if matches!(x.first(), Some(kasane_ir::Inline::Text(t)) if t == "Notes"))))
        });
        assert!(has_notes_leadin, "expected a **Notes** lead-in");
    }
}
