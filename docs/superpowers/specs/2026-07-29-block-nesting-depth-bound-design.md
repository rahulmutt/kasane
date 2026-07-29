# kasane — Block-Nesting Depth Bound Design Spec

**Date:** 2026-07-29
**Status:** Approved (design), pending implementation plan
**Repo:** kasane

## 1. Purpose & scope

README's Known limitations records one open safety item: block nesting
(`Block::List` / `Block::Footnote`) is bounded nowhere, so a document that
nests lists or footnotes deeply enough aborts the process with `fatal runtime
error: stack overflow` instead of failing recoverably. This item closes it.

The hazard is the last unbounded recursion at the untrusted-input boundary.
Inline nesting was bounded by the property-tier item
(`2026-07-29-core-property-tier-design.md` §2.2) with a two-constant design —
`epub::xhtml::MAX_INLINE_DEPTH` (64, fidelity) under `kasane_ir::MAX_INLINE_DEPTH`
(256, safety). This item applies the same shape to blocks.

### Confirmed, not assumed

Reproduced against `88f117c` with a 541 KB EPUB holding a 30,000-deep `<ul>`,
stored 1:1 (no decompression bomb, so none of the bomb guards apply). Depth
bisected in a debug build:

| Mode | Survives | Aborts |
|---|---|---|
| single-file (`main`, 8 MiB stack) | 2,000 | 4,000 |
| batch (rayon worker) | 500 | 1,000 |

Batch mode aborts roughly 4x shallower, because a rayon worker gets a smaller
stack than `main`. **Any bound must be sized against the rayon worker, not
`main`** — §4.

The producer side is not the problem. The XHTML parser's `frames` is an
explicit `Vec`, so it builds the 30,000-deep `Block::List` without recursing
and hands it downstream intact. Every abort is in a consumer.

### Scope

- `crates/kasane-adapters/src/epub/xhtml.rs` — the fidelity bound (§2).
- `crates/kasane-ir/src/lib.rs` — the safety constant (§3).
- The seven recursive block walks across three crates (§3).
- `crates/kasane-adapters/src/fuzz_entry.rs` — a block-depth assertion, plus
  one new committed seed (§5).
- `crates/kasane-writer/tests/generator/` — nested-list generation (§5).
- README, AGENTS.md, `nav.rs`'s comment, `fuzz_entry`'s comment (§6).

### Non-goals

- **Rewriting the walks to be iterative.** Considered and rejected — §7.
- **Raising rayon's worker `stack_size`.** Moves the cliff without removing it
  and does nothing for library callers of `structure()` — §7.
- **Re-tuning MOBI's `normalize.rs` `MAX_DEPTH`.** It stays at 500; §2.3
  explains why it is no longer the binding constraint and what its comment must
  say.
- **Bounding anything other than block depth.** Inline depth, island nesting,
  zone depth and outline depth are each already bounded by their own constant.

## 2. The fidelity bound

### 2.1 Where it lives

`epub::xhtml::MAX_BLOCK_DEPTH`, applied in the streaming parser's `frames`
handling — the same file and the same layer as `MAX_INLINE_DEPTH`.

### 2.2 Flatten, do not truncate

The depth compared against the bound is `frames.len()` — every `BlockFrame`
kind, not only the two that can nest. That is deliberately conservative:
`Table` and `Figure` frames cannot produce nested `Block`s, but neither can
they appear in a long chain, so counting them costs nothing real and keeps the
check a single length comparison rather than a filtered count that has to be
kept in sync with the enum.

When a frame-pushing start tag arrives while `frames.len()` is already at
`MAX_BLOCK_DEPTH`, no new frame is pushed. The tag's content is contributed to
the enclosing item as sibling blocks instead. In practice the tags that reach
this are `<ul>`, `<ol>` and the footnote container (`<aside>` carrying
`epub:type="footnote"`), since those are the two frame kinds that can chain.

**Nothing is dropped.** Text past the bound survives; only the nesting
structure collapses to the bound's level. This is the exact analogue of
`wrap_inline`'s "the content is contributed as flat text", and it is what
distinguishes this bound from MOBI's `normalize.rs` `MAX_DEPTH`, which drops.

Because a suppressed open pushed no frame, the matching end tag must not pop
one. The parser therefore carries a counter of suppressed opens and decrements
it on the matching close before considering a real pop. Getting this wrong
would unbalance `frames` and corrupt every block after the deep list, not just
the deep list — so it is asserted directly (§5).

### 2.3 One site covers three formats

MOBI/AZW3 re-serializes through this same parser (`normalize_html` produces
XHTML for `xhtml_to_blocks`), so it inherits the bound with no MOBI-side
change.

This matters more than it sounds. MOBI is already near the cliff today:
`normalize.rs`'s `MAX_DEPTH = 500` caps tag depth, and batch mode aborts
somewhere in (500, 1000]. That 500 was chosen empirically against `serialize`'s
own mutual recursion, not against anything downstream of it, and its comment
does not claim otherwise. After this item the parser bound is far tighter, so
`MAX_DEPTH` stops being the value that decides whether the process survives.
Its comment gains a sentence saying so; its value does not change.

PPTX needs no change: `slide.rs`'s `build_list` recurses, but on `level`, which
is a `u8` — an existing test asserts `lvl` values above 255 fail to parse, so
that recursion is already capped at 256. PDF and DjVu never nest blocks.

## 3. The safety bound

`kasane_ir::MAX_BLOCK_DEPTH`, a safety bound for hand-built `Document`s reaching
the published `structure()` from an external caller. Adapter-produced IR can
never reach it, because §2's fidelity bound is strictly lower — an ordering
invariant stated in both constants' doc comments, exactly as the inline pair
states it.

`depth: usize` is threaded through all seven recursive block walks, the same
shape as `clone_inlines_at` / `fix_inlines_at` / `inl_at`:

| Walk | Crate | Behaviour at the bound |
|---|---|---|
| `section::clone_block` | core | emit `Block::Raw { note: "nesting truncated at the block depth bound" }` instead of descending |
| `balance::est_tokens_block` | core | base cost, do not descend |
| `refs::fix_block` | core | return |
| `markdown::render_block` | writer | emit the note text |
| `epub::fix_block_links` | adapters | return |
| `mobi::strip_empty_anchor_links` | adapters | return |
| `mobi::any_empty_anchor_link_in_blocks` | adapters | return |

`clone_block` is the first core walk to touch the IR, so truncation happens
exactly once, and every later core and writer walk sees already-shallow blocks.
Their bounds are defence in depth, not the load-bearing check. Stating which
walk is load-bearing is the point: a reader must not conclude from the table
that four independent truncations can stack.

The three adapter-side walks run on parser output that §2 already keeps
shallow, so they are unreachable in practice. They get the bound anyway, so the
invariant does not depend on which adapter fed them.

### 3.1 The inventory AGENTS.md gives is incomplete

AGENTS.md names four walks (`section::clone_block`, `balance::est_tokens_block`,
`refs::fix_block`, `kasane_writer::blocks_to_markdown`). There are seven. The
three it omits are the adapter-side ones above — and `epub::fix_block_links`
runs on **every** EPUB and MOBI parse, before `structure()` is ever called. A
core-only bound would therefore not have fixed the reported reproducer. §6
corrects the list.

## 4. Choosing the values

Measured, not guessed — the discipline `MAX_INLINE_DEPTH`'s comment already
records, and the reason its value is defensible.

The measurement runs on the **tightest stack the code actually uses**. That is
a rayon worker in batch mode, not `main`; §1's table is the evidence. The
end-to-end figures there bracket all seven walks together; the implementation
measures the binding walk on its own and pins the figure in the constant's doc
comment, naming the thread it was measured on.

Candidates to confirm by measurement: **safety 128, fidelity 32**. 32 is far
beyond any real book — list nesting rarely exceeds a handful of levels — and
leaves the safety bound 4x headroom.

These are candidates, not TBDs: the two constraints that decide them are fixed
here, and the measurement only picks values satisfying both. First, safety is
at most a quarter of the measured abort depth on the tightest thread, the same
4x margin `MAX_INLINE_DEPTH` took. Second, fidelity is at most a quarter of
safety, so adapter IR cannot approach the safety bound. If the measurement
comes in low enough that 128 violates the first constraint, safety drops to fit
it and fidelity follows; the candidates move, the constraints do not.

One interaction the measurement must include: a block walk at depth *d* can
call an inline walk up to `MAX_INLINE_DEPTH` deep, so the two budgets compose.
Measuring block depth against flat inline content would overstate the safe
value.

## 5. Testing

- **`xhtml.rs` unit test**, mirroring the two existing inline-depth tests: a
  3,000-deep `<ul>` parses to block depth at most `MAX_BLOCK_DEPTH` **and** the
  innermost text is still present. Both halves are required — the second is the
  claim that separates flattening from truncation, and without it the test
  passes against a bound that silently drops content.
- **Frame-balance test** for §2.2's suppressed-open counter: a document with a
  deep list *followed by* ordinary blocks parses those trailing blocks
  correctly.
- **`fuzz_entry::adapter`** gains `max_block_depth(&doc)` beside
  `max_inline_depth`, and its assertion. Both traversals are iterative, for the
  reason `max_inline_depth`'s comment already gives: a recursive checker can
  overflow its own stack before returning a value to check, which reads as a
  crash in the test code rather than in the code under test.
- **A committed seed**, `fuzz/seeds/epub/deep-blocks.epub` — the 30,000-deep
  `<ul>` reproducer from §1 — so `tests/fuzz_corpus.rs` replays it on stable
  without a nightly toolchain. `KNOWN_OPEN` stays empty: the bug is fixed in
  the same branch, so the seed is armed immediately. Seeds in this repo are
  committed binaries with no generator script (`deep-nesting.epub` is the
  precedent), so the recipe is recorded here instead: a minimal EPUB 3 whose
  single spine item is `<h1>Deep</h1>` followed by `<ul><li>` x 30,000, one
  text character, then the matching closes, stored uncompressed.
- **Core and writer unit tests** with hand-built deep `Document`s, in the style
  of `teardown_document_survives_deep_block_and_inline_nesting`: `structure()`
  and `file_to_markdown` return normally at depth 100,000 on a libtest thread.
- **CLI end-to-end in batch mode** — the tight stack, per §1 — converting the
  deep EPUB and asserting exit 0. Single-file mode would pass against a bound
  4x too loose.
- **Property tier**: extend `crates/kasane-writer/tests/generator/` to draw
  block nesting up to the fidelity bound. It draws shallow nesting today, so
  conservation, the size guard and link resolution do not currently cover
  nested lists at all. Any `properties.proptest-regressions` file this produces
  is committed.

## 6. Documentation

Four places assert the current state and must change together, or the docs
contradict the code:

- **README**, Known limitations: the "Block nesting has no depth bound" entry,
  rewritten to state the new behaviour — deep lists flatten past the bound,
  content is kept, structure past it is not.
- **AGENTS.md**: the "bounded **nowhere**" paragraph, and its walk list, which
  is wrong today (§3.1).
- **`crates/kasane-core/src/nav.rs`**: the comment block that spells out at
  length that block nesting is unbounded everywhere.
- **`crates/kasane-adapters/src/fuzz_entry.rs`**: `max_inline_depth`'s doc
  comment, which states the same thing and explains that the function
  deliberately does not check block depth.

Plus the sentence on `normalize.rs`'s `MAX_DEPTH` from §2.3.

## 7. Approaches considered

**Rewrite the seven walks iteratively, with no bound at all** (the shape
`kasane_ir::teardown_document` already uses on the drop side). No constant, no
fidelity/safety split, no depth at which anything is lost. Rejected on
reviewability: `render_block` interleaves strings while descending — list items
and footnote bodies are rendered into nested buffers — so it needs an
output-fragment stack, and `clone_block` needs a build-up stack.
`teardown_document` is the easy case precisely because it only frees. The
chosen design reuses a pattern already written, reviewed and documented in this
repo, and keeps the block and inline stories symmetric in the docs.

**Hybrid — iterative where mechanical, bounded where awkward.** Rejected: two
idioms for one hazard means the invariant cannot be stated in one sentence, and
a reader has to check per-walk which rule applies.

**Adapter-side flattening only.** Smallest change that kills the reported
reproducer. Rejected: `structure()` is published API, so an external caller
with a deep hand-built `Document` still aborts the process, and the core and
writer walks stay a live hazard the docs must keep disclosing.

**Raise rayon's worker `stack_size`.** Moves the cliff without removing it, and
does nothing for library callers.

## 8. Verification

- `mise run lint && mise run test` green.
- The §1 reproducer converts successfully in **batch** mode (exit 0), not just
  single-file mode.
- `cargo test -p kasane-adapters --test fuzz_corpus` replays the new
  `deep-blocks.epub` seed, and `KNOWN_OPEN` is still empty.
- Every constant's doc comment names the measured figure and the thread it was
  measured on.
- No document in the repo still claims block nesting is unbounded.
