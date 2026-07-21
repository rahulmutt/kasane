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
    // Filepos link rendered as a relative markdown link to chapter-two file.
    // Find a file that has links and references chapter-two.
    let has_chapter_two_link = files
        .iter()
        .find(|(_, s)| s.contains("](") && s.contains("chapter-two"))
        .map(|(link_file, link_src)| {
            // Extract all link targets from this file and verify at least one points to chapter-two.
            let has_target = link_src.contains("chapter-two.md");
            let target_exists = if has_target {
                let chapter_two = link_file.parent().unwrap().join("02-chapter-two.md");
                chapter_two.exists()
            } else {
                false
            };
            has_target && target_exists
        });
    assert!(
        has_chapter_two_link.unwrap_or(false),
        "link to chapter-two file missing or invalid"
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
    // Cross-part link resolved to a real relative .md path.
    // Find a file that has links and references part-two.
    let has_part_two_link = files
        .iter()
        .find(|(_, s)| s.contains("](") && s.contains("part-two"))
        .map(|(link_file, link_src)| {
            // Extract all link targets from this file and verify at least one points to part-two.
            let has_target = link_src.contains("part-two.md") || link_src.contains("02-part-two");
            let target_exists = if has_target {
                let part_two = link_file.parent().unwrap().join("02-part-two.md");
                part_two.exists()
            } else {
                false
            };
            has_target && target_exists
        });
    assert!(
        has_part_two_link.unwrap_or(false),
        "link to part-two file missing or invalid"
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
