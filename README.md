# kasane

Convert documents and ebooks (EPUB, PPTX, MOBI, AZW3, PDF, DjVu today) into an
AI-agent-friendly, progressively-disclosed Markdown file tree.

## Quick start
    mise install
    mise run build
    mise run convert book.epub -o out/book
    # open out/book/index.md and drill into linked sections

### Batch conversion

    kasane books/ -o out/            # every document under books/, recursively
    kasane a.epub b.pdf -o out/      # several files at once
    kasane books/ -o out/ -j 4       # 4 workers (default: all cores)

Each document lands at its path relative to the root it was found under, so
`books/a/ch.epub` becomes `out/a/ch/index.md`. `out/index.md` is a library
index linking every document and naming every failure; its links are
percent-encoded, so `War and Peace/` becomes `War%20and%20Peace/index.md`. A
single file argument is unchanged: `-o` is that document's own root.

Directories are walked recursively and filtered by extension. A symlink named
directly on the command line is followed; a symlink encountered while walking
is skipped, so a linked directory can't introduce a cycle or let the walk
escape its root. `-o` is required whenever more than one document could be
produced. One file's failure never aborts the run.

| Exit code | Meaning |
|---|---|
| 0 | every document converted |
| 1 | nothing converted, or a usage problem (missing `-o`, duplicate destinations, no documents found) |
| 2 | every failure was an unsupported format, DRM, or encryption |
| 3 | some documents converted, some failed |

## Install
    cargo install kasane-cli   # installs the `kasane` binary

## Development
    mise run test    # run all tests
    mise run lint    # fmt check + clippy -D warnings

### Fuzzing

Every adapter parses untrusted input, so the boundary is fuzzed with
`cargo-fuzz`. Twelve targets cover the five format adapters, format detection,
the two ZIP container formats past their CRC check, and the sub-parsers a
whole-file fuzzer would rarely reach — the math island capture, PalmDOC
decompression, the path guards, and XML entity resolution.

    mise run fuzz math_island                    # one target, until you stop it
    mise run fuzz epub -- -max_total_time=60     # one target, 60 seconds
    mise run fuzz-all                            # every target, 5 minutes each

Targets build on a pinned nightly (libFuzzer needs it); everything else in this
repo stays on the pinned stable toolchain. CI runs the full set weekly.

Beyond panics and hangs, the targets assert that the decompression-bomb guards
hold under libFuzzer's RSS limit, that no asset filename escapes `_assets/`, and
that `safe_entry_name` / `resolve_rel` never emit a traversal.

**When the fuzzer finds a crash, commit the reproducer** from
`fuzz/artifacts/<target>/`. If the underlying bug gets fixed right away, `cargo
test` replays that reproducer on the stable toolchain from then on, so the fix
stays fixed. If the bug is left open instead, the reproducer is still
committed but also listed in `KNOWN_OPEN` in `tests/fuzz_corpus.rs`, which
skips it during the stable replay so the suite stays green; removing it from
that list is what re-arms the regression test once the bug is fixed.

Two findings are open this way today: a stack overflow in the `pdf` adapter,
and a path-confinement leak in `guards` (`resolve_rel` normalizes `..` in its
`target` argument but not in `base_dir`). Their reproducers live under
`fuzz/artifacts/{pdf,guards}/`. The quarantine above only protects the stable
`cargo test` run — `mise run fuzz`/`mise run fuzz-all` still reproduce both
crashes, so expect those two targets to fail immediately, and expect the
weekly fuzz CI run to be red on them, until they're fixed.

The `ocr` feature is not fuzzed — it links C (Tesseract, Leptonica), which needs
its own sanitizer setup.

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
- Math is recovered as LaTeX: MathML (EPUB) and OMML (PPTX) equations convert to
  GitHub-Flavored `$…$` (inline) / `$$…$$` (display) over a documented construct
  subset — fractions, sub/superscripts, roots, n-ary operators
  with limits, basic matrices, fenced groups (OMML `<m:d>`, including its
  multi-argument and `sepChr` forms; on the MathML side only the deprecated
  `<mfenced>` — an `<mrow><mo>(</mo>…<mo>)</mo></mrow>` fence converts as
  ordinary operators, not as a `\left…\right` group), and (MathML only) common
  accents. A construct outside the
  subset degrades best-effort: the unmapped part becomes `\mathord{?}` and a
  partial display equation is followed by an "equation partially converted"
  note; a partial equation appearing inline (including a display equation that
  folds to an inline context such as a table cell) is marked by the token alone.
  Content MathML is not converted. An equation whose markup is malformed,
  oversized or nested past the parser's bound degrades to the placeholder and is
  accompanied by a note naming the reason; the markup it spanned is re-read as
  ordinary content rather than being swallowed.
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
- Batch mode holds one document in memory per worker, so a directory of large
  PDFs at a high `-j` can use a lot of RAM; `-j 1` is the mitigation. Two
  inputs whose output directories would clash — either identical, or one
  nested inside the other, as when `ch.epub` sits beside a `ch/` directory —
  are rejected before any conversion starts rather than silently overwriting
  each other.
