# Codebase map

Pipeline: input file -> detect -> adapter -> IR -> structure() -> write_tree -> Markdown tree.

- `crates/kasane-ir`      Intermediate representation types. Depends on nothing.
- `crates/kasane-adapters` Format detection + parsers (EPUB, PPTX). Untrusted-input boundary; see `guard.rs` and `ziputil.rs` (every guarded zip read goes through it).
- `crates/kasane-core`    Pure structuring engine: fold -> balance -> paths -> refs -> nav. No I/O.
- `crates/kasane-writer`  IR -> GitHub-Flavored Markdown; atomic tree writing.
- `crates/kasane-cli`     `kasane` binary; wires the pipeline; owns exit codes.

## Workflows
- `mise run test` — all tests   - `mise run lint` — fmt + clippy   - `mise run convert <file> -o <dir>` — convert

## Conventions
- Cross-refs are symbolic (`RefTarget::Internal`) until pass 4 resolves them to relative links.
- Adapters must never trust input: guard decompression ratio/size and entry-name traversal.
- Every change ships green under `mise run lint && mise run test`.
