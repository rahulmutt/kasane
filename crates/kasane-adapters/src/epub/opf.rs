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
    // quick-xml does not resolve external entities -> XXE-safe by default.
    let mut title = String::new();
    let mut authors = vec![];
    let mut language = None;
    let mut manifest: std::collections::HashMap<String, String> = Default::default();
    let mut spine_ids: Vec<String> = vec![];
    let mut cur: Option<&'static str> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"title" => cur = Some("title"),
                    b"creator" => cur = Some("creator"),
                    b"language" => cur = Some("language"),
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
                let txt = t.unescape().unwrap_or_default().to_string();
                match cur.take() {
                    Some("title") => title = txt,
                    Some("creator") => authors.push(txt),
                    Some("language") => language = Some(txt),
                    _ => {}
                }
            }
            Ok(Event::End(_)) => cur = None,
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
