use crate::ParseError;
use std::io::Read;

pub(crate) type ZipReader<'a> = zip::ZipArchive<std::io::Cursor<&'a [u8]>>;

pub(crate) fn read_entry_bytes(
    zip: &mut ZipReader,
    name: &str,
    total_read: &mut u64,
) -> Result<Vec<u8>, ParseError> {
    let f = zip
        .by_name(name)
        .map_err(|_| ParseError::Malformed(format!("missing entry: {name}")))?;
    // Reject on declared metadata first (cheap), then bound the ACTUAL read so a
    // lying/small declared size cannot lead to an unbounded decompression.
    if !crate::guard::check_expansion(f.compressed_size(), f.size()) {
        return Err(ParseError::Bomb);
    }
    let cap = crate::guard::MAX_TOTAL_BYTES;
    let mut buf = Vec::new();
    f.take(cap + 1)
        .read_to_end(&mut buf)
        .map_err(|e| ParseError::Malformed(e.to_string()))?;
    if buf.len() as u64 > cap {
        return Err(ParseError::Bomb);
    }
    // MAX_TOTAL_BYTES is an absolute cap on the whole archive's decompressed output,
    // not a per-entry budget: accumulate across every call and stop once the running
    // total would exceed it, on top of the per-entry bound above.
    *total_read += buf.len() as u64;
    if *total_read > cap {
        return Err(ParseError::Bomb);
    }
    Ok(buf)
}

pub(crate) fn read_entry_string(
    zip: &mut ZipReader,
    name: &str,
    total_read: &mut u64,
) -> Result<String, ParseError> {
    let buf = read_entry_bytes(zip, name, total_read)?;
    String::from_utf8(buf).map_err(|e| ParseError::Malformed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_zip(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        w.start_file(name, opts).unwrap();
        std::io::Write::write_all(&mut w, contents).unwrap();
        w.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn reads_bytes_and_string_and_accumulates() {
        let bytes = tiny_zip("a.txt", b"hello");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut total = 0u64;
        let b = read_entry_bytes(&mut zip, "a.txt", &mut total).unwrap();
        assert_eq!(b, b"hello");
        assert_eq!(total, 5);
        let s = read_entry_string(&mut zip, "a.txt", &mut total).unwrap();
        assert_eq!(s, "hello");
        assert_eq!(total, 10);
    }

    #[test]
    fn rejects_once_aggregate_cap_exceeded() {
        let bytes = tiny_zip("b.txt", b"hello");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut total = crate::guard::MAX_TOTAL_BYTES - 2;
        let r = read_entry_bytes(&mut zip, "b.txt", &mut total);
        assert!(matches!(r, Err(ParseError::Bomb)));
    }
}
