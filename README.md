# kasane

Convert documents and ebooks (EPUB, PPTX, MOBI, AZW3, PDF, DjVu today) into an
AI-agent-friendly, progressively-disclosed Markdown file tree.

## Quick start
    mise install
    mise run build
    mise run convert book.epub -o out/book
    # open out/book/index.md and drill into linked sections

## Install
    cargo install kasane-cli   # installs the `kasane` binary

## Development
    mise run test    # run all tests
    mise run lint    # fmt check + clippy -D warnings

### OCR (optional)

OCR is off by default and is the only feature that links a C library. Build
with the `ocr` feature (needs Tesseract + Leptonica installed, plus the
language's `traineddata`):

    cargo build -F ocr
    kasane scan.pdf -o out/scan --ocr --ocr-lang eng
    #   --ocr-lang <LANG>   language(s) to use, e.g. "eng+deu" (default: eng)
    #   --ocr-no-image      emit OCR text even at low confidence, never a page image

On a build without `-F ocr`, passing `--ocr` fails fast with a clear error
(exit code 2) instead of silently ignoring the flag. A missing `traineddata`
pack for the requested language also fails fast.

See AGENTS.md for the codebase map.

## Known limitations (this build)

- DRM-protected MOBI/AZW3 files are detected and rejected (exit code 2);
  kasane never breaks DRM.
- MathML (EPUB) and OMML (PPTX) math are not yet converted to LaTeX.
- HUFF/CDIC-compressed MOBI books decode through the `mobi` crate; their
  in-book `filepos` links may resolve approximately.
- PDF conversion is for born-digital PDFs. Headings come from the PDF outline
  (bookmarks) at page granularity, or from font-size inference when there is no
  outline. Multi-column layout is read as a single column; tables become
  paragraphs; PDF has no math markup to recover.
- Scanned/image-only PDF pages: with an `-F ocr` build and `--ocr`, text is
  recovered by OCR (text-first; the page image is kept as a fallback when OCR is
  not confident). OCR runs only on pages whose image kasane already decodes
  (JPEG/Flate). Bilevel scans compressed with CCITT/JBIG2 (and JPEG2000) are not
  decoded, so they are noted but not OCR'd. Without `--ocr`, scanned pages emit
  the page image plus a placeholder note, as before.
- Password-protected PDFs: the common permissions-only case (empty user
  password) is converted transparently; a real user password is rejected
  (exit code 2). DRM is never broken.
- DjVu conversion recovers the file's hidden OCR text layer, structured by the
  document's own zone hierarchy (page/column/region/paragraph/line), so
  multi-column reading order is preserved without geometric re-sorting. Text
  fidelity is only as good as the file's embedded OCR text layer — kasane does
  not run its own OCR on DjVu pages.
- Headings come from the document's NAVM outline (bookmarks) at page
  granularity when one is present; with no outline, headings are inferred
  document-wide from line height instead. When an outline exists, its title is
  spliced in as the heading *and* the matching text-layer line still appears in
  the body text below it, so a chapter title can appear twice in the output —
  a known cosmetic limitation shared with the PDF adapter's outline handling.
- Text-less pages now emit the rendered page image: the bilevel JB2 mask as a
  compact 1-bit PNG, or a full IW44 render (RGB PNG) when the page has no mask.
  A rendered page carries a marker that its text is un-OCR'd — "page image only;
  no text layer, OCR not enabled" when there was no text layer, or "page image
  only; text layer present but empty" when the layer decoded to nothing. If a
  page fails to render, the bare placeholder note is emitted instead. Pages that
  recovered text get no image. This describes the default, no-`--ocr` build.
  With an `-F ocr` build and `--ocr`, kasane OCRs these text-less pages itself:
  recovered text replaces the image when OCR is confident, otherwise the page
  image is kept with a note. Reading order and inferred headings come from the
  OCR line boxes, matching text-layer pages.
- Only bundled (single-file) DjVu documents are supported; indirect
  (multi-file) documents are rejected with a clear message (exit code 1, not
  2 — this is a format-support gap, not DRM). Tables become paragraphs; DjVu
  has no math markup to recover.
