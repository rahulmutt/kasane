# Length-4 union gate — negative direction

Closes the follow-up named by
`docs/superpowers/specs/2026-08-23-length-4-structural-tier-design.md` §6.1:
the `union4` gate added in PR #50 only ever ran in its *passing* direction in
CI. `crates/kasane-writer/tests/ratchet_gate_cases.sh` now drives it into
failure on every run.

## The directions, passing

`all-directions-pass.txt` — `mise run census-ratchet-cases` on this branch.
Direction 1 requires a `union4 ... ok` row (the gate is live at this base),
direction 2 requires `queue+ ... FAIL`, direction 3 injects
`[Text("z"), Text("z"), Text("z"), Text("z")]` into
`census-len4-known-structure-corrupt.txt` and requires
`union4 ... FAIL -- 1 added`. That injection is the one
`2026-08-23-len4-structural-tier/union4-gate.txt` recorded by hand; it now runs
in CI.

## Can the new assertions fail?

A green run proves nothing if the harness cannot produce the failing shape, so
each assertion was driven into failure against a throwaway copy of the script.
The census files were confirmed restored (`git status`) after every one.

- `break-wrong-gate.txt` — direction 3 rewritten to inject into the **length-3**
  queue instead. `census-ratchet` still exits non-zero (the length-3 gates
  speak), and the old exit-status-only check would have called that a pass.
  Exit 1: *"census-ratchet failed, but not on the union4 gate this case
  targets."* This is why extending the script was not additive — both
  directions had to start matching their own row.
- `break-vacuous-probe.txt` — `probe4` set to a shape already in the length-4
  queue, so injecting it grows nothing. Exit 1 before the gate is ever run:
  *"the probe shape is already in ... so injecting it proves nothing."*
- `break-skipped-baseline.txt` — run with `KASANE_RATCHET_BASE=97b2604` (the
  PR #49 merge, which predates the length-4 files), so `union4` reports
  `skipped (no baseline)` and would catch nothing. Direction 1 refuses, exit 1,
  naming the rebase. Without this the negative direction would have failed for
  an unrelated reason and reported a gate proven that never ran.

## Still uncovered

The length-3 `union` gate and both `ceiling_check`s are still only ever seen
passing. Scoped out deliberately; recorded here and in `AGENTS.md` rather than
left for the next reader to discover.
