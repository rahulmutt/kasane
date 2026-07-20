# Toolchain Upgrade, mise Tasks, and CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upgrade the pinned Rust toolchain to 1.97.1, make mise the single source for both tools and tasks (retiring `just`), and stand up CI, dependency updates, security audit, and tag-driven releases on GitHub Actions.

**Architecture:** Six sequential phases across seven tasks. Task 1 raises the toolchain and absorbs the resulting clippy fallout — it is the only task with unpredictable scope, and it lands green before any workflow file exists. Task 2 swaps the task runner. Tasks 3–5 add workflows that consume those mise tasks. Tasks 6–7 prepare manifests for crates.io and add the release workflow, last because they are the only work blocked on a repository secret.

**Tech Stack:** Rust 1.97.1, cargo, mise 2026.7.x (tools + tasks), GitHub Actions, cargo-deny 0.20.2, Dependabot.

## Global Constraints

- Rust toolchain pin: `1.97.1`. MSRV (`rust-version`) moves with it to `1.97`.
- Edition stays `2021`. Bumping to edition 2024 is deliberately out of scope.
- CI runs on Linux only (`ubuntu-latest`); release adds `ubuntu-24.04-arm`.
- Every GitHub Action is pinned by **commit SHA**, never by tag.
- Every task ends green under `mise run lint && mise run test` (from Task 2 onward; Task 1 uses `just`).
- `deny.toml` is scoped to `advisories` only. License and banned-crate policy are out of scope.
- Files under `docs/superpowers/plans/` and the 2026-07-19 spec are historical records — do **not** rewrite their `just` references.
- Repository URL: `https://github.com/rahulmutt/kasane`.

## File Structure

**Created:**
- `.github/workflows/ci.yml` — lint + test on push/PR
- `.github/workflows/audit.yml` — cargo-deny advisories, on Cargo.lock change + weekly
- `.github/workflows/release.yml` — binaries + crates.io publish on `v*` tags
- `.github/dependabot.yml` — cargo + github-actions ecosystems
- `deny.toml` — advisories configuration

**Modified:**
- `mise.toml` — toolchain pin, cargo-deny pin, `[tasks]` section
- `Cargo.toml` — `rust-version`, `version`, `repository`, `[workspace.dependencies]`
- `crates/*/Cargo.toml` — relaxed pins, workspace version/description/deps
- `Cargo.lock` — refreshed
- `README.md`, `AGENTS.md` — `just` → `mise run`

**Deleted:**
- `justfile`

---

### Task 1: Upgrade toolchain to 1.97.1 and dissolve the MSRV-driven pins

**Files:**
- Modify: `mise.toml`
- Modify: `Cargo.toml` (`[workspace.package]`)
- Modify: `crates/kasane-cli/Cargo.toml`
- Modify: `crates/kasane-writer/Cargo.toml`
- Modify: `Cargo.lock` (regenerated)
- Modify: any source file clippy flags (unknown until Step 5)

**Interfaces:**
- Consumes: nothing.
- Produces: a workspace that builds and lints clean on Rust 1.97.1. Later tasks assume `cargo clippy --workspace --all-targets -- -D warnings` exits 0.

- [ ] **Step 1: Confirm the current state is green before changing anything**

Run: `just lint && just test`
Expected: PASS. If this fails, stop — the baseline is broken and any clippy output after the upgrade would be uninterpretable.

- [ ] **Step 2: Raise the toolchain pin**

Edit `mise.toml`, changing only the rust line:

```toml
[tools]
rust = "1.97.1"
"cargo:just" = "1.46.0"
```

Leave `cargo:just` alone — Task 2 removes it. Removing it here would break Step 1's own tooling mid-task.

- [ ] **Step 3: Install and verify the new toolchain**

Run: `mise install && rustc --version && cargo --version`
Expected: `rustc 1.97.1 (...)`. If it reports 1.83.0, the install did not take — do not proceed.

- [ ] **Step 4: Raise the MSRV and relax the three pins**

In `Cargo.toml`, `[workspace.package]`:

```toml
[workspace.package]
edition = "2021"
rust-version = "1.97"
license = "Apache-2.0"
```

In `crates/kasane-cli/Cargo.toml`, delete both pin comments and relax both versions:

```toml
[dependencies]
kasane-adapters = { path = "../kasane-adapters" }
kasane-core = { path = "../kasane-core" }
kasane-writer = { path = "../kasane-writer" }
clap = { version = "4", features = ["derive"] }
anyhow = "1"

[dev-dependencies]
tempfile = "3"
```

In `crates/kasane-writer/Cargo.toml`, delete the pin comment and relax:

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 5: Update the lockfile and see what breaks**

Run: `cargo update && just lint 2>&1 | tee /tmp/clippy-after-upgrade.txt`

Expected: this is the step with unknown scope. Jumping 14 Rust releases under `-D warnings` means clippy has gained lints that the existing code has never been checked against. Two outcomes are both normal:
- Clean pass → skip to Step 7.
- A list of clippy errors → continue to Step 6.

- [ ] **Step 6: Fix the clippy findings**

Group the output by lint name to see what you're actually dealing with:

Run: `grep -oE '^error: .*|`clippy::[a-z_]+`' /tmp/clippy-after-upgrade.txt | sort | uniq -c | sort -rn`

Then, in order of cheapness:

1. **Let clippy fix the mechanical ones first.** Many new lints are auto-fixable:

   Run: `cargo clippy --workspace --all-targets --fix --allow-dirty`

   Re-run `just lint` afterward and re-check what remains.

2. **Fix the rest by hand.** Read each remaining lint's explanation before changing code — a new lint firing on this codebase usually means the idiom improved, not that the code was wrong:

   Run: `cargo clippy --explain <lint_name>`

3. **Suppress only with justification.** If a new lint is genuinely wrong for this codebase (false positive, or a style it disagrees with), suppress it narrowly rather than reshaping correct code. Prefer a targeted attribute at the site:

   ```rust
   // clippy's suggestion allocates per iteration; the explicit loop is hot-path.
   #[allow(clippy::some_lint_name)]
   ```

   Only if a lint is noisy across the whole workspace, add it to `Cargo.toml`:

   ```toml
   [workspace.lints.clippy]
   all = { level = "warn", priority = -1 }
   some_lint_name = "allow"   # reason it does not fit this codebase
   ```

   Never blanket-allow to make the build pass without reading the lint.

- [ ] **Step 7: Verify lint and tests are both green on the new toolchain**

Run: `just lint && just test`
Expected: PASS, with zero clippy warnings.

The test run matters as much as the lint run here — relaxing `clap` and `tempfile` pulled in new major-ish dependency versions, and behavior changes in argument parsing or temp-file handling would show up as test failures, not lint errors.

- [ ] **Step 8: Confirm the pins actually moved**

Run: `grep -E '^(name|version)' Cargo.lock | grep -A1 -E 'name = "(clap|tempfile)"'`
Expected: `clap` above 4.5.48 and `tempfile` above 3.14.0. If either is unchanged, the `=` pin was not removed correctly.

- [ ] **Step 9: Commit**

```bash
git add mise.toml Cargo.toml Cargo.lock crates/
git commit -m "chore: upgrade to rust 1.97.1, raise MSRV, drop MSRV-driven pins"
```

---

### Task 2: Replace just with mise tasks

**Files:**
- Modify: `mise.toml`
- Delete: `justfile`
- Modify: `README.md:7-14`
- Modify: `AGENTS.md:12,17`

**Interfaces:**
- Consumes: the green workspace from Task 1.
- Produces: `mise run build`, `mise run test`, `mise run lint`, `mise run convert <args>`. Tasks 3, 4, and 7 invoke `mise run lint` and `mise run test` by these exact names.

- [ ] **Step 1: Add the tasks and drop the just pin**

Replace the whole of `mise.toml` with:

```toml
[tools]
rust = "1.97.1"

[tasks.build]
run = "cargo build --workspace"

[tasks.test]
run = "cargo test --workspace"

[tasks.lint]
run = ["cargo fmt --all -- --check",
       "cargo clippy --workspace --all-targets -- -D warnings"]

[tasks.convert]
run = "cargo run -p kasane-cli --"
```

Two notes on why this shape:
- `lint` is a two-element list, not one `&&` string. mise runs list entries in order and **stops at the first non-zero exit**, so a fmt failure short-circuits clippy — same semantics as the old justfile, verified.
- The task is `convert`, not `run`, because `mise run run` reads badly. mise appends trailing args to the last command, so `mise run convert book.epub -o out/book` expands to `cargo run -p kasane-cli -- book.epub -o out/book`. No `--` separator needed; flags pass through cleanly.

- [ ] **Step 2: Verify each task before deleting the justfile**

Run: `mise install && mise run lint && mise run test && mise run build`
Expected: all PASS.

Run: `mise run convert --help`
Expected: kasane's clap-generated help text. This is the arg-passthrough check — if flags were being swallowed by mise, `--help` would print mise's help instead of kasane's.

- [ ] **Step 3: Delete the justfile**

```bash
git rm justfile
```

- [ ] **Step 4: Confirm just is fully gone from live files**

Run: `grep -rn "just " README.md AGENTS.md mise.toml Cargo.toml crates/ 2>/dev/null`
Expected: no output.

Do **not** widen this grep to `docs/` — the plans and the 2026-07-19 spec reference `just` as a record of what was built then, and rewriting history there would be wrong.

- [ ] **Step 5: Update README.md**

Replace the Quick start and Development sections:

```markdown
## Quick start
    mise install
    mise run build
    mise run convert book.epub -o out/book
    # open out/book/index.md and drill into linked sections

## Development
    mise run test    # run all tests
    mise run lint    # fmt check + clippy -D warnings
```

Leave the rest of README.md untouched.

- [ ] **Step 6: Update AGENTS.md**

Line 12 becomes:

```markdown
- `mise run test` — all tests   - `mise run lint` — fmt + clippy   - `mise run convert <file> -o <dir>` — convert
```

Line 17 becomes:

```markdown
- Every change ships green under `mise run lint && mise run test`.
```

- [ ] **Step 7: Re-run the grep from Step 4**

Run: `grep -rn "just " README.md AGENTS.md`
Expected: no output.

- [ ] **Step 8: Commit**

```bash
git add mise.toml README.md AGENTS.md
git commit -m "chore: replace just with mise tasks"
```

---

### Task 3: CI workflow

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `mise run lint` and `mise run test` from Task 2.
- Produces: a `ci` workflow with a `check` job. Task 5's Dependabot PRs are gated by it.

- [ ] **Step 1: Create the workflow**

```yaml
name: ci

on:
  push:
    branches: [main]
  pull_request:

# A newer push to the same ref makes the in-flight run irrelevant. Cancel it
# rather than paying for a result nobody will read. Never cancel on main:
# those runs are the record of what actually landed.
concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}

permissions:
  contents: read

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@93cb6efe18208431cddfb8368fd83d5badbf9bfd # v5

      # Installs the exact toolchain pinned in mise.toml, so CI and a laptop
      # resolve from one file and cannot drift.
      - uses: jdx/mise-action@dad1bfd3df957f44999b559dd69dc1671cb4e9ea # v4.2.1

      - uses: Swatinem/rust-cache@c19371144df3bb44fab255c43d04cbc2ab54d1c4 # v2.9.1

      - name: Lint
        run: mise run lint

      - name: Test
        run: mise run test
```

Every `uses:` is a commit SHA with the tag in a trailing comment. Tags are mutable — a compromised or retagged action would otherwise execute silently. The comment is what Dependabot reads to know which version the SHA represents.

- [ ] **Step 2: Verify the YAML parses**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml')); print('ok')"`
Expected: `ok`

- [ ] **Step 3: Confirm lint and test still pass locally**

Run: `mise run lint && mise run test`
Expected: PASS. CI runs exactly these two commands, so a local failure guarantees a CI failure.

- [ ] **Step 4: Commit and push on a branch**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add lint and test workflow"
git push -u origin HEAD
```

- [ ] **Step 5: Open a PR and watch the run**

Workflow files cannot be validated locally — the only real verification is observation.

Run: `gh pr create --fill && gh pr checks --watch`
Expected: the `check` job passes.

If mise-action fails to find the toolchain, confirm `mise.toml` is committed. If clippy fails in CI but passed locally, the cache is serving a stale toolchain — re-run with the cache disabled to confirm before debugging further.

---

### Task 4: Security audit with cargo-deny

**Files:**
- Create: `deny.toml`
- Create: `.github/workflows/audit.yml`
- Modify: `mise.toml` (`[tools]`)

**Interfaces:**
- Consumes: the `[tools]` block from Task 2.
- Produces: `cargo deny check advisories`, runnable locally and in CI.

- [ ] **Step 1: Pin cargo-deny as a tool**

Add to `mise.toml` `[tools]`:

```toml
[tools]
rust = "1.97.1"
"cargo:cargo-deny" = "0.20.2"
```

Pinning it here rather than installing it only in CI means the audit is reproducible on a laptop — a scanner you cannot run locally is one you will not run before pushing.

- [ ] **Step 2: Install and verify**

Run: `mise install && cargo deny --version`
Expected: `cargo-deny 0.20.2`

This builds from source and takes several minutes on a cold cache. That is expected.

- [ ] **Step 3: Create deny.toml**

```toml
# Scoped to advisories only. License and banned-crate policy are deliberately
# out of scope: they are a policy discussion, and letting one block the other
# would mean shipping no vulnerability scanning at all.
[advisories]
ignore = []
```

- [ ] **Step 4: Run the audit and see the real state**

Run: `cargo deny check advisories`
Expected: one of two outcomes, both normal:
- `advisories ok` → proceed.
- One or more advisories reported → these are real findings against the committed `Cargo.lock`, not plan failures. For each: upgrade the affected dependency if a patched version exists (`cargo update -p <crate>`), then re-run. Only if no fix exists yet, add it to `ignore` with a comment naming the advisory ID and why it is not exploitable here — for example, an advisory in a code path this crate never calls.

This matters more than boilerplate for kasane specifically: `kasane-adapters` is an explicit untrusted-input boundary, and its `zip` and `quick-xml` dependencies are exactly where a parser CVE lands.

- [ ] **Step 5: Create the audit workflow**

```yaml
name: audit

on:
  push:
    branches: [main]
    paths: ['**/Cargo.toml', 'Cargo.lock', 'deny.toml', '.github/workflows/audit.yml']
  pull_request:
    paths: ['**/Cargo.toml', 'Cargo.lock', 'deny.toml', '.github/workflows/audit.yml']
  # The important trigger. Advisories are published against dependencies that
  # were already committed, so an audit that only fires on code changes goes
  # stale silently. Mondays 07:00 UTC.
  schedule:
    - cron: '0 7 * * 1'

permissions:
  contents: read

jobs:
  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@93cb6efe18208431cddfb8368fd83d5badbf9bfd # v5
      - uses: jdx/mise-action@dad1bfd3df957f44999b559dd69dc1671cb4e9ea # v4.2.1
      - uses: Swatinem/rust-cache@c19371144df3bb44fab255c43d04cbc2ab54d1c4 # v2.9.1
      - name: Audit dependencies
        run: cargo deny check advisories
```

- [ ] **Step 6: Verify the YAML parses**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/audit.yml')); print('ok')"`
Expected: `ok`

- [ ] **Step 7: Commit**

```bash
git add mise.toml deny.toml .github/workflows/audit.yml
git commit -m "ci: add cargo-deny advisory audit on change and weekly"
```

---

### Task 5: Dependabot

**Files:**
- Create: `.github/dependabot.yml`

**Interfaces:**
- Consumes: the CI workflow from Task 3 (which gates the PRs Dependabot opens).
- Produces: nothing other tasks depend on.

- [ ] **Step 1: Create the config**

```yaml
version: 2
updates:
  - package-ecosystem: cargo
    directory: /
    schedule:
      interval: weekly
    open-pull-requests-limit: 5

  # Required, not optional. The workflows pin actions by commit SHA, and a SHA
  # pin never updates itself — without this, those pins silently rot and stop
  # receiving upstream security fixes. Dependabot reads the version from the
  # trailing comment next to each SHA.
  - package-ecosystem: github-actions
    directory: /
    schedule:
      interval: weekly
```

- [ ] **Step 2: Verify the YAML parses**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/dependabot.yml')); print('ok')"`
Expected: `ok`

- [ ] **Step 3: Commit**

```bash
git add .github/dependabot.yml
git commit -m "ci: add dependabot for cargo and github-actions"
```

- [ ] **Step 4: Note the known gap**

Dependabot does not understand `mise.toml`. The Rust toolchain pin and the cargo-deny pin remain manual bumps. This is the accepted cost of mise-first and needs no code — but state it plainly when reporting the task complete, so nobody later assumes the toolchain is being watched.

---

### Task 6: Prepare manifests for crates.io

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/kasane-ir/Cargo.toml`
- Modify: `crates/kasane-core/Cargo.toml`
- Modify: `crates/kasane-writer/Cargo.toml`
- Modify: `crates/kasane-adapters/Cargo.toml`
- Modify: `crates/kasane-cli/Cargo.toml`

**Interfaces:**
- Consumes: nothing from Tasks 3–5.
- Produces: manifests that `cargo package --workspace` accepts. Task 7's publish job depends on this.

**Context — why this task exists:** `cargo publish` rejects a path dependency with no version, because the registry copy has no path to follow. Publishing also requires `description`. Neither is present today.

- [ ] **Step 1: Confirm the current manifests are unpublishable**

Run: `cargo package -p kasane-core --allow-dirty 2>&1 | tail -5`
Expected: FAIL, complaining that dependency `kasane-ir` is missing a version specification. This is the failure Steps 2–4 fix — see it before fixing it.

- [ ] **Step 2: Add workspace-level version, metadata, and internal deps**

Replace `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.97"
license = "Apache-2.0"
repository = "https://github.com/rahulmutt/kasane"

# Internal crates are declared once, with both a path (for local builds) and a
# version (which is what cargo records in the published manifest). Declaring
# them here rather than per-crate means a release bumps two lines total, not
# five package versions plus every path dependency that names them.
[workspace.dependencies]
kasane-ir = { path = "crates/kasane-ir", version = "0.1.0" }
kasane-core = { path = "crates/kasane-core", version = "0.1.0" }
kasane-writer = { path = "crates/kasane-writer", version = "0.1.0" }
kasane-adapters = { path = "crates/kasane-adapters", version = "0.1.0" }

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
```

Two cautions on this replacement:

- `kasane-cli` is absent from `[workspace.dependencies]` deliberately — nothing depends on it.
- **Preserve any lint allows Task 1 added.** If the clippy fallout in Task 1 Step 6 required workspace-level `[workspace.lints.clippy]` entries, they are in the file you are about to overwrite. Check before replacing:

  Run: `git show HEAD:Cargo.toml | sed -n '/\[workspace.lints.clippy\]/,$p'`

  Carry any entries beyond the `all = ...` line into the new block, comments included. Dropping them silently re-breaks `mise run lint`, and Step 4 is where you would find out.

- [ ] **Step 3: Convert each crate to workspace version and description**

`crates/kasane-ir/Cargo.toml`:

```toml
[package]
name = "kasane-ir"
description = "Intermediate representation types for the kasane document converter"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[lints]
workspace = true
```

`crates/kasane-core/Cargo.toml`:

```toml
[package]
name = "kasane-core"
description = "Pure structuring engine for kasane: fold, balance, paths, refs, nav"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
kasane-ir.workspace = true

[lints]
workspace = true
```

`crates/kasane-writer/Cargo.toml`:

```toml
[package]
name = "kasane-writer"
description = "Renders the kasane IR to GitHub-Flavored Markdown file trees"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
kasane-ir.workspace = true
kasane-core.workspace = true
anyhow = "1"

[dev-dependencies]
tempfile = "3"

[lints]
workspace = true
```

`crates/kasane-adapters/Cargo.toml`:

```toml
[package]
name = "kasane-adapters"
description = "Format detection and parsers (EPUB, PPTX) for the kasane document converter"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
kasane-ir.workspace = true
zip = { version = "2", default-features = false, features = ["deflate"] }
quick-xml = "0.36"
thiserror = "1"

[dev-dependencies]
kasane-core.workspace = true
kasane-writer.workspace = true
tempfile = "3"

[lints]
workspace = true
```

`crates/kasane-cli/Cargo.toml`:

```toml
[package]
name = "kasane-cli"
description = "CLI that converts documents and ebooks into agent-friendly Markdown trees"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[[bin]]
name = "kasane"
path = "src/main.rs"

[dependencies]
kasane-adapters.workspace = true
kasane-core.workspace = true
kasane-writer.workspace = true
clap = { version = "4", features = ["derive"] }
anyhow = "1"

[dev-dependencies]
tempfile = "3"

[lints]
workspace = true
```

The binary stays `kasane` even though the crate is `kasane-cli`. The bare `kasane` name on crates.io belongs to an unrelated crate (a "Tetter REST API client", owner `korewaChino`, last published 2024-09), so installation is `cargo install kasane-cli`. This is not obtainable — do not spend time trying.

- [ ] **Step 4: Verify the workspace still builds and tests**

Run: `mise run lint && mise run test`
Expected: PASS. The dependency graph is unchanged; only how it is declared moved.

- [ ] **Step 5: Verify every crate is now packageable**

Run: `cargo package --workspace --allow-dirty 2>&1 | tail -20`
Expected: all five crates package successfully. The Step 1 error about a missing version specification must be gone.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/
git commit -m "chore: add crates.io metadata and lockstep workspace versioning"
```

---

### Task 7: Release workflow

**Files:**
- Create: `.github/workflows/release.yml`
- Modify: `README.md` (install section)

**Interfaces:**
- Consumes: publishable manifests from Task 6, `mise run test` from Task 2.
- Produces: nothing other tasks depend on.

**Blocked on:** `CARGO_REGISTRY_TOKEN` must exist as a repository secret before the `publish` job can succeed. Confirm with the repository owner before Step 5.

- [ ] **Step 1: Determine whether cargo supports workspace publishing**

Cargo 1.90 stabilized publishing a whole workspace in dependency order, which handles both ordering and index-propagation waits. Confirm rather than assume:

Run: `cargo publish --help | grep -- --workspace && echo SUPPORTED || echo NOT_SUPPORTED`

If `SUPPORTED`, use the `publish` job exactly as written in Step 2. If `NOT_SUPPORTED`, replace that job's final step with the sequential fallback in Step 3.

- [ ] **Step 2: Create the release workflow**

```yaml
name: release

on:
  push:
    tags: ['v*']

permissions:
  contents: write   # required to create the GitHub Release

jobs:
  binaries:
    strategy:
      fail-fast: false
      matrix:
        include:
          # Native runners per architecture rather than cross-compiling: no
          # cross-linker to configure, and each binary is built on the
          # architecture it will actually run on.
          - runner: ubuntu-latest
            target: x86_64-unknown-linux-gnu
          - runner: ubuntu-24.04-arm
            target: aarch64-unknown-linux-gnu
    runs-on: ${{ matrix.runner }}
    steps:
      - uses: actions/checkout@93cb6efe18208431cddfb8368fd83d5badbf9bfd # v5
      - uses: jdx/mise-action@dad1bfd3df957f44999b559dd69dc1671cb4e9ea # v4.2.1
      - uses: Swatinem/rust-cache@c19371144df3bb44fab255c43d04cbc2ab54d1c4 # v2.9.1

      - name: Build release binary
        run: cargo build --release -p kasane-cli

      - name: Package tarball
        run: |
          tar -czf "kasane-${{ github.ref_name }}-${{ matrix.target }}.tar.gz" \
            -C target/release kasane

      - uses: softprops/action-gh-release@3bb12739c298aeb8a4eeaf626c5b8d85266b0e65 # v2
        with:
          files: kasane-${{ github.ref_name }}-${{ matrix.target }}.tar.gz

  publish:
    # Only publish once binaries prove the tag builds on both architectures.
    # An unpublish on crates.io is not possible; a failed release job is.
    needs: binaries
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@93cb6efe18208431cddfb8368fd83d5badbf9bfd # v5
      - uses: jdx/mise-action@dad1bfd3df957f44999b559dd69dc1671cb4e9ea # v4.2.1

      - name: Test before publishing
        run: mise run test

      - name: Publish to crates.io
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
        run: cargo publish --workspace
```

- [ ] **Step 3: Sequential fallback, only if Step 1 reported NOT_SUPPORTED**

Replace the final step of the `publish` job with:

```yaml
      - name: Publish to crates.io
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
        # Strict dependency order. Each crate must appear in the index before
        # the next one can resolve it, hence the wait between publishes.
        run: |
          for crate in kasane-ir kasane-core kasane-writer kasane-adapters kasane-cli; do
            echo "Publishing $crate"
            cargo publish -p "$crate"
            sleep 30
          done
```

- [ ] **Step 4: Verify the YAML parses**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml')); print('ok')"`
Expected: `ok`

- [ ] **Step 5: Confirm the secret exists**

Run: `gh secret list | grep CARGO_REGISTRY_TOKEN`
Expected: the secret is listed. If absent, stop and ask the repository owner to add it — Step 7 will fail without it, and a partially-published workspace cannot be undone.

- [ ] **Step 6: Add an install section to README.md**

Insert after the Quick start section:

```markdown
## Install
    cargo install kasane-cli   # installs the `kasane` binary
```

- [ ] **Step 7: Commit, then verify with a throwaway pre-release tag**

```bash
git add .github/workflows/release.yml README.md
git commit -m "ci: add release workflow for binaries and crates.io"
git push
```

A release workflow that has never run is not a working release workflow, and the first real tag is the worst place to discover that. Verify with a pre-release tag first:

```bash
git tag v0.1.0-rc.1
git push origin v0.1.0-rc.1
gh run watch
```

Expected: both `binaries` jobs produce tarballs attached to the release, and `publish` succeeds.

Be aware this genuinely publishes `0.1.0-rc.1` to crates.io. That is intended — it is a real end-to-end check, and a pre-release version is not served to `cargo install` by default. Versions can be yanked but never deleted, so do not iterate on failures by re-tagging the same version; bump to `-rc.2` each attempt.

- [ ] **Step 8: Clean up the test release**

```bash
gh release delete v0.1.0-rc.1 --yes
git push --delete origin v0.1.0-rc.1
git tag -d v0.1.0-rc.1
```

The crates.io publish of `0.1.0-rc.1` stays — it cannot be removed. Leave it.

---

## Verification

After all seven tasks:

- [ ] `mise run lint && mise run test` passes
- [ ] `cargo deny check advisories` passes
- [ ] `grep -rn "just " README.md AGENTS.md mise.toml` returns nothing
- [ ] `test ! -f justfile`
- [ ] `rustc --version` reports 1.97.1
- [ ] A pull request shows the `ci` check passing
- [ ] `.github/` contains `dependabot.yml` and three workflows

## Out of Scope

- macOS and Windows CI
- Code coverage reporting
- License and banned-crate policy in `deny.toml`
- Migrating to edition 2024
- Rewriting `just` references in `docs/superpowers/plans/` or the 2026-07-19 spec
