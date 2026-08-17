# Codebase map

Pipeline: input file -> detect -> adapter -> IR -> structure() -> write_tree -> Markdown tree.

- `crates/kasane-ir`      Intermediate representation types. Depends on nothing.
- `crates/kasane-gfm`     Leaf crate depending on `kasane-ir` alone: what GFM does to a
  heading's text, and the two slug rules that follow from it (design spec
  `2026-08-14-shared-gfm-text-model-design.md` §1-§2). Owns `fold_newlines` — the
  newline-run fold `kasane-core` and `kasane-writer` used to keep in step by
  hand as two separate functions in two crates that could not depend on each
  other — the two heading-text projections (`title_text`, which keeps
  `nav::walk`'s existing skip of `Inline::FootnoteRef`, feeding `nav`/`refs`/
  `balance`/a file's title; and `rendered_text`, the same walk except a
  `FootnoteRef(n)` contributes `[^n]`, matching what the writer actually
  prints), and both slug rules, `anchor_slug`/`path_slug`, over the shared
  `is_word` character class. `kasane-core` depends on it for `paths`/`nav`/
  `refs`/`balance` and does not re-export the slug seams; `kasane-writer`
  depends on it directly for the fold. Of the five anchor divergences
  `slug.rs`'s module doc has recorded as surviving, four are now closed — a
  footnote reference's digits via `rendered_text`, a trailing `#` run via
  `kasane-writer::escape::atx_closing` escaping it before GitHub ever sees it,
  a heading containing a **single** empty inline code span via
  `section::clone_inlines_at` canonicalizing `Inline::Code("")` to a single
  space before any anchor is computed, and a heading containing **two or more
  adjacent** empty code spans via `kasane-writer` rendering a run of adjacent
  same-delimiter inlines as one span, so the pair prints one span over both
  padding spaces and the line ids what the anchor says. One survives. It is
  the empty-id fallback (`EMPTY_FALLBACK`), a deliberate choice rather than a
  construction defect.
  The canonicalization invariant is established by
  `fold_sections` and nowhere else: `SectionTree`/`SectionNode` have all-`pub`
  fields and `balance`/`assign_paths` are exported, so a hand-assembled tree
  can still anchor un-canonicalized inlines. Nothing in this repo does.
- `crates/kasane-adapters` Format detection + parsers (EPUB, PPTX, MOBI/AZW3, PDF, DjVu). Untrusted-input boundary; see `guard.rs` and `ziputil.rs` (every guarded zip read goes through it). The MOBI/AZW3 adapter (`mobi/`) normalizes HTML via html5ever and reuses the EPUB XHTML parser; fixtures are hand-built by `tests/fixtures/{mobi,azw3}/make_*.py`. The PDF adapter (`pdf/`) builds on `lopdf`: `content.rs` interprets content-stream text operators into positioned runs, `layout.rs` groups them into lines/paragraphs and infers headings by font size, `outline.rs` maps the `/Outlines` TOC to per-page headings behind a pre-flight over both graphs `get_toc` reads — the outline graph and the destination name tree (`catalog/Dests`, or `catalog/Names` then `Dests`). `outline_is_traversable` and `named_destinations_are_traversable` share one bounded-walk helper, `is_bounded_and_acyclic` (visited-set on `ObjectId`, plus depth and node caps), and each supplies its own per-node contents check. Topology is bounded because `lopdf`'s own walk follows the outline's `/First` recursively, `/Next` iteratively, and the destination tree's `/Kids`, all unbounded, so a cyclic or oversized graph would abort the process or hang. Contents are checked because `lopdf` then indexes and `unwrap`s those nodes unvalidated: an outline destination array shorter than two elements (`outlines.rs:100-101`), a name-tree entry with no `/D`, a short `/D`, or a non-string key (`destinations.rs:56-66`) each panic. Both checks mirror `lopdf` exactly, down to `/A`'s `/D` shadowing `/Dest` and the error paths `lopdf` takes before it indexes anything — validating more than `lopdf` touches would drop legitimate outlines. A rejected graph degrades to font-size inference; `outline_dup::title_line_mask` (crate root, shared with the DjVu adapter) then drops the page lines that merely reprint a spliced outline title, since an outline also suppresses size inference and the printed title would otherwise land in the body under its own heading — the line is a paragraph of its own or fused into the next depending on the page's leading, which is why the filter runs on lines rather than blocks. `has_text` is therefore read off the *unfiltered* lines: a chapter-opener page whose only line is its own title must not become a scanned page once that line goes. `get_toc` itself is additionally wrapped in `catch_unwind` as production-only defence in depth (it cannot help the fuzzer, whose panic hook aborts before unwinding), `image.rs` extracts embedded images; fixtures are hand-built by `tests/fixtures/pdf/make_pdf_fixtures.py`. The DjVu adapter (`djvu/`) builds on `djvu-rs`: `doc.rs` is the sole seam over the crate (container, hidden text layer, NAVM outline) with panic/bomb guards; `text.rs` turns the text-layer zone hierarchy into reading-order lines (multi-column safe, since it follows the zone tree's own order) and infers headings by line height when no outline is present; `outline.rs` maps NAVM bookmarks to per-page headings, deduplicated against the page's own text by the shared `outline_dup` filter described under the PDF adapter; DjVu's half of that filter also forwards a dropped line's `para_start` to the next kept line, or the text after the title merges into the paragraph before it. `image.rs` renders text-less pages to a page image — the JB2 mask as a 1-bit PNG, falling back to a full IW44 render — bounded by a decoded-pixel budget in `doc.rs` (`MAX_RENDER_PIXELS`); text-bearing pages remain text-only. The committed fixtures `tests/fixtures/djvu/{sample,scanned}.djvu` are generated by a committed pure-Rust generator, `cargo run -p kasane-adapters --example make_djvu_fixture` (not DjVuLibre, which is unavailable in this environment); see `tests/fixtures/djvu/README.md`. The OCR seam (`ocr/`) sits behind the PDF and DjVu adapters: the `TextExtractor` trait and its data types (`OcrLine`, `OcrOptions`) compile on every build, while `TesseractExtractor` (`tesseract.rs`) is gated behind the opt-in `-F ocr` feature and is the sole C dependency (Tesseract + Leptonica). `Adapter::parse` is the no-OCR convenience; `parse_with(.., &ParseOptions)` carries the optional extractor. Text-less pages are OCR'd text-first with the page image as a fallback; PDF OCR covers only decodable-raster pages (CCITT/JBIG2/JPX are not decoded). Build/lint the feature with `mise run test-ocr` / `mise run lint-ocr`. The math seam (`math/`) converts MathML (EPUB) and OMML (PPTX) equations to LaTeX behind two front-ends (`mathml.rs`, `omml.rs`) over a shared `MathNode` model (`ast.rs`) and one emitter (`latex.rs` with symbol lookups delegated to `symbols.rs`); adapters isolate a math island from their streaming parse via `capture_island` and parse it with `roxmltree`. Islands are re-parsed under a synthetic root binding the MathML default namespace, the `mml:` prefix, and the OMML `m:` prefix, with front-ends matching by local name; both prefixed and default-namespaced islands thus work. Islands are untrusted: `capture_island` bounds captured bytes and raw element nesting *while streaming* (roxmltree recurses per element level and has no nesting guard, so an over-deep island would abort the process on a stack overflow) and returns `Result<String, CaptureError>`; on any abnormal outcome it rewinds the reader to just past the island's start tag, so the markup it spanned is re-read as ordinary content instead of vanishing, and both adapters pair the degraded equation with a `Block::Raw` note naming the reason. Every other guard degrades to `\mathord{?}` placeholders rather than panicking. `Inline::Math`/`Block::MathBlock` are the only IR touchpoints.
  `fuzz_entry.rs` is a test seam, not API: one `fn(&[u8])` per fuzz target,
  living inside this crate so it can reach `pub(crate)` internals
  (`math::capture_island`, `mobi::palmdoc::decompress`, `guard::*`,
  `xmltext::resolve_general_ref`) that the separate `fuzz/` workspace cannot.
  Each function either returns or panics — a
  panic is the finding. `tests/fuzz_corpus.rs` replays `fuzz/seeds/**` and
  `fuzz/artifacts/**` through those same functions on stable, so fuzz coverage
  reaches PR CI without a nightly toolchain. `kasane-gfm` has its own
  `fuzz_entry.rs` for the same reason, reaching `slug::path_slug` and
  `slug::anchor_slug`; the stable replay for both lives in
  `kasane-adapters/tests/fuzz_corpus.rs`, which takes both `kasane-core` (for
  this crate's own end-to-end pipeline test) and `kasane-gfm` (for the `slug`
  seam) as dev-dependencies, so one harness covers every target.
- `crates/kasane-core`    Pure structuring engine: fold -> balance -> paths -> refs -> nav. No I/O.
  `balance`'s SPLIT fires on any node whose own body is over `max_tokens`, not
  only on leaves: a container's body is the run of blocks between its heading
  and its first subheading (for the root, the whole preamble), and leaving that
  in the container's file at any size broke the size guard for `index.md`
  itself. The synthetic `Part N` sections are *prepended* to the existing
  children so pre-order flattening still reads in document order — which means
  they also take the leading path numbers (`01-part-1.md`, `02-part-2.md`, then
  `03-real-chapter.md`). This is a user-visible **path** change, not just a
  content one; README's "Output shape" is the user-facing statement of it.
  `est_tokens` is a `#[doc(hidden)] pub` test seam, not API — the same
  convention `kasane-adapters` uses for `fuzz_entry`, and for the same reason:
  the property tier needs the engine's own token estimate, and a copy in the
  test would drift.
  `slug.rs` owns two rules, not one. `anchor_slug` is a deliberate mirror of
  GitHub's heading-id algorithm (Unicode-lowercase, remove everything outside
  Ruby's `\p{Word}`, map spaces to hyphens, no collapsing and no interior
  trimming) so in-book cross-references resolve when the tree is rendered on
  GitHub; `path_slug` starts from the same character class but collapses
  separator runs, trims, and caps at 64 bytes, because a filename wants
  different things than a fragment. Ruby's `\p{Word}` is `Alphabetic + Mark +
  Decimal_Number + Connector_Punctuation + Join_Control`, which is narrower
  than "Letter, Mark, Number" in one direction (`Other_Number` — `½`, `①` —
  is outside it) and wider in two others (Join_Control, and everything
  `Alphabetic` carries beyond `L*`: `Nl` such as `Ⅷ`, and `Other_Alphabetic`
  such as the circled letter `Ⓐ`). `is_word` spells the alphabetic term as
  `char::is_alphabetic()`, which **is** Unicode's `Alphabetic` derived
  property rather than the `L*` category group — that is what makes the class
  exact rather than approximate, and `unicode-properties` is needed only for
  the Mark term and for telling `Nd` from `No`. The two rules therefore diverge on the character
  class too, not only in the tail: `anchor_slug` keeps ZWJ/ZWNJ because GitHub
  does and because they sit inside ordinary Persian, Urdu and Devanagari
  words, while `path_slug` drops them, since a filename must not carry
  invisible characters and `fuzz_entry::slug`'s confinement argument rests on
  the path alphabet staying closed. They also diverge on normalization, and
  that one is load-bearing: `path_slug` NFC-folds and **`anchor_slug` must
  not**, because `nav::walk` sets a file's title from unnormalized
  `title_text` and `file_to_markdown` writes it verbatim as the heading line,
  so an NFC fragment against an NFD heading is a link kasane breaks against
  its own output. The fold and both slug rules live in `kasane-gfm` (above);
  a heading's anchor is computed from the text its line actually prints —
  `rendered_text` for a body heading, the printed title for a file's title
  heading — and P9/P10/P11 in `kasane-writer/tests/properties.rs` are what
  check the writer against that claim, by parsing its own rendered output.
  Being a mirror, the anchor rule carries drift risk against
  github.com, and the case table in `slug.rs`'s tests is where that mirror is
  written down — one divergence still survives there on purpose: the empty-id fallback.
  `rendered_text`, `escape::atx_closing`, and `section::clone_inlines_at`'s
  empty-code-span canonicalization closed the other three the table used to
  record. That table pins kasane's *reading* of the algorithm, not the
  algorithm, so it cannot catch a misreading. The
  external check that can is recorded in design spec §8.1 (first run
  2026-08-09, 13/13 ids matching) and re-run at §8.3 on 2026-08-14 — 13 of 14
  ids identical, codepoints included, the empty-id fallback being the only
  divergence that run's probe cases hit (the probe carries no empty-code-span
  case); re-run it again when the table next changes. Duplicate anchors are suffixed per file
  in render order, which is why `place` counts headings nested inside list
  items even though it deliberately gives them no anchor: GitHub assigns them
  ids, so they consume a suffix slot. `assign_paths` takes the document title
  because `index.md` renders *that* as its heading, not the (empty) root node
  title. `path_slug_of` and `anchors_for_headings` are `#[doc(hidden)] pub`
  test seams, same convention as `est_tokens`; so is
  `section::canonicalize_inlines`, which exposes the engine's own inline
  canonicalization because P12 must anchor the very inlines the engine anchors
  and a copy of that rule in the test would drift.
- `crates/kasane-writer`  IR -> GitHub-Flavored Markdown; atomic tree writing. Also emits the batch library index (`library.rs`).
  `file_to_markdown` opens every file with its frontmatter title as a heading.
  `escape.rs`'s `Pos` is the writer's escaping-position vocabulary: `LineStart`,
  `AfterFootnoteRef`, `Mid`. It has three states rather than two because a
  `[^n]` that opened the line makes a following `:` a footnote *definition*
  delimiter, and the `:` belongs to the *next* inline — `markdown.rs` computes
  the position, `escape.rs` still owns every rule. Whitespace at `LineStart`
  becomes `&#32;`/`&#9;`, not a backslash: the backslash form suppresses the
  construct only by losing the whitespace, and it cannot reach the
  four-spaces-is-an-indented-code-block case at all.
  Every one-line context — `Block::Heading`, the title heading, `code_span`,
  `label`, `yaml_scalar` — folds newlines through `kasane_gfm::fold_newlines`,
  the one function both `kasane-core` and `kasane-writer` call, replacing what
  used to be two hand-kept copies of the same fold in two crates that could
  not depend on each other.
  `escape::atx_closing` is the other writer-side fix the shared model needed:
  a heading whose printed line ends in a run of `#` re-parses as an ATX
  heading with a *closing* sequence, so a real parser (and GitHub) reads less
  text than the IR holds. `Block::Heading` and `file_to_markdown` both apply it
  last, after escaping, so the run survives into the rendered line as `\###`
  and `kasane-gfm`'s anchor is computed from that same printed line rather
  than from IR text a parser would trim. It is a writer fix rather than an
  anchor-rule fix on purpose: teaching `anchor_slug` about closing sequences
  would buy parity by agreeing that the rendered heading may drop text the
  document had, conceding the escaping invariant (§5) rather than upholding it.
  `fold_inline_newlines` collapses a newline run spanning an inline boundary
  before the one-line contexts render, which is what keeps the rendered
  heading line matching `kasane-gfm`'s `anchor_fold` without widening what
  `fold_newlines` has to track. Its recursion's depth guard returns an
  empty `Vec` at the bound rather than cloning the remainder: `Inline`'s
  derived `Clone` is itself recursive, so cloning would only relabel the
  recursion the guard exists to stop — a hand-built tree 10,000 deep aborted
  the process on the 2 MiB stack rayon's batch workers use. Empty is safe
  because the only consumer, `inlines_to_md_at`, has its own
  `MAX_INLINE_DEPTH` guard and discards anything that deep before it is ever
  read.
  That is load-bearing, not cosmetic: `fold_sections` consumes a section's
  heading into `SectionNode.title` and never re-emits it, while `assign_paths`
  records the anchor as `path#anchor_slug(title)` — without the heading here, every
  in-book cross-reference pointed at an anchor no file contained. Its sibling
  half is `assign_paths`' scan of top-level body blocks, which anchors a
  heading `balance` demoted into its parent's body when it merged a tiny
  subsection; the two only work together.
  `tests/properties.rs` is design spec §9's property tier: it generates
  adapter-realistic `Document`s (`tests/generator/`), runs `structure()`, renders
  each file with `file_to_markdown`, and asserts six invariants against the
  resulting Markdown — conservation, link resolution, the size guard, the
  prev/next chain, path well-formedness, determinism. It reaches the writer
  rather than stopping at `kasane-core` because §9's link invariant is about a
  real file *and a real anchor*, which only rendered text can answer.
  `tests/census.rs` is a second, exhaustive tier alongside it rather than a
  replacement: it renders every sequence of length 1-3 over a small inline
  alphabet, parses the result, and checks the recovered text against
  `kasane_gfm::rendered_text` — the same equality a property samples, run over
  all of a chosen alphabet instead of generated cases. It is what found the
  emphasis-seam defects three property rounds missed, because a property draws
  from an alphabet someone chose and a census draws from all of it; see
  `census-known-corrupt.txt` below.
  `file_to_markdown` is what both the property suite and `write_tree_contents`
  render through, so what CI asserts is what a conversion writes.
  `escape.rs` is the only path from document text to an output buffer, and
  `Ctx` is a *required* argument on `inlines_to_md` rather than a defaulted
  one — that is the mechanism, not a convention: a new `Inline` arm or a new
  caller cannot inherit flow rules into a table cell by omission, because it
  does not compile until it names a context. `Inline::Text` is the only arm
  that calls `escape::text`; every other arm emits markup the writer chose,
  which must not be escaped. The governing invariant is that escaping never
  changes what the Markdown *renders* to, because `anchor_slug` computes
  fragments from unescaped IR text while GitHub computes ids from rendered
  text — which is also why `library.rs`'s former `link_text` (it replaced `[`
  with `(`) could not become the shared rule. Two destination encoders exist
  and differ on exactly one character: `dest_path` encodes `%` because a
  literal `%` in a filename would read back as an escape, and `dest_url` must
  not, because an `href` from a source document is already percent-encoded.
  The merged-table path emits HTML tags rather than Markdown markup, since GFM
  parses nothing inside an HTML block. `fuzz_entry.rs` is the `escape` fuzz
  seam, asserting postconditions (P7 in `tests/properties.rs` owns the round
  trip, because it can take `pulldown-cmark` as a dev-dependency and the
  library cannot).
  `Block::Raw` is the one documented exception to that invariant, not another
  case where the rendered text happens not to matter: an HTML comment admits
  no escape mechanism at all, unlike flow text, cells, code spans, HTML and
  YAML, each of which has one, so `comment_note` can only transform a note —
  breaking up a `--` run — rather than escape it. That is safe rather than a
  bug for two reasons together: there is no way to represent `-->` literally
  inside `<!-- -->` at all, and a comment's content is never rendered, so no
  reader ever sees the difference the transformation makes. It is load-bearing
  rather than precautionary because untrusted text really does reach it —
  `epub/xhtml.rs` and `epub/mod.rs` both build a note via
  `format!("image unavailable: {src}")` from an `<img src>` attribute the
  source document supplied.
  Delimiter runs that share a character never abut in the printed line, by four
  rules: a container at the edge of an emphasis run whose delimiter shares the
  run's own *character* is spliced into it; a container *anywhere* in a run
  whose `Delim` equals the run's own is spliced too, even where the nesting it
  replaces would sometimes have printed correctly; two adjacent runs spelled
  with the same character are fused into one run; and a delimiter that would
  fail to flank on either side where it lands is not emitted at all. CommonMark
  can express some of what these rules give up -- `[Emph(a), Strong(b)]` is
  expressible as two spans (`*a***b**` recovers `ab`) and a same-`Delim`
  container can nest safely when its own delimiters are one-sided-flanking (`*a
  *b* c*` keeps its inner `<em>`) -- but telling that safe spelling apart from
  one that corrupts (`*a*b*c*`) means reasoning about how a parser pairs
  delimiters, the mirror this repo has refused three times. So the writer trades
  the span boundary for the text -- which is the invariant -- uniformly rather
  than case by case, and `kasane-writer/tests/census.rs` is the exhaustive check
  that no such collision reaches the printed line regardless.
  `markdown.rs` decides all of this on a flattened view of the *printed*
  stream rather than on IR siblings, because the two are not the same list: an
  unresolved link prints only its children, so those children stand beside the
  link's own neighbours, and a fused run concatenates its members' children
  into one span, so the last child of one member stands beside the first child
  of the next. Scanning IR siblings alone left collisions open at both of
  those seams.
  Math is the one inline the writer escapes nothing inside: `Inline::Math`
  and `Block::MathBlock` are both pushed verbatim — the inline form between
  `$…$`, the block form between `$$…$$` — on the strength of a contract that
  lives in a different crate and is otherwise invisible from here. There is no
  escape to fall back on, which is why it is a contract rather than a rule:
  `\$` would corrupt adapter output that already spells a literal dollar that
  way, and neutralizing `\`/`{`/`}` would destroy the `\frac{1}{2}` an adapter
  legitimately emits. So `escape::math_span`/`math_block` carry a self-check
  instead of an escape: content that would close the delimiter (a `$`; any
  newline inline, since inline math can land in a table cell; a blank line in
  the block form) degrades to a code span or a fenced block, which cannot break
  out by construction. Adapter output never reaches that branch — it exists for
  a caller who builds `Inline::Math` by hand and calls `blocks_to_markdown`,
  the same reader `render_block`'s depth guard is written for.
  `kasane-adapters` neutralizes `$`, `{`, `}`, a backslash, and newlines in
  **every** node kind that carries document text, specifically because those
  are the characters that would corrupt the delimiters this writer generates —
  a stray `$` closes the span early, an unbalanced `{`/`}` breaks a `\text{}`
  group it opened. `math::latex::sanitize` (`math/latex.rs`) covers
  `<mn>`/`<mtext>` (`Number`/`Text`) and, via `latex::fence`, the untrusted
  `mfenced open=`/`<m:begChr>` delimiter attributes;
  `math::symbols::map_text` (`math/symbols.rs`) applies the identical set,
  character for character, to `<mi>`/`<mo>` and every OMML run
  (`Ident`/`Op`) — which is all of PowerPoint's equation text and the majority
  of MathML's, so a guarantee scoped to `<mn>`/`<mtext>` would leave the span
  open on almost every real equation. That completeness is what lets
  `omml::nary_op` hand `map_text` a raw operator character rather than a
  ready-made `\sum`: with no emitter-chosen LaTeX on that path, a backslash in
  it can be dropped. Changing the writer's math delimiter, or the adapter's
  neutralized set, changes half of a cross-crate contract without seeing the
  other half; this note and `latex.rs`'s own doc comment are the only two
  places it is written down.
- `crates/kasane-cli`     `kasane` binary; wires the pipeline; owns exit codes.
  `convert.rs` converts one document (`WorkItem` -> `Converted`) and returns a
  `Result` rather than exiting, which is what makes per-file failure isolation
  possible; `discover.rs` expands file/directory arguments into the work list
  (recursive walk, extension filter, output mapping, destination-collision
  check — equal *and* nested output directories are rejected up front, because
  `write_tree` swaps whole directories. Paths named on the command line are
  trusted; paths found by walking are not, so a walked symlink is skipped and
  an unreadable walked directory is a `warning:` on stderr rather than a fatal
  error); `batch.rs` fans out across rayon workers,
  preserving input order. Mode is keyed on the invocation shape: a lone
  argument that is not a directory is single-file mode (unchanged output
  layout), anything else is batch mode with a library index at
  `<out>/index.md`. Each worker builds its own `TesseractExtractor`, so
  nothing non-`Send` crosses a thread; `main` validates `--ocr-lang` once up
  front so a bad language fails the run, not every document.

## Workflows
- `mise run test` — all tests   - `mise run lint` — fmt + clippy   - `mise run convert <file> -o <dir>` — convert
- `mise run fuzz <target>` / `mise run fuzz-all` — fuzz the untrusted-input boundary (nightly; see README)
- In this sandbox, `mise run fuzz <target>` false-positives as a crash for
  every target, not just `slug`: LeakSanitizer's atexit leak scan needs
  `ptrace`, which this environment does not grant, so the run ends with an
  empty-content artifact that looks like a finding (verified to predate this
  branch — reproduced on the shipped `detect` target). Workaround:
  `ASAN_OPTIONS=detect_leaks=0`, which disables only the leak scan; ASan's
  use-after-free/overflow instrumentation and the fuzz seams' own assertions
  stay on.
- Dependabot watches `Cargo.toml`/`Cargo.lock` and GitHub Actions, but **cannot read `mise.toml`**.
  The stable Rust toolchain pin, the nightly-2026-07-01 toolchain pin, the cargo-deny pin, and the
  cargo-fuzz pin there are all manual bumps and get no automated security PRs.

## Conventions
- Cross-refs are symbolic (`RefTarget::Internal`) until pass 4 resolves them to relative links.
- Adapters must never trust input: guard decompression ratio/size and entry-name traversal.
- A crash the fuzzer finds gets its reproducer committed to `fuzz/artifacts/<target>/`.
  If the bug is fixed, that commit is what makes the crash a permanent regression
  test on stable from then on. If it's left open, the reproducer is committed
  anyway and the (target, file) pair goes in `KNOWN_OPEN` (`tests/fuzz_corpus.rs`)
  so the stable suite stays green without dropping the input; remove the entry
  when the fix lands.
- The nightly toolchain pin, like the Rust and cargo-deny pins, is a manual bump.
- A failing property writes `crates/kasane-writer/tests/properties.proptest-regressions`.
  Commit it, for the same reason a fuzz reproducer is committed: it is what makes
  the found case a permanent regression test.
- `crates/kasane-writer/tests/census-known-corrupt.txt` is a ratchet, not a
  todo list: `census.rs` fails the build if a shape is corrupt and unlisted,
  *and* if a listed shape is no longer corrupt, so the file cannot grow
  silently or rot into stale excuses. Regenerate it with
  `KASANE_CENSUS_BLESS=1 cargo test -p kasane-writer --test census` and read
  the diff — that diff is the exact evidence a reviewer wants, of what a
  change fixed or broke.
- The census has two tiers, and four files. The text tier above compares what
  a parser recovers against `kasane_gfm::rendered_text`. The **structural**
  tier compares, for each character, the stack of emphasis containers enclosing
  it on both sides — a loss that leaves the text byte-identical (a `<strong>`
  coming back as an `<em>`, a nesting level dropped) is invisible to the first
  tier and caught by the second. It runs only where the text tier already
  passes, since per-character alignment presupposes equal strings.
  `census-known-structure-corrupt.txt` is its queue, target zero;
  `census-inexpressible.txt` holds the shapes **this writer's `*`-only
  alphabet** cannot express. It is not a statement about Markdown, which is
  what this entry claimed until 2026-08-17: CommonMark also has `_`, and
  alternating the two spells every mechanism the file names — `_*x*_` is
  `<em><em>x</em></em>`, `__**x**__` is doubly strong, `__*x*__` is
  `<strong><em>x</em></strong>`. A probe over every `*`/`_` spelling found
  1,740 of its 1,984 entries expressible, so read it as the queue for the
  alphabet-widening item. What is genuinely unspellable is narrower and has a
  different cause — CommonMark's left-flanking rule, which stops any delimiter
  opening between a letter and punctuation, so `[Text("a"), Text("a"),
  Emph([Code("x")])]` cannot emphasize with `*` or `_`.
  The split between those two files is **computed on every bless,
  never hand-edited**: a shape is permanent only if it both nests, directly, a
  same-class container (`<em><em>x</em></em>` or
  `<strong><strong>x</strong></strong>`) or a `<strong>` whose sole child is
  an `<em>`, and differs from the IR only by collapsing adjacent identical
  classes and dropping an emphasis directly inside a strong. The asymmetry is
  deliberate: `<em><strong>x</strong></em>` IS spellable, and keeping it out
  of the permanent file is what stops a regression laundering itself as a
  representational limit.
  One bless command rewrites all three shape files — but **not** the fourth,
  `census-permanent-count.txt`, a one-integer ceiling on how many entries
  `census-inexpressible.txt` may hold. A bless lowers it to match a shrink and
  never raises it, so growing the permanent file leaves the test failing until a
  human raises the number in the same commit. That asymmetry is the point:
  moving a shape into the permanent file asserts no writer change can ever fix
  it, which is the one claim here nothing downstream re-examines, and it is the
  claim that went wrong for 748 shapes at once. `mise run census-ratchet` is
  the other half — it compares the committed files against the merge base with
  `main` and fails if the text or structure queue, or their union with the
  permanent file, gained any shape. The union is what makes reclassification
  safe to allow: a shape may move between the two files, but none may become
  corrupt that was not. It runs in CI *after* `mise run test`, because it takes
  the files' accuracy on trust and only the test establishes that.
  Design spec
  `2026-08-16-structural-census-design.md`; its §6 recorded the largest queued
  family, 2,002 shapes losing a level because
  `splice_children`'s edge rule keys on the delimiter character and
  `Delim::ch()` maps both classes to `*`.
  `2026-08-16-cross-class-edge-splice-design.md` narrows that edge rule so
  `Emph[Strong[x]]` prints `***x***`, and files the mirror shape permanent
  because `***x***` always resolves em-outermost: 366 of the 2,002 went clean
  and 748 moved to the permanent file, leaving 888 still queued -- most of
  them blocked by `run_end` fusing the shape with a `*`-delimited neighbour
  (that spec's §6).
- Inline nesting is bounded twice, deliberately. `epub::xhtml::MAX_INLINE_DEPTH`
  (64) is a fidelity bound that flattens without losing content;
  `kasane_ir::MAX_INLINE_DEPTH` (256) is a safety bound in the core and writer's
  recursive walks, which adapter-produced IR can never reach. Unbounded, deep
  nesting aborts the process on a stack overflow.
  BLOCK nesting (`Block::List`/`Block::Footnote`) is bounded the same way, and
  by the same two-constant shape: `epub::xhtml::MAX_BLOCK_DEPTH` is the
  fidelity bound that flattens without losing content (and covers MOBI/AZW3
  too, which re-serializes through that parser), while
  `kasane_ir::MAX_BLOCK_DEPTH` is the safety bound in the recursive walks. A
  compile-time assertion in `epub/xhtml.rs` enforces
  `MAX_BLOCK_DEPTH * 4 <= kasane_ir::MAX_BLOCK_DEPTH`, so raising the fidelity
  bound past a quarter of the safety bound fails the build rather than
  silently weakening the design. Eleven production walks recurse on block
  nesting and all eleven carry the bound. Six run in the EPUB/MOBI adapters during
  `parse`, before `kasane-core` is reached: `epub::fix_block_links`,
  `mobi::strip_empty_anchor_links`, `epub::collect_figure_keys`,
  `epub::degrade_failed_figures`, `epub::collect_note_refs`,
  `epub::xhtml::flatten_block_inlines`. Five are in the core and writer:
  `section::clone_block`, `balance::est_tokens_block`, `paths::count_headings`,
  `refs::fix_block`, `kasane_writer::blocks_to_markdown`.
  `clone_block` is the load-bearing one: it is the first core walk to touch
  the IR, so the later four see already-shallow blocks. The drop side is
  separately safe via `kasane_ir::teardown_document`'s explicit worklist.
- Every change ships green under `mise run lint && mise run test`.
