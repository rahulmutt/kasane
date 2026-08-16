# kasane — Structural Census Design Spec

**Date:** 2026-08-16
**Status:** Designed, not implemented.
**Parent spec:** `2026-08-15-emphasis-seam-design.md` (§8, first residual
bullet, whose "a later item works it down" names the queue this instrument
measures; and the follow-up its post-mortem recorded as having no item — "a
structural check in the test tier, since the census measures text only, and two
structure losses in this item were invisible to every automated check").
**Repo:** kasane

## 1. Purpose & scope

`crates/kasane-writer/tests/census.rs` compares the text a parser recovers
against `kasane_gfm::rendered_text`. It is the instrument that found the
emphasis-seam defects six whole-pipeline properties and three review rounds had
missed. It cannot see structure at all.

That blind spot is not hypothetical. The emphasis-seam item shipped two known
structural losses, both of which leave the recovered text byte-identical and so
pass the census silently:

| IR | printed | structure lost |
|---|---|---|
| `Emph[Text("a "), Emph[Text("b")], Text(" c")]` | `*a b c*` | inner `<em>` |
| `[Emph[Text("a")], Strong[Text("b")]]` | `*ab*` | `<strong>` became `<em>` |

This item adds a second assertion, on the same alphabet and the same shapes,
that compares **structure**. It defines the relation, records what the relation
finds, and lands the ratchet. It does not fix what it finds; §6 records the
defect it surfaces and hands it to its own item.

Scope is the markdown inline path — `inlines_to_md_at`, reached through
`Block::Para`. Two of the writer's three inline renderers (`Ctx::Cell` for pipe
tables, `inlines_to_html` for the `has_merged` fallback) have no exhaustive
sweep of any kind, for text or structure; widening to them is a separate item
(§7).

## 2. The relation

A shape is **structurally corrupt** when, for some character of the recovered
text, the stack of enclosing emphasis containers differs between the IR and the
parsed output.

**IR side.** Walk the inlines exactly as `kasane_gfm::rendered_text`'s `walk`
does, carrying a stack:

- `Emph` pushes `Em`, `Strong` pushes `St`, both recursing at `depth + 1`.
- `Link` pushes **nothing** and recurses. This mirrors `flatten_into`
  (`markdown.rs:237-238`, stated at `markdown.rs:393-397`), which splices every
  non-`External` target away before the emit loop ever sees it — a transparent
  link is not a structural level in the output and must not be one here.
- `Text`, `Code`, `Math` contribute their characters at the current stack.
- `FootnoteRef(n)` contributes `[^n]`, matching `rendered_text`'s `notes: true`
  projection.
- The `MAX_INLINE_DEPTH` cutoff is inherited unchanged: a subtree past the bound
  contributes nothing, on both sides.

**Parsed side.** Walk pulldown events under the census's existing `Options`,
pushing on `Start(Tag::Emphasis)` / `Start(Tag::Strong)`, popping on the
matching `End`, and contributing the characters of `Text`, `Code`, `InlineMath`
and `DisplayMath` at the current stack.

**Gate.** The comparison runs only on shapes the *text* census already passes.
Per-character alignment presupposes the two strings are equal, and structure is
meaningless where the text is scrambled. This composes with the emphasis-seam
item's remaining work: as its 32-shape family drains, those shapes graduate into
structural checking without anyone editing this test.

### 2.1 Why this relation and not tree equality

The writer is *supposed* to be lossy. Four transformations erase structure by
design, and a naive tree comparison would flag nearly every shape for reasons
that are all correct:

| Transformation | Effect |
|---|---|
| Adjacent run fusion | `Emph[a], Emph[b]` → one `<em>ab</em>` |
| Same-`Delim` splice (`splice_children`) | `Emph[a, Emph[b], c]` → `<em>a b c</em>` |
| Flanking decline | delimiters not spelled; content emitted bare |
| Transparent link splice (`flatten_into`) | non-`External` targets vanish |

A per-character context vector prices these correctly without modelling any of
them. Fusion is silent, because every character keeps the same enclosing class;
the link splice is silent, because links are not pushed on either side; and the
splice and the decline show up as genuine context differences, which is what
they are.

### 2.2 Approaches rejected

**A predicted-structure oracle** — model the four transformations, assert exact
equality — reimplements the writer inside its own test. Any defect present in
both writer and model is invisible, which is precisely the failure mode that
carried two structure losses past three review rounds. Rejected on that ground
alone.

**A structural golden snapshot** — record every shape's parsed structure, review
the diff — needs no theory of correctness and matches the "read the diff, that
is the evidence" culture. Rejected because it cannot distinguish corrupt from
merely different, and so cannot ratchet; a ~7,000-entry golden that churns on
every writer change trains a reviewer to bless diffs, and the CI guard that
follows this item has nothing to lock.

**Collapsing adjacent identical classes** to quiet the same-class family was
measured and is wrong: it would mask `Emph[a, Emph[b], c]`, which round-trips
correctly today (`2026-08-15-emphasis-seam-design.md` §8) and is therefore a
fixable loss if it ever breaks. Only *direct* same-class nesting is
inexpressible; see §4.

## 3. Guards

Three conditions are **hard failures, not skips**. The probe (§5) had zero of
each, so any occurrence means the instrument is broken rather than the writer:

| Guard | Meaning if it fires |
|---|---|
| Concatenated IR characters ≠ `rendered_text(&seq)` | the walk has drifted from the real projection |
| Character sequences differ after trimming | text matched but alignment failed — impossible by construction |
| Event stack non-empty at end of parse | unbalanced parse; the comparison is meaningless |

The first is load-bearing and is why this relation needs no `kasane-gfm` API
change. The test re-derives `rendered_text` from its own walk every run and
asserts equality, so it cannot silently check the wrong projection. Drift is
impossible rather than merely unlikely.

## 4. Two files, and why the split is computed

`tests/census-known-structure-corrupt.txt` is a **queue**, target zero.
`tests/census-inexpressible.txt` is **permanent**, carrying a header that states
the reason.

The permanent file exists because one family is not a defect at any level:
`<em><em>x</em></em>` has no CommonMark spelling. `**x**` is strong, not nested
emphasis. No writer change can close these, and leaving 1,236 unclosable
entries in a queue would make "shrink to zero" meaningless — the same risk
`2026-08-15-emphasis-seam-design.md` §8 names when it says the danger is a list
read as an acceptance rather than a queue.

A second file is a carve-out, and carve-outs hide judgments. This one is
**computed on every run, never hand-curated.** A shape is filed as inexpressible
only if both hold:

1. it contains a container whose **sole** child is a same-class container
   (`Emph` whose only child is `Emph`, or `Strong` whose only child is
   `Strong`); and
2. every mismatching position is explained purely by collapsing adjacent
   identical classes.

Condition 1 is the one that does the work. Without it, condition 2 alone would
file `Emph[a, Emph[b], c]` as permanent — a shape that round-trips correctly
today and whose breakage would be a real regression. With it, that shape stays
in the queue, because its inner `Emph` has siblings and is therefore
expressible. A shape that stops satisfying either condition moves back to the
queue on the next bless, without anyone editing a file.

`KASANE_CENSUS_BLESS=1` rewrites all three files, so the reviewer-facing
workflow is unchanged: change the writer, bless, read the diff.

Each shape therefore lands in exactly one state, and the states are what a
reviewer reads:

| Text | Structure | State |
|---|---|---|
| matches | matches | clean |
| matches | differs, inexpressible | permanent, second file |
| matches | differs, otherwise | **queue** — the new signal |
| differs | not evaluated | already named in the text allowlist |

## 5. What the probe measured

Measured 2026-08-16 with a throwaway probe implementing §2 against the committed
alphabet at `7587dcc`, then deleted. Numbers are measurements, not estimates.

```
total shapes                            7,239
  self-check failures                       0
  alignment failures                        0
  text corrupt (gated out)                 32   unchanged from the committed allowlist
structurally corrupt                    4,048   (56%)
  ├─ direct same-class nesting          1,236   permanent, §4
  └─ queue                              2,812
       ├─ mixed-class nesting           2,002   §6
       ├─ emph(code), emphasis dropped    240   flanking decline
       ├─ emph(code), other               330
       └─ other                           240
```

Zero self-check and zero alignment failures are the evidence that the relation
is well-defined over this alphabet, not merely plausible. The 32 text-corrupt
shapes match the committed `census-known-corrupt.txt` exactly.

The last two rows, 570 shapes, are **not characterized beyond the predicate that
grouped them**. They are known to be genuine context differences that are
neither the mixed-class family nor an outright drop. Naming their mechanisms is
work this item does not do; the queue makes them visible, which is the point.

## 6. What this item finds and does not fix

The probe surfaced a defect that no existing check can see and that no spec
records. **Every `Emph[Strong[x]]` and `Strong[Emph[x]]` loses a level** — 2,002
shapes, 49% of the structurally corrupt set:

```
IR       Emph[Strong[Text("a")]]
printed  *a*
parsed   <em>a</em>          the <strong> is gone
correct  ***a***             parses as <em><strong>a</strong></em>
```

The mechanism is `splice_children`'s edge rule. `edge_to_splice(&children, ch)`
keys on the delimiter **character**, and `Delim::ch()` (`escape.rs:540`) maps
both `Delim::Emph` and `Delim::Strong` to `'*'`. A `Strong` at the edge of an
`Emph` run therefore matches the edge rule and is spliced away.

The character-keying is deliberate and well-argued — `escape.rs:530-536` and
`2026-08-15-emphasis-seam-design.md` §2.1 explain that two runs collide when
they share a character, since `*` and `**` abut into a `***` run a parser splits
where the writer did not intend. What is unrecorded is the *cost*: §8 prices the
same-`Delim` splice's unconditional flattening and never mentions that the edge
rule destroys mixed-class nesting outright. This spec records it.

It is fixable, not a representational limit: `***a***` expresses the shape
correctly. The symptom — bold inside italic silently ceasing to be bold in a
document converter — is arguably worse than the 32-shape text family that
remains open, and the family is ~60× larger. It gets its own item and its own
design; this one only makes it visible and bounded.

## 7. Non-goals

- **Fixing anything.** The queue starts at 2,812 and this item closes none of
  it. Every entry is a question left open on purpose.
- **The other two inline renderers.** `Ctx::Cell` and `inlines_to_html` have no
  exhaustive sweep for text *or* structure; the merged-table path reimplements
  every inline from scratch and the emphasis-seam work never touched it. Its own
  item.
- **Block structure.** The census renders exactly one `Block::Para`. A block
  alphabet (`List`/`Footnote` nesting, `MAX_BLOCK_DEPTH` truncation, table
  shape) needs its own relation, since the deliberate erasure there is depth
  truncation rather than emphasis flattening. Its own item.
- **The CI ratchet.** That the allowlists may only shrink against `main` is the
  next item, and it depends on this one only for the file format.

## 8. Verification and risk

`mise run lint && mise run test` green, with `lint` covering `--all-targets`
plus `fmt --check`.

The proof specific to this item is that the committed instrument reproduces §5:
4,048 structurally corrupt, split 2,812 queue / 1,236 permanent, the text
allowlist unchanged at 32, and all three §3 guards silent. Two hand-written unit
tests pin the relation's edges independently of the bless output, so a bug in
the bless path cannot make the instrument vacuous:

- `[Emph[Text("a")], Strong[Text("b")]]` **is** reported corrupt — the known
  loss the alphabet reaches.
- `[Emph[Text("a")], Emph[Text("b")]]` **is not** — adjacent-run fusion is
  intentional, and a check that flags it would be unusable.

Four residual risks, recorded rather than closed:

- **The known loss `*a *b* c*` is unreachable by this alphabet.** The census
  nests only through single-child alphabet elements, so no shape produces a
  container holding both text siblings and a nested container. Of the two
  structure losses that motivated this item, the instrument catches one. Closing
  the gap means extending the alphabet, which changes the shape count for every
  assertion including the text one, and belongs to its own item rather than
  being smuggled in here.
- **A 2,812-line queue can be read as an acceptance.** The same risk §8 of the
  parent spec names for a 32-line one, at 88× the size. The mitigations are that
  a stale entry fails the build, that §6 names the mechanism behind 2,002 of
  them, and that the CI ratchet lands next.
- **570 queue entries have no named mechanism.** They are grouped by a
  predicate, not diagnosed. A reader should not infer from this spec that the
  queue is four families; it is at least four, and the tail is unexamined.
- **Condition 1 is scoped to the whole shape, not to the mismatching
  position.** `nests_same_class_directly` asks whether `seq` contains direct
  same-class nesting *anywhere*, and `differs_only_by_collapse` is evaluated
  over all positions, so a shape carrying an unrelated direct same-class
  container can satisfy condition 1 on the strength of a mismatch that
  belongs to a different position entirely — filing that position permanent
  on a loss that is actually fixable, which is exactly what condition 1
  exists to prevent. This is unreachable at the committed alphabet: every
  alphabet container is single-child, so no shape has a container holding
  both text and a nested container, which is the same fact the first
  residual risk above rests on. It therefore goes live in the same item that
  widens the alphabet — which is also when the transparent-`Link` gap below
  goes live, so that item must make condition 1 per-position *before* it
  widens anything. A sibling gap sits next to it: `nests_same_class_directly`
  does not treat "a container whose sole child is a transparent `Link` that
  itself wraps a same-class container" as satisfying condition 1, even though
  §2 makes a `Link` structurally invisible. Its failure direction is safe
  today — such a shape lands in the queue rather than the permanent file,
  i.e. it over-queues rather than over-excuses.
