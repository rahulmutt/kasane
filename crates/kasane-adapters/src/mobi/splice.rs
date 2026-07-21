/// Insert `<a id="{prefix}{offset}"></a>` markers into `raw` at each byte
/// offset. Marker ids embed the ORIGINAL offset (links carry the same number,
/// so ids and hrefs meet without a side table). Performs single-pass rebuild
/// to avoid quadratic cost: O(N + len(raw)) where N = distinct offsets.
pub(crate) fn splice_markers(raw: &mut Vec<u8>, offsets: &[u64], prefix: &str) {
    let mut offs: Vec<u64> = offsets.to_vec();
    offs.sort_unstable();
    offs.dedup();

    // Compute insertion points for each offset (snap-forward + after_heading logic).
    // Collect as (insertion_point, original_offset) pairs sorted ascending by point.
    let mut insertions: Vec<(usize, u64)> = offs
        .iter()
        .map(|&o| {
            let start = usize::try_from(o).unwrap_or(raw.len()).min(raw.len());
            let lt = raw[start..]
                .iter()
                .position(|&b| b == b'<')
                .map(|p| start + p)
                .unwrap_or(raw.len());
            let at = after_heading(raw, lt).unwrap_or(lt);
            (at, o)
        })
        .collect();
    insertions.sort_unstable_by_key(|&(point, _)| point);

    // Single-pass rebuild: copy spans from raw, emit markers between them.
    let mut output = Vec::new();
    let mut pos = 0;
    for &(at, o) in &insertions {
        if at > pos {
            output.extend_from_slice(&raw[pos..at]);
        }
        let marker = format!("<a id=\"{prefix}{o}\"></a>");
        output.extend_from_slice(marker.as_bytes());
        pos = at;
    }
    if pos < raw.len() {
        output.extend_from_slice(&raw[pos..]);
    }
    *raw = output;
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

    #[test]
    fn many_offsets_completes_quickly() {
        // Generate ~100KB buffer with repeating HTML and 10_000 distinct offsets.
        let html_chunk = "<p>abcdefghij</p>";
        let mut buffer = String::new();
        for _ in 0..650 {
            buffer.push_str(html_chunk);
        }
        let mut v = buffer.as_bytes().to_vec();
        let initial_len = v.len();

        // Generate 10_000 distinct offsets spread across buffer.
        let mut offsets: Vec<u64> = (0..10_000)
            .map(|i| (i * initial_len as u64) / 10_000)
            .collect();
        offsets.sort_unstable();
        offsets.dedup();
        let count = offsets.len();

        splice_markers(&mut v, &offsets, "kasane-fp-");

        // Count markers in output: each is 32 + 5 + digits bytes roughly.
        let output = String::from_utf8(v).unwrap();
        let marker_count = output.matches("<a id=\"kasane-fp-").count();
        assert_eq!(marker_count, count);
    }
}
