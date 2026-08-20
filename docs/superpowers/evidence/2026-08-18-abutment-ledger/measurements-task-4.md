# Task 4 report — License the splice-site cells, with the deep census

**Status: BLOCKED.** Nothing committed. The working tree is back at `4082e20`,
clean and green (`mise run lint` ok; `cargo test --workspace` all green).

I completed Steps 1–6 far enough to measure the change, and the measurement says
the change as briefed **breaks the writer's core text invariant**. Three
independent tiers agree — the length-3 census text tier, the new deep tier, and
`properties.rs`'s P13. Separately, the deep tier as specified **already fails on
unmodified `main`**, so it cannot ship green even with the four cells reverted.

I did not bless the census, did not commit, and did not weaken any test.

---

## 1. What I built (all reverted; preserved as a patch)

- Five unit tests in `crates/kasane-writer/src/markdown.rs`'s `mod tests`, with
  the brief's exact names and expected strings, using the established `para`
  helper rather than an inline `blocks_to_markdown` call (per the parent's
  interface note).
- `Ledger::LICENSED` widened to the five cells, verbatim from the brief.
- `the_conservative_ledger_licenses_nothing_and_licensed_starts_at_one_cell`
  renamed to `the_conservative_ledger_licenses_nothing`, `count_ones() == 1`
  dropped, both `may_abut` assertions kept.
- `crates/kasane-writer/tests/census_deep.rs`, verbatim from the brief.

Preserved (not in the repo):
- `<scratchpad>/wip/task-4-attempt.patch` — the full diff of the above.
- `<scratchpad>/wip/census_deep.rs` — the deep tier as written.

(`<scratchpad>` = `/tmp/claude-1000/-workspace/cc18a818-d76f-41f2-83bd-89d8dbe86690/scratchpad`)

---

## 2. TDD evidence

### RED — before turning the cells on

`cargo test -p kasane-writer --lib -- _edge_of_ _head_of_ a_strong_wrapping_only`

```
running 5 tests
test markdown::tests::a_strong_wrapping_only_an_emph_still_splices ... ok
test markdown::tests::an_emph_at_the_head_of_a_strong_run_keeps_its_delimiters ... FAILED
test markdown::tests::a_strong_at_one_edge_of_an_emph_run_keeps_its_delimiters ... FAILED
test markdown::tests::a_strong_at_the_head_of_an_emph_run_keeps_its_delimiters ... FAILED
test markdown::tests::an_emph_at_one_edge_of_a_strong_run_keeps_its_delimiters ... FAILED

---- an_emph_at_the_head_of_a_strong_run_keeps_its_delimiters ----
  left: "**ba**"     right: "***b*a**"
---- a_strong_at_one_edge_of_an_emph_run_keeps_its_delimiters ----
  left: "*ab*"       right: "*a**b***"
---- a_strong_at_the_head_of_an_emph_run_keeps_its_delimiters ----
  left: "*ba*"       right: "***b**a*"
---- an_emph_at_one_edge_of_a_strong_run_keeps_its_delimiters ----
  left: "**ab**"     right: "**a*b***"

test result: FAILED. 1 passed; 4 failed; 164 filtered out
```

Exactly the brief's prediction: the four spliced strings `*ab*`, `*ba*`,
`**ab**`, `**ba**`, and `a_strong_wrapping_only_an_emph_still_splices` already
green.

### GREEN — after turning the four cells on

All five pass. `cargo test -p kasane-writer --lib` → **168 passed, 1 failed**,
and the one failure is *not* the test the brief predicted:

```
---- markdown::tests::fusing_nested_emphasis_does_not_leak_its_delimiters ----
markdown.rs:2235: assertion `left == right` failed
  left: "***ab***"
 right: "*ab*"
```

`splicing_mid_buffer_costs_a_span_that_would_round_trip` **passes unchanged**
(see §6). `fusing_adjacent_runs_costs_a_structural_boundary` also passes. Spec
§7 predicted the wrong two tests.

---

## 3. Blocker A — the four cells corrupt recovered text

### The mechanism

The four edge cells are individually safe and **jointly unsafe**. When a run has
a licensed container at *both* its head edge and its tail edge, `edge_to_splice`
asks about each separately, both answer "may abut", and both stand. The printed
line then opens and closes with three-character delimiter runs that CommonMark
does not pair as the writer intends:

```
[Emph[Strong[a]], Emph[a], Emph[Strong[a]]]
  main today: "*aaa*"              recovers "aaa"   (text clean, structure queued)
  four cells: "***a**a**a***"      recovers "aaa**" (TEXT CORRUPT — two asterisks leak)

[Strong[Emph[a]], Emph[a], Strong[Emph[a]]]
  main today: "**aaa**"            recovers "aaa"
  four cells: "***a*a*a***"        recovers "aaa**" (TEXT CORRUPT)
```

Design spec §3.2 explicitly claims this case needs no cell:

> "Both edges collide, distinct containers" is not a triple — it is two
> candidates, each asked separately, and the splice loop's fixpoint handles the
> second after the first resolves.

**That claim is false when both candidates are licensed.** The fixpoint handles
the second *after the first splices*; when neither splices there is no second
iteration and both survive.

### Per-subset measurement (length-3 census, 7,239 shapes)

Baseline = `main`'s shipped `Ledger` (bit 0, `EMPH_OVER_STRONG_WHOLE_RUN`).
`txtreg` = shapes textually clean on `main` that become textually corrupt.

```
subset                                txtreg   q->clean  p->clean   q->p   p->q
(none)                                     0          0         0      0      0
EmStHead                                   0         48         0     28      0
EmStTail                                   0         48         0     28      0
EmStHead+EmStTail                          8        118         0     56      0
StEmHead                                   0          0        48      0      8
EmStHead+StEmHead                          0         48        48     28      8
EmStTail+StEmHead                          0         48        48     28      8
EmStHead+EmStTail+StEmHead                 8        118        48     56      8
StEmTail                                   0          0        48      0      8
EmStHead+StEmTail                          0         48        48     28      8
EmStTail+StEmTail                          0         48        48     28      8
EmStHead+EmStTail+StEmTail                 8        118        48     56      8
StEmHead+StEmTail                          8          0        97      0     37
EmStHead+StEmHead+StEmTail                 8         48        97     28     37
EmStTail+StEmHead+StEmTail                 8         48        97     28     37
ALL FOUR (what the brief asks for)        16        118        97     56     37
```

The head/tail pair of *either* class direction produces 8 text corruptions; both
pairs produce 16.

### Why the committed §2 probe did not see it

Spec §2's `newly_corrupt` column counts only shapes **clean under the baseline**
that become not-clean. All 16 of these were *structurally* corrupt (and
textually clean) on the baseline, so they were never clean and never moved a
counter — in either the `CONSERVATIVE` rows or the `ALL_CELLS_VS_LICENSED` row.
The probe cannot distinguish "corrupt" from "worse than corrupt". §2 already
documented one blind spot in that metric and closed it with a second baseline;
this is a third, orthogonal one.

### Independent confirmation from `properties.rs`

The workspace run failed **P13, `p13_inline_text_survives_rendering`** — the
writer's text-conservation property — and proptest shrank to one of the same
family:

```
cc db4092… # shrinks to inlines =
  [Strong([Emph([Text("a")])]), Emph([Text("a")]), Emph([Emph([Text("a")])])]
```

(That regression line was written to
`crates/kasane-writer/tests/properties.proptest-regressions`; I reverted it,
since committing it would pin a case against a change that is not landing.)

### The "one licensed edge per run" fix does not close it either

I probed the obvious repair — when both edges are licensed, splice the tail one
anyway, so at most one edge per run stands. On the length-3 census it looks
good: `txtreg` 0, 96 queue→clean, 96 permanent→clean. But on the length-4/5 deep
corpus it still leaves **336 text corruptions**, one element further out:

```
[Emph[Strong[a]], Emph[a], Emph[Strong[a]], Emph[a]] -> "***a**a**a**a*"
[Strong[Emph[a]], Emph[a], Strong[Emph[a]], Emph[a]] -> "***a*a*a*a**"
```

So the failure is not "both edges at once"; it is that a licensed edge abutment
produces a 3-character delimiter run whose pairing depends on what follows,
which is the delimiter-pairing question §3.3 says `may_abut` must never ask.
This is not a table arm I can fix — it is the ledger's premise.

---

## 4. Blocker B — 37 shapes move permanent → queue, and they are real regressions

The ratchet gates the **queue** for growth (`check queue … gate` in
`mise.toml`), so any permanent → queue move fails `mise run census-ratchet`.
Turning on either `StEm` cell moves 8; both move 37.

They are not reclassifications. Example:

```
[Text("a"), Strong[Emph[a]], Strong[a]]
  main:  "a**aa**"  -> a<strong>aa</strong>
         stacks: [] [St] [St]   vs IR [] [St,Em] [St]
         = the Em erased inside a St  -> Inexpressible (correctly permanent)

  cells: "a*a*a"    -> a<em>a</em>a
         stacks: [] [Em] []     vs IR [] [St,Em] [St]
         = the <strong> is gone entirely -> Corrupt
```

Mechanism: the two `Strong`s fuse; the `Emph` at the fused run's head edge is
now licensed and stands; the run's own `**` then fails to flank (`a` before,
`*` after) and `emphasis_run` takes its **decline** branch, printing its
children bare and dropping the `<strong>` altogether. This is design spec
§4.3's flanking consequence, and it is a strictly worse output than today's.

The ratchet is right to fail here. This is the direction §6 does *not* worry
about, and it turns out to be the one that bites.

(For the record on §6's actual worry, queue → permanent: the four cells move
**56** shapes that way. I did not enumerate and justify them individually
because the change is not landing; if a revised cell set is approved I will do
that enumeration against the revised bless.)

---

## 5. Blocker C — the deep tier fails on unmodified `main`

I wrote `census_deep.rs` verbatim from the brief, checked out `markdown.rs` at
`4082e20` (i.e. `Ledger::LICENSED` = the single shipped cell), and ran it:

```
2561 newly-licensed spelling(s) do not round-trip -- a cell in `may_abut` is
wrong, not a residual to be recorded:
  structure: [Emph([Strong([Text("a")])]), Text("a"), Text("a"), Text("a")] -> "**a**aaa"
  structure: [Text("a"), Emph([Strong([Text("a")])]), Text("a"), Text("a")]  -> "a**a**aa"
  structure: [Text("*"), Emph([Strong([Text("a")])]), Text("a"), Text("a")]  -> "\\***a**aa"
  structure: [Code("x"), Emph([Strong([Text("a")])]), Text("a"), Text("a")]  -> "`x`**a**aa"
  … 2557 more, all structural, none textual
```

Baseline table (deep corpus, 19,208 shapes; `kept` = renderings differ from
`Ledger::CONSERVATIVE`):

| ledger under test | kept | text failures | structure failures |
|---|---|---|---|
| `main` today (bit 0 only) | 3,513 | 0 | **2,561** |
| the four cells added (bits 0–4) | 11,123 | **744** | 7,771 |

Every one of the 2,561 is the same family: `Emph[Strong[x]]` with any
neighbour. The outer `*` declines to flank, `**a**` prints bare, the `<em>` is
lost. That family is a **recorded residual** — the length-3 instance is line 568
of `census-known-structure-corrupt.txt`:

```
[Emph([Strong([Text("a")])]), Text("a"), Text("a")]
```

So the deep tier's stated contract ("No allowlist and no queue. A corrupt shape
here is a wrong cell in `may_abut`, not a residual.") is **incompatible with the
residual the length-3 census already records for the cell `main` ships**. The
differencing filter does not isolate "spellings this item newly licensed"; it
isolates "spellings the whole ledger changed", and the whole ledger includes
Task 1's cell, whose §4.3 consequence is queued rather than clean.

I did **not** add anything to any allowlist, per hard rule 1.

### Wall-clock (the answer to the §5.3 question)

```
$ time cargo test -p kasane-writer --test census_deep
  test time 1.06s   real 1.36s
$ time cargo test --workspace          # whole suite, with census_deep present
  real 7.9s
```

**Recommendation: keep lengths 4+5.** The tier costs ~1.1 s against a ~7.9 s
workspace suite — about 14 %, nowhere near doubling it. No §5.2 log entry is
needed and I made none. (This number is for the tier's *run*; it holds whether
the tier passes or fails, since it evaluates the whole corpus before asserting.)

---

## 6. Step 7 — the pinned-loss test needed no rewrite, and I made none

`splicing_mid_buffer_costs_a_span_that_would_round_trip` **already pins exactly
what the brief asks it to be rewritten to pin**:

```rust
Inline::Emph(vec![
    Inline::Text("a "),
    Inline::Emph(vec![Inline::Text("b")]),   // same `Delim`, Interior
    Inline::Text(" c"),
])
// asserts para == "*a b c*", recovered == "a b c"
```

That is a same-`Delim` container mid-buffer, refused by
`may_abut(Emph, Emph, Site::Interior)`, which none of the four cells touches —
`bit_for` has no same-`Delim` arm at any site. The test **passed unchanged**
with all four cells on. The brief's Step 4 expectation that it would fail is
mistaken; it appears to have assumed a cross-class shape.

Applying hard rule 2 as written: the old assertion did not break, so there is no
"case the old assertion made" that needs a covering cell, and nothing here is
weakened. The brief's sentence ("The cross-class half of this trade is now
licensed by `cell::STRONG_OVER_EMPH_HEAD_EDGE` / `TAIL_EDGE`…") would be a
doc-comment addition only, and I have not added it, because the cells it names
are not landing.

The test that *did* break, `fusing_nested_emphasis_does_not_leak_its_delimiters`,
I can name cells for — case 2 (`[Emph[Strong[a]], Emph[Strong[b]]]` → `***ab***`,
structurally **Clean**, a genuine recovery) is covered by
`EMPH_OVER_STRONG_HEAD_EDGE` + `TAIL_EDGE` acting on the fused run's children;
case 3 (`[Strong[Emph[a]], Strong[Emph[b]]]` → `***ab***`) is
`STRONG_OVER_EMPH_HEAD_EDGE` + `TAIL_EDGE`, and it is a **regression**
(Inexpressible → Corrupt: em/strong order inverted rather than a level erased).
I did not rewrite it, since the change producing it is not landing.

---

## 7. Bless magnitude vs. Task 3's per-cell figures

I did **not** bless. `git diff --stat crates/kasane-writer/tests/*.txt` is empty;
`census-known-corrupt.txt` is untouched, as spec §6 requires.

Had I blessed, the four cells would have produced:

| | Task 3 predicted | measured here |
|---|---|---|
| queue → clean | ≤ 292 (7-cell union) | **118** |
| permanent → clean | ≤ 97 (7-cell union) | **97** |
| queue → permanent | not measured | **56** |
| permanent → queue | not measured | **37** |
| `census-known-corrupt.txt` (text) grows by | 0 | **16** |

The two recovery columns are consistent with §2 (118 < 292 because the two
`RunSeam` cells stay off; 97 matches the union's permanent figure exactly). The
last three rows are movements §2's probe was structurally unable to measure, and
two of them are the blockers above.

---

## 8. What I need to unblock

The ledger's per-triple, text-blind lookup is not sufficient for the four
container-edge cells. Three questions for you:

1. **§3.2's "both edges need no cell" claim is wrong.** Does the table need a
   site or a rule for "the run already has one licensed edge"? My probe of the
   obvious version (at most one licensed edge per run) still leaves 336 text
   corruptions at length 4/5, so I don't think a positional rule closes it.
2. **§4.3's decline branch defeats the `StEm` cells outright.** Licensing an
   `Emph` at a `Strong` run's edge can make the `Strong` run itself decline and
   vanish. Deciding that requires knowing what flanks the run — the text
   reasoning §3.3 forbids `may_abut` from doing. Should the `StEm` cells be
   dropped from this item?
3. **The deep tier's no-allowlist contract does not survive contact with the
   shipped ledger** (2,561 failures on `main`). Options I can see: baseline the
   differencing filter against `main`'s shipped `LICENSED` instead of
   `CONSERVATIVE` (which reduces the kept set to what *this item* changed), or
   give the tier the length-3 census's queue as an allowlist (which §5.3 refuses
   by design). Which?

I have not chosen among these; each is a spec revision, not an implementation
detail.

---

## 9. Self-review notes

- Used `para(...)` rather than the brief's inline `blocks_to_markdown(...)` +
  `.trim()`, per the parent's interface note. Names and expected strings are the
  brief's verbatim.
- The brief's fifth test, `a_strong_wrapping_only_an_emph_still_splices`, is a
  near-duplicate of the existing
  `a_strong_run_wrapping_only_an_emph_still_loses_its_emph` (same IR shape, "b"
  instead of "a", one assertion instead of three). I added it with a
  cross-reference sentence in its doc comment, but flag it: if the four cells are
  re-scoped, consider dropping it rather than carrying two tests for one refusal.
- The two `RunSeam` cells stayed off throughout. For the record, they are far
  worse than the edge cells: each alone produces **210** text corruptions on the
  length-3 census, and the seven-cell union produces 460.
- All probe scaffolding (`tests/zz_probe.rs`) removed. Tree verified clean via
  `git status --porcelain` (empty) and green via `mise run lint` +
  `cargo test --workspace`.
