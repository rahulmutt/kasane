# kasane — Length-4 Structural Tier Design Spec

**Status:** implemented on branch `len4-structural-tier` (PR #50). §6.1's
deliberate gap was closed afterwards on branch `len4-union-gate-case` — see
that section.
**Date:** 2026-08-23.
**Closes the follow-up named by:**
`2026-08-23-delimiter-choice-ordering-design.md` §6.1.
**Falsifies documentation in:** `census_len4.rs`'s module doc, `census.rs`'s
module doc, `AGENTS.md`'s census entry. See §7.

## 1. Purpose & scope

**No shipped gate prices emphasis structure above length 3.** `census.rs`'s
structural tier stops at length 3, and `census_len4.rs` is a **text**-only
tier. This item adds the structural tier at length 4.

The gap is measured, not theoretical. The delimiter-choice-ordering branch
introduced **135 structural regressions at length 4 and 3,134 at length 5** —
shapes that moved `Inexpressible → Corrupt`, so a `<strong>` came back as an
`<em>` and its text migrated across a structural boundary. Every text tier
stayed at zero, because the text was byte-identical either way. The 2.48M-shape
length-5 text sweep stayed at zero. The defect was caught **only** because that
family happened to also have a length-3 member, which `census.rs`'s structural
ratchet refused. A family that started at length 4 would have shipped silently
(`2026-08-23-delimiter-choice-ordering-design.md` §1.1).

This item ships **only the guard**. It does not fix any of the 41,443 shapes it
newly names. See §9.

## 2. What length 4 actually holds

Measured 2026-08-23 at `97b2604` (main), debug profile, by a throwaway probe
calling `census_support::classify_with` with `Ledger::LICENSED` over the
odometer enumeration `census_len4.rs` already uses:

| | count | share |
|---|---|---|
| shapes (19⁴) | 130,321 | |
| `Clean` | 78,725 | 60.4% |
| `Corrupt` | **41,443** | 31.8% |
| `Inexpressible` | **10,153** | 7.8% |

Cost and weight, same run:

| | measured |
|---|---|
| structural pass, **debug** (CI's profile) | **5.6s** |
| existing length-4 *text* tier, debug | 3.75s |
| queue file | 41,443 lines, 3.63 MiB (**142 KB** gzipped) |
| permanent file | 10,153 lines, 0.86 MiB (**35 KB** gzipped) |
| longest shape line | 182 bytes |

Two of these numbers settled decisions the design would otherwise have had to
guess at, and both went against the intuition:

- **Git weight is not the objection it looks like.** 4.5 MiB of working tree is
  ~177 KB of compressed object. The reason to hesitate about a per-shape file at
  this scale is bless-diff review noise, not repository size, and noise is the
  cost §3 accepts deliberately.
- **Release is not required.** §6.1 costed the tier at "roughly a second in
  release", and `census_len4.rs`'s own doc gives cost as the reason lengths 5
  and 6 are not shipped. But CI runs `mise run test`, which is `cargo test
  --workspace` in **debug**, and the tier is 5.6s there — the same order as a
  tier already shipped in that lane. Nothing needs a release profile, a separate
  job, or a feature gate.

Lengths 5 and 6 stay unshipped and this item does not revisit that: at 2,476,099
shapes, length 5 costs minutes, and §6.1 names 4 as the smallest guard that
would have spoken.

## 3. Three files, on the length-3 contract

| file | contents |
|---|---|
| `census-len4-known-structure-corrupt.txt` | 41,443 shapes — the queue |
| `census-len4-inexpressible.txt` | 10,153 shapes, under a generated header |
| `census-len4-permanent-count.txt` | `10153` — the ceiling |

Every rule is the length-3 rule, deliberately and without variation:

- The queue is a **ratchet, not an acceptance**, checked two-directionally. A
  corrupt shape absent from the file fails, so a regression cannot ship quietly.
  A listed shape that is no longer corrupt fails too, so the file cannot rot into
  stale excuses.
- The permanent file's header is **generated** from a constant, and its split
  into classes is **computed on every bless, never hand-edited**.
- The ceiling is a **ceiling, not a count**. A bless lowers it to match a shrink;
  raising it is a hand edit in the commit that needs it, so a permanence claim
  appears in the diff. `KASANE_CENSUS_BLESS=1` writes the shape files and stops
  there.

### 3.1 Why per-shape and not counts

A count-only gate — two integers, "corrupt may not rise, inexpressible may not
rise" — would have caught §1.1's defect exactly: corrupt +135, inexpressible
−135. It is a two-line diff, trivially reviewable, and it was seriously
considered.

It was rejected because it is **blind to a swap**. Ten shapes fixed and ten
different shapes broken nets zero in every count, and this program's changes
routinely move thousands of shapes in both directions at once — the
delimiter-choice branch improved 31,376 shapes at length 4 while regressing 135.
A gate that can only see the net cannot see that shape. The per-shape file is the
only form that cannot miss it, and the census's founding claim is that the
allowlist diff "is the exact list of shapes your change fixed or broke, which is
the evidence a reviewer wants."

### 3.2 What per-shape costs, stated plainly

Bless diffs in the tens of thousands of lines. The delimiter-choice branch alone
improved 31,376 length-4 shapes to `Clean`, every one of which is a line deleted
from one of these two files, plus 135 lines moved between them. **Nobody will read
that diff line by line**, and this spec does not pretend otherwise: what a
reviewer reads is `mise run census-ratchet`'s per-set delta table and the first
ten shapes of any gated growth, both of which already exist. The file's value is
that it makes a swap *impossible to hide*, and that it stays greppable — the same
`grep -c 'Emph(\[Strong'` that pins the length-3 permanent split works here.

### 3.3 Why the permanent file keeps a ceiling at length 4

`Inexpressible` is decided by a mechanical predicate, not a human judgement, so
one could argue the ceiling is redundant: laundering 41k queue entries into
permanence would require changing that predicate, and a reviewer sees a code
change. That argument is equally true at length 3, and the ceiling was built
anyway, because permanence is **the one claim in this census that nothing
downstream re-examines** — the queue is worked item by item, permanence is read
as settled. It went wrong for 748 shapes at once in a single bless, and ~78% of
the permanent file turned out wrong on 2026-08-23. Ten thousand claims deserve
the same visible number that 433 do.

The cost is one hand-edited integer on the rare change that moves shapes
queue → permanent. That happened once at length 3 in a year.

## 4. Where the code goes

### 4.1 The ratchet helpers move to `census_support`

`ratchet`, `blessing`, and `permanence_ceiling` live in `census.rs` today, the
last two closed over hard-coded path constants. They move to
`census_support/mod.rs` and take their path as a parameter; `census.rs` calls
them from there and keeps its constants.

This is not tidying. Two independent copies of the ratchet could drift apart,
and if they did, one tier's gate would stop meaning what its documentation says
— the identical argument `context_walks_with`'s doc already makes for sharing
the render/gate/walk setup between `classify_with` and `census.rs`'s alignment
guard. `census_support/mod.rs` is described in `AGENTS.md` as "the oracle all of
this shares, extracted so a tier measures with the census's own instrument
rather than a copy that drifts". The ratchet is part of that instrument.

`blessing()` in particular must be spelled once: its own doc says two readers
disagreeing about what a bless is "would let one of them write while the other
asserts against the file it just changed". A second copy in a second tier is
exactly that hazard, one file further away.

### 4.2 The test is a second `#[test]` in `census_len4.rs`

Not a new test file. Tests within one binary run on parallel threads, while
separate test binaries run sequentially, so beside the text tier the structural
pass costs `max(3.75, 5.6) ≈ 5.7s` of wall clock rather than `3.75 + 5.6 =
9.3s` — a net **~+2s** to `mise run test`.

That figure is an expectation from two separately-measured passes, not a
measurement of the pair, and it assumes at least two free cores on the runner.
The implementation verifies it. If it does not hold, the honest cost is +5.6s
and the tier ships anyway; the placement decision does not depend on it.

Shapes are built by odometer, as the text tier already does — `shapes()` is
fixed at lengths 1–3 and is not extended, because a `Vec` of 130,321 shapes
materialized up front is a cost the odometer does not pay.

### 4.3 The instrument guards are not duplicated

`census.rs` carries two guards that are not about the writer at all: that the
context walk reproduces `rendered_text`, and that the two walks align character
for character. Both check that the *instrument* has not drifted — that nobody
edited `ir_context` or `rendered_text` without the other. That is a property of
the projection, not of the sequence length, and length 3 already exercises every
alphabet element in every position. They stay where they are, and this spec
records the reasoning so a later reader does not read their absence as an
oversight.

## 5. The cross-revision half

`mise run census-ratchet` gains the two new shape files under a **second
union**, `queue4 ∪ perm4`, gated the same way the length-3 union is: no shape
may enter it by any route. The two sets are reported but not individually gated,
and the ceiling's no-gratuitous-raise check is mirrored for
`census-len4-permanent-count.txt`.

### 5.1 There is no length-4 analogue of the queue-promotion rule

At length 3 the structure queue may grow, but only by shapes that left
`census-known-corrupt.txt` in the same commit: text-corrupt → structure-corrupt
is a promotion, since text is the invariant and the span boundary is not.

**That rule has no length-4 form, because there is no length-4 text file.** The
length-4 text tier asserts zero and carries no allowlist by design — its own doc
says it "cannot rot into stale excuses, because it has no file to rot into". So
the length-4 union is two files, not three, and every queue growth is gated by
the union alone. This is not a weakening: the promotion rule exists to *permit* a
growth the union would otherwise forbid, and at length 4 there is no such growth
to permit. If the length-4 text tier ever stops asserting zero, that is a
regression which fails the tier outright, and this rule is revisited then.

### 5.2 What catches `Inexpressible → Corrupt`

The union does not — the shape was already in it. What catches it is the tier's
**own per-shape ratchet**, failing with "N shape(s) newly structurally corrupt"
because the file does not list them. That is precisely the mechanism that spoke
at length 3 in §1.1, and §6 verifies it fires at length 4.

### 5.3 One bless command

A new `mise run census-bless` runs both binaries under `KASANE_CENSUS_BLESS=1`.
Blessing is currently one command naming one binary; adding a second tier makes
it possible to bless one and leave the other stale. That failure is loud — CI
fails on the un-blessed tier — but the task removes the trap rather than relying
on the alarm, and gives `AGENTS.md` one command to name instead of two.

## 6. Proving the tier bites

A clean run of a new guard proves nothing unless the guard can be shown to
produce a failure. The verification:

Check out `05bb516` — delimiter choice before the splice, condition 4 not yet
landed — in a `git worktree`. Copy in this branch's `census_len4.rs` and
`census_support/mod.rs`, and pin the three baseline files from `HEAD`. Run the
tier.

**Expected: the queue ratchet fails, naming ~135 shapes.** `ratchet`'s first
assertion panics before its second, and before the permanent file's `ratchet`
call runs at all — so one run reports one direction, never both. The permanent
side is confirmed by blessing inside the worktree and diffing the two revisions'
files: ~135 lines added to the queue and the same ~135 removed from the
permanent file, which is `Inexpressible → Corrupt` shown shape by shape.

That short-circuit is now recorded on `ratchet`'s own doc, so the next reader
does not design a verification around a failure mode it cannot produce.

`census_support/mod.rs`, `kasane-ir`, and `kasane-writer`'s public surface are
byte-identical between `05bb516` and `97b2604` — only `choose_mark`'s body
differs — so the copied instrument measures the older writer without
contamination. That is the same invariant `zz_structural_len4_5.rs`'s doc
records for the archived sweep.

Evidence lands in `docs/superpowers/evidence/2026-08-23-len4-structural-tier/`,
including the failure output and the shape list.

### 6.1 The gap this leaves, stated rather than hidden

`ratchet_gate_cases.sh` is **not** extended. It exercises the census-ratchet
task's *negative* direction — injecting an unjustified queue growth and failing
if the gate accepts it — and it will keep doing so for the length-3 queue only.

So the length-4 union gate added in §5 will only ever run in its passing
direction in CI. That is the silent-gate failure mode this repo has recorded
twice, and the reason `ratchet_gate_cases.sh` exists. §6 proves the *tier* bites
once, on this branch, at a fixed revision; nothing keeps proving the *task's*
new union bites.

This is a deliberate scope decision, taken with the gap known. Closing it is a
one-case extension of an existing script and remains available as a follow-up.

**Closed 2026-08-24**, on branch `len4-union-gate-case`, exactly as predicted:
one more direction in `ratchet_gate_cases.sh`, injecting a shape outside the
census alphabet into `census-len4-known-structure-corrupt.txt` and requiring
`union4 ... FAIL -- 1 added`. Evidence:
`docs/superpowers/evidence/2026-08-24-len4-union-gate-case/`, including the
runs that prove each new assertion can fail. The paragraphs above stay as
written — they were
true of *this* branch, and the reasoning for taking the gap knowingly is worth
more on record than a rewrite that pretends it never existed.

Two things surfaced in the closing that this section did not predict. First,
the existing direction asserted only that `census-ratchet` **exited non-zero**,
which cannot distinguish the gate under test from any of the other seven rows
that fail the same task — so the extension was not additive: both directions
had to start matching their own row. Second, a `union4` that skips for want of
a baseline catches nothing, so the *passing* direction now refuses a skipped
row and says to rebase, rather than letting the negative direction report a
gate proven that never ran.

Still uncovered, and stated for the same reason §6.1 was: the length-3 union
gate and both ceilings' no-gratuitous-raise check are still only ever seen
passing.

**Closed 2026-08-25**, on branch `ceiling-and-union3-gate-cases`, as four more
directions in the same script — one per gate, plus one that is not a gate case
at all. Two things the paragraph above did not anticipate.

First, the length-3 union cannot be exercised the way the length-4 one was. A
shape appended to the queue trips `queue+` as well, and a row that fails
alongside another proves only that *something* spoke. The injection goes into
the **permanent** file instead, where `perm` is report-only and no other gate
reads the file, leaving `union` as the only gate that can speak. Direction 2's
own table is the evidence for the alternative being unusable: it shows `queue+`
and `union` failing together on the same injection.

Second, the ceilings needed a direction that asserts a **pass**. Alone among
these gates, `ceiling_check`'s predicate has two terms — `raised` **and**
`nothing moved in` — so driving it into failure exercises only the first. A
check that had lost `&& [ "$grew" -eq 0 ]` would reject every raise, including
the legitimate ones the ceiling exists to make reviewable, and both failure
directions would stay green. Direction 7 models a real promotion (one shape out
of the queue into the permanent file, ceiling raised by one) and requires the
whole task green. That is measured rather than argued: the evidence directory
carries a run per gate-break, and the `grew`-term break is red *only* at
direction 7.

Both facts generalise past this script. A negative direction is worth what its
isolation is worth — which is why every direction here matches its own row or
line — and a gate with a compound predicate needs a positive direction as well,
or half of it is untested. Evidence:
`docs/superpowers/evidence/2026-08-25-ceiling-and-union3-gate-cases/`.

## 7. Documentation this item falsifies

Each of these is false the moment the tier lands, and each is corrected in the
same commit as the code:

- **`census_len4.rs`'s module doc**, opening and final paragraph. The opening
  ("The text tier at length 4, asserting zero... This one carries none, and
  cannot rot into stale excuses, because it has no file to rot into") is the
  first thing a reader meets, and it is false the moment the structural tier's
  three files exist. The final paragraph: "And this tier is text-only, which
  is its own gap… A structural length-4 tier is the named follow-up". Both are
  rewritten to describe the tier that now sits beside it, keeping the recorded
  history of *why* the gap existed.
- **`census.rs`'s module doc**: "There are two tiers, and three files."
- **`AGENTS.md`**'s census entry: "The census has two tiers, and four files";
  the description of `census_len4.rs` as "the text tier at length 4"; the
  three-gate description of `mise run census-ratchet`; and the bless command,
  which becomes `mise run census-bless`.
- **`2026-08-23-delimiter-choice-ordering-design.md` §6.1**: "It is a named
  follow-up, not part of this branch." Gains a status line recording that it
  landed and where — the sentence stays true of *that* branch and must not be
  rewritten to pretend otherwise.

## 8. Tests

- **The tier itself**, two-directionally, over 130,321 shapes.
- **The permanence ceiling**, asserted after both ratchets so an unlisted shape
  is reported by the specific error rather than by the ceiling's.
- **The generated-header claim.** `census.rs` carries
  `the_permanent_file_holds_exactly_the_five_condition_four_refusals`, which
  exists because the header's class split is hand-written prose inside a
  generated file, and a hand-edit there passes the `#`-filtering checker and is
  silently reverted by the next bless. The length-4 header carries the same
  hazard and gets the same kind of test: whatever class split its header states
  is asserted against the entries. The length-4 split is computed during
  implementation and written into both places at once.
- **No new instrument guards** (§4.3).
- **No extension of `ratchet_gate_cases.sh`** (§6.1). *(Scope of this branch,
  left standing as written. The extension landed 2026-08-24 on
  `len4-union-gate-case`; §6.1 records what it took.)*

## 9. Non-goals

- **Fixing anything.** 41,443 corrupt shapes become visible; none are fixed.
  This item ships a guard, and a guard that also changes the writer cannot be
  trusted to have measured the writer.
- **Length 5 or 6.** Minutes, not seconds. Unchanged.
  *(Landed 2026-08-26 as `census_len5.rs` and `census_len6.rs` —
  `2026-08-26-length-5-6-novelty-tier-design.md`. The sentence above stays as
  written: it was true of this branch, and "minutes, not seconds" was the
  honest reading of a debug-profile measurement at the time.)*
- **Widening the census alphabet.** The 19 elements are the census's own, and
  zero at length 4 says nothing about text outside them — the property tier
  remains the only guard there. That scope statement is load-bearing:
  `census-inexpressible.txt` spent months asserting "Markdown cannot express"
  when it meant "this writer does not express".
- **Revisiting the permanence *predicate*.** Whether
  `nests_same_class_directly`/`nests_strong_over_emph_directly` scoped to the
  whole shape rather than the mismatching position is right at length 4 is a
  real question — `2026-08-16-structural-census-design.md` §8 flags the scoping
  as a residual risk "once the alphabet stops being single-child-only" — but it
  is a question about the relation, not about the tier that reports it.

## 10. Verification and risk

**The measurement this spec rests on** is a single throwaway probe run
(§2). The implementation reproduces every figure from the shipped tier itself
before the files are committed; if the shipped tier disagrees with the probe on
any count, the probe is wrong and this spec's numbers are corrected rather than
the tier's.

**Risk: the permanent file's split is guessed.** §8's header test needs the
length-4 class split, and this spec does not state it — 10,153 entries have not
been broken down by class. Computing it is implementation work, and the risk is
that the classes at length 4 are not the two the length-3 header names. If they
are not, the header constant describes what is actually there; it is not forced
into the length-3 shape.

**Risk: `~+2s`, not `+5.6s`, is an expectation.** §4.2. Verified in
implementation; the tier ships either way.

**Risk: bless-diff noise makes the queue file unreviewed in practice.** Accepted
and stated in §3.2. The mitigation is that nothing about the file *requires*
line-by-line review to work — it is a set-equality assertion, and the delta table
plus the first-ten-shapes output is the human-readable surface.

**What would falsify the item.** If the tier, run against `05bb516`, does *not*
fail — or fails on a count materially unlike 135 — then either the tier does not
measure what §1.1 measured or the archived sweep was wrong. Either way the item
stops until that disagreement is resolved, because a guard whose one
demonstration of biting did not bite is not a guard.
