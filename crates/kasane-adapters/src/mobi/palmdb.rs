use crate::ParseError;

pub(crate) fn be16(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(off..off + 2)?.try_into().ok()?))
}
pub(crate) fn be32(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(off..off + 4)?.try_into().ok()?))
}
pub(crate) fn malformed(what: &str) -> ParseError {
    ParseError::Malformed(what.into())
}

/// PalmDB container: slices records out of the input by the offset table.
/// Holds no copies; every record is a bounds-checked view into the input.
pub(crate) struct PalmDb<'a> {
    bytes: &'a [u8],
    offsets: Vec<u32>, // record starts, plus a final sentinel = file length
}

impl<'a> PalmDb<'a> {
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
        let n = be16(bytes, 76).ok_or_else(|| malformed("palmdb: truncated header"))? as usize;
        let table_end = 78 + n * 8;
        if n == 0 || bytes.len() < table_end {
            return Err(malformed("palmdb: bad record count"));
        }
        let mut offsets = Vec::with_capacity(n + 1);
        for i in 0..n {
            let off = be32(bytes, 78 + i * 8).ok_or_else(|| malformed("palmdb: bad table"))?;
            // offsets must be nondecreasing and inside the file: a lying table
            // must fail parse, not produce reversed/OOB slices later.
            if (off as usize) < table_end
                || off as usize > bytes.len()
                || offsets.last().is_some_and(|&p| off < p)
            {
                return Err(malformed("palmdb: lying record offset"));
            }
            offsets.push(off);
        }
        offsets.push(bytes.len() as u32);
        Ok(Self { bytes, offsets })
    }

    // Not consumed yet: kept for later tasks (INDX record iteration in the KF8 pipeline).
    #[allow(dead_code)]
    pub(crate) fn num_records(&self) -> usize {
        self.offsets.len() - 1
    }

    pub(crate) fn record(&self, i: usize) -> Option<&'a [u8]> {
        if i + 1 >= self.offsets.len() {
            return None;
        }
        Some(&self.bytes[self.offsets[i] as usize..self.offsets[i + 1] as usize])
    }
}

/// The fields kasane needs from the PalmDOC + MOBI headers (record 0).
/// Offsets per the format cheat-sheet in the plan header (verified against
/// calibre's headers.py).
pub(crate) struct MobiHeader {
    pub compression: u16, // 1 none, 2 PalmDoc, 17480 HUFF/CDIC
    pub encryption: u16,  // non-zero = DRM
    pub text_length: u32,
    pub text_records: u16,
    pub encoding: u32, // 65001 UTF-8, 1252 WIN1252
    // Not read directly yet: MOBI 6 wiring only needs `kf8.is_some()`; the
    // raw version number is reserved for the KF8 pipeline task.
    #[allow(dead_code)]
    pub version: u32, // >= 8 means KF8
    pub extra_flags: u16,
    pub first_image_rec: Option<u32>,
    pub kf8: Option<Kf8Indices>,
}

// Consumed by the KF8 pipeline task, which resolves `kindle:pos` links via
// the FRAG/SKEL tables these indices point into.
#[allow(dead_code)]
pub(crate) struct Kf8Indices {
    pub frag_index: u32,
    pub skel_index: u32,
}

pub(crate) fn parse_header(rec0: &[u8]) -> Result<MobiHeader, ParseError> {
    if rec0.get(16..20) != Some(b"MOBI") {
        return Err(malformed("no MOBI header"));
    }
    let header_length = be32(rec0, 20).ok_or_else(|| malformed("truncated MOBI header"))?;
    let version = be32(rec0, 36).unwrap_or(0);
    let extra_flags = if header_length >= 0xE4 {
        be16(rec0, 0xF2).unwrap_or(0)
    } else {
        0
    };
    let none_if_absent = |v: Option<u32>| v.filter(|&x| x != 0xFFFF_FFFF && x != 0);
    let kf8 = if version >= 8 {
        match (
            none_if_absent(be32(rec0, 0xF8)),
            none_if_absent(be32(rec0, 0xFC)),
        ) {
            (Some(frag_index), Some(skel_index)) => Some(Kf8Indices {
                frag_index,
                skel_index,
            }),
            _ => None,
        }
    } else {
        None
    };
    Ok(MobiHeader {
        compression: be16(rec0, 0).unwrap_or(0),
        encryption: be16(rec0, 12).unwrap_or(0),
        text_length: be32(rec0, 4).unwrap_or(0),
        text_records: be16(rec0, 8).unwrap_or(0),
        encoding: be32(rec0, 28).unwrap_or(65001),
        version,
        extra_flags,
        first_image_rec: none_if_absent(be32(rec0, 0x6C)),
        kf8,
    })
}

/// The MOBI "full name" (title): offset u32@0x54, length u32@0x58,
/// record0-relative. Length is capped: this field is attacker-controlled.
pub(crate) fn full_name(rec0: &[u8]) -> Option<String> {
    let off = be32(rec0, 0x54)? as usize;
    let len = be32(rec0, 0x58)? as usize;
    if len == 0 || len > 1024 {
        return None;
    }
    let raw = rec0.get(off..off.checked_add(len)?)?;
    let s = String::from_utf8_lossy(raw).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// Backward base-128 varint at the end of `rec`: 7 data bits per byte,
// big-endian, the most significant byte carries 0x80. The value is the
// trailing entry's total size, size bytes included.
fn trailing_entry_size(rec: &[u8]) -> usize {
    let mut bitpos = 0;
    let mut result: usize = 0;
    let mut psize = rec.len();
    loop {
        if psize == 0 {
            return result;
        }
        let v = rec[psize - 1];
        result |= ((v & 0x7F) as usize) << bitpos;
        bitpos += 7;
        psize -= 1;
        if v & 0x80 != 0 || bitpos >= 28 {
            return result;
        }
    }
}

/// Strip per-record trailing entries per extra_data_flags: one
/// backward-varint-sized entry per set bit above bit 0; bit 0 is the
/// multibyte-overlap entry of `(last_byte & 3) + 1` bytes. All subtractions
/// saturate so hostile sizes can never underflow.
pub(crate) fn strip_trailing(rec: &[u8], extra_flags: u16) -> &[u8] {
    let mut end = rec.len();
    let mut flags = extra_flags >> 1;
    while flags != 0 {
        if flags & 1 != 0 {
            let sz = trailing_entry_size(&rec[..end]).min(end);
            end -= sz;
        }
        flags >>= 1;
    }
    if extra_flags & 1 != 0 && end > 0 {
        let n = ((rec[end - 1] & 0b11) as usize + 1).min(end);
        end -= n;
    }
    &rec[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hand-built two-record PalmDB: 78-byte header + 2 entries + 2 pad bytes.
    fn tiny_db() -> Vec<u8> {
        let mut b = vec![0u8; 78];
        b[60..68].copy_from_slice(b"BOOKMOBI");
        b[76..78].copy_from_slice(&2u16.to_be_bytes());
        let tbl_end = 78 + 2 * 8 + 2;
        // record 0: 4 bytes "AAAA", record 1: 3 bytes "BBB"
        for (i, off) in [tbl_end as u32, tbl_end as u32 + 4].iter().enumerate() {
            b.extend_from_slice(&off.to_be_bytes());
            b.push(0);
            b.extend_from_slice(&(i as u32).to_be_bytes()[1..]);
        }
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(b"AAAABBB");
        b
    }

    #[test]
    fn slices_records_by_offset_table() {
        let raw = tiny_db();
        let db = PalmDb::parse(&raw).unwrap();
        assert_eq!(db.num_records(), 2);
        assert_eq!(db.record(0).unwrap(), b"AAAA");
        assert_eq!(db.record(1).unwrap(), b"BBB");
        assert!(db.record(2).is_none());
    }

    #[test]
    fn rejects_truncated_and_lying_offsets() {
        assert!(PalmDb::parse(&[0u8; 10]).is_err());
        let mut raw = tiny_db();
        // point record 0 past EOF
        let n = 78;
        raw[n..n + 4].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        assert!(PalmDb::parse(&raw).is_err());
    }

    // Minimal record 0: PalmDOC(16) + MOBI header, version 6.
    fn rec0_v6() -> Vec<u8> {
        let mut r = vec![0u8; 248];
        r[0..2].copy_from_slice(&2u16.to_be_bytes()); // compression: PalmDoc
        r[4..8].copy_from_slice(&1000u32.to_be_bytes()); // text_length
        r[8..10].copy_from_slice(&1u16.to_be_bytes()); // text_records
        r[12..14].copy_from_slice(&0u16.to_be_bytes()); // encryption
        r[16..20].copy_from_slice(b"MOBI");
        r[20..24].copy_from_slice(&232u32.to_be_bytes()); // header_length
        r[28..32].copy_from_slice(&65001u32.to_be_bytes()); // encoding
        r[36..40].copy_from_slice(&6u32.to_be_bytes()); // version
        r[0x6C..0x70].copy_from_slice(&3u32.to_be_bytes()); // first image rec
        r[0xF2..0xF4].copy_from_slice(&0b11u16.to_be_bytes()); // extra_flags
        r
    }

    #[test]
    fn parses_v6_header_fields() {
        let h = parse_header(&rec0_v6()).unwrap();
        assert_eq!(h.compression, 2);
        assert_eq!(h.encryption, 0);
        assert_eq!(h.text_length, 1000);
        assert_eq!(h.text_records, 1);
        assert_eq!(h.encoding, 65001);
        assert_eq!(h.version, 6);
        assert_eq!(h.extra_flags, 0b11);
        assert_eq!(h.first_image_rec, Some(3));
        assert!(h.kf8.is_none());
    }

    #[test]
    fn parses_kf8_indices_for_v8() {
        let mut r = rec0_v6();
        r.resize(264, 0);
        r[20..24].copy_from_slice(&248u32.to_be_bytes());
        r[36..40].copy_from_slice(&8u32.to_be_bytes());
        r[0xF8..0xFC].copy_from_slice(&7u32.to_be_bytes()); // frag index
        r[0xFC..0x100].copy_from_slice(&5u32.to_be_bytes()); // skel index
        let h = parse_header(&r).unwrap();
        let k = h.kf8.expect("v8 must expose kf8 indices");
        assert_eq!(k.frag_index, 7);
        assert_eq!(k.skel_index, 5);
    }

    #[test]
    fn header_without_mobi_magic_is_malformed() {
        assert!(parse_header(&vec![0u8; 300]).is_err());
    }

    #[test]
    fn strip_trailing_removes_flagged_entries() {
        // flags bit1 set: one backward-varint-sized entry at the end.
        // Entry: 3 data bytes + the size byte itself = 4 total; backward
        // varint "4" is one byte 0x84 (MSB has 0x80).
        let rec = b"BODYxxx\x84";
        assert_eq!(strip_trailing(rec, 0b10), b"BODY");
        // flags bit0: multibyte overlap, (last & 3) + 1 bytes
        let rec2 = b"BODY\xE2\x82\x02"; // last byte & 3 == 2 -> strip 3
        assert_eq!(strip_trailing(rec2, 0b01), b"BODY");
        // both
        let rec3 = b"BODY\xE2\x82\x02xxx\x84";
        assert_eq!(strip_trailing(rec3, 0b11), b"BODY");
        // flags 0: untouched
        assert_eq!(strip_trailing(b"BODY", 0), b"BODY");
        // never underflows on hostile sizes
        assert_eq!(strip_trailing(b"\xFF", 0b10), b"");
    }

    #[test]
    fn fixture_roundtrip_container_header_and_text() {
        let bytes = std::fs::read("../../tests/fixtures/mobi/minimal.mobi").unwrap();
        assert!(matches!(
            crate::detect(&bytes, Some("mobi")),
            Some(crate::Format::Mobi)
        ));
        let db = PalmDb::parse(&bytes).unwrap();
        let h = parse_header(db.record(0).unwrap()).unwrap();
        assert_eq!(h.compression, 1);
        assert_eq!(h.encryption, 0);
        assert_eq!(h.version, 6);
        assert_eq!(h.encoding, 65001);
        let text: Vec<u8> = (1..=h.text_records as usize)
            .flat_map(|i| strip_trailing(db.record(i).unwrap(), h.extra_flags).to_vec())
            .collect();
        assert_eq!(text.len(), h.text_length as usize);
        let s = String::from_utf8(text).unwrap();
        assert!(s.contains("<h1>Chapter Two"));
        assert!(s.contains("filepos="));
        // image record present where the header says
        let img = db.record(h.first_image_rec.unwrap() as usize).unwrap();
        assert!(img.starts_with(b"\x89PNG"));
    }

    #[test]
    fn drm_fixture_reports_encryption() {
        let bytes = std::fs::read("../../tests/fixtures/mobi/minimal-drm.mobi").unwrap();
        let db = PalmDb::parse(&bytes).unwrap();
        let h = parse_header(db.record(0).unwrap()).unwrap();
        assert_eq!(h.encryption, 2);
    }
}
