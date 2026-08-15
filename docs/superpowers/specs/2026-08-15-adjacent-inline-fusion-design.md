# kasane — Adjacent Inline Fusion Design Spec

**Date:** 2026-08-15
**Status:** Designed 2026-08-15. Not yet implemented.
**Parent spec:** `2026-08-09-markdown-escaping-design.md` (§ "Recorded as open",
the adjacent-code-spans bullet), which that bullet says "needs its own design".
The bullet was opened by `2026-08-14-empty-code-span-anchor-design.md`, whose
Status block and § Non-goals correction both point here.
**Repo:** kasane

## 1. Purpose & scope

The writer emits each inline independently. Where two neighbours print with the
same delimiter, the delimiters meet with nothing between them and a real parser
reads one span where the IR held two. The text inside comes back wrong, and the
delimiters that should have separated the two spans come back as visible
characters in the middle of it.

Three shapes are affected. Each was measured, not reasoned about (§1,
"Confirmed"):

| IR | printed | what a parser recovers |
|---|---|---|
| `[Code("x"), Code("y")]` | ``` `x``y` ``` | one span reading ``` x``y ``` |
| `[Emph(a), Emph(b)]` | `*a**b*` | one `<em>` reading `a**b` |
| `[Strong(a), Strong(b)]` | `**a****b**` | one `<strong>` reading `a****b` |

Only the first was recorded before this item. The other two are found here, and
they are worse: the code case leaks backticks, which `anchor_slug` discards, but
the emphasis cases leak asterisks into the visible text of an ordinary
paragraph. This spec closes all three, because they are one defect — a
collision between writer-chosen delimiters across an inline boundary — and
because the repo's standing rule is that a defect found beside the one being
fixed is closed in the same branch.

The anchor divergence `kasane_gfm::slug`'s module doc records is *downstream* of
the code case, and closes with it (§4). It is the reason this item exists but
not the shape of it: whoever reads only the anchor symptom will fix the wrong
crate.

### Scope

1. A maximal run of adjacent inlines that print with the same delimiter renders
   as one span over the concatenation of their contents (§2.1).
2. The run is grouped at emission, inside `inlines_to_md_at`, and no `Inline` is
   cloned or rewritten to do it (§2.2).
3. An inline that prints nothing does not break a run (§2.3).
4. `escape::code_span`, `escape::math_span` and `emphasize` keep their rules
   unchanged; the only extraction is `math_span`'s degradation guard, which
   gains a name so the class function can call it too (§2.1).
5. `kasane-gfm` does not change at all (§4).
6. The divergence count in `slug.rs`, `README.md`, `AGENTS.md` and two design
   specs drops from two to one (§6).

### Confirmed, not assumed

Every row below was measured at this item's base by rendering the IR through
`kasane_writer::blocks_to_markdown` and parsing the result with
`pulldown-cmark` 0.13 — the same parser `tests/properties.rs` uses as its
oracle. The probe was a throwaway test, deleted; §5 is where these become
committed tests.

- **The three fusing shapes are exactly the three in the table above.** Their
  parsed forms are a `Code` event over ``` x``y ```, one `Emphasis` over
  `a**b`, and one `Strong` over `a****b`.
- **Fusion is not limited to two members.** `[Code("x"), Code("y"), Code("z")]`
  prints ``` `x``y``z` ``` and comes back as one span reading
  ``` x``y``z ```.
- **An inline that prints nothing is transparent at the boundary.**
  `[Code("x"), Text(""), Code("y")]` prints ``` `x``y` ``` and fuses, as does
  `[Emph(a), Text(""), Emph(b)]`. Grouping on IR adjacency alone would leave
  both broken.
- **Whitespace at the boundary already prevents an emphasis collision.**
  `emphasize` (`markdown.rs:286-294`) hoists a leading or trailing whitespace
  run *outside* the delimiters, so `[Emph("a "), Emph("b")]` prints `*a* *b*`
  and parses as two `<em>`s. This is why the emphasis case has gone unnoticed:
  it needs two runs that meet flush.
- **Document text at the boundary is already safe.** `escape::text` escapes a
  trailing backtick or `*`, so `[Text("a`"), Code("y")]` prints
  ``` a\``y` ``` and `[Text("a*"), Emph(b)]` prints `a\**b*`; both parse as
  intended. The collision is markup-meets-markup only.
- **A `Math` inline that degrades collides as a code span.** `math_span`
  (`escape.rs:531-539`) falls back to `code_span` when its content holds `$`,
  `\n` or `\r`. So `[Math("$"), Math("$")]` prints ``` `$``$` ``` and fuses;
  so do `[Code("x"), Math("a$b")]` and `[Math("a$b"), Code("y")]`, in both
  orders. **This is why the rule cannot be keyed on the `Inline` variant.**
- **Non-degrading `Math` does not collide.** `[Math("x"), Math("y")]` prints
  `$x$$y$` and comes back as two `InlineMath` events; `[Code("x"), Math("y")]`
  and `[Math("x"), Text("y")]` are likewise clean. See § Non-goals for the
  caveat about GitHub's own math extension.
- **Mixed emphasis nesting keeps its text.** `[Strong([Emph(a)]), Emph(b)]`
  prints `***a****b*` and recovers `<em><strong>a</strong></em><em>b</em>` —
  the nesting reassociates, the text survives. `[Emph(a), Strong([Emph(b)])]`
  likewise. Not a text-fidelity defect, so not in scope.
- **`Emph` with no printing content prints nothing.** `emphasize` returns its
  inner string unchanged when the trimmed inner is empty, so `Emph([])` and
  `Emph([Text("")])` print the empty string: `[Emph([]), Emph(b)]` prints
  `*b*`. A *whitespace-only* `Emph` is a different case — `Emph([Text(" ")])`
  prints a bare space, which genuinely separates its neighbours.
- **`escape::text` never deletes.** It normalizes newlines and adds escapes and
  character references; nothing in it removes a character. So `Inline::Text(t)`
  prints the empty string if and only if `t` is empty — the vacuity predicate
  in §2.3 can test the string directly.
- **The shape this item produces for the recorded divergence agrees.** A
  heading of `[Text("a"), Code("  "), Text("b")]` — one span over the
  concatenation of the two canonicalized empty spans — prints ``## a`  `b``,
  parses to the heading text `a  b`, and ids `a--b`, which is exactly what
  `anchor_slug_of` returns for the same inlines. Measured, not derived from
  CommonMark's all-spaces carve-out, though that carve-out is why it holds.
- **The property tier cannot draw any of this.** `generator::build`
  (`generator/mod.rs:268-272`) composes every block's inlines as
  `[Text(payload)] ++ deco`, and `deco` is a single inline drawn from
  `inlines(3)` (`generator/mod.rs:377`), which returns a one-element `Vec`. No
  generated block has ever held two inlines side by side. Six whole-pipeline
  properties missed this defect for that reason and no other.

### Non-goals

- **Adjacent `Inline::Math`.** `$x$$y$` parses as two inline maths under
  `pulldown-cmark`; GitHub's math extension is a separate implementation and
  cannot be checked from this environment. It is excluded because the fix that
  applies to the other classes is *unavailable* here rather than merely
  unwanted: fusing `$x$` and `$y$` into `$xy$` states a different equation,
  which is content corruption rather than a repair. Added to the external
  probe's case list (§8) instead.
- **`Strong([Emph])` beside `Emph`.** Text survives, nesting reassociates
  (§1, "Confirmed"). The fix itself trades structure for text everywhere else;
  it would be incoherent to spend a rule reclaiming structure here.
- **Rule 1's hand-built residue.** A caller who renders `[Code(""), Code("")]`
  through `blocks_to_markdown` without going through `structure` still gets one
  span, now printing a single space rather than a fused pair, against an anchor
  computed from un-canonicalized inlines. Divergent before this item and
  divergent after, in a different spelling. §4 says why that is unchanged in
  kind and not a regression.
- **The write/anchor mismatch *class*.** Computing an anchor from the rendered
  line would close it permanently. Rejected as this item's shape by
  `2026-08-14-empty-code-span-anchor-design.md` §7 approach D, and nothing here
  forecloses it — this item closes one member of the class by moving the
  printed line onto the anchor, which is the same direction that approach goes.

## 2. The change

### 2.1 The delimiter class

A new `escape.rs` function answers, for one inline, which delimiter it will
print with:

```rust
enum Delim { Backtick, Emph, Strong }

fn delim(i: &Inline) -> Option<Delim> {
    match i {
        Inline::Code(_) => Some(Delim::Backtick),
        Inline::Math(t) if math_degrades(t) => Some(Delim::Backtick),
        Inline::Emph(_) => Some(Delim::Emph),
        Inline::Strong(_) => Some(Delim::Strong),
        _ => None,
    }
}
```

`math_degrades` is `math_span`'s existing guard —
`s.contains('$') || s.contains('\n') || s.contains('\r')` — extracted and
named. Both `math_span` and `delim` call it. The extraction is the whole cost of
keying on the printed delimiter rather than the IR variant, and that keying is
load-bearing: a rule that matched `Inline::Code` alone would leave the three
degrading-math shapes in §1 broken while looking complete.

Naming the guard also removes the "two places to forget" hazard the repo names
elsewhere. If a future edit widens what `math_span` degrades — a `|` in a cell,
say — the class widens with it in the same edit, and cannot silently fail to.

The rule the class enables:

> A maximal run of adjacent inlines with the same `Some(Delim)` renders as one
> span over the concatenation of their contents.

A run of one is what happens today, so a document containing no adjacent pair
renders byte-identically.

### 2.2 Where the run is grouped

In `inlines_to_md_at`'s loop (`markdown.rs:211-250`), which becomes an
index-based scan over runs rather than a `for` over items. It is **not** an
IR-rewriting pre-pass beside `escape::fold_inline_newlines`, which is the
tempting shape and is wrong here.

A pre-pass has to merge `Emph(x)` with `Emph(y)` into `Emph(x ++ y)`, which
means cloning `x` and `y` wholesale. `Inline`'s derived `Clone` recurses once
per nesting level — `fold_seq`'s own comment (`escape.rs:344-351`) says so, and
is why every recursive walk in this repo rebuilds level by level under a depth
guard instead of cloning a subtree. `blocks_to_markdown` is public API over a
public IR, so a hand-built inline nested past the bound would overflow the stack
inside that clone, *before* `inlines_to_md_at`'s existing `MAX_INLINE_DEPTH`
guard had the chance to discard it. The guard would still be there and would
still be right; the pre-pass would simply run first.

Emission-time grouping never clones an inline:

- **A backtick run** collects each member's content as `&str` — `Code(t)`'s
  `t`, and a degrading `Math(t)`'s raw `t`, which is what `math_span` would
  have passed to `code_span` anyway — concatenates them, and calls
  `escape::code_span` once. `Ctx` threading is unchanged, so a run inside a
  table cell escapes `|` across the concatenation exactly as a single span
  does.
- **An emphasis or strong run** renders each member's children through the
  existing `inlines_to_md_at` recursion into one buffer, then calls
  `emphasize` once with the run's delimiter.

`escape::code_span`, `escape::math_span` and `emphasize` are untouched. The
depth guard, the `Ctx` argument and the four `Pos` rules stay where they are;
`pos` is recomputed per member instead of per inline, by the same four rules,
so a run of one behaves exactly as today. Members after the first in an
emphasis run see the `pos` their predecessor's output produced rather than the
run's opening `pos` — which matters only for a member whose predecessor printed
nothing, and is what keeps the single-member case unchanged.

### 2.3 What breaks a run

An inline that prints nothing must not break a run, or
`[Code("x"), Text(""), Code("y")]` stays fused (§1, "Confirmed"). The scan
therefore skips vacuous inlines, decided by a predicate that mirrors the
renderer exactly:

```rust
fn renders_empty(i: &Inline, depth: usize) -> bool {
    match i {
        Inline::Text(t) => t.is_empty(),
        Inline::Emph(x) | Inline::Strong(x) => {
            depth + 1 >= kasane_ir::MAX_INLINE_DEPTH
                || x.iter().all(|c| renders_empty(c, depth + 1))
        }
        Inline::Link { target, inlines } if !matches!(target, RefTarget::External(_)) => {
            depth + 1 >= kasane_ir::MAX_INLINE_DEPTH
                || inlines.iter().all(|c| renders_empty(c, depth + 1))
        }
        _ => false,
    }
}
```

Four details, each of them a mirror of something already established:

- `Text(t)` is vacuous exactly when `t` is empty, because `escape::text` never
  deletes (§1, "Confirmed").
- `Emph`/`Strong` are vacuous exactly when every child is, because `emphasize`
  returns its inner string unchanged when that string is blank, and the inner
  string is the concatenation of the children's output.
- `Link` splits by target the same way the renderer does. An `External` link
  always emits its `[...](...)` brackets, so it is never vacuous even with
  empty `inlines`. Any other target renders through the "unresolved -> text"
  arm instead, which prints only `inlines_to_md_at(inlines, ...)` with no
  brackets at all — a container exactly like `Emph`/`Strong`, and it follows
  the same rule. This is reachable beyond hand-built IR: the shared EPUB
  XHTML parser emits a bare empty anchor as `Inline::Link` with empty
  `inlines`, stripped only on the mobi path — the epub adapter builds
  `Internal`-target links from the same parser. Missing this arm was a false
  negative caught in review: `renders_empty` reported `false` for every
  `Link`, so `[Code("x"), Link{Internal(_), []}, Code("y")]` still broke the
  run at an inline emitting zero characters and printed the exact fused
  collision this item exists to close.
- The depth term is not a safety bound bolted on; it is the mirror.
  `inlines_to_md_at` returns `String::new()` at `depth >= MAX_INLINE_DEPTH`, so
  a container whose children sit at or past the bound really does print
  nothing, and the predicate must say so to stay exact. It takes the caller's
  absolute `depth` for that reason, not a fresh counter.

Everything else is non-vacuous by construction: `Code("")` prints `` ` ` ``,
`Math("")` prints `$$`, a `FootnoteRef` prints `[^n]`. A whitespace-only
inline is deliberately non-vacuous — it prints a space, which separates the
neighbours it sits between, and treating it as transparent would fuse two
spans a reader can see are apart.

### 2.4 What moves that was not broken

The rule is uniform, so it also fuses a run that was already rendering
correctly. `[Emph("a "), Emph("b")]` prints `*a* *b*` today, because
`emphasize` hoists the trailing space outside the delimiters, and will print
`*a b*` after this item. Same text, one `<em>` where there were two.

The alternative — fusing only where a collision is actually predicted — would
mean deciding "does this member's output end in whitespace" in a second place
that has to stay in step with `emphasize`'s hoisting forever, for output bytes
whose rendered difference is one HTML element boundary. The uniform rule is
chosen because it is statable in one sentence and testable as an equality, and
because the mirror it avoids is exactly the kind this repo has spent its last
three items retiring.

What the fix costs everywhere, stated once: **the span boundary**. Two `<code>`
chips become one, two `<em>`s become one. That loss is invisible in the rendered
text, and within plain CommonMark it is forced: two code spans cannot be written
in a row at all, so no spelling keeps both the boundary and the text. It is not
forced absolutely — §7's B and C each keep the boundary, one by alternating
delimiters and one by inserting inline HTML, and each is rejected on its own
grounds rather than on impossibility. The text is what is corrupt today, and the
text is what every option here preserves.

## 3. Blast radius

- **`markdown.rs`'s `inlines_to_md_at`** — the loop becomes a run scan. Every
  Markdown inline sequence in the crate passes through it: paragraph and
  heading bodies, table cells, list-item content, `Emph`/`Strong` children,
  link labels, figure captions. One seam, all of them.
- **`markdown.rs`'s `inlines_to_html`** — untouched, and must be. The
  merged-table fallback prints `<code>x</code><code>y</code>`, which is already
  correct: HTML has no delimiter to collide. Fusing there would be a change
  with no defect behind it.
- **`escape::math_span`** — one guard extracted to `math_degrades`, same
  expression, same call site behaviour.
- **`escape::code_span` and `emphasize`** — no change. They see a longer
  content string or a longer inner buffer, which they already handle: a
  backtick run's fence length is computed from the concatenation, so a run
  whose members each carry backticks gets one fence long enough for all of
  them.
- **`kasane-core`** — no change. The empty-span canonicalization in
  `clone_inlines_at` stays exactly as it is, and this item depends on it
  running first (§4).
- **`kasane-gfm`** — no change at all (§4).
- **Rendered output for existing fixtures** — unchanged. Converting
  `tests/fixtures/epub/rich.epub` before and after must produce an identical
  tree; a diff means a run was grouped where no run exists.

## 4. Why the anchor is then correct

`rendered_text` already concatenates span contents across inlines. Fusing at
emission moves the *printed line* onto the anchor rather than teaching either
side about the other, so `kasane-gfm` needs no change and gains no knowledge of
the writer's rules.

The recorded divergence closes as a consequence. `kasane-core` canonicalizes
`Inline::Code("")` to `Inline::Code(" ")` on the way in, so the writer receives
`[Text("a"), Code(" "), Code(" "), Text("b")]`. One span over `"  "` takes
`code_span`'s Rule 2 — all-spaces content, which CommonMark's carve-out does
not strip — and prints `` `  ` ``. The heading line reads `a  b` and ids
`a--b`; `rendered_text` of the same inlines is `a  b`, which `anchor_slug` maps
to `a--b`. Measured, both halves (§1, "Confirmed"). The trade
`2026-08-14-empty-code-span-anchor-design.md` recorded is paid back: the mixed
shapes it closed stay closed, and the adjacent shape it opened closes here.

The ordering — canonicalize in core, fuse in the writer — holds by
construction, since `structure` runs before any writer walk. It is worth naming
because the answer depends on it rather than because the reverse would break:
fusing first and canonicalizing after would give one empty span, one padding
space, a line reading `a b` and an anchor of `a-b`. Self-consistent too, and
wrong for a different reason — it would say two empty spans contribute one
space between them. Under this ordering each empty span contributes the space
it prints, which is what the canonicalization was written to mean.

The un-canonicalized caller is the case where that ordering never happens, and
they stay divergent: the writer fuses `[Code(""), Code("")]` to one `` ` ` ``,
the line reads `a b` and ids `a-b`, while an anchor taken from their raw
inlines still says `ab`. Divergent before this item in one spelling and after
it in another — see § Non-goals, and `escape::code_span`'s Rule 1 comment,
which already carries the reachability argument for that caller.

For the two emphasis classes there is nothing to close on the anchor side:
`*` is outside `is_word`, so `anchor_slug` drops the leaked asterisks and both
sides said the same thing before this item. That is precisely why the anchor
properties cannot catch the emphasis defect, and why §5.2 adds a text property
rather than another anchor one.

## 5. Testing

### 5.1 Unit

In `markdown.rs`'s test module, each asserting the printed bytes and, where the
point is what a reader sees, the parsed result:

- the three fusing classes, each as a two-member and a three-member run;
- `[Code("x"), Text(""), Code("y")]` and `[Emph(a), Text(""), Emph(b)]` — the
  vacuous-inline break, which is the case a naive implementation gets wrong;
- `[Emph([Text(" ")]), Emph(b)]` — the whitespace-only inline, which must
  *not* be treated as vacuous;
- both mixed orders of a code span and a degrading `Math`, plus
  `[Math("$"), Math("$")]` — the shapes that fail if the class is keyed on the
  IR variant;
- a run inside `Ctx::Cell`, asserting the `|` escaping applies across the
  concatenation;
- a run nested inside `Emph`, so the recursion is covered rather than only the
  top level;
- a backtick run whose members each contain backticks, asserting one fence long
  enough for the concatenation;
- the non-fusing controls — `[Emph(a), Strong(b)]`, `[Code(x), Math(y)]`,
  `[Math(x), Math(y)]`, `[Emph("a "), Emph("b")]` — so a later change that
  over-fuses fails something. The last of these pins §2.4's deliberate
  byte change.

In `escape.rs`'s test module: `math_degrades` agrees with `math_span`'s
observable behaviour on the boundary inputs (`$`, `\n`, `\r`, and content with
none of them), so the extraction cannot drift from the branch it was taken
from.

### 5.2 Property

Two changes, and the second is the one that matters.

**The generator gains adjacency.** `case()` draws `deco` from `inlines(3)`,
which yields a one-element `Vec`; the draw becomes
`proptest::collection::vec(inlines(3), 1..=3)` flattened, so blocks hold
neighbouring inlines at all. `deco` is still appended after the payload run,
never wrapped around it, so the conservation invariant's arithmetic is
untouched and `Expect` needs no change.

**Widening alone is not enough, and this is the trap.** Two properties look
like they would catch a fused run and neither does. The anchor properties do
not: a fused `[Code("x"), Code("y")]` heading prints a line reading
``` x``y ```, backticks and
asterisks are both outside `is_word`, so `anchor_slug` discards them and the
parsed line and the IR agree on `xy` — P2, P9 and P12 pass over corrupt text.
P7, the round-trip property, does not either: it checks that each *sentinel
payload* survives into the parsed text, and the payload is always the leading
`Text` run `build` stamps in. The fused text lives in `deco`, which no sentinel
covers and P7 never looks at. Between them that is the second reason this
defect was invisible to the tier, and the reason widening the generator has to
come with a property that reads the deco inlines.

So: one new property, P13, over inline sequences rather than whole documents.
Draw a short sequence of inlines from a restricted alphabet — no hostile
fragments, no newlines, no characters `escape::text` transforms — render a
`Block::Para`, parse it with `parse_events`, and assert the recovered text
equals `kasane_gfm::rendered_text` of the same inlines — already exported
(`lib.rs:20`) and already the projection the anchor rule reads, so the property
needs no new seam. The restriction is what makes this
an equality instead of a fuzzy containment; it is the move P12 made with
`P12_TEXTS`, for the same reason, and it is stated in the property's doc
comment so the next reader does not widen the alphabet and get a mystery
failure from the fold.

### 5.3 The pinned test flips

`adjacent_empty_code_spans_diverge_from_the_line_they_print`
(`properties.rs:827`) asserts the current wrong values deliberately, and its
doc comment nominates itself: "this test is what should fail when that lands."
It becomes an agreement test and is renamed to say so. Its body keeps both
halves — the heading anchor *and* the two-ordinary-code-spans paragraph, which
is where the content half is asserted with no heading and no empty span
involved — with the assertions inverted: a printed line of ``## a`  `b``, a
parsed heading of `a  b`, and one anchor value compared against the other
instead of the two being held apart.

## 6. Documentation

Two divergences are recorded as surviving across five documents, and this item
takes the count to one — `EMPTY_FALLBACK`, the deliberate choice, alone. Two
code comments name the fusion as well and are corrected with them:

- **`kasane_gfm::slug` module doc** (`slug.rs:68-88`) — "Two divergences are
  left, one a choice and one a defect" becomes one, and the adjacent-code-spans
  bullet goes. The prose above it that explains what `rendered_text` closed
  gains the fusion as a fourth mechanism, named as the writer's fix rather than
  this rule's.
- **`AGENTS.md:19-40`** — the `kasane-gfm` entry's "three are now closed …
  Two survive" becomes four and one, and the long adjacent-span paragraph
  collapses to a sentence recording that the writer now fuses runs. The
  `kasane-writer` description gains the rule itself: adjacent same-delimiter
  inlines render as one span, and why.
- **`AGENTS.md:98-104`** — the note that the adjacent case "is deliberately not
  in this table and cannot be" is removed with the case.
- **`README.md:146,164-177`** — "with two exceptions" and "The two exceptions"
  become one, the second bullet is deleted, and the closing paragraph's "Three
  anchors that used to diverge no longer do" becomes four, describing the new
  one in the same reader-facing terms as its neighbours.
- **`2026-08-09-markdown-escaping-design.md:491-517`** — the
  "Adjacent code spans, which the writer fuses" bullet gets a "closed" note in
  the shape the two bullets above it already use: what closed it, by which
  mechanism, and what the bullet predicted instead. It predicted a fix scoped
  to code spans; the fix covers three delimiter classes, two of which the
  bullet did not know about. §5's invariant (`:537`) — "escaping must never
  change what the Markdown renders to" — gains a sentence saying it now holds
  across an inline boundary, which it never has. The open-case list gains one
  new entry in the closed one's place: adjacent `Inline::Math`, recorded as an
  **unverified question** rather than a known defect, since it is clean under
  the parser the tier uses and unchecked against GitHub's own extension (§8).
- **`2026-08-14-empty-code-span-anchor-design.md`** — the Status block's scope
  correction and the § Non-goals correction that calls this shape "the module
  doc's second entry" both gain the closure note.
- **`escape::code_span`'s Rule 1 comment** (`escape.rs:454-479`) and
  **`clone_inlines_at`'s canonicalization comment** (`section.rs:160-171`) both
  reference the fusion; both are corrected. Rule 1's comment keeps its
  hand-built-caller reachability argument, which this item does not change.

## 7. Approaches considered

**A. Fuse adjacent same-delimiter runs at emission — chosen.** One rule over
three classes, at the seam where the defect is, with no IR rewriting and no
cloning. `kasane-gfm` is untouched and the anchor divergence closes as a
consequence rather than as a second fix. Costs the span boundary (§2.4), a
byte change for whitespace-separated emphasis runs that were already correct,
and one extracted predicate.

**B. Delimiter alternation.** Print the second emphasis run with `_`
(`*a*_b_` parses as two spans) and merge only code. Keeps the structural
boundary for the emphasis classes. Rejected: it buys that with CommonMark's
left- and right-flanking rules and the intraword-`_` carve-out, which are
Unicode-punctuation-sensitive and would become a fourth hand-mirrored rule in a
repo that has spent three items retiring hand-mirrored rules — and it buys it
for the class where the boundary matters least, while code, where a reader
actually sees two chips, still has to merge. Two rules for one defect, and the
riskier one covers the cheaper half.

**C. Separate the spans with an invisible marker.** An inline HTML comment
(`<!-- -->`) between them keeps both spans and renders as nothing. Rejected:
it makes every fused pair depend on inline HTML surviving the renderer's
sanitizer, it puts markup the IR never asked for into table cells and heading
lines, and it would need its own argument about what GitHub's heading-id
algorithm does with a comment. A fix that adds an unverified dependency to
close a verified defect is a bad trade.

**D. Canonicalize in `kasane-core::clone_inlines_at`, as the empty-span item
did.** Merge adjacent same-kind inlines in the IR before the writer sees them.
Reuses an established seam and fixes anchors and render together for anything
`structure`d. Rejected on the ground that separates this item from its parent:
that item fixed an *anchor* defect, and an anchor exists only for IR that went
through `structure`, so the seam covered every caller that had the bug. This is
a *content* defect. `blocks_to_markdown` is public API over a public IR, and a
caller who renders hand-built IR would still get one span reading ``` x``y ```
back — the writer would still be the thing that is wrong, with the fix sitting
in another crate.

Two further costs, either of them decisive on its own. The clone hazard in §2.2
applies here in full, since this approach *is* the IR rewrite. And `kasane-core`
cannot express the class: grouping a degrading `Math` with a `Code` requires
knowing `math_span`'s degradation rule, so this approach either imports an
escaping rule into core — which
`2026-08-14-shared-gfm-text-model-design.md` § Non-goals rules out — or leaves
the three degrading-math shapes broken.

**E. Compute anchors from the rendered line.** Would close the mismatch class
including this member. Still rejected as an item shape, still not foreclosed;
see § Non-goals.

## 8. Verification and risk

`mise run lint && mise run test` green, with `lint` covering `--all-targets`
plus `fmt --check`.

The proof specific to this item is §5.2's P13 — parsed text equals
`rendered_text` over sequences that can hold adjacent inlines — together with
the flipped test in §5.3 and the non-fusing controls in §5.1. Converting
`tests/fixtures/epub/rich.epub` before and after must produce an identical
tree, and that is a real check rather than a formality: the fixture's XHTML
holds no adjacent inline pair — verified by grepping its unzipped contents for
`</em><em>` and its siblings — so any diff at all means a run was grouped where
no run exists.

Two residual risks, both recorded rather than closed:

- **Adjacent `Inline::Math`.** Clean under `pulldown-cmark`, unverified against
  GitHub's math extension, and not fixable by fusing (§ Non-goals). It is not
  an anchor question, so the slug-widening probe is the wrong instrument; it is
  recorded in the escaping spec's open list as a question (§6) and answered by
  rendering a document with two adjacent inline equations on github.com,
  whenever someone next has an excuse to look. Two adjacent equations with no
  text between them are producible by the PPTX and EPUB math paths, so this is
  reachable rather than hypothetical.
- **The anchor mirror itself**, unchanged in kind by this item. `anchor_slug`
  mirrors github.com's filter and github.com can move; §8.1/§8.3 of the
  slug-widening spec remains how that is checked. This item adds one case worth
  probing when it next runs: a heading holding two adjacent code spans, whose
  printed line this item changes.
