#[cfg(test)]
mod tests {
    #[test]
    fn fixtures_load_in_lopdf() {
        for name in ["minimal", "no-outline", "image", "scanned"] {
            let path = format!("../../tests/fixtures/pdf/{name}.pdf");
            let bytes = std::fs::read(&path).unwrap_or_else(|_| panic!("missing {path}"));
            assert!(bytes.starts_with(b"%PDF"), "{name} lacks %PDF magic");
            let doc = lopdf::Document::load_mem(&bytes)
                .unwrap_or_else(|e| panic!("{name} failed to load: {e}"));
            assert!(doc.get_pages().len() >= 1, "{name} has no pages");
        }
    }
}
