# PPTX Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a PPTX input adapter to kasane so `kasane deck.pptx -o out` emits the same progressive-disclosure Markdown tree the EPUB path already produces — one section per slide, with lists, tables, hyperlinks, images, and speaker notes.

**Architecture:** A new `pptx` module in `kasane-adapters` behind the existing `Adapter` trait. It reads the PPTX zip, orders slides via `presentation.xml` relationships (not filenames), parses each slide's DrawingML with a `quick-xml` state machine into IR blocks, extracts referenced media into the `AssetBag`, and appends speaker notes. The core engine, writer, and CLI are reused unchanged except a one-line dispatch arm. A shared `ziputil` helper (lifted from the EPUB adapter) provides bomb-guarded zip reads.

**Tech Stack:** Rust 2021, `quick-xml 0.36`, `zip 2`, `thiserror 1` (all already dependencies of `kasane-adapters`). No new crates.

## Global Constraints

- **Pure Rust, no new dependencies.** Everything is built on the crate's existing `quick-xml`/`zip`.
- **Rust toolchain pinned** at `1.83.0` via `mise.toml`.
- **Untrusted input boundary.** Every zip entry read goes through the bomb guard (**200:1** max expansion ratio, **512 MiB** absolute aggregate cap across the whole archive). Relationship targets are normalized and **confined to the archive root** — any `..` escape is rejected. Media filenames are sanitized before writing.
- **XXE-safe:** `quick-xml` is used with no entity/DTD expansion (it does not resolve external entities by default; do not enable any expansion).
- **Cross-references stay symbolic in the IR.** PPTX cross-slide links are out of scope for v1 and degrade to plain text; external hyperlinks become `RefTarget::External`.
- **Degrade, don't die:** a slide that fails to parse still emits its heading plus a `Raw` note; a broken rel drops one image/link, never the slide; the presentation is never aborted on one bad slide.
- **`clippy -D warnings` and `rustfmt`** must pass; every task ends green under `just lint && just test`.

---

## File Structure

```
crates/kasane-adapters/src/
  ziputil.rs        NEW — shared bomb-guarded zip reads + aggregate byte counter
  guard.rs          MODIFY — add resolve_rel(base_dir, target)
  epub/mod.rs       MODIFY — delegate read_entry to ziputil (no behavior change)
  pptx/
    mod.rs          NEW — PptxAdapter: orchestrates the whole parse
    rels.rs         NEW — presentation slide order + .rels parsing + SlideRels
    slide.rs        NEW — DrawingML slide XML -> Vec<Block>
  lib.rs            MODIFY — register PptxAdapter in adapter_for()
crates/kasane-adapters/examples/
  gen_minimal_pptx.rs  NEW — regenerates the checked-in fixture (reproducible)
crates/kasane-cli/src/main.rs   MODIFY — help text "EPUB, PPTX"
tests/fixtures/pptx/minimal.pptx  NEW — tiny hand-built fixture (checked in)
```

---

## Type Reference (locked signatures)

Defined across Tasks 1–3, consumed by later tasks. Later tasks rely on exactly these names.

```rust
// ziputil.rs  (Task 1)  — pub(crate)
pub(crate) type ZipReader<'a> = zip::ZipArchive<std::io::Cursor<&'a [u8]>>;
pub(crate) fn read_entry_bytes(zip: &mut ZipReader, name: &str, total_read: &mut u64)
    -> Result<Vec<u8>, crate::ParseError>;
pub(crate) fn read_entry_string(zip: &mut ZipReader, name: &str, total_read: &mut u64)
    -> Result<String, crate::ParseError>;

// guard.rs  (Task 2)
/// Resolve `target` (which may contain `..`) against `base_dir`, normalize, and
/// confine to the archive root. Returns None if it escapes root or resolves empty.
pub fn resolve_rel(base_dir: &str, target: &str) -> Option<String>;

// rels.rs  (Task 3)
pub struct Rel { pub id: String, pub ty: String, pub target: String, pub external: bool }
pub fn parse_rels(xml: &str) -> Vec<Rel>;
/// r:id values from <p:sldIdLst> in display order.
pub fn parse_slide_order(presentation_xml: &str) -> Vec<String>;

pub enum RelTarget { External(String), Internal(String) } // Internal = absolute archive path
pub struct SlideRels(pub std::collections::HashMap<String, RelTarget>);
impl SlideRels {
    pub fn empty() -> Self;
    pub fn get(&self, id: &str) -> Option<&RelTarget>;
}

// slide.rs  (Tasks 4–7)
pub fn slide_to_blocks(xml: &str, next_id: &mut u32, rels: &SlideRels) -> Vec<kasane_ir::Block>;
pub fn notes_to_blocks(xml: &str) -> Vec<kasane_ir::Block>;
```

---

### Task 1: Shared zip-read helper + EPUB refactor

**Files:**
- Create: `crates/kasane-adapters/src/ziputil.rs`
- Modify: `crates/kasane-adapters/src/lib.rs` (add `mod ziputil;`)
- Modify: `crates/kasane-adapters/src/epub/mod.rs` (delegate `read_entry`)
- Test: `crates/kasane-adapters/src/ziputil.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: `crate::ParseError`, `crate::guard`.
- Produces: `ZipReader`, `read_entry_bytes`, `read_entry_string` (see Type Reference).

- [ ] **Step 1: Write the failing test**

Create `crates/kasane-adapters/src/ziputil.rs`:
```rust
use crate::ParseError;
use std::io::Read;

pub(crate) type ZipReader<'a> = zip::ZipArchive<std::io::Cursor<&'a [u8]>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_zip(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        w.start_file(name, opts).unwrap();
        std::io::Write::write_all(&mut w, contents).unwrap();
        w.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn reads_bytes_and_string_and_accumulates() {
        let bytes = tiny_zip("a.txt", b"hello");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut total = 0u64;
        let b = read_entry_bytes(&mut zip, "a.txt", &mut total).unwrap();
        assert_eq!(b, b"hello");
        assert_eq!(total, 5);
        let s = read_entry_string(&mut zip, "a.txt", &mut total).unwrap();
        assert_eq!(s, "hello");
        assert_eq!(total, 10);
    }

    #[test]
    fn rejects_once_aggregate_cap_exceeded() {
        let bytes = tiny_zip("b.txt", b"hello");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut total = crate::guard::MAX_TOTAL_BYTES - 2;
        let r = read_entry_bytes(&mut zip, "b.txt", &mut total);
        assert!(matches!(r, Err(ParseError::Bomb)));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-adapters ziputil`
Expected: FAIL — `read_entry_bytes` / `read_entry_string` not found.

- [ ] **Step 3: Implement the shared helper**

Add above the `#[cfg(test)]` block in `crates/kasane-adapters/src/ziputil.rs`:
```rust
pub(crate) fn read_entry_bytes(
    zip: &mut ZipReader,
    name: &str,
    total_read: &mut u64,
) -> Result<Vec<u8>, ParseError> {
    let f = zip
        .by_name(name)
        .map_err(|_| ParseError::Malformed(format!("missing entry: {name}")))?;
    // Reject on declared metadata first (cheap), then bound the ACTUAL read so a
    // lying/small declared size cannot lead to an unbounded decompression.
    if !crate::guard::check_expansion(f.compressed_size(), f.size()) {
        return Err(ParseError::Bomb);
    }
    let cap = crate::guard::MAX_TOTAL_BYTES;
    let mut buf = Vec::new();
    f.take(cap + 1)
        .read_to_end(&mut buf)
        .map_err(|e| ParseError::Malformed(e.to_string()))?;
    if buf.len() as u64 > cap {
        return Err(ParseError::Bomb);
    }
    // MAX_TOTAL_BYTES is an absolute cap on the whole archive's decompressed output,
    // not a per-entry budget: accumulate across every call and stop once the running
    // total would exceed it, on top of the per-entry bound above.
    *total_read += buf.len() as u64;
    if *total_read > cap {
        return Err(ParseError::Bomb);
    }
    Ok(buf)
}

pub(crate) fn read_entry_string(
    zip: &mut ZipReader,
    name: &str,
    total_read: &mut u64,
) -> Result<String, ParseError> {
    let buf = read_entry_bytes(zip, name, total_read)?;
    String::from_utf8(buf).map_err(|e| ParseError::Malformed(e.to_string()))
}
```

Add `mod ziputil;` to `crates/kasane-adapters/src/lib.rs` (next to the other `mod` lines).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kasane-adapters ziputil`
Expected: PASS.

- [ ] **Step 5: Refactor EPUB to delegate**

In `crates/kasane-adapters/src/epub/mod.rs`, delete the private `read_entry` function (the whole `fn read_entry(...) -> Result<String, ParseError> { ... }` block) and its now-unused `use std::io::Read;`. Replace the three call sites `read_entry(&mut zip, X, &mut total_read)` with `crate::ziputil::read_entry_string(&mut zip, X, &mut total_read)`.

The two EPUB tests that exercised `read_entry` directly (`read_entry_accumulates_total_read_across_calls`, `read_entry_rejects_once_aggregate_cap_is_exceeded`) now live in `ziputil.rs` — delete them from `epub/mod.rs` along with the `tiny_zip_with_entry` helper. Keep the two `find_opf_path_*` tests.

- [ ] **Step 6: Run the whole suite to verify no regression**

Run: `just lint && just test`
Expected: PASS; no clippy warnings; the EPUB golden test `parses_minimal_epub_to_ir` still passes.

- [ ] **Step 7: Commit**

```bash
git add crates/kasane-adapters/src/ziputil.rs crates/kasane-adapters/src/lib.rs crates/kasane-adapters/src/epub/mod.rs
git commit -m "refactor(adapters): lift bomb-guarded zip reads into shared ziputil"
```

---

### Task 2: `resolve_rel` path guard

**Files:**
- Modify: `crates/kasane-adapters/src/guard.rs`
- Test: `crates/kasane-adapters/src/guard.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub fn resolve_rel(base_dir: &str, target: &str) -> Option<String>`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/kasane-adapters/src/guard.rs`:
```rust
#[test]
fn resolve_rel_normalizes_and_confines() {
    // media referenced from a slide: ../media/image1.png relative to ppt/slides
    assert_eq!(resolve_rel("ppt/slides", "../media/image1.png").as_deref(),
               Some("ppt/media/image1.png"));
    // slide referenced from presentation rels: base ppt
    assert_eq!(resolve_rel("ppt", "slides/slide1.xml").as_deref(),
               Some("ppt/slides/slide1.xml"));
    // "." and empty segments are ignored
    assert_eq!(resolve_rel("ppt/slides", "./../media/./i.png").as_deref(),
               Some("ppt/media/i.png"));
    // leading slash is package-absolute (from archive root)
    assert_eq!(resolve_rel("ppt/slides", "/ppt/media/i.png").as_deref(),
               Some("ppt/media/i.png"));
    // escaping the root is rejected
    assert_eq!(resolve_rel("ppt", "../../etc/passwd"), None);
    // resolving to empty (the root itself) is rejected
    assert_eq!(resolve_rel("ppt", ".."), None);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-adapters resolve_rel`
Expected: FAIL — `resolve_rel` not found.

- [ ] **Step 3: Implement `resolve_rel`**

Add to `crates/kasane-adapters/src/guard.rs` (above the `tests` module):
```rust
/// Resolve a relationship `target` (which may contain `..`) against `base_dir`,
/// normalizing `.`/`..` and confining the result to the archive root. A leading
/// `/` makes the target package-absolute (resolved from root). Returns `None` if
/// the target escapes the root or resolves to nothing.
pub fn resolve_rel(base_dir: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = if target.starts_with('/') || base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return None; // escaped the archive root
                }
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kasane-adapters resolve_rel && just lint`
Expected: PASS; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-adapters/src/guard.rs
git commit -m "feat(adapters): add resolve_rel path guard for OPC relationships"
```

---

### Task 3: Slide ordering & relationship parsing

**Files:**
- Create: `crates/kasane-adapters/src/pptx/mod.rs` (module wiring only)
- Create: `crates/kasane-adapters/src/pptx/rels.rs`
- Modify: `crates/kasane-adapters/src/lib.rs` (add `mod pptx;`)
- Test: `crates/kasane-adapters/src/pptx/rels.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: `crate::guard::resolve_rel`, `quick_xml`.
- Produces: `Rel`, `parse_rels`, `parse_slide_order`, `RelTarget`, `SlideRels` (see Type Reference).

- [ ] **Step 1: Create the module skeleton**

Create `crates/kasane-adapters/src/pptx/mod.rs`:
```rust
mod rels;
mod slide;
```

Add `mod pptx;` to `crates/kasane-adapters/src/lib.rs` (next to `mod epub;`). Create an empty `crates/kasane-adapters/src/pptx/slide.rs` with a single line so the module compiles:
```rust
// DrawingML slide parser — implemented in Tasks 4–7.
```

- [ ] **Step 2: Write the failing test**

Create `crates/kasane-adapters/src/pptx/rels.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_slide_order_from_sldidlst() {
        // Note the r:id order is 3 then 2 — display order differs from filename order.
        let xml = r#"<p:presentation xmlns:r="x">
          <p:sldIdLst>
            <p:sldId id="256" r:id="rId3"/>
            <p:sldId id="257" r:id="rId2"/>
          </p:sldIdLst></p:presentation>"#;
        assert_eq!(parse_slide_order(xml), vec!["rId3", "rId2"]);
    }

    #[test]
    fn parses_relationships_with_targetmode() {
        let xml = r#"<Relationships>
          <Relationship Id="rId2" Type="http://x/slide" Target="slides/slide1.xml"/>
          <Relationship Id="rId3" Type="http://x/hyperlink" Target="https://e.com" TargetMode="External"/>
        </Relationships>"#;
        let rels = parse_rels(xml);
        assert_eq!(rels.len(), 2);
        let hy = rels.iter().find(|r| r.id == "rId3").unwrap();
        assert!(hy.external);
        assert!(hy.ty.ends_with("hyperlink"));
        assert_eq!(hy.target, "https://e.com");
    }

    #[test]
    fn slide_rels_resolves_internal_vs_external() {
        let xml = r#"<Relationships>
          <Relationship Id="rId2" Type="http://x/image" Target="../media/image1.png"/>
          <Relationship Id="rId3" Type="http://x/hyperlink" Target="https://e.com" TargetMode="External"/>
        </Relationships>"#;
        let sr = SlideRels::from_rels(parse_rels(xml), "ppt/slides");
        assert!(matches!(sr.get("rId2"), Some(RelTarget::Internal(p)) if p == "ppt/media/image1.png"));
        assert!(matches!(sr.get("rId3"), Some(RelTarget::External(u)) if u == "https://e.com"));
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p kasane-adapters rels`
Expected: FAIL — `parse_slide_order` etc. not found.

- [ ] **Step 4: Implement rels parsing**

Add above the `tests` module in `crates/kasane-adapters/src/pptx/rels.rs`:
```rust
use crate::guard::resolve_rel;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;

pub struct Rel {
    pub id: String,
    pub ty: String,
    pub target: String,
    pub external: bool,
}

fn attr(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
}

pub fn parse_rels(xml: &str) -> Vec<Rel> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut out = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"Relationship" => {
                let id = attr(&e, b"Id").unwrap_or_default();
                let ty = attr(&e, b"Type").unwrap_or_default();
                let target = attr(&e, b"Target").unwrap_or_default();
                let external = attr(&e, b"TargetMode").as_deref() == Some("External");
                if !id.is_empty() {
                    out.push(Rel { id, ty, target, external });
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

pub fn parse_slide_order(presentation_xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(presentation_xml);
    reader.config_mut().expand_empty_elements = true;
    let mut out = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"sldId" => {
                if let Some(id) = attr(&e, b"r:id") {
                    out.push(id);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

pub enum RelTarget {
    External(String),
    Internal(String),
}

pub struct SlideRels(pub HashMap<String, RelTarget>);

impl SlideRels {
    pub fn empty() -> Self {
        SlideRels(HashMap::new())
    }

    /// Build from parsed rels, resolving internal targets against `base_dir`.
    /// Internal targets that escape the archive root are dropped.
    pub fn from_rels(rels: Vec<Rel>, base_dir: &str) -> Self {
        let mut map = HashMap::new();
        for r in rels {
            let t = if r.external {
                RelTarget::External(r.target)
            } else {
                match resolve_rel(base_dir, &r.target) {
                    Some(p) => RelTarget::Internal(p),
                    None => continue,
                }
            };
            map.insert(r.id, t);
        }
        SlideRels(map)
    }

    pub fn get(&self, id: &str) -> Option<&RelTarget> {
        self.0.get(id)
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p kasane-adapters rels && just lint`
Expected: PASS; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-adapters/src/lib.rs crates/kasane-adapters/src/pptx
git commit -m "feat(adapters): parse PPTX slide order and relationships"
```

---

### Task 4: Slide DrawingML parser — text baseline

**Files:**
- Modify: `crates/kasane-adapters/src/pptx/slide.rs`
- Test: `crates/kasane-adapters/src/pptx/slide.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: `crate::pptx::rels::{SlideRels, RelTarget}`, `kasane_ir::{Block, BlockId, Inline}`.
- Produces:
  ```rust
  pub fn slide_to_blocks(xml: &str, next_id: &mut u32, rels: &SlideRels) -> Vec<Block>;
  pub fn notes_to_blocks(xml: &str) -> Vec<Block>;
  // internal, extended in later tasks:
  pub(crate) enum Shape { Title(Vec<Inline>), Body(Vec<Paragraph>) }
  pub(crate) struct Paragraph { pub level: u8, pub inlines: Vec<Inline> }
  pub(crate) fn parse_shapes(xml: &str, rels: &SlideRels) -> Vec<Shape>;
  ```

This task establishes the shape-parsing skeleton and the text mapping. Lists, tables, hyperlinks, and figures are added in Tasks 5–7 by extending `Shape`, `parse_shapes`, and the mapping — the public signatures do not change.

- [ ] **Step 1: Write the failing test**

Replace the contents of `crates/kasane-adapters/src/pptx/slide.rs` with a test module (implementation added next step):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pptx::rels::SlideRels;
    use kasane_ir::{Block, Inline};

    fn text_of(inls: &[Inline]) -> String {
        inls.iter().map(|i| match i {
            Inline::Text(t) => t.clone(),
            Inline::Strong(x) | Inline::Emph(x) => text_of(x),
            _ => String::new(),
        }).collect()
    }

    const SLIDE: &str = r#"<p:sld xmlns:a="a" xmlns:p="p">
      <p:cSld><p:spTree>
        <p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><a:r><a:t>The Title</a:t></a:r></a:p></p:txBody></p:sp>
        <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p>
            <a:r><a:t>plain </a:t></a:r>
            <a:r><a:rPr b="1"/><a:t>bold</a:t></a:r>
          </a:p></p:txBody></p:sp>
      </p:spTree></p:cSld></p:sld>"#;

    #[test]
    fn title_becomes_h1_and_runs_carry_bold() {
        let mut id = 0u32;
        let blocks = slide_to_blocks(SLIDE, &mut id, &SlideRels::empty());
        // first block is the H1 title
        match &blocks[0] {
            Block::Heading { level, inlines, .. } => {
                assert_eq!(*level, 1);
                assert_eq!(text_of(inlines), "The Title");
            }
            _ => panic!("expected heading"),
        }
        // the body paragraph with a bold run
        let para = blocks.iter().find_map(|b| match b {
            Block::Para(inls) => Some(inls),
            _ => None,
        }).expect("a paragraph");
        assert_eq!(text_of(para), "plain bold");
        assert!(para.iter().any(|i| matches!(i, Inline::Strong(_))));
    }

    #[test]
    fn missing_title_falls_back_to_slide_n_via_caller() {
        // A slide with no title placeholder yields no Title shape; slide_to_blocks
        // still returns a heading built by the fallback path.
        let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><a:r><a:t>body only</a:t></a:r></a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:sld>"#;
        let mut id = 0u32;
        let blocks = slide_to_blocks(xml, &mut id, &SlideRels::empty());
        assert!(matches!(&blocks[0], Block::Heading { level: 1, .. }));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-adapters slide`
Expected: FAIL — `slide_to_blocks` not found.

- [ ] **Step 3: Implement the parser skeleton and text mapping**

Prepend to `crates/kasane-adapters/src/pptx/slide.rs` (above the test module):
```rust
use crate::pptx::rels::{RelTarget, SlideRels};
use kasane_ir::{Block, BlockId, Inline};
use quick_xml::events::Event;
use quick_xml::Reader;

pub(crate) struct Paragraph {
    pub level: u8,
    pub inlines: Vec<Inline>,
}

pub(crate) enum Shape {
    Title(Vec<Inline>),
    Body(Vec<Paragraph>),
}

// Run-formatting state carried while inside <a:r>.
#[derive(Default)]
struct RunFmt {
    bold: bool,
    italic: bool,
}

fn attr_bool(e: &quick_xml::events::BytesStart, key: &[u8]) -> bool {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .map(|a| {
            let v = String::from_utf8_lossy(&a.value);
            v == "1" || v == "true"
        })
        .unwrap_or(false)
}

fn attr_str(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
}

fn styled(text: String, fmt: &RunFmt) -> Inline {
    let mut inl = Inline::Text(text);
    if fmt.bold {
        inl = Inline::Strong(vec![inl]);
    }
    if fmt.italic {
        inl = Inline::Emph(vec![inl]);
    }
    inl
}

pub(crate) fn parse_shapes(xml: &str, _rels: &SlideRels) -> Vec<Shape> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut buf = Vec::new();

    let mut shapes = Vec::new();
    let mut in_sp = false;
    let mut sp_is_title = false;
    let mut paras: Vec<Paragraph> = Vec::new();
    let mut cur_para: Option<Paragraph> = None;
    let mut fmt = RunFmt::default();
    let mut in_run = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"sp" => {
                    in_sp = true;
                    sp_is_title = false;
                    paras = Vec::new();
                }
                b"ph" => {
                    let t = attr_str(&e, b"type").unwrap_or_default();
                    if t == "title" || t == "ctrTitle" {
                        sp_is_title = true;
                    }
                }
                b"p" if in_sp => {
                    let mut level = 0u8;
                    // <a:pPr lvl="N"> may be the next event; capture inline attr if empty-expanded
                    if let Some(l) = attr_str(&e, b"lvl") {
                        level = l.parse().unwrap_or(0);
                    }
                    cur_para = Some(Paragraph { level, inlines: Vec::new() });
                }
                b"pPr" => {
                    if let (Some(p), Some(l)) = (cur_para.as_mut(), attr_str(&e, b"lvl")) {
                        p.level = l.parse().unwrap_or(0);
                    }
                }
                b"r" if in_sp => {
                    in_run = true;
                    fmt = RunFmt::default();
                }
                b"rPr" if in_run => {
                    fmt.bold = attr_bool(&e, b"b");
                    fmt.italic = attr_bool(&e, b"i");
                }
                _ => {}
            },
            Ok(Event::Text(t)) if in_run => {
                let s = t.unescape().unwrap_or_default().to_string();
                if !s.is_empty() {
                    if let Some(p) = cur_para.as_mut() {
                        p.inlines.push(styled(s, &fmt));
                    }
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"r" => in_run = false,
                b"p" if in_sp => {
                    if let Some(p) = cur_para.take() {
                        paras.push(p);
                    }
                }
                b"sp" => {
                    in_sp = false;
                    let inls: Vec<Inline> =
                        paras.iter().flat_map(|p| p.inlines.clone()).collect();
                    if sp_is_title {
                        shapes.push(Shape::Title(inls));
                    } else if !paras.iter().all(|p| p.inlines.is_empty()) {
                        shapes.push(Shape::Body(std::mem::take(&mut paras)));
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    shapes
}

// Map a body shape's paragraphs to blocks. Extended in Task 5 to build nested lists.
fn body_to_blocks(paras: Vec<Paragraph>, out: &mut Vec<Block>) {
    for p in paras {
        if !p.inlines.is_empty() {
            out.push(Block::Para(p.inlines));
        }
    }
}

pub fn slide_to_blocks(xml: &str, next_id: &mut u32, rels: &SlideRels) -> Vec<Block> {
    let shapes = parse_shapes(xml, rels);
    let mut out = Vec::new();

    // Heading first: the title shape's text, or a "Slide N"-style fallback. The
    // caller (Task 8) sets a real "Slide N" title when no Title shape is present;
    // here we emit an empty heading the caller can fill, keeping ids monotonic.
    let title_inls = shapes.iter().find_map(|s| match s {
        Shape::Title(t) if !t.is_empty() => Some(t.clone()),
        _ => None,
    });
    let id = BlockId(*next_id);
    *next_id += 1;
    out.push(Block::Heading {
        level: 1,
        id,
        inlines: title_inls.unwrap_or_default(),
    });

    for s in shapes {
        match s {
            Shape::Title(_) => {}
            Shape::Body(paras) => body_to_blocks(paras, &mut out),
        }
    }
    out
}

pub fn notes_to_blocks(xml: &str) -> Vec<Block> {
    let mut out = Vec::new();
    for s in parse_shapes(xml, &SlideRels::empty()) {
        if let Shape::Body(paras) = s {
            body_to_blocks(paras, &mut out);
        }
    }
    out
}
```

> **Note on the empty-title fallback:** `slide_to_blocks` always emits a level-1
> heading. When the slide has no title text, the heading's inlines are empty; Task 8's
> orchestration fills empty slide headings with `Slide N`. This keeps `BlockId`
> allocation in one place.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kasane-adapters slide && just lint`
Expected: PASS; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-adapters/src/pptx/slide.rs
git commit -m "feat(adapters): parse PPTX slide title and text runs to IR"
```

---

### Task 5: Slide bullet lists (nested by `lvl`)

**Files:**
- Modify: `crates/kasane-adapters/src/pptx/slide.rs` (replace `body_to_blocks`)
- Test: `crates/kasane-adapters/src/pptx/slide.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: `Paragraph`, `kasane_ir::Block`.
- Produces: same `slide_to_blocks` signature; body mapping now yields nested `List`s.

Rule (explicit v1): within one body shape, if it has more than one paragraph **or** any paragraph has `level > 0`, emit a bulleted `List` nested by `level`; a lone `level == 0` paragraph stays a `Para`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/kasane-adapters/src/pptx/slide.rs`:
```rust
#[test]
fn body_with_levels_becomes_nested_list() {
    use kasane_ir::Block;
    let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
      <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
      <p:txBody>
        <a:p><a:r><a:t>A</a:t></a:r></a:p>
        <a:p><a:pPr lvl="1"/><a:r><a:t>A1</a:t></a:r></a:p>
        <a:p><a:r><a:t>B</a:t></a:r></a:p>
      </p:txBody></p:sp>
    </p:spTree></p:cSld></p:sld>"#;
    let mut id = 0u32;
    let blocks = slide_to_blocks(xml, &mut id, &crate::pptx::rels::SlideRels::empty());
    let list = blocks.iter().find_map(|b| match b {
        Block::List { items, .. } => Some(items),
        _ => None,
    }).expect("a list");
    assert_eq!(list.len(), 2); // top-level items A and B
    // A's item contains a nested List holding A1
    let a_has_nested = list[0].iter().any(|b| matches!(b, Block::List { .. }));
    assert!(a_has_nested, "A1 should nest under A");
}

#[test]
fn lone_paragraph_stays_para() {
    use kasane_ir::Block;
    let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
      <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
      <p:txBody><a:p><a:r><a:t>solo</a:t></a:r></a:p></p:txBody></p:sp>
    </p:spTree></p:cSld></p:sld>"#;
    let mut id = 0u32;
    let blocks = slide_to_blocks(xml, &mut id, &crate::pptx::rels::SlideRels::empty());
    assert!(blocks.iter().any(|b| matches!(b, Block::Para(_))));
    assert!(!blocks.iter().any(|b| matches!(b, Block::List { .. })));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-adapters slide`
Expected: FAIL — body still emits flat `Para`s; the nested-list assertions fail.

- [ ] **Step 3: Replace `body_to_blocks` with the list builder**

Replace the `body_to_blocks` function in `crates/kasane-adapters/src/pptx/slide.rs` with:
```rust
fn body_to_blocks(paras: Vec<Paragraph>, out: &mut Vec<Block>) {
    let non_empty: Vec<Paragraph> = paras.into_iter().filter(|p| !p.inlines.is_empty()).collect();
    if non_empty.is_empty() {
        return;
    }
    if non_empty.len() == 1 && non_empty[0].level == 0 {
        out.push(Block::Para(non_empty.into_iter().next().unwrap().inlines));
        return;
    }
    out.push(build_list(&non_empty, 0, &mut 0));
}

// Build a bulleted List for paragraphs at `depth`, consuming from index `*i`.
// A paragraph deeper than `depth` becomes a nested List under the previous item.
fn build_list(paras: &[Paragraph], depth: u8, i: &mut usize) -> Block {
    let mut items: Vec<Vec<Block>> = Vec::new();
    while *i < paras.len() {
        let lvl = paras[*i].level;
        if lvl < depth {
            break; // belongs to an ancestor list
        }
        if lvl == depth {
            items.push(vec![Block::Para(paras[*i].inlines.clone())]);
            *i += 1;
        } else {
            // deeper: nest under the most recent item at this depth
            let nested = build_list(paras, depth + 1, i);
            if let Some(last) = items.last_mut() {
                last.push(nested);
            } else {
                // no parent item (malformed jump in levels): promote to this depth
                items.push(vec![nested]);
            }
        }
    }
    Block::List { ordered: false, items }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kasane-adapters slide && just lint`
Expected: PASS; no warnings (earlier `title_becomes_h1...` test still passes — a single bold-run body paragraph is a lone `level 0` para → `Para`).

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-adapters/src/pptx/slide.rs
git commit -m "feat(adapters): map PPTX bullet levels to nested lists"
```

---

### Task 6: Slide tables (`<a:tbl>`)

**Files:**
- Modify: `crates/kasane-adapters/src/pptx/slide.rs` (extend `Shape`, `parse_shapes`, mapping)
- Test: `crates/kasane-adapters/src/pptx/slide.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: `kasane_ir::Table`.
- Produces: `Shape` gains a `Table(kasane_ir::Table)` variant; `slide_to_blocks` emits `Block::Table`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/kasane-adapters/src/pptx/slide.rs`:
```rust
#[test]
fn graphic_frame_table_becomes_table_block() {
    use kasane_ir::Block;
    let xml = r#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
      <p:graphicFrame><a:graphic><a:graphicData><a:tbl>
        <a:tr>
          <a:tc><a:txBody><a:p><a:r><a:t>H1</a:t></a:r></a:p></a:txBody></a:tc>
          <a:tc><a:txBody><a:p><a:r><a:t>H2</a:t></a:r></a:p></a:txBody></a:tc>
        </a:tr>
        <a:tr>
          <a:tc><a:txBody><a:p><a:r><a:t>a</a:t></a:r></a:p></a:txBody></a:tc>
          <a:tc><a:txBody><a:p><a:r><a:t>b</a:t></a:r></a:p></a:txBody></a:tc>
        </a:tr>
      </a:tbl></a:graphicData></a:graphic></p:graphicFrame>
    </p:spTree></p:cSld></p:sld>"#;
    let mut id = 0u32;
    let blocks = slide_to_blocks(xml, &mut id, &crate::pptx::rels::SlideRels::empty());
    let t = blocks.iter().find_map(|b| match b {
        Block::Table(t) => Some(t),
        _ => None,
    }).expect("a table");
    assert_eq!(t.header.len(), 2);
    assert_eq!(t.rows.len(), 1);
    assert!(!t.has_merged);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-adapters slide`
Expected: FAIL — no `Block::Table` emitted.

- [ ] **Step 3: Extend the parser and mapping for tables**

In `crates/kasane-adapters/src/pptx/slide.rs`:

(a) Add a variant to the `Shape` enum:
```rust
pub(crate) enum Shape {
    Title(Vec<Inline>),
    Body(Vec<Paragraph>),
    Table(kasane_ir::Table),
}
```

(b) Extend `parse_shapes`. Add table-accumulator state alongside the existing locals (after `let mut in_run = false;`):
```rust
    let mut in_tbl = false;
    let mut tbl_rows: Vec<Vec<Vec<Inline>>> = Vec::new();
    let mut cur_row: Vec<Vec<Inline>> = Vec::new();
    let mut cur_cell: Vec<Inline> = Vec::new();
    let mut in_cell = false;
    let mut has_merged = false;
```

Add these arms to the `Event::Start` match (before the `_ => {}`):
```rust
                b"tbl" => {
                    in_tbl = true;
                    tbl_rows = Vec::new();
                }
                b"tr" if in_tbl => cur_row = Vec::new(),
                b"tc" if in_tbl => {
                    // gridSpan/hMerge/vMerge/rowSpan => the writer's HTML fallback
                    if attr_str(&e, b"gridSpan").is_some()
                        || attr_str(&e, b"rowSpan").is_some()
                        || attr_bool(&e, b"hMerge")
                        || attr_bool(&e, b"vMerge")
                    {
                        has_merged = true;
                    }
                    in_cell = true;
                    cur_cell = Vec::new();
                }
                b"r" if in_cell => {
                    in_run = true;
                    fmt = RunFmt::default();
                }
```

Extend the `Event::Text` handling so cell runs are captured. Replace the existing `Ok(Event::Text(t)) if in_run => { ... }` arm with:
```rust
            Ok(Event::Text(t)) if in_run => {
                let s = t.unescape().unwrap_or_default().to_string();
                if !s.is_empty() {
                    if in_cell {
                        cur_cell.push(styled(s, &fmt));
                    } else if let Some(p) = cur_para.as_mut() {
                        p.inlines.push(styled(s, &fmt));
                    }
                }
            }
```

Add these arms to the `Event::End` match (before the `_ => {}`):
```rust
                b"tc" if in_tbl => {
                    in_cell = false;
                    cur_row.push(std::mem::take(&mut cur_cell));
                }
                b"tr" if in_tbl => tbl_rows.push(std::mem::take(&mut cur_row)),
                b"tbl" => {
                    in_tbl = false;
                    let mut rows = std::mem::take(&mut tbl_rows);
                    let header = if rows.is_empty() { Vec::new() } else { rows.remove(0) };
                    shapes.push(Shape::Table(kasane_ir::Table {
                        header,
                        rows,
                        has_merged,
                    }));
                    has_merged = false;
                }
```

> The existing `b"r" if in_sp =>` start arm and `b"r" => in_run = false` end arm still
> apply; a run inside a cell matches the new `b"r" if in_cell =>` arm. `in_sp` is false
> inside a `graphicFrame`, so table runs never leak into `cur_para`.

(c) In `slide_to_blocks`, handle the new variant in the mapping loop:
```rust
    for s in shapes {
        match s {
            Shape::Title(_) => {}
            Shape::Body(paras) => body_to_blocks(paras, &mut out),
            Shape::Table(t) => out.push(Block::Table(t)),
        }
    }
```

Also update the `notes_to_blocks` match to ignore tables (notes rarely contain them):
```rust
    for s in parse_shapes(xml, &SlideRels::empty()) {
        match s {
            Shape::Body(paras) => body_to_blocks(paras, &mut out),
            _ => {}
        }
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kasane-adapters slide && just lint`
Expected: PASS; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-adapters/src/pptx/slide.rs
git commit -m "feat(adapters): parse PPTX tables to IR Table blocks"
```

---

### Task 7: Slide hyperlinks & pictures

**Files:**
- Modify: `crates/kasane-adapters/src/pptx/slide.rs` (hyperlink runs + `<p:pic>` figures)
- Test: `crates/kasane-adapters/src/pptx/slide.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: `SlideRels`, `RelTarget`, `kasane_ir::{AssetRef, RefTarget}`.
- Produces: runs carrying `<a:hlinkClick>` become `Inline::Link{External}`; `<p:pic>` becomes `Shape::Picture` mapped to `Block::Figure`. `AssetRef.key` is the resolved archive path of the media.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/kasane-adapters/src/pptx/slide.rs`:
```rust
#[test]
fn hyperlink_run_and_picture_resolve_via_rels() {
    use kasane_ir::{Block, Inline, RefTarget};
    use crate::pptx::rels::{RelTarget as RT, SlideRels};
    use std::collections::HashMap;

    let mut m = HashMap::new();
    m.insert("rId2".to_string(), RT::External("https://example.com".into()));
    m.insert("rId3".to_string(), RT::Internal("ppt/media/image1.png".into()));
    let rels = SlideRels(m);

    let xml = r#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:r="r"><p:cSld><p:spTree>
      <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
      <p:txBody><a:p>
        <a:r><a:rPr><a:hlinkClick r:id="rId2"/></a:rPr><a:t>link text</a:t></a:r>
      </a:p></p:txBody></p:sp>
      <p:pic><p:nvPicPr><p:cNvPr id="5" name="Pic" descr="a cat"/></p:nvPicPr>
        <p:blipFill><a:blip r:embed="rId3"/></p:blipFill></p:pic>
    </p:spTree></p:cSld></p:sld>"#;

    let mut id = 0u32;
    let blocks = slide_to_blocks(xml, &mut id, &rels);

    // hyperlink
    let has_link = blocks.iter().any(|b| matches!(b, Block::Para(inls)
        if inls.iter().any(|i| matches!(i,
            Inline::Link { target: RefTarget::External(u), .. } if u == "https://example.com"))));
    assert!(has_link, "hyperlink run should become an external link");

    // figure
    let fig = blocks.iter().find_map(|b| match b {
        Block::Figure { image, caption, .. } => Some((image.key.clone(), caption.clone())),
        _ => None,
    }).expect("a figure");
    assert_eq!(fig.0, "ppt/media/image1.png");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-adapters slide`
Expected: FAIL — no link, no figure.

- [ ] **Step 3: Add hyperlink and picture handling**

In `crates/kasane-adapters/src/pptx/slide.rs`:

(a) Update the `use` line to import `RefTarget`:
```rust
use kasane_ir::{AssetRef, Block, BlockId, Inline, RefTarget};
```

(b) Add a `Picture` variant to `Shape`:
```rust
pub(crate) enum Shape {
    Title(Vec<Inline>),
    Body(Vec<Paragraph>),
    Table(kasane_ir::Table),
    Picture { key: String, alt: String },
}
```

(c) Extend `RunFmt` with an optional link URL, and add picture state. Change the struct:
```rust
#[derive(Default)]
struct RunFmt {
    bold: bool,
    italic: bool,
    link: Option<String>,
}
```
Change `styled` so a link wraps the (already emphasized) inline:
```rust
fn styled(text: String, fmt: &RunFmt) -> Inline {
    let mut inl = Inline::Text(text);
    if fmt.bold {
        inl = Inline::Strong(vec![inl]);
    }
    if fmt.italic {
        inl = Inline::Emph(vec![inl]);
    }
    match &fmt.link {
        Some(url) => Inline::Link {
            target: RefTarget::External(url.clone()),
            inlines: vec![inl],
        },
        None => inl,
    }
}
```

(d) In `parse_shapes`, add picture-accumulator state next to the others:
```rust
    let mut in_pic = false;
    let mut pic_alt = String::new();
    let mut pic_key: Option<String> = None;
```
`parse_shapes` already takes `_rels`; rename its parameter to `rels` and use it. Add to `Event::Start`:
```rust
                b"pic" => {
                    in_pic = true;
                    pic_alt = String::new();
                    pic_key = None;
                }
                b"cNvPr" if in_pic => {
                    pic_alt = attr_str(&e, b"descr").unwrap_or_default();
                }
                b"blip" if in_pic => {
                    if let Some(rid) = attr_str(&e, b"embed").or_else(|| attr_str(&e, b"r:embed")) {
                        if let Some(RelTarget::Internal(p)) = rels.get(&rid) {
                            pic_key = Some(p.clone());
                        }
                    }
                }
                b"hlinkClick" if in_run => {
                    if let Some(rid) = attr_str(&e, b"id").or_else(|| attr_str(&e, b"r:id")) {
                        if let Some(RelTarget::External(u)) = rels.get(&rid) {
                            fmt.link = Some(u.clone());
                        }
                    }
                }
```
Add to `Event::End`:
```rust
                b"pic" => {
                    in_pic = false;
                    if let Some(key) = pic_key.take() {
                        shapes.push(Shape::Picture {
                            key,
                            alt: std::mem::take(&mut pic_alt),
                        });
                    }
                }
```

> `hlinkClick` is an empty element; with `expand_empty_elements = true` it fires a
> `Start` (matched above) then an `End` — the `End` falls through harmlessly.

(e) In `slide_to_blocks`, map the new variant:
```rust
    for s in shapes {
        match s {
            Shape::Title(_) => {}
            Shape::Body(paras) => body_to_blocks(paras, &mut out),
            Shape::Table(t) => out.push(Block::Table(t)),
            Shape::Picture { key, alt } => out.push(Block::Figure {
                image: AssetRef { key, bytes_ref: 0 },
                caption: if alt.is_empty() { Vec::new() } else { vec![Inline::Text(alt)] },
                number: None,
            }),
        }
    }
```
Add a `Shape::Picture { .. } => {}` (or keep the existing `_ => {}`) arm in `notes_to_blocks`'s match so it still compiles.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kasane-adapters slide && just lint`
Expected: PASS; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-adapters/src/pptx/slide.rs
git commit -m "feat(adapters): resolve PPTX hyperlinks and pictures via slide rels"
```

---

### Task 8: PptxAdapter orchestration

**Files:**
- Modify: `crates/kasane-adapters/src/pptx/mod.rs`
- Test: `crates/kasane-adapters/src/pptx/mod.rs` (inline `#[test]`, builds an in-memory PPTX)

**Interfaces:**
- Consumes: `crate::ziputil`, `crate::guard::resolve_rel`, `rels::*`, `slide::*`, `crate::{Adapter, ParseError}`, `kasane_ir::*`.
- Produces: `pub struct PptxAdapter;` implementing `Adapter`.

- [ ] **Step 1: Write the failing test**

Replace `crates/kasane-adapters/src/pptx/mod.rs` with the module declarations plus a test that builds a minimal PPTX in memory:
```rust
mod rels;
mod slide;

// implementation added in Step 3

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Adapter;

    fn add(w: &mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>, name: &str, data: &[u8]) {
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        w.start_file(name, opts).unwrap();
        std::io::Write::write_all(w, data).unwrap();
    }

    fn build_pptx() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        add(&mut w, "[Content_Types].xml", b"<Types/>");
        // presentation lists slide rId3 THEN rId2 -> display order is slide2 then slide1
        add(&mut w, "ppt/presentation.xml", br#"<p:presentation xmlns:r="r">
          <p:sldIdLst><p:sldId r:id="rId3"/><p:sldId r:id="rId2"/></p:sldIdLst>
          </p:presentation>"#);
        add(&mut w, "ppt/_rels/presentation.xml.rels", br#"<Relationships>
          <Relationship Id="rId2" Type="x/slide" Target="slides/slide1.xml"/>
          <Relationship Id="rId3" Type="x/slide" Target="slides/slide2.xml"/>
          </Relationships>"#);
        add(&mut w, "ppt/slides/slide1.xml", br#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><a:r><a:t>First</a:t></a:r></a:p></p:txBody></p:sp>
          <p:pic><p:nvPicPr><p:cNvPr id="5" name="P" descr="pic"/></p:nvPicPr>
          <p:blipFill><a:blip r:embed="rId9"/></p:blipFill></p:pic>
          </p:spTree></p:cSld></p:sld>"#);
        add(&mut w, "ppt/slides/_rels/slide1.xml.rels", br#"<Relationships>
          <Relationship Id="rId9" Type="x/image" Target="../media/image1.png"/>
          <Relationship Id="rId8" Type="x/notesSlide" Target="../notesSlides/notesSlide1.xml"/>
          </Relationships>"#);
        add(&mut w, "ppt/slides/slide2.xml", br#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><a:r><a:t>no title here</a:t></a:r></a:p></p:txBody></p:sp>
          </p:spTree></p:cSld></p:sld>"#);
        add(&mut w, "ppt/notesSlides/notesSlide1.xml", br#"<p:notes xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><a:r><a:t>speaker note</a:t></a:r></a:p></p:txBody></p:sp>
          </p:spTree></p:cSld></p:notes>"#);
        add(&mut w, "ppt/media/image1.png", b"\x89PNG\r\n\x1a\nFAKE");
        w.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn parses_pptx_in_display_order_with_media_and_notes() {
        use kasane_ir::Block;
        let bytes = build_pptx();
        let (doc, assets) = PptxAdapter.parse(&bytes, "deck.pptx").unwrap();
        assert_eq!(doc.meta.source_format, "pptx");

        // Display order: slide2 (rId3) comes before slide1 (rId2). slide2 has no title
        // -> "Slide 1" fallback; slide1's title is "First".
        let headings: Vec<String> = doc.nodes.iter().filter_map(|n| match &n.block {
            Block::Heading { inlines, .. } => Some(
                inlines.iter().map(|i| if let kasane_ir::Inline::Text(t) = i { t.clone() } else { String::new() }).collect()
            ),
            _ => None,
        }).collect();
        assert_eq!(headings, vec!["Slide 1".to_string(), "First".to_string()]);

        // media extracted into the AssetBag
        assert_eq!(assets.items.len(), 1);
        assert_eq!(assets.items[0].key, "ppt/media/image1.png");
        assert!(assets.items[0].bytes.starts_with(b"\x89PNG"));

        // speaker note appended under a bold "Notes" lead-in
        let has_notes_leadin = doc.nodes.iter().any(|n| matches!(&n.block,
            Block::Para(inls) if inls.iter().any(|i| matches!(i, kasane_ir::Inline::Strong(x)
                if matches!(x.first(), Some(kasane_ir::Inline::Text(t)) if t == "Notes")))));
        assert!(has_notes_leadin, "expected a **Notes** lead-in");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-adapters pptx`
Expected: FAIL — `PptxAdapter` not found.

- [ ] **Step 3: Implement the adapter**

Insert into `crates/kasane-adapters/src/pptx/mod.rs` between the `mod` lines and the test module:
```rust
use crate::guard::resolve_rel;
use crate::ziputil::{read_entry_bytes, read_entry_string};
use crate::{Adapter, ParseError};
use kasane_ir::*;
use rels::{parse_rels, parse_slide_order, RelTarget, SlideRels};
use std::collections::HashMap;

pub struct PptxAdapter;

impl Adapter for PptxAdapter {
    fn parse(&self, bytes: &[u8], source_path: &str) -> Result<(Document, AssetBag), ParseError> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| ParseError::Malformed(e.to_string()))?;
        let mut total = 0u64;

        // 1. Slide display order via presentation.xml + its rels.
        let pres = read_entry_string(&mut zip, "ppt/presentation.xml", &mut total)?;
        let order = parse_slide_order(&pres);
        let pres_rels_xml =
            read_entry_string(&mut zip, "ppt/_rels/presentation.xml.rels", &mut total)
                .unwrap_or_default();
        let mut rid_to_slide: HashMap<String, String> = HashMap::new();
        for r in parse_rels(&pres_rels_xml) {
            if r.ty.ends_with("slide") && !r.external {
                if let Some(p) = resolve_rel("ppt", &r.target) {
                    rid_to_slide.insert(r.id, p);
                }
            }
        }
        let slide_paths: Vec<String> = order
            .iter()
            .filter_map(|rid| rid_to_slide.get(rid).cloned())
            .collect();

        // 2. Each slide -> blocks (+ media + notes).
        let mut nodes = Vec::new();
        let mut assets = AssetBag::default();
        let mut seen_media: HashMap<String, String> = HashMap::new(); // archive path -> filename
        let mut next_id = 0u32;

        for (idx, spath) in slide_paths.iter().enumerate() {
            let Ok(sxml) = read_entry_string(&mut zip, spath, &mut total) else {
                // Degrade: unreadable slide still gets a heading + raw note.
                push_slide_fallback(&mut nodes, &mut next_id, idx, spath);
                continue;
            };
            let sdir = parent_dir(spath);
            let srels_path = rels_path_for(spath);
            let srels_xml = read_entry_string(&mut zip, &srels_path, &mut total).unwrap_or_default();
            let parsed_rels = parse_rels(&srels_xml);

            // Notes target (internal, .../notesSlide) before building SlideRels (which consumes rels).
            let notes_target = parsed_rels
                .iter()
                .find(|r| r.ty.ends_with("notesSlide") && !r.external)
                .and_then(|r| resolve_rel(&sdir, &r.target));

            let slide_rels = SlideRels::from_rels(parsed_rels, &sdir);
            let mut blocks = slide::slide_to_blocks(&sxml, &mut next_id, &slide_rels);

            // Fill an empty title heading with "Slide N".
            if let Some(Block::Heading { inlines, .. }) = blocks.first_mut() {
                if inlines.is_empty() {
                    *inlines = vec![Inline::Text(format!("Slide {}", idx + 1))];
                }
            }

            // Extract referenced media into the AssetBag (once per archive path).
            for b in &blocks {
                if let Block::Figure { image, .. } = b {
                    if !seen_media.contains_key(&image.key) {
                        if let Ok(data) = read_entry_bytes(&mut zip, &image.key, &mut total) {
                            let filename = safe_media_filename(&image.key, seen_media.len());
                            seen_media.insert(image.key.clone(), filename.clone());
                            assets.items.push(AssetItem {
                                key: image.key.clone(),
                                filename,
                                bytes: data,
                            });
                        }
                    }
                }
            }

            // Speaker notes appended under a bold "Notes" lead-in.
            if let Some(nt) = notes_target {
                if let Ok(nxml) = read_entry_string(&mut zip, &nt, &mut total) {
                    let note_blocks = slide::notes_to_blocks(&nxml);
                    if !note_blocks.is_empty() {
                        blocks.push(Block::Para(vec![Inline::Strong(vec![Inline::Text(
                            "Notes".into(),
                        )])]));
                        blocks.extend(note_blocks);
                    }
                }
            }

            for b in blocks {
                nodes.push(Node {
                    block: b,
                    prov: Provenance {
                        source_pages: None,
                        source_href: Some(spath.clone()),
                    },
                });
            }
        }

        let doc = Document {
            meta: DocMeta {
                title: derive_title(source_path),
                authors: vec![],
                language: None,
                source_format: "pptx".into(),
                source_path: source_path.to_string(),
            },
            nodes,
        };
        Ok((doc, assets))
    }
}

fn push_slide_fallback(nodes: &mut Vec<Node>, next_id: &mut u32, idx: usize, spath: &str) {
    let id = BlockId(*next_id);
    *next_id += 1;
    let prov = Provenance { source_pages: None, source_href: Some(spath.to_string()) };
    nodes.push(Node {
        block: Block::Heading { level: 1, id, inlines: vec![Inline::Text(format!("Slide {}", idx + 1))] },
        prov: prov.clone(),
    });
    nodes.push(Node { block: Block::Raw { note: "unparsable slide".into() }, prov });
}

fn parent_dir(path: &str) -> String {
    path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default()
}

// "ppt/slides/slide1.xml" -> "ppt/slides/_rels/slide1.xml.rels"
fn rels_path_for(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, file)) => format!("{}/_rels/{}.rels", dir, file),
        None => format!("_rels/{}.rels", path),
    }
}

fn safe_media_filename(archive_path: &str, n: usize) -> String {
    let base = archive_path.rsplit('/').next().unwrap_or("image");
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    // Prefix an index to guarantee uniqueness even if basenames collide across dirs.
    format!("{:03}-{}", n, if cleaned.is_empty() { "image".into() } else { cleaned })
}

fn derive_title(source_path: &str) -> String {
    let stem = source_path
        .rsplit('/')
        .next()
        .and_then(|f| f.rsplit_once('.').map(|(s, _)| s).or(Some(f)))
        .unwrap_or("Untitled");
    if stem.is_empty() { "Untitled".into() } else { stem.to_string() }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kasane-adapters pptx && just lint`
Expected: PASS; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-adapters/src/pptx/mod.rs
git commit -m "feat(adapters): PptxAdapter orchestration — order, media, notes"
```

---

### Task 9: Wire into the pipeline + checked-in fixture + end-to-end test

**Files:**
- Modify: `crates/kasane-adapters/src/lib.rs` (register `PptxAdapter`, re-export)
- Modify: `crates/kasane-cli/src/main.rs` (help text)
- Create: `crates/kasane-adapters/examples/gen_minimal_pptx.rs`
- Create: `tests/fixtures/pptx/minimal.pptx` (generated, then committed)
- Test: `crates/kasane-adapters/src/lib.rs` (inline `#[test]`, end-to-end via the fixture)

**Interfaces:**
- Consumes: `PptxAdapter`, `detect`, `Format::Pptx`.
- Produces: `adapter_for(Format::Pptx)` returns a `PptxAdapter`; `kasane deck.pptx` works.

- [ ] **Step 1: Register the adapter**

In `crates/kasane-adapters/src/lib.rs`:

(a) add the re-export next to `pub use epub::EpubAdapter;`:
```rust
pub use pptx::PptxAdapter;
```
(b) add the dispatch arm in `adapter_for`:
```rust
    match fmt {
        Format::Epub => Ok(Box::new(EpubAdapter)),
        Format::Pptx => Ok(Box::new(PptxAdapter)),
        _ => Err(ParseError::Unsupported),
    }
```
(c) make the pptx module's adapter public — in `crates/kasane-adapters/src/pptx/mod.rs`, the `pub struct PptxAdapter;` is already public; ensure `mod pptx;` in `lib.rs` exposes it (the re-export in (a) handles visibility).

- [ ] **Step 2: Write the fixture generator**

Create `crates/kasane-adapters/examples/gen_minimal_pptx.rs`. This assembles the same in-memory PPTX as Task 8's test (two slides, one image, one table, notes) and writes it to the checked-in fixture path, so the binary is reproducible:
```rust
use std::io::Write;

fn add(w: &mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>, name: &str, data: &[u8]) {
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    w.start_file(name, opts).unwrap();
    w.write_all(data).unwrap();
}

fn main() {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    add(&mut w, "[Content_Types].xml", b"<Types/>");
    add(&mut w, "ppt/presentation.xml", br#"<p:presentation xmlns:r="r"><p:sldIdLst><p:sldId r:id="rId2"/><p:sldId r:id="rId3"/></p:sldIdLst></p:presentation>"#);
    add(&mut w, "ppt/_rels/presentation.xml.rels", br#"<Relationships><Relationship Id="rId2" Type="x/slide" Target="slides/slide1.xml"/><Relationship Id="rId3" Type="x/slide" Target="slides/slide2.xml"/></Relationships>"#);
    add(&mut w, "ppt/slides/slide1.xml", br#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:r="r"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Welcome</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>First point</a:t></a:r></a:p><a:p><a:pPr lvl="1"/><a:r><a:t>Sub point</a:t></a:r></a:p></p:txBody></p:sp><p:pic><p:nvPicPr><p:cNvPr id="5" name="P" descr="a diagram"/></p:nvPicPr><p:blipFill><a:blip r:embed="rId9"/></p:blipFill></p:pic></p:spTree></p:cSld></p:sld>"#);
    add(&mut w, "ppt/slides/_rels/slide1.xml.rels", br#"<Relationships><Relationship Id="rId9" Type="x/image" Target="../media/image1.png"/><Relationship Id="rId8" Type="x/notesSlide" Target="../notesSlides/notesSlide1.xml"/></Relationships>"#);
    add(&mut w, "ppt/slides/slide2.xml", br#"<p:sld xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Data</a:t></a:r></a:p></p:txBody></p:sp><p:graphicFrame><a:graphic><a:graphicData><a:tbl><a:tr><a:tc><a:txBody><a:p><a:r><a:t>Name</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>Value</a:t></a:r></a:p></a:txBody></a:tc></a:tr><a:tr><a:tc><a:txBody><a:p><a:r><a:t>x</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>1</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#);
    add(&mut w, "ppt/notesSlides/notesSlide1.xml", br#"<p:notes xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Remember to smile.</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#);
    add(&mut w, "ppt/media/image1.png", b"\x89PNG\r\n\x1a\nFAKEPNGDATA");
    w.finish().unwrap();
    std::fs::create_dir_all("tests/fixtures/pptx").unwrap();
    std::fs::write("tests/fixtures/pptx/minimal.pptx", buf.into_inner()).unwrap();
    println!("wrote tests/fixtures/pptx/minimal.pptx");
}
```

- [ ] **Step 3: Generate and inspect the fixture**

Run: `cargo run -p kasane-adapters --example gen_minimal_pptx`
Expected: prints `wrote tests/fixtures/pptx/minimal.pptx`; the file exists.
Sanity check detection: it is a zip containing `ppt/`, so `detect` classifies it as PPTX.

- [ ] **Step 4: Write the end-to-end test**

Add to `crates/kasane-adapters/src/lib.rs`, inside its existing `#[cfg(test)] mod tests`:
```rust
    #[test]
    fn end_to_end_pptx_fixture_to_sitetree() {
        let bytes = std::fs::read("../../tests/fixtures/pptx/minimal.pptx").unwrap();
        assert!(matches!(detect(&bytes, Some("pptx")), Some(Format::Pptx)));

        let (doc, assets) = PptxAdapter.parse(&bytes, "minimal.pptx").unwrap();
        assert_eq!(doc.meta.source_format, "pptx");
        assert_eq!(doc.meta.title, "minimal");

        // slide order + title fallback: slide1 "Welcome", slide2 "Data"
        let headings: Vec<String> = doc.nodes.iter().filter_map(|n| match &n.block {
            Block::Heading { inlines, .. } => Some(kasane_ir_text(inlines)),
            _ => None,
        }).collect();
        assert_eq!(headings, vec!["Welcome".to_string(), "Data".to_string()]);

        // media flushed through the whole pipeline
        assert_eq!(assets.items.len(), 1);

        // structuring + writing succeeds end to end
        let site = kasane_core::structure(doc, &kasane_core::Options::default());
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("deck");
        kasane_writer::write_tree(&site, &assets, &out, false).unwrap();
        assert!(out.join("index.md").exists());
        assert!(out.join("_assets").read_dir().unwrap().next().is_some());
    }
```

Add the dev-dependencies this test needs to `crates/kasane-adapters/Cargo.toml`:
```toml
[dev-dependencies]
kasane-core = { path = "../kasane-core" }
kasane-writer = { path = "../kasane-writer" }
tempfile = "3"
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p kasane-adapters end_to_end && just lint`
Expected: PASS; no warnings.

- [ ] **Step 6: Update the CLI help text**

In `crates/kasane-cli/src/main.rs`, change the input doc comment from:
```rust
    /// Input document (EPUB supported in this build)
```
to:
```rust
    /// Input document (EPUB, PPTX supported in this build)
```

- [ ] **Step 7: Full green + commit**

Run: `just lint && just test`
Expected: PASS across the whole workspace; no warnings.

```bash
git add crates/kasane-adapters/src/lib.rs crates/kasane-adapters/Cargo.toml \
        crates/kasane-adapters/examples/gen_minimal_pptx.rs \
        crates/kasane-cli/src/main.rs tests/fixtures/pptx/minimal.pptx
git commit -m "feat(cli): wire PPTX adapter end-to-end with checked-in fixture"
```

---

## Self-Review

**Spec coverage** (checked against `2026-07-19-pptx-adapter-design.md`):

| Spec item | Task |
|---|---|
| §2 streaming quick-xml state machine | 4–7 |
| §3 module layout + `ziputil` refactor | 1, 3 |
| §4 slide order via presentation.xml rels | 3, 8 |
| §5 title→H1 (Slide N fallback) | 4, 8 |
| §5 paras/runs/bold/italic | 4 |
| §5 nested bullet lists by lvl | 5 |
| §5 tables (merged → has_merged) | 6 |
| §5 hyperlinks (external; cross-slide → plain text) | 7 |
| §5 pictures → Figure + AssetBag | 7, 8 |
| §5 notes appended under **Notes** | 8 |
| §5 provenance source_href | 8 |
| §6 bomb guard on every read | 1, 8 |
| §6 resolve_rel confinement | 2, 3, 8 |
| §6 media filename sanitization | 8 |
| §6 XXE-safe (no entity expansion) | 3, 4 (quick-xml default) |
| §7 degrade: unparsable slide → heading + Raw | 8 |
| §7 broken rel drops one image/link | 3 (`from_rels` skips), 7, 8 |
| §8 unit tests (slide, rels) | 3–7 |
| §8 golden fixture minimal.pptx | 9 |
| §8 end-to-end | 9 |
| §9 CLI one-arm + help | 9 |
| §10 no new crate deps (dev-deps only) | 9 |

Cross-slide internal links intentionally degrade to plain text (`SlideRels` only maps external hyperlinks to `External`; an internal `hlinkClick` target has no `External` entry, so `fmt.link` stays `None` and the run is plain text) — matches the §1/§5 non-goal.

**Placeholder scan:** no TBD/TODO; every code step shows complete code; every command has expected output.

**Type consistency:** `slide_to_blocks(xml, next_id, rels)` and `notes_to_blocks(xml)` signatures are fixed in Task 4 and unchanged through Tasks 5–7 (only `Shape`, `parse_shapes`, and the mapping loop grow). `AssetRef{key, bytes_ref}` keys set in Task 7 match `AssetItem.key` in Task 8, which is what the writer looks up. `SlideRels`/`RelTarget`/`Rel` names are consistent across Tasks 3, 7, 8. `read_entry_bytes`/`read_entry_string` names are consistent across Tasks 1 and 8.
