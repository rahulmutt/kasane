use super::palmdb::{be16, be32, malformed};
use crate::ParseError;
use std::collections::HashMap;

/// One parsed index entry: raw name plus tag -> values.
#[allow(dead_code)] // consumed by Task 10 (KF8 pipeline)
pub(crate) struct IndexEntry {
    pub name: Vec<u8>,
    pub tags: HashMap<u8, Vec<u64>>,
}

/// Forward base-128 varint: big-endian 7-bit groups, final byte carries 0x80.
#[allow(dead_code)] // consumed by Task 10 (KF8 pipeline)
fn fwd_varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v: u64 = 0;
    for _ in 0..8 {
        let b = *data.get(*pos)?;
        *pos += 1;
        v = (v << 7) | (b & 0x7F) as u64;
        if b & 0x80 != 0 {
            return Some(v);
        }
    }
    None
}

/// Kindle base-32: digits then A..V, case-insensitive (used by
/// kindle:pos fid/off and kindle:embed indexes).
#[allow(dead_code)] // consumed by Task 10 (KF8 pipeline)
pub(crate) fn base32(s: &str) -> Option<u64> {
    let mut v: u64 = 0;
    for c in s.chars() {
        let d = match c {
            '0'..='9' => c as u64 - '0' as u64,
            'A'..='V' => c as u64 - 'A' as u64 + 10,
            'a'..='v' => c as u64 - 'a' as u64 + 10,
            _ => return None,
        };
        v = v.checked_mul(32)?.checked_add(d)?;
    }
    Some(v)
}

/// Parse one index: `first` is the INDX header record (with the TAGX table);
/// the following `data-record count` records hold the entries, addressed
/// through their IDXT offset tables. Every offset is bounds-checked; a lying
/// table is a Malformed error for the caller to degrade on.
#[allow(dead_code)] // consumed by Task 10 (KF8 pipeline)
pub(crate) fn read_index(
    db: &super::palmdb::PalmDb,
    first: usize,
) -> Result<Vec<IndexEntry>, ParseError> {
    let hdr = db
        .record(first)
        .ok_or_else(|| malformed("kf8: index header record missing"))?;
    if hdr.get(0..4) != Some(b"INDX") {
        return Err(malformed("kf8: not an INDX record"));
    }
    let hlen = be32(hdr, 4).ok_or_else(|| malformed("kf8: truncated INDX"))? as usize;
    let data_recs = be32(hdr, 0x18).ok_or_else(|| malformed("kf8: truncated INDX"))? as usize;
    if data_recs == 0 || data_recs > 4096 {
        return Err(malformed("kf8: implausible index record count"));
    }
    if hdr.get(hlen..hlen + 4) != Some(b"TAGX") {
        return Err(malformed("kf8: TAGX missing"));
    }
    let tagx_len = be32(hdr, hlen + 4).ok_or_else(|| malformed("kf8: truncated TAGX"))? as usize;
    let ctrl_count = be32(hdr, hlen + 8).ok_or_else(|| malformed("kf8: truncated TAGX"))? as usize;
    if !(1..=4).contains(&ctrl_count) || tagx_len < 16 {
        return Err(malformed("kf8: implausible TAGX"));
    }
    let mut table = vec![]; // (tag, values_per_entry, mask, end_of_control)
    let mut p = hlen + 12;
    while p + 4 <= hlen + tagx_len && p + 4 <= hdr.len() {
        table.push((hdr[p], hdr[p + 1], hdr[p + 2], hdr[p + 3]));
        p += 4;
    }

    let mut entries = vec![];
    for r in 0..data_recs {
        let rec = db
            .record(first + 1 + r)
            .ok_or_else(|| malformed("kf8: index data record missing"))?;
        if rec.get(0..4) != Some(b"INDX") {
            return Err(malformed("kf8: bad index data record"));
        }
        let idxt = be32(rec, 0x14).ok_or_else(|| malformed("kf8: truncated data record"))? as usize;
        let count =
            be32(rec, 0x18).ok_or_else(|| malformed("kf8: truncated data record"))? as usize;
        if rec.get(idxt..idxt + 4) != Some(b"IDXT") || count > 65536 {
            return Err(malformed("kf8: lying IDXT"));
        }
        for e in 0..count {
            let off =
                be16(rec, idxt + 4 + e * 2).ok_or_else(|| malformed("kf8: lying IDXT"))? as usize;
            entries.push(
                parse_entry(rec, off, ctrl_count, &table)
                    .ok_or_else(|| malformed("kf8: lying index entry"))?,
            );
        }
    }
    Ok(entries)
}

// Entry = name_len u8, name, `ctrl_count` control bytes, then forward-varint
// values per the TAGX table (KindleUnpack's getTagMap).
#[allow(dead_code)] // consumed by Task 10 (KF8 pipeline)
fn parse_entry(
    rec: &[u8],
    off: usize,
    ctrl_count: usize,
    table: &[(u8, u8, u8, u8)],
) -> Option<IndexEntry> {
    let namelen = *rec.get(off)? as usize;
    let name = rec.get(off + 1..off + 1 + namelen)?.to_vec();
    let ctrl_start = off + 1 + namelen;
    let ctrl = rec.get(ctrl_start..ctrl_start + ctrl_count)?;
    let mut pos = ctrl_start + ctrl_count;
    let mut tags: HashMap<u8, Vec<u64>> = HashMap::new();
    let mut cb = 0usize;
    for &(tag, nvals, mask, eof) in table {
        if eof == 1 {
            cb += 1;
            continue;
        }
        let v = *ctrl.get(cb)? & mask;
        if v == 0 {
            continue;
        }
        let mut vals = vec![];
        if v == mask && mask.count_ones() > 1 {
            // All mask bits set: a varint gives the byte length of the values.
            let total = fwd_varint(rec, &mut pos)? as usize;
            let end = pos.checked_add(total)?;
            if end > rec.len() {
                return None;
            }
            while pos < end {
                vals.push(fwd_varint(rec, &mut pos)?);
            }
        } else {
            let count = (v >> mask.trailing_zeros()) as usize;
            for _ in 0..count.checked_mul(nvals as usize)? {
                vals.push(fwd_varint(rec, &mut pos)?);
            }
        }
        tags.insert(tag, vals);
    }
    Some(IndexEntry { name, tags })
}

/// SKEL table row: tag 1 = fragment count, tag 6 = (start, length).
#[allow(dead_code)] // consumed by Task 10 (KF8 pipeline)
pub(crate) struct SkelEntry {
    pub frag_count: u64,
    pub start: u64,
    pub len: u64,
}

/// FRAG table row: name = decimal insert position, tag 6.1 = length.
/// (tag 6.0 is the fragment's start, unused: fragments are consumed
/// sequentially from the stream right after their skeleton.)
#[allow(dead_code)] // consumed by Task 10 (KF8 pipeline)
pub(crate) struct FragEntry {
    pub insert_pos: u64,
    pub len: u64,
}

#[allow(dead_code)] // consumed by Task 10 (KF8 pipeline)
pub(crate) fn skel_entries(idx: &[IndexEntry]) -> Vec<SkelEntry> {
    idx.iter()
        .filter_map(|e| {
            let t1 = e.tags.get(&1)?;
            let t6 = e.tags.get(&6)?;
            Some(SkelEntry {
                frag_count: *t1.first()?,
                start: *t6.first()?,
                len: *t6.get(1)?,
            })
        })
        .collect()
}

#[allow(dead_code)] // consumed by Task 10 (KF8 pipeline)
pub(crate) fn frag_entries(idx: &[IndexEntry]) -> Vec<FragEntry> {
    idx.iter()
        .filter_map(|e| {
            let insert_pos = std::str::from_utf8(&e.name).ok()?.parse().ok()?;
            let t6 = e.tags.get(&6)?;
            Some(FragEntry {
                insert_pos,
                len: *t6.get(1)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobi::palmdb::PalmDb;

    fn fwd(mut v: u64) -> Vec<u8> {
        let mut bs = vec![];
        loop {
            bs.insert(0, (v & 0x7F) as u8);
            v >>= 7;
            if v == 0 {
                break;
            }
        }
        *bs.last_mut().unwrap() |= 0x80;
        bs
    }

    // (header_record, data_record) for one index.
    fn build_index(
        tagx: &[(u8, u8, u8, u8)],
        entries: &[(&[u8], u8, &[u64])],
    ) -> (Vec<u8>, Vec<u8>) {
        let hlen = 0x30u32;
        let mut hdr = vec![0u8; hlen as usize];
        hdr[0..4].copy_from_slice(b"INDX");
        hdr[4..8].copy_from_slice(&hlen.to_be_bytes());
        hdr[0x18..0x1C].copy_from_slice(&1u32.to_be_bytes()); // one data record
        hdr[0x1C..0x20].copy_from_slice(&65001u32.to_be_bytes());
        hdr[0x24..0x28].copy_from_slice(&(entries.len() as u32).to_be_bytes());
        hdr.extend_from_slice(b"TAGX");
        hdr.extend_from_slice(&((12 + 4 * tagx.len()) as u32).to_be_bytes());
        hdr.extend_from_slice(&1u32.to_be_bytes()); // control byte count
        for &(t, n, m, e) in tagx {
            hdr.extend_from_slice(&[t, n, m, e]);
        }

        let dh = 0x20usize;
        let mut blob = vec![];
        let mut offs = vec![];
        for (name, ctrl, values) in entries {
            offs.push((dh + blob.len()) as u16);
            blob.push(name.len() as u8);
            blob.extend_from_slice(name);
            blob.push(*ctrl);
            for &v in *values {
                blob.extend(fwd(v));
            }
        }
        let idxt_start = (dh + blob.len()) as u32;
        let mut data = vec![0u8; dh];
        data[0..4].copy_from_slice(b"INDX");
        data[4..8].copy_from_slice(&(dh as u32).to_be_bytes());
        data[0x14..0x18].copy_from_slice(&idxt_start.to_be_bytes());
        data[0x18..0x1C].copy_from_slice(&(entries.len() as u32).to_be_bytes());
        data.extend_from_slice(&blob);
        data.extend_from_slice(b"IDXT");
        for o in offs {
            data.extend_from_slice(&o.to_be_bytes());
        }
        (hdr, data)
    }

    // Wrap records in a PalmDB container (same layout as Task 1's tiny_db).
    fn db_bytes(records: &[Vec<u8>]) -> Vec<u8> {
        let n = records.len();
        let mut b = vec![0u8; 78];
        b[60..68].copy_from_slice(b"BOOKMOBI");
        b[76..78].copy_from_slice(&(n as u16).to_be_bytes());
        let mut pos = (78 + 8 * n + 2) as u32;
        for (i, r) in records.iter().enumerate() {
            b.extend_from_slice(&pos.to_be_bytes());
            b.push(0);
            b.extend_from_slice(&(i as u32).to_be_bytes()[1..]);
            pos += r.len() as u32;
        }
        b.extend_from_slice(&[0, 0]);
        for r in records {
            b.extend_from_slice(r);
        }
        b
    }

    const SKEL_TAGX: &[(u8, u8, u8, u8)] = &[(1, 1, 0x01, 0), (6, 2, 0x02, 0), (0, 0, 0, 1)];
    const FRAG_TAGX: &[(u8, u8, u8, u8)] = &[
        (2, 1, 0x01, 0),
        (3, 1, 0x02, 0),
        (4, 1, 0x04, 0),
        (6, 2, 0x08, 0),
        (0, 0, 0, 1),
    ];

    #[test]
    fn parses_skel_index() {
        let (h, d) = build_index(
            SKEL_TAGX,
            &[
                (b"SKEL0000000", 0x03, &[1, 0, 40]),
                (b"SKEL0000001", 0x03, &[1, 100, 38]),
            ],
        );
        let raw = db_bytes(&[h, d]);
        let db = PalmDb::parse(&raw).unwrap();
        let idx = read_index(&db, 0).unwrap();
        assert_eq!(idx.len(), 2);
        let skels = skel_entries(&idx);
        assert_eq!(skels.len(), 2);
        assert_eq!(
            (skels[0].frag_count, skels[0].start, skels[0].len),
            (1, 0, 40)
        );
        assert_eq!(
            (skels[1].frag_count, skels[1].start, skels[1].len),
            (1, 100, 38)
        );
    }

    #[test]
    fn parses_frag_index_with_multibyte_varints() {
        let (h, d) = build_index(
            FRAG_TAGX,
            &[(b"33", 0x0F, &[0, 0, 0, 40, 300])], // 300 needs a 2-byte varint
        );
        let raw = db_bytes(&[h, d]);
        let db = PalmDb::parse(&raw).unwrap();
        let frags = frag_entries(&read_index(&db, 0).unwrap());
        assert_eq!(frags.len(), 1);
        assert_eq!((frags[0].insert_pos, frags[0].len), (33, 300));
    }

    #[test]
    fn absent_tags_and_bad_names_are_skipped_not_fatal() {
        // ctrl 0x01: only tag 1 present, no tag 6 -> not a usable skel entry
        let (h, d) = build_index(SKEL_TAGX, &[(b"SKEL0000000", 0x01, &[5])]);
        let raw = db_bytes(&[h, d]);
        let db = PalmDb::parse(&raw).unwrap();
        let idx = read_index(&db, 0).unwrap();
        assert_eq!(idx.len(), 1);
        assert!(skel_entries(&idx).is_empty());
        // frag entry with a non-decimal name is skipped
        let (h2, d2) = build_index(FRAG_TAGX, &[(b"notanum", 0x0F, &[0, 0, 0, 1, 2])]);
        let raw2 = db_bytes(&[h2, d2]);
        let db2 = PalmDb::parse(&raw2).unwrap();
        assert!(frag_entries(&read_index(&db2, 0).unwrap()).is_empty());
    }

    #[test]
    fn truncated_or_lying_index_errors_without_panic() {
        let (h, mut d) = build_index(SKEL_TAGX, &[(b"SKEL0000000", 0x03, &[1, 0, 40])]);
        // point IDXT past the end of the record
        d[0x14..0x18].copy_from_slice(&0xFFFFu32.to_be_bytes());
        let raw = db_bytes(&[h, d]);
        let db = PalmDb::parse(&raw).unwrap();
        assert!(read_index(&db, 0).is_err());
        // missing data record
        let (h2, _) = build_index(SKEL_TAGX, &[(b"SKEL0000000", 0x03, &[1, 0, 40])]);
        let raw2 = db_bytes(&[h2]);
        let db2 = PalmDb::parse(&raw2).unwrap();
        assert!(read_index(&db2, 0).is_err());
    }

    #[test]
    fn base32_decodes_kindle_alphabet() {
        assert_eq!(base32("0001"), Some(1));
        assert_eq!(base32("000A"), Some(10));
        assert_eq!(base32("V"), Some(31));
        assert_eq!(base32("10"), Some(32));
        assert_eq!(base32("0j"), Some(19)); // case-insensitive
        assert_eq!(base32("0W"), None); // outside alphabet
        assert_eq!(base32(""), Some(0));
    }

    #[test]
    fn azw3_fixture_indexes_round_trip() {
        let bytes = std::fs::read("../../tests/fixtures/azw3/minimal.azw3").unwrap();
        let db = PalmDb::parse(&bytes).unwrap();
        let h = crate::mobi::palmdb::parse_header(db.record(0).unwrap()).unwrap();
        let k = h.kf8.expect("fixture must be KF8");
        let skels = skel_entries(&read_index(&db, k.skel_index as usize).unwrap());
        let frags = frag_entries(&read_index(&db, k.frag_index as usize).unwrap());
        assert_eq!(skels.len(), 2);
        assert_eq!(frags.len(), 2);
        assert_eq!(skels[0].frag_count, 1);
        assert_eq!(skels[0].start, 0);
        assert!(frags[0].insert_pos > 0 && frags[0].len > 0);
    }
}
