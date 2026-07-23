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
- Scanned/image-only PDF pages are emitted as the page image plus a placeholder
  note; text is not recovered until the OCR feature (`-F ocr`) lands. Bilevel
  scans compressed with CCITT/JBIG2 are noted but not extracted.
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
- Scanned page images (JB2/IW44) are not rendered in this build. A page with no
  text layer at all becomes a placeholder note ("no text layer; OCR not
  enabled"); a page whose text layer is present but decodes to nothing gets a
  different note ("text layer present but empty; no recoverable text").
- Only bundled (single-file) DjVu documents are supported; indirect
  (multi-file) documents are rejected with a clear message (exit code 1, not
  2 — this is a format-support gap, not DRM). Tables become paragraphs; DjVu
  has no math markup to recover.
