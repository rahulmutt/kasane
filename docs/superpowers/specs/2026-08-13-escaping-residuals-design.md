# kasane — Escaping Residuals Design Spec

**Date:** 2026-08-13
**Status:** Approved (design), pending implementation plan
**Repo:** kasane

## 1. Purpose & scope

The Markdown escaping policy (2026-08-09, PR #35) closed the writer's escaping
gap but left three shapes unfixed, recorded in that spec's §4.5 rather than in
a ledger. This item closes all three.

Restated from §4.5, with the mechanism each one actually needs:

| | Shape | What breaks |
|---|---|---|
| **A** | `Para([FootnoteRef(1), Text(": note")])` | Renders `[^1]: note` at column 0, which parses as a footnote *definition* rather than a paragraph containing a reference. The paragraph is lost. |
| **B** | `Text("  # h")` at column 0; `Emph([Text("    x")])` | `escape::text` clears `line_start` on any leading space, so the `#` opens a heading. Four or more spaces open an indented code block, which no marker escaping can reach. |
| **C** | Heading `[Text("A\r"), Code("\nB")]` | `anchor_fold` sees the two newline characters adjacent in `inline_text` and emits `a-b`; the renderer folds each run separately, so GitHub computes `a--b`. The emitted cross-reference is dead. |

A and B lose or corrupt document content. C emits a link that resolves to
nothing, which is the failure §5 of the escaping spec exists to prevent.

### Confirmed, not assumed

Every mechanism below was chosen after probing `pulldown-cmark` — already a
`kasane-writer` dev-dependency, with the same GFM extensions the round-trip
property enables — rather than by reading the CommonMark spec. That order
mattered: three readings that looked obviously correct were wrong.

| Probe | Result |
|---|---|
| `[^1]\: note` with no matching definition | **No** `FootnoteReference` — the ref decomposes into `Text("[")`, `Text("^1")`, `Text("]")` |
| `[^1]\: note` with a matching definition present | `FootnoteReference("1")` + `Text(": note")` — ref intact, definition gone |
| `   \# h` | `Paragraph`, `Text("# h")` — **the three leading spaces are stripped** |
| `&#32;  # h` | `Paragraph`, `Text(" ")` + `Text("  # h")` — whitespace preserved, `#` inert |
| `&#32;   x` | `Paragraph`, not `CodeBlock(Indented)` |
| `&#9;x` | `Paragraph` — the tab case behaves like the space case |
| `\|  x \|` in a table row | `Text("x")` — **a cell's leading whitespace is dropped today** |
| `# A ` `` `  B ` `` | `Text("A ")` + `Code(" B")` — rendered text `A  B`, id `a--b` |

The first row is why A looked unfixable at first glance and is not: a footnote
reference only parses as one when a definition exists, so a probe without one
measures the probe, not the mechanism. The third row is why B needs a
character reference rather than the backslash §4.5 assumed — the backslash
form suppresses the heading but silently deletes the whitespace, violating §5
of the escaping spec while appearing to succeed.

### Scope

- A three-state escaping position replacing `escape::text`'s `at_line_start:
  bool`, so A's rule can live in `escape.rs` (§2).
- A numeric-character-reference rule for whitespace at a line start, covering
  `Ctx::Flow` and `Ctx::Cell` (§3).
- A cell-edge rule for a cell's trailing whitespace (§3.3).
- A pre-render inline fold applied in the writer's one-line contexts (§4).
- Deterministic unit tests per residual, a focused property for C's class, and
  two new `HOSTILE` fragments (§5).
- Documentation: §4.5 of the escaping spec rewritten, README, AGENTS.md (§6).

### Non-goals

- **Ending the core/writer fold mirror.** Approach (iii) in §7 — moving the
  anchor rule into a crate both `kasane-core` and `kasane-writer` depend on —
  is the only change that closes the divergence *class*, including the
  footnote-ref and trailing-`#` cases `slug.rs` documents as surviving on
  purpose. It is an architecture change with its own ripple through
  `assign_paths`, and bundling it here would swallow A and B.
- **`inline_text`'s treatment of `Inline::FootnoteRef`.** Unchanged; `nav`,
  `refs` and `balance` want its current behaviour, as `slug.rs` already
  records.
- **Whitespace in the merged-table HTML fallback.** Not reachable by escaping
  at all (§3.4).
- **The `insta` snapshot tier.** Still the last unbuilt tier from the original
  design spec §9.

## 2. The escaping position

`escape::text`'s `at_line_start: bool` widens to a three-state value:

```rust
pub(crate) enum Pos {
    /// The next character emitted lands at the start of a line.
    LineStart,
    /// The next character lands directly after a `[^n]` that itself opened
    /// the line.
    AfterFootnoteRef,
    /// Anywhere else.
    Mid,
}
```

This exists so A's rule can stay inside `escape.rs`. §2 of the escaping spec
states that `escape.rs` owns every rule, and A's trigger — "the previous
inline was a footnote reference at column 0" — is positional information the
renderer already computes. Widening the position keeps `markdown.rs` doing
only what it does today (reporting where the next character lands) instead of
deciding what to escape.

`inlines_to_md_at` currently recomputes `line_start = s.ends_with('\n')` after
every arm. That becomes:

1. If the arm appended nothing, the position is **unchanged**.
2. Otherwise, if the accumulated output ends with `\n`, `LineStart`.
3. Otherwise, if this arm was `Inline::FootnoteRef` and the position before it
   was `LineStart`, `AfterFootnoteRef`.
4. Otherwise, `Mid`.

Rule 1 is not defensive padding. `Para([FootnoteRef(1), Text(""), Text(":
x")])` is a shape the IR permits, and without it the empty run would reset the
position to `Mid` and the colon would go unescaped.

Rule 3 is deliberately narrow. `Para([Emph([FootnoteRef(1)]), Text(": x")])`
renders `*[^1]*: x`, which begins with `*` and was never a definition; the
`Emph` arm yields `Mid`, which is correct.

**The escape itself is gated to `Ctx::Flow`.** `Pos::AfterFootnoteRef` arises
in `Ctx::Cell` too, because `render_table` renders every cell with
`at_line_start: true` (§3.2), but a cell is inline context where `[^1]:` is
never a definition. Escaping there would emit a backslash that renders as
nothing but exists for no reason. `escape::text` already takes `Ctx`, so the
gate costs nothing.

### 2.1 What this does not change

`Block::Footnote` renders its body blocks with `at_line_start: true` even
though they land after the `[^{n}]: ` prefix. That is a pre-existing
inaccuracy, and it errs in the safe direction — a needless backslash on a body
that happens to start with `#`, never a missed escape. It stays as it is.
Correcting it would move a call site from over-escaping to exactly-escaping
with no defect to point at, which is not a change this item should make.

## 3. Whitespace at a line start

**Rule.** At `Pos::LineStart`, a space or a tab is emitted as a numeric
character reference — `&#32;` or `&#9;` — and the position drops to `Mid`.

Only the first character of the run needs it. Everything after it is no longer
at column 0, so no marker following it needs escaping either: `&#32;  # h`
emits an inert `#` with no backslash, and `&#32;   x` is a paragraph rather
than an indented code block.

The character reference is the mechanism because it is the only one that
preserves the text. §4.5 assumed the fix was to make `at_line_start` survive a
leading whitespace run and then escape the marker — but `   \# h` parses as a
paragraph whose text is `# h`, with the three spaces gone. That is a §5
violation of the escaping spec (escaping must never change what the Markdown
renders to) presenting as a fix. It also cannot address four-or-more spaces at
all, since an indented code block has no marker to escape.

§4.5 further characterized this as a change to "the line-start rules for every
context at once", which is why it was held back. That characterization followed
from the backslash mechanism, not from the defect: as a character reference it
is one rule in `escape.rs`, and it changes no other rule's behaviour.

### 3.1 `Ctx::Flow`

As above.

### 3.2 `Ctx::Cell`, leading edge

This falls out for free and fixes a live defect. `render_table` already renders
every cell with `at_line_start: true`, so the rule fires without any new
plumbing — and GFM strips a cell's leading whitespace, so `|  x |` renders `x`
today, losing document text. With the reference it renders `  x`.

### 3.3 `Ctx::Cell`, trailing edge

A cell's *trailing* whitespace is stripped by the same GFM rule, and the
positional state cannot see it — "the last character of this cell" is not a
line-start question. `escape.rs` therefore grows a cell-edge helper that
`render_table` applies to each rendered cell, replacing a trailing space or tab
with its character reference.

Fixing one edge and not the other would leave §5's invariant half-true inside
`Ctx::Cell`, which is the kind of asymmetry that reads as an oversight later.
The helper lives in `escape.rs` for the same reason as everything else in §2.

The surrounding `format!("| {} |", …)` padding is unaffected: a cell rendering
to `x&#9;` becomes `| x&#9; |`, whose content parses as `x` plus a tab once the
literal padding spaces are trimmed.

### 3.4 `Ctx::Html`

No change, and no fix available. `html_text` escapes `&`, so no reference is
emitted — and it would not help if one were, because an HTML renderer collapses
whitespace runs whether they arrived literally or as `&#32;`. Whitespace
fidelity is not recoverable inside the merged-table fallback. This belongs with
the other costs of that path, which §3.3 of the escaping spec already lists.

### 3.5 Interaction with `emphasize`

§4.5's own fix moved whitespace at the edges of rendered inner content outside
the emphasis delimiters. That looks like it now double-handles the same
whitespace, and it does not.

At column 0, `Emph([Text(" x")])` renders `*&#32;x*` rather than the current
` *x*`: the inner render has already replaced the space, so `emphasize`'s
`trim` finds no leading whitespace to extract. The result is still correct —
`*&` is left-flanking, so the emphasis applies, and the leading space stays
inside the emphasis span where the IR put it.

`emphasize` remains necessary for the two cases the position cannot see:
trailing whitespace, which is never at a line start, and leading whitespace
mid-line, as in `[Text("a"), Emph([Text(" b")])]`.

## 4. The pre-render inline fold

**Rule.** A normalization pass over `&[Inline]` collapses a newline run that
spans an inline boundary to a single `\n`, exactly as `normalize_newlines`
already collapses a run inside one `Inline::Text`. It rewrites the content of
`Inline::Text`, `Inline::Code` and `Inline::Math` leaves, walking the tree in
render order.

The writer applies it wherever it already calls `escape::one_line` over
rendered *inlines*: `Block::Heading`, the figure alt text, and an external
link's label. `one_line` then turns the surviving single `\n` into the one
space `anchor_fold` predicted.

Only the heading feeds an anchor. The pass applies to all three anyway,
because `one_line` is the writer's marker for "this is one line by grammar",
and letting the fold mean different things in different one-line contexts is
how the divergence got in.

`anchor_fold` and `slug::fold_newlines` are **untouched**. That is the reason
for choosing this over folding per-inline on the anchor side (§7): AGENTS.md
already describes the two folds as living in different crates with no shared
function, kept in step by hand. Teaching the anchor side about inline
boundaries would add `code_span`'s padding rules to what that hand-kept mirror
has to track. This keeps its surface exactly as wide as it is today.

### 4.1 `Inline::FootnoteRef` is opaque

A newline run does not collapse across a footnote reference. It renders as
visible `[^1]` text, so collapsing there would drop a space GitHub actually
renders.

`[Text("a\n"), FootnoteRef(1), Text("\nb")]` in a heading therefore still
diverges. That residual is not this item's to close: its cause is
`inline_text` skipping `Inline::FootnoteRef` while the writer renders it,
which is the divergence `slug.rs` already documents as surviving on purpose
(`## Notes[^1]` anchors `notes` here and `notes1` on GitHub). Closing it means
approach (iii).

### 4.2 `Inline::Math` changes delimiter in one-line contexts

`math_span` degrades to a code span when its content holds a newline, because
inline math can land in a GFM table cell where any newline ends the row. After
the fold there is no newline left in a one-line context, so that degradation
stops firing there and the equation renders as `$…$`.

The rendered *text* is unchanged, so §5 of the escaping spec holds. The reason
for the carve-out has simply evaporated in that context. Math inside a
`Block::Para` still degrades exactly as it does today.

## 5. Testing

### 5.1 Deterministic unit tests are the regression guard

One per residual, in the module that owns the rule.

`escape.rs`'s case table gains: the `:` at `Pos::AfterFootnoteRef`; a leading
space and a leading tab at `Pos::LineStart` in `Ctx::Flow`; the same two in
`Ctx::Cell`; the cell-edge helper's trailing space and tab.

`markdown.rs` gains: the `Pos` threading cases from §2 (an empty text run
between the reference and the colon, `Emph([FootnoteRef])` yielding `Mid`), a
table row exercising both cell edges, and the `[Text("A\r"), Code("\nB")]`
heading from §4.

Each asserts against `pulldown-cmark` rather than against a reading of the
CommonMark spec. Every mechanism in this design was chosen because a parser
confirmed it, and §1 records three readings that were wrong before it did.

### 5.2 A focused property covers C's class

`[Text(a + nl₁), X(nl₂ + b)]` in a heading, where `X` ranges over `Inline::Code`,
`Inline::Math`, `Inline::Emph`, `Inline::Strong` and `Inline::Link`, and each
`nl` over `\n`, `\r` and `\r\n`. It asserts P2's equality: the anchor
recomputed from parsed heading text equals what `anchors_for_headings`
computed from the IR.

This is P2's assertion over a generator narrow enough to reach the shape.

### 5.3 What the main property tier does and does not cover

`HOSTILE` gains a `\r`-terminated fragment and a newline-leading fragment.
They make the boundary shape *reachable* by the main tier, which is what makes
shrinking useful if it ever fires. They do not make it *covered*, and this
spec says so rather than repeating §4.5's claim in the other direction:

The payload must end with the one fragment (1 in 25, since `payload` is
`"{token} {hostile}"` and `HOSTILE` holds 25 fragments once these two are
added), the decoration must be an `Inline::Code` (1 in 13 by `inlines`'
weights), and that code's filler must draw the other fragment as its first
word (1 in 100 — a 1-in-4 hostile draw over 25 fragments). That is about 1 in
32,500 per shape. A default run is 256 cases of 1–40 shapes, so roughly 5,000
shapes, which puts the odds of the shape appearing at all near **15% per
run** — about one run in seven.

§5.1 and §5.2 are what hold this line. The fragments are worth adding anyway;
they are not worth claiming as coverage.

### 5.4 P7 needs no change

Character references decode to the characters they stand for, so the
round-trip property's payload occurrence counts are unaffected.

P7 also cannot *confirm* B: it whitespace-normalizes the event stream before
counting, so preserved leading whitespace is invisible to it. That is §5.1's
job.

### 5.5 Fuzzing

`fuzz_entry::escape` already loops `escape::text` over
`[Ctx::Flow, Ctx::Cell] × [true, false]`; the inner loop becomes the three
`Pos` states.

Its existing assertions do not need relaxing for the character reference.
`assert_unescaped_specials_are_absent` checks `` ` ``, `*`, `_`, `[`, `]`,
`~` and `$`, none of which appear in `&#32;` or `&#9;`, and `&` is not on that
list — so an emitted reference passes unchanged. The `#` in a reference is
likewise unchecked, and correctly so: `#` is a `LINE_START` character, not an
inline opener.

## 6. Documentation

**Escaping spec §4.5** stops being a list of deferred items and becomes a
record of three closed ones, naming the two cases that genuinely remain: the
footnote-ref-adjacent fold (§4.1) and whitespace in the merged-table HTML
fallback (§3.4). **§8 of that spec** gains approach (iii) as the
considered-and-not-taken option, so the next reader hitting an anchor
divergence finds the argument instead of re-deriving it.

**README**'s Markdown-as-text bullet under "Known limitations" gains the
character reference: the source now contains `&#32;` or `&#9;` where document
text has leading whitespace on a line, alongside the backslashes already
described. The cell fix is a behaviour change and gets its own sentence — text
that was silently dropped now survives.

**AGENTS.md**'s `kasane-writer` entry gains `Pos`, the writer's new
escaping-position vocabulary.

**`generator/mod.rs`**'s `is_comment` doc comment says "of `HOSTILE`'s 21
fragments, only `-->` triggers it; the other 20 round-trip". `HOSTILE` holds
23 today — the two newline-run fragments were added without updating the
count — and §5.3 adds two more. The count gets corrected to 25 in the same
change that makes it wrong again, since the claim it supports (only `-->`
triggers `comment_note`'s transformation) stays true and is worth keeping
checkable.

**AGENTS.md**'s `kasane-core` entry and **`slug.rs`**'s module docs are
unchanged, and that is the concrete payoff of §7's choice: the passage
describing the two hand-kept folds stays accurate word for word.

## 7. Approaches considered

For C. A and B had one workable mechanism each once the probes in §1 ruled out
the alternatives.

**(i) Fold per-inline on the anchor side.** `anchor_fold` folds each inline
separately instead of over the concatenated `inline_text`, matching what the
renderer does. Smallest diff and confined to `kasane-core`.

Rejected because it widens the mirror. The hand-kept correspondence between
`escape::one_line` and `slug::fold_newlines` is already flagged in AGENTS.md as
a standing hazard — "a future change to one fold that is not mirrored in the
other reopens exactly the anchor mismatch this pairing closed". Under (i) the
mirror would additionally have to track where the writer's inline boundaries
fall and how `code_span` pads. A wider standing hazard is a poor trade for a
smaller diff.

**(ii) Pre-render inline fold.** Chosen. §4.

**(iii) A shared crate, ending the mirror.** Move `one_line` and the anchor
rule into a crate both `kasane-core` and `kasane-writer` depend on, and compute
the anchor from a shared model of the rendered line.

This is the principled end state. It is the only option that closes the
divergence *class* rather than one instance — the footnote-ref case (§4.1) and
the trailing-`#` case both fall to it, and both are currently documented as
surviving on purpose. It was not chosen here because it touches `assign_paths`,
which needs the anchor at structuring time and therefore before anything is
rendered, and because an architecture change of that size would swallow A and
B. It deserves its own item.

## 8. Verification and risk

**Behaviour changes that reach existing output.** Three, all of which change
Markdown source that previously rendered wrongly:

1. A line of document text beginning with whitespace now carries a leading
   `&#32;` or `&#9;`. Previously the whitespace was dropped or the line became
   a heading, list, quote or indented code block.
2. A table cell with leading or trailing whitespace now preserves it.
3. A heading whose inline sequence splits a newline run across a boundary now
   embeds an anchor that resolves.

None of these changes a path, because `path_slug` does not fold newlines and
takes IR text upstream of escaping. Only (3) changes an anchor, and it changes
it from one that resolves to nothing to one that resolves.

**Risks.**

- **The mirror still drifts.** This item narrows what diverges but does not
  end the hand-kept correspondence. §7's approach (iii) is the exit; until it
  is taken, AGENTS.md's warning stands unchanged.
- **`pulldown-cmark` is not github.com.** Every probe in §1 was run against
  the dev-dependency, which is a GFM implementation and not the renderer
  kasane's anchors target. The external check for anchor parity remains the
  one recorded in the slug-widening spec §8.1 (run 2026-08-09, 13/13 ids
  matching a real render); §5.2's new heading shapes are candidates for the
  next run of it.
- **Character references are visible in the source.** A reader of the emitted
  Markdown sees `&#32;` where the book had a space. That is the same trade the
  backslashes already made, and it is the trade §5 of the escaping spec asks
  for: the rendered text is what must be faithful.
