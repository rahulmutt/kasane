pub const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_RATIO: u64 = 200;

/// Guard against decompression bombs given compressed and (running) decompressed sizes.
pub fn check_expansion(compressed: u64, decompressed: u64) -> bool {
    decompressed <= MAX_TOTAL_BYTES
        && (compressed == 0 || decompressed / compressed.max(1) <= MAX_RATIO)
}

/// The crate's single path-confinement primitive: turn an untrusted in-archive
/// reference into a zip entry key that cannot name anything outside the archive
/// root.
///
/// `target` (an OPF manifest href, an EPUB container rootfile path, an `img
/// src`, a PPTX relationship target -- anything an adapter reads out of a
/// document) is resolved against `base_dir`, the directory of the file it was
/// read from. `.` and empty segments are dropped, `..` pops, and a leading `/`
/// makes the target package-absolute (resolved from the root, ignoring
/// `base_dir`). Returns `None` -- reject, do not fall back -- if the target
/// escapes the root or resolves to nothing.
///
/// Callers may use a `Some` result directly as a zip key: it is already
/// normalized and confined, and re-guarding it is what used to drop legal
/// chapters whose names merely contained `..`. Percent-encoded references must
/// be run through `percent_decode` BEFORE they get here, so encoded separators
/// are normalized by this loop rather than surviving as opaque segments.
pub fn resolve_rel(base_dir: &str, target: &str) -> Option<String> {
    // A package-absolute target resolves from the archive root, so base_dir is
    // not consulted at all.
    let base = if target.starts_with('/') {
        ""
    } else {
        base_dir
    };
    let mut parts: Vec<&str> = Vec::new();
    // Both sources run through the SAME loop. Splitting base_dir raw was the
    // bug: its segments never saw the `..` arm, so a `..` passed straight
    // through into the result and defeated the confinement contract above.
    for seg in base.split('/').chain(target.split('/')) {
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

/// Percent-decode the `%XX` escapes in an href's path.
///
/// EPUB hrefs are IRI references; zip entry names are literal bytes. A chapter
/// stored as `OEBPS/ch 1.xhtml` is written `href="ch%201.xhtml"`, and the
/// undecoded key misses `by_name` -- the chapter then vanishes from the
/// converted book at exit 0.
///
/// Deliberately not a URL parser: it decodes `%XX` and nothing else. An invalid
/// or truncated escape is left verbatim, so a filename holding a literal `%`
/// (`100%.xhtml`) still resolves. Escapes decode to bytes first, so a multi-byte
/// UTF-8 sequence like `%C3%A9` round-trips as one character; if the decoded
/// bytes are not valid UTF-8 the input is returned verbatim rather than being
/// mangled into replacement characters.
///
/// Call this BEFORE `resolve_rel`, never after: a `%2F` decodes to a separator
/// that must still pass through the segment loop to be normalized and confined.
/// Decoding after resolution would reintroduce unnormalized separators into a
/// key already declared confined.
pub(crate) fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let src = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        if src[i] == b'%' {
            if let Some(byte) = src.get(i + 1..i + 3).and_then(hex_pair) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(src[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// The byte two hex digits denote, or `None` if either is not a hex digit.
fn hex_pair(digits: &[u8]) -> Option<u8> {
    let hi = char::from(digits[0]).to_digit(16)?;
    let lo = char::from(digits[1]).to_digit(16)?;
    Some((hi * 16 + lo) as u8)
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

    #[test]
    fn percent_decode_decodes_escapes_and_leaves_invalid_ones_verbatim() {
        // The ordinary case: a space in a filename.
        assert_eq!(percent_decode("ch%201.xhtml"), "ch 1.xhtml");
        // A multi-byte UTF-8 sequence spans two escapes.
        assert_eq!(percent_decode("caf%C3%A9.xhtml"), "café.xhtml");
        // Lowercase hex digits are equally valid.
        assert_eq!(percent_decode("caf%c3%a9.xhtml"), "café.xhtml");
        // Nothing to decode.
        assert_eq!(percent_decode("ch1.xhtml"), "ch1.xhtml");
        // A literal `%` in a filename must survive: not an escape at all,
        // truncated, or not hex.
        assert_eq!(percent_decode("100%.xhtml"), "100%.xhtml");
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("a%zz.xhtml"), "a%zz.xhtml");
        assert_eq!(percent_decode("50%%20off"), "50% off");
        // Decoded bytes that are not valid UTF-8 leave the input verbatim
        // rather than yielding U+FFFD replacement characters.
        assert_eq!(percent_decode("a%FF.xhtml"), "a%FF.xhtml");
    }

    #[test]
    fn percent_decoded_separators_are_normalized_and_confined_by_resolve_rel() {
        // Decoding happens BEFORE resolve_rel, so an encoded separator becomes
        // a real segment boundary that the normalizer sees.
        assert_eq!(
            resolve_rel("OEBPS", &percent_decode("sub%2Fch.xhtml")).as_deref(),
            Some("OEBPS/sub/ch.xhtml")
        );
        assert_eq!(
            resolve_rel("OEBPS", &percent_decode("%2E%2E%2Fshared%2Fch.xhtml")).as_deref(),
            Some("shared/ch.xhtml")
        );
        // The security case: an escaping shape is rejected in encoded form
        // exactly as it is in literal form.
        assert_eq!(
            resolve_rel("OEBPS", &percent_decode("%2E%2E%2F%2E%2E%2Fetc%2Fpasswd")),
            None
        );
        assert_eq!(
            resolve_rel("OEBPS", &percent_decode("..%2F..%2Fetc/passwd")),
            None
        );
        assert_eq!(
            resolve_rel("OEBPS", &percent_decode("%2E%2E%2F%2E%2E%2Fetc%2Fpasswd")),
            resolve_rel("OEBPS", "../../etc/passwd")
        );
    }

    #[test]
    fn resolve_rel_rejects_escaping_base_dir() {
        // The #22 reproducer's shape: `..` in base_dir must pop like it does in
        // target, not pass through into the result.
        assert_eq!(resolve_rel("../a", "x"), None);
        assert_eq!(resolve_rel("..", "x"), None);
        assert_eq!(resolve_rel("a/../../b", "x"), None);
    }

    #[test]
    fn resolve_rel_normalizes_interior_base_dir() {
        // An interior `..` in base_dir normalizes rather than being emitted.
        assert_eq!(resolve_rel("a/../b", "x").as_deref(), Some("b/x"));
        assert_eq!(resolve_rel("a/./b", "x").as_deref(), Some("a/b/x"));
        // An empty base_dir still resolves against the archive root.
        assert_eq!(resolve_rel("", "a/b.xml").as_deref(), Some("a/b.xml"));
        // The gap recorded in the outline-and-guard-hardening spec's §7:
        // the deleted safe_entry_name returned Some("") for these, and the
        // `guards` fuzz target asserted non-emptiness only for resolve_rel.
        // resolve_rel is now the only confinement primitive, so this is
        // where that postcondition gets pinned.
        assert_eq!(resolve_rel("", ""), None);
        assert_eq!(resolve_rel("", "."), None);
    }
}
