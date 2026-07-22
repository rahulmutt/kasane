#![allow(dead_code)]
//! The sole seam over the `djvu-rs` crate. Everything else in `djvu/` consumes
//! the port types defined here, never `djvu-rs` directly.

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

#[cfg(test)]
mod tests {
    use super::*;

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
