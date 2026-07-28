# EPUB Path-Guard Unification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete `guard::safe_entry_name`, routing both of its EPUB call sites through `guard::resolve_rel`, which closes three defects at once: `safe_entry_name("")` returning `Some("")`, a substring `..` check that silently drops a chapter named `..foo.xhtml`, and unnormalized OPF manifest hrefs that silently drop a chapter at `../shared/ch.xhtml`.

**Architecture:** `kasane-adapters` currently has two overlapping path-confinement primitives. `safe_entry_name` is a substring blacklist that returns its input unchanged; `resolve_rel` is a segment-wise normalizer that confines to the archive root. `resolve_rel` already has the contract we want, and `safe_entry_name` has exactly two production callers, both in the EPUB adapter. Moving both onto `resolve_rel` leaves `safe_entry_name` with no callers, so it is deleted rather than hardened — removing the drift surface that produced these defects.

**Tech Stack:** Rust (pinned stable via mise), `quick-xml`, `zip`, `cargo-fuzz` on pinned nightly-2026-07-01.

**Spec:** `docs/superpowers/specs/2026-07-28-epub-path-guard-unification-design.md`

## Global Constraints

- Every change ships green under `mise run lint && mise run test`. `mise run lint` is fmt check + `clippy --all-targets -D warnings` — plain `cargo clippy` is not sufficient.
- `mod guard` stays private to `kasane-adapters` (`lib.rs:6`). Do not make it `pub mod`.
- Do not change `resolve_rel`'s implementation. It is the survivor, not the subject.
- Confinement is non-negotiable: `../etc/passwd`, `OEBPS/../../etc/passwd`, and every other escaping shape must still resolve to `None` after every task.
- A crash the fuzzer finds gets its reproducer committed to `fuzz/artifacts/<target>/`. This plan expects no new crash (see Task 3), but if one appears, commit it.

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `crates/kasane-adapters/src/guard.rs` | Path confinement primitives | Delete `safe_entry_name` + its unit test; add two `resolve_rel` regression cases |
| `crates/kasane-adapters/src/epub/opf.rs` | OPF package-document parsing | Delete `join_href`; resolve manifest hrefs via `resolve_rel` at insert time |
| `crates/kasane-adapters/src/epub/mod.rs` | EPUB adapter entry point | Route the container.xml rootfile through `resolve_rel`; drop the spine-href guard |
| `crates/kasane-adapters/src/fuzz_entry.rs` | Fuzz seam (`pub(crate)` reach-through) | Delete the `safe_entry_name` assertion block; fix two doc comments |
| `crates/kasane-adapters/tests/fuzz_corpus.rs` | Stable replay of seeds + artifacts | Correct the stale `KNOWN_OPEN` comment |
| `fuzz/seeds/guards/empty.bin` | Fuzz seed | Create — locks the empty-name shape into stable replay |
| `README.md` | Front door | Fix the fuzzing paragraph naming `safe_entry_name` |

---

### Task 1: Resolve OPF manifest hrefs through `resolve_rel`

Closes defect 3: a manifest `href="../shared/ch.xhtml"` under `OEBPS/` is concatenated into `OEBPS/../shared/ch.xhtml`, which the later guard rejects, so the chapter silently vanishes from the book.

**Files:**
- Modify: `crates/kasane-adapters/src/epub/opf.rs:1-2` (imports), `:75` (manifest insert), `:124-130` (delete `join_href`)
- Test: `crates/kasane-adapters/src/epub/opf.rs` (the `mod tests` block at the bottom of the same file)

**Interfaces:**
- Consumes: `crate::guard::resolve_rel(base_dir: &str, target: &str) -> Option<String>` — already exists, unchanged.
- Produces: `Opf.spine_hrefs: Vec<String>` now holds **normalized, root-confined** archive paths. Task 2 relies on this: every entry is usable directly as a `zip::by_name` key with no further guarding.

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `crates/kasane-adapters/src/epub/opf.rs`:

```rust
#[test]
fn manifest_hrefs_are_normalized_and_confined() {
    let xml = r#"<package><metadata><dc:title>T</dc:title></metadata>
      <manifest>
        <item id="a" href="../shared/ch.xhtml"/>
        <item id="b" href="..foo.xhtml"/>
        <item id="c" href="./ch1.xhtml"/>
        <item id="d" href="../../etc/passwd"/>
      </manifest>
      <spine>
        <itemref idref="a"/><itemref idref="b"/>
        <itemref idref="c"/><itemref idref="d"/>
      </spine></package>"#;
    let opf = parse_opf(xml, "OEBPS");
    assert_eq!(
        opf.spine_hrefs,
        vec![
            // `..` pops OEBPS instead of surviving into the key.
            "shared/ch.xhtml".to_string(),
            // `..foo` is a filename, not a traversal: the segment is not `..`.
            "OEBPS/..foo.xhtml".to_string(),
            // `.` segments are dropped.
            "OEBPS/ch1.xhtml".to_string(),
            // `d` escapes the archive root, so it never enters the manifest.
        ]
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p kasane-adapters manifest_hrefs_are_normalized_and_confined`

Expected: FAIL. `join_href` concatenates, so the actual vector is
`["OEBPS/../shared/ch.xhtml", "OEBPS/..foo.xhtml", "OEBPS/./ch1.xhtml", "OEBPS/../../etc/passwd"]` — four entries where three are expected, and three of the four strings differ.

- [ ] **Step 3: Import `resolve_rel`**

At the top of `crates/kasane-adapters/src/epub/opf.rs`, after the two existing `quick_xml` imports:

```rust
use crate::guard::resolve_rel;
use quick_xml::events::Event;
use quick_xml::Reader;
```

- [ ] **Step 4: Resolve at manifest-insert time**

Replace line 75's body. Before:

```rust
                        if !id.is_empty() {
                            manifest.insert(id, join_href(opf_dir, &href));
                        }
```

After:

```rust
                        // Resolve once, here, where opf_dir is in scope: an href
                        // that escapes the archive root never enters the manifest,
                        // so no later stage has to re-guard it.
                        if !id.is_empty() {
                            if let Some(path) = resolve_rel(opf_dir, &href) {
                                manifest.insert(id, path);
                            }
                        }
```

- [ ] **Step 5: Delete `join_href`**

Remove lines 124-130 of `crates/kasane-adapters/src/epub/opf.rs` entirely:

```rust
fn join_href(dir: &str, href: &str) -> String {
    if dir.is_empty() {
        href.to_string()
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), href)
    }
}
```

It has no unit test of its own and no other caller, so nothing else is removed alongside it.

- [ ] **Step 6: Run the new test to verify it passes**

Run: `cargo test -p kasane-adapters manifest_hrefs_are_normalized_and_confined`

Expected: PASS.

- [ ] **Step 7: Run the full suite**

Run: `mise run test`

Expected: PASS. In particular `opf.rs`'s existing `spine_hrefs == vec!["OEBPS/ch1.xhtml"]` assertion still holds — `resolve_rel("OEBPS", "ch1.xhtml")` is `Some("OEBPS/ch1.xhtml")`, identical to what `join_href` produced.

- [ ] **Step 8: Lint**

Run: `mise run lint`

Expected: PASS. If clippy flags the nested `if` from Step 4 as `collapsible_if`, collapse it:

```rust
                        if !id.is_empty() {
                            if let Some(path) = resolve_rel(opf_dir, &href) {
                                manifest.insert(id, path);
                            }
                        }
```
becomes
```rust
                        if let (false, Some(path)) =
                            (id.is_empty(), resolve_rel(opf_dir, &href))
                        {
                            manifest.insert(id, path);
                        }
```

- [ ] **Step 9: Commit**

```bash
git add crates/kasane-adapters/src/epub/opf.rs
git commit -m "fix(epub): resolve OPF manifest hrefs instead of concatenating them

join_href concatenated opf_dir and href without normalizing, so a legal
manifest entry href=\"../shared/ch.xhtml\" under OEBPS/ became
OEBPS/../shared/ch.xhtml, tripped the later `..` guard, and the chapter
was silently dropped from the book. resolve_rel normalizes and confines
in one step, and an escaping href now never enters the manifest at all."
```

---

### Task 2: Delete `safe_entry_name`

Closes defect 1 (`safe_entry_name("") == Some("")`, the gap recorded in the parent spec's §7) and defect 2 (substring `..` matching drops a chapter named `..foo.xhtml`). After Task 1 the spine hrefs arrive pre-resolved, so the guard at the call site is redundant; the rootfile path is the only remaining caller and `resolve_rel("", …)` is exactly its contract.

**Files:**
- Modify: `crates/kasane-adapters/src/guard.rs:4-10` (delete function), `:104-112` (delete test), `:132-158` (add two cases)
- Modify: `crates/kasane-adapters/src/epub/mod.rs:4` (import), `:29-30` (rootfile), `:49-56` (spine loop)
- Modify: `crates/kasane-adapters/src/fuzz_entry.rs:15` (import), `:137` (doc), `:145-152` (assertion block), `:214` (doc)
- Modify: `README.md:61`
- Test: `crates/kasane-adapters/src/guard.rs` and `crates/kasane-adapters/src/epub/mod.rs` (both `mod tests` blocks)

**Interfaces:**
- Consumes: `Opf.spine_hrefs` from Task 1 — normalized, root-confined, directly usable as `zip::by_name` keys.
- Produces: `crate::guard` exports `resolve_rel`, `check_expansion`, `has_scheme`, `parent_dir`, `safe_media_filename`. `safe_entry_name` no longer exists; Task 3 must not reference it.

- [ ] **Step 1: Write the failing end-to-end test**

Add to the `mod tests` block in `crates/kasane-adapters/src/epub/mod.rs`, after `build_epub2`:

```rust
/// An EPUB whose spine mixes a filename that merely *contains* `..`, an href
/// that legitimately walks out of the OPF directory, and one that escapes the
/// archive root entirely.
fn build_epub_awkward_hrefs() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    add(&mut w, "mimetype", b"application/epub+zip");
    add(&mut w, "META-INF/container.xml",
        br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#);
    add(
        &mut w,
        "OEBPS/content.opf",
        br#"<package><metadata><dc:title>T</dc:title></metadata>
        <manifest><item id="c1" href="..foo.xhtml" media-type="application/xhtml+xml"/>
        <item id="c2" href="../shared/ch.xhtml" media-type="application/xhtml+xml"/>
        <item id="c3" href="../../outside.xhtml" media-type="application/xhtml+xml"/></manifest>
        <spine><itemref idref="c1"/><itemref idref="c2"/><itemref idref="c3"/></spine></package>"#,
    );
    add(&mut w, "OEBPS/..foo.xhtml", b"<body><p>dotdot chapter</p></body>");
    add(&mut w, "shared/ch.xhtml", b"<body><p>shared chapter</p></body>");
    w.finish().unwrap();
    buf.into_inner()
}

fn has_para_text(doc: &Document, needle: &str) -> bool {
    doc.nodes.iter().any(|n| {
        matches!(&n.block,
            Block::Para(i) if i.iter().any(|x| matches!(x, Inline::Text(t) if t == needle)))
    })
}

#[test]
fn awkward_but_legal_spine_hrefs_are_read_and_escaping_ones_are_not() {
    let bytes = build_epub_awkward_hrefs();
    let (doc, _) = EpubAdapter.parse(&bytes, "b.epub").unwrap();
    // A filename containing `..` is not a traversal.
    assert!(
        has_para_text(&doc, "dotdot chapter"),
        "chapter at OEBPS/..foo.xhtml was dropped"
    );
    // An href that walks out of the OPF dir but stays inside the archive.
    assert!(
        has_para_text(&doc, "shared chapter"),
        "chapter at shared/ch.xhtml was dropped"
    );
    // Escaping the archive root is still rejected -- silently, not fatally:
    // the other two chapters must still convert.
    assert!(!has_para_text(&doc, "outside"));
}

#[test]
fn empty_rootfile_path_is_rejected_as_unsafe() {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    add(&mut w, "mimetype", b"application/epub+zip");
    add(&mut w, "META-INF/container.xml",
        br#"<container><rootfiles><rootfile full-path=""/></rootfiles></container>"#);
    w.finish().unwrap();
    let err = EpubAdapter.parse(&buf.into_inner(), "b.epub").unwrap_err();
    assert!(
        err.to_string().contains("unsafe rootfile path"),
        "expected the guard to reject an empty rootfile path, got: {err}"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p kasane-adapters awkward_but_legal_spine_hrefs empty_rootfile_path_is_rejected`

Expected: both FAIL.

- `awkward_but_legal_spine_hrefs_are_read_and_escaping_ones_are_not` fails on the first assertion: Task 1 resolves `..foo.xhtml` to `OEBPS/..foo.xhtml`, but `safe_entry_name` rejects it for containing `..`, so the chapter is dropped. (The `shared chapter` assertion already passes after Task 1 — it is a regression lock for defect 3.)
- `empty_rootfile_path_is_rejected_as_unsafe` fails because `safe_entry_name("")` returns `Some("")`, so the error is `malformed input: missing entry: ` from the zip lookup rather than `unsafe rootfile path`.

- [ ] **Step 3: Delete `safe_entry_name` from `guard.rs`**

Remove lines 4-10 of `crates/kasane-adapters/src/guard.rs`:

```rust
/// Sanitize a zip entry name; None if it escapes the archive root.
pub fn safe_entry_name(name: &str) -> Option<String> {
    if name.starts_with('/') || name.contains("..") {
        return None;
    }
    Some(name.to_string())
}
```

And its unit test, `rejects_traversal_names` (lines 104-112):

```rust
    #[test]
    fn rejects_traversal_names() {
        assert!(safe_entry_name("../etc/passwd").is_none());
        assert!(safe_entry_name("/abs").is_none());
        assert_eq!(
            safe_entry_name("OEBPS/ch1.xhtml"),
            Some("OEBPS/ch1.xhtml".to_string())
        );
    }
```

- [ ] **Step 4: Add the postcondition regression cases to `resolve_rel`'s tests**

Append to the existing `resolve_rel_normalizes_interior_base_dir` test in `crates/kasane-adapters/src/guard.rs`, inside its body:

```rust
        // The gap recorded in the outline-and-guard-hardening spec's §7:
        // the deleted safe_entry_name returned Some("") for these, and the
        // `guards` fuzz target asserted non-emptiness only for resolve_rel.
        // resolve_rel is now the only confinement primitive, so this is
        // where that postcondition gets pinned.
        assert_eq!(resolve_rel("", ""), None);
        assert_eq!(resolve_rel("", "."), None);
```

These pass immediately — `resolve_rel` already drops empty and `.` segments and returns `None` on an empty result. They are regression locks for a postcondition that was previously asserted nowhere, not red-first tests.

- [ ] **Step 5: Route the rootfile through `resolve_rel`**

In `crates/kasane-adapters/src/epub/mod.rs`, delete the import on line 4:

```rust
use crate::guard::safe_entry_name;
```

Then replace lines 29-30. Before:

```rust
        let opf_path = crate::guard::safe_entry_name(&opf_path)
            .ok_or(ParseError::Malformed("unsafe rootfile path".into()))?;
```

After:

```rust
        // container.xml's full-path is archive-root-relative, so the base is "".
        let opf_path = crate::guard::resolve_rel("", &opf_path)
            .ok_or(ParseError::Malformed("unsafe rootfile path".into()))?;
```

- [ ] **Step 6: Drop the spine-href guard**

In `crates/kasane-adapters/src/epub/mod.rs`, replace lines 49-56. Before:

```rust
        for href in &parsed.spine_hrefs {
            let Some(name) = safe_entry_name(href) else {
                continue;
            };
            let Ok(xml) = crate::ziputil::read_entry_string(&mut zip, &name, &mut total_read)
            else {
                continue;
            };
```

After:

```rust
        for href in &parsed.spine_hrefs {
            // parse_opf resolved and confined these, so they are usable as zip
            // keys directly -- re-guarding here is what used to drop legal
            // chapters whose names merely contained `..`.
            let name = href;
            let Ok(xml) = crate::ziputil::read_entry_string(&mut zip, name, &mut total_read)
            else {
                continue;
            };
```

The rest of the loop body is unchanged: `name` is now `&String`, so `name.clone()`, `name.rsplit_once('/')`, and the `Provenance` uses all still compile.

- [ ] **Step 7: Update the fuzz seam**

In `crates/kasane-adapters/src/fuzz_entry.rs`, change the import on line 15:

```rust
use crate::guard::{check_expansion, resolve_rel};
```

Change the doc comment on line 137. Before:

```rust
/// Three pure functions whose postconditions are security-critical and, until
/// now, asserted nowhere.
```

After:

```rust
/// Two pure functions whose postconditions are security-critical and, until
/// now, asserted nowhere.
```

Delete the `safe_entry_name` assertion block (lines 145-152):

```rust
    if let Some(name) = safe_entry_name(target) {
        // safe_entry_name rejects `..` as a substring, so the substring form is
        // the right check for *its* output.
        assert!(
            !name.contains("..") && !name.starts_with('/'),
            "safe_entry_name admitted an escaping name: {name:?} from {target:?}"
        );
    }
```

The `resolve_rel` block that follows is unchanged — it already asserts non-empty, non-absolute, and segment-wise no-`..`.

Fix the `build_zip` doc comment on line 214. Before:

```rust
/// instead, which is where `safe_entry_name`, the bomb guards, and the OPF /
```

After:

```rust
/// instead, which is where `resolve_rel`, the bomb guards, and the OPF /
```

- [ ] **Step 8: Update the README**

In `README.md`, change line 61. Before:

```
that `safe_entry_name` / `resolve_rel` never emit a traversal.
```

After:

```
that `resolve_rel` never emits a traversal.
```

- [ ] **Step 9: Run the tests to verify they pass**

Run: `cargo test -p kasane-adapters awkward_but_legal_spine_hrefs empty_rootfile_path_is_rejected`

Expected: both PASS.

- [ ] **Step 10: Run the full suite and lint**

Run: `mise run test && mise run lint`

Expected: PASS. `grep -rn safe_entry_name --include='*.rs' --include='*.md' . | grep -v '^./target'` should now return only lines in `docs/superpowers/` (the spec and this plan), which describe the deletion and are correct as written.

- [ ] **Step 11: Commit**

```bash
git add crates/kasane-adapters/src/guard.rs \
        crates/kasane-adapters/src/epub/mod.rs \
        crates/kasane-adapters/src/fuzz_entry.rs \
        README.md
git commit -m "fix(epub): delete safe_entry_name in favour of resolve_rel

safe_entry_name matched \`..\` as a substring and returned Some(\"\") for an
empty name -- the gap recorded in the outline-and-guard-hardening spec's
§7. Both of its production callers are EPUB paths that resolve_rel
already handles correctly, so hardening it would have meant maintaining
a second confinement primitive with no callers. Deleting it closes the
empty-name gap and stops dropping chapters named like ..foo.xhtml, and
leaves the crate with one path guard that cannot drift against another.

The assertion issue #22 predicted is not added: it already exists on
resolve_rel, which is now the only primitive left for it to cover."
```

---

### Task 3: Lock the empty-name shape into the stable corpus, and fix the stale quarantine comment

**Files:**
- Create: `fuzz/seeds/guards/empty.bin`
- Modify: `crates/kasane-adapters/tests/fuzz_corpus.rs:49-54`

**Interfaces:**
- Consumes: `crate::fuzz_entry::guards` from Task 2 — now exercising `resolve_rel` and `check_expansion` only.
- Produces: nothing later tasks depend on. This is the final task.

- [ ] **Step 1: Create the seed**

The `guards` target splits its input on the first NUL into `(base, target)`. A lone NUL byte yields an empty base and an empty target — the exact shape the deleted function mishandled.

```bash
printf '\0' > fuzz/seeds/guards/empty.bin
```

`.gitattributes` already marks `fuzz/seeds/**` as `-text -diff`, so no attribute change is needed.

- [ ] **Step 2: Verify the seed replays green on stable**

Run: `cargo test -p kasane-adapters --test fuzz_corpus -- --nocapture`

Expected: PASS. `replays_committed_seeds` picks up the new file automatically — it walks `fuzz/seeds/*/` and has no hardcoded count.

- [ ] **Step 3: Correct the stale quarantine comment**

`KNOWN_OPEN` is empty (line 55) because the parent item fixed both bugs and un-quarantined them, but the comment above it still asserts a live crash. In `crates/kasane-adapters/tests/fuzz_corpus.rs`, replace lines 49-54. Before:

```rust
///
/// This quarantine protects the stable replay test ONLY. It has no effect on
/// `mise run fuzz` / `mise run fuzz-all`, which still reproduce these same
/// crashes on nightly -- `guards` deterministically around run #3. That is
/// intended, not a gap to close: a weekly fuzz job going red on a real,
/// unfixed bug is the correct signal, and fuzz targets get no skip logic.
```

After:

```rust
///
/// This quarantine protects the stable replay test ONLY. It has no effect on
/// `mise run fuzz` / `mise run fuzz-all`, which still reproduce a quarantined
/// crash on nightly. That is intended, not a gap to close: a weekly fuzz job
/// going red on a real, unfixed bug is the correct signal, and fuzz targets
/// get no skip logic. The list is empty whenever every committed reproducer
/// has a landed fix behind it, which is the steady state.
```

- [ ] **Step 4: Run the full suite and lint**

Run: `mise run test && mise run lint`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add fuzz/seeds/guards/empty.bin crates/kasane-adapters/tests/fuzz_corpus.rs
git commit -m "test(fuzz): seed the empty-name shape and de-stale the quarantine note

The empty base/target pair is what safe_entry_name mishandled before it
was deleted; seeding it makes the shape replayed coverage on stable
rather than something only the nightly fuzzer reaches.

KNOWN_OPEN has been empty since the outline-and-guard-hardening item
un-quarantined both reproducers, but its doc comment still claimed
\`guards\` crashes deterministically around run #3."
```

- [ ] **Step 6: Verify on the pinned nightly**

`mise run lint && mise run test` proves the unit, end-to-end, and stable-replay layers. The OPF resolution path this plan changed sits inside the untrusted-input boundary, so the rest runs on nightly-2026-07-01:

```bash
mise run fuzz guards -- -max_total_time=60
mise run fuzz epub -- -max_total_time=60
mise run fuzz epub_zip -- -max_total_time=60
```

Expected: all three clean, no crash artifact written. `guards` crashed deterministically around run #3 before the parent item's `resolve_rel` fix landed; it must stay clean now. `epub` and `epub_zip` are the two targets that actually reach the changed OPF path.

No new artifact is expected. Issue #22 predicted a third finding *conditional on* adding the missing assertion to `safe_entry_name`; deleting the function makes that crash unreachable, so there is nothing to reproduce and `KNOWN_OPEN` stays empty. If the fuzzer nevertheless finds something, commit the reproducer to `fuzz/artifacts/<target>/` per the repo convention and treat it as a new finding.

- [ ] **Step 7: Confirm the behaviour the item actually owes**

Beyond green CI, the proof this item owes is that two chapters silently missing from a converted EPUB today now appear. `awkward_but_legal_spine_hrefs_are_read_and_escaping_ones_are_not` (Task 2) is that proof; confirm it is present and passing:

Run: `cargo test -p kasane-adapters awkward_but_legal_spine_hrefs -- --exact --nocapture`

Expected: PASS.

---

## Plan Self-Review

**1. Spec coverage:**

| Spec section | Task |
|---|---|
| §3.1 delete `safe_entry_name` + its test | Task 2, Steps 3-4 |
| §3.2 `join_href` → `resolve_rel` at `opf.rs:75` | Task 1, Steps 3-5 |
| §3.3 rootfile via `resolve_rel("", …)`; drop spine guard; drop import | Task 2, Steps 5-6 |
| §3.4 fuzz seam: import, assertion block | Task 2, Step 7 |
| §5.1 `guard.rs` empty/`.` regression cases | Task 2, Step 4 |
| §5.2 `opf.rs` href resolution units | Task 1, Step 1 |
| §5.3 `epub/mod.rs` end-to-end, three hrefs | Task 2, Step 1 |
| §5.4 `empty.bin` seed; no new artifact | Task 3, Steps 1, 6 |
| §6 README:61, fuzz_entry:137, fuzz_entry:214 | Task 2, Steps 7-8 |
| §6 fuzz_corpus.rs:50-54 stale comment | Task 3, Step 3 |
| §8 nightly verification | Task 3, Step 6 |

No gaps. §4's behavior delta is covered by the tests in Tasks 1 and 2; §2 and §7 are rationale with no implementation.

**2. Placeholder scan:** No TBD/TODO/"handle edge cases". Every code step shows the complete before-and-after text; every command names its expected outcome. Task 2 Step 4 is explicitly flagged as passing immediately rather than red-first, so its outcome is not mistaken for a broken TDD cycle.

**3. Type consistency:** `resolve_rel(&str, &str) -> Option<String>` is used identically in Tasks 1 and 2. `Opf.spine_hrefs: Vec<String>` is produced by Task 1 and consumed by Task 2's loop, where `href: &String` and `name = href` keeps `name.clone()` and `name.rsplit_once('/')` compiling. Test helpers `add`, `build_epub`, `build_epub2` already exist in `epub/mod.rs`'s test module; `build_epub_awkward_hrefs` and `has_para_text` are new and defined in Task 2 Step 1 before use. `Document`, `Block`, `Inline` are already in scope in that module via `use super::*`.
