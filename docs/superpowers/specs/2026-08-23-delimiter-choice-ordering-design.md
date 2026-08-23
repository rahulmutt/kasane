# kasane — Delimiter-Choice Ordering Design Spec

**Status:** implemented on branch `delimiter-choice-ordering`, with one
correction the census ratchet forced after this document was written — see
§1's headline and §3's fourth condition.
**Date:** 2026-08-23.
**Supersedes the framing of:** the "`_` alphabet" item, as described in
`census-inexpressible.txt`'s header, `AGENTS.md` §census, and the
2026-08-17 probe that sized it.
**Evidence:** `docs/superpowers/evidence/2026-08-23-underscore-alphabet/`.

## 1. Purpose & scope

The writer spells every emphasis container with `*`. Where a container's child
would print the same character, `splice_children` deletes the child rather than
risk a delimiter collision, and the nesting level is lost. 3,714 shapes of the
census's 19-element alphabet are wrong for this reason: 1,730 queued in
`census-known-structure-corrupt.txt`, 1,984 parked in
`census-inexpressible.txt`.

This item makes the delimiter **character** a decision the writer takes *before*
the splice consults it, and lets a run spell itself `_` when that is what keeps
a child alive. Measured, it recovers **1,670** of those 3,714 shapes, with zero
new text loss and — *after the fourth condition of §3* — zero structural
regressions.

It does not widen the writer's alphabet. The alphabet was never the constraint —
see §2.

### 1.1 The "zero regressions" this document claimed, and how it was wrong

As drafted, this section claimed the recovery came "with zero regressions" full
stop. That was false, and a gate caught it.

The claim rested on §2.4's `broken (clean → wrong) = 0`, which was correctly
measured and is still true. But `clean → wrong` is not the whole of
"regression". A shape can also move from `Inexpressible` — a loss already
accounted for and parked as permanent — to `Corrupt`, and probe 2 never counted
that transition.
When the reorder shipped as commit `05bb516`, `mise run census-ratchet` found
**five length-3 shapes** that had done exactly that: an `Emph` saved from the
splice fused into the `Strong` beside it, so a `<strong>` came back as an `<em>`
and its text migrated across a structural boundary. Text was byte-identical
either way, which is why the text tier and the 2.48M-shape length-5 sweep of
§2.6 both stayed at zero and only the structural gate spoke.

Five was the length-3 tip. A structural probe over the full census alphabet at
lengths 4 and 5 —
`docs/superpowers/evidence/2026-08-23-underscore-alphabet/harnesses/structural-len4-5-sweep.sh`,
which runs `main`, `05bb516` and `0909b3a` side by side — measured **135
regressions at length 4 and 3,134 at length 5** for the same reason.

§3's fourth condition, shipped as `0909b3a`, closes every one of them. Re-run
against `main`, the branch then has **zero `Inexpressible → Corrupt` and zero
`clean → wrong` at lengths 3, 4 and 5**, with every recovery intact — 31,376
shapes improved to `Clean` at length 4 and 588,423 at length 5, unchanged from
the unfixed branch to the shape, not merely to the count. That, not the drafted
sentence, is the measurement the headline now rests on.

The transferable half is in §10; the gate that should have spoken earlier and
did not is §6.1.

## 2. The measurement that gates this item, and the premise it destroyed

### 2.1 The prior estimate was measured at the wrong layer

The 2026-08-17 probe (archived, not committed) searched every `*`/`_`
assignment per shape and reported 3,122 of 3,682 structurally-wrong shapes
expressible once `_` is allowed — a projected win of ~2,198. `AGENTS.md` and
`census-inexpressible.txt`'s header both record that figure and both name the
follow-up "the alphabet-widening item".

That probe measured **CommonMark's** capability. It never measured **this
pipeline's**. The two differ, and the difference is total.

### 2.2 `_` at the emission site recovers nothing

Probe 1 (`harnesses/probe-hook.patch`, `harnesses/zz_underscore.rs`) put a hook
at `markdown.rs:380` — the writer's single delimiter-emission site — able to
force any `*`/`_` assignment across a run. Alphabet: the census's 19 elements
plus `Text("_")` plus three multi-child containers, 23 elements, 12,719 shapes
of length 1–3, each rendered inside five enclosing contexts.

| context | baseline wrong | fixed by *any* assignment | broken by the flank-guarded rule |
|---|---|---|---|
| alone | 6,977 | **0** | 0 |
| letter | 8,838 | **0** | 0 |
| punct | 6,977 | **0** | 0 |
| space | 6,977 | **0** | 0 |
| letter/space | 8,015 | **0** | 0 |

Zero, in every context. Not 2,198.

### 2.3 Why: the container is gone before a delimiter is chosen

The emission log records containers present against delimiters actually chosen:

```
Emph[Emph[a]]      containers=2  emissions=1  → *a*     (never  _*a*_ )
Strong[Strong[a]]  containers=2  emissions=1  → **a**   (never __**a**__)
Strong[Emph[a]]    containers=2  emissions=1  → **a**   (never __*a*__ )
Emph[Strong[a]]    containers=2  emissions=2  → ***a*** (already correct)
```

Three of the four shapes reach the emission site with one container already
deleted. `markdown.rs:1478` is why:

```rust
let children = splice_children(run_children(members), want, ledger);  // splices, assuming `*`
…
RunOut::Emitted(emphasize(&inner, markup))                            // markup chosen here
```

The splice resolves a collision against a character that has not been chosen
yet. By the time `markup` exists, the container `_` would have saved is gone.
The three shapes `census-inexpressible.txt`'s header advertises as the payoff
are exactly the three where this fires.

**The prior estimate and the file's own original claim are the same error at
opposite signs.** The header once read "Markdown cannot express this, at any
level" and was corrected on 2026-08-17 because Markdown *can*. The correction
then over-shot: Markdown can, and this writer still cannot, because the loss
happens two decisions earlier. Both statements were measured at the wrong layer.

### 2.4 The reorder, measured

Probe 2 (`harnesses/probe-2-reorder.patch`, `harnesses/zz_sweep.rs`) chooses the
character before `splice_children` and suppresses the splice when the characters
differ. Baselines reproduce the committed files exactly — 1,730 and 1,984 —
before anything is claimed.

| | ship | reorder | Δ |
|---|---|---|---|
| `census-inexpressible.txt` | 1,984 | **428** | −1,556 |
| `census-known-structure-corrupt.txt` | 1,730 | **1,616** | −114 |
| recovered (wrong → clean) | — | **1,670** | |
| broken (clean → wrong) | — | **0** | |
| new text loss | — | **0** | |

114 + 1,556 = 1,670, so **no shape moves from permanent into the queue**; both
files purely shrink. §6 depends on this.

Every figure in that table is a correct measurement of probe 2 and is left as
measured. Note what the table does **not** have a row for: `Inexpressible →
Corrupt`. `broken` counts only `clean → wrong`, so a shape that was already
counted as a loss could get worse without moving any number here. That is the
transition §1.1 is about, and it is why the shipped `census-inexpressible.txt`
is 433 rather than the 428 this row predicts.

### 2.5 The context spread is the finding, not the headline

| enclosing context | recovered | broken |
|---|---|---|
| alone / punct / space | 1,670 | 0 |
| letter + space | 736 | 0 |
| letter both sides | **192** | 0 |

The census renders every sequence in isolation, which is the single most
permissive context for `_`: paragraph boundaries flank it, so it always opens.
Between letters — where emphasis actually sits in prose — the recovery falls by
89%.

Reporting only the 1,670 would repeat the 2,198 mistake in new clothes. Both
numbers are load-bearing and an implementation must not quote one alone.

### 2.5.1 The same wall accounts for the entire residual

Dumping the 428 that remain permanent rather than counting them: **every one of
them contains letter text adjacent to the nested container. Not one lacks it.**
156 have the container first, so it is the *closing* delimiter that is
letter-flanked; the rest are preceded by letter text and it is the opener.

Those 428 are exactly the flanking class, and they are exactly 428 of the
shipped file's 433 — verified shape-by-shape against
`harnesses/p2-residual-permanent.txt`. The other five are §1.1's, and their
mechanism is not this one; §9 says why they must not be folded in.

So §2.5's context spread and the residual are one phenomenon, measured from two
directions — the flanking wall outside the shape and inside it. And refusing is
mandatory, not conservative:

```
_*a*_a   → _<em>a</em>_a          the pair fails, and the literal `_` characters
a_*a*_   → a_<em>a</em>_          land in the text — this is text loss, not
_*a*_ a  → <em><em>a</em></em> a  merely structure loss; a space is enough to fix
```

That is why condition 2 of §3 exists and why §2.4 measures 0 new text loss.
After this item the permanent file finally holds only the class its own header
describes.

### 2.6 Text, exhaustively

Under the reorder, the text tier is **0 of 130,321** at length 4 and **0 of
2,476,099** at length 5. The length-5 run was `--release`, so it is evidence
about text and **not** about the `probe_edges` invariant — see §5.3.

`Text("_")` is inert: 1,141 length-3 shapes containing a literal underscore,
zero text loss under ship behaviour and under the reorder alike. `_` is in
`escape.rs`'s `ALWAYS`, so it is escaped before it can be read as markup. That
was a symmetric-looking assumption; it is now a run measurement.

## 3. The rule

A run spells itself `_` / `__` when **all four** hold. None subsumes another.
Conditions 1–3 are as designed. Condition 4 was added after the census ratchet
falsified §1's headline; see §1.1 for what it caught.

1. **A collision would otherwise fire** — `edge_to_splice` or
   `same_delim_to_splice` names a child. Without this condition `_` would churn
   the spelling of documents that already round-trip, buying nothing. This
   condition is why §2.4 measures 0 broken.

2. **Both flanks permit it** — `before != Flank::Other && after != Flank::Other`.
   CommonMark forbids `_` opening intraword. This is the whole of the §2.5
   spread, and `Flank` already carries exactly the needed trichotomy, so nothing
   new is computed.

3. **The parent run did not spell itself `_`.** `_*a*_` is correct, but a nested
   run consulting only its own flanks would see `_` on both sides — punctuation,
   permissive — take `_` itself, and emit `___a___`, which parses as em-over-
   strong and reintroduces the collision one level down.

4. **Declining the splice actually saves the container** —
   `!fuses_across_classes(raw_children, ledger)`. This one is not about
   spelling. The other three ask whether `_` is *legal* and *useful*; this asks
   whether the child a `_` keeps is worth keeping. A `_` run splices nothing, so
   its children are neighbours in one printed line — and `run_len` groups
   printed neighbours by the character a child is *predicted* to print
   (`Delim::child_ch`, which is `*` for both `Emph` and `Strong`), not by class.
   A saved `Emph` beside a `Strong` is therefore absorbed into it and its text
   comes back wearing a class it was never in. A splice *erases* an emphasis
   level; that fuse *substitutes* one, and the structural census counts a
   substitution as corruption where it counts an erasure as merely
   inexpressible. Where the choice is between them, `*` is the lesser loss.

   Keyed on the **classes**, not on the bare fact of a fuse, and that
   distinction is load-bearing: `[Emph([Emph a]), Emph([Emph b])]` fuses too,
   loses no class doing it, and `_*ab*_` → `<em><em>ab</em></em>` is a genuine
   recovery this must not give back. "Decline on any fusion" would have.

Conditions 2 and 3 close different halves, measured:

```
*_x_*      → <em><em>x</em></em>     inner `_`, punctuation flanks: safe
*a_b_c*    → <em>a_b_c</em>          inner `_`, letter flanks: inert, and the
                                     literal `_` characters land in the text
_a*b*c_    → <em>a<em>b</em>c</em>   outer `_`, inner `*`: both survive
```

Condition 2 is what makes the inner case safe; condition 3 is what makes the
outer case safe. Condition 4 is what makes `[Emph([Emph a]), Emph([Strong a]),
Emph([Emph a])]` print `*a**a**a*` rather than `_*aaa*_`, keeping the
`<strong>`.

Stated once: **two runs collide exactly when they are spelled with the same
character.** That is what `Delim::ch()`'s doc comment has claimed the rule was
since 2026-08-15. This item is where the code starts implementing it.

### 3.1 Why the splice is skipped rather than made cleverer

`same_delim_to_splice`'s doc explains that it splices unconditionally because
separating `*a *b* c*` (safe — whitespace stops the inner delimiters flanking,
so a parser pairs them with each other) from `*a*b*c*` (corrupt — they pair with
the outer pair, dropping the middle character's emphasis) would mean reasoning
about delimiter pairing at splice time: the hand-mirrored CommonMark rule
`2026-08-15-emphasis-seam-design.md` §7 approach A rejects.

When the two characters differ that reasoning is unnecessary. CommonMark pairs
delimiters only with delimiters of the same character, so mis-pairing is
impossible by construction. **This item does not adopt the rejected approach; it
removes the case that needed it.**

## 4. The seam

### 4.1 `Delim` keeps the class, a new `Mark` carries the character

`escape::delim(&Inline) -> Option<Delim>` is unchanged: which class an inline
prints is still intrinsic to the inline. Which *character* an emphasis class is
spelled with stops being intrinsic, so `Delim::ch()` is removed and replaced by

```rust
struct Mark { class: Delim, ch: char }   // (Emph, '*') → "*"    (Strong, '_') → "__"
```

A run's `Mark` is decided once, by §3, before `splice_children` is called.

### 4.2 A child's character is predicted conservatively, in one place

The splice rules ask "does this child collide with me", which needs the child's
character, which the child has not chosen yet. Rather than recurse, one
prediction, stated once: **a child is always predicted `*`.**

That is *exact* when the run chose `_`, because condition 3 forces every child
to `*`. It is *conservative* when the run chose `*`: a child that could have
safely taken `_` is spliced first instead. The cost is a missed recovery, never
a corruption, and the 1,670 of §2.4 was measured with this conservatism in
place.

### 4.3 Functions touched

- `edge_to_splice(children, run: Mark, ledger)` — compares `run.ch` against the
  predicted child character, replacing `want.ch()` vs `inner.ch()`.
- `same_delim_to_splice(children, run: Mark, ledger)` — becomes *same class and
  same character*. Its existing deliberate distinction survives unchanged: a
  `Strong` child inside an `Emph` run is still left alone, because `*a**b**c*`
  round-trips.
- `emphasis_run` — chooses the `Mark`, then splices, then emphasises with
  `run.markup()`. Gains `parent_ch`.
- `may_abut`, `Ledger`, `Site`, `cell::*` — **untouched.** They are reached only
  when the characters match, which is exactly the situation whose semantics are
  unchanged. No cell moves; no ledger re-blessing.

`same_delim_to_splice`'s doc warns that its position-blind `Site::Interior`
query would fight `edge_to_splice` under precisely this kind of widening, since
`edge_to_splice` runs first and defers a child that this function then splices
anyway. Because both rules now agree on the character before either fires, that
conflict cannot arise. The hazard is **retired by this change, not inherited**.

### 4.4 Threading `parent_ch`

Condition 3 needs a nested run to know what its parent chose, so `parent_ch`
rides through `inlines_to_md_flat` and `probe_edges`. It is carried as the
parent's `Mark` — a value the callee needs anyway — rather than as a side
flag.

`probe_edges` is the sharp edge here, and §5.3 states the risk rather than
burying it.

## 5. Risk

### 5.1 What cannot break

Heading anchors. `kasane_gfm::rendered_text` recurses through `Emph`/`Strong`
without emitting delimiters (`walk`, `kasane-gfm`), so it is markup-free.
`anchor_slug` and `path_slug` read it, so no anchor moves, and `kasane-gfm`,
`nav`, `refs`, `balance` and P9–P11 are untouched by this item. This is worth
stating because a large share of this repo couples through that one function.

### 5.2 What is measured not to break

0 broken and 0 new text loss across 12,719 shapes × 5 contexts (§2.4, §2.5); 0
text loss at length 4 over 130,321 shapes and at length 5 over 2,476,099
(§2.6).

### 5.3 The `probe_edges` invariant, stated honestly

`probe_edges` computes a run's flanking edges *without rendering*, and
`debug_assert_eq!(edge, Edge::of(&inner))` is the only thing keeping it in step
with the render it stands in for. A container child's first printed character is
its delimiter — so if `probe_edges` and the render disagree about which
character a child prints, that invariant goes soft in exactly the way its own
doc comment warns about.

Evidence available, and its limits: with the probe patch in, the assert stayed
silent across the full census and the 130,321-shape length-4 tier **in debug**.
The length-5 sweep ran under `--release`, where `debug_assert` compiles out, so
it says nothing here.

Two plan-level requirements follow. `probe_edges` must be given `parent_ch`
explicitly rather than inferring a child's character; and the length-5 sweep
must be re-run **in debug** so it covers the invariant and not only text.

### 5.4 The probe used a global; the implementation must not

Both probes carried `parent_ch` in an atomic, which is why they could be written
without touching signatures. `probe_edges` never saw it. That is acceptable in a
throwaway and is not acceptable in the seam — §4.4 is the requirement, and §5.3
is why.

## 6. The census and the ratchet

Expected motion, measured in §2.4, and what actually shipped:

| file | before | expected | **shipped** |
|---|---|---|---|
| `census-known-corrupt.txt` (text) | 0 | 0 | **0** |
| `census-known-structure-corrupt.txt` | 1,730 | 1,616 | **1,611** |
| `census-inexpressible.txt` | 1,984 | 428 | **433** |
| `census-permanent-count.txt` | 1984 | 428 | **433** |

Every motion is in a direction the ratchet already permits: a bless lowers the
permanent ceiling to match a shrink and never raises it, the structure queue
shrinks, and the union shrinks.

The shipped column differs from the expected one by exactly the five shapes of
§1.1: condition 4 returns them from the queue to the permanent file, so the
queue is five shorter and the permanent file five longer than probe 2 predicted.
The gated union is unchanged either way — 1,616 + 428 = 1,611 + 433 = 2,044,
against 3,714 at base — so the full 1,670-shape recovery survives the
correction whole.

The one hand edit this branch needed is the ceiling raise, 428 → 433, in the
commit that needed it (`permanence_ceiling`'s doc: a bless only ever lowers it).
It is a permanence *claim*, and the asymmetry that makes raising it visible is
deliberate. It is honest here because all five shapes were already permanent at
the base `d4fc510` — the feature commit's bless had removed that headroom
prematurely — and the ratchet reports `0 shape(s) newly permanent`. The drafted
"no hand edit and no `+N` to explain" held only for the three-condition rule.

### 6.1 The gate that should have spoken earlier, and the follow-up it names

`census.rs`'s **structural** tier stops at length 3, and `census_len4.rs` is a
**text**-only tier. So **no shipped gate prices structure above length 3.**
§1.1's defect was caught only because the affected family happens to have a
length-3 member; the same family is 135 shapes at length 4 and 3,134 at length 5, and a
family that started at length 4 would have shipped silently.

That is a real gap in this repo's guards, not a property of this item. A
**structural** length-4 tier is the smallest shipped guard that would have
spoken — roughly a second in release, where `census_len4.rs`'s own doc gives
cost as the reason the longer tiers are not shipped. It is a named follow-up,
not part of this branch. This branch measured the gap with an archived probe
instead —
`docs/superpowers/evidence/2026-08-23-underscore-alphabet/harnesses/zz_structural_len4_5.rs`,
reproducible on demand via the sweep script beside it — rather than assuming it
away.

## 7. Tests

Beyond the re-bless, each of §3's four conditions is pinned separately, since
none subsumes another:

1. The three spellings the header advertises now render: `Emph[Emph[a]]` →
   `_*a*_`, `Strong[Strong[a]]` → `__**a**__`, `Strong[Emph[a]]` → `__*a*__`.
2. **Condition 2**, flank refusal: `[Text("a"), Emph([Emph([Text("a")])]),
   Text("c")]` stays `a*a*c`, unchanged from today.
3. **Condition 3**, parent guard: `___a___` is emitted nowhere.
4. **The conservative prediction's cost**: `Emph[Emph[Emph[a]]]` → `_*a*_`,
   one level short. Pinned so the limit is known rather than silent — and
   pinned as a limit of §4.2, **not** as a representational one. Markdown
   spells that shape: `_*_a_*_` parses as `<em><em><em>a</em></em></em>`. The
   middle run splices its child because §4.2 predicts that child will print
   `*`; a less conservative prediction recovers this. The shape is outside the
   census's 19-element alphabet, so it costs nothing today and is pinned to
   keep the cost visible if that changes.
5. **The preserved distinction**: `*a**b**c*` still round-trips, i.e.
   `same_delim_to_splice`'s class-vs-character split survives §4.3.
6. `census_len4.rs` stays at zero with no allowlist.
7. The length-5 text sweep re-run in debug, as a one-off plan task, not shipped
   — the same call `2026-08-21-declined-run-rescan-design.md` §2.2 made for
   lengths 5 and 6, and for the same reason: it costs minutes, not seconds.
8. **Condition 4**, added after this list was drafted:
   `a_run_declines_underscore_when_the_child_it_saves_would_fuse_into_another_class`
   pins all five shapes of §1.1 end-to-end — the rendered Markdown and the
   recovered HTML, plus an explicit assertion that the `<strong>` survives,
   since that is the actual guarantee. It closes with the control that makes the
   rule's boundary legible: `[Emph([Emph a]), Emph([Emph b])]` still prints
   `_*ab*_`, so simplifying condition 4 to "any fusion" goes red naming the
   recovery it would give back.

## 8. Documentation this item falsifies

All of it is wrong *today*, so it is corrected in this branch rather than
deferred.

- **`AGENTS.md` §census** — "the alphabet-widening item" and the
  1,740-of-1,984 figure. It was never the alphabet. Replace with the ordering
  framing and the measured drop, which shipped as 1,551 (1,984 → 433) rather
  than the 1,556 this line was drafted with; see §6.
- **`census-inexpressible.txt`'s header** — same claim, same correction, plus
  the measured residual and why it is a floor. **The header is generated, not
  stored:** `ratchet()` writes the `INEXPRESSIBLE_HEADER` constant in
  `census.rs` ahead of the entries, and the checker filters `#` lines — so a
  hand-edit of the `.txt` passes the gate and is silently reverted by the next
  bless. Edit the constant and re-bless.
- **`census_support/mod.rs:272`** — `Structure::Inexpressible` still reads
  "Markdown cannot express this shape at any level. Permanent." The 2026-08-17
  correction reached the `.txt` headers and `AGENTS.md` and missed this one.
- **This document.** §1's "zero regressions" and §3's "all three" were
  falsified by the branch's own gate before it merged; §1.1, §3 condition 4,
  §6's shipped column and §6.1 are the correction. Left legible rather than overwritten,
  which is the convention every file above is corrected under.
- **`Delim::ch()`'s doc comment** — "the coincidence that this writer never
  spells emphasis with `_`" stops being true. It should state what now holds:
  the character is chosen per run, and the rule is keyed on the choice.
- **`same_delim_to_splice`'s hazard paragraph** — the conflict it warns about is
  retired by §4.3. It should say so rather than keep warning.

## 9. Non-goals

- **The 428 flanking residual is a floor for `*`/`_`, and §2.5.1 says exactly
  why.** Every one of them is flanked by letter text, where neither character
  can open or close, and where emitting anyway loses *text*. Only an HTML tag
  spells them. This is a floor for the two-character alphabet, not a floor for
  the writer — see the next bullet — and it is emphatically not a queue.

  The shipped permanent file is **433**, not 428: condition 4 adds five shapes
  that are *not* part of this floor. Those five are spellable with `_` — the
  writer declines on purpose, because taking it would cost a class (§3
  condition 4, §1.1). They are a deliberate trade, and if the fuse they dodge is
  ever fixed at its source they leave the file. Do not read them as
  representational limits, and do not fold them into the floor.

  Two cautions on the 428. It is **not** a floor because nesting depth
  exhausts the alphabet: alternation handles depth 3 (`_*_a_*_`), and the first
  draft of this spec claimed otherwise. And it is not corroborated by the
  2026-08-17 probe's ~244: that figure came from a different method over a
  different classification, and the two are merely the same order of magnitude.
- **The `letter` context's 192-vs-1,670 is not a defect of this item.** It is
  CommonMark's left-flanking wall, which stops any delimiter opening between a
  letter and punctuation. Only an HTML tag spells those shapes, which carries a
  product question — HTML in Markdown output — and belongs to its own item.
- **Items 2b (`Ctx::Cell`, `inlines_to_html`) and 2c (block structure) stay
  unstarted and unmeasured.** This item touches neither, and no measurement here
  says anything about them.
- **No search over assignments.** §3 is a fixed rule with a conservative child
  prediction (§4.2). A pre-pass assigning characters across the whole inline
  tree would permit search, but nothing measured needs it.

## 10. Transferable finding

**Ask which layer a measurement was taken at.** The 2,198 estimate was not
sloppy — it was a careful exhaustive search that answered "can CommonMark spell
this" when the question was "can this pipeline emit it". It was believed for six
days and named the item. The check that caught it cost one hook at one line and
counted *emissions against containers* — an instrument that compares what the
pipeline did against what the input contained, rather than comparing the
pipeline's output against an external standard.

This is the fourth head/tail-style asymmetry the writer has produced
(`2026-08-18-abutment-ledger-design.md` §2b.5,
`2026-08-21-declined-run-rescan-design.md` §2.3, §2.5 above), and the second
found only by dumping shape sets rather than comparing counts.

**A second instance, inside this document.** Its first draft asserted that the
428 residual was a floor because alternating chains exhaust a two-character
alphabet, and pinned `Emph[Emph[Emph[a]]]` as the proof. That was reasoned, not
measured, and it was wrong twice over: `_*_a_*_` spells depth 3, and the actual
residual has nothing to do with depth — it is 428 flanking refusals, which
dumping the set showed in one pass (§2.5.1). Both corrections cost one parser
call and one `grep`. A claim about a *floor* asserts that no future change can
help, which is the one class of claim in this repo that nothing downstream
re-examines; it should never be the part of a spec that was reasoned rather than
run.

**A third instance, and it is the same shape as the first.** §2.4 measured
`broken (clean → wrong) = 0` and §1 reported it as "zero regressions". The
measurement was right; the word was wider than the measurement. The transition
that actually regressed — `Inexpressible → Corrupt`, a parked loss turning
into an unparked one — was never in the probe's table, so a set that was already
"wrong" could get worse without moving a counter. Ask which *transitions* a
zero covers, not just which shapes: an allowlist is a place a regression can
hide, because both sides of it read as failure.

That one was caught by a gate rather than by a re-measurement, and only because
the family had a length-3 member — which is §6.1's follow-up, and the reason
that gap is written down here rather than left to the next person to rediscover.
