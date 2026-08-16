# kasane — Cross-Class Edge Splice Design Spec

**Date:** 2026-08-16
**Status:** Designed, not implemented.
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
forms a run of one member whose entire content is the inner container — whole-run
coverage. Adjacent-run fusion reaches a few of the single-edge configurations
(`[Emph[Strong[a]], Emph[Text("b")]]` fuses to one run with a head-edge
collision), but the mass of the 2,002 sits in the two whole-run rows, which is
why a rule covering only those rows wins nearly the whole expressible half.

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

One normalization rather than two arms, and iterated rather than applied once,
because a shape can be erased both ways at once. `Strong[Emph[Emph[a]]]` prints
`**a**`, giving an IR stack of `[Strong, Emph, Emph]` against a recovered
`[Strong]`: collapse yields `[Strong, Emph]`, the drop then yields `[Strong]`,
and the walks agree. Applying the two steps once in the other order leaves
`[Strong, Emph]` and files a genuinely unspellable shape corrupt, which is why
the fixpoint is specified rather than an order.

```rust
if (nests_same_class_directly(seq) || nests_strong_over_emph_directly(seq))
    && differs_only_by_erasure(&ir, &got)
{
    return Structure::Inexpressible;
}
```

**The laundering hazard is closed by condition 1's direction and by the drop's
direction together.** If §3's fix regresses and `Emph[Strong[x]]` loses its
`<strong>`, the IR stack is `[Emph, Strong]` against a recovered `[Emph]`. The
drop removes an `Emph` preceded by a `Strong`, and this stack has a `Strong`
preceded by an `Emph` — the normalization does not touch it, the walks stay
unequal, and the shape satisfies neither condition-1 predicate. It lands in the
queue, where the ratchet fails the build. The same holds for the 110 shapes
carrying both orders: they qualify as inexpressible on their `Strong`-outer
half, but a regression in their `Emph`-outer half survives normalization and
breaks condition 2.

`INEXPRESSIBLE_HEADER` needs rewriting. It currently explains only
`<em><em>x</em></em>` and says a shape lands in the file by containing a
same-class container, which stops being the whole truth.

## 5. Tests

### 5.1 Census

The instrument is the existing one; this item re-blesses it and reads the diff.
Predicted movement, **confirmed at bless and not asserted in advance**:

| file | before | after |
|---|---|---|
| `census-known-corrupt.txt` (text) | 32 | 32 |
| `census-known-structure-corrupt.txt` (queue) | 2,812 | ~810 |
| `census-inexpressible.txt` (permanent) | 1,236 | ~2,292 |

with ~946 shapes going clean.

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
- **The ~810 remaining queue entries.** The tail
  `2026-08-16-structural-census-design.md` §8 already flags as having no named
  mechanism. This item does not diagnose it.
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
- **Inline depth.** Declining a splice keeps a container the old rule flattened,
  so these shapes render one level deeper and shapes near `MAX_INLINE_DEPTH`
  could truncate where they previously did not. `inline_depth.rs` is the check,
  and it needs an explicit look rather than an assumption.
- **The permanent file nearly doubles** and becomes the majority of the corrupt
  set — ~2,292 permanent against ~810 queue. A reader can take that as the
  project giving up. The mitigations are that the file is computed on every
  bless, that §5.2's two edges assert the direction condition 1 rests on, and
  that the header is rewritten to explain both mechanisms.
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
  to convert, not one predicate. If §5.1's permanent count comes back above
  ~2,292, this is the first place to look.

## 8. What this fixes that was not the stated goal

`inlines_to_html` (`markdown.rs:164`), the merged-table renderer, emits nested
`<em>`/`<strong>` directly and never had this defect. The Markdown path did.
Narrowing the edge rule moves the two renderers *toward* each other rather than
apart — a divergence closed as a side effect, and worth stating because item 2b
will inherit a smaller gap than it would have.
