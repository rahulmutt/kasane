//! Hermetic DjVu fixture generator (pure Rust, via the `djvu-rs` crate).
//!
//! Regenerate the fixture with:
//!
//! ```sh
//! cargo run -p kasane-adapters --example make_djvu_fixture
//! ```
//!
//! Writes `tests/fixtures/djvu/sample.djvu`: a bundled, single-page DjVu with a
//! three-line text layer (a taller "heading" line plus two shorter body lines)
//! and one NAVM outline bookmark pointing at page 1.
//!
//! This replaces the DjVuLibre (`cjb2` / `djvused`) recipe from the task brief.
//! DjVuLibre is not available in this environment, and generating the fixture
//! with the very crate that kasane parses it with (`djvu-rs`) makes the file
//! hermetic and round-trip-guaranteed against the same reader/writer.

use djvu_rs::djvu_encode::PageEncoder;
use djvu_rs::djvu_mut::DjVuDocumentMut;
use djvu_rs::text::{Rect, TextLayer, TextZone, TextZoneKind};
use djvu_rs::{Bitmap, DjVuBookmark, DjVuDocument};

/// A `Line` text zone at a top-left-origin rectangle.
fn line(text: &str, x: u32, y: u32, w: u32, h: u32) -> TextZone {
    TextZone {
        kind: TextZoneKind::Line,
        rect: Rect {
            x,
            y,
            width: w,
            height: h,
        },
        text: text.to_string(),
        children: Vec::new(),
    }
}

fn main() {
    const W: u32 = 64;
    const H: u32 = 64;

    // 1. A blank bilevel page with a couple of ink pixels — enough to be a
    //    genuine scan without bloating the JB2 mask.
    let mut bm = Bitmap::new(W, H);
    bm.set_black(1, 1);
    bm.set_black(2, 2);

    // 2. Text layer: one Page zone containing three Line zones, in reading
    //    order, using the page's top-left pixel coordinate system. The first
    //    line's height (16) is clearly greater than the two body lines' (8),
    //    which is what lets the adapter treat it as a heading. All rects fit
    //    inside the 64x64 page.
    let heading = "Chapter One";
    let body1 = "First body line.";
    let body2 = "Second body line.";
    let full = format!("{heading}\n{body1}\n{body2}");
    let page = TextZone {
        kind: TextZoneKind::Page,
        rect: Rect {
            x: 0,
            y: 0,
            width: W,
            height: H,
        },
        text: full.clone(),
        children: vec![
            line(heading, 4, 4, 56, 16),
            line(body1, 4, 28, 56, 8),
            line(body2, 4, 44, 56, 8),
        ],
    };
    let layer = TextLayer {
        text: full,
        zones: vec![page],
    };

    // 3. Encode a single-page `FORM:DJVU` that carries the text layer (TXTz).
    let single = PageEncoder::from_bitmap(&bm)
        .with_dpi(100)
        .with_text_layer(layer)
        .encode()
        .expect("encode single-page FORM:DJVU");

    // 4. Bundle it into a `FORM:DJVM`. A document-level NAVM outline can only
    //    be attached to a bundle, not a bare single-page form.
    let bundled = djvu_rs::djvm::merge(&[&single]).expect("bundle into FORM:DJVM");

    // 5. Attach one NAVM bookmark to page 1 (DjVu internal URL "#1").
    let mut doc = DjVuDocumentMut::from_bytes(&bundled).expect("open bundle for mutation");
    doc.set_bookmarks(&[DjVuBookmark {
        title: heading.to_string(),
        url: "#1".to_string(),
        children: Vec::new(),
    }])
    .expect("attach NAVM bookmark");
    let bytes = doc.try_into_bytes().expect("serialize bundled DjVu");

    // --- Round-trip verification (de-risks Task 3 / Task 7) ---
    assert!(
        bytes.starts_with(b"AT&T"),
        "fixture must begin with the AT&T DjVu preamble"
    );
    let parsed = DjVuDocument::parse(&bytes).expect("re-parse generated DjVu");
    assert_eq!(parsed.page_count(), 1, "expected a single page");
    let tl = parsed
        .page(0)
        .expect("page 0")
        .text_layer()
        .expect("text layer decode")
        .expect("text layer present");
    for needle in [heading, body1, body2] {
        assert!(
            tl.text.contains(needle),
            "round-tripped text layer is missing {needle:?}"
        );
    }
    let bms = parsed.bookmarks();
    assert_eq!(bms.len(), 1, "expected exactly one NAVM bookmark");
    assert_eq!(bms[0].title, heading, "bookmark title mismatch");
    assert_eq!(bms[0].url, "#1", "bookmark should target page 1");

    std::fs::create_dir_all("tests/fixtures/djvu").expect("create fixtures dir");
    std::fs::write("tests/fixtures/djvu/sample.djvu", &bytes).expect("write sample.djvu");
    println!(
        "wrote tests/fixtures/djvu/sample.djvu ({} bytes): {} page, {} bookmark(s), \
         text-layer lines = [{heading:?}, {body1:?}, {body2:?}]",
        bytes.len(),
        parsed.page_count(),
        bms.len(),
    );
}
