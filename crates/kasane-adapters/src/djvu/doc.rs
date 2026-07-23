//! The sole seam over the `djvu-rs` crate. Everything else in `djvu/` consumes
//! the port types defined here, never `djvu-rs` directly.

use crate::guard::MAX_TOTAL_BYTES;
use crate::ParseError;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Recursion cap for the zone and bookmark trees; deeper nodes are dropped
/// rather than blowing the stack on a hostile file. `text.rs` imports this same
/// constant so the two caps share one value. Their *effective* depths can still
/// differ by one: `root_zone` wraps multiple top-level zones in a synthetic
/// `Page`, so a node kept here at conversion-depth 64 is seen by `text.rs` at
/// flatten-depth 65 and dropped there. That is the safe direction — the
/// flattening cap is the stricter of the two, so nothing slips past a guard.
pub(crate) const MAX_ZONE_DEPTH: usize = 64;

/// Node budget for one page's zone tree. A depth cap alone does not bound a
/// *wide* tree: a 256 MB `TXTz` can encode on the order of 10^7 zones, all of
/// which we would otherwise clone into a parallel `Zone` tree before the
/// downstream byte guard ever sees them. Two million nodes is far past any real
/// scanned page (a dense page is ~10^4 zones) while bounding the clone at a few
/// hundred MB of `Zone` structs plus their text — held for one page at a time,
/// with cumulative text separately capped against `MAX_TOTAL_BYTES`. Exhausting
/// the budget truncates the tree — degrade, don't die.
const MAX_ZONE_NODES: usize = 2_000_000;

/// Node budget for the document outline, same rationale. Outlines are orders of
/// magnitude smaller than text layers, so the bound is correspondingly tighter.
const MAX_BOOKMARK_NODES: usize = 100_000;

/// Spec-mandated rejection text for indirect (multi-file) documents. It must
/// not read as "unsupported"/"DRM"/"encrypted" — those map to other exit codes.
const INDIRECT_MSG: &str = "indirect multi-file DjVu not supported; provide the bundled document";

/// An axis-aligned bounding box in page pixel coordinates.
#[derive(Clone, Copy, Debug)]
pub struct BBox {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl BBox {
    /// Box height; used as a font-size proxy for heading inference.
    pub fn height(&self) -> f32 {
        (self.y1 - self.y0).abs()
    }
}

/// A node in the DjVu hidden-text zone hierarchy, normalized to our own type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneKind {
    Page,
    Column,
    Region,
    Para,
    Line,
    Word,
    Char,
    /// No `djvu-rs` kind maps here today; kept so the port covers the format.
    #[allow(dead_code)]
    Other,
}

/// A text-layer zone: a container (Page/Column/Region/Para/Line) or a leaf
/// (Word/Char). Leaves carry `text`; containers usually have `text == ""`.
#[derive(Clone, Debug)]
pub struct Zone {
    pub kind: ZoneKind,
    pub bbox: BBox,
    pub text: String,
    pub children: Vec<Zone>,
}

/// One NAVM outline entry, resolved to a 1-based destination page.
#[derive(Clone, Debug)]
pub struct Bookmark {
    pub title: String,
    pub page: u32,
    pub children: Vec<Bookmark>,
}

/// Opaque handle over the parsed `djvu-rs` document.
pub struct DjvuDoc {
    inner: djvu_rs::DjVuDocument,
}

/// Run a `djvu-rs` call, turning a panic into `ParseError::Malformed` so a bug
/// in a young dependency degrades instead of crashing the process.
fn guard_panic<T>(f: impl FnOnce() -> Result<T, ParseError>) -> Result<T, ParseError> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => Err(ParseError::Malformed("djvu decode panicked".into())),
    }
}

/// Map a `djvu-rs` document error, translating the indirect-resolution family
/// into the spec's rejection message.
fn map_doc_error(e: djvu_rs::DocError) -> ParseError {
    match e {
        djvu_rs::DocError::NoResolver | djvu_rs::DocError::IndirectResolve(_) => {
            ParseError::Malformed(INDIRECT_MSG.into())
        }
        other => ParseError::Malformed(other.to_string()),
    }
}

/// `true` when the document's component directory says its pages are bundled
/// in this file. A bare single-page `FORM:DJVU` has no `DIRM` at all, which is
/// bundled by construction.
fn is_bundled(doc: &djvu_rs::DjVuDocument) -> bool {
    match doc.raw_chunk(b"DIRM") {
        // DIRM flags byte: bit 7 set = bundled, clear = indirect.
        Some(dirm) => dirm.first().is_none_or(|b| b & 0x80 != 0),
        None => true,
    }
}

pub fn open(bytes: &[u8]) -> Result<DjvuDoc, ParseError> {
    if bytes.len() as u64 > MAX_TOTAL_BYTES {
        return Err(ParseError::Bomb);
    }
    guard_panic(|| {
        let inner = djvu_rs::DjVuDocument::parse(bytes).map_err(map_doc_error)?;
        // Indirect (multi-file) documents keep their pages in sibling files we
        // were never handed; reject rather than emit a hollow document.
        if !is_bundled(&inner) {
            return Err(ParseError::Malformed(INDIRECT_MSG.into()));
        }
        Ok(DjvuDoc { inner })
    })
}

pub fn page_count(doc: &DjvuDoc) -> u32 {
    // Deliberately the one `djvu-rs` call outside `guard_panic`: upstream this is
    // `pages.len()`, which is infallible and cannot panic. Do not "fix" it.
    doc.inner.page_count() as u32
}

/// Text-layer zone tree for a 1-based `page`; `None` when the page is missing,
/// has no hidden text, or fails to decode (degrade, don't die).
pub fn page_text(doc: &DjvuDoc, page: u32) -> Option<Zone> {
    if page == 0 {
        return None;
    }
    guard_panic(|| {
        let Ok(p) = doc.inner.page((page - 1) as usize) else {
            return Ok(None);
        };
        let Ok(Some(layer)) = p.text_layer() else {
            return Ok(None);
        };
        Ok(root_zone(&layer))
    })
    .ok()
    .flatten()
}

/// A `TextLayer` holds a *forest*. Collapse it to the single root the rest of
/// the adapter expects: a lone zone is used as-is, several are wrapped in a
/// synthetic `Page` spanning their union.
fn root_zone(layer: &djvu_rs::text::TextLayer) -> Option<Zone> {
    let mut budget = MAX_ZONE_NODES;
    let mut children: Vec<Zone> = layer
        .zones
        .iter()
        .filter_map(|z| convert_zone(z, 0, &mut budget))
        .collect();
    match children.len() {
        0 => None,
        1 => children.pop(),
        _ => {
            let bbox = children
                .iter()
                .map(|c| c.bbox)
                .reduce(|a, b| BBox {
                    x0: a.x0.min(b.x0),
                    y0: a.y0.min(b.y0),
                    x1: a.x1.max(b.x1),
                    y1: a.y1.max(b.y1),
                })
                .unwrap_or(BBox {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 0.0,
                    y1: 0.0,
                });
            Some(Zone {
                kind: ZoneKind::Page,
                bbox,
                text: String::new(),
                children,
            })
        }
    }
}

/// Map a `djvu-rs` text zone into our `Zone`, bounding recursion depth and the
/// total node count. `budget` is decremented per converted node; once it hits
/// zero the remaining nodes are dropped and what was built so far is returned.
fn convert_zone(z: &djvu_rs::text::TextZone, depth: usize, budget: &mut usize) -> Option<Zone> {
    if depth > MAX_ZONE_DEPTH || *budget == 0 {
        return None;
    }
    *budget -= 1;
    // `Rect` is top-left origin, width/height in page pixels.
    let r = &z.rect;
    let children = z
        .children
        .iter()
        .filter_map(|c| convert_zone(c, depth + 1, budget))
        .collect();
    Some(Zone {
        kind: map_kind(z.kind),
        bbox: BBox {
            x0: r.x as f32,
            y0: r.y as f32,
            x1: (r.x + r.width) as f32,
            y1: (r.y + r.height) as f32,
        },
        text: z.text.clone(),
        children,
    })
}

fn map_kind(k: djvu_rs::text::TextZoneKind) -> ZoneKind {
    use djvu_rs::text::TextZoneKind as Z;
    match k {
        Z::Page => ZoneKind::Page,
        Z::Column => ZoneKind::Column,
        Z::Region => ZoneKind::Region,
        Z::Para => ZoneKind::Para,
        Z::Line => ZoneKind::Line,
        Z::Word => ZoneKind::Word,
        Z::Character => ZoneKind::Char,
    }
}

/// NAVM outline roots; empty when the document has none.
pub fn bookmarks(doc: &DjvuDoc) -> Vec<Bookmark> {
    guard_panic(|| {
        let mut budget = MAX_BOOKMARK_NODES;
        Ok(doc
            .inner
            .bookmarks()
            .iter()
            .filter_map(|b| convert_bookmark(b, 0, &mut budget))
            .collect())
    })
    .unwrap_or_default()
}

/// Same contract as `convert_zone`: depth-capped, node-budgeted, and truncating
/// (not erroring) when the budget runs out.
fn convert_bookmark(
    b: &djvu_rs::DjVuBookmark,
    depth: usize,
    budget: &mut usize,
) -> Option<Bookmark> {
    if depth > MAX_ZONE_DEPTH || *budget == 0 {
        return None;
    }
    *budget -= 1;
    let title = b.title.trim().to_string();
    if title.is_empty() {
        return None;
    }
    let children = b
        .children
        .iter()
        .filter_map(|c| convert_bookmark(c, depth + 1, budget))
        .collect();
    Some(Bookmark {
        title,
        page: page_from_url(&b.url),
        children,
    })
}

/// DjVu internal URLs encode the destination as `#<1-based page>`. Anything we
/// cannot resolve to a page number becomes 0, which `outline.rs` drops.
fn page_from_url(url: &str) -> u32 {
    url.trim()
        .strip_prefix('#')
        .and_then(|n| n.parse::<u32>().ok())
        .unwrap_or(0)
}

/// Document title from the `METa` chunk, if any; `None` when absent or blank.
pub fn title(doc: &DjvuDoc) -> Option<String> {
    guard_panic(|| Ok(doc.inner.metadata().ok().flatten()))
        .ok()
        .flatten()
        .and_then(|m| m.title)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bytes() -> Vec<u8> {
        std::fs::read("../../tests/fixtures/djvu/sample.djvu").expect("fixture must exist")
    }

    fn sample() -> DjvuDoc {
        open(&sample_bytes()).expect("fixture must open")
    }

    /// Concatenate leaf zone text in document order.
    fn flatten_text(z: &Zone) -> String {
        let mut out = z.text.clone();
        for c in &z.children {
            let sub = flatten_text(c);
            if !sub.is_empty() {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(&sub);
            }
        }
        out
    }

    /// Collect every zone of `kind`, in document order.
    fn collect_kind(z: &Zone, kind: ZoneKind, out: &mut Vec<Zone>) {
        if z.kind == kind {
            out.push(z.clone());
        }
        for c in &z.children {
            collect_kind(c, kind, out);
        }
    }

    /// A minimal FORM:DJVM whose DIRM marks the single component as *not*
    /// bundled — i.e. an indirect (multi-file) document.
    fn indirect_djvm_bytes() -> Vec<u8> {
        // BZZ-encoded DIRM metadata for 1 Page component named "chicken.djvu"
        // (borrowed from djvu-rs's own indirect-document test vector).
        let bzz_meta: &[u8] = &[
            0xff, 0xff, 0xed, 0xbf, 0x8a, 0x1f, 0xbe, 0xad, 0x14, 0x57, 0x10, 0xc9, 0x63, 0x19,
            0x11, 0xf0, 0x85, 0x28, 0x12, 0x8a, 0xbf,
        ];
        let mut dirm_data = vec![
            0x00, // flags: bundled bit (0x80) CLEAR => indirect
            0x00, // nfiles high byte
            0x01, // nfiles low byte
        ];
        dirm_data.extend_from_slice(bzz_meta);
        let dirm = djvu_rs::iff::Chunk::Leaf {
            id: *b"DIRM",
            data: dirm_data,
        };
        djvu_rs::iff::partial_emit(*b"DJVM", &[djvu_rs::iff::EmitPart::Chunk(&dirm)])
            .expect("fits within u32")
    }

    #[test]
    fn opens_single_page_document() {
        assert_eq!(page_count(&sample()), 1);
    }

    #[test]
    fn accepts_bundled_document() {
        // The fixture is a bundled FORM:DJVM; opening it must not trip the
        // indirect guard.
        assert!(open(&sample_bytes()).is_ok());
    }

    #[test]
    fn rejects_indirect_document_with_the_exact_message() {
        match open(&indirect_djvm_bytes()) {
            Err(ParseError::Malformed(m)) => assert_eq!(
                m, "indirect multi-file DjVu not supported; provide the bundled document",
                "got: {m}"
            ),
            Err(other) => panic!("expected Malformed, got {other:?}"),
            Ok(_) => panic!("indirect document must be rejected"),
        }
    }

    #[test]
    fn page_text_returns_a_zone_tree_with_the_lines() {
        let root = page_text(&sample(), 1).expect("sample has a text layer");
        let flat = flatten_text(&root);
        assert!(flat.contains("Chapter One"), "got: {flat}");
        assert!(flat.contains("First body line."), "got: {flat}");
        assert!(flat.contains("Second body line."), "got: {flat}");
    }

    #[test]
    fn page_text_preserves_line_order_and_geometry() {
        let root = page_text(&sample(), 1).expect("sample has a text layer");
        let mut lines = Vec::new();
        collect_kind(&root, ZoneKind::Line, &mut lines);
        assert_eq!(lines.len(), 3, "fixture has three lines");

        let texts: Vec<String> = lines.iter().map(flatten_text).collect();
        assert!(texts[0].contains("Chapter One"), "got: {texts:?}");
        assert!(texts[1].contains("First body line."), "got: {texts:?}");
        assert!(texts[2].contains("Second body line."), "got: {texts:?}");

        // Heading inference keys off line height: the heading line
        // must be strictly taller than the body lines.
        let h: Vec<f32> = lines.iter().map(|l| l.bbox.height()).collect();
        assert!(h[0] > h[1], "heading height {} vs body {}", h[0], h[1]);
        assert!(h[0] > h[2], "heading height {} vs body {}", h[0], h[2]);
    }

    #[test]
    fn page_text_is_none_for_out_of_range_page() {
        assert!(page_text(&sample(), 99).is_none());
    }

    #[test]
    fn bookmarks_carry_the_outline_entry() {
        let bm = bookmarks(&sample());
        assert_eq!(bm.len(), 1);
        assert_eq!(bm[0].title, "Chapter One");
        assert_eq!(bm[0].page, 1);
    }

    /// The CLI (`kasane-cli::exit_code_for`) routes an error message to exit 2
    /// when it contains "unsupported", "DRM" or "encrypted", and to exit 1
    /// otherwise. `exit_code_for` is private to that binary, so we pin the
    /// property on the exact string it is handed: the rendered `Display` of the
    /// `ParseError`. Asserting here (rather than in `kasane-cli`) keeps the check
    /// honest — the CLI has no access to `INDIRECT_MSG` and could only re-type
    /// the literal, which would drift silently.
    #[test]
    fn indirect_message_routes_to_exit_one_not_two() {
        let rendered = format!("{}", ParseError::Malformed(INDIRECT_MSG.into()));
        for keyword in ["unsupported", "DRM", "encrypted"] {
            assert!(
                !rendered.contains(keyword),
                "indirect rejection must route to exit 1, but the CLI's exit-2 \
                 keyword {keyword:?} appears in: {rendered}"
            );
        }
        // Sanity: the exit-2 variants really do carry those keywords, so the
        // assertion above is testing a live property and not a tautology.
        assert!(format!("{}", ParseError::Unsupported).contains("unsupported"));
        assert!(format!("{}", ParseError::Drm).contains("DRM"));
        assert!(format!("{}", ParseError::Encrypted).contains("encrypted"));
    }

    fn tz(
        kind: djvu_rs::text::TextZoneKind,
        children: Vec<djvu_rs::text::TextZone>,
    ) -> djvu_rs::text::TextZone {
        djvu_rs::text::TextZone {
            kind,
            rect: djvu_rs::text::Rect {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            },
            text: "x".into(),
            children,
        }
    }

    fn count_zones(z: &Zone) -> usize {
        1 + z.children.iter().map(count_zones).sum::<usize>()
    }

    #[test]
    fn zone_conversion_truncates_at_the_node_budget() {
        // A wide, shallow tree: one Page with 1000 Word children (1001 nodes),
        // well inside the depth cap but past a deliberately small budget.
        let wide = tz(
            djvu_rs::text::TextZoneKind::Page,
            (0..1000)
                .map(|_| tz(djvu_rs::text::TextZoneKind::Word, vec![]))
                .collect(),
        );

        let mut budget = 10usize;
        let converted = convert_zone(&wide, 0, &mut budget).expect("root fits in the budget");
        assert_eq!(
            count_zones(&converted),
            10,
            "conversion must stop at budget"
        );
        assert_eq!(budget, 0, "budget must be fully consumed");

        // A zero budget yields nothing at all, and returns rather than recursing.
        let mut none_left = 0usize;
        assert!(convert_zone(&wide, 0, &mut none_left).is_none());

        // With the production budget the same tree converts in full.
        let mut prod = MAX_ZONE_NODES;
        let full = convert_zone(&wide, 0, &mut prod).unwrap();
        assert_eq!(count_zones(&full), 1001);
    }

    #[test]
    fn bookmark_conversion_truncates_at_the_node_budget() {
        fn rb(children: Vec<djvu_rs::DjVuBookmark>) -> djvu_rs::DjVuBookmark {
            djvu_rs::DjVuBookmark {
                title: "t".into(),
                url: "#1".into(),
                children,
            }
        }
        fn count_bm(b: &Bookmark) -> usize {
            1 + b.children.iter().map(count_bm).sum::<usize>()
        }

        let wide = rb((0..1000).map(|_| rb(vec![])).collect());

        let mut budget = 7usize;
        let converted = convert_bookmark(&wide, 0, &mut budget).expect("root fits");
        assert_eq!(count_bm(&converted), 7, "conversion must stop at budget");
        assert_eq!(budget, 0);

        let mut none_left = 0usize;
        assert!(convert_bookmark(&wide, 0, &mut none_left).is_none());

        let mut prod = MAX_BOOKMARK_NODES;
        assert_eq!(
            count_bm(&convert_bookmark(&wide, 0, &mut prod).unwrap()),
            1001
        );
    }

    #[test]
    fn rejects_non_djvu_bytes() {
        assert!(matches!(open(b"not a djvu"), Err(ParseError::Malformed(_))));
    }

    #[test]
    fn title_is_absent_without_metadata() {
        // The fixture carries no METa chunk; mod.rs falls back to the filename.
        assert_eq!(title(&sample()), None);
    }

    #[test]
    fn bbox_height_is_absolute_span() {
        let b = BBox {
            x0: 0.0,
            y0: 100.0,
            x1: 50.0,
            y1: 130.0,
        };
        assert!((b.height() - 30.0).abs() < 0.001);
        // Height is orientation-independent (DjVu y may increase downward).
        let flipped = BBox {
            x0: 0.0,
            y0: 130.0,
            x1: 50.0,
            y1: 100.0,
        };
        assert!((flipped.height() - 30.0).abs() < 0.001);
    }
}
