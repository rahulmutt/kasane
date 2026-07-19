use kasane_ir::Block;

pub struct SiteTree {
    pub files: Vec<FileNode>,
}

pub struct FileNode {
    pub path: String,
    pub frontmatter: Frontmatter,
    pub blocks: Vec<Block>,
}

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
