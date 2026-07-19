#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Epub,
    Pptx,
    Mobi,
    Azw3,
    Pdf,
    Djvu,
}

pub fn detect(bytes: &[u8], ext_hint: Option<&str>) -> Option<Format> {
    if bytes.starts_with(b"%PDF") {
        return Some(Format::Pdf);
    }
    if bytes.len() > 68 && &bytes[60..68] == b"BOOKMOBI" {
        return Some(if ext_hint == Some("azw3") {
            Format::Azw3
        } else {
            Format::Mobi
        });
    }
    if bytes.starts_with(b"AT&T") {
        return Some(Format::Djvu);
    }
    if bytes.starts_with(b"PK\x03\x04") {
        // ZIP container: EPUB has "mimetype" == application/epub+zip; PPTX has ppt/.
        if zip_has_epub_mimetype(bytes) {
            return Some(Format::Epub);
        }
        if zip_has_entry(bytes, "ppt/") {
            return Some(Format::Pptx);
        }
        // AZW3 can be zip-less; fall through to hint
    }
    match ext_hint {
        Some("epub") => Some(Format::Epub),
        Some("pptx") => Some(Format::Pptx),
        Some("mobi") => Some(Format::Mobi),
        Some("azw3") => Some(Format::Azw3),
        Some("pdf") => Some(Format::Pdf),
        Some("djvu") | Some("djv") => Some(Format::Djvu),
        _ => None,
    }
}

fn zip_has_epub_mimetype(bytes: &[u8]) -> bool {
    use std::io::Read;
    let Ok(mut z) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return false;
    };
    let Ok(f) = z.by_name("mimetype") else {
        return false;
    };
    let mut s = String::new();
    // bound the read: the mimetype string is tiny; never decompress a huge entry here
    f.take(64).read_to_string(&mut s).ok();
    s.trim() == "application/epub+zip"
}

fn zip_has_entry(bytes: &[u8], prefix: &str) -> bool {
    let Ok(mut z) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return false;
    };
    (0..z.len()).any(|i| {
        z.by_index(i)
            .map(|f| f.name().starts_with(prefix))
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_epub_by_zip_and_mimetype() {
        let bytes = std::fs::read("../../tests/fixtures/epub/minimal.epub").unwrap();
        assert!(matches!(detect(&bytes, Some("epub")), Some(Format::Epub)));
    }
    #[test]
    fn detects_pdf_by_magic() {
        assert!(matches!(detect(b"%PDF-1.7\n...", None), Some(Format::Pdf)));
    }
}
