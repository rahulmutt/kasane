pub const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_RATIO: u64 = 200;

/// Sanitize a zip entry name; None if it escapes the archive root.
pub fn safe_entry_name(name: &str) -> Option<String> {
    if name.starts_with('/') || name.contains("..") {
        return None;
    }
    Some(name.to_string())
}

/// Guard against decompression bombs given compressed and (running) decompressed sizes.
pub fn check_expansion(compressed: u64, decompressed: u64) -> bool {
    decompressed <= MAX_TOTAL_BYTES
        && (compressed == 0 || decompressed / compressed.max(1) <= MAX_RATIO)
}

/// Resolve a relationship `target` (which may contain `..`) against `base_dir`,
/// normalizing `.`/`..` and confining the result to the archive root. A leading
/// `/` makes the target package-absolute (resolved from root). Returns `None` if
/// the target escapes the root or resolves to nothing.
pub fn resolve_rel(base_dir: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = if target.starts_with('/') || base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// True when `href` starts with a URL scheme (`http:`, `data:`, `mailto:`, …)
/// rather than being a document-relative path. A colon only counts before the
/// first `/`, `#`, or `?`.
pub(crate) fn has_scheme(href: &str) -> bool {
    href.chars()
        .take_while(|c| !matches!(c, '/' | '#' | '?'))
        .any(|c| c == ':')
}

pub(crate) fn parent_dir(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(d, _)| d.to_string())
        .unwrap_or_default()
}

pub(crate) fn safe_media_filename(archive_path: &str, n: usize) -> String {
    let base = archive_path.rsplit('/').next().unwrap_or("image");
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Prefix an index to guarantee uniqueness even if basenames collide across dirs.
    format!(
        "{:03}-{}",
        n,
        if cleaned.is_empty() {
            "image".into()
        } else {
            cleaned
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn has_scheme_detects_urls_not_paths() {
        assert!(has_scheme("http://x/a.png"));
        assert!(has_scheme("data:image/png;base64,AA"));
        assert!(has_scheme("mailto:a@b"));
        assert!(!has_scheme("images/a.png"));
        assert!(!has_scheme("../images/a.png"));
        assert!(!has_scheme("a/b:c.png")); // colon after a slash is not a scheme
        assert!(!has_scheme("#frag"));
    }

    #[test]
    fn rejects_traversal_names() {
        assert!(safe_entry_name("../etc/passwd").is_none());
        assert!(safe_entry_name("/abs").is_none());
        assert_eq!(
            safe_entry_name("OEBPS/ch1.xhtml"),
            Some("OEBPS/ch1.xhtml".to_string())
        );
    }

    #[test]
    fn check_expansion_ratio_boundary() {
        assert!(check_expansion(1, 200));
        assert!(!check_expansion(1, 201));
    }

    #[test]
    fn check_expansion_absolute_cap_boundary() {
        assert!(check_expansion(
            super::MAX_TOTAL_BYTES / 100,
            super::MAX_TOTAL_BYTES
        ));
        assert!(!check_expansion(
            super::MAX_TOTAL_BYTES / 100,
            super::MAX_TOTAL_BYTES + 1
        ));
    }

    #[test]
    fn resolve_rel_normalizes_and_confines() {
        // media referenced from a slide: ../media/image1.png relative to ppt/slides
        assert_eq!(
            resolve_rel("ppt/slides", "../media/image1.png").as_deref(),
            Some("ppt/media/image1.png")
        );
        // slide referenced from presentation rels: base ppt
        assert_eq!(
            resolve_rel("ppt", "slides/slide1.xml").as_deref(),
            Some("ppt/slides/slide1.xml")
        );
        // "." and empty segments are ignored
        assert_eq!(
            resolve_rel("ppt/slides", "./../media/./i.png").as_deref(),
            Some("ppt/media/i.png")
        );
        // leading slash is package-absolute (from archive root)
        assert_eq!(
            resolve_rel("ppt/slides", "/ppt/media/i.png").as_deref(),
            Some("ppt/media/i.png")
        );
        // escaping the root is rejected
        assert_eq!(resolve_rel("ppt", "../../etc/passwd"), None);
        // resolving to empty (the root itself) is rejected
        assert_eq!(resolve_rel("ppt", ".."), None);
    }
}
