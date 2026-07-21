use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// Per-element attribute rewrite hook: (lowercased tag name, attrs).
pub(crate) type AttrHook<'a> = &'a dyn Fn(&str, &mut Vec<(String, String)>);

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

const HEADING: &[&str] = &["h1", "h2", "h3", "h4", "h5", "h6"];

// html5ever's tree construction is spec-correct in leaving a heading open
// across a nested <p> (`<h1>T<p>x` parses as `<h1>T<p>x</p></h1>`, matching
// real browsers -- the HTML5 "optional tags" appendix never allows an h1..h6
// end tag to be implied). Mobipocket Creator output relies on exactly that
// omission, and `epub::xhtml::xhtml_to_blocks` flattens any block nested
// inside a still-open block into the outer block's inline text (spec §2
// "flatten, never drop"), so left as-is a dangling `<h1>Title<p>body` would
// silently fold the whole paragraph into the heading. Closing the heading
// early -- and re-emitting the block-level child (and everything after it)
// as a sibling -- turns that MOBI-ism into two separate, well-formed blocks.
//
// The offending block-level element isn't always a *direct* child of the
// heading: formatting elements don't auto-close on `<p>` either, so
// `<h1><b>T<p>x` puts the `<p>` inside the still-open `<b>`
// (`h1 > b > [Text, p]`). `serialize` therefore searches for the cut point
// through inline wrappers, not just among the heading's direct children --
// see the `scanning` parameter.
const BLOCK_LEVEL: &[&str] = &[
    "p",
    "div",
    "ul",
    "ol",
    "table",
    "blockquote",
    "pre",
    "hr",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "figure",
    "aside",
    "dl",
];

// Hostile MOBI content can nest tens of thousands of tags deep. Recursing
// `serialize` one native stack frame per level with no cap risks overflowing
// the thread stack; capping the depth and dropping (not corrupting) whatever
// lies beyond it is the "degrade, don't die" choice for an untrusted-input
// boundary. Chosen empirically: a debug build's default (2 MiB) test-thread
// stack overflowed around ~1,500 levels of `serialize`/`serialize_children`
// mutual recursion (see `deeply_nested_input_does_not_overflow_the_stack`),
// so 500 leaves a healthy margin while still being far beyond any real
// e-book's markup nesting (which rarely exceeds a few dozen levels).
const MAX_DEPTH: usize = 500;

/// Parse sloppy MOBI-era HTML with html5ever (which applies real HTML5
/// recovery: closes dangling <p>, quotes bare attributes, repairs nesting)
/// and re-serialize just the <body> as well-formed XHTML for
/// `epub::xhtml::xhtml_to_blocks`. The round trip is deliberate: it keeps
/// the hardened streaming parser as the single IR emitter.
pub(crate) fn normalize_html(html: &str, hook: AttrHook) -> String {
    let dom = html5ever::parse_document(RcDom::default(), Default::default())
        .from_utf8()
        // Reading from an in-memory slice cannot fail with an I/O error.
        .read_from(&mut html.as_bytes())
        .expect("in-memory read");
    let mut out = String::with_capacity(html.len() + html.len() / 4);
    out.push_str("<body>");
    if let Some(body) = find_element(&dom.document, "body") {
        let children: Vec<Handle> = body.children.borrow().iter().cloned().collect();
        for child in &children {
            serialize(child, hook, &mut out, 0, false);
        }
    }
    out.push_str("</body>");
    out
}

// Iterative (explicit-stack) search: a recursive descent here would recurse
// one native frame per DOM level with no cap, and a hostile MOBI can nest
// tens of thousands of tags before (or instead of) ever opening a <body>.
// The stack lives on the heap, so depth is bounded only by memory, not by
// the thread's call stack. Children are pushed in reverse so popping (LIFO)
// still visits them in the same left-to-right, pre-order sequence the old
// recursive version did -- the first `body` encountered in document order
// wins, same as before.
fn find_element(root: &Handle, name: &str) -> Option<Handle> {
    let mut stack = vec![root.clone()];
    while let Some(h) = stack.pop() {
        if let NodeData::Element { name: n, .. } = &h.data {
            if n.local.as_ref() == name {
                return Some(h);
            }
        }
        for c in h.children.borrow().iter().rev() {
            stack.push(c.clone());
        }
    }
    None
}

// Serializes one node. `depth` bounds native recursion (see MAX_DEPTH).
//
// `scanning` is true while walking inside a still-open heading (including
// through any inline wrappers nested in it): the moment a block-level
// element is reached, this returns `Some(leftover)` instead of writing it,
// where `leftover` is that element plus every node after it in document
// order (its own remaining siblings). The caller at each level either closes
// its own tag and re-throws `leftover` (inline wrapper, e.g. <b>) or -- if
// it IS the heading -- closes the heading and then serializes `leftover` at
// the top level, non-scanning, before returning `None` itself (the cut is
// fully resolved once a heading absorbs it, so it never bubbles past its
// own heading).
fn serialize(
    h: &Handle,
    hook: AttrHook,
    out: &mut String,
    depth: usize,
    scanning: bool,
) -> Option<Vec<Handle>> {
    if depth > MAX_DEPTH {
        // Degrade: stop descending into this subtree rather than risk a
        // stack overflow. Whatever this node's parent already opened still
        // gets closed by the caller, so the output stays well-formed; only
        // content beyond the cap is dropped.
        return None;
    }
    match &h.data {
        NodeData::Text { contents } => {
            out.push_str(&escape_text(&contents.borrow()));
            None
        }
        NodeData::Comment { .. } | NodeData::ProcessingInstruction { .. } => None,
        NodeData::Element { name, attrs, .. } => {
            let tag = name.local.as_ref().to_string();
            // mbp:pagebreak and friends: drop the tag, keep any children.
            if tag.contains(':') {
                return serialize_children(h, hook, out, depth, scanning);
            }
            if scanning && BLOCK_LEVEL.contains(&tag.as_str()) {
                // Cut point: leave this node (and its subtree) completely
                // untouched -- the caller re-serializes it fresh, outside
                // the heading, once the heading closes.
                return Some(vec![h.clone()]);
            }
            let mut a: Vec<(String, String)> = attrs
                .borrow()
                .iter()
                .map(|at| (at.name.local.as_ref().to_string(), at.value.to_string()))
                .collect();
            hook(&tag, &mut a);
            out.push('<');
            out.push_str(&tag);
            for (k, v) in &a {
                // A hostile attribute name could otherwise break out of the tag.
                if !k.is_empty()
                    && k.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ':' || c == '_')
                {
                    out.push(' ');
                    out.push_str(k);
                    out.push_str("=\"");
                    out.push_str(&escape_attr(v));
                    out.push('"');
                }
            }
            if VOID.contains(&tag.as_str()) {
                out.push_str("/>");
                return None;
            }
            out.push('>');
            let is_heading = HEADING.contains(&tag.as_str());
            let leftover = serialize_children(h, hook, out, depth, scanning || is_heading);
            out.push_str("</");
            out.push_str(&tag);
            out.push('>');
            if is_heading {
                // Absorb the cut here: promote whatever's left to sibling
                // content, fully outside the heading, and stop propagating.
                if let Some(promoted) = leftover {
                    for n in &promoted {
                        serialize(n, hook, out, depth, false);
                    }
                }
                None
            } else {
                leftover
            }
        }
        // Document / Doctype wrappers: recurse into children.
        _ => serialize_children(h, hook, out, depth, scanning),
    }
}

fn serialize_children(
    h: &Handle,
    hook: AttrHook,
    out: &mut String,
    depth: usize,
    scanning: bool,
) -> Option<Vec<Handle>> {
    let children: Vec<Handle> = h.children.borrow().iter().cloned().collect();
    for (i, c) in children.iter().enumerate() {
        if let Some(mut leftover) = serialize(c, hook, out, depth + 1, scanning) {
            leftover.extend(children[i + 1..].iter().cloned());
            return Some(leftover);
        }
    }
    None
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_hook() -> impl Fn(&str, &mut Vec<(String, String)>) {
        |_: &str, _: &mut Vec<(String, String)>| {}
    }

    #[test]
    fn closes_unclosed_paragraphs_and_wraps_in_body() {
        let out = normalize_html("<p>one<p>two", &no_hook());
        assert_eq!(out, "<body><p>one</p><p>two</p></body>");
    }

    #[test]
    fn quotes_bare_attributes_and_self_closes_voids() {
        let out = normalize_html(
            "<p><a filepos=0000000042>x</a><br><img src=i.png></p>",
            &no_hook(),
        );
        assert!(out.contains("<a filepos=\"0000000042\">x</a>"));
        assert!(out.contains("<br/>"));
        assert!(out.contains("<img src=\"i.png\"/>"));
    }

    #[test]
    fn drops_namespaced_tags_but_keeps_children() {
        let out = normalize_html("<p>a</p><mbp:pagebreak/><p>b</p>", &no_hook());
        assert!(!out.contains("mbp"));
        assert!(out.contains("<p>a</p>") && out.contains("<p>b</p>"));
    }

    #[test]
    fn escapes_text_and_attr_values() {
        let out = normalize_html("<p title='a<b&\"c'>x & y < z</p>", &no_hook());
        assert!(out.contains("x &amp; y &lt; z"));
        assert!(out.contains("a&lt;b&amp;&quot;c"));
    }

    #[test]
    fn hook_rewrites_attributes() {
        let hook = |tag: &str, attrs: &mut Vec<(String, String)>| {
            if tag == "img" {
                if let Some(i) = attrs.iter().position(|(k, _)| k == "recindex") {
                    let n = attrs[i].1.trim_start_matches('0').to_string();
                    attrs.retain(|(k, _)| k != "recindex" && k != "src");
                    attrs.push(("src".into(), format!("kasane-rec-{n}")));
                }
            }
        };
        let out = normalize_html("<p><img recindex=\"00003\" alt=\"pic\"></p>", &hook);
        assert!(out.contains("src=\"kasane-rec-3\""));
        assert!(!out.contains("recindex"));
        assert!(out.contains("alt=\"pic\""));
    }

    #[test]
    fn drops_comments_and_head_content() {
        let out = normalize_html(
            "<html><head><title>T</title></head><body><!-- c --><p>x</p></body></html>",
            &no_hook(),
        );
        assert_eq!(out, "<body><p>x</p></body>");
    }

    #[test]
    fn feeds_cleanly_into_the_epub_parser() {
        let out = normalize_html("<h1>T<p>alpha <b>bee", &no_hook());
        let mut id = 0u32;
        let mut note = 1u32;
        let fp = crate::epub::xhtml::xhtml_to_blocks(&out, "", &mut id, &mut note);
        assert!(fp
            .blocks
            .iter()
            .any(|b| matches!(b, kasane_ir::Block::Heading { .. })));
        assert!(fp
            .blocks
            .iter()
            .any(|b| matches!(b, kasane_ir::Block::Para(_))));
    }

    // Recursively flattens an inline vec's text, mirroring epub::xhtml's own
    // test helper -- just enough to check whether a given word ended up
    // inside a heading's inline content.
    fn text_of(inls: &[kasane_ir::Inline]) -> String {
        let mut s = String::new();
        for i in inls {
            match i {
                kasane_ir::Inline::Text(t) | kasane_ir::Inline::Code(t) => s.push_str(t),
                kasane_ir::Inline::Emph(x) | kasane_ir::Inline::Strong(x) => {
                    s.push_str(&text_of(x))
                }
                kasane_ir::Inline::Link { inlines, .. } => s.push_str(&text_of(inlines)),
                kasane_ir::Inline::Math(_) | kasane_ir::Inline::FootnoteRef(_) => {}
            }
        }
        s
    }

    #[test]
    fn heading_split_detects_block_level_through_inline_wrapper() {
        // Unlike `feeds_cleanly_into_the_epub_parser`'s `<h1>T<p>...`, the
        // <p> here is not a direct child of <h1>: html5ever doesn't
        // auto-close <b> on <p> either, so tree construction nests it as
        // `h1 > b > [Text("Title"), p]`. The heading-split logic must find
        // the block-level descendant through the inline wrapper, not just
        // among <h1>'s direct children, or "body text" silently folds into
        // the heading (spec's flatten-never-drop rule in xhtml_to_blocks).
        let out = normalize_html("<h1><b>Title<p>body text", &no_hook());
        let mut id = 0u32;
        let mut note = 1u32;
        let fp = crate::epub::xhtml::xhtml_to_blocks(&out, "", &mut id, &mut note);

        let heading_text = fp
            .blocks
            .iter()
            .find_map(|b| match b {
                kasane_ir::Block::Heading { inlines, .. } => Some(text_of(inlines)),
                _ => None,
            })
            .expect("a heading");
        assert!(
            !heading_text.contains("body text"),
            "paragraph must not be folded into the heading, got heading text {heading_text:?}"
        );
        assert!(
            fp.blocks
                .iter()
                .any(|b| matches!(b, kasane_ir::Block::Para(_))),
            "paragraph must survive as its own block, got {:?}",
            fp.blocks
        );
    }

    #[test]
    fn deeply_nested_input_does_not_overflow_the_stack() {
        // A hostile MOBI file with tens of thousands of nested tags must
        // degrade (drop content beyond MAX_DEPTH), not crash the process.
        // Exercises `serialize`'s depth cap, `find_element`'s iterative
        // (non-recursive) walk, and whatever the underlying RcDom tree does
        // when it drops at the end of this call.
        //
        // Uses <i> rather than <div>: profiling this test while tuning
        // MAX_DEPTH found html5ever's own tree construction is roughly
        // quadratic in nesting depth for <div> specifically (100k <div>s:
        // tens of seconds just to parse, before normalize_html's own code
        // ever runs) but near-linear for <i> (100k <i>s parses in well
        // under a second). That parse-time cost is upstream of this crate
        // and not a stack-overflow risk (no crash, just slow), so it's
        // orthogonal to what this test is pinning; <i> keeps the test fast
        // while still exercising 100k levels of native-recursion avoidance.
        const N: usize = 100_000;
        let mut html = String::with_capacity(N * "<i>".len() + N * "</i>".len() + 4);
        for _ in 0..N {
            html.push_str("<i>");
        }
        html.push_str("text");
        for _ in 0..N {
            html.push_str("</i>");
        }
        let out = normalize_html(&html, &no_hook());
        assert!(out.starts_with("<body>"));
        assert!(out.ends_with("</body>"));
    }
}
