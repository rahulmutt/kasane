/// Insert `<a id="{prefix}{offset}"></a>` markers into `raw` at each byte
/// offset. Marker ids embed the ORIGINAL offset (links carry the same number,
/// so ids and hrefs meet without a side table). Insertions run in descending
/// offset order so earlier offsets stay valid while later text shifts.
pub(crate) fn splice_markers(raw: &mut Vec<u8>, offsets: &[u64], prefix: &str) {
    let mut offs: Vec<u64> = offsets.to_vec();
    offs.sort_unstable();
    offs.dedup();
    for &o in offs.iter().rev() {
        let start = (o as usize).min(raw.len());
        let lt = raw[start..]
            .iter()
            .position(|&b| b == b'<')
            .map(|p| start + p)
            .unwrap_or(raw.len());
        let at = after_heading(raw, lt).unwrap_or(lt);
        let marker = format!("<a id=\"{prefix}{o}\"></a>");
        raw.splice(at..at, marker.into_bytes());
    }
}

// If raw[lt..] opens <h1>..<h6>, return the position just after its matching
// closing tag. xhtml_to_blocks assigns a heading's BlockId at its End event
// and maps other ids to the nearest PRECEDING heading, so only a marker
// placed after </hN> resolves to that heading.
fn after_heading(raw: &[u8], lt: usize) -> Option<usize> {
    let d = *raw.get(lt + 2)?;
    if raw.get(lt + 1) != Some(&b'h') || !(b'1'..=b'6').contains(&d) {
        return None;
    }
    let close: [u8; 4] = [b'<', b'/', b'h', d];
    let cpos = raw[lt..]
        .windows(4)
        .position(|w| w == close)
        .map(|p| lt + p)?;
    raw[cpos..]
        .iter()
        .position(|&b| b == b'>')
        .map(|p| cpos + p + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn splice(s: &str, offs: &[u64]) -> String {
        let mut v = s.as_bytes().to_vec();
        splice_markers(&mut v, offs, "kasane-fp-");
        String::from_utf8(v).unwrap()
    }

    #[test]
    fn snaps_forward_to_next_tag() {
        // offset 4 is inside "one"; marker lands before <p>two
        assert_eq!(
            splice("<p>one</p><p>two</p>", &[4]),
            "<p>one<a id=\"kasane-fp-4\"></a></p><p>two</p>"
        );
    }

    #[test]
    fn offset_at_tag_start_lands_before_it() {
        assert_eq!(
            splice("<p>a</p><p>b</p>", &[8]),
            "<p>a</p><a id=\"kasane-fp-8\"></a><p>b</p>"
        );
    }

    #[test]
    fn heading_targets_land_after_the_heading() {
        // Heading BlockIds are assigned at the heading's End event, and other
        // ids map to the nearest PRECEDING heading — so the marker must sit
        // after </h2> to resolve to that heading.
        assert_eq!(
            splice("<p>a</p><h2>Two</h2>", &[8]),
            "<p>a</p><h2>Two</h2><a id=\"kasane-fp-8\"></a>"
        );
    }

    #[test]
    fn multiple_offsets_keep_original_ids_and_positions() {
        let out = splice("<p>aaaa</p><p>bbbb</p><p>cccc</p>", &[13, 2]);
        assert!(out.contains("<p>aaaa<a id=\"kasane-fp-2\"></a></p>"));
        assert!(out.contains("<p>bbbb<a id=\"kasane-fp-13\"></a></p>"));
    }

    #[test]
    fn oob_offset_appends_at_end_and_duplicates_dedup() {
        let out = splice("<p>x</p>", &[999, 999]);
        assert_eq!(out, "<p>x</p><a id=\"kasane-fp-999\"></a>");
    }
}
