mod content;
mod doc;
mod image;
mod layout;
mod outline;

use crate::{Adapter, ParseError};
use kasane_ir::{AssetBag, Document};

pub struct PdfAdapter;

impl Adapter for PdfAdapter {
    fn parse(&self, _bytes: &[u8], _source_path: &str) -> Result<(Document, AssetBag), ParseError> {
        // Filled in over Tasks 3–9.
        Err(ParseError::Malformed("pdf adapter not yet implemented".into()))
    }
}
