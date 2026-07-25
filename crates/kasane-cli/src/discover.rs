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
pub fn discover(inputs: &[PathBuf], out: &Path) -> Result<Vec<WorkItem>> {
    let mut items = Vec::new();
    for input in inputs {
        // A path the user named is trusted: follow it. Paths found by walking
        // are not — `walk` keeps using symlink_metadata, so the walk still
        // cannot escape its root or hit a cycle.
        let meta =
            std::fs::metadata(input).with_context(|| format!("reading {}", input.display()))?;
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

/// Reject two inputs whose output directories would clash, before any
/// conversion starts, so a long run cannot die halfway through.
///
/// Two ways to clash, both fatal:
///
/// - **Equal** destinations — the second document would overwrite the first.
/// - **Nested** destinations — `write_tree` swaps the whole output directory
///   (`rename(out, backup)` / `rename(tmp, out)` / `remove_dir_all(backup)`),
///   so a document whose directory *contains* another's annihilates it. The
///   ordinary trigger is a document beside a directory of the same stem:
///   `books/ch.epub` maps to `out/ch`, which contains `out/ch/inner` from
///   `books/ch/inner.epub`.
///
/// Nesting is found by sorting the destinations and testing each against its
/// predecessor. Sorting makes the check order-independent, and adjacency is
/// enough: if `a` contains `c` and `b` sorts between them, `a` contains `b`
/// too, so the `(a, b)` pair fires first. `Path::starts_with` is
/// component-wise, so `out/ch` is correctly *not* inside `out/chapter`.
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

    // Destinations are distinct by now, so `starts_with` means strict nesting.
    let mut sorted: Vec<&WorkItem> = items.iter().collect();
    sorted.sort_by(|a, b| a.out_dir.cmp(&b.out_dir));
    for pair in sorted.windows(2) {
        let (outer, inner) = (pair[0], pair[1]);
        if inner.out_dir.starts_with(&outer.out_dir) {
            bail!(
                "nested output directory {} inside {}: {} would be written over {}, \
                 whose whole directory is replaced",
                inner.out_dir.display(),
                outer.out_dir.display(),
                outer.input.display(),
                inner.input.display()
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
    fn multiple_matches_in_one_directory_are_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let books = dir.path().join("books");
        std::fs::create_dir_all(&books).unwrap();
        // Written out of order so an unsorted (or read_dir-order) walk would fail.
        std::fs::write(books.join("m.pdf"), "x").unwrap();
        std::fs::write(books.join("a.epub"), "x").unwrap();
        std::fs::write(books.join("z.djvu"), "x").unwrap();

        let items = discover(&[books], &dir.path().join("out")).unwrap();
        assert_eq!(rels(&items), vec!["a.epub", "m.pdf", "z.djvu"]);
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

    fn item(input: &str, out_dir: &str) -> WorkItem {
        WorkItem {
            rel: input.into(),
            input: PathBuf::from(input),
            out_dir: PathBuf::from(out_dir),
        }
    }

    /// `write_tree` swaps whole directories, so an output directory that
    /// *contains* another's annihilates it. Nesting must be rejected up front,
    /// exactly like equality.
    #[test]
    fn an_output_directory_nested_inside_another_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let books = dir.path().join("books");
        std::fs::create_dir_all(books.join("ch")).unwrap();
        std::fs::write(books.join("ch.epub"), "x").unwrap();
        std::fs::write(books.join("ch/inner.epub"), "x").unwrap();
        let out = dir.path().join("out");

        let err = discover(&[books], &out).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ch.epub"), "got: {msg}");
        assert!(msg.contains("inner.epub"), "got: {msg}");
        assert!(
            msg.contains(&out.join("ch").display().to_string()),
            "got: {msg}"
        );
        assert!(
            msg.contains(&out.join("ch/inner").display().to_string()),
            "got: {msg}"
        );
    }

    #[test]
    fn nesting_is_rejected_with_the_container_discovered_first() {
        let err = check_collisions(&[
            item("books/ch.epub", "out/ch"),
            item("books/ch/inner.epub", "out/ch/inner"),
        ])
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("books/ch.epub") && msg.contains("books/ch/inner.epub"),
            "got: {msg}"
        );
        assert!(
            msg.contains("out/ch") && msg.contains("out/ch/inner"),
            "got: {msg}"
        );
    }

    #[test]
    fn nesting_is_rejected_with_the_contained_directory_discovered_first() {
        let err = check_collisions(&[
            item("books/ch/inner.epub", "out/ch/inner"),
            item("books/ch.epub", "out/ch"),
        ])
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("books/ch.epub") && msg.contains("books/ch/inner.epub"),
            "got: {msg}"
        );
        assert!(
            msg.contains("out/ch") && msg.contains("out/ch/inner"),
            "got: {msg}"
        );
    }

    /// Containment is a *path* relation, not a string one: `out/ch` is not a
    /// prefix of `out/chapter`. A naive `str::starts_with` would reject this.
    #[test]
    fn a_shared_name_prefix_is_not_containment() {
        assert!(check_collisions(&[
            item("books/ch.epub", "out/ch"),
            item("books/chapter.epub", "out/chapter"),
        ])
        .is_ok());
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

    /// A top-level argument is a path the user named directly, so it is
    /// trusted: a symlink to a directory, passed on the command line, is
    /// walked as that directory rather than treated as an explicit file.
    /// Contrast `symlinked_directories_are_not_followed`, which pins the
    /// opposite behavior for symlinks discovered *inside* a walk.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_top_level_directory_is_walked() {
        let dir = tree();
        let books = dir.path().join("books");
        let link = dir.path().join("books-link");
        std::os::unix::fs::symlink(&books, &link).unwrap();

        let items = discover(&[link], &dir.path().join("out")).unwrap();
        assert_eq!(rels(&items), vec!["a/ch.epub", "b/ch.epub", "top.pdf"]);
    }
}
