# Delimiter-choice ordering — design-phase probes

Archived apparatus for the measurements recorded in
`docs/superpowers/specs/2026-08-23-delimiter-choice-ordering-design.md` §2.

The spec reproduces every figure inline, so nothing here is load-bearing for
reading it. What lives here is the *apparatus*: the two throwaway patches and
their harnesses, kept because the first one overturned the item's premise and
the second replaced it with a measured number.

## Nothing in this directory is a test

None of it is compiled, run in CI, or maintained. Same convention as
`2026-08-18-abutment-ledger/` and `2026-08-21-declined-run-rescan/` beside it.

## The directory name is the disproved framing, kept deliberately

This item was called "the `_` alphabet" for six days. It is not an alphabet
item — see §2.3 of the spec — and the spec is filed under
`delimiter-choice-ordering`. The directory keeps the old name so that anyone
searching for the item by the name it was planned under lands on the evidence
that retired it.

## What was measured

### Probe 1 — `_` at the emission site (`probe-hook.patch`, `zz_underscore.rs`)

A hook at `markdown.rs:380`, the writer's single delimiter-emission site, able
to force any `*`/`_` assignment across a run. 23-element alphabet (the census's
19, plus `Text("_")`, plus three multi-child containers), 12,719 shapes of
length 1–3, five enclosing contexts, all 2^k assignments per shape.

**Result: 0 shapes fixed, in every context.** The prior estimate was ~2,198.

The cause is in the emission log, which counts containers present against
delimiters actually chosen: `Emph[Emph[a]]` arrives at the emission site with
`containers=2, emissions=1`. `splice_children` runs first and deletes the inner
container to avoid a collision with a character that has not been chosen yet, so
by the time a delimiter is picked there is only one container left to pick for.

The 2026-08-17 probe that produced the ~2,198 measured whether *CommonMark* can
spell these shapes. It can. It never measured whether *this pipeline* can emit
them. That distinction is the whole item.

`probe-1-report.txt` is the raw per-context output.

### Probe 2 — choosing the character before the splice (`probe-2-reorder.patch`, `zz_sweep.rs`, `zz_len4.rs`, `zz_len5.rs`)

Same hook site, reordered: choose the character first, suppress the splice when
the run's character differs from its children's, and forbid a child from taking
the character its parent took.

Baselines reproduce the committed shape files exactly — 1,730 and 1,984 —
before anything is claimed.

| | ship | reorder |
|---|---|---|
| `census-inexpressible.txt` | 1,984 | **428** |
| `census-known-structure-corrupt.txt` | 1,730 | **1,616** |
| recovered | — | **1,670** |
| broken | — | **0** |
| new text loss | — | **0** |

Text under the reorder: 0 of 130,321 at length 4, 0 of 2,476,099 at length 5.

**The context spread matters more than the headline.** Recovery is 1,670 when a
shape is rendered alone — the census's only context, and the most permissive one
for `_`, since paragraph boundaries always let it open. Between letters it is
**192**. Reporting the 1,670 by itself would have repeated the error that
produced the 2,198.

`p2-recovered.txt`, `p2-broken.txt` and `p2-newtext.txt` are the shape sets, not
the counts. Two of this repo's four measured head/tail asymmetries were
invisible to counts and fell out only of diffing sets, which is why the sets are
what is archived.

## Both probes cheated in the same way, and the spec says so

`parent_ch` was carried in an atomic so neither patch had to touch a signature,
which means `probe_edges` never saw it. That is fine in a throwaway and is not
fine in the seam: spec §4.4 requires it be threaded explicitly, and §5.3 records
what the `debug_assert_eq!(edge, Edge::of(&inner))` invariant was and was not
exercised against. The length-5 sweep ran under `--release`, where that assert
compiles out, so it is evidence about text only.

## Files

| file | what it is |
|---|---|
| `probe-hook.patch` | probe 1's writer patch — forced assignment at the emission site |
| `zz_underscore.rs` | probe 1's sweep: ceiling, locality, regression, five contexts |
| `probe-1-report.txt` | probe 1's raw output |
| `probe-2-reorder.patch` | probe 2's writer patch — choice before splice |
| `zz_sweep.rs` | probe 2's census-population sweep and per-context regression |
| `zz_len4.rs` | probe 2's length-4 text tier, ship vs reorder |
| `zz_len5.rs` | probe 2's length-5 text tier (release; see caveat above) |
| `zz_emission_counts.rs` | probe 1 spot check: containers present vs delimiters chosen — the §2.3 instrument |
| `zz_pairing.rs` | CommonMark pairing behaviour for `*`/`_` combinations, straight from the parser |
| `zz_literal_underscore.rs` | `Text("_")` inertness, ship behaviour vs the flank-guarded rule |
| `p2-recovered.txt` | the 1,670 shapes recovered |
| `p2-broken.txt` | empty — the 0 broken |
| `p2-newtext.txt` | empty — the 0 new text losses |
| `zz_resid.rs` | dumps the residual permanent set (produced §2.5.1) |
| `p2-residual-permanent.txt` | the 428 that remain permanent, all flanking refusals |

## A correction this directory records against its own spec

The spec's first draft claimed the 428 residual was a floor because alternating
chains exhaust a two-character alphabet, citing `Emph[Emph[Emph[a]]]`. Both
halves were wrong, and `zz_resid.rs` plus one parser call is what showed it:
`_*_a_*_` parses as `<em><em><em>a</em></em></em>`, so depth 3 is spellable; and
the residual is not about depth at all. Every one of the 428 has letter text
adjacent to the nested container — 156 with the container first, so the *closing*
delimiter is letter-flanked, the rest letter-flanked on the opener.

`p2-residual-permanent.txt` is that set. It is archived because counting it
would not have caught the error; dumping it did.

## Post-implementation verification

### Length-5 text tier, re-run in debug against the shipped writer

The length-5 text tier was re-run **in debug** against the shipped
implementation (`crates/kasane-writer/tests/zz_len5_debug.rs`, deleted after
use, never committed — see the harness convention above), where
`debug_assert_eq!(edge, Edge::of(&inner))` is live:

```
$ cargo test -p kasane-writer --test zz_len5_debug -- --nocapture
test no_shape_of_length_five_loses_text_or_desynchronises_the_probe ... ok
test result: ok. 1 passed; 0 failed; ...; finished in 87.91s
```

**0 of 2,476,099 shapes lose text, and the probe/render invariant holds.**
That closes the gap this directory records above — both design-phase probes
carried `parent_ch` in a global, so `probe_edges` never saw it and the
length-5 figure was `--release` only. Re-run twice against the shipped
writer (once against the pre-condition-4 commit `05bb516`, once against
`0909b3a`); both are 0 of 2,476,099.

### A regression the text tier could not see, and the structural gap that let it through

The census ratchet (`mise run census-ratchet`) caught a real structural
regression in commit `05bb516` that this directory's text-only sweeps above
never could: 5 length-3 shapes moved from `Inexpressible` (a clean, already-
accounted-for loss of one nesting level) to `Corrupt` (an unrelated `Strong`
losing its own delimiters, with its text migrating across a structural
boundary). Byte-identical *text* either way, which is exactly why every text
sweep — including the 2.48M-shape length-5 sweep above — stayed at zero.
Commit `0909b3a` ("only take `_` where declining the splice saves the child")
fixed it: a fourth condition on `choose_mark` declines `_` where the run it
would save fuses two different delimiter classes together, since that fuse
substitutes one class for another where the plain splice would only have
erased a level.

The census's own structural tier (`census.rs`) stops at length 3, and
`census_len4.rs` is text-only — **no shipped gate prices structure above
length 3.** The defect was caught only because the affected family happens
to have a length-3 member; the same family is much larger at length 4 and 5.
That was measured directly, with a reconstructed structural probe over the
full 19-element census alphabet at lengths 4 and 5, on three revisions —
`main` (`d4fc510`), the branch before the fix (`05bb516`), and the branch
with the fix (`0909b3a`) — comparing each shape's `classify_with` result
line-for-line against `main`:

```
$ docs/superpowers/evidence/2026-08-23-underscore-alphabet/harnesses/structural-len4-5-sweep.sh
-- length-4 --
  base->nofix   INEXPR->CORRUPT regressions: 135     ->CLEAN improvements: 31376
  base->fix     INEXPR->CORRUPT regressions: 0        ->CLEAN improvements: 31376
  nofix->fix    shapes that move (any direction): 135
-- length-5 --
  base->nofix   INEXPR->CORRUPT regressions: 3134     ->CLEAN improvements: 588423
  base->fix     INEXPR->CORRUPT regressions: 0         ->CLEAN improvements: 588423
  nofix->fix    shapes that move (any direction): 3134
```

**135 shapes at length 4 and 3,134 at length 5 regressed from `Inexpressible`
to `Corrupt` before the fix, and the fix closes every one of them — zero
regressions against `main` at either length, every recovery intact.** These
figures reproduce exactly the numbers measured (and archived only in a task
report, not committed) by the fix's own author; this harness makes them
reproducible on demand rather than asserted.

A **structural** length-4 tier is the smallest shipped guard that would have
spoken here; it is a follow-up, not part of this branch.

### Harness

| file | what it is |
|---|---|
| `harnesses/zz_structural_len4_5.rs` | classifies every length-4 and length-5 census shape with `classify_with`, one revision at a time |
| `harnesses/structural-len4-5-sweep.sh` | runs the probe above against `main`, `05bb516`, and `0909b3a` via `git worktree`, and prints the transition table |

Reproduce with:

```
docs/superpowers/evidence/2026-08-23-underscore-alphabet/harnesses/structural-len4-5-sweep.sh
```

It builds and runs three separate worktrees (~20s each in release once cargo's
registry cache is warm) and cleans them up on exit; it is evidence, not a
test — never compiled under `crates/`, never run in CI.
