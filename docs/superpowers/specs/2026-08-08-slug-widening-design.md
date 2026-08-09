# kasane — Slug Widening Design Spec

**Date:** 2026-08-08
**Status:** Approved (design), pending implementation plan
**Repo:** kasane

## 1. Purpose & scope

README's Known limitations records one open correctness item, and names it as
open work: heading anchors and filenames use kasane's own slug rule, which
keeps ASCII letters and digits only. Two consequences, both live on real books:

- **Punctuation diverges from GFM.** `## Don't Panic` anchors as `#don-t-panic`
  where GitHub computes `#dont-panic`, so an in-book cross-reference to that
  heading resolves inside kasane's own tree and nowhere else.
- **Any purely non-Latin heading collapses to the literal `section`.** A
  two-chapter Japanese book emits `01-section.md` and `02-section.md`, and their
  cross-references point at `#section` while the rendered headings anchor as
  `#第二章`. Filenames lose the title the same way.

This item closes both: anchors become a deliberate mirror of GitHub's algorithm,
and filenames carry the title in any script while staying portable.

### Confirmed, not assumed

`slug()` (`crates/kasane-core/src/paths.rs:81`) is the single function behind
both jobs. It is called from three places, all in `place()`:

| Call site | Purpose |
|---|---|
| `paths.rs:30` | the section node's own anchor, `path#slug(title)` |
| `paths.rs:39` | a body heading's anchor (a subsection `balance` demoted) |
| `paths.rs:46` | the child's file or directory name |

`resolve_refs` (`refs.rs:63-67`) rewrites every `RefTarget::Internal` to the
anchor map's value, and `relativize` (`refs.rs:120`) splits it back on `#`, so
the path half and the fragment half are already handled separately downstream.
Nothing requires the two halves to be computed by the same rule.

Two properties of the existing code shape the design and are worth stating
before the rules, because a reader who does not know them will over-build:

1. **Every non-root path component already carries an `NN-` ordinal prefix**
   (`paths.rs:48,51`). Only `index.md` does not. Windows reserved names
   (`CON`, `NUL`, `COM1`) are therefore structurally impossible — no
   component's stem is ever bare — and sibling collisions are impossible too,
   including the case-insensitive collisions macOS and Windows would produce
   and the NFC-vs-NFD collisions macOS would produce.
2. **Link destinations are emitted raw** (`markdown.rs:146`), with no
   percent-encoding. That is safe today only because slugs are ASCII
   alphanumerics and hyphens. It stays safe under the new rule for a stronger
   reason: see §4.

### Scope

- A new `crates/kasane-core/src/slug.rs` holding two rules (§2).
- Per-file duplicate-anchor suffixing, matching GFM (§3.1).
- A byte cap on path components (§3.2).
- The character-set argument that keeps destinations raw, pinned by a test
  (§4), and the untrusted-text-to-filename argument, pinned by a new `slug`
  fuzz target (§5.3).
- Two new direct dependencies for `kasane-core` (§2.3).
- Documentation of the churn and of the one divergence that survives (§6).

### Non-goals

- **The `insta` snapshot tier.** The remaining unbuilt design-spec §9 tier.
- **The repo-wide Markdown escaping policy.** A known deferred item since the
  batch-mode spec; untouched here.
- **Total path length.** Depth comes from heading nesting plus whatever `-o`
  the user passes, so a deep book in a deep output directory can still exceed
  Windows' default 260-character `MAX_PATH`. Documented in §6, not solved.
- **The empty-id case.** A heading with no Word characters at all diverges
  from GitHub by design; see §3.3.

## 2. Two rules, one module

`slug.rs` replaces `paths.rs`'s single `slug()` with two functions over the
same inline text (`inline_text` moves there unchanged, bound included).

### 2.1 `anchor_slug(&[Inline]) -> String`

> **Corrected 2026-08-09, after the whole-branch review.** This section
> originally specified an NFC step and defined Ruby's `\p{Word}` as "Letter,
> Mark, Number, and Connector_Punctuation". Both were wrong, the plan
> transcribed both faithfully, and the code shipped them. The two errors and
> their consequences are recorded below rather than silently overwritten,
> because this spec is what a future reader will check the mirror against.

A deliberate mirror of GitHub's TOC filter, in its order:

1. Unicode-lowercase.
2. **Remove** — not replace — every character outside `\p{Word}`, `-`, and
   space. Ruby defines `\p{Word}` in `tool/enc-unicode.rb`, matching UTS#18
   Annex C, as `Alphabetic + Mark + Decimal_Number + Connector_Punctuation +
   Join_Control`; GitHub's TOC filter keeps exactly that via
   `/[^\p{Word}\- ]/u`. So `_` survives, combining marks survive, and ZWJ/ZWNJ
   survive.
3. Map each remaining space to `-`.

**There is no normalization step.** GitHub performs none, and neither can
kasane: `nav::walk` sets a file's title from unnormalized `inline_text` and
`file_to_markdown` writes that verbatim as the heading line, so NFC-folding
the fragment while rendering the heading unnormalized produces a link that
resolves in *no* renderer — GitHub, mdBook, or a local preview — whenever the
source text is NFD. NFD input is realistic, not theoretical: macOS-sourced
EPUBs and PDF text extraction produce it routinely. §2.2's `path_slug` keeps
NFC, where it is a genuine benefit and where the `NN-` ordinal prefix already
neutralizes collisions.

**`Number` is not the right group.** Ruby has `Decimal_Number` (`Nd`), not
`Nd + Nl + No`. `Letter_Number` (`Nl`, `Ⅷ`) is still in the set because
`Alphabetic` contains it, but `Other_Number` (`No`) is not: `½` U+00BD and `①`
U+2460 are both `No`, so `## Fig ½` anchors `fig-` and `## ①はじめに` anchors
`はじめに`. Circled numerals are common in Japanese and Chinese headings.

**Join_Control is in the set, and it matters most to this item's audience.**
ZWNJ U+200C is `Cf`, and it appears *inside* ordinary Persian and Urdu words
(`می‌رود`) and in Devanagari. Dropping it mis-anchors every such heading.
`path_slug` still drops it (§2.2): a filename must not carry an invisible
character, and §5.3's confinement argument depends on the path alphabet
staying closed. This is the one place the two rules' character classes differ.

`unicode-properties` (§2.3) exposes General_Category only, so `Alphabetic` is
approximated as the Letter group plus `Nl` — `Alphabetic` is `L* + Nl +
Other_Alphabetic`, and `Other_Alphabetic` is almost entirely `Mn`/`Mc`, which
the Mark group already covers. The residue this misses is the `So` characters
carrying `Other_Alphabetic`, such as the circled Latin letters (`Ⓐ`).

No run-collapsing and no trimming, because GitHub does neither. The visible
consequence is that exact parity means deliberately emitting anchors that look
wrong: `## Background & Notes` anchors as `background--notes`, because the `&`
is removed and each of the two surviving spaces becomes a hyphen.

| Heading | Anchor |
|---|---|
| `Don't Panic` | `dont-panic` |
| `Background & Notes` | `background--notes` |
| `foo_bar` | `foo_bar` |
| `第二章` | `第二章` |
| `हिन्दी` | `हिन्दी` |
| `Fig ½` | `fig-` |
| `①はじめに` | `はじめに` |
| `Part Ⅷ` | `part-ⅷ` |
| `می‌رود` (with ZWNJ) | `می‌رود` (ZWNJ kept) |
| `Cafe` + U+0301 (NFD) | `cafe` + U+0301, *not* `café` |

A title consisting only of Join_Control characters is left to the mirror: the
anchor is the ZWNJ itself, non-empty, so §3.3's fallback does not fire. That
is what GitHub computes, so the link resolves in both places; guarding it
would manufacture a divergence where there is none. `path_slug`, which drops
Join_Control, does fall back to `section` for the same title.

### 2.2 `path_slug(&[Inline]) -> String`

§2.1's character class minus Join_Control, plus NFC, then it diverges where a
filename should: collapse runs of separators to a single `-`, trim leading and
trailing `-`, and cap at a byte budget (§3.2). `Background & Notes` becomes
`background-notes`, not `background--notes`.

The two rules are independent by construction — one lands in the path portion
of a link, the other in the fragment — so nothing forces them to agree, and
nothing breaks when they don't. They diverge on three axes: the tail (here),
Join_Control (§2.1), and NFC (§2.1). NFC belongs to this rule alone because
this rule is choosing a *filename*: the NFD and NFC spellings of one title
should land in one place, and the ordinal prefix already makes any resulting
collision harmless.

### 2.3 Dependencies

`kasane-core` currently depends on `kasane-ir` and nothing else. Matching
`\p{Word}` needs the Mark general categories, which std does not expose:
`char::is_alphanumeric()` is Alphabetic + Numeric, and the Devanagari virama
(U+094D) and similar combining signs are separate marks that NFC does not
compose away, so `हिन्दी` would slug to `हिनदी`.

This item therefore takes two direct dependencies:

- `unicode-normalization` — NFC, for `path_slug` only (§2.1, §2.2).
- `unicode-properties` — `General_Category`, for the Mark classes and for the
  `Nd`/`Nl`/`No` distinction §2.1 turns on.

Both are already in `Cargo.lock` transitively, so this adds direct edges rather
than a new subtree. It does cost `kasane-core` its zero-third-party-dependency
status, which is a deliberate trade for exact parity on Indic and Thai scripts.

## 3. Uniqueness, length, and the fallback

### 3.1 Duplicate suffixing

GitHub uniquifies per rendered page in document order: the first occurrence is
bare, the next gets `-1`, then `-2`. Two headings titled `Notes` in one kasane
file currently produce the same anchor, so one cross-reference silently lands
on the other's heading.

`place()` grows a per-file counter. The order it already walks — the node's own
title first, then body blocks in order — is exactly the order
`file_to_markdown` renders, because that function prepends the title heading.

One wrinkle decides whether parity actually holds. `place()` scans only
*top-level* body blocks for headings, deliberately: a heading nested inside a
list item was never folded into a section, and giving it an anchor would invent
structure the engine does not model. That stays true. But GitHub still assigns
such a heading an id when it renders, so it still **consumes a slot in the
duplicate counter**. Counting only the top-level headings would put our `-1` on
the wrong heading.

So the counter walks the body recursively in render order, while `anchors`
keeps taking only top-level entries. That is a new recursive block walk, which
per repo convention carries `kasane_ir::MAX_BLOCK_DEPTH`. AGENTS.md maintains a
counted inventory of the walks that carry that bound; the count and the prose
there move with this change.

### 3.2 The byte cap

64 bytes per path slug, truncated on a char boundary, then trailing `-` and
trailing combining marks trimmed so truncation cannot leave a dangling hyphen
or a dangling mark.

64 bytes is roughly 64 Latin characters or 21 CJK characters — comfortably a
chapter title — and with the `NN-` prefix and the `.md` suffix a component
stays far inside the 255-byte per-component limit. Truncation can make two
sibling slugs identical; the ordinal prefix already makes that harmless.

Anchors are not capped. They are not filenames, and capping them would break
parity for no benefit.

### 3.3 The fallback, and the divergence that survives

Both rules keep the `section` fallback for a title with no Word characters at
all (`## ***`, `## —`). GitHub gives such a heading an empty id. kasane cannot
emit an empty anchor without producing a dead link, so it diverges here,
including in the suffixed form: kasane emits `section`, `section-1` where
GitHub emits the empty string and `-1`.

This is the one case where README's limitation narrows rather than disappears.

## 4. Why destinations stay raw

`markdown.rs:146` emits `[{}]({})` with the destination unencoded. Under the
new rule the character set of a path component is closed: §2.2's class and
`-`, nothing else. Every character that would break a bare Markdown
destination — space, `(`, `)`, `#`, `?`, `%` — is outside it and is therefore
already removed. Raw stays correct, and stays readable in a way
percent-encoding would not. Note that §2.1's widening of the *anchor* class to
Join_Control does not touch this argument: it is `path_slug` that produces
path components, and Join_Control is exactly the character it does not admit.

`library.rs` percent-encodes for a different reason and keeps doing so: its
relative directories come from the filesystem, so they are arbitrary — spaces,
parentheses, anything the user named a directory. Nothing there changes.

The closed-character-set claim is the load-bearing half of this section, so it
is pinned by a test that asserts the emitted set over the case table rather
than left as an argument in prose.

## 5. Testing

### 5.1 The case table

Parity is verified by a curated table of title → expected-anchor cases derived
from GitHub's documented algorithm, one row per rule, each commented with the
rule it pins: punctuation removed not replaced, `&` leaving a double hyphen,
underscore retained, CJK passthrough, Devanagari marks surviving, an NFD input
anchoring *differently* from its NFC twin while both reach the same filename,
`Other_Number` dropped and `Letter_Number` kept, ZWNJ surviving inside a
Persian word for the anchor and being dropped for the path, emoji stripped, a
run of spaces preserved as a run of hyphens, and the empty-title fallback.

The path table runs the same inputs against the different expectations, plus
the cap: a long CJK title truncating on a char boundary, and a truncation
landing mid-grapheme trimming the dangling mark.

Duplicate suffixing gets its own cases, including the one that matters — a
heading nested inside a list item consuming a counter slot without gaining an
anchor.

This is a mirror, so it carries mirror-drift risk against github.com, the same
class the PDF adapter took on mirroring `lopdf`. Nothing in CI can ask
github.com what it computes. The table is where a future reader learns the
rule, and §8's hand check is the only thing that tests the derivation rather
than this spec's reading of it.

### 5.2 The property tier

`properties.rs:99` recomputes the engine's slug from each rendered heading line
via the `slug_of` seam, then asserts the anchor is in that set. Duplicate
suffixing breaks that: an anchor is no longer a function of one heading's text,
it depends on what preceded it in the file.

`slug_of` is therefore replaced as a seam by one that takes a file's heading
texts **in order** and returns the anchors in order. The property keeps
asserting against the engine's own rule instead of a copy, which is the whole
reason the seam exists.

The generator is widened to draw non-Latin and punctuation-bearing titles, so
the link invariant exercises this rule rather than staying on the ASCII path
README currently describes.

One existing line in that helper inverts under the new rule and would
otherwise produce a false P2 failure. `heading_slugs` strips `*`, `_`, and
`` ` `` from a rendered heading line before slugging it, because
`inlines_to_md` writes those around `Emph`/`Strong`/`Code` while the engine
slugs `inline_text`, which never sees a marker. `_` was in that set defensively
— the writer never emits it — but `_` is a Word character, so the engine now
*keeps* a literal underscore that the helper would strip: a heading `foo_bar`
anchors as `foo_bar` and the helper would compute `foobar`. `_` comes out of
the strip set, `*` and `` ` `` stay, and the generator draws a `foo_bar`-shaped
word so the regression is caught rather than reasoned about.

### 5.3 Fuzzing

This item makes `kasane-core` write filenames derived from untrusted adapter
text, and core is not where the repo's input-distrust convention currently
lives. The convention holds anyway, by construction rather than by a guard:
`/`, `\`, `.`, NUL, the fullwidth solidus (U+FF0F), and the RTL override
(U+202E) are all outside `\p{Word}` and are removed, so traversal and
control-character injection are impossible in the output, and §3.2 bounds the
length.

Repo habit is to pin such an argument with something executable. A **new
`slug` target** asserts the postconditions: the output contains no path
separator, is never a bare `.` or `..`, is never empty, and respects the byte
cap.

It has to be a new target rather than an assertion inside `guards`.
`fuzz_entry` lives in `kasane-adapters`, which depends on `kasane-ir` alone —
it cannot reach `kasane-core`, and giving it a dependency on core would invert
the crate layering for a test seam. So `kasane-core` grows its own
`fuzz_entry.rs`, mirroring the adapters convention and for the same stated
reason (a test seam, not API), and `fuzz/Cargo.toml` takes `kasane-core` as a
second path dependency plus a `[[bin]]` entry.

The stable replay stays in one place. `crates/kasane-adapters/tests/fuzz_corpus.rs`
dispatches every corpus directory by name and **panics on a directory it does
not recognize**, precisely so a renamed target cannot silently stop being
replayed — so `fuzz/seeds/slug/` must be registered there whatever else
happens. That file is a test target, so it takes `kasane-core` as a
`[dev-dependencies]` entry of `kasane-adapters`: acyclic, confined to tests,
and cheaper than standing up a second replay harness that would duplicate the
`KNOWN_OPEN` quarantine machinery.

`TARGET_COUNT` goes 12 → 13, and README's "Twelve targets" with it.

### 5.4 End-to-end

Existing end-to-end content assertions that name paths move with the churn
(§6).

## 6. Documentation

- **README, Known limitations.** The slug bullet is rewritten: anchors now
  match GFM, filenames carry the title in any script, and what remains is the
  empty-id case (§3.3) and total path length (§1, non-goals).
- **README, Output shape.** A second paragraph on churn, alongside the one the
  `Part N` change already added. Every book with punctuation in a heading
  changes paths (`01-don-t-panic.md` → `01-dont-panic.md`), and a non-Latin
  book changes wholesale (`01-section.md` → `01-第二章.md`). `write_tree` swaps
  the directory, so nothing stale survives, but anything outside the tree that
  referenced the old paths needs updating.
- **README, Property tests.** The paragraph reading the link invariant
  precisely currently says the generator draws ASCII titles and that the
  invariant says nothing about GitHub resolving the same anchor. Both clauses
  change (§5.2).
- **README, Fuzzing.** "Twelve targets" becomes thirteen, and the sentence
  listing what the targets cover gains the slug rule.
- **AGENTS.md.** `kasane-core` gains `slug.rs` in the map, with the two-rule
  split and the reason for it; the bounded-walk inventory count and prose move
  with §3.1; the `fuzz_entry.rs` note gains its `kasane-core` counterpart.

## 7. Approaches considered

**A. Two rules in one new `slug.rs` — chosen.** Anchors mirror GFM exactly;
paths optimize for being filenames. Independent by construction, since one
lands in the path portion of a link and the other in the fragment. Costs two
rules to keep in a reader's head, mitigated by them sharing a character class
and a normalization step and differing only in the tail.

**B. One rule for both.** Simplest to explain and impossible to drift apart.
Rejected: it forces a choice between filenames carrying GFM's artifacts
(`01-background--notes.md`, `01-foo_bar.md`, no length cap) and anchors that do
not resolve on GitHub. The whole item is that those two consumers want
different things.

**C. Mirror GFM in the writer instead of core.** Puts the anchor rule next to
the Markdown it must agree with. Rejected: `assign_paths` computes anchors in
core and `resolve_refs` consumes them there, so the writer would be recomputing
a value core has already committed to — the drift risk moves inside kasane
instead of being eliminated.

**D. Cross-check parity against a GFM crate (comrak) in CI.** Would catch a
class of drift the table cannot. Rejected as the verification mechanism:
comrak's rule is itself an approximation of github.com's TOC filter, so a
mismatch would not say which side is wrong, and it adds a pinned dependency to
answer a question §8's hand check answers directly.

**E. Unicode with ASCII transliteration fallback.** Prefer the Unicode slug,
transliterate when it yields nothing. Rejected: it adds a transliteration
dependency and a second code path for a case (§3.3) that is rare and better
served by an honest documented divergence.

## 8. Verification

`mise run lint && mise run test` covers the case tables, the duplicate-suffix
cases, the widened property tier, and the updated end-to-end assertions.

Beyond that:

- `mise run fuzz guards` on the pinned nightly, for §5.3's assertions.
- A converted real EPUB with non-Latin headings, rendered and inspected by
  hand. The case table is derived from GitHub's documented algorithm, so one
  human spot-check against an actual GitHub render is the only thing that tests
  the derivation itself rather than this spec's reading of it.
