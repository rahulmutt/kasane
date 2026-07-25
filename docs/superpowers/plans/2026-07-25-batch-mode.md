# Batch Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let `kasane` convert many documents in one run — point it at directories and/or files and get one Markdown tree per document under a shared output root, plus a library `index.md` linking them all.

**Architecture:** `crates/kasane-cli/src/main.rs` (currently one 161-line file with the whole pipeline inline in `run()`) splits into four modules: `convert` (one document, returns `Result`, never exits), `discover` (path expansion → work list), `batch` (rayon fan-out + per-file outcomes), and `main` (args, mode dispatch, summary, exit codes). `kasane-writer` gains one public function, `write_library_index`. `kasane-ir`, `kasane-core`, and `kasane-adapters` are untouched.

**Tech Stack:** Rust 2021, clap 4 (derive), anyhow, rayon (new), tempfile (dev).

**Spec:** `docs/superpowers/specs/2026-07-25-batch-mode-design.md`

## Global Constraints

- Every task ships green under **`mise run lint && mise run test`** — that is `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`. Clippy runs with `--all-targets`, so warnings in test code fail the build too.
- MSRV / toolchain: **rust-version 1.97** (workspace `Cargo.toml`), pinned to **1.97.1** in `mise.toml`.
- The default build stays **pure Rust**. `rayon` is a normal Rust dependency; nothing here adds a C dependency. OCR code stays behind `#[cfg(feature = "ocr")]`.
- Tasks touching OCR must also pass the OCR gate: **`mise run lint-ocr && mise run test-ocr`** (needs Tesseract + Leptonica + `eng` traineddata). Tasks 1 and 5 touch OCR paths.
- Adapters must never trust input; `detect` stays authoritative and runs on file bytes. The extension filter added in Task 2 decides only *which files to consider*, never how to parse them.
- No new committed fixtures. Tests build temp trees from the existing `tests/fixtures/`.
- Conventional-commit style messages, matching the existing history (`feat(cli):`, `test(cli):`, `docs(batch):`).

## Two deliberate deviations from the spec

Both are improvements found while writing this plan. Implement the plan, not the spec, where they differ:

1. **`WorkItem` lives in `convert.rs`, not `discover.rs`** (spec §9 sketched it in `discover.rs`). It is `convert_one`'s input type, and `discover` is built one task later — putting it with its consumer means no task ever leaves dead code behind, which matters because clippy runs with `-D warnings`.
2. **No `yaml_str` in the library index.** Spec §5 says the library-index frontmatter reuses the writer's `yaml_str` quoting helper so a title containing `:` or `#` cannot break YAML. It turns out the frontmatter holds only `kind: library` and two integers — no title ever reaches YAML — so the helper is not needed and `frontmatter::yaml_str` stays private. Title safety is handled by `link_text` in the body, where titles actually appear.

## File Structure

| File | Responsibility |
|---|---|
| `crates/kasane-cli/src/convert.rs` | **new** — `WorkItem`, `ConvertOptions`, `Converted`, `convert_one`. One document end to end. Returns `Result`; never exits. |
| `crates/kasane-cli/src/discover.rs` | **new** — recursive walk, extension filter, output mapping, duplicate-destination detection. Pure path logic; reads no file contents. |
| `crates/kasane-cli/src/batch.rs` | **new** — `Outcome`, `run_batch`: the rayon pool and per-file result collection. |
| `crates/kasane-cli/src/main.rs` | **modified** — arg parsing, mode dispatch, run summary, exit-code policy. Shrinks to wiring. |
| `crates/kasane-writer/src/library.rs` | **new** — `LibraryEntry`, `LibraryFailure`, `write_library_index`. |
| `crates/kasane-writer/src/lib.rs` | **modified** — declare and re-export the library-index surface. |
| `crates/kasane-cli/tests/e2e.rs` | **modified** — batch end-to-end cases. |
| `crates/kasane-cli/Cargo.toml` | **modified** — `rayon` dependency. |
| `README.md`, `AGENTS.md` | **modified** — batch usage, exit-code table, codebase map. |

## Task Order

1. Extract `convert_one` — no behavior change, single-file mode uses it
2. `discover` — walk, filter, map, collision-check (pure, unit-tested)
3. Batch wiring — `Vec<PathBuf>` inputs, `-j`, mode dispatch, rayon pool
4. Exit-code policy and run summary
5. Library index — writer function plus wiring
6. Documentation

---

### Task 1: Extract `convert_one` into `convert.rs`

Pure refactor: move the body of `run()` into a reusable function that returns `Result` instead of driving the process exit. Single-file behavior must not change. This is what later tasks call once per document.

**Files:**
- Create: `crates/kasane-cli/src/convert.rs`
- Modify: `crates/kasane-cli/src/main.rs`

**Interfaces:**
- Consumes: nothing (first task).
- Produces:
  - `pub struct WorkItem { pub input: PathBuf, pub out_dir: PathBuf, pub rel: String }`
  - `impl WorkItem { pub fn rel_dir(&self) -> String }`
  - `pub struct ConvertOptions { pub max_tokens: usize, pub min_tokens: usize, pub force: bool, pub ocr: bool, pub ocr_lang: String, pub ocr_no_image: bool }`
  - `pub struct Converted { pub title: String, pub format: String, pub files: usize }`
  - `pub fn convert_one(item: &WorkItem, opts: &ConvertOptions) -> anyhow::Result<Converted>`

- [ ] **Step 1: Write the failing test**

Create `crates/kasane-cli/src/convert.rs` containing **only** this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(rel)
    }

    fn opts() -> ConvertOptions {
        ConvertOptions {
            max_tokens: 2000,
            min_tokens: 200,
            force: false,
            ocr: false,
            ocr_lang: "eng".into(),
            ocr_no_image: false,
        }
    }

    #[test]
    fn converts_one_document_and_reports_its_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let out_dir = dir.path().join("book");
        let item = WorkItem {
            input: fixture("epub/minimal.epub"),
            out_dir: out_dir.clone(),
            rel: "minimal.epub".into(),
        };

        let done = convert_one(&item, &opts()).unwrap();

        assert_eq!(done.title, "Minimal Book");
        assert_eq!(done.format, "epub");
        assert!(done.files > 0, "expected at least one emitted file");
        assert!(out_dir.join("index.md").exists());
    }

    #[test]
    fn a_drm_document_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let item = WorkItem {
            input: fixture("mobi/minimal-drm.mobi"),
            out_dir: dir.path().join("out"),
            rel: "minimal-drm.mobi".into(),
        };

        let err = convert_one(&item, &opts()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("DRM"), "expected a DRM error, got: {msg}");
    }

    #[test]
    fn rel_dir_drops_the_extension() {
        let item = WorkItem {
            input: "x/a/ch.epub".into(),
            out_dir: "out/a/ch".into(),
            rel: "a/ch.epub".into(),
        };
        assert_eq!(item.rel_dir(), "a/ch");
    }
}
```

Add `mod convert;` as the first line of `crates/kasane-cli/src/main.rs`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p kasane-cli`
Expected: FAIL — compile errors, `cannot find type WorkItem in this scope` (and the same for `ConvertOptions`, `Converted`, `convert_one`).

- [ ] **Step 3: Write the implementation**

Prepend to `crates/kasane-cli/src/convert.rs`, above the test module:

```rust
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// One document to convert: where it is read from and where its tree goes.
pub struct WorkItem {
    /// File to read.
    pub input: PathBuf,
    /// Output root for this document's own tree.
    pub out_dir: PathBuf,
    /// `input` relative to the root it was discovered under, extension kept.
    /// Shown in the run summary and in the library index's failure list.
    pub rel: String,
}

impl WorkItem {
    /// `rel` with its extension dropped: the document's directory beneath the
    /// output root, and the link target used in the library index.
    pub fn rel_dir(&self) -> String {
        Path::new(&self.rel)
            .with_extension("")
            .to_string_lossy()
            .into_owned()
    }
}

/// Per-run conversion settings, identical for every document. Plain data, so it
/// is `Send + Sync` and can be shared across workers by reference.
pub struct ConvertOptions {
    pub max_tokens: usize,
    pub min_tokens: usize,
    pub force: bool,
    /// Only read on `-F ocr` builds; a non-ocr build rejects `--ocr` in `main`.
    #[cfg_attr(not(feature = "ocr"), allow(dead_code))]
    pub ocr: bool,
    pub ocr_lang: String,
    pub ocr_no_image: bool,
}

/// What a successful conversion produced, for the summary and library index.
pub struct Converted {
    pub title: String,
    pub format: String,
    pub files: usize,
}

/// Convert exactly one document. Returns `Err` rather than exiting, which is
/// what lets a batch run isolate one file's failure from the rest.
pub fn convert_one(item: &WorkItem, opts: &ConvertOptions) -> Result<Converted> {
    let bytes = std::fs::read(&item.input)
        .with_context(|| format!("reading {}", item.input.display()))?;
    let ext = item.input.extension().and_then(|s| s.to_str());
    let fmt = kasane_adapters::detect(&bytes, ext).context("unsupported or unrecognized format")?;
    let adapter = kasane_adapters::adapter_for(fmt).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Each call builds its own extractor, so nothing non-`Send` is shared when
    // this runs on a worker thread. `main` has already validated the language.
    #[cfg(feature = "ocr")]
    let extractor = if opts.ocr {
        Some(
            kasane_adapters::ocr::TesseractExtractor::new(&opts.ocr_lang)
                .map_err(|e| anyhow::anyhow!("{e}"))?,
        )
    } else {
        None
    };

    let ocr_opts = kasane_adapters::ocr::OcrOptions {
        lang: opts.ocr_lang.clone(),
        force_text: opts.ocr_no_image,
        ..Default::default()
    };

    #[cfg(feature = "ocr")]
    let parse_opts = kasane_adapters::ParseOptions {
        ocr: extractor
            .as_ref()
            .map(|e| e as &dyn kasane_adapters::ocr::TextExtractor),
        ocr_opts,
    };
    #[cfg(not(feature = "ocr"))]
    let parse_opts = kasane_adapters::ParseOptions {
        ocr: None,
        ocr_opts,
    };

    let (doc, assets) = adapter
        .parse_with(&bytes, &item.input.to_string_lossy(), &parse_opts)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // `structure` consumes `doc`, so capture the metadata first.
    let title = doc.meta.title.clone();
    let format = doc.meta.source_format.clone();

    let core_opts = kasane_core::Options {
        max_tokens: opts.max_tokens,
        min_tokens: opts.min_tokens,
    };
    let site = kasane_core::structure(doc, &core_opts);
    let files = site.files.len();

    kasane_writer::write_tree(&site, &assets, &item.out_dir, opts.force)?;

    Ok(Converted {
        title,
        format,
        files,
    })
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p kasane-cli`
Expected: PASS — the three new unit tests plus the existing `e2e` tests.

- [ ] **Step 5: Rewrite `run()` to use `convert_one`**

Replace the body of `run()` in `crates/kasane-cli/src/main.rs` (lines 71–134) with:

```rust
fn run() -> Result<()> {
    let args = Args::parse();
    ensure_ocr_available(args.ocr)?;
    validate_ocr_lang(args.ocr, &args.ocr_lang)?;

    let out = args.out.clone().unwrap_or_else(|| {
        PathBuf::from(
            args.input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("out"),
        )
    });
    if out.as_os_str().is_empty() {
        bail!("could not determine output directory");
    }

    let item = WorkItem {
        rel: args
            .input
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        input: args.input.clone(),
        out_dir: out.clone(),
    };
    let opts = ConvertOptions {
        max_tokens: args.max_tokens,
        min_tokens: args.min_tokens,
        force: args.force,
        ocr: args.ocr,
        ocr_lang: args.ocr_lang.clone(),
        ocr_no_image: args.ocr_no_image,
    };

    let done = convert_one(&item, &opts)?;
    eprintln!("wrote {} files to {}", done.files, out.display());
    Ok(())
}
```

Add the import near the top of `main.rs`, after `mod convert;`:

```rust
use convert::{convert_one, ConvertOptions, WorkItem};
```

Keep `use std::process::ExitCode;` — `main()` still uses it. Drop `Context` from the anyhow import, since it moved to `convert.rs`: the line becomes `use anyhow::{bail, Result};`. (Task 3 puts it back.)

- [ ] **Step 6: Add the up-front OCR language validation**

Add below `ensure_ocr_available` in `main.rs`. This preserves today's fail-fast behavior: without it, a bad `--ocr-lang` would become one identical failure per document once batch mode exists.

```rust
/// Construct and drop one extractor up front so a bad `--ocr-lang` fails before
/// any document is converted, instead of failing once inside every worker.
#[cfg(feature = "ocr")]
fn validate_ocr_lang(ocr_requested: bool, lang: &str) -> Result<()> {
    if ocr_requested {
        kasane_adapters::ocr::TesseractExtractor::new(lang)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    Ok(())
}

#[cfg(not(feature = "ocr"))]
fn validate_ocr_lang(_ocr_requested: bool, _lang: &str) -> Result<()> {
    Ok(())
}
```

- [ ] **Step 7: Verify the refactor changed no behavior**

Run: `cargo test -p kasane-cli`
Expected: PASS, including every pre-existing `e2e` test unchanged.

Run: `mise run lint`
Expected: clean.

- [ ] **Step 8: Verify the OCR build**

Run: `mise run lint-ocr && mise run test-ocr`
Expected: clean and passing. (Skip only if Tesseract + Leptonica are unavailable in your environment; say so explicitly in the task report rather than claiming it passed.)

- [ ] **Step 9: Commit**

```bash
git add crates/kasane-cli/src/convert.rs crates/kasane-cli/src/main.rs
git commit -m "refactor(cli): extract convert_one so one document is a reusable unit

Moves the pipeline body out of run() into convert.rs behind
convert_one(&WorkItem, &ConvertOptions) -> Result<Converted>. Returning
a Result instead of driving the process exit is what will let batch mode
isolate a single file's failure. No behavior change.

Keeps the --ocr-lang validation in main so it still fails fast once per
run rather than once per document."
```

---

### Task 2: `discover` — walk, filter, map, collision-check

Pure path logic, fully unit-testable with no conversion. Not yet wired into `main` — it is consumed in Task 3, so this task's tests are what keep it alive.

**Files:**
- Create: `crates/kasane-cli/src/discover.rs`
- Modify: `crates/kasane-cli/src/main.rs` (add `mod discover;`)

**Interfaces:**
- Consumes: `WorkItem` from Task 1 (`crate::convert::WorkItem`).
- Produces: `pub fn discover(inputs: &[PathBuf], out: &Path) -> anyhow::Result<Vec<WorkItem>>`

> **Dead-code note:** until Task 3 calls it, `discover` is used only by its own tests, and clippy in a binary crate *will* warn `function is never used`. Put `#[allow(dead_code)] // wired into main in the next task` on the `discover` function itself, and **delete that attribute in Task 3**. Never add a crate-level allow.

- [ ] **Step 1: Write the failing tests**

Create `crates/kasane-cli/src/discover.rs` with **only** this test module:

```rust
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

        let items = discover(&[odd.clone()], &dir.path().join("out")).unwrap();
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
        assert!(!rels(&items).iter().any(|r| r.ends_with(".txt") || r.ends_with(".zip")));
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
        assert!(msg.contains("a/ch.epub") && msg.contains("b/ch.epub"), "got: {msg}");
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-cli discover`
Expected: FAIL — `cannot find function discover in this scope`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/kasane-cli/src/discover.rs`:

```rust
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
```

Add `mod discover;` to `main.rs` beneath `mod convert;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kasane-cli discover`
Expected: PASS — all nine tests.

- [ ] **Step 5: Lint**

Run: `mise run lint`
Expected: clean. If clippy reports `function discover is never used`, apply the `#[allow(dead_code)]` described in the dead-code note above, with its `// wired into main in the next task` comment.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-cli/src/discover.rs crates/kasane-cli/src/main.rs
git commit -m "feat(cli): discover the work list from files and directories

Recursive walk with a case-insensitive extension filter, output dirs
mirroring each input's path relative to its root, and duplicate
destinations rejected up front. Symlinks are not followed.

The filter picks candidates only; detect() still runs on the bytes in
convert_one, so extensions are never trusted for parsing."
```

---

### Task 3: Batch wiring — multi-input args, `-j`, mode dispatch, rayon

Batch mode starts working end to end. Exit codes stay crude here (0 on full success, 1 otherwise); Task 4 makes them precise.

**Files:**
- Create: `crates/kasane-cli/src/batch.rs`
- Modify: `crates/kasane-cli/src/main.rs`, `crates/kasane-cli/Cargo.toml`
- Test: `crates/kasane-cli/tests/e2e.rs`

**Interfaces:**
- Consumes: `convert_one`, `ConvertOptions`, `Converted`, `WorkItem` (Task 1); `discover` (Task 2).
- Produces:
  - `pub struct Outcome { pub item: WorkItem, pub result: anyhow::Result<Converted> }`
  - `pub fn run_batch(items: Vec<WorkItem>, jobs: usize, opts: &ConvertOptions) -> anyhow::Result<Vec<Outcome>>` — returns `Err` only if the thread pool cannot be built; per-document failures live in each `Outcome.result`.

- [ ] **Step 1: Add the rayon dependency**

In `crates/kasane-cli/Cargo.toml`, under `[dependencies]`, after `anyhow = "1"`:

```toml
rayon = "1"
```

Run: `cargo build -p kasane-cli`
Expected: builds; `Cargo.lock` picks up `rayon`, `rayon-core`, `crossbeam-deque`, `crossbeam-epoch`.

- [ ] **Step 2: Write the failing e2e test**

Append to `crates/kasane-cli/tests/e2e.rs`:

```rust
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
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().into_owned();
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
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p kasane-cli --test e2e`
Expected: FAIL — `converts_a_directory_of_documents` fails because a directory input is currently read as a file (`reading .../books: Is a directory`), and `-j` is an unknown argument.

- [ ] **Step 4: Write `batch.rs`**

Create `crates/kasane-cli/src/batch.rs`:

```rust
use crate::convert::{convert_one, ConvertOptions, Converted, WorkItem};
use anyhow::{Context, Result};
use rayon::prelude::*;

/// One document's fate in a batch run.
pub struct Outcome {
    pub item: WorkItem,
    pub result: Result<Converted>,
}

/// Convert every item across `jobs` workers.
///
/// `into_par_iter().collect()` preserves input order, so the summary and the
/// library index are deterministic no matter which document finishes first.
/// Per-document failures are carried in each `Outcome`; the `Result` here is
/// only for a thread pool that cannot be built.
pub fn run_batch(
    items: Vec<WorkItem>,
    jobs: usize,
    opts: &ConvertOptions,
) -> Result<Vec<Outcome>> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .context("create worker thread pool")?;

    Ok(pool.install(|| {
        items
            .into_par_iter()
            .map(|item| {
                let result = convert_one(&item, opts);
                // Printed as each document finishes, so this is completion
                // order; the end-of-run summary is in input order.
                match &result {
                    Ok(done) => eprintln!("  {} -> {} ({} files)", item.rel, item.out_dir.display(), done.files),
                    Err(e) => eprintln!("  {} FAILED: {e:#}", item.rel),
                }
                Outcome { item, result }
            })
            .collect()
    }))
}
```

- [ ] **Step 5: Rewrite the argument struct and dispatch in `main.rs`**

Replace the `input` field in `struct Args` with:

```rust
    /// Input documents and/or directories to convert
    #[arg(required = true, num_args = 1..)]
    inputs: Vec<PathBuf>,
```

Add, after `force`:

```rust
    /// Parallel workers for batch mode (default: available parallelism)
    #[arg(short = 'j', long)]
    jobs: Option<std::num::NonZeroUsize>,
```

`NonZeroUsize` is what makes `-j 0` a clap usage error rather than "unlimited".

Add `mod batch;` beneath `mod discover;`, extend the imports:

```rust
use batch::run_batch;
use convert::{convert_one, ConvertOptions, WorkItem};
use discover::discover;
```

(`Outcome` is reached only through `run_batch`'s return type in this task; Task 4 adds it to this import when it uses the type by name.)

Replace `run()` with:

```rust
/// A single positional argument that is not a directory means single-file mode.
/// Keying on the argument rather than on what a walk finds keeps the output
/// shape predictable from the command line alone — and a nonexistent path stays
/// in single-file mode, so it still reports "reading <path>" as it does today.
fn is_dir(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

fn run() -> Result<()> {
    let args = Args::parse();
    ensure_ocr_available(args.ocr)?;
    validate_ocr_lang(args.ocr, &args.ocr_lang)?;

    let opts = ConvertOptions {
        max_tokens: args.max_tokens,
        min_tokens: args.min_tokens,
        force: args.force,
        ocr: args.ocr,
        ocr_lang: args.ocr_lang.clone(),
        ocr_no_image: args.ocr_no_image,
    };

    if args.inputs.len() == 1 && !is_dir(&args.inputs[0]) {
        return run_single(&args.inputs[0], &args, &opts);
    }
    run_many(&args, &opts)
}

fn run_single(input: &Path, args: &Args, opts: &ConvertOptions) -> Result<()> {
    let out = args.out.clone().unwrap_or_else(|| {
        PathBuf::from(input.file_stem().and_then(|s| s.to_str()).unwrap_or("out"))
    });
    if out.as_os_str().is_empty() {
        bail!("could not determine output directory");
    }

    let item = WorkItem {
        rel: input
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        input: input.to_path_buf(),
        out_dir: out.clone(),
    };
    let done = convert_one(&item, opts)?;
    eprintln!("wrote {} files to {}", done.files, out.display());
    Ok(())
}

fn run_many(args: &Args, opts: &ConvertOptions) -> Result<()> {
    let Some(out) = args.out.clone() else {
        bail!("converting more than one document requires an output root: add `-o <DIR>`");
    };

    // Spec §4: the non-empty check applies once to the output root, up front.
    // Per-document directories are created fresh below, and `force` is still
    // passed through to write_tree so a re-run behaves as it does today.
    if !args.force && out.exists() {
        let non_empty = out
            .read_dir()
            .with_context(|| format!("inspect output directory {}", out.display()))?
            .next()
            .is_some();
        if non_empty {
            bail!(
                "output directory {} is not empty (use --force)",
                out.display()
            );
        }
    }

    let items = discover(&args.inputs, &out)?;
    if items.is_empty() {
        bail!("no supported documents found");
    }

    let jobs = args
        .jobs
        .map(|n| n.get())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1));

    let outcomes = run_batch(items, jobs, opts)?;
    let failures = outcomes.iter().filter(|o| o.result.is_err()).count();
    if failures > 0 {
        bail!("{failures} of {} documents failed", outcomes.len());
    }
    eprintln!("converted {} documents to {}", outcomes.len(), out.display());
    Ok(())
}
```

Add `use std::path::Path;` to the imports (alongside `PathBuf`), and put `Context` back into the anyhow import — `run_many`'s non-empty check uses `with_context`, so the line becomes `use anyhow::{bail, Context, Result};` again. Delete the `#[allow(dead_code)]` Task 2 put on `discover`.

> `Outcome` is imported but only used through `run_batch`'s return type here; Task 4 uses it directly. If clippy flags the unused import, drop `Outcome` from the `use batch::{...}` line now and re-add it in Task 4.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p kasane-cli`
Expected: PASS — the four new e2e tests, the Task 1 and Task 2 unit tests, and every pre-existing e2e test.

- [ ] **Step 7: Lint**

Run: `mise run lint`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-cli/src/batch.rs crates/kasane-cli/src/main.rs \
        crates/kasane-cli/Cargo.toml crates/kasane-cli/tests/e2e.rs Cargo.lock
git commit -m "feat(cli): batch mode with rayon fan-out and -j/--jobs

One or more files and/or directories now convert in a single run, each
document landing at its path relative to its root under -o. Mode is keyed
on the invocation shape, so a lone file argument keeps today's output
layout byte-for-byte.

par_iter preserves input order, so -j does not change the emitted tree.
Exit codes stay crude here; the next task makes them precise."
```

---

### Task 4: Exit-code policy and run summary

Turn "0 or 1" into the spec's policy: 3 for partial success, 2 when every failure is unsupported/DRM/encrypted, 1 otherwise — plus an end-of-run summary in input order.

**Files:**
- Modify: `crates/kasane-cli/src/main.rs`
- Test: `crates/kasane-cli/tests/e2e.rs`

**Interfaces:**
- Consumes: `Outcome` (Task 3), `exit_code_for` (existing, `main.rs:38`).
- Produces:
  - `fn batch_exit_code(total: usize, failure_msgs: &[String]) -> u8`
  - `fn run() -> Result<u8>` — **replaces** Task 3's `fn run() -> Result<()>`; `main()` maps the `Ok` value straight to `ExitCode`.

- [ ] **Step 1: Write the failing unit tests**

Add to the `mod tests` block at the bottom of `crates/kasane-cli/src/main.rs`:

```rust
    #[test]
    fn batch_exit_codes_follow_the_outcome() {
        let drm = "DRM-protected content is not supported".to_string();
        let broken = "malformed input: bad xref".to_string();

        // everything converted
        assert_eq!(batch_exit_code(3, &[]), 0);
        // partial success
        assert_eq!(batch_exit_code(3, &[broken.clone()]), 3);
        assert_eq!(batch_exit_code(3, &[drm.clone(), broken.clone()]), 3);
        // every document failed, all of them unsupported/DRM/encrypted
        assert_eq!(batch_exit_code(2, &[drm.clone(), drm.clone()]), 2);
        // every document failed, mixed reasons
        assert_eq!(batch_exit_code(2, &[drm, broken.clone()]), 1);
        assert_eq!(batch_exit_code(1, &[broken]), 1);
    }

    #[test]
    fn no_documents_found_is_exit_one() {
        // Guards a subtle overlap: this message contains "supported" but must
        // not match exit_code_for's "unsupported" keyword.
        assert_eq!(exit_code_for("no supported documents found"), 1);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-cli --bin kasane`
Expected: FAIL — `cannot find function batch_exit_code in this scope`.

- [ ] **Step 3: Write the implementation**

Add above `fn main()` in `main.rs`:

```rust
/// Batch exit policy (design spec §7): 0 all converted, 3 partial success,
/// 2 when every failure was unsupported/DRM/encrypted, 1 otherwise.
fn batch_exit_code(total: usize, failure_msgs: &[String]) -> u8 {
    if failure_msgs.is_empty() {
        return 0;
    }
    if failure_msgs.len() < total {
        return 3;
    }
    if failure_msgs.iter().all(|m| exit_code_for(m) == 2) {
        2
    } else {
        1
    }
}
```

Change `main()` to carry the success-path code through:

```rust
fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(exit_code_for(&format!("{e:#}")))
        }
    }
}
```

Change the three run functions' signatures to `-> Result<u8>`; `run_single` ends with `Ok(0)`, and `run` returns whatever it delegates to. Replace `run_many`'s tail (everything after `let outcomes = run_batch(...)?;`) with:

```rust
    let failure_msgs: Vec<String> = outcomes
        .iter()
        .filter_map(|o| o.result.as_ref().err().map(|e| format!("{e:#}")))
        .collect();
    let converted = outcomes.len() - failure_msgs.len();

    // Summary in input order, so it is deterministic even though the per-file
    // lines above appeared in completion order.
    eprintln!(
        "converted {converted} of {} documents to {}",
        outcomes.len(),
        out.display()
    );
    if !failure_msgs.is_empty() {
        eprintln!("failed:");
        for o in outcomes.iter().filter(|o| o.result.is_err()) {
            let e = o.result.as_ref().err().expect("filtered to failures");
            eprintln!("  {} — {e:#}", o.item.rel);
        }
    }

    Ok(batch_exit_code(outcomes.len(), &failure_msgs))
```

Also change `run_many`'s empty-work-list bail to name the inputs:

```rust
    if items.is_empty() {
        let names: Vec<String> = args
            .inputs
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        bail!("no supported documents found in {}", names.join(", "));
    }
```

- [ ] **Step 4: Run the unit tests to verify they pass**

Run: `cargo test -p kasane-cli --bin kasane`
Expected: PASS.

- [ ] **Step 5: Write the failing e2e tests**

Append to `crates/kasane-cli/tests/e2e.rs`:

```rust
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

    assert_eq!(run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]), 2);
}

#[test]
fn a_directory_with_no_documents_exits_one() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("books");
    std::fs::create_dir_all(&books).unwrap();
    std::fs::write(books.join("notes.txt"), "nothing here").unwrap();
    let out = tmp.path().join("out");

    assert_eq!(run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]), 1);
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
    assert!(!out.exists(), "collision must be caught before any conversion");
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

    assert_eq!(run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]), 3);
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

    assert_eq!(run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]), 1);
    // Nothing was converted into it.
    assert!(!out.join("a").exists());

    assert_eq!(
        run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str(), "--force".as_ref()]),
        0
    );
    assert!(out.join("a/minimal/index.md").exists());
}
```

- [ ] **Step 6: Run the e2e tests to verify they pass**

Run: `cargo test -p kasane-cli --test e2e`
Expected: PASS. If `partial_success_exits_three` returns 1, check that the DRM fixture is being walked — `.mobi` must be in `SUPPORTED_EXTS`.

- [ ] **Step 7: Full gate**

Run: `mise run lint && mise run test`
Expected: clean and green.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-cli/src/main.rs crates/kasane-cli/tests/e2e.rs
git commit -m "feat(cli): batch exit-code policy and end-of-run summary

0 when every document converted, 3 on partial success, 2 when every
failure was unsupported/DRM/encrypted, 1 otherwise. Missing -o, a
duplicate destination, and an input set holding no documents all exit 1
without writing anything.

The summary is rendered in input order even though the per-file lines
stream in completion order."
```

---

### Task 5: Library index

Give the batch output an entry point: `<out>/index.md` listing every converted document and every failure. Emitted by `kasane-writer`, the only crate that generates Markdown.

**Files:**
- Create: `crates/kasane-writer/src/library.rs`
- Modify: `crates/kasane-writer/src/lib.rs`, `crates/kasane-cli/src/main.rs`
- Test: `crates/kasane-cli/tests/e2e.rs`

**Interfaces:**
- Consumes: `Outcome` (Task 3), `Converted` (Task 1), `WorkItem::rel_dir` (Task 1).
- Produces:
  - `pub struct LibraryEntry { pub title: String, pub rel_dir: String, pub format: String, pub files: usize }`
  - `pub struct LibraryFailure { pub input: String, pub reason: String }`
  - `pub fn write_library_index(entries: &[LibraryEntry], failures: &[LibraryFailure], out: &Path) -> anyhow::Result<()>`

- [ ] **Step 1: Write the failing tests**

Create `crates/kasane-writer/src/library.rs` with **only** this test module:

```rust
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
        assert!(md.contains("- [a/untitled](a/untitled/index.md)"), "got: {md}");
    }

    #[test]
    fn link_text_cannot_break_out_of_the_label() {
        let md = write(&[entry("Bracket] and\nnewline", "a/odd")], &[]);
        assert!(md.contains("- [Bracket) and newline](a/odd/index.md)"), "got: {md}");
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
        assert!(md.contains("- `c/bad.pdf` — malformed input: bad xref"), "got: {md}");
        assert!(md.contains("0 of 1 inputs converted."));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-writer`
Expected: FAIL — `cannot find type LibraryEntry in this scope`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/kasane-writer/src/library.rs`:

```rust
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
    s.push_str(&format!("{} of {total} inputs converted.\n\n", entries.len()));

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
    s.replace('[', "(").replace(']', ")").replace(['\n', '\r'], " ")
}

/// Collapse a multi-line error message onto a single bullet line.
fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}
```

Add to `crates/kasane-writer/src/lib.rs`, beside the existing `mod` and `pub use` lines:

```rust
mod library;

pub use library::{write_library_index, LibraryEntry, LibraryFailure};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kasane-writer`
Expected: PASS — five new tests plus the three existing writer tests.

- [ ] **Step 5: Wire it into `run_many`**

In `crates/kasane-cli/src/main.rs`, insert immediately before the `Ok(batch_exit_code(...))` line:

```rust
    let entries: Vec<kasane_writer::LibraryEntry> = outcomes
        .iter()
        .filter_map(|o| {
            o.result.as_ref().ok().map(|c| kasane_writer::LibraryEntry {
                title: c.title.clone(),
                rel_dir: o.item.rel_dir(),
                format: c.format.clone(),
                files: c.files,
            })
        })
        .collect();
    let lib_failures: Vec<kasane_writer::LibraryFailure> = outcomes
        .iter()
        .filter_map(|o| {
            o.result.as_ref().err().map(|e| kasane_writer::LibraryFailure {
                input: o.item.rel.clone(),
                reason: format!("{e:#}"),
            })
        })
        .collect();
    kasane_writer::write_library_index(&entries, &lib_failures, &out)?;
```

- [ ] **Step 6: Write the failing e2e tests**

Append to `crates/kasane-cli/tests/e2e.rs`:

```rust
#[test]
fn a_batch_run_writes_a_library_index() {
    let tmp = tempfile::tempdir().unwrap();
    let books = library(tmp.path());
    let out = tmp.path().join("out");

    assert_eq!(run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]), 0);

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

    assert_eq!(run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]), 3);

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

    assert_eq!(run_kasane(&[books.as_os_str(), "-o".as_ref(), out.as_os_str()]), 2);

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
        run_kasane(&[fixture("epub/minimal.epub").as_os_str(), "-o".as_ref(), out.as_os_str()]),
        0
    );

    let idx = std::fs::read_to_string(out.join("index.md")).unwrap();
    assert!(idx.contains("title: Minimal Book"));
    assert!(!idx.contains("kind: library"));
}
```

Run: `cargo test -p kasane-cli --test e2e`
Expected: PASS once Step 5's wiring is in place. (Write the tests before the wiring if you want to see them fail first — they will fail on the missing `out/index.md`.)

- [ ] **Step 7: Add the OCR fail-fast test**

Append to `crates/kasane-cli/tests/e2e.rs`:

```rust
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
```

- [ ] **Step 8: Full gate, both builds**

Run: `mise run lint && mise run test`
Expected: clean and green.

Run: `mise run lint-ocr && mise run test-ocr`
Expected: clean and green. (If Tesseract is unavailable, say so explicitly rather than claiming a pass.)

- [ ] **Step 9: Commit**

```bash
git add crates/kasane-writer/src/library.rs crates/kasane-writer/src/lib.rs \
        crates/kasane-cli/src/main.rs crates/kasane-cli/tests/e2e.rs
git commit -m "feat(writer): library index for batch runs

<out>/index.md lists every converted document with a link into its tree
and every failure with its reason, so an agent has an entry point and a
failed run leaves an on-disk record. Written even when everything failed;
never written in single-file mode.

Markdown generation stays in kasane-writer, so the index is unit-testable
without the CLI."
```

---

### Task 6: Documentation

**Files:**
- Modify: `README.md`, `AGENTS.md`

- [ ] **Step 1: Document batch mode in `README.md`**

Under `## Quick start`, after the existing `mise run convert` lines, add:

```markdown
### Batch conversion

    kasane books/ -o out/            # every document under books/, recursively
    kasane a.epub b.pdf -o out/      # several files at once
    kasane books/ -o out/ -j 4       # 4 workers (default: all cores)

Each document lands at its path relative to the root it was found under, so
`books/a/ch.epub` becomes `out/a/ch/index.md`. `out/index.md` is a library index
linking every document and naming every failure. A single file argument is
unchanged: `-o` is that document's own root.

Directories are walked recursively and filtered by extension; symlinks are not
followed. `-o` is required whenever more than one document could be produced.
One file's failure never aborts the run.

| Exit code | Meaning |
|---|---|
| 0 | every document converted |
| 1 | nothing converted, or a usage problem (missing `-o`, duplicate destinations, no documents found) |
| 2 | every failure was an unsupported format, DRM, or encryption |
| 3 | some documents converted, some failed |
```

- [ ] **Step 2: Add the limitation to `## Known limitations (this build)`**

```markdown
- Batch mode holds one document in memory per worker, so a directory of large
  PDFs at a high `-j` can use a lot of RAM; `-j 1` is the mitigation. Two inputs
  that would produce the same output directory are rejected before any
  conversion starts rather than silently renamed.
```

- [ ] **Step 3: Update the `AGENTS.md` codebase map**

Replace the `crates/kasane-cli` line with:

```markdown
- `crates/kasane-cli`     `kasane` binary; wires the pipeline; owns exit codes.
  `convert.rs` converts one document (`WorkItem` -> `Converted`) and returns a
  `Result` rather than exiting, which is what makes per-file failure isolation
  possible; `discover.rs` expands file/directory arguments into the work list
  (recursive walk, extension filter, output mapping, duplicate-destination
  check); `batch.rs` fans out across rayon workers, preserving input order.
  Mode is keyed on the invocation shape: a lone file argument is single-file
  mode (unchanged output layout), anything else is batch mode with a library
  index at `<out>/index.md`. Each worker builds its own `TesseractExtractor`,
  so nothing non-`Send` crosses a thread; `main` validates `--ocr-lang` once up
  front so a bad language fails the run, not every document.
```

And add to the `kasane-writer` line: `Also emits the batch library index (`library.rs`).`

- [ ] **Step 4: Verify the documented commands actually work**

Run each README command against a temp directory of fixtures and confirm the described output shape. Do not skip this — a README that documents a flag spelling the CLI does not accept is the most common way this task goes wrong.

- [ ] **Step 5: Full gate**

Run: `mise run lint && mise run test`
Expected: clean and green.

- [ ] **Step 6: Commit**

```bash
git add README.md AGENTS.md
git commit -m "docs(batch): document batch mode, exit codes, and the CLI modules"
```

---

## Verification Checklist

Before opening the PR:

- [ ] `mise run lint && mise run test` green
- [ ] `mise run lint-ocr && mise run test-ocr` green (or explicitly reported as un-runnable)
- [ ] Every pre-existing e2e test still passes **unmodified** — the single-file path is untouched
- [ ] `kasane book.epub -o out/` puts the book at `out/index.md` with no library index
- [ ] `kasane books/ -o out/` writes `out/index.md` plus one tree per document
- [ ] `-j 1` and `-j 4` produce byte-identical trees
- [ ] Exit codes: 0 all-ok, 3 partial, 2 all-DRM, 1 nothing-converted / usage
- [ ] A non-empty `-o` root is refused without `--force` and accepted with it
- [ ] A run where every document failed still writes `out/index.md`
