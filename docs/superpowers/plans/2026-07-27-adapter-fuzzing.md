# Adapter Fuzzing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add coverage-guided fuzzing over kasane's untrusted-input boundary, with every crash it finds replayable as an ordinary stable-toolchain regression test.

**Architecture:** A `#[doc(hidden)] pub mod fuzz_entry` inside `kasane-adapters` holds one `fn(&[u8])` per fuzz target. It lives inside the crate so it can reach `pub(crate)` internals (`capture_island`, `palmdoc::decompress`, `guard::*`) without widening the public API. A separate `fuzz/` cargo workspace holds twelve three-line libFuzzer wrappers that call those functions on nightly; a stable integration test replays committed corpus files through the exact same functions. One body, two callers.

**Tech Stack:** Rust, `cargo-fuzz` 0.13.2 + libFuzzer (nightly), `zip` 2.x (already a dependency, used for the structured ZIP builder), mise, GitHub Actions.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-07-27-adapter-fuzzing-design.md`. Read it before starting.
- **Branch:** `adapter-fuzzing`. Already created, already holds the spec commit.
- **Every change ships green under `mise run lint && mise run test`.** This is a standing repo rule from AGENTS.md. `mise run lint` is `cargo fmt --all -- --check` plus `cargo clippy --workspace --all-targets -- -D warnings`. Plain `cargo clippy` is not sufficient — `--all-targets` is required, because test and example targets are where warnings hide.
- **`fuzz_entry` must also compile under the `ocr` feature.** Verify with `mise run lint-ocr && mise run test-ocr` before the final commit of any task that touches it.
- **No new dependency on `kasane-adapters`.** The ZIP builder uses the existing `zip` crate; multi-argument targets carve their inputs out of the byte slice by hand. Do not add `arbitrary` to the library crate.
- **Uniform target signature:** every function in `fuzz_entry` is `pub fn <name>(data: &[u8])`. No return value, no other parameters. This is what lets the replay test dispatch by directory name and keeps the libFuzzer wrappers identical.
- **A `fuzz_entry` function must never return an error to signal a finding.** It either returns normally or panics. A panic is the finding.
- **Do not fix bugs the fuzzer finds as part of this plan.** Commit the reproducer to `fuzz/artifacts/<target>/`, note it, and move on. Fixes are follow-up work (spec §1).
- **The `ocr` feature is not fuzzed** (spec §1 non-goals). No target may call into `crate::ocr`.
- **Constants, copied verbatim from the source — do not retype from memory:**
  - `guard::MAX_TOTAL_BYTES = 512 * 1024 * 1024`
  - `guard::MAX_RATIO = 200`
  - `math::MAX_ISLAND_BYTES = 256 * 1024`
  - `math::MAX_ISLAND_NESTING = 128`
  - `math::MAX_MATH_DEPTH = 64`
- **libFuzzer run limits (spec §4.2), used identically in the mise task and CI:** `-rss_limit_mb=2048 -malloc_limit_mb=2048 -timeout=25`

## File Structure

| File | Responsibility |
|---|---|
| `crates/kasane-adapters/src/fuzz_entry.rs` | **Create.** The shared target bodies + invariant assertions. The whole seam. |
| `crates/kasane-adapters/src/lib.rs` | **Modify.** Add `#[doc(hidden)] pub mod fuzz_entry;`. |
| `crates/kasane-adapters/tests/fuzz_corpus.rs` | **Create.** Stable replay over `fuzz/seeds/**` and `fuzz/artifacts/**`. |
| `fuzz/Cargo.toml` | **Create.** Own workspace; depends on `kasane-adapters` + `libfuzzer-sys`. |
| `fuzz/fuzz_targets/*.rs` | **Create.** Twelve wrappers. |
| `fuzz/seeds/<target>/*` | **Create.** Committed starting inputs. |
| `fuzz/artifacts/.gitkeep` | **Create.** Committed crash reproducers land here. |
| `Cargo.toml` (root) | **Modify.** Add `exclude = ["fuzz"]`. |
| `.gitignore` | **Modify.** Add `/fuzz/corpus` and `/fuzz/target`. |
| `mise.toml` | **Modify.** Nightly toolchain, `cargo-fuzz`, `fuzz` and `fuzz-all` tasks. |
| `.github/workflows/fuzz.yml` | **Create.** Weekly cron + workflow_dispatch, matrix over targets. |
| `.github/dependabot.yml` | **Modify.** Third entry for `/fuzz`. |
| `README.md`, `AGENTS.md` | **Modify.** Docs (spec §9). |

Tasks 1–3 build `fuzz_entry` and are each independently testable on stable with no nightly toolchain. Task 4 makes it load-bearing in PR CI. Tasks 5–6 add the actual fuzzer and its automation. Task 7 documents it.

---

### Task 1: The `fuzz_entry` seam and the five per-format targets

**Files:**
- Create: `crates/kasane-adapters/src/fuzz_entry.rs`
- Modify: `crates/kasane-adapters/src/lib.rs` (module declaration, near the other `mod` lines at the top)
- Test: inline `#[cfg(test)] mod tests` in `crates/kasane-adapters/src/fuzz_entry.rs`

**Interfaces:**
- Consumes: `Adapter::parse(&self, bytes: &[u8], source_path: &str) -> Result<(Document, AssetBag), ParseError>`; the five adapter unit structs `EpubAdapter`, `PptxAdapter`, `MobiAdapter`, `PdfAdapter`, `DjvuAdapter`; `kasane_ir::AssetBag { items: Vec<AssetItem> }` where `AssetItem { key: String, filename: String, bytes: Vec<u8> }`.
- Produces: `pub fn epub(data: &[u8])`, `pub fn pptx(data: &[u8])`, `pub fn mobi(data: &[u8])`, `pub fn pdf(data: &[u8])`, `pub fn djvu(data: &[u8])` — all in `kasane_adapters::fuzz_entry`. Also the private helper `fn assert_assets_contained(assets: &AssetBag)`, used by Task 3.

**Context you need:** `AssetItem::filename` is the name the writer flushes into `_assets/`. It is produced by `guard::safe_media_filename`, which maps every character outside `[A-Za-z0-9._-]` to `_` and prefixes an index. So a well-behaved filename is a bare basename with no path separator at all. That is the invariant to assert — not a general "path is relative" check.

- [ ] **Step 1: Write the failing test**

Create `crates/kasane-adapters/src/fuzz_entry.rs` containing *only* the test module for now, so the test fails on missing items rather than missing file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kasane_ir::{AssetBag, AssetItem};

    fn bag(filename: &str) -> AssetBag {
        AssetBag {
            items: vec![AssetItem {
                key: "k".into(),
                filename: filename.into(),
                bytes: vec![],
            }],
        }
    }

    #[test]
    fn assets_containment_accepts_a_sanitized_basename() {
        assert_assets_contained(&bag("001-image.png"));
    }

    #[test]
    #[should_panic(expected = "path separator")]
    fn assets_containment_rejects_a_separator() {
        assert_assets_contained(&bag("../../etc/passwd"));
    }

    #[test]
    #[should_panic(expected = "traversal")]
    fn assets_containment_rejects_dotdot() {
        assert_assets_contained(&bag(".."));
    }

    #[test]
    fn every_fixture_survives_its_adapter() {
        let cases: &[(&str, fn(&[u8]))] = &[
            ("epub/minimal.epub", epub as fn(&[u8])),
            ("epub/rich.epub", epub),
            ("pptx/minimal.pptx", pptx),
            ("mobi/minimal.mobi", mobi),
            ("mobi/minimal-drm.mobi", mobi),
            ("azw3/minimal.azw3", mobi),
            ("azw3/lying-skel.azw3", mobi),
            ("pdf/minimal.pdf", pdf),
            ("pdf/no-outline.pdf", pdf),
            ("pdf/image.pdf", pdf),
            ("pdf/scanned.pdf", pdf),
            ("djvu/sample.djvu", djvu),
            ("djvu/scanned.djvu", djvu),
        ];
        for (rel, f) in cases {
            let path = format!("../../tests/fixtures/{rel}");
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            f(&bytes);
        }
    }

    #[test]
    fn truncated_and_empty_inputs_survive() {
        let bytes = std::fs::read("../../tests/fixtures/epub/rich.epub").unwrap();
        for f in [epub as fn(&[u8]), pptx, mobi, pdf, djvu] {
            f(&[]);
            f(&bytes[..bytes.len() / 2]);
            f(&bytes[..1]);
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-adapters fuzz_entry`
Expected: FAIL — compile error, `fuzz_entry` is not declared as a module in `lib.rs`, and `assert_assets_contained`/`epub`/etc. do not exist.

- [ ] **Step 3: Write minimal implementation**

Add to `crates/kasane-adapters/src/lib.rs`, immediately after the `mod detect;` line so it sorts with the other module declarations:

```rust
#[doc(hidden)]
pub mod fuzz_entry;
```

Prepend to `crates/kasane-adapters/src/fuzz_entry.rs`, above the test module:

```rust
//! Shared bodies for the fuzz targets in `fuzz/`.
//!
//! **This is a test seam, not public API.** It is `pub` only so the `fuzz/`
//! crate — a separate cargo workspace — can call it, and it lives *inside*
//! this crate so it can reach `pub(crate)` internals such as
//! `math::capture_island` and `mobi::palmdoc::decompress` without widening the
//! real public surface.
//!
//! Every function here has the same shape: `fn(&[u8])`. It takes arbitrary
//! bytes and either returns normally or panics. A panic **is** the finding —
//! these functions never return an error to report one. That uniformity is
//! what lets `tests/fuzz_corpus.rs` dispatch by directory name and keeps every
//! libFuzzer wrapper identical.

use crate::{Adapter, DjvuAdapter, EpubAdapter, MobiAdapter, PdfAdapter, PptxAdapter};
use kasane_ir::AssetBag;

pub fn epub(data: &[u8]) {
    adapter(&EpubAdapter, data, "fuzz.epub");
}

pub fn pptx(data: &[u8]) {
    adapter(&PptxAdapter, data, "fuzz.pptx");
}

/// Covers MOBI and AZW3/KF8 alike — they are one adapter.
pub fn mobi(data: &[u8]) {
    adapter(&MobiAdapter, data, "fuzz.mobi");
}

pub fn pdf(data: &[u8]) {
    adapter(&PdfAdapter, data, "fuzz.pdf");
}

pub fn djvu(data: &[u8]) {
    adapter(&DjvuAdapter, data, "fuzz.djvu");
}

/// A rejected parse is a perfectly good outcome — most fuzzer inputs are not
/// valid documents. Only a *successful* parse has assets worth checking.
fn adapter(a: &dyn Adapter, data: &[u8], source_path: &str) {
    if let Ok((_doc, assets)) = a.parse(data, source_path) {
        assert_assets_contained(&assets);
    }
}

/// AGENTS.md: "No path traversal: sanitize archive entry names and asset
/// filenames, confine writes to `_assets/`." `AssetItem::filename` is what the
/// writer actually creates inside `_assets/`, and `guard::safe_media_filename`
/// is supposed to reduce it to a bare basename. Nothing checked that until now.
fn assert_assets_contained(assets: &AssetBag) {
    for item in &assets.items {
        let f = item.filename.as_str();
        assert!(!f.is_empty(), "empty asset filename");
        assert!(
            !f.contains('/') && !f.contains('\\'),
            "asset filename contains a path separator: {f:?}"
        );
        assert!(
            f != "." && f != "..",
            "asset filename is a directory traversal: {f:?}"
        );
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters fuzz_entry`
Expected: PASS, 5 tests.

If `every_fixture_survives_its_adapter` panics with an assertion from `assert_assets_contained`, you have found a real pre-existing bug on a committed fixture. Do not weaken the assertion to make it pass. Stop and report it.

- [ ] **Step 5: Verify both feature sets and lint**

Run: `mise run lint && mise run test`
Expected: PASS.

Run: `mise run lint-ocr && mise run test-ocr`
Expected: PASS. (This catches an unused-import warning that only appears under one feature set — `-D warnings` makes it fatal.)

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-adapters/src/fuzz_entry.rs crates/kasane-adapters/src/lib.rs
git commit -m "test(fuzz): fuzz_entry seam and the five per-format targets"
```

---

### Task 2: Detection and the four sub-parser targets

**Files:**
- Modify: `crates/kasane-adapters/src/fuzz_entry.rs` (append functions; extend the inline test module)

**Interfaces:**
- Consumes: `crate::detect::detect(bytes: &[u8], ext_hint: Option<&str>) -> Option<Format>`; `crate::guard::{safe_entry_name, resolve_rel, check_expansion, MAX_RATIO, MAX_TOTAL_BYTES}`; `crate::math::{capture_island, mathml_to_latex, omml_to_latex}` where `capture_island(reader: &mut quick_xml::Reader<&[u8]>, start: &quick_xml::events::BytesStart) -> Result<String, CaptureError>`; `crate::mobi::palmdoc::decompress(data: &[u8]) -> Vec<u8>`; `crate::xmltext::resolve_general_ref(r: &quick_xml::events::BytesRef<'_>) -> String`.
- Produces: `pub fn detect(data: &[u8])`, `pub fn math_island(data: &[u8])`, `pub fn palmdoc(data: &[u8])`, `pub fn guards(data: &[u8])`, `pub fn xmltext(data: &[u8])`. Also the private helper `fn split2(data: &[u8]) -> (&[u8], &[u8])`, used by Task 3.

**Context you need — read this before writing the assertions:**

1. **`resolve_rel` returns a `/`-joined path whose segments came from `split('/')`.** A segment equal to `..` pops, so no *segment* is ever `..` — but a segment may legitimately be `..foo`, which *contains* `".."`. Asserting `!path.contains("..")` would therefore report false crashes on valid input. Assert on **components**: `!p.split('/').any(|s| s == "..")`.

2. **`safe_entry_name` is deliberately stricter** — it rejects any name containing `..` as a substring. Its output can be checked with the substring form.

3. **`check_expansion` monotonicity is the property worth asserting.** The guard is called repeatedly as `decompressed` grows during streaming decompression. That is only sound if the predicate, once false, stays false as `decompressed` increases. Assert it directly: it is the assumption every caller silently makes.

4. **`capture_island` and `resolve_general_ref` take quick-xml types, not bytes.** Do not synthesize those types. Run a real `Reader` over the input and call the function on the events it produces — that is the realistic path and it is also what the adapters do.

5. **Multi-argument targets carve their inputs from the byte slice** (NUL-separated) rather than pulling in `arbitrary`. Keep the uniform `fn(&[u8])` signature.

- [ ] **Step 1: Write the failing test**

Append these tests inside the existing `mod tests` block in `crates/kasane-adapters/src/fuzz_entry.rs`:

```rust
    #[test]
    fn split2_splits_on_the_first_nul_only() {
        assert_eq!(split2(b"ab\0cd\0ef"), (&b"ab"[..], &b"cd\0ef"[..]));
        assert_eq!(split2(b"abc"), (&b"abc"[..], &b""[..]));
        assert_eq!(split2(b""), (&b""[..], &b""[..]));
    }

    #[test]
    fn sub_parsers_survive_hostile_shapes() {
        // Deeply nested XML: the case capture_island exists to stop. 4x the
        // MAX_ISLAND_NESTING bound of 128, which would abort the process via
        // stack overflow if it reached roxmltree unguarded.
        let deep = format!(
            "<math>{}{}</math>",
            "<mrow>".repeat(512),
            "</mrow>".repeat(512)
        );
        math_island(deep.as_bytes());

        // Unclosed island: exercises the rewind path.
        math_island(b"<math><mrow><mi>x</mi>");

        // Entity-expansion shapes.
        xmltext(b"<p>&lt;&amp;&undefined;&#x41;&#999999999;</p>");

        // Back-reference opcodes with distances pointing before the buffer.
        palmdoc(&[0x80, 0x00, 0xFF, 0xC0, 0x01, 0x02]);

        // Traversal attempts, NUL-separated base and target.
        guards(b"ppt/slides\0../../../../etc/passwd");
        guards(b"\0/abs/path");

        for f in [
            detect as fn(&[u8]),
            math_island,
            palmdoc,
            guards,
            xmltext,
        ] {
            f(&[]);
            f(&[0u8; 64]);
            f(b"<<<>>>&&&\0\0\0");
        }
    }

    #[test]
    fn check_expansion_is_monotone_in_decompressed() {
        // The property every streaming caller depends on: once the guard says
        // no, growing the decompressed size never makes it say yes again.
        for compressed in [0u64, 1, 7, 1024, u64::MAX] {
            for decompressed in [0u64, 1, 200, 201, crate::guard::MAX_TOTAL_BYTES] {
                if !crate::guard::check_expansion(compressed, decompressed) {
                    for grown in [decompressed.saturating_add(1), u64::MAX] {
                        assert!(
                            !crate::guard::check_expansion(compressed, grown),
                            "non-monotone at compressed={compressed} decompressed={decompressed} grown={grown}"
                        );
                    }
                }
            }
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-adapters fuzz_entry`
Expected: FAIL — compile error, `split2`, `detect`, `math_island`, `palmdoc`, `guards`, `xmltext` do not exist.

- [ ] **Step 3: Write minimal implementation**

Extend the `use` block at the top of `fuzz_entry.rs`:

```rust
use crate::guard::{check_expansion, resolve_rel, safe_entry_name};
use crate::math::{capture_island, mathml_to_latex, omml_to_latex};
use crate::mobi::palmdoc::decompress;
use crate::xmltext::resolve_general_ref;
use crate::{Adapter, DjvuAdapter, EpubAdapter, MobiAdapter, PdfAdapter, PptxAdapter};
use kasane_ir::AssetBag;
use quick_xml::events::Event;
use quick_xml::Reader;
```

Append the functions:

```rust
/// Split on the first NUL. Lets a multi-argument target keep the uniform
/// `fn(&[u8])` signature without pulling `arbitrary` into the library crate.
fn split2(data: &[u8]) -> (&[u8], &[u8]) {
    match data.iter().position(|b| *b == 0) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &[]),
    }
}

/// First byte selects the extension hint; the rest is the content. Detection is
/// the first code in the process to touch hostile bytes.
pub fn detect(data: &[u8]) {
    const HINTS: [Option<&str>; 8] = [
        None,
        Some("epub"),
        Some("pptx"),
        Some("mobi"),
        Some("azw3"),
        Some("pdf"),
        Some("djvu"),
        Some("../../etc/passwd"),
    ];
    let (hint, body) = match data.split_first() {
        Some((h, rest)) => (HINTS[(*h as usize) % HINTS.len()], rest),
        None => (None, data),
    };
    let _ = crate::detect(body, hint);
}

/// The highest-value target in the set. `capture_island` is the only thing
/// standing between an over-deep island and a stack overflow inside roxmltree,
/// and a stack overflow aborts the process — no `Result` plumbing recovers from
/// it. Feed the captured island to both front-ends, which are entry points in
/// their own right and re-check the budget themselves.
pub fn math_island(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut reader = Reader::from_str(text);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => return,
            Ok(Event::Start(start)) => {
                let start = start.into_owned();
                if let Ok(island) = capture_island(&mut reader, &start) {
                    let _ = mathml_to_latex(&island);
                    let _ = omml_to_latex(&island);
                }
            }
            Ok(_) => {}
        }
    }
}

/// A hand-rolled LZ77-style decoder reading attacker-controlled back-reference
/// distances and lengths.
pub fn palmdoc(data: &[u8]) {
    let _ = decompress(data);
}

/// Three pure functions whose postconditions are security-critical and, until
/// now, asserted nowhere.
pub fn guards(data: &[u8]) {
    let (base, target) = split2(data);
    let (Ok(base), Ok(target)) = (std::str::from_utf8(base), std::str::from_utf8(target)) else {
        return;
    };

    if let Some(name) = safe_entry_name(target) {
        // safe_entry_name rejects `..` as a substring, so the substring form is
        // the right check for *its* output.
        assert!(
            !name.contains("..") && !name.starts_with('/'),
            "safe_entry_name admitted an escaping name: {name:?} from {target:?}"
        );
    }

    if let Some(path) = resolve_rel(base, target) {
        // resolve_rel joins segments, and a `..` segment pops rather than being
        // emitted -- but a segment may legitimately *contain* `..` (e.g.
        // `..foo`). Check components, not substrings, or valid input reports as
        // a crash.
        assert!(
            !path.split('/').any(|s| s == ".."),
            "resolve_rel emitted a traversal component: {path:?} from base={base:?} target={target:?}"
        );
        assert!(
            !path.starts_with('/') && !path.is_empty(),
            "resolve_rel emitted an absolute or empty path: {path:?}"
        );
    }

    // Monotonicity: the streaming callers re-check as `decompressed` grows, so
    // a predicate that could flip back to true would let a bomb through.
    let (c, d) = (
        u64::from_le_bytes(std::array::from_fn(|i| *data.get(i).unwrap_or(&0))),
        u64::from_le_bytes(std::array::from_fn(|i| *data.get(i + 8).unwrap_or(&0))),
    );
    if !check_expansion(c, d) {
        assert!(
            !check_expansion(c, d.saturating_add(1)) && !check_expansion(c, u64::MAX),
            "check_expansion is non-monotone at compressed={c} decompressed={d}"
        );
    }
}

/// The entity-expansion surface. Drive a real reader so the `BytesRef` values
/// are the ones the adapters actually see.
pub fn xmltext(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut reader = Reader::from_str(text);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => return,
            Ok(Event::GeneralRef(r)) => {
                let _ = resolve_general_ref(&r);
            }
            Ok(_) => {}
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters fuzz_entry`
Expected: PASS, 8 tests.

If the quick-xml event variant names do not match (this codebase pins quick-xml 0.41 — check `Event::GeneralRef` against `crates/kasane-adapters/src/epub/xhtml.rs`, which already matches on it), fix the match arms rather than the test.

- [ ] **Step 5: Verify both feature sets and lint**

Run: `mise run lint && mise run test`
Expected: PASS.

Run: `mise run lint-ocr && mise run test-ocr`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-adapters/src/fuzz_entry.rs
git commit -m "test(fuzz): detection and sub-parser fuzz entry points"
```

---

### Task 3: The structured ZIP builder

**Files:**
- Modify: `crates/kasane-adapters/src/fuzz_entry.rs` (append builder + two targets; extend the inline test module)

**Interfaces:**
- Consumes: `split2` and `adapter` from Tasks 1–2; `zip::write::{ZipWriter, SimpleFileOptions}`; `zip::CompressionMethod`.
- Produces: `pub fn epub_zip(data: &[u8])`, `pub fn pptx_zip(data: &[u8])`, and `fn build_zip(data: &[u8]) -> Vec<u8>`.

**Why this task exists (spec §4.1):** EPUB and PPTX are ZIP containers and the `zip` crate verifies CRCs on read. Mutating raw bytes almost always breaks a CRC, so a raw-bytes-only target bounces off the container and never reaches the OPF parser, the XHTML parser, `safe_entry_name`, or the math island capture. This builder assembles a *structurally valid* archive — correct headers and CRCs — from fuzzer-controlled entry names and member contents, so mutation lands where the interesting code is.

The raw `epub`/`pptx` targets from Task 1 stay. They test the container; this tests past it.

**Input encoding:** the byte slice is a sequence of NUL-separated fields read pairwise — `name1 \0 content1 \0 name2 \0 content2 \0 ...`. A trailing unpaired name gets empty content. Cap at 64 entries so one input cannot build an enormous archive and starve the fuzzer's throughput.

- [ ] **Step 1: Write the failing test**

Append inside `mod tests`:

```rust
    #[test]
    fn build_zip_produces_a_readable_archive() {
        let raw = b"mimetype\0application/epub+zip\0META-INF/container.xml\0<container/>";
        let bytes = build_zip(raw);
        let mut ar = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..]))
            .expect("builder must emit a structurally valid archive");
        assert_eq!(ar.len(), 2);
        assert_eq!(ar.by_index(0).unwrap().name(), "mimetype");
        assert_eq!(ar.by_index(1).unwrap().name(), "META-INF/container.xml");
    }

    #[test]
    fn build_zip_tolerates_hostile_entry_names() {
        // Names the builder must not choke on -- rejecting them is the
        // *adapter's* job, not the builder's.
        let raw = b"../../etc/passwd\0x\0/abs\0y\0\0z";
        let bytes = build_zip(raw);
        assert!(zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).is_ok());
    }

    #[test]
    fn zip_targets_survive_arbitrary_input() {
        for f in [epub_zip as fn(&[u8]), pptx_zip] {
            f(&[]);
            f(b"mimetype\0application/epub+zip");
            f(b"ppt/slides/slide1.xml\0<p:sld><m:oMath/></p:sld>");
            f(b"../../escape\0<x/>\0OEBPS/a.xhtml\0<math><mrow/></math>");
            f(&[0u8; 256]);
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-adapters fuzz_entry`
Expected: FAIL — compile error, `build_zip`, `epub_zip`, `pptx_zip` do not exist.

- [ ] **Step 3: Write minimal implementation**

Add to the `use` block:

```rust
use std::io::Write as _;
```

Append:

```rust
/// Maximum members the builder will emit from one input. Without a cap a single
/// fuzzer input could build an archive with thousands of entries, which costs
/// throughput without buying coverage.
const MAX_ZIP_ENTRIES: usize = 64;

/// Assemble a structurally valid ZIP -- correct local headers, central
/// directory and CRCs -- from fuzzer-controlled entry names and contents.
///
/// The `zip` crate verifies CRCs on read, so raw byte mutation is rejected at
/// the container before it ever reaches the parsers underneath. Generating a
/// valid container puts the mutation budget on entry names and member payloads
/// instead, which is where `safe_entry_name`, the bomb guards, and the OPF /
/// XHTML / math parsers live.
///
/// Input is NUL-separated fields read pairwise: name, content, name, content...
fn build_zip(data: &[u8]) -> Vec<u8> {
    let mut out = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut rest = data;
    for _ in 0..MAX_ZIP_ENTRIES {
        if rest.is_empty() {
            break;
        }
        let (name, after) = split2(rest);
        let (content, after) = split2(after);
        rest = after;
        // Lossy is right here: an entry name is a string to the zip crate, and
        // invalid UTF-8 should still produce *an* entry rather than skipping it.
        let name = String::from_utf8_lossy(name).into_owned();
        // The zip crate rejects an empty name outright, which would abort the
        // whole build; substitute rather than lose every later entry.
        let name = if name.is_empty() { "_".to_string() } else { name };
        if out.start_file(name, opts).is_err() {
            continue;
        }
        let _ = out.write_all(content);
    }
    match out.finish() {
        Ok(c) => c.into_inner(),
        Err(_) => Vec::new(),
    }
}

/// EPUB past the container (see `build_zip`).
pub fn epub_zip(data: &[u8]) {
    adapter(&EpubAdapter, &build_zip(data), "fuzz.epub");
}

/// PPTX past the container (see `build_zip`).
pub fn pptx_zip(data: &[u8]) {
    adapter(&PptxAdapter, &build_zip(data), "fuzz.pptx");
}
```

Add to the `use` block:

```rust
use zip::write::{SimpleFileOptions, ZipWriter};
use zip::CompressionMethod;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters fuzz_entry`
Expected: PASS, 11 tests.

If `SimpleFileOptions` does not resolve, the pinned `zip` 2.4.2 may name it `FileOptions`. Check `cargo doc -p zip --open` or the fixture generators, and use whichever the pinned version exports.

- [ ] **Step 5: Verify both feature sets and lint**

Run: `mise run lint && mise run test`
Expected: PASS.

Run: `mise run lint-ocr && mise run test-ocr`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-adapters/src/fuzz_entry.rs
git commit -m "test(fuzz): structured ZIP builder for the epub/pptx targets"
```

---

### Task 4: Seeds and the stable replay test

**Files:**
- Create: `crates/kasane-adapters/tests/fuzz_corpus.rs`
- Create: `fuzz/seeds/math_island/{deep-nesting.xml,unclosed.xml,omml-fraction.xml,mathml-fraction.xml}`
- Create: `fuzz/seeds/palmdoc/{backref.bin,literals.bin}`
- Create: `fuzz/seeds/guards/{traversal.bin,absolute.bin}`
- Create: `fuzz/seeds/xmltext/entities.xml`
- Create: `fuzz/seeds/epub_zip/minimal.bin`, `fuzz/seeds/pptx_zip/minimal.bin`
- Create: `fuzz/artifacts/.gitkeep`

**Interfaces:**
- Consumes: every `pub fn` in `kasane_adapters::fuzz_entry` from Tasks 1–3.
- Produces: nothing later tasks call. This is the gate that makes the seam load-bearing in PR CI.

**Why seeds get replayed too (spec §5):** with an empty `artifacts/` the replay test would pass vacuously and `fuzz_entry` would be dead code until the fuzzer's first find. Replaying seeds exercises the entry functions in `mise run test` from day one.

**Why an unknown directory is a failure, not a skip:** if a target is renamed and the map is not updated, a silent skip means its corpus quietly stops being replayed and a fixed crash can regress unnoticed.

- [ ] **Step 1: Write the failing test**

Create `crates/kasane-adapters/tests/fuzz_corpus.rs`:

```rust
//! Replays committed fuzz corpora through the same `fuzz_entry` functions the
//! libFuzzer targets call — on the pinned **stable** toolchain, inside
//! `mise run test`, on every PR.
//!
//! `fuzz/artifacts/<target>/` holds crash reproducers. Committing one is
//! mandatory when the fuzzer finds a crash: that is what turns a one-off find
//! into a permanent regression test.
//!
//! `fuzz/seeds/<target>/` holds hand-written starting inputs. They are replayed
//! too, so these functions are exercised here from day one rather than staying
//! dead code until the first crash lands.

use kasane_adapters::fuzz_entry;
use std::path::Path;

/// Every fuzz target, by the directory name its corpus lives under.
///
/// Adding a target means adding it here. An unrecognized directory is a test
/// failure (see `unknown_corpus_directory_is_a_failure`), so a renamed target
/// cannot silently stop being replayed.
fn target(name: &str) -> Option<fn(&[u8])> {
    Some(match name {
        "epub" => fuzz_entry::epub,
        "pptx" => fuzz_entry::pptx,
        "mobi" => fuzz_entry::mobi,
        "pdf" => fuzz_entry::pdf,
        "djvu" => fuzz_entry::djvu,
        "epub_zip" => fuzz_entry::epub_zip,
        "pptx_zip" => fuzz_entry::pptx_zip,
        "detect" => fuzz_entry::detect,
        "math_island" => fuzz_entry::math_island,
        "palmdoc" => fuzz_entry::palmdoc,
        "guards" => fuzz_entry::guards,
        "xmltext" => fuzz_entry::xmltext,
        _ => return None,
    })
}

const TARGET_COUNT: usize = 12;

fn corpus_root(which: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz").join(which)
}

/// Run every file under `fuzz/<which>/<target>/` through `<target>`.
fn replay(which: &str) -> usize {
    let root = corpus_root(which);
    if !root.is_dir() {
        return 0;
    }
    let mut ran = 0;
    for entry in std::fs::read_dir(&root).expect("corpus root is readable") {
        let dir = entry.expect("readable dir entry").path();
        if !dir.is_dir() {
            continue; // .gitkeep and friends
        }
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let f = target(&name).unwrap_or_else(|| {
            panic!(
                "{}/{name}/ has no matching fuzz target. Rename the directory or \
                 add the target to `target()` — a silent skip would stop replaying it.",
                root.display()
            )
        });
        for file in std::fs::read_dir(&dir).expect("corpus dir is readable") {
            let path = file.expect("readable file entry").path();
            if !path.is_file() {
                continue;
            }
            let bytes = std::fs::read(&path).expect("corpus file is readable");
            // A panic here is the point: it means a previously-found crash has
            // regressed, and the panic message names the offending input.
            f(&bytes);
            ran += 1;
        }
    }
    ran
}

#[test]
fn replays_committed_seeds() {
    let ran = replay("seeds");
    assert!(
        ran > 0,
        "no seed inputs were replayed — fuzz/seeds/ is empty or missing, which \
         would make this whole test pass vacuously"
    );
}

#[test]
fn replays_committed_crash_artifacts() {
    // Legitimately zero until the fuzzer finds something. Once a reproducer is
    // committed it is replayed forever.
    replay("artifacts");
}

#[test]
fn every_target_is_reachable_by_name() {
    let names = [
        "epub", "pptx", "mobi", "pdf", "djvu", "epub_zip", "pptx_zip", "detect",
        "math_island", "palmdoc", "guards", "xmltext",
    ];
    assert_eq!(names.len(), TARGET_COUNT);
    for n in names {
        assert!(target(n).is_some(), "target {n} is not mapped");
    }
    assert!(target("no_such_target").is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-adapters --test fuzz_corpus`
Expected: FAIL — `replays_committed_seeds` panics with "no seed inputs were replayed", because `fuzz/seeds/` does not exist yet.

- [ ] **Step 3: Create the seed corpus**

Seeds are deliberately tiny and must not duplicate `tests/fixtures` (spec §5). Create each file:

```bash
mkdir -p fuzz/seeds/{math_island,palmdoc,guards,xmltext,epub_zip,pptx_zip} fuzz/artifacts
touch fuzz/artifacts/.gitkeep

# math_island: the shapes capture_island exists to handle
python3 -c "open('fuzz/seeds/math_island/deep-nesting.xml','w').write('<math>'+'<mrow>'*200+'<mi>x</mi>'+'</mrow>'*200+'</math>')"
printf '<math><mrow><mi>x</mi>' > fuzz/seeds/math_island/unclosed.xml
printf '<math><mfrac><mi>a</mi><mi>b</mi></mfrac></math>' > fuzz/seeds/math_island/mathml-fraction.xml
printf '<m:oMath><m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f></m:oMath>' > fuzz/seeds/math_island/omml-fraction.xml

# palmdoc: literal run, then back-reference opcodes
printf '\x01A\x05hello\x09B' > fuzz/seeds/palmdoc/literals.bin
printf '\x80\x00\xc0\x41\xbf\xff' > fuzz/seeds/palmdoc/backref.bin

# guards: NUL-separated base and target
printf 'ppt/slides\x00../../../../etc/passwd' > fuzz/seeds/guards/traversal.bin
printf '\x00/absolute/path' > fuzz/seeds/guards/absolute.bin

# xmltext: named, numeric, and undefined entities
printf '<p>&lt;&amp;&gt;&#x41;&#65;&#999999999;&undefined;</p>' > fuzz/seeds/xmltext/entities.xml

# zip builders: NUL-separated name/content pairs
printf 'mimetype\x00application/epub+zip\x00META-INF/container.xml\x00<container><rootfiles><rootfile full-path="OEBPS/c.opf"/></rootfiles></container>' > fuzz/seeds/epub_zip/minimal.bin
printf '[Content_Types].xml\x00<Types/>\x00ppt/slides/slide1.xml\x00<p:sld><m:oMath/></p:sld>' > fuzz/seeds/pptx_zip/minimal.bin
```

The five raw per-format targets and `detect` get no seed directory — their corpus is seeded from `tests/fixtures` at run time by the mise task in Task 5, and `replay()` skips a target with no directory.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters --test fuzz_corpus`
Expected: PASS, 3 tests. `replays_committed_seeds` replays 11 files.

- [ ] **Step 5: Prove the replay test can actually fail**

This is the step that distinguishes a wired-up regression gate from one that only looks wired up. Do all four sub-steps.

```bash
# 5a. A corpus directory with no matching target must fail loudly.
mkdir -p fuzz/seeds/not_a_target && printf 'x' > fuzz/seeds/not_a_target/x.bin
cargo test -p kasane-adapters --test fuzz_corpus
# Expected: FAIL, "has no matching fuzz target"
rm -rf fuzz/seeds/not_a_target

# 5b. An input that trips an invariant must fail loudly. `guards` asserts that
# resolve_rel never emits a `..` component; feed it something and confirm the
# harness would report a violation by temporarily inverting the assertion.
#     - Open crates/kasane-adapters/src/fuzz_entry.rs
#     - In `guards`, change `!path.split('/').any(|s| s == "..")` to `path.split('/').any(|s| s == "..")`
cargo test -p kasane-adapters --test fuzz_corpus
# Expected: FAIL, "resolve_rel emitted a traversal component"
#     - REVERT that edit now.

# 5c. Confirm the revert took.
cargo test -p kasane-adapters --test fuzz_corpus
# Expected: PASS

# 5d. Confirm seeds are not silently skipped: temporarily move them aside.
mv fuzz/seeds /tmp/seeds-check && cargo test -p kasane-adapters --test fuzz_corpus
# Expected: FAIL, "no seed inputs were replayed"
mv /tmp/seeds-check fuzz/seeds
```

Do not proceed until 5a, 5b and 5d have each been observed to fail and 5c to pass. A green suite that has never been shown to go red proves nothing.

- [ ] **Step 6: Verify both feature sets and lint**

Run: `mise run lint && mise run test`
Expected: PASS.

Run: `mise run lint-ocr && mise run test-ocr`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/kasane-adapters/tests/fuzz_corpus.rs fuzz/seeds fuzz/artifacts/.gitkeep
git commit -m "test(fuzz): stable replay of committed seeds and crash artifacts"
```

---

### Task 5: The fuzz crate, toolchain, and mise tasks

**Files:**
- Create: `fuzz/Cargo.toml`, `fuzz/.gitignore`
- Create: `fuzz/fuzz_targets/{epub,pptx,mobi,pdf,djvu,epub_zip,pptx_zip,detect,math_island,palmdoc,guards,xmltext}.rs`
- Modify: `Cargo.toml` (root — workspace `exclude`)
- Modify: `.gitignore`
- Modify: `mise.toml`

**Interfaces:**
- Consumes: every `pub fn` in `kasane_adapters::fuzz_entry`.
- Produces: `mise run fuzz <target>` and `mise run fuzz-all`, used by Task 6's workflow.

**Resolve this first — it decides the task bodies (spec §6).** mise's rust backend advertises a floating `nightly` and `mise ls-remote rust` does not list dated nightlies, but `ls-remote` lists releases and the backend may still pass a dated spec through to rustup. Check directly:

```bash
mise install rust@nightly-2026-07-01 && mise exec rust@nightly-2026-07-01 -- rustc --version
```

- **If it resolves:** pin the dated nightly in `mise.toml`. The toolchain story stays single-sourced, as intended.
- **If it does not:** pin the floating `nightly` and add the comment shown in Step 4. A floating pin is real drift and gets written down, not buried — it is the only pin in this repo that can change under CI without a commit.

Use whichever resolved as `<NIGHTLY>` throughout the rest of this task.

- [ ] **Step 1: Scaffold the fuzz crate**

Create `fuzz/Cargo.toml`. Note `[workspace]` with no members — that is what makes it its own workspace, and it pairs with the root `exclude`.

```toml
[package]
name = "kasane-fuzz"
version = "0.0.0"
publish = false
edition = "2021"

[package.metadata]
cargo-fuzz = true

# Its own workspace. cargo-fuzz builds this crate with nightly-only flags that
# must not leak into the root workspace, which stays on the pinned stable
# toolchain. The root Cargo.toml excludes this directory to match.
[workspace]

[dependencies]
libfuzzer-sys = "0.4"
kasane-adapters = { path = "../crates/kasane-adapters" }

# Fuzz targets must be built with debug assertions and overflow checks on, in
# release mode. Without this an integer overflow that would panic in a debug
# build silently wraps and the fuzzer never reports it.
[profile.release]
debug = 1
debug-assertions = true
overflow-checks = true
```

Create `fuzz/.gitignore`:

```
corpus
target
```

- [ ] **Step 2: Write the twelve targets**

Every target is identical apart from the function name. Create `fuzz/fuzz_targets/epub.rs`:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    kasane_adapters::fuzz_entry::epub(data);
});
```

Create the other eleven the same way, substituting the function name for each of: `pptx`, `mobi`, `pdf`, `djvu`, `epub_zip`, `pptx_zip`, `detect`, `math_island`, `palmdoc`, `guards`, `xmltext`.

Then register each as a bin target by appending twelve blocks to `fuzz/Cargo.toml` — one per target, following this shape exactly:

```toml
[[bin]]
name = "epub"
path = "fuzz_targets/epub.rs"
test = false
doc = false
bench = false
```

- [ ] **Step 3: Exclude the fuzz crate from the root workspace**

In the root `Cargo.toml`, add to the existing `[workspace]` table:

```toml
exclude = ["fuzz"]
```

And add to the root `.gitignore`, which currently contains only `/target`:

```
/fuzz/corpus
/fuzz/target
```

- [ ] **Step 4: Add the toolchain pin and tasks to `mise.toml`**

In `[tools]`, add `cargo-fuzz` next to the existing `cargo-deny` pin, and the nightly toolchain. If the dated nightly did **not** resolve, use this comment verbatim:

```toml
# cargo-fuzz builds its targets with libFuzzer and sanitizer flags that only
# exist on nightly. The stable pin above is what everything else builds with;
# this is used by `mise run fuzz` and the fuzz workflow, nowhere else.
#
# NOTE: this is a FLOATING pin. mise's rust backend does not resolve dated
# nightlies, so unlike every other pin in this file it can change under CI
# without a commit. If a fuzz run fails in a way the code does not explain,
# suspect a toolchain move first.
"rust-nightly" = { version = "nightly", components = "rustfmt,clippy" }
"cargo:cargo-fuzz" = "0.13.2"
```

If the dated nightly **did** resolve, use `version = "nightly-2026-07-01"` and drop the second paragraph of that comment.

Then name the toolchain **once**, in `[env]`, so the two tasks below and the CI
workflow all reference one string rather than each hardcoding a channel. Set it
to whichever spec resolved above — `nightly` or `nightly-2026-07-01`:

```toml
[env]
# The toolchain `cargo fuzz` builds targets with. Named here so `mise run fuzz`,
# `mise run fuzz-all`, and .github/workflows/fuzz.yml cannot drift apart. mise's
# rust backend installs it through rustup, which is what makes `cargo +<name>`
# resolve.
KASANE_FUZZ_TOOLCHAIN = "nightly"
```

Add the tasks:

```toml
[tasks.fuzz]
description = "Fuzz one target: mise run fuzz <target> [-- -max_total_time=60]"
run = """
set -euo pipefail
target="$1"; shift || true
mkdir -p "fuzz/corpus/$target"
# Seed per target (design spec §5). The per-format and ZIP targets get their own
# format's fixtures; detect gets everything; the sub-parser targets take seeds
# only -- an EPUB archive is not a useful starting input for palmdoc.
case "$target" in
  epub|epub_zip) cp -f tests/fixtures/epub/*.epub "fuzz/corpus/$target/" 2>/dev/null || true ;;
  pptx|pptx_zip) cp -f tests/fixtures/pptx/*.pptx "fuzz/corpus/$target/" 2>/dev/null || true ;;
  mobi)          cp -f tests/fixtures/mobi/*.mobi tests/fixtures/azw3/*.azw3 "fuzz/corpus/$target/" 2>/dev/null || true ;;
  pdf)           cp -f tests/fixtures/pdf/*.pdf "fuzz/corpus/$target/" 2>/dev/null || true ;;
  djvu)          cp -f tests/fixtures/djvu/*.djvu "fuzz/corpus/$target/" 2>/dev/null || true ;;
  detect)        find tests/fixtures -type f ! -name '*.py' ! -name '*.md' -exec cp -f {} "fuzz/corpus/$target/" \\; 2>/dev/null || true ;;
esac
if [ -d "fuzz/seeds/$target" ]; then cp -f "fuzz/seeds/$target/"* "fuzz/corpus/$target/" 2>/dev/null || true; fi
cargo "+$KASANE_FUZZ_TOOLCHAIN" fuzz run "$target" "fuzz/corpus/$target" -- \\
  -rss_limit_mb=2048 -malloc_limit_mb=2048 -timeout=25 "$@"
"""

[tasks.fuzz-all]
description = "Fuzz every target for 5 minutes each"
run = """
set -euo pipefail
for t in epub pptx mobi pdf djvu epub_zip pptx_zip detect math_island palmdoc guards xmltext; do
  echo "=== $t ==="
  mise run fuzz "$t" -- -max_total_time=300
done
"""
```

Confirm `cargo "+$KASANE_FUZZ_TOOLCHAIN"` actually resolves before relying on it:

```bash
mise exec -- cargo "+$KASANE_FUZZ_TOOLCHAIN" --version
```

Expected: a `cargo 1.x.y-nightly` banner. If it errors with "toolchain not installed", mise's rust backend did not register the nightly with rustup; substitute `mise exec rust-nightly -- cargo fuzz run ...` in both tasks and drop the `+` form. Verify in Step 5 rather than assuming.

- [ ] **Step 5: Run every target briefly and confirm it executes**

Compiling is not evidence. Each target must be observed to *run* and accumulate coverage.

```bash
mise install
for t in epub pptx mobi pdf djvu epub_zip pptx_zip detect math_island palmdoc guards xmltext; do
  echo "=== $t ==="
  mise run fuzz "$t" -- -max_total_time=20 -print_final_stats=1
done
```

Expected for each: libFuzzer banner, a rising `cov:` counter, and `Done ... runs:` at exit. A target that reports `runs: 0` or a flat `cov:` is not fuzzing anything — investigate before continuing.

Expected `NEW_FUNC` / coverage growth specifically on `math_island`, `guards`, `epub_zip`, and `pptx_zip`; those are the targets whose whole purpose is reaching code the raw targets cannot.

If any target crashes: that is a real finding. Commit the reproducer under `fuzz/artifacts/<target>/` and note it in the commit message. Do **not** fix the underlying bug in this plan (Global Constraints).

- [ ] **Step 6: Confirm the stable side still passes**

Run: `mise run lint && mise run test`
Expected: PASS. The root workspace must be completely unaffected by the new `fuzz/` directory — if `cargo` tries to build the fuzz crate, the `exclude` in Step 3 is wrong.

Run: `mise run lint-ocr && mise run test-ocr`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add fuzz/Cargo.toml fuzz/.gitignore fuzz/fuzz_targets Cargo.toml .gitignore mise.toml
# plus any fuzz/artifacts/<target>/ reproducers found in Step 5
git commit -m "test(fuzz): cargo-fuzz targets, nightly pin, and mise tasks"
```

---

### Task 6: CI workflow and dependency coverage

**Files:**
- Create: `.github/workflows/fuzz.yml`
- Modify: `.github/dependabot.yml`

**Interfaces:**
- Consumes: `mise run fuzz <target>` from Task 5.
- Produces: nothing later tasks call.

**Context you need:** model this on the existing `.github/workflows/audit.yml`. That file pins actions by commit SHA with a trailing version comment — Dependabot reads the version from that comment, and a SHA pin with no comment silently rots. Copy the exact SHAs already in use in `audit.yml` rather than looking up new ones. `audit.yml` runs its cron at `0 7 * * 1`; offset this one so the two do not contend.

- [ ] **Step 1: Write the workflow**

Create `.github/workflows/fuzz.yml`:

```yaml
name: fuzz

on:
  workflow_dispatch:
    inputs:
      target:
        description: 'Single target to fuzz (blank = all)'
        required: false
        type: string
      duration:
        description: 'Seconds per target'
        required: false
        default: '300'
        type: string
  # Advisories aside, fuzzing finds nothing if nobody runs it. Offset an hour
  # from audit.yml's 07:00 Monday so the two do not contend.
  schedule:
    - cron: '0 8 * * 1'

permissions:
  contents: read

concurrency:
  group: fuzz-${{ github.ref }}
  cancel-in-progress: true

jobs:
  fuzz:
    runs-on: ubuntu-latest
    strategy:
      # One crashing target must not cancel the other eleven — each is an
      # independent finding.
      fail-fast: false
      matrix:
        target:
          - epub
          - pptx
          - mobi
          - pdf
          - djvu
          - epub_zip
          - pptx_zip
          - detect
          - math_island
          - palmdoc
          - guards
          - xmltext
    steps:
      - uses: actions/checkout@93cb6efe18208431cddfb8368fd83d5badbf9bfd # v5
      - uses: jdx/mise-action@dad1bfd3df957f44999b559dd69dc1671cb4e9ea # v4.2.1
      - uses: Swatinem/rust-cache@c19371144df3bb44fab255c43d04cbc2ab54d1c4 # v2.9.1

      - name: Fuzz ${{ matrix.target }}
        if: ${{ github.event.inputs.target == '' || github.event.inputs.target == matrix.target }}
        run: mise run fuzz ${{ matrix.target }} -- -max_total_time=${{ github.event.inputs.duration || '300' }}

      # A crash writes a reproducer to fuzz/artifacts/<target>/. Upload it so it
      # can be committed — that is what turns a find into a permanent regression
      # test (see crates/kasane-adapters/tests/fuzz_corpus.rs).
      - name: Upload crash reproducers
        if: failure()
        uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4.6.2
        with:
          name: fuzz-artifacts-${{ matrix.target }}
          path: fuzz/artifacts/
          if-no-files-found: ignore
```

Verify the `upload-artifact` SHA against the current release before committing — it is the one action here not already used elsewhere in this repo, so it cannot be copied from `audit.yml`.

- [ ] **Step 2: Close the dependency-coverage gap**

`fuzz/` is an excluded workspace with its own `Cargo.lock`, so `audit.yml`'s `cargo deny check advisories` does not scan it (spec §8). Dependabot can still keep it updated. Append to `.github/dependabot.yml`:

```yaml
  # fuzz/ is a separate cargo workspace with its own lockfile, so the `/` entry
  # above does not see it and `cargo deny check advisories` in audit.yml does
  # not scan it. These are dev-only dependencies that never ship in the released
  # binary, but they still get update PRs.
  - package-ecosystem: cargo
    directory: /fuzz
    schedule:
      interval: weekly
    open-pull-requests-limit: 3
```

- [ ] **Step 3: Validate the workflow parses**

Run: `python3 -c "import yaml,sys; [yaml.safe_load(open(f)) for f in ['.github/workflows/fuzz.yml','.github/dependabot.yml']]; print('ok')"`
Expected: `ok`

- [ ] **Step 4: Confirm the repo is still green**

Run: `mise run lint && mise run test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/fuzz.yml .github/dependabot.yml
git commit -m "ci(fuzz): weekly fuzz workflow and fuzz/ dependency updates"
```

- [ ] **Step 6: Trigger the workflow once and confirm it passes**

This requires the branch to be pushed. After pushing:

```bash
gh workflow run fuzz.yml --ref adapter-fuzzing -f duration=60
sleep 90 && gh run list --workflow=fuzz.yml --limit 1
gh run watch
```

Expected: twelve matrix jobs, all green. A workflow that has never run is not verified. If a job fails on a crash, download the artifact, commit the reproducer under `fuzz/artifacts/<target>/`, and confirm `cargo test -p kasane-adapters --test fuzz_corpus` now replays it.

---

### Task 7: Documentation

**Files:**
- Modify: `README.md` (Development section)
- Modify: `AGENTS.md` (codebase map, Workflows, Conventions)

**Interfaces:**
- Consumes: everything built in Tasks 1–6.
- Produces: nothing.

- [ ] **Step 1: Document the workflow in README**

Add to `README.md` under `## Development`, after the `mise run lint` line and before the `### OCR (optional)` subsection:

```markdown
### Fuzzing

Every adapter parses untrusted input, so the boundary is fuzzed with
`cargo-fuzz`. Twelve targets cover the five format adapters, format detection,
the two ZIP container formats past their CRC check, and the sub-parsers a
whole-file fuzzer would rarely reach — the math island capture, PalmDOC
decompression, the path guards, and XML entity resolution.

    mise run fuzz math_island                    # one target, until you stop it
    mise run fuzz epub -- -max_total_time=60     # one target, 60 seconds
    mise run fuzz-all                            # every target, 5 minutes each

Targets build on a pinned nightly (libFuzzer needs it); everything else in this
repo stays on the pinned stable toolchain. CI runs the full set weekly.

Beyond panics and hangs, the targets assert that the decompression-bomb guards
hold under libFuzzer's RSS limit, that no asset filename escapes `_assets/`, and
that `safe_entry_name` / `resolve_rel` never emit a traversal.

**When the fuzzer finds a crash, commit the reproducer** from
`fuzz/artifacts/<target>/`. `cargo test` replays every committed reproducer on
the stable toolchain, so a fixed crash stays fixed.

The `ocr` feature is not fuzzed — it links C (Tesseract, Leptonica), which needs
its own sanitizer setup.
```

- [ ] **Step 2: Document the seam in AGENTS.md**

In `AGENTS.md`, append to the `crates/kasane-adapters` bullet in the codebase map:

```markdown
  `fuzz_entry.rs` is a test seam, not API: one `fn(&[u8])` per fuzz target,
  living inside this crate so it can reach `pub(crate)` internals
  (`math::capture_island`, `mobi::palmdoc::decompress`, `guard::*`) that the
  separate `fuzz/` workspace cannot. Each function either returns or panics — a
  panic is the finding. `tests/fuzz_corpus.rs` replays `fuzz/seeds/**` and
  `fuzz/artifacts/**` through those same functions on stable, so fuzz coverage
  reaches PR CI without a nightly toolchain.
```

Replace the Workflows line so the new task is discoverable alongside the others:

```markdown
- `mise run test` — all tests   - `mise run lint` — fmt + clippy   - `mise run convert <file> -o <dir>` — convert
- `mise run fuzz <target>` / `mise run fuzz-all` — fuzz the untrusted-input boundary (nightly; see README)
```

Add to the Conventions list, after the "Adapters must never trust input" line:

```markdown
- A crash the fuzzer finds gets its reproducer committed to `fuzz/artifacts/<target>/`.
  That is what makes it a permanent regression test; fixing the bug without
  committing the input leaves nothing guarding the fix.
- The nightly toolchain pin, like the Rust and cargo-deny pins, is a manual bump.
```

- [ ] **Step 3: Verify the documented commands actually work**

Every command in the docs gets run. Documentation that was never executed is a guess.

```bash
mise run fuzz math_island -- -max_total_time=15
mise run fuzz epub -- -max_total_time=15
```

Expected: both run and exit cleanly.

- [ ] **Step 4: Final full verification**

```bash
mise run lint && mise run test
mise run lint-ocr && mise run test-ocr
```

Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add README.md AGENTS.md
git commit -m "docs(fuzz): document the fuzzing workflow and the fuzz_entry seam"
```

---

## Completion Checklist

Spec §10 makes these completion conditions, not nice-to-haves:

- [ ] All twelve targets observed to **run** and accumulate coverage — not merely to compile (Task 5, Step 5).
- [ ] The replay test observed to **fail** on an unknown corpus directory, on a tripped invariant, and on missing seeds — then pass again after revert (Task 4, Step 5).
- [ ] `mise run lint && mise run test` green.
- [ ] `mise run lint-ocr && mise run test-ocr` green.
- [ ] `fuzz.yml` triggered once via `workflow_dispatch` and observed to pass (Task 6, Step 6).
- [ ] Any crash found has its reproducer committed under `fuzz/artifacts/<target>/` and is replayed by `cargo test`. Underlying fixes are follow-up work, not part of this plan.
