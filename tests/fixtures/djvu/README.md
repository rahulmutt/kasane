# DjVu test fixtures

## `sample.djvu`

A bundled (`FORM:DJVM`), single-page DjVu document used by the DjVu adapter's
detection, document, and end-to-end tests. It contains:

- a 64x64 bilevel page (a `FORM:DJVU` component);
- a text layer (`TXTz`) with one `Page` zone wrapping three `Line` zones, in
  reading order — `"Chapter One"` (a taller heading line), `"First body line."`,
  and `"Second body line."` (two shorter body lines); and
- one NAVM outline bookmark, title `"Chapter One"`, targeting page 1 (`"#1"`).

The file begins with the `AT&T` DjVu preamble that `detect()` keys on.

## Regeneration

This fixture is generated **in pure Rust with the `djvu-rs` crate** (a
dependency of `kasane-adapters`), by the committed example generator. Regenerate
it from the workspace root with:

```sh
cargo run -p kasane-adapters --example make_djvu_fixture
```

Source: `crates/kasane-adapters/examples/make_djvu_fixture.rs`. The generator
also asserts a full round-trip (re-parses the bytes it wrote and checks the page
count, the three text-layer lines, and the bookmark) before writing the file.

### Why not DjVuLibre?

The original task brief produced this fixture out-of-band with DjVuLibre
(`cjb2` + `djvused set-txt/set-outline`). DjVuLibre is **not available** in this
environment (not installed, not installable, network blocked), so the fixture is
instead authored with `djvu-rs` itself. Because the *same* crate both writes and
reads the file, round-trip compatibility with the parser kasane actually uses is
guaranteed, and the fixture stays consistent with the repo's pure-Rust,
hermetic-generator convention (mirroring `tests/fixtures/pdf/make_pdf_fixtures.py`).
