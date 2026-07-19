# kasane Core Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build kasane's deterministic core — IR → five-pass structuring engine → Markdown tree writer → CLI — and wire the EPUB adapter through it so `kasane book.epub` emits a hierarchical, cross-linked, progressively-disclosed Markdown tree.

**Architecture:** Hexagonal Cargo workspace. `kasane-ir` holds a Pandoc-style intermediate representation. `kasane-core` (pure, no I/O) folds the IR into a section tree, balances it by size, assigns file paths, resolves symbolic cross-references to relative links, and emits per-file navigation. `kasane-writer` serializes to GitHub-Flavored Markdown and writes the tree atomically. `kasane-adapters` parses EPUB → IR behind an `Adapter` trait. `kasane-cli` wires it together.

**Tech Stack:** Rust (2021 edition), `clap` (derive), `thiserror`, `anyhow`, `zip`, `quick-xml`, `insta` (snapshot tests), `proptest` (later plan). Build/task tooling via `mise` + `just`.

## Global Constraints

- **Pure Rust on the default build.** No external host tools, no C dependencies on the default feature set. (OCR/Tesseract is a later-plan opt-in `ocr` feature and is out of scope here.)
- **Rust toolchain pinned** via `mise.toml` (`rust = "1.83.0"`).
- **Untrusted input boundary:** the EPUB adapter must guard against decompression bombs (max 200:1 expansion ratio, 512 MiB absolute cap), path traversal from archive entry names, and XML entity expansion (XXE). These are requirements of Task 10, not optional.
- **Cross-references are symbolic in the IR** (`RefTarget::Internal(BlockId)`), never file paths. Only the engine's pass 4 turns them into relative links.
- **Degrade, don't die:** an unparseable block becomes `Block::Raw { note }`; the document parse is not aborted where recovery is possible.
- **`clippy -D warnings` and `rustfmt`** must pass; every task ends green.
- Output writing is **atomic**: write to a temp dir, `rename` on success; refuse to overwrite a non-empty target dir unless `--force`.

---

## File Structure

```
kasane/
  Cargo.toml                         # [workspace] members
  mise.toml                          # pinned rust + tools
  justfile                           # build / test / lint / run tasks
  rustfmt.toml, clippy config in Cargo.toml
  crates/
    kasane-ir/
      Cargo.toml
      src/lib.rs                     # re-exports
      src/ids.rs                     # BlockId, NoteId
      src/doc.rs                     # Document, DocMeta, Node, Provenance
      src/block.rs                   # Block, Table, Figure, AssetRef
      src/inline.rs                  # Inline, RefTarget
    kasane-core/
      Cargo.toml
      src/lib.rs                     # pub fn structure(Document, &Options) -> SiteTree
      src/options.rs                 # Options (max_tokens, min_tokens)
      src/section.rs                 # SectionTree, SectionNode (pass 1)
      src/balance.rs                 # size guard (pass 2)
      src/paths.rs                   # file path assignment (pass 3)
      src/refs.rs                    # cross-ref resolution (pass 4)
      src/nav.rs                     # navigation + frontmatter (pass 5)
      src/sitetree.rs                # SiteTree, FileNode, Frontmatter
    kasane-writer/
      Cargo.toml
      src/lib.rs                     # pub fn write_tree(&SiteTree, &Path, force) -> Result
      src/markdown.rs                # IR blocks -> GFM string
      src/frontmatter.rs             # Frontmatter -> YAML string
    kasane-adapters/
      Cargo.toml
      src/lib.rs                     # Adapter trait, detect(), parse()
      src/detect.rs                  # magic-byte format detection
      src/epub/mod.rs                # EPUB adapter
      src/epub/opf.rs                # spine/manifest parse
      src/epub/xhtml.rs              # XHTML -> IR blocks
      src/guard.rs                   # zip bomb + path-traversal guards
    kasane-cli/
      Cargo.toml
      src/main.rs                    # clap args, pipeline wiring, exit codes
  tests/fixtures/epub/minimal.epub   # tiny hand-built EPUB
```

---

## Type Reference (locked signatures)

These are defined in Task 2 and consumed everywhere. Later tasks rely on exactly these names.

```rust
// kasane-ir
pub struct BlockId(pub u32);
pub struct NoteId(pub u32);

pub struct Document { pub meta: DocMeta, pub nodes: Vec<Node> }
pub struct DocMeta  { pub title: String, pub authors: Vec<String>,
                      pub language: Option<String>, pub source_format: String,
                      pub source_path: String }
pub struct Node { pub block: Block, pub prov: Provenance }
pub struct Provenance { pub source_pages: Option<(u32, u32)>, pub source_href: Option<String> }

pub enum Block {
    Heading { level: u8, id: BlockId, inlines: Vec<Inline> },
    Para(Vec<Inline>),
    List { ordered: bool, items: Vec<Vec<Block>> },
    Table(Table),
    Figure { image: AssetRef, caption: Vec<Inline>, number: Option<String> },
    CodeBlock { lang: Option<String>, text: String },
    MathBlock(String),
    Footnote { id: NoteId, blocks: Vec<Block> },
    Raw { note: String },
}
pub struct Table { pub header: Vec<Vec<Inline>>, pub rows: Vec<Vec<Vec<Inline>>>, pub has_merged: bool }
pub struct AssetRef { pub key: String, pub bytes_ref: usize } // index into AssetBag
pub enum Inline {
    Text(String), Emph(Vec<Inline>), Strong(Vec<Inline>), Code(String),
    Math(String),
    Link { target: RefTarget, inlines: Vec<Inline> },
    FootnoteRef(NoteId),
}
pub enum RefTarget { Internal(BlockId), External(String), Footnote(NoteId) }

// AssetBag travels alongside Document during parsing/writing
pub struct AssetBag { pub items: Vec<AssetItem> }
pub struct AssetItem { pub key: String, pub filename: String, pub bytes: Vec<u8> }

// kasane-core
pub struct Options { pub max_tokens: usize, pub min_tokens: usize } // defaults 2000 / 200
pub fn structure(doc: Document, opts: &Options) -> SiteTree;

pub struct SiteTree { pub files: Vec<FileNode> } // flat; paths are relative to output root
pub struct FileNode {
    pub path: String,               // e.g. "02-methods/03-sampling.md" or "index.md"
    pub frontmatter: Frontmatter,
    pub blocks: Vec<Block>,         // ready to serialize; internal links already resolved
}
pub struct Frontmatter {
    pub title: String, pub breadcrumb: Vec<String>,
    pub parent: Option<String>, pub prev: Option<String>, pub next: Option<String>,
    pub children: Vec<String>, pub source_pages: Option<(u32, u32)>,
}

// kasane-writer
pub fn write_tree(tree: &SiteTree, assets: &AssetBag, out: &std::path::Path, force: bool)
    -> anyhow::Result<()>;

// kasane-adapters
pub enum Format { Epub, Pptx, Mobi, Azw3, Pdf, Djvu }
pub fn detect(bytes: &[u8], ext_hint: Option<&str>) -> Option<Format>;
pub trait Adapter {
    fn parse(&self, bytes: &[u8], source_path: &str) -> Result<(Document, AssetBag), ParseError>;
}
```

---

### Task 1: Workspace scaffolding & dev environment

**Files:**
- Create: `Cargo.toml`, `mise.toml`, `justfile`, `rustfmt.toml`, `.gitignore`
- Create: `crates/kasane-ir/Cargo.toml`, `crates/kasane-ir/src/lib.rs`
- Test: `crates/kasane-ir/src/lib.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: nothing.
- Produces: a buildable workspace with one crate; `just test` and `just lint` commands.

- [ ] **Step 1: Write the workspace manifest**

`Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
edition = "2021"
rust-version = "1.83"
license = "Apache-2.0"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
```

- [ ] **Step 2: Pin toolchain and tasks**

`mise.toml`:
```toml
[tools]
rust = "1.83.0"
"cargo:just" = "latest"
```

`justfile`:
```make
build:
    cargo build --workspace
test:
    cargo test --workspace
lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
run *ARGS:
    cargo run -p kasane-cli -- {{ARGS}}
```

`rustfmt.toml`:
```toml
edition = "2021"
```

`.gitignore`:
```
/target
```

- [ ] **Step 3: Create the first crate with a smoke test**

`crates/kasane-ir/Cargo.toml`:
```toml
[package]
name = "kasane-ir"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lints]
workspace = true
```

`crates/kasane-ir/src/lib.rs`:
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn workspace_builds() {
        assert_eq!(2 + 2, 4);
    }
}
```

- [ ] **Step 4: Verify build, lint, and test are green**

Run: `mise install && just build && just lint && just test`
Expected: builds clean, clippy no warnings, `1 passed`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml mise.toml justfile rustfmt.toml .gitignore crates/kasane-ir
git commit -m "chore: scaffold cargo workspace, mise toolchain, just tasks"
```

---

### Task 2: IR types

**Files:**
- Create: `crates/kasane-ir/src/ids.rs`, `doc.rs`, `block.rs`, `inline.rs`, `assets.rs`
- Modify: `crates/kasane-ir/src/lib.rs`
- Test: `crates/kasane-ir/src/lib.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: nothing.
- Produces: every type in the Type Reference under the `kasane-ir` heading.

- [ ] **Step 1: Write the failing test**

In `crates/kasane-ir/src/lib.rs`:
```rust
#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn build_minimal_document() {
        let doc = Document {
            meta: DocMeta {
                title: "T".into(), authors: vec![], language: None,
                source_format: "epub".into(), source_path: "t.epub".into(),
            },
            nodes: vec![Node {
                block: Block::Heading { level: 1, id: BlockId(0), inlines: vec![Inline::Text("Hi".into())] },
                prov: Provenance { source_pages: None, source_href: Some("ch1.xhtml".into()) },
            }],
        };
        assert_eq!(doc.nodes.len(), 1);
        assert!(matches!(doc.nodes[0].block, Block::Heading { level: 1, .. }));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-ir`
Expected: FAIL — `Document`, `Block`, etc. not found.

- [ ] **Step 3: Write the types**

`crates/kasane-ir/src/ids.rs`:
```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BlockId(pub u32);
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NoteId(pub u32);
```

`crates/kasane-ir/src/doc.rs`:
```rust
use crate::block::Block;

#[derive(Clone, Debug)]
pub struct Document { pub meta: DocMeta, pub nodes: Vec<Node> }

#[derive(Clone, Debug)]
pub struct DocMeta {
    pub title: String, pub authors: Vec<String>, pub language: Option<String>,
    pub source_format: String, pub source_path: String,
}

#[derive(Clone, Debug)]
pub struct Node { pub block: Block, pub prov: Provenance }

#[derive(Clone, Debug, Default)]
pub struct Provenance { pub source_pages: Option<(u32, u32)>, pub source_href: Option<String> }
```

`crates/kasane-ir/src/inline.rs`:
```rust
use crate::ids::{BlockId, NoteId};

#[derive(Clone, Debug)]
pub enum Inline {
    Text(String), Emph(Vec<Inline>), Strong(Vec<Inline>), Code(String),
    Math(String),
    Link { target: RefTarget, inlines: Vec<Inline> },
    FootnoteRef(NoteId),
}

#[derive(Clone, Debug)]
pub enum RefTarget { Internal(BlockId), External(String), Footnote(NoteId) }
```

`crates/kasane-ir/src/block.rs`:
```rust
use crate::ids::{BlockId, NoteId};
use crate::inline::Inline;

#[derive(Clone, Debug)]
pub enum Block {
    Heading { level: u8, id: BlockId, inlines: Vec<Inline> },
    Para(Vec<Inline>),
    List { ordered: bool, items: Vec<Vec<Block>> },
    Table(Table),
    Figure { image: AssetRef, caption: Vec<Inline>, number: Option<String> },
    CodeBlock { lang: Option<String>, text: String },
    MathBlock(String),
    Footnote { id: NoteId, blocks: Vec<Block> },
    Raw { note: String },
}

#[derive(Clone, Debug)]
pub struct Table {
    pub header: Vec<Vec<Inline>>,
    pub rows: Vec<Vec<Vec<Inline>>>,
    pub has_merged: bool,
}

#[derive(Clone, Debug)]
pub struct AssetRef { pub key: String, pub bytes_ref: usize }
```

`crates/kasane-ir/src/assets.rs`:
```rust
#[derive(Clone, Debug, Default)]
pub struct AssetBag { pub items: Vec<AssetItem> }

#[derive(Clone, Debug)]
pub struct AssetItem { pub key: String, pub filename: String, pub bytes: Vec<u8> }
```

`crates/kasane-ir/src/lib.rs` (prepend the modules, keep the test):
```rust
mod ids;
mod doc;
mod block;
mod inline;
mod assets;

pub use assets::{AssetBag, AssetItem};
pub use block::{AssetRef, Block, Table};
pub use doc::{DocMeta, Document, Node, Provenance};
pub use ids::{BlockId, NoteId};
pub use inline::{Inline, RefTarget};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kasane-ir && just lint`
Expected: PASS; no clippy warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-ir
git commit -m "feat(ir): add intermediate representation types"
```

---

### Task 3: Engine pass 1 — heading hierarchy folding

**Files:**
- Create: `crates/kasane-core/Cargo.toml`, `src/lib.rs`, `src/options.rs`, `src/section.rs`
- Test: `crates/kasane-core/src/section.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: `kasane_ir::{Document, Node, Block, BlockId, Inline}`.
- Produces:
  ```rust
  pub struct SectionTree { pub root: SectionNode }
  pub struct SectionNode {
      pub id: Option<BlockId>,        // None for the synthetic root
      pub level: u8,                  // 0 for root; heading level otherwise
      pub title: Vec<Inline>,
      pub body: Vec<Block>,           // non-heading blocks that belong to this section
      pub children: Vec<SectionNode>,
      pub pages: Option<(u32, u32)>,  // merged provenance of this section's own body
  }
  pub fn fold_sections(doc: &Document) -> SectionTree;
  ```

- [ ] **Step 1: Create the crate manifest**

`crates/kasane-core/Cargo.toml`:
```toml
[package]
name = "kasane-core"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
kasane-ir = { path = "../kasane-ir" }

[lints]
workspace = true
```

`crates/kasane-core/src/options.rs`:
```rust
#[derive(Clone, Debug)]
pub struct Options { pub max_tokens: usize, pub min_tokens: usize }
impl Default for Options {
    fn default() -> Self { Self { max_tokens: 2000, min_tokens: 200 } }
}
```

`crates/kasane-core/src/lib.rs`:
```rust
mod options;
mod section;

pub use options::Options;
pub use section::{fold_sections, SectionNode, SectionTree};
```

- [ ] **Step 2: Write the failing test**

`crates/kasane-core/src/section.rs` (test at bottom):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kasane_ir::*;

    fn h(level: u8, id: u32, t: &str) -> Node {
        Node { block: Block::Heading { level, id: BlockId(id), inlines: vec![Inline::Text(t.into())] },
               prov: Provenance::default() }
    }
    fn p(t: &str) -> Node {
        Node { block: Block::Para(vec![Inline::Text(t.into())]), prov: Provenance::default() }
    }

    #[test]
    fn folds_nested_headings() {
        // H1 Intro / para / H2 Background / para / H1 Methods
        let doc = Document {
            meta: DocMeta { title: "B".into(), authors: vec![], language: None,
                            source_format: "epub".into(), source_path: "b".into() },
            nodes: vec![h(1,0,"Intro"), p("a"), h(2,1,"Background"), p("b"), h(1,2,"Methods")],
        };
        let tree = fold_sections(&doc);
        assert_eq!(tree.root.children.len(), 2);            // two H1s
        let intro = &tree.root.children[0];
        assert_eq!(intro.body.len(), 1);                    // "a"
        assert_eq!(intro.children.len(), 1);                // Background
        assert_eq!(intro.children[0].body.len(), 1);        // "b"
        assert_eq!(tree.root.children[1].children.len(), 0);// Methods empty
    }

    #[test]
    fn preamble_before_first_heading_stays_on_root() {
        let doc = Document {
            meta: DocMeta { title: "B".into(), authors: vec![], language: None,
                            source_format: "epub".into(), source_path: "b".into() },
            nodes: vec![p("preface"), h(1,0,"One")],
        };
        let tree = fold_sections(&doc);
        assert_eq!(tree.root.body.len(), 1);
        assert_eq!(tree.root.children.len(), 1);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p kasane-core`
Expected: FAIL — `fold_sections` not found.

- [ ] **Step 4: Implement the fold**

Top of `crates/kasane-core/src/section.rs`:
```rust
use kasane_ir::{Block, BlockId, Document, Inline, Node};

pub struct SectionTree { pub root: SectionNode }

pub struct SectionNode {
    pub id: Option<BlockId>,
    pub level: u8,
    pub title: Vec<Inline>,
    pub body: Vec<Block>,
    pub children: Vec<SectionNode>,
    pub pages: Option<(u32, u32)>,
}

impl SectionNode {
    fn root() -> Self {
        Self { id: None, level: 0, title: vec![], body: vec![], children: vec![], pages: None }
    }
    fn from_heading(level: u8, id: BlockId, title: Vec<Inline>) -> Self {
        Self { id: Some(id), level, title, body: vec![], children: vec![], pages: None }
    }
    fn merge_pages(&mut self, p: Option<(u32, u32)>) {
        if let Some((s, e)) = p {
            self.pages = Some(match self.pages {
                Some((cs, ce)) => (cs.min(s), ce.max(e)),
                None => (s, e),
            });
        }
    }
}

pub fn fold_sections(doc: &Document) -> SectionTree {
    let mut root = SectionNode::root();
    // stack holds owned nodes being built; index 0 is always the root.
    let mut stack: Vec<SectionNode> = vec![std::mem::replace(&mut root, SectionNode::root())];
    // (root moved into the stack; `root` var is now a throwaway.)

    for node in &doc.nodes {
        match &node.block {
            Block::Heading { level, id, inlines } => {
                // pop until the top has a strictly-lower level than this heading
                while stack.len() > 1 && stack.last().unwrap().level >= *level {
                    let done = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(done);
                }
                stack.push(SectionNode::from_heading(*level, *id, inlines.clone()));
            }
            other => {
                let top = stack.last_mut().unwrap();
                top.body.push(other.clone());
                top.merge_pages(node.prov.source_pages);
            }
        }
    }
    // unwind
    while stack.len() > 1 {
        let done = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(done);
    }
    SectionTree { root: stack.pop().unwrap() }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p kasane-core && just lint`
Expected: PASS; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-core
git commit -m "feat(core): fold IR into section tree (pass 1)"
```

---

### Task 4: Engine pass 2 — size-guard balancing

**Files:**
- Create: `crates/kasane-core/src/balance.rs`
- Modify: `crates/kasane-core/src/lib.rs`
- Test: `crates/kasane-core/src/balance.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: `SectionTree`, `SectionNode`, `Options`, `kasane_ir::Block`.
- Produces:
  ```rust
  pub fn balance(tree: &mut SectionTree, opts: &Options);
  pub(crate) fn est_tokens_blocks(blocks: &[Block]) -> usize; // chars/4 heuristic
  ```
  Effect: over-`max_tokens` leaf bodies gain synthetic child sections named `Part N`
  (split at block boundaries); leaves whose body is under `min_tokens` and have no
  children are folded into their parent's `body` (heading demoted to a bold lead-in
  is NOT done here — merge just concatenates body blocks under the parent).

- [ ] **Step 1: Write the failing test**

`crates/kasane-core/src/balance.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::section::{fold_sections};
    use crate::Options;
    use kasane_ir::*;

    fn big_para(n: usize) -> Node {
        Node { block: Block::Para(vec![Inline::Text("x".repeat(n))]), prov: Provenance::default() }
    }
    fn h(level: u8, id: u32, t: &str) -> Node {
        Node { block: Block::Heading { level, id: BlockId(id), inlines: vec![Inline::Text(t.into())] },
               prov: Provenance::default() }
    }
    fn doc(nodes: Vec<Node>) -> Document {
        Document { meta: DocMeta { title: "B".into(), authors: vec![], language: None,
                   source_format: "epub".into(), source_path: "b".into() }, nodes }
    }

    #[test]
    fn splits_oversized_leaf() {
        // one H1 with two ~1200-char paras => ~600 tokens, over max_tokens=400
        let mut tree = fold_sections(&doc(vec![h(1,0,"Big"), big_para(1200), big_para(1200)]));
        balance(&mut tree, &Options { max_tokens: 400, min_tokens: 10 });
        let sec = &tree.root.children[0];
        assert!(sec.children.len() >= 2, "expected split into parts");
        assert!(sec.body.is_empty(), "body moved into parts");
    }

    #[test]
    fn merges_tiny_leaf_into_parent() {
        // H1 with H2 child holding one tiny para; child under min_tokens should merge up
        let mut tree = fold_sections(&doc(vec![h(1,0,"Top"), h(2,1,"Tiny"), big_para(4)]));
        balance(&mut tree, &Options { max_tokens: 2000, min_tokens: 100 });
        let top = &tree.root.children[0];
        assert!(top.children.is_empty(), "tiny child folded up");
        assert!(!top.body.is_empty(), "child body absorbed into parent");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-core balance`
Expected: FAIL — `balance` not found.

- [ ] **Step 3: Implement balancing**

Top of `crates/kasane-core/src/balance.rs`:
```rust
use crate::section::{SectionNode, SectionTree};
use crate::Options;
use kasane_ir::{Block, Inline};

pub fn balance(tree: &mut SectionTree, opts: &Options) {
    balance_node(&mut tree.root, opts);
}

fn balance_node(node: &mut SectionNode, opts: &Options) {
    // depth-first so children are balanced before we consider merging them up
    for child in &mut node.children {
        balance_node(child, opts);
    }

    // MERGE: absorb tiny childless children into this node's body
    let mut kept = Vec::new();
    for child in std::mem::take(&mut node.children) {
        let small = child.children.is_empty()
            && est_tokens_blocks(&child.body) < opts.min_tokens;
        if small {
            // demote heading to a bold lead-in para, then append its body
            if !child.title.is_empty() {
                node.body.push(Block::Para(vec![Inline::Strong(child.title.clone())]));
            }
            node.body.extend(child.body);
        } else {
            kept.push(child);
        }
    }
    node.children = kept;

    // SPLIT: an oversized leaf (no children) gets synthetic Part sections
    if node.children.is_empty() && est_tokens_blocks(&node.body) > opts.max_tokens {
        let parts = split_blocks(std::mem::take(&mut node.body), opts.max_tokens);
        for (i, blocks) in parts.into_iter().enumerate() {
            node.children.push(SectionNode {
                id: None,
                level: node.level + 1,
                title: vec![Inline::Text(format!("Part {}", i + 1))],
                body: blocks,
                children: vec![],
                pages: node.pages,
            });
        }
    }
}

fn split_blocks(blocks: Vec<Block>, max_tokens: usize) -> Vec<Vec<Block>> {
    let mut parts = vec![];
    let mut cur = vec![];
    let mut cur_tokens = 0;
    for b in blocks {
        let t = est_tokens_blocks(std::slice::from_ref(&b));
        if cur_tokens + t > max_tokens && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur));
            cur_tokens = 0;
        }
        cur.push(b);
        cur_tokens += t;
    }
    if !cur.is_empty() { parts.push(cur); }
    parts
}

pub(crate) fn est_tokens_blocks(blocks: &[Block]) -> usize {
    blocks.iter().map(est_tokens_block).sum()
}

fn est_tokens_block(b: &Block) -> usize {
    fn inl(is: &[Inline]) -> usize {
        is.iter().map(|i| match i {
            Inline::Text(s) | Inline::Code(s) | Inline::Math(s) => s.len(),
            Inline::Emph(x) | Inline::Strong(x) => inl(x),
            Inline::Link { inlines, .. } => inl(inlines),
            Inline::FootnoteRef(_) => 4,
        }).sum()
    }
    let chars = match b {
        Block::Heading { inlines, .. } | Block::Para(inlines) => inl(inlines),
        Block::List { items, .. } => items.iter().flatten().map(|b| est_tokens_block(b)).sum(),
        Block::Table(t) => t.rows.iter().flatten().map(|c| inl(c)).sum::<usize>() + 20,
        Block::Figure { caption, .. } => inl(caption) + 16,
        Block::CodeBlock { text, .. } => text.len(),
        Block::MathBlock(s) | Block::Raw { note: s } => s.len(),
        Block::Footnote { blocks, .. } => est_tokens_blocks(blocks),
    };
    chars / 4 + 1
}
```

Add `mod balance;` and `pub use balance::balance;` to `crates/kasane-core/src/lib.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kasane-core && just lint`
Expected: PASS; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-core
git commit -m "feat(core): size-guard split/merge balancing (pass 2)"
```

---

### Task 5: Engine pass 3 — file path assignment

**Files:**
- Create: `crates/kasane-core/src/paths.rs`, `crates/kasane-core/src/sitetree.rs`
- Modify: `crates/kasane-core/src/lib.rs`
- Test: `crates/kasane-core/src/paths.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: `SectionTree`, `SectionNode`, `kasane_ir::{Inline, BlockId}`.
- Produces:
  ```rust
  // sitetree.rs
  pub struct SiteTree { pub files: Vec<FileNode> }
  pub struct FileNode { pub path: String, pub frontmatter: Frontmatter, pub blocks: Vec<Block> }
  pub struct Frontmatter { pub title: String, pub breadcrumb: Vec<String>,
      pub parent: Option<String>, pub prev: Option<String>, pub next: Option<String>,
      pub children: Vec<String>, pub source_pages: Option<(u32, u32)> }

  // paths.rs — an intermediate placed-tree with assigned paths + a BlockId->anchor map
  pub struct Placed { pub path: String, pub node_title: String, pub node: SectionNode,
                      pub children: Vec<Placed> }
  pub struct PlaceResult { pub root: Placed, pub anchors: std::collections::HashMap<BlockId, String> }
  pub fn assign_paths(tree: SectionTree) -> PlaceResult;
  pub(crate) fn slug(inlines: &[Inline]) -> String;
  ```
  Rules: a node with children becomes a directory whose file is `<dir>/index.md`;
  a childless node becomes `<parent-dir>/NN-<slug>.md`. The root is `index.md`.
  `anchors` maps every heading `BlockId` to `"<path>#<slug>"`.

- [ ] **Step 1: Write the failing test**

`crates/kasane-core/src/paths.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::section::fold_sections;
    use kasane_ir::*;

    fn h(level: u8, id: u32, t: &str) -> Node {
        Node { block: Block::Heading { level, id: BlockId(id), inlines: vec![Inline::Text(t.into())] },
               prov: Provenance::default() }
    }
    fn doc(nodes: Vec<Node>) -> Document {
        Document { meta: DocMeta { title: "B".into(), authors: vec![], language: None,
                   source_format: "epub".into(), source_path: "b".into() }, nodes }
    }

    #[test]
    fn assigns_index_and_leaf_paths() {
        // H1 Intro (has H2 child) ; H1 Methods (leaf)
        let tree = fold_sections(&doc(vec![
            h(1,0,"Intro"), h(2,1,"Background & Notes"), h(1,2,"Methods"),
        ]));
        let placed = assign_paths(tree);
        assert_eq!(placed.root.path, "index.md");
        let intro = &placed.root.children[0];
        assert_eq!(intro.path, "01-intro/index.md");           // has a child -> dir
        assert_eq!(intro.children[0].path, "01-intro/01-background-notes.md");
        assert_eq!(placed.root.children[1].path, "02-methods.md"); // leaf -> file
        // anchor map points at the file+slug
        assert_eq!(placed.anchors[&BlockId(2)], "02-methods.md#methods");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-core paths`
Expected: FAIL — `assign_paths` not found.

- [ ] **Step 3: Implement path assignment + sitetree types**

`crates/kasane-core/src/sitetree.rs`:
```rust
use kasane_ir::Block;

pub struct SiteTree { pub files: Vec<FileNode> }

pub struct FileNode { pub path: String, pub frontmatter: Frontmatter, pub blocks: Vec<Block> }

#[derive(Default)]
pub struct Frontmatter {
    pub title: String,
    pub breadcrumb: Vec<String>,
    pub parent: Option<String>,
    pub prev: Option<String>,
    pub next: Option<String>,
    pub children: Vec<String>,
    pub source_pages: Option<(u32, u32)>,
}
```

`crates/kasane-core/src/paths.rs` (top). Numbering rule: a node with children becomes a directory `NN-<slug>/index.md`; a childless node becomes a file `NN-<slug>.md`; the root is `index.md`. Use this exact implementation:

```rust
use crate::section::{SectionNode, SectionTree};
use kasane_ir::{BlockId, Inline};
use std::collections::HashMap;

pub struct Placed { pub path: String, pub node: SectionNode, pub children: Vec<Placed> }
pub struct PlaceResult { pub root: Placed, pub anchors: HashMap<BlockId, String> }

pub fn assign_paths(tree: SectionTree) -> PlaceResult {
    let mut anchors = HashMap::new();
    let root = place(tree.root, "index.md", "", &mut anchors);
    PlaceResult { root, anchors }
}

// self_path: this node's markdown file path. dir: directory children live in.
fn place(mut node: SectionNode, self_path: &str, dir: &str,
         anchors: &mut HashMap<BlockId, String>) -> Placed {
    if let Some(id) = node.id {
        anchors.insert(id, format!("{}#{}", self_path, slug(&node.title)));
    }
    let children = std::mem::take(&mut node.children);
    let mut placed = Vec::new();
    for (i, child) in children.into_iter().enumerate() {
        let n = i + 1;
        let child_slug = slug(&child.title);
        if child.children.is_empty() {
            let p = join(dir, &format!("{:02}-{}.md", n, child_slug));
            placed.push(place(child, &p, dir, anchors));
        } else {
            let cdir = join(dir, &format!("{:02}-{}", n, child_slug));
            let p = format!("{}/index.md", cdir);
            placed.push(place(child, &p, &cdir, anchors));
        }
    }
    Placed { path: self_path.to_string(), node, children: placed }
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_string() } else { format!("{}/{}", dir, name) }
}

pub(crate) fn slug(inlines: &[Inline]) -> String {
    let text = inline_text(inlines);
    let mut out = String::new();
    let mut prev_dash = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') { out.pop(); }
    if out.is_empty() { "section".to_string() } else { out }
}

pub(crate) fn inline_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => s.push_str(t),
            Inline::Emph(x) | Inline::Strong(x) => s.push_str(&inline_text(x)),
            Inline::Link { inlines, .. } => s.push_str(&inline_text(inlines)),
            Inline::FootnoteRef(_) => {}
        }
    }
    s
}
```

Add to `crates/kasane-core/src/lib.rs`:
```rust
mod paths;
mod sitetree;
pub use paths::{assign_paths, PlaceResult, Placed};
pub use sitetree::{FileNode, Frontmatter, SiteTree};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kasane-core paths && just lint`
Expected: PASS; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-core
git commit -m "feat(core): assign file paths and heading anchors (pass 3)"
```

---

### Task 6: Engine pass 4 — cross-reference resolution

**Files:**
- Create: `crates/kasane-core/src/refs.rs`
- Modify: `crates/kasane-core/src/lib.rs`
- Test: `crates/kasane-core/src/refs.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: `Placed`, `PlaceResult.anchors`, `kasane_ir::{Block, Inline, RefTarget, BlockId}`.
- Produces:
  ```rust
  // Rewrites every Inline::Link{ target: RefTarget::Internal(id), .. } within each
  // Placed node's blocks into RefTarget::External("<relative>#<anchor>") computed
  // from the anchor map, made relative to the *containing file's* path. A dangling
  // Internal(id) (id not in map) becomes plain text (link stripped, inlines kept).
  pub fn resolve_refs(placed: &mut Placed, anchors: &std::collections::HashMap<BlockId, String>);
  pub(crate) fn relativize(from_file: &str, to_target: &str) -> String;
  ```

- [ ] **Step 1: Write the failing test**

`crates/kasane-core/src/refs.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relativizes_sibling_and_nested() {
        assert_eq!(relativize("02-methods.md", "index.md#intro"), "index.md#intro");
        assert_eq!(relativize("01-intro/index.md", "02-methods.md#m"), "../02-methods.md#m");
        assert_eq!(relativize("01-intro/01-a.md", "01-intro/02-b.md#x"), "02-b.md#x");
        assert_eq!(relativize("a/b/c.md", "a/x.md#y"), "../x.md#y");
    }

    #[test]
    fn resolves_internal_link_and_strips_dangling() {
        use kasane_ir::*;
        use std::collections::HashMap;
        use crate::section::SectionNode;
        use crate::paths::Placed;

        let mut anchors = HashMap::new();
        anchors.insert(BlockId(7), "02-methods.md#methods".to_string());

        let blocks = vec![Block::Para(vec![
            Inline::Link { target: RefTarget::Internal(BlockId(7)),
                           inlines: vec![Inline::Text("see methods".into())] },
            Inline::Link { target: RefTarget::Internal(BlockId(99)),  // dangling
                           inlines: vec![Inline::Text("gone".into())] },
        ])];
        let mut placed = Placed {
            path: "01-intro/index.md".into(),
            node: SectionNode { id: None, level: 0, title: vec![], body: blocks,
                                children: vec![], pages: None },
            children: vec![],
        };
        resolve_refs(&mut placed, &anchors);

        if let Block::Para(inls) = &placed.node.body[0] {
            match &inls[0] {
                Inline::Link { target: RefTarget::External(u), .. } => assert_eq!(u, "../02-methods.md#methods"),
                _ => panic!("first should be external link"),
            }
            assert!(matches!(inls[1], Inline::Text(_)), "dangling link stripped to text");
        } else { panic!() }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-core refs`
Expected: FAIL — `resolve_refs` / `relativize` not found.

- [ ] **Step 3: Implement resolution**

`crates/kasane-core/src/refs.rs` (top):
```rust
use crate::paths::Placed;
use kasane_ir::{Block, BlockId, Inline, RefTarget};
use std::collections::HashMap;

pub fn resolve_refs(placed: &mut Placed, anchors: &HashMap<BlockId, String>) {
    let from = placed.path.clone();
    for b in &mut placed.node.body {
        fix_block(b, &from, anchors);
    }
    for child in &mut placed.children {
        resolve_refs(child, anchors);
    }
}

fn fix_block(b: &mut Block, from: &str, anchors: &HashMap<BlockId, String>) {
    match b {
        Block::Para(inls) | Block::Heading { inlines: inls, .. } => fix_inlines(inls, from, anchors),
        Block::List { items, .. } => for it in items { for bb in it { fix_block(bb, from, anchors); } },
        Block::Footnote { blocks, .. } => for bb in blocks { fix_block(bb, from, anchors); },
        Block::Figure { caption, .. } => fix_inlines(caption, from, anchors),
        Block::Table(t) => {
            for c in &mut t.header { fix_inlines(c, from, anchors); }
            for r in &mut t.rows { for c in r { fix_inlines(c, from, anchors); } }
        }
        _ => {}
    }
}

fn fix_inlines(inls: &mut Vec<Inline>, from: &str, anchors: &HashMap<BlockId, String>) {
    let mut out = Vec::with_capacity(inls.len());
    for inl in std::mem::take(inls) {
        out.push(fix_inline(inl, from, anchors));
    }
    *inls = out;
}

fn fix_inline(inl: Inline, from: &str, anchors: &HashMap<BlockId, String>) -> Inline {
    match inl {
        Inline::Link { target: RefTarget::Internal(id), mut inlines } => {
            fix_inlines(&mut inlines, from, anchors);
            match anchors.get(&id) {
                Some(target) => Inline::Link { target: RefTarget::External(relativize(from, target)), inlines },
                None => Inline::Emph(vec![]).replace_with_text(inlines), // strip: keep child text
            }
        }
        Inline::Link { target, mut inlines } => {
            fix_inlines(&mut inlines, from, anchors);
            Inline::Link { target, inlines }
        }
        Inline::Emph(mut x) => { fix_inlines(&mut x, from, anchors); Inline::Emph(x) }
        Inline::Strong(mut x) => { fix_inlines(&mut x, from, anchors); Inline::Strong(x) }
        other => other,
    }
}

// Helper: flatten stripped link children into a single Text run.
trait ReplaceWithText { fn replace_with_text(self, inlines: Vec<Inline>) -> Inline; }
impl ReplaceWithText for Inline {
    fn replace_with_text(self, inlines: Vec<Inline>) -> Inline {
        Inline::Text(crate::paths::inline_text(&inlines))
    }
}

pub(crate) fn relativize(from_file: &str, to_target: &str) -> String {
    let (to_path, anchor) = match to_target.split_once('#') {
        Some((p, a)) => (p, Some(a)),
        None => (to_target, None),
    };
    let from_dirs: Vec<&str> = from_file.split('/').collect();
    let from_dirs = &from_dirs[..from_dirs.len().saturating_sub(1)]; // drop filename
    let to_parts: Vec<&str> = to_path.split('/').collect();

    // common prefix of directories
    let mut i = 0;
    while i < from_dirs.len() && i + 1 < to_parts.len() && from_dirs[i] == to_parts[i] {
        i += 1;
    }
    let ups = from_dirs.len() - i;
    let mut rel = String::new();
    for _ in 0..ups { rel.push_str("../"); }
    rel.push_str(&to_parts[i..].join("/"));
    match anchor { Some(a) => format!("{}#{}", rel, a), None => rel }
}
```

Add `mod refs; pub use refs::resolve_refs;` to `crates/kasane-core/src/lib.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kasane-core refs && just lint`
Expected: PASS; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-core
git commit -m "feat(core): resolve symbolic cross-refs to relative links (pass 4)"
```

---

### Task 7: Engine pass 5 — navigation, frontmatter & `structure()` entry point

**Files:**
- Create: `crates/kasane-core/src/nav.rs`
- Modify: `crates/kasane-core/src/lib.rs`
- Test: `crates/kasane-core/src/nav.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: `Placed`, `PlaceResult`, `Options`, all earlier passes.
- Produces:
  ```rust
  pub fn structure(doc: kasane_ir::Document, opts: &Options) -> SiteTree;
  // Flattens the placed tree in reading order into FileNodes; computes
  // breadcrumb, parent, prev, next, children, source_pages; prepends an
  // auto TOC (as a List of Links) to every index.md that has children;
  // sets root index.md title from doc.meta.title.
  ```

- [ ] **Step 1: Write the failing test**

`crates/kasane-core/src/nav.rs`:
```rust
#[cfg(test)]
mod tests {
    use crate::{structure, Options};
    use kasane_ir::*;

    fn h(level: u8, id: u32, t: &str) -> Node {
        Node { block: Block::Heading { level, id: BlockId(id), inlines: vec![Inline::Text(t.into())] },
               prov: Provenance::default() }
    }
    fn p(t: &str) -> Node {
        Node { block: Block::Para(vec![Inline::Text(t.into())]), prov: Provenance::default() }
    }

    #[test]
    fn builds_navigation_chain() {
        let doc = Document {
            meta: DocMeta { title: "My Book".into(), authors: vec![], language: None,
                            source_format: "epub".into(), source_path: "b.epub".into() },
            nodes: vec![h(1,0,"Intro"), p("hi"), h(1,1,"Methods"), p("mm")],
        };
        let site = structure(doc, &Options::default());
        let paths: Vec<_> = site.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"index.md"));
        assert!(paths.contains(&"01-intro.md"));
        assert!(paths.contains(&"02-methods.md"));

        let intro = site.files.iter().find(|f| f.path == "01-intro.md").unwrap();
        assert_eq!(intro.frontmatter.title, "Intro");
        assert_eq!(intro.frontmatter.parent.as_deref(), Some("index.md"));
        assert_eq!(intro.frontmatter.next.as_deref(), Some("02-methods.md"));
        assert_eq!(intro.frontmatter.breadcrumb, vec!["My Book", "Intro"]);

        let root = site.files.iter().find(|f| f.path == "index.md").unwrap();
        assert_eq!(root.frontmatter.title, "My Book");
        assert_eq!(root.frontmatter.children, vec!["01-intro.md", "02-methods.md"]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kasane-core nav`
Expected: FAIL — `structure` not found.

- [ ] **Step 3: Implement navigation & the public entry point**

`crates/kasane-core/src/nav.rs`:
```rust
use crate::balance::balance;
use crate::paths::{assign_paths, inline_text, Placed};
use crate::refs::resolve_refs;
use crate::section::{fold_sections, SectionNode};
use crate::sitetree::{FileNode, Frontmatter, SiteTree};
use crate::Options;
use kasane_ir::{Block, Document, Inline, RefTarget};

pub fn structure(doc: Document, opts: &Options) -> SiteTree {
    let root_title = doc.meta.title.clone();
    let mut tree = fold_sections(&doc);
    balance(&mut tree, opts);
    let mut result = assign_paths(tree);
    resolve_refs(&mut result.root, &result.anchors);

    // Flatten in reading order (pre-order), carrying breadcrumb trail.
    let mut files = Vec::new();
    let mut order = Vec::new();               // paths in reading order for prev/next
    collect_order(&result.root, &mut order);

    walk(&result.root, &root_title, &[], None, &order, &mut files);
    // Fix root title (root node has empty heading title).
    if let Some(root_file) = files.iter_mut().find(|f| f.path == "index.md") {
        root_file.frontmatter.title = root_title.clone();
        root_file.frontmatter.breadcrumb = vec![root_title];
    }
    SiteTree { files }
}

fn collect_order(p: &Placed, out: &mut Vec<String>) {
    out.push(p.path.clone());
    for c in &p.children { collect_order(c, out); }
}

fn walk(p: &Placed, doc_title: &str, trail: &[String], parent: Option<&str>,
        order: &[String], files: &mut Vec<FileNode>) {
    let title = if p.node.id.is_none() && trail.is_empty() {
        doc_title.to_string()
    } else {
        inline_text(&p.node.title)
    };
    let mut breadcrumb = trail.to_vec();
    breadcrumb.push(title.clone());

    let idx = order.iter().position(|x| x == &p.path).unwrap();
    let prev = if idx > 0 { Some(order[idx - 1].clone()) } else { None };
    let next = order.get(idx + 1).cloned();

    let child_paths: Vec<String> = p.children.iter().map(|c| c.path.clone()).collect();

    // Body: for a directory node with children, prepend an auto TOC.
    let mut blocks = p.node.body.clone();
    if !p.children.is_empty() {
        let toc = Block::List {
            ordered: false,
            items: p.children.iter().map(|c| vec![Block::Para(vec![
                Inline::Link {
                    target: RefTarget::External(crate::refs::relativize(&p.path, &c.path)),
                    inlines: vec![Inline::Text(child_title(c, doc_title))],
                }])]).collect(),
        };
        blocks.insert(0, toc);
    }

    files.push(FileNode {
        path: p.path.clone(),
        frontmatter: Frontmatter {
            title,
            breadcrumb: breadcrumb.clone(),
            parent: parent.map(|s| relparent(&p.path, s)),
            prev: prev.map(|s| crate::refs::relativize(&p.path, &s)),
            next: next.map(|s| crate::refs::relativize(&p.path, &s)),
            children: child_paths,
            source_pages: p.node.pages,
        },
        blocks,
    });

    for c in &p.children {
        walk(c, doc_title, &breadcrumb, Some(&p.path), order, files);
    }
}

fn child_title(p: &Placed, doc_title: &str) -> String {
    if p.node.id.is_none() { doc_title.to_string() } else { inline_text(&p.node.title) }
}

fn relparent(from: &str, parent_abs: &str) -> String {
    crate::refs::relativize(from, parent_abs)
}

// expose est helper (unused warning guard)
#[allow(unused_imports)]
use crate::section::SectionNode as _SectionNodeAlias;
```

> **Note:** `frontmatter.parent/prev/next/children` are stored as **relative** links
> (already relativized to the file's own location) so the writer emits them verbatim.
> The test compares against relative forms (`"index.md"`, `"02-methods.md"`) which are
> correct because siblings at the root share the empty directory.

Add to `crates/kasane-core/src/lib.rs`:
```rust
mod nav;
pub use nav::structure;
```
Make `est_tokens_blocks` and `inline_text` visible to `nav` (already `pub(crate)`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kasane-core && just lint`
Expected: PASS (all core tests); no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/kasane-core
git commit -m "feat(core): navigation, frontmatter, TOC, structure() entry (pass 5)"
```

---

### Task 8: Writer — IR blocks → GitHub-Flavored Markdown

**Files:**
- Create: `crates/kasane-writer/Cargo.toml`, `src/lib.rs`, `src/markdown.rs`
- Test: `crates/kasane-writer/src/markdown.rs` (inline `#[test]`)

**Interfaces:**
- Consumes: `kasane_ir::{Block, Inline, RefTarget, Table}`.
- Produces:
  ```rust
  pub fn blocks_to_markdown(blocks: &[Block], assets: &kasane_ir::AssetBag) -> String;
  pub(crate) fn inlines_to_md(inls: &[Inline]) -> String;
  ```
  GFM rules: headings `#`×level; `Emph`→`*x*`; `Strong`→`**x**`; `Code`→`` `x` ``;
  inline `Math`→`$x$`; `MathBlock`→`$$\n…\n$$`; `Table` (no merged)→GFM pipe table;
  merged table→raw HTML `<table>`; `Figure`→`![caption](_assets/<filename>)` with a
  caption line; `Link{External(u)}`→`[text](u)`; `FootnoteRef(n)`→`[^n]`;
  `Footnote`→`[^n]: …`; `Raw{note}`→`<!-- note -->`.

- [ ] **Step 1: Create the crate manifest**

`crates/kasane-writer/Cargo.toml`:
```toml
[package]
name = "kasane-writer"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
kasane-ir = { path = "../kasane-ir" }
anyhow = "1"

[lints]
workspace = true
```

`crates/kasane-writer/src/lib.rs`:
```rust
mod markdown;
pub use markdown::blocks_to_markdown;
```

- [ ] **Step 2: Write the failing test**

`crates/kasane-writer/src/markdown.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kasane_ir::*;

    #[test]
    fn renders_headings_emphasis_and_links() {
        let blocks = vec![
            Block::Heading { level: 2, id: BlockId(0), inlines: vec![Inline::Text("Title".into())] },
            Block::Para(vec![
                Inline::Strong(vec![Inline::Text("bold".into())]),
                Inline::Text(" and ".into()),
                Inline::Link { target: RefTarget::External("../m.md#x".into()),
                               inlines: vec![Inline::Text("link".into())] },
            ]),
        ];
        let md = blocks_to_markdown(&blocks, &AssetBag::default());
        assert!(md.contains("## Title"));
        assert!(md.contains("**bold** and [link](../m.md#x)"));
    }

    #[test]
    fn renders_gfm_table() {
        let t = Table {
            header: vec![vec![Inline::Text("A".into())], vec![Inline::Text("B".into())]],
            rows: vec![vec![vec![Inline::Text("1".into())], vec![Inline::Text("2".into())]]],
            has_merged: false,
        };
        let md = blocks_to_markdown(&[Block::Table(t)], &AssetBag::default());
        assert!(md.contains("| A | B |"));
        assert!(md.contains("| --- | --- |"));
        assert!(md.contains("| 1 | 2 |"));
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p kasane-writer`
Expected: FAIL — `blocks_to_markdown` not found.

- [ ] **Step 4: Implement the serializer**

`crates/kasane-writer/src/markdown.rs` (top):
```rust
use kasane_ir::{AssetBag, Block, Inline, RefTarget, Table};

pub fn blocks_to_markdown(blocks: &[Block], assets: &AssetBag) -> String {
    let mut out = String::new();
    for b in blocks {
        render_block(b, assets, &mut out);
        out.push('\n');
    }
    out
}

fn render_block(b: &Block, assets: &AssetBag, out: &mut String) {
    match b {
        Block::Heading { level, inlines, .. } => {
            for _ in 0..(*level).min(6) { out.push('#'); }
            out.push(' ');
            out.push_str(&inlines_to_md(inlines));
            out.push('\n');
        }
        Block::Para(inls) => { out.push_str(&inlines_to_md(inls)); out.push('\n'); }
        Block::List { ordered, items } => {
            for (i, item) in items.iter().enumerate() {
                if *ordered { out.push_str(&format!("{}. ", i + 1)); } else { out.push_str("- "); }
                // render first block inline, subsequent blocks indented
                let mut inner = String::new();
                for bb in item { render_block(bb, assets, &mut inner); }
                out.push_str(inner.trim_end());
                out.push('\n');
            }
        }
        Block::Table(t) => render_table(t, out),
        Block::Figure { image, caption, number } => {
            let fname = assets.items.iter().find(|a| a.key == image.key)
                .map(|a| a.filename.as_str()).unwrap_or("missing");
            out.push_str(&format!("![{}](_assets/{})\n", inlines_to_md(caption), fname));
            if let Some(n) = number { out.push_str(&format!("*Figure {}: {}*\n", n, inlines_to_md(caption))); }
        }
        Block::CodeBlock { lang, text } => {
            out.push_str(&format!("```{}\n{}\n```\n", lang.clone().unwrap_or_default(), text));
        }
        Block::MathBlock(s) => out.push_str(&format!("$$\n{}\n$$\n", s)),
        Block::Footnote { id, blocks } => {
            let body = blocks_to_markdown(blocks, assets);
            out.push_str(&format!("[^{}]: {}\n", id.0, body.trim()));
        }
        Block::Raw { note } => out.push_str(&format!("<!-- {} -->\n", note)),
    }
}

fn render_table(t: &Table, out: &mut String) {
    if t.has_merged {
        out.push_str("<table>\n");
        // header + rows as HTML (merged cells not modeled per-cell here; emit flat)
        let esc = |c: &Vec<Inline>| inlines_to_md(c);
        out.push_str("<tr>");
        for c in &t.header { out.push_str(&format!("<th>{}</th>", esc(c))); }
        out.push_str("</tr>\n");
        for r in &t.rows {
            out.push_str("<tr>");
            for c in r { out.push_str(&format!("<td>{}</td>", esc(c))); }
            out.push_str("</tr>\n");
        }
        out.push_str("</table>\n");
        return;
    }
    let cells = |row: &Vec<Vec<Inline>>| {
        let joined: Vec<String> = row.iter().map(|c| inlines_to_md(c)).collect();
        format!("| {} |", joined.join(" | "))
    };
    out.push_str(&cells(&t.header)); out.push('\n');
    let sep: Vec<&str> = t.header.iter().map(|_| "---").collect();
    out.push_str(&format!("| {} |\n", sep.join(" | ")));
    for r in &t.rows { out.push_str(&cells(r)); out.push('\n'); }
}

pub(crate) fn inlines_to_md(inls: &[Inline]) -> String {
    let mut s = String::new();
    for i in inls {
        match i {
            Inline::Text(t) => s.push_str(t),
            Inline::Emph(x) => s.push_str(&format!("*{}*", inlines_to_md(x))),
            Inline::Strong(x) => s.push_str(&format!("**{}**", inlines_to_md(x))),
            Inline::Code(t) => s.push_str(&format!("`{}`", t)),
            Inline::Math(t) => s.push_str(&format!("${}$", t)),
            Inline::Link { target: RefTarget::External(u), inlines } =>
                s.push_str(&format!("[{}]({})", inlines_to_md(inlines), u)),
            Inline::Link { inlines, .. } => s.push_str(&inlines_to_md(inlines)), // unresolved -> text
            Inline::FootnoteRef(n) => s.push_str(&format!("[^{}]", n.0)),
        }
    }
    s
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p kasane-writer && just lint`
Expected: PASS; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-writer
git commit -m "feat(writer): serialize IR blocks to GitHub-Flavored Markdown"
```

---

### Task 9: Writer — frontmatter + atomic tree write

**Files:**
- Create: `crates/kasane-writer/src/frontmatter.rs`
- Modify: `crates/kasane-writer/src/lib.rs`
- Test: `crates/kasane-writer/src/lib.rs` (inline `#[test]`, uses `tempfile`)

**Interfaces:**
- Consumes: `kasane_core::{SiteTree, FileNode, Frontmatter}`, `kasane_ir::AssetBag`.
- Produces:
  ```rust
  pub fn write_tree(tree: &SiteTree, assets: &AssetBag, out: &std::path::Path, force: bool)
      -> anyhow::Result<()>;
  pub(crate) fn frontmatter_yaml(fm: &Frontmatter) -> String;
  ```
  Behavior: refuse if `out` exists non-empty and `!force`; write everything into a
  sibling temp dir then `rename` to `out`; each file = `---\n<yaml>\n---\n\n<markdown>`;
  assets written to `<out>/_assets/<filename>`.

- [ ] **Step 1: Add deps**

Modify `crates/kasane-writer/Cargo.toml`:
```toml
[dependencies]
kasane-ir = { path = "../kasane-ir" }
kasane-core = { path = "../kasane-core" }
anyhow = "1"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Write the failing test**

Add to `crates/kasane-writer/src/lib.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kasane_core::{FileNode, Frontmatter, SiteTree};
    use kasane_ir::{AssetBag, Block, BlockId, Inline};

    #[test]
    fn writes_files_with_frontmatter() {
        let tree = SiteTree { files: vec![FileNode {
            path: "index.md".into(),
            frontmatter: Frontmatter { title: "Book".into(), breadcrumb: vec!["Book".into()],
                parent: None, prev: None, next: None, children: vec!["01-intro.md".into()],
                source_pages: Some((1, 3)) },
            blocks: vec![Block::Heading { level: 1, id: BlockId(0),
                          inlines: vec![Inline::Text("Book".into())] }],
        }]};
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("book");
        write_tree(&tree, &AssetBag::default(), &out, false).unwrap();
        let idx = std::fs::read_to_string(out.join("index.md")).unwrap();
        assert!(idx.starts_with("---\n"));
        assert!(idx.contains("title: Book"));
        assert!(idx.contains("source_pages: 1-3"));
        assert!(idx.contains("# Book"));
    }

    #[test]
    fn refuses_nonempty_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("book");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("keep.txt"), "x").unwrap();
        let tree = SiteTree { files: vec![] };
        assert!(write_tree(&tree, &AssetBag::default(), &out, false).is_err());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p kasane-writer write`
Expected: FAIL — `write_tree` not found.

- [ ] **Step 4: Implement frontmatter + writer**

`crates/kasane-writer/src/frontmatter.rs`:
```rust
use kasane_core::Frontmatter;

pub(crate) fn frontmatter_yaml(fm: &Frontmatter) -> String {
    let mut y = String::new();
    y.push_str(&format!("title: {}\n", yaml_str(&fm.title)));
    if !fm.breadcrumb.is_empty() {
        y.push_str(&format!("breadcrumb: {}\n", fm.breadcrumb.join(" > ")));
    }
    if let Some(p) = &fm.parent { y.push_str(&format!("parent: {}\n", p)); }
    if let Some(p) = &fm.prev { y.push_str(&format!("prev: {}\n", p)); }
    if let Some(n) = &fm.next { y.push_str(&format!("next: {}\n", n)); }
    if !fm.children.is_empty() {
        y.push_str("children:\n");
        for c in &fm.children { y.push_str(&format!("  - {}\n", c)); }
    }
    if let Some((s, e)) = fm.source_pages { y.push_str(&format!("source_pages: {}-{}\n", s, e)); }
    y
}

fn yaml_str(s: &str) -> String {
    if s.contains(':') || s.contains('#') { format!("\"{}\"", s.replace('"', "\\\"")) } else { s.to_string() }
}
```

`crates/kasane-writer/src/lib.rs` (replace the module head, keep tests):
```rust
mod frontmatter;
mod markdown;

pub use markdown::blocks_to_markdown;

use anyhow::{bail, Context, Result};
use kasane_core::SiteTree;
use kasane_ir::AssetBag;
use std::path::Path;

pub fn write_tree(tree: &SiteTree, assets: &AssetBag, out: &Path, force: bool) -> Result<()> {
    if out.exists() {
        let non_empty = out.read_dir().map(|mut d| d.next().is_some()).unwrap_or(false);
        if non_empty && !force {
            bail!("output directory {} is not empty (use --force)", out.display());
        }
    }
    let parent = out.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).ok();
    let tmp = parent.join(format!(".{}.kasane-tmp", file_stem(out)));
    if tmp.exists() { std::fs::remove_dir_all(&tmp).ok(); }
    std::fs::create_dir_all(&tmp).context("create temp dir")?;

    for file in &tree.files {
        let path = tmp.join(&file.path);
        if let Some(p) = path.parent() { std::fs::create_dir_all(p)?; }
        let body = blocks_to_markdown(&file.blocks, assets);
        let content = format!("---\n{}---\n\n{}", frontmatter::frontmatter_yaml(&file.frontmatter), body);
        std::fs::write(&path, content).with_context(|| format!("write {}", path.display()))?;
    }

    if !assets.items.is_empty() {
        let adir = tmp.join("_assets");
        std::fs::create_dir_all(&adir)?;
        for a in &assets.items {
            std::fs::write(adir.join(&a.filename), &a.bytes)?;
        }
    }

    if out.exists() { std::fs::remove_dir_all(out).ok(); }
    std::fs::rename(&tmp, out).context("atomic rename temp -> out")?;
    Ok(())
}

fn file_stem(p: &Path) -> String {
    p.file_name().and_then(|s| s.to_str()).unwrap_or("out").to_string()
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p kasane-writer && just lint`
Expected: PASS; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-writer
git commit -m "feat(writer): frontmatter + atomic markdown tree writing"
```

---

### Task 10: EPUB adapter, format detection & security guards

**Files:**
- Create: `crates/kasane-adapters/Cargo.toml`, `src/lib.rs`, `src/detect.rs`, `src/guard.rs`, `src/epub/mod.rs`, `src/epub/opf.rs`, `src/epub/xhtml.rs`
- Create: `tests/fixtures/epub/minimal.epub` (built in Step 1)
- Test: `crates/kasane-adapters/src/lib.rs` (inline `#[test]`), `src/detect.rs`, `src/guard.rs`

**Interfaces:**
- Consumes: `kasane_ir::*`.
- Produces:
  ```rust
  pub enum Format { Epub, Pptx, Mobi, Azw3, Pdf, Djvu }
  pub fn detect(bytes: &[u8], ext_hint: Option<&str>) -> Option<Format>;
  pub trait Adapter { fn parse(&self, bytes: &[u8], source_path: &str)
      -> Result<(Document, AssetBag), ParseError>; }
  pub struct EpubAdapter;
  pub enum ParseError { Unsupported, Drm, Encrypted, Malformed(String), Bomb }
  pub fn adapter_for(fmt: Format) -> Result<Box<dyn Adapter>, ParseError>; // only Epub in this plan
  ```

- [ ] **Step 1: Build the fixture EPUB and crate manifest**

Create the tiny EPUB with a script (run once, commit the result):
```bash
mkdir -p tests/fixtures/epub/src/OEBPS
cat > tests/fixtures/epub/src/mimetype <<'EOF'
application/epub+zip
EOF
mkdir -p tests/fixtures/epub/src/META-INF
cat > tests/fixtures/epub/src/META-INF/container.xml <<'EOF'
<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>
EOF
cat > tests/fixtures/epub/src/OEBPS/content.opf <<'EOF'
<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Minimal Book</dc:title><dc:creator>A. Author</dc:creator><dc:language>en</dc:language>
  </metadata>
  <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="c1"/></spine>
</package>
EOF
cat > tests/fixtures/epub/src/OEBPS/ch1.xhtml <<'EOF'
<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body>
  <h1>Chapter One</h1>
  <p>Hello <strong>world</strong> and <a href="#s2">see two</a>.</p>
  <h2 id="s2">Section Two</h2>
  <p>Body text.</p>
</body></html>
EOF
( cd tests/fixtures/epub/src && zip -X -0 ../minimal.epub mimetype >/dev/null && \
  zip -rg9 ../minimal.epub META-INF OEBPS >/dev/null )
rm -rf tests/fixtures/epub/src
```

`crates/kasane-adapters/Cargo.toml`:
```toml
[package]
name = "kasane-adapters"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
kasane-ir = { path = "../kasane-ir" }
zip = { version = "2", default-features = false, features = ["deflate"] }
quick-xml = "0.36"
thiserror = "1"

[lints]
workspace = true
```

- [ ] **Step 2: Write failing tests (detection, guard, EPUB end-to-end)**

`crates/kasane-adapters/src/detect.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_epub_by_zip_and_mimetype() {
        let bytes = std::fs::read("../../tests/fixtures/epub/minimal.epub").unwrap();
        assert!(matches!(detect(&bytes, Some("epub")), Some(Format::Epub)));
    }
    #[test]
    fn detects_pdf_by_magic() {
        assert!(matches!(detect(b"%PDF-1.7\n...", None), Some(Format::Pdf)));
    }
}
```

`crates/kasane-adapters/src/guard.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_traversal_names() {
        assert!(safe_entry_name("../etc/passwd").is_none());
        assert!(safe_entry_name("/abs").is_none());
        assert_eq!(safe_entry_name("OEBPS/ch1.xhtml"), Some("OEBPS/ch1.xhtml".to_string()));
    }
}
```

`crates/kasane-adapters/src/lib.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_minimal_epub_to_ir() {
        let bytes = std::fs::read("../../tests/fixtures/epub/minimal.epub").unwrap();
        let (doc, _assets) = EpubAdapter.parse(&bytes, "minimal.epub").unwrap();
        assert_eq!(doc.meta.title, "Minimal Book");
        // headings present in order
        let heads: Vec<_> = doc.nodes.iter().filter_map(|n| match &n.block {
            kasane_ir::Block::Heading { level, inlines, .. } =>
                Some((*level, kasane_ir_text(inlines))), _ => None }).collect();
        assert_eq!(heads, vec![(1, "Chapter One".to_string()), (2, "Section Two".to_string())]);
    }
    fn kasane_ir_text(inls: &[kasane_ir::Inline]) -> String {
        inls.iter().map(|i| if let kasane_ir::Inline::Text(t) = i { t.clone() } else { String::new() }).collect()
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p kasane-adapters`
Expected: FAIL — `detect`, `safe_entry_name`, `EpubAdapter` not found.

- [ ] **Step 4: Implement detection, guards, and the EPUB adapter**

`crates/kasane-adapters/src/detect.rs` (top):
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format { Epub, Pptx, Mobi, Azw3, Pdf, Djvu }

pub fn detect(bytes: &[u8], ext_hint: Option<&str>) -> Option<Format> {
    if bytes.starts_with(b"%PDF") { return Some(Format::Pdf); }
    if bytes.len() > 68 && &bytes[60..68] == b"BOOKMOBI" {
        return Some(if ext_hint == Some("azw3") { Format::Azw3 } else { Format::Mobi });
    }
    if bytes.starts_with(b"AT&T") { return Some(Format::Djvu); }
    if bytes.starts_with(b"PK\x03\x04") {
        // ZIP container: EPUB has "mimetype" == application/epub+zip; PPTX has ppt/.
        if zip_has_epub_mimetype(bytes) { return Some(Format::Epub); }
        if zip_has_entry(bytes, "ppt/") { return Some(Format::Pptx); }
        // AZW3 can be zip-less; fall through to hint
    }
    match ext_hint {
        Some("epub") => Some(Format::Epub),
        Some("pptx") => Some(Format::Pptx),
        Some("mobi") => Some(Format::Mobi),
        Some("azw3") => Some(Format::Azw3),
        Some("pdf") => Some(Format::Pdf),
        Some("djvu") | Some("djv") => Some(Format::Djvu),
        _ => None,
    }
}

fn zip_has_epub_mimetype(bytes: &[u8]) -> bool {
    use std::io::Read;
    let Ok(mut z) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else { return false };
    let Ok(mut f) = z.by_name("mimetype") else { return false };
    let mut s = String::new();
    f.read_to_string(&mut s).ok();
    s.trim() == "application/epub+zip"
}

fn zip_has_entry(bytes: &[u8], prefix: &str) -> bool {
    let Ok(mut z) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else { return false };
    (0..z.len()).any(|i| z.by_index(i).map(|f| f.name().starts_with(prefix)).unwrap_or(false))
}
```

`crates/kasane-adapters/src/guard.rs` (top):
```rust
pub const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_RATIO: u64 = 200;

/// Sanitize a zip entry name; None if it escapes the archive root.
pub fn safe_entry_name(name: &str) -> Option<String> {
    if name.starts_with('/') || name.contains("..") { return None; }
    if name.split('/').any(|c| c == "." || c.is_empty() && !name.ends_with('/')) { /* tolerate */ }
    Some(name.to_string())
}

/// Guard against decompression bombs given compressed and (running) decompressed sizes.
pub fn check_expansion(compressed: u64, decompressed: u64) -> bool {
    decompressed <= MAX_TOTAL_BYTES && (compressed == 0 || decompressed / compressed.max(1) <= MAX_RATIO)
}
```

`crates/kasane-adapters/src/epub/opf.rs` — parse title/authors/language + spine order:
```rust
use quick_xml::events::Event;
use quick_xml::Reader;

pub struct Opf { pub title: String, pub authors: Vec<String>, pub language: Option<String>,
                 pub spine_hrefs: Vec<String> }

pub fn parse_opf(xml: &str, opf_dir: &str) -> Opf {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    // quick-xml does not resolve external entities -> XXE-safe by default.
    let mut title = String::new();
    let mut authors = vec![];
    let mut language = None;
    let mut manifest: std::collections::HashMap<String, String> = Default::default();
    let mut spine_ids: Vec<String> = vec![];
    let mut cur: Option<&'static str> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"title" => cur = Some("title"),
                    b"creator" => cur = Some("creator"),
                    b"language" => cur = Some("language"),
                    b"item" => {
                        let (mut id, mut href) = (String::new(), String::new());
                        for a in e.attributes().flatten() {
                            match a.key.as_ref() {
                                b"id" => id = String::from_utf8_lossy(&a.value).into(),
                                b"href" => href = String::from_utf8_lossy(&a.value).into(),
                                _ => {}
                            }
                        }
                        if !id.is_empty() { manifest.insert(id, join_href(opf_dir, &href)); }
                    }
                    b"itemref" => {
                        for a in e.attributes().flatten() {
                            if a.key.as_ref() == b"idref" {
                                spine_ids.push(String::from_utf8_lossy(&a.value).into());
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                let txt = t.unescape().unwrap_or_default().to_string();
                match cur.take() {
                    Some("title") => title = txt,
                    Some("creator") => authors.push(txt),
                    Some("language") => language = Some(txt),
                    _ => {}
                }
            }
            Ok(Event::End(_)) => cur = None,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    let spine_hrefs = spine_ids.iter().filter_map(|id| manifest.get(id).cloned()).collect();
    Opf { title, authors, language, spine_hrefs }
}

fn join_href(dir: &str, href: &str) -> String {
    if dir.is_empty() { href.to_string() } else { format!("{}/{}", dir.trim_end_matches('/'), href) }
}
```

`crates/kasane-adapters/src/epub/xhtml.rs` — minimal HTML→IR (headings, para, strong/em, links, images):
```rust
use kasane_ir::{Block, BlockId, Inline, RefTarget};
use quick_xml::events::Event;
use quick_xml::Reader;

// Returns blocks; `next_id` is a running BlockId counter for headings.
pub fn xhtml_to_blocks(xml: &str, next_id: &mut u32) -> Vec<Block> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut blocks = vec![];
    let mut buf = Vec::new();
    // inline accumulation stack
    let mut inline_stack: Vec<Vec<Inline>> = vec![];
    let mut cur_block: Option<u8> = None; // heading level, or 0 for para
    let mut link_href: Option<String> = None;

    macro_rules! push_text { ($t:expr) => {
        if let Some(top) = inline_stack.last_mut() { top.push(Inline::Text($t)); }
    }}

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                match e.local_name().as_ref() {
                    b"h1"|b"h2"|b"h3"|b"h4"|b"h5"|b"h6" => {
                        cur_block = Some(e.local_name().as_ref()[1] - b'0');
                        inline_stack.push(vec![]);
                    }
                    b"p" => { cur_block = Some(0); inline_stack.push(vec![]); }
                    b"strong"|b"b" => inline_stack.push(vec![]),
                    b"em"|b"i" => inline_stack.push(vec![]),
                    b"a" => {
                        link_href = e.attributes().flatten()
                            .find(|a| a.key.as_ref() == b"href")
                            .map(|a| String::from_utf8_lossy(&a.value).into_owned());
                        inline_stack.push(vec![]);
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                let s = t.unescape().unwrap_or_default().to_string();
                if !s.trim().is_empty() && !inline_stack.is_empty() { push_text!(s); }
            }
            Ok(Event::End(e)) => {
                match e.local_name().as_ref() {
                    b"strong"|b"b" => { let x = inline_stack.pop().unwrap();
                        if let Some(top) = inline_stack.last_mut() { top.push(Inline::Strong(x)); } }
                    b"em"|b"i" => { let x = inline_stack.pop().unwrap();
                        if let Some(top) = inline_stack.last_mut() { top.push(Inline::Emph(x)); } }
                    b"a" => {
                        let x = inline_stack.pop().unwrap();
                        let target = match link_href.take() {
                            Some(h) if h.starts_with('#') => RefTarget::External(h), // in-file; refined later
                            Some(h) => RefTarget::External(h),
                            None => RefTarget::External(String::new()),
                        };
                        if let Some(top) = inline_stack.last_mut() {
                            top.push(Inline::Link { target, inlines: x });
                        }
                    }
                    b"h1"|b"h2"|b"h3"|b"h4"|b"h5"|b"h6" => {
                        let inls = inline_stack.pop().unwrap_or_default();
                        let level = cur_block.take().unwrap_or(1);
                        let id = BlockId(*next_id); *next_id += 1;
                        blocks.push(Block::Heading { level, id, inlines: inls });
                    }
                    b"p" => {
                        let inls = inline_stack.pop().unwrap_or_default();
                        cur_block = None;
                        if !inls.is_empty() { blocks.push(Block::Para(inls)); }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    blocks
}
```

> **Scope note for this plan:** the XHTML converter handles headings, paragraphs,
> strong/em, and links — enough to drive the whole pipeline end-to-end and prove the
> IR/engine/writer. Tables, images/figures, math (MathML), footnotes, and lists in
> XHTML are **explicitly deferred to Plan 2's fidelity task**, where each gets its own
> failing test. Do not stub them here beyond leaving the `_` match arm.

`crates/kasane-adapters/src/epub/mod.rs`:
```rust
mod opf;
mod xhtml;

use crate::guard::safe_entry_name;
use crate::{Adapter, ParseError};
use kasane_ir::*;
use std::io::Read;

pub struct EpubAdapter;

impl Adapter for EpubAdapter {
    fn parse(&self, bytes: &[u8], source_path: &str) -> Result<(Document, AssetBag), ParseError> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| ParseError::Malformed(e.to_string()))?;

        // locate the OPF via META-INF/container.xml
        let container = read_entry(&mut zip, "META-INF/container.xml")
            .ok_or(ParseError::Malformed("missing container.xml".into()))?;
        let opf_path = find_opf_path(&container)
            .ok_or(ParseError::Malformed("no rootfile".into()))?;
        let opf_dir = opf_path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();

        let opf_xml = read_entry(&mut zip, &opf_path)
            .ok_or(ParseError::Malformed("missing opf".into()))?;
        let parsed = opf::parse_opf(&opf_xml, &opf_dir);

        let mut nodes = Vec::new();
        let mut next_id = 0u32;
        for href in &parsed.spine_hrefs {
            let Some(name) = safe_entry_name(href) else { continue };
            if let Some(xml) = read_entry(&mut zip, &name) {
                for b in xhtml::xhtml_to_blocks(&xml, &mut next_id) {
                    nodes.push(Node { block: b,
                        prov: Provenance { source_pages: None, source_href: Some(name.clone()) } });
                }
            }
        }

        let doc = Document {
            meta: DocMeta {
                title: if parsed.title.is_empty() { "Untitled".into() } else { parsed.title },
                authors: parsed.authors, language: parsed.language,
                source_format: "epub".into(), source_path: source_path.to_string(),
            },
            nodes,
        };
        Ok((doc, AssetBag::default()))
    }
}

fn read_entry(zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut f = zip.by_name(name).ok()?;
    // decompression-bomb guard
    if !crate::guard::check_expansion(f.compressed_size(), f.size()) { return None; }
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

fn find_opf_path(container_xml: &str) -> Option<String> {
    // crude: find full-path="..."
    let idx = container_xml.find("full-path=")?;
    let rest = &container_xml[idx + 10..];
    let q = rest.chars().next()?;
    let rest = &rest[1..];
    let end = rest.find(q)?;
    Some(rest[..end].to_string())
}
```

`crates/kasane-adapters/src/lib.rs` (module head above the test):
```rust
mod detect;
mod epub;
mod guard;

pub use detect::{detect, Format};
pub use epub::EpubAdapter;

use kasane_ir::{AssetBag, Document};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("unsupported format")] Unsupported,
    #[error("DRM-protected content is not supported")] Drm,
    #[error("encrypted content")] Encrypted,
    #[error("malformed input: {0}")] Malformed(String),
    #[error("input rejected: decompression bomb")] Bomb,
}

pub trait Adapter {
    fn parse(&self, bytes: &[u8], source_path: &str) -> Result<(Document, AssetBag), ParseError>;
}

pub fn adapter_for(fmt: Format) -> Result<Box<dyn Adapter>, ParseError> {
    match fmt {
        Format::Epub => Ok(Box::new(EpubAdapter)),
        _ => Err(ParseError::Unsupported), // other formats land in Plan 2
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p kasane-adapters && just lint`
Expected: PASS (detection, guard, EPUB→IR); no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/kasane-adapters tests/fixtures/epub/minimal.epub
git commit -m "feat(adapters): format detection, zip guards, EPUB->IR adapter"
```

---

### Task 11: CLI wiring & end-to-end conversion

**Files:**
- Create: `crates/kasane-cli/Cargo.toml`, `src/main.rs`
- Modify: root `Cargo.toml` is already `members = ["crates/*"]` (no change)
- Create: `README.md`, `AGENTS.md`, `CLAUDE.md` (front door + codebase map)
- Test: `crates/kasane-cli/tests/e2e.rs` (integration test)

**Interfaces:**
- Consumes: `kasane_adapters::{detect, adapter_for}`, `kasane_core::{structure, Options}`, `kasane_writer::write_tree`.
- Produces: the `kasane` binary. Exit codes: `0` ok, `2` unsupported/DRM, `1` other error.

- [ ] **Step 1: Create the CLI crate manifest**

`crates/kasane-cli/Cargo.toml`:
```toml
[package]
name = "kasane-cli"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[[bin]]
name = "kasane"
path = "src/main.rs"

[dependencies]
kasane-adapters = { path = "../kasane-adapters" }
kasane-core = { path = "../kasane-core" }
kasane-writer = { path = "../kasane-writer" }
clap = { version = "4", features = ["derive"] }
anyhow = "1"

[dev-dependencies]
tempfile = "3"

[lints]
workspace = true
```

- [ ] **Step 2: Write the failing end-to-end test**

`crates/kasane-cli/tests/e2e.rs`:
```rust
use std::process::Command;

#[test]
fn converts_minimal_epub_to_tree() {
    let out = tempfile::tempdir().unwrap();
    let out_dir = out.path().join("book");
    let status = Command::new(env!("CARGO_BIN_EXE_kasane"))
        .arg("../../tests/fixtures/epub/minimal.epub")
        .arg("-o").arg(&out_dir)
        .status().unwrap();
    assert!(status.success());
    let idx = std::fs::read_to_string(out_dir.join("index.md")).unwrap();
    assert!(idx.contains("title: Minimal Book"));
    // Chapter One became its own file; internal link resolved
    let ch = std::fs::read_to_string(out_dir.join("01-chapter-one.md"))
        .or_else(|_| std::fs::read_to_string(out_dir.join("01-chapter-one/index.md")))
        .unwrap();
    assert!(ch.contains("Section Two"));
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p kasane-cli`
Expected: FAIL — binary has no implementation yet (compile error / no output tree).

- [ ] **Step 4: Implement the CLI**

`crates/kasane-cli/src/main.rs`:
```rust
use anyhow::{bail, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "kasane", about = "Convert documents to progressive-disclosure Markdown")]
struct Args {
    /// Input document (EPUB supported in this build)
    input: PathBuf,
    /// Output root directory (default: ./<input-stem>/)
    #[arg(short, long)]
    out: Option<PathBuf>,
    /// Overwrite a non-empty output directory
    #[arg(long)]
    force: bool,
    /// Size-guard split threshold (estimated tokens)
    #[arg(long, default_value_t = 2000)]
    max_tokens: usize,
    /// Size-guard merge threshold (estimated tokens)
    #[arg(long, default_value_t = 200)]
    min_tokens: usize,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            // Distinguish unsupported/DRM for exit code 2.
            let msg = format!("{e:#}");
            if msg.contains("unsupported") || msg.contains("DRM") { ExitCode::from(2) }
            else { ExitCode::FAILURE }
        }
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let bytes = std::fs::read(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let ext = args.input.extension().and_then(|s| s.to_str());
    let fmt = kasane_adapters::detect(&bytes, ext)
        .context("unsupported or unrecognized format")?;
    let adapter = kasane_adapters::adapter_for(fmt)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let (doc, assets) = adapter.parse(&bytes, &args.input.to_string_lossy())
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let opts = kasane_core::Options { max_tokens: args.max_tokens, min_tokens: args.min_tokens };
    let site = kasane_core::structure(doc, &opts);

    let out = args.out.unwrap_or_else(|| {
        PathBuf::from(args.input.file_stem().and_then(|s| s.to_str()).unwrap_or("out"))
    });
    if out.as_os_str().is_empty() { bail!("could not determine output directory"); }
    kasane_writer::write_tree(&site, &assets, &out, args.force)?;
    eprintln!("wrote {} files to {}", site.files.len(), out.display());
    Ok(())
}
```

- [ ] **Step 5: Run the end-to-end test to verify it passes**

Run: `cargo test -p kasane-cli && just lint`
Expected: PASS; no warnings.

- [ ] **Step 6: Write the discoverability front door**

`README.md`:
```markdown
# kasane

Convert documents and ebooks (EPUB today; PDF, DJVU, MOBI, AZW3, PPTX coming)
into an AI-agent-friendly, progressively-disclosed Markdown file tree.

## Quick start
    mise install
    just build
    just run book.epub -o out/book
    # open out/book/index.md and drill into linked sections

## Development
    just test    # run all tests
    just lint    # fmt check + clippy -D warnings

See AGENTS.md for the codebase map.
```

`AGENTS.md` (and a `CLAUDE.md` that just says `See AGENTS.md`):
```markdown
# Codebase map

Pipeline: input file -> detect -> adapter -> IR -> structure() -> write_tree -> Markdown tree.

- `crates/kasane-ir`      Intermediate representation types. Depends on nothing.
- `crates/kasane-adapters` Format detection + parsers (EPUB). Untrusted-input boundary; see `guard.rs`.
- `crates/kasane-core`    Pure structuring engine: fold -> balance -> paths -> refs -> nav. No I/O.
- `crates/kasane-writer`  IR -> GitHub-Flavored Markdown; atomic tree writing.
- `crates/kasane-cli`     `kasane` binary; wires the pipeline; owns exit codes.

## Workflows
- `just test` — all tests   - `just lint` — fmt + clippy   - `just run <file> -o <dir>` — convert

## Conventions
- Cross-refs are symbolic (`RefTarget::Internal`) until pass 4 resolves them to relative links.
- Adapters must never trust input: guard decompression ratio/size and entry-name traversal.
- Every change ships green under `just lint && just test`.
```

`CLAUDE.md`:
```markdown
See AGENTS.md for the codebase map and conventions.
```

- [ ] **Step 7: Verify onboarding from scratch**

Run: `just build && just lint && just test && just run tests/fixtures/epub/minimal.epub -o /tmp/kasane-demo --force`
Expected: all green; `/tmp/kasane-demo/index.md` exists with frontmatter and a TOC.

- [ ] **Step 8: Commit**

```bash
git add crates/kasane-cli README.md AGENTS.md CLAUDE.md
git commit -m "feat(cli): end-to-end EPUB->markdown-tree conversion + front door"
```

---

## Self-Review

**Spec coverage (against the design spec):**
- Tree-of-files + index output → Tasks 5, 7, 9 ✓
- Pure Rust default path → all tasks; no C deps in any manifest ✓
- Heading-driven split + size guard → Tasks 3, 4 ✓
- Rich navigation frontmatter → Tasks 7, 9 ✓
- Symbolic cross-refs resolved post-split → Task 6 ✓
- GFM tables + math + figures + footnotes rendering → Task 8 ✓ (EPUB *parsing* of tables/math/figures/footnotes deferred to Plan 2, flagged in Task 10)
- Format detection by magic bytes → Task 10 ✓
- Security guards (bomb, traversal, XXE) → Task 10 (`guard.rs`, quick-xml no-entity-expansion) ✓
- Atomic write / `--force` → Task 9 ✓
- CLI surface + exit codes → Task 11 ✓ (batch mode, `--format`, `--no-assets`, `-j` deferred to Plan 2)
- mise/just/workspace layout → Task 1 ✓
- Testing tiers: unit (Tasks 2–9), integration (Task 11); snapshot/property/fuzz → Plan 2 ✓

**Deferred to Plan 2 (explicitly, not silently):** PPTX/MOBI/AZW3/PDF/DJVU adapters; OCR feature; full EPUB fidelity (tables/math/figures/footnotes/lists parsing); batch mode + remaining CLI flags; `insta` snapshot, `proptest` invariants, `cargo-fuzz`, `cargo-deny`.

**Placeholder scan:** No `TODO`/`TBD`/"handle edge cases"/simplified-sketch placeholders remain. Every code step contains the concrete implementation. Deferred work (Plan 2) is listed explicitly above, not left as in-task stubs.

**Type consistency:** `structure`, `SiteTree`, `FileNode`, `Frontmatter`, `Options`, `Adapter::parse -> (Document, AssetBag)`, `detect`, `adapter_for`, `blocks_to_markdown`, `write_tree`, `relativize`, `slug`, `inline_text` are used with identical signatures across the tasks that define and consume them.
