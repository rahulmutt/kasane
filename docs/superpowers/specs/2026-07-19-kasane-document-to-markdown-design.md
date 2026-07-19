# kasane — Design Spec

**Date:** 2026-07-19
**Status:** Approved (design), pending implementation plan
**Repo:** kasane — "A document processor that produces progressively disclosed, AI agent friendly Markdown output"

## 1. Purpose & Scope

kasane is a Rust CLI that converts any of six document/ebook formats — **PDF, DJVU, EPUB, MOBI, AZW3, PPTX** — into an **AI-agent-friendly, progressively disclosed Markdown file tree**: a hierarchical, cross-linked directory of Markdown files an agent enters at a root `index.md` and drills into on demand.

### Confirmed decisions

| Decision | Choice |
|---|---|
| Output shape | **Tree of files + index.** One directory per document; `index.md` map links down into per-chapter/per-section files; cross-links are relative paths. |
| Parsing strategy | **Pure Rust** crates only on the default path. No mandatory host tools. Single static binary. |
| Scanned / no-text-layer content | **OCR is an opt-in Cargo feature** (`-F ocr`, off by default). Only this feature links a C library (Tesseract). Core stays pure Rust. |
| Fidelity | Preserve **tables, math (LaTeX), images + captions, footnotes & internal references** faithfully. |
| File boundaries | **Heading-driven split with a size guard** (split over-long sections, merge tiny ones) + rich per-file navigation frontmatter. |
| Internal architecture | **Normalized IR + hexagonal pipeline** (approach A). |

### Non-goals (v1)

- No breaking of DRM. DRM-protected inputs are detected and rejected with a clear error.
- No mandatory LLM/summarization step. Conversion is deterministic and offline.
- No OCR on the default build. DJVU and scanned PDFs are best-effort (text layer / images + placeholder note) unless `-F ocr`.
- No Bazel; a Cargo workspace is the right size.

## 2. Architecture

Hexagonal pipeline. Format parsers are input adapters; the Markdown tree writer is the output adapter; the IR + structuring engine is the domain core.

```
input file ─▶ [format detection] ─▶ [input adapter] ─▶ IR ─▶ [structuring engine] ─▶ [markdown tree writer] ─▶ output dir
                                        (untrusted           (domain core:              (output adapter)
                                         boundary)           all the real logic)
```

Adding a format = one new adapter; the core and writer are untouched. The structuring engine is tested against IR fixtures with no real files involved.

## 3. The Intermediate Representation (IR)

One format-agnostic document model that every adapter targets and the core consumes. Modeled in spirit on Pandoc's AST, pure Rust.

```rust
struct Document {
    meta: DocMeta,          // title, authors, language, source format, source path
    blocks: Vec<Block>,     // linear stream; headings imply hierarchy
}

enum Block {
    Heading { level: u8, id: BlockId, inlines: Vec<Inline> },
    Para(Vec<Inline>),
    List { ordered: bool, items: Vec<Vec<Block>> },
    Table(Table),                 // rows/cells; merged-cell flag → HTML fallback
    Figure { image: AssetRef, caption: Vec<Inline>, number: Option<String> },
    CodeBlock { lang: Option<String>, text: String },
    MathBlock(String),            // LaTeX
    Footnote { id: NoteId, blocks: Vec<Block> },
    Raw { note: String },         // e.g. "scanned page, no text layer"
}

enum Inline {
    Text(String), Emph(Vec<Inline>), Strong(Vec<Inline>), Code(String),
    Math(String),                 // inline LaTeX
    Link { target: RefTarget, inlines: Vec<Inline> },
    FootnoteRef(NoteId),
}

enum RefTarget { Internal(BlockId), External(Url), Footnote(NoteId) }

struct Provenance { source_pages: Option<Range<u32>>, source_href: Option<String> } // per top-level block
```

Key properties:

- **Cross-references are symbolic** (`RefTarget::Internal(BlockId)`), never file paths, in the IR. The structuring engine assigns files first, then resolves every internal ref to the correct `relative/path.md#anchor`. This keeps cross-linking correct regardless of how the size guard splits content.
- **Provenance** (source page range / href) rides along so frontmatter can render `source_pages: 12–18`.
- **Assets** (images) are collected into an `AssetBag` during parsing; the writer flushes them to `_assets/` and rewrites `AssetRef`s to relative paths.

## 4. Format detection & input adapters

### Detection

By **magic bytes first** (not extension — this is the untrusted boundary). ZIP-container sniffing distinguishes EPUB/AZW3/PPTX by internal structure; PDF `%PDF`; MOBI `BOOKMOBI`; DJVU `AT&T`/`FORM…DJVU`. Extension is only a tiebreaker hint.

### Adapter port

```rust
trait Adapter {
    fn sniff(bytes: &[u8]) -> Confidence;
    fn parse(&self, input: Source, opts: &ParseOpts) -> Result<Document, ParseError>;
}
```

### Per-format plan

| Format | Pure-Rust approach | Notes / honest limits |
|---|---|---|
| **EPUB** | `zip` + `quick-xml`; OPF spine order, each XHTML → IR; MathML → LaTeX. | Highest-fidelity source. `<a href>` → clean footnotes/refs. |
| **PPTX** | `zip` + `quick-xml` over `ppt/slides/*.xml`. One heading per slide; text frames → paras/lists; media → figures; notes appended. | Slides are the natural heading unit. |
| **MOBI** | `mobi` crate → PalmDOC/record decode → HTML → IR. | Older MOBI: fine. |
| **AZW3 (KF8)** | `mobi` crate (KF8) → HTML payload → IR. | DRM detected → **fail clearly** ("DRM-protected, unsupported"). |
| **PDF** | `pdf`/`lopdf` text layer + positions; heuristic layout pass groups runs into paragraphs, infers headings from font-size/weight clusters; images → figures. | Born-digital only in core. No text layer → `Raw{"scanned page"}` + page image, unless `-F ocr`. |
| **DJVU** | Minimal pure-Rust container parse for embedded **hidden text layer** → IR; else metadata + page images. | Weakest pure-Rust format. With `-F ocr`, page images route through OCR. Documented plainly. |

### OCR seam

A `TextExtractor` trait sits behind PDF/DJVU. Default `BornDigitalExtractor` reads the embedded text layer; `-F ocr` swaps in `TesseractExtractor` — the only component that links C. The rest of the codebase is agnostic to which is active.

### Security (untrusted input boundary)

Every adapter treats input as hostile:
- Decompression-bomb guards: max expansion ratio + absolute size cap on ZIP/PalmDOC.
- XXE-safe XML: entity expansion disabled in `quick-xml`.
- No path traversal: sanitize archive entry names and asset filenames, confine writes to `_assets/`.
- Bounded recursion depth.
- Degrade, don't die: a corrupt block becomes a `Raw` note; the whole-document parse is not aborted where recovery is possible.

## 5. Structuring engine (domain core)

Takes one `Document` (flat block stream), produces a `SiteTree` (files + resolved links). Pure, no I/O, fully unit-testable. Five ordered passes:

1. **Build heading hierarchy.** Fold the linear stream into a `SectionTree` using heading levels: blocks between heading *N* and the next heading of level ≤ *N* form that node's body.
2. **Size-guard balancing.** Weight each node's body in estimated tokens (chars/4, configurable).
   - **Split:** a node whose own body exceeds `max_tokens` (default ~2000) with no sub-headings gets synthetic `part-01.md`, `part-02.md` split at paragraph/list boundaries — never mid-table, mid-figure, or mid-paragraph.
   - **Merge:** a leaf under `min_tokens` (default ~200) folds into its parent's `index.md` instead of becoming its own file.
   - Both thresholds are CLI/config flags.
3. **Assign file paths.** Walk the balanced tree; surviving-file nodes get zero-padded, slugified paths (`02-methods/03-sampling.md`); container nodes get `index.md`. Produces the `BlockId → (path, anchor)` map.
4. **Resolve cross-references.** Rewrite each `RefTarget::Internal` into a relative `../foo/bar.md#anchor`. Footnotes resolve to same-file anchors (or a per-section footnotes block). Dangling refs (target dropped/merged) degrade to plain text + logged warning — never a broken link.
5. **Emit navigation.** Per-file frontmatter: `title`, `breadcrumb`, `parent`, `prev`/`next` (document reading order via in-order traversal), `source_pages`, `children`. Each `index.md` gets an auto TOC of its children.

Example frontmatter:

```yaml
---
title: Background
breadcrumb: Book > Intro > Background
parent: ../index.md
prev: 01-overview.md
next: ../02-methods/index.md
source_pages: 12-18
---
```

## 6. Markdown tree writer (output adapter)

Mechanical serialization of IR to GitHub-Flavored Markdown:
- Tables → GFM tables; merged-cell tables → raw HTML fallback.
- Math → `$…$` (inline) / `$$…$$` (block).
- Writes frontmatter, flushes the `AssetBag` to `_assets/`, writes files.
- Root `index.md`: document title, metadata, source-format note, top-level TOC — the agent's entry point.
- The only component that touches the filesystem for output.

**Safety:** output dir computed from input stem; refuses to overwrite a non-empty dir unless `--force`; writes to a temp dir and atomically renames on success, so a crash never leaves a half-written tree.

## 7. CLI

`clap` (derive):

```
kasane <INPUT> [OUTPUT_DIR] [options]

  <INPUT>              file, or a directory / glob for batch mode
  -o, --out <DIR>      output root (default: ./<input-stem>/)
      --force          overwrite non-empty output dir
      --max-tokens <N> size-guard split threshold (default 2000)
      --min-tokens <N> size-guard merge threshold (default 200)
      --format <FMT>   override auto-detection
      --no-assets      skip image extraction (text-only)
      --ocr            use OCR extractor (only in -F ocr builds; else clear error)
  -j, --jobs <N>       parallel workers for batch mode
  -v / -q, --json-logs verbosity; machine-readable log stream
```

Batch mode fans out across files with `rayon`; one file's failure never aborts the batch (per-file exit summary at the end).

## 8. Error handling

- `thiserror` for typed library errors: `UnsupportedFormat`, `DrmProtected`, `Encrypted`, `NoTextLayer`, `Malformed{..}`.
- `anyhow` at the CLI boundary for context + friendly messages.
- Distinct actionable exit codes (e.g. `2` unsupported/DRM, `3` partial success in batch).
- Degrade-don't-die: a corrupt page becomes a `Raw` note, not a crash.

## 9. Testing strategy

Tiered for a fast PR loop:

- **Unit** — structuring engine against hand-built IR fixtures: heading folding, split/merge thresholds, cross-ref resolution, prev/next ordering, dangling-ref degradation. No files touched.
- **Golden/snapshot** (`insta`) — one tiny real fixture per format → assert emitted tree + file contents. Catches fidelity regressions.
- **Property** (`proptest`) — invariants for any `Document`: every emitted internal link resolves to a real file+anchor; no file exceeds `max_tokens` unless atomically unsplittable; every input block appears exactly once (no loss/dup); prev/next forms a complete chain.
- **Fuzz** (`cargo-fuzz`) — adapters against malformed/hostile inputs; the security boundary must never panic or hang.
- **Static** — `clippy -D warnings`, `rustfmt`, `cargo-deny` (license + advisory audit).

## 10. Project layout & dev environment

```
kasane/
  Cargo.toml            # workspace
  crates/
    kasane-ir/          # IR types (no deps on adapters)
    kasane-adapters/    # six adapters + detection + TextExtractor
    kasane-core/        # structuring engine (pure)
    kasane-writer/      # markdown tree writer
    kasane-cli/         # clap binary
  tests/fixtures/       # tiny per-format sample docs
  mise.toml             # pinned Rust toolchain + dev tools (cargo-deny, cargo-fuzz, insta)
  AGENTS.md / CLAUDE.md # front door
  justfile              # named workflows: build, test, lint, fuzz, run
```

- **mise-first**, pinned Rust version and dev tools (devkit developer-environment). No Bazel.
- **`just` tasks** expose every workflow by name; README + AGENTS.md are the single-sourced discoverability front door; a codebase-map section explains crate boundaries. Onboarding verified by running `just test` fresh.
- OCR's C dependency (Tesseract), when the `ocr` feature is on, is provided via mise/devenv — the only non-Rust build input, off the default path.

## 11. Crate dependency direction

`kasane-ir` depends on nothing internal. `kasane-adapters`, `kasane-core`, `kasane-writer` all depend on `kasane-ir` and nothing on each other (except writer/core sharing IR types). `kasane-cli` wires them together. This enforces the hexagonal boundaries at the compiler level: the domain core cannot reach into an adapter.
