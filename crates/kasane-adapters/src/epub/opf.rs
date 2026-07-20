use quick_xml::events::Event;
use quick_xml::Reader;

pub struct Opf {
    pub title: String,
    pub authors: Vec<String>,
    pub language: Option<String>,
    pub spine_hrefs: Vec<String>,
}

pub fn parse_opf(xml: &str, opf_dir: &str) -> Opf {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    // A bare `&` in e.g. <dc:title> is common in real EPUBs. quick-xml 0.41
    // raises IllFormedError on it (0.36 passed it through); this loop's
    // `Err(_) => break` would then abandon the OPF before <manifest>/<spine>,
    // losing the ENTIRE book at exit 0. Recover the `&` as literal text instead.
    reader.config_mut().allow_dangling_amp = true;
    // quick-xml does not resolve external entities -> XXE-safe by default.
    let mut title = String::new();
    let mut authors = vec![];
    let mut language = None;
    let mut manifest: std::collections::HashMap<String, String> = Default::default();
    let mut spine_ids: Vec<String> = vec![];
    // `cur` marks which metadata element we are inside; `acc` accumulates its
    // text across every Text/GeneralRef fragment until End flushes it. quick-xml
    // 0.41 splits `A &amp; B` into three events, so a parser that assigns on the
    // first fragment keeps only `A ` and drops the rest.
    let mut cur: Option<&'static str> = None;
    let mut acc = String::new();
    let mut buf = Vec::new();

    // Commits whatever text accumulated for the element that just ended.
    macro_rules! flush {
        () => {
            // An empty element carries no text events, so it must not commit an
            // empty author/title -- matching the pre-accumulation behavior.
            match cur.take().filter(|_| !acc.is_empty()) {
                Some("title") => title = std::mem::take(&mut acc),
                // One push per <dc:creator> element, not per text fragment.
                Some("creator") => authors.push(std::mem::take(&mut acc)),
                Some("language") => language = Some(std::mem::take(&mut acc)),
                _ => acc.clear(),
            }
        };
    }

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"title" => {
                        cur = Some("title");
                        acc.clear();
                    }
                    b"creator" => {
                        cur = Some("creator");
                        acc.clear();
                    }
                    b"language" => {
                        cur = Some("language");
                        acc.clear();
                    }
                    b"item" => {
                        let (mut id, mut href) = (String::new(), String::new());
                        for a in e.attributes().flatten() {
                            match a.key.as_ref() {
                                b"id" => id = String::from_utf8_lossy(&a.value).into(),
                                b"href" => href = String::from_utf8_lossy(&a.value).into(),
                                _ => {}
                            }
                        }
                        if !id.is_empty() {
                            manifest.insert(id, join_href(opf_dir, &href));
                        }
                    }
                    b"itemref" => {
                        for a in e.attributes().flatten() {
                            if a.key.as_ref() == b"idref" {
                                spine_ids.push(String::from_utf8_lossy(&a.value).into());
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if cur.is_some() {
                    // No unescape() here: the reader splits text at every
                    // reference, so an Event::Text can never contain a `&...;`.
                    // With allow_dangling_amp it would also turn a recovered
                    // `& Jerry` into "" via Err(UnterminatedEntity).
                    let txt = t.decode().map(|d| d.into_owned()).unwrap_or_default();
                    acc.push_str(&txt);
                }
            }
            // quick-xml 0.41 emits entity/character references in text content as
            // their own event instead of folding them into Event::Text.
            Ok(Event::GeneralRef(r)) => {
                if cur.is_some() {
                    acc.push_str(&crate::xmltext::resolve_general_ref(&r));
                }
            }
            Ok(Event::End(_)) => flush!(),
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    let spine_hrefs = spine_ids
        .iter()
        .filter_map(|id| manifest.get(id).cloned())
        .collect();
    Opf {
        title,
        authors,
        language,
        spine_hrefs,
    }
}

fn join_href(dir: &str, href: &str) -> String {
    if dir.is_empty() {
        href.to_string()
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), href)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_title_text_entities() {
        // `A &amp; B` puts the reference between two Text fragments, so under
        // quick-xml 0.41 -- which splits text at every reference -- this
        // exercises resolve_general_ref's unescape() call via the accumulator
        // path. Event::Text can never contain a `&...;` once the reader splits
        // on it, so that arm only decodes and deliberately does not unescape.
        let xml = r#"<package><metadata>
          <dc:title>A &amp; B</dc:title>
        </metadata></package>"#;
        let opf = parse_opf(xml, "OEBPS");
        assert_eq!(opf.title, "A & B");
    }

    #[test]
    fn bare_ampersand_in_title_does_not_lose_the_manifest_and_spine() {
        // The total-loss case: a dangling `&` in <dc:title> raised
        // IllFormedError under quick-xml 0.41, and `Err(_) => break` abandoned
        // the OPF before <manifest>/<spine> -- the whole book vanished at
        // exit 0. Assert the spine survives, not merely that the title reads
        // correctly.
        let xml = r#"<package><metadata>
          <dc:title>Tom & Jerry</dc:title>
        </metadata>
        <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
        <spine><itemref idref="c1"/></spine>
        </package>"#;
        let opf = parse_opf(xml, "OEBPS");
        assert_eq!(opf.title, "Tom & Jerry");
        assert_eq!(opf.spine_hrefs, vec!["OEBPS/ch1.xhtml".to_string()]);
    }

    #[test]
    fn resolves_numeric_and_boundary_references_in_metadata() {
        // References at the leading and trailing edge, plus decimal and hex
        // character references. quick-xml 0.41 splits each into its own event,
        // so the accumulator -- not the first fragment -- must decide the value.
        let xml = r#"<package><metadata>
          <dc:title>&lt;caf&#233;&#xE9;&gt;</dc:title>
          <dc:language>en&#45;GB</dc:language>
        </metadata></package>"#;
        let opf = parse_opf(xml, "OEBPS");
        assert_eq!(opf.title, "<caféé>");
        assert_eq!(opf.language.as_deref(), Some("en-GB"));
    }

    #[test]
    fn each_creator_element_yields_exactly_one_author() {
        // The accumulator commits on End, so a creator split across several
        // text/reference fragments must still push a single entry.
        let xml = r#"<package><metadata>
          <dc:creator>Ann &amp; Bob</dc:creator>
          <dc:creator>Cy</dc:creator>
          <dc:creator></dc:creator>
        </metadata></package>"#;
        let opf = parse_opf(xml, "OEBPS");
        assert_eq!(opf.authors, vec!["Ann & Bob".to_string(), "Cy".to_string()]);
    }

    #[test]
    fn keeps_unresolvable_entity_as_source_text() {
        let xml = r#"<package><metadata>
          <dc:title>a&nbsp;b</dc:title>
        </metadata></package>"#;
        let opf = parse_opf(xml, "OEBPS");
        assert_eq!(opf.title, "a&nbsp;b");
    }
}
