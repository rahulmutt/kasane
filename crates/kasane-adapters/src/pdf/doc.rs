use crate::guard::MAX_TOTAL_BYTES;
use crate::ParseError;
use lopdf::{Document, ObjectId};

/// Per-page content-stream decompression cap (bomb guard).
pub const MAX_CONTENT_BYTES: usize = MAX_TOTAL_BYTES as usize;

/// Open a PDF from bytes. If the document is encrypted, attempt decryption with
/// the empty user password (the common "permissions only" case). A real user
/// password yields `ParseError::Encrypted`; we never crack or prompt.
pub fn open(bytes: &[u8]) -> Result<Document, ParseError> {
    let mut doc = Document::load_mem(bytes).map_err(|e| ParseError::Malformed(e.to_string()))?;
    if doc.is_encrypted() {
        doc.decrypt("").map_err(|_| ParseError::Encrypted)?;
    }
    Ok(doc)
}

/// 1-based page number → page object id, ascending by page number.
pub fn pages(doc: &Document) -> Vec<(u32, ObjectId)> {
    let mut v: Vec<(u32, ObjectId)> = doc.get_pages().into_iter().collect();
    v.sort_by_key(|(n, _)| *n);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(name: &str) -> Vec<u8> {
        std::fs::read(format!("../../tests/fixtures/pdf/{name}.pdf")).unwrap()
    }

    #[test]
    fn opens_and_counts_pages() {
        let doc = open(&read("minimal")).unwrap();
        assert_eq!(pages(&doc).len(), 2);
        // page numbers are 1-based and ascending
        let nums: Vec<u32> = pages(&doc).iter().map(|(n, _)| *n).collect();
        assert_eq!(nums, vec![1, 2]);
    }

    #[test]
    fn rejects_non_pdf() {
        assert!(matches!(open(b"not a pdf"), Err(ParseError::Malformed(_))));
    }
}
