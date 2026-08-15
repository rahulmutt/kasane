# kasane — Emphasis Seam Design Spec

**Date:** 2026-08-15
**Status:** Implemented 2026-08-15. The edge trim, the run fuse and the
flanking decline each landed with unit coverage; the census
(`kasane-writer/tests/census.rs`) is committed with its ratcheting allowlist,
and P13 now draws delimiter-bearing emphasis children. A fourth rule not in
this design — a same-`Delim` splice applied anywhere in a run, not only at an
edge — was added mid-execution to close 8 shapes still corrupt after the
three rules above (5 of them regressions Tasks 3-4 introduced against this
item's own base, 3 shapes that reverted to a base-corrupt state after being
transiently fixed and then re-broken); see §8's result note and
`splice_children`'s doc comment in `markdown.rs`. The allowlist still names
the shapes §8 records as out of this item's scope.
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

  > **Corrected 2026-08-15, after implementation.** This bullet is true for
  > *that* shape and false as a general claim. Task 3 regressed
  > `[Emph([Emph([Text("a")])]), Emph([Emph([Text("a")])]), Emph([Code("x")])]`
  > — a same-`Delim` container sitting mid-buffer, between two other members
  > of the fused run — and it stayed corrupt through Task 5: the run fuse's
  > flattening puts the second member's `Emph([Text("a")])` between the first
  > member's already-spliced `a` and the third member's `` `x` ``, printed as
  > `` *a*a*`x`* `` at that point, which a real parser reads as
  > `Emphasis("a")` then a bare `a`, a literal `*`, `` `x` ``, and a second
  > literal `*` — recovering `` aa*x* `` against `aax`, the delimiters
  > leaking into visible text exactly as the other three families in §1 do.
  > This and several
  > shapes like it are what the same-`Delim` splice — not described anywhere
  > in this file's §2.2 — was added to close (see `splice_children`'s doc
  > comment in `markdown.rs`, and §8's result note below for the exact
  > count). "Nothing mid-buffer" is what this section designed for; it is not
  > what shipped.
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

> **Corrected 2026-08-15, after implementation.** `also` and `nests_alone` did
> both go, exactly as designed here. But "an edge rule needs no exception" did
> not hold: mid-execution measurement found a family of shapes (§1's
> corrected "same-class container mid-buffer" bullet above) where a
> same-`Delim` container loses text if it is left mid-buffer, and closing
> them meant restoring a rule shaped like `also` — splicing anywhere in a
> run, not only at an edge — re-keyed on `Delim` instead of the character
> `edge_to_splice` uses, and with no `nests_alone`-shaped exception, since the
> assertion that exception protected had already moved to `kasane-adapters`.
> `same_delim_to_splice` in `markdown.rs` is that rule; see
> `splice_children`'s doc comment for why two differently-keyed rules exist
> rather than one, and §8's result note for what it cost. "Removes more code
> than it adds" was true of this task's own diff and is not true of the item
> as a whole once the inserted task is counted.

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

### 2.5 A fourth rule, added mid-execution

§2.1-§2.4 above were not revised to add the same-`Delim` splice a later task
inserted; §8's result note is the current record of why it exists and what it
cost, and `splice_children`'s doc comment (`markdown.rs`) is the current
record of the rule itself.

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

P13's alphabet gains `Strong(vec![Emph(vec![Text(w)])])` and
`Emph(vec![Emph(vec![Text(w)])])` — two of the three widenings the parent spec
records as blocked. `Emph(vec![Code(w)])` stays out.

> **Corrected 2026-08-15, after implementation.** This section originally
> claimed all three widenings would unblock; only two did. With
> `Emph(vec![Code(w)])` in the alphabet the property failed on roughly one
> run in four, on `[Code("a"), Emph([Code("a")]), Text("a")]`: the middle
> member declines its delimiters (§2.3) and prints its bare `Code` child, so a
> leading backtick lands immediately after the previous code span's closing
> backtick, and a parser reads the adjacent pair as one delimiter — the
> shape recovers `` a``aa `` against `rendered_text`'s `aaa`. This is not one
> of the defects §1 named; it is a member of the residual family that §8 and
> the census allowlist already track (the shape is one of the 32 the allowlist
> still names, corrupt at this item's base and not closed by it). Widening the
> alphabet to include it would make the property intermittently fail on a
> pre-existing defect, which is worse than not drawing the shape at all, so
> the third arm is deferred rather than landed. `P13_WORDS`'s doc comment
> loses the two blocking entries that *did* close, keeps its
> restricted-alphabet argument, which is unrelated and still true, and records
> the `Emph(vec![Code(w)])` exclusion with this counterexample.

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

> **Result, recorded 2026-08-15, re-derived from the committed artifacts
> rather than taken on trust (see the derivation note at the end of this
> block).** The committed census (`kasane-writer/tests/census.rs`) is the
> authority for what this item achieved, not the throwaway probe's
> ~5,800-shape, 715-corrupt count in §1 and § "Confirmed" — a different
> instrument, run at a different revision. The committed allowlist went 686
> (first bless, Task 2) → 602 (the edge trim) → 290 (the run fuse) → 48 (the
> flanking decline) → **32**, where it stands now — the line count never grew
> at any commit, so the letter of "must not grow by one" held throughout. But
> the *set of named shapes* did churn in a way this sentence did not
> anticipate. The edge-trim commit newly corrupted 9 shapes that were correct
> at this item's own base while fixing 93 others (net 686 → 602). The
> run-fuse commit fixed 317 shapes relative to the edge-trim state — a count
> that *includes* 4 of the edge trim's 9 regressions, closed as a side effect
> of `run_end`'s grouping change, not 317 in addition to them — while newly
> corrupting 5 shapes of its own (net 602 → 290). That left 10 distinct
> shapes named in the allowlist that neither rule closed: the edge trim's 5
> surviving regressions, plus the run fuse's own 5. Of those 10, only 7 were
> ever regressions against this item's own base (the edge trim's 5, and 2 of
> the run fuse's 5); the other 3 of the run fuse's 5 were shapes already
> corrupt at base, fixed transiently by the edge trim, and re-broken by the
> run fuse — back to the base's own state, not a new regression against it.
> None of this was the three-family scope this spec designs; it was landed
> under controller ruling. The flanking decline closed 2 of the 10 as a side
> effect, leaving 8 (5 genuine regressions, 3 reversions) still named after
> Task 5. An inserted task between Tasks 5 and 6 closed all 8 with a fourth
> rule (a same-`Delim` splice, `splice_children` in `markdown.rs`, not
> designed here — see its doc comment) rather than by the flanking decline
> alone. See the plan's "Amendments during execution" section for the
> sequence; this spec's §2 was not revised to add the fourth rule, since the
> rule's own doc comment is now the more current record of it.
>
> The fourth rule's own cost is not the fuse's structural-boundary trade
> (below): it is applied *unconditionally*, splicing every same-`Delim`
> mid-buffer container regardless of whether the nesting it replaces would
> have round-tripped safely. `*a *b* c*` parses today as
> `Emphasis["a ", Emphasis["b"], " c"]`, its inner `<em>` intact, because the
> whitespace around the inner `*`s stops them from flanking the way the outer
> pair does; the same-`Delim` splice flattens it to `*a b c*` anyway, losing a
> nested `<em>` on a shape that was not broken. Telling that shape apart from
> `*a*b*c*`, where the same nesting *does* corrupt, means reasoning about how
> a parser pairs delimiters at splice time — design spec §7 approach A, which
> this item exists to retire rather than reimplement — so the rule is
> deliberately unconditional and pays the cost on shapes nobody measured as
> broken. Pinned by
> `splicing_mid_buffer_costs_a_span_that_would_round_trip` in
> `crates/kasane-writer/src/markdown.rs`.
>
> **Derivation.** Ran
> `git show <sha>:crates/kasane-writer/tests/census-known-corrupt.txt | sort`
> for `2bef986`, `af8e53b`, `fb19772`, `ebf5cfd`, `d989fae` and `9033515`, and
> compared consecutive and non-consecutive pairs with `comm`. `comm -13 A B`
> names shapes new in `B`; `comm -23 A B` names shapes fixed between `A` and
> `B`; `comm -12`/`comm -23` against `2bef986.txt` sorts a set into
> "already corrupt at base" versus "not." The 9-, 93-, 5- and 317-shape counts
> match the af8e53b and fb19772 commit messages exactly. The 10-shape
> outstanding set, the 7-versus-3 base-regression split, the 2 the flanking
> decline closed, and the 8 that reached Task 5b were computed, not asserted.

Four residual risks, recorded rather than closed — the fourth added by the
inserted task, see the result note above:

- **The shapes the allowlist still names after this item.** It closes the
  three families in §1's table, plus a fourth this item found and closed
  mid-execution (the same-`Delim` splice; see the result note above), and does
  not claim the rest: 32 shapes remain named. All 32 are one family — verified
  against the committed allowlist, not assumed — the flanking decline's
  exposed backtick seam: `[backtick-bearing, Emph|Strong([Code]), Text]` and
  its mirror `[Text, Emph|Strong([Code]), backtick-bearing]`.
  `[Code("x"), Emph([Code("x")]), Text("a")]` is the census alphabet's
  instance, and the same shape §5.3 records as why `Emph(vec![Code(w)])` did
  not join P13's alphabet. The allowlist makes the family visible and
  bounded; a later item works it down. The shape of that fix is already
  known: a re-scan. If a declined run's children re-entered the outer view
  before run detection instead of landing directly in the buffer,
  `[Code("x"), Emph([Code("x")]), Text("a")]` would fuse to `` `xx` `` and
  recover `xxa`, closing the whole residual set without any delimiter-pairing
  logic — no approach-A reasoning required, just one more pass over the
  declined run's own exposed edge. The risk today is that the list is read as
  an acceptance rather than a queue, which is why §5.1 makes a stale entry
  fail the build.
- **The structural loss in §1.** `<em>a</em><strong>b</strong>` becomes one
  `<em>`. Deliberate, pinned by a unit test, and reversible only by approach A.
- **The structural loss the same-`Delim` splice pays unconditionally.**
  `*a *b* c*` — a nested `<em>` that round-trips correctly today, because the
  inner delimiters are one-sided-flanking — is spliced to `*a b c*` anyway,
  the same as a shape that would corrupt. Deliberate, for the same reason
  approach A is rejected: telling the two apart means pairing delimiters like
  a parser, at splice time. Pinned by
  `splicing_mid_buffer_costs_a_span_that_would_round_trip`, and reversible
  only by approach A, same as the bullet above.
- **`Delim::ch()` invites a `_` spelling.** Nothing here adds one, and the
  intraword-`_` carve-out is why. If a later item wants `_`, the character
  accessor is where it starts and the flanking rules it would import are what
  it must price.
