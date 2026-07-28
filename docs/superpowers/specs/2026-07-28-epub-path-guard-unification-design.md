# kasane — EPUB Path-Guard Unification Design Spec

**Date:** 2026-07-28
**Status:** Approved (design), pending implementation plan
**Repo:** kasane
**Parent spec:** `2026-07-28-pdf-outline-and-guard-hardening-design.md` (§7, "Follow-up work recorded, not done")

## 1. Purpose & scope

The outline-and-guard-hardening item closed #21 and #22 and recorded one gap it
deliberately did not close (that spec's §7): `safe_entry_name("")` returns
`Some("")`, and the `guards` fuzz target asserts `!name.is_empty()` for
`resolve_rel`'s output but not for `safe_entry_name`'s. Issue #22 predicted that
adding the missing assertion might surface a third finding.

This item is that follow-up. Tracing the gap to its call sites turned up two
further defects of the same class — both of them fidelity bugs rather than
security ones — and showed that the cleanest closure is to delete
`safe_entry_name` rather than harden it.

### The three defects

1. **`safe_entry_name("") == Some("")`** — the recorded §7 gap. An unenforced
   postcondition, not a live exploit: both production call sites feed the
   result straight to `zip::by_name`, which fails on `""`, so the EPUB adapter
   either errors on the OPF or `continue`s past a spine item.

2. **`..` is matched as a substring, not as a path segment.** `safe_entry_name`
   rejects any name *containing* `..`, so a chapter legitimately named
   `..foo.xhtml` is rejected and silently dropped from the book.

3. **OPF manifest hrefs are joined by string concatenation, never normalized.**
   `epub/opf.rs:124`'s `join_href` concatenates `opf_dir` and `href`, so a legal
   manifest entry `href="../shared/ch.xhtml"` under `OEBPS/` becomes
   `OEBPS/../shared/ch.xhtml`, trips defect 2's `..` rejection, and the chapter
   is silently dropped — even though `shared/ch.xhtml` really is in the archive.
   This is the most common real-world shape of the bug class and the largest of
   the three in impact.

### Boundary

`crates/kasane-adapters/src/guard.rs`, `epub/opf.rs`, `epub/mod.rs`,
`fuzz_entry.rs`, and the `fuzz_corpus.rs` quarantine comment. No IR, core,
writer, or CLI change. No other adapter is touched: `safe_entry_name` has no
callers outside the EPUB adapter.

### Non-goals

- **Changing `resolve_rel`.** It already has the contract this item wants
  (segment-wise `..`, confinement to the archive root, `None` on empty). It is
  the survivor, not the subject.
- **Making `resolve_rel` public API.** `mod guard` stays private to
  `kasane-adapters`.
- **A new fuzz target.** The `guards` target already covers the surviving
  primitive.
- **PPTX / MOBI / PDF / DjVu path handling.** PPTX already routes through
  `resolve_rel` (`pptx/mod.rs:33,65`, `pptx/rels.rs:133`); the others do not
  read container-relative paths.

## 2. Why deletion rather than hardening

`mod guard` is private to the crate (`lib.rs:6`), and `safe_entry_name` has
exactly two production call sites, both in `epub/mod.rs`:

- **line 29** — the container.xml rootfile `full-path`, which is
  archive-root-relative.
- **line 50** — each spine href, already joined against `opf_dir` by
  `join_href`.

Fixing defect 3 means routing the second through `resolve_rel`. Fixing defect 1
and 2 on the first is exactly `resolve_rel("", name)`. Move both and
`safe_entry_name` has no callers left outside the fuzz seam, so hardening it
would mean maintaining a second confinement primitive that nothing calls — and
the divergence between two overlapping guards is what produced this item in the
first place.

The assertion issue #22 predicted therefore never gets added: it already exists,
on `resolve_rel` (`fuzz_entry.rs:163-166`), and after this change that is the
only primitive left for it to cover. The predicted third finding cannot occur,
because the function that would have produced it is gone.

## 3. The change

### 3.1 `guard.rs`

Delete `safe_entry_name` and its `rejects_traversal_names` unit test.
`resolve_rel`, `check_expansion`, `has_scheme`, `parent_dir`, and
`safe_media_filename` are untouched.

### 3.2 `epub/opf.rs`

`join_href` (line 124) is deleted. Its single caller, line 75's
`manifest.insert(id, join_href(opf_dir, &href))`, becomes a guarded insert:

```rust
if let Some(path) = resolve_rel(opf_dir, &href) {
    manifest.insert(id, path);
}
```

A manifest item whose href does not resolve is therefore dropped at parse time
rather than admitted into `manifest` and dropped later at the zip read.
Resolution happens once, where the directory context lives. `join_href` has no
unit test of its own, so nothing is removed alongside it.

`spine_hrefs` is the only path-bearing field on `Opf`, so this is the only
consumer to adjust.

### 3.3 `epub/mod.rs`

Line 29's `safe_entry_name(&opf_path)` becomes `resolve_rel("", &opf_path)`,
keeping the same `ParseError::Malformed("unsafe rootfile path")` on `None`.

Line 50's `let Some(name) = safe_entry_name(href) else { continue };` is
removed; `href` arrives pre-resolved from §3.2 and is used directly as the zip
key. The `use crate::guard::safe_entry_name;` import at line 4 goes with it.

### 3.4 `fuzz_entry.rs`

The `safe_entry_name` block (lines 145-152) is deleted along with the function,
and `safe_entry_name` leaves the `use` at line 15. The `resolve_rel` block
(lines 154-167) is unchanged — it already asserts non-empty, non-absolute, and
segment-wise no-`..`.

## 4. Behavior delta

Confinement is unchanged. `../etc/passwd`, `OEBPS/../../etc/passwd`, and every
other escaping shape still resolve to `None` and are still rejected.

Four inputs change, all toward correctness:

| input | before | after |
|---|---|---|
| `""` or `"."` | accepted, zip lookup then fails | rejected up front (the §7 gap) |
| `OEBPS` + `..foo.xhtml` | chapter silently dropped | chapter read |
| `OEBPS` + `../shared/ch.xhtml` | chapter silently dropped | resolves to `shared/ch.xhtml` |
| `/OEBPS/x` | rejected | root-relative `OEBPS/x`, still confined |

### The one regression risk

`resolve_rel` normalizes, where `safe_entry_name` passed the name through
unmodified. An archive containing a *literal* zip entry named
`OEBPS/./content.opf` would stop resolving, because the lookup key is now the
normalized `OEBPS/content.opf`. The inverse case — an href written
`./content.opf` against an entry stored plainly — is far more common and starts
working where it previously failed. Net positive, but it is a real behavior
change and is recorded here rather than left to be discovered.

## 5. Tests

Four layers, each pinned to a specific defect so a regression names itself.

### 5.1 `guard.rs` units

Add the §7 gap as explicit regression cases on the surviving primitive:

```rust
assert_eq!(resolve_rel("", ""), None);
assert_eq!(resolve_rel("", "."), None);
```

These are the assertions that were missing on the deleted function. Putting them
on `resolve_rel` is what actually closes #22's prediction.

### 5.2 `epub/opf.rs` units

Manifest href resolution directly, against `opf_dir = "OEBPS"`:

- `..foo.xhtml` → `OEBPS/..foo.xhtml` (defect 2)
- `../shared/ch.xhtml` → `shared/ch.xhtml` (defect 3)
- `../../etc/passwd` → item dropped from the manifest
- `./ch1.xhtml` → `OEBPS/ch1.xhtml` (normalization)

The cheapest place to prove the fidelity fixes.

### 5.3 `epub/mod.rs` end to end

Following the pattern the parent item established — drive the seam, do not only
unit-test the guard. A `build_epub` variant whose spine carries:

- a chapter at `..foo.xhtml`,
- a chapter whose href is `../shared/ch.xhtml`, stored in the archive at
  `shared/ch.xhtml`,
- a chapter whose href escapes the root.

Assert the first two chapters' text reaches `doc.nodes`, and that the third is
absent without erroring the parse. Today the first two are silently missing, so
this is the test that goes red first.

### 5.4 Fuzz corpus

Add `fuzz/seeds/guards/empty.bin` — a lone NUL byte, so `split2` yields an empty
base and an empty target — which makes the empty-name shape replayed coverage on
stable via `fuzz_corpus.rs` rather than something only the nightly fuzzer
reaches.

**No new artifact is committed, and that is not a skipped step.** The repo
convention is that a crash the fuzzer finds gets its reproducer committed. Issue
#22 predicted a third finding *conditional on* adding the missing assertion to
`safe_entry_name`. Deleting the function makes that crash unreachable, so there
is no crash to reproduce. `KNOWN_OPEN` stays empty.

## 6. Documentation

Four references go stale with the deletion; all ship in the same branch:

- **`README.md:61`** — "that `safe_entry_name` / `resolve_rel` never emit a
  traversal" names a function that will not exist. Becomes `resolve_rel` alone.
- **`fuzz_entry.rs:137`** — "Three pure functions whose postconditions are
  security-critical" becomes two.
- **`fuzz_entry.rs:214`** — `safe_entry_name` in the `build_zip` rationale
  becomes `resolve_rel`.
- **`fuzz_corpus.rs:50-54`** — the comment still claims `guards` crashes
  "deterministically around run #3" while `KNOWN_OPEN` (line 55) is empty. That
  is drift from the parent item, which fixed the crash and un-quarantined it.
  Correct the comment to describe the quarantine mechanism without asserting a
  live crash.

`AGENTS.md` needs no change: it refers to `guard::*` generically.

The README's "Known limitations" list also needs no change. This item removes a
silent failure without introducing a documented one — chapters outside the OPF
directory simply convert now, where before they vanished — so there is no new
limitation to describe and no existing entry that becomes wrong.

## 7. Approaches considered

**A. Delete `safe_entry_name`, route both call sites through `resolve_rel` —
chosen.** One confinement primitive in the crate. All three defects fall out of
a single deletion, and the drift surface that produced the item is removed
rather than re-created. The assertion #22 asked for already exists on the
survivor.

**B. Keep `safe_entry_name` for the container.xml rootfile, move only the spine
path.** Preserves a nominally distinct "raw archive-root name" contract for the
one path that genuinely is root-relative. Rejected: `resolve_rel("", name)` *is*
that contract, so the distinction buys a function with one caller and a
hand-maintained mirror of the same rules — the exact configuration that let the
two guards drift apart.

**C. Harden `safe_entry_name` in place: reject empty, switch `..` to a
segment-wise check.** The original scoping of this item, before the call-site
trace. Rejected once it became clear both production callers were moving to
`resolve_rel`: identical behavior at every call site, but the crate keeps two
overlapping guards. It also does not fix defect 3, which needs normalization
rather than a tolerated `..`.

**D. Make `safe_entry_name` segment-wise while splitting on `\` as well as
`/`.** Considered as a refinement of C, to keep `..\..\etc` rejected once the
substring check was gone. Moot under A: `resolve_rel` splits on `/` only, and
its output is used solely as a zip lookup key, never joined onto a filesystem
path, so a backslash-bearing name is opaque and fails the lookup harmlessly.

## 8. Verification

`mise run lint && mise run test` is necessary but not sufficient: it proves the
unit and end-to-end layers and the stable corpus replay. The OPF resolution path
sits inside the untrusted-input boundary, so the rest runs on the pinned
nightly:

- `mise run fuzz guards -- -max_total_time=60` — must stay clean. It crashed
  deterministically around run #3 before the parent item's `resolve_rel` fix
  landed.
- `mise run fuzz epub -- -max_total_time=60` and
  `mise run fuzz epub_zip -- -max_total_time=60` — the two targets that actually
  reach the changed OPF path.

The specific proof this item owes, beyond green CI, is §5.3: the two chapters
that are silently missing from a converted EPUB today must appear in the output
afterwards.
