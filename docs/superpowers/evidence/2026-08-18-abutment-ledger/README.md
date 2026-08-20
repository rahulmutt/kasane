# Abutment ledger — measurement evidence

Archived evidence for the disproof recorded in
`docs/superpowers/specs/2026-08-18-abutment-ledger-design.md` §2b and §5.4.

The spec reproduces every figure inline, so nothing here is load-bearing for
reading it. What lives here is the *apparatus*: the reports that carry the full
per-subset tables, and the harnesses that produced the numbers. Without them
§2b.1's 3,328 / 2,528 / 800 could not be re-derived except by rewriting a tier
§5.4 says is broken.

## Nothing in this directory is a test

None of it is compiled, run in CI, or maintained. `harnesses/` holds `.rs` files
that were test targets in a scratch worktree; they are archived as artifacts, not
restored as tests, which is why several still carry their original
`THROWAWAY … Not for commit.` banners. Read those banners as "not for commit *as
a test target*" — that remains true, and is why they sit under `docs/` rather
than `crates/kasane-writer/tests/`.

To re-derive a figure, copy the relevant file into `crates/kasane-writer/tests/`
on a checkout at or after `9ca27fe` and run it. Expect to adjust: they were
written against a working tree with the licensing cells turned on, which no
commit on `main` has.

## Measurements

| file | what it is |
|---|---|
| `measurements-task-4.md` | The blocked implementer's report: per-subset table over the length-3 census (7,239 shapes), the three blockers, and the reasoning that stopped the task. |
| `measurements-verification.md` | An independent second agent's verification in an isolated worktree. Replicated the length-3 table cell for cell, then measured what nobody had: lengths 4-7, and the census's own 19-element alphabet. **Authoritative where the two differ** (§2b's stated precedence). |

## Harnesses

| file | what it is |
|---|---|
| `census_deep.rs` | The deep census tier as §5.2/§5.3 specified it. **Broken as designed — do not trust its verdict.** It fails 2,561 times on unmodified `main`, and its 7-element corpus omits `Emph[Code]`/`Strong[Code]`, the elements that witness the surviving cell's 800 text losses. §5.4 records both defects. Preserved because it is the artifact §5.4 is *about*. |
| `zz_verify.rs`, `zz_cases.rs` | The verification pass's case-by-case checks — reproducing the cited shapes and confirming the blockers' mechanisms. |
| `zz_stress.rs` … `zz_stress4.rs` | The extended-corpus sweeps. `zz_stress4.rs` is the one that asks which *family* the edge cells' text regressions belong to, and is behind the head/tail asterisk split (2,528 versus 0). |
| `reverted-licensing-attempt.patch` | The reverted Task 4 working state: five unit tests, `Ledger::LICENSED` widened to five cells, plus the tier. Never committed to a branch. |

## The one-paragraph version

The item aimed to license abutment cells and recover ~924 inline shapes. A
committed probe re-measured the recoverable set at 389. Attempting to license the
cells then showed that **no cell set is free of text loss**: head cells produce
2,528 asterisk mis-pairings where tail cells produce zero, because a licensed
edge leaves a three-character delimiter run whose pairing depends on what follows
it. The failure is positional; `may_abut`'s key is structural. That is the
ledger's premise, not its table — which is why the spec records a disproof rather
than a smaller cell set.
