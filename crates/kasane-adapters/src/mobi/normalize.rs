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

fn is_block_level(h: &Handle) -> bool {
    matches!(&h.data, NodeData::Element { name, .. } if BLOCK_LEVEL.contains(&name.local.as_ref()))
}

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
        for child in body.children.borrow().iter() {
            serialize(child, hook, &mut out);
        }
    }
    out.push_str("</body>");
    out
}

fn find_element(h: &Handle, name: &str) -> Option<Handle> {
    if let NodeData::Element { name: n, .. } = &h.data {
        if n.local.as_ref() == name {
            return Some(h.clone());
        }
    }
    for c in h.children.borrow().iter() {
        if let Some(found) = find_element(c, name) {
            return Some(found);
        }
    }
    None
}

fn serialize(h: &Handle, hook: AttrHook, out: &mut String) {
    match &h.data {
        NodeData::Text { contents } => out.push_str(&escape_text(&contents.borrow())),
        NodeData::Comment { .. } | NodeData::ProcessingInstruction { .. } => {}
        NodeData::Element { name, attrs, .. } => {
            let tag = name.local.as_ref().to_string();
            // mbp:pagebreak and friends: drop the tag, keep any children.
            if tag.contains(':') {
                for c in h.children.borrow().iter() {
                    serialize(c, hook, out);
                }
                return;
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
                return;
            }
            out.push('>');
            if HEADING.contains(&tag.as_str()) {
                let children = h.children.borrow();
                let split = children.iter().position(is_block_level);
                let (head, rest) = match split {
                    Some(i) => children.split_at(i),
                    None => (&children[..], &[][..]),
                };
                for c in head {
                    serialize(c, hook, out);
                }
                out.push_str("</");
                out.push_str(&tag);
                out.push('>');
                for c in rest {
                    serialize(c, hook, out);
                }
                return;
            }
            for c in h.children.borrow().iter() {
                serialize(c, hook, out);
            }
            out.push_str("</");
            out.push_str(&tag);
            out.push('>');
        }
        // Document / Doctype wrappers: recurse into children.
        _ => {
            for c in h.children.borrow().iter() {
                serialize(c, hook, out);
            }
        }
    }
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
}
