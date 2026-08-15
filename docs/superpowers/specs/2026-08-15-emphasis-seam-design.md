# kasane — Emphasis Seam Design Spec

**Date:** 2026-08-15
**Status:** Designed 2026-08-15. Not yet implemented.
**Parent spec:** `2026-08-09-markdown-escaping-design.md` (§ "Recorded as open"),
whose last three bullets this item closes together. Two of them were found by
`2026-08-15-adjacent-inline-fusion-design.md`'s review while widening P13's
alphabet; the third is a regression that item introduced, recorded by its
whole-branch review.
**Repo:** kasane

## 1. Purpose & scope

`emphasize` wraps an already-rendered inner buffer in `*` or `**`. It moves edge
*whitespace* outside the delimiters, which is what CommonMark's flanking rules
ask for, and looks at nothing else. Three defects follow from that, and they are
one defect seen at three seams: **the writer emits a delimiter without knowing
what it lands against.**

| IR | printed | recovers | should be |
|---|---|---|---|
| `[Emph(a), Strong(b), Strong([Emph(c)])]` | `*a***b*c***` | `abc**` | `abc` |
| `[Text("a"), Emph([Code("a")])]` | ``a*`a`*`` | `a*a*` | `aa` |
| `[Emph([Text("a")]), Strong([Code("bc")])]` | ``*a***`bc`**`` | `a**bc**` | `abc` |

The first is a **regression**: it recovered `abc` before the adjacent-inline
fusion item and `abc**` after it. The other two are older and were measured
identical at that item's base. All three leak delimiter characters into the
visible text of an ordinary paragraph, which is the §5 invariant of the parent
spec — escaping must never change what the Markdown renders to.

The instrument that found them is worth naming here, because §5 turns it into a
committed test: an exhaustive differential census over every inline sequence of
length 1-3 drawn from an 18-element alphabet, ~5,800 shapes, comparing the text
`pulldown-cmark` recovers against `kasane_gfm::rendered_text`. It counted 1604
corrupt shapes at the fusion item's base, 1356 mid-item and 715 at its head.
Six whole-pipeline properties and three review rounds had missed every shape in
the table above; one census pass found all of them.

### Scope

1. The printed stream never places two delimiter runs sharing a character next
   to each other. Where it would, the structure between them is spliced or
   fused away (§2.1, §2.2).
2. A run whose delimiter cannot be spelled where it lands — because it would
   abut the content's own punctuation and so flank on neither side — renders
   its children bare (§2.3).
3. `kasane-cli`'s inline-depth assertion moves to `kasane-adapters`, where it
   reads `Emph` nesting off the parsed IR instead of counting `*` in writer
   output (§2.4). That is what releases the `nests_alone` carve-out.
4. The census becomes a committed test with a checked-in allowlist, so a new
   corrupt shape fails the build (§5.1).
5. The parent spec's last three open bullets close (§6).

### The trade this item makes, stated once

**Text fidelity is the invariant; emphasis structure is expendable at a
colliding seam.** This is the trade `2026-08-15-adjacent-inline-fusion-design.md`
§2.4 already made — two `<em>`s became one to keep the prose clean — and this
item extends it rather than inventing it. Concretely: `<em>a</em><strong>b</strong>`
with nothing between them renders as one `<em>` where it renders as both today.
The recovered text is identical. A boundary is lost on a shape that is not
broken.

That cost is real and is paid deliberately. The alternative is §7 approach A:
mirror CommonMark's delimiter matching — flanking *and* the multiple-of-three
rule — inside the writer, precisely enough to tell `[Emph(a), Strong(b)]`
(correct today) from `[Emph(a), Strong([Code("bc")])]` (corrupt today), which
differ only in the character after the second run's opening delimiter. This
repo has refused a hand-mirrored CommonMark rule three times, and the refusals
were right: the mirror drifts every time the parser moves, and nothing in the
test tier notices.

### Confirmed, not assumed

Every row below was measured by rendering IR through
`kasane_writer::blocks_to_markdown` and parsing the result with
`pulldown-cmark` 0.13 — the oracle `tests/properties.rs` already uses — at the
revisions named. The probes were throwaway; §5 is where they become committed
tests.

- **The three shapes in §1's table**, at the fusion item's base `ad9e5ab` and
  at its head. Row 1 recovered `abc` at the base.
- **18 shapes correct at `ad9e5ab` are corrupt at the fusion item's head**, all
  of them row 1's family. 13 came from that item's original run scan and 4 from
  its final fix wave.
- **The census counts**: 1604 → 1356 → 715 corrupt shapes across the item's
  three revisions.
- **Mid-buffer nesting is safe and must not be touched.**
  `Emph([Text("a"), Strong(b), Text("c")])` prints `*a**b**c*` and recovers
  `abc`. A container sitting between other content contributes its delimiters
  with content on both sides, so nothing abuts. Only the *edges* leak.
- **A same-class container mid-buffer is also safe.** `Emph([Emph(b)])` between
  text prints `**b**`, which a parser reads as a strong span: structure
  reassociates, text survives. The fusion item's blanket `also` splice was
  therefore buying nothing mid-buffer while flattening a level.
- **`[Emph(a), Strong(b)]` is correct today** — `*a***b**` recovers `ab` — and
  `[Emph(a), Strong([Code("bc")])]` is not. They differ only in what follows
  the second run's opening `**`. This pair is the whole argument for §7
  approach A being the only *precise* option, and against taking it.
- **A backtick at an edge does not collide with `*`.** `Emph([Code("x")])`
  alone prints `` *`x`* `` and recovers `x`. The collision is between
  delimiters sharing a *character*, which is why §2.1's test is on the
  character and not on "is a delimiter".

### Non-goals

- **Mirroring CommonMark's delimiter matching.** §7 approach A. Rejected on
  drift, not on difficulty.
- **Alternating `*` and `_`.** Rejected by the fusion item's §7 B for the
  flanking and intraword-`_` rules it would import, and rejected again here
  for the same reason. This item removes hand-mirrored rules; it does not add
  one.
- **The remaining census shapes.** 715 shapes are corrupt at the fusion item's
  head. This item closes the families in §1's table; it does not claim to close
  every one. §5.1's allowlist is what keeps the rest visible and prevents the
  set from growing — the ratchet is the deliverable, not a zero.
- **`Inline::Code("")`'s round-trip divergence.** `code_span`'s Rule 1 prints
  `` ` ` `` against `rendered_text`'s empty string. Acknowledged, unreachable
  after `structure`, and excluded from the census alphabet exactly as P13
  excludes it.

## 2. The change

### 2.1 One rule, two seams

> The printed stream never places two delimiter runs sharing a character next
> to each other. Wherever it would, the structure between them is spliced or
> fused away.

`escape::Delim` gains the character it spells with, so the rule can be written
as stated rather than inferred from the variant list:

```rust
impl Delim {
    fn ch(self) -> char {
        match self {
            Delim::Backtick => '`',
            Delim::Emph | Delim::Strong => '*',
        }
    }
}
```

Today that makes `Emph` and `Strong` collide with each other and neither
collide with `Backtick`, which is what the measurements say. It also keeps the
rule true *as written* if a later item ever spells emphasis with `_`, rather
than true by the coincidence that this writer never does.

**Seam one — inside a run (the edge trim).** A run's flattened children are
trimmed at both edges: while the outermost *printing* element is a container
whose `Delim::ch()` equals the run's own, that container is spliced — replaced
by its own children, flattened — and the trim repeats, because splicing exposes
a new edge. Printing-ness is `renders_empty`, already defined.

**Seam two — between runs (the fuse).** Two adjacent runs whose delimiters
share a character are one run. The first member's class wins, so
`[Emph(a), Strong(b)]` renders as one `Emph` over `a` + `b`.

**The two seams are ordered, and the order is load-bearing.** Fusing happens
first, at run detection; trimming happens second, on the merged run's children.
The reverse would trim the edges of runs that are about to be merged, so an
element that ends up mid-buffer — where §1 confirms nothing collides — would
have been flattened for a collision that the fuse then removes. One pass of
each suffices: the trim's own loop handles the edges the fuse exposes, and
trimming never produces a new adjacent run, only new children.

Worked against the shapes that drove the item:

| IR | after trim / fuse | prints | recovers |
|---|---|---|---|
| `[Emph(a), Strong(b), Strong([Emph(c)])]` | trim `Emph(c)` off the strong run's tail | `*a***bc**` | `abc` |
| `[Emph([Emph(a)]), Emph([Text("bc")])]` | trim `Emph(a)` off the head | `*abc*` | `abc` |
| `Emph([Emph([Text("a")])])` | trim | `*a*` | `a` |
| `[Emph([Text("a")]), Strong([Code("bc")])]` | fuse to one `Emph` | ``*a`bc`*`` | `abc` |
| `Emph([Text("a"), Strong(b), Text("c")])` | untouched — no edge container | `*a**b**c*` | `abc` |

### 2.2 What this deletes

The fusion item's `flatten_into` carries an `also` parameter that splices a
same-`Delim` container *everywhere* in a run's children, and `nests_alone` is a
hand-cut exception to it covering the one arrangement that blanket rule broke.
Both go: `also` was doing edge work with a blanket rule and then apologising
for it mid-buffer, and an edge rule needs no exception. `flatten_into` keeps
only its link transparency, `emphasis_run` loses its double `run_children`
call, and `nests_alone` is deleted.

**This design removes more code than it adds.** That is worth stating because
the item's headline is a new rule, and a new rule that shrinks the file is a
different proposition from one that grows it.

### 2.3 The other collision: content punctuation

§2.1 is about a delimiter meeting another delimiter. A delimiter can also meet
the *content's* own punctuation, and then there is nothing to splice or fuse
with:

`[Text("a"), Emph([Code("a")])]` prints `` a*`a`* ``. The opening `*` is
preceded by a letter and followed by a backtick, so it is neither left- nor
right-flanking, and CommonMark leaves it as a literal asterisk. The paragraph
reads `a*a*` with both asterisks visible.

The decision cannot live in `emphasize`, which never sees past its own inner
buffer: the closing delimiter's flanking depends on the character *after* it,
which is the next element in the stream. `[Emph([Code("a")]), Text("a")]` fails
on exactly that side — the opening `*` is fine at line start, and the closing
one is preceded by a backtick and followed by a letter.

So the decision moves up to `inlines_to_md_flat`, which holds the whole view:

> Before wrapping a run, the scan asks whether the delimiter it would add can
> both open and close where it lands. If either side fails, the run renders its
> children bare.

It needs two character classes, each computable without rendering anything
twice:

- **Before**: the last character of the buffer emitted so far; line start
  counts as whitespace, which is what CommonMark says.
- **After**: the first character the rest of the view will emit. Walk forward
  past `renders_empty` elements and classify the first printing one — a
  container yields `*`, a code span or a degrading `Math` a backtick, a
  non-degrading `Math` a `$`, a `FootnoteRef` or an external `Link` a `[`, a
  `Text` its own first character (which stays punctuation if `escape::text`
  prefixes a backslash, so the class survives escaping). An exhausted view is
  end of line, which counts as whitespace.

`emphasize` itself does not change. It stays a pure spelling function; the
scan decides whether to call it with delimiters at all.

The two rules compose without either knowing about the other. §1's third
shape fuses under §2.1 into one `Emph` over `a` + `` `bc` ``; the fused run's
opening `*` is then followed by `a` and its closing `*` followed by end of
line, so both flank and the delimiters survive: `` *a`bc`* ``.

### 2.4 Moving the assertion that forced a carve-out

`kasane-cli/tests/e2e.rs` counts `*` characters in writer output to prove the
EPUB adapter's inline-flattening bound reached the CLI path. It is an *adapter*
property observed through *writer* bytes, and it is why `nests_alone` exists:
under §2.1's trim, `Emph([Emph([Text("a")])])` prints `*a*`, one asterisk
rather than the stack the assertion counts.

Both spellings are text-correct, so the assertion pins nothing wrong — but the
one it pins is structurally worse: 64 stacked `*` are read as 32 nested
`<strong>`, a semantic the IR never held. The assertion moves into
`kasane-adapters` and asserts `Emph` nesting depth on the parsed IR, which is
the property it was always about. The writer's central rule then needs no
special case.

## 3. Blast radius

- **`markdown.rs`** — `flatten_into` loses `also`; `emphasis_run` is rewritten
  around the edge trim; `nests_alone` is deleted; `inlines_to_md_flat` gains
  the run fuse, the spellability decision and one-element lookahead. Every
  Markdown inline sequence in the crate passes through it.
- **`escape.rs`** — `Delim::ch()`. No other change; `code_span`, `math_span`
  and the class function keep their rules.
- **`emphasize`** — unchanged, and deliberately so. The fusion item's spec §3
  put it out of scope and this item keeps it there: it gains no knowledge, it
  is simply not called with delimiters when they cannot be spelled.
- **`kasane-cli/tests/e2e.rs`** — the `*`-counting assertion leaves.
- **`kasane-adapters`** — it arrives, as an IR-depth assertion.
- **`kasane-writer/tests/properties.rs`** — P13's alphabet widens; the census
  arrives (§5.1).
- **`kasane-gfm`, `kasane-core`** — no change, behavioural or otherwise.
- **Rendered output for existing fixtures** — `tests/fixtures/epub/rich.epub`
  must convert to an identical tree, as in the fusion item. The fixture holds
  no colliding seam, so any diff means a rule fired where no collision exists.

## 4. Why the anchors stay correct

`*` is outside `is_word`, so `anchor_slug` discards it either way: every shape
in §1's table anchors the same before and after. This item is a content-fidelity
fix with no anchor consequence, which is the opposite of its parent's shape and
worth saying plainly, because the parent item's reader will expect otherwise.

`rendered_text` concatenates span contents across inlines, so a spliced or
fused run projects exactly as its members did. No `kasane-gfm` change, and the
agreement the fusion item established between a heading's printed line and its
anchor is preserved by construction rather than re-argued.

## 5. Testing

### 5.1 The census, committed

A new test in `crates/kasane-writer/tests/` renders every inline sequence of
length 1-3 over a fixed alphabet, parses each with `parse_events`, and compares
the recovered text against `kasane_gfm::rendered_text`. ~5,800 shapes,
deterministic, no proptest.

It carries a **checked-in allowlist of known-corrupt shapes**. A corrupt shape
absent from the list fails the build; a shape the list names that is *no longer*
corrupt also fails, so the file cannot rot into a set of stale excuses. Fixing
a family means deleting lines from it.

The allowlist is the design decision here, not an implementation detail. 715
shapes are corrupt at this item's base and this item closes three families, not
all of them; a test asserting zero could not be committed, and a test asserting
nothing would be worthless. A ratchet is what makes the next regression
impossible to ship quietly — which is the failure this whole item is a response
to. It is also, in effect, the `insta` snapshot tier the parent spec's §9 has
listed as unbuilt since 2026-08-09.

The alphabet excludes `Inline::Code("")` for the reason P13 documents.

### 5.2 Unit

In `markdown.rs`'s test module, one shape per family, asserting printed bytes,
plus the parsed recovery where the point is what a reader sees:

- the edge trim, at the head and at the tail, and one requiring two iterations;
- the run fuse, including `[Emph(a), Strong(b)]`, whose *structure* changes —
  this is §1's stated cost and it should be pinned so a later reader meets it
  as a decision rather than a surprise;
- the content-punctuation drop, in both the opening-side and closing-side
  orders;
- the controls that must **not** move: `Emph([Text("a"), Strong(b), Text("c")])`
  mid-buffer, `Emph([Code("x")])` alone, and a run of one.

### 5.3 Property

P13's alphabet gains `Strong(vec![Emph(vec![Text(w)])])`,
`Emph(vec![Code(w)])` and `Emph(vec![Emph(vec![Text(w)])])` — the three
widenings the parent spec records as blocked, all three unblocked by this item.
`P13_WORDS`'s doc comment loses the two blocking entries and keeps its
restricted-alphabet argument, which is unrelated and still true.

## 6. Documentation

- **`2026-08-09-markdown-escaping-design.md`** — the last three open bullets
  (edge punctuation, lone nested emphasis, the wrap-seam regression) each get a
  closure note in the shape the bullets above them use: what closed it, by
  which mechanism, and what the bullet predicted instead. The lone-nested
  bullet predicted a `(character, length)` delimiter class; what closed it is
  the edge trim.
- **`2026-08-15-adjacent-inline-fusion-design.md`** — §2.1's "a document
  containing no adjacent pair renders byte-identically" is already false at
  that item's head and is corrected here rather than left; §2.4's cost
  statement gains this item's extension of it; §2.2's `also` description goes
  with the parameter.
- **`AGENTS.md`** — the `kasane-writer` entry's fusion sentence becomes the
  general rule: delimiter runs sharing a character never abut, and a delimiter
  that cannot be spelled where it lands is not emitted.
- **`README.md`** — no user-visible anchor change (§4), so the Known
  limitations list gains nothing. Check rather than assume: the fusion item
  added a bullet there and this item does not.

## 7. Approaches considered

**A. Mirror CommonMark's delimiter matching.** Implement flanking *and* the
multiple-of-three rule in the writer, and emit delimiters only where they
resolve as intended. The only *precise* option: it is what distinguishes the
pair in § "Confirmed", which differ
only in the character after an opening delimiter (`[Emph(a), Strong(b)]`
against `[Emph(a), Strong([Code("bc")])]`). Rejected because it is a
fourth hand-mirrored CommonMark rule in a repo that has spent three items
retiring them, because it drifts silently whenever the parser moves, and
because §5.1's census would be the only thing that ever noticed the drift —
at which point the census is doing the work and the mirror is the liability.

**B. Splice or fuse at every colliding seam — chosen.** One rule for two seams,
keyed on the delimiter character, plus a separate rule for the content
collision that has nothing to fuse with. Statable in two sentences, deletes an
existing special case, and needs no knowledge of how the parser resolves runs —
only that two same-character runs must not touch. Costs a structural boundary
on shapes that are correct today (§1).

**C. Drop the second run's delimiters where they would abut.** Same trigger as
B, different remedy: emit no emphasis rather than fusing. Rejected because it
loses the same boundary B loses *and* the element with it — `[Emph(a),
Strong(b)]` would render `*a*b`, one element where B keeps one and today keeps
two. Strictly worse than B for the same trigger.

**D. Separate abutting delimiters with an invisible marker.** An inline HTML
comment renders as nothing and would keep every boundary. Rejected for the
reasons the fusion item's §7 C already gives: it makes correctness depend on
inline HTML surviving a sanitizer, puts markup the IR never asked for into
heading lines and table cells, and needs its own argument about GitHub's
heading-id algorithm.

**E. Fix it in `kasane-core` by canonicalizing the IR.** Rejected on the ground
the fusion item's §7 D establishes: this is a content defect in a public writer
over a public IR, so a caller rendering hand-built IR would still get the leak,
with the fix sitting in another crate. The clone hazard applies here in full.

## 8. Verification and risk

`mise run lint && mise run test` green, with `lint` covering `--all-targets`
plus `fmt --check`.

The proof specific to this item is §5.1's census: the allowlist must shrink by
at least the 18 regressed shapes and the two older families, and must not grow
by one. Converting `tests/fixtures/epub/rich.epub` must produce a tree
identical to the fusion item's, since that fixture holds no colliding seam.

Three residual risks, recorded rather than closed:

- **The shapes the allowlist still names after this item.** It closes three
  families and does not claim the rest. The allowlist makes them visible and
  bounded; a later item works the list down. The risk is that the list is read
  as an acceptance rather than a queue, which is why §5.1 makes a stale entry
  fail the build.
- **The structural loss in §1.** `<em>a</em><strong>b</strong>` becomes one
  `<em>`. Deliberate, pinned by a unit test, and reversible only by approach A.
- **`Delim::ch()` invites a `_` spelling.** Nothing here adds one, and the
  intraword-`_` carve-out is why. If a later item wants `_`, the character
  accessor is where it starts and the flanking rules it would import are what
  it must price.
