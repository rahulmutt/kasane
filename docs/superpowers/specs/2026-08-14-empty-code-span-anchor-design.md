# kasane — Empty Code Span Anchor Design Spec

**Date:** 2026-08-14
**Status:** Designed, not implemented.
**Parent spec:** `2026-08-09-markdown-escaping-design.md` (§ "Recorded as open",
the empty-inline-code-span bullet), carried forward by
`2026-08-14-shared-gfm-text-model-design.md` (§ Non-goals, and the second
divergence named in its own Status block).
**Repo:** kasane

## 1. Purpose & scope

The shared GFM text model item closed two of the four anchor divergences
`kasane_gfm::slug`'s module doc recorded, leaving two: the `EMPTY_FALLBACK`
empty-id case, which is a deliberate choice, and one real construction defect.
This item closes the defect.

The defect: `escape::code_span`'s Rule 1 pads an empty code span to a single
space, because CommonMark has no other way to express one. That space is a real
character in the rendered line, and GitHub computes a heading's id from the
rendered line. The anchor rule computes from the IR's inlines, where
`Inline::Code("")` contributes nothing. A heading built from the text `a`, an
empty code span, and the text `b` therefore prints as ``a` `b``, which GitHub
ids `a-b`, while kasane embeds `ab` in every cross-reference to it. The link is
dead in the tree kasane itself produced.

### Scope

1. `Inline::Code("")` is canonicalized to `Inline::Code(" ")` once, on the way
   into the structuring engine, so no consumer downstream of that point can see
   the empty form.
2. No code span's rendering changes, by construction (§2.2).
3. Two things do change, deliberately and only for a heading that owns a file:
   its title line and its filename (§3). Stated here so it is not read as a
   side effect.
4. Tests pin the anchor result, and the byte-identity the approach rests on.
5. The divergence count drops from two to one everywhere it is written down.

### Confirmed, not assumed

- **Rule 1's trigger is exactly `t.is_empty()`.** `code_span` folds newlines
  first, and `fold_newlines` maps each newline to a space and never deletes;
  `Ctx::Cell`'s `|` → `\|` substitution only grows the string. So
  `content.is_empty()` after both transforms holds if and only if the input was
  literally empty. There is no "empty after folding" case to also catch, and
  the canonicalization's condition can be the same literal test.
- **`clone_inlines_at` is a choke point, not merely a common path.**
  `fold_sections` clones `doc` wholesale into the `SectionTree`, and
  `structure` destroys the original on the next statement via
  `kasane_ir::teardown_document(doc)` (`nav.rs:12-36`). `balance`,
  `assign_paths`, `resolve_refs`, `nav`, and every writer walk therefore read
  cloned IR only. Every inline that reaches an anchor or a rendered line passes
  through `clone_inlines_at` exactly once.
- **The defect is confined to the body-heading path.** A heading is printed by
  two paths, each anchored from its own projection, and only one of them is
  broken. A body heading renders through `Block::Heading` → `code_span` and is
  anchored by `count_headings` from `rendered_text` (`paths.rs:119`) — the
  printed line has the padding space, the anchor's input does not, and that is
  the bug. A heading that owns a file renders as `file_to_markdown`'s title
  heading, which prints `Frontmatter::title` — the plain `title_text` string
  `nav::walk` builds (`nav.rs:73`), with no backticks in it at all
  (`lib.rs:49-53`) — and `place` anchors that path from `title_text` to match
  (`paths.rs:60-63`, whose comment says why). It prints `# ab` and anchors
  `ab`: already self-consistent, and it stays consistent under every approach
  in §7. What separates them is what it costs to keep it that way.
- **`Code("")` and `Code(" ")` print the same bytes.** `Code("")` takes Rule 1
  and prints `` ` ` ``; `Code(" ")` takes Rule 2 (all-spaces content, which
  CommonMark's carve-out does not strip, so no padding) and prints `` ` ` ``.
- **The `est_tokens` precedent covers the test seam this needs.** AGENTS.md
  already documents `est_tokens` as a `#[doc(hidden)] pub` test seam existing
  because "the property tier needs the engine's own token estimate, and a copy
  in the test would drift". §5.2 needs the identical thing for the identical
  reason.

### Non-goals

- **The `EMPTY_FALLBACK` empty-id divergence.** A choice, not a construction
  defect: an empty fragment is a dead link. It stays, and after this item it is
  the only surviving entry in the module doc's list.
- **Math in a heading.** GitHub renders `$…$` through MathJax and what that
  does to the id is unmodelled. Untouched here; its own item if it is ever
  wanted.
- **Whitespace inside the merged-table HTML fallback.** Still open, still
  unreachable, and §3's `<code> </code>` note explains why this item does not
  disturb it.
- **The write/anchor mismatch *class*.** Computing an anchor from the rendered
  line rather than from IR inlines would close the class, at the cost of
  restructuring the `assign_paths`/writer ordering. Considered and rejected as
  this item's shape (§7, approach D); nothing here forecloses it.

## 2. The change

### 2.1 The arm

One arm in `section::clone_inlines_at` (`section.rs:146`):

```rust
Inline::Code(t) if t.is_empty() => Inline::Code(" ".into()),
```

placed before the existing `Inline::Code(t) => Inline::Code(t.clone())` arm.

The site is chosen for the choke-point property above: this is the first core
walk to touch adapter or caller IR, and the only one every inline is guaranteed
to pass through. A separate normalization pass would mean a second full walk of
the tree for one arm, and a second place to forget.

It does widen the function's charter. Today `clone_inlines_at` exists to bound
inline nesting while cloning; after this it also canonicalizes one inline
spelling. The comment above `clone_block` (`section.rs:84-94`), which is where
that charter is written down, gains a sentence saying so — the alternative,
leaving a canonicalization undocumented inside a function whose stated job is
depth bounding, is how the next reader deletes it.

### 2.2 Why no code span's rendering moves

Rule 1 and Rule 2 emit the same three bytes for these two inputs (§1,
"Confirmed"). The canonicalization therefore moves an `Inline::Code("")` from
Rule 1 to Rule 2 and changes nothing a reader sees *of the span itself* —
wherever a code span is printed as a code span, the page is byte-identical.
This is load-bearing rather than incidental, and it is why §5.1 pins it with a
test rather than a comment: if Rule 2 ever stopped matching Rule 1, this item
would start rewriting documents.

It is not a claim that the whole output is unchanged. The one place a heading's
inlines are printed *without* their markup is `file_to_markdown`'s title
heading, which flattens through `title_text` — there the added space is
visible, and the filename derived from it moves with it. §3 is the full
account.

### 2.3 Why the anchor is then correct

After canonicalization, `rendered_text` of `[Text("a"), Code(" "), Text("b")]`
is `a b`, `anchor_slug` gives `a-b`, and GitHub's id for the printed line
``a` `b`` is `a-b`. The bug is closed.

The file-title path moves too, though it was not broken: `title_text` now gives
`a b`, the printed title heading becomes `# a b`, and GitHub ids that `a-b`.
Still self-consistent, and now *identical* to the body path's answer — the same
source heading anchors the same way whether or not `balance` gave it a file of
its own. That property is A's, not the bug fix's, and §7 is where it earns its
cost.

## 3. Blast radius

Four consumers read an `Inline::Code`'s content. All four are affected — two
visibly, two below the noise floor:

- **`title_text`** — the file's own title heading, the frontmatter `title`,
  breadcrumb entries, TOC link labels, library index rows, and the plain-text
  fallback `refs` leaves where a link was stripped. A title of
  `[a, Code(""), b]` becomes `a b` rather than `ab`, and every one of those
  surfaces shows the space. Intended: a code span flattened to plain text has
  lost its backticks either way, and `a b` is what the body heading's own
  printed line shows a reader. The `ab` spelling was the one that agreed with
  nothing.
- **`path_slug`**, which reaches the same projection by its own route:
  `path_fold` calls `title_text` on the node's inlines (`slug.rs:172-178`). The
  file becomes `a-b.md` rather than `ab.md` — a user-visible **path** change,
  not only a content one.
  It is narrow, reaching only documents with an empty code span in a heading
  that owns a file, but it is stated here plainly rather than left to be
  discovered, in the same spirit as `balance`'s `Part N` renumbering.
- **`balance::est_tokens`** (`balance.rs:151`) — one byte where there were
  zero. Below the noise floor of a `max_tokens` decision.
- **`inlines_to_html`** (`markdown.rs:177`), the merged-table fallback —
  `<code></code>` becomes `<code> </code>`. Invisible: an HTML renderer
  collapses that run. It lands inside the fallback-whitespace case both parent
  specs already record as open and unreachable by escaping, and does not
  enlarge it.

Nothing else in production matches on `Inline::Code`'s content. `nav.rs:397`'s
`inline_contains_text` does, but it sits inside that module's `#[cfg(test)]`
block and is a test helper, not a consumer.

## 4. What this does to Rule 1

Rule 1 stays. `blocks_to_markdown` is public API over a public IR, so a caller
who hand-builds `Inline::Code("")` and renders it without going through
`structure` still reaches `code_span`, and CommonMark still has no way to spell
an empty code span. What changes is its status: it stops being a divergence and
becomes an ordinary rendering rule, because the only consumer that could
disagree with it — the anchor — is now unreachable from the empty form. That
caller has no anchor at all; `assign_paths` never ran for them.

Rule 1's comment is rewritten accordingly (§6). Keeping the current text, which
describes a live dead-cross-reference defect, would leave the next reader
hunting a bug that no longer exists.

## 5. Testing

### 5.1 Unit

- **`escape.rs`: `code_span("", ctx) == code_span(" ", ctx)`.** The
  load-bearing test of this item. The entire approach rests on Rule 1 and
  Rule 2 printing identical bytes; a future edit to either would otherwise
  break the fix silently, and invisibly, since the symptom is a dead anchor
  rather than a failing render. Asserted for `Ctx::Flow` and `Ctx::Cell`, the
  two contexts `code_span` is reached with; `Ctx::Html` renders a code span
  through `inlines_to_html`'s own `<code>` arm instead and never reaches it.
- **`section.rs`:** the arm fires at top level and nested inside `Emph` and
  inside a `Link`'s label, and a non-empty `Inline::Code` is untouched.
- **`paths.rs`:** a heading `[Text("a"), Code(""), Text("b")]` anchors `a-b`.
  Built as a `Document` and run through `fold_sections` before `assign_paths`,
  not against a hand-built `SectionTree` — the canonicalization lives in
  `fold_sections`' clone, so a test that skips it would pass while the pipeline
  stayed broken.

### 5.2 Property — P12

`crates/kasane-writer/tests/properties.rs` gains P12 — P11 is already taken by
the trailing-`#` property — in the shape P9 and P10
already use: build the inlines, render them, parse the rendered line back with
`parse_events`, and assert the parsed anchor equals the anchor the engine
embeds.

One wrinkle decides a small API question. P9 and P10 call
`anchor_slug_of(&inlines)` on raw inlines, which is faithful today because
nothing transforms inlines between the IR and the anchor. After this item that
is no longer true: the canonicalization lives in `kasane-core`, so a P12 built
the P9/P10 way would test a pipeline that does not exist and would fail against
a correct implementation.

The property therefore runs its inlines through the engine's own canonicalizer,
reached by exposing `section::clone_inlines_at` from `kasane-core` as a
`#[doc(hidden)] pub` test seam — the convention AGENTS.md already documents for
`est_tokens`, adopted here for the reason it gives there: a copy in the test
would drift. `kasane-writer` already depends on `kasane-core`, so no new edge
appears in the graph.

### 5.3 Generator

Unchanged. `generator/mod.rs:203` draws code-span content from `filler()`,
which never yields an empty string, so the main tier neither gains coverage of
this case nor gains failures from the change. P12 constructs the shape
directly, as P9 and P10 do for theirs.

Worth recording rather than acting on: were the generator to emit
`Inline::Code("")`, the main tier's round-trip properties would fail on
Rule 1's acknowledged round-trip divergence — `` ` ` `` parses back as
`Code(" ")`, not `Code("")`. After this item that divergence is unreachable for
any IR that went through `structure`, so a future generator change would want
to canonicalize its output rather than avoid the empty string.

## 6. Documentation

This closes the second of the two surviving divergences, so the count moves in
several places at once. All of them:

- **`kasane_gfm::slug` module doc** (`slug.rs:69-73`) — remove the bullet. The
  list becomes `EMPTY_FALLBACK` alone, and the surrounding prose that counts
  the survivors is corrected with it.
- **`escape::code_span` Rule 1 comment** (`escape.rs:454-461`) — rewritten per
  §4: a plain rendering rule, unreachable from core-structured IR, retained for
  direct `blocks_to_markdown` callers who have no anchor to diverge from.
- **`AGENTS.md:24`** — the `kasane-gfm` entry's "leaving two" becomes one, and
  the empty-code-span clause goes.
- **`AGENTS.md:88`** — "two divergences still survive there on purpose" becomes
  one, and "`rendered_text` and `escape::atx_closing` closed the other two"
  becomes three closed, naming the canonicalization as the third mechanism.
- **`README.md:163-175`** — "The two exceptions" becomes one exception; the
  empty-code-span bullet is deleted; and the closing paragraph's "Two anchors
  that used to diverge no longer do" becomes three, describing the new one in
  the same reader-facing terms as its two neighbours.
- **`2026-08-09-markdown-escaping-design.md:468-476`** — the open bullet gets a
  "closed" note in exactly the shape the footnote-ref bullet below it already
  uses: what closed it, by which mechanism, and what the original bullet
  predicted instead.
- **`2026-08-14-shared-gfm-text-model-design.md`** — the Status block's second
  divergence and the § Non-goals reference to it both gain the closure note.

## 7. Approaches considered

**A. Canonicalize `Inline::Code("")` in the IR before anchors are assigned —
chosen.** One arm at the pipeline's single choke point. It removes the
disagreement instead of teaching one side about the other, so no rule is
duplicated: `escape.rs` keeps its rules, `kasane-gfm` stays free of escaping
knowledge, and the empty form simply ceases to exist downstream. It also makes
a heading's anchor independent of whether `balance` gave it its own file, and
retires Rule 1's acknowledged round-trip divergence for every IR that went
through `structure` (§5.3). No code span's rendering moves (§2.2). Costs a
widened charter for `clone_inlines_at` (§2.1), the title-line and filename
change in §3, and a dependency on Rule 1/Rule 2 byte-identity that §5.1 pins.

This is a judgment call rather than a forced move, and the runner-up is close.
B is a smaller change that fixes the same bug and leaves titles and paths
untouched. A is chosen because the cost it pays is one-time and visible, while
the cost B pays is a standing mirror — and the last three items in this repo
have each been about retiring one of those.

**B. Teach `rendered_text` that an empty `Inline::Code` contributes a space.**
Three lines, correct, and the narrowest fix available: it touches only the
projection the defect is in, so titles and paths do not move. Defensible
against the shared-GFM spec's "escaping rules stay in the writer" non-goal
too, whose stated criterion was that only the fold had a second consumer —
this rule would now have one as well.

Rejected on one ground, not several. It is a mirror: Rule 1's padding and the
new arm would have to agree forever, by hand, across two crates, with no
compiler or test able to notice them drifting apart — a change to Rule 1 would
break anchors silently, since the symptom is a dead link rather than a failed
render. That is precisely the failure class the shared-GFM item existed to end,
and re-creating it one item later to save a title change is the wrong trade.

The secondary cost is smaller and worth naming honestly: under B the same
source heading anchors `ab` or `a-b` depending on whether `balance` split it
into its own file. Each answer is right for the line it was computed from, so
no link breaks — it is an inconsistency, not a defect.

**C. Stop padding inside a heading, so the line prints `ab`.** Agreement
reached by moving the render to the anchor, and the direct analogue of
`fold_inline_newlines`' writer-side pre-pass. Rejected: it deletes visible
characters from the output — a reader loses the `` ` ` `` the source document
contained — and it splits `code_span`'s behaviour by block context, which it
does not have today.

**D. Compute anchors from the rendered line rather than from IR inlines.**
Closes the whole mismatch class, including math-in-a-heading, permanently.
Rejected as this item's shape: it inverts the `assign_paths`-before-render
ordering the engine is built on, and bundling it here would swallow a defect a
single match arm closes. Recorded as available, not foreclosed.

## 8. Verification and risk

`mise run lint && mise run test` green, with `lint` covering `--all-targets`
plus `fmt --check`.

The proof specific to this item is the pair of assertions in §5.1 and §5.2:
byte-identity of the two `code_span` outputs, and the parsed-render/embedded
anchor agreement for the shape. Converting `tests/fixtures/epub/rich.epub`
before and after must produce an identical tree, since the fixture has no empty
code span — a diff there means the arm fired somewhere it should not.

**The residual risk is the anchor mirror itself**, unchanged in kind by this
item: `anchor_slug` mirrors github.com's filter, and github.com can move. The
external-oracle method in §8.1/§8.3 of `2026-08-08-slug-widening-design.md`
remains the way that is checked, and this case — a heading with an empty code
span between two words — is worth adding to the probe's cases the next time it
runs, since it was not among them when the shared-GFM item ran the oracle on
2026-08-14.
