use crate::escape;
use anyhow::{Context, Result};
use std::path::Path;

/// One successfully converted document, as it appears in the library index.
pub struct LibraryEntry {
    /// `DocMeta.title`; falls back to `rel_dir` when empty.
    pub title: String,
    /// Document directory relative to the library root, e.g. `a/dune`.
    pub rel_dir: String,
    /// `DocMeta.source_format`, e.g. `epub`.
    pub format: String,
    /// Number of Markdown files in the document's tree.
    pub files: usize,
}

/// One input that could not be converted.
pub struct LibraryFailure {
    /// Input path relative to its root, extension kept, e.g. `c/drm.azw3`.
    pub input: String,
    pub reason: String,
}

/// Write `<out>/index.md`: the entry point for a batch run.
///
/// Written even when every document failed, so a failed run leaves an on-disk
/// record rather than only a stderr trace. The frontmatter holds no free text —
/// only `kind` and two counts — so no YAML quoting is needed; titles appear
/// solely as link labels, where `escape::label` handles them under the same
/// policy as the rest of the writer.
pub fn write_library_index(
    entries: &[LibraryEntry],
    failures: &[LibraryFailure],
    out: &Path,
) -> Result<()> {
    let total = entries.len() + failures.len();

    let mut s = String::new();
    s.push_str("---\nkind: library\n");
    s.push_str(&format!("documents: {}\n", entries.len()));
    s.push_str(&format!("failed: {}\n", failures.len()));
    s.push_str("---\n\n# Converted documents\n\n");
    s.push_str(&format!(
        "{} of {total} inputs converted.\n\n",
        entries.len()
    ));

    for e in entries {
        let title = if e.title.trim().is_empty() {
            &e.rel_dir
        } else {
            &e.title
        };
        s.push_str(&format!(
            "- [{}]({}/index.md) — {}, {} files\n",
            escape::label(title),
            escape::dest_path(&e.rel_dir),
            e.format,
            e.files
        ));
    }

    if !failures.is_empty() {
        s.push_str("\n## Failed\n\n");
        for f in failures {
            s.push_str(&format!(
                "- {} — {}\n",
                escape::code_span(&f.input, escape::Ctx::Flow),
                escape::label(&f.reason)
            ));
        }
    }

    std::fs::create_dir_all(out).with_context(|| format!("create {}", out.display()))?;
    let path = out.join("index.md");
    std::fs::write(&path, s).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, rel_dir: &str) -> LibraryEntry {
        LibraryEntry {
            title: title.into(),
            rel_dir: rel_dir.into(),
            format: "epub".into(),
            files: 7,
        }
    }

    fn write(entries: &[LibraryEntry], failures: &[LibraryFailure]) -> String {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("lib");
        write_library_index(entries, failures, &out).unwrap();
        std::fs::read_to_string(out.join("index.md")).unwrap()
    }

    #[test]
    fn lists_entries_and_omits_the_failed_section() {
        let md = write(&[entry("Dune", "a/dune"), entry("SICP", "b/sicp")], &[]);

        assert!(md.starts_with("---\nkind: library\n"));
        assert!(md.contains("documents: 2"));
        assert!(md.contains("failed: 0"));
        assert!(md.contains("2 of 2 inputs converted."));
        assert!(md.contains("- [Dune](a/dune/index.md) — epub, 7 files"));
        assert!(md.contains("- [SICP](b/sicp/index.md) — epub, 7 files"));
        assert!(!md.contains("## Failed"));
    }

    #[test]
    fn lists_failures_with_their_reason() {
        let md = write(
            &[entry("Dune", "a/dune")],
            &[LibraryFailure {
                input: "c/drm.azw3".into(),
                reason: "DRM-protected, unsupported".into(),
            }],
        );

        assert!(md.contains("documents: 1"));
        assert!(md.contains("failed: 1"));
        assert!(md.contains("1 of 2 inputs converted."));
        assert!(md.contains("## Failed"));
        assert!(md.contains("- `c/drm.azw3` — DRM-protected, unsupported"));
    }

    #[test]
    fn an_empty_title_falls_back_to_the_directory_name() {
        let md = write(&[entry("   ", "a/untitled")], &[]);
        assert!(
            md.contains("- [a/untitled](a/untitled/index.md)"),
            "got: {md}"
        );
    }

    #[test]
    fn a_bracket_in_the_title_cannot_break_out_of_the_label() {
        let md = write(&[entry("Bracket] and\nnewline", "a/odd")], &[]);
        assert!(
            md.contains("- [Bracket\\] and newline](a/odd/index.md)"),
            "got: {md}"
        );
    }

    #[test]
    fn a_multiline_failure_reason_stays_on_one_bullet() {
        let md = write(
            &[],
            &[LibraryFailure {
                input: "c/bad.pdf".into(),
                reason: "malformed input:\nbad xref".into(),
            }],
        );
        assert!(
            md.contains("- `c/bad.pdf` — malformed input: bad xref"),
            "got: {md}"
        );
        assert!(md.contains("0 of 1 inputs converted."));
    }

    #[test]
    fn a_space_in_rel_dir_is_percent_encoded_in_the_link_destination() {
        let md = write(&[entry("War and Peace", "War and Peace")], &[]);
        assert!(
            md.contains("- [War and Peace](War%20and%20Peace/index.md) — epub, 7 files"),
            "got: {md}"
        );
    }

    #[test]
    fn parens_in_rel_dir_are_percent_encoded_in_the_link_destination() {
        let md = write(&[entry("Book (Annotated)", "Book (Annotated)")], &[]);
        assert!(
            md.contains("- [Book (Annotated)](Book%20%28Annotated%29/index.md) — epub, 7 files"),
            "got: {md}"
        );
    }

    /// `#` starts a fragment, so `C#%20Notes/index.md` parses as path `C`
    /// with fragment `#%20Notes/index.md` — a link to nowhere.
    #[test]
    fn a_hash_in_rel_dir_is_percent_encoded_in_the_link_destination() {
        let md = write(&[entry("C# Notes", "C# Notes")], &[]);
        assert!(
            md.contains("- [C# Notes](C%23%20Notes/index.md) — epub, 7 files"),
            "got: {md}"
        );
    }

    /// `?` starts a query string, with the same effect.
    #[test]
    fn a_question_mark_in_rel_dir_is_percent_encoded_in_the_link_destination() {
        let md = write(&[entry("What Now?", "What Now?/vol 1")], &[]);
        assert!(
            md.contains("- [What Now?](What%20Now%3F/vol%201/index.md) — epub, 7 files"),
            "got: {md}"
        );
    }

    /// A literal `%` in a filename would otherwise be read back as the start
    /// of an escape: `50%20off` decodes to `50 off`, a directory that does
    /// not exist.
    #[test]
    fn a_percent_in_rel_dir_is_percent_encoded_in_the_link_destination() {
        let md = write(&[entry("Rich Book", "50%20off")], &[]);
        assert!(
            md.contains("- [Rich Book](50%2520off/index.md) — epub, 7 files"),
            "got: {md}"
        );
    }

    #[test]
    fn an_ordinary_rel_dir_passes_through_the_link_destination_unchanged() {
        let md = write(&[entry("Dune", "a/dune")], &[]);
        assert!(
            md.contains("- [Dune](a/dune/index.md) — epub, 7 files"),
            "got: {md}"
        );
    }

    #[test]
    fn empty_title_fallback_label_stays_literal_while_the_destination_still_encodes() {
        // The label falls back to `rel_dir` verbatim (through `escape::label`,
        // which does not escape spaces); the destination still goes through
        // `escape::dest_path`. Label and destination have different escaping
        // rules.
        let md = write(&[entry("   ", "a/War Stories")], &[]);
        assert!(
            md.contains("- [a/War Stories](a/War%20Stories/index.md)"),
            "got: {md}"
        );
    }

    #[test]
    fn a_newline_in_the_failure_input_stays_on_one_bullet() {
        let md = write(
            &[],
            &[LibraryFailure {
                input: "c/weird\nname.pdf".into(),
                reason: "malformed input".into(),
            }],
        );
        assert!(
            md.contains("- `c/weird name.pdf` — malformed input"),
            "got: {md}"
        );
    }

    #[test]
    fn a_title_with_markdown_in_it_is_escaped_not_substituted() {
        // link_text replaced `[` with `(`, which changes the rendered text.
        // Section 5 forbids that: anchors are computed from unescaped IR text.
        let md = write(&[entry("A [b] *c*", "a/x")], &[]);
        assert!(
            md.contains("- [A \\[b\\] \\*c\\*](a/x/index.md)"),
            "got: {md}"
        );
    }

    #[test]
    fn a_failure_reason_is_escaped_inside_its_backticks() {
        let md = write(
            &[],
            &[LibraryFailure {
                input: "c/d`e.azw3".into(),
                reason: "bad [thing]\nsecond line".into(),
            }],
        );
        assert!(md.contains("bad \\[thing\\] second line"), "got: {md}");
        assert!(
            md.lines().filter(|l| l.starts_with("- ")).count() == 1,
            "the reason spilled onto another line: {md}"
        );
    }
}
