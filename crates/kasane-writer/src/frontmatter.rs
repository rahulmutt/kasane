use kasane_core::Frontmatter;

pub(crate) fn frontmatter_yaml(fm: &Frontmatter) -> String {
    let mut y = String::new();
    y.push_str(&format!("title: {}\n", yaml_str(&fm.title)));
    if !fm.breadcrumb.is_empty() {
        y.push_str(&format!("breadcrumb: {}\n", fm.breadcrumb.join(" > ")));
    }
    if let Some(p) = &fm.parent {
        y.push_str(&format!("parent: {}\n", p));
    }
    if let Some(p) = &fm.prev {
        y.push_str(&format!("prev: {}\n", p));
    }
    if let Some(n) = &fm.next {
        y.push_str(&format!("next: {}\n", n));
    }
    if !fm.children.is_empty() {
        y.push_str("children:\n");
        for c in &fm.children {
            y.push_str(&format!("  - {}\n", c));
        }
    }
    if let Some((s, e)) = fm.source_pages {
        y.push_str(&format!("source_pages: {}-{}\n", s, e));
    }
    y
}

fn yaml_str(s: &str) -> String {
    if s.contains(':') || s.contains('#') {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}
