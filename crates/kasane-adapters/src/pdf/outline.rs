use lopdf::{Dictionary, Document, ObjectId};
use std::collections::{BTreeMap, HashSet};

/// A heading derived from a `/Outlines` entry.
#[derive(Clone, Debug)]
pub struct OutlineHeading {
    pub level: u8,
    pub title: String,
}

/// Caps on the `/Outlines` graph we are willing to hand to `get_toc`. Depth
/// bounds `/First` descent; the node count bounds the `/Next` sibling walk,
/// which is where lopdf grows a Vec one entry per iteration.
const MAX_OUTLINE_DEPTH: usize = 64;
const MAX_OUTLINE_NODES: usize = 10_000;

/// Resolve `key` in `node` to a dictionary, keeping the `ObjectId` when the
/// edge went through a reference. `Document::get_dict_in_dict` would resolve
/// the same edge but discards the id, and the id is exactly what cycle
/// detection needs. An inline dictionary yields `None` for the id — it cannot
/// refer to itself, so it cannot close a cycle.
fn edge<'a>(
    doc: &'a Document,
    node: &'a Dictionary,
    key: &[u8],
) -> Option<(Option<ObjectId>, &'a Dictionary)> {
    let obj = node.get(key).ok()?;
    let (id, resolved) = doc.dereference(obj).ok()?;
    Some((id, resolved.as_dict().ok()?))
}

/// Resolve `tree`'s `/Kids` array to the entries
/// `Document::get_named_destinations` recurses into: only array entries that
/// are references resolving to a dictionary --
/// `kid.as_reference().and_then(|id| self.get_dictionary(id))` in lopdf's own
/// code. A non-reference or non-dictionary kid is skipped by lopdf, not
/// recursed, so it is skipped here too. This is a separate helper from
/// `edge` rather than a reuse of it: `/Kids` is an array of children, not a
/// single named edge, so `edge`'s one-dictionary-in, one-dictionary-out shape
/// does not fit.
fn kids<'a>(doc: &'a Document, tree: &'a Dictionary) -> Vec<(ObjectId, &'a Dictionary)> {
    let Ok(kids) = tree.get(b"Kids") else {
        return Vec::new();
    };
    let Ok(kids) = kids.as_array() else {
        return Vec::new();
    };
    kids.iter()
        .filter_map(|kid| {
            let id = kid.as_reference().ok()?;
            let dict = doc.get_dictionary(id).ok()?;
            Some((id, dict))
        })
        .collect()
}

/// True when the destination name tree -- `/Dests` in the catalog, or
/// `/Dests` inside `/Names` -- is finite and small enough for
/// `Document::get_named_destinations` to walk. `Document::get_outlines`
/// resolves and walks this tree *before* it touches a single outline node, so
/// it has to be proven finite here too, on the same terms as the outline
/// graph below: `get_named_destinations` recurses `/Kids` with neither a
/// visited set nor a depth bound -- the same unbounded-recursion shape as the
/// outline's `/First`, just reached one call earlier.
///
/// Tree selection mirrors lopdf exactly: `catalog/Dests` first, and only if
/// that is absent, `catalog/Names/Dests`. If neither resolves there is
/// nothing to walk, so this returns `true` -- there is nothing to reject.
///
/// `/Names` within a node is iterated, not recursed, by
/// `get_named_destinations`, so it cannot be the source of an abort and is
/// not walked here -- only `/Kids` is bounded.
fn named_destinations_are_traversable(doc: &Document, catalog: &Dictionary) -> bool {
    let tree = edge(doc, catalog, b"Dests").or_else(|| {
        let (_, names) = edge(doc, catalog, b"Names")?;
        edge(doc, names, b"Dests")
    });
    let Some((root_id, root)) = tree else {
        return true;
    };

    let mut visited: HashSet<ObjectId> = HashSet::new();
    let mut nodes = 0usize;
    let mut stack: Vec<((Option<ObjectId>, &Dictionary), usize)> = vec![((root_id, root), 1)];

    while let Some(((id, node), depth)) = stack.pop() {
        if depth > MAX_OUTLINE_DEPTH {
            return false;
        }
        nodes += 1;
        if nodes > MAX_OUTLINE_NODES {
            return false;
        }
        if let Some(id) = id {
            if !visited.insert(id) {
                return false; // already seen: the tree is cyclic
            }
        }
        for (kid_id, kid) in kids(doc, node) {
            stack.push(((Some(kid_id), kid), depth + 1));
        }
    }
    true
}

/// True when both graphs lopdf walks on the way to a table of contents are
/// finite and small enough to hand to `get_toc`: the `/Outlines` tree itself,
/// and the destination name tree (`/Dests` or `/Names/Dests`) that lopdf
/// resolves and walks first, before a single outline node is touched. lopdf
/// walks `/First` and `/Kids` recursively and `/Next` iteratively, with
/// neither a visited set nor a depth bound anywhere in either graph, so a
/// cycle in either one either overflows the stack (`/First`, `/Kids`) or
/// spins forever while growing a Vec (`/Next`). The overflow aborts the
/// process and cannot be caught, so both graphs must be proven finite
/// *before* `get_toc` is called at all.
///
/// The walk uses an explicit stack, so fixing a recursion bug does not
/// introduce recursion of its own. It follows the same edges lopdf follows,
/// including lopdf's reassignment of the start node to the root's `/First`
/// when the root has one.
///
/// `visited` is global to each walk rather than per-path, so a node reachable
/// by two routes is rejected even though it is acyclic. That is deliberate:
/// an outline item has one `/Parent`, sharing is malformed, and the cost of
/// the stricter rule is a fallback to font-size headings.
fn outline_is_traversable(doc: &Document) -> bool {
    // No catalog: get_toc fails harmlessly on its own for both graphs below.
    let Ok(catalog) = doc.catalog() else {
        return true;
    };

    if !named_destinations_are_traversable(doc, catalog) {
        return false;
    }

    let Some((root_id, root)) = edge(doc, catalog, b"Outlines") else {
        return true;
    };
    let start = edge(doc, root, b"First").unwrap_or((root_id, root));

    let mut visited: HashSet<ObjectId> = HashSet::new();
    let mut nodes = 0usize;
    let mut stack: Vec<((Option<ObjectId>, &Dictionary), usize)> = vec![(start, 1)];

    while let Some(((id, node), depth)) = stack.pop() {
        if depth > MAX_OUTLINE_DEPTH {
            return false;
        }
        nodes += 1;
        if nodes > MAX_OUTLINE_NODES {
            return false;
        }
        if let Some(id) = id {
            if !visited.insert(id) {
                return false; // already seen: the graph is cyclic
            }
        }
        if let Some(first) = edge(doc, node, b"First") {
            stack.push((first, depth + 1));
        }
        if let Some(next) = edge(doc, node, b"Next") {
            stack.push((next, depth));
        }
    }
    true
}

/// Map each page number to the outline headings that target it, in outline
/// order. lopdf's `get_toc` resolves destinations to page numbers and levels;
/// a document without an outline yields an empty map (never an error).
pub fn outline_by_page(doc: &Document) -> BTreeMap<u32, Vec<OutlineHeading>> {
    let mut map: BTreeMap<u32, Vec<OutlineHeading>> = BTreeMap::new();
    // A hostile outline is dropped whole. The empty map is the same signal
    // this function already produces for a get_toc error, and pdf/mod.rs
    // reads it as "no outline" and falls back to font-size inference.
    if !outline_is_traversable(doc) {
        return map;
    }
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
    use lopdf::{dictionary, Object};

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

    /// An outline root whose /First points at itself — the shape of the #21
    /// reproducer. Drives lopdf's recursive arm.
    fn first_self_cycle() -> lopdf::Document {
        let mut doc = lopdf::Document::with_version("1.5");
        let root = doc.new_object_id();
        doc.objects.insert(
            root,
            Object::Dictionary(dictionary! { "Type" => "Outlines", "First" => root }),
        );
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Outlines" => root });
        doc.trailer.set("Root", cat);
        doc
    }

    /// A single outline item that is its own /Next sibling. Drives lopdf's
    /// iterative arm, which hangs rather than overflowing.
    fn next_self_cycle() -> lopdf::Document {
        let mut doc = lopdf::Document::with_version("1.5");
        let (root, item) = (doc.new_object_id(), doc.new_object_id());
        doc.objects.insert(
            item,
            Object::Dictionary(dictionary! {
                "Title" => Object::string_literal("Loop"),
                "Parent" => root,
                "Next" => item,
            }),
        );
        doc.objects.insert(
            root,
            Object::Dictionary(dictionary! { "Type" => "Outlines", "First" => item }),
        );
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Outlines" => root });
        doc.trailer.set("Root", cat);
        doc
    }

    /// Two nodes whose /First edges point at each other. Proves the visited set
    /// catches more than the degenerate self-edge.
    fn mutual_first_cycle() -> lopdf::Document {
        let mut doc = lopdf::Document::with_version("1.5");
        let (root, a, b) = (
            doc.new_object_id(),
            doc.new_object_id(),
            doc.new_object_id(),
        );
        doc.objects.insert(
            a,
            Object::Dictionary(
                dictionary! { "Title" => Object::string_literal("A"), "First" => b },
            ),
        );
        doc.objects.insert(
            b,
            Object::Dictionary(
                dictionary! { "Title" => Object::string_literal("B"), "First" => a },
            ),
        );
        doc.objects.insert(
            root,
            Object::Dictionary(dictionary! { "Type" => "Outlines", "First" => a }),
        );
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Outlines" => root });
        doc.trailer.set("Root", cat);
        doc
    }

    #[test]
    fn rejects_first_self_cycle() {
        assert!(outline_by_page(&first_self_cycle()).is_empty());
    }

    #[test]
    fn rejects_next_self_cycle() {
        assert!(outline_by_page(&next_self_cycle()).is_empty());
    }

    #[test]
    fn rejects_mutual_first_cycle() {
        assert!(outline_by_page(&mutual_first_cycle()).is_empty());
    }

    /// A well-formed minimal outline plus a `/Dests` name tree whose `/Kids`
    /// points back at itself. `Document::get_named_destinations` recurses
    /// `/Kids` with neither a visited set nor a depth bound -- the same
    /// unbounded-recursion shape as `first_self_cycle` above, just reached
    /// one call earlier: lopdf resolves and walks this tree before
    /// `get_outlines` ever touches an outline node, so this would recurse
    /// forever (and abort the process) even though the outline itself is
    /// fine.
    fn dests_kids_cycle() -> lopdf::Document {
        let mut doc = lopdf::Document::with_version("1.5");
        let item = doc.new_object_id();
        doc.objects.insert(
            item,
            Object::Dictionary(dictionary! { "Title" => Object::string_literal("Heading") }),
        );
        let outline_root = doc.add_object(dictionary! { "Type" => "Outlines", "First" => item });

        let dests = doc.new_object_id();
        doc.objects.insert(
            dests,
            Object::Dictionary(dictionary! { "Kids" => vec![Object::Reference(dests)] }),
        );

        let cat = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Outlines" => outline_root,
            "Dests" => dests,
        });
        doc.trailer.set("Root", cat);
        doc
    }

    #[test]
    fn rejects_dests_kids_cycle() {
        assert!(outline_by_page(&dests_kids_cycle()).is_empty());
    }

    /// A normal, acyclic `/Dests` tree: a root whose `/Kids` reaches one leaf
    /// carrying a `/Names` array. Proves the destination-tree walk is a
    /// boundary, not a blanket refusal of every document with destinations.
    /// Tested through the private `outline_is_traversable` rather than
    /// `outline_by_page`: producing real headings would additionally require
    /// a full page tree, which the traversability guard never inspects (it
    /// only walks `/Kids`, never resolves a destination to a page).
    fn acyclic_dests_tree() -> lopdf::Document {
        let mut doc = lopdf::Document::with_version("1.5");
        let leaf = doc.new_object_id();
        doc.objects.insert(
            leaf,
            Object::Dictionary(dictionary! {
                "Names" => vec![
                    Object::string_literal("A"),
                    Object::Dictionary(dictionary! {
                        "D" => vec![Object::Integer(0), Object::Name(b"Fit".to_vec())],
                    }),
                ],
            }),
        );
        let root = doc.new_object_id();
        doc.objects.insert(
            root,
            Object::Dictionary(dictionary! { "Kids" => vec![Object::Reference(leaf)] }),
        );
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Dests" => root });
        doc.trailer.set("Root", cat);
        doc
    }

    #[test]
    fn accepts_acyclic_destination_tree() {
        assert!(outline_is_traversable(&acyclic_dests_tree()));
    }

    /// An acyclic chain of `n` items linked by `key`: "First" makes it deep
    /// (exercising the depth cap), "Next" makes it wide (the node cap).
    fn chain(n: usize, key: &str) -> lopdf::Document {
        let mut doc = lopdf::Document::with_version("1.5");
        let ids: Vec<lopdf::ObjectId> = (0..n).map(|_| doc.new_object_id()).collect();
        for (i, id) in ids.iter().enumerate() {
            let mut d = dictionary! { "Title" => Object::string_literal("N") };
            if let Some(next) = ids.get(i + 1) {
                d.set(key, *next);
            }
            doc.objects.insert(*id, Object::Dictionary(d));
        }
        let root = doc.add_object(dictionary! { "Type" => "Outlines", "First" => ids[0] });
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Outlines" => root });
        doc.trailer.set("Root", cat);
        doc
    }

    #[test]
    fn caps_bound_acyclic_but_oversized_outlines() {
        // Depth: a /First chain one past the cap is rejected, one comfortably
        // inside it is not -- so the cap is a boundary, not a blanket refusal.
        assert!(!outline_is_traversable(&chain(
            MAX_OUTLINE_DEPTH + 1,
            "First"
        )));
        assert!(outline_is_traversable(&chain(
            MAX_OUTLINE_DEPTH - 4,
            "First"
        )));
        // Nodes: a /Next chain stays at depth 1, so only the node cap can fire.
        // This is the bound that stops lopdf growing a Vec per sibling.
        assert!(!outline_is_traversable(&chain(
            MAX_OUTLINE_NODES + 1,
            "Next"
        )));
        assert!(outline_is_traversable(&chain(MAX_OUTLINE_NODES, "Next")));
    }
}
