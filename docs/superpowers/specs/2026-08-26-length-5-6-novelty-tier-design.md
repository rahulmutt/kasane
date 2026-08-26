# kasane — Length-5/6 Novelty Tier Design Spec

**Status:** designed 2026-08-26; not yet implemented.
**Date:** 2026-08-26.
**Closes the standing item:** "lengths 5 and 6 stay unpriced for structure as
well as text" — `AGENTS.md`'s census entry,
`2026-08-23-length-4-structural-tier-design.md` §9,
`2026-08-21-declined-run-rescan-design.md` §2.2.
**Falsifies documentation in:** `AGENTS.md`'s census entry, `census_len4.rs`'s
module doc. See §7.

## 1. Purpose & scope

Lengths 5 and 6 have been unpriced for structure since the census existed, and
the recorded reason is cost: *"minutes, not seconds."* This item prices them,
and ships a guard at both.

**The recorded reason was measured against the wrong profile.** `mise run test`
builds debug; the census tiers are pure compute and the profile dominates. At
length 4 the same binary takes **5.64 s** in debug and **0.72 s** in release.
Length 5 in release is **25 s**, which is seconds, not minutes. Length 6 is
genuinely minutes — ~5.4 min for the structural walk — and that distinction is
what splits the two lengths across two venues in §5.

Cost was never the binding constraint anyway. §2 is.

This item ships **only guards**. It fixes none of the 1,204,312 non-clean
length-5 shapes it prices. See §9.

## 2. What lengths 5 and 6 actually hold

Measured 2026-08-25/26 at `699b471` (main), **release** profile, by throwaway
probes calling `census_support::classify_with` and `census_support::text_is_clean`
with `Ledger::LICENSED` over a base-19 odometer generalising the one
`census_len4.rs` already uses. The probes were deleted; §10 states what happens
if the shipped tier disagrees with them.

| | length 4 | length 5 | length 6 |
|---|---|---|---|
| shapes | 130,321 | 2,476,099 | 47,045,881 |
| text-corrupt | 0 | **0** | **0** |
| structure queue | 41,443 | **983,694** | *unmeasured* |
| structure permanent | 10,153 | **220,618** | *unmeasured* |
| union (non-clean) | 51,596 | **1,204,312** | **26,501,436** |
| **novel** (§2.1) | **0** | **0** | **0** |
| release walk | 0.7 s | 25 s | ~5.4 min |
| per-shape file, if written | 4.7 MB | **136 MB** | ~2.6 GB |

The length-6 queue/permanent split is **not measured**, only their union. This
spec does not guess it — the same refusal
`2026-08-23-length-4-structural-tier-design.md` made about its header split. §4
is why no length-6 figure needs it.

### 2.1 The novelty relation

A shape of length *n* is **novel** when it is non-clean and all *n* of its
single-deletion sub-shapes, of length *n−1*, are clean.

Novelty is **zero at every length measured** — 4 against ≤3, 5 against ≤4, 6
against ≤5. Corruption in this alphabet does not *originate* above length 3.
Every non-clean shape at 4, 5 and 6 has a non-clean single-deletion sub-shape,
so it belongs to a family already visible to a shipped tier.

**Deletion, not contiguous substring, and that is measured rather than
chosen.** Of the 1,204,312 non-clean length-5 shapes, all 1,204,312 have a
non-clean single-deletion sub-shape but only **1,204,044** have a non-clean
contiguous one. A substring relation would report 268 false novelties on a
clean tree. Deletion is the relation that is actually zero.

### 2.2 Why this kills the per-shape file

`2026-08-23-length-4-structural-tier-design.md` §3.1 rejects count-only gates
because they are **blind to a swap**: ten shapes fixed and ten broken nets zero.
That argument holds wherever the set's members carry information about the
length they are filed under.

At length 5 none of them do. Every member of that 1,204,312-shape set is
derivable from lengths 1–4 by §2.1, so `census-len5-known-structure-corrupt.txt`
would be a 112 MB index of the length-4 tier rather than evidence about length
5 — rewritten whole on every bless, and asserting nothing the length-4 tier does
not already assert per-shape. The file is rejected on **what it would record**,
not on its size. Size is why the question was asked.

What §3.1's argument does still buy is covered in §3 by three counts.

## 3. What each length asserts

| | length 5 | length 6 |
|---|---|---|
| text tier | assert 0, no file | assert 0, no file |
| novelty | assert 0, no file | assert 0, no file |
| counts | `census-len5-counts.txt` | **none** |
| blessed by | `mise run census-bless` | nothing to bless |
| venue | PR CI, release | weekly + dispatch |

### 3.1 No allowlist, and what that costs

Both zero-assertions carry no file, on `census_len4.rs`'s text-tier contract
verbatim: *"the answer is zero and a file it does not have cannot rot into
stale excuses."*

`novel == 0` is a property of **this writer today**, not a theorem. If a
legitimate novel shape ever appears there is deliberately nowhere to put it: the
tier fails and stays failing. That is accepted, and it means the first such
failure is a **design item, not a bless**. The assertion's failure text must say
so in those terms — a message that reads like a bless prompt will get a file
added, which is the exact rot the missing file exists to prevent.

### 3.2 Why length 6 carries no counts

Counts go stale on every writer improvement. Length 6's bless costs ~5.4 min and
its venue is weekly, so its counts would be found stale **on main, up to a week
late** — worse than no counts at all, because a routinely-red weekly job stops
being read.

Zero-assertions do not have that property: zero stays zero under improvement.
Length 6 therefore commits **no files whatsoever** — nothing to bless, nothing
to rot, and the weekly run either passes or has found something real.

This also disposes of a wrinkle: `census-ratchet` resolves its base to HEAD on a
push to main, so its git-diff half is inert there. With no length-6 files there
is nothing for it to compare, and length 6's guard is entirely the tier's own
assertion — which works identically on main and on a branch.

### 3.3 The gap the counts close, and the one they do not

Novelty catches corruption that *originates* at length 5 or 6. It does not catch
a change that multiplies corruption **inside families already known at length
4** — no novel shape moves.

At length 5 the three counts cover that. At length 6 it is **uncovered**, and
stated rather than hidden; §3.2 is the trade.

## 4. Where the code goes

### 4.1 `census_support` takes the length-generic machinery

- **`for_each_shape(len, f)`** — the base-19 odometer, generalised.
  `census_len4.rs`'s `for_each_length_four_shape` becomes a call to it,
  **deleting** the second copy of the carry loop rather than adding a third.
  That is the drift `census_support` exists to prevent, and this item would
  otherwise be the change that proved the point by breaking it.
  `the_length_four_odometer_visits_every_shape` keeps guarding it at 130,321.
- **`nonclean_bitset(len)`** — non-clean shapes as a bitset keyed by base-19
  value. 19^5 is 310 KB; a string-keyed `HashSet` of `format!("{seq:?}")` — what
  the first probe used — is ~100 MB and materially slower, and the length-6
  check needs the length-5 set resident.
- **`novel_count(len, &shorter)`** — §2.1's predicate.
- **`counts(len)`** — `(queue, permanent, union)`.

### 4.2 Two thin binaries, `#[ignore]`d rather than excluded

`census_len5.rs` and `census_len6.rs` are **normal test targets** whose deep
tests carry `#[ignore]`. `mise run test` (debug) reports them as `ignored` with
their reason; two new release tasks run them with `--include-ignored`:

```
mise run census-len5   cargo test -p kasane-writer --release --test census_len5 -- --include-ignored
mise run census-len6   cargo test -p kasane-writer --release --test census_len6 -- --include-ignored
```

`mise run census-bless` gains a third line for `census_len5` — release, like the
task above, since a debug bless of that tier costs ~3.3 min for no benefit. It
gains **no** length-6 line: §3.2, there is nothing to bless.

**`test = false` in `Cargo.toml` was tried and rejected.** It does keep a target
out of `cargo test`, but a target so marked is invisible to
`cargo clippy --all-targets` as well: a file containing nothing but
`this is not valid rust at all !!!` passes **both** gates, exit 0. A tier that
stopped compiling would stay green until someone ran the weekly task by hand.
`#[ignore]` keeps the target compiled, linted, and visible in every test run.

### 4.3 `census-len5-counts.txt`

Three labelled lines:

```
queue 983694
permanent 220618
union 1204312
```

One file with three counts, diverging from the one-integer-per-file convention
on purpose: `census-permanent-count.txt` is one integer because it is one
**claim** — permanence, the claim nothing downstream re-examines. These three
are a summary read together, and `union`'s gate means nothing to a reader
without the other two beside it.

## 5. Venues

```
PR CI     Lint │ Test │ Census length 5 │ Census ratchet │ Census ratchet, negative directions
weekly    census-deep.yml → mise run census-len6
```

**Budgets, from §2 and subject to §10's runner caveat.** The length-5 step is
the structural walk (25 s) plus the text walk (10 s) plus the length-4 reference
bitset (~1 s): **~35 s**. The length-6 task is its own two walks plus the
length-5 reference bitset: **~10 min**.

**The length-5 step precedes `census-ratchet`, and that ordering is
load-bearing.** It is the existing argument — *"Run first, it could pass on
files the test was about to reject"* — applied to a new step. `census-ratchet`
compares committed counts across revisions and takes their **accuracy** on
trust. A hand-edited `union 1204311` sails through it, union having shrunk; only
the length-5 tier's own assertion catches the lie.

**`census-deep.yml`** follows `fuzz.yml`: `workflow_dispatch` plus `schedule` at
09:00 Monday, offset from `audit.yml` (07:00) and `fuzz.yml` (08:00) so the
three do not contend. It needs **no `fetch-depth: 0`** — with no committed
length-6 files there is no merge base to resolve. That is a simplification the
main job cannot have, not an omission from it.

Weekly cadence means a novel-at-6 regression surfaces on main up to a week late.
That is the bargain `fuzz.yml` already makes here, and it is stated so nobody
reads the tier as a PR gate.

## 6. The ratchet gates

`census-ratchet` gains three rows — `queue5`, `perm5`, `union5` — read from
`census-len5-counts.txt` by a `count_check` helper mirroring `ceiling_check`'s
shape. It skips while the file is absent at the base, exactly as `union4` does,
and the marker goes dead on the first merge base that has the file.

**Only `union5` gates.** That is the length-3/4 logic reproduced rather than a
new rule — there, *"the union is what makes the move safe to allow."* Permanent
growing while the union is flat **is** queue→permanent movement; permanent
growing while the union grows fails on the union. So the ceiling's two-term
predicate has no length-5 form, and needs none: there is no set difference to
compute "newly permanent" from, and nothing asks for one.

### 6.1 The negative direction

`ratchet_gate_cases.sh` gains **direction 8**: bump `union` in the counts file,
assert the **`union5` row** reports FAIL.

Asserting the row rather than the task's exit status is the recorded trap —
`census-ratchet` fails as a whole for any of its gated rows, and an unrelated
gate speaking first is indistinguishable from the one under test. `queue5` and
`perm5` get no direction because they do not gate, consistent with `perm`,
`queue4` and `perm4`.

**No positive direction.** `union5`'s predicate has one term; the ceiling was the
only two-term gate here, which is why it needed one. A permanently-skipping bug
is caught by direction 8 itself: a row that skipped cannot report FAIL.

## 7. Documentation this item falsifies

- **`AGENTS.md`'s census entry**: *"Lengths 5 and 6 stay unpriced for structure
  as well as text: minutes, not seconds."* Both halves go. Replaced by what
  shipped, including the release/debug distinction §1 rests on.
- **`census_len4.rs`'s module doc**, two sentences carrying "minutes, not
  seconds". The neighbouring claim — that the length-5/6 **text** sweeps ran and
  were zero — is **confirmed** by §2, not falsified; only the "not shipped"
  reasoning goes.
- **`2026-08-23-length-4-structural-tier-design.md` §9**: *"Length 5 or 6.
  Minutes, not seconds. Unchanged."* Gets a **status line**, not a rewrite. That
  is the convention `2026-08-23-delimiter-choice-ordering-design.md` §6.1
  established: the sentence stays true of *that* branch and must not be edited
  to pretend this work was in scope there.
- **`2026-08-21-declined-run-rescan-design.md` §2.2**, the standing call for the
  lengths 5 and 6 sweeps — same treatment.

## 8. Tests

- **The novelty predicate, proven to bite.** A tier asserting zero that has never
  been *seen* to fail is indistinguishable from one that always passes — the
  silent-gate mode this repo has recorded twice. The writer cannot be made to
  emit novel corruption to order, so the demonstration is on the predicate:
  `novel_count` over hand-built bitsets in which one length-5 shape is non-clean
  and all five of its single-deletions are clean, asserting it reports exactly 1.
  That pins the function that could otherwise return zero forever.
- **The deletion-vs-substring choice**, §2.1: a shape derivative only under
  deletion is counted derivative. Without this, a later "simplification" to
  substring matching passes every other test and starts reporting 268 novelties.
- **The odometers**, at 2,476,099 and 47,045,881. The length-4 odometer test's
  argument carries over unchanged: a truncated enumeration is invisible to the
  classifying tests whenever the dropped shapes are `Clean`.
- **`census-len5-counts.txt` against reality**, both directions.
- **Direction 8** (§6.1).

## 9. Non-goals

- **Fixing anything.** 1,204,312 non-clean length-5 shapes are priced; none are
  fixed. A guard that also changes the writer cannot be trusted to have measured
  the writer.
- **Per-shape files at length 5 or 6.** Rejected in §2.2 on what they would
  record, with a measurement.
- **Length-6 counts.** §3.2.
- **Length 7+.** 19^7 = 893,871,739. Unmeasured, unproposed.
- **Widening the census alphabet.** The 19 elements are the census's own, and
  zero here says nothing about text outside them; the property tier remains the
  only guard there. That scope statement is load-bearing —
  `census-inexpressible.txt` spent months asserting "Markdown cannot express"
  when it meant "this writer does not express".
- **Revisiting the permanence predicate.** Still a real question
  (`2026-08-16-structural-census-design.md` §8), still a question about the
  relation rather than about a tier reporting it.

## 10. Verification and risk

**This spec rests on throwaway probe runs** (§2), now deleted. The
implementation reproduces every figure from the shipped tiers before anything is
committed; if a shipped tier disagrees with the probe on any count, **the probe
is wrong and this spec's numbers are corrected rather than the tier's**.

**Risk: every cost figure is one run on one machine.** CI runners are slower. If
the length-5 step materially overruns on a real runner, §5's venue decision is
revisited rather than forced, and the implementation reports the CI-observed
number instead of this one.

**Risk: the zero-assertions have no escape hatch.** §3.1. Accepted, with the
failure-message requirement that follows from it.

**Risk: length 6 is uncovered for derivative volume.** §3.3. Accepted, named.

**Risk: `nonclean_bitset(5)` is resident during the length-6 walk.** 310 KB, so
the risk is not memory but that the length-6 task's cost is the length-5 walk
*plus* its own — ~6 min, before the length-6 text tier. §5's ~10 min budget
assumes one combined walk computing both tiers; if `classify_with` and
`text_is_clean` cannot share a walk economically, the figure rises and is
reported rather than hidden.

**What would falsify the item.** If `novel` is **not 0** at length 5 or 6 when
the shipped tier computes it, §2.2's whole representation argument collapses —
the case for dropping the 112 MB file *rests* on that zero — and the item stops
until the disagreement is resolved. A guard whose founding measurement did not
reproduce is not a guard.
