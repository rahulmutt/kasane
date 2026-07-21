# PDF Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a pure-Rust `PdfAdapter` that converts born-digital PDFs into kasane's IR, wired into the existing pipeline so `kasane convert book.pdf -o out/` produces a progressive-disclosure Markdown tree.

**Architecture:** A new `crates/kasane-adapters/src/pdf/` module built on the `lopdf` crate. `doc.rs` opens and decrypts; `content.rs` interprets page content streams into positioned text runs; `layout.rs` groups runs into lines/paragraphs and infers headings by font size; `outline.rs` turns the PDF's own `/Outlines` (via `lopdf::Document::get_toc`) into headings; `image.rs` extracts embedded images; `mod.rs` assembles per-page blocks into a `Document`. Detection already returns `Format::Pdf`; only `adapter_for` and docs change.

**Tech Stack:** Rust (edition 2021), `lopdf` 0.44 (object model, content-stream parsing, `/ToUnicode` decoding, RC4/AES decryption, TOC), `png` 0.17 (encode FlateDecode rasters), Python 3 stdlib (hermetic fixture generator).

## Global Constraints

- **Pure Rust on the default path.** No mandatory C libraries. `lopdf` and `png` are pure Rust. (Copied from spec §Confirmed decisions.)
- **Untrusted-input boundary.** Every decode is bomb-guarded against `crate::guard::MAX_TOTAL_BYTES` (512 MiB) / `MAX_RATIO` (200); recursion is depth-bounded; a corrupt page/stream degrades to a `Block::Raw` note rather than aborting the parse.
- **Never break DRM.** Only the empty-user-password case is decrypted; a real user password yields `ParseError::Encrypted` and CLI exit code 2. No password cracking, no `--password` flag.
- **Single-column reading order.** Multi-column layout, table reconstruction, and math recovery are explicit non-goals for this adapter.
- **Every change ships green under `mise run lint && mise run test`** (rustfmt check + `clippy -D warnings` + all tests).
- **Toolchain:** `rust-version = "1.97"` (workspace). Edition 2021.

## Deviations from the approved spec (v1 refinements discovered while grounding on lopdf's API)

1. **Heading placement is page-granular.** The spec described resolving each outline entry to "page + y" and splicing at that y. `lopdf::Document::get_toc()` resolves outline entries to a **page number + level + title** but not an intra-page y. So outline headings are placed at the **start of their target page's block stream**, in outline order. This is simpler, robust, and fully adequate for heading-driven file splitting. Documented as a limitation.
2. **Text decoding meets/exceeds the spec.** The spec allowed a WinAnsi fallback when `/ToUnicode` is absent. We reuse lopdf's public `Dictionary::get_font_encoding` + `Encoding::bytes_to_string`, which already handle `/ToUnicode` CMaps *and* standard encodings — so we get correct Unicode for free.
3. **Image coverage is explicit.** `FlateDecode` DeviceGray/DeviceRGB 8-bit rasters → PNG; `DCTDecode` → JPEG passthrough. `CCITTFax`, `JBIG2`, `JPXDecode`, indexed/ICCBased colorspaces → a `Block::Raw` note naming the filter, not a hard failure. (Bilevel scans are typically CCITT/JBIG2, so pure-image scans get a placeholder note until the future `-F ocr` feature — matching the spec's "scanned page" degradation intent.)

---

## File Structure

**Created:**
- `crates/kasane-adapters/src/pdf/mod.rs` — `PdfAdapter`, per-page assembly into `Document` + `AssetBag`.
- `crates/kasane-adapters/src/pdf/doc.rs` — open bytes, empty-password decrypt, page enumeration, bomb-guarded content access.
- `crates/kasane-adapters/src/pdf/content.rs` — content-stream interpreter → `Vec<TextRun>` (positioned, font-sized, Unicode text).
- `crates/kasane-adapters/src/pdf/layout.rs` — `TextRun`s → `Line`s → paragraphs + font-size heading inference.
- `crates/kasane-adapters/src/pdf/outline.rs` — `/Outlines` (via `get_toc`) → per-page heading list.
- `crates/kasane-adapters/src/pdf/image.rs` — page `/XObject` images → `AssetItem` + `Block::Figure`, or a `Raw` note.
- `tests/fixtures/pdf/make_pdf_fixtures.py` — hermetic (stdlib-only) generator for the four fixtures.
- `tests/fixtures/pdf/{minimal,no-outline,image,scanned}.pdf` — generated fixtures.

**Modified:**
- `crates/kasane-adapters/Cargo.toml` — add `lopdf`, `png`.
- `crates/kasane-adapters/src/lib.rs` — `mod pdf; pub use pdf::PdfAdapter;`; register in `adapter_for`.
- `crates/kasane-cli/src/main.rs` — route `"encrypted"` to exit code 2; refresh the input-format help text.
- `README.md` — move PDF from "coming" to supported; add limitations.
- `AGENTS.md` — add the `pdf/` module to the codebase map.
- `deny.toml` — only if `lopdf`'s transitive crates trip an advisory (handle in Task 1 if it arises).

---

## Task 1: Add dependencies and register the adapter

**Files:**
- Modify: `crates/kasane-adapters/Cargo.toml`
- Modify: `crates/kasane-adapters/src/lib.rs`
- Create: `crates/kasane-adapters/src/pdf/mod.rs`

**Interfaces:**
- Produces: `pub struct PdfAdapter;` implementing `Adapter`; `adapter_for(Format::Pdf)` returns `Ok(Box::new(PdfAdapter))`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module at the bottom of `crates/kasane-adapters/src/lib.rs`:

```rust
    #[test]
    fn pdf_format_has_an_adapter() {
        // Regression: Pdf used to fall into the `_ => Unsupported` arm.
        assert!(adapter_for(Format::Pdf).is_ok());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-adapters pdf_format_has_an_adapter`
Expected: FAIL — `adapter_for(Format::Pdf)` currently returns `Err(Unsupported)`, so `.is_ok()` is false. (If it does not compile yet because `PdfAdapter` is unresolved, that also counts as red; proceed.)

- [ ] **Step 3: Add the dependencies**

Add to `crates/kasane-adapters/Cargo.toml` under `[dependencies]` (keep existing entries):

```toml
lopdf = { version = "0.44", default-features = false }
png = "0.17"
```

Then fetch and confirm they resolve:

Run: `cargo fetch -p kasane-adapters 2>&1 | tail -3`
Expected: no error (crates download / already cached).

- [ ] **Step 4: Create the stub adapter**

Create `crates/kasane-adapters/src/pdf/mod.rs`:

```rust
mod content;
mod doc;
mod image;
mod layout;
mod outline;

use crate::{Adapter, ParseError};
use kasane_ir::{AssetBag, Document};

pub struct PdfAdapter;

impl Adapter for PdfAdapter {
    fn parse(&self, _bytes: &[u8], _source_path: &str) -> Result<(Document, AssetBag), ParseError> {
        // Filled in over Tasks 3–9.
        Err(ParseError::Malformed("pdf adapter not yet implemented".into()))
    }
}
```

Create empty module files so `mod.rs` compiles (they are filled in later tasks):

Run: `touch crates/kasane-adapters/src/pdf/content.rs crates/kasane-adapters/src/pdf/doc.rs crates/kasane-adapters/src/pdf/image.rs crates/kasane-adapters/src/pdf/layout.rs crates/kasane-adapters/src/pdf/outline.rs`

- [ ] **Step 5: Register the module and adapter**

In `crates/kasane-adapters/src/lib.rs`, add `mod pdf;` next to the other `mod` lines and `pub use pdf::PdfAdapter;` next to the other `pub use` lines. Then change the `adapter_for` match:

```rust
pub fn adapter_for(fmt: Format) -> Result<Box<dyn Adapter>, ParseError> {
    match fmt {
        Format::Epub => Ok(Box::new(EpubAdapter)),
        Format::Pptx => Ok(Box::new(PptxAdapter)),
        Format::Mobi | Format::Azw3 => Ok(Box::new(MobiAdapter)),
        Format::Pdf => Ok(Box::new(PdfAdapter)),
        Format::Djvu => Err(ParseError::Unsupported),
    }
}
```

- [ ] **Step 6: Run test and clippy to verify green**

Run: `cargo test -p kasane-adapters pdf_format_has_an_adapter && cargo clippy -p kasane-adapters -- -D warnings`
Expected: test PASS; clippy clean. If `cargo deny` (via `mise run lint`) later flags a new transitive advisory from `lopdf`, add a scoped `[advisories] ignore` entry to `deny.toml` with a one-line comment (mirror the existing `RUSTSEC-2021-0153` entry), and note it in the commit.

- [ ] **Step 7: Commit**

```bash
git add crates/kasane-adapters/Cargo.toml Cargo.lock crates/kasane-adapters/src/lib.rs crates/kasane-adapters/src/pdf/
git commit -m "feat(pdf): register PdfAdapter and add lopdf/png deps"
```

---

## Task 2: Hermetic PDF fixture generator

Build a stdlib-only Python generator (matching the `make_minimal_mobi.py` convention) that emits four fixtures used across later tasks.

**Files:**
- Create: `tests/fixtures/pdf/make_pdf_fixtures.py`
- Create (generated): `tests/fixtures/pdf/{minimal,no-outline,image,scanned}.pdf`
- Test: `crates/kasane-adapters/src/pdf/doc.rs` (temporary magic-byte test, replaced in Task 3)

**Interfaces:**
- Produces: on-disk fixtures. `minimal.pdf`: 2 pages, an `/Outlines` with 2 entries ("Chapter One"→page 1, "Section Two"→page 2), body text on each page. `no-outline.pdf`: 1 page, one large-font line ("Big Title") + body lines, no outline. `image.pdf`: 1 page, body text + one 2×2 DeviceRGB FlateDecode image. `scanned.pdf`: 1 page, one 2×2 DeviceRGB FlateDecode image and **no text**.

- [ ] **Step 1: Write the generator**

Create `tests/fixtures/pdf/make_pdf_fixtures.py`:

```python
#!/usr/bin/env python3
"""Hermetic PDF fixture generator (stdlib only). Regenerate with:
    python3 tests/fixtures/pdf/make_pdf_fixtures.py
Emits minimal.pdf, no-outline.pdf, image.pdf, scanned.pdf next to this file.
"""
import os
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))


class Pdf:
    """Minimal PDF writer that tracks object byte offsets for the xref table."""

    def __init__(self):
        self.objects = {}   # num -> bytes (object body, without "N 0 obj"/"endobj")
        self.order = []     # emission order of object numbers

    def add(self, num, body: bytes):
        self.objects[num] = body
        self.order.append(num)

    def stream_obj(self, dict_extra: bytes, data: bytes) -> bytes:
        return b"<< /Length %d %s >>\nstream\n%s\nendstream" % (len(data), dict_extra, data)

    def build(self, root_num: int) -> bytes:
        out = bytearray(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n")
        offsets = {}
        for num in self.order:
            offsets[num] = len(out)
            out += b"%d 0 obj\n" % num
            out += self.objects[num]
            out += b"\nendobj\n"
        xref_pos = len(out)
        max_num = max(self.order)
        out += b"xref\n0 %d\n" % (max_num + 1)
        out += b"0000000000 65535 f \n"
        for num in range(1, max_num + 1):
            if num in offsets:
                out += b"%010d 00000 n \n" % offsets[num]
            else:
                out += b"0000000000 65535 f \n"
        out += b"trailer\n<< /Size %d /Root %d 0 R >>\n" % (max_num + 1, root_num)
        out += b"startxref\n%d\n%%%%EOF" % xref_pos
        return bytes(out)


def text_stream(ops: bytes) -> bytes:
    return b"BT\n" + ops + b"ET\n"


def show(x, y, size, s: str) -> bytes:
    esc = s.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
    return b"/F1 %d Tf\n1 0 0 1 %d %d Tm\n(%s) Tj\n" % (size, x, y, esc.encode("latin-1"))


def rgb_image_stream(w: int, h: int) -> bytes:
    # w*h RGB pixels, deflate-compressed => /Filter /FlateDecode.
    raw = bytes([((i * 37) % 256) for i in range(w * h * 3)])
    return zlib.compress(raw)


def font_obj() -> bytes:
    return b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"


def build_minimal() -> bytes:
    p = Pdf()
    # 1 catalog, 2 pages tree, 3+4 page, 5+6 content, 7 font, 8 outlines, 9+10 outline items
    p.add(1, b"<< /Type /Catalog /Pages 2 0 R /Outlines 8 0 R >>")
    p.add(2, b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>")
    p.add(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
             b"/Resources << /Font << /F1 7 0 R >> >> /Contents 5 0 R >>")
    p.add(4, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
             b"/Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>")
    c1 = text_stream(show(20, 170, 12, "Chapter One") + show(20, 150, 12, "First body line."))
    c2 = text_stream(show(20, 170, 12, "Section Two") + show(20, 150, 12, "Second body line."))
    p.add(5, p.stream_obj(b"", c1))
    p.add(6, p.stream_obj(b"", c2))
    p.add(7, font_obj())
    p.add(8, b"<< /Type /Outlines /First 9 0 R /Last 10 0 R /Count 2 >>")
    p.add(9, b"<< /Title (Chapter One) /Parent 8 0 R /Next 10 0 R /Dest [3 0 R /Fit] >>")
    p.add(10, b"<< /Title (Section Two) /Parent 8 0 R /Prev 9 0 R /Dest [4 0 R /Fit] >>")
    return p.build(1)


def build_no_outline() -> bytes:
    p = Pdf()
    p.add(1, b"<< /Type /Catalog /Pages 2 0 R >>")
    p.add(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    p.add(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
             b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>")
    body = (show(20, 170, 24, "Big Title")
            + show(20, 140, 12, "Ordinary paragraph text one.")
            + show(20, 124, 12, "Ordinary paragraph text two."))
    p.add(4, font_obj())
    p.add(5, p.stream_obj(b"", text_stream(body)))
    return p.build(1)


def build_image(with_text: bool) -> bytes:
    p = Pdf()
    p.add(1, b"<< /Type /Catalog /Pages 2 0 R >>")
    p.add(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    p.add(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
             b"/Resources << /Font << /F1 4 0 R >> /XObject << /Im0 6 0 R >> >> "
             b"/Contents 5 0 R >>")
    p.add(4, font_obj())
    ops = b"q 100 0 0 100 20 20 cm /Im0 Do Q\n"
    if with_text:
        ops = text_stream(show(20, 170, 12, "Figure caption text.")) + ops
    p.add(5, p.stream_obj(b"", ops))
    img = rgb_image_stream(2, 2)
    p.add(6, p.stream_obj(
        b"/Type /XObject /Subtype /Image /Width 2 /Height 2 "
        b"/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode", img))
    return p.build(1)


def main():
    open(os.path.join(HERE, "minimal.pdf"), "wb").write(build_minimal())
    open(os.path.join(HERE, "no-outline.pdf"), "wb").write(build_no_outline())
    open(os.path.join(HERE, "image.pdf"), "wb").write(build_image(with_text=True))
    open(os.path.join(HERE, "scanned.pdf"), "wb").write(build_image(with_text=False))
    print("wrote minimal.pdf, no-outline.pdf, image.pdf, scanned.pdf")


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Generate the fixtures**

Run: `python3 tests/fixtures/pdf/make_pdf_fixtures.py && ls -l tests/fixtures/pdf/*.pdf`
Expected: prints the "wrote ..." line; four `.pdf` files exist and are non-empty.

- [ ] **Step 3: Write a temporary validity test**

Put this in `crates/kasane-adapters/src/pdf/doc.rs` (it is replaced in Task 3):

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn fixtures_load_in_lopdf() {
        for name in ["minimal", "no-outline", "image", "scanned"] {
            let path = format!("../../tests/fixtures/pdf/{name}.pdf");
            let bytes = std::fs::read(&path).unwrap_or_else(|_| panic!("missing {path}"));
            assert!(bytes.starts_with(b"%PDF"), "{name} lacks %PDF magic");
            let doc = lopdf::Document::load_mem(&bytes)
                .unwrap_or_else(|e| panic!("{name} failed to load: {e}"));
            assert!(doc.get_pages().len() >= 1, "{name} has no pages");
        }
    }
}
```

- [ ] **Step 4: Run the validity test**

Run: `cargo test -p kasane-adapters fixtures_load_in_lopdf`
Expected: PASS — all four fixtures load and report ≥1 page. (`minimal.pdf` reports 2.)

- [ ] **Step 5: Commit**

```bash
git add tests/fixtures/pdf/ crates/kasane-adapters/src/pdf/doc.rs
git commit -m "test(pdf): hermetic fixture generator (text, outline, image, scanned)"
```

---

## Task 3: `doc.rs` — open, decrypt, enumerate pages

**Files:**
- Modify: `crates/kasane-adapters/src/pdf/doc.rs` (replace the temporary test module)

**Interfaces:**
- Produces:
  - `pub const MAX_CONTENT_BYTES: usize` — per-stream decompression cap.
  - `pub fn open(bytes: &[u8]) -> Result<lopdf::Document, ParseError>` — loads, and if encrypted, attempts empty-password decryption; maps failures to `ParseError`.
  - `pub fn pages(doc: &lopdf::Document) -> Vec<(u32, lopdf::ObjectId)>` — 1-based page number → page object id, ascending.
- Consumes: `crate::ParseError`, `crate::guard::MAX_TOTAL_BYTES`.

- [ ] **Step 1: Write the failing tests**

Replace the entire contents of `crates/kasane-adapters/src/pdf/doc.rs` with:

```rust
use crate::guard::MAX_TOTAL_BYTES;
use crate::ParseError;
use lopdf::{Document, ObjectId};

/// Per-page content-stream decompression cap (bomb guard).
pub const MAX_CONTENT_BYTES: usize = MAX_TOTAL_BYTES as usize;

/// Open a PDF from bytes. If the document is encrypted, attempt decryption with
/// the empty user password (the common "permissions only" case). A real user
/// password yields `ParseError::Encrypted`; we never crack or prompt.
pub fn open(bytes: &[u8]) -> Result<Document, ParseError> {
    let mut doc = Document::load_mem(bytes).map_err(|e| ParseError::Malformed(e.to_string()))?;
    if doc.is_encrypted() {
        doc.decrypt("").map_err(|_| ParseError::Encrypted)?;
    }
    Ok(doc)
}

/// 1-based page number → page object id, ascending by page number.
pub fn pages(doc: &Document) -> Vec<(u32, ObjectId)> {
    let mut v: Vec<(u32, ObjectId)> = doc.get_pages().into_iter().collect();
    v.sort_by_key(|(n, _)| *n);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(name: &str) -> Vec<u8> {
        std::fs::read(format!("../../tests/fixtures/pdf/{name}.pdf")).unwrap()
    }

    #[test]
    fn opens_and_counts_pages() {
        let doc = open(&read("minimal")).unwrap();
        assert_eq!(pages(&doc).len(), 2);
        // page numbers are 1-based and ascending
        let nums: Vec<u32> = pages(&doc).iter().map(|(n, _)| *n).collect();
        assert_eq!(nums, vec![1, 2]);
    }

    #[test]
    fn rejects_non_pdf() {
        assert!(matches!(open(b"not a pdf"), Err(ParseError::Malformed(_))));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail, then pass**

Run: `cargo test -p kasane-adapters pdf::doc`
Expected: the two tests compile and PASS (`open` and `pages` are now defined). If red, fix compile errors shown by the compiler before proceeding.

- [ ] **Step 3: Verify clippy is clean**

Run: `cargo clippy -p kasane-adapters -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/kasane-adapters/src/pdf/doc.rs
git commit -m "feat(pdf): open + empty-password decrypt + page enumeration"
```

---

## Task 4: `content.rs` — content-stream interpreter → positioned text runs

Interpret a page's content stream, tracking the CTM (`q`/`Q`/`cm`) and text matrix (`BT`/`Tf`/`Td`/`TD`/`Tm`/`T*`/`TL`), decoding shown text (`Tj`/`TJ`/`'`/`"`) via each font's lopdf `Encoding` (which handles `/ToUnicode`). Emit one `TextRun` per show operator.

**Files:**
- Modify: `crates/kasane-adapters/src/pdf/content.rs`

**Interfaces:**
- Produces:
  - `pub struct TextRun { pub x: f32, pub y: f32, pub size: f32, pub text: String }`
  - `pub fn page_text_runs(doc: &lopdf::Document, page_id: lopdf::ObjectId) -> Vec<TextRun>` — never panics; returns `vec![]` on any decode error.
- Consumes: `super::doc::MAX_CONTENT_BYTES`.

- [ ] **Step 1: Write the failing test**

Put this at the bottom of `crates/kasane-adapters/src/pdf/content.rs` (the implementation goes above it in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::doc::{open, pages};

    fn runs(name: &str) -> Vec<TextRun> {
        let bytes = std::fs::read(format!("../../tests/fixtures/pdf/{name}.pdf")).unwrap();
        let doc = open(&bytes).unwrap();
        let (_, page1) = pages(&doc)[0];
        page_text_runs(&doc, page1)
    }

    #[test]
    fn extracts_positioned_text_from_page_one() {
        let r = runs("minimal");
        let texts: Vec<&str> = r.iter().map(|t| t.text.as_str()).collect();
        assert!(texts.contains(&"Chapter One"), "got {texts:?}");
        assert!(texts.contains(&"First body line."), "got {texts:?}");
        // Tm placed "Chapter One" at y=170, size 12.
        let title = r.iter().find(|t| t.text == "Chapter One").unwrap();
        assert!((title.y - 170.0).abs() < 1.0, "y was {}", title.y);
        assert!((title.size - 12.0).abs() < 0.5, "size was {}", title.size);
        assert!((title.x - 20.0).abs() < 1.0, "x was {}", title.x);
    }

    #[test]
    fn font_size_survives_for_large_heading() {
        let r = runs("no-outline");
        let big = r.iter().find(|t| t.text == "Big Title").unwrap();
        assert!(big.size > 20.0, "expected large size, got {}", big.size);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-adapters pdf::content`
Expected: FAIL to compile — `TextRun` / `page_text_runs` not defined.

- [ ] **Step 3: Write the interpreter**

Insert above the test module in `crates/kasane-adapters/src/pdf/content.rs`:

```rust
use super::doc::MAX_CONTENT_BYTES;
use lopdf::content::Content;
use lopdf::{Document, Encoding, Object, ObjectId};
use std::collections::BTreeMap;

/// One text-showing operation, positioned in device space.
#[derive(Clone, Debug)]
pub struct TextRun {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub text: String,
}

/// 2×3 affine matrix [a b c d e f] using the PDF row-vector convention
/// (a point [x y 1] is transformed as [x y 1] · M).
type Mat = [f32; 6];
const IDENT: Mat = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// m · n (apply m first, then n).
fn mul(m: Mat, n: Mat) -> Mat {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

fn translate(tx: f32, ty: f32) -> Mat {
    [1.0, 0.0, 0.0, 1.0, tx, ty]
}

fn nums(operands: &[Object]) -> Vec<f32> {
    operands.iter().map(|o| o.as_float().unwrap_or(0.0)).collect()
}

/// Interpret a page's content stream into positioned, Unicode-decoded text runs.
/// Never panics; returns an empty vec if the page has no readable content.
pub fn page_text_runs(doc: &Document, page_id: ObjectId) -> Vec<TextRun> {
    let fonts = doc.get_page_fonts(page_id).unwrap_or_default();
    let encodings: BTreeMap<Vec<u8>, Encoding> = fonts
        .into_iter()
        .filter_map(|(name, font)| font.get_font_encoding(doc).ok().map(|enc| (name, enc)))
        .collect();

    let bytes = match doc.get_page_content_with_limit(page_id, MAX_CONTENT_BYTES) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let content = match Content::decode(&bytes) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut runs = Vec::new();
    let mut ctm_stack: Vec<Mat> = Vec::new();
    let mut ctm = IDENT;
    let mut tm = IDENT; // text matrix
    let mut tlm = IDENT; // text line matrix
    let mut font_size = 0.0f32;
    let mut leading = 0.0f32;
    let mut encoding: Option<&Encoding> = None;

    for op in &content.operations {
        match op.operator.as_str() {
            "q" => ctm_stack.push(ctm),
            "Q" => {
                if let Some(m) = ctm_stack.pop() {
                    ctm = m;
                }
            }
            "cm" => {
                let n = nums(&op.operands);
                if n.len() == 6 {
                    ctm = mul([n[0], n[1], n[2], n[3], n[4], n[5]], ctm);
                }
            }
            "BT" => {
                tm = IDENT;
                tlm = IDENT;
            }
            "Tf" => {
                if let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                    encoding = encodings.get(name);
                }
                font_size = op.operands.get(1).and_then(|o| o.as_float().ok()).unwrap_or(font_size);
            }
            "TL" => leading = op.operands.first().and_then(|o| o.as_float().ok()).unwrap_or(leading),
            "Td" => {
                let n = nums(&op.operands);
                if n.len() == 2 {
                    tlm = mul(translate(n[0], n[1]), tlm);
                    tm = tlm;
                }
            }
            "TD" => {
                let n = nums(&op.operands);
                if n.len() == 2 {
                    leading = -n[1];
                    tlm = mul(translate(n[0], n[1]), tlm);
                    tm = tlm;
                }
            }
            "Tm" => {
                let n = nums(&op.operands);
                if n.len() == 6 {
                    tlm = [n[0], n[1], n[2], n[3], n[4], n[5]];
                    tm = tlm;
                }
            }
            "T*" => {
                tlm = mul(translate(0.0, -leading), tlm);
                tm = tlm;
            }
            "Tj" | "TJ" => {
                if let Some(enc) = encoding {
                    if let Some(run) = show(&op.operands, enc, tm, ctm, font_size) {
                        runs.push(run);
                    }
                }
            }
            "'" => {
                tlm = mul(translate(0.0, -leading), tlm);
                tm = tlm;
                if let Some(enc) = encoding {
                    if let Some(run) = show(&op.operands, enc, tm, ctm, font_size) {
                        runs.push(run);
                    }
                }
            }
            "\"" => {
                tlm = mul(translate(0.0, -leading), tlm);
                tm = tlm;
                if let (Some(enc), Some(s)) = (encoding, op.operands.get(2)) {
                    if let Some(run) = show(std::slice::from_ref(s), enc, tm, ctm, font_size) {
                        runs.push(run);
                    }
                }
            }
            _ => {}
        }
    }
    runs
}

/// Build a TextRun from a show operator's operands at the current matrices.
fn show(operands: &[Object], enc: &Encoding, tm: Mat, ctm: Mat, font_size: f32) -> Option<TextRun> {
    let mut text = String::new();
    decode_into(operands, enc, &mut text);
    if text.trim().is_empty() {
        return None;
    }
    let trm = mul(tm, ctm); // text rendering matrix (translation + scale, ignoring rise)
    // vertical scale magnitude of the composed matrix
    let yscale = (trm[1] * trm[1] + trm[3] * trm[3]).sqrt();
    Some(TextRun {
        x: trm[4],
        y: trm[5],
        size: font_size * if yscale.is_finite() && yscale > 0.0 { yscale } else { 1.0 },
        text,
    })
}

/// Append decoded text from Tj (string) / TJ (array of strings + kerning numbers).
/// A large negative kerning advance is rendered as a space.
fn decode_into(operands: &[Object], enc: &Encoding, out: &mut String) {
    for op in operands {
        match op {
            Object::String(bytes, _) => {
                if let Ok(s) = enc.bytes_to_string(bytes) {
                    out.push_str(&s);
                }
            }
            Object::Array(arr) => {
                for el in arr {
                    match el {
                        Object::String(bytes, _) => {
                            if let Ok(s) = enc.bytes_to_string(bytes) {
                                out.push_str(&s);
                            }
                        }
                        Object::Real(n) if *n <= -100.0 => out.push(' '),
                        Object::Integer(n) if *n <= -100 => out.push(' '),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters pdf::content`
Expected: both tests PASS.

- [ ] **Step 5: Verify clippy is clean**

Run: `cargo clippy -p kasane-adapters -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-adapters/src/pdf/content.rs
git commit -m "feat(pdf): content-stream interpreter -> positioned text runs"
```

---

## Task 5: `layout.rs` — runs → lines → paragraphs + font-size headings

Pure functions over `Vec<TextRun>`. No PDF/lopdf types, so tests are synthetic.

**Files:**
- Modify: `crates/kasane-adapters/src/pdf/layout.rs`

**Interfaces:**
- Consumes: `super::content::TextRun`, `kasane_ir::{Block, BlockId, Inline}`.
- Produces:
  - `pub struct Line { pub y: f32, pub x: f32, pub size: f32, pub text: String }`
  - `pub fn group_lines(runs: Vec<TextRun>) -> Vec<Line>` — reading order (top→bottom, left→right), runs on one y-band merged into a line.
  - `pub fn modal_body_size(pages: &[Vec<Line>]) -> f32` — the most common rounded line size across the document (0.0 if empty).
  - `pub fn page_blocks_no_headings(lines: &[Line], next_id: &mut u32, body_size: f32) -> Vec<Block>` — paragraphs, with a line promoted to `Heading` when its size exceeds `body_size` by ≥15%.

- [ ] **Step 1: Write the failing tests**

Put at the bottom of `crates/kasane-adapters/src/pdf/layout.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::content::TextRun;
    use kasane_ir::{Block, Inline};

    fn run(x: f32, y: f32, size: f32, t: &str) -> TextRun {
        TextRun { x, y, size, text: t.into() }
    }

    fn heading_text(b: &Block) -> Option<(u8, String)> {
        if let Block::Heading { level, inlines, .. } = b {
            Some((*level, inline_text(inlines)))
        } else {
            None
        }
    }
    fn para_text(b: &Block) -> Option<String> {
        if let Block::Para(inlines) = b { Some(inline_text(inlines)) } else { None }
    }
    fn inline_text(inls: &[Inline]) -> String {
        inls.iter().map(|i| match i { Inline::Text(t) => t.clone(), _ => String::new() }).collect()
    }

    #[test]
    fn groups_runs_into_lines_in_reading_order() {
        // Two runs on the same line (same y), then a lower line.
        let runs = vec![
            run(60.0, 170.0, 12.0, "world"),
            run(20.0, 170.0, 12.0, "hello"),
            run(20.0, 150.0, 12.0, "next"),
        ];
        let lines = group_lines(runs);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "hello world"); // sorted by x within the line
        assert_eq!(lines[1].text, "next");
    }

    #[test]
    fn promotes_large_line_to_heading_and_merges_body() {
        let page = vec![
            run(20.0, 190.0, 24.0, "Big Title"),
            run(20.0, 160.0, 12.0, "Body line one."),
            run(20.0, 146.0, 12.0, "Body line two."),
        ];
        let lines = group_lines(page);
        let body = modal_body_size(&[lines.clone()]);
        assert!((body - 12.0).abs() < 0.01, "body size {body}");
        let mut id = 0u32;
        let blocks = page_blocks_no_headings(&lines, &mut id, body);
        assert_eq!(heading_text(&blocks[0]), Some((1, "Big Title".into())));
        // The two 12pt lines merge into a single paragraph.
        assert_eq!(para_text(&blocks[1]).as_deref(), Some("Body line one. Body line two."));
        assert_eq!(blocks.len(), 2);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters pdf::layout`
Expected: FAIL to compile — `Line` / `group_lines` / `modal_body_size` / `page_blocks_no_headings` not defined.

- [ ] **Step 3: Write the layout logic**

Insert above the test module:

```rust
use super::content::TextRun;
use kasane_ir::{Block, BlockId, Inline};

/// A single visual line of text.
#[derive(Clone, Debug)]
pub struct Line {
    pub y: f32,
    pub x: f32,
    pub size: f32,
    pub text: String,
}

/// Group runs into lines (same y-band) in reading order: top→bottom, left→right.
pub fn group_lines(mut runs: Vec<TextRun>) -> Vec<Line> {
    if runs.is_empty() {
        return Vec::new();
    }
    // Sort top→bottom (larger y first), then left→right.
    runs.sort_by(|a, b| {
        b.y.partial_cmp(&a.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut lines: Vec<Line> = Vec::new();
    for r in runs {
        // y-tolerance scales with font size (sub/superscripts stay on the line).
        let tol = (r.size * 0.5).max(2.0);
        match lines.last_mut() {
            Some(last) if (last.y - r.y).abs() <= tol => {
                if !last.text.ends_with(' ') && !r.text.starts_with(' ') {
                    last.text.push(' ');
                }
                last.text.push_str(r.text.trim());
                last.size = last.size.max(r.size);
                last.x = last.x.min(r.x);
            }
            _ => lines.push(Line { y: r.y, x: r.x, size: r.size, text: r.text.trim().to_string() }),
        }
    }
    for l in &mut lines {
        l.text = l.text.trim().to_string();
    }
    lines.retain(|l| !l.text.is_empty());
    lines
}

/// Most common rounded line size across all pages — the document's body size.
pub fn modal_body_size(pages: &[Vec<Line>]) -> f32 {
    use std::collections::HashMap;
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for page in pages {
        for l in page {
            *counts.entry(l.size.round() as i32).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(sz, _)| sz as f32)
        .unwrap_or(0.0)
}

const HEADING_RATIO: f32 = 1.15;

/// Build paragraph/heading blocks for one page, with no outline available.
/// A line ≥15% larger than the body size becomes a heading; consecutive
/// body-size lines merge into a paragraph, split on large vertical gaps.
pub fn page_blocks_no_headings(lines: &[Line], next_id: &mut u32, body_size: f32) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut prev_y: Option<f32> = None;
    let mut prev_size = body_size;

    let flush = |blocks: &mut Vec<Block>, para: &mut Vec<String>| {
        if !para.is_empty() {
            let text = para.join(" ");
            blocks.push(Block::Para(vec![Inline::Text(text)]));
            para.clear();
        }
    };

    for l in lines {
        let is_heading = body_size > 0.0 && l.size >= body_size * HEADING_RATIO;
        if is_heading {
            flush(&mut blocks, &mut para);
            let id = BlockId(*next_id);
            *next_id += 1;
            blocks.push(Block::Heading {
                level: heading_level(l.size, body_size),
                id,
                inlines: vec![Inline::Text(l.text.clone())],
            });
        } else {
            // Paragraph break on a vertical gap larger than 1.5× line height.
            if let Some(py) = prev_y {
                if (py - l.y) > prev_size.max(l.size) * 1.5 {
                    flush(&mut blocks, &mut para);
                }
            }
            para.push(l.text.clone());
        }
        prev_y = Some(l.y);
        prev_size = l.size;
    }
    flush(&mut blocks, &mut para);
    blocks
}

/// Bucket a heading size into levels 1–3 by how far it exceeds the body size.
fn heading_level(size: f32, body: f32) -> u8 {
    let ratio = if body > 0.0 { size / body } else { 1.0 };
    if ratio >= 1.8 {
        1
    } else if ratio >= 1.4 {
        2
    } else {
        3
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters pdf::layout`
Expected: both tests PASS.

- [ ] **Step 5: Verify clippy is clean**

Run: `cargo clippy -p kasane-adapters -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-adapters/src/pdf/layout.rs
git commit -m "feat(pdf): line grouping, paragraph merge, font-size headings"
```

---

## Task 6: `outline.rs` — `/Outlines` → per-page headings

**Files:**
- Modify: `crates/kasane-adapters/src/pdf/outline.rs`

**Interfaces:**
- Produces:
  - `pub struct OutlineHeading { pub level: u8, pub title: String }`
  - `pub fn outline_by_page(doc: &lopdf::Document) -> std::collections::BTreeMap<u32, Vec<OutlineHeading>>` — page number → headings targeting that page, in outline order. Empty map when the document has no outline.
- Consumes: `lopdf::Document::get_toc`.

- [ ] **Step 1: Write the failing tests**

Put at the bottom of `crates/kasane-adapters/src/pdf/outline.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::doc::open;

    fn doc(name: &str) -> lopdf::Document {
        open(&std::fs::read(format!("../../tests/fixtures/pdf/{name}.pdf")).unwrap()).unwrap()
    }

    #[test]
    fn maps_outline_entries_to_pages() {
        let map = outline_by_page(&doc("minimal"));
        assert_eq!(map.get(&1).unwrap()[0].title, "Chapter One");
        assert_eq!(map.get(&2).unwrap()[0].title, "Section Two");
        assert_eq!(map.get(&1).unwrap()[0].level, 1);
    }

    #[test]
    fn empty_when_no_outline() {
        assert!(outline_by_page(&doc("no-outline")).is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters pdf::outline`
Expected: FAIL to compile — `OutlineHeading` / `outline_by_page` not defined.

- [ ] **Step 3: Write the outline mapping**

Insert above the test module:

```rust
use lopdf::Document;
use std::collections::BTreeMap;

/// A heading derived from a `/Outlines` entry.
#[derive(Clone, Debug)]
pub struct OutlineHeading {
    pub level: u8,
    pub title: String,
}

/// Map each page number to the outline headings that target it, in outline
/// order. lopdf's `get_toc` resolves destinations to page numbers and levels;
/// a document without an outline yields an empty map (never an error).
pub fn outline_by_page(doc: &Document) -> BTreeMap<u32, Vec<OutlineHeading>> {
    let mut map: BTreeMap<u32, Vec<OutlineHeading>> = BTreeMap::new();
    let Ok(toc) = doc.get_toc() else {
        return map; // Error::NoOutline (or any error) -> no outline headings
    };
    for entry in toc.toc {
        let page = entry.page as u32;
        let title = entry.title.trim().to_string();
        if page == 0 || title.is_empty() {
            continue;
        }
        // Outline depth is 1-based in lopdf; clamp to the IR heading range 1–6.
        let level = entry.level.clamp(1, 6) as u8;
        map.entry(page).or_default().push(OutlineHeading { level, title });
    }
    map
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters pdf::outline`
Expected: both tests PASS.

- [ ] **Step 5: Verify clippy is clean**

Run: `cargo clippy -p kasane-adapters -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-adapters/src/pdf/outline.rs
git commit -m "feat(pdf): outline (/Outlines via get_toc) -> per-page headings"
```

---

## Task 7: `mod.rs` — assemble pages into a Document

Wire `doc` + `content` + `layout` + `outline` into `PdfAdapter::parse`, producing a `Document` with page provenance. Images come in Task 8; this task handles text + headings only.

**Files:**
- Modify: `crates/kasane-adapters/src/pdf/mod.rs`

**Interfaces:**
- Consumes: `doc::{open, pages, MAX_CONTENT_BYTES}`, `content::page_text_runs`, `layout::{group_lines, modal_body_size, page_blocks_no_headings, Line}`, `outline::outline_by_page`.
- Produces: a working `impl Adapter for PdfAdapter` (assets empty until Task 8). Adds `pub(crate) fn page_lines(...)` reuse is not required; keep helpers private.

- [ ] **Step 1: Write the failing tests**

Add a test module at the bottom of `crates/kasane-adapters/src/pdf/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kasane_ir::{Block, Inline};

    fn parse(name: &str) -> Document {
        let bytes = std::fs::read(format!("../../tests/fixtures/pdf/{name}.pdf")).unwrap();
        PdfAdapter.parse(&bytes, &format!("{name}.pdf")).unwrap().0
    }
    fn text(inls: &[Inline]) -> String {
        inls.iter().map(|i| match i { Inline::Text(t) => t.clone(), _ => String::new() }).collect()
    }
    fn headings(doc: &Document) -> Vec<(u8, String)> {
        doc.nodes.iter().filter_map(|n| match &n.block {
            Block::Heading { level, inlines, .. } => Some((*level, text(inlines))),
            _ => None,
        }).collect()
    }

    #[test]
    fn outline_headings_in_order_with_page_provenance() {
        let doc = parse("minimal");
        assert_eq!(doc.meta.source_format, "pdf");
        assert_eq!(headings(&doc), vec![(1, "Chapter One".into()), (1, "Section Two".into())]);
        // Every node carries a source page.
        assert!(doc.nodes.iter().all(|n| n.prov.source_pages.is_some()));
        // "Section Two" heading is provenanced to page 2.
        let sec = doc.nodes.iter().find(|n| matches!(&n.block,
            Block::Heading { inlines, .. } if text(inlines) == "Section Two")).unwrap();
        assert_eq!(sec.prov.source_pages, Some((2, 2)));
    }

    #[test]
    fn font_size_fallback_when_no_outline() {
        let doc = parse("no-outline");
        assert_eq!(headings(&doc), vec![(1, "Big Title".into())]);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters pdf::tests`
Expected: FAIL — `parse` still returns `Err(Malformed("pdf adapter not yet implemented"))`.

- [ ] **Step 3: Implement assembly**

Replace the body of `crates/kasane-adapters/src/pdf/mod.rs` above the test module with:

```rust
mod content;
pub(crate) mod doc;
mod image;
mod layout;
mod outline;

use crate::{Adapter, ParseError};
use content::page_text_runs;
use kasane_ir::*;
use layout::{group_lines, modal_body_size, page_blocks_no_headings, Line};
use outline::outline_by_page;

pub struct PdfAdapter;

impl Adapter for PdfAdapter {
    fn parse(&self, bytes: &[u8], source_path: &str) -> Result<(Document, AssetBag), ParseError> {
        let pdf = doc::open(bytes)?;
        let page_list = doc::pages(&pdf);
        let outline = outline_by_page(&pdf);

        // First pass: group each page's text into lines (needed for the doc-wide body size).
        let page_lines: Vec<(u32, Line0)> = page_list
            .iter()
            .map(|&(num, id)| (num, Line0 { id, lines: group_lines(page_text_runs(&pdf, id)) }))
            .collect();
        let all_lines: Vec<Vec<Line>> = page_lines.iter().map(|(_, p)| p.lines.clone()).collect();
        let body_size = modal_body_size(&all_lines);
        let has_outline = !outline.is_empty();

        let mut nodes = Vec::new();
        let mut next_id = 0u32;

        for (num, page) in &page_lines {
            let prov = Provenance { source_pages: Some((*num, *num)), source_href: None };

            // Outline headings for this page come first, at page granularity.
            if let Some(hs) = outline.get(num) {
                for h in hs {
                    let id = BlockId(next_id);
                    next_id += 1;
                    nodes.push(Node {
                        block: Block::Heading {
                            level: h.level,
                            id,
                            inlines: vec![Inline::Text(h.title.clone())],
                        },
                        prov: prov.clone(),
                    });
                }
            }

            // Body blocks. Suppress the font-size heading fallback when the
            // document has a real outline (avoid double headings).
            let effective_body = if has_outline { f32::MAX } else { body_size };
            let blocks = page_blocks_no_headings(&page.lines, &mut next_id, effective_body);
            if blocks.is_empty() && outline.get(num).is_none() {
                // Nothing extracted for this page yet (may be image-only; Task 8
                // adds figures/scanned notes). Keep the page represented.
                nodes.push(Node { block: Block::Raw { note: raw_empty_note(*num) }, prov: prov.clone() });
            }
            for b in blocks {
                nodes.push(Node { block: b, prov: prov.clone() });
            }
        }

        let doc_out = Document {
            meta: DocMeta {
                title: derive_title(&pdf, source_path),
                authors: pdf_authors(&pdf),
                language: None,
                source_format: "pdf".into(),
                source_path: source_path.to_string(),
            },
            nodes,
        };
        Ok((doc_out, AssetBag::default()))
    }
}

/// Per-page grouped lines plus the page object id.
struct Line0 {
    id: lopdf::ObjectId,
    lines: Vec<Line>,
}

fn raw_empty_note(page: u32) -> String {
    format!("page {page}: no extractable text")
}

/// Title from the document Info dictionary, falling back to the file stem.
fn derive_title(pdf: &lopdf::Document, source_path: &str) -> String {
    if let Some(t) = info_string(pdf, b"Title") {
        if !t.trim().is_empty() {
            return t;
        }
    }
    source_path
        .rsplit(['/', '\\'])
        .next()
        .and_then(|f| f.strip_suffix(".pdf").or(Some(f)))
        .unwrap_or("document")
        .to_string()
}

fn pdf_authors(pdf: &lopdf::Document) -> Vec<String> {
    match info_string(pdf, b"Author") {
        Some(a) if !a.trim().is_empty() => vec![a],
        _ => vec![],
    }
}

/// Read a UTF-8/PDFDocEncoded string from the trailer's /Info dictionary.
fn info_string(pdf: &lopdf::Document, key: &[u8]) -> Option<String> {
    let info_ref = pdf.trailer.get(b"Info").ok()?.as_reference().ok()?;
    let dict = pdf.get_dictionary(info_ref).ok()?;
    let obj = dict.get(key).ok()?;
    let bytes = obj.as_str().ok()?;
    Some(String::from_utf8_lossy(bytes).into_owned())
}
```

Note: `Line0.id` is unused until Task 8 (image extraction needs the page id). Add `#[allow(dead_code)]` above the `id` field for this task to keep clippy quiet, and remove the allow in Task 8:

```rust
struct Line0 {
    #[allow(dead_code)]
    id: lopdf::ObjectId,
    lines: Vec<Line>,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters pdf::tests`
Expected: both tests PASS.

- [ ] **Step 5: Verify clippy + whole-crate tests**

Run: `cargo clippy -p kasane-adapters -- -D warnings && cargo test -p kasane-adapters`
Expected: clippy clean; all adapter tests (epub/pptx/mobi/pdf) pass.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-adapters/src/pdf/mod.rs
git commit -m "feat(pdf): assemble pages into IR with outline headings + provenance"
```

---

## Task 8: `image.rs` — embedded images and scanned-page notes

Extract each page's `/XObject` images into the `AssetBag` as `Block::Figure`s, and mark text-less image pages with a scanned-page `Raw` note.

**Files:**
- Modify: `crates/kasane-adapters/src/pdf/image.rs`
- Modify: `crates/kasane-adapters/src/pdf/mod.rs` (call image extraction per page; drop the Task-7 `#[allow(dead_code)]`)

**Interfaces:**
- Consumes: `lopdf::Document`, `kasane_ir::{AssetBag, AssetItem, AssetRef, Block}`, `crate::guard::MAX_TOTAL_BYTES`.
- Produces:
  - `pub struct PageImages { pub figures: Vec<Block>, pub had_image: bool, pub skipped: Vec<String> }`
  - `pub fn extract_page_images(doc: &lopdf::Document, page_id: lopdf::ObjectId, assets: &mut AssetBag) -> PageImages` — appends `AssetItem`s to `assets`, returns matching `Block::Figure`s. `FlateDecode` DeviceGray/DeviceRGB 8-bit → PNG; `DCTDecode` → JPEG passthrough; anything else → a `skipped` filter name (caller emits a `Raw` note).

- [ ] **Step 1: Write the failing tests**

Put at the bottom of `crates/kasane-adapters/src/pdf/image.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::doc::{open, pages};
    use kasane_ir::AssetBag;

    fn extract(name: &str) -> (PageImages, AssetBag) {
        let doc = open(&std::fs::read(format!("../../tests/fixtures/pdf/{name}.pdf")).unwrap()).unwrap();
        let (_, page1) = pages(&doc)[0];
        let mut assets = AssetBag::default();
        let pi = extract_page_images(&doc, page1, &mut assets);
        (pi, assets)
    }

    #[test]
    fn extracts_flate_rgb_image_as_png() {
        let (pi, assets) = extract("image");
        assert!(pi.had_image);
        assert_eq!(pi.figures.len(), 1);
        assert_eq!(assets.items.len(), 1);
        // FlateDecode RGB is re-encoded to PNG.
        assert!(assets.items[0].filename.ends_with(".png"));
        assert!(assets.items[0].bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn scanned_page_has_image_but_no_text() {
        let (pi, assets) = extract("scanned");
        assert!(pi.had_image);
        assert_eq!(assets.items.len(), 1);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters pdf::image`
Expected: FAIL to compile — `PageImages` / `extract_page_images` not defined.

- [ ] **Step 3: Write image extraction**

Insert above the test module in `crates/kasane-adapters/src/pdf/image.rs`:

```rust
use crate::guard::MAX_TOTAL_BYTES;
use kasane_ir::{AssetBag, AssetItem, AssetRef, Block};
use lopdf::{Document, Object, ObjectId};

/// Result of scanning one page for images.
pub struct PageImages {
    pub figures: Vec<Block>,
    pub had_image: bool,
    /// Filter names of images we recognized but could not decode.
    pub skipped: Vec<String>,
}

/// Extract a page's `/XObject` images. Supported: FlateDecode DeviceGray/RGB
/// 8-bit (re-encoded to PNG) and DCTDecode (JPEG passthrough). Others are
/// reported in `skipped` for the caller to note. Bomb-guarded per image.
pub fn extract_page_images(doc: &Document, page_id: ObjectId, assets: &mut AssetBag) -> PageImages {
    let mut figures = Vec::new();
    let mut skipped = Vec::new();
    let mut had_image = false;

    let xobjects = match page_xobject_ids(doc, page_id) {
        Some(x) => x,
        None => return PageImages { figures, had_image, skipped },
    };

    for id in xobjects {
        let Ok(obj) = doc.get_object(id) else { continue };
        let Ok(stream) = obj.as_stream() else { continue };
        let dict = &stream.dict;
        if dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) != Some(b"Image") {
            continue;
        }
        had_image = true;

        let filter = last_filter(dict);
        let key = format!("pdf-image-{}-{}", id.0, id.1);
        let idx = assets.items.len();

        match filter.as_deref() {
            Some(b"DCTDecode") => {
                let bytes = stream.content.clone();
                if bytes.len() as u64 > MAX_TOTAL_BYTES {
                    skipped.push("DCTDecode(too large)".into());
                    continue;
                }
                push_asset(assets, &mut figures, &key, idx, format!("{key}.jpg"), bytes);
            }
            Some(b"FlateDecode") => match flate_to_png(doc, stream) {
                Ok(png) => push_asset(assets, &mut figures, &key, idx, format!("{key}.png"), png),
                Err(reason) => skipped.push(reason),
            },
            other => {
                let name = other.map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_else(|| "unknown".into());
                skipped.push(name);
            }
        }
    }

    PageImages { figures, had_image, skipped }
}

fn push_asset(assets: &mut AssetBag, figures: &mut Vec<Block>, key: &str, idx: usize, filename: String, bytes: Vec<u8>) {
    assets.items.push(AssetItem { key: key.to_string(), filename, bytes });
    figures.push(Block::Figure {
        image: AssetRef { key: key.to_string(), bytes_ref: idx },
        caption: vec![],
        number: None,
    });
}

/// Page `/Resources /XObject` entries, resolved to object ids.
fn page_xobject_ids(doc: &Document, page_id: ObjectId) -> Option<Vec<ObjectId>> {
    let (resources, _) = doc.get_page_resources(page_id).ok()?;
    let resources = resources?;
    let xobj = resources.get(b"XObject").ok()?;
    let dict = match xobj {
        Object::Reference(r) => doc.get_dictionary(*r).ok()?,
        Object::Dictionary(d) => d,
        _ => return None,
    };
    Some(dict.iter().filter_map(|(_, v)| v.as_reference().ok()).collect())
}

/// The last filter in a stream dict's `/Filter` (Name or Array of Names).
fn last_filter(dict: &lopdf::Dictionary) -> Option<Vec<u8>> {
    match dict.get(b"Filter").ok()? {
        Object::Name(n) => Some(n.clone()),
        Object::Array(a) => a.last().and_then(|o| o.as_name().ok()).map(|n| n.to_vec()),
        _ => None,
    }
}

/// Re-encode a FlateDecode DeviceGray/DeviceRGB 8-bit image as PNG.
/// Returns Err(reason) for unsupported colorspaces/depths.
fn flate_to_png(doc: &Document, stream: &lopdf::Stream) -> Result<Vec<u8>, String> {
    let dict = &stream.dict;
    let width = dict.get(b"Width").and_then(|o| o.as_i64()).map_err(|_| "no width".to_string())? as u32;
    let height = dict.get(b"Height").and_then(|o| o.as_i64()).map_err(|_| "no height".to_string())? as u32;
    let bpc = dict.get(b"BitsPerComponent").and_then(|o| o.as_i64()).unwrap_or(8);
    if bpc != 8 {
        return Err(format!("FlateDecode({bpc}bpc)"));
    }
    let color = match colorspace_name(doc, dict) {
        Some(b"DeviceRGB") => png::ColorType::Rgb,
        Some(b"DeviceGray") => png::ColorType::Grayscale,
        _ => return Err("FlateDecode(colorspace)".to_string()),
    };
    let raw = stream
        .decompressed_content_with_limit(MAX_TOTAL_BYTES as usize)
        .map_err(|_| "FlateDecode(decompress)".to_string())?;
    let expected = (width as usize) * (height as usize) * color.samples();
    if raw.len() < expected {
        return Err("FlateDecode(short)".to_string());
    }

    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| format!("png header: {e}"))?;
        writer.write_image_data(&raw[..expected]).map_err(|e| format!("png data: {e}"))?;
    }
    Ok(out)
}

fn colorspace_name<'a>(doc: &'a Document, dict: &'a lopdf::Dictionary) -> Option<&'a [u8]> {
    match dict.get(b"ColorSpace").ok()? {
        Object::Name(n) => Some(n),
        Object::Reference(r) => doc.get_object(*r).ok()?.as_name().ok(),
        _ => None,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters pdf::image`
Expected: both tests PASS.

- [ ] **Step 5: Integrate image extraction into `mod.rs`**

In `crates/kasane-adapters/src/pdf/mod.rs`: remove the `#[allow(dead_code)]` on `Line0.id`, add `use image::extract_page_images;` near the other `use` lines, thread a mutable `AssetBag` through, and insert figures per page. Replace the per-page body-emission block inside the loop with:

```rust
        for (num, page) in &page_lines {
            let prov = Provenance { source_pages: Some((*num, *num)), source_href: None };

            if let Some(hs) = outline.get(num) {
                for h in hs {
                    let id = BlockId(next_id);
                    next_id += 1;
                    nodes.push(Node {
                        block: Block::Heading {
                            level: h.level,
                            id,
                            inlines: vec![Inline::Text(h.title.clone())],
                        },
                        prov: prov.clone(),
                    });
                }
            }

            let effective_body = if has_outline { f32::MAX } else { body_size };
            let text_blocks = page_blocks_no_headings(&page.lines, &mut next_id, effective_body);
            let has_text = !text_blocks.is_empty();
            for b in text_blocks {
                nodes.push(Node { block: b, prov: prov.clone() });
            }

            // Images, and a scanned-page note for text-less image pages.
            let imgs = extract_page_images(&pdf, page.id, &mut assets);
            for f in imgs.figures {
                nodes.push(Node { block: f, prov: prov.clone() });
            }
            if imgs.had_image && !has_text {
                nodes.push(Node {
                    block: Block::Raw { note: "scanned page: no text layer; OCR not enabled".into() },
                    prov: prov.clone(),
                });
            }
            for filter in imgs.skipped {
                nodes.push(Node {
                    block: Block::Raw { note: format!("image not extracted (filter: {filter})") },
                    prov: prov.clone(),
                });
            }

            // Fully empty page (no heading, text, or image) still gets represented.
            let page_has_heading = outline.get(num).is_some();
            if !has_text && !imgs.had_image && !page_has_heading {
                nodes.push(Node { block: Block::Raw { note: raw_empty_note(*num) }, prov: prov.clone() });
            }
        }
```

Declare `let mut assets = AssetBag::default();` before the loop (replacing the `AssetBag::default()` passed to `Ok((doc_out, ...))`), and return `Ok((doc_out, assets))`.

- [ ] **Step 6: Add the scanned-page assembly test**

Add to the `mod.rs` test module:

```rust
    #[test]
    fn scanned_page_yields_figure_and_note() {
        let bytes = std::fs::read("../../tests/fixtures/pdf/scanned.pdf").unwrap();
        let (doc, assets) = PdfAdapter.parse(&bytes, "scanned.pdf").unwrap();
        assert_eq!(assets.items.len(), 1);
        assert!(doc.nodes.iter().any(|n| matches!(&n.block, Block::Figure { .. })));
        assert!(doc.nodes.iter().any(|n| matches!(&n.block,
            Block::Raw { note } if note.contains("scanned page"))));
    }
```

- [ ] **Step 7: Run tests + clippy to verify green**

Run: `cargo test -p kasane-adapters pdf && cargo clippy -p kasane-adapters -- -D warnings`
Expected: all `pdf::*` tests pass; clippy clean.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-adapters/src/pdf/image.rs crates/kasane-adapters/src/pdf/mod.rs
git commit -m "feat(pdf): extract images (Flate->PNG, DCT passthrough) + scanned notes"
```

---

## Task 9: Encryption round-trip and CLI exit code

Prove the empty-password path converts and a real password is rejected as `Encrypted` with exit code 2.

**Files:**
- Modify: `crates/kasane-adapters/src/pdf/doc.rs` (add encryption tests)
- Modify: `crates/kasane-cli/src/main.rs` (route `"encrypted"` to exit 2)
- Modify: `crates/kasane-adapters/Cargo.toml` ( `tempfile` is already a workspace test dep for adapters? add under `[dev-dependencies]` only if missing)

**Interfaces:**
- Consumes: `lopdf::{Document, EncryptionState, EncryptionVersion, Permissions}`.
- Produces: no new public API; behavior + exit-code contract.

- [ ] **Step 1: Write the failing adapter tests**

Add to the `tests` module in `crates/kasane-adapters/src/pdf/doc.rs`:

```rust
    use lopdf::{EncryptionState, EncryptionVersion, Permissions};

    fn encrypt_minimal(owner: &str, user: &str) -> Vec<u8> {
        let mut doc = lopdf::Document::load_mem(&read("minimal")).unwrap();
        let version = EncryptionVersion::V1 {
            document: &doc,
            owner_password: owner,
            user_password: user,
            permissions: Permissions::all(),
        };
        let state = EncryptionState::try_from(version).unwrap();
        doc.encrypt(&state).unwrap();
        let mut buf = Vec::new();
        doc.save_to(&mut buf).unwrap();
        buf
    }

    #[test]
    fn decrypts_empty_user_password() {
        // Owner and user passwords both empty: the "permissions only" case.
        let bytes = encrypt_minimal("", "");
        let doc = open(&bytes).unwrap();
        assert_eq!(pages(&doc).len(), 2);
    }

    #[test]
    fn rejects_real_user_password() {
        // A non-empty owner AND user password => empty-password auth must fail.
        let bytes = encrypt_minimal("owner-secret", "user-secret");
        assert!(matches!(open(&bytes), Err(ParseError::Encrypted)));
    }
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters pdf::doc`
Expected: both new tests PASS (the `open` logic from Task 3 already handles this — these tests lock the contract). If `rejects_real_user_password` fails because `decrypt("")` authenticated via an empty *owner* password, confirm the owner password in `encrypt_minimal` is non-empty (it is here).

- [ ] **Step 3: Write the failing CLI exit-code test**

Add a test module at the bottom of `crates/kasane-cli/src/main.rs` (guarded so it only builds under test):

```rust
#[cfg(test)]
mod tests {
    // Exit-code classification must treat encrypted PDFs like DRM (code 2).
    fn code_for(msg: &str) -> u8 {
        if msg.contains("unsupported") || msg.contains("DRM") || msg.contains("encrypted") {
            2
        } else {
            1
        }
    }

    #[test]
    fn encrypted_maps_to_exit_two() {
        assert_eq!(code_for("encrypted content"), 2);
        assert_eq!(code_for("DRM-protected content is not supported"), 2);
        assert_eq!(code_for("malformed input: bad xref"), 1);
    }
}
```

- [ ] **Step 4: Run it to verify it fails**

Run: `cargo test -p kasane-cli encrypted_maps_to_exit_two`
Expected: FAIL to compile — `code_for` mirrors logic not yet extracted; the test defines its own `code_for`, so it will actually PASS. To make this a true red→green, instead refactor `main.rs` first: extract the classifier and have both `main` and the test call it. Do Step 5 now, then this test references the real function.

- [ ] **Step 5: Extract and extend the exit-code classifier in `main.rs`**

In `crates/kasane-cli/src/main.rs`, replace the inline classification inside `main` with a shared function and add `"encrypted"`:

```rust
/// Map an error message to an exit code: 2 for unsupported/DRM/encrypted, else 1.
fn exit_code_for(msg: &str) -> u8 {
    if msg.contains("unsupported") || msg.contains("DRM") || msg.contains("encrypted") {
        2
    } else {
        1
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(exit_code_for(&format!("{e:#}")))
        }
    }
}
```

Then change the test in Step 3 to call the real function:

```rust
#[cfg(test)]
mod tests {
    use super::exit_code_for;

    #[test]
    fn encrypted_maps_to_exit_two() {
        assert_eq!(exit_code_for("encrypted content"), 2);
        assert_eq!(exit_code_for("DRM-protected content is not supported"), 2);
        assert_eq!(exit_code_for("malformed input: bad xref"), 1);
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p kasane-cli encrypted_maps_to_exit_two && cargo test -p kasane-adapters pdf::doc`
Expected: all PASS.

- [ ] **Step 7: Verify clippy across the workspace**

Run: `cargo clippy --workspace -- -D warnings`
Expected: clean. (If `tempfile` or `lopdf` dev-usage triggers an unused-import warning, resolve it before committing.)

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-adapters/src/pdf/doc.rs crates/kasane-cli/src/main.rs crates/kasane-adapters/Cargo.toml Cargo.lock
git commit -m "feat(pdf): empty-password decrypt tests + encrypted exit code 2"
```

---

## Task 10: End-to-end pipeline test and documentation

**Files:**
- Modify: `crates/kasane-adapters/src/lib.rs` (end-to-end test)
- Modify: `README.md`, `AGENTS.md`
- Modify: `crates/kasane-cli/src/main.rs` (help text)

**Interfaces:**
- Consumes: `detect`, `PdfAdapter`, `kasane_core::{structure, Options}`, `kasane_writer::write_tree`.

- [ ] **Step 1: Write the failing end-to-end test**

Add to the `tests` module in `crates/kasane-adapters/src/lib.rs` (mirrors `end_to_end_pptx_fixture_to_sitetree`):

```rust
    #[test]
    fn end_to_end_pdf_fixture_to_sitetree() {
        let bytes = std::fs::read("../../tests/fixtures/pdf/image.pdf").unwrap();
        assert!(matches!(detect(&bytes, Some("pdf")), Some(Format::Pdf)));

        let (doc, assets) = PdfAdapter.parse(&bytes, "image.pdf").unwrap();
        assert_eq!(doc.meta.source_format, "pdf");

        let site = kasane_core::structure(doc, &kasane_core::Options::default());
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("pdfout");
        kasane_writer::write_tree(&site, &assets, &out, false).unwrap();
        assert!(out.join("index.md").exists());
        // The FlateDecode image was flushed to _assets/.
        assert!(out.join("_assets").read_dir().unwrap().next().is_some());
    }
```

- [ ] **Step 2: Run it to verify it passes**

Run: `cargo test -p kasane-adapters end_to_end_pdf_fixture_to_sitetree`
Expected: PASS — the full pipeline runs and writes `index.md` plus a flushed asset. (This is green immediately because Tasks 1–9 built the machinery; the test locks the end-to-end contract.)

- [ ] **Step 3: Update the CLI help text**

In `crates/kasane-cli/src/main.rs`, change the input doc comment:

```rust
    /// Input document (EPUB, PPTX, MOBI, AZW3, PDF supported in this build)
    input: PathBuf,
```

- [ ] **Step 4: Update README**

In `README.md`: change the opening line from "(EPUB, PPTX, MOBI, AZW3 today; PDF and DJVU coming)" to "(EPUB, PPTX, MOBI, AZW3, PDF today; DJVU coming)". Add to "Known limitations (this build)":

```markdown
- PDF conversion is for born-digital PDFs. Headings come from the PDF outline
  (bookmarks) at page granularity, or from font-size inference when there is no
  outline. Multi-column layout is read as a single column; tables become
  paragraphs; PDF has no math markup to recover.
- Scanned/image-only PDF pages are emitted as the page image plus a placeholder
  note; text is not recovered until the OCR feature (`-F ocr`) lands. Bilevel
  scans compressed with CCITT/JBIG2 are noted but not extracted.
- Password-protected PDFs: the common permissions-only case (empty user
  password) is converted transparently; a real user password is rejected
  (exit code 2). DRM is never broken.
```

- [ ] **Step 5: Update AGENTS.md codebase map**

In `AGENTS.md`, extend the `crates/kasane-adapters` bullet to mention PDF, e.g. append:

```
The PDF adapter (`pdf/`) builds on `lopdf`: `content.rs` interprets content-stream text operators into positioned runs, `layout.rs` groups them into lines/paragraphs and infers headings by font size, `outline.rs` maps the `/Outlines` TOC to per-page headings, `image.rs` extracts embedded images; fixtures are hand-built by `tests/fixtures/pdf/make_pdf_fixtures.py`.
```

- [ ] **Step 6: Full verification**

Run: `mise run lint && mise run test`
Expected: rustfmt check clean, `clippy -D warnings` clean, all tests pass across the workspace.

- [ ] **Step 7: Manual smoke test (optional but recommended)**

Run: `mise run convert tests/fixtures/pdf/minimal.pdf -o /tmp/kasane-pdf-smoke && cat /tmp/kasane-pdf-smoke/index.md`
Expected: an `index.md` linking to sections titled "Chapter One" and "Section Two".

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-adapters/src/lib.rs crates/kasane-cli/src/main.rs README.md AGENTS.md
git commit -m "test(pdf): end-to-end pipeline + docs for PDF support"
```

---

## Self-Review (completed during planning)

**Spec coverage:**
- Structure ambition (outline + line grouping): Tasks 5, 6, 7. ✅
- lopdf + own text pass: Tasks 3, 4. ✅
- Empty-password decrypt / real → Encrypted / exit 2: Tasks 3, 9. ✅
- Single-column, page provenance: Tasks 5, 7. ✅
- Images (Flate→PNG, DCT), scanned note: Task 8. ✅
- Security (bomb guards, degrade-don't-die): `MAX_CONTENT_BYTES`/`decompressed_content_with_limit` (Tasks 3, 4, 8); empty/error pages → `Raw` (Tasks 7, 8). ✅
- Wiring (`adapter_for`, lib, CLI, README, AGENTS): Tasks 1, 9, 10. ✅
- Hermetic fixtures: Task 2. ✅

**Deviations flagged:** heading page-granularity, image-filter coverage, text decoding via lopdf — documented in the "Deviations" section above and the README limitations.

**Type consistency:** `TextRun {x,y,size,text}` (Task 4) is consumed by `group_lines` (Task 5); `Line {y,x,size,text}` (Task 5) is consumed by `page_blocks_no_headings` (Tasks 5, 7); `OutlineHeading {level,title}` (Task 6) is consumed in `mod.rs` (Task 7); `PageImages {figures,had_image,skipped}` (Task 8) is consumed in `mod.rs` (Task 8). `doc::{open,pages,MAX_CONTENT_BYTES}` names are stable across Tasks 3–8. `exit_code_for` (Task 9) is reused unchanged in Task 10. ✅

**Recursion/bomb note:** lopdf owns page-tree/outline traversal and enforces the decompression limits we pass (`MAX_CONTENT_BYTES`, `decompressed_content_with_limit`), so no hand-rolled depth counter is needed; our own code adds no unbounded recursion.
