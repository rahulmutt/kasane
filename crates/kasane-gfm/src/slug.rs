//! The two slug rules.
//!
//! `anchor_slug` is a deliberate mirror of GitHub's heading-id algorithm, so
//! an in-book cross-reference resolves when the tree is rendered on GitHub.
//! `path_slug` turns the same text into a portable file or directory name.
//!
//! They share `is_word` as their base character class and then diverge on
//! four axes, all deliberate. An anchor lands in the fragment of a link and a
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
//! 4. **Newline folding.** `anchor_slug` first folds every newline spelling in
//!    the inline text to a single space, collapsing runs; `path_slug` does
//!    not. See [`crate::text::fold_newlines`] for why.
//!
//! That third one is the one a future reader will want to "fix". Do not.
//! kasane writes a file's heading line from the *unnormalized* title text
//! (`nav::walk`'s `title_text` → `Frontmatter::title` →
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
//! # The two known divergences that survive on purpose
//!
//! The anchor is computed from `rendered_text`, the projection of what the
//! writer actually emits for a heading's line — not from the IR's inline text
//! directly. That closed the two divergences this section used to document:
//! a footnote reference now contributes its digits (`rendered_text` renders
//! `Inline::FootnoteRef(n)` as `[^n]`, the way GitHub sees it), and a title
//! ending in a run of `#` now anchors correctly once `kasane-writer`'s
//! `escape::atx_closing` escapes that run before it ever reaches this rule,
//! so the printed line still says `Intro ###` in full rather than the `Intro`
//! a real parser would strip a bare closing sequence down to.
//!
//! Two divergences are left:
//!
//! - **The empty id.** A title with no character in the class at all gets
//!   [`EMPTY_FALLBACK`] rather than GitHub's empty id, because an empty
//!   fragment is a dead link. This one is a choice rather than a construction
//!   defect: kasane could match GitHub exactly by emitting no id, and
//!   deliberately doesn't.
//! - **An empty inline code span inside a heading.** Given
//!   `[Inline::Text("a"), Inline::Code(""), Inline::Text("b")]` in a
//!   `Block::Heading`, `kasane-writer::escape::code_span` renders the empty
//!   span as `` ` ` `` — a single padding space, CommonMark's only way to
//!   express an empty code span — so the printed line is `` a` `b `` and a
//!   real parser reads its text as `a b`, with GitHub computing the id
//!   `a-b`. `rendered_text` does not model that padding space (it takes an
//!   `Inline::Code`'s content verbatim), so this rule computes `ab` and
//!   `anchor_slug` embeds `ab` — a dead cross-reference against GitHub's
//!   own render. This is a real construction defect, not a choice, and it
//!   pre-dates this crate: the old `inline_text` produced the same `ab`.
//!   It is documented rather than fixed here because the only fix is a
//!   change to `code_span`'s output, which has its own callers and its own
//!   fuzz postconditions and belongs to its own item, not this one. See
//!   `escape::code_span`'s Rule 1 comment for the writer side.

use crate::text::{fold_newlines, rendered_text, title_text};
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

/// The anchor's fold: the printed line, newlines folded to spaces, outer
/// whitespace trimmed, Unicode-lowercased, and **not normalized**.
fn anchor_fold(line: &str) -> String {
    fold_newlines(line)
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
    title_text(inlines)
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
pub fn anchor_slug(line: &str) -> String {
    let out: String = anchor_fold(line)
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
pub fn path_slug(inlines: &[Inline]) -> String {
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

/// Test seam for the anchor rule over real inline structure, same rationale as
/// `path_slug_of` and `anchors_for_headings`. Composes the projection with the
/// rule, which is what a body heading does. No counter is threaded because a
/// single heading has no duplicate to suffix.
#[doc(hidden)]
pub fn anchor_slug_of(inlines: &[Inline]) -> String {
    AnchorCounter::new().next(&rendered_text(inlines))
}

/// Anchors for one file's headings, in the order the file renders them, from
/// the text those lines print.
#[doc(hidden)]
pub fn anchors_for_headings(headings: &[String]) -> Vec<String> {
    let mut counter = AnchorCounter::new();
    headings.iter().map(|t| counter.next(t)).collect()
}

/// Assigns anchors to one file's headings in render order, uniquifying
/// duplicates the way GitHub does.
///
/// One instance per file. The first occurrence of a base keeps it, the next
/// gets `-1`, then `-2`. GitHub does not re-check whether the suffixed form
/// itself collides with an existing id, and neither does this -- mirroring the
/// quirk is the point.
pub struct AnchorCounter {
    seen: std::collections::HashMap<String, usize>,
}

impl AnchorCounter {
    pub fn new() -> Self {
        Self {
            seen: std::collections::HashMap::new(),
        }
    }

    /// The anchor for the next heading in render order, computed from the text
    /// that heading's line prints. Every heading the file renders must pass
    /// through here, including ones that get no anchor of their own — they
    /// still consume a slot on the rendered page.
    ///
    /// Taking `&str` rather than `&[Inline]` is the enforcement, not a
    /// convenience: a caller cannot hand it an inline run and receive an
    /// anchor for a line it is not going to print. The two heading paths print
    /// different things — a body heading prints the writer's rendering of its
    /// inlines, a file's title heading prints `Frontmatter::title` verbatim —
    /// and each projects accordingly.
    pub fn next(&mut self, line: &str) -> String {
        let base = anchor_slug(line);
        let n = self.seen.entry(base.clone()).or_insert(0);
        let out = if *n == 0 { base } else { format!("{base}-{n}") };
        *n += 1;
        out
    }
}

impl Default for AnchorCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kasane_ir::{Inline, NoteId};

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
        assert_eq!(anchor_slug("Don't Panic"), "dont-panic");
        // `&` is removed and BOTH surviving spaces become hyphens. The double
        // hyphen is correct GFM output, not a bug.
        assert_eq!(anchor_slug("Background & Notes"), "background--notes");
        // `_` is Connector_Punctuation, which is inside `\p{Word}`.
        assert_eq!(anchor_slug("foo_bar"), "foo_bar");
        // CJK passes through untouched; there is nothing to downcase.
        assert_eq!(anchor_slug("第二章"), "第二章");
        // Devanagari matras and the virama are Marks, also inside `\p{Word}`.
        assert_eq!(anchor_slug("हिन्दी"), "हिन्दी");
        // Symbols are outside `\p{Word}`; the two spaces around the emoji
        // survive as two hyphens.
        assert_eq!(anchor_slug("Hello 🎉 World"), "hello--world");
        // Unicode-aware downcasing.
        assert_eq!(anchor_slug("CHAPTER One"), "chapter-one");
        // Outer whitespace is trimmed because a Markdown renderer strips it
        // from the heading's text before computing the id. Interior runs are
        // not trimmed, per the row above.
        assert_eq!(anchor_slug("  Intro  "), "intro");
        // An embedded newline is not itself in `\p{Word}`/`-`/space, but by
        // the time GitHub computes an id the renderer has already folded it
        // to a literal space (`kasane_gfm::fold_newlines`), so the anchor must match
        // that rendered line -- one hyphen, not a silently dropped character.
        // NOT covered by the 2026-08-09 github.com parity check (design spec
        // §8.2): that run predates this fold.
        assert_eq!(anchor_slug("line\nbreak"), "line-break");
        // `\r\n` is two bytes but ONE line ending. `kasane_gfm::fold_newlines` folds
        // the pair to a single space before it ever considers a lone `\r` or
        // `\n`, so this must anchor identically to the lone-`\n` row above --
        // one hyphen, not two. Also not covered by the 2026-08-09 check.
        assert_eq!(anchor_slug("line\r\nbreak"), "line-break");
        // A blank line inside a heading's text is still ONE separator on the
        // rendered line: `escape::text` collapses the run so one paragraph
        // stays one block, and `kasane_gfm::fold_newlines` folds what is left to a
        // single space. Asserting two hyphens here (as this table did) was
        // asserting an anchor against a heading line kasane does not emit.
        // Not covered by the 2026-08-09 github.com parity check either.
        assert_eq!(anchor_slug("a\n\nb"), "a-b");
        assert_eq!(anchor_slug("a\r\n\r\nb"), "a-b");
        // Literal spaces are a DIFFERENT mechanism and are still not
        // collapsed -- the `Background & Notes` row above is what that looks
        // like, and this change must not touch it.
        assert_eq!(anchor_slug("a  b"), "a--b");
        // No character in the class at all: GitHub emits an empty id, which
        // would be a dead link here. The documented divergence.
        assert_eq!(anchor_slug("***"), "section");
        // `Other_Number` is OUTSIDE Ruby's `\p{Word}`, which has
        // `Decimal_Number` rather than the whole `Number` group. The vulgar
        // fraction is removed like any other symbol; the space before it
        // survives as a trailing hyphen, since the anchor rule never trims.
        assert_eq!(anchor_slug("Fig ½"), "fig-");
        // A circled numeral is `Other_Number` too, and it is common in
        // Japanese and Chinese headings. GitHub drops it.
        assert_eq!(anchor_slug("①はじめに"), "はじめに");
        // `Letter_Number` IS inside the set -- it arrives via `Alphabetic`,
        // not via `Number` -- and downcases like any other letter.
        assert_eq!(anchor_slug("Part Ⅷ"), "part-ⅷ");
        // `Other_Alphabetic` is in `Alphabetic` too, and reaches this rule
        // only because `is_word` asks `char::is_alphabetic()` rather than
        // testing the `L*` category group. `Ⓐ` U+24B6 is `So`; it is kept, and
        // it downcases to `ⓐ` U+24D0.
        assert_eq!(anchor_slug("Ⓐ Notes"), "ⓐ-notes");
        // The boundary is `Alphabetic`, not "looks like a letter": the
        // PARENTHESIZED small letter is `So` WITHOUT `Other_Alphabetic`, so it
        // is dropped, and the space after it still becomes a hyphen. GitHub
        // drops it for the same reason.
        assert_eq!(anchor_slug("⒜ Notes"), "-notes");
        // Join_Control is inside Ruby's `\p{Word}` and GitHub keeps it. ZWNJ
        // sits INSIDE this ordinary Persian word, so dropping it would
        // mis-anchor the heading against every GitHub render.
        assert_eq!(anchor_slug("می\u{200C}رود"), "می\u{200C}رود");
        // A title that is nothing but a Join_Control character anchors to that
        // character, not to `section`: the result is non-empty, so the
        // fallback does not fire, and it is exactly the id GitHub computes, so
        // the link resolves in kasane's tree and on GitHub alike. An invisible
        // anchor is odd to look at but is not a broken one, and guarding it
        // would manufacture a divergence where there is currently none.
        assert_eq!(anchor_slug("\u{200C}"), "\u{200C}");
        // A footnote reference contributes its digits to the id: the
        // projection is what puts `[^1]` into the rendered line, and GitHub's
        // id filter strips `[`, `^` and `]`, so a resolved (superscript) and
        // an unresolved (literal `[^1]`) reference land on the same digits.
        assert_eq!(
            anchor_slug_of(&[Inline::Text("Notes".into()), Inline::FootnoteRef(NoteId(1))]),
            "notes1"
        );
        // A trailing `#` run preceded by a space is an ATX closing sequence.
        // This row is only a *parity* claim once the writer escapes the run
        // (design spec 2026-08-14 §4.2): GitHub ids the line it actually
        // renders, and after the escape that line is `Intro ###` in full.
        // Before the escape the rendered line was `Intro`, and GitHub said
        // `intro`.
        assert_eq!(anchor_slug("Intro ###"), "intro-");
        // The all-`#` content case: the text slugs to nothing, so it takes
        // `EMPTY_FALLBACK`. This is the documented empty-id divergence
        // (GitHub emits no id at all for a heading with no `\p{Word}`
        // character), not a new one -- see the module doc.
        assert_eq!(anchor_slug("###"), "section");
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
        assert_eq!(anchor_slug(nfc), "café");
        assert_eq!(anchor_slug(nfd), "cafe\u{0301}");
        assert_ne!(
            anchor_slug(nfc),
            anchor_slug(nfd),
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
        assert_eq!(anchor_slug(&long).len(), 120);
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
