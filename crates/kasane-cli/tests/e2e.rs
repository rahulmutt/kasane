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
fn converts_a_deeply_nested_epub_without_aborting() {
    // fuzz/seeds/epub/deep-nesting.epub nests <em> 5000 deep. Before the two
    // depth bounds landed (design spec 2026-07-29 §2.2) this aborted the
    // process on a stack overflow in the core's and writer's inline walks --
    // which, in batch mode, would have taken every other worker down with it.
    let out = tempfile::tempdir().unwrap();
    let out_dir = out.path().join("deep");
    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg("../../fuzz/seeds/epub/deep-nesting.epub")
        .arg("-o")
        .arg(&out_dir)
        .status()
        .unwrap();
    assert!(status.success(), "conversion failed: {:?}", status);
    let idx = std::fs::read_to_string(out_dir.join("index.md")).unwrap();
    assert!(idx.contains("title: Deep Nesting"));
    // `title: Deep Nesting` comes from the OPF's <dc:title>, not the chapter
    // -- it is present even in the exact bug this seed exists to catch (the
    // chapter silently dropped by guard.rs's zip-bomb ratio guard, book
    // parsing to zero body nodes), so it alone proves nothing about the
    // chapter surviving. Read the chapter file and check its actual content:
    // "bottom" must be present, and it must be wrapped in a run of AT LEAST
    // 64 `*` -- proving both that the chapter was not dropped, and that Task
    // 2's epub::xhtml::MAX_INLINE_DEPTH (64) flattening bound reached the
    // real CLI path (5000 nested <em> in, at most 64 nested emphasis out).
    let chapter = std::fs::read_to_string(out_dir.join("01-deep.md"))
        .expect("chapter file 01-deep.md must exist -- the chapter must not be dropped");
    assert!(chapter.contains("bottom"), "chapter body text missing");
    let stars_before = chapter
        .split("bottom")
        .next()
        .unwrap()
        .chars()
        .rev()
        .take_while(|&c| c == '*')
        .count();
    assert!(
        stars_before >= 64,
        "expected at least 64 '*' immediately before \"bottom\" (Task 2's flattening \
         bound reaching the CLI path), got {stars_before} in: {chapter:?}"
    );
}

#[test]
fn batch_mode_converts_a_deeply_block_nested_epub_without_aborting() {
    // fuzz/seeds/epub/deep-blocks.epub nests <ul> 30,000 deep. Batch mode
    // specifically: conversions run on rayon workers, which get a smaller
    // stack than `main` -- measured at roughly a quarter of the survivable
    // depth (design spec 2026-07-29 §1). A single-file version of this test
    // would pass against a bound four times too loose, so the directory
    // argument here is load-bearing, not incidental.
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("books");
    std::fs::create_dir_all(&books).unwrap();
    std::fs::copy(
        "../../fuzz/seeds/epub/deep-blocks.epub",
        books.join("deep-blocks.epub"),
    )
    .unwrap();
    let out_dir = tmp.path().join("out");

    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg(&books)
        .arg("-o")
        .arg(&out_dir)
        .status()
        .unwrap();

    assert!(
        status.success(),
        "batch conversion of a 30,000-deep list must exit 0, not abort with 134: {status:?}"
    );
    // The library index proves batch mode ran, not single-file mode.
    assert!(out_dir.join("index.md").exists(), "library index missing");
    // And the content survived the flattening rather than being truncated:
    // the innermost list item's text is the seed's only body text.
    let all = read_all_md(&out_dir);
    assert!(
        all.contains('x'),
        "innermost list item text missing -- the bound must flatten, not truncate"
    );
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

fn run_kasane(args: &[&std::ffi::OsStr]) -> i32 {
    Command::new(env!("CARGO_BIN_EXE_kasane"))
        .args(args)
        .status()
        .unwrap()
        .code()
        .expect("process must exit normally")
}

#[test]
fn partial_success_exits_three() {
    let tmp = tempfile::tempdir().unwrap();
    let books = library(tmp.path());
    // A DRM-protected MOBI converts to a failure, deterministically.
    std::fs::copy(fixture("mobi/minimal-drm.mobi"), books.join("locked.mobi")).unwrap();
    let out = tmp.path().join("out");

    let code = run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]);

    assert_eq!(code, 3);
    // The documents that could convert still did.
    assert!(out.join("a/minimal/index.md").exists());
    assert!(out.join("b/minimal/index.md").exists());
}

#[test]
fn all_documents_drm_protected_exits_two() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("books");
    std::fs::create_dir_all(&books).unwrap();
    std::fs::copy(fixture("mobi/minimal-drm.mobi"), books.join("locked.mobi")).unwrap();
    let out = tmp.path().join("out");

    assert_eq!(
        run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]),
        2
    );
}

#[test]
fn a_directory_with_no_documents_exits_one() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("books");
    std::fs::create_dir_all(&books).unwrap();
    std::fs::write(books.join("notes.txt"), "nothing here").unwrap();
    let out = tmp.path().join("out");

    assert_eq!(
        run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]),
        1
    );
    assert!(!out.exists(), "nothing should be written");
}

#[test]
fn duplicate_stems_are_rejected_before_writing_anything() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("x")).unwrap();
    std::fs::create_dir_all(tmp.path().join("y")).unwrap();
    std::fs::copy(fixture("epub/minimal.epub"), tmp.path().join("x/ch.epub")).unwrap();
    std::fs::copy(fixture("epub/minimal.epub"), tmp.path().join("y/ch.epub")).unwrap();
    let out = tmp.path().join("out");

    let code = run_kasane(&[
        tmp.path().join("x/ch.epub").as_os_str(),
        tmp.path().join("y/ch.epub").as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);

    assert_eq!(code, 1);
    assert!(
        !out.exists(),
        "collision must be caught before any conversion"
    );
}

/// A document sitting next to a directory of the same stem (`ch.epub` beside
/// `ch/`) maps to nested output directories. `write_tree` swaps whole
/// directories, so converting `ch.epub` would delete `ch/inner`'s tree.
#[test]
fn a_nested_output_directory_is_rejected_before_writing_anything() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("books");
    std::fs::create_dir_all(books.join("ch")).unwrap();
    std::fs::copy(fixture("epub/minimal.epub"), books.join("ch.epub")).unwrap();
    std::fs::copy(fixture("epub/minimal.epub"), books.join("ch/inner.epub")).unwrap();
    let out = tmp.path().join("out");

    let code = run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]);

    assert_eq!(code, 1);
    assert!(
        !out.exists(),
        "nesting must be caught before any conversion"
    );
}

#[test]
fn batch_without_an_output_root_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let books = library(tmp.path());

    assert_eq!(run_kasane(&[books.as_os_str()]), 1);
}

#[test]
fn a_walked_file_that_fails_to_parse_is_a_failure_not_a_skip() {
    let tmp = tempfile::tempdir().unwrap();
    let books = library(tmp.path());
    // Matching extension, unparseable content: the directory asserted this is
    // a document, so it must be reported, not silently dropped.
    std::fs::write(books.join("broken.epub"), b"definitely not a zip").unwrap();
    let out = tmp.path().join("out");

    assert_eq!(
        run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]),
        3
    );
    assert!(out.join("a/minimal/index.md").exists());
    assert!(!out.join("broken").exists());
}

#[test]
fn a_non_empty_output_root_needs_force() {
    let tmp = tempfile::tempdir().unwrap();
    let books = library(tmp.path());
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("keep.txt"), "x").unwrap();

    assert_eq!(
        run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]),
        1
    );
    // Nothing was converted into it.
    assert!(!out.join("a").exists());

    assert_eq!(
        run_kasane(&[
            books.as_os_str(),
            "-o".as_ref(),
            out.as_os_str(),
            "--force".as_ref()
        ]),
        0
    );
    assert!(out.join("a/minimal/index.md").exists());
}

#[test]
fn a_batch_run_writes_a_library_index() {
    let tmp = tempfile::tempdir().unwrap();
    let books = library(tmp.path());
    let out = tmp.path().join("out");

    assert_eq!(
        run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]),
        0
    );

    let idx = std::fs::read_to_string(out.join("index.md")).unwrap();
    assert!(idx.contains("kind: library"));
    assert!(idx.contains("documents: 2"));
    assert!(idx.contains("(a/minimal/index.md)"));
    assert!(idx.contains("(b/minimal/index.md)"));
    assert!(!idx.contains("## Failed"));
}

#[test]
fn a_failure_is_recorded_in_the_library_index() {
    let tmp = tempfile::tempdir().unwrap();
    let books = library(tmp.path());
    std::fs::copy(fixture("mobi/minimal-drm.mobi"), books.join("locked.mobi")).unwrap();
    let out = tmp.path().join("out");

    assert_eq!(
        run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]),
        3
    );

    let idx = std::fs::read_to_string(out.join("index.md")).unwrap();
    assert!(idx.contains("failed: 1"));
    assert!(idx.contains("## Failed"));
    assert!(idx.contains("locked.mobi"));
}

#[test]
fn an_all_failed_run_still_writes_the_index() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("books");
    std::fs::create_dir_all(&books).unwrap();
    std::fs::copy(fixture("mobi/minimal-drm.mobi"), books.join("locked.mobi")).unwrap();
    let out = tmp.path().join("out");

    assert_eq!(
        run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]),
        2
    );

    // A failed run leaves an on-disk record, not just a stderr trace.
    let idx = std::fs::read_to_string(out.join("index.md")).unwrap();
    assert!(idx.contains("documents: 0"));
    assert!(idx.contains("0 of 1 inputs converted."));
    assert!(idx.contains("locked.mobi"));
}

#[test]
fn single_file_mode_writes_no_library_index() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("book");

    assert_eq!(
        run_kasane(&[
            fixture("epub/minimal.epub").as_os_str(),
            "-o".as_ref(),
            out.as_os_str()
        ]),
        0
    );

    let idx = std::fs::read_to_string(out.join("index.md")).unwrap();
    assert!(idx.contains("title: Minimal Book"));
    assert!(!idx.contains("kind: library"));
}

/// Pins the up-front `--ocr-lang` validation: a bad language must fail before
/// any document is converted, not once per document inside the workers.
#[cfg(feature = "ocr")]
#[test]
fn a_bad_ocr_language_fails_before_converting_anything() {
    let tmp = tempfile::tempdir().unwrap();
    let books = library(tmp.path());
    let out = tmp.path().join("out");

    let code = run_kasane(&[
        books.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
        "--ocr".as_ref(),
        "--ocr-lang".as_ref(),
        "zzz".as_ref(),
    ]);

    // OcrError::MissingLanguage matches none of exit_code_for's exit-2
    // keywords, so this is 1 — and nothing was written.
    assert_eq!(code, 1);
    assert!(!out.exists(), "no document should have been converted");
}
