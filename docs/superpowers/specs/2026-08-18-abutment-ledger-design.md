# kasane — Abutment Ledger Design Spec

**Date:** 2026-08-18
**Status:** Designed. Not implemented.
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
item recovers them without widening the alphabet.

**In scope:** both halves. Shapes in the queue and shapes currently filed
permanent, since the same writer change reaches both and splitting them would
leave the census asserting that shapes this branch fixes are unfixable.

**Out of scope:** the alphabet. Nothing here emits `_`. See §8.

## 2. The measurement that gates this item

**No count in this spec is load-bearing until task one runs.** The 924/390/534
split comes from a probe that was archived rather than committed, and whose
finer 302/118/10 sub-split was found unreproducible and cut from the
edge-splice spec's §6. That is a specific, recent failure of exactly this
evidence, and it is the reason this section exists.

Task one is a committed re-measurement:

1. Render every shape in `census-known-structure-corrupt.txt` and
   `census-inexpressible.txt` twice — once under the licensed ledger (§3), once
   under an all-`false` ledger reproducing today's output byte for byte.
2. Parse both with the census's own oracle (`pulldown-cmark` 0.13, the option
   set in `census.rs::parser_options`).
3. Record, per shape: unchanged / newly clean / newly corrupt.

It ships as an `#[ignore]`d test beside the census, not as a script somebody
runs and describes. The knob it needs is §5.1's, which the deep census needs
anyway — the probe is not extra machinery, it is the first consumer of the
machinery this item builds.

If the measurement disagrees with 924/390/534, **the measurement wins** and this
spec's counts are corrected in the same commit that records them.

## 3. The seam

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

## 5. The deep census

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

## 6. What happens to the four census files

- **`census-known-corrupt.txt` (32, text tier)** — expected untouched. This item
  changes structure, not recovered text. If the bless moves this file, that is a
  finding to diagnose before blessing, not a diff to accept.
- **`census-known-structure-corrupt.txt` (1,698)** — shrinks by the queued share
  (hypothesis: 390).
- **`census-inexpressible.txt` (1,984)** — shrinks by the permanent share
  (hypothesis: 534).
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

## 8. Non-goals

- **Widening the alphabet to `_`.** Item 2, priced by the same probe at ~2,198
  further shapes, and the item that must also measure whether `_` breaks shapes
  that are clean today — the probe examined only already-failing ones. It also
  owns making the permanence condition per-position.
- **The 32-shape backtick text family** (`census-known-corrupt.txt`), whose fix
  shape is known and recorded at emphasis-seam spec §8.
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
4. **The measurement disagreeing with 924/390/534.** Mitigation: §2 — the
   measurement wins, and it runs before any rule is written.
5. **queue -> permanent laundering (§6).** Mitigation: per-shape justification
   in the PR body; the union ratchet alone does not catch it.
