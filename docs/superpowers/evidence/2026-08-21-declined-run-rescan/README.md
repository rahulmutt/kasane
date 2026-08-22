# Declined-run rescan — design-phase probes

Archived apparatus for the measurements recorded in
`docs/superpowers/specs/2026-08-21-declined-run-rescan-design.md` §2.3 and §5.1,
and shipped as PR #47.

The spec reproduces every figure inline, so nothing here is load-bearing for
reading it. What lives here is the *apparatus*: the two throwaway
implementations that produced the 16/32 split — the single measurement that
overturned the parent spec's own prediction.

## Nothing in this directory is a test

None of it is compiled, run in CI, or maintained. Same convention as
`2026-08-18-abutment-ledger/` beside it.

## What was measured, and why it mattered

`2026-08-15-emphasis-seam-design.md` §8 predicted that re-entering a declined
run's children into the outer flat view would close all 32 shapes of the
backtick text family. Probe A implemented exactly that proposal. It closed
**16** — the tail half, where the collision is with what follows. The head half
survives because `inlines_to_md_flat` walks forward, so the element before a
declined run is already a substring of the output buffer and restarting the scan
can never reach it. Probe B added a one-run buffer rollback and closed all 32.

That split is why the shipped item exists as its own spec rather than as the
parent's §8 implemented as written.

## The two probes

**Probe A — forward-only rescan.** `probe-a-forward-only.patch`, an uncommitted
working-tree diff recovered from the scratch worktree it was written in. Against
`3188c5b` it moves 16 shapes from `census-known-corrupt.txt` to
`census-known-structure-corrupt.txt`.

```bash
git checkout 3188c5b
git apply docs/superpowers/evidence/2026-08-21-declined-run-rescan/probe-a-forward-only.patch
cargo test -p kasane-writer --test census
```

**Probe B — rescan plus rollback, and the ratchet's union.** Preserved as the
annotated tag `archive/rescan-probe-b` (commits `7b190fd` "probe B" and
`312e7a9` "probe: union includes text queue"), branched from `3188c5b`. The
first moves all 32; the second is the one-line change that put the text queue
into the ratchet's union, which is what made the promotion read as a fix rather
than a +32 regression.

```bash
git show archive/rescan-probe-b^{commit}
git log --oneline 3188c5b..archive/rescan-probe-b^{commit}
```

## How these differ from what shipped

Both are throwaway. Neither has any of what the reviews added to the shipped
implementation, and reading them as a guide to the current writer will mislead:

- no `probe_edges` — both render a declined run's children to decide whether the
  delimiter flanks, then discard the render. That is the `R(k) = 2·R(k+1)`
  blowup the first review found (9.86 s at depth 22).
- no rollback skip guard, so probe B re-renders a recursive external-link
  predecessor on every decline — a second 2^depth path.
- the growing splice still memmoves the paragraph tail, an O(n²) in paragraph
  *breadth* that the census structurally cannot witness: 61,864 declines across
  the whole census population, zero of them growing splices, because every
  container in the census alphabet has exactly one child.

All three are closed on `main` with permanent guards. See the spec's §3 and the
PR for the measurements.
