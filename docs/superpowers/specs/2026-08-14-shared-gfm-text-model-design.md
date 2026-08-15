# kasane — Shared GFM Text Model Design Spec

**Date:** 2026-08-14
**Status:** Implemented. The external oracle (§6) re-ran against a real
github.com render on 2026-08-14: 13 of 14 ids identical, codepoints included;
the only divergence the probe's cases hit is the pre-existing, intentional
`EMPTY_FALLBACK` case (Non-goals, below). A second, pre-existing divergence —
a heading's empty inline code span rendering as a padding space that
`rendered_text` does not model, so the embedded anchor is a dead
cross-reference — was not among the probe's cases and survives too; unlike
`EMPTY_FALLBACK` it is a real construction defect, not a choice (see
`kasane_gfm::slug`'s module doc). Neither is a defect this item introduced.
The empty-code-span divergence was closed on 2026-08-14 by its own item,
`2026-08-14-empty-code-span-anchor-design.md`, for a heading carrying a
*single* empty span — that item's own Status block records the adjacent-span
shape it did not close, and left divergent. `EMPTY_FALLBACK` remains, by
choice. **The adjacent-span shape closed 2026-08-15** by
`2026-08-15-adjacent-inline-fusion-design.md`,
in the writer rather than in either canonicalization: a run of adjacent
same-delimiter inlines now renders as one span, so the adjacent-span shape no
longer diverges either, and `EMPTY_FALLBACK` is the only divergence this crate
still records.
See §8.1/§8.3 of `2026-08-08-slug-widening-design.md` for the method and the
table.
**Repo:** kasane

## 1. Purpose & scope

The escaping residuals item (2026-08-13, PR #36) recorded approach (iii) in its
§7 as "the principled end state … it deserves its own item". This is that item.

Two rules in two crates predict the same thing — what a heading line renders to
— and are kept in step by hand:

| | Where | What it does |
|---|---|---|
| `escape::one_line` | `kasane-writer` | Folds every newline spelling in a rendered heading line to one space, collapsing runs. |
| `slug::fold_newlines` | `kasane-core` | The same fold, applied to `inline_text` before `anchor_slug` computes the fragment. |

`kasane-core` cannot depend on `kasane-writer`, so there is no shared function,
and AGENTS.md carries the standing warning that a change to one fold that is not
mirrored in the other reopens the anchor mismatch the pairing closed.

The mirror is the visible half of the problem. The invisible half is that
*whatever* predicts the rendered line lives in the structuring engine, where
nothing forces it to agree with the writer, and two divergences are documented
today as surviving on purpose because of it:

| | Shape | Today | GitHub |
|---|---|---|---|
| **A** | `## Notes[^1]` | `notes` — `inline_text` skips `Inline::FootnoteRef` | `notes1` — the reference is visible text |
| **B** | `## Intro ###` | `intro-` — the IR text is slugged whole | `intro` — the trailing run is an ATX *closing sequence* and is stripped |

A also has a second face, recorded as open in the escaping spec's §4.5: a
newline run split across a `FootnoteRef` cannot collapse, because the reference
is text the fold cannot see.

### Scope

1. A new leaf crate, `kasane-gfm`, owning the newline fold, the rendered-text
   projection, and both slug rules. One definition, two consumers.
2. **A** closed by the projection: `Inline::FootnoteRef(n)` contributes `[^n]`.
3. **B** closed in the *writer*, by escaping the closing sequence, not in the
   anchor rule.
4. The property that pins the two against each other extended to cover both
   shapes, and `parse_events`' blind spot for footnote references closed first.

### Confirmed, not assumed

- **`Event::FootnoteReference` falls into `parse_events`' `_ => {}` arm**
  (`properties.rs:160`). The parsed side of the §5.2 property therefore skips a
  reference exactly as `inline_text` does, which is why A has been invisible to
  the tier since it was written. The blind spot is symmetric with the defect,
  not independent of it.
- **`#` is escaped only at a line start** (`escape.rs`'s `LINE_START` handling),
  and both heading paths render their content at `Pos::Mid`. Nothing today
  touches a trailing `#`, so there is no existing escape to untangle.
- **`kasane-writer` already depends on `kasane-core`** (`FileNode`, `SiteTree`).
  The dependency edge that would make a core-hosted fold work already exists;
  §7 explains why a new crate is still the right home.
- **`Frontmatter::title` is a `String`**, flattened by `nav::walk` through
  `inline_text` (`nav.rs:70-74`), and `file_to_markdown` prints it verbatim.
  `place` anchors the *inlines* of the same node. Those two agree today only
  because both drop a footnote reference; §3 is where that stops being luck.
- **Anchors are computed at structuring time** (`assign_paths`), before anything
  renders. Every mechanism here is a pure function of IR inlines, so nothing in
  this item disturbs that ordering.

### Non-goals

- **Whitespace inside the merged-table HTML fallback** (escaping spec §3.4).
  Unreachable by escaping and unrelated to the mirror; it stays open.
- **The empty-id divergence.** `EMPTY_FALLBACK` is a choice, not a construction
  defect: an empty fragment is a dead link. It stays.
- **Math in a heading.** GitHub renders `$…$` through MathJax and what it does
  to the id is unmodelled today. Unchanged here, and called out in §6.
- **Moving the escaping rules.** `escape.rs` stays in the writer. Only the fold
  crosses the boundary, because only the fold has a second consumer.

## 2. The crate

`crates/kasane-gfm`, a leaf depending on `kasane-ir` and nothing else. Its
charter is one sentence: *what GFM does to heading text, and the two slug rules
that follow from it.*

Moved wholesale out of `kasane-core/src/slug.rs`:

- `is_word` / `is_join_control`, with the derivation comment for Ruby's
  `\p{Word}` — the `char::is_alphabetic()` term, the `Nd`-not-`No` term, and the
  Mark term that is the only reason `unicode-properties` is a dependency at all
- `fold_newlines`, which `kasane-writer::escape::one_line` now *is* rather than
  mirrors
- `inline_text`, renamed `title_text` (§3), and the new `rendered_text`
- `anchor_fold`, `path_fold`, `anchor_slug`, `path_slug`, `AnchorCounter`,
  `MAX_PATH_SLUG_BYTES`, `EMPTY_FALLBACK`, `truncate_to`, `trim_tail`
- the module doc: the four-axis divergence table, the do-not-add-NFC argument,
  and the GitHub-mirror drift warning, all intact
- `unicode-normalization` and `unicode-properties`, comments included;
  `kasane-core`'s manifest drops both

Both slug rules move together. They share `is_word`, and one module doc explains
the four axes on which they deliberately differ; splitting them across crates
would duplicate the predicate and scatter the argument that makes the
duplication legible.

`kasane-core` takes `kasane-gfm` as a dependency (`paths`, `nav`, `refs`,
`balance` all call in) and keeps `est_tokens`. It does **not** re-export the
slug seams. `kasane-writer` depends on `kasane-gfm` directly for the fold, so
`properties.rs` importing `anchor_slug_of` and `anchors_for_headings` from
`kasane_gfm` and `est_tokens` from `kasane_core` says where each rule lives; a
pass-through re-export would be one more thing to drift.

Fuzz churn is imports only, deliberately: the `slug` seam moves to
`kasane-gfm::fuzz_entry`, and `fuzz/fuzz_targets/slug.rs` and
`kasane-adapters/tests/fuzz_corpus.rs` swap `kasane_core` for `kasane_gfm`. The
target name, `fuzz/seeds/slug/`, `fuzz/artifacts/`, `KNOWN_OPEN` and README's
"thirteen targets" are all untouched.

A release now bumps **six** lines rather than five. `Cargo.toml`'s comment says
five and is corrected here.

## 3. The rendered-text projection

```rust
/// The text the writer's rendering of this inline run renders back to.
pub fn rendered_text(inlines: &[Inline]) -> String
```

Identical to today's `inline_text` in every arm but one:
`Inline::FootnoteRef(n)` contributes `[^n]` instead of nothing. That single arm
is the whole of **A**.

It is correct whether or not the reference resolves. GitHub renders a resolved
reference as a superscript `1` and leaves an unresolved one as the literal text
`[^1]`; its id filter removes `[`, `^` and `]`, so both land on `notes1`. The
projection does not have to know which happened — which matters, because after a
`balance` split the definition may be in a different file from the reference.

### 3.1 `title_text` keeps skipping references

`title_text` is today's `inline_text`, renamed and otherwise unchanged. It is
not a legacy path to converge on `rendered_text` later.

The string it produces becomes `Frontmatter::title`, every `breadcrumb` entry,
every TOC link label in `nav::walk`, and the library index row. A `[^1]` in any
of those renders a footnote reference in a navigation surface, pointing at a
definition that likely lives in another file — a dangling marker in `index.md`
is worse than an absent one in a title. `refs`' stripped-link flattening and
`balance`'s title comparisons want the same behaviour, unchanged.

### 3.2 The anchor's input becomes the line it will print

The two heading paths print different things, so they project differently:

| Path | Prints | Anchor input |
|---|---|---|
| `count_headings` (body heading) | the writer's rendering of the inlines, `[^n]` included | `rendered_text(inlines)` |
| `place` (file title heading) | `Frontmatter::title`, verbatim | the anchor rule over that same title string |
| `place`, root | `doc_title` | `doc_title`, as today |

`AnchorCounter::next` takes `&str` — the projected line text — instead of
`&[Inline]`. That is the enforcement rather than a convenience: a caller cannot
hand it an inline run and receive an anchor for a line it is not going to print.
`anchor_slug_of` keeps its inline-taking shape for the property tier by
composing the projection with the rule.

The rule's own `anchor_fold` still runs on whatever string it is given, so the
newline fold, the trim and the lowercase apply to a projected title exactly as
they do to a projected body heading — the projection concatenates text and does
nothing else. `anchors_for_headings` loses its `Inline::Text` re-wrapping and
applies the rule to its strings directly, which is what it always meant.

Note what this does **not** touch. `inline_text`'s behaviour is unchanged, so
`nav`, `refs` and `balance` see nothing. The ripple the slug module doc warned
about — "closing either means changing `inline_text`, whose other callers want
exactly its current behaviour" — is avoided by narrowing `place`'s input rather
than widening the flattening. That is why A is a small diff and not the
architectural excavation the doc feared.

## 4. The writer

### 4.1 One fold

`escape::one_line` is deleted. Its three callers — `Block::Heading`,
`file_to_markdown`'s title line, and `code_span` — call
`kasane_gfm::fold_newlines`. The collapse behaviour that lets one fold serve all
three (escaping spec §4.1) is unchanged; it is now unchanged *in one place*.

`fold_inline_newlines` stays in the writer and stays as it is. It handles a
newline run spanning an inline boundary, which is stateful and genuinely
writer-local. What changes is its doc comment: the paragraph naming the
footnote-reference residual goes, because treating a `FootnoteRef` as opaque is
now correct rather than a limitation. The reference is visible text between two
real separators; the fold was always right, and the projection now agrees with
it. Escaping spec §4.5's second open case closes here, with no fold change —
exactly as that section predicted.

### 4.2 The closing sequence

A new rule in `escape.rs`, applied by both heading paths at the same point:
after the content is escaped and folded, before the newline is pushed. If the
content ends in a run of `#` preceded by a space or tab, or consists entirely of
`#`, the first `#` of that run is escaped.

The mechanism turns on an ordering worth writing down. CommonMark strips the
closing sequence at *block* level, from raw text, before inline parsing runs. In
`## Intro \###` the trailing run is preceded by `\` rather than a space, so the
block-level scan does not strip it; inline parsing then turns `\#` back into a
literal `#`, and the line renders `Intro ###`. `## Intro###` needs nothing — a
run with no space before it was never a closing sequence. `## \###` covers the
all-`#` content case, whose text then slugs to nothing and takes
`EMPTY_FALLBACK`, which is the documented empty-id choice and not a new
divergence.

### 4.3 Why the fix is here and not in the anchor rule

Teaching `anchor_slug` about closing sequences would make it agree with GitHub
by conceding that the rendered heading may silently drop document text — the
`###` the IR holds would simply not appear for a reader. That is a §5 violation
of the escaping spec ("escaping must never change what the Markdown renders
to"), presenting as a parity fix. Escaping the run instead restores the
invariant, and today's `intro-` becomes *correct* rather than replaced.

The direction generalizes: where the writer can make the rendered text equal the
IR text, that is the fix, and the projection stays a description of the IR. The
projection grows an arm only where the writer legitimately emits visible text
the IR does not spell — which, after this item, is `FootnoteRef` alone.

### 4.4 What the writer does not import

`kasane-writer` never calls `rendered_text`. Its obligation is the converse of
the projection's claim: what it emits must render back to what the projection
predicts. That is a claim to be *checked*, not shared — a shared function
between the two would only assert the agreement by construction while the actual
output drifted underneath it. §5 is the check.

## 5. Testing

### 5.1 `parse_events`' blind spot closes first

`parse_events` sends `Event::FootnoteReference` to `_ => {}`, so the parsed
heading text skips a reference exactly as `inline_text` does, and the two sides
of the §5.2 property agree on a wrong answer. The new arm pushes the
reference's label, modelling GitHub's superscript digit.

This lands **first**, before the projection changes: against today's code it
fails, against the fix it passes. Any other ordering ships a test that cannot
distinguish the two.

One honest limit of the proxy, recorded rather than smoothed over:
`pulldown-cmark` numbers a reference by its label, GitHub by definition order.
kasane emits numeric labels, so the two coincide for its output, and a reference
whose definition landed in another file renders literally, where the filter
yields the same digits anyway. A pathological renumbering remains possible in
principle. This is a known limit of testing against a proxy renderer, and one
more reason §6's external check is the real oracle.

### 5.2 The existing property covers both shapes

The residuals spec's §5.2 property — P9 — already renders a `Block::Heading`
and asserts `anchor_slug_of(inlines)` equals the id computed from parsing the
emitted line. That comparison is the check this item turns on, and it gains two
siblings built the same way:

- **P10**, an `Inline::FootnoteRef` between two text runs, drawn both with its
  definition in the same file and without it, covering **A** and the newline run
  that cannot collapse across a reference
- **P11**, a heading ending in a `#` run, asserting first that the rendered line
  still carries the run and then that the ids agree, covering **B**

Siblings rather than arms of P9: its shape is a newline run split across an
inline boundary, `[Text("a\n"), second("\nb")]`, and neither new case is a
`second` — one is an opaque inline carrying no text, the other is a property of
the line's tail. What matters is that all three make the same comparison, and
that comparison is what replaces AGENTS.md's hand-kept mirror warning with a
machine check.

### 5.3 Generator and the wider tier

`generator/mod.rs`'s `HOSTILE` gains one fragment, `"tail ###"`, so P1, P2 and
P7 draw the closing-sequence shape into every block kind rather than only the
heading P11 builds. `is_comment`'s doc comment says `HOSTILE` has 25 fragments;
the count is corrected in the same change that makes it wrong again, since the
claim it supports — that only `-->` triggers `comment_note`'s transformation —
stays true of the new fragment and stays worth checking.

The footnote shape is deliberately *not* added to the generator. Reaching it
means giving the generator `Inline::FootnoteRef` decorations, which changes
what P1's sentinel accounting has to model, and it would buy a shape P10
already hits on every run.

### 5.4 Unit level

`anchor_matches_github`'s case table moves with the module and gains the
footnote-reference, `Intro ###` and `###`-only cases. `escape.rs` gains
parser-verified tests for the closing-sequence escape on both heading paths, in
the style of `the_title_heading_renders_to_exactly_the_trimmed_title`.
`paths.rs`'s unit tests gain a title-heading case with a reference in the node
title, pinning §3.2's table: the anchor follows the printed title, not the
inlines.

### 5.5 Fuzzing

Imports move; the `slug` seam's postconditions are unchanged. The `escape` seam
gains one: a rendered heading line never ends in an unescaped closing sequence.

## 6. The external oracle

`slug.rs`'s case table pins kasane's *reading* of GitHub's algorithm, not the
algorithm — three corrections to that reading came out of review and the table
agreed with the code every time. Only an external render can catch a
misreading, and this item edits the reading twice.

The check is design spec §8.1's, last run 2026-08-09 (13/13 ids matching). It is
re-run here over the existing 13 cases plus `## Notes[^1]`, `## Intro ###` and
`## ###`, with the footnote case rendered both with and without its definition
in the same document, since §3's argument is that both spellings yield the same
id. The implementation attempts the render from the sandbox; if the network path
is unavailable, it produces the probe document and kasane's predicted ids for a
manual run. Which of the two happened, and the result, is recorded in README and
in this spec's status before the item is called done.

Math in a heading is *not* added to the probe. It is unmodelled today and stays
that way; widening the oracle to a case the projection makes no claim about
would invite reading its result as a guarantee.

## 7. Approaches considered

**(i) A new leaf crate. Chosen.** §2.

**(ii) Host the shared rules in `kasane-core`.** `kasane-writer` already depends
on `kasane-core`, so the mirror ends with no new crate and a much smaller diff:
`escape::one_line` becomes a call into core.

Rejected on the boundary story rather than on cost. AGENTS.md describes
`kasane-core` as the pure structuring engine — fold, balance, paths, refs, nav —
and this would make it the home of a Markdown-rendering vocabulary that the
writer imports back out. The next rendering rule with two consumers would then
have no principled home either, and the map would have to explain why the
renderer's fold lives in the structurer. Cheaper today, muddier at every
subsequent decision. The projection is genuinely a third thing: what GFM does to
text is not structuring and not writing, and both must obey it.

**(iii) Compute anchors from the real rendered lines.** No model at all: render
first, parse the emitted heading lines, compute ids from what a parser actually
sees.

Rejected. It puts a Markdown parser on the *production* path — `pulldown-cmark`
is a dev-dependency today, and the property tier's independence from the library
is what makes P7's round trip meaningful. `assign_paths` needs anchors before
anything renders, so it also forces a two-phase render. And it does not remove
the model: it relocates it into "kasane's parser agrees with GitHub's", which is
the same mirror with a heavier dependency and a worse failure mode.

**(iv) Teach `anchor_slug` about ATX closing sequences.** Rejected in §4.3: it
buys parity by conceding content loss.

## 8. Documentation

**AGENTS.md** gains a `crates/kasane-gfm` entry, and the `kasane-core` passage
about the two hand-kept folds is *cut* rather than softened. The hazard it warns
about no longer exists, and a warning about a mirror that is gone sends the next
reader looking for code that is not there. One sentence replaces it: where the
rules live, that the writer must obey them, and that the §5.2 property is what
checks it. The `kasane-writer` entry loses `one_line` and gains the
closing-sequence rule.

**The module doc** moves with the code. Its "Known divergences that survive on
purpose" section drops the footnote-reference and trailing-`#` entries and keeps
`EMPTY_FALLBACK`; the four-axis table and the NFC argument survive verbatim.

**README**'s "Heading anchors match GitHub's rule, with three exceptions"
becomes two exceptions, not one: implementation surfaced a second,
pre-existing divergence this design did not know about (an empty inline code
span in a heading), so `EMPTY_FALLBACK` does not end up alone. The
empty-code-span divergence was closed on 2026-08-14 by its own item,
`2026-08-14-empty-code-span-anchor-design.md`, for the single-span case only;
README still reads "two exceptions", the second now being the adjacent-span
shape that item left divergent. `EMPTY_FALLBACK` remains, by
choice. The two removed bullets are not simply deleted — a reader
converting a tree that an older build produced needs to know those anchors
changed, so each gets a sentence saying what it was and that it is now correct.
The case-table path changes to `crates/kasane-gfm/src/…`, and the oracle line
records whatever §6 produced.

**The two older specs get pointers, not rewrites.** They are records of a
decision at their date. The exception is the escaping spec's §4.5 "Two cases
remain open", which is a live status claim rather than a record: it becomes one
open case, naming this item for the other, and §8's approach (iii) is marked
taken.

**The design spec** gains the crate in §10 and §11. §11 also needs a correction
it has been owed for a while: it says `kasane-core` and `kasane-writer` depend
on `kasane-ir` "and nothing on each other", which has been untrue since the
writer began taking `FileNode` and `SiteTree`.

**`Cargo.toml`**'s release comment: five lines becomes six.

## 9. Verification and risk

**Behaviour changes that reach existing output.** Two, both turning a dead
anchor into a live one:

1. A heading carrying a footnote reference anchors `notes1` where it anchored
   `notes`. Every in-book cross-reference to it changes with it, in step.
2. A heading ending in a space-preceded run of `#` now renders that run — the
   Markdown source gains a backslash — and keeps its current `intro-` anchor,
   which becomes correct because the rendered text no longer drops the run.

No path changes: `path_slug` is untouched and takes IR text upstream of all of
this.

**Risks.**

- **The projection is still a prediction.** It is now a single, tested one, but
  `rendered_text` claiming what GitHub renders remains a mirror in the same
  sense `anchor_slug` is. §5.2 pins it against `pulldown-cmark`, §6 against a
  real render; neither is github.com in CI, and nothing can be.
- **Footnote numbering.** §5.1's proxy caveat is the residual: if GitHub ever
  renders a reference's number differently from its label, the id follows
  GitHub's number and the projection is wrong for that heading. The literal and
  resolved spellings coinciding is what keeps this narrow.
- **A sixth published crate.** One more version to bump in lockstep, and
  `Cargo.toml`'s comment is the only thing enforcing that. The comment is
  updated; nothing else changes, and no CI or `deny.toml` file enumerates
  crates.
- **The move is large and mechanical.** `slug.rs` is 642 lines including its
  tests and its module doc is load-bearing. Moving it and changing it in one
  commit would make the review unable to separate the two; the plan sequences
  the move as a pure relocation first, with the behaviour changes after.
