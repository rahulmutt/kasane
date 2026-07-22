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

    use lopdf::{EncryptionState, EncryptionVersion, Permissions};

    fn read(name: &str) -> Vec<u8> {
        std::fs::read(format!("../../tests/fixtures/pdf/{name}.pdf")).unwrap()
    }

    fn encrypt_minimal(owner: &str, user: &str) -> Vec<u8> {
        let mut doc = lopdf::Document::load_mem(&read("minimal")).unwrap();
        // The fixture has no trailer /ID; lopdf's key-derivation algorithm
        // requires one (`EncryptionError::MissingFileID` otherwise), so add one.
        doc.trailer.set(
            "ID",
            lopdf::Object::Array(vec![
                lopdf::Object::string_literal(b"0123456789ABCDEF"),
                lopdf::Object::string_literal(b"0123456789ABCDEF"),
            ]),
        );
        let version = EncryptionVersion::V1 {
            document: &doc,
            owner_password: owner,
            user_password: user,
            permissions: Permissions::all(),
        };
        let state = EncryptionState::try_from(version).unwrap();
        doc.encrypt(&state).unwrap();
        let mut buf = Vec::new();
        doc.save_to(&mut buf).unwrap();
        buf
    }

    #[test]
    fn decrypts_empty_user_password() {
        // Owner and user passwords both empty: the "permissions only" case.
        let bytes = encrypt_minimal("", "");
        let doc = open(&bytes).unwrap();
        assert_eq!(pages(&doc).len(), 2);
    }

    #[test]
    fn rejects_real_user_password() {
        // A non-empty owner AND user password => empty-password auth must fail.
        let bytes = encrypt_minimal("owner-secret", "user-secret");
        assert!(matches!(open(&bytes), Err(ParseError::Encrypted)));
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
