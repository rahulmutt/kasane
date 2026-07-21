# MOBI/AZW3 adapter — Design Spec

**Date:** 2026-07-21
**Status:** Approved (design), pending implementation plan
**Parent spec:** `2026-07-19-kasane-document-to-markdown-design.md` (MOBI and AZW3 rows of the per-format plan)

## 1. Purpose & scope

Add an input adapter for classic MOBI (MOBI 6) and AZW3 (KF8) ebooks at **full
fidelity parity with the EPUB adapter**: lists, tables, figures with extracted
assets, code blocks, and resolved internal links — including MOBI-native link
targets (`filepos` byte offsets in MOBI 6, `kindle:pos` URIs in KF8).

**In scope:** both variants in one adapter; format detection; DRM detection with
clear rejection; image extraction into the `AssetBag`; document metadata from
EXTH records; README/AGENTS.md updates (including removing the two stale
"Known limitations" entries closed by the EPUB-fidelity merge).

**Out of scope:** DRM removal (rejected per parent spec); Topaz/KFX and other
Amazon formats; MOBI *output*; CSS/flow interpretation (kasane does not consume
stylesheets); heuristic footnote detection — MOBI 6 has no semantic footnote
markup, so note links resolve as ordinary internal links, the same non-lossy
stance the EPUB fidelity spec took for books without EPUB3 semantics.

### Confirmed decisions

| Decision | Choice |
|---|---|
| Fidelity | Full parity with EPUB from day one, via parser reuse. |
| Internal links | Resolve `filepos` and `kindle:pos` to real block targets. |
| Container layer | The `mobi` crate (mobi-rs, MIT): PalmDB parse, PalmDoc + HUFF/CDIC decompression, image records, `encryption()` DRM check, EXTH metadata. |
| KF8 reassembly | Built by us on mobi-rs raw records (SKEL/FRAG INDX parsing); mobi-rs does not provide it. |
| HTML normalization | `html5ever` (pure Rust) parses sloppy HTML to a DOM and re-serializes well-formed XHTML for the existing streaming parser. |

Approaches rejected: hand-rolling the whole PalmDB/decompression stack (re-does
mobi-rs before adapter work starts, and HUFF/CDIC decoders have CVE history
elsewhere — reuse and guard instead), and splitting AZW3 into a follow-up item
(modern Kindle books are AZW3; shipping without it undercuts the format's value).

## 2. Architecture & data flow

New module `crates/kasane-adapters/src/mobi/`. One adapter handles both
variants; the fork is internal.

```
bytes ─▶ mobi-rs (PalmDB, decompress, EXTH meta, image records, encryption())
              │
   MOBI 6 ────┤──── KF8/AZW3
   raw HTML stream        raw ML stream + INDX records
   │                      │
   filepos anchor splice  SKEL/FRAG reassembly into parts,
   │                      kindle:pos → aid anchors, kindle:embed → assets
   ▼                      ▼
   HTML normalization (html5ever: sloppy HTML → well-formed XHTML)
   ▼
   existing xhtml_to_blocks (block-frame parser, untouched)
   ▼
   IR Document (links symbolic; assets in AssetBag; core/writer untouched)
```

The structural bet: **both variants converge on normalized XHTML and reuse
`xhtml_to_blocks` unchanged.** The parse→serialize→reparse round trip is
deliberate — chosen over teaching the streaming parser HTML5 recovery rules or
duplicating its block-frame logic as a DOM walker. Chapters are small; the
double parse is cheap.

Variant selection: MOBI header version ≥ 8, or a KF8 boilerplate section in a
combo MOBI+KF8 file, takes the KF8 path (combo files prefer the KF8 half,
matching calibre). Anything else takes the MOBI 6 path.

MOBI 6 has no per-chapter files — the book is one HTML stream. The adapter
emits one linear block stream; the core folds structure from headings. KF8
parts behave like EPUB spine items.

## 3. MOBI 6 path

- **Text** via mobi-rs `content_as_string_lossy()`, which handles the format's
  two encodings (UTF-8, WIN1252).
- **Internal links.** `<a filepos=NNNNNNNNNN>` targets byte offsets into the
  raw HTML stream, which normalization would shift. Before normalization:
  collect all `filepos` values, splice `<a id="kasane-fp-N"/>` markers into the
  raw stream at those offsets — snapped forward to the next tag boundary — and
  rewrite each link to `href="#kasane-fp-N"`. The existing per-file anchor-map
  machinery then resolves them like any EPUB internal link.
- **Images.** `<img recindex="NNNNN">` is a 1-based index into the image record
  list. Normalization rewrites `recindex` to a synthetic `src` naming an asset;
  bytes come from `image_records()`, extension sniffed from magic bytes
  (JPEG/PNG/GIF/BMP). Assets flow into the `AssetBag` under the same size
  guards as EPUB.
- **`mbp:` tags** (`<mbp:pagebreak>` et al.) are dropped as markup, contents
  kept — structure comes from headings.

## 4. KF8/AZW3 path

- **Reassembly.** Parse the SKEL and FRAG (DIVTBL) INDX records; cut the raw ML
  stream into skeletons and insert each fragment at its recorded offset,
  yielding N XHTML parts. This is calibre's `mobi8` algorithm; the MobileRead
  KF8 page documents the tables. Every offset/length is bounds-checked against
  the actual stream — a lying index degrades that part to a `Raw` note, never a
  panic or out-of-bounds read.
- **Trailing entries (stated risk).** mobi-rs may not strip per-record trailing
  entries (multibyte/TBS data appended to text records). If its output proves
  polluted, we strip them ourselves from the raw records using the header's
  `extra_data_flags` — a small, well-documented computation. The implementation
  plan must verify this early, on a real AZW3, before building on the stream.
- **Internal links.** `kindle:pos:fid:XXXX:off:YYYYYYYYYY` (base-32 fields) →
  fragment table → (part, byte offset) → snap to the nearest enclosing or
  following tag carrying an `aid` attribute, which becomes the anchor id. Links
  rewrite to part-relative refs and resolve through the existing anchor map.
- **Images.** `<img src="kindle:embed:XXXX?mime=...">` — base-32 resource index
  into the same image records; handled like `recindex` above. `kindle:flow:`
  (CSS) references are ignored.

**Metadata (both paths):** title, author, language from EXTH records via
mobi-rs into `DocMeta`.

## 5. Detection

`detect.rs` gains the PalmDB sniff: `BOOKMOBI` at byte offset 60 →
`Format::Mobi`. Other Palm database types (including Topaz) remain unsupported
with a clear error. Extension stays a tiebreaker only, per convention.

## 6. Security (untrusted boundary)

mobi-rs decompresses internally, so the EPUB-style ratio guard cannot wrap the
inner loop. The adapter enforces caps around it instead:

- **Before decompression:** reject files whose headers declare a text length or
  record count exceeding the `guard.rs` limits (512 MiB absolute, 200:1
  declared-vs-file-size ratio — reuse the existing constants).
- **After decompression:** re-check the actual output size against the same
  caps as a backstop against lying headers.
- INDX/SKEL/FRAG parsing is fully bounds-checked; base-32 fields parse into
  checked integers; malformed tables degrade to `Raw`.
- The filepos splice inserts only at verified tag boundaries. `html5ever`
  bounds its own nesting.

## 7. Error handling

- `encryption() != None` → existing `DrmProtected` error (exit code 2), message
  naming the format.
- Unreadable container → `Malformed`.
- Per-part parse failures degrade to `Raw` notes — degrade, don't die.

## 8. Testing

- **Unit** — filepos splicing (offset math, tag-boundary snapping), base-32
  decoding, SKEL/FRAG parsing and reassembly against hand-built byte fixtures,
  trailing-entry stripping, recindex/kindle:embed rewriting, normalization of
  representative sloppy-HTML snippets.
- **End-to-end** — `tests/fixtures/mobi/minimal.mobi` and
  `tests/fixtures/azw3/rich.azw3`; the rich fixture is generated from the
  existing `rich.epub` via Calibre's `ebook-convert`, committed as a binary
  with its generator script (`make_rich_azw3.sh`), matching the established
  fixture pattern. The rich test asserts the full fidelity set survives:
  lists, tables, figures + assets, code, working internal links.
- **Negative** — a DRM-flagged fixture asserting clear rejection; lying-INDX
  fixtures asserting degradation.
- Green under `mise run lint && mise run test`.

## 9. Dependencies & docs

- Add `mobi` and `html5ever` to the adapters crate (both permissively
  licensed; cargo-deny vets them).
- README: add MOBI/AZW3 to the supported list; delete the two stale "Known
  limitations" entries closed by the EPUB-fidelity merge.
- AGENTS.md codebase map: note the new `mobi/` module and that it reuses the
  EPUB XHTML parser via normalization.
