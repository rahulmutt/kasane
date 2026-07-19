use crate::ids::{BlockId, NoteId};

#[derive(Clone, Debug)]
pub enum Inline {
    Text(String),
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    Code(String),
    Math(String),
    Link {
        target: RefTarget,
        inlines: Vec<Inline>,
    },
    FootnoteRef(NoteId),
}

#[derive(Clone, Debug)]
pub enum RefTarget {
    Internal(BlockId),
    External(String),
    Footnote(NoteId),
}
