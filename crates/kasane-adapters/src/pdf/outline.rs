use lopdf::Document;
use std::collections::BTreeMap;

/// A heading derived from a `/Outlines` entry.
#[derive(Clone, Debug)]
pub struct OutlineHeading {
    pub level: u8,
    pub title: String,
}

/// Map each page number to the outline headings that target it, in outline
/// order. lopdf's `get_toc` resolves destinations to page numbers and levels;
/// a document without an outline yields an empty map (never an error).
pub fn outline_by_page(doc: &Document) -> BTreeMap<u32, Vec<OutlineHeading>> {
    let mut map: BTreeMap<u32, Vec<OutlineHeading>> = BTreeMap::new();
    let Ok(toc) = doc.get_toc() else {
        return map; // Error::NoOutline (or any error) -> no outline headings
    };
    for entry in toc.toc {
        let page = entry.page as u32;
        let title = entry.title.trim().to_string();
        if page == 0 || title.is_empty() {
            continue;
        }
        // Outline depth is 1-based in lopdf; clamp to the IR heading range 1–6.
        let level = entry.level.clamp(1, 6) as u8;
        map.entry(page)
            .or_default()
            .push(OutlineHeading { level, title });
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::doc::open;

    fn doc(name: &str) -> lopdf::Document {
        open(&std::fs::read(format!("../../tests/fixtures/pdf/{name}.pdf")).unwrap()).unwrap()
    }

    #[test]
    fn maps_outline_entries_to_pages() {
        let map = outline_by_page(&doc("minimal"));
        assert_eq!(map.get(&1).unwrap()[0].title, "Chapter One");
        assert_eq!(map.get(&2).unwrap()[0].title, "Section Two");
        assert_eq!(map.get(&1).unwrap()[0].level, 1);
    }

    #[test]
    fn empty_when_no_outline() {
        assert!(outline_by_page(&doc("no-outline")).is_empty());
    }
}
