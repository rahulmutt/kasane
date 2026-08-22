# kasane — Declined-Run Rescan Design Spec

**Date:** 2026-08-21
**Status:** Designed, measured, not implemented. Every figure below was
produced by a throwaway probe in an isolated worktree (§2); nothing is on a
branch.
**Parent specs:** `2026-08-15-emphasis-seam-design.md` (§8, whose residual
bullet names this family and proposes the fix this item measures),
`2026-08-16-structural-census-design.md` (§8, the residual program),
`2026-08-18-abutment-ledger-design.md` (§2b.5, the finding that shapes this
item's guard).
**Repo:** kasane

## 1. Purpose & scope

`census-known-corrupt.txt` holds 32 shapes, and they are the last shapes in
this repo that lose **text**. Every other queued census entry loses a span
boundary — a `<strong>` returning as an `<em>`, a nesting level dropped — which
the writer trades away deliberately, because text is the invariant and
structure is not. These 32 do not trade anything. They hand a reader different
characters than the document had.

All 32 are one family, and the mechanism is recorded at the site: when
`emphasis_run` finds that its delimiter would not flank where it lands, it
declines to emit the delimiter and pushes the run's rendered children straight
into the output buffer. Those children re-expose their own edges to whatever
already stands beside them, and **nothing re-scans that seam**. For
`[Code("x"), Emph([Code("x")]), Text("a")]` the buffer ends up holding
`` `x``x`a ``, in which a parser reads one code span spanning both — recovering
`x``xa` where the IR said `xxa`.

**In scope:** the seam. All 32 shapes, and the general defect behind them.

**Out of scope:** the 1,698 structurally-queued shapes, the `_` alphabet,
`Ctx::Cell`/`inlines_to_html`. See §9.

## 2. The measurement that gates this item

The parent spec's §8 states the fix as fact:

> If a declined run's children re-entered the outer view before run detection
> instead of landing directly in the buffer, `[Code("x"), Emph([Code("x")]),
> Text("a")]` would fuse to `` `xx` `` and recover `xxa`, **closing the whole
> residual set** without any delimiter-pairing logic.

That prediction was never measured. It is wrong by half, and this section is
the measurement — run before this spec's §3 was written, for the reason the
ledger spec's Risk 4 records: the same repo has already shipped a spec whose
§2 table priced an item at 924 shapes that measured at 389.

### 2.1 Three variants

| variant | on decline, `emphasis_run` hands back | outer loop |
|---|---|---|
| **A** | the spliced children | splices over `items[i..end]`, restarts at `i` |
| **B** | the spliced children | as A, **plus** rolls the buffer back one run and restarts there |
| **C** | the **un**-spliced children | as B |

A is the parent spec's proposal, stated exactly.

### 2.2 The text tier, exhaustive, over the census's own 19-element alphabet

| length | shapes swept | corrupt on `main` | A | B | C |
|---|---|---|---|---|---|
| 1–3 | 7,239 | 32 | 16 | **0** | **0** |
| 4 | 130,321 | 1,344 | 672 | **0** | **0** |
| 5 | 2,476,099 | 40,128 | 20,160 | **0** | **0** |
| 6 | 47,045,881 | 1,043,776 | — | **0** | — |

**Newly text-corrupt shapes: zero in every cell.** At lengths 4 and 5 this is a
set difference (`comm` over the sorted shape dumps from both worktrees), not a
count comparison; at length 6 the head total is 0, so it holds trivially.

### 2.3 What the measurement changed

**A closes 16 of 32, not 32.** The survivors are exactly the head half,
`[backtick-bearing, Emph|Strong([Code]), Text]`; the tail half,
`[Text, Emph|Strong([Code]), backtick-bearing]`, closes. The cause is
mundane rather than deep, and it is why A cannot be repaired by widening what
it splices: `inlines_to_md_flat` walks forward, and by the time a run declines,
the element before it is already a substring of `s`. Restarting at `i` can fuse
the exposed edge with what *follows*. Nothing in a forward-only loop can reach
what precedes.

This repo has now measured a head/tail asymmetry twice, on two unrelated
changes, for two unrelated reasons — the ledger's was CommonMark delimiter
pairing (§2b.3 there), this one is the emit loop's own direction. Neither was
predicted. **Read a symmetric-looking rule as an untested claim until a sweep
says otherwise.**

**C is measurement-identical to B** at every length and on every census file.
Un-splicing was worth testing — the splice exists to stop a container colliding
with the run's own delimiter, and a declined run prints no delimiter, so it
looked like structure being discarded for nothing. It is not: the outer loop
re-splices whatever needs it when the children re-enter the view. C is
therefore dropped under YAGNI, as an extra behaviour change buying nothing
measurable.

### 2.4 Approach E, and why it is blocked rather than merely harder

Deciding the decline *before* run detection would need no emission, no
rollback, and no splice. It cannot be done: `emphasis_run` classifies flanking
from the first and last character of the **rendered** core (`core.chars()
.next()` and `.next_back()`), so knowing whether a run declines requires having
rendered it. The circularity is in the flanking rule, not in this writer's
arrangement of it. Recorded so it is not retried.

### 2.5 Gates, measured

`mise run test` exit 0 and `mise run lint` exit 0 under B. `mise run
census-ratchet` **fails**; §5 is that failure and its repair. Wall-clock on a
render-bound 130k-shape sweep: 406ms on `main`, 433ms under B, **+6.6%**.

## 3. The seam

Two changes, both in `crates/kasane-writer/src/markdown.rs`.

### 3.1 `emphasis_run` returns what it decided

Today the function returns `String` and its decline branch falls through to
`inner` — the children, already rendered, already flattened into text. It
becomes:

```rust
/// What one emphasis run contributed: either printed text, or the children it
/// declined to wrap, handed back for re-entry into the outer view.
enum RunOut<'a> {
    Emitted(String),
    Declined(Vec<Flat<'a>>),
}
```

The decline branch returns `RunOut::Declined(children)` — the same `children`
the function already computed via `splice_children`, so no new call and no new
allocation. The rendered `inner` is discarded on that path.

This is the whole conceptual move. A declined run printed no delimiter, so its
children are not "the run's contents" in any sense a parser can see: they are
plain neighbours in the printed line. Putting them in the buffer asserts
otherwise, and the 32 shapes are what that assertion costs.

### 3.2 `inlines_to_md_flat` gains a working view and a checkpoint stack

The loop takes ownership of its view (`let mut items: Vec<Flat<'a>> =
items.to_vec()`) so a decline can rewrite it. `Flat<'a>` is `(&'a Inline,
usize)` and `Copy`, and `splice_children` already returns an owned `Vec<Flat<'a>>`
borrowing the same tree, so every lifetime here is already in hand.

Alongside it, one checkpoint per iteration:

```rust
let mut marks: Vec<(usize, usize, Pos)> = Vec::new();   // index, buffer len, pos
```

pushed at the top of the loop, before anything renders. On `RunOut::Declined`:

1. `items.splice(i..end, children)` — the container is gone from the view,
   replaced by what it held.
2. Pop this run's own checkpoint, then pop the predecessor's.
3. `s.truncate(plen)`, restore `pos`, set `i = pi`, `continue`.

When there is no predecessor — the declined run opened the view — step 3 is
skipped and the loop restarts at `i` with `s` and `pos` untouched, which is
variant A's behaviour and correct: there is nothing behind the run to fuse
with. Nothing is truncated in that case, and the splice in step 1 is still what
makes progress.

The predecessor is then re-rendered against a view in which the declined
container no longer stands between it and the exposed children — which is
exactly what lets `` `x` `` and `` `x` `` meet as one backtick run, print
`` `xx` ``, and recover `xxa`.

**Why a stack and not one saved slot.** A rollback can cascade: the
re-rendered predecessor may itself now decline, because the view past it has
changed. The stack pops naturally into *its* predecessor. A single slot would
handle one level and silently mis-handle two.

**Why one run back and not more.** One run is what §2 measured, and it reaches
zero at every length through 6. Backing up further has no measured benefit and
would enlarge the re-render window for nothing.

> **Amended, 2026-08-22 — the view is split at the cursor, not one vector.**
> This section's `items.to_vec()` + `items.splice(i..end, children)` is the
> shape that shipped, and the final review of this branch found it quadratic in
> paragraph **breadth**. A container that declines and hands back more than one
> child grows the vector, and `Vec::splice` then memmoves the whole untouched
> tail; with `k` such declines in one paragraph the cost is O(k·n). Measured in
> release on `[Code("x"), Emph([Code("x"), Code("y")]), Text("a")]` repeated
> `n` times: 27ms / 102ms / 419ms / 1.66s at n = 8k/16k/32k/64k — 4.0x per
> doubling against the pre-decline renderer's exact 2.0x. The single-child
> control (`Emph([Code("x")])`, an equal-length splice `Vec::splice` fills
> without shifting anything) stayed linear at 5.5 / 12.5 / 23 / 49ms over the
> same range, which is what isolates the growing splice from the rollback and
> from `run_end`.
>
> This one is unlike the other two costs this branch recorded: both of those
> are bounded by `MAX_INLINE_DEPTH`. Nothing bounds how many inlines an adapter
> puts in a single `Block::Para`, and this writer sits behind the
> EPUB/PDF/DjVu/MOBI untrusted-input boundary, so it is reachable from a
> hostile file.
>
> The view is now two vectors split at the cursor: `scanned`, the consumed
> prefix in forward order, and `pending`, the unscanned remainder held
> **reversed**. A decline truncates `pending` to the run's own slot and pushes
> the children back on reversed, which is O(children) and never touches the
> tail. `scanned.len()` *is* the cursor's logical index, so the checkpoint
> stack keeps the same plain indices into the same logical view this section
> describes; a rollback moves the predecessor's items back across the split,
> O(that run's own length). `run_end` and `next_class` grew iterator forms
> (`run_len`, `next_class_of`) so the reversed half asks the same code the
> forward half does, rather than keeping a second copy of either walk in step
> by hand. Steps 1-3 above are otherwise unchanged, and so is every word of the
> reasoning around them.
>
> Output is unchanged, which is the only thing that lets this land against a
> branch whose whole evidence base is measured counts: a differential render
> against the pre-fix commit over 1,518,937 renders — the census alphabet at
> lengths 3 and 4 across all 128 ledgers, a 13-element multi-child alphabet at
> lengths 3 and 4, and the breadth shapes themselves — diverges **zero** times.
> The same four breadths now read 7.4 / 14.6 / 26.8 / 54.4ms, 2.0x per
> doubling, matching the control. Guarded by
> `crates/kasane-writer/tests/inline_breadth.rs`, which is red on the pre-fix
> commit (3.28x at the first doubling) and green after.
>
> **What the census could not have caught.** Every container in the census
> alphabet has exactly one child, so every census decline splices one slot into
> one slot. Instrumented over that alphabet at lengths 3 and 4 across all 128
> ledgers: 61,864 declines, **zero** growing splices. The defect was
> structurally invisible to the branch's own primary instrument, which is why
> the new test carries its own multi-child corpus.

### 3.3 Termination

Each decline permanently removes at least one emphasis container from `items`,
replacing it with its children. Containers in a document are finite, so
declines are finite. Between two declines the loop advances `i` monotonically.
Therefore the loop terminates.

The empirical half of that argument is §2.2's length-6 row: 47,045,881 shapes,
every combination of the alphabet including every adversarial alternation of
declining runs and backtick-bearing neighbours, all completed.

## 4. What it costs

The 32 shapes move from `census-known-corrupt.txt` to
`census-known-structure-corrupt.txt`. `census-known-corrupt.txt` becomes
**empty**.

That is a promotion, and the census's own architecture is what says so:
`classify_with` returns `Clean` when the text is corrupt, because per-character
structural alignment presupposes equal strings. A text-corrupt shape is
therefore *structurally unclassified* — not clean, unexamined. Fixing its text
is what makes the structural question answerable at all, and these 32 answer it
badly. They join 1,698 shapes already queued under the same verdict.

Measured deltas, from a real bless under B:

| file | base | head | delta |
|---|---|---|---|
| `census-known-corrupt.txt` | 32 | **0** | −32 |
| `census-known-structure-corrupt.txt` | 1,698 | 1,730 | +32 |
| `census-inexpressible.txt` (entries) | 1,984 | 1,984 | 0 |
| `census-permanent-count.txt` | 1,984 | 1,984 | 0 |

The permanent file does not move, and that matters more than its zero suggests:
the queue/permanent split is computed on every bless and never hand-edited, so
this is the census stating that none of the 32 meets the permanent conditions —
not a promise anyone made. The claim that went wrong for 748 shapes at once is
the one this item does not touch.

## 5. The ratchet repair

`mise run census-ratchet` fails under B, on the queue gate and on the union:

```text
set          base     head    delta   verdict
text           32        0      -32   ok
queue        1698     1730      +32   FAIL -- 32 added
perm         1984     1984       +0   ok
union        3682     3714      +32   FAIL -- 32 added
```

This is not the item misbehaving. It is a blind spot in the ratchet, and it is
the exact mirror of the ledger branch's most transferable finding.

### 5.1 Why the union is wrong as built

The union is `queue ∪ permanent` — it does not include the text file. Its
stated premise is "a shape may move between the two files, but none may become
corrupt that was not." But a text-corrupt shape is structurally unclassified
(§4), so it is outside the union entirely, even though it is in the **worst**
state the census can record. Fixing its text moves it from invisible to
counted, and the union reads a strict improvement as +32.

§2b.5 of the ledger spec records structural gates staying silent through
thousands of text losses. This is the same defect from the other side: a
structural gate crying regression at a text fix. Both follow from the two tiers
being *ordered* while the gates treat them as peers.

**Repair:** the union becomes `text ∪ queue ∪ permanent`. Measured: 3,714 →
3,714, **+0**. It still forbids a shape becoming corrupt that was not — a newly
text-corrupt shape is a shape the union never held.

### 5.2 The queue gate

The queue gate is the direct rule that the structure queue may not grow. It
needs the same relaxation the union already grants queue↔permanent moves, and
for the same reason: this is reclassification, not regression.

**Repair:** admit `queue_added \ text_removed`. A shape may enter the structure
queue only if that same shape left `census-known-corrupt.txt` in the same
commit.

The premise is verified, not assumed — over this item's own bless, with
`comm` on the sorted files:

```text
text_removed = 32   queue_added = 32
queue_added not justified by a text removal: 0
text_removed that did not enter the queue:   0
```

**It preserves the one case on record where only the queue gate spoke.** In
ledger §2b.4 a shape entered the queue while the text file stood unchanged at
32; `text_removed` is empty there, so every queue addition is unjustified and
it still fails. The relaxation is scoped exactly to the direction that is an
improvement, and that direction is identifiable by set membership rather than
by judgement.

### 5.3 What stays as it is

The permanent file remains reported and ungated, and `census-permanent-count.txt`
remains the asymmetric ceiling a bless may lower and never raise. Nothing in
this item touches the claim those two guard.

## 6. The length-4 text tier

A new test target renders every sequence of length 4 over `census_support`'s
19-element alphabet, recovers the text with the shared oracle, and asserts the
corrupt count is **zero**.

No allowlist, no bless, no ratchet interaction, because the measured answer is
0 and this item is what makes it 0. That is a materially tighter assertion than
the length 1–3 census can make, and it is why the tier is worth shipping rather
than archiving: it cannot rot into stale excuses, because it has no file to rot
into.

It reuses `census_support::{alphabet, text_is_clean}` rather than restating
them, on the same reasoning the module's own doc gives: a tier must measure
with the census's own instrument or it drifts.

**Why length 4 specifically.** Ledger §2b.5 is the branch's most transferable
finding — `structreg` measured 0 in every row of every table while text losses
ran into the thousands, because the census stops at length 3 and the losses
lived at length ≥ 4. A guard at 4 is the smallest one that would have spoken.
§2.2's lengths 5 and 6 are evidence for this spec and are not shipped; they cost
minutes, not seconds.

**Cost:** 130,321 shapes, ~2.3s debug against a ~7.9s workspace suite. Recorded
here because it is a real +29%, and because ledger Risk 3 is the precedent for
measuring a tier's wall-clock before committing to it rather than after.

> **Corrected, 2026-08-22.** Both halves of that figure were wrong, in the same
> direction. Measured after the tier shipped, by running `cargo test
> --workspace` with and without `census_len4.rs` and summing the per-binary
> times libtest reports (both configurations excluding `inline_breadth.rs`,
> which the same wave added): the final review of this branch got a tier cost of
> ~2.6-2.9s against a ~6.35s baseline; the fix wave that followed re-measured
> on its own machine and got 3.05s against 5.68s (8.73s vs 5.68s, two runs
> each, spread under 0.05s). So the tier is **+43% to +54%**, not +29% — a
> larger absolute delta than the ~2.3s predicted *and* against a smaller
> baseline than the 7.9s assumed. Do not attribute the whole gap to the
> baseline: normalising the measured delta back onto this section's own 7.9s
> figure still gives +33% to +39%. Machines differ, which is why the ratio is
> stated as a range and the method is stated with it.
>
> The verdict is unchanged and this is not a re-litigation of the tier: it
> buys an exhaustive text guard one length past where every previous
> instrument stopped, which is the length the abutment ledger's losses lived
> at (ledger §2b.5). The point is that a price paid is worth stating
> accurately, per this plan's own Task 5 Step 4 — a materially larger number is
> worth reporting rather than absorbing.

This tier is also the reason §5's repair can be trusted going forward: with a
text gate at length 4 in CI, a future change that trades text for structure
fails loudly instead of arriving as a queue growth someone has to adjudicate.

## 7. Documentation this item falsifies

These are records that will assert the opposite of what ships. They are
corrected in place, not deleted — the same treatment
`census-inexpressible.txt`'s header got on 2026-08-17.

- **`emphasis_run`'s decline-branch comment.** Its closing argument is that the
  exposed seam is left unscanned on a measured claim, that no shape is corrupt
  *only* because of this decline, and that "a future shape corrupt only through
  this seam would not be caught by anything here." This item scans the seam and
  ships the guard. The comment's worked example —
  `[Code("x"), Emph([Code("x")]), Text("a")]` printing `` `x``x`a `` — stays,
  because it is now the description of what the rescan fixes.
- **`2026-08-15-emphasis-seam-design.md` §8's residual bullet.** It names the
  32 shapes, and it predicts a rescan closes all of them. §2.3 measured 16.
  Corrected with the number and a pointer here.
- **AGENTS.md**, two places: the four-rules paragraph in the `kasane-writer`
  entry, and the census bullet under Conventions that describes
  `census-known-corrupt.txt` as a live ratchet with entries in it.

## 8. Tests

- **The census, blessed.** `census-known-corrupt.txt` empty; the 32 in the
  structure queue. The bless diff is the evidence a reviewer reads.
- **The length-4 tier** (§6), asserting zero.
- **A rollback/`Pos` test.** Risk 1 in §10. A shape whose declined run sits at
  `Pos::LineStart` and after a `FootnoteRef`, checking the restored position
  drives escaping the same way an un-rolled-back one does. This is the one
  behaviour §2's sweeps cover only incidentally, because the census renders
  every shape as a bare paragraph.
- **A cascading-decline test**, pinning §3.3: a shape where the re-rendered
  predecessor itself declines, so the stack pops twice. Named because a single
  saved slot passes every other test in this list.
- **`mise run census-ratchet`** green under §5's repair, plus a check that a
  simulated §2b.4 move (a queue addition with the text file unchanged) still
  fails.

## 9. Non-goals

- **The 1,730 structurally-queued shapes.** This item leaves the structure
  queue larger than it found it, by exactly the 32 it promoted.
- **The `_` alphabet.** The next item in the residual program, ~2,198 shapes,
  and the one that must also price whether `_` breaks shapes clean today.
- **`Ctx::Cell` / `inlines_to_html`**, and block structure. Still unmeasured.
- **Approach E** (§2.4), blocked by the flanking rule's own circularity.

## 10. Verification and risk

`mise run lint && mise run test`, then `mise run census-ratchet`. Both of the
first two are already green under the probe (§2.5); the third is green only
with §5's repair, and that pairing is the point — the code change and the gate
change land together or the gate is measuring the wrong thing.

Risks, in the order they deserve worry:

1. **The rollback against `Pos`-sensitive escaping.** `Pos` has three states,
   not two, because a `[^n]` that opened a line makes a following `:` a
   footnote-definition delimiter. The checkpoint restores it, but the evidence
   is a passing suite rather than an argument, and §2's sweeps render every
   shape as a bare paragraph — so the `LineStart` and `AfterFootnoteRef` states
   are barely exercised by 47M shapes. Mitigation: §8's targeted test. This is
   the risk most likely to turn a text fix into a different text bug.

   > **Qualified, 2026-08-21.** §8's targeted test
   > (`a_rollback_restores_the_escaping_position_on_a_genuine_predecessor_re_render`)
   > pins the restore only under a non-shipped ledger
   > (`Ledger::from_bits(cell::EMPH_BESIDE_STRONG_RUN_SEAM)`), not under
   > either shipped one. Under any shipped ledger, `pos = ppos` appears
   > unobservable: `run_end` fuses any touching `Emph`/`Strong` pair under
   > `LICENSED` and `CONSERVATIVE` alike, so the rollback's
   > `predecessor_is_emphasis` disjunct — the only branch that genuinely
   > re-renders a `Pos`-aware predecessor — is unreachable there, and on the
   > other disjunct the re-rendered predecessor can only be a backtick or a
   > degrading-math run, whose escaping (`escape::code_span`/`math_span`)
   > takes no `Pos` argument at all and so cannot consume it. The mitigation
   > is real, and the test is the only place this repo has ever measured the
   > restore firing — but it measures a cell this writer does not ship, so
   > this risk's own scope is narrower than its mitigation implied.
2. **"Zero" is scoped to a 19-element alphabet**, not to all IR. §2.2's rows
   are exhaustive over that alphabet and say nothing about text outside it. The
   property tier remains the only guard there. This scope statement is load-
   bearing rather than modest: `census-inexpressible.txt` spent months asserting
   "Markdown cannot express" when it meant "this alphabet cannot express," and
   88% of it was wrong.
3. **Suite wall-clock, +29%** (§6). Mitigation: measured before committing, and
   the tier is text-only at a single length for exactly this reason.

   > **Corrected, 2026-08-22.** +43% to +54%, measured after the tier shipped;
   > see §6's own correction block for the figures and the method. The
   > mitigation stands and so does the tier — the correction is to the price,
   > not to the decision. What it does cost this entry is the weight it put on
   > "measured before committing": the pre-commit figure was ~1.5x low on the
   > delta and ~1.3x high on the baseline, so a design-time measurement is
   > evidence that the cost is bounded, not evidence of what the cost is. The
   > figure that binds is the one taken after, against the assembled suite.
4. **Termination.** Argued in §3.3 and swept to length 6. The argument depends
   on the splice strictly removing a container; a future edit that has
   `Declined` hand back anything containing the run's own members would break
   it silently. Mitigation: §8's cascading test, and the enum's shape — the
   variant carries children, not members.
5. **The measurement disagreeing with §2.** It already did once, which is why
   this spec exists rather than the parent's §8 being implemented as written.
   Per the parent's rule the measurement wins, and the archived counts get
   corrected in the same commit that records the new ones.

**One risk this list cannot rank, recorded because the last two items both hit
it.** Both the head/tail asymmetries this repo has measured were invisible to
the instrument in use at the time and were found only by widening the corpus —
the ledger's by going to length 4, this one's by dumping shape *sets* rather
than counts and diffing them. A count that improves is not evidence about which
shapes improved.
