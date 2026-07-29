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

### Output shape

Every emitted file opens, after its YAML frontmatter, with its own title as a
Markdown heading. That heading is what an in-book cross-reference
(`some/file.md#slug`) lands on; before, such links pointed at an anchor no file
contained. Which slug they use — and where that diverges from GitHub's — is
under Known limitations.

A section's file holds the blocks between its heading and its first subheading.
When that run is over the token budget it is split into synthetic `Part N`
files — and that now applies to a *container* file such as `index.md` too, not
only to leaves. A book with a long preface therefore emits
`01-part-1.md`, `02-part-2.md`, … for material that older builds left inside
`index.md`, and the real chapters shift down the numbering with it. Re-running
kasane over an input converted by an earlier build can therefore change which
paths exist, not just what is in them. `write_tree` replaces the output
directory wholesale, so nothing stale is left behind — but anything outside the
tree that referenced the old paths needs updating.

## Install
    cargo install kasane-cli   # installs the `kasane` binary

## Development
    mise run test    # run all tests
    mise run lint    # fmt check + clippy -D warnings

### Property tests

`kasane-core`'s structuring engine is checked with `proptest`: generated
documents run through `structure()` and the Markdown writer, and six invariants
are asserted against the rendered text — every block appears exactly once, every
internal link resolves to a real file and a real anchor, the size guard holds,
`prev`/`next` forms a complete chain, no path escapes the tree, and rendering is
deterministic. They run in `mise run test` with no extra setup.

Read the link invariant precisely: it holds *against kasane's own slug rule*,
and the generator draws adapter-realistic ASCII titles and shallow block
nesting. It therefore says nothing about whether GitHub resolves the same
anchor, nor about non-Latin headings or deep list/footnote nesting — both are
under Known limitations below.

When a property fails it writes `crates/kasane-writer/tests/properties.proptest-regressions`.
**Commit that file** — like a fuzz reproducer, it is what replays the failing
case on every subsequent run.

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
that `resolve_rel` never emits a traversal.

**When the fuzzer finds a crash, commit the reproducer** from
`fuzz/artifacts/<target>/`. If the underlying bug gets fixed right away, `cargo
test` replays that reproducer on the stable toolchain from then on, so the fix
stays fixed. If the bug is left open instead, the reproducer is still
committed but also listed in `KNOWN_OPEN` in `tests/fuzz_corpus.rs`, which
skips it during the stable replay so the suite stays green; removing it from
that list is what re-arms the regression test once the bug is fixed.

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

- Heading anchors use kasane's own slug rule, not GitHub's. It keeps ASCII
  letters and digits only (`slug()` in `crates/kasane-core/src/paths.rs`) and
  turns every other run of characters into a single `-`. Two consequences:
  punctuation diverges from GFM — `## Don't Panic` is anchored `#don-t-panic`
  where GitHub computes `#dont-panic` — and a heading with no ASCII
  alphanumerics at all, i.e. any purely non-Latin script, collapses to the
  literal `section`. A two-chapter Japanese book emits `01-section.md` and
  `02-section.md` whose in-book links point at `#section` while the rendered
  headings anchor as `#第二章` in GitHub, so those links do not resolve there.
  Filenames lose the title the same way. Widening the slug is open work.
- Block nesting has no depth bound. Inline nesting does (see AGENTS.md), but
  lists and footnotes are still walked, cloned and rendered recursively, so a
  document that nests them deeply enough aborts the process with `fatal runtime
  error: stack overflow` (SIGABRT, shell exit 134) instead of failing with an
  error. Reproduced with a ~540 KB EPUB holding a 30,000-deep `<ul>`, stored
  1:1 — no decompression bomb is involved, so none of the bomb guards apply.
  Bounding it is open work.
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
  paragraphs; PDF has no math markup to recover. Bookmarks are ignored
  entirely — headings fall back to font-size inference — when either the
  outline itself or the document's internal destination-name table, which the
  bookmark lookup also reads, is cyclic, implausibly large, or malformed in a
  way the underlying PDF library would crash on. Either one is enough on its
  own: a document with pristine bookmarks and a damaged destination table
  falls back too.
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
