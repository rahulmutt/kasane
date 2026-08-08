//! The two slug rules.
//!
//! `anchor_slug` is a deliberate mirror of GitHub's heading-id algorithm, so
//! an in-book cross-reference resolves when the tree is rendered on GitHub.
//! `path_slug` turns the same text into a portable file or directory name.
//!
//! They share a character class and a normalization step and diverge only in
//! the tail. That is deliberate, not an oversight: an anchor lands in the
//! fragment of a link and a path slug lands in the path portion, so nothing
//! forces them to agree and nothing breaks when they don't.
//!
//! Being a mirror, `anchor_slug` carries drift risk against github.com, the
//! same class the PDF adapter took on mirroring `lopdf`. The case table in
//! this file's tests is where that mirror is written down.

use kasane_ir::Inline;
use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategory, GeneralCategoryGroup, UnicodeGeneralCategory};

/// Byte budget for one path slug.
///
/// Roughly 64 Latin characters or 21 CJK ones -- comfortably a chapter title.
/// With the `NN-` ordinal prefix and the `.md` suffix that `paths.rs` adds, a
/// component stays far inside the 255-byte per-component limit. Anchors are
/// deliberately NOT capped: they are not filenames, and capping them would
/// break GFM parity for no benefit.
pub(crate) const MAX_PATH_SLUG_BYTES: usize = 64;

/// Emitted when a title has no `\p{Word}` character at all (`## ***`, `## —`).
///
/// GitHub gives such a heading an empty id. kasane cannot: an empty anchor is
/// a dead link. This is the one documented divergence from GFM.
const EMPTY_FALLBACK: &str = "section";

/// Ruby's `\p{Word}`, which is exactly what GitHub's TOC filter keeps: Letter,
/// Mark, Number, and Connector_Punctuation (so `_` survives).
///
/// Mark is why this needs a table rather than `char::is_alphanumeric()`.
/// After NFC the Devanagari virama (U+094D) is still a separate Mark, and
/// dropping it would slug `हिन्दी` as `हिनदी`.
fn is_word(c: char) -> bool {
    matches!(
        c.general_category_group(),
        GeneralCategoryGroup::Letter | GeneralCategoryGroup::Mark | GeneralCategoryGroup::Number
    ) || c.general_category() == GeneralCategory::ConnectorPunctuation
}

/// The shared prefix of both rules: the inline text, outer whitespace trimmed,
/// NFC-normalized, Unicode-lowercased.
///
/// The trim mirrors the renderer rather than the filter: a Markdown parser
/// strips a heading's surrounding whitespace before GitHub ever computes an
/// id, so `##   Intro  ` and `## Intro` anchor identically. Interior runs are
/// left alone, which is what produces the double hyphens.
fn normalized(inlines: &[Inline]) -> String {
    inline_text(inlines)
        .trim()
        .nfc()
        .flat_map(char::to_lowercase)
        .collect()
}

/// GitHub's algorithm, in its order: normalize, downcase, remove everything
/// outside `\p{Word}`/`-`/space, then map each remaining space to `-`.
///
/// No run-collapsing and no interior trimming, because GitHub does neither.
/// Exact parity therefore means deliberately emitting anchors that look wrong:
/// `Background & Notes` anchors as `background--notes`, since the `&` is
/// removed and each of the two surviving spaces becomes a hyphen.
pub(crate) fn anchor_slug(inlines: &[Inline]) -> String {
    let out: String = normalized(inlines)
        .chars()
        .filter(|c| is_word(*c) || *c == '-' || *c == ' ')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect();
    if out.is_empty() {
        EMPTY_FALLBACK.to_string()
    } else {
        out
    }
}

/// The same character class and normalization, then it diverges where a
/// filename should: separator runs collapse to a single `-`, the tail is
/// trimmed, and the result is capped at `MAX_PATH_SLUG_BYTES`.
///
/// Everything outside `\p{Word}` is REMOVED, exactly as the anchor rule
/// removes it -- only space and `-` act as separators. That is what makes
/// `Don't Panic` a `dont-panic` file rather than the old `don-t-panic`.
///
/// Truncation can make two sibling slugs identical. That is harmless: every
/// non-root component carries an `NN-` ordinal prefix, which is already what
/// makes sibling collisions impossible -- including the case-insensitive ones
/// macOS and Windows would produce, and the NFC-vs-NFD ones macOS would.
pub(crate) fn path_slug(inlines: &[Inline]) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in normalized(inlines).chars() {
        if is_word(c) {
            out.push(c);
            prev_dash = false;
        } else if (c == ' ' || c == '-') && !prev_dash && !out.is_empty() {
            // `!out.is_empty()` is what makes a leading separator impossible,
            // so there is nothing to trim off the front later.
            out.push('-');
            prev_dash = true;
        }
    }
    truncate_to(&mut out, MAX_PATH_SLUG_BYTES);
    trim_tail(&mut out);
    if out.is_empty() {
        EMPTY_FALLBACK.to_string()
    } else {
        out
    }
}

/// Truncate to at most `max` bytes without splitting a `char`.
fn truncate_to(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

/// Drop a trailing `-`, and drop a trailing Mark only when it has no base to
/// attach to.
///
/// A Mark with a real base is a legitimate word ending -- Devanagari and Thai
/// words routinely end in a vowel sign -- so popping every trailing Mark
/// unconditionally (the earlier version of this function) silently deleted a
/// letter of the word. `truncate_to` cannot orphan a Mark on its own: it cuts
/// a suffix on a char boundary, so a surviving Mark always keeps whatever
/// preceded it. The only genuinely dangling cases are a Mark with nothing
/// before it at all (mark-only input) and a Mark directly after a separator
/// (the separator is not a base).
fn trim_tail(s: &mut String) {
    loop {
        match s.chars().next_back() {
            Some('-') => {
                s.pop();
            }
            Some(c) if c.general_category_group() == GeneralCategoryGroup::Mark => {
                let base = s
                    .chars()
                    .rev()
                    .find(|c| c.general_category_group() != GeneralCategoryGroup::Mark);
                match base {
                    None | Some('-') => {
                        s.pop();
                    }
                    _ => break,
                }
            }
            _ => break,
        }
    }
}

/// Test seam for `path_slug`, same rationale as `est_tokens` and `slug_of`:
/// the fuzz seam and the property tier need the engine's own rule rather than
/// a copy of it that can drift.
#[doc(hidden)]
pub fn path_slug_of(inlines: &[Inline]) -> String {
    path_slug(inlines)
}

/// Anchor-rule test seam. Kept under its historical name so
/// `kasane-writer`'s property tier keeps compiling; Task 3 replaces it with
/// the ordered form that duplicate suffixing requires.
#[doc(hidden)]
pub fn slug_of(inlines: &[Inline]) -> String {
    anchor_slug(inlines)
}

/// The visible text of an inline run, bounded by `MAX_INLINE_DEPTH`.
///
/// Moved here from `paths.rs`: it exists to feed the slug rules, and leaving
/// it there would make `paths` and `slug` mutually dependent.
pub(crate) fn inline_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    inline_text_at(inlines, 0, &mut s);
    s
}

fn inline_text_at(inlines: &[Inline], depth: usize, s: &mut String) {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return;
    }
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => s.push_str(t),
            Inline::Emph(x) | Inline::Strong(x) => inline_text_at(x, depth + 1, s),
            Inline::Link { inlines, .. } => inline_text_at(inlines, depth + 1, s),
            Inline::FootnoteRef(_) => {}
        }
    }
}

/// Assigns anchors to one file's headings in render order, uniquifying
/// duplicates the way GitHub does.
///
/// One instance per file. The first occurrence of a base keeps it, the next
/// gets `-1`, then `-2`. GitHub does not re-check whether the suffixed form
/// itself collides with an existing id, and neither does this -- mirroring the
/// quirk is the point.
pub(crate) struct AnchorCounter {
    seen: std::collections::HashMap<String, usize>,
}

impl AnchorCounter {
    pub(crate) fn new() -> Self {
        Self {
            seen: std::collections::HashMap::new(),
        }
    }

    /// The anchor for the next heading in render order. Every heading the file
    /// renders must pass through here, including ones that get no anchor of
    /// their own -- they still consume a slot on the rendered page.
    pub(crate) fn next(&mut self, inlines: &[Inline]) -> String {
        let base = anchor_slug(inlines);
        let n = self.seen.entry(base.clone()).or_insert(0);
        let out = if *n == 0 { base } else { format!("{base}-{n}") };
        *n += 1;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kasane_ir::Inline;

    fn t(s: &str) -> Vec<Inline> {
        vec![Inline::Text(s.to_string())]
    }

    /// Each row names the rule it pins. These are derived from GitHub's TOC
    /// filter: downcase, remove everything outside `\p{Word}`/`-`/space, then
    /// map spaces to hyphens. No collapsing and no trimming of interior runs,
    /// which is why some rows look wrong and are not.
    #[test]
    fn anchor_matches_github() {
        // punctuation is REMOVED, not replaced -- the old rule made this
        // `don-t-panic`, which resolved nowhere on GitHub.
        assert_eq!(anchor_slug(&t("Don't Panic")), "dont-panic");
        // `&` is removed and BOTH surviving spaces become hyphens. The double
        // hyphen is correct GFM output, not a bug.
        assert_eq!(anchor_slug(&t("Background & Notes")), "background--notes");
        // `_` is Connector_Punctuation, which is inside `\p{Word}`.
        assert_eq!(anchor_slug(&t("foo_bar")), "foo_bar");
        // CJK passes through untouched; there is nothing to downcase.
        assert_eq!(anchor_slug(&t("第二章")), "第二章");
        // Devanagari matras and the virama are Marks, also inside `\p{Word}`.
        assert_eq!(anchor_slug(&t("हिन्दी")), "हिन्दी");
        // Symbols are outside `\p{Word}`; the two spaces around the emoji
        // survive as two hyphens.
        assert_eq!(anchor_slug(&t("Hello 🎉 World")), "hello--world");
        // Unicode-aware downcasing.
        assert_eq!(anchor_slug(&t("CHAPTER One")), "chapter-one");
        // Outer whitespace is trimmed because a Markdown renderer strips it
        // from the heading's text before computing the id. Interior runs are
        // not trimmed, per the row above.
        assert_eq!(anchor_slug(&t("  Intro  ")), "intro");
        // No Word character at all: GitHub emits an empty id, which would be a
        // dead link here. The documented divergence.
        assert_eq!(anchor_slug(&t("***")), "section");
    }

    /// NFC runs before anything else, so a decomposed title and its composed
    /// twin cannot slug differently. macOS-sourced text is the realistic
    /// source of NFD input.
    #[test]
    fn nfd_and_nfc_agree() {
        let nfc = "Café"; // é = U+00E9
        let nfd = "Cafe\u{0301}"; // e + COMBINING ACUTE
        assert_eq!(anchor_slug(&t(nfc)), anchor_slug(&t(nfd)));
        assert_eq!(anchor_slug(&t(nfc)), "café");
        assert_eq!(path_slug(&t(nfc)), path_slug(&t(nfd)));
    }

    /// Same character class as the anchor, then it diverges where a filename
    /// should: separator runs collapse, the result is trimmed and capped.
    #[test]
    fn path_slug_is_a_filename_not_an_anchor() {
        assert_eq!(path_slug(&t("Don't Panic")), "dont-panic");
        // where the anchor keeps `background--notes`
        assert_eq!(path_slug(&t("Background & Notes")), "background-notes");
        assert_eq!(path_slug(&t("foo_bar")), "foo_bar");
        assert_eq!(path_slug(&t("第二章")), "第二章");
        // The anchor table has a Devanagari row; the path-slug table didn't,
        // and that asymmetry is exactly what let `trim_tail` silently eat the
        // final vowel sign. `हिन्दी` ends in a Mark with a real base, which is
        // a legitimate word ending, not a dangling tail.
        assert_eq!(path_slug(&t("हिन्दी")), "हिन्दी");
        assert_eq!(path_slug(&t("Hello 🎉 World")), "hello-world");
        assert_eq!(path_slug(&t("***")), "section");
        // A leading separator is never emitted, so nothing needs trimming off
        // the front.
        assert_eq!(path_slug(&t("  Intro  ")), "intro");
    }

    /// Traversal and separator injection are impossible by construction: `/`,
    /// `\`, `.`, NUL, the fullwidth solidus and the RTL override are all
    /// outside `\p{Word}` and are removed rather than mapped to anything.
    #[test]
    fn path_slug_cannot_emit_a_separator() {
        assert_eq!(path_slug(&t("../../etc/passwd")), "etcpasswd");
        assert_eq!(path_slug(&t("a\\b")), "ab");
        assert_eq!(path_slug(&t("..")), "section");
        assert_eq!(path_slug(&t("a\u{FF0F}b")), "ab");
        assert_eq!(path_slug(&t("a\u{202E}b")), "ab");
        assert_eq!(path_slug(&t("a\u{0}b")), "ab");
    }

    /// 64 bytes, cut on a char boundary. A CJK title hits the cap three times
    /// faster than a Latin one: 64/3 = 21 characters, 63 bytes.
    #[test]
    fn path_slug_caps_at_the_byte_budget() {
        let long = "第".repeat(40);
        let out = path_slug(&t(&long));
        assert_eq!(out, "第".repeat(21));
        assert!(out.len() <= MAX_PATH_SLUG_BYTES);
        // Anchors are not filenames and are deliberately uncapped.
        assert_eq!(anchor_slug(&t(&long)).len(), 120);
        // The cap can land exactly on a separator: 63 `a`s + the collapsed
        // space is 64 bytes ending in `-`, and the `tail` after it is cut
        // off entirely. `trim_tail` must run AFTER `truncate_to` to remove
        // that dangling hyphen; a refactor that swapped the order would
        // leave `a`*63 + `-` and fail this.
        assert_eq!(
            path_slug(&t(&format!("{} tail", "a".repeat(63)))),
            "a".repeat(63)
        );
    }

    /// A trailing Mark survives when it has a base (61 `a`s plus a 3-byte
    /// virama is exactly 64 bytes, so nothing is cut by the cap either), and
    /// is dropped only when it is genuinely dangling: no base at all, or a
    /// base that is itself a separator.
    #[test]
    fn path_slug_trims_a_dangling_tail() {
        let s = format!("{}{}", "a".repeat(61), "\u{094D}");
        assert_eq!(s.len(), 64);
        assert_eq!(path_slug(&t(&s)), s);
        assert_eq!(path_slug(&t("Intro -")), "intro");
        // A mark with no base at all is the true dangling case.
        assert_eq!(path_slug(&t("\u{0301}")), "section");
        // A mark directly after a separator has no base either -- the
        // separator collapses away, leaving nothing for the mark to attach to.
        assert_eq!(path_slug(&t("a \u{0301}")), "a");
    }
}
