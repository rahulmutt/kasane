use super::indx::{base32, FragEntry, SkelEntry};

pub(crate) struct Part {
    pub name: String,
    pub html: Vec<u8>,
}

pub(crate) struct Assembled {
    pub parts: Vec<Part>,
    /// Per FRAG-table row: (part index, fragment's byte offset within that
    /// part). (usize::MAX, 0) marks fragments of a degraded skeleton.
    pub frag_pos: Vec<(usize, usize)>,
}

const DEGRADED: &[u8] = b"<body><p>[unreadable section: damaged KF8 index]</p></body>";

/// KindleUnpack's buildParts: skeleton k's fragments sit contiguously in the
/// stream right after it; each is inserted at (insert_pos - skel_start) into
/// the growing skeleton. Every offset is validated; a lying entry degrades
/// that part alone.
///
/// `insert_pos` is expressed against the ORIGINAL (unspliced) skeleton
/// bytes. Once the first fragment of a skeleton is spliced in, `part` grows,
/// so the 2nd+ fragment's true splice point -- and the offset recorded in
/// `frag_pos` -- must add the cumulative length of every fragment already
/// inserted into this same skeleton (`shift` below). Skipping that
/// adjustment would still build byte-identical output for the common
/// single-fragment-per-skeleton case (both committed fixtures are exactly
/// that), but would misplace and mis-record the 2nd+ fragment of a
/// multi-fragment skeleton -- see the synthetic multi-fragment test below.
pub(crate) fn assemble(raw: &[u8], skels: &[SkelEntry], frags: &[FragEntry]) -> Assembled {
    let mut parts = Vec::new();
    let mut frag_pos = vec![(usize::MAX, 0usize); frags.len()];
    let mut fragptr = 0usize;
    for (pi, sk) in skels.iter().enumerate() {
        let name = format!("part{pi:04}.xhtml");
        let mut consumed = 0usize;
        let built = (|| {
            let sstart = usize::try_from(sk.start).ok()?;
            let slen = usize::try_from(sk.len).ok()?;
            let send = sstart.checked_add(slen).filter(|&e| e <= raw.len())?;
            let mut base = send;
            let mut part = raw.get(sstart..send)?.to_vec();
            // frag_count is attacker data: cap it by the table so a huge
            // count can't spin.
            let fc = usize::try_from(sk.frag_count)
                .ok()
                .filter(|&c| fragptr.checked_add(c).is_some_and(|e| e <= frags.len()))?;
            let mut shift = 0usize; // cumulative growth from earlier fragments in this skeleton
            for _ in 0..fc {
                let fr = &frags[fragptr + consumed];
                let flen = usize::try_from(fr.len).ok()?;
                let fend = base.checked_add(flen).filter(|&e| e <= raw.len())?;
                let raw_off = usize::try_from(fr.insert_pos)
                    .ok()?
                    .checked_sub(sstart)
                    .filter(|&i| i <= slen)?;
                let insert = raw_off.checked_add(shift).filter(|&i| i <= part.len())?;
                frag_pos[fragptr + consumed] = (pi, insert);
                part.splice(insert..insert, raw[base..fend].iter().copied());
                shift += flen;
                base = fend;
                consumed += 1;
            }
            Some(part)
        })();
        match built {
            Some(html) => {
                fragptr += consumed;
                parts.push(Part { name, html });
            }
            None => {
                eprintln!("warning: lying KF8 index, degrading {name}");
                // Un-claim anything this skeleton touched, then skip its
                // claimed count (bounded by the table).
                for fp in frag_pos.iter_mut().skip(fragptr).take(consumed) {
                    *fp = (usize::MAX, 0);
                }
                fragptr = fragptr
                    .saturating_add(usize::try_from(sk.frag_count).unwrap_or(usize::MAX))
                    .min(frags.len());
                parts.push(Part {
                    name,
                    html: DEGRADED.to_vec(),
                });
            }
        }
    }
    Assembled { parts, frag_pos }
}

/// Distinct kindle:pos hrefs across all parts, scanned on raw bytes BEFORE
/// normalization so splice offsets are exact.
pub(crate) fn collect_kindle_pos(asm: &Assembled) -> Vec<String> {
    const NEEDLE: &[u8] = b"kindle:pos:fid:";
    let mut out = std::collections::BTreeSet::new();
    for p in &asm.parts {
        let raw = &p.html;
        let mut i = 0;
        while let Some(q) = raw[i..]
            .windows(NEEDLE.len())
            .position(|w| w == NEEDLE)
            .map(|q| i + q)
        {
            let mut j = q;
            while raw
                .get(j)
                .is_some_and(|&b| !matches!(b, b'"' | b'\'' | b'<' | b'>' | b' ' | b'\t' | b'\n'))
            {
                j += 1;
            }
            if let Ok(s) = std::str::from_utf8(&raw[q..j]) {
                out.insert(s.to_string());
            }
            i = q + NEEDLE.len();
        }
    }
    out.into_iter().collect()
}

/// kindle:pos:fid:XXXX:off:YYYYYYYYYY -> (part index, part-local byte
/// offset). fid indexes the FRAG table; the target is that fragment's
/// part-local start plus off.
pub(crate) fn resolve_kindle_pos(href: &str, asm: &Assembled) -> Option<(usize, usize)> {
    let rest = href.strip_prefix("kindle:pos:fid:")?;
    let (fid_s, off_s) = rest.split_once(":off:")?;
    let fid = usize::try_from(base32(fid_s)?).ok()?;
    let off = usize::try_from(base32(off_s)?).ok()?;
    let &(part, fstart) = asm.frag_pos.get(fid)?;
    if part == usize::MAX {
        return None;
    }
    Some((part, fstart.checked_add(off)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobi::indx::{FragEntry, SkelEntry};

    // skeleton "<body></body>" + fragment "<p>x</p>" inserted before </body>
    fn one_part() -> (Vec<u8>, Vec<SkelEntry>, Vec<FragEntry>) {
        let skel = b"<body></body>".to_vec();
        let frag = b"<p>x</p>";
        let mut raw = skel.clone();
        raw.extend_from_slice(frag);
        let skels = vec![SkelEntry {
            frag_count: 1,
            start: 0,
            len: skel.len() as u64,
        }];
        let frags = vec![FragEntry {
            insert_pos: 6,
            len: frag.len() as u64,
        }];
        (raw, skels, frags)
    }

    #[test]
    fn assembles_fragment_into_skeleton() {
        let (raw, skels, frags) = one_part();
        let asm = assemble(&raw, &skels, &frags);
        assert_eq!(asm.parts.len(), 1);
        assert_eq!(asm.parts[0].name, "part0000.xhtml");
        assert_eq!(asm.parts[0].html, b"<body><p>x</p></body>");
        assert_eq!(asm.frag_pos, vec![(0, 6)]);
    }

    #[test]
    fn lying_skeleton_degrades_that_part_only() {
        let (raw, mut skels, frags) = one_part();
        skels[0].len = 9999;
        let asm = assemble(&raw, &skels, &frags);
        assert_eq!(asm.parts.len(), 1);
        let s = String::from_utf8_lossy(&asm.parts[0].html).into_owned();
        assert!(s.contains("unreadable section"), "got: {s}");
        assert_eq!(asm.frag_pos[0].0, usize::MAX);
    }

    #[test]
    fn frag_count_beyond_table_degrades_without_hanging() {
        let (raw, mut skels, frags) = one_part();
        skels[0].frag_count = u64::MAX; // hostile: must not loop forever
        let asm = assemble(&raw, &skels, &frags);
        assert!(String::from_utf8_lossy(&asm.parts[0].html).contains("unreadable section"));
    }

    #[test]
    fn resolves_kindle_pos_to_part_and_offset() {
        let (raw, skels, frags) = one_part();
        let asm = assemble(&raw, &skels, &frags);
        // fid 0000, off 3 -> part 0, fragment start 6 + 3 = 9
        assert_eq!(
            resolve_kindle_pos("kindle:pos:fid:0000:off:0000000003", &asm),
            Some((0, 9))
        );
        assert_eq!(resolve_kindle_pos("kindle:pos:fid:000W:off:0", &asm), None);
        assert_eq!(resolve_kindle_pos("kindle:pos:fid:0005:off:0", &asm), None);
        assert_eq!(resolve_kindle_pos("nonsense", &asm), None);
    }

    #[test]
    fn collects_distinct_kindle_pos_hrefs() {
        let asm = Assembled {
            parts: vec![Part {
                name: "part0000.xhtml".into(),
                html: b"<a href=\"kindle:pos:fid:0001:off:0000000000\">x</a>\
                        <a href='kindle:pos:fid:0001:off:0000000000'>y</a>"
                    .to_vec(),
            }],
            frag_pos: vec![],
        };
        assert_eq!(
            collect_kindle_pos(&asm),
            vec!["kindle:pos:fid:0001:off:0000000000".to_string()]
        );
    }

    // Controller-mandated regression test: the interface contract says
    // `frag_pos[fid]` is the fragment's FINAL part-local byte offset. With
    // 2 fragments in one skeleton, the 2nd fragment's recorded position
    // must account for the shift caused by splicing the 1st fragment in
    // first -- not just its raw (pre-splice) insert_pos.
    #[test]
    fn second_fragment_position_accounts_for_first_fragments_shift() {
        // skeleton: "<body></body>" (13 bytes); both fragments are logically
        // "insert right before </body>", so both carry the SAME raw
        // insert_pos (6, relative to the original unspliced skeleton).
        let skel = b"<body></body>".to_vec();
        assert_eq!(skel.len(), 13);
        let frag_a = b"<p>x</p>"; // 8 bytes
        let frag_b = b"<p>y</p>"; // 8 bytes
        let mut raw = skel.clone();
        raw.extend_from_slice(frag_a);
        raw.extend_from_slice(frag_b);
        let skels = vec![SkelEntry {
            frag_count: 2,
            start: 0,
            len: skel.len() as u64,
        }];
        let frags = vec![
            FragEntry {
                insert_pos: 6,
                len: frag_a.len() as u64,
            },
            FragEntry {
                insert_pos: 6,
                len: frag_b.len() as u64,
            },
        ];

        let asm = assemble(&raw, &skels, &frags);
        assert_eq!(asm.parts.len(), 1);
        assert_eq!(asm.parts[0].html, b"<body><p>x</p><p>y</p></body>");
        // 1st fragment: no shift yet, lands right after "<body>" (offset 6).
        assert_eq!(asm.frag_pos[0], (0, 6));
        // 2nd fragment: same raw insert_pos (6), but the 1st fragment (8
        // bytes) already grew the part, so its TRUE final offset is 14 --
        // exactly where "<p>y</p>" actually starts in the assembled part.
        assert_eq!(asm.frag_pos[1], (0, 14));
        assert_eq!(&asm.parts[0].html[14..14 + frag_b.len()], frag_b);

        // A kindle:pos link into the 2nd fragment must resolve using the
        // corrected (post-shift) offset, not the raw pre-splice one.
        assert_eq!(
            resolve_kindle_pos("kindle:pos:fid:0001:off:0000000002", &asm),
            Some((0, 16)) // 14 + 2, landing inside "<p>y</p>"
        );
    }
}
