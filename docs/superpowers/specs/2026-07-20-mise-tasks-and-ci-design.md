# Toolchain upgrade, mise tasks, and CI — design

Date: 2026-07-20

## Goal

Three things, in dependency order:

1. Upgrade the pinned toolchain to current versions.
2. Make mise the single source for both tools and tasks, retiring `just`.
3. Add continuous integration on GitHub Actions, plus dependency updates,
   a security audit, and tag-driven releases.

## Decisions

| Question | Decision |
| --- | --- |
| MSRV | Moves with the toolchain to 1.97. One version to reason about; no separate MSRV job. |
| Task runner | mise `[tasks]`. `just` is removed entirely. |
| CI platforms | Linux only. Widening is a two-line matrix change later. |
| Release targets | Linux x86_64 + aarch64 binaries on `v*` tags. |
| crates.io | Yes, but sequenced last — it is the only piece blocked on a repo secret. |
| Crate versioning | Lockstep via `workspace.package`, so one line bumps all five crates. |

## Phase 1 — toolchain upgrade

`mise.toml` `[tools]`:

- `rust`: `1.83.0` → `1.97.1`
- `cargo:just`: removed (see phase 2)
- `cargo:cargo-deny`: `0.20.2` added (see phase 4)

`Cargo.toml` `[workspace.package]`: `rust-version` `1.83` → `1.97`.

### Dependency pins that this dissolves

Two dependencies are `=`-pinned *solely* because of Rust 1.83, each carrying a
comment saying so. Raising the toolchain removes the reason, so they relax and
the comments go:

| Crate | Current | Becomes |
| --- | --- | --- |
| `kasane-cli` | `clap = "=4.5.48"` | `clap = "4"` |
| `kasane-cli` | `tempfile = "=3.14.0"` | `tempfile = "3"` |
| `kasane-writer` | `tempfile = "=3.14.0"` | `tempfile = "3"` |

Then `cargo update`, and commit the refreshed `Cargo.lock`.

### The risk in this phase

This jumps 14 Rust releases under `-D warnings`. Clippy has gained lints in that
span, so **expect real clippy fixes to fall out**, in unknown number. This phase
lands as its own commit, fully green, before any CI work — otherwise CI goes
green and then red for reasons unrelated to CI.

## Phase 2 — mise tasks replace just

`justfile` is deleted. `mise.toml` gains:

```toml
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

`just run` becomes `mise run convert` — `mise run run` reads badly.

Docs to update: `README.md` (4 references) and `AGENTS.md` (2 references).
Files under `docs/superpowers/plans/` and the 2026-07-19 spec keep their `just`
references; they are the historical record of what was built at the time, not
live documentation.

## Phase 3 — CI workflow

`.github/workflows/ci.yml`. Triggers: push to `main`, and pull requests. A
concurrency group keyed on the ref cancels superseded runs.

Single job on `ubuntu-latest`:

1. `actions/checkout` @ `93cb6efe18208431cddfb8368fd83d5badbf9bfd`
2. `jdx/mise-action` @ `dad1bfd3df957f44999b559dd69dc1671cb4e9ea` — installs the
   exact pinned toolchain, so CI and local resolve identically from one file
3. `Swatinem/rust-cache` @ `c19371144df3bb44fab255c43d04cbc2ab54d1c4`
4. `mise run lint`
5. `mise run test`

All actions are pinned by commit SHA, not tag. Tags are mutable; SHAs are not.

## Phase 4 — security audit

A second workflow, `.github/workflows/audit.yml`, running `cargo deny check
advisories` against the committed `Cargo.lock`.

Triggers: pushes touching `Cargo.lock` or `deny.toml`, **and a weekly schedule**.
The schedule is the point — advisories are published without anyone touching the
code, so an audit that only runs on push goes stale silently.

`deny.toml` is scoped to `advisories` only at first. License and banned-crate
policy are deliberately deferred: they turn into a policy debate that should not
block getting CI up.

cargo-deny is pinned in `mise.toml` rather than installed by a CI-only action,
so it runs identically on a laptop.

This matters more than boilerplate here: `kasane-adapters` is an explicit
untrusted-input boundary, and its zip and XML parsing dependencies are exactly
where a parser CVE would land.

## Phase 5 — Dependabot

`.github/dependabot.yml`, weekly, two ecosystems:

- `cargo` — application dependencies, gated by the CI from phase 3
- `github-actions` — required, because phase 3 pins actions by SHA, and those
  pins rot invisibly without an updater

**Known gap:** Dependabot does not understand `mise.toml`. The Rust toolchain
pin stays a manual bump. This is the accepted cost of mise-first.

## Phase 6 — release and crates.io

Sequenced last. Everything before this is unblocked; this phase is not.

### Prerequisites

1. **Path dependencies carry no version.** `kasane-ir = { path = "../kasane-ir" }`
   is rejected by `cargo publish`, which needs a version to record in the
   registry.
2. **`version` moves to `[workspace.package]`**, with each crate using
   `version.workspace = true`, so one line bumps all five.

   These two interact. Giving each path dep a literal `version = "0.1.0"` would
   mean hand-editing five dependency lines on every release — exactly what
   lockstep versioning is meant to avoid. So the internal crates are declared
   once in `[workspace.dependencies]`:

   ```toml
   [workspace.dependencies]
   kasane-ir = { path = "crates/kasane-ir", version = "0.1.0" }
   # ...one line per internal crate
   ```

   and consumed as `kasane-ir.workspace = true`. One bump site for the package
   version, one for the dependency versions.
3. **Required metadata is missing.** No `description` (crates.io requires it)
   and no `repository`. Add `repository` to `[workspace.package]`, plus a
   per-crate `description`.
4. **`CARGO_REGISTRY_TOKEN` must be added as a repository secret.** This is the
   one step that cannot be done from here.

### Name availability

Checked against crates.io on 2026-07-20:

- `kasane` — **taken** by an unrelated crate (a "Tetter REST API client",
  owner `korewaChino`, last published 2024-09).
- `kasane-ir`, `kasane-core`, `kasane-writer`, `kasane-adapters`,
  `kasane-cli` — all available.

Consequence: the binary is still named `kasane`, but installation is
`cargo install kasane-cli`. The bare name is not obtainable.

### Workflow

`.github/workflows/release.yml`, triggered on `v*` tags, two jobs:

**`binaries`** — matrix of `x86_64` on `ubuntu-latest` and `aarch64` on
`ubuntu-24.04-arm`. Native ARM runners rather than cross-compilation: no
cross-linker setup, and the binary is built on the architecture it targets.
Artifacts are tarballed and attached via `softprops/action-gh-release` @
`3bb12739c298aeb8a4eeaf626c5b8d85266b0e65`.

**`publish`** — `cargo publish` in strict dependency order, sequentially, since
each crate waits on the previous to appear in the index:

```
kasane-ir → kasane-core → kasane-writer → kasane-adapters → kasane-cli
```

## Testing

Each phase ends green under `mise run lint && mise run test`.

Phases 3–6 add workflow files, which cannot be validated locally. They are
verified by observation: phase 3 by opening a pull request and watching the run,
phase 6 by pushing a throwaway pre-release tag (`v0.1.0-rc.1`) and confirming
both jobs before any real tag.

## Out of scope

- macOS and Windows CI — deferred with the Linux-only decision.
- Code coverage reporting.
- License and banned-crate policy in `deny.toml`.
- Rewriting historical plans under `docs/superpowers/plans/`.
