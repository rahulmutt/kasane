# kasane

Convert documents and ebooks (EPUB today; PDF, DJVU, MOBI, AZW3, PPTX coming)
into an AI-agent-friendly, progressively-disclosed Markdown file tree.

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

- Internal in-document EPUB links (`#frag` and `file.xhtml#frag`) are passed
  through unresolved as external references, not yet turned into working
  cross-file links. Resolving them to real block targets is deferred to
  Plan 2's XHTML-fidelity task.
- EPUB tables, math, figures, footnotes, and lists are not yet parsed
  (Plan 2).
