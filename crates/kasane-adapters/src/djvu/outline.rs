//! Pure: NAVM `Bookmark` tree -> per-page headings. No `djvu-rs`, no files.
//! Functions are added in Task 3.

use super::doc::Bookmark;
use std::collections::BTreeMap;

#[allow(dead_code)]
const MAX_OUTLINE_DEPTH: usize = 64;

/// A heading derived from one NAVM bookmark.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct OutlineHeading {
    pub level: u8,
    pub title: String,
}

/// Map each 1-based page to the outline headings targeting it, in outline order.
/// Depth (1-based) becomes the heading level, clamped to the IR range 1–6.
/// An empty slice yields an empty map (never an error).
#[allow(dead_code)]
pub fn outline_by_page(bookmarks: &[Bookmark]) -> BTreeMap<u32, Vec<OutlineHeading>> {
    let mut map: BTreeMap<u32, Vec<OutlineHeading>> = BTreeMap::new();
    walk(bookmarks, 1, &mut map);
    map
}

#[allow(dead_code)]
fn walk(nodes: &[Bookmark], depth: usize, map: &mut BTreeMap<u32, Vec<OutlineHeading>>) {
    if depth > MAX_OUTLINE_DEPTH {
        return;
    }
    for b in nodes {
        let title = b.title.trim().to_string();
        if b.page > 0 && !title.is_empty() {
            let level = depth.clamp(1, 6) as u8;
            map.entry(b.page)
                .or_default()
                .push(OutlineHeading { level, title });
        }
        walk(&b.children, depth + 1, map);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::djvu::doc::Bookmark;

    fn bm(title: &str, page: u32, children: Vec<Bookmark>) -> Bookmark {
        Bookmark {
            title: title.into(),
            page,
            children,
        }
    }

    #[test]
    fn nested_bookmarks_become_leveled_headings_by_page() {
        let tree = vec![bm(
            "Chapter One",
            1,
            vec![bm("Section A", 2, vec![]), bm("Section B", 3, vec![])],
        )];
        let map = outline_by_page(&tree);
        assert_eq!(map.get(&1).unwrap()[0].title, "Chapter One");
        assert_eq!(map.get(&1).unwrap()[0].level, 1);
        assert_eq!(map.get(&2).unwrap()[0].title, "Section A");
        assert_eq!(map.get(&2).unwrap()[0].level, 2);
        assert_eq!(map.get(&3).unwrap()[0].level, 2); // depth 2 -> level 2
    }

    #[test]
    fn drops_entries_with_no_page_or_empty_title() {
        let tree = vec![bm("", 1, vec![]), bm("Real", 0, vec![])];
        assert!(outline_by_page(&tree).is_empty());
    }

    #[test]
    fn dropped_parent_does_not_prune_valid_children() {
        // Parent has empty title (dropped), but child has real page and title (kept).
        // Verify the child still appears and its level reflects true depth (depth increments even if parent dropped).
        let tree = vec![bm("", 1, vec![bm("Valid Child", 2, vec![])])];
        let map = outline_by_page(&tree);

        // Page 1 should have no headings (parent was dropped).
        assert!(
            !map.contains_key(&1) || map[&1].is_empty(),
            "empty-title parent should be dropped"
        );

        // Page 2 should have the child heading.
        let page_2_headings = map
            .get(&2)
            .expect("child with valid page should appear in map");
        assert_eq!(
            page_2_headings.len(),
            1,
            "exactly one child heading for page 2"
        );
        assert_eq!(page_2_headings[0].title, "Valid Child");

        // Child is at depth 2 (parent was depth 1; depth increments unconditionally even if parent dropped).
        assert_eq!(
            page_2_headings[0].level, 2,
            "child at depth 2 should have level 2"
        );
    }

    #[test]
    fn deep_tree_is_bounded_not_infinite() {
        // Build a chain deeper than the cap; must terminate and clamp level to 6.
        let mut node = bm("leaf", 1, vec![]);
        for _ in 0..200 {
            node = bm("x", 1, vec![node]);
        }
        let map = outline_by_page(&[node]);
        let headings = map.get(&1).expect("page 1 should have headings");

        // Should have exactly MAX_OUTLINE_DEPTH headings (one per depth 1..=MAX_OUTLINE_DEPTH);
        // depth > MAX_OUTLINE_DEPTH causes early return without processing.
        assert_eq!(
            headings.len(),
            MAX_OUTLINE_DEPTH,
            "all depths 1..={} should be pushed; depth > {} returns early",
            MAX_OUTLINE_DEPTH,
            MAX_OUTLINE_DEPTH
        );

        // All levels should be clamped to 1..=6.
        assert!(headings.iter().all(|h| (1..=6).contains(&h.level)));
    }
}
