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
