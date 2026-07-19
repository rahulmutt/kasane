# kasane

Convert documents and ebooks (EPUB today; PDF, DJVU, MOBI, AZW3, PPTX coming)
into an AI-agent-friendly, progressively-disclosed Markdown file tree.

## Quick start
    mise install
    just build
    just run book.epub -o out/book
    # open out/book/index.md and drill into linked sections

## Development
    just test    # run all tests
    just lint    # fmt check + clippy -D warnings

See AGENTS.md for the codebase map.
