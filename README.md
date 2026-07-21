# kasane

Convert documents and ebooks (EPUB, PPTX, MOBI, AZW3, PDF today; DJVU
coming) into an AI-agent-friendly, progressively-disclosed Markdown file tree.

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
