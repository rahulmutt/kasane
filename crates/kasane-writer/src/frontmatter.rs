use crate::escape::yaml_scalar;
use kasane_core::Frontmatter;

pub(crate) fn frontmatter_yaml(fm: &Frontmatter) -> String {
    let mut y = String::new();
    y.push_str(&format!("title: {}\n", yaml_scalar(&fm.title)));
    if !fm.breadcrumb.is_empty() {
        y.push_str(&format!(
            "breadcrumb: {}\n",
            yaml_scalar(&fm.breadcrumb.join(" > "))
        ));
    }
    if let Some(p) = &fm.parent {
        y.push_str(&format!("parent: {}\n", yaml_scalar(p)));
    }
    if let Some(p) = &fm.prev {
        y.push_str(&format!("prev: {}\n", yaml_scalar(p)));
    }
    if let Some(n) = &fm.next {
        y.push_str(&format!("next: {}\n", yaml_scalar(n)));
    }
    if !fm.children.is_empty() {
        y.push_str("children:\n");
        for c in &fm.children {
            y.push_str(&format!("  - {}\n", yaml_scalar(c)));
        }
    }
    if let Some((s, e)) = fm.source_pages {
        y.push_str(&format!("source_pages: {}-{}\n", s, e));
    }
    y
}

#[cfg(test)]
mod tests {
    use super::*;
    use kasane_core::Frontmatter;

    fn fm(title: &str) -> Frontmatter {
        Frontmatter {
            title: title.to_string(),
            breadcrumb: vec!["Book".into(), title.to_string()],
            parent: Some("../index.md".into()),
            prev: None,
            next: None,
            children: vec!["01-a.md".into()],
            source_pages: Some((1, 2)),
        }
    }

    #[test]
    fn every_string_scalar_is_quoted() {
        let y = frontmatter_yaml(&fm("Notes: a study"));
        assert!(y.contains("title: \"Notes: a study\""), "{y}");
        assert!(y.contains("breadcrumb: \"Book > Notes: a study\""), "{y}");
        assert!(y.contains("parent: \"../index.md\""), "{y}");
        assert!(y.contains("  - \"01-a.md\""), "{y}");
        // Numeric, built by format! from two integers: never text, never quoted.
        assert!(y.contains("source_pages: 1-2"), "{y}");
    }

    #[test]
    fn a_title_that_would_break_the_block_is_contained() {
        let y = frontmatter_yaml(&fm("- \"quoted\"\nsecond"));
        assert!(y.contains("title: \"- \\\"quoted\\\" second\""), "{y}");
        assert_eq!(
            y.lines().filter(|l| l.starts_with("title:")).count(),
            1,
            "the title must not spill onto a second line: {y}"
        );
    }
}
