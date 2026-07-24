# kasane — Math → LaTeX Design Spec

**Date:** 2026-07-24
**Status:** Approved (design), pending implementation plan
**Repo:** kasane

## 1. Purpose & scope

Today kasane silently drops math. The IR already has the slots —
`Inline::Math(String)` and `Block::MathBlock(String)` — and the writer already
renders them as GitHub-Flavored Markdown math (`$…$` inline, `$$\n…\n$$`
display, `kasane-writer/src/markdown.rs:70,121`). But nothing populates those
slots: the EPUB XHTML dispatcher has no `<math>` branch, and the PPTX adapter
has no OMML handling. A `<math>` element is treated as an unknown block
boundary — its equation is lost and any stray text nodes inside it get flattened
into garbled inline text.

This item fills the missing half: two front-end mappers that convert **MathML**
(EPUB) and **OMML** (PPTX) into LaTeX, populating the IR slots the rest of the
pipeline already understands. It closes the top item in the README's "Known
limitations" and the deferred math item in both the EPUB and PPTX specs.

kasane targets *AI-agent-friendly* Markdown, and LaTeX in `$$` fences is the
lingua franca an LLM reads math in. A dropped equation is often the single most
information-dense thing on a page (STEM textbooks, papers, technical decks), so
recovering it disproportionately improves the output's usefulness.

### Boundary

The work lives entirely inside `crates/kasane-adapters/`: a new shared `math/`
module, plus wiring in the existing `epub/` and `pptx/` walkers. `kasane-ir`,
`kasane-core`, and `kasane-writer` are **untouched** — the mappers produce the
`Inline::Math` / `Block::MathBlock` values those crates already handle. The
LaTeX string is the only thing that crosses into existing machinery.

### Non-goals (v1)

- **No new IR types.** In particular, no inline-level note/marker variant (see
  §4 for how inline partials self-mark instead).
- **No writer changes.** GFM `$…$` / `$$…$$` rendering already exists.
- **No exhaustive MathML/OMML coverage.** v1 targets an explicitly documented
  construct subset (§3); anything outside degrades with a note (§4).
- **No Content MathML** (`apply`/`ci`/`cn`). Presentation MathML only.
- **No LaTeX→anything, no rendering.** We emit LaTeX source, not images.

### Confirmed decisions

| Decision | Choice |
|---|---|
| Formats | **Both** MathML (EPUB) and OMML (PPTX), over a shared core. |
| Architecture | **Shared internal `MathNode` model.** Two front-ends map XML → `MathNode`; one emitter renders `MathNode` → LaTeX. |
| Coverage bar | **Documented subset**, enumerated in §3; one fixture per construct; anything else degrades. |
| Degradation | **Best-effort + note.** Emit what maps; unmapped sub-expressions become an in-band placeholder; partial *display* equations also get an adjacent note. |

## 2. Architecture

Math conversion is an adapter-boundary concern (untrusted format XML → IR),
shared by two adapters that both already live in `kasane-adapters`. It is a new
sibling module, not a new crate:

```
crates/kasane-adapters/src/math/
  mod.rs      // public entry points + re-exports
  ast.rs      // MathNode: the shared, format-agnostic math model
  symbols.rs  // Unicode codepoint → LaTeX command table (shared)
  mathml.rs   // MathML island → MathNode   (EPUB front-end)
  omml.rs     // OMML island   → MathNode   (PPTX front-end)
  latex.rs    // MathNode → LaTeX + degradation/note policy (single home)
```

Two public entry points, each returning the same outcome type:

```rust
pub struct MathConversion {
    pub latex: String,   // best-effort LaTeX (NOT wrapped in $ / $$)
    pub complete: bool,  // false ⇒ at least one sub-expression degraded
}

pub fn mathml_to_latex(island: &str) -> MathConversion
pub fn omml_to_latex(island: &str)  -> MathConversion
```

**Data flow.** The adapter's streaming `quick-xml` reader hits a math start tag
→ captures the raw island bytes (§4) → hands them to the matching entry point →
gets back `MathConversion` → writes `Inline::Math(latex)` (inline context) or
`Block::MathBlock(latex)` (display context) into the IR it is already building.
The `complete` flag drives the degradation note.

Three independently testable units behind one small outcome type — two
front-ends and one emitter — mirroring the hexagonal boundary used elsewhere in
the codebase. Both front-ends produce the identical `MathNode`, so `latex.rs` is
the **only** place LaTeX is generated and the **only** home for the degradation
policy; EPUB and PPTX cannot drift.

### New dependency

`roxmltree` (pure-Rust, read-only XML tree). Math is inherently a tree
(a fraction has numerator + denominator children; a sub-superscript has a base
and two scripts), which the streaming `quick-xml` reader makes painful to
recurse over. Each front-end parses the isolated math *island* once with
`roxmltree` and recurses over the resulting tree. This is the one added
dependency; it is confined to the `math/` module.

## 3. The math model & the v1 subset

### `MathNode` (`ast.rs`)

One recursive, format-agnostic enum that both front-ends target and the emitter
consumes:

```rust
enum MathNode {
    Row(Vec<MathNode>),                        // grouping / sequence
    Ident(String),                             // variable (mi / run text)
    Number(String),                            // mn
    Op(String),                                // operator/symbol, already LaTeX-mapped
    Text(String),                              // mtext / literal text run
    Frac(Box<MathNode>, Box<MathNode>),        // num, den
    Sup(Box<MathNode>, Box<MathNode>),         // base, sup
    Sub(Box<MathNode>, Box<MathNode>),         // base, sub
    SubSup(Box<MathNode>, Box<MathNode>, Box<MathNode>), // base, sub, sup
    Sqrt(Box<MathNode>),
    Root(Box<MathNode>, Box<MathNode>),        // radicand, index
    Fenced { open: String, close: String, body: Box<MathNode> },
    Nary { op: String, sub: Option<Box<MathNode>>, sup: Option<Box<MathNode>>, body: Box<MathNode> },
    Matrix(Vec<Vec<MathNode>>),                // rows of cells
    Accent { kind: AccentKind, base: Box<MathNode> }, // hat/bar/vec/tilde/dot
    Unsupported,                               // a subtree outside the subset
}
```

### Documented v1 subset

Each construct gets a fixture; anything else maps to `Unsupported` and degrades
per §4.

- **Atoms:** identifiers, numbers, text runs, operators/symbols (via `symbols.rs`).
- **Fractions.**
- **Scripts:** superscript, subscript, and combined sub+superscript.
- **Radicals:** square root and nth-root.
- **Fenced groups:** `( ) [ ] { } | ⟨ ⟩`.
- **N-ary operators with limits:** sum, integral, product (sub/sup as limits).
- **Basic matrices** → `pmatrix` / `bmatrix` (chosen by the surrounding fence).
- **Common accents:** hat, bar, vec, tilde, dot.

### Symbol table (`symbols.rs`)

One shared Unicode-codepoint → LaTeX-command map that both front-ends run
identifier/operator text through: Greek (`α`→`\alpha`), relations (`≤`→`\leq`,
`≠`→`\neq`), operators (`∑`→`\sum`, `∫`→`\int`, `→`→`\to`), etc. ASCII-safe
characters pass through unchanged. An unmapped non-ASCII symbol becomes a
degraded placeholder (§4).

## 4. Mapping, isolation, and degradation

### Island isolation (shared)

Both adapters drive a streaming `quick-xml` reader, which is awkward for tree
recursion. When the reader hits a math start tag, we capture the raw island:
record the byte position at the start tag, depth-count matching start/end tags
for that element name, and slice out the full `<math>…</math>` /
`<m:oMath>…</m:oMath>` substring. That slice is parsed once with `roxmltree`,
and the front-end recurses over the tree into `MathNode`. The outer streaming
parse is untouched; tree work is confined to the island.

### `mathml.rs` (EPUB front-end)

Maps Presentation MathML onto `MathNode`: `mi/mn/mo/mtext`, `mfrac`,
`msup/msub/msubsup`, `msqrt/mroot`, `mrow`, `mfenced` (and `mrow` bracketed by
`mo` fences), n-ary `mo`+`msubsup`, `mtable/mtr/mtd`, `mover/munder` accents.
Content MathML (`apply/ci/cn`) is out of subset → `Unsupported`.

### `omml.rs` (PPTX front-end)

Maps OMML onto the same `MathNode`: `m:r/m:t` runs, `m:f` (`m:num`/`m:den`),
`m:sSup/m:sSub/m:sSubSup` (`m:e`/`m:sup`/`m:sub`), `m:rad` (`m:deg`/`m:e`),
`m:d` delimiters, `m:nary` (n-ary with `m:sub`/`m:sup`/`m:e`), `m:m/m:mr`
matrices.

### Inline vs display decision

- **EPUB:** `<math display="block">`, or a `<math>` that is the sole flow
  content of its block, → `Block::MathBlock`; a `<math>` inside running
  paragraph text → `Inline::Math`. `xhtml.rs` already tracks paragraph/inline
  context; this reuses that state.
- **PPTX:** `m:oMathPara` → display (`Block::MathBlock`); a bare `m:oMath`
  inside a run → inline (`Inline::Math`).

### Degradation & the note policy (`latex.rs`, single home)

The emitter renders `MathNode → String`. Every `Unsupported` node and every
unmapped symbol emits one consistent **in-band placeholder token** — rendered as
`\mathord{?}` — and sets `complete = false`. At the adapter:

- **Display/block math** that came back `complete = false` → emit
  `Block::MathBlock(latex)` **followed by** `Block::Raw { note: "equation
  partially converted" }`, reusing the existing note idiom so a partial display
  equation carries a visible, greppable flag.
- **Inline math** has no inline note type in the IR, and we deliberately do not
  add one (§1 non-goals). For inline, the in-band `\mathord{?}` placeholder *is*
  the signal — a partial inline equation is self-marking, no sibling note.

This block-note vs inline-placeholder asymmetry is the one deliberate wrinkle,
accepted to avoid an inline IR-marker type for a single consumer.

### Untrusted-input guards

The island is untrusted (it arrives from inside guarded zip content, but the
math subtree itself is unbounded). Two soft bounds, per the discipline in
`guard.rs`, both **never panic**: a maximum island byte size, and a maximum
recursion depth in the front-end. Tripping either bound, or any `roxmltree`
parse error, yields `MathConversion { latex: "\\mathord{?}", complete: false }`
— degrade-with-note, never abort the document.

## 5. Testing strategy

Hand-built fixtures, behavior-per-fixture, everything green under
`mise run lint && mise run test`.

- **Emitter unit tests (`latex.rs`)** — the center of gravity. One test per
  documented construct asserting `MathNode → LaTeX`: fraction, sup/sub/subsup,
  sqrt/root, fenced, n-ary-with-limits, matrix, accents, symbol-mapped
  operators. Degradation: a tree containing `Unsupported` emits `\mathord{?}`
  and reports `complete = false`.
- **Front-end parity tests (`mathml.rs`, `omml.rs`)** — small inline XML islands
  → expected `MathConversion.latex`, one per construct, run for **both** dialects
  so parity is proven (e.g. `x²`, `½`, `√2`, `∑` with limits, a 2×2 matrix each
  render to identical LaTeX from MathML and from OMML). Out-of-subset islands
  (Content MathML `apply`; an unmapped OMML element) → placeholder +
  `complete = false`.
- **Guard tests** — oversized island, pathologically deep island, and a
  malformed/truncated island (roxmltree parse error) each return
  degrade-with-note, no panic.
- **Integration fixtures (end-to-end to Markdown)** — two new hand-built
  fixtures matching the existing `make_*.py` generator convention:
  - a tiny **EPUB** with one inline and one display MathML equation → asserts
    `$…$` and `$$…$$` land in the output tree, and that a partial display
    equation emits the adjacent `Block::Raw` note.
  - a tiny **PPTX** slide with one `m:oMath` (inline) and one `m:oMathPara`
    (display) → the same assertions.

## 6. Docs (definition of done)

- **README** — replace the "MathML (EPUB) and OMML (PPTX) math are not yet
  converted to LaTeX" limitation with the supported-subset + best-effort-note
  behavior.
- **AGENTS.md** — add the `math/` seam to the codebase map (shared `MathNode`
  model, two front-ends, one LaTeX emitter; `roxmltree` island parsing;
  degrade-with-note boundary).
