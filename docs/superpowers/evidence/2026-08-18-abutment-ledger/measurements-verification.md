# Task 4 — independent verification and measurement

**Verifier:** a second agent, isolated in its own git worktree at `4082e20`
(branch `abutment-ledger`). Nothing was committed or pushed. `markdown.rs` and
every census file were restored after each experiment; the only residue in the
worktree is a set of untracked `zz_*.rs` throwaway probes.

**Scope.** Part 1 re-checks the three blockers in `task-4-report.md` with
independently written evidence. Part 2 supplies the measurement nobody had
made: whether the two length-3-clean single cells stay clean at length 4/5.
Part 3 answers whether any cell set is shippable.

**Headline.** All three blockers are real. Blocker A's *symptom* reproduces
exactly but its stated *mechanism* is a length-3 artifact and understates the
problem. And the measurement that was supposed to decide the item kills the
last candidate: the two cells that look clean at length 3 do not stay clean —
`EMPH_OVER_STRONG_HEAD_EDGE` costs 168 text corruptions on the spec's own deep
corpus, and `EMPH_OVER_STRONG_TAIL_EDGE`, which survives both of the brief's
corpora, costs 800 on the census's own 19-element alphabet at the same lengths.

---

## 0. Method and instruments

Everything below uses the shared oracle `census_support` (`render`,
`classify_with`, `text_is_clean`, `parsed_text`, `shapes`, `alphabet`) — no
reimplementation. Ledgers are built with `Ledger::from_bits`; the writer source
was left at `4082e20` for all sweeps, so cell selection is a parameter, not an
edit.

Per-shape status under a ledger is a four-way classification, matching what the
census files actually record:

```
TextCorrupt  parsed_text(render(seq, L)) != rendered_text(seq)   -> census-known-corrupt.txt
Clean        text ok and classify_with == Clean
Queue        text ok and classify_with == Corrupt                -> census-known-structure-corrupt.txt
Perm         text ok and classify_with == Inexpressible          -> census-inexpressible.txt
```

**Baseline is the shipped ledger throughout** — `Ledger::LICENSED` at `4082e20`,
i.e. `EMPH_OVER_STRONG_WHOLE_RUN` (bit 0) alone — never `CONSERVATIVE`. Every
"regression" below is against what users get today.

- **text regression** = `Clean|Queue|Perm` under the baseline, `TextCorrupt`
  under the ledger being tested.
- **structure regression** = `Clean` under the baseline, anything else under
  the ledger being tested.

Corpora:

| corpus | alphabet | lengths | shapes |
|---|---|---|---|
| length-3 census | `census_support::alphabet()`, 19 elements | 1-3 | 7,239 |
| deep (spec §5.2) | 7 elements | 4, 5 | 19,208 |
| **extended** (mine, beyond the brief) | the census's own 19 elements | 4 | 130,321 |
| **extended** (mine, beyond the brief) | the census's own 19 elements | 5 | 2,476,099 |

The extended corpus is not in the brief. I added it because the deep corpus's
seven elements are a deliberate narrowing, and the narrowing turns out to be
load-bearing — see §2.3.

Baseline composition, for reference:

```
length-3 (7,239): TextCorrupt 32   Clean 3,525   Queue 1,698   Perm 1,984
deep    (19,208): TextCorrupt  0   Clean 3,904   Queue 11,673  Perm 3,631
```

The four length-3 figures match the four census files on disk exactly (32 /
1,698 / 1,984, ceiling 1,984), which is the instrument's own sanity check.

---

## Part 1 — the three blockers

### Blocker A — the four edge cells jointly corrupt recovered text

**Verdict: CONFIRMED WITH CORRECTIONS.** The corruption is real and worse than
reported. The mechanism as stated is right about the two cited shapes and wrong
as a general explanation.

#### A.1 The cited shapes reproduce byte for byte

```
[Emph[Strong[a]], Emph[a], Emph[Strong[a]]]
  shipped (bit 0)        md="*aaa*"          recovered="aaa"    want="aaa"  text ok   struct=Corrupt
  four cells             md="***a**a**a***"  recovered="aaa**"  want="aaa"  TEXT LOST struct=Clean
  EmStHead only          md="***a**aa*"      recovered="aaa"    want="aaa"  text ok   struct=Corrupt
  EmStTail only          md="*aa**a***"      recovered="aaa"    want="aaa"  text ok   struct=Corrupt
  EmStHead+EmStTail      md="***a**a**a***"  recovered="aaa**"  want="aaa"  TEXT LOST struct=Clean

[Strong[Emph[a]], Emph[a], Strong[Emph[a]]]
  shipped (bit 0)        md="**aaa**"        recovered="aaa"    want="aaa"  text ok   struct=Corrupt
  four cells             md="***a*a*a***"    recovered="aaa**"  want="aaa"  TEXT LOST struct=Clean
  StEmHead only          md="**aaa**"        recovered="aaa"    (unchanged)
  StEmTail only          md="**aaa**"        recovered="aaa"    (unchanged)
  StEmHead+StEmTail      md="***a*a*a***"    recovered="aaa**"  want="aaa"  TEXT LOST struct=Clean
```

Two stray asterisks leak into recovered text, exactly as reported. Note the
second row of each block: the shape becomes structurally `Clean` while its text
is destroyed — which is why `classify_with` alone cannot see it, and why the
committed §2 probe's `newly_corrupt` column reads 0. The report's diagnosis of
the probe's blind spot is correct.

#### A.2 The stated mechanism is right for these shapes

`edge_to_splice` considers exactly two candidates — the first and last
*printing* child — and returns the first of them that is **not** licensed. With
`EmStHead` alone the head container stands and the tail one is spliced
(`***a**aa*`); with `EmStTail` alone the mirror (`*aa**a***`); with both, the
function returns `None`, `same_delim_to_splice` finds nothing (`Emph` is not
`Strong`), and `splice_children`'s `while let` exits on its first iteration
with both containers standing. So design spec §3.2's "the splice loop's
fixpoint handles the second after the first resolves" does fail when both
candidates are licensed — there is no second iteration because nothing
resolved. That much of the report is confirmed by the renderings above.

#### A.3 …but "both edges" is not the generator, and the report understates it

At length 4 a **single** licensed edge corrupts text:

```
[Emph[Strong[a]], Emph[a], Emph[Strong[a]], Emph[a]]   EmStHead only
  md="***a**a**a**a*"   recovered="aaa*a*"   want="aaaa"   TEXT LOST
```

Here only one cell is on, and the tail candidate is a bare `Text` that
`edge_to_splice` rejects before `may_abut` is consulted — so "both edges
licensed" is not present. The four members fuse into one `Emph` run whose
children are `[Strong[a], Text a, Strong[a], Text a]`; the *interior* `Strong`
at index 2 is never a splice candidate at all (by design — "a container between
other content has nothing adjacent to abut"), and the licensed head edge turns
the run's opening into `***`. The rest is delimiter pairing.

The mirror shape `[Emph[a], Emph[Strong[a]], Emph[a], Emph[Strong[a]]]` under
`EmStTail` renders `*a**a**a**a***` and round-trips fine. The failure is
*positional and asymmetric*, which is the strongest possible evidence that it
is a delimiter-pairing question — precisely the question §3.3 forbids
`may_abut` from asking.

The report's own closing sentence in its §3 ("the failure is not 'both edges at
once'; it is that a licensed edge abutment produces a 3-character delimiter run
whose pairing depends on what follows") is the correct diagnosis. Its headline
mechanism, its per-subset table framing, and its unblock question 1 all lead
with the narrower "both edges" story, which is a length-3 artifact. **The
correction matters**, because "both edges" invites the repair "license at most
one edge per run", and that repair is already known not to work (report §3's
336-corruption probe; independently, my §2.2 numbers show single cells failing
on their own).

#### A.4 P13 fails independently — confirmed

With `LICENSED` widened to the five cells (the patch's exact change) and
nothing else touched:

```
cargo test -p kasane-writer --test properties
  test result: FAILED. 15 passed; 1 failed
  failures: p13_inline_text_survives_rendering
```

The shrink case reproduces deterministically:

```
[Strong[Emph[a]], Emph[a], Emph[Emph[a]]]
  shipped (bit 0)  md="**aaa**"      recovered="aaa"    want="aaa"
  five cells       md="***a*a*a***"  recovered="aaa**"  want="aaa"
```

Same family as A.1. With `EMPH_OVER_STRONG_TAIL_EDGE` alone, `properties`
passes 16/16.

Also confirmed while I was there: with the five cells on, the two tests spec §7
predicted would fail (`fusing_adjacent_runs_costs_a_structural_boundary`,
`splicing_mid_buffer_costs_a_span_that_would_round_trip`) both **pass**, and
`fusing_nested_emphasis_does_not_leak_its_delimiters` fails instead. The
report's §6 is correct and §7 of the spec names the wrong two tests.

#### A.5 The proptest regression file — clean

`/workspace/crates/kasane-writer/tests/properties.proptest-regressions` in the
user's tree is byte-identical to the committed file at `4082e20` (13 lines, no
`db40924940…` line). **No stray regression line is being carried.**

One caveat for whoever picks the work back up: the preserved patch
`wip/task-4-attempt.patch` **does** contain the regression-line hunk. Applying
it as-is reintroduces the stray line the report says it reverted. The rest of
the patch matches the report's description of it: `LICENSED` widened to five
cells, five new unit tests with the names and strings the report lists, the
ledger test renamed with `count_ones() == 1` dropped, and `census_deep.rs`
added. The `census_deep.rs` hunk is byte-identical to the preserved standalone
`wip/census_deep.rs`.

---

### Blocker B — the `StrongOverEmph` cells cause 37 permanent → queue moves

**Verdict: CONFIRMED.** Example, mechanism, and counts all replicate.

```
[Text("a"), Strong[Emph[a]], Strong[a]]
  shipped (bit 0)     md="a**aa**"  recovered="aaa"  struct=Inexpressible  (correctly permanent)
  StEmHead            md="a*a*a"    recovered="aaa"  struct=Corrupt        (queue — the <strong> is gone)
  StEmHead+StEmTail   md="a*a*a"    recovered="aaa"  struct=Corrupt
  StEmTail            md="a**aa**"  (unchanged)
  EmStHead / EmStTail md="a**aa**"  (unchanged)
```

`a*a*a` parses as `a<em>a</em>a`: the `<strong>` has vanished entirely, and an
`<em>` that the IR nests *inside* it is now at top level. The IR's stacks are
`[] [St,Em] [St]`; the recovered stacks are `[] [Em] []`. That is strictly
worse than today's `[] [St] [St]`, which merely erases the inner `<em>` — the
one erasure `differs_only_by_erasure` forgives, and the reason the shape is
filed permanent today.

Mechanism confirmed: the two `Strong` members share `*` so `run_end` fuses them
into one run whose children are `[Emph[a], Text a]`; with
`STRONG_OVER_EMPH_HEAD_EDGE` on, `edge_to_splice` finds the head `Emph`,
`may_abut(Strong, Emph, HeadEdge)` licenses it, and it stands. The run's own
`**` then sits between `a` (`Flank::Other`) and `*` (`Flank::Punct`), so
`can_open` is false, `emphasis_run` takes its decline branch and prints the
children bare without any `**`. This is exactly design spec §4.3's flanking
consequence, and §4.3's own words for the decline branch ("renders children
bare and… exposes a seam that nothing re-scans") describe the loss.

One small correction: the cited shape is moved by `STRONG_OVER_EMPH_HEAD_EDGE`
specifically. `STRONG_OVER_EMPH_TAIL_EDGE` leaves it alone; its own 8
permanent→queue moves are a different family. The report's counts (8 for either
cell alone, 37 for both) replicate exactly — see the table in §2.1.

**The ratchet does gate this.** I verified it by running the real task rather
than by reading it: I moved one line from `census-inexpressible.txt` to
`census-known-structure-corrupt.txt` on disk and ran the task against `4082e20`.

```
$ KASANE_RATCHET_BASE=HEAD mise run census-ratchet
set          base     head    delta   verdict
text           32       32       +0   ok
queue        1698     1699       +1   FAIL -- 1 added
           [Code("x"), Code("x"), Emph([Emph([Text("a")])])]
perm         1984     1983       -1   ok
union        3682     3682       +0   ok
census ratchet FAILED: the allowlists may only shrink against main.
```

Note that the **union is unchanged** — the union rule §6 relies on would have
let this through. It is the `queue … gate` line that catches it. The report's
observation that this is "the direction §6 does *not* worry about" is correct
and is worth carrying into any spec revision.

---

### Blocker C — the deep tier as specified fails on unmodified `main`

**Verdict: CONFIRMED**, count exact.

I copied `wip/census_deep.rs` into the worktree with `markdown.rs` untouched at
`4082e20` (`LICENSED` = bit 0 alone) and ran it:

```
$ cargo test -p kasane-writer --test census_deep
2561 newly-licensed spelling(s) do not round-trip -- a cell in `may_abut` is
wrong, not a residual to be recorded:
  structure: [Emph([Strong([Text("a")])]), Text("a"), Text("a"), Text("a")] -> "**a**aaa"
  structure: [Text("a"), Emph([Strong([Text("a")])]), Text("a"), Text("a")] -> "a**a**aa"
  …
test result: FAILED. 0 passed; 1 failed
```

Composition of the kept set, measured separately:

```
kept (renderings differ from CONSERVATIVE): 3,513
  text failures:                                0
  structure failures:                       2,561   (Corrupt 2,125, Inexpressible 436)
  structure failures with a top-level Emph[Strong[..]]:  2,561  (all of them)
```

Both of the report's numbers (2,561 failures, 3,513 kept) are exact, and its
claim that the family is uniformly `Emph[Strong[x]]`-with-a-neighbour is
confirmed — every single failing shape has a top-level `Emph[Strong[…]]`.

The recorded residual is where the report says it is. Line 568 of
`crates/kasane-writer/tests/census-known-structure-corrupt.txt` is verbatim:

```
[Emph([Strong([Text("a")])]), Text("a"), Text("a")]
```

So the tier's contract ("a corrupt shape here is a wrong cell in `may_abut`,
not a residual") contradicts a residual the length-3 census already records for
the cell the writer already ships. The report's diagnosis of the cause is
right: differencing against `CONSERVATIVE` isolates "spellings the whole ledger
changed", and the whole ledger includes Task 1's shipped cell, whose §4.3
consequence is queued rather than clean.

One refinement worth recording: 436 of the 2,561 classify `Inexpressible`, not
`Corrupt`, so the tier as written would also fail on shapes the census files
call *permanent*. Baselining the filter against the shipped `LICENSED` (the
report's first option) fixes both halves; an allowlist keyed to the length-3
queue would not, since 436 of these are not in the queue file's family at all.

---

## Part 2 — do the length-3-clean cells stay clean?

### 2.1 Length-3 census (7,239 shapes), baseline = shipped ledger

Full subset sweep. My numbers, produced by an independently written harness,
**reproduce the prior implementer's table cell for cell.** Its length-3
measurements are trustworthy.

| ledger (all include `Whole` = bit 0) | txtreg | structreg | q→clean | p→clean | q→p | p→q |
|---|---|---|---|---|---|---|
| `Whole` (control, = ships today) | 0 | 0 | 0 | 0 | 0 | 0 |
| `Whole+EmStHead` | **0** | 0 | 48 | 0 | 28 | **0** |
| `Whole+EmStTail` | **0** | 0 | 48 | 0 | 28 | **0** |
| `Whole+EmStHead+EmStTail` | 8 | 0 | 118 | 0 | 56 | 0 |
| `Whole+StEmHead` | 0 | 0 | 0 | 48 | 0 | 8 |
| `Whole+StEmTail` | 0 | 0 | 0 | 48 | 0 | 8 |
| `Whole+StEmHead+StEmTail` | 8 | 0 | 0 | 97 | 0 | 37 |
| `Whole+EmStHead+StEmHead` | 0 | 0 | 48 | 48 | 28 | 8 |
| `Whole+EmStTail+StEmHead` | 0 | 0 | 48 | 48 | 28 | 8 |
| `Whole+EmStHead+StEmTail` | 0 | 0 | 48 | 48 | 28 | 8 |
| `Whole+EmStTail+StEmTail` | 0 | 0 | 48 | 48 | 28 | 8 |
| `Whole+EmStHead+EmStTail+StEmHead` | 8 | 0 | 118 | 48 | 56 | 8 |
| `Whole+EmStHead+EmStTail+StEmTail` | 8 | 0 | 118 | 48 | 56 | 8 |
| `Whole+EmStHead+StEmHead+StEmTail` | 8 | 0 | 48 | 97 | 28 | 37 |
| `Whole+EmStTail+StEmHead+StEmTail` | 8 | 0 | 48 | 97 | 28 | 37 |
| **`Whole+`all four (the brief's ask)** | **16** | 0 | 118 | 97 | 56 | **37** |
| `Whole+EmStSeam` | 210 | 0 | 76 | 0 | 215 | 0 |
| `Whole+StEmSeam` | 210 | 0 | 76 | 0 | 215 | 0 |
| all seven cells | 460 | 0 | 292 | 97 | 512 | 37 |

`structreg` is 0 in every row: no shape that is structurally `Clean` today
loses cleanliness at length 3 under any cell set. That is spec §2's
`ALL_CELLS_VS_LICENSED,shipped_baseline,389,0` row, reproduced — and it is
precisely the reassurance that does not survive §2.2. The recoveries
(292 / 97 at the seven-cell union) also reproduce §2's headline exactly.

### 2.2 Deep corpus, spec §5.2's seven elements, lengths 4-5 (19,208 shapes)

**This is the measurement nobody had made.**

| ledger | text regressions | structure regressions |
|---|---|---|
| `Whole` (control, = ships today) | **0** | 0 |
| `Whole+EmStHead` | **168** | 0 |
| `Whole+EmStTail` | **0** | 0 |
| `Whole+EmStHead+EmStTail` | 372 | 0 |
| `Whole+StEmHead` | 168 | 0 |
| `Whole+StEmTail` | 0 | 0 |
| `Whole+StEmHead+StEmTail` | 372 | 0 |
| `Whole+`all four | 744 | 0 |

`EMPH_OVER_STRONG_HEAD_EDGE` alone — one of the two subsets the prior
implementer's table showed clean on both metrics — **loses text on 168 shapes**
at lengths 4-5. `STRONG_OVER_EMPH_HEAD_EDGE` alone loses 168 as well. Samples:

```
[Emph[Strong[a]], Emph[a], Emph[Strong[a]], Emph[a]]  -> "***a**a**a**a*"  recovered "aaa*a*"
[Strong[Emph[a]], Emph[a], Strong[Emph[a]], Emph[a]]  -> "***a*a*a*a**"    recovered "aaaa**"
```

Length-3 cleanliness therefore does not imply length-4/5 cleanliness for these
cells, exactly as the brief anticipated. The two **tail** cells are clean on
this corpus. I pushed them further on the same seven elements:
`EMPH_OVER_STRONG_TAIL_EDGE` is still 0 text regressions and 0 structure
regressions at **length 6** (117,649 shapes) and **length 7** (823,543 shapes).
On the corpora the brief names, the tail cell survives everything.

### 2.3 Extended corpus — the census's own 19-element alphabet at lengths 4-5

The deep corpus is seven elements. It contains `Code("x")` and
`Emph[Strong[a]]` but **no emphasis container wrapping a code span** — no
`Emph[Code("x")]`, no `Strong[Code("x")]`. Those two elements are in the
length-3 census's alphabet, and they are the ones that witness the other half
of §4.3's decline branch. Re-running the same sweep over the census's own
19 elements at lengths 4 and 5:

| ledger | txtreg (len 4) | txtreg (len 5) | total | structreg |
|---|---|---|---|---|
| `Whole` (control) | 0 | 0 | 0 | 0 |
| `Whole+EmStHead` | 80 | 3,248 | **3,328** | 0 |
| `Whole+EmStTail` | 16 | 784 | **800** | 0 |
| `Whole+StEmHead` | 80 | 3,248 | **3,328** | 0 |
| `Whole+StEmTail` | 16 | 784 | **800** | 0 |
| `Whole+`all four | 640 | — | — | 0 |

(2,606,420 shapes swept per cell. The control is trivially 0: identical ledger,
identical rendering.)

Splitting the failures by which delimiter leaks into recovered text:

| ledger | backtick family | asterisk family |
|---|---|---|
| `EmStHead` | 800 | 2,528 |
| `EmStTail` | 800 | **0** |
| `StEmHead` | 800 | 2,528 |
| `StEmTail` | 800 | **0** |

The tail cells never mis-pair an asterisk. Their entire cost is the
**code-span adjacency family**, reached through §4.3's decline branch:

```
[Code("x"), Emph[Code("x")], Emph[Strong[a]], Text("a")]
  shipped (bit 0)  md="`x`*`x`a*a"    recovered="xxaa"    want="xxaa"  text ok, struct=Corrupt
  EmStTail         md="`x``x`**a**a"  recovered="x``xaa"  want="xxaa"  TEXT LOST
```

With the tail `Strong` licensed to stand, the `Emph` run's `*` no longer flanks,
`emphasis_run` declines, the children print bare, and the leading `` `x` `` of
`Emph[Code("x")]` abuts the preceding `` `x` `` into `` `x``x` `` — one code
span whose content is ``x``x``.

That family is a **pre-existing writer defect**, not a new one: its length-3
instance is line 1 of `census-known-corrupt.txt`,

```
[Code("x"), Emph([Code("x")]), Text("a")]
```

and it is design spec §8's "32-shape backtick text family", an explicit
non-goal with a known fix shape. The tail cell does not create it — it *reaches*
it, on 800 shapes that render cleanly today. Relative to the shipped ledger that
is still text loss, and it is invisible to every gate the branch currently has:
at length 3 the tail cell's text regressions are 0, so it would ship silently.

---

## Part 3 — is there a shippable cell set?

**Under the brief's literal criterion** — zero text regressions on the length-3
census and on the spec §5.2 deep corpus, plus zero permanent→queue moves —
**exactly one non-empty subset survives:**

> `EMPH_OVER_STRONG_WHOLE_RUN | EMPH_OVER_STRONG_TAIL_EDGE`
> (i.e. the one new cell `EMPH_OVER_STRONG_TAIL_EDGE`)

with these figures:

| metric | value |
|---|---|
| text regressions, length 3 | 0 |
| text regressions, deep corpus (len 4-5) | 0 |
| text regressions, deep alphabet len 6 / len 7 | 0 / 0 |
| structure regressions, both corpora | 0 |
| queue → clean | **48** |
| permanent → clean | 0 |
| queue → permanent | **28** (each needs individual §6 justification) |
| permanent → queue | 0 |
| `properties.rs` | 16/16 green |

Its bless would delete 76 lines from `census-known-structure-corrupt.txt`
(1,698 → 1,622; I confirmed the number by running the census under that ledger)
and add 28 to `census-inexpressible.txt` (1,984 → 2,012), which **exceeds the
permanence ceiling** and would require a hand-raised
`census-permanent-count.txt` — the deliberately visible one-line diff §6 and
`permanence_ceiling` exist to force. So the cost of 48 recoveries is 28 new
permanence claims, on a branch whose own spec §2 records that the last bless to
make 748 such claims at once was later found 88% wrong.

**Under the honest criterion — no text loss at all — the answer is: none.**

`EMPH_OVER_STRONG_TAIL_EDGE` loses recovered text on 800 shapes at lengths 4-5
over the census's own alphabet, and the reason it looks clean on the spec's deep
corpus is that the corpus omits `Emph[Code]` and `Strong[Code]`. That is a gap
in §5.2's corpus, not a property of the cell. I would not call the cell
shippable on the strength of a corpus that cannot see its failure mode,
especially on a branch whose §2 exists because an earlier unreproducible
measurement was taken at face value.

Two things soften that verdict, and the controller should weigh them rather than
have me weigh them:

1. The tail cells' 800 failures are **entirely** the already-recorded
   backtick-adjacency defect (§8 non-goal, known fix shape at emphasis-seam spec
   §8), never asterisk mis-pairing. The head cells' failures include 2,528
   asterisk mis-pairings, which are the ledger premise itself failing.
2. Nothing in the current gate set would catch the 800 — they are all at
   lengths 4-5, and the deep tier as specified cannot run at all (Blocker C).

If the item proceeds with `EMPH_OVER_STRONG_TAIL_EDGE`, it should proceed with
the backtick family fixed first, or with the 800 measured, named, and accepted
in the PR body — not with §5.2's corpus as the evidence that they do not exist.

**Also worth stating plainly:** the item's original scope was 389 shapes
(292 queue + 97 permanent). The largest text-clean, permanent→queue-free subset
recovers **48 queue shapes and 0 permanent shapes** — about 16% of the queue
share and none of the permanent share, at the price of 28 fresh permanence
claims. Whatever the decision on the tail cell, §6's "shrinks by 292 / shrinks
by 97" is not reachable by any subset of these seven cells.

---

## Closing section — opinion, clearly separated from the measurements

Everything above is measured. This part is not.

I think the ledger premise, as §3.1-§3.3 define it, cannot be repaired for the
edge sites, and the evidence I would point at is the asymmetry in §A.3 rather
than any one corruption count. `may_abut(Emph, Strong, HeadEdge)` and
`may_abut(Emph, Strong, TailEdge)` are the same question by the table's own
symmetry, and the writer answers them the same way — yet the head licence loses
text on 2,528 shapes by asterisk mis-pairing and the tail licence loses none. A
structural key that cannot distinguish two cases the *output* distinguishes is
not under-specified; it is keyed on the wrong thing. The distinguishing fact is
where the three-character run sits relative to the rest of the printed line,
which is delimiter pairing.

Two consequences I would not want lost in a spec revision:

- **Structure regressions are not a proxy for text regressions here.** Every
  table above shows `structreg = 0` while `txtreg` runs into the thousands.
  The failing shapes are ones already in the queue or the permanent file, so
  every structural counter stays silent while text is destroyed. §2's probe,
  the census's structural tier, and the ratchet's union rule are all blind to
  this in the same way. Any future gate for this family has to lead with the
  text tier at length ≥ 4.
- **§4.3's decline branch is the real cost centre, not the splice table.** Both
  Blocker B and the tail cells' 800 failures are the same event: a licensed
  abutment shortens or re-flanks a run, `emphasis_run` declines, and the
  children print bare into whatever is beside them. §4.3 predicted this and
  §5.3's differencing filter was supposed to catch it; the filter is the one
  piece of the design that Blocker C shows cannot run. That is an unlucky
  coincidence, and it is the piece I would fix first regardless of which cells
  survive — a working deep tier baselined against the shipped ledger is cheap
  (~1 s wall clock, matching the prior report's measurement) and is what would
  have caught all of this before Task 4.

---

## Appendix — reproducing this

All probes were written as `#[ignore]`d tests in the verifier's worktree, using
`census_support` for every render and classification, with `markdown.rs`
unmodified except where noted:

| file | what it does |
|---|---|
| `tests/zz_verify.rs` | the §2.1 and §2.2 tables |
| `tests/zz_cases.rs` | Blocker A/B case dumps, the head/tail asymmetry, tail cell at length 6-7 |
| `tests/zz_stress.rs`, `zz_stress2.rs`, `zz_stress3.rs` | the §2.3 extended-corpus sweeps and the Blocker C composition |
| `tests/zz_stress4.rs` | the backtick/asterisk family split, and the P13 shrink case |

The two experiments that touched `markdown.rs` — widening `LICENSED` to five
cells for the P13 run, and to the tail cell alone for the `properties`/`census`
runs — were reverted immediately; the ratchet demonstration edited two census
files on disk and restored them from git. Nothing was committed, blessed, or
pushed, and the user's working tree at `/workspace` was never written to except
for this report.
