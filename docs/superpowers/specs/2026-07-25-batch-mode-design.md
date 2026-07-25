# kasane — Batch Mode Design Spec

**Date:** 2026-07-25
**Status:** Approved (design), pending implementation plan
**Repo:** kasane

## 1. Purpose & scope

kasane converts exactly one document per invocation today. The original design
spec (§7) specified batch mode — "a directory / glob for batch mode", `-j/--jobs`
fan-out via `rayon`, "one file's failure never aborts the batch (per-file exit
summary at the end)", and an exit code 3 for partial success — and none of it was
built. This item builds it.

Batch mode turns kasane from a one-book tool into a library converter: point it
at a directory of documents and get one navigable Markdown tree per document
under a shared root, with a library `index.md` at the top linking them all.

### Boundary

The work lives almost entirely in `crates/kasane-cli/`, which grows from a single
161-line `main.rs` into four small modules (§3). `kasane-writer` gains exactly one
public function, `write_library_index` (§5). `kasane-ir`, `kasane-core`, and
`kasane-adapters` are untouched — batch mode is I/O, concurrency, and process-exit
policy, none of which belongs in the domain core.

### Non-goals

Three other unbuilt pieces of design spec §7 are deliberately **out of scope**,
each independently shippable once this restructure lands:

- `--format` (detection override)
- `--no-assets` (text-only conversion)
- `-v` / `-q` / `--json-logs` (verbosity and machine-readable log stream)

Also out of scope: the `insta` / `proptest` / `cargo-fuzz` tiers from design spec
§9, and the deferred math/OCR follow-ups from PRs #14 and #15.

## 2. CLI surface

```
kasane <INPUT>... [options]

  <INPUT>...        one or more files and/or directories
  -o, --out <DIR>   output root
  -j, --jobs <N>    parallel workers (default: available parallelism)
```

`--force`, `--max-tokens`, `--min-tokens`, `--ocr`, `--ocr-lang`, and
`--ocr-no-image` are unchanged and apply per document across the whole run.

`Args.input: PathBuf` becomes `Args.inputs: Vec<PathBuf>` with
`#[arg(num_args = 1..)]`.

### Mode dispatch

**Keyed on invocation shape, never on what a directory walk happens to find**, so
the output layout is predictable from the command line alone:

- Exactly one positional argument, and it is a **file** → **single-file mode**.
  Behavior is unchanged byte-for-byte: `<out>` is the document root, `<out>/index.md`
  is the document, and no library index is written.
- Anything else (multiple positional arguments, or any directory argument) →
  **batch mode**. A directory containing exactly one document still gets the batch
  layout.

### Two deliberate restrictions

- **Batch mode requires `-o`.** The single-file default (`./<stem>/`) has no sane
  batch analogue; a bare `kasane books/` would scatter document directories into
  the current directory. Omitting `-o` in batch mode is a pre-flight error (exit 1)
  whose message names a suggested `-o`. Single-file mode keeps its existing default.
- **`-j 0` is a usage error**, not "unlimited"; clap validates `1..`.

The `-j` default is `std::thread::available_parallelism()`, falling back to 1 if the
platform cannot report it.

## 3. Modules

`crates/kasane-cli/src/` becomes four modules. The load-bearing property is that
**single-file and batch share one conversion path** — there is no second
implementation to drift.

```rust
// discover.rs — pure path logic; no file contents are read. Batch mode only:
// single-file mode constructs its one WorkItem directly (see below).
struct WorkItem {
    input: PathBuf,    // file to read
    out_dir: PathBuf,  // <out>/<path relative to its root, extension dropped>
    rel: String,       // that relative path, for the index and the summary
}
fn discover(inputs: &[PathBuf], out: &Path) -> Result<Vec<WorkItem>>

// convert.rs — one document, start to finish; returns, never exits
struct ConvertOptions {
    max_tokens: usize, min_tokens: usize, force: bool,
    ocr: bool, ocr_lang: String, ocr_no_image: bool,
}
struct Converted { title: String, format: String, files: usize }
fn convert_one(item: &WorkItem, opts: &ConvertOptions) -> Result<Converted>

// batch.rs
struct Outcome { item: WorkItem, result: Result<Converted> }
fn run_batch(items: Vec<WorkItem>, jobs: usize, opts: &ConvertOptions) -> Vec<Outcome>

// main.rs — arg parsing, mode dispatch, summary, exit codes
```

`convert_one` is the existing body of `run()`: read → `detect` → `adapter_for` →
`parse_with` → `kasane_core::structure` → `kasane_writer::write_tree`. It returns
`Result` rather than propagating to a process exit, which is what makes per-file
failure isolation fall out for free.

Single-file mode builds one `WorkItem` (with `out_dir` = `-o`, or `./<stem>` when
omitted), calls `convert_one` directly — no thread pool, no library index — and
maps any error through the existing `exit_code_for`.

## 4. Discovery and output mapping

### Which files are considered

- **Directory arguments are walked recursively** and filtered **by extension**
  (`epub`, `pptx`, `mobi`, `azw3`, `pdf`, `djvu`, `djv`; case-insensitive).
  Anything else in the directory is silently skipped.
- **Explicitly named files bypass the filter entirely** and always attempt
  conversion, so `kasane oddly-named-file` works exactly as it does today.
- **`detect` remains authoritative.** It runs on the file's bytes inside
  `convert_one`, as it does now. The extension filter decides only *which files to
  consider*, never *how to parse them* — the untrusted-input rule is untouched.
- A **walked** file whose extension matched but whose content fails detection is a
  per-file **failure**, not a silent skip: the directory asserted it was a document.

**Why extension and not content sniffing during the walk:** `detect` distinguishes
EPUB from PPTX by opening the ZIP central directory, which lives at the *end* of the
file, so sniffing requires reading each candidate in full. Sniffing a 500-book
directory would read every byte twice — once to discover, once to convert. The walk
reads `read_dir` metadata only.

### Walk details

Recursive `std::fs::read_dir`. Entries are **sorted by file name within each
directory**, so the work list, library index, and summary are deterministic.
**Symlinks are not followed** — no cycle detection needed, and the walk cannot
escape its root. Dotfiles get no special case.

### Output mapping

Every input carries the root it came from:

- an explicitly named file is its own root, so it maps to its **stem**
- a directory argument is the root for everything beneath it

Output directory = `<out>/<path relative to that root, extension dropped>`.

```
kasane books/ -o out/

  books/a/ch.epub  ->  out/a/ch/index.md
  books/b/ch.epub  ->  out/b/ch/index.md
  books/top.pdf    ->  out/top/index.md

kasane x/ch.epub y/ch.epub -o out/     # explicit files: stem only

  x/ch.epub  ->  out/ch/   \  duplicate destination:
  y/ch.epub  ->  out/ch/   /  pre-flight error, nothing written
```

Structure is preserved, so duplicate stems in different subdirectories cannot
collide. The **full mapping is built and checked for duplicate destinations before
any conversion starts**, so a long run cannot die halfway through on a name clash.
A collision is a pre-flight error naming both inputs.

### `--force`

The non-empty check applies **once to `<out>`**, up front, before any work. Per-
document directories are then created fresh, and `force` is still passed through to
`write_tree`, so re-running over an existing tree behaves exactly as it does today.

## 5. Library index

Batch mode writes `<out>/index.md`: the entry point an agent lands on before
drilling into any single document. Because it is Markdown, it is emitted by
`kasane-writer` — the only crate that generates Markdown — rather than by the CLI.
This also makes it unit-testable against a `Vec<LibraryEntry>` with no CLI, no
fixtures, and no conversion.

```rust
// kasane-writer, new public surface — one function
pub struct LibraryEntry { title: String, rel_dir: String, format: String, files: usize }
pub struct LibraryFailure { input: String, reason: String }

pub fn write_library_index(
    entries: &[LibraryEntry],
    failures: &[LibraryFailure],
    out: &Path,
) -> Result<()>;
```

```markdown
---
kind: library
documents: 12
failed: 2
---

# Converted documents

12 of 14 inputs converted.

- [Dune](a/dune/index.md) — epub, 34 files
- [SICP](b/sicp/index.md) — pdf, 112 files

## Failed

- `c/drm.azw3` — DRM-protected, unsupported
- `c/broken.pdf` — malformed input: bad xref
```

- `kind: library` is what lets an agent distinguish this index from a document
  index at a glance.
- Frontmatter values reuse the crate's existing `yaml_str` quoting helper, so a
  title containing `:` or `#` cannot break the frontmatter.
- Title comes from `DocMeta.title`, falling back to the relative directory name
  when it is empty.
- Link text is sanitized for the narrow corrupting subset (`[`, `]`, newlines).
  The repo-wide Markdown escaping policy remains a known deferred item; this does
  not attempt to solve it.
- The `## Failed` section is omitted entirely when nothing failed.
- Entries and failures are rendered in **input order**, not completion order.
- `LibraryEntry.format` is `DocMeta.source_format`; `LibraryEntry.files` is
  `site.files.len()`.
- `LibraryFailure.input` is the failing input's path relative to its root, extension
  kept (what the user would recognize — `c/drm.azw3`, not `c/drm`);
  `LibraryFailure.reason` is `format!("{e:#}")` of the `anyhow::Error`.
- The index is written **even when every document failed** (entries empty, every
  input listed under `## Failed`), so a failed run leaves an on-disk record rather
  than only a stderr trace.

## 6. Concurrency

```rust
rayon::ThreadPoolBuilder::new()
    .num_threads(jobs)
    .build()?
    .install(|| items.par_iter().map(|it| convert_one(it, opts)).collect())
```

`par_iter().collect()` preserves input order into the results `Vec`, so the library
index and the final summary are deterministic regardless of which document finishes
first. `rayon` is a new dependency (`rayon`, `rayon-core`, `crossbeam-deque`,
`crossbeam-epoch`; `crossbeam-utils` is already in the lock file) — the fan-out
mechanism design spec §7 named, and preferable to hand-rolling a worker pool and
result-ordering scheme.

### Thread safety, and OCR

`ConvertOptions` is plain data (`usize` / `bool` / `String`) and so is `Send + Sync`
by construction. `anyhow::Error` is already `Send + Sync`, so failures ride back out
of the pool unchanged.

`ParseOptions.ocr` is an `Option<&dyn TextExtractor>` with no `Send`/`Sync` bound,
and `LepTess` is not shareable across threads. Rather than adding bounds or
serializing OCR behind a mutex, **each worker constructs its own
`TesseractExtractor`** inside `convert_one` from `opts.ocr_lang`. Nothing crosses a
thread, and `TextExtractor`'s bounds are unchanged. This also builds one engine per
*file* instead of the current one per *page*, partly closing the "LepTess re-init
per call" follow-up from PR #14.

**Preserved behavior:** today a bad `--ocr-lang` fails fast at exit 2 because the
CLI constructs the extractor once before converting anything. If construction only
happened inside workers, that would degrade into N identical per-file failures at
exit 3. So `main` constructs and drops **one** extractor up front purely to validate
the language, then each worker builds its own. The non-`ocr` build's `--ocr`
rejection (`ensure_ocr_available`) stays exactly where it is.

### Memory

N workers hold N documents in memory simultaneously (file bytes + IR + assets), so a
directory of large PDFs at a high `-j` can use substantial RAM. `-j 1` is the
mitigation. Documented as an honest limitation; no streaming or memory cap in this
item.

## 7. Error handling and exit codes

`convert_one` returns `Result`; `run_batch` collects every outcome; nothing aborts
the run.

| Outcome | Exit code |
|---|---|
| Single-file mode | unchanged: 0, or `exit_code_for` → 1 / 2 |
| Batch: every document converted | 0 |
| Batch: some converted, some failed | 3 |
| Batch: every document failed | 2 if *every* failure maps to 2 (unsupported / DRM / encrypted), else 1 |
| No documents discovered | 1, message `no supported documents found in books/` |
| Pre-flight error (missing `-o` in batch, duplicate destinations) | 1 |

A caller can therefore distinguish "mostly worked" (3) from "nothing worked" (1/2)
without parsing output, and a one-file batch behaves like the single-file path.

**A known wart, documented rather than fixed:** clap already exits **2** on
argument-parse errors, and 2 also means unsupported/DRM. That overload exists today;
renumbering would break anyone scripting against kasane's exit codes. Pre-flight
checks introduced here use 1 to stay clear of it.

### Console output

- A per-file line on stderr as each document finishes — **completion order**, so
  non-deterministic under `-j`.
- A final summary in **input order**: `converted 12 of 14 documents to out/`,
  followed by the failures, so they are not lost in scrollback.
- Single-file mode keeps its existing `wrote N files to <dir>`.

Tests assert against the summary and the library index, never the streaming lines.

## 8. Testing

Matching the repo's existing tiers (unit + e2e). The `insta` / `proptest` /
`cargo-fuzz` tiers from design spec §9 are a separate item.

**Unit — `discover`** (pure; no conversion, no file contents):

- relative-path preservation under a nested root
- stem-only mapping for explicitly named files
- extension filter accepts each supported extension and rejects others
- explicitly named files bypass the filter
- duplicate destinations detected before any work
- per-directory sort ordering is stable
- symlinked directories are not followed

**Unit — `write_library_index`:**

- entries only → no `## Failed` section
- entries + failures
- a title requiring YAML quoting (`:` / `#`)
- a title containing `]`, sanitized in link text
- an empty title falls back to the relative directory name

**Unit — exit-code policy:** table-driven over outcome counts — all-ok, partial,
all-failed-mixed, all-failed-DRM, none-found.

**E2E** in `crates/kasane-cli/tests/e2e.rs`, building temp trees from the existing
committed fixtures — **no new fixtures are committed**:

- nested directory holding an EPUB and a PDF → exit 0, `out/index.md` is a library
  index, documents land at their relative paths
- the same plus a junk `.pdf` → exit 3; the failure appears in both the index and
  the summary; the good documents still convert
- a directory of only junk documents → exit 1
- a directory with no matching extensions → exit 1 with the "no supported documents"
  message
- two explicitly named files sharing a stem → exit 1, nothing written
- `-j 1` and `-j 4` over the same input produce identical trees
- **the existing single-file tests pass unchanged** — the regression guard on the
  `Vec<PathBuf>` change

**OCR gate** (`mise run test-ocr`): one cheap test that batch + `--ocr-lang zzz`
fails fast at exit 2 *before* converting anything, pinning the up-front language
validation from §6.

Every change ships green under `mise run lint && mise run test` (fmt check +
`clippy --workspace --all-targets -D warnings`), per the repo convention.

## 9. File-change summary

- `crates/kasane-cli/src/main.rs` — `inputs: Vec<PathBuf>`, `-j/--jobs`, mode
  dispatch, summary rendering, exit-code policy; conversion body moves out.
- `crates/kasane-cli/src/discover.rs` — new; `WorkItem`, recursive walk, extension
  filter, output mapping, collision detection.
- `crates/kasane-cli/src/convert.rs` — new; `ConvertOptions`, `Converted`,
  `convert_one` (the former `run()` body), per-worker OCR extractor construction.
- `crates/kasane-cli/src/batch.rs` — new; `Outcome`, `run_batch`, the rayon pool.
- `crates/kasane-cli/Cargo.toml` — `rayon` dependency.
- `crates/kasane-writer/src/library.rs` — new; `LibraryEntry`, `LibraryFailure`,
  `write_library_index`, reusing `yaml_str`.
- `crates/kasane-writer/src/lib.rs` — re-export the library-index surface.
- `crates/kasane-cli/tests/e2e.rs` — batch e2e cases above.
- `README.md` — batch usage, the `-o` requirement, exit-code table, the memory
  limitation.
- `AGENTS.md` — codebase map entry for the new CLI modules and the writer's
  library-index surface.
