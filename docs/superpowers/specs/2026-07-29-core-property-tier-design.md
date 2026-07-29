# kasane — Structuring-Engine Property Tier Design Spec

**Date:** 2026-07-29
**Status:** Approved (design), pending implementation plan
**Repo:** kasane
**Parent spec:** `2026-07-19-kasane-document-to-markdown-design.md` (§9, the `proptest` tier)
**Sibling spec:** `2026-07-27-adapter-fuzzing-design.md` (§1, which deferred this tier by name)

## 1. Purpose & scope

Design spec §9 lists five test tiers. Three are built: unit, fuzz, and static.
The **property tier** and the golden/snapshot (`insta`) tier are not. The
adapter-fuzzing item deferred the property tier explicitly — "it is independently shippable and stays out of
scope; this item touches only the untrusted-input boundary."

This item builds it. §9 names the invariants: every emitted internal link
resolves to a real file and anchor; no file exceeds `max_tokens` unless
atomically unsplittable; every input block appears exactly once (no loss, no
duplication); `prev`/`next` forms a complete chain.

`kasane-core` is the right target and the one that most needs it. It is 912
lines across five ordered passes (`fold_sections` → `balance` → `assign_paths` →
`resolve_refs` → `structure`/nav), it holds essentially all of the project's
real logic, and unlike every adapter it has no format fixtures to lean on — its
existing tests are six hand-built examples. Example-based tests cover the shapes
someone thought of; these passes interact (a merge changes what a split sees; a
split changes what path assignment sees; both change what refs resolve against),
and interactions are what properties are for.

### Scope

The tier runs **end to end over `kasane-core` and `kasane-writer`**: generate a
`Document`, run `structure()`, render each file with `blocks_to_markdown`, assert
invariants against the resulting text. The link invariant is the reason. Inside
`kasane-core` alone it degrades to "the anchor string matches the slug core
assigned to itself" — self-consistent and blind to whether that anchor exists in
the emitted Markdown. §9's wording is "resolves to a real file+anchor", and only
the rendered text can answer that.

Designing that invariant found two live defects (§2). Both are fixed here, under
the standing rule that a defect found next to the one being worked on gets
closed in-branch rather than documented and deferred.

### Non-goals

- **`insta` snapshot tests.** The remaining unbuilt §9 tier. Out of scope.
- **Adapter-level properties.** The untrusted-input boundary is the fuzzer's
  job and is already covered by twelve targets.
- **`write_tree`'s filesystem behavior.** The atomic swap, the `--force` guard,
  and the temp-dir cleanup are covered by its own unit tests. Nothing in this
  tier touches the filesystem.
- **The `ocr` feature.** Not on the default path; nothing in this tier is
  feature-gated.
- **A deep sweep workflow.** One budget, PR CI, no new pinned tool (§7).

## 2. Two defects found while designing the invariants

### 2.1 Every in-book cross-reference resolves to a dead anchor

`fold_sections` consumes a section's `Block::Heading` into `SectionNode.title`
and never re-emits it. `assign_paths` records the section's anchor as
`format!("{}#{}", self_path, slug(&node.title))` (`paths.rs:30`), and
`resolve_refs` rewrites every `RefTarget::Internal` to that target
(`refs.rs:63-67`). The writer then emits frontmatter plus `node.body` — and
`node.body` does not contain the heading. Converting `tests/fixtures/epub/rich.epub`
shows it directly:

```
---
title: Chapter One
breadcrumb: Rich Book > Chapter One
parent: index.md
prev: index.md
next: 02-chapter-two.md
---

Intro with *emphasis*, `inline_code()`, and a
```

No `# Chapter One`. Nothing in the file produces a `#chapter-one` anchor. The
EPUB adapter (`epub/mod.rs:293`) and the MOBI adapter both emit real
`RefTarget::Internal` links for in-book references, so this is live on real
books, not a theoretical gap: every cross-reference lands on the right file at
the wrong place, and any Markdown renderer that validates anchors reports it as
broken.

A second symptom rides along: a section file currently opens with no visible
title at all. For a project whose entire purpose is a progressively-disclosed
tree an agent drills into, a file that does not name itself in its body is a
fidelity bug in its own right.

**Fix.** The writer emits the file's title as a heading at the top of the body,
from `Frontmatter.title`, so the anchor `resolve_refs` points at exists. Its
level follows `breadcrumb.len()`, clamped to 1..=6, so the root `index.md` opens
with `#` and a depth-two section with `##`.

**Merged subsections are a different case, and not a defect.** `balance` removes
an absorbed child from the tree before `assign_paths` runs, so no anchor is ever
recorded for it; `refs.rs:63-68` finds nothing in the map and strips the link to
plain text. That is the documented pass-4 behavior — "dangling refs degrade to
plain text, never a broken link" — so merged subsections lose links rather than
breaking them.

They can do better, and this item makes them. Two changes, which are only worth
making together:

1. `balance`'s merge path demotes an absorbed child to a real `Block::Heading`
   carrying its original `BlockId`, rather than
   `Block::Para(vec![Inline::Strong(title)])` (`balance.rs:24-28`). A synthetic
   split part has `id: None` and nothing can link to it, so that case keeps the
   bold lead-in.
2. `place` (`paths.rs:23`) additionally scans each node's top-level `body` for
   `Block::Heading { id, inlines, .. }` and records
   `anchors[id] = "{self_path}#{slug(inlines)}"`.

Without (2), (1) is purely cosmetic — `### Tiny` instead of `**Tiny**`. With
both, a cross-reference into a merged subsection resolves to the exact heading it
names instead of degrading to text. Only top-level body blocks are scanned: a
heading nested inside a list item was never folded into a section either, and
giving it an anchor would invent structure the engine does not otherwise model.

`blocks_to_markdown` takes `&[Block]` and never sees a `Frontmatter`, so the
title heading cannot be prepended there, and prepending it inside
`write_tree_contents` would put it on a path the property suite never runs
(the suite deliberately touches no filesystem). The writer therefore grows one
public function, `file_to_markdown(&FileNode, &AssetBag) -> String`, which
prepends the title heading and delegates to `blocks_to_markdown`.
`write_tree_contents` and the property suite both call it, so what CI asserts is
byte-for-byte what a conversion writes.

This changes the output of every conversion. Existing end-to-end content
assertions are updated as part of the change.

### 2.2 Deep inline nesting aborts the process

`inline_text` (`paths.rs:84`), `est_tokens_block` (`balance.rs:75`),
`fix_inlines` (`refs.rs:48`) and `inlines_to_md` (`markdown.rs:113`) all recurse
on inline nesting depth with no bound. A `Document` holding a single `Para` with
10,000 nested `Inline::Emph` aborts:

```
thread 'deep_inline_nesting_does_not_overflow' has overflowed its stack
fatal runtime error: stack overflow, aborting        (signal: 6, SIGABRT)
```

This was measured, not inferred. Reachability from untrusted input is the
concern: the EPUB XHTML parser builds nested inlines from an iterative frame
stack with no depth limit, so deeply nested `<em>` in a hostile book produces
exactly this shape. The existing `epub` fuzz target exercises adapter *parse*
only — it never calls `structure()` or the writer — so the path is unfuzzed
today. A stack overflow aborts the process; it is not a recoverable error, and
in batch mode it takes every other document's worker down with it.

**Fix, from both sides.**

1. **At the adapter boundary,** where the repo's convention puts guards
   ("Adapters must never trust input"): the EPUB XHTML parser flattens inline
   nesting past a documented depth constant, in the style of the existing
   `guard.rs` bounds — `MAX_INLINE_DEPTH = 64` in `epub/xhtml.rs`, past which a
   closing inline tag contributes its text content instead of adding another
   `Inline` level. No content is lost. 64 is far past any real book's
   `<em><strong><a>` layering.
2. **In the core and writer,** so `kasane-core`'s published `structure()` is
   safe for any caller and not only for kasane's own adapters: the four
   recursive inline walks carry a depth counter and stop descending past
   `kasane_ir::MAX_INLINE_DEPTH = 256`, contributing nothing below it. The
   constant lives in `kasane-ir` because both crates need the same value.

The two bounds differ deliberately. The adapter's 64 is a *fidelity* bound that
preserves content, and it is what every real document meets. The core's 256 is a
*safety* bound that drops content past it, and adapter-produced IR can never
reach it — it exists only for a hand-built `Document` from an external caller.

The values are measured, not guessed. In a debug build on a libtest thread,
nesting depth 256 and 1024 both complete; 4096 aborts. 256 leaves at least a
4× margin under the tightest stack the test suite runs on.

A fuzz seed carrying the shape is committed, and the shared `adapter()` helper in
`fuzz_entry` gains an inline-depth assertion (against the core's 256, the bound
that actually matters for safety) so every format adapter is held to it, in the
same style as the existing `assert_assets_contained`.

## 3. Architecture

One new test target. No new crate, no filesystem, no new workflow, no change to
any published crate's dependencies.

```
crates/kasane-writer/
  Cargo.toml                    # + proptest dev-dependency (pinned)
  tests/
    properties.rs               # the property suite; `mod generator;`
    generator/mod.rs            # adapter-realistic Document strategies
    proptest-regressions/
      properties.txt            # committed on failure — the permanent record
```

The pipeline under test, entirely in memory:

```
generator::document()  ─▶  Document  ─▶  kasane_core::structure(doc, &opts)  ─▶  SiteTree
                                                                                    │
                             per FileNode: file_to_markdown(&file, &assets)   ◀──────┘
                                                  │
                                        Vec<(path, rendered_text)>  ─▶  invariants
```

`file_to_markdown` (§2.1) is the same function `write_tree_contents` calls, so
the text the properties assert against is the text a real conversion writes.

`kasane-writer` hosts the target because it is the only crate that already
depends on both `kasane-ir` and `kasane-core`, so the suite reaches the whole
pipeline without adding a single dependency edge. `proptest` goes in
`[dev-dependencies]`, which never reaches consumers of the published crate.

Frontmatter is read from the typed `Frontmatter` struct, not from emitted YAML:
`frontmatter_yaml` is private, and the invariants concern `prev`/`next`/
`children`/`breadcrumb` as values, not YAML syntax.

PR CI picks the target up automatically through the existing
`cargo test --workspace` in `mise run test`. No `mise.toml` change.

## 4. The generator

### 4.1 Sentinels

Strategies compose without shared state, so per-block uniqueness cannot come
from a counter threaded through generation. The strategy instead produces a
skeleton whose text carries a placeholder, and a single deterministic
`prop_map` stamps sequential sentinels (`zq0000`, `zq0001`, …) over the finished
skeleton. Uniqueness holds by construction, and shrinking stays sound: removing
blocks re-stamps the remainder, and the invariant requires only that the values
be distinct, never that a given block keeps a given value.

Every other piece of generated text is drawn from a fixed word list over an
alphabet excluding the `zq` prefix, so generated content cannot collide with a
sentinel by accident.

### 4.2 Expected multiplicity

Counting is against `file_to_markdown` output only, never frontmatter YAML.
The table is grounded in what `markdown.rs` actually renders:

| Block | Sentinel appears | Why |
|---|---|---|
| Para, List, Table, CodeBlock, MathBlock, Raw, Footnote | exactly 1 | one render site each |
| Figure, `number: None` | exactly 1 | alt text only |
| Figure, `number: Some(_)` | exactly 2 | `markdown.rs:54-61` renders the caption as alt text *and* again in the `*Figure N: …*` line |
| Heading | at least 1 | legitimately recurs: the file's own title heading, the parent's TOC link, a merge lead-in |

The Figure row is the only place a property encodes writer behavior. It is two
lines, and it describes a deliberate accessibility choice (alt text plus a
visible caption), so it is documented here rather than silently tolerated.

### 4.3 Shapes

Adapter-realistic, because a failure should be unambiguously reachable from a
real document:

- **Heading levels** drawn from 1..=6 in arbitrary sequence. Real EPUBs skip
  levels, and every adapter clamps to this range (`pdf/outline.rs:407`,
  `djvu/outline.rs:34`) or hardcodes level 1.
- **Inline nesting** capped at depth 3.
- **Block mix** weighted toward `Para`, with the rarer variants present.
- **Figures** come with a matching `AssetBag` entry so the renderer resolves a
  real filename instead of `"missing"`.
- **Cross-references** generated both ways: a `RefTarget::Internal` pointing at a
  `BlockId` that *is* a generated heading, and at one that is not — the latter
  exercising the dangling-ref strip path (`refs.rs:68`).

Shapes outside this domain but inside the type are covered by named unit tests
(§6) rather than by widening the generator, which would make every failure
require triage for "can an adapter actually produce this?".

### 4.4 Tripping the size guard cheaply

`Options` is generated alongside the document: `max_tokens` in 40..400,
`min_tokens` in 5..40, with `min < max` enforced by construction. A 30-block
document then routinely fires both the merge path and the split path, instead of
needing thousands of blocks to exceed a 2000-token default. Documents cap at ~40
blocks and strings at ~200 characters, which is what keeps the suite inside a
second or two.

## 5. The invariants

**P1 — Conservation.** Every sentinel appears across the rendered files with the
multiplicity in §4.2. Catches loss and duplication in one assertion, and stays
true regardless of how `balance()` rewrites a block, because it tracks content
rather than structure.

**P2 — Link resolution, end to end.** No `RefTarget::Internal` survives in any
`FileNode`. Every relative link in the rendered Markdown resolves, from its
containing file, to a path that is a real `FileNode`; where the link carries a
`#anchor`, that anchor matches a heading actually rendered in the target file.
This is the property §2.1's fix exists to satisfy, and the reason the tier
reaches into the writer at all.

**P3 — Size guard.** For every emitted file, estimated tokens ≤ `max_tokens`,
with two named escapes: a file holding a single block that alone exceeds the
budget (atomically unsplittable), and a container file whose synthesized TOC
pushed it over — bounded by the TOC's own weight, not unbounded. On the lower
side: no file below `min_tokens` unless it is a direct child of root
(`balance.rs:19` deliberately preserves top-level sections) or is a container.

**P4 — Navigation chain.** Following `next` from `index.md` visits every file
exactly once and terminates at the file whose `next` is `None`; `prev` is its
exact inverse. Both are stored relativized (`nav.rs:88-89`), so the property
resolves them back to tree paths through the same helper P2 uses — one
implementation, exercised twice.

**P5 — Path well-formedness.** All paths unique; none contains a `..` segment, a
leading `/`, or an empty segment; every `children` entry names a real file;
`parent` resolves to the real parent; breadcrumb depth is consistent with
position in the tree.

**P6 — Determinism.** `structure()` plus render, run twice over the same
`Document`, yields byte-identical output. Cheap, and it pins the fact that the
`HashMap<BlockId, String>` anchor map (`paths.rs:13`) never leaks iteration
order into output.

### 5.1 The two core test seams

P3 needs `est_tokens_blocks` (`balance.rs:70`) and P2 needs `slug`
(`paths.rs:61`). Both are `pub(crate)` in `kasane-core`. Re-implementing either
in the test would create a second source of truth that drifts silently — the
test would keep passing against its own arithmetic or its own slug rule while
the engine's changed underneath it.

`kasane-core` instead exposes both as `#[doc(hidden)] pub`: `est_tokens` and
`slug_of`. This is the convention `kasane-adapters` already uses for
`fuzz_entry`, for the same reason: a test needs an internal, and widening it to
ordinary `pub` would leak an implementation detail into the crate's real API.

P2 uses `slug_of` to state its anchor check honestly: the target file must render
a heading whose text slugifies, *by the engine's own rule*, to the anchor
`resolve_refs` emitted. Whether that rule matches a given Markdown renderer's
anchor derivation (GitHub's differs on punctuation) is a separate fidelity
question and is not in scope here.

### 5.2 Where P3 is most likely to fire first

Stated up front rather than discovered mid-implementation:

- `split_blocks` (`balance.rs:51`) fills parts greedily, so the final part can be
  a runt below `min_tokens`.
- Container files receive their TOC in `nav::walk` *after* balancing sized them,
  so an `index.md` can exceed `max_tokens` purely from its own TOC.

P3 is written to state the intended invariant, with the escapes above named
explicitly. If either case fires beyond its escape, it is a defect and is fixed
in-branch under the standing rule.

## 6. Typed-edge unit tests

Ordinary `#[test]`s, not properties: the shapes are specific and known, and
generating them would widen the generator's domain past what adapters produce.

1. **Heading level 255.** `balance.rs:41` computes `node.level + 1` when splitting
   an oversized leaf, which overflow-panics in debug. Unreachable through today's
   adapters, reachable through the public `structure()` API. Fixed with a
   saturating increment.
2. **Deep inline nesting.** The §2.2 shape, asserting the depth-bounded walks
   return instead of aborting. Placed on both sides of the fix: a core/writer
   test for the bounded recursion, and an EPUB-level test that a book with
   deeply nested `<em>` converts.

## 7. CI and reproducers

One budget: proptest's default 256 cases per property, with document size capped
by the generator rather than by a runtime knob. There is no `PROPTEST_CASES`
environment variable to set, and therefore none to drift between a laptop and
CI.

A failing property writes `proptest-regressions/properties.txt`. **That file gets
committed**, exactly as `fuzz/artifacts/` reproducers are, and for the same
reason: it is what turns a bug the search found into a permanent regression test
that runs on every subsequent PR.

No new workflow. No new pinned tool. `proptest` is an ordinary dev-dependency,
visible to Dependabot through `Cargo.toml` — unlike the toolchain and
`cargo-fuzz` pins in `mise.toml`, which are manual bumps.

## 8. Documentation

- **AGENTS.md** — the codebase map gains the property tier under
  `crates/kasane-writer`, and the `est_tokens` seam is noted beside the existing
  `fuzz_entry` seam so the two read as one convention.
- **README** — the testing section gains a line naming the tier and what it
  asserts; the fuzzing section's "commit the reproducer" paragraph gains the
  `proptest-regressions` analogue.
- **README known-limitations** — the §2.1 fix changes visible output (files now
  open with a title heading); any limitation text that implied otherwise is
  corrected.

## 9. Approaches considered

**Generator: `proptest-derive` on the IR types.** Derive `Arbitrary` behind a
feature on `kasane-ir`. Far less code, but it pulls a proc-macro into the one
crate that today depends on nothing internal or external, and uniform variant
selection produces shapeless documents that rarely trip the size guard. It also
fights the adapter-realistic decision: you would be constraining a derive after
the fact. Rejected.

**Generator: a shared `kasane-testgen` workspace crate.** Reusable by future
writer and adapter round-trip properties. Correct with three consumers; there is
one. It adds a workspace member that must carry `publish = false` and stay out of
the release for no benefit today. Rejected on YAGNI.

**Scope: `kasane-core` only.** Smallest blast radius and fastest cases, and it
matches §9's wording literally. Rejected because it cannot check §9's actual
link invariant — and because it would not have found §2.1, which is the single
highest-value thing this item does.

**Input domain: full type space.** Generate anything the IR permits, fuzz-style.
Would find the level-255 overflow automatically. Rejected because every failure
then needs triage for adapter reachability, and shrinking through a wide space is
slow; the two known type-space edges are covered deliberately by §6 instead.

**Anchor fix: drop the anchor for whole-file targets.** Have `resolve_refs` emit
a bare path when the target section *is* the file. Minimal and changes no output
shape. Rejected: merged subsections would still resolve to a dead `#slug` on the
parent, and it leaves files rendering with no visible title.

**CI: a weekly deep sweep.** Mirror `fuzz.yml` with a high-case-count job.
Rejected for now — one more workflow and one more knob to keep in sync, for a
suite whose search space is deliberately narrow. Revisit if the tier finds
nothing for several months, which would suggest 256 cases is too few.

## 10. Verification

`mise run lint && mise run test` green, which now includes the property suite,
the typed-edge tests, and every existing end-to-end assertion updated for the
§2.1 output change.

The two defects are verified against their reproducers specifically:

- §2.1 — converting `tests/fixtures/epub/rich.epub` produces section files that
  open with a title heading, and an EPUB carrying an in-book cross-reference
  resolves it to an anchor that exists in the target file.
- §2.2 — the 10,000-deep nesting case returns instead of aborting, from both the
  core/writer entry and an EPUB carrying the shape; the committed fuzz seed
  replays green in `tests/fuzz_corpus.rs`.
