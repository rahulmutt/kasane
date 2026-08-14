//! Pure: which of a page's text lines merely reprint one of its outline titles.
//!
//! Both the PDF and DjVu adapters splice a page's outline (bookmark/NAVM)
//! headings ahead of that page's own recovered text, so a chapter title that is
//! also *printed* on the page appeared twice: once as the heading, once in the
//! body. An outline also suppresses size-based heading inference, so the
//! printed line is never a heading of its own — it is either a paragraph
//! (`minimal.pdf`, whose leading exceeds the paragraph-break gap) or fused into
//! the following one (`sample.djvu`, one zone-level paragraph). Which of the
//! two it is depends on the page's layout, not on anything the adapter knows,
//! which is why this works at the line level, before blocks are assembled.
//!
//! The rule is deliberately conservative: a false positive silently deletes
//! document text, which is worse than the duplicate it removes. Matching is on
//! whole lines, by equality after normalization, with no prefix or substring
//! form.

/// Longest run of consecutive lines that may jointly match one title. Two
/// covers a title set across two printed lines; the cap is what keeps a long
/// title from claiming a paragraph.
const MAX_RUN: usize = 3;

/// Per-line drop mask, `true` where the line reproduces one of `titles`.
///
/// Each title, in outline order, claims the first not-yet-claimed run of 1..=3
/// consecutive lines whose joined normalization equals the title's. A claimed
/// line cannot serve a second title, and a title that matches nothing leaves
/// the page untouched.
pub(crate) fn title_line_mask(lines: &[&str], titles: &[&str]) -> Vec<bool> {
    let mut drop = vec![false; lines.len()];
    let norm: Vec<String> = lines.iter().map(|l| normalize(l)).collect();

    for title in titles {
        let want = normalize(title);
        // A title that normalizes away (`***`, `—`) would otherwise match every
        // line that also normalizes away.
        if want.is_empty() {
            continue;
        }
        if let Some((start, len)) = find_run(&norm, &drop, &want) {
            drop[start..start + len].fill(true);
        }
    }
    drop
}

/// First run of unclaimed, non-empty lines whose joined normalization is `want`.
fn find_run(norm: &[String], drop: &[bool], want: &str) -> Option<(usize, usize)> {
    for start in 0..norm.len() {
        let mut joined = String::new();
        for len in 1..=MAX_RUN.min(norm.len() - start) {
            let i = start + len - 1;
            // A claimed or empty-normalizing line ends every run through it.
            if drop[i] || norm[i].is_empty() {
                break;
            }
            if !joined.is_empty() {
                joined.push(' ');
            }
            joined.push_str(&norm[i]);
            if joined == want {
                return Some((start, len));
            }
            // Runs only grow, so once past the target there is nothing longer
            // to find from this start.
            if joined.len() > want.len() {
                break;
            }
        }
    }
    None
}

/// Lowercase, every non-alphanumeric character to a separator, runs collapsed,
/// trimmed. Folds `CHAPTER 1 — The Beginning` and `Chapter 1: the beginning`
/// onto one another.
///
/// Not `kasane_core::slug`'s word class: that one is a mirror of GitHub's
/// heading-id filter and carries its own drift contract, which this comparison
/// has no reason to be bound to (and `kasane-adapters` does not depend on
/// `kasane-core`).
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_sep = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push(' ');
            }
            pending_sep = false;
            out.extend(c.to_lowercase());
        } else {
            pending_sep = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(lines: &[&str], titles: &[&str]) -> Vec<bool> {
        title_line_mask(lines, titles)
    }

    #[test]
    fn drops_a_line_equal_to_the_title() {
        assert_eq!(
            mask(&["Chapter One", "First body line."], &["Chapter One"]),
            vec![true, false]
        );
    }

    #[test]
    fn folds_case_and_punctuation() {
        // Printed all-caps with an em dash; bookmarked in title case with a colon.
        assert_eq!(
            mask(
                &["CHAPTER 1 — THE BEGINNING", "Body."],
                &["Chapter 1: The Beginning"]
            ),
            vec![true, false]
        );
    }

    #[test]
    fn matches_a_title_printed_across_two_lines() {
        assert_eq!(
            mask(
                &["Chapter 1", "The Beginning", "Body."],
                &["Chapter 1: The Beginning"]
            ),
            vec![true, true, false]
        );
    }

    #[test]
    fn a_run_longer_than_the_cap_is_not_matched() {
        let lines = ["a", "b", "c", "d"];
        assert_eq!(mask(&lines, &["a b c d"]), vec![false; 4]);
    }

    #[test]
    fn no_match_changes_nothing() {
        let lines = ["Something else entirely.", "More body."];
        assert_eq!(mask(&lines, &["Chapter One"]), vec![false, false]);
    }

    #[test]
    fn no_prefix_or_substring_match() {
        // The line must account for the whole title, and vice versa.
        assert_eq!(
            mask(&["Chapter 1"], &["Chapter 1: The Beginning"]),
            vec![false]
        );
        assert_eq!(
            mask(
                &["Chapter 1: The Beginning, continued"],
                &["Chapter 1: The Beginning"]
            ),
            vec![false]
        );
    }

    #[test]
    fn each_title_claims_at_most_one_line() {
        // A running header repeating the title leaves the second copy alone.
        assert_eq!(
            mask(&["Notes", "Body.", "Notes"], &["Notes"]),
            vec![true, false, false]
        );
    }

    #[test]
    fn a_claimed_line_does_not_serve_a_second_title() {
        // Two identical bookmarks on one page consume two distinct lines.
        assert_eq!(
            mask(&["Notes", "Body.", "Notes"], &["Notes", "Notes"]),
            vec![true, false, true]
        );
    }

    #[test]
    fn several_titles_match_anywhere_on_the_page_in_order() {
        let lines = ["14 · A Tale", "Section A", "Body.", "Section B", "More."];
        assert_eq!(
            mask(&lines, &["Section A", "Section B"]),
            vec![false, true, false, true, false]
        );
    }

    #[test]
    fn a_title_that_normalizes_away_never_matches() {
        // `***` against a line that also normalizes to nothing.
        assert_eq!(mask(&["———", "Body."], &["***"]), vec![false, false]);
    }

    #[test]
    fn an_empty_normalizing_line_breaks_a_run() {
        // "Chapter 1" + "The Beginning" would match, but the divider between
        // them is not part of any title.
        assert_eq!(
            mask(
                &["Chapter 1", "———", "The Beginning"],
                &["Chapter 1: The Beginning"]
            ),
            vec![false, false, false]
        );
    }

    #[test]
    fn no_titles_drops_nothing() {
        assert_eq!(mask(&["Anything", "at all"], &[]), vec![false, false]);
    }

    #[test]
    fn normalization_folds_whitespace_runs_and_edges() {
        assert_eq!(normalize("  Chapter\t\t One  "), "chapter one");
        assert_eq!(normalize("***"), "");
    }
}
