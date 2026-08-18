# kasane — Abutment Ledger Design Spec

**Date:** 2026-08-18
**Status:** Partly implemented on branch `abutment-ledger`. The seam (`Site`,
`Ledger`, `may_abut`), the shared census oracle and the committed per-cell probe
ship; rendered output is byte-identical to `main`, because the licensing this
item existed to do was measured and abandoned. See §2b.
**Parent specs:** `2026-08-16-structural-census-design.md` (§8, the residual
program), `2026-08-16-cross-class-edge-splice-design.md` (§6, whose "closing the
fusion share is out of scope here" is this item's scope).
**Repo:** kasane

## 1. Purpose & scope

Two of the writer's four delimiter rules are conservative by construction: they
collapse rather than gamble.

- `run_end` (`markdown.rs:454-467`) groups a run by the delimiter's
  **character**, so any adjacent `Emph` or `Strong` merges into one run
  regardless of class, and the boundary between them is gone before anything
  else is asked.
- `splice_children` (`markdown.rs:641`) replaces a colliding container with its
  own children, by an edge rule keyed on the character and a rule keyed on the
  `Delim` that looks everywhere.

Both discard structure on shapes whose plain `*`-only spelling already
round-trips. An archived probe priced that at **924 shapes** — 390 in
`census-known-structure-corrupt.txt` and 534 in `census-inexpressible.txt`,
where they sit under the claim that no writer change can ever close them. This
item recovers them without widening the alphabet — re-measured at 389, see §2.

**In scope:** both halves. Shapes in the queue and shapes currently filed
permanent, since the same writer change reaches both and splitting them would
leave the census asserting that shapes this branch fixes are unfixable.

**Out of scope:** the alphabet. Nothing here emits `_`. See §8.

## 2. The measurement that gates this item

**No count in this spec was load-bearing until this measurement ran.** The
924/390/534 split quoted in §1 comes from a probe that was archived rather
than committed, and whose finer 302/118/10 sub-split was found unreproducible
and cut from the edge-splice spec's §6. That is a specific, recent failure of
exactly this evidence, and it is the reason this section exists.

The re-measurement is committed at `crates/kasane-writer/tests/census_probe.rs`
rather than archived, and is re-run with:

```text
cargo test -p kasane-writer --test census_probe -- --ignored --nocapture
```

It renders every shape in the length 1-3 census (`census_support::shapes`)
under each [`Ledger`](#51-the-knob) cell in isolation, and once more under the
union of all seven cells, comparing each rendering against the same shape
rendered under `Ledger::CONSERVATIVE` — the all-bits-off ledger. `CONSERVATIVE`
is **not** today's shipped output: it is pre-`0ac2c48` output, one cell below
what `main` ships as `LICENSED` (`markdown.rs`'s `Ledger` doc comment). A shape
counts `newly_clean` if it is clean (`census_support::text_is_clean` and
`classify_with` both agree) under the cell/union and was not clean under
`CONSERVATIVE`, split by which of the two *structural* census files lists it
(`census-known-structure-corrupt.txt`, the queue; `census-inexpressible.txt`,
permanent) — the table below is scoped to those two files and says nothing
about `census-known-corrupt.txt` (32 shapes, text tier; §6 expects it
untouched). `newly_corrupt` counts the reverse: clean under `CONSERVATIVE`,
not clean under the cell/union. Both files were read in their pre-branch
state: no cell beyond `emph_over_strong_whole_run` had been turned on yet, and
no bless had moved either file, so the counts below are the baseline this
item's later blesses are measured against.

Every cell's `newly_corrupt` column measured `0`: no cell licensed here
corrupts a shape that was clean under `CONSERVATIVE`.

| cell | queue newly_clean | permanent newly_clean | newly_corrupt |
|---|---|---|---|
| `emph_over_strong_whole_run` | 0 | 0 | 0 |
| `emph_over_strong_head_edge` | 48 | 0 | 0 |
| `emph_over_strong_tail_edge` | 48 | 0 | 0 |
| `strong_over_emph_head_edge` | 0 | 48 | 0 |
| `strong_over_emph_tail_edge` | 0 | 48 | 0 |
| `emph_beside_strong_run_seam` | 52 | 0 | 0 |
| `strong_beside_emph_run_seam` | 52 | 0 | 0 |
| **`ALL_CELLS` (union, not a sum)** | **292** | **97** | **0** |

`ALL_CELLS` is not the column sum: one shape can be recovered by more than one
cell, and a cell can recover a shape only once another cell has stopped a
fusion from swallowing it (§4.3). `emph_over_strong_whole_run` measures `0`
against both files not because it recovers nothing, but because it is the one
cell already in `Ledger::LICENSED` (Task 1) — the shapes it recovers were
already blessed out of both census files by the `cross-class-edge-splice`
branch (`0ac2c48`/`ffc9162`/`bf58001`) before this probe ran, so against the
*current* files it has nothing left to take.

**The `CONSERVATIVE`-only rows above have a blind spot.** `newly_corrupt`
there only catches a regression among shapes clean under `CONSERVATIVE`; a
shape `LICENSED` already recovers — clean under `LICENSED`, not clean under
`CONSERVATIVE` — sits outside that comparison entirely, since a regression on
it leaves both `clean` and `was_clean` false and moves no counter. The probe
closes that gap with one further measurement, the seven-cell union compared
against a `LICENSED` baseline instead:

```text
ALL_CELLS_VS_LICENSED,shipped_baseline,389,0
```

`0` in the last column: no cell, alone or combined, corrupts a shape that
ships clean **today** either. This is the row that actually licenses calling
the `newly_corrupt` gate an early warning for the item — the `CONSERVATIVE`
rows alone would have missed a regression on anything `LICENSED` already
fixed.

**The measurement disagrees with 924/390/534, and per this section's own rule
the measurement wins: the archived probe was wrong.** The combined recovery
this item licenses is **389 shapes total — 292 from the structure queue, 97
from the permanent file** — not 924, and not the 390/534 split. §6's counts
are corrected to these measured numbers in the same commit that records this
table.

## 2b. The disproof

**The licensing this spec exists to authorize did not ship, and §3-§5 describe
an abandoned design.** The seam shipped (Task 1, §3.1 and §4), the shared census
oracle shipped (Task 2), §2's probe shipped (Task 3), and `Ledger` itself
shipped (§5.1). The cells did not: exactly one cell is licensed on branch
`abutment-ledger` — `EMPH_OVER_STRONG_WHOLE_RUN`, reproducing the deleted
`sole_child_nests_canonically` — so rendered output is byte-identical to `main`
and no census file moved. **No cell set is safe to license**, and the reason is
the ledger's key, not its arms.

The evidence is two documents in
`.superpowers/sdd/2026-08-18-abutment-ledger/`: the blocked implementer's
measurements (`task-4-report.md`) and a second agent's independent verification
in an isolated worktree (`task-4-verification.md`), which reproduced the
length-3 table cell for cell and then measured what nobody had — lengths 4-7,
and the census's own 19-element alphabet. Where they differ, the verification
governs. **The baseline throughout is the shipped ledger** (bit 0 alone, what
users get today), never `CONSERVATIVE`, so every regression below is against
what ships. Three corpora: the length-3 census, 7,239 shapes over 19 elements;
§5.2's deep corpus, 19,208 shapes over 7 elements at lengths 4-5; and an
extended corpus the verification added, the census's own 19 elements at lengths
4 and 5 — 130,321 + 2,476,099 = 2,606,420 shapes swept per cell.

### 2b.1 No cell set is safe to license

Under this item's literal criterion — zero text regressions on the length-3
census and on §5.2's deep corpus, zero permanent -> queue moves — **exactly one
non-empty subset survives:** `EMPH_OVER_STRONG_TAIL_EDGE`. It recovers **48**
queue shapes and **0** permanent ones, at the cost of **28** queue -> permanent
moves, each of which §6 requires justified individually. That is about 16% of the
queued share this item was scoped to recover (§2's 292) and none of the
permanent share (§2's 97). **§6's "shrinks by 292 / shrinks by 97" is not
reachable by any subset of the seven cells.**

Under the criterion that actually matters — no text loss at all — **none
survives.**

- `EMPH_OVER_STRONG_TAIL_EDGE` loses recovered text on **800** shapes at lengths
  4-5 over the census's own 19-element alphabet (16 at length 4, 784 at length
  5). It measures 0 on §5.2's deep corpus, and at lengths 6 and 7 over that
  corpus's alphabet as well, only because those seven elements cannot witness its
  failure mode at all — see §5.4.
- `EMPH_OVER_STRONG_HEAD_EDGE`, the other subset the length-3 table showed clean
  on every metric, loses text on **168** shapes on §5.2's own deep corpus and
  **3,328** on the extended one.
- The mirror pair behaves identically: `STRONG_OVER_EMPH_HEAD_EDGE` loses 168 on
  the deep corpus and 3,328 on the extended one; `STRONG_OVER_EMPH_TAIL_EDGE`
  loses 0 on the deep corpus and 800 on the extended one.
- All four edge cells together: **16** text regressions at length 3, **744** on
  the deep corpus, **640** at length 4 of the extended one.
- The two `RunSeam` cells were never close: **210** text regressions each at
  length 3 alone, **460** for the seven-cell union.

Length-3 cleanliness does not imply length-4 cleanliness for any of these cells,
and §5.2's deep corpus does not imply the census's own alphabet.

### 2b.2 §3.2's "both edges need no cell" claim is false

§3.2 asserts that "both edges collide, distinct containers" needs no cell,
because "the splice loop's fixpoint handles the second after the first
resolves." It does not. `edge_to_splice` returns the first candidate that is
*not* licensed; when both are licensed it returns `None`, `splice_children`'s
`while let` exits on its first iteration, and both containers stand:

```text
[Emph[Strong[a]], Emph[a], Emph[Strong[a]]]
  shipped     "*aaa*"           recovers "aaa"    text ok,   structurally Corrupt
  head+tail   "***a**a**a***"   recovers "aaa**"  TEXT LOST, structurally Clean
```

The fixpoint handles the second candidate *after the first splices*; when
neither splices there is no second iteration. Note the second line: the shape
becomes structurally `Clean` while its text is destroyed, which is why §2's
probe reads 0 (§2b.5).

**The correction that matters is that "both edges" is not the generator.** It is
a length-3 artifact, and believing it invites the repair "license at most one
edge per run" — a repair that is already measured dead, at **336** text
corruptions on lengths 4-5. A *single* licensed edge corrupts text at length 4:

```text
[Emph[Strong[a]], Emph[a], Emph[Strong[a]], Emph[a]]   head cell alone
  "***a**a**a**a*"   recovers "aaa*a*"   want "aaaa"   TEXT LOST
```

Here the tail candidate is a bare `Text`, which `edge_to_splice` rejects before
`may_abut` is ever consulted — "both edges licensed" is not present. The mirror
shape under the tail cell round-trips fine.

### 2b.3 The premise fails, not an arm

Split the extended corpus's text losses by which delimiter leaks into the
recovered text:

| cell | backtick family | asterisk family |
|---|---|---|
| `EMPH_OVER_STRONG_HEAD_EDGE` | 800 | **2,528** |
| `EMPH_OVER_STRONG_TAIL_EDGE` | 800 | **0** |
| `STRONG_OVER_EMPH_HEAD_EDGE` | 800 | **2,528** |
| `STRONG_OVER_EMPH_TAIL_EDGE` | 800 | **0** |

Head and tail are the same question by this spec's own symmetry, and `may_abut`
answers them identically — yet a head licence mis-pairs an asterisk on 2,528
shapes and a tail licence on **zero**. A structural key that cannot separate two
cases the *output* separates is not under-specified; it is keyed on the wrong
thing. The distinguishing fact is where the three-character delimiter run sits
relative to the rest of the printed line: **the failure is positional, and
`may_abut`'s key is structural.**

Deciding it correctly means reasoning about how a parser pairs delimiters —
which is exactly what §3.3 forbids `may_abut` from doing, and what AGENTS.md
records this repo has refused three times. **This is the ledger's premise, not a
wrong arm.** No further cell, no sixth `Site`, and no "already has one licensed
edge" rule reaches it; only becoming the resolver §3.3 exists to prevent does.

### 2b.4 The `StrongOverEmph` cells fail separately, through §4.3

Licensing an `Emph` to stand at a `Strong` run's edge can make the `Strong` run
itself fail to flank, so `emphasis_run` takes its decline branch, prints the
children bare, and drops the `<strong>` entirely:

```text
[Text("a"), Strong[Emph[a]], Strong[a]]
  shipped    "a**aa**"  ->  a<strong>aa</strong>   stacks [] [St] [St]   Inexpressible
  StEmHead   "a*a*a"    ->  a<em>a</em>a           stacks [] [Em] []     Corrupt
```

Today's output erases the inner `<em>` — the single erasure
`differs_only_by_erasure` forgives, and the reason the shape is filed permanent.
The licensed output loses the `<strong>` outright and promotes to top level an
`<em>` the IR nests inside it. That is strictly worse, and **37 shapes move
permanent -> queue** under both `StrongOverEmph` cells (8 under either alone).

`mise run census-ratchet` catches this, and **only through its queue gate**. The
verification simulated one such move and ran the real task:

```text
set          base     head    delta   verdict
text           32       32       +0   ok
queue        1698     1699       +1   FAIL -- 1 added
           [Code("x"), Code("x"), Emph([Emph([Text("a")])])]
perm         1984     1983       -1   ok
union        3682     3682       +0   ok
census ratchet FAILED: the allowlists may only shrink against main.
```

**The union is unchanged.** The union rule §6 leans on — a shape may move between
the two files, but none may become corrupt that was not — lets this through. §6
worries about queue -> permanent laundering; this is the opposite direction, and
it is the one that bites.

### 2b.5 Every structural gate on this branch is blind to this family

`structreg` — shapes structurally `Clean` under the shipped ledger that stop
being clean — measured **0 in every single row** of every table above: all
nineteen length-3 rows, all eight deep-corpus rows, every extended-corpus row.
Over those same rows, text losses run to 3,328 for one cell and 744 for four.
§2's `ALL_CELLS_VS_LICENSED,shipped_baseline,389,0` row is that same 0, and it is
precisely the reassurance that does not survive contact with length 4.

The reason is structural rather than accidental: the shapes that lose text are
already in the queue or the permanent file, so no structural counter has
anything to move. §2's probe, the census's structural tier, and the ratchet's
union rule are blind to this family in the same way, for the same reason.
**This is the most transferable finding on the branch.** A structural green is
not evidence about text; any future gate for a change of this kind has to lead
with the text tier at length >= 4.

Only the text tiers spoke. `properties.rs`'s P13
(`p13_inline_text_survives_rendering`) fails with the four edge cells on, shrinking
deterministically to `[Strong[Emph[a]], Emph[a], Emph[Emph[a]]]` -> `***a*a*a***`
recovering `aaa**`; with `EMPH_OVER_STRONG_TAIL_EDGE` alone it is 16/16 green,
which is the same blindness one corpus further out.

## 3. The seam

> **Shipped, and disproven as a licensing mechanism.** §3.1's interface and
> §3.3's refusal are what Task 1 built and what the writer carries today. §3.2's
> hypothesis table is the abandoned part: its two container-edge rows and its
> `RunSeam` row license six cells, every one of them was measured separately, and
> not one is safe (§2b.1); its "two rows need no cell" paragraph is false
> (§2b.2). §2b.3 is why the key, not the rows, is the defect. Kept intact — the
> next attempt needs to know exactly what was tried.

### 3.1 Interface

One function owns the question the two rules currently answer separately, with
different keys, in prose that has to keep them consistent by hand:

```rust
/// May a delimiter run of `inner`'s class stand adjacent to a run of
/// `outer`'s class at this structural site, or must it be collapsed?
fn may_abut(outer: escape::Delim, inner: escape::Delim, site: Site, ledger: Ledger) -> bool

/// Where in the printed stream the abutment would happen.
enum Site {
    /// `inner` is the run's entire printing content.
    WholeRun,
    /// `inner` is the run's first printing child, and is not its only one.
    HeadEdge,
    /// `inner` is the run's last printing child, and is not its only one.
    TailEdge,
    /// `inner` sits between other printing content.
    Interior,
    /// The boundary between two adjacent runs, which `run_end` is deciding
    /// whether to fuse.
    RunSeam,
}
```

`Site` is closed. It names structural positions, not text.

`may_abut` itself is two functions in one, both table-shaped and neither
reading text: a `match` on `(outer, inner, site)` that names, for each
licensed triple, which bit of a [`Ledger`](#51-the-knob) it belongs to and
returns `None` for everything unlisted; and the licensing check itself, which
looks that bit up and tests it against the `ledger` argument. The `match` is
the table §3.2 describes; the `Ledger` it's keyed against is what makes the
same table answer differently depending on which cells are turned on. An
unlisted triple has no bit at all, so it is refused under every `Ledger`
including one with every other bit set — that is what keeps the default
conservative regardless of how many cells later widening turns on.

### 3.2 The table

The body is a `match`, one arm per licensed triple, each carrying the measured
row that licenses it. **An unlisted triple returns `false`**, which is today's
behaviour — so the default is the conservative rule, and every recovery in this
item is an explicit, individually-licensed entry.

The edge-splice spec's §2 table gives the starting hypothesis for the
container sites. It is a hypothesis, not the table:

| outer | inner | site | hypothesis | source |
|---|---|---|---|---|
| `Emph` | `Strong` | `WholeRun` | `true` | edge-splice §2 row 1 |
| `Strong` | `Emph` | `WholeRun` | `false` | edge-splice §2 row 7 — tie-break always resolves em-outermost |
| `Emph` | `Strong` | `HeadEdge` / `TailEdge` | `true` | edge-splice §2 rows 2, 3, 6 |
| `Strong` | `Emph` | `HeadEdge` / `TailEdge` | `true` | edge-splice §2 rows 4, 5 |
| any | same `Delim` | any | `false` | `splice_children`'s `Delim` rule; unchanged |
| any | any | `RunSeam` | **unmeasured** | §2 produces these |

Two rows from the edge-splice boundary table need no cell of their own. "Both
edges collide, distinct containers" is not a triple — it is two candidates, each
asked separately, and the splice loop's fixpoint handles the second after the
first resolves. "Merged run longer than three characters" is determined by the
class pair alone: `Emph`+`Strong` and `Strong`+`Emph` merge to three, and every
pair that would exceed three is a same-`Delim` pair the table already refuses.
No length parameter is needed, and adding one would invite a caller to compute
what the pair already decides.

**The first of those two claims is false — see §2b.2.** When both edge
candidates are licensed there is no first resolution, so there is no second
iteration, and both containers stand. The correction is narrower than the defect:
a *single* licensed edge corrupts text at length 4, so "both edges" is a length-3
artifact rather than the generator (§2b.2), and the repair it suggests is
measured dead at 336 text corruptions.

### 3.3 Why this is a ledger and not the pairing mirror

AGENTS.md records that telling a safe spelling from a corrupting one "means
reasoning about how a parser pairs delimiters, the mirror this repo has refused
three times." This item does not lift that refusal. `may_abut` reads no text,
takes no buffer, and computes nothing: it is a lookup keyed on two enum values
and a structural position, and it falls through to `false`. The writer's
existing `can_open`/`can_close` remain the only place CommonMark's own rules are
spelled out, and they are unchanged.

The discipline this requires is a reviewer instruction, not just a comment: **an
arm of `may_abut` that inspects anything beyond its three parameters is the
mirror re-entering by the back door**, and should be rejected in review even if
its answer is correct.

## 4. The two call sites

> **Both call sites shipped as described** (Task 1), routing through the seam
> with one cell licensed, byte-identical to `main`. §4.3 is the part that
> matters in hindsight: it named the interaction that killed the item, and
> understated it (§2b.4, §5.4).

### 4.1 `splice_children`

`edge_to_splice` keeps its shape — find the first and last printing child whose
delimiter *character* matches the run's — and replaces its
`sole_child_nests_canonically` exemption with the seam:

```
site = WholeRun   if the candidate is the run's only printing child
       HeadEdge   if it is the first of several
       TailEdge   if it is the last of several
splice the candidate iff !may_abut(want, candidate_delim, site)
```

`sole_child_nests_canonically` is **deleted**, not kept alongside. It is exactly
the `(Emph, Strong, WholeRun)` cell, and two spellings of one rule is how the
edge rule and the `Delim` rule drifted apart in the first place. Its doc
comment's three load-bearing conditions survive as the table's three
distinctions: the class check is the `(outer, inner)` key, the
single-printing-child check is `Site::WholeRun`, and the caller invariant about
`Backtick` is preserved because `edge_to_splice` still filters on the character
before it asks.

`same_delim_to_splice` also routes through the seam, asking
`may_abut(want, want, Interior)`. The table answers `false`, so **behaviour is
identical to today** — but the refusal is now written in the one place item 2
will come looking for it, rather than only in a paragraph of `splice_children`'s
doc.

Deeper nesting needs no special case: the splice loop already repeats to a
fixpoint, and edge-splice §3.3 traces `Emph[Strong[Emph[x]]]` through it.

### 4.2 `run_end`

Today it extends while the next element is vacuous or shares the delimiter
character. It gains one clause:

```
extend while  renders_empty(next)
           || (delim_char(next) == ch && !may_abut(class_so_far, next_class, RunSeam))
```

When the ledger says the two may abut, the run stops and the next run starts:
`[Emph(a), Strong(b)]` prints `*a***b**` rather than fusing to one `*ab*` run.

**The ordering trap.** A run's *class* comes from its first printing member,
which the emit loop computes from `members` only after `run_end` has returned
the extent — so consulting a class-keyed seam inside `run_end` looks circular.
It is not. `run_end` walks left to right, and the first printing member is
encountered before any later member, so `class_so_far` — the class of the first
printing member seen so far, or `None` while every member so far is vacuous — is
well-defined at every seam. While it is `None` the seam is not consulted and the
run extends as today, which is correct: a run of purely vacuous members prints
nothing, so no abutment exists to license.

This is spelled out here because it is precisely the kind of ordering detail
that produces a defect invisible in either function read alone — the failure
mode §6 of the edge-splice spec had to walk back.

### 4.3 The flanking consequence

Declining to fuse shortens `end`, so the emit loop's
`after_class = next_class(&items[end..])` now classifies the *following run's*
`*` as `Flank::Punct` where it previously saw whatever followed the fused run.
That can flip `can_open`/`can_close` and send `emphasis_run` down its decline
branch, which renders children bare and — by its own doc comment — exposes a
seam that nothing re-scans.

This is the sharpest risk in the item. It is invisible in the ledger, because
the ledger answers a structural question and this is a text-position
consequence of the answer. §5 is what has to catch it, and the differencing
filter in §5.3 is chosen specifically so that it catches it whether or not
anyone predicted the interaction.

**Measured: this is the item's whole cost centre, and it reaches further than
this subsection says.** The decline branch is what turns a licensed abutment
into the 37 permanent -> queue moves of §2b.4 *and* into the 800 text losses of
§2b.1 — the same event both times, a run re-flanked into printing its children
bare. It is reached from `splice_children`, not only from the shortened runs
`run_end` produces, which is a case this subsection does not cover. And §5.3's
filter, the stated mitigation, is the one piece of the design that cannot run
(§5.4).

## 5. The deep census

> **Never committed.** `Ledger` (§5.1) shipped; the tier itself was written
> verbatim from §5.2/§5.3, measured, and discarded, because as specified it
> fails 2,561 times on unmodified `main` and its corpus cannot witness the
> failure it exists to catch. §5.4 records both defects. The design content
> below is left intact because those two defects are the transferable part.

### 5.1 The knob

One `#[doc(hidden)] pub` test seam, the same convention as `est_tokens`,
`path_slug_of`, and `section::canonicalize_inlines`:

```rust
#[doc(hidden)]
pub struct Ledger(u32);

impl Ledger {
    pub const CONSERVATIVE: Ledger = Ledger(0);
    pub const LICENSED: Ledger = Ledger(/* the shipped cells, or'd together */);
    pub const CELLS: &'static [(&'static str, u32)] = &[/* every named cell */];
    pub fn from_bits(bits: u32) -> Ledger { Ledger(bits) }
    pub fn bits(self) -> u32 { self.0 }
}
```

A bitset rather than the two-value mode this section originally specified.
The two-value shape cannot price cells separately: it can only ask "old
behaviour or new," which is exactly the granularity the last probe's finer
302/118/10 sub-split needed and did not have — that split was found
unreproducible and cut (§2), the specific, recent failure this design is
built not to repeat. A bitset lets §2's probe and this section's own deep
census hold every other cell fixed and flip one, so a bad cell's cost is
attributable to that cell rather than smeared across whatever else `Licensed`
happened to also turn on.

`CONSERVATIVE` (the empty set) forces every `may_abut` call to `false`,
reproducing today's output byte for byte. `LICENSED` is what the writer
ships with. Two consumers of the seam: §2's probe, and §5.3's filter. It is a
test seam and not API for the same reason the others are — the alternative
is a copy of the writer's own rules in a test, which drifts.

### 5.2 The corpus

A second alphabet, seven elements, chosen to put both emphasis classes next to
each other and next to a non-emphasis delimiter:

```
Text("a"), Text("*"), Code("x"), Emph[Text("a")], Strong[Text("a")],
Emph[Strong[Text("a")]], Strong[Emph[Text("a")]]
```

At lengths 4 and 5 that is 7^4 + 7^5 = 2,401 + 16,807 = **19,208 shapes**,
roughly 2.6x the existing census's 7,239.

Lengths 4 and 5 rather than 3 for a specific reason, and it is not that the
licensed configurations are absent at length 3 — several appear there. It is
that their *consequences* need the extra elements. §4.3's flanking flip needs a
licensed abutment with a neighbour on **both** sides, which is four elements;
two licensed abutments in one printed stream, where the first one's shortened
run changes the second one's flank class, is five. A length-3 corpus can witness
a cell's answer but not what the answer does to its neighbours, and neighbour
interaction is this item's stated risk.

### 5.3 What it asserts

Each shape renders twice, under `Licensed` and under `Conservative`. **Only
shapes whose two renderings differ are kept**, and every kept shape is asserted
clean against the oracle on both the text tier and the structural tier.

No allowlist, no queue, no permanent file. A corrupt shape in this tier is a
wrong cell in `may_abut`, not a residual to be recorded — which is what makes
the tier's meaning exact: *every spelling this item newly licensed round-trips.*

The filter is computed by differencing rather than by matching shapes someone
anticipated, and that is the design decision that covers §4.3: a flanking
regression changes the rendering, therefore lands in the kept set, therefore is
asserted, without anyone having to predict it.

If the tier's wall-clock does not fit PR CI alongside `mise run test`, length 4
alone is authorized as a fallback (2,401 shapes) — **logged in the spec and the
PR body, not dropped silently**, per the "no silent caps" rule the census
program has followed since the 2a item.

### 5.4 Correction: the tier as designed cannot run, and its corpus is blind

`crates/kasane-writer/tests/census_deep.rs` was written verbatim from §5.2 and
§5.3, measured, and **never committed**. It has two defects, either one
disqualifying.

**Its differencing filter baselines against the wrong ledger.** §5.3 keeps the
shapes whose `LICENSED` and `CONSERVATIVE` renderings differ, which isolates
*spellings the whole ledger changed*, not *spellings this item newly licensed* —
and the whole ledger already includes Task 1's shipped cell, whose §4.3
consequence is queued rather than clean. Run as specified against unmodified
`main`, the tier fails **2,561 times**, out of 3,513 kept shapes. Every one of
the 2,561 is a top-level `Emph[Strong[..]]` with a neighbour, and its length-3
instance is a residual the census already records — line 568 of
`census-known-structure-corrupt.txt`:

```text
[Emph([Strong([Text("a")])]), Text("a"), Text("a")]
```

So §5.3's contract — "a corrupt shape in this tier is a wrong cell in
`may_abut`, not a residual to be recorded" — contradicts a residual the length-3
census records for the cell the writer already ships. **The refinement that kills
the obvious repair: 436 of the 2,561 classify `Inexpressible`, not `Corrupt`**
(the other 2,125 are `Corrupt`), so handing the tier the length-3 queue as an
allowlist — which §5.3 refuses by design anyway — would still leave 436
failures. Only re-baselining the filter against the shipped `LICENSED` fixes
both halves.

**§5.2's corpus cannot witness the failure it exists to catch.** Its seven
elements include `Code("x")` and `Emph[Strong[a]]` but **no emphasis container
wrapping a code span** — no `Emph[Code("x")]`, no `Strong[Code("x")]`. Both are
in the length-3 census's own alphabet, and they are precisely what witnesses
`EMPH_OVER_STRONG_TAIL_EDGE`'s 800 text losses (§2b.1):

```text
[Code("x"), Emph[Code("x")], Emph[Strong[a]], Text("a")]
  shipped     "`x`*`x`a*a"    recovers "xxaa"    text ok, structurally Corrupt
  tail cell   "`x``x`**a**a"  recovers "x``xaa"  TEXT LOST
```

With the tail `Strong` licensed to stand, the `Emph` run's `*` no longer flanks,
`emphasis_run` declines, the children print bare, and the leading `` `x` `` of
`Emph[Code("x")]` abuts the preceding `` `x` `` into one code span. That family
is a pre-existing writer defect the tail cell *reaches* rather than creates —
§8's backtick non-goal — but relative to the shipped ledger it is still text
loss, on 800 shapes that render cleanly today. **The tier as designed would have
run green on that cell and blessed it as clean.** The lesson generalises past
this item: a differencing filter is only as good as the alphabet it differences
over, and a corpus narrowed for speed carries a blind spot someone has to name.

**Wall-clock was never the constraint.** The tier ran in 1.06s against a 7.9s
workspace suite, about 14%. §5.3's authorized length-4 fallback was not needed
and was not taken.

## 6. What happens to the four census files

- **`census-known-corrupt.txt` (32, text tier)** — expected untouched. This item
  changes structure, not recovered text. If the bless moves this file, that is a
  finding to diagnose before blessing, not a diff to accept.
- **`census-known-structure-corrupt.txt` (1,698)** — shrinks by the queued share
  (measured: 292, §2 — not the archived 390).
- **`census-inexpressible.txt` (1,984)** — shrinks by the permanent share
  (measured: 97, §2 — not the archived 534).
- **`census-permanent-count.txt` (1,984)** — a bless lowers it to match the
  shrink. No hand edit, no ceiling bump.

`mise run census-ratchet` gates the text queue, the structure queue, and their
union with the permanent file against the merge base, for **growth only**.
Shapes leaving all three files pass it unaided.

**One movement needs per-shape review.** The permanent/queue split is computed
on every bless, and its second condition — `differs_only_by_erasure` — compares
recovered structure against the IR, which this change alters. A shape can
therefore newly satisfy it and move queue -> permanent. The union rule permits
this, and it is the one direction that can launder a regression as a
representational limit — the exact failure that put 748 shapes in the permanent
file on a premise this item disproves. **Every queue -> permanent move must be
named and justified individually in the PR body.** A bless diff showing such a
move without that justification is not reviewable.

## 7. Tests

1. **Existing length-3 census.** The bless diff, read line by line, is the
   primary evidence. §6 states what each file is expected to do.
2. **Deep census**, `crates/kasane-writer/tests/census_deep.rs`, per §5.
3. **Unit, in `markdown.rs`'s test module.** One pinned printed string per
   licensed cell, *and one per refusal that stays* — including
   `(Strong, Emph, WholeRun)` and the `Interior` same-`Delim` arm — so a later
   widening cannot flip them without a visible test change. One test asserts the
   default arm: an unlisted triple returns `false`.
4. **`run_end`'s ordering.** A direct test that a vacuous leading member leaves
   `class_so_far` unset and does not consult the seam, pairing with the existing
   `a_vacuous_leading_member_does_not_downgrade_the_run_class`.
5. **Properties (`tests/properties.rs`)** unchanged and green, P1-P13.

**Two existing tests are predicted to fail and need rewriting:**
`fusing_adjacent_runs_costs_a_structural_boundary` and
`splicing_mid_buffer_costs_a_span_that_would_round_trip`. Both exist to pin a
deliberate loss, and some of those losses are what this item recovers. This is
the one place in the work where a green suite is bought by editing a test, so
each rewrite must state **which ledger cell now covers the case**. If no cell
does, the test is right and the change is wrong.

**Both predictions are wrong, measured with the four edge cells added to the
shipped one: both tests pass unchanged.** `splicing_mid_buffer_costs_a_span_that_would_round_trip`
already pins what this section wanted it rewritten to pin — a same-`Delim`
container mid-buffer, refused by `may_abut(Emph, Emph, Interior)`, which no cell
touches because `bit_for` has no same-`Delim` arm at any site. Nothing here is
weakened, because nothing here broke. The test that *does* break is
`fusing_nested_emphasis_does_not_leak_its_delimiters`, which renders `***ab***`
where it pins `*ab*`; of the two cases behind that, one is a genuine recovery and
one — `[Strong[Emph[a]], Strong[Emph[b]]]` — is a regression, `Inexpressible` ->
`Corrupt` with em/strong order inverted rather than a level erased. Point 2 of
this list is moot — the deep census was never committed (§5.4) — and point 5 is
violated: P13 fails with the cells on (§2b.5).

## 8. Non-goals

- **Widening the alphabet to `_`.** Item 2, priced by the same probe at ~2,198
  further shapes, and the item that must also measure whether `_` breaks shapes
  that are clean today — the probe examined only already-failing ones. It also
  owns making the permanence condition per-position.
- **The 32-shape backtick text family** (`census-known-corrupt.txt`), whose fix
  shape is known and recorded at emphasis-seam spec §8. **Measured since: not
  separable from this item after all.** Every one of
  `EMPH_OVER_STRONG_TAIL_EDGE`'s 800 text losses is this family, reached through
  §4.3's decline branch rather than created (§5.4). Any retry has to close it
  first, or measure and accept those 800 in the open.
- **Items 2b** (`Ctx::Cell`, `inlines_to_html` — no exhaustive sweep of any kind
  exists) **and 2c** (block structure). Both unstarted and unmeasured.
- **The genuinely unspellable residual.** ~560 shapes, ~370 of them with no
  same-class nesting at all, blocked by CommonMark's left-flanking rule rather
  than by alternation exhaustion: `[Text("a"), Text("a"), Emph([Code("x")])]`
  cannot open a delimiter between a letter and a backtick with `*` or `_`. Only
  an HTML tag spells those, and no alphabet widening reaches them.
- **Re-scanning the seam `emphasis_run`'s decline branch exposes.** Pre-existing,
  documented in that branch's own comment, and unchanged here. §4.3 is about
  reaching that branch more often, not about fixing what it does.

## 9. Verification and risk

`mise run lint && mise run test` green, with `lint` covering `--all-targets`
plus `fmt --check`, then `mise run census-ratchet`. The proof specific to this
item is the bless diff plus the deep tier, both read rather than accepted.

Risks, in the order they deserve worry:

1. **The flanking interaction (§4.3).** Shortening a run changes the following
   flank class, which can push `emphasis_run` down its unscanned decline branch.
   Mitigation: §5.3's differencing filter, which catches it without predicting
   it. This is the risk most likely to turn a recovery into a text loss.
2. **The ledger drifting into a resolver.** Mitigation: the closed `Site` enum,
   the `false` default, and §3.3's explicit reviewer instruction.
3. **Deep-tier wall-clock.** Mitigation: measure before committing; §5.3's
   authorized fallback to length 4, logged.
4. **The measurement disagreeing with 924/390/534.** This happened: §2's
   committed probe measured 389 (292/97), not 924 (390/534). Per §2's own
   rule the measurement won, and the archived counts were corrected in the
   same commit that recorded the new ones.
5. **queue -> permanent laundering (§6).** Mitigation: per-shape justification
   in the PR body; the union ratchet alone does not catch it.

**Which of these materialised.**

1. **Risk 1 is what killed the item, and it killed it more broadly than §4.3
   anticipated.** The flanking interaction reaches the splice call site as well
   as the fusion one: it is the mechanism behind both the 37 permanent -> queue
   moves (§2b.4) and the 800 text losses (§2b.1, §5.4). Its stated mitigation is
   §5.3's differencing filter, which is the one piece of the design that cannot
   run (§5.4) — so the risk ranked first arrived with its guard missing.
2. **Risk 2 did not materialise, and that is the finding.** The ledger never
   drifted into a resolver: the closed `Site`, the `false` default and §3.3's
   reviewer instruction held for the entire branch. Becoming a resolver is
   exactly what it would have had to do to work (§2b.3). The mitigation was
   effective and the design was wrong — the discipline protected a premise that
   could not carry the item.
3. **Risk 3 did not materialise.** 1.06s against a 7.9s workspace suite (§5.4).
4. Unchanged: this happened, as this row already records.
5. **Risk 5 is real and measured** — 56 queue -> permanent moves under the four
   cells, 28 under the one that survives the literal criterion (§2b.1) — but the
   *opposite* direction, which this list does not name, is what the ratchet
   actually caught (§2b.4).

**And one risk nobody listed, which is the branch's most transferable finding:
that structural gates would stay silent through all of it.** `structreg` measured
0 in every row of every table while text losses ran into the thousands (§2b.5).
