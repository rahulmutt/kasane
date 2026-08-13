# kasane — Markdown Escaping Policy Design Spec

**Date:** 2026-08-09
**Status:** Approved (design), pending implementation plan
**Repo:** kasane

## 1. Purpose & scope

The batch-mode spec (2026-07-25) named "the repo-wide Markdown escaping policy"
a known deferred item; the slug-widening spec (2026-08-08) repeated it as a
non-goal. This item closes it.

The gap is that `kasane-writer` puts document text into a Markdown file without
ever asking whether that text is Markdown. `inlines_to_md`'s `Inline::Text` arm
is a bare `s.push_str(t)` (`markdown.rs:137`). Every character a book might
legitimately contain — `*`, `_`, `[`, `` ` ``, `|`, `<`, `&`, a leading `#` —
is therefore emitted as markup rather than as text. The failure is not
theoretical and not confined to prose:

- A paragraph reading `see chapter 3 [the long detour]` renders with the
  bracketed phrase swallowed as a broken link reference.
- A cell whose text contains `|` splits into two cells, and every row after it
  is misaligned against the header.
- A `<pre>` block containing a backtick escapes its own code span
  (`markdown.rs:140`, a fixed single-backtick wrapper).
- A title beginning with `-` or containing `: ` breaks the YAML frontmatter
  block, because `yaml_str` quotes only when the string contains `:` or `#`
  (`frontmatter.rs:30-36`).
- An external `href` carrying a space or a `)` — ordinary in EPUB and PPTX —
  ends the destination early (`markdown.rs:145-149`).

Two of these lose content outright; the rest corrupt structure. All of them
originate at the untrusted-input boundary and travel intact to the output.

### Confirmed, not assumed

Every site in `kasane-writer` where text reaches a buffer, and what it does
today:

| Site | Today | Class |
|---|---|---|
| `markdown.rs:137` `Inline::Text` | raw | text |
| `markdown.rs:140` `Inline::Code` | fixed `` ` `` wrapper | code |
| `markdown.rs:141` `Inline::Math` | fixed `$` wrapper | verbatim |
| `markdown.rs:145-149` external link | label + destination raw | text + dest |
| `markdown.rs:151` `FootnoteRef` | `[^{n}]`, `n: u32` | safe |
| `markdown.rs:27-34` heading | inlines raw, one line assumed | text |
| `markdown.rs:39-54` list item | body appended after marker, no indent | structure |
| `markdown.rs:96-108` merged table | text raw inside `<th>`/`<td>` | HTML |
| `markdown.rs:112-115` GFM table | cells joined on a raw `|` | text + structure |
| `markdown.rs:67-74` figure | alt text and caption raw; asset filename raw | text + dest |
| `markdown.rs:76-82` code block | fixed ` ``` ` fence; `lang` raw | code |
| `markdown.rs:86` footnote | body trimmed onto one line | structure |
| `markdown.rs:88` `Block::Raw` | note raw inside `<!-- -->` | HTML |
| `lib.rs:32` file title heading | raw, one line assumed | text |
| `frontmatter.rs:5-26` frontmatter | conditional quoting; other scalars raw | YAML |
| `library.rs:80-84` `link_text` | `[`/`]`→parens, newlines→space | text |
| `library.rs:106-122` `link_dest` | percent-encoding | dest |

The last two are the only escaping that exists today. Its doc comment already
calls itself a stopgap for "a separate, known-deferred item" — this one.

Three facts from outside the writer shape the design:

1. **Anchors and paths are computed from IR text, not from rendered text.**
   `anchor_slug` and `path_slug` take `&[Inline]` (`kasane-core/src/slug.rs`).
   Escaping happens strictly downstream of them, so it cannot change a path or
   a fragment — provided it also does not change what the Markdown *renders*
   to. §5 makes that the governing invariant.
2. **`est_tokens` sizes IR, not output** (`balance.rs:171`), so the size guard
   is likewise unaffected by added backslashes.
3. **The property tier's helpers are alphabet-bound.** `links_in`
   (`properties.rs:53-62`) and `heading_anchors` (`properties.rs:115-148`)
   carry doc comments stating they are correct only because the generator draws
   from `WORDS` plus a `zq####` sentinel, neither containing a bracket or a
   `#`, and that "a generator that ever draws arbitrary text needs a real
   Markdown pass here first." This item is that generator. §6.3 is the pass
   those comments asked for.

### Scope

- A new `crates/kasane-writer/src/escape.rs` holding every rule (§2, §3).
- A required `Ctx` argument on the inline renderers, so a call site must name
  its context (§2).
- The structural fixes escaping alone cannot make: newline folding, footnote
  and list-item continuation, code-span and fence widening, comment safety
  (§4).
- `library.rs`'s two local functions folded into the shared module (§2).
- A round-trip property (P7) over a real GFM parser, the existing helpers
  rebuilt on it, a widened generator alphabet, and an `escape` fuzz target
  (§6).
- One new dev-dependency, `pulldown-cmark` (§6.1).

### Non-goals

- **The `insta` snapshot tier.** Still design spec §9's last unbuilt tier.
- **Total path length.** Unchanged from the slug spec: depth comes from
  heading nesting plus `-o`, and Windows' 260-character default remains
  reachable.
- **Destination *sanitization*.** A `javascript:` or `data:` URL inherited
  from a source `href` is escaped here so it cannot break the Markdown
  grammar, and is otherwise passed through. Whether kasane should refuse to
  emit such a destination at all is a security-policy question with its own
  trade-offs and deserves its own item.
- **The slug and anchor rules.** Untouched; §5 is the argument that they stay
  untouched.

## 2. One module, one way in

`escape.rs` is `pub(crate)` and owns every rule. The design constraint behind
its shape is that the contexts do not share a mechanism: a backslash escapes in
flow text, is a literal character inside a code span, is inert inside an HTML
block, and means something else again inside a YAML double-quoted scalar. A
single `escape(s: &str)` would therefore be wrong in three of the six places it
was called.

```rust
pub(crate) enum Ctx { Flow, Cell, Html }

pub(crate) fn text(s: &str, ctx: Ctx, at_line_start: bool) -> String;
pub(crate) fn code_span(s: &str, ctx: Ctx) -> String;
pub(crate) fn fenced_block(text: &str, lang: Option<&str>) -> String;
pub(crate) fn dest_path(s: &str) -> String;
pub(crate) fn dest_url(s: &str) -> String;
pub(crate) fn label(s: &str) -> String;
pub(crate) fn one_line(s: &str) -> String;
pub(crate) fn math_span(s: &str, ctx: Ctx) -> String;
pub(crate) fn math_block(s: &str) -> String;
pub(crate) fn yaml_scalar(s: &str) -> String;
```

`code_span` and `math_span` take a `Ctx` for one reason and only one: GFM's
table grammar splits a row on `|` *before* any inline is parsed, so a pipe
inside a code span or a math span still ends the cell, and GFM's own answer is
a backslash it strips back out (`` `b \| az` `` renders `b | az`; `$\|x\|$`
recovers `InlineMath("|x|")`). Nothing else about either is
context-dependent.

`math_span` and `math_block` are the writer's half of the math contract (§3.7)
and did not exist in this section's first draft, which had the two math arms
formatting `${t}$` and `$$\n{s}\n$$` inline in `markdown.rs`.

`inlines_to_md` and `inlines_to_md_at` take `Ctx` as a required parameter, not
as a defaulted field or a struct with a `Default` impl. That is the enforcement
mechanism: a new `Inline` arm, or a new caller in a future block renderer,
cannot silently inherit flow rules into a cell — it does not compile until it
names a context. `Inline::Text` becomes the only arm that calls
`escape::text`, and it is the only place in the crate permitted to.

`at_line_start` is threaded rather than inferred. It is `true` for the first
character a renderer emits on a line, and `escape::text` re-arms it internally
after every `\n` it passes through, so `Inline::Text("intro\n# not a heading")`
inside a paragraph escapes the `#` beginning its second line. Contexts where a
newline cannot occur at all — headings, GFM cells, YAML scalars — fold newlines
before escaping (§4.1) and pass `at_line_start: true` exactly once.

`library.rs` loses `link_text` and `link_dest`; the library index calls
`escape::label` and `escape::dest_path`. This is what makes the policy
repo-wide rather than a second local dialect. `write_library_index`'s output is
unchanged in every case its current tests cover, since `link_dest`'s rule
*becomes* `dest_path`'s rule.

## 3. The rules

This section is the case table. It is reproduced in `escape.rs`'s module
documentation, and §6.1 pins each row with a test.

### 3.1 `Ctx::Flow`

Paragraph text, heading text, list-item text, footnote-body text, figure
captions, link labels, image alt text.

**Escaped at any position**, with a preceding `\`:

| Char | Construct it would open |
|---|---|
| `\` | an escape — must be first, or every escape below becomes escapable |
| `` ` `` | code span |
| `*` | emphasis, strong |
| `_` | emphasis, strong |
| `[` | link, image, footnote reference, link reference definition |
| `]` | the closing half of the same |
| `<` | autolink, raw HTML, HTML comment, declaration |
| `~` | GFM strikethrough |
| `$` | math, which GitHub renders and kasane emits deliberately |

`&` is escaped **conditionally**: only when the text that follows would parse
as an entity reference — `&` followed by an ASCII-alphanumeric name and `;`, or
`&#` and decimal digits and `;`, or `&#x` and hex digits and `;`. CommonMark
defines that grammar exactly, so the lookahead is precise rather than
heuristic, and it matters because `&` is common in real titles: unconditional
escaping would render every "Q&A" and "Tom & Jerry" backslashed for no parsing
benefit. `<` gets no equivalent lookahead: it is rarer in prose and its grammar
is four constructs wide, so an approximate test would be the risk the exact one
avoids.

`!` is deliberately **not** escaped. It is only meaningful immediately before
`[`, and `[` is unconditionally escaped, so `!\[` cannot form an image. Every
exclamation mark in the corpus stays clean as a consequence of a rule made
elsewhere.

**Escaped only when `at_line_start`:**

| Char | Construct |
|---|---|
| `#` | ATX heading |
| `-` | bullet, setext h2 underline, thematic break |
| `+` | bullet |
| `>` | block quote |
| `=` | setext h1 underline |
| `|` | table row |
| `.` / `)` after a leading digit run | ordered-list marker (`1\.`) |

Escaping `-` and `=` at line start covers setext underlines and `---`
thematic breaks without separate rules. `` ` `` and `~` need no line-start rule
because they are already escaped everywhere, which also disposes of fences.

### 3.2 `Ctx::Cell`

`Ctx::Flow`'s rules, plus `|` escaped at **every** position — GFM's documented
in-cell escape — and newlines folded per §4.1. A cell is a single line by
grammar, so `at_line_start` is true only for the cell's first character.

### 3.3 `Ctx::Html`

The `has_merged` `<table>` fallback (`markdown.rs:92-110`).

Backslash escapes do not apply: GFM parses an HTML block's content as raw HTML,
not as Markdown. Text is therefore HTML-escaped — `&`→`&amp;` (here
unconditionally; inside HTML every `&` is an entity opener), `<`→`&lt;`,
`>`→`&gt;`, `"`→`&quot;`.

The same fact forces a second change, and it is a fidelity fix rather than a
safety one: **inline markup in this path must be emitted as HTML tags**, not as
Markdown. Today `esc = |c| inlines_to_md(c)` (`markdown.rs:96`) puts `**bold**`
inside a `<td>`, which GitHub renders with the asterisks visible, because
nothing inside that HTML block is parsed as Markdown. Under `Ctx::Html` the
renderer emits `<strong>`, `<em>`, `<code>`, `<a href="…">` instead. Math is the
one inline that stays as-is: `$…$` is not parsed inside an HTML block either,
so a merged-cell equation degrades to its literal LaTeX. That is a documented
limitation of the merged-cell path, not a regression — it renders no worse than
today, and the alternative is emitting MathML we do not have.

### 3.4 Code spans and fences

Neither takes backslash escapes; both are solved by choosing a delimiter the
content cannot contain.

**`code_span`** follows CommonMark: the wrapper is a backtick run one longer
than the longest backtick run in the content. Padding with a single space at
each end is added in three cases:

1. Empty content: a single space fills the span (the one acknowledged divergence
   from round-trip, required because CommonMark cannot express an empty code span).
2. Content containing backticks, or starting/ending with space: a reader strips
   exactly one space from each end when the span begins and ends with space but
   does not consist entirely of spaces, so the padding is invisible.
3. Content consisting entirely of spaces receives no padding, because the
   carve-out means no stripping occurs; the input round-trips exactly.

Newlines inside a code span fold to spaces, since a blank line would end the
enclosing paragraph.

**`fence`** picks a backtick fence of `max(3, longest_run + 1)` against runs at
any position in the body, and sanitizes the info string to its first
whitespace-free token with backticks removed — a `lang` carrying a space, a
backtick or a newline breaks the opening fence, and an info string is a single
token by grammar anyway.

### 3.5 Destinations

Two functions, and the split is load-bearing.

**`dest_path`** is today's `link_dest`, unchanged in rule and moved in
location: it percent-encodes `%`, `#`, `?`, space, `(`, `)`, `<`, `>`, `\`,
`"`, and control characters, leaving `/` literal as a path separator. It is for
destinations kasane *constructs* from the filesystem: `_assets/<filename>` and
the library index's `rel_dir`. Encoding `%` is essential there, because a
literal `%` in a filename would otherwise read back as an escape.

An in-book cross-reference is **not** one of them, despite the shape looking
like a path. `refs::relativize` resolves a `RefTarget::Internal` into a
`RefTarget::External` holding `path#anchor`, which reaches `dest_url`
(`markdown.rs`'s external-link arm) — and that is correct, not an oversight:
`dest_path` encodes `#`, so routing a cross-reference through it would
percent-encode the fragment separator and break every anchor kasane emits. The
path half is safe under `dest_url`'s narrower set because `path_slug`'s
alphabet is closed (`markdown.rs`'s
`path_slugs_contain_nothing_that_breaks_a_bare_destination` pins it), and the
anchor half likewise (`anchors_contain_nothing_that_breaks_a_bare_destination`).

**`dest_url`** is for `RefTarget::External`, which is a URL that arrived from a
source document's `href` and is therefore *already* percent-encoded. Encoding
`%` again would turn every legitimately-encoded link into a broken one, so
`dest_url` leaves `%` alone and encodes only what ends or nests a bare
destination: space, `(`, `)`, `<`, `>`, `"`, `\`, and controls.

The asymmetry is the whole point of having two functions, and each carries the
other's name in its doc comment so a future reader meets both at once.

### 3.6 YAML scalars

`yaml_scalar` replaces `yaml_str` and **always double-quotes**. Inside the
quotes: `\`→`\\`, `"`→`\"`, and newlines and control characters folded to a
space (§4.1).

Quoting unconditionally deletes the question of which characters require
quoting — a question today's implementation answers with `:` and `#` and gets
wrong for a leading `-`, `[`, `{`, `&`, `*`, `!`, `|`, `>`, `%`, `@`, a quote
character, a trailing space, and the bare words `true`, `null` and `~`, each of
which YAML reads as something other than a string. The cost is two bytes per
line.

Every scalar the frontmatter emits is quoted, not only `title`: the joined
`breadcrumb`, `parent`, `prev`, `next`, and each `children` entry. The path
fields draw from the closed slug alphabet and do not need it, but a uniform
block has one rule instead of five, and the closed-alphabet argument is one
more thing that would have to stay true. `source_pages` stays unquoted — it is
`{integer}-{integer}` built by `format!`, never text.

### 3.7 Math

`Inline::Math` and `Block::MathBlock` carry LaTeX, not Markdown, and the writer
escapes nothing inside them: there is no escape available. `\$` would corrupt
adapter output that already spells a literal dollar that way, and neutralizing
`\`, `{` or `}` would destroy the `\frac{1}{2}` an adapter legitimately emits.
Safety comes from a contract in `kasane-adapters`, which neutralizes `$`, `{`,
`}`, `\` and newlines in every node kind that carries document text —
`math::latex::sanitize` for `<mn>`/`<mtext>` and for `mfenced`'s delimiter
attributes, `math::symbols::map_text` for `<mi>`/`<mo>` and every OMML run.

The writer nonetheless carries a **self-check**, because `blocks_to_markdown`
is public API over a public IR and a caller who builds `Inline::Math` by hand
never passes an adapter. Content that would close the delimiter the writer
opens — a `$` in either form, any newline in the inline form (which can land in
a table cell), a blank line in the block form — degrades to a code span or a
fenced block instead. The LaTeX is still there for a reader, verbatim, and it
cannot break out of a code fence by construction. Adapter-produced math never
reaches that branch, which is exactly the shape of `render_block`'s depth guard.

`math_block`'s blank-line test runs against the **wrapped** string rather than
the content, because the wrapper supplies newlines of its own: content that
merely starts or ends with a single newline completes a blank line together
with them, and `"a\n"` produced `"$$\na\n\n$$\n"`, which a real parser reads as
two paragraphs of literal `$` with the closing fence stranded.

`math_span` takes a `Ctx`, and that half is **not** hand-built-IR defence:
`pptx/slide.rs` pushes `Inline::Math` straight into a table cell, and `|`
survives the adapter's `map_text` untouched (it is `ascii_graphic` and outside
the symbol table). A PPTX cell holding `|x|` emitted `$|x|$`, which GFM splits
into `$` and `x` and then drops the row's real last cell — content loss, or a
destroyed table when the row is the header. Both branches escape it: the
verbatim one because the span itself sits in the cell, the degrade one by
passing `ctx` into `code_span`. The backslash is consumed by the table grammar
before the math renderer sees it, so `$\|x\|$` recovers `InlineMath("|x|")`.

## 4. What escaping alone cannot fix

### 4.1 Newlines

A newline is not an escapable character, so each context decides:

| Context | Rule |
|---|---|
| Heading (`markdown.rs`'s `Block::Heading`, `lib.rs`'s title) | fold any *run* of `\r\n`/`\n`/`\r` to one space |
| GFM cell | fold to `<br>` |
| HTML cell | fold to `<br>` |
| Link label, image alt | fold a run to one space |
| Code span | fold a run to one space |
| YAML scalar | fold a run to one space |
| Flow text | keep a single `\n` as a soft break; collapse runs of 2+ to one |

The run-collapsing is what lets `escape::one_line` be a *single* function
serving every row above it, and it is load-bearing rather than tidy. The two
heading paths reach it from opposite directions — `Block::Heading` escapes then
folds, so `escape::text`'s own collapse has already run; `file_to_markdown`'s
title heading folds then escapes, so nothing has collapsed anything — and
`code_span` folds unescaped content, a third variant. Without the collapse the
three disagreed on a blank line (`## A B` against `# A  B`), and
`kasane-core`'s `anchor_fold`, which predicts the rendered heading line to
compute a fragment, can only predict one of them: a heading containing a blank
line rendered `## A B` (GitHub id `a-b`) while the emitted cross-reference
pointed at `#a--b`. `slug::fold_newlines` is the hand-kept mirror and collapses
identically. Literal spaces are a different mechanism and are *not* collapsed
on either side, so `Background & Notes` still anchors `background--notes`.

A heading is one line by grammar: a newline in a title today turns the tail of
the title into a separate block. `<br>` is GFM's only multi-line-cell carrier.
The blank-line collapse in flow text is what keeps one `Block::Para` one block;
without it, a text run containing `\n\n` splits into two paragraphs, which the
structural half of P7 would (correctly) fail on. In practice most of these
newlines are PDF and DjVu line-break artifacts rather than authored breaks.

### 4.2 Footnote continuation

`markdown.rs:86` emits `[^{id}]: {body.trim()}`. A body with more than one line
— any multi-block footnote — puts its second line at column zero, outside the
definition, where it becomes a sibling paragraph. Fixed by indenting every
continuation line by four spaces, GFM's footnote continuation indent, with
interior blank lines left genuinely blank (no trailing spaces).

### 4.3 List-item continuation

The same bug in a different renderer. `markdown.rs:39-54` pushes `- ` (or
`N. `) and then the item's entire rendered body, so an item holding a paragraph
*and* a nested list drops to column zero on its second line and leaves the
item. Fixed by indenting continuation lines by the marker's width — two spaces
for `- `, `N. `'s own width for an ordered item.

This changes existing output for nested and multi-block list items. It does
**not** change the marker line, so a heading leading a list item still renders
`- ## Notes`, and `heading_anchors` (`properties.rs`, rebuilt on
`pulldown-cmark` by Task 13 — the hand-rolled `strip_list_markers` scanner it
originally used no longer exists) and the anchor counting it feeds keep
working unchanged: a real GFM parser reads `- ## Notes` as a heading
regardless of the marker prefix, with no line-scanning helper needed.

### 4.4 `Block::Raw` comments

`<!-- {note} -->` (`markdown.rs:88`) breaks on a note containing `-->`, and is
malformed for a note ending in `-`. Notes are **not** always internal fixed
strings — `epub/xhtml.rs` and `epub/mod.rs` both build one with
`format!("image unavailable: {src}")`, where `src` is an untrusted `<img
src>` attribute value read straight off the document — so this is load-bearing
mitigation on live, adapter-reachable content, not defence in depth on a
surface that can't be hit: `--` runs in the note are broken with a space, and
a trailing `-` gets one.

An HTML comment has no escape mechanism at all, unlike every other construct
this spec covers — no backslash, no entity, nothing that lets `-->` appear
literally inside `<!-- -->`. `comment_note` therefore cannot escape a
dangerous run, only transform it, and the transformation is forced by the
format rather than chosen. §5 documents the consequence: `Block::Raw` is the
one place this policy's own invariant does not hold.

### 4.5 Emphasis delimiters at a line start

`at_line_start` guards `Inline::Text`, but not the markup the *writer* emits at
column 0. `Block::Para([Emph([Text(" x")])])` rendered `* x*`, which a GFM
parser reads as a bullet list rather than a paragraph — reachable from
`<p><em> Note:</em> …</p>`. The same output has a second defect independent of
position: CommonMark's flanking rules mean a `*` with adjacent whitespace is
never an emphasis delimiter, so the emphasis was silently lost mid-line too.

Both are fixed by moving whitespace at the edges of the rendered inner content
*outside* the delimiters — `" *x*"` — which is what CommonMark's "emphasis
cannot begin or end with whitespace" rule asks for anyway. The rendered text is
unchanged (§5), the emphasis now applies, and the first character on the line
is a space rather than a bullet marker. Inner content that is entirely
whitespace gets no delimiters at all.

All three sibling cases recorded here as unfixed were closed by the escaping
residuals item (2026-08-13); see that spec for the mechanisms. Two of the
three were smaller than this section assumed, and for the same reason: this
section reasoned from the CommonMark grammar where a parser was available.
`   \# h` does suppress the heading, but the parser then strips the three
leading spaces, so the backslash form loses the text it was meant to protect —
a §5 violation presenting as a fix. A numeric character reference preserves it
and disarms the whole run, which made the "changes the line-start rules for
every context at once" characterization an artifact of the mechanism rather
than of the defect.

Two cases remain open, both narrower than what was closed:

- **A newline run split across an `Inline::FootnoteRef`.** The reference
  renders as visible `[^1]` text that `inline_text` skips, so the fold cannot
  collapse across it without dropping a space GitHub renders. Same root cause
  as the `## Notes[^1]` divergence `kasane-core::slug` documents as surviving
  on purpose; it needs §8's approach (iii), not a fold change.
- **Whitespace inside the merged-table HTML fallback.** Not reachable by
  escaping: an HTML renderer collapses whitespace runs whether they arrived
  literally or as `&#32;`. One more cost of that path, alongside §3.3's.

The emphasis fix in this section still has no property-tier coverage — see
§6.4 for why the generator cannot reach it.

## 5. The invariant that ties this to the slug rules

**Escaping must never change what the Markdown renders to.**

This is not a style preference; it is what keeps §1's fact 1 true. `anchor_slug`
computes a fragment from unescaped IR inlines, while GitHub computes the
heading's id from the *rendered* text of that heading. The two agree today
because the rendered text and the IR text are the same string. A backslash
escape preserves that: `\*` renders as `*`, so the rendered text is unchanged
and both sides still agree. An escaping scheme that instead replaced or dropped
characters — mapping `[` to `(` the way `library.rs`'s `link_text` does, say —
would break every in-book cross-reference to a heading containing one.

That is also why `link_text`'s bracket substitution does not survive the move:
`escape::label` escapes rather than substitutes.

Three consequences follow, and all three are load-bearing:

- Paths are byte-identical before and after this change, with no exception.
  Anchors are byte-identical **except** for a heading whose inline text
  contains a newline: `anchor_slug` was corrected in `kasane-core` to fold
  an embedded `\n` the same way `escape::one_line` folds it in the rendered
  heading line, which moves that anchor's hyphenation to match what a real
  parser now computes. Slug design spec §8.2
  (`2026-08-08-slug-widening-design.md`) has the evidence and the two rows
  that pin it. Outside that one shape, the churn is entirely in file
  *content* — the exact opposite of the slug branch, which churned paths and
  left content alone.
- The size guard is unaffected: `est_tokens` measures IR (`balance.rs:171`).
- The newline foldings in §4.1 are the one deliberate exception, and each is a
  case where the current output does not render as its text at all.

**`Block::Raw` is a documented exception to the invariant itself**, not
another case where the rendered text happens not to matter. §4.4's
`comment_note` breaks up a `--` run inside a note, and that transformation
*does* change what the comment's content reads as. It is an exception rather
than a bug for two reasons together: an HTML comment has no escape mechanism
at all, so there is no way to represent `-->` literally inside `<!-- -->` —
every other construct in this spec has one (a backslash in flow text, `<br>`
in a cell, a longer delimiter run for a code span, entity escapes in HTML,
quoting in YAML) and `Block::Raw` alone does not; and a comment's content is
never rendered, so no reader ever sees the difference the transformation
makes. §6.2's round-trip property (P7) accordingly excludes `Block::Raw`
payloads from its check — proving a note cannot break out of its comment is
the fuzz seam's job (§6.5), not P7's.

## 6. Testing

### 6.1 The case table

`escape.rs`'s unit tests pin every row of §3: each always-escaped character,
each line-start character in both positions (escaped at a line start, untouched
mid-line), the conditional `&` in both directions (`&amp;` escaped, `Q&A` not),
`!` unescaped before an escaped `[`, backtick-run selection at each width,
padding for content that begins or ends with a backtick, fence widening, the
`%` asymmetry between `dest_path` and `dest_url`, and each YAML scalar shape
that today's conditional quoting gets wrong.

Following `slug.rs`'s precedent, the table is documentation of kasane's reading
of CommonMark — §6.2 is what checks the reading.

### 6.2 P7, the round-trip property

`pulldown-cmark` joins `kasane-writer`'s dev-dependencies (MIT, test-only,
never in the shipped binary), with the GFM extensions kasane emits enabled:
tables, footnotes, strikethrough, **and math**. Math was disabled in the
original plan, on the reasoning that kasane's own `$…$` would then arrive as
literal text, matching an escaped `\$` in prose. That covers the `$`
delimiter but not math *content*: with the extension off, a hostile
character legitimately present in verbatim math is read back through the
ordinary inline grammar instead of treated as opaque, which is not what
GitHub does. With math on, `InlineMath`/`DisplayMath` event text is
collected the same way `Text`/`Code` is, and the `\$`-in-prose case was
re-verified directly against the parser: an escaped `$` still never opens a
math span, so the two sides still agree without a special case.

**What shipped is an occurrence count, not an equality.** For each file of a
generated case: render with `file_to_markdown`, parse, fold the event stream
into one string, whitespace-normalize it, and require each generated sentinel's
**payload** to appear the number of times its shape says it should
(`Expect::Exactly(1)` for most, `AtLeast(1)` for a heading, `Exactly(2)` for a
numbered figure's twice-rendered caption). Payloads whose `Block::Raw` note
`escape::comment_note` actually transforms are skipped — §5's carve-out, scoped
to the transformation rather than to the shape.

That is weaker than the "derive the same sequence from the IR and require exact
equality" this section originally specified, in three ways worth naming rather
than glossing: the generated decoration inlines and the filler text around a
payload are not checked at all; *extra* recovered text is invisible, since only
occurrences of the payload are counted; and whitespace normalization means a
lost space cannot fail it. Every other character in a payload is checked, which
is what makes a missed escape a failure. The equality form was not built
because deriving the IR-side sequence means re-implementing render order —
including what `balance` moved between files and what `nav` synthesized — in
the test, which is the drift the `#[doc(hidden)]` test seams exist to avoid.

The structural half is **P8**, a separate property, and it checks: one footnote
definition per `Block::Footnote`; the full sequence of heading levels in render
order, the file's own title heading first at its breadcrumb depth and each
`Block::Heading` after it at the level `render_block` clamps to; and, per
non-merged GFM table, the row count and a cell count.

The **row** count is the one that catches an unescaped `|`, and it catches it
through the *header*: an extra pipe there changes the header's column count,
the delimiter row stops matching it, and GFM stops recognizing the table
entirely — 0 rows parsed against the IR's N. Both pipe defects this branch
found (`escape::code_span`, then `escape::math_span`) presented that way.

The **cell** count cannot fail independently and is kept as documentation of
the intended grid rather than as a check. `pulldown-cmark` pads a short row and
drops a long one against the header's column count (`firstpass.rs`), so a
recognized table always reports exactly `header.len() * (1 + rows.len())` —
`want_cells` by construction — and an unrecognized one reports 0 for rows and
cells alike, where the row assertion fires first. An earlier revision of this
section claimed it saw what a row count could not; it does not.

List nesting depth is deliberately **not** checked. Unlike a row or a heading it
has no event asserting "this is the same list the IR built": `balance` may have
moved a list into another file, and a nested list that lost its indent appears
as a *sibling* list, so the check would have to reconstruct the tree from the
event stream. P1's `Expect::Exactly` plus `markdown.rs`'s
continuation-indent unit tests cover the failure §4.3 was written for.

The merged-table path arrives as `Event::Html`; the test extracts its text
through a small tag-stripping and entity-decoding step rather than adding an
HTML parser to the writer's dev-dependencies.

### 6.3 The existing helpers, rebuilt

`links_in` and `heading_anchors` move onto pulldown events. This is required,
not opportunistic: both are correct today only under the narrow generator
alphabet, and their own doc comments say so and name the widening in §6.4 as
the trigger. Under hostile text, `links_in`'s string scan would collect a `](`
inside a fenced code block and `heading_anchors` would count a paragraph line
that merely begins with `#` — and, as those comments spell out, each error runs
in the *unsafe* direction, producing a false P2 failure rather than a lenient
check.

Rebuilt on events, `heading_anchors` reads real heading text and feeds it to
`kasane_core::anchors_for_headings`, which is what makes P2 strong here: the
engine computes the anchor from unescaped IR text, the test recomputes it from
parsed rendered text, and P2 passing on a hostile title is a direct proof of
§5's invariant.

### 6.4 The widened generator

A hostile draw (`HOSTILE`) joins `WORDS` in `tests/generator/mod.rs`, mixed
into the same `filler()` both feed: the inline openers `*star*`, `_under_`,
`[bracket]`, `]close`, `` `tick` ``, `<html>`, `$math$`, `~~strike~~`,
`back\slash`, `!bang[`; the entity pair `&amp;` and `&raw`; the line-start
openers `#hash`, `- bullet`, `1. ordered`, `> quote`, `= setext`; a `|pipe|`; a
` ```fence `; the comment closer `-->`; and three newline shapes —
`line\nbreak`, `a\n\nb` and `a\r\nb`. The combining-mark and CJK samples live
in `WORDS` (`हिन्दी`, `第二章`) alongside `&`, `don't` and `foo_bar`, which
exercise the slug rules.

It feeds section titles, paragraph text, table cell text, code-block text,
figure captions, `Block::Raw` notes, `Block::MathBlock` content, footnote
bodies — and, since this section's first draft did not reach them, **code-span
text and external `href`s**: `generator::inlines()` draws `Inline::Code` and an
`Inline::Link { target: RefTarget::External(_) }` alongside `Emph`/`Strong`, the
`href` from a small `HREFS` pool holding a space and a `)`, an already-encoded
`%20`, a query and a fragment, and `<`/`>`/`"`. Without them
`escape::code_span`, `escape::dest_url` on a real `href`, and
`inlines_to_html`'s `<a href>` composition had no property-tier coverage at all
— and adding them immediately failed P8 on the unescaped `|` inside a code span
in a table cell, and P2 on `code_span` folding a newline run differently from
`anchor_fold`.

Titles carry the most weight and are drawn from it most often, because one
title reaches the heading line, the frontmatter scalar, the anchor, the path
component, and the library entry — five contexts from one string.

`Inline::Math` is drawn in `generator::inlines()` too, and is adapter-realistic
rather than defensive: `pptx/slide.rs` pushes it into a table cell and into a
paragraph. It is the only route by which the tier reaches `escape::math_span`,
whose `Ctx::Cell` pipe rule a real PPTX equation needs — reverting that rule
fails P8 immediately.

**What the tier still cannot reach.** §4.5's emphasis fix has no property
coverage and is pinned by its unit test alone. `build()` appends the generated
decoration inlines *after* the sentinel text, deliberately, so that the
payload always renders as a bare run and P1's occurrence count stays exact —
which also means an `Emph`/`Strong` can never be the first thing on a line,
and P7 checks only payload occurrences, never decoration content. Adding
whitespace-flanked fragments to `HOSTILE` does not help: measured, with
`emphasize` reverted to the pre-fix `format!("*{inner}*")` and `" lead"` /
`"trail "` added, the tier stays green over 2048 cases. Reaching it would mean
changing where decoration sits relative to the payload, which trades a real
invariant for a hypothetical one.

`Block::MathBlock` draws the raw hostile fragment like every other shape. An
earlier revision pre-neutralized `$` for that one shape, modelling
`kasane-adapters`'s math contract instead of testing anything — and the
contract had a hole exactly there. It is closed (§3.7), the writer carries a
self-check for the callers no adapter guards, and the generator now draws the
hostile fragment.

The sentinel scheme is unaffected: `zq####` is alphanumeric, so no escape can
appear inside a sentinel and P1's occurrence counting keeps working. `WORDS`
stays free of the `zq` prefix for the same reason as before, and the hostile
draw is likewise free of it.

### 6.5 Fuzzing

A new `escape` target, on the `slug` precedent and by the same argument:
untrusted text entering a format with a grammar. `kasane-writer` gains a
`#[doc(hidden)] pub` `fuzz_entry.rs` seam — the convention already used by
`kasane-adapters` and `kasane-core` — exposing a `fn(&[u8])` that treats its
input as UTF-8 text, runs it through each context, and asserts the round-trip:
what a real parser recovers equals what went in.

It is wired into `fuzz/` and into the stable replay in
`kasane-adapters/tests/fuzz_corpus.rs`, which already takes `kasane-core` as a
dev-dependency so one harness covers every target; it gains `kasane-writer` for
the same reason. Seeds cover each context's hostile characters.

### 6.6 End-to-end

One conversion of an existing fixture, asserting that the emitted tree's paths
and anchors are byte-identical to those the same fixture produced before the
change — §5's first consequence, checked rather than asserted.

**Pass condition, not a bare diff.** Paths must match with zero exceptions.
An anchor diff is legitimate, and expected rather than a regression, *only*
for a heading whose inline text contains a newline (`\n` or `\r\n`) — the
one shape `anchor_slug`'s newline fold moves, per the amendment to this
section's list above and slug design spec §8.2. Any other anchor diff, or
any path diff at all, is a real regression. If the fixture used here
happens to contain such a heading, expect and confirm exactly that diff
rather than treating its presence as a failure.

## 7. Documentation

- **README, Known limitations.** A short entry: output contains backslash
  escapes, and text in a source document that looks like Markdown is preserved
  as literal text rather than becoming markup. That is the user-visible
  semantic — a book that literally prints `*` shows `*`.
- **AGENTS.md,** under `crates/kasane-writer`: `escape.rs` is the only path
  from text to output; `Ctx` is a required argument rather than a defaulted
  one, and that is the mechanism preventing a future call site from inheriting
  the wrong rules; `dest_path` and `dest_url` differ on `%` and why.
- **This spec's §1** records, dated, that the item deferred since the
  batch-mode spec is now closed — the same in-place-correction habit the slug
  spec used, so a reader arriving at the older spec's "known deferred item"
  line can find its resolution.

## 8. Approaches considered

**A. A module plus a required `Ctx` argument.** Chosen. One module owns the
rules, `markdown.rs` keeps its shape, and the diff is per-arm and reviewable
against §3's table. Its weakness is that "every text path goes through
`escape`" is enforced by the type of one parameter plus review, not by
construction.

**B. An emitter type owning the buffer and a context stack.** Rendering becomes
`sink.text(t)` / `sink.raw("**")`, with escaping applied at push time, so
omitting it is structurally impossible. Genuinely stronger on exactly A's
weakness — but it rewrites all of `markdown.rs` and most of its tests, and the
churn would bury the policy inside a refactor. P7 covers the failure B prevents
by construction (a forgotten call site changes the recovered text), which is
what makes A's weakness affordable. B remains the natural next refactor if a
third context ever appears.

**C. Escape once at the IR boundary, before rendering.** Rejected on two
independent grounds. `anchor_slug`, `path_slug`, `est_tokens` and `nav`'s titles
all read IR text, so pre-escaping would corrupt anchors, paths and the size
guard — §5's invariant inverted. And escaping is not context-free: the same
string needs a backslash in flow text, an HTML entity in a cell of a merged
table, a percent-encoding in a destination, and a YAML escape in the
frontmatter, so no single pre-pass can be correct for all four.

**D. A shared crate for the anchor rule.** Not taken, here or — as approach
(iii) — by the residuals item (2026-08-13), and recorded so the next anchor
divergence finds the argument rather than re-deriving it.

`anchor_slug` predicts the rendered heading line from IR inlines, and
`escape::one_line` produces that line — two hand-kept mirrors in crates that
cannot depend on each other. Moving both onto a shared crate would end the
mirror and close the whole divergence class at once, including the
footnote-reference case (§4.5) and the trailing-`#` case, both of which are
currently documented as surviving on purpose.

It was not taken because `assign_paths` needs the anchor at structuring time,
before anything is rendered, so the shared model has to be a *prediction* of
the rendered line either way — the crate boundary moves, the prediction
remains. That makes it an architecture change with a real ripple and a
smaller payoff than it first appears, and it deserves its own item rather
than riding along with a fix.

## 9. Verification and risk

**Churn.** Every document whose prose contains an escaped character re-renders
with backslashes, and every file's frontmatter changes because all scalars
become quoted. Paths and cross-link targets are byte-identical (§5, checked
by §6.6); anchors are byte-identical with the one exception §5 and §6.6 both
name — a heading whose inline text contains a newline, where `anchor_slug`'s
fold moves the hyphenation to match what a real parser now computes. Nested
and multi-block list items re-render with continuation indents (§4.3).

**The oracle is a mirror, and that is a named risk.** `pulldown-cmark` is a
CommonMark + GFM implementation, not the `cmark-gfm` GitHub runs, so P7 pins
kasane's output against *a* conforming parser rather than against github.com.
The exposure is much lower than the anchor rule's: backslash escaping is core
CommonMark, specified precisely and stable, where the anchor rule mirrors a
GitHub-specific filter with no specification at all. The rows most worth
distrusting are the ones where GFM extends CommonMark — the `|` cell escape and
`<br>` in cells — and those are the rows a spot check against a real render
should cover if the table ever changes.

**A rule that over-escapes is invisible to P7.** Escaping `!` everywhere, or
`&` unconditionally, would round-trip perfectly and simply produce noisier
output. The case table (§6.1) is the only check on over-escaping, which is why
it pins the negative cases — `Q&A` unescaped, `!` unescaped, a mid-line `#`
unescaped — and not only the positive ones.

**Ship gate.** `mise run lint && mise run test` green, P7 and the widened
generator included; the `escape` fuzz target run locally with
`ASAN_OPTIONS=detect_leaks=0` per AGENTS.md's sandbox note; §6.6's before/after
path comparison performed and recorded.
