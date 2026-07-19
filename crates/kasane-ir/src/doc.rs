use crate::block::Block;

#[derive(Clone, Debug)]
pub struct Document {
    pub meta: DocMeta,
    pub nodes: Vec<Node>,
}

#[derive(Clone, Debug)]
pub struct DocMeta {
    pub title: String,
    pub authors: Vec<String>,
    pub language: Option<String>,
    pub source_format: String,
    pub source_path: String,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub block: Block,
    pub prov: Provenance,
}

#[derive(Clone, Debug, Default)]
pub struct Provenance {
    pub source_pages: Option<(u32, u32)>,
    pub source_href: Option<String>,
}
