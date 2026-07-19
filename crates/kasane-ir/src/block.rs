use crate::ids::{BlockId, NoteId};
use crate::inline::Inline;

#[derive(Clone, Debug)]
pub enum Block {
    Heading {
        level: u8,
        id: BlockId,
        inlines: Vec<Inline>,
    },
    Para(Vec<Inline>),
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    Table(Table),
    Figure {
        image: AssetRef,
        caption: Vec<Inline>,
        number: Option<String>,
    },
    CodeBlock {
        lang: Option<String>,
        text: String,
    },
    MathBlock(String),
    Footnote {
        id: NoteId,
        blocks: Vec<Block>,
    },
    Raw {
        note: String,
    },
}

#[derive(Clone, Debug)]
pub struct Table {
    pub header: Vec<Vec<Inline>>,
    pub rows: Vec<Vec<Vec<Inline>>>,
    pub has_merged: bool,
}

#[derive(Clone, Debug)]
pub struct AssetRef {
    pub key: String,
    pub bytes_ref: usize,
}
