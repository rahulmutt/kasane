# kasane

Convert documents and ebooks (EPUB, PPTX, MOBI, AZW3 today; PDF and DJVU
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
