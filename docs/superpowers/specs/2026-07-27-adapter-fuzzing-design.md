# kasane — Adapter Fuzzing Design Spec

**Date:** 2026-07-27
**Status:** Approved (design), pending implementation plan
**Repo:** kasane

## 1. Purpose & scope

Every adapter in kasane parses untrusted input. AGENTS.md states the rule
plainly — "Adapters must never trust input: guard decompression ratio/size and
entry-name traversal" — and the original design spec (§4, §9) backed it with two
commitments: hand-written guards at the boundary, and `cargo-fuzz` run against
those adapters, where "the security boundary must never panic or hang."

The guards were built. The fuzzing was not. Six formats, a hand-rolled PalmDOC
decompressor, a streaming XML island capture written specifically to stop a
stack overflow, and a set of path-traversal guards have shipped with no
adversarial testing of any kind. This item builds that tier.

The goal is not "add a fuzzer." It is to convert four claims the codebase
currently makes in prose into claims a machine checks:

1. No input makes an adapter panic or abort.
2. No input makes an adapter hang.
3. No input defeats the decompression-bomb guards (`MAX_TOTAL_BYTES`,
   `MAX_RATIO`).
4. No input produces an asset path that escapes its output directory.

### Boundary

The work is confined to a new top-level `fuzz/` crate, one new module in
`crates/kasane-adapters/` (`fuzz_entry.rs`), one new integration test in that
same crate, one new CI workflow, and additions to `mise.toml`,
`.github/dependabot.yml`, `.gitignore`, README, and AGENTS.md. No existing
adapter, IR type, core pass, or writer behavior changes. If fuzzing finds bugs,
fixing them is follow-up work, not part of this item.

### Non-goals

- **The `ocr` feature.** It links Tesseract and Leptonica; fuzzing across FFI
  into C is a separate exercise with its own sanitizer requirements. Targets
  build on the default, pure-Rust feature set only. This is a deliberate,
  documented gap (§8).
- **`proptest` for the structuring engine.** Design spec §9's property tier
  covers `kasane-core` invariants (no block loss or duplication, link
  resolution, prev/next chain). It is independently shippable and stays out of
  scope; this item touches only the untrusted-input boundary.
- **`insta` snapshot tests.** The third unbuilt §9 tier. Out of scope.
- **Fixing what the fuzzer finds.** Each crash gets its reproducer committed
  (§5) and a fix lands as its own change.

## 2. Why the entry point lives inside `kasane-adapters`

`cargo-fuzz` requires `fuzz/` to be its own cargo workspace, excluded from the
root one. That creates two problems this design has to solve together.

**Visibility.** The highest-value targets are not public. `math::capture_island`,
`math::wrap_island`, and `mobi::palmdoc::decompress` are all `pub(crate)`. An
external `fuzz/` crate cannot call them, and widening them to `pub` to satisfy a
test would leak internals into the library's real API.

**Toolchain split.** Fuzz targets build only on nightly (libFuzzer plus
sanitizer flags), but PR CI runs on the pinned stable toolchain and must be able
to replay crash reproducers as ordinary regression tests. The same code has to
be callable from both sides.

One seam solves both: a `#[doc(hidden)] pub mod fuzz_entry` **inside**
`kasane-adapters`. It reaches `pub(crate)` items because it is in the crate, and
it exposes one byte-in function per target:

```rust
// crates/kasane-adapters/src/fuzz_entry.rs
#[doc(hidden)]
pub fn epub(data: &[u8]) { /* parse; assert asset-path containment */ }
#[doc(hidden)]
pub fn math_island(data: &[u8]) { /* capture_island -> mathml + omml */ }
#[doc(hidden)]
pub fn palmdoc(data: &[u8]) { /* decompress */ }
// ...one per target
```

Each function takes bytes, exercises the code under test, discards `Result`s,
and asserts only the invariants in §4. Each `fuzz/fuzz_targets/*.rs` is then a
three-line libfuzzer wrapper, and the stable replay test (§5) calls the
identical function. One body, one source of truth, and the nightly/stable split
stops mattering.

The alternative — duplicating one-line bodies between the fuzz targets and the
replay test — was rejected: it cannot express the `pub(crate)` targets at all,
and the two copies drift.

The `#[doc(hidden)]` marker and a note in AGENTS.md make it explicit that this
module is a test seam, not API.

## 3. Layout

```
fuzz/
  Cargo.toml              # own workspace; root Cargo.toml gains exclude = ["fuzz"]
  fuzz_targets/*.rs       # 12 three-line wrappers over kasane_adapters::fuzz_entry
  seeds/<target>/         # COMMITTED, tiny: hand-written starting inputs
  corpus/<target>/        # gitignored; seeded from tests/fixtures at run time
  artifacts/<target>/     # COMMITTED crash reproducers - the regression corpus
crates/kasane-adapters/
  src/fuzz_entry.rs       # the shared bodies
  tests/fuzz_corpus.rs    # stable replay over fuzz/seeds/** and fuzz/artifacts/**
.github/workflows/fuzz.yml
```

`.gitignore` gains `/fuzz/corpus` and `/fuzz/target`. `fuzz/artifacts` and
`fuzz/seeds` are explicitly **not** ignored.

## 4. The targets

Twelve targets in four tiers.

| Tier | Target | Entry point |
|---|---|---|
| Per-format | `epub` | `EpubAdapter::parse(data, "fuzz.epub")` |
| | `pptx` | `PptxAdapter::parse(data, "fuzz.pptx")` |
| | `mobi` | `MobiAdapter::parse(data, "fuzz.mobi")` — covers MOBI and AZW3/KF8 |
| | `pdf` | `PdfAdapter::parse(data, "fuzz.pdf")` |
| | `djvu` | `DjvuAdapter::parse(data, "fuzz.djvu")` |
| Structured ZIP | `epub_zip` | builder (§4.1) → `EpubAdapter::parse` |
| | `pptx_zip` | builder (§4.1) → `PptxAdapter::parse` |
| Detection | `detect` | `detect(bytes, ext_hint)`, hint from `Unstructured` |
| Sub-parser | `math_island` | `capture_island` → `mathml_to_latex` + `omml_to_latex` |
| | `palmdoc` | `mobi::palmdoc::decompress` |
| | `guards` | `safe_entry_name`, `resolve_rel`, `check_expansion` |
| | `xmltext` | `resolve_general_ref` |

`math_island` is the highest-value target in the set. `capture_island` exists
for one reason: roxmltree recurses per element level and has no nesting guard,
so an over-deep island would abort the process on a stack overflow. A stack
overflow is an abort, not an unwind — no amount of `Result` plumbing recovers
from it, and no unit test currently probes the bound. That is precisely the
failure mode a fuzzer is for.

`palmdoc::decompress` is a hand-rolled LZ77-style decoder reading
attacker-controlled back-references. `guards` covers three pure functions whose
postconditions are security-critical and currently unasserted.

### 4.1 The structured ZIP builder

EPUB and PPTX are ZIP containers and the `zip` crate verifies CRCs on read. Raw
byte mutation therefore bounces off the CRC check and rarely reaches the XML
parsers underneath — a raw-bytes-only `epub` target would spend most of its
budget fuzzing ZIP framing.

`epub_zip` and `pptx_zip` fix this with an `Arbitrary` implementation in
`fuzz_entry` that assembles a well-formed archive — correct local headers,
central directory, and CRCs — from fuzzer-controlled entry **names**, sizes, and
member **contents**. Mutation then lands where the interesting code is:
`safe_entry_name`, `resolve_rel`, the bomb guards, the OPF and XHTML parsers,
and the math island capture.

The raw `epub` and `pptx` targets stay alongside them. The builder tests what
happens past the container; the raw targets keep testing the container itself,
where a real attacker's file has whatever CRC it likes.

### 4.2 What the targets assert

Panics and aborts are caught by libFuzzer for free. The rest are explicit,
because each one turns an existing prose claim into a checked one:

- **Hangs** — `-timeout=25` flags any single input that runs long.
- **Memory** — `-rss_limit_mb=2048` and `-malloc_limit_mb=2048`. This is what
  actually tests the bomb guards: if `MAX_TOTAL_BYTES` or `MAX_RATIO` fails to
  hold, RSS blows and libFuzzer reports an OOM against a specific input. Those
  two constants are currently asserted by nothing.
- **Path containment** — on every adapter target, each `AssetRef` in the
  returned `AssetBag` must be a relative path with no `..` component and no root
  prefix. AGENTS.md states this as a hard convention; nothing checks it today.
- **Guard postconditions** — on the `guards` target: when `safe_entry_name` or
  `resolve_rel` returns `Some`, the result never contains a `..` component,
  never starts with `/`, and never carries a drive prefix. `check_expansion`
  never returns `true` for a ratio past `MAX_RATIO`.

Determinism checks (parse twice, compare) are deliberately omitted — they halve
throughput for a property no reported bug suggests is at risk.

## 5. Corpora and the stable replay test

Three directories with three different jobs:

- **`fuzz/seeds/<target>/`** — committed, deliberately small. Hand-written
  starting inputs for targets with no natural fixture: a few MathML and OMML
  islands, some hostile archive entry names, a short PalmDOC record. It does not
  duplicate `tests/fixtures`.
- **`fuzz/corpus/<target>/`** — generated, gitignored. The `mise run fuzz` task
  populates it before each run from two sources, matched **per target**:
  `fuzz/seeds/<target>/` always, plus the format's own files under
  `tests/fixtures/` for the five per-format and two ZIP targets only
  (`tests/fixtures/epub/*.epub` seeds `epub` and `epub_zip`, and so on). The
  `detect` target gets every fixture. The four sub-parser targets take seeds
  only — an EPUB archive is not a useful starting input for `palmdoc`. Corpora
  grow large and are machine-specific; committing them would bloat the repo for
  no gain.
- **`fuzz/artifacts/<target>/`** — committed. `cargo-fuzz` writes a reproducer
  here for every crash it finds. **Committing that reproducer is mandatory**, and
  the rule is recorded in AGENTS.md so it outlives this change.

`crates/kasane-adapters/tests/fuzz_corpus.rs` walks both `fuzz/seeds/**` and
`fuzz/artifacts/**`, maps each directory name to its `fuzz_entry` function, and
runs every file through it. It is pure stable Rust, runs inside `mise run test`
and therefore on every PR, and needs no nightly toolchain. Every crash the
fuzzer has ever found stays fixed.

Replaying **seeds** as well as crashes matters on day one. With an empty
`artifacts/` the test would otherwise pass vacuously and `fuzz_entry` would be
dead code until the first find; seeding it means the entry functions are
exercised by the normal suite immediately.

An unrecognized directory name under `artifacts/` or `seeds/` is a test
**failure**, not a skip — otherwise a renamed target silently stops being
replayed.

## 6. Toolchain and tasks

`mise.toml` gains a pinned nightly toolchain and `cargo-fuzz` (0.13.2, current
as of this spec):

```toml
"cargo:cargo-fuzz" = "0.13.2"
```

`cargo-fuzz` itself installs on stable; only the targets it builds need
nightly.

**Open implementation detail — resolve first, it decides the task bodies.**
mise's rust backend advertises a floating `nightly` and does not list dated
nightlies in `mise ls-remote rust`. A dated pin is preferred, to match how
everything else in this repo is pinned. `mise ls-remote` lists releases, and the
backend may still pass a dated spec through to rustup, so this needs a direct
check during implementation:

- **If a dated spec resolves** — pin it (`nightly-2026-07-01` or later) and the
  toolchain story stays single-sourced in `mise.toml`, as intended.
- **If it does not** — pin the floating `nightly` in `mise.toml` with a comment
  in the same spirit as the existing Dependabot note. A floating pin is real
  drift and gets written down, not buried. It is also the only pin in this repo
  that can change under CI without a commit, which is worth saying out loud in
  that comment.

Either way the toolchain is named in exactly one place and the tasks below
reference it rather than hardcoding a channel.

New tasks:

- `mise run fuzz <target>` — seed the corpus from `tests/fixtures` and
  `fuzz/seeds`, then `cargo +<pinned-nightly> fuzz run <target>` with the §4.2
  limits.
- `mise run fuzz-all` — every target for a fixed `-max_total_time` budget.

## 7. CI

A new `.github/workflows/fuzz.yml`, modeled on the existing `audit.yml`:

- **Triggers:** `workflow_dispatch` (with optional target and duration inputs)
  plus a weekly cron, offset from `audit.yml`'s Monday 07:00 UTC so the two jobs
  do not contend.
- **Shape:** a matrix over the twelve targets, five minutes each — five minutes
  of wall clock rather than an hour.
- **Failure:** a crash uploads `fuzz/artifacts/` as a workflow artifact and
  fails the job, so the reproducer can be committed.
- **Permissions:** `contents: read`, matching the other workflows.
- Actions are pinned by commit SHA with a trailing version comment, matching the
  existing workflows so Dependabot can read them.

PR CI (`ci.yml`) is untouched. It gains fuzz coverage only through the stable
replay test inside `mise run test`.

## 8. Gaps this creates

Written down here rather than discovered later, following the precedent set by
the `mise.toml` Dependabot note:

- **`cargo deny` does not scan `fuzz/`.** It is an excluded workspace with its
  own `Cargo.lock`, so `audit.yml`'s `cargo deny check advisories` skips it.
  Mitigated by adding a third entry to `.github/dependabot.yml`
  (`package-ecosystem: cargo`, `directory: /fuzz`) so those dependencies still
  get update PRs. They are dev-only and never ship in the released binary.
- **The nightly pin is a manual bump.** Same class as the existing `rust` and
  `cargo-deny` pins — Dependabot cannot read `mise.toml`.
- **The `ocr` feature is unfuzzed** (§1 non-goals).
- **`kasane-core` and `kasane-writer` are unfuzzed.** They consume IR built by
  adapters, not raw bytes; the `proptest` tier is the right tool and is separate
  work.

## 9. Documentation

- **README** — a "Fuzzing" subsection under Development: what the targets cover,
  how to run one, and that OCR is excluded.
- **AGENTS.md** — the workflows line gains `mise run fuzz`; the codebase map
  gains a sentence on the `fuzz_entry` seam; the conventions list gains the rule
  that a crash reproducer is committed to `fuzz/artifacts/`.

## 10. Verification

A fuzz suite that has never failed is indistinguishable from one that is not
wired up. Implementation is not complete until:

1. Every one of the twelve targets has been run briefly and confirmed to execute
   and accumulate coverage — not merely to compile.
2. The replay test has been shown to **fail**: hand it a deliberately crashing
   input, watch `mise run test` go red, then remove it.
3. `mise run lint && mise run test` is green, and `mise run lint-ocr &&
   mise run test-ocr` is green — the `fuzz_entry` module compiles under both
   feature sets.
4. The `fuzz.yml` workflow has been triggered once via `workflow_dispatch` and
   observed to pass.
