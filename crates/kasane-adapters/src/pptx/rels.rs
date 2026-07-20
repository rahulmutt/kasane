use crate::guard::resolve_rel;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;

pub struct Rel {
    pub id: String,
    pub ty: String,
    pub target: String,
    pub external: bool,
}

fn attr(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .map(unescape_attr)
}

/// Returns the local part of an attribute key: the substring after the last
/// `:`, or the whole key if there is no `:`. Used to match namespaced
/// attributes (e.g. `r:id`, `rel:id`) prefix-agnostically.
fn local_name(key: &[u8]) -> &[u8] {
    match key.iter().rposition(|&b| b == b':') {
        Some(i) => &key[i + 1..],
        None => key,
    }
}

/// Finds the first *namespaced* attribute (key containing a `:`) whose local
/// name matches `local`. Used to match e.g. `r:id` or `rel:id` regardless of
/// which prefix the document binds to the relationships namespace, without
/// accidentally matching an unrelated unprefixed attribute of the same local
/// name (e.g. `sldId`'s own unprefixed `id` attribute).
///
/// `xmlns:` declarations are explicitly excluded: `xmlns:embed="..."` has the
/// same local name (`embed`) as a real `r:embed` attribute, so a namespace
/// declaration placed earlier in the tag would otherwise shadow the genuine
/// attribute and silently drop the reference it carries.
pub(crate) fn attr_local(e: &quick_xml::events::BytesStart, local: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| {
            let key = a.key.as_ref();
            key.contains(&b':') && !key.starts_with(b"xmlns:") && local_name(key) == local
        })
        .map(unescape_attr)
}

/// Unescapes an attribute's XML entities (e.g. `&amp;` -> `&`). Falls back to
/// the raw lossy-UTF8 value if unescaping fails, so malformed/untrusted input
/// degrades gracefully instead of panicking or aborting the parse.
pub(crate) fn unescape_attr(a: quick_xml::events::attributes::Attribute) -> String {
    a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
        .map(|v| v.into_owned())
        .unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned())
}

pub fn parse_rels(xml: &str) -> Vec<Rel> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut out = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"Relationship" => {
                let id = attr(&e, b"Id").unwrap_or_default();
                let ty = attr(&e, b"Type").unwrap_or_default();
                let target = attr(&e, b"Target").unwrap_or_default();
                let external = attr(&e, b"TargetMode").as_deref() == Some("External");
                if !id.is_empty() {
                    out.push(Rel {
                        id,
                        ty,
                        target,
                        external,
                    });
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

pub fn parse_slide_order(presentation_xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(presentation_xml);
    reader.config_mut().expand_empty_elements = true;
    let mut out = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"sldId" => {
                if let Some(id) = attr_local(&e, b"id") {
                    out.push(id);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

pub enum RelTarget {
    External(String),
    Internal(String),
}

pub struct SlideRels(pub HashMap<String, RelTarget>);

impl SlideRels {
    pub fn empty() -> Self {
        SlideRels(HashMap::new())
    }

    /// Build from parsed rels, resolving internal targets against `base_dir`.
    /// Internal targets that escape the archive root are dropped.
    pub fn from_rels(rels: Vec<Rel>, base_dir: &str) -> Self {
        let mut map = HashMap::new();
        for r in rels {
            let t = if r.external {
                RelTarget::External(r.target)
            } else {
                match resolve_rel(base_dir, &r.target) {
                    Some(p) => RelTarget::Internal(p),
                    None => continue,
                }
            };
            map.insert(r.id, t);
        }
        SlideRels(map)
    }

    pub fn get(&self, id: &str) -> Option<&RelTarget> {
        self.0.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_slide_order_from_sldidlst() {
        // Note the r:id order is 3 then 2 — display order differs from filename order.
        let xml = r#"<p:presentation xmlns:r="x">
          <p:sldIdLst>
            <p:sldId id="256" r:id="rId3"/>
            <p:sldId id="257" r:id="rId2"/>
          </p:sldIdLst></p:presentation>"#;
        assert_eq!(parse_slide_order(xml), vec!["rId3", "rId2"]);
    }

    #[test]
    fn parses_relationships_with_targetmode() {
        let xml = r#"<Relationships>
          <Relationship Id="rId2" Type="http://x/slide" Target="slides/slide1.xml"/>
          <Relationship Id="rId3" Type="http://x/hyperlink" Target="https://e.com" TargetMode="External"/>
        </Relationships>"#;
        let rels = parse_rels(xml);
        assert_eq!(rels.len(), 2);
        let hy = rels.iter().find(|r| r.id == "rId3").unwrap();
        assert!(hy.external);
        assert!(hy.ty.ends_with("hyperlink"));
        assert_eq!(hy.target, "https://e.com");
    }

    #[test]
    fn slide_rels_resolves_internal_vs_external() {
        let xml = r#"<Relationships>
          <Relationship Id="rId2" Type="http://x/image" Target="../media/image1.png"/>
          <Relationship Id="rId3" Type="http://x/hyperlink" Target="https://e.com" TargetMode="External"/>
        </Relationships>"#;
        let sr = SlideRels::from_rels(parse_rels(xml), "ppt/slides");
        assert!(
            matches!(sr.get("rId2"), Some(RelTarget::Internal(p)) if p == "ppt/media/image1.png")
        );
        assert!(matches!(sr.get("rId3"), Some(RelTarget::External(u)) if u == "https://e.com"));
    }

    #[test]
    fn unescapes_target_attribute_entities() {
        let xml = r#"<Relationships>
          <Relationship Id="rId1" Type="http://x/hyperlink" Target="page.html?a=1&amp;b=2" TargetMode="External"/>
        </Relationships>"#;
        let rels = parse_rels(xml);
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].target, "page.html?a=1&b=2");
    }

    #[test]
    fn parses_slide_order_with_non_r_prefix() {
        let xml = r#"<p:presentation xmlns:rel="x">
          <p:sldIdLst>
            <p:sldId id="256" rel:id="rId3"/>
            <p:sldId id="257" rel:id="rId2"/>
          </p:sldIdLst></p:presentation>"#;
        assert_eq!(parse_slide_order(xml), vec!["rId3", "rId2"]);
    }

    #[test]
    fn attr_local_ignores_xmlns_decoy_for_blip_embed() {
        // A preceding xmlns:embed declaration has local name "embed", same as
        // the real r:embed attribute. It must not shadow the real one.
        use quick_xml::events::Event;
        use quick_xml::Reader;
        let xml = r#"<a:blip xmlns:embed="EVIL" r:embed="rId3"/>"#;
        let mut reader = Reader::from_str(xml);
        reader.config_mut().expand_empty_elements = true;
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf).unwrap() {
                Event::Start(e) => {
                    assert_eq!(attr_local(&e, b"embed").as_deref(), Some("rId3"));
                    break;
                }
                Event::Eof => panic!("no start event"),
                _ => {}
            }
            buf.clear();
        }
    }

    #[test]
    fn parses_slide_order_ignoring_xmlns_id_decoy() {
        // A preceding xmlns:id declaration has local name "id", same as the
        // real r:id attribute on sldId. It must not shadow the real one.
        let xml = r#"<p:presentation xmlns:id="x">
          <p:sldIdLst>
            <p:sldId id="256" xmlns:id="EVIL" r:id="rId3"/>
            <p:sldId id="257" r:id="rId2"/>
          </p:sldIdLst></p:presentation>"#;
        assert_eq!(parse_slide_order(xml), vec!["rId3", "rId2"]);
    }
}
