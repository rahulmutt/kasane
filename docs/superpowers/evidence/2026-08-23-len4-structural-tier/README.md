# Length-4 structural tier — evidence

Design: `docs/superpowers/specs/2026-08-23-length-4-structural-tier-design.md`.

## Does the tier bite?

`05bb516` is delimiter-choice-before-splice without condition 4 — the commit
whose 135 length-4 `Inexpressible → Corrupt` regressions this tier exists to
catch, and which every shipped gate missed at the time.

Method: `git worktree` at `05bb516`, with this branch's `census_len4.rs`,
`census_support/mod.rs` and three `census-len4-*.txt` baselines copied in.
`census_support`, `kasane-ir` and `kasane-writer`'s public surface are
byte-identical across the two revisions; only `choose_mark`'s body differs, so
the instrument measures the older writer without contamination.

- `queue-direction.txt` — the tier failing: `135 shape(s) newly structurally
  corrupt`.
- `both-directions.diff` — the same run blessed inside the worktree and diffed
  against this branch's files: 135 lines into the queue, 135 out of the
  permanent file. One run cannot show both directions, because `ratchet`'s first
  assertion short-circuits its second.

## Does the cross-revision gate bite?

- `union4-gate.txt` — `mise run census-ratchet` with `KASANE_RATCHET_BASE` set
  to the tier commit, once with a shape injected into the length-4 queue
  (`union4 FAIL -- 1 added`, exit 1) and once clean (exit 0).

**Scope, stated rather than implied.** That union check was a one-off, run by
hand on 2026-08-23. `ratchet_gate_cases.sh` was deliberately not extended
(design §6.1), so in CI the length-4 union gate only ever ran in its passing
direction — the silent-gate failure mode this repo has recorded twice.

**Superseded 2026-08-24.** The extension landed on branch
`len4-union-gate-case`: `ratchet_gate_cases.sh` now carries a third direction
that reproduces `union4-gate.txt`'s injection on every CI run, so this file is
history rather than the only proof. Design §6.1 records what closing it took.
The files here are kept as the record of the by-hand run that stood in the
meantime.

## Wall clock

Spec §4.2 and §10 both claim the implementation verifies the wall-clock cost
of both tiers together. Measured on this machine on 2026-08-23 by running
`cargo test -p kasane-writer --test census_len4` after the final-review fix
wave: `finished in 6.48s` (a second run measured `6.32s`). This is a
measurement on this machine on this date, not a universal claim — the design's
own prediction (§4.2) is ≈5.7s on parallel threads, with a machine that has no
second free core landing nearer 9.3s.
