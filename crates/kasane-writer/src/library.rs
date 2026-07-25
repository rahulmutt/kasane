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
/// solely as link labels, where `link_text` neutralizes them.
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
            link_text(title),
            e.rel_dir,
            e.format,
            e.files
        ));
    }

    if !failures.is_empty() {
        s.push_str("\n## Failed\n\n");
        for f in failures {
            s.push_str(&format!("- `{}` — {}\n", f.input, one_line(&f.reason)));
        }
    }

    std::fs::create_dir_all(out).with_context(|| format!("create {}", out.display()))?;
    let path = out.join("index.md");
    std::fs::write(&path, s).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Neutralize the narrow subset that would corrupt a Markdown link label. The
/// repo-wide escaping policy is a separate, known-deferred item.
fn link_text(s: &str) -> String {
    s.replace('[', "(")
        .replace(']', ")")
        .replace(['\n', '\r'], " ")
}

/// Collapse a multi-line error message onto a single bullet line.
fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
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
    fn link_text_cannot_break_out_of_the_label() {
        let md = write(&[entry("Bracket] and\nnewline", "a/odd")], &[]);
        assert!(
            md.contains("- [Bracket) and newline](a/odd/index.md)"),
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
}
