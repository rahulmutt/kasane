use std::process::Command;

#[test]
fn converts_minimal_epub_to_tree() {
    let out = tempfile::tempdir().unwrap();
    let out_dir = out.path().join("book");
    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg("../../tests/fixtures/epub/minimal.epub")
        .arg("-o")
        .arg(&out_dir)
        .status()
        .unwrap();
    assert!(status.success());
    let idx = std::fs::read_to_string(out_dir.join("index.md")).unwrap();
    assert!(idx.contains("title: Minimal Book"));
    // Chapter One became its own file; internal link resolved
    let ch = std::fs::read_to_string(out_dir.join("01-chapter-one.md"))
        .or_else(|_| std::fs::read_to_string(out_dir.join("01-chapter-one/index.md")))
        .unwrap();
    assert!(ch.contains("Section Two"));
}

#[test]
fn converts_rich_epub_with_full_fidelity() {
    let out = tempfile::tempdir().unwrap();
    let out_dir = out.path().join("rich");
    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg("../../tests/fixtures/epub/rich.epub")
        .arg("-o")
        .arg(&out_dir)
        // Disable merge/split so section->file mapping is deterministic.
        .arg("--min-tokens")
        .arg("0")
        .arg("--max-tokens")
        .arg("100000")
        .status()
        .unwrap();
    assert!(status.success());

    // Gather every emitted markdown file.
    let mut all = String::new();
    let mut files: Vec<(std::path::PathBuf, String)> = vec![];
    let mut stack = vec![out_dir.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "md") {
                let s = std::fs::read_to_string(&p).unwrap();
                all.push_str(&s);
                files.push((p, s));
            }
        }
    }

    // Lists (nested), table, code — present somewhere in the tree.
    assert!(all.contains("- alpha"), "bullet list missing");
    assert!(all.contains("beta-one"), "nested list item missing");
    assert!(all.contains("| Name | Value |"), "GFM table header missing");
    assert!(all.contains("```rust"), "code block language missing");
    assert!(all.contains("`inline_code()`"), "inline code missing");

    // Image: link in markdown + actual bytes flushed under _assets/.
    assert!(
        all.contains("![The red dot](_assets/"),
        "figure link missing"
    );
    let assets: Vec<_> = std::fs::read_dir(out_dir.join("_assets"))
        .unwrap()
        .collect();
    assert_eq!(assets.len(), 1, "exactly one extracted asset");

    // Footnote: ref and definition in the SAME file.
    let fnote_file = files
        .iter()
        .find(|(_, s)| s.contains("[^1]") && !s.contains("[^1]:"))
        .or_else(|| files.iter().find(|(_, s)| s.contains("[^1]")));
    let (_, s) = fnote_file.expect("no file contains the footnote ref");
    assert!(
        s.contains("[^1]") && s.contains("[^1]: Footnote body text."),
        "footnote ref and definition must share a file"
    );

    // Cross-chapter link resolved to a real relative .md path.
    let (link_file, link_src) = files
        .iter()
        .find(|(_, s)| s.contains("](") && s.contains("the second section"))
        .expect("cross-chapter link text missing");
    let target = link_src
        .split("[the second section](")
        .nth(1)
        .and_then(|r| r.split(')').next())
        .expect("link not in markdown form — was it stripped to text?");
    let target_path = link_file
        .parent()
        .unwrap()
        .join(target.split('#').next().unwrap());
    assert!(
        target_path.exists(),
        "link target {target} does not exist on disk"
    );
}

fn read_all_md(out_dir: &std::path::Path) -> String {
    let mut all = String::new();
    let mut stack = vec![out_dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "md") {
                all.push_str(&std::fs::read_to_string(&p).unwrap());
            }
        }
    }
    all
}

fn read_all_md_with_files(
    out_dir: &std::path::Path,
) -> (String, Vec<(std::path::PathBuf, String)>) {
    let mut all = String::new();
    let mut files: Vec<(std::path::PathBuf, String)> = vec![];
    let mut stack = vec![out_dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "md") {
                let s = std::fs::read_to_string(&p).unwrap();
                all.push_str(&s);
                files.push((p, s));
            }
        }
    }
    (all, files)
}

#[test]
fn converts_minimal_mobi_to_tree() {
    let out = tempfile::tempdir().unwrap();
    let out_dir = out.path().join("book");
    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg("../../tests/fixtures/mobi/minimal.mobi")
        .arg("-o")
        .arg(&out_dir)
        .arg("--min-tokens")
        .arg("0")
        .status()
        .unwrap();
    assert!(status.success());
    let idx = std::fs::read_to_string(out_dir.join("index.md")).unwrap();
    assert!(idx.contains("title: Minimal Mobi"));
    let (all, files) = read_all_md_with_files(&out_dir);
    assert!(all.contains("Chapter One") && all.contains("Chapter Two"));
    assert!(
        !all.contains("[]()"),
        "stray empty anchor-marker link leaked into emitted markdown"
    );
    assert!(all.contains("- alpha"), "bullet list missing");
    assert!(all.contains("beta-one"), "nested list item missing");
    assert!(
        all.contains("![The red dot](_assets/"),
        "figure link missing"
    );
    let assets: Vec<_> = std::fs::read_dir(out_dir.join("_assets"))
        .unwrap()
        .collect();
    assert_eq!(assets.len(), 1, "exactly one extracted asset");
    // Verify asset is real PNG bytes.
    let asset_path = assets[0].as_ref().unwrap().path();
    let asset_bytes = std::fs::read(&asset_path).unwrap();
    assert!(
        asset_bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "extracted asset is not a valid PNG"
    );
    // Filepos link resolved: the body cross-reference lives in the
    // chapter-one file ("...see [Chapter Two](02-chapter-two.md#chapter-two)").
    // index.md also contains the substring "[Chapter Two](" via its own
    // auto-generated TOC entry ("- [Chapter Two](02-chapter-two.md)"), and
    // read_dir enumerates index.md first, so it must be excluded by name to
    // bind the assertion to the resolved cross-reference rather than the TOC.
    let (link_file, link_src) = files
        .iter()
        .find(|(p, s)| {
            p.file_name().and_then(|n| n.to_str()) != Some("index.md")
                && s.contains("[Chapter Two](")
        })
        .expect("body cross-reference '[Chapter Two](' missing outside index.md");
    let href = link_src
        .split("[Chapter Two](")
        .nth(1)
        .and_then(|r| r.split(')').next())
        .expect("link not in markdown form — was it stripped to text?");
    let target_path = link_file
        .parent()
        .unwrap()
        .join(href.split('#').next().unwrap());
    assert!(
        target_path.exists(),
        "link target {href} does not exist on disk"
    );
    let target_content = std::fs::read_to_string(&target_path).unwrap();
    assert!(
        target_content.contains("Chapter Two"),
        "resolved link target does not contain the expected heading"
    );
}

#[test]
fn converts_minimal_azw3_to_tree() {
    let out = tempfile::tempdir().unwrap();
    let out_dir = out.path().join("book");
    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg("../../tests/fixtures/azw3/minimal.azw3")
        .arg("-o")
        .arg(&out_dir)
        .arg("--min-tokens")
        .arg("0")
        .status()
        .unwrap();
    assert!(status.success());
    let idx = std::fs::read_to_string(out_dir.join("index.md")).unwrap();
    assert!(idx.contains("title: KF8 Minimal"));
    let (all, files) = read_all_md_with_files(&out_dir);
    assert!(all.contains("Part One") && all.contains("Part Two"));
    assert!(
        !all.contains("[]()"),
        "stray empty anchor-marker link leaked into emitted markdown"
    );
    assert!(all.contains("| Name | Value |"), "GFM table header missing");
    assert!(all.contains("```rust"), "code block language missing");
    assert!(
        all.contains("![The red dot](_assets/"),
        "kindle:embed figure missing"
    );
    let assets: Vec<_> = std::fs::read_dir(out_dir.join("_assets"))
        .unwrap()
        .collect();
    assert_eq!(assets.len(), 1, "exactly one extracted asset");
    // Verify asset is real PNG bytes.
    let asset_path = assets[0].as_ref().unwrap().path();
    let asset_bytes = std::fs::read(&asset_path).unwrap();
    assert!(
        asset_bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "extracted asset is not a valid PNG"
    );
    // Cross-part link resolved: the body cross-reference lives in the
    // part-one file ("...see [Part Two](02-part-two.md#part-two)"). index.md
    // also contains the substring "[Part Two](" via its own auto-generated
    // TOC entry ("- [Part Two](02-part-two.md)"), and read_dir enumerates
    // index.md first, so it must be excluded by name to bind the assertion
    // to the resolved cross-reference rather than the TOC.
    let (link_file, link_src) = files
        .iter()
        .find(|(p, s)| {
            p.file_name().and_then(|n| n.to_str()) != Some("index.md") && s.contains("[Part Two](")
        })
        .expect("body cross-reference '[Part Two](' missing outside index.md");
    let href = link_src
        .split("[Part Two](")
        .nth(1)
        .and_then(|r| r.split(')').next())
        .expect("link not in markdown form — was it stripped to text?");
    let target_path = link_file
        .parent()
        .unwrap()
        .join(href.split('#').next().unwrap());
    assert!(
        target_path.exists(),
        "link target {href} does not exist on disk"
    );
    let target_content = std::fs::read_to_string(&target_path).unwrap();
    assert!(
        target_content.contains("Part Two"),
        "resolved link target does not contain the expected heading"
    );
}

#[test]
fn drm_mobi_exits_2() {
    let out = tempfile::tempdir().unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg("../../tests/fixtures/mobi/minimal-drm.mobi")
        .arg("-o")
        .arg(out.path().join("x"))
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(2));
}

#[test]
fn lying_skel_azw3_still_converts() {
    let out = tempfile::tempdir().unwrap();
    let out_dir = out.path().join("book");
    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg("../../tests/fixtures/azw3/lying-skel.azw3")
        .arg("-o")
        .arg(&out_dir)
        .arg("--min-tokens")
        .arg("0")
        .status()
        .unwrap();
    assert!(status.success(), "degrade, don't die");
    assert!(read_all_md(&out_dir).contains("Part Two"));
}

use std::path::{Path, PathBuf};

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(rel)
}

/// `books/a/minimal.epub`, `books/b/minimal.pdf`, `books/notes.txt`
fn library(dir: &Path) -> PathBuf {
    let books = dir.join("books");
    std::fs::create_dir_all(books.join("a")).unwrap();
    std::fs::create_dir_all(books.join("b")).unwrap();
    std::fs::copy(fixture("epub/minimal.epub"), books.join("a/minimal.epub")).unwrap();
    std::fs::copy(fixture("pdf/minimal.pdf"), books.join("b/minimal.pdf")).unwrap();
    std::fs::write(books.join("notes.txt"), "not a document").unwrap();
    books
}

/// Every file under `root`, as (relative path, contents), sorted.
fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn converts_a_directory_of_documents() {
    let tmp = tempfile::tempdir().unwrap();
    let books = library(tmp.path());
    let out = tmp.path().join("out");

    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg(&books)
        .arg("-o")
        .arg(&out)
        .status()
        .unwrap();

    assert!(status.success(), "expected exit 0, got {status:?}");
    // Each document keeps its path relative to the walk root.
    assert!(out.join("a/minimal/index.md").exists());
    assert!(out.join("b/minimal/index.md").exists());
    // The non-document is skipped silently.
    assert!(!out.join("notes").exists());
}

#[test]
fn single_file_output_shape_is_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("book");

    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg(fixture("epub/minimal.epub"))
        .arg("-o")
        .arg(&out)
        .status()
        .unwrap();

    assert!(status.success());
    // `out` IS the document root — not a library wrapper around it.
    let idx = std::fs::read_to_string(out.join("index.md")).unwrap();
    assert!(idx.contains("title: Minimal Book"));
    assert!(!out.join("minimal").exists());
}

#[test]
fn multiple_explicit_files_convert_together() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");

    // Distinct stems, so no collision. (Two fixtures both named `minimal`
    // would collide by design — see `duplicate_stems_are_rejected` in Task 4.)
    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg(fixture("epub/minimal.epub"))
        .arg(fixture("epub/rich.epub"))
        .arg("-o")
        .arg(&out)
        .status()
        .unwrap();

    assert!(status.success());
    assert!(out.join("minimal/index.md").exists());
    assert!(out.join("rich/index.md").exists());
}

#[test]
fn jobs_does_not_change_the_output() {
    let tmp = tempfile::tempdir().unwrap();
    let books = library(tmp.path());

    let mut trees = Vec::new();
    for jobs in ["1", "4"] {
        let out = tmp.path().join(format!("out-{jobs}"));
        let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
            .arg(&books)
            .arg("-o")
            .arg(&out)
            .arg("-j")
            .arg(jobs)
            .status()
            .unwrap();
        assert!(status.success());
        trees.push(snapshot(&out));
    }
    assert_eq!(trees[0], trees[1], "-j must not change the emitted tree");
}
