# DjVu Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a pure-Rust `DjvuAdapter` that converts a bundled DjVu document into kasane's IR, recovering structured text from the hidden OCR text layer and headings from the NAVM outline (font-height fallback otherwise).

**Architecture:** Hexagonal port over the `djvu-rs` crate. A thin `doc.rs` seam is the *only* place that touches `djvu-rs`; it exposes our own value types (`Zone`, `Bookmark`). `text.rs` and `outline.rs` are pure functions over those types — unit-tested with synthetic inputs and no files, exactly like `pdf/layout.rs` / `pdf/outline.rs`. `mod.rs` orchestrates open → per-page text→blocks → outline merge → `Document`.

**Tech Stack:** Rust, `djvu-rs` (IFF container, text layer, NAVM outline), `djvu-bzz` (transitive), the existing `kasane-ir` / `kasane-core` / `kasane-writer` crates.

## Global Constraints

- **Pure-Rust dependencies only** — no external binaries at runtime. `djvu-rs` and `djvu-bzz` are pinned in `Cargo.toml` (Dependabot-watched).
- **Minimal surface of `djvu-rs`** — use container/page-enumeration, text layer (`TXTa`/`TXTz`), and NAVM outline only. Do **not** invoke JB2/IW44 image decoding or page rendering in this cut.
- **Untrusted-input rigor** (same as the zip/PDF adapters): cap total extracted text bytes against `guard::MAX_TOTAL_BYTES` → `ParseError::Bomb`; bound recursion (depth cap + node budget) when walking the zone and outline trees; wrap every `djvu-rs` call at the `doc.rs` seam in `std::panic::catch_unwind` and map a panic to `ParseError::Malformed`; degrade-don't-die per page (a bad page/zone becomes a `Raw` note, parsing continues).
- **Provenance:** every emitted node carries page-native `source_pages: Some((n, n))`.
- **Indirect (multi-file) DjVu is rejected** with `ParseError::Malformed("indirect multi-file DjVu not supported; provide the bundled document")`. This message maps to **exit 1** (it does not contain the substrings `unsupported`/`DRM`/`encrypted`). Do not reword it to contain `unsupported`.
- **No encryption path** — DjVu has no mainstream encryption; there is nothing to detect or reject.
- Every task ends green under **`mise run lint && mise run test`** (`mise run lint` = `cargo fmt --check` + `cargo clippy --all-targets -D warnings`).
- Heading levels are clamped to **1–6** (IR range); inferred heading levels bucket to 1–3.

---

## File Structure

New module `crates/kasane-adapters/src/djvu/`:

| File | Responsibility |
|---|---|
| `mod.rs` | `DjvuAdapter` + `impl Adapter`; orchestration; the pure `page_nodes` helper. |
| `doc.rs` | The `djvu-rs` seam: `open`, `page_count`, `page_text`, `bookmarks`, `title`; our port types `DjvuDoc`, `Zone`, `ZoneKind`, `BBox`, `Bookmark`. |
| `text.rs` | Pure: zone tree → `Line`s → `Block`s; modal body height; height-based heading inference. |
| `outline.rs` | Pure: `Bookmark` tree → per-page `OutlineHeading`s. |

Modified: `crates/kasane-adapters/src/lib.rs`, `crates/kasane-adapters/Cargo.toml`, `crates/kasane-cli/src/main.rs`, `README.md`, `AGENTS.md`.
New fixtures: `tests/fixtures/djvu/sample.djvu` (+ `tests/fixtures/djvu/README.md`).

---

### Task 1: Dependency + module scaffold + port types

Establishes the crate dependency and the `doc.rs` port types (the interface every later task consumes). `adapter_for(Format::Djvu)` stays `Unsupported` until Task 7 wires the real adapter, so the crate keeps compiling.

**Files:**
- Modify: `crates/kasane-adapters/Cargo.toml` (add `djvu-rs`)
- Create: `crates/kasane-adapters/src/djvu/mod.rs`
- Create: `crates/kasane-adapters/src/djvu/doc.rs`
- Create: `crates/kasane-adapters/src/djvu/text.rs` (empty stub)
- Create: `crates/kasane-adapters/src/djvu/outline.rs` (empty stub)
- Modify: `crates/kasane-adapters/src/lib.rs:1-12` (add `mod djvu;`)

**Interfaces:**
- Produces (consumed by all later tasks):
  - `Zone { kind: ZoneKind, bbox: BBox, text: String, children: Vec<Zone> }`
  - `enum ZoneKind { Page, Column, Region, Para, Line, Word, Char, Other }`
  - `BBox { x0: f32, y0: f32, x1: f32, y1: f32 }` with `fn height(&self) -> f32`
  - `Bookmark { title: String, page: u32, children: Vec<Bookmark> }`

- [ ] **Step 1: Add the dependency**

Edit `crates/kasane-adapters/Cargo.toml`, in `[dependencies]` after the `png` line, add:

```toml
djvu-rs = "0.27"
```

- [ ] **Step 2: Create the port types with a unit test in `doc.rs`**

Create `crates/kasane-adapters/src/djvu/doc.rs`:

```rust
//! The sole seam over the `djvu-rs` crate. Everything else in `djvu/` consumes
//! the port types defined here, never `djvu-rs` directly.

/// An axis-aligned bounding box in page pixel coordinates.
#[derive(Clone, Copy, Debug)]
pub struct BBox {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl BBox {
    /// Box height; used as a font-size proxy for heading inference.
    pub fn height(&self) -> f32 {
        (self.y1 - self.y0).abs()
    }
}

/// A node in the DjVu hidden-text zone hierarchy, normalized to our own type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneKind {
    Page,
    Column,
    Region,
    Para,
    Line,
    Word,
    Char,
    Other,
}

/// A text-layer zone: a container (Page/Column/Region/Para/Line) or a leaf
/// (Word/Char). Leaves carry `text`; containers usually have `text == ""`.
#[derive(Clone, Debug)]
pub struct Zone {
    pub kind: ZoneKind,
    pub bbox: BBox,
    pub text: String,
    pub children: Vec<Zone>,
}

/// One NAVM outline entry, resolved to a 1-based destination page.
#[derive(Clone, Debug)]
pub struct Bookmark {
    pub title: String,
    pub page: u32,
    pub children: Vec<Bookmark>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_height_is_absolute_span() {
        let b = BBox { x0: 0.0, y0: 100.0, x1: 50.0, y1: 130.0 };
        assert!((b.height() - 30.0).abs() < 0.001);
        // Height is orientation-independent (DjVu y may increase downward).
        let flipped = BBox { x0: 0.0, y0: 130.0, x1: 50.0, y1: 100.0 };
        assert!((flipped.height() - 30.0).abs() < 0.001);
    }
}
```

- [ ] **Step 3: Create empty pure-module stubs**

Create `crates/kasane-adapters/src/djvu/text.rs` (just the module doc — functions are added in Tasks 5 and 6):

```rust
//! Pure: DjVu text-layer zones -> IR blocks. No `djvu-rs`, no files.
//! Functions are added in Tasks 5 (page_lines) and 6 (page_blocks).
```

Create `crates/kasane-adapters/src/djvu/outline.rs`:

```rust
//! Pure: NAVM `Bookmark` tree -> per-page headings. No `djvu-rs`, no files.
//! Functions are added in Task 3.
```

- [ ] **Step 4: Create `mod.rs` declaring the submodules**

Create `crates/kasane-adapters/src/djvu/mod.rs`:

```rust
mod doc;
mod outline;
mod text;
```

- [ ] **Step 5: Register the module in `lib.rs`**

In `crates/kasane-adapters/src/lib.rs`, add `mod djvu;` to the module list (keep alphabetical: after `mod detect;`, before `mod epub;`). Do **not** add a `pub use` yet and do **not** change `adapter_for` — that is Task 7.

- [ ] **Step 6: Verify it builds and the unit test passes**

Run: `cargo test -p kasane-adapters djvu::doc::tests::bbox_height_is_absolute_span -- --nocapture`
Expected: PASS (1 test). `cargo build -p kasane-adapters` succeeds. `djvu-rs` resolves in `Cargo.lock`.

- [ ] **Step 7: Lint**

Run: `mise run lint`
Expected: clean (dead-code warnings are acceptable *only* if `#[allow(dead_code)]` is not needed yet; if clippy flags the unused port types, add `#![allow(dead_code)]` at the top of `doc.rs` — it will be exercised by Task 3+).

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-adapters/Cargo.toml crates/kasane-adapters/Cargo.lock crates/kasane-adapters/src/djvu crates/kasane-adapters/src/lib.rs
git commit -m "feat(djvu): scaffold module + djvu-rs dep + port types"
```

---

### Task 2: Commit the end-to-end fixture

`text.rs`/`outline.rs` need no files, but `doc.rs` (Task 3) and the end-to-end test (Task 7) need one real DjVu. NAVM and (potentially) the text-zone tree are BZZ-encoded, so hand-emitting valid bytes with stdlib is unreliable — this is the single, spec-acknowledged break from the hermetic-generator convention (spec §6). Commit one small real `.djvu` produced out-of-band with DjVuLibre, with the exact commands documented so it is reproducible.

**Files:**
- Create: `tests/fixtures/djvu/sample.djvu` (binary)
- Create: `tests/fixtures/djvu/README.md`
- Test: `crates/kasane-adapters/src/detect.rs` (add a detection test)

- [ ] **Step 1: Produce `sample.djvu` (out-of-band, DjVuLibre)**

On a machine with DjVuLibre (`cjb2`, `djvused` — `apt install djvulibre-bin`), from `tests/fixtures/djvu/`:

```bash
# 1x1 black-on-white bitonal page -> a valid single-page FORM:DJVU
python3 -c "open('blank.pbm','wb').write(b'P4\n64 64\n' + bytes(64*8))"
cjb2 blank.pbm sample.djvu

# Text layer: one large 'heading' line + two body lines.
cat > txt.txt <<'EOF'
(page 0 0 64 64
 (line 4 40 60 60 "Chapter One")
 (line 4 20 60 30 "First body line.")
 (line 4 8  60 18 "Second body line."))
EOF

# NAVM outline: one bookmark to page 1.
cat > outline.txt <<'EOF'
(bookmarks
 ("Chapter One" "#1"))
EOF

djvused sample.djvu -e 'select 1; set-txt txt.txt; set-outline outline.txt; save'
rm blank.pbm txt.txt outline.txt
```

The resulting `sample.djvu` must be a bundled single-page document with a text layer (three lines, the first taller) and one NAVM bookmark. Keep it small (a few KB).

- [ ] **Step 2: Document reproduction**

Create `tests/fixtures/djvu/README.md` recording the commands above verbatim and noting: *this is the one fixture kasane cannot generate with stdlib because NAVM/TXT chunks are BZZ-encoded; regenerate with DjVuLibre if it needs to change.*

- [ ] **Step 3: Add a detection test**

In `crates/kasane-adapters/src/detect.rs`, inside `#[cfg(test)] mod tests`, add:

```rust
#[test]
fn detects_djvu_by_magic_and_ext() {
    let bytes = std::fs::read("../../tests/fixtures/djvu/sample.djvu").unwrap();
    assert!(matches!(detect(&bytes, Some("djvu")), Some(Format::Djvu)));
    // Magic alone (AT&T preamble) is enough, no hint.
    assert!(matches!(detect(&bytes, None), Some(Format::Djvu)));
}
```

- [ ] **Step 4: Run the detection test**

Run: `cargo test -p kasane-adapters detect::tests::detects_djvu_by_magic_and_ext`
Expected: PASS. (Confirms the fixture starts with the `AT&T` preamble `detect` already keys on.)

- [ ] **Step 5: Commit**

```bash
git add tests/fixtures/djvu crates/kasane-adapters/src/detect.rs
git commit -m "test(djvu): committed sample.djvu fixture + detection test"
```

---

### Task 3: `doc.rs` — the `djvu-rs` seam

Fill in the four seam functions. This is the **only** task that reads `djvu-rs`'s real API. The port-type signatures below are fixed; adapt the *internal* mapping to `djvu-rs` 0.27's actual names (verify against `https://docs.rs/djvu-rs/0.27`). Contain panics and bound extraction here.

**Files:**
- Modify: `crates/kasane-adapters/src/djvu/doc.rs`

**Interfaces:**
- Consumes: `crate::ParseError`, `crate::guard::MAX_TOTAL_BYTES`.
- Produces:
  - `pub struct DjvuDoc` (opaque wrapper over the `djvu-rs` document)
  - `pub fn open(bytes: &[u8]) -> Result<DjvuDoc, ParseError>`
  - `pub fn page_count(doc: &DjvuDoc) -> u32`
  - `pub fn page_text(doc: &DjvuDoc, page: u32) -> Option<Zone>` — 1-based `page`; `None` when the page has no text layer
  - `pub fn bookmarks(doc: &DjvuDoc) -> Vec<Bookmark>` — empty when absent
  - `pub fn title(doc: &DjvuDoc) -> Option<String>` — from metadata, else `None`

- [ ] **Step 1: Write failing seam tests against the fixture**

Add to the `#[cfg(test)] mod tests` block in `doc.rs`:

```rust
fn sample() -> DjvuDoc {
    open(&std::fs::read("../../tests/fixtures/djvu/sample.djvu").unwrap()).unwrap()
}

#[test]
fn opens_single_page_document() {
    assert_eq!(page_count(&sample()), 1);
}

#[test]
fn page_text_returns_a_zone_tree_with_the_lines() {
    let root = page_text(&sample(), 1).expect("sample has a text layer");
    // Flatten all leaf text; the three fixture lines must be present in order.
    let flat = flatten_text(&root);
    assert!(flat.contains("Chapter One"), "got: {flat}");
    assert!(flat.contains("First body line."), "got: {flat}");
    assert!(flat.contains("Second body line."), "got: {flat}");
}

#[test]
fn bookmarks_carry_the_outline_entry() {
    let bm = bookmarks(&sample());
    assert_eq!(bm.len(), 1);
    assert_eq!(bm[0].title, "Chapter One");
    assert_eq!(bm[0].page, 1);
}

#[test]
fn rejects_non_djvu_bytes() {
    assert!(matches!(open(b"not a djvu"), Err(ParseError::Malformed(_))));
}

// Test helper: concatenate leaf zone text in document order.
fn flatten_text(z: &Zone) -> String {
    let mut out = z.text.clone();
    for c in &z.children {
        let sub = flatten_text(c);
        if !sub.is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&sub);
        }
    }
    out
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p kasane-adapters djvu::doc::tests -- --nocapture`
Expected: FAIL — `open`/`page_count`/`page_text`/`bookmarks` not found (or `todo!`).

- [ ] **Step 3: Implement the seam**

Add above the tests in `doc.rs`. Replace the `djvu-rs` calls marked `// API:` with the crate's actual 0.27 API after checking docs.rs; keep the signatures and the guard/panic logic exactly as written.

```rust
use crate::guard::MAX_TOTAL_BYTES;
use crate::ParseError;
use std::panic::{catch_unwind, AssertUnwindSafe};

const MAX_ZONE_DEPTH: usize = 64;

/// Opaque handle over the parsed `djvu-rs` document.
pub struct DjvuDoc {
    inner: djvu_rs::DjVuDocument,
}

/// Run a `djvu-rs` call, turning a panic into `ParseError::Malformed` so a bug
/// in a young dependency degrades instead of crashing the process.
fn guard_panic<T>(f: impl FnOnce() -> Result<T, ParseError>) -> Result<T, ParseError> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => Err(ParseError::Malformed("djvu decode panicked".into())),
    }
}

pub fn open(bytes: &[u8]) -> Result<DjvuDoc, ParseError> {
    if bytes.len() as u64 > MAX_TOTAL_BYTES {
        return Err(ParseError::Bomb);
    }
    guard_panic(|| {
        // API: parse a bundled document from memory.
        let inner = djvu_rs::DjVuDocument::from_bytes(bytes)
            .map_err(|e| ParseError::Malformed(e.to_string()))?;
        // API: reject indirect (multi-file) documents — pages live in external files.
        if inner.is_indirect() {
            return Err(ParseError::Malformed(
                "indirect multi-file DjVu not supported; provide the bundled document".into(),
            ));
        }
        Ok(DjvuDoc { inner })
    })
}

pub fn page_count(doc: &DjvuDoc) -> u32 {
    // API: number of pages in the bundled document.
    doc.inner.page_count() as u32
}

pub fn page_text(doc: &DjvuDoc, page: u32) -> Option<Zone> {
    guard_panic(|| {
        // API: fetch the page's text layer; None/empty => no OCR text.
        let Some(layer) = doc.inner.page((page - 1) as usize).and_then(|p| p.text_layer())
        else {
            return Ok(None);
        };
        Ok(convert_zone(layer.root(), 0))
    })
    .ok()
    .flatten()
}

pub fn bookmarks(doc: &DjvuDoc) -> Vec<Bookmark> {
    guard_panic(|| {
        // API: NAVM outline root bookmarks; empty when absent.
        Ok(doc
            .inner
            .bookmarks()
            .iter()
            .filter_map(|b| convert_bookmark(b, 0))
            .collect())
    })
    .unwrap_or_default()
}

pub fn title(doc: &DjvuDoc) -> Option<String> {
    // API: document metadata title, if any.
    doc.inner
        .metadata_title()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Map a `djvu-rs` text zone into our `Zone`, bounding recursion depth.
fn convert_zone(z: &djvu_rs::text::TextZone, depth: usize) -> Option<Zone> {
    if depth > MAX_ZONE_DEPTH {
        return None;
    }
    let (x0, y0, x1, y1) = z.bbox(); // API: (x0,y0,x1,y1)
    let children = z
        .children()
        .iter()
        .filter_map(|c| convert_zone(c, depth + 1))
        .collect();
    Some(Zone {
        kind: map_kind(z.kind()), // API: zone type enum
        bbox: BBox { x0: x0 as f32, y0: y0 as f32, x1: x1 as f32, y1: y1 as f32 },
        text: z.text().unwrap_or_default().to_string(),
        children,
    })
}

fn map_kind(k: djvu_rs::text::ZoneType) -> ZoneKind {
    use djvu_rs::text::ZoneType as Z;
    match k {
        Z::Page => ZoneKind::Page,
        Z::Column => ZoneKind::Column,
        Z::Region => ZoneKind::Region,
        Z::Paragraph => ZoneKind::Para,
        Z::Line => ZoneKind::Line,
        Z::Word => ZoneKind::Word,
        Z::Character => ZoneKind::Char,
        _ => ZoneKind::Other,
    }
}

fn convert_bookmark(b: &djvu_rs::DjVuBookmark, depth: usize) -> Option<Bookmark> {
    if depth > MAX_ZONE_DEPTH {
        return None;
    }
    let title = b.title().trim().to_string();
    let page = b.page_number().unwrap_or(0) as u32; // API: resolved 1-based page
    let children = b
        .children()
        .iter()
        .filter_map(|c| convert_bookmark(c, depth + 1))
        .collect();
    if title.is_empty() {
        return None;
    }
    Some(Bookmark { title, page, children })
}
```

Note for the implementer: if `djvu-rs` 0.27's method names differ (e.g. `open`/`num_pages`/`get_page`/`outline`), adjust only the `// API:` lines; the public `pub fn` signatures and the guard/recursion logic are fixed by this plan. If `page_text` returns text only via `TXTz` and the fixture's `set-txt` produced `TXTa`, confirm both are handled; if not, regenerate the fixture so `djvused` writes `TXTz` (it does by default on `save`).

- [ ] **Step 4: Run the seam tests**

Run: `cargo test -p kasane-adapters djvu::doc::tests`
Expected: PASS (5 tests: bbox height + 4 seam tests).

- [ ] **Step 5: Lint & commit**

```bash
mise run lint
git add crates/kasane-adapters/src/djvu/doc.rs
git commit -m "feat(djvu): doc.rs seam over djvu-rs (open, pages, text, outline; guarded)"
```

---

### Task 4: `outline.rs` — NAVM tree → per-page headings

Pure function over `Bookmark`, mirroring `pdf/outline.rs`. Depth = heading level (clamped 1–6); bounded recursion.

**Files:**
- Modify: `crates/kasane-adapters/src/djvu/outline.rs`

**Interfaces:**
- Consumes: `super::doc::Bookmark`.
- Produces:
  - `pub struct OutlineHeading { pub level: u8, pub title: String }`
  - `pub fn outline_by_page(bookmarks: &[Bookmark]) -> std::collections::BTreeMap<u32, Vec<OutlineHeading>>`

- [ ] **Step 1: Write failing tests**

Replace the contents of `outline.rs` doc comment area by adding tests first (keep the module doc line):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::djvu::doc::Bookmark;

    fn bm(title: &str, page: u32, children: Vec<Bookmark>) -> Bookmark {
        Bookmark { title: title.into(), page, children }
    }

    #[test]
    fn nested_bookmarks_become_leveled_headings_by_page() {
        let tree = vec![bm(
            "Chapter One",
            1,
            vec![bm("Section A", 2, vec![]), bm("Section B", 3, vec![])],
        )];
        let map = outline_by_page(&tree);
        assert_eq!(map.get(&1).unwrap()[0].title, "Chapter One");
        assert_eq!(map.get(&1).unwrap()[0].level, 1);
        assert_eq!(map.get(&2).unwrap()[0].title, "Section A");
        assert_eq!(map.get(&2).unwrap()[0].level, 2);
        assert_eq!(map.get(&3).unwrap()[0].level, 2); // depth 2 -> level 2
    }

    #[test]
    fn drops_entries_with_no_page_or_empty_title() {
        let tree = vec![bm("", 1, vec![]), bm("Real", 0, vec![])];
        assert!(outline_by_page(&tree).is_empty());
    }

    #[test]
    fn deep_tree_is_bounded_not_infinite() {
        // Build a chain deeper than the cap; must terminate and clamp level to 6.
        let mut node = bm("leaf", 1, vec![]);
        for _ in 0..200 {
            node = bm("x", 1, vec![node]);
        }
        let map = outline_by_page(&[node]);
        assert!(map.get(&1).unwrap().iter().all(|h| (1..=6).contains(&h.level)));
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p kasane-adapters djvu::outline::tests`
Expected: FAIL — `outline_by_page` / `OutlineHeading` not found.

- [ ] **Step 3: Implement**

Prepend to `outline.rs` (below the module doc comment):

```rust
use super::doc::Bookmark;
use std::collections::BTreeMap;

const MAX_OUTLINE_DEPTH: usize = 64;

/// A heading derived from one NAVM bookmark.
#[derive(Clone, Debug)]
pub struct OutlineHeading {
    pub level: u8,
    pub title: String,
}

/// Map each 1-based page to the outline headings targeting it, in outline order.
/// Depth (1-based) becomes the heading level, clamped to the IR range 1–6.
/// An empty slice yields an empty map (never an error).
pub fn outline_by_page(bookmarks: &[Bookmark]) -> BTreeMap<u32, Vec<OutlineHeading>> {
    let mut map: BTreeMap<u32, Vec<OutlineHeading>> = BTreeMap::new();
    walk(bookmarks, 1, &mut map);
    map
}

fn walk(nodes: &[Bookmark], depth: usize, map: &mut BTreeMap<u32, Vec<OutlineHeading>>) {
    if depth > MAX_OUTLINE_DEPTH {
        return;
    }
    for b in nodes {
        let title = b.title.trim().to_string();
        if b.page > 0 && !title.is_empty() {
            let level = depth.clamp(1, 6) as u8;
            map.entry(b.page).or_default().push(OutlineHeading { level, title });
        }
        walk(&b.children, depth + 1, map);
    }
}
```

- [ ] **Step 4: Run to confirm pass**

Run: `cargo test -p kasane-adapters djvu::outline::tests`
Expected: PASS (3 tests).

- [ ] **Step 5: Lint & commit**

```bash
mise run lint
git add crates/kasane-adapters/src/djvu/outline.rs
git commit -m "feat(djvu): outline.rs — NAVM bookmarks to per-page headings"
```

---

### Task 5: `text.rs` (part 1) — zones → lines

Walk the zone tree in document order into flat `Line`s, preserving column/region reading order and marking paragraph starts. Bounded recursion.

**Files:**
- Modify: `crates/kasane-adapters/src/djvu/text.rs`

**Interfaces:**
- Consumes: `super::doc::{Zone, ZoneKind}`.
- Produces:
  - `pub struct Line { pub text: String, pub height: f32, pub para_start: bool }`
  - `pub fn page_lines(root: &Zone) -> Vec<Line>`

- [ ] **Step 1: Write failing tests**

Add to `text.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::djvu::doc::{BBox, Zone, ZoneKind};

    fn z(kind: ZoneKind, h: f32, text: &str, children: Vec<Zone>) -> Zone {
        Zone {
            kind,
            bbox: BBox { x0: 0.0, y0: 0.0, x1: 10.0, y1: h },
            text: text.into(),
            children,
        }
    }
    fn word(t: &str, h: f32) -> Zone {
        z(ZoneKind::Word, h, t, vec![])
    }
    fn line(h: f32, words: &[&str]) -> Zone {
        z(ZoneKind::Line, h, "", words.iter().map(|w| word(w, h)).collect())
    }

    #[test]
    fn concatenates_words_into_line_text_with_height() {
        let page = z(
            ZoneKind::Page,
            0.0,
            "",
            vec![z(
                ZoneKind::Para,
                0.0,
                "",
                vec![line(12.0, &["Hello", "world"])],
            )],
        );
        let lines = page_lines(&page);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Hello world");
        assert!((lines[0].height - 12.0).abs() < 0.01);
        assert!(lines[0].para_start);
    }

    #[test]
    fn first_line_of_each_paragraph_marks_para_start() {
        let page = z(
            ZoneKind::Page,
            0.0,
            "",
            vec![
                z(ZoneKind::Para, 0.0, "", vec![line(12.0, &["a"]), line(12.0, &["b"])]),
                z(ZoneKind::Para, 0.0, "", vec![line(12.0, &["c"])]),
            ],
        );
        let starts: Vec<bool> = page_lines(&page).iter().map(|l| l.para_start).collect();
        assert_eq!(starts, vec![true, false, true]);
    }

    #[test]
    fn columns_are_read_in_hierarchy_order() {
        // Two columns; hierarchy order (col1 then col2) is the reading order.
        let col = |t: &str| z(ZoneKind::Column, 0.0, "", vec![z(ZoneKind::Para, 0.0, "", vec![line(12.0, &[t])])]);
        let page = z(ZoneKind::Page, 0.0, "", vec![col("left"), col("right")]);
        let texts: Vec<String> = page_lines(&page).into_iter().map(|l| l.text).collect();
        assert_eq!(texts, vec!["left".to_string(), "right".to_string()]);
    }

    #[test]
    fn line_zone_with_direct_text_and_no_word_children_is_used() {
        // Some encoders put text directly on the Line zone.
        let page = z(ZoneKind::Page, 0.0, "", vec![z(ZoneKind::Line, 14.0, "Direct", vec![])]);
        let lines = page_lines(&page);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Direct");
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p kasane-adapters djvu::text::tests`
Expected: FAIL — `page_lines` / `Line` not found.

- [ ] **Step 3: Implement `page_lines`**

Prepend to `text.rs` (below the module doc comment):

```rust
use super::doc::{Zone, ZoneKind};

/// One visual line of recovered text plus a font-size proxy (zone height) and
/// whether it opens a paragraph (first line under a Para/Region/Column zone).
#[derive(Clone, Debug)]
pub struct Line {
    pub text: String,
    pub height: f32,
    pub para_start: bool,
}

const MAX_ZONE_DEPTH: usize = 64;

/// Flatten a page's zone tree into lines in document (reading) order. The zone
/// hierarchy already encodes columns/regions, so honoring its order yields
/// correct multi-column reading order without geometric re-sorting.
pub fn page_lines(root: &Zone) -> Vec<Line> {
    let mut lines = Vec::new();
    walk(root, 0, &mut true, &mut lines);
    lines
}

/// `pending_para_start` is set when we cross into a new paragraph container and
/// consumed by the next line emitted.
fn walk(z: &Zone, depth: usize, pending_para_start: &mut bool, out: &mut Vec<Line>) {
    if depth > MAX_ZONE_DEPTH {
        return;
    }
    match z.kind {
        ZoneKind::Line => {
            let text = line_text(z);
            if !text.is_empty() {
                out.push(Line {
                    text,
                    height: z.bbox.height(),
                    para_start: std::mem::replace(pending_para_start, false),
                });
            }
        }
        ZoneKind::Para | ZoneKind::Region | ZoneKind::Column => {
            *pending_para_start = true;
            for c in &z.children {
                walk(c, depth + 1, pending_para_start, out);
            }
        }
        _ => {
            for c in &z.children {
                walk(c, depth + 1, pending_para_start, out);
            }
        }
    }
}

/// Line text: direct text if present, else Word/Char children joined by spaces.
fn line_text(line: &Zone) -> String {
    let direct = line.text.trim();
    if !direct.is_empty() {
        return direct.to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    for w in &line.children {
        let t = w.text.trim();
        if !t.is_empty() {
            parts.push(t.to_string());
        }
    }
    parts.join(" ")
}
```

- [ ] **Step 4: Run to confirm pass**

Run: `cargo test -p kasane-adapters djvu::text::tests`
Expected: PASS (4 tests).

- [ ] **Step 5: Lint & commit**

```bash
mise run lint
git add crates/kasane-adapters/src/djvu/text.rs
git commit -m "feat(djvu): text.rs — zone tree to reading-order lines"
```

---

### Task 6: `text.rs` (part 2) — lines → blocks + heading inference

Add modal body height and block building: paragraphs from `para_start` boundaries, headings from line height when `infer_headings` is true (suppressed when an outline exists). Mirrors `pdf/layout.rs`.

**Files:**
- Modify: `crates/kasane-adapters/src/djvu/text.rs`

**Interfaces:**
- Consumes: `Line` (Task 5), `kasane_ir::{Block, BlockId, Inline}`.
- Produces:
  - `pub fn modal_body_height(pages: &[Vec<Line>]) -> f32`
  - `pub fn page_blocks(lines: &[Line], next_id: &mut u32, body_height: f32, infer_headings: bool) -> Vec<Block>`

- [ ] **Step 1: Write failing tests**

Add to the `#[cfg(test)] mod tests` block in `text.rs`:

```rust
    fn body_line(t: &str) -> Line {
        Line { text: t.into(), height: 12.0, para_start: false }
    }

    #[test]
    fn modal_body_height_is_the_commonest_rounded_height() {
        let pages = vec![vec![
            Line { text: "h".into(), height: 24.0, para_start: true },
            body_line("a"),
            body_line("b"),
        ]];
        assert!((modal_body_height(&pages) - 12.0).abs() < 0.01);
    }

    #[test]
    fn tall_line_becomes_heading_and_body_lines_merge() {
        let lines = vec![
            Line { text: "Big Title".into(), height: 24.0, para_start: true },
            Line { text: "Body one.".into(), height: 12.0, para_start: true },
            Line { text: "Body two.".into(), height: 12.0, para_start: false },
        ];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 12.0, true);
        match &blocks[0] {
            Block::Heading { level, inlines, .. } => {
                assert_eq!(*level, 1);
                assert_eq!(inline_text(inlines), "Big Title");
            }
            other => panic!("expected heading, got {other:?}"),
        }
        assert_eq!(para_text(&blocks[1]).as_deref(), Some("Body one. Body two."));
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn paragraph_boundary_splits_on_para_start() {
        let lines = vec![
            Line { text: "one".into(), height: 12.0, para_start: true },
            Line { text: "two".into(), height: 12.0, para_start: true },
        ];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 12.0, true);
        assert_eq!(blocks.len(), 2);
        assert_eq!(para_text(&blocks[0]).as_deref(), Some("one"));
        assert_eq!(para_text(&blocks[1]).as_deref(), Some("two"));
    }

    #[test]
    fn infer_headings_false_keeps_tall_lines_as_paragraphs() {
        let lines = vec![Line { text: "Big".into(), height: 24.0, para_start: true }];
        let mut id = 0u32;
        let blocks = page_blocks(&lines, &mut id, 12.0, false);
        assert!(matches!(blocks[0], Block::Para(_)));
    }

    fn inline_text(inls: &[Inline]) -> String {
        inls.iter()
            .map(|i| match i {
                Inline::Text(t) => t.clone(),
                _ => String::new(),
            })
            .collect()
    }
    fn para_text(b: &Block) -> Option<String> {
        if let Block::Para(inls) = b {
            Some(inline_text(inls))
        } else {
            None
        }
    }
```

Also add `use kasane_ir::{Block, Inline};` to the test module's `use` lines if not already imported via `super::*`.

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p kasane-adapters djvu::text::tests`
Expected: FAIL — `modal_body_height` / `page_blocks` not found.

- [ ] **Step 3: Implement**

Add to `text.rs` (after `page_lines`, before the tests). Add `use kasane_ir::{Block, BlockId, Inline};` to the top-of-file imports:

```rust
const HEADING_RATIO: f32 = 1.15;

/// Most common rounded line height across all pages — the document body height.
pub fn modal_body_height(pages: &[Vec<Line>]) -> f32 {
    use std::collections::HashMap;
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for page in pages {
        for l in page {
            *counts.entry(l.height.round() as i32).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(h, _)| h as f32)
        .unwrap_or(0.0)
}

/// Build blocks for one page. When `infer_headings`, a line ≥15% taller than the
/// body height becomes a heading (level bucketed 1–3); otherwise every line is
/// body text. Consecutive body lines merge into a paragraph, split on
/// `para_start`.
pub fn page_blocks(
    lines: &[Line],
    next_id: &mut u32,
    body_height: f32,
    infer_headings: bool,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut para: Vec<String> = Vec::new();

    let flush = |blocks: &mut Vec<Block>, para: &mut Vec<String>| {
        if !para.is_empty() {
            blocks.push(Block::Para(vec![Inline::Text(para.join(" "))]));
            para.clear();
        }
    };

    for l in lines {
        let is_heading =
            infer_headings && body_height > 0.0 && l.height >= body_height * HEADING_RATIO;
        if is_heading {
            flush(&mut blocks, &mut para);
            let id = BlockId(*next_id);
            *next_id += 1;
            blocks.push(Block::Heading {
                level: heading_level(l.height, body_height),
                id,
                inlines: vec![Inline::Text(l.text.clone())],
            });
        } else {
            if l.para_start {
                flush(&mut blocks, &mut para);
            }
            para.push(l.text.clone());
        }
    }
    flush(&mut blocks, &mut para);
    blocks
}

/// Bucket a heading height into levels 1–3 by how far it exceeds the body.
fn heading_level(height: f32, body: f32) -> u8 {
    let ratio = if body > 0.0 { height / body } else { 1.0 };
    if ratio >= 1.8 {
        1
    } else if ratio >= 1.4 {
        2
    } else {
        3
    }
}
```

- [ ] **Step 4: Run to confirm pass**

Run: `cargo test -p kasane-adapters djvu::text::tests`
Expected: PASS (8 tests total in the module).

- [ ] **Step 5: Lint & commit**

```bash
mise run lint
git add crates/kasane-adapters/src/djvu/text.rs
git commit -m "feat(djvu): text.rs — modal body height + block/heading building"
```

---

### Task 7: `mod.rs` — orchestration + `DjvuAdapter` + wiring

Assemble the pipeline, wire `adapter_for`/`lib.rs`, and expose a pure `page_nodes` helper (so the no-text-page path is unit-testable without a file). Enforce the total-bytes bomb guard here.

**Files:**
- Modify: `crates/kasane-adapters/src/djvu/mod.rs`
- Modify: `crates/kasane-adapters/src/lib.rs` (`pub use`, `adapter_for`)

**Interfaces:**
- Consumes: `doc::{open, page_count, page_text, bookmarks, title, Zone}`, `outline::{outline_by_page, OutlineHeading}`, `text::{page_lines, modal_body_height, page_blocks, Line}`, `kasane_ir::*`, `crate::{Adapter, ParseError, guard::MAX_TOTAL_BYTES}`.
- Produces: `pub struct DjvuAdapter` implementing `Adapter`.

- [ ] **Step 1: Write failing tests**

Replace `mod.rs` contents with the module declarations plus a test block (implementation added next step):

```rust
mod doc;
mod outline;
mod text;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Adapter;
    use doc::{BBox, Zone, ZoneKind};
    use kasane_ir::{Block, Inline};
    use outline::OutlineHeading;

    fn sample() -> kasane_ir::Document {
        let bytes = std::fs::read("../../tests/fixtures/djvu/sample.djvu").unwrap();
        DjvuAdapter.parse(&bytes, "sample.djvu").unwrap().0
    }
    fn text(inls: &[Inline]) -> String {
        inls.iter()
            .map(|i| match i {
                Inline::Text(t) => t.clone(),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn end_to_end_outline_heading_and_page_provenance() {
        let doc = sample();
        assert_eq!(doc.meta.source_format, "djvu");
        let heads: Vec<String> = doc
            .nodes
            .iter()
            .filter_map(|n| match &n.block {
                Block::Heading { inlines, .. } => Some(text(inlines)),
                _ => None,
            })
            .collect();
        assert!(heads.contains(&"Chapter One".to_string()), "heads: {heads:?}");
        // Page-native provenance on every node.
        assert!(doc.nodes.iter().all(|n| n.prov.source_pages == Some((1, 1))));
    }

    #[test]
    fn no_text_page_emits_a_raw_note_not_an_error() {
        // Pure helper: a page with no text layer and no outline heading.
        let mut id = 0u32;
        let nodes = page_nodes(3, None, &[], &mut id, 0.0, true);
        assert_eq!(nodes.len(), 1);
        assert!(matches!(&nodes[0].block, Block::Raw { note } if note.contains("no text layer")));
        assert_eq!(nodes[0].prov.source_pages, Some((3, 3)));
    }

    #[test]
    fn page_with_outline_heading_suppresses_height_inference() {
        // A tall line + an outline heading: only the outline heading is a Heading.
        let root = Zone {
            kind: ZoneKind::Page,
            bbox: BBox { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0 },
            text: String::new(),
            children: vec![Zone {
                kind: ZoneKind::Line,
                bbox: BBox { x0: 0.0, y0: 0.0, x1: 10.0, y1: 24.0 },
                text: "Tall body".into(),
                children: vec![],
            }],
        };
        let headings = [OutlineHeading { level: 1, title: "Real".into() }];
        let mut id = 0u32;
        let nodes = page_nodes(1, Some(&root), &headings, &mut id, 12.0, false);
        let heads: Vec<String> = nodes
            .iter()
            .filter_map(|n| match &n.block {
                Block::Heading { inlines, .. } => Some(text(inlines)),
                _ => None,
            })
            .collect();
        assert_eq!(heads, vec!["Real".to_string()]);
        assert!(nodes.iter().any(|n| matches!(&n.block, Block::Para(p) if text(p) == "Tall body")));
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p kasane-adapters djvu::tests`
Expected: FAIL — `DjvuAdapter` / `page_nodes` not found.

- [ ] **Step 3: Implement orchestration**

Insert between the `mod` lines and the test block in `mod.rs`:

```rust
use crate::guard::MAX_TOTAL_BYTES;
use crate::{Adapter, ParseError};
use doc::Zone;
use kasane_ir::*;
use outline::{outline_by_page, OutlineHeading};
use text::{modal_body_height, page_blocks, page_lines, Line};

pub struct DjvuAdapter;

impl Adapter for DjvuAdapter {
    fn parse(&self, bytes: &[u8], source_path: &str) -> Result<(Document, AssetBag), ParseError> {
        let djvu = doc::open(bytes)?;
        let n = doc::page_count(&djvu);
        let outline = outline_by_page(&doc::bookmarks(&djvu));
        let has_outline = !outline.is_empty();

        // First pass: per-page lines (needed for the doc-wide body height).
        let page_lines_all: Vec<(u32, Vec<Line>)> = (1..=n)
            .map(|p| (p, doc::page_text(&djvu, p).map(|z| page_lines(&z)).unwrap_or_default()))
            .collect();
        let body_height =
            modal_body_height(&page_lines_all.iter().map(|(_, l)| l.clone()).collect::<Vec<_>>());

        let mut nodes = Vec::new();
        let mut next_id = 0u32;
        let mut budget = MAX_TOTAL_BYTES;

        for (p, lines) in &page_lines_all {
            // Total-bytes bomb guard across recovered text.
            let page_bytes: usize = lines.iter().map(|l| l.text.len()).sum();
            budget = budget.saturating_sub(page_bytes as u64);
            if budget == 0 && page_bytes > 0 {
                return Err(ParseError::Bomb);
            }
            let empty = Vec::new();
            let headings = outline.get(p).unwrap_or(&empty);
            let root = if lines.is_empty() { None } else { Some(()) };
            // Rebuild a Zone-free path: page_nodes takes the already-computed lines.
            nodes.extend(page_nodes_from_lines(
                *p,
                lines,
                headings,
                &mut next_id,
                body_height,
                !has_outline,
                root.is_some(),
            ));
        }

        let out = Document {
            meta: DocMeta {
                title: doc::title(&djvu).unwrap_or_else(|| derive_title(source_path)),
                authors: vec![],
                language: None,
                source_format: "djvu".into(),
                source_path: source_path.to_string(),
            },
            nodes,
        };
        Ok((out, AssetBag::default()))
    }
}

/// Build the nodes for one page from its lines. `has_text` distinguishes a page
/// with an (empty-after-filtering) text layer from one with none.
fn page_nodes_from_lines(
    page: u32,
    lines: &[Line],
    headings: &[OutlineHeading],
    next_id: &mut u32,
    body_height: f32,
    infer_headings: bool,
    has_text: bool,
) -> Vec<Node> {
    let prov = Provenance { source_pages: Some((page, page)), source_href: None };
    let mut out = Vec::new();

    for h in headings {
        let id = BlockId(*next_id);
        *next_id += 1;
        out.push(Node {
            block: Block::Heading {
                level: h.level,
                id,
                inlines: vec![Inline::Text(h.title.clone())],
            },
            prov: prov.clone(),
        });
    }

    let blocks = page_blocks(lines, next_id, body_height, infer_headings);
    let had_blocks = !blocks.is_empty();
    for b in blocks {
        out.push(Node { block: b, prov: prov.clone() });
    }

    // No recoverable text and no outline heading on this page -> honest note.
    if !has_text && headings.is_empty() && !had_blocks {
        out.push(Node {
            block: Block::Raw { note: "no text layer; OCR not enabled".into() },
            prov: prov.clone(),
        });
    }
    out
}

/// Test-facing wrapper: assemble a page from an optional text-layer zone.
#[cfg(test)]
fn page_nodes(
    page: u32,
    text_root: Option<&Zone>,
    headings: &[OutlineHeading],
    next_id: &mut u32,
    body_height: f32,
    infer_headings: bool,
) -> Vec<Node> {
    let lines = text_root.map(page_lines).unwrap_or_default();
    page_nodes_from_lines(
        page,
        &lines,
        headings,
        next_id,
        body_height,
        infer_headings,
        text_root.is_some(),
    )
}

/// Title from the source filename stem (DjVu metadata title handled in `doc.rs`).
fn derive_title(source_path: &str) -> String {
    source_path
        .rsplit(['/', '\\'])
        .next()
        .and_then(|f| f.strip_suffix(".djvu").or_else(|| f.strip_suffix(".djv")).or(Some(f)))
        .unwrap_or("document")
        .to_string()
}
```

Implementer note: the `root`/`has_text` plumbing above keeps the no-text path (`page_text` → `None`) distinct from an empty-but-present layer. Confirm `page_nodes` (test wrapper) and `page_nodes_from_lines` share one code path so the tests exercise real behavior.

- [ ] **Step 4: Wire `lib.rs`**

In `crates/kasane-adapters/src/lib.rs`:
- Add `pub use djvu::DjvuAdapter;` next to the other adapter re-exports.
- In `adapter_for`, change the DjVu arm from `Format::Djvu => Err(ParseError::Unsupported),` to:

```rust
        Format::Djvu => Ok(Box::new(DjvuAdapter)),
```

- [ ] **Step 5: Run adapter tests + confirm pass**

Run: `cargo test -p kasane-adapters djvu::`
Expected: PASS (all djvu module tests, incl. the 3 orchestration tests).

- [ ] **Step 6: Lint & commit**

```bash
mise run lint
git add crates/kasane-adapters/src/djvu/mod.rs crates/kasane-adapters/src/lib.rs
git commit -m "feat(djvu): mod.rs orchestration + register DjvuAdapter"
```

---

### Task 8: End-to-end test + docs

Prove the whole pipeline (`detect → parse → structure → write_tree`) and update user-facing docs. Mirrors the PPTX/PDF end-to-end tests in `lib.rs`.

**Files:**
- Modify: `crates/kasane-adapters/src/lib.rs` (add an end-to-end test)
- Modify: `crates/kasane-cli/src/main.rs:12` (help blurb)
- Modify: `README.md`
- Modify: `AGENTS.md`

- [ ] **Step 1: Write the end-to-end test**

In `crates/kasane-adapters/src/lib.rs`, in `#[cfg(test)] mod tests`, add:

```rust
    #[test]
    fn end_to_end_djvu_fixture_to_sitetree() {
        let bytes = std::fs::read("../../tests/fixtures/djvu/sample.djvu").unwrap();
        assert!(matches!(detect(&bytes, Some("djvu")), Some(Format::Djvu)));

        let (doc, assets) = DjvuAdapter.parse(&bytes, "sample.djvu").unwrap();
        assert_eq!(doc.meta.source_format, "djvu");

        let site = kasane_core::structure(doc, &kasane_core::Options::default());
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("djvuout");
        kasane_writer::write_tree(&site, &assets, &out, false).unwrap();
        assert!(out.join("index.md").exists());
    }
```

- [ ] **Step 2: Run the end-to-end test**

Run: `cargo test -p kasane-adapters end_to_end_djvu_fixture_to_sitetree`
Expected: PASS.

- [ ] **Step 3: Update the CLI help blurb**

In `crates/kasane-cli/src/main.rs`, change the input doc comment (line ~12):

```rust
    /// Input document (EPUB, PPTX, MOBI, AZW3, PDF, DjVu supported in this build)
    input: PathBuf,
```

(No exit-code change: indirect DjVu maps to exit 1 via the existing `exit_code_for`.)

- [ ] **Step 4: Update README**

In `README.md`:
- Change the opening line from `EPUB, PPTX, MOBI, AZW3, PDF today; DJVU coming` to `EPUB, PPTX, MOBI, AZW3, PDF, DjVu today`.
- Under "Known limitations (this build)", add:

```markdown
- DjVu conversion recovers the file's hidden OCR text layer (structured by the
  document's own zones, so multi-column reading order is preserved) and its NAVM
  outline as headings; with no outline, headings are inferred from line height.
  Scanned page images (JB2/IW44) are not rendered — a page with no text layer
  becomes a placeholder note until a later `-F` rendering/OCR feature lands. Only
  bundled DjVu documents are supported; indirect multi-file documents are
  rejected (exit code 1). Tables become paragraphs.
```

- [ ] **Step 5: Update AGENTS.md codebase map**

In `AGENTS.md`, extend the `crates/kasane-adapters` bullet to mention the DjVu adapter, e.g. append:

```
The DjVu adapter (`djvu/`) builds on `djvu-rs`: `doc.rs` is the sole seam over the crate (container, text layer, NAVM outline) with panic/bomb guards; `text.rs` turns the hidden-text zone hierarchy into reading-order lines and infers headings by line height; `outline.rs` maps NAVM bookmarks to per-page headings. Image layers (JB2/IW44) are intentionally not decoded in this build; the one committed binary fixture `tests/fixtures/djvu/sample.djvu` is regenerated with DjVuLibre (see its README).
```

- [ ] **Step 6: Full verification**

Run: `mise run lint && mise run test`
Expected: all green across the workspace.

- [ ] **Step 7: Commit**

```bash
git add crates/kasane-adapters/src/lib.rs crates/kasane-cli/src/main.rs README.md AGENTS.md
git commit -m "test(djvu): end-to-end pipeline + docs for DjVu support"
```

---

## Self-Review

**Spec coverage:**
- Foundation (djvu-rs, minimal surface) → Tasks 1, 3. ✅
- Outline-driven headings → Task 4 + Task 7 splice. ✅
- Line-height fallback → Task 6 + Task 7 `infer_headings = !has_outline`. ✅
- Zone-hierarchy reading order / multi-column → Task 5 (`columns_are_read_in_hierarchy_order`). ✅
- Paragraphs from zones → Task 5 `para_start` + Task 6 grouping. ✅
- Page-native provenance → Task 7. ✅
- No-text page → Raw note → Task 7 (`no_text_page_emits_a_raw_note_not_an_error`). ✅
- Indirect rejection (exit 1) → Task 3 `open` + Global Constraints. ✅
- No encryption path → nothing added; explicit in constraints. ✅
- Untrusted-input rigor (bomb bytes, recursion caps, catch_unwind, degrade) → Task 3 (`guard_panic`, depth caps), Task 4/5 depth caps, Task 7 byte budget. ✅
- Testing: unit (Tasks 4–6), seam (Task 3), adapter + no-text (Task 7), end-to-end (Task 8). ✅
- Wiring (adapter_for, lib.rs, Cargo.toml, CLI, README, AGENTS) → Tasks 1, 7, 8. ✅

**Placeholder scan:** No `TBD`/`add error handling`/`similar to`. The `// API:` markers in Task 3 are explicit, named verification points (djvu-rs 0.27 method names), not vague placeholders — each has a concrete default and a fixed surrounding signature.

**Type consistency:** `Zone`/`ZoneKind`/`BBox`/`Bookmark` defined in Task 1, consumed unchanged in 3/4/5/7. `Line { text, height, para_start }` from Task 5 used in Task 6/7. `OutlineHeading { level, title }` from Task 4 used in Task 7. `page_blocks(lines, next_id, body_height, infer_headings)` signature identical in Tasks 6 and 7. `page_nodes_from_lines` and the `#[cfg(test)] page_nodes` wrapper share one code path. ✅

**Known risk carried into execution:** the `djvu-rs` 0.27 API names (Task 3) and whether the DjVuLibre-generated fixture's text chunk parses via `page_text` (Task 3, Step 3 note). Both are isolated to `doc.rs` + the fixture; the pure logic (Tasks 4–6, ~80% of the code) is unaffected and fully tested without files.
