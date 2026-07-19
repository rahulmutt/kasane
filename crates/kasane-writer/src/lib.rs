mod frontmatter;
mod markdown;

pub use markdown::blocks_to_markdown;

use anyhow::{bail, Context, Result};
use kasane_core::SiteTree;
use kasane_ir::AssetBag;
use std::path::Path;

pub fn write_tree(tree: &SiteTree, assets: &AssetBag, out: &Path, force: bool) -> Result<()> {
    if out.exists() {
        let non_empty = out
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
        if non_empty && !force {
            bail!(
                "output directory {} is not empty (use --force)",
                out.display()
            );
        }
    }
    let parent = out.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).ok();
    let tmp = parent.join(format!(".{}.kasane-tmp", file_stem(out)));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).ok();
    }
    std::fs::create_dir_all(&tmp).context("create temp dir")?;

    for file in &tree.files {
        let path = tmp.join(&file.path);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let body = blocks_to_markdown(&file.blocks, assets);
        let content = format!(
            "---\n{}---\n\n{}",
            frontmatter::frontmatter_yaml(&file.frontmatter),
            body
        );
        std::fs::write(&path, content).with_context(|| format!("write {}", path.display()))?;
    }

    if !assets.items.is_empty() {
        let adir = tmp.join("_assets");
        std::fs::create_dir_all(&adir)?;
        for a in &assets.items {
            std::fs::write(adir.join(&a.filename), &a.bytes)?;
        }
    }

    if out.exists() {
        std::fs::remove_dir_all(out).ok();
    }
    std::fs::rename(&tmp, out).context("atomic rename temp -> out")?;
    Ok(())
}

fn file_stem(p: &Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("out")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::write_tree;
    use kasane_core::{FileNode, Frontmatter, SiteTree};
    use kasane_ir::{AssetBag, Block, BlockId, Inline};

    #[test]
    fn writes_files_with_frontmatter() {
        let tree = SiteTree {
            files: vec![FileNode {
                path: "index.md".into(),
                frontmatter: Frontmatter {
                    title: "Book".into(),
                    breadcrumb: vec!["Book".into()],
                    parent: None,
                    prev: None,
                    next: None,
                    children: vec!["01-intro.md".into()],
                    source_pages: Some((1, 3)),
                },
                blocks: vec![Block::Heading {
                    level: 1,
                    id: BlockId(0),
                    inlines: vec![Inline::Text("Book".into())],
                }],
            }],
        };
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
