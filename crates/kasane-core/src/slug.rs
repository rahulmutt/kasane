//! The two slug rules.
//!
//! `anchor_slug` is a deliberate mirror of GitHub's heading-id algorithm, so
//! an in-book cross-reference resolves when the tree is rendered on GitHub.
//! `path_slug` turns the same text into a portable file or directory name.
//!
//! They share `is_word` as their base character class and then diverge on
//! three axes, all deliberate. An anchor lands in the fragment of a link and a
//! path slug lands in the path portion, so nothing forces them to agree and
//! nothing breaks when they don't.
//!
//! 1. **The tail.** `path_slug` collapses separator runs, trims, and caps at
//!    [`MAX_PATH_SLUG_BYTES`]; the anchor does none of that, because GitHub
//!    does none of it.
//! 2. **Join_Control.** ZWNJ (U+200C) and ZWJ (U+200D) are inside Ruby's
//!    `\p{Word}`, so GitHub keeps them — and they sit *inside* ordinary
//!    Persian, Urdu and Devanagari words (`می‌رود`), not just in exotic text.
//!    `anchor_slug` keeps them. `path_slug` drops them: a filename does not
//!    want invisible characters, and the `slug` fuzz target's confinement
//!    argument rests on the path alphabet staying closed.
//! 3. **NFC.** `path_slug` normalizes; `anchor_slug` deliberately does not.
//!
//! That third one is the one a future reader will want to "fix". Do not.
//! kasane writes a file's heading line from the *unnormalized* title text
//! (`nav::walk`'s `inline_text` → `Frontmatter::title` →
//! `file_to_markdown`), and no renderer — GitHub included — normalizes before
//! computing a heading id. Folding NFC in here therefore produced a fragment
//! that matched no heading kasane itself had emitted: a link broken against
//! kasane's own output whenever the source text was NFD, which macOS-sourced
//! EPUBs and PDF text extraction routinely produce. `path_slug` keeps NFC
//! because it is choosing a *filename*, where the NFD and NFC spellings of one
//! title should land in one place, and where the `NN-` ordinal prefix already
//! makes any resulting collision harmless.
//!
//! Being a mirror, `anchor_slug` carries drift risk against github.com, the
//! same class the PDF adapter took on mirroring `lopdf`. The case table in
//! this file's tests is where that mirror is written down.
//!
//! The table pins this crate's *reading* of GitHub's algorithm, which is not
//! the same as pinning the algorithm: three corrections to that reading came
//! out of review, and the table agreed with the code every time, because it
//! encoded the same reading. Only an external oracle tests the derivation. One
//! was run on 2026-08-09 against a real github.com render — 13 of 13 ids
//! matched, codepoints included — and design spec §8.1 records the method and
//! the cases. Re-run it when this table is next edited.
//!
//! # Known divergences that survive on purpose
//!
//! The anchor is computed from the IR's inline text, not from what a Markdown
//! parser gets back out of the line the writer emits. Two cases follow from
//! that and are documented rather than fixed: closing either means changing
//! `inline_text`, whose other callers (`nav`, `refs`, `balance`) want exactly
//! its current behaviour, and that ripple is not a pre-merge change.
//!
//! - **Footnote references.** `inline_text` skips `Inline::FootnoteRef`, but
//!   the writer renders it as `[^1]`. `## Notes[^1]` therefore anchors `notes`
//!   here and `notes1` on GitHub.
//! - **A title ending in a run of `#`.** `## Intro ###` re-parses as an ATX
//!   heading with a *closing* sequence, so GitHub sees the text `Intro` and
//!   computes `intro`; kasane slugs the IR text `Intro ###` and computes
//!   `intro-`.
//!
//! One more divergence is by choice rather than by construction:
//!
//! - **The empty id.** A title with no character in the class at all gets
//!   [`EMPTY_FALLBACK`] rather than GitHub's empty id, because an empty
//!   fragment is a dead link.

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

/// Emitted when a title has no character in the slug's class at all (`## ***`,
/// `## —`, `## ½`).
///
/// GitHub gives such a heading an empty id. kasane cannot: an empty anchor is
/// a dead link. This is the one documented divergence from GFM in the shared
/// part of the two rules.
const EMPTY_FALLBACK: &str = "section";

/// Ruby's `\p{Word}` **minus Join_Control**: the class both rules share.
///
/// Ruby builds `\p{Word}` in `tool/enc-unicode.rb` as `Alphabetic + Mark +
/// Decimal_Number + Connector_Punctuation + Join_Control`, matching UTS#18
/// Annex C, and GitHub's TOC filter keeps exactly that set via
/// `/[^\p{Word}\- ]/u`. This is that set term for term, with nothing
/// approximated:
///
/// - **`char::is_alphabetic()` is Unicode's `Alphabetic` derived property**,
///   not the `L*` general-category group. That is the fact this predicate
///   turns on, and it is easy to miss: `Alphabetic` is `L* + Nl +
///   Other_Alphabetic`, so `char::is_alphabetic()` already answers for `Ⅷ`
///   (`Nl`) and for the circled Latin letters `Ⓐ`/`ⓐ` (`So` carrying
///   `Other_Alphabetic`) without a `Letter_Number` arm or a hand-kept table.
///   It is correspondingly false for the *parenthesized* Latin letters
///   (`⒜`, `So` but not `Other_Alphabetic`), which GitHub also drops.
/// - **The whole `Number` group would be too wide.** Ruby has
///   `Decimal_Number`, not `Nd + Nl + No`. `Nl` is in the set via
///   `Alphabetic`, but `Other_Number` (`½`, `①`) is *outside* it — which
///   matters, because circled numerals are common in Japanese and Chinese
///   headings.
///
/// Mark is the term std alone cannot supply, and is why this needs
/// `unicode-properties` at all: a Mark is not `Alphabetic`, the Devanagari
/// virama (U+094D) is a separate Mark that NFC does not compose away, and
/// dropping it would slug `हिन्दी` as `हिनदी`.
///
/// Join_Control is deliberately *not* here, because only `anchor_slug` wants
/// it; see the module doc and [`is_join_control`].
fn is_word(c: char) -> bool {
    c.is_alphabetic()
        || c.general_category_group() == GeneralCategoryGroup::Mark
        || matches!(
            c.general_category(),
            GeneralCategory::DecimalNumber | GeneralCategory::ConnectorPunctuation
        )
}

/// Unicode's `Join_Control`, which is exactly these two characters.
///
/// Ruby's `\p{Word}` includes them and GitHub therefore keeps them, so
/// `anchor_slug` must too: ZWNJ appears inside ordinary Persian and Urdu words
/// and ZWJ inside Devanagari conjuncts, so dropping them would give every such
/// heading an anchor github.com does not compute. `path_slug` drops them —
/// see the module doc for why the two rules part ways here.
fn is_join_control(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}')
}

/// The anchor's fold: the inline text, outer whitespace trimmed,
/// Unicode-lowercased, and **not normalized**.
///
/// The trim mirrors the renderer rather than the filter: a Markdown parser
/// strips a heading's surrounding whitespace before GitHub ever computes an
/// id, so `##   Intro  ` and `## Intro` anchor identically. Interior runs are
/// left alone, which is what produces the double hyphens.
///
/// The absence of NFC is the load-bearing part; the module doc says why.
fn anchor_fold(inlines: &[Inline]) -> String {
    inline_text(inlines)
        .trim()
        .chars()
        .flat_map(char::to_lowercase)
        .collect()
}

/// The path slug's fold: the same trim and lowercase, plus NFC.
///
/// Normalizing is a genuine benefit for a *filename* — one title spelled NFD
/// and NFC lands in one place, which is what macOS's NFD filesystem names make
/// realistic — and costs nothing, because the `NN-` ordinal prefix already
/// makes sibling collisions impossible.
fn path_fold(inlines: &[Inline]) -> String {
    inline_text(inlines)
        .trim()
        .nfc()
        .flat_map(char::to_lowercase)
        .collect()
}

/// GitHub's algorithm, in its order: downcase, remove everything outside
/// `\p{Word}`/`-`/space, then map each remaining space to `-`.
///
/// There is no normalization step, deliberately, and that is not an omission —
/// GitHub performs none either, and adding one broke links against kasane's
/// own rendered headings. The module doc has the argument.
///
/// No run-collapsing and no interior trimming, because GitHub does neither.
/// Exact parity therefore means deliberately emitting anchors that look wrong:
/// `Background & Notes` anchors as `background--notes`, since the `&` is
/// removed and each of the two surviving spaces becomes a hyphen.
pub(crate) fn anchor_slug(inlines: &[Inline]) -> String {
    let out: String = anchor_fold(inlines)
        .chars()
        .filter(|c| is_word(*c) || is_join_control(*c) || *c == '-' || *c == ' ')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect();
    if out.is_empty() {
        EMPTY_FALLBACK.to_string()
    } else {
        out
    }
}

/// The shared character class, plus NFC, then it diverges where a filename
/// should: separator runs collapse to a single `-`, the tail is trimmed, and
/// the result is capped at `MAX_PATH_SLUG_BYTES`.
///
/// Everything outside the class is REMOVED, exactly as the anchor rule removes
/// it -- only space and `-` act as separators. That is what makes `Don't
/// Panic` a `dont-panic` file rather than the old `don-t-panic`. Join_Control
/// is removed here and kept by the anchor rule, which is the one place the two
/// character classes differ; the module doc says why, and `fuzz_entry::slug`
/// is what fails if the path alphabet is widened by hand.
///
/// Truncation can make two sibling slugs identical. That is harmless: every
/// non-root component carries an `NN-` ordinal prefix, which is already what
/// makes sibling collisions impossible -- including the case-insensitive ones
/// macOS and Windows would produce, and the NFC-vs-NFD ones macOS would.
pub(crate) fn path_slug(inlines: &[Inline]) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in path_fold(inlines).chars() {
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

/// Test seam for `path_slug`, same rationale as `est_tokens` and
/// `anchors_for_headings`: the fuzz seam and the property tier need the
/// engine's own rule rather than a copy of it that can drift.
#[doc(hidden)]
pub fn path_slug_of(inlines: &[Inline]) -> String {
    path_slug(inlines)
}

/// Anchors for one file's headings, in the order the file renders them.
///
/// Test seam for the property tier, and the reason `slug_of` could not stay:
/// with duplicate suffixing an anchor depends on what preceded it on the page,
/// so a per-heading function cannot express the rule. The tier asserts against
/// the engine's own counter rather than a copy of it that can drift.
#[doc(hidden)]
pub fn anchors_for_headings(headings: &[String]) -> Vec<String> {
    let mut counter = AnchorCounter::new();
    headings
        .iter()
        .map(|t| counter.next(&[Inline::Text(t.clone())]))
        .collect()
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
    /// map spaces to hyphens. No normalization, no collapsing and no trimming
    /// of interior runs, which is why some rows look wrong and are not.
    ///
    /// `\p{Word}` here is Ruby's, i.e. `Alphabetic + Mark + Decimal_Number +
    /// Connector_Punctuation + Join_Control` — see `is_word` for the two
    /// places that is narrower than "Letter, Mark, Number".
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
        // No character in the class at all: GitHub emits an empty id, which
        // would be a dead link here. The documented divergence.
        assert_eq!(anchor_slug(&t("***")), "section");
        // `Other_Number` is OUTSIDE Ruby's `\p{Word}`, which has
        // `Decimal_Number` rather than the whole `Number` group. The vulgar
        // fraction is removed like any other symbol; the space before it
        // survives as a trailing hyphen, since the anchor rule never trims.
        assert_eq!(anchor_slug(&t("Fig ½")), "fig-");
        // A circled numeral is `Other_Number` too, and it is common in
        // Japanese and Chinese headings. GitHub drops it.
        assert_eq!(anchor_slug(&t("①はじめに")), "はじめに");
        // `Letter_Number` IS inside the set -- it arrives via `Alphabetic`,
        // not via `Number` -- and downcases like any other letter.
        assert_eq!(anchor_slug(&t("Part Ⅷ")), "part-ⅷ");
        // `Other_Alphabetic` is in `Alphabetic` too, and reaches this rule
        // only because `is_word` asks `char::is_alphabetic()` rather than
        // testing the `L*` category group. `Ⓐ` U+24B6 is `So`; it is kept, and
        // it downcases to `ⓐ` U+24D0.
        assert_eq!(anchor_slug(&t("Ⓐ Notes")), "ⓐ-notes");
        // The boundary is `Alphabetic`, not "looks like a letter": the
        // PARENTHESIZED small letter is `So` WITHOUT `Other_Alphabetic`, so it
        // is dropped, and the space after it still becomes a hyphen. GitHub
        // drops it for the same reason.
        assert_eq!(anchor_slug(&t("⒜ Notes")), "-notes");
        // Join_Control is inside Ruby's `\p{Word}` and GitHub keeps it. ZWNJ
        // sits INSIDE this ordinary Persian word, so dropping it would
        // mis-anchor the heading against every GitHub render.
        assert_eq!(anchor_slug(&t("می\u{200C}رود")), "می\u{200C}رود");
        // A title that is nothing but a Join_Control character anchors to that
        // character, not to `section`: the result is non-empty, so the
        // fallback does not fire, and it is exactly the id GitHub computes, so
        // the link resolves in kasane's tree and on GitHub alike. An invisible
        // anchor is odd to look at but is not a broken one, and guarding it
        // would manufacture a divergence where there is currently none.
        assert_eq!(anchor_slug(&t("\u{200C}")), "\u{200C}");
    }

    /// The anchor is deliberately NOT normalized; the path slug is.
    ///
    /// This test used to assert the opposite, and asserting it was what hid a
    /// real bug. kasane renders a file's heading line from the *unnormalized*
    /// title (`nav::walk` -> `Frontmatter::title` -> `file_to_markdown`), and
    /// no renderer normalizes before computing an id. NFC in the anchor path
    /// therefore emitted a fragment matching no heading kasane itself had
    /// written, for any NFD input -- which macOS-sourced EPUBs and PDF text
    /// extraction produce routinely. Do not "fix" these back into agreement.
    ///
    /// `path_slug` still normalizes, because it is choosing a filename rather
    /// than a fragment: NFD and NFC of one title should land in one place, and
    /// the `NN-` ordinal prefix already makes collisions harmless.
    #[test]
    fn nfd_and_nfc_diverge_for_anchors_and_agree_for_paths() {
        let nfc = "Café"; // é = U+00E9
        let nfd = "Cafe\u{0301}"; // e + COMBINING ACUTE
        assert_eq!(anchor_slug(&t(nfc)), "café");
        assert_eq!(anchor_slug(&t(nfd)), "cafe\u{0301}");
        assert_ne!(
            anchor_slug(&t(nfc)),
            anchor_slug(&t(nfd)),
            "each anchor must match the heading line kasane renders for it"
        );
        assert_eq!(path_slug(&t(nfc)), "café");
        assert_eq!(path_slug(&t(nfd)), "café");
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
        // `Other_Number` is outside the shared class, so it goes here too --
        // and the hyphen the anchor rule leaves dangling is trimmed.
        assert_eq!(path_slug(&t("Fig ½")), "fig");
        assert_eq!(path_slug(&t("①はじめに")), "はじめに");
        // `Letter_Number` is inside it, here as there.
        assert_eq!(path_slug(&t("Part Ⅷ")), "part-ⅷ");
        // `Other_Alphabetic` now flows into FILENAMES as well, which is what
        // we want: `Ⓐ` is an ordinary printing character with a case mapping,
        // it breaks no path component, and dropping it here while the anchor
        // keeps it would make the file's name and its heading disagree about
        // the title for no benefit. Join_Control is the only member of the
        // anchor class a filename must refuse. NFC leaves `Ⓐ` alone -- its
        // decomposition is compatibility (`<circle>`), which is NFKC's job.
        assert_eq!(path_slug(&t("Ⓐ Notes")), "ⓐ-notes");
        assert_eq!(path_slug(&t("⒜ Notes")), "notes");
        // Join_Control is where the two classes part: the anchor keeps the
        // ZWNJ, a filename must not carry an invisible character, and the
        // `slug` fuzz target's closed-alphabet argument depends on it.
        assert_eq!(path_slug(&t("می\u{200C}رود")), "میرود");
        // So a ZWNJ-only title, which the anchor rule slugs to the ZWNJ
        // itself, falls back here.
        assert_eq!(path_slug(&t("\u{200C}")), "section");
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
