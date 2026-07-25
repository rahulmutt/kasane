use crate::convert::WorkItem;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Extensions a directory walk will consider. `detect` remains authoritative on
/// the file's bytes inside `convert_one`; this only decides which files are
/// candidates, because sniffing a ZIP container needs the whole file (the
/// central directory sits at the end) and walking would then read every byte
/// twice.
const SUPPORTED_EXTS: &[&str] = &["epub", "pptx", "mobi", "azw3", "pdf", "djvu", "djv"];

fn has_supported_ext(p: &Path) -> bool {
    p.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|e| SUPPORTED_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Expand the positional inputs into the run's work list.
///
/// A named file is its own root and maps to its stem. A directory is the root
/// for everything beneath it, walked recursively, and each document keeps its
/// path relative to that root (extension dropped) as its output directory.
#[allow(dead_code)] // wired into main in the next task
pub fn discover(inputs: &[PathBuf], out: &Path) -> Result<Vec<WorkItem>> {
    let mut items = Vec::new();
    for input in inputs {
        let meta = std::fs::symlink_metadata(input)
            .with_context(|| format!("reading {}", input.display()))?;
        if meta.is_dir() {
            walk(input, input, out, &mut items)?;
        } else {
            let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
            items.push(WorkItem {
                rel: input
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                input: input.clone(),
                out_dir: out.join(stem),
            });
        }
    }
    check_collisions(&items)?;
    Ok(items)
}

fn walk(dir: &Path, root: &Path, out: &Path, items: &mut Vec<WorkItem>) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading directory {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    // Deterministic work list, library index, and summary.
    entries.sort();

    for path in entries {
        let meta = std::fs::symlink_metadata(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        // Never follow symlinks: no cycles, and the walk cannot escape its root.
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            walk(&path, root, out, items)?;
        } else if has_supported_ext(&path) {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            items.push(WorkItem {
                rel: rel.to_string_lossy().into_owned(),
                out_dir: out.join(rel.with_extension("")),
                input: path.clone(),
            });
        }
    }
    Ok(())
}

/// Reject two inputs mapping to the same output directory before any
/// conversion starts, so a long run cannot die halfway through.
fn check_collisions(items: &[WorkItem]) -> Result<()> {
    let mut seen: HashMap<&Path, &Path> = HashMap::new();
    for it in items {
        if let Some(prev) = seen.insert(it.out_dir.as_path(), it.input.as_path()) {
            bail!(
                "duplicate output directory {}: both {} and {} map to it",
                it.out_dir.display(),
                prev.display(),
                it.input.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a temp tree: `books/a/ch.epub`, `books/b/ch.epub`, `books/top.pdf`,
    /// `books/notes.txt`, `books/a/cover.png`.
    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let books = dir.path().join("books");
        std::fs::create_dir_all(books.join("a")).unwrap();
        std::fs::create_dir_all(books.join("b")).unwrap();
        for (rel, body) in [
            ("a/ch.epub", "x"),
            ("b/ch.epub", "x"),
            ("top.pdf", "x"),
            ("notes.txt", "x"),
            ("a/cover.png", "x"),
        ] {
            std::fs::write(books.join(rel), body).unwrap();
        }
        dir
    }

    fn rels(items: &[WorkItem]) -> Vec<String> {
        items.iter().map(|i| i.rel.clone()).collect()
    }

    #[test]
    fn walks_recursively_and_keeps_only_supported_extensions() {
        let dir = tree();
        let out = dir.path().join("out");
        let items = discover(&[dir.path().join("books")], &out).unwrap();

        // Sorted within each directory, directories descended in sorted order.
        assert_eq!(rels(&items), vec!["a/ch.epub", "b/ch.epub", "top.pdf"]);
    }

    #[test]
    fn output_dir_mirrors_the_path_relative_to_its_root() {
        let dir = tree();
        let out = dir.path().join("out");
        let items = discover(&[dir.path().join("books")], &out).unwrap();

        assert_eq!(items[0].out_dir, out.join("a/ch"));
        assert_eq!(items[1].out_dir, out.join("b/ch"));
        assert_eq!(items[2].out_dir, out.join("top"));
    }

    #[test]
    fn an_explicit_file_is_its_own_root_and_maps_to_its_stem() {
        let dir = tree();
        let out = dir.path().join("out");
        let items = discover(&[dir.path().join("books/a/ch.epub")], &out).unwrap();

        assert_eq!(rels(&items), vec!["ch.epub"]);
        assert_eq!(items[0].out_dir, out.join("ch"));
    }

    #[test]
    fn an_explicit_file_bypasses_the_extension_filter() {
        let dir = tempfile::tempdir().unwrap();
        let odd = dir.path().join("oddly-named-file");
        std::fs::write(&odd, "x").unwrap();

        let items = discover(std::slice::from_ref(&odd), &dir.path().join("out")).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].input, odd);
    }

    #[test]
    fn every_supported_extension_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let books = dir.path().join("books");
        std::fs::create_dir_all(&books).unwrap();
        // Distinct stems: `out_dir` drops the extension, so seven files named
        // `doc.*` would all map to `out/doc` and trip the collision check that
        // `duplicate_destinations_are_rejected_before_any_work` pins. This test
        // is about the extension filter, not about collisions.
        for ext in ["epub", "pptx", "mobi", "azw3", "pdf", "djvu", "djv"] {
            std::fs::write(books.join(format!("doc-{ext}.{ext}")), "x").unwrap();
        }
        // Two extensions a document could plausibly sit beside, both rejected.
        std::fs::write(books.join("doc.txt"), "x").unwrap();
        std::fs::write(books.join("doc.zip"), "x").unwrap();

        let items = discover(&[books], &dir.path().join("out")).unwrap();
        assert_eq!(items.len(), 7, "got: {:?}", rels(&items));
        assert!(!rels(&items)
            .iter()
            .any(|r| r.ends_with(".txt") || r.ends_with(".zip")));
    }

    #[test]
    fn the_extension_filter_is_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let books = dir.path().join("books");
        std::fs::create_dir_all(&books).unwrap();
        std::fs::write(books.join("SHOUT.EPUB"), "x").unwrap();

        let items = discover(&[books], &dir.path().join("out")).unwrap();
        assert_eq!(rels(&items), vec!["SHOUT.EPUB"]);
    }

    #[test]
    fn duplicate_destinations_are_rejected_before_any_work() {
        let dir = tree();
        let out = dir.path().join("out");
        // Two explicit files sharing a stem both map to `out/ch`.
        let err = discover(
            &[
                dir.path().join("books/a/ch.epub"),
                dir.path().join("books/b/ch.epub"),
            ],
            &out,
        )
        .unwrap_err();

        let msg = format!("{err:#}");
        assert!(msg.contains("duplicate output directory"), "got: {msg}");
        assert!(
            msg.contains("a/ch.epub") && msg.contains("b/ch.epub"),
            "got: {msg}"
        );
    }

    #[test]
    fn nested_duplicate_stems_do_not_collide() {
        let dir = tree();
        // `books/a/ch.epub` and `books/b/ch.epub` under one root are fine.
        assert!(discover(&[dir.path().join("books")], &dir.path().join("out")).is_ok());
    }

    #[test]
    fn a_missing_input_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = discover(&[dir.path().join("nope")], &dir.path().join("out")).unwrap_err();
        assert!(format!("{err:#}").contains("nope"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directories_are_not_followed() {
        let dir = tree();
        let books = dir.path().join("books");
        std::os::unix::fs::symlink(books.join("a"), books.join("link")).unwrap();

        let items = discover(&[books], &dir.path().join("out")).unwrap();
        // `link/ch.epub` must not appear.
        assert_eq!(rels(&items), vec!["a/ch.epub", "b/ch.epub", "top.pdf"]);
    }
}
