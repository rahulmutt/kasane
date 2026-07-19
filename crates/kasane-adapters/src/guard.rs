pub const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_RATIO: u64 = 200;

/// Sanitize a zip entry name; None if it escapes the archive root.
pub fn safe_entry_name(name: &str) -> Option<String> {
    if name.starts_with('/') || name.contains("..") {
        return None;
    }
    Some(name.to_string())
}

/// Guard against decompression bombs given compressed and (running) decompressed sizes.
pub fn check_expansion(compressed: u64, decompressed: u64) -> bool {
    decompressed <= MAX_TOTAL_BYTES
        && (compressed == 0 || decompressed / compressed.max(1) <= MAX_RATIO)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_traversal_names() {
        assert!(safe_entry_name("../etc/passwd").is_none());
        assert!(safe_entry_name("/abs").is_none());
        assert_eq!(
            safe_entry_name("OEBPS/ch1.xhtml"),
            Some("OEBPS/ch1.xhtml".to_string())
        );
    }
}
