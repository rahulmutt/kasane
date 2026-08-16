# kasane — Cross-Class Edge Splice Design Spec

**Date:** 2026-08-16
**Status:** Implemented on branch `cross-class-edge-splice`.
**Parent spec:** `2026-08-16-structural-census-design.md` (§6, "What this item
finds and does not fix", which prices this defect at 2,002 shapes and hands it
"its own item and its own design").
**Repo:** kasane

## 1. Purpose & scope

`splice_children`'s edge rule keys on the delimiter **character**, and
`Delim::ch()` maps both `Delim::Emph` and `Delim::Strong` to `'*'`. A `Strong`
at the edge of an `Emph` run therefore matches the edge rule and is spliced
away:

```
IR       Emph[Strong[Text("a")]]
printed  *a*
parsed   <em>a</em>          the <strong> is gone
correct  ***a***             parses as <em><strong>a</strong></em>
```

The structural census counts 2,002 shapes in this family, 49% of its corrupt
set — `1,056` containing `Emph[Strong[…]]`, `1,056` containing
`Strong[Emph[…]]`, `110` containing both. Bold inside italic silently ceasing to
be bold is a worse symptom than the 32-shape text family still open, and the
family is ~60× larger.

This item narrows the edge rule so the expressible half round-trips, and moves
the inexpressible half into the permanent file where it belongs. It does not
drain the family to zero, because half of it cannot be spelled with `*` alone.

## 2. What the probe measured

The edge rule is over-broad, not wrong. In most configurations the abutting
`*`/`**` is exactly the canonical spelling and round-trips today if the splice
simply does not happen. Measured against `pulldown-cmark` 0.13, the parser the
census already uses as its oracle:

| IR | un-spliced spelling | parses as | |
|---|---|---|---|
| `Emph[Strong[b]]` | `***b***` | `<em><strong>b</strong></em>` | ✓ |
| `Emph[Text(a), Strong[b]]` | `*a**b***` | `<em>a<strong>b</strong></em>` | ✓ |
| `Emph[Strong[b], Text(a)]` | `***b**a*` | `<em><strong>b</strong>a</em>` | ✓ |
| `Strong[Text(a), Emph[b]]` | `**a*b***` | `<strong>a<em>b</em></strong>` | ✓ |
| `Strong[Emph[b], Text(a)]` | `***b*a**` | `<strong><em>b</em>a</strong>` | ✓ |
| `Emph[Code(x), Strong[b]]` | `` *`x`**b*** `` | `<em><code>x</code><strong>b</strong></em>` | ✓ |
| `Strong[Emph[b]]` | `***b***` | `<em><strong>b</strong></em>` | ✗ order flipped |
| `Emph[Strong[b], Text(a), Strong[c]]` | `***b**a**c***` | `<em><strong>b</strong>a</em><em>c</em>**` | ✗ shredded |
| `Emph[Strong[Emph[b]]]` | `****b****` | `<strong><strong>b</strong></strong>` | ✗ |
| `Strong[Emph[Strong[b]]]` | `*****b*****` | `<em><strong><strong>b</strong></strong></em>` | ✗ |

The middle column is what a *blanket* un-splice would print — the edge rule
never firing — which is the hypothetical the measurement had to price. It is not
what §3 emits: the narrow rule declines only on the two safe whole-run rows, so
the last four rows keep splicing and keep the output they have today.
`Emph[Strong[Emph[b]]]` prints `***b***` under §3, not the `****b****` the table
prices (§3.3 traces it).

The boundary the measurements draw:

| configuration | verdict |
|---|---|
| one edge collides (head or tail), container does not span the run | safe, both orders, any neighbour |
| whole-run coverage, outer `Emph` | safe — `***x***` is the canonical spelling |
| whole-run coverage, outer `Strong` | unsafe — the tie-break always resolves em-outermost |
| both edges collide, distinct containers | unsafe |
| merged run longer than three characters | unsafe |

The asymmetry in the middle two rows is the whole item. A merged `***` run is
split by the parser according to what closes first; when the colliding container
spans the entire run, both the opening and the closing run are the merged
three, the split is decided by CommonMark's fixed tie-break, and that tie-break
is always em-outermost. `Emph[Strong[x]]` is what the tie-break produces.
`Strong[Emph[x]]` is what it destroys, and no `*`-only spelling produces it —
`<strong><em>x</em></strong>` requires `**_x_**`.

### 2.1 Why the alphabet makes whole-run coverage the dominant case

`census.rs`'s alphabet contains exactly two cross-class elements, `em(st(t))`
and `st(em(t))`, both single-child. A sequence element that is one of those
forms a run of one member whose entire content is the inner container —
whole-run coverage, considering that member in isolation. The mass of the
2,002 shapes genuinely does sit in the two whole-run rows by member type:
most of the family is built by placing one of these two single-child
elements somewhere in a length-1-to-3 sequence, not by constructing a
multi-child single-edge shape directly.

**That does not mean a rule covering only those two rows wins nearly the
whole expressible half — an earlier draft of this section argued exactly
that, and it was wrong.** `run_end` (§6 below) groups a delimiter run by
**character**, not by `Delim`: any `*`-delimited neighbour — `Emph` or
`Strong`, whichever class — fuses with a whole-run-coverage member the
moment the two sit next to each other in the same short sequence, which is
common once sequences reach length 3. Fusion destroys the very thing
"whole-run coverage" was standing in for — `sole_child_nests_canonically`'s
sole-printing-child precondition — before `edge_to_splice` or
`same_delim_to_splice` ever asks whether the member's own content nests
canonically. Measured directly, by rendering the actual post-fix queue
family through the real writer (§6): of the 548 queued shapes containing
`Emph[Strong[…]]`, only 118 (22%) are genuinely isolated in this sense; 430
(78%) sit next to a real `*`-delimited neighbour and fuse with it. Fusion,
not the writer's left-flanking rule on its own, is what keeps most of the
family from round-tripping — §6 gives the full measured split, including
what happens to the ones that do fuse. §5.1 records what the gap between
this section's estimate and the measured result costs the fix's coverage.

## 3. The change

### 3.1 `edge_to_splice` keys on the `Delim`, not the character

The function cannot currently tell the safe case from the unsafe one because it
is handed a `char`, and the entire distinction lives in the class:

```rust
fn edge_to_splice(children: &[Flat<'_>], want: escape::Delim) -> Option<usize>
```

with `ch = want.ch()` computed inside. This makes the two candidate sources'
signatures parallel — `same_delim_to_splice` already takes `want` — and states
the rule in one place instead of leaving it inferable from the call site.

`Delim::ch()` itself does not change. The character-keying is correct and
well-argued (`escape.rs:527-542`, and `2026-08-15-emphasis-seam-design.md`
§2.1): two runs collide when they share a character. What was missing is the one
configuration where the collision is not a corruption.

### 3.2 The exception

```rust
/// Whether the edge candidate at `idx` is the run's entire content and nests
/// the one way `*` alone can spell.
///
/// `Emph` wrapping nothing but a `Strong` prints `***x***`, and a parser
/// splitting that run resolves it em-outermost — which is what the IR meant,
/// so the splice would destroy a shape that round-trips. The converse does
/// not hold: `Strong` wrapping nothing but an `Emph` prints the same
/// `***x***` and resolves the same way, against the IR, so it keeps splicing
/// and is filed inexpressible (§4).
fn sole_child_nests_canonically(
    children: &[Flat<'_>],
    idx: usize,
    want: escape::Delim,
) -> bool
```

True when all three hold:

1. `idx` is the **only** printing child, by the same `renders_empty` test
   `edge_to_splice` already uses to find the edges;
2. the child's own `Delim` **differs** from `want`;
3. `want == Delim::Emph`.

Condition 2 is load-bearing even though it looks redundant. Without it the
predicate would also decline the splice for `Emph[Emph[x]]`, and the behaviour
would happen to stay correct only because `same_delim_to_splice` catches that
shape a moment later. Stating it makes the rule true by construction rather than
by the ordering of two other rules.

`same_delim_to_splice` is untouched: same-class nesting still splices
unconditionally, for the reason `splice_children`'s doc gives — telling a
corrupting same-class nest from a safe one is the hand-mirrored CommonMark
delimiter-pairing logic `2026-08-15-emphasis-seam-design.md` §7 approach A
refuses.

A declined candidate cannot loop. With `want == Emph` and the child a `Strong`,
the `Delim` rule looks for an `Emph` and finds none, so the loop terminates with
the container in place.

### 3.3 It composes under nesting without a special case

`Emph[Strong[Emph[b]]]`: the outer `Emph` run declines and keeps the `Strong`;
the inner `Strong` run does not qualify (condition 3 fails), splices its `Emph`,
and prints `**b**`; the whole prints `***b***`. The inner `Emph` is lost — but
that is the pre-existing `Strong`-outer limit reappearing one level down, not a
new corruption, and the shape is filed inexpressible by §4 on the strength of
the `Strong[Emph[…]]` it contains.

### 3.4 Approaches rejected

- **The enumerated-safe whitelist.** Also un-splice the single-edge cases
  (`*a**b***`, `***b**a*`), guarded on exactly one edge colliding and the merged
  run being no longer than three. It wins strictly more, and more again once
  items 2b and 2c widen the alphabet. Rejected here because that guard *is*
  delimiter-pairing reasoning, and because most of what it licenses is
  unreachable by the current alphabet — the census could not prove it safe.
  Shipping a rule broader than the instrument that guards it is the failure this
  whole program is a response to. §3.2's predicate is written so this is a later
  widening of the same function rather than a rewrite.
- **Swapping the splice direction** — rewriting `Strong[Emph[x]]` into
  `Emph[Strong[x]]` so everything becomes spellable. Rejected on sight: it
  silently changes what the document means, which is worse than dropping a level
  visibly.
- **Alternating `*` and `_`.** The only thing that makes both nesting orders
  spellable. Rejected by the fusion item's §7 B and again by the emphasis-seam
  spec's Non-goals, both for the flanking and intraword-`_` rules it imports.
  Rejected a third time here for the same reason.

## 4. The census relation gains a second inexpressibility condition

`Strong[Emph[x]]` has no `*`-only spelling, so leaving it in a target-zero queue
misrepresents it. Moving it needs the relation to say why.

**Condition 1 gains a disjunct**, a sibling of `nests_same_class_directly`:

```rust
/// Whether `seq` contains a `Strong` whose sole child is an `Emph`.
///
/// Directional on purpose. `***x***` always resolves em-outermost, so
/// `<strong><em>x</em></strong>` has no `*`-only spelling; `Emph[Strong[x]]`
/// has one and is fixed in §3. Matching both orders here would let a
/// regression of the fixed family launder itself into the permanent file.
fn nests_strong_over_emph_directly(seq: &[Inline]) -> bool
```

recursing through containers and `Link` exactly as its sibling does.

**Condition 2 gains the drop.** `differs_only_by_collapse` becomes
`differs_only_by_erasure`, normalizing each position's stack by collapsing
adjacent identical classes *and* dropping an `Emph` immediately preceded by a
`Strong`, applied to both walks and iterated to a fixpoint.

A **drop**, not a swap. §3 leaves `Strong[Emph[x]]` spliced, so it prints
`**x**` and the parser recovers `[Strong]` against an IR of `[Strong, Emph]` —
the level is deleted, not reordered. Nothing in this design ever prints
`***x***` for a `Strong`-outer shape, so a swap normalization would never fire.

One normalization, not two rules applied as separate sweeps, because a shape
can need both erasures at once, and a two-sweep implementation — collapse
every adjacent same-class pair across the whole stack, then drop every
`Emph`-after-`Strong` across the whole stack, as two independent passes —
would stall on some shapes. `Strong[Emph[Emph[a]]]` prints `**a**`, giving an
IR stack of `[Strong, Emph, Emph]` against a recovered `[Strong]`: a naive
two-sweep version collapses the two `Emph`s first to reach `[Strong, Emph]`,
then drops in a second sweep to reach `[Strong]` — two passes, with `[Strong,
Emph]` genuinely materialized in between.

That is not what the implemented `differs_only_by_erasure` does, and the
difference is worth stating precisely rather than glossing. It runs both
rules in a single scan, testing each element against the last *kept* element
of the output being built rather than against its original predecessor: both
`Emph`s in `[Strong, Emph, Emph]` are tested against the same kept `Strong`
and dropped in the same pass, so `[Strong, Emph]` is never materialized and
the fixpoint is reached in one pass, not two. A pass's output is a fixpoint
by construction — nothing is pushed after an element it equals, or after a
`Strong` it would be dropped against — so the surrounding loop confirms the
result is unchanged and exits without doing further work. The loop is kept as
a cheap guard, not because this two-rule, two-class rule set needs genuine
multi-pass iteration — it does not, for any shape the current alphabet can
build — but because adding a third rule or a third class would make one-pass
sufficiency stop being obvious, while the loop is already correct for that
case too.

```rust
if (nests_same_class_directly(seq) || nests_strong_over_emph_directly(seq))
    && differs_only_by_erasure(&ir, &got)
{
    return Structure::Inexpressible;
}
```

**The laundering hazard is closed by condition 2's directional drop; condition
1's direction is a secondary belt, not the primary guard.** If §3's fix
regresses and `Emph[Strong[x]]` loses its `<strong>`, the IR stack is `[Emph,
Strong]` against a recovered `[Emph]`. The drop removes an `Emph` immediately
preceded by a `Strong`, never a `Strong` preceded by an `Emph`, and this stack
has the latter — the normalization does not touch it, the walks stay unequal,
the shape is `Corrupt`, and it lands in the queue where the ratchet fails the
build, **whatever condition 1 concludes about the shape**.

That last clause is load-bearing, not a flourish: condition 1 is scoped to the
whole shape (§4's own recursion, §7), so it can be satisfied by same-class
nesting that has nothing to do with the regression being reasoned about. 37
shapes in the permanent file today contain both the fixed `Emph[Strong[x]]`
pattern and an unrelated direct same-class nest (`Emph[Emph[…]]` or
`Strong[Strong[…]]`) elsewhere in the same sequence, so
`nests_same_class_directly` already holds for them independent of whether the
`Emph[Strong[x]]` position regresses. A regressed shape drawn from that set
does *not* have the property an earlier draft of this paragraph claimed for
every regressed shape — "satisfies neither condition-1 predicate" — because it
satisfies the same-class disjunct regardless of the regression. It is still
caught, because condition 2 is what is actually doing the work:
`differs_only_by_erasure` fails on the `[Emph, Strong]`-vs-`[Emph]` mismatch
at the regressed position independent of what condition 1 concludes elsewhere
in the shape. The same holds for the 110 shapes carrying both cross-class
orders anywhere in the census's full alphabet (§1's pre-fix count, not §6's
94 — those are the subset of the 110 that are still queued after this item's
fix; the other 16 moved to the permanent file and none are in the text-tier
allowlist): they qualify as inexpressible on their `Strong`-outer half, but a
regression in their `Emph`-outer half survives normalization and breaks
condition 2 just the same.

`INEXPRESSIBLE_HEADER` needs rewriting. It currently explains only
`<em><em>x</em></em>` and says a shape lands in the file by containing a
same-class container, which stops being the whole truth.

## 5. Tests

### 5.1 Census

The instrument is the existing one; this item re-blesses it and reads the diff.
Measured at bless:

| file | before | after |
|---|---|---|
| `census-known-corrupt.txt` (text) | 32 | 32 |
| `census-known-structure-corrupt.txt` (queue) | 2,812 | 1,698 |
| `census-inexpressible.txt` (permanent) | 1,236 | 1,984 |

366 shapes went clean: `2,812 − 1,698 − (1,984 − 1,236) = 366`.

That is well short of §2.1's estimate. §2.1 argued the alphabet's two
whole-child cross-class forms dominate and reasoned the fix should win "nearly
the whole expressible half" — informally, ~946 shapes going clean against a
queue drained to ~810. The measured result is 366 clean against a queue of
1,698: roughly a third of the predicted win, not nearly all of it. The gap is
not a miscount; it is two mechanisms behind §6's 548-shape residual, and the
dominant one is not the one an earlier draft of this section named. §6 gives
the measured split, obtained by rendering the queue's actual 548-shape
`Emph[Strong[…]]`-only family through the real writer: 430 of them (78%)
**fuse** with an adjacent `*`-delimited sibling elsewhere in the same short
sequence — `run_end` groups by character, not `Delim` (§2.1, §6) — which
destroys the sole-printing-child precondition the exemption needs before the
splice rules ever run. Only 118 (22%) are the isolated case originally
described here, blocked purely by `emphasis_run`'s left-flanking rule
(`can_open`) with no fusion involved. §2.1 counted occurrences of the
pattern; it did not check whether a same-character neighbour was standing
next to most of them.

### 5.2 Pinned relation edges

Two new, alongside the three existing, so the bless path is not the only thing
asserting the relation:

- `[Strong([Emph([Text("a")])])]` **is** inexpressible.
- `[Emph([Strong([Text("a")])])]` is **neither** inexpressible nor corrupt — it
  is clean after §3. This is the guard that matters most: it fails loudly if the
  fix regresses, and it fails if condition 1 ever loses its direction.

### 5.3 Unit, in `markdown.rs`'s test module

- `Emph[Strong[a]]` prints `***a***` and recovers `<em><strong>a</strong></em>`.
- `Strong[Emph[a]]` prints `**a**`, pinned as a cost so a later reader meets it
  as a decision rather than a surprise.
- `Emph[Emph[a]]` still prints `*a*` — the control for §3.2's condition 2.

### 5.4 Two existing tests are predicted unchanged

`fusing_nested_emphasis_does_not_leak_its_delimiters` (every case has two
printing children, or `want == Strong`) and
`a_same_class_container_mid_buffer_is_spliced` (three printing children after
fusion). If either moves, that is evidence the predicate is wrong — not a test
to re-bless.

## 6. Non-goals

- **Draining the family to zero.** ~1,056 shapes are inexpressible with `*`
  alone. They move file, not state.
- **The single-edge configurations.** §3.4's first rejected approach. They stay
  spliced and stay in the queue, along with `Emph[Text(" "), Strong[a]]`, which
  `emphasize` would make safe by trimming the space outward. Both are
  deliberately conservative losses.
- **The queue's 1,698 remaining entries**, which measurement (§5.1) shows
  decompose disjointly into four families rather than the single undiagnosed
  tail this section originally named:
  - **810** contain neither `Emph([Strong(` nor `Strong([Emph(` — the tail
    `2026-08-16-structural-census-design.md` §8 already flags as having no
    named mechanism. This item does not diagnose it.
  - **548** contain `Emph([Strong(` only — a residual family this item does
    not close, and one §2.1's original draft mis-explained. Measured by
    rendering every one of the 548 through the real writer (not inferred
    from reading the splice code alone), two mechanisms are in play, and
    `splice_children`'s edge rule is demonstrably the majority one — the
    opposite of what an earlier draft of this bullet claimed:
    - **430 (78%) fuse with an adjacent `*`-delimited sibling.** `run_end`
      (`markdown.rs:454-466`) groups a run by delimiter **character**, not
      by `Delim`: any neighbouring `Emph` or `Strong`, regardless of class,
      merges into the same run as the target member the moment the two are
      adjacent, which is common at the census's length-3 sequences. Fusion
      destroys `sole_child_nests_canonically`'s sole-printing-child
      precondition (§3.2) before it is ever asked whether the member's own
      nesting is canonical, so the exemption cannot fire. Which rule then
      splices the target's `Strong` away depends on which member prints
      first in the fused run, since that decides the run's class and markup
      (`markdown.rs:287–297`):
      - **302 (55%)** land in a run classed `Emph` (a real `Emph` sibling
        prints first), so the ordinary edge rule (`edge_to_splice`) removes
        the target's `Strong` outright and the bold text disappears
        entirely — `[Text("a"), Emph([Text("a")]),
        Emph([Strong([Text("a")])])]` prints `"a*aa*"`, no `**` anywhere.
      - **118 (22%)** land in a run classed `Strong` (a real `Strong`
        sibling prints first), so the target's own `Strong` is now nested
        *inside* a run of its own class and `same_delim_to_splice` removes
        it the way it always removes same-class nesting, unconditionally —
        the letter survives but merges into the neighbour's own bold run
        with no trace of the original `Emph[Strong[…]]` boundary —
        `[Text("a"), Strong([Text("a")]), Emph([Strong([Text("a")])])]`
        prints `"a**aa**"`.
      - **10** carry more than one `Emph[Strong[…]]` occurrence and land in
        both outcomes across the different occurrences in the same shape.
    - **118 (22%) are genuinely isolated** — no fused neighbour — and fail
      for the mechanism an earlier draft of this bullet named exclusively:
      `emphasis_run`'s left-flanking rule (`can_open`) declines to spell the
      outer `*` because it is preceded by a letter and followed by the
      inner `**`'s punctuation, regardless of what §3.2's predicate decided
      — `[Text("a"), Emph([Strong([Text("a")])])]` prints `"a**a**"`.

    (302 + 118 + 118 + 10 = 548; the two 118s are coincidentally equal and
    name different populations — one is fused-but-visually-bold, the other
    unfused-and-blocked.) Fusion, not left-flanking, is the mechanism behind
    most of §5.1's 366-vs-~946 shortfall. Closing the fusion share is out of
    scope here: it means grouping `run_end` by `Delim` as well as character,
    or widening `sole_child_nests_canonically` to survive fusion — not
    changing the edge rule this item already narrowed.
  - **246** contain `Strong([Emph(` only — these satisfy condition 1 (§4) but
    fail condition 2's `differs_only_by_erasure`, so they stay queued rather
    than moving to the permanent file.
  - **94** carry both orders — the post-fix queue subset of §1's pre-fix,
    whole-alphabet count of 110 (§4 disambiguates the two).
- **Widening the alphabet**, and therefore also **making condition 1
  per-position**. Both belong to the item that widens, per the 2a spec's §8.
- **Block structure and the merged-table HTML path.** Items 2c and 2b.

## 7. Verification and risk

`mise run lint && mise run test` green, with `lint` covering `--all-targets`
plus `fmt --check`. The proof specific to this item is the bless diff, read
rather than accepted.

**The text tier is the first number to check, not the last.** The fix changes
printed bytes, and a text-tier regression — scrambled characters — is strictly
worse than the structural defect being closed. If `census-known-corrupt.txt`
grows by even one line, the approach is wrong.

Risks, recorded rather than closed:

- **Blast radius at the seam.** `Emph[Strong[a]]` emits `***a***` where it
  emitted `*a*`, so every neighbouring run sees a different left and right
  context — the exact seam the emphasis-seam item hardened. The census is
  exhaustive over sequences of length 1–3, so every neighbour pairing the
  alphabet can build is covered. Nothing covers length 4 and beyond.
- **Inline depth.** Declining a splice keeps a container the old rule
  flattened, so these shapes render one level deeper and shapes near
  `MAX_INLINE_DEPTH` could truncate where they previously did not.
  `inline_depth.rs` is the check, and it needs an explicit look rather than an
  assumption. **Measured, not assumed:** the deepest input depth at which
  content survives is 255 for both a same-class chain and an alternating
  cross-class chain — equal, so the retained container costs no headroom and
  the risk is not realized. The reason is structural, not coincidental: the
  writer's depth guard fires on the input IR's structural depth, walked
  before any splicing decision, so declining a splice cannot consume depth
  budget that guard has already accounted for. `inline_depth.rs`'s
  `cross_class_nesting_truncates_no_earlier_than_same_class` pins this by
  comparing the two helpers against each other rather than asserting a
  literal depth, so the assertion survives any future change to
  `MAX_INLINE_DEPTH`'s value.
- **The permanent file nearly doubles** and becomes the majority of the
  corrupt set. Predicted at ~2,292 permanent against a queue of ~810 (a ~74%
  share); measured, the permanent file grew to 1,984 (up from 1,236, a gain of
  748, not the ~1,056 predicted) against a queue of 1,698 (down from 2,812) —
  a smaller majority, ~54% of the 3,682-shape corrupt set, than predicted, but
  still a majority, because the queue also drained far less than §2.1
  estimated. §5.1 and §6 record why. A reader can still take the permanent
  file's growth as the project giving up. The mitigations are that the file is
  computed on every bless, that §5.2's two edges assert the direction
  condition 1 rests on, and that the header is rewritten to explain both
  mechanisms.
- **Sequencing.** That growth lands while the CI ratchet — the third residual
  item — is still unbuilt, and it is precisely the guard against a future bless
  laundering a shape into the permanent file. `ratchet()` today is a
  file-equality check against the checked-in file, not a comparison against
  `main`, so it cannot tell a legitimate reclassification from a quiet
  acceptance. This item is an argument for the CI ratchet landing immediately
  after it.
- **Both conditions widen, so the whole-shape scoping hazard widens with
  them.** Condition 1 gains a second predicate, and condition 2 gains a
  normalization step and so becomes strictly more permissive. The 2a spec §8
  records that condition 1 is scoped to the whole shape rather than to the
  mismatching position; a shape could therefore satisfy condition 1 on one
  position and condition 2 on another. It stays unreachable at this alphabet for
  the same single-child reason that spec gives — no container holds both text
  and a nested container — but the per-position conversion deferred to the
  alphabet-widening item now has two predicates and one extra normalization step
  to convert, not one predicate. **Measured: the permanent count came back at
  1,984 entries, below the ~2,292 §5.1 predicted, so the hazard stayed
  unreached at this alphabet.** A separate check turned up a pre-existing
  blind spot in the plan rather than a new hazard: Task 2's Step 9 laundering
  grep — which tests only whether a permanent-file shape contains
  `Strong([Emph(` without also containing `Emph([Strong(` — returns 5 at
  `8b9d05e`, the commit before this branch existed, so it was already a
  false-positive source before this item started. It grew to 37 once Task 1's
  §3 fix landed (`0ac2c48`) and stayed at 37 through Task 2's bless, so the
  growth traces to the fix changing what round-trips, not to a laundered
  classification during this item's blesses: those 37 shapes reach the
  permanent file legitimately through `nests_same_class_directly` (condition
  1's other disjunct) rather than through a laundered regression (see §4's
  corrected laundering argument). The check that asks the intended question
  — an `Emph`-outer shape in the permanent file with no justifying mechanism
  at all, neither disjunct of condition 1 — returns 0.

## 8. What this fixes that was not the stated goal

`inlines_to_html` (`markdown.rs:164`), the merged-table renderer, emits nested
`<em>`/`<strong>` directly and never had this defect. The Markdown path did.
Narrowing the edge rule moves the two renderers *toward* each other rather than
apart — a divergence closed as a side effect, and worth stating because item 2b
will inherit a smaller gap than it would have.
