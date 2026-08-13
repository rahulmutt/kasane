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

Paths also changed with the slug rule. Punctuation is now removed rather than
turned into a separator, so `01-don-t-panic.md` became `01-dont-panic.md`, and
a heading in any script now keeps its text, so a Japanese book that used to
emit `01-section.md` and `02-section.md` now emits `01-第二章.md` and its
siblings. Filenames are therefore no longer ASCII, which matters if you pipe
the tree through tooling that assumes they are. Heading anchors changed with
the same rule and for the same reason — `#don-t-panic` became `#dont-panic` —
so a deep link into a previously generated tree needs updating too, not just a
path. As with the `Part N` change, `write_tree` replaces the output
directory wholesale so nothing stale is left behind — but anything outside the
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
which is now a deliberate mirror of GitHub's — the generator draws non-Latin
and punctuation-bearing titles, so the invariant exercises that rule rather
than an ASCII subset of it. What it still cannot say is whether github.com
computes the same anchor: nothing in CI can ask it. The mirror is written down
as a case table in `crates/kasane-core/src/slug.rs`. Deep list/footnote
nesting remains under Known limitations.

When a property fails it writes `crates/kasane-writer/tests/properties.proptest-regressions`.
**Commit that file** — like a fuzz reproducer, it is what replays the failing
case on every subsequent run.

### Fuzzing

Every adapter parses untrusted input, so the boundary is fuzzed with
`cargo-fuzz`. Thirteen targets cover the five format adapters, format detection,
the two ZIP container formats past their CRC check, and the sub-parsers a
whole-file fuzzer would rarely reach — the math island capture, PalmDOC
decompression, the path guards, XML entity resolution, and the slug rule that
turns untrusted title text into a filename.

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

- Heading anchors match GitHub's rule, with three exceptions. Anchors are
  computed the way GitHub computes them — Unicode-aware, punctuation removed
  rather than replaced, `_` kept, duplicates within a file suffixed `-1`,
  `-2` — so `## Don't Panic` anchors as `#dont-panic` and `## 第二章` as
  `#第二章`, both of which resolve on GitHub. The rule was checked against a
  real GitHub render on 2026-08-09 and every case matched; it is still a
  mirror, so it can drift if GitHub's changes. Note that exact parity means
  some anchors look wrong and are not. `## Background & Notes` anchors as
  `#background--notes`, because GFM removes the `&` and turns each surviving
  space into a hyphen. And the character set is Ruby's `\p{Word}`, the set
  GitHub's own filter keeps: alphabetic characters, combining marks, decimal
  digits, `_`, and the zero-width joiner and non-joiner. "Alphabetic" is
  Unicode's property of that name, so it is wider than "a letter" — Roman
  numerals like `Ⅷ` and circled letters like `Ⓐ` are in — and narrower than
  "looks like one": parenthesized letters like `⒜` are out, as are all
  non-decimal numerals, so `## Fig ½` anchors as `#fig-` and `## ①はじめに`
  as `#はじめに`. The three exceptions:
  - A heading with none of those characters at all (`## ***`, `## —`, `## ½`)
    gets an empty id from GitHub; kasane emits `#section`, because an empty
    anchor is a dead link. A heading that is *only* a zero-width non-joiner is
    not this case — it anchors to that invisible character, exactly as GitHub
    does.
  - A heading ending in a footnote reference, `## Notes[^1]`, anchors
    `#notes` here and `#notes1` on GitHub: kasane slugs the heading's text
    without the rendered `[^1]` marker.
  - A heading whose title ends in a run of `#`, rendered `## Intro ###`,
    anchors `#intro-` here and `#intro` on GitHub, which reads that run as an
    ATX closing sequence.

  Filenames carry the title in any script, capped at 64 bytes of title per
  component; they drop the zero-width joiners an anchor keeps, since a
  filename should not contain invisible characters.
  What they do not carry is total path length: depth comes from heading
  nesting plus whatever `-o` you pass, so a deep book in a deep output
  directory can still exceed Windows' 260-character default path limit.
- Text that looks like Markdown is preserved as text, not as markup. A book
  that literally prints `*`, `|`, `` ` ``, `[`, `&` or a line beginning with
  `#` converts to a file where those characters render as themselves, which
  means the Markdown source contains backslash escapes — `a\*b`, `1\. two`,
  `a\|b` inside a table cell.
  Leading whitespace on a line is carried by a character reference — `&#32;`
  or `&#9;` — rather than a backslash, because a backslash would suppress the
  construct by losing the whitespace. Table cells now keep the whitespace at
  both their edges, which GFM would otherwise trim away: text that earlier
  builds dropped silently now survives.
  That is deliberate: the source document is content, not syntax. Two
  consequences worth knowing. A newline inside a
  heading, a table cell, a link label or a frontmatter title is folded — to a
  space, or to `<br>` in a cell — because those places are a single line by
  grammar. And a merged-cell table, which is emitted as raw HTML, carries its
  emphasis as `<strong>`/`<em>` tags and its equations as literal LaTeX, since
  GitHub parses no Markdown inside an HTML block.
- Block nesting is bounded, and deep nesting flattens rather than failing.
  Lists and footnotes nested past the EPUB parser's fidelity bound stop
  producing further nesting: an over-deep list's items become siblings at the
  bound's level and an over-deep footnote container becomes transparent. Every
  text run survives — only the nesting relationship past the bound is lost.
  A ~540 KB EPUB holding a 30,000-deep `<ul>` converts normally, where older
  builds aborted with `fatal runtime error: stack overflow` (SIGABRT, shell
  exit 134). The bound applies to EPUB and MOBI/AZW3 alike, since MOBI
  re-serializes through the same parser. A second, higher bound in
  `kasane-ir` protects the structuring engine and writer from hand-built
  `Document`s passed to `structure()` by external callers; past it a
  truncation note is emitted in place of the over-deep subtree.
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
