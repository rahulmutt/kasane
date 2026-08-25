# Every census-ratchet gate now has a negative direction

Branch `ceiling-and-union3-gate-cases`, 2026-08-25. Base `66eccad`.

`mise run census-ratchet-cases` grew four directions, closing the gap
`2026-08-23-length-4-structural-tier-design.md` §6.1 named twice:

- **direction 4** — the length-3 `union` gate.
- **directions 5 and 6** — `ceiling_check`'s no-gratuitous-raise test, at
  length 3 and length 4.
- **direction 7** — the same check's *positive* direction: a justified raise
  must pass.

`00-all-directions-pass.txt` is the whole script green on this branch, with
every table it read.

## What each new assertion is worth

A direction that has only been seen passing proves nothing — the point of the
script it lives in. So each was checked by breaking the gate it targets and
confirming that direction, and no other, speaks. Every file below is a full
run of `census-ratchet-cases` against a mutated `mise.toml`; `mise.toml` was
restored between runs and is unmodified in the commit.

| file | break applied to `census-ratchet` | direction that spoke |
|---|---|---|
| `01-break-union3-to-report.txt` | `check union … gate` → `report` | 4 |
| `02-break-union3-deleted.txt` | the `check union …` call removed | 1 (guard) |
| `03-break-ceiling-disabled.txt` | `ceiling_check`'s failure branch → `if false` | 5 |
| `04-break-ceiling-grew-term.txt` | `&& [ "$grew" -eq 0 ]` dropped | **7 only** |
| `05-break-ceiling3-absent.txt` | `ceiling_check len3` pointed at a path absent at the base | 1 (guard) |
| `06-break-ceiling-len4-only.txt` | failure branch narrowed to `label = len3` | 6 |

Three of these are the reason the directions are shaped the way they are.

**`04` is why direction 7 exists.** Dropping the `grew` term leaves directions
5 and 6 **green** — both still print their expected
`FAIL ceiling(lenN) raised …` — because they only ever exercise the `raised`
half of a two-term predicate. The check is now one that rejects *every* raise,
including the legitimate promotions the ceiling exists to make reviewable, and
only direction 7 is red. No failure direction can see this.

**`06` is why 5 and 6 are separate rather than one case trusting a shared
helper.** `03` disables both ceilings, so direction 5 speaks first and 6 rides
on it. Narrowing the break to `len3` leaves 5 passing and forces 6 to speak on
its own.

**`02` and `05` are why direction 1 gained three guards.** A deleted `union`
check and a ceiling absent at the merge base both make their gate silently
stop gating, and the directions that target them would then report a gate
proven that never ran — the exact failure mode this script exists to close.
Direction 1 now refuses to continue in either case. This is the length-3 and
ceiling form of the `union4 … skipped (no baseline)` guard added 2026-08-24.

## What is not covered

The `text` gate. Its file is empty at both ends, so any shape injected into it
is an addition to the union too, and `union` fails alongside it — the same
non-isolation that ruled out putting direction 4's probe in the queue. There is
no injection that makes the text gate speak alone, so it has no direction here.
