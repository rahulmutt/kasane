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
