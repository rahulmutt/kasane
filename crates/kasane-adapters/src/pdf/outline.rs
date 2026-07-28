use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{BTreeMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// A heading derived from a `/Outlines` entry.
#[derive(Clone, Debug)]
pub struct OutlineHeading {
    pub level: u8,
    pub title: String,
}

/// Caps on the graphs we are willing to hand to `get_toc`: the `/Outlines`
/// tree and the destination name tree. Depth bounds every descending edge --
/// the outline's `/First` and the name tree's `/Kids` -- while the node count
/// bounds the whole walk, and so in particular the `/Next` sibling chain,
/// which is where lopdf grows a Vec one entry per iteration. Together they are
/// the *topology* half of the pre-flight; the other half checks the node
/// contents lopdf indexes into (see `outline_dest_is_indexable` and
/// `destination_names_are_indexable`) and needs no cap of its own.
const MAX_OUTLINE_DEPTH: usize = 64;
const MAX_OUTLINE_NODES: usize = 10_000;

/// One node of a walked graph: the `ObjectId` the edge into it went through,
/// plus the dictionary itself. The id is `None` for an inline dictionary,
/// which cannot refer to itself and so cannot close a cycle.
type Node<'a> = (Option<ObjectId>, &'a Dictionary);

/// Resolve `key` in `node` to a dictionary, keeping the `ObjectId` when the
/// edge went through a reference. `Document::get_dict_in_dict` would resolve
/// the same edge but discards the id, and the id is exactly what cycle
/// detection needs.
fn edge<'a>(doc: &'a Document, node: &'a Dictionary, key: &[u8]) -> Option<Node<'a>> {
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

/// Prove a graph rooted at `start` is finite -- acyclic, no deeper than
/// `MAX_OUTLINE_DEPTH`, and no larger than `MAX_OUTLINE_NODES` nodes -- and
/// that every node in it satisfies the caller's `node_is_safe`. This is the
/// one place the explicit stack, the `ObjectId`-keyed visited set, and both
/// caps live: `outline_is_traversable` (the `/Outlines` `/First` + `/Next`
/// graph) and `named_destinations_are_traversable` (the `/Dests`/`/Names`
/// `/Kids` tree) both call this rather than each keeping their own copy of
/// the bookkeeping.
///
/// `node_is_safe` and `expand` are each called once per node, after that node
/// has already passed the depth, node-count, and cycle checks below.
///
/// `node_is_safe` is the caller's check on that node's *contents* -- the
/// values lopdf will index or `unwrap` once the graph is handed to `get_toc`.
/// Returning `false` rejects the whole walk on the spot. Topology is this
/// helper's business; which values are dangerous is the caller's, because the
/// two graphs are read by different lopdf functions.
///
/// `expand` supplies that node's outgoing edges: each paired with the child's
/// depth *relative to the child's own parent* -- 1 for a depth-increasing edge
/// (`/First`, `/Kids`), 0 for a same-depth edge (`/Next`, which lopdf walks as
/// a sibling, not a child). Preserving that distinction is what lets one walk
/// model two graphs whose depth accounting is not the same shape.
///
/// The visited set is global to the whole walk rather than reset per path,
/// so a node reachable by two routes is rejected even though it is acyclic.
/// Each caller's own doc comment explains why that stricter rule costs it
/// nothing.
fn is_bounded_and_acyclic<'a>(
    start: Node<'a>,
    mut node_is_safe: impl FnMut(&'a Dictionary) -> bool,
    mut expand: impl FnMut(&'a Dictionary) -> Vec<(usize, Node<'a>)>,
) -> bool {
    let mut visited: HashSet<ObjectId> = HashSet::new();
    let mut nodes = 0usize;
    let mut stack: Vec<(Node<'a>, usize)> = vec![(start, 1)];

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
        if !node_is_safe(node) {
            return false;
        }
        for (depth_delta, child) in expand(node) {
            stack.push((child, depth + depth_delta));
        }
    }
    true
}

/// True when every `/Names` entry on this destination-tree node is one
/// `Document::get_named_destinations` can consume without panicking.
///
/// lopdf walks `/Names` in (key, value) pairs and, for each pair, builds a
/// `Destination` from the value's first two array elements and the key read as
/// a string. None of those three steps is checked, so each is a panic:
///
/// * `val[0]` / `val[1]` -- index out of bounds on an array shorter than two
///   (`destinations.rs:57`, `:60`, `:65`),
/// * `key.as_str().unwrap()` -- a key that is not a `String` object
///   (`destinations.rs:58`, `:61`, `:66`),
/// * `dict.get(b"D").as_ref().unwrap()` -- a destination dictionary with no
///   `/D` at all (`destinations.rs:56`, `:64`).
///
/// The three line triples are the three value shapes lopdf accepts -- a
/// reference resolving to a dictionary, a reference resolving straight to an
/// array, and an inline dictionary -- and it does the identical unchecked work
/// in all three, so all three are covered here.
///
/// Deliberately *not* rejected: anything lopdf turns into an `Error` rather
/// than a panic. A `/Names` that is not an array, a `/D` that is present but
/// not an array (including a `/D` that is a reference -- `Dictionary::get` is
/// raw and `as_array()` fails on it), or a value that is neither a reference
/// nor a dictionary. lopdf either propagates an error, which `outline_by_page`
/// already degrades to an empty map, or silently skips the entry. Rejecting
/// those here would drop outlines lopdf handles fine.
fn destination_names_are_indexable(doc: &Document, node: &Dictionary) -> bool {
    // `tree.get(b"Names")` is raw and `names.as_array()?` errors on anything
    // that is not literally an array, so only an array reaches the loop.
    let Ok(names) = node.get(b"Names") else {
        return true;
    };
    let Ok(names) = names.as_array() else {
        return true;
    };
    for pair in names.chunks(2) {
        // lopdf pulls key and value off one iterator and breaks the moment
        // either is missing, so a trailing odd entry is dropped, not indexed.
        let [key, val] = pair else {
            break;
        };
        // The object lopdf will call `as_array()` on and then index.
        let dest = match val.as_reference() {
            Ok(id) => match doc.get_dictionary(id) {
                // destinations.rs:55-58 -- a reference to a dictionary.
                Ok(dict) => match dict.get(b"D") {
                    Ok(d) => d,
                    Err(_) => return false, // the unwrap at destinations.rs:56
                },
                // destinations.rs:59-61 -- a reference to an array, indexed
                // as-is with no `/D` in between.
                Err(_) => match doc.get_object(id) {
                    Ok(obj) if obj.as_array().is_ok() => obj,
                    // Neither dictionary nor array: lopdf skips the entry.
                    _ => continue,
                },
            },
            // destinations.rs:63-66 -- an inline dictionary.
            Err(_) => match val.as_dict() {
                Ok(dict) => match dict.get(b"D") {
                    Ok(d) => d,
                    Err(_) => return false, // the unwrap at destinations.rs:64
                },
                // Neither a reference nor a dictionary: lopdf skips the entry.
                Err(_) => continue,
            },
        };
        if let Ok(arr) = dest.as_array() {
            if arr.len() < 2 {
                return false;
            }
            // Only reached once the array is long enough, which is exactly
            // when lopdf goes on to read the key as a string.
            if key.as_str().is_err() {
                return false;
            }
        }
    }
    true
}

/// The destination object `Document::get_outline` will hand to
/// `build_outline_result` for `node`, or `None` when lopdf returns an error
/// first and so never indexes anything.
///
/// `/A` versus `/Dest`: `get_outline` opens with
/// `self.get_dict_in_dict(node, b"A")` (`outlines.rs:14`). When that resolves,
/// the *action's* `/D` is the destination and `/Dest` is never read at all
/// (`outlines.rs:28`, `:32`); only when `/A` is absent or is not a dictionary
/// does lopdf fall back to `/Dest` (`outlines.rs:17`). So exactly one of the
/// two keys is live per node, and which one depends on `/A`.
///
/// Every other exit on the `/A` path returns `Err` *before* any indexing, and
/// `get_outlines` ignores that error and keeps walking (`outlines.rs:71`) --
/// an `/S` that is missing, not a name, or not `GoTo`/`GoToR`; a missing
/// `/Title`; a `/Title` that is neither a resolvable reference nor a string.
/// The `/Dest` path likewise needs both `/Dest` and `/Title` present. Those
/// exits are mirrored here rather than ignored, because treating them as
/// dangerous would reject outlines lopdf renders perfectly well.
fn outline_dest<'a>(doc: &'a Document, node: &'a Dictionary) -> Option<&'a Object> {
    let Some((_, action)) = edge(doc, node, b"A") else {
        // outlines.rs:17 -- no action, so /Dest, and /Title must exist too.
        let dest = node.get(b"Dest").ok()?;
        node.get(b"Title").ok()?;
        return Some(dest);
    };
    // outlines.rs:20-23
    let command = action.get(b"S").ok()?.as_name().ok()?;
    if command != b"GoTo" && command != b"GoToR" {
        return None;
    }
    // outlines.rs:24-32
    let title = node.get(b"Title").ok()?;
    match title.as_reference() {
        Ok(id) => {
            doc.get_object(id).ok()?;
        }
        Err(_) => {
            title.as_str().ok()?;
        }
    }
    action.get(b"D").ok()
}

/// True when the destination lopdf resolves for this outline node is one
/// `build_outline_result` can index.
///
/// `build_outline_result` matches on the destination and, for an array, takes
/// elements `[0]` and `[1]` with no length check at all
/// (`outlines.rs:100-101`) -- a one-element `/Dest` is the committed
/// reproducer. A `String` destination is a name-tree lookup whose miss is a
/// harmless `Ok(None)`; anything else is an `Err`.
///
/// A `Reference` destination is followed with `get_object` and re-matched
/// (`outlines.rs:111-113`), so the value actually indexed is the
/// *dereferenced* one -- hence the `dereference` here. Chain length needs no
/// bound of its own: `Document::dereference` stops at `DEREF_LIMIT` (128) and
/// returns `Err(ReferenceLimit)`, which lopdf propagates rather than panics.
fn outline_dest_is_indexable(doc: &Document, node: &Dictionary) -> bool {
    let Some(dest) = outline_dest(doc, node) else {
        return true;
    };
    let Ok((_, dest)) = doc.dereference(dest) else {
        return true; // a dangling or over-long chain is an Err inside lopdf
    };
    match dest {
        Object::Array(arr) => arr.len() >= 2,
        _ => true,
    }
}

/// True when the destination name tree -- `/Dests` in the catalog, or
/// `/Dests` inside `/Names` -- is finite and small enough for
/// `Document::get_named_destinations` to walk, and every entry it holds is one
/// that function can consume without panicking. `Document::get_outlines`
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
/// not walked here -- only `/Kids` is bounded. Its *entries* are still checked
/// on every node, by `destination_names_are_indexable`: iterating them is what
/// makes lopdf index and unwrap unvalidated values.
fn named_destinations_are_traversable(doc: &Document, catalog: &Dictionary) -> bool {
    let tree = edge(doc, catalog, b"Dests").or_else(|| {
        let (_, names) = edge(doc, catalog, b"Names")?;
        edge(doc, names, b"Dests")
    });
    let Some(start) = tree else {
        return true;
    };

    is_bounded_and_acyclic(
        start,
        |node| destination_names_are_indexable(doc, node),
        |node| {
            kids(doc, node)
                .into_iter()
                .map(|(id, dict)| (1, (Some(id), dict)))
                .collect()
        },
    )
}

/// True when both graphs lopdf walks on the way to a table of contents are
/// safe to hand to `get_toc`: finite and small enough, *and* free of the
/// unchecked values lopdf indexes into. The two graphs are the `/Outlines`
/// tree itself and the destination name tree (`/Dests` or `/Names/Dests`)
/// that lopdf resolves and walks first, before a single outline node is
/// touched. See `is_bounded_and_acyclic` for the shared walk mechanics,
/// `named_destinations_are_traversable` for the other graph, and
/// `outline_dest_is_indexable` for this one's per-node contents check.
///
/// This graph's own two edges: `/First` descends -- lopdf follows it
/// recursively -- and `/Next` stays at the same depth -- lopdf follows it
/// iteratively, growing a `Vec` one entry per pass. The start node is
/// reassigned to the root's `/First` when the root has one, exactly as
/// `Document::get_outlines` reassigns it, since that is where lopdf's own
/// walk actually begins.
///
/// The visited set is global to the walk rather than per-path, so a node
/// reachable by two routes is rejected even though it is acyclic. That is
/// deliberate here: an outline item has one `/Parent`, sharing is malformed,
/// and the cost of the stricter rule is a fallback to font-size headings.
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

    is_bounded_and_acyclic(
        start,
        |node| outline_dest_is_indexable(doc, node),
        |node| {
            let mut children = Vec::new();
            if let Some(first) = edge(doc, node, b"First") {
                children.push((1, first));
            }
            if let Some(next) = edge(doc, node, b"Next") {
                children.push((0, next));
            }
            children
        },
    )
}

/// Call `get_toc` with a panic handler, yielding `None` on either an error or
/// a panic. This is the same call this crate already makes over `djvu-rs`
/// (`djvu::doc::guard_panic`) and over OCR engines (`ocr::extract_guarded`):
/// a young third-party parser indexes and unwraps unvalidated input, so a bug
/// in it degrades instead of killing `kasane convert`.
///
/// It is defence in depth for production ONLY, and must never be mistaken for
/// the thing that keeps the `pdf` fuzz target green. `libfuzzer-sys` installs
/// a panic hook that calls `process::abort()` *before* unwinding
/// (`libfuzzer-sys-0.4.13/src/lib.rs:91-95`), so under the fuzzer the hook
/// fires and the process dies before this handler is ever reached. Only the
/// pre-flight above -- proving the graphs finite and their destinations
/// indexable -- prevents the crash the fuzzer sees.
fn guarded_toc(doc: &Document) -> Option<lopdf::Toc> {
    match catch_unwind(AssertUnwindSafe(|| doc.get_toc())) {
        Ok(Ok(toc)) => Some(toc),
        // Error::NoOutline (or any error) -> no outline headings.
        Ok(Err(_)) | Err(_) => None,
    }
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
    let Some(toc) = guarded_toc(doc) else {
        return map;
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

    /// A document whose destination name tree is a single node carrying the
    /// `/Names` array `build` returns, reached from the catalog either
    /// directly (`catalog/Dests`) or through the fallback lopdf only consults
    /// when the direct key is absent (`catalog/Names/Dests`) --
    /// `outlines.rs:47-56`. `build` gets the document first so an entry can
    /// point at an object.
    fn names_tree_at(
        under_names: bool,
        build: impl FnOnce(&mut lopdf::Document) -> Vec<Object>,
    ) -> lopdf::Document {
        let mut doc = lopdf::Document::with_version("1.5");
        let names = build(&mut doc);
        let tree = doc.add_object(dictionary! { "Names" => names });
        let cat = if under_names {
            let nm = doc.add_object(dictionary! { "Dests" => tree });
            doc.add_object(dictionary! { "Type" => "Catalog", "Names" => nm })
        } else {
            doc.add_object(dictionary! { "Type" => "Catalog", "Dests" => tree })
        };
        doc.trailer.set("Root", cat);
        doc
    }

    fn names_tree(build: impl FnOnce(&mut lopdf::Document) -> Vec<Object>) -> lopdf::Document {
        names_tree_at(false, build)
    }

    /// A two-element destination array, the shape lopdf indexes happily.
    fn good_dest() -> Vec<Object> {
        vec![Object::Integer(0), Object::Name(b"Fit".to_vec())]
    }

    #[test]
    fn accepts_well_formed_named_destinations() {
        // Both value shapes that carry a /D: an inline dictionary and a
        // reference to one.
        let inline = names_tree(|_| {
            vec![
                Object::string_literal("A"),
                Object::Dictionary(dictionary! { "D" => good_dest() }),
            ]
        });
        assert!(outline_is_traversable(&inline));
        let by_ref = names_tree(|doc| {
            let target = doc.add_object(dictionary! { "D" => good_dest() });
            vec![Object::string_literal("A"), Object::Reference(target)]
        });
        assert!(outline_is_traversable(&by_ref));
        // And the third shape: a reference straight to the array itself.
        let bare_array = names_tree(|doc| {
            let target = doc.add_object(Object::Array(good_dest()));
            vec![Object::string_literal("A"), Object::Reference(target)]
        });
        assert!(outline_is_traversable(&bare_array));
    }

    /// `destinations.rs:56` and `:64` do `dict.get(b"D").as_ref().unwrap()`:
    /// a destination dictionary with no `/D` panics on the `unwrap`.
    #[test]
    fn rejects_named_destination_dictionary_without_d() {
        let by_ref = names_tree(|doc| {
            let target = doc.add_object(dictionary! { "Type" => "Anything" });
            vec![Object::string_literal("A"), Object::Reference(target)]
        });
        assert!(!outline_is_traversable(&by_ref));
        let inline = names_tree(|_| {
            vec![
                Object::string_literal("A"),
                Object::Dictionary(dictionary! { "Type" => "Anything" }),
            ]
        });
        assert!(!outline_is_traversable(&inline));
    }

    /// `destinations.rs:57`, `:60` and `:65` clone `val[0]` and `val[1]` with
    /// no length check, in all three value shapes.
    #[test]
    fn rejects_short_named_destination_array() {
        for short in [Vec::new(), vec![Object::Integer(0)]] {
            let inline = names_tree(|_| {
                vec![
                    Object::string_literal("A"),
                    Object::Dictionary(dictionary! { "D" => short.clone() }),
                ]
            });
            assert!(!outline_is_traversable(&inline), "inline /D {short:?}");
            let bare_array = names_tree(|doc| {
                let target = doc.add_object(Object::Array(short.clone()));
                vec![Object::string_literal("A"), Object::Reference(target)]
            });
            assert!(!outline_is_traversable(&bare_array), "bare array {short:?}");
        }
    }

    /// `destinations.rs:58`, `:61` and `:66` do `key.as_str().unwrap()`: a
    /// name-tree key that is not a string object panics, even with a
    /// perfectly good destination beside it.
    #[test]
    fn rejects_non_string_named_destination_key() {
        let d = names_tree(|_| {
            vec![
                Object::Integer(7),
                Object::Dictionary(dictionary! { "D" => good_dest() }),
            ]
        });
        assert!(!outline_is_traversable(&d));
    }

    /// The other half of the contract: shapes lopdf turns into an `Error` or
    /// skips outright are *not* rejected. Each of these would be a false
    /// rejection -- a document whose outline lopdf handles fine, silently
    /// downgraded to font-size headings.
    #[test]
    fn accepts_named_destination_shapes_lopdf_never_indexes() {
        // A /D that is present but not an array hits `as_array()?`, a plain
        // Err (destinations.rs:56). A reference /D lands here too:
        // `Dictionary::get` is raw, so lopdf never resolves it.
        let non_array_d = names_tree(|_| {
            vec![
                Object::string_literal("A"),
                Object::Dictionary(dictionary! { "D" => Object::Name(b"Fit".to_vec()) }),
            ]
        });
        assert!(outline_is_traversable(&non_array_d));
        // A value that is neither a reference nor a dictionary is skipped
        // (destinations.rs:67-69), as is a reference resolving to neither.
        let neither = names_tree(|_| vec![Object::string_literal("A"), Object::Integer(1)]);
        assert!(outline_is_traversable(&neither));
        let dangling =
            names_tree(|_| vec![Object::string_literal("A"), Object::Reference((99, 0))]);
        assert!(outline_is_traversable(&dangling));
        // An odd trailing entry never becomes a (key, value) pair: lopdf
        // breaks out of the loop (destinations.rs:50-53).
        let odd = names_tree(|_| vec![Object::string_literal("A")]);
        assert!(outline_is_traversable(&odd));
        // A /Names that is not an array is `as_array()?` -> Err.
        let non_array_names = {
            let mut doc = lopdf::Document::with_version("1.5");
            let tree = doc.add_object(dictionary! { "Names" => Object::Integer(1) });
            let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Dests" => tree });
            doc.trailer.set("Root", cat);
            doc
        };
        assert!(outline_is_traversable(&non_array_names));
    }

    /// The `catalog/Names` -> `/Dests` fallback arm. lopdf only looks there
    /// when `catalog/Dests` is absent (`outlines.rs:47-56`) and
    /// `named_destinations_are_traversable` mirrors that with `.or_else`;
    /// every other destination test puts the tree at `catalog/Dests`, so this
    /// is the only cover the mirrored arm has. Both a topology rejection and
    /// a contents rejection have to reach through it, and a good tree has to
    /// survive it.
    #[test]
    fn walks_the_destination_tree_nested_under_catalog_names() {
        let short_dest = names_tree_at(true, |_| {
            vec![
                Object::string_literal("A"),
                Object::Dictionary(dictionary! { "D" => Vec::<Object>::new() }),
            ]
        });
        assert!(!outline_is_traversable(&short_dest));

        let cyclic = {
            let mut doc = lopdf::Document::with_version("1.5");
            let dests = doc.new_object_id();
            doc.objects.insert(
                dests,
                Object::Dictionary(dictionary! { "Kids" => vec![Object::Reference(dests)] }),
            );
            let nm = doc.add_object(dictionary! { "Dests" => dests });
            let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Names" => nm });
            doc.trailer.set("Root", cat);
            doc
        };
        assert!(!outline_is_traversable(&cyclic));

        let good = names_tree_at(true, |_| {
            vec![
                Object::string_literal("A"),
                Object::Dictionary(dictionary! { "D" => good_dest() }),
            ]
        });
        assert!(outline_is_traversable(&good));
    }

    /// A document whose outline is one item, the dictionary `build` returns.
    /// Everything else is well-formed, so a rejection can only have come from
    /// that item.
    fn outline_item(
        build: impl FnOnce(&mut lopdf::Document) -> lopdf::Dictionary,
    ) -> lopdf::Document {
        let mut doc = lopdf::Document::with_version("1.5");
        let item = build(&mut doc);
        let id = doc.add_object(item);
        let root = doc.add_object(dictionary! { "Type" => "Outlines", "First" => id });
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Outlines" => root });
        doc.trailer.set("Root", cat);
        doc
    }

    /// `outlines.rs:100-101` clones `obj_array[0]` and `obj_array[1]` with no
    /// length check. The one-element case is the committed reproducer
    /// `fuzz/artifacts/pdf/crash-e7c53a807e9309b1ff1fe35f15d0b0418624c3cd`,
    /// whose `/Dest` is `[4 <nul...>]`.
    #[test]
    fn rejects_short_dest_array() {
        for short in [Vec::new(), vec![Object::Integer(4)]] {
            let d = outline_item(|_| {
                dictionary! { "Title" => Object::string_literal("X"), "Dest" => short.clone() }
            });
            assert!(!outline_is_traversable(&d), "/Dest {short:?}");
        }
        let good = outline_item(|_| {
            dictionary! { "Title" => Object::string_literal("X"), "Dest" => good_dest() }
        });
        assert!(outline_is_traversable(&good));
    }

    /// `build_outline_result` follows a `Reference` destination with
    /// `get_object` and re-matches what it lands on (`outlines.rs:111-113`),
    /// and `get_object` dereferences the whole chain -- so the value that
    /// gets indexed is the dereferenced one, not the reference.
    #[test]
    fn rejects_short_dest_array_behind_a_reference_chain() {
        let d = outline_item(|doc| {
            let arr = doc.add_object(Object::Array(vec![Object::Integer(3)]));
            let hop = doc.add_object(Object::Reference(arr));
            dictionary! { "Title" => Object::string_literal("X"), "Dest" => hop }
        });
        assert!(!outline_is_traversable(&d));
    }

    /// `get_outline` reads `/A` first and, when it resolves to a dictionary,
    /// takes the destination from the *action's* `/D` and never looks at
    /// `/Dest` at all (`outlines.rs:14-32`). Both directions matter: the
    /// action's `/D` must be checked, and a hostile `/Dest` that lopdf will
    /// never read must not be.
    #[test]
    fn checks_the_action_destination_and_ignores_the_shadowed_dest() {
        let live_action_dest = outline_item(|_| {
            dictionary! {
                "Title" => Object::string_literal("X"),
                "Dest" => good_dest(),
                "A" => dictionary! {
                    "S" => Object::Name(b"GoTo".to_vec()),
                    "D" => vec![Object::Integer(3)],
                },
            }
        });
        assert!(!outline_is_traversable(&live_action_dest));

        let shadowed_dest = outline_item(|_| {
            dictionary! {
                "Title" => Object::string_literal("X"),
                "Dest" => vec![Object::Integer(3)],
                "A" => dictionary! {
                    "S" => Object::Name(b"GoToR".to_vec()),
                    "D" => good_dest(),
                },
            }
        });
        assert!(outline_is_traversable(&shadowed_dest));
    }

    /// Every exit `get_outline` takes *before* it indexes anything. Each
    /// makes it return `Err`, which `get_outlines` ignores as it walks on
    /// (`outlines.rs:71`), so the short destination sitting there is never
    /// touched. Rejecting any of these would be a false rejection.
    #[test]
    fn accepts_outline_nodes_lopdf_never_indexes() {
        let short = || vec![Object::Integer(3)];
        // /S is not GoTo or GoToR (outlines.rs:21-23).
        let wrong_command = outline_item(|_| {
            dictionary! {
                "Title" => Object::string_literal("X"),
                "A" => dictionary! { "S" => Object::Name(b"URI".to_vec()), "D" => short() },
            }
        });
        assert!(outline_is_traversable(&wrong_command));
        // /S missing, or not a name (outlines.rs:20).
        let no_command = outline_item(|_| {
            dictionary! {
                "Title" => Object::string_literal("X"),
                "A" => dictionary! { "D" => short() },
            }
        });
        assert!(outline_is_traversable(&no_command));
        // /Title missing, on the action path (outlines.rs:24) ...
        let action_without_title = outline_item(|_| {
            dictionary! {
                "A" => dictionary! { "S" => Object::Name(b"GoTo".to_vec()), "D" => short() },
            }
        });
        assert!(outline_is_traversable(&action_without_title));
        // ... and on the /Dest path (outlines.rs:17).
        let dest_without_title = outline_item(|_| dictionary! { "Dest" => short() });
        assert!(outline_is_traversable(&dest_without_title));
        // /Title is neither a resolvable reference nor a string
        // (outlines.rs:25-31).
        let unusable_title = outline_item(|_| {
            dictionary! {
                "Title" => Object::Integer(1),
                "A" => dictionary! { "S" => Object::Name(b"GoTo".to_vec()), "D" => short() },
            }
        });
        assert!(outline_is_traversable(&unusable_title));
        // A /Dest that is a name-tree lookup, not an array: a miss is
        // Ok(None) (outlines.rs:103-110), never a panic.
        let named_dest = outline_item(|_| {
            dictionary! {
                "Title" => Object::string_literal("X"),
                "Dest" => Object::string_literal("nowhere"),
            }
        });
        assert!(outline_is_traversable(&named_dest));
        // An item with no destination at all: get_outline errors at
        // `node.get(b"Dest")?`.
        let no_dest = outline_item(|_| dictionary! { "Title" => Object::string_literal("X") });
        assert!(outline_is_traversable(&no_dest));
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
        // `chain(n, "First")` puts its deepest node at depth n -- the walk
        // starts at the root's /First, which is ids[0], at depth 1 -- so the
        // cap is asserted at its exact boundary, the way the node cap below
        // is.
        assert!(outline_is_traversable(&chain(MAX_OUTLINE_DEPTH, "First")));
        // Nodes: a /Next chain stays at depth 1, so only the node cap can fire.
        // This is the bound that stops lopdf growing a Vec per sibling.
        assert!(!outline_is_traversable(&chain(
            MAX_OUTLINE_NODES + 1,
            "Next"
        )));
        assert!(outline_is_traversable(&chain(MAX_OUTLINE_NODES, "Next")));
    }
}
