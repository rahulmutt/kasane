/// Insert `<a id="{prefix}{offset}"></a>` markers into `raw` at each byte
/// offset. Marker ids embed the ORIGINAL offset (links carry the same number,
/// so ids and hrefs meet without a side table). Complexity: O(N + len(raw))
/// where N = distinct offsets. Achieves this via: (1) single-pass rebuild,
/// (2) shared monotone cursor for forward scan to next `<`, (3) per-digit
/// memoization for heading close-tag detection.
pub(crate) fn splice_markers(raw: &mut Vec<u8>, offsets: &[u64], prefix: &str) {
    let mut offs: Vec<u64> = offsets.to_vec();
    offs.sort_unstable();
    offs.dedup();

    // Compute insertion points for each offset (snap-forward + after_heading logic).
    // Collect as (insertion_point, original_offset) pairs sorted ascending by point.
    // Use shared monotone cursor to avoid rescanning tag-free spans.
    let mut insertions: Vec<(usize, u64)> = Vec::with_capacity(offs.len());
    let mut prev_lt: Option<usize> = None;
    let mut heading_memo: [Option<(usize, usize)>; 6] = [None; 6];

    for &o in &offs {
        let start = usize::try_from(o).unwrap_or(raw.len()).min(raw.len());
        // Reuse previous scan result if start <= last found `<`, else continue scanning.
        let lt = if let Some(plt) = prev_lt {
            if start <= plt {
                // Offset is before the < we found last time, reuse it
                plt
            } else {
                // Offset is past the < we found last time, scan from start
                raw[start..]
                    .iter()
                    .position(|&b| b == b'<')
                    .map(|p| start + p)
                    .unwrap_or(raw.len())
            }
        } else {
            // First iteration, scan normally
            raw[start..]
                .iter()
                .position(|&b| b == b'<')
                .map(|p| start + p)
                .unwrap_or(raw.len())
        };
        prev_lt = Some(lt);
        let at = after_heading_with_memo(raw, lt, &mut heading_memo).unwrap_or(lt);
        insertions.push((at, o));
    }
    insertions.sort_unstable_by_key(|&(point, _)| point);

    // Single-pass rebuild: copy spans from raw, emit markers between them.
    let estimate = raw.len() + (insertions.len() * 32);
    let mut output = Vec::with_capacity(estimate);
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
// placed after </hN> resolves to that heading. Memoizes per-digit close-tag
// positions to avoid rescanning in nested heading chains.
fn after_heading_with_memo(
    raw: &[u8],
    lt: usize,
    memo: &mut [Option<(usize, usize)>; 6],
) -> Option<usize> {
    let d = *raw.get(lt + 2)?;
    if raw.get(lt + 1) != Some(&b'h') || !(b'1'..=b'6').contains(&d) {
        return None;
    }
    let digit_idx = (d - b'1') as usize;
    let close: [u8; 4] = [b'<', b'/', b'h', d];

    // Check memo: if memoized close position >= current lt, reuse it.
    if let Some((memoized_close, _)) = memo[digit_idx] {
        if memoized_close >= lt {
            return raw[memoized_close..]
                .iter()
                .position(|&b| b == b'>')
                .map(|p| memoized_close + p + 1);
        }
    }

    // Scan for close tag starting from where we left off or from lt.
    // Clamp to lt to avoid matching orphan closing tags before this heading's open.
    let scan_from = memo[digit_idx].map(|(_, end)| end).unwrap_or(lt).max(lt);
    let cpos = raw[scan_from..]
        .windows(4)
        .position(|w| w == close)
        .map(|p| scan_from + p)?;

    let result = raw[cpos..]
        .iter()
        .position(|&b| b == b'>')
        .map(|p| cpos + p + 1)?;

    memo[digit_idx] = Some((cpos, cpos + 4));
    Some(result)
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

    #[test]
    fn large_tag_free_span_with_many_offsets_avoids_quadratic_rescan() {
        // Adversarial: ~100KB of plain text (no tags) followed by one tag.
        // 10_000 distinct offsets all inside the text. Without optimization,
        // each would rescan up to the trailing tag.
        let mut buffer = String::new();
        for _ in 0..100_000 {
            buffer.push('a');
        }
        buffer.push_str("</p>");
        let mut v = buffer.as_bytes().to_vec();
        let initial_len = v.len();

        // 10_000 distinct offsets inside the tag-free span.
        let mut offsets: Vec<u64> = (0..10_000)
            .map(|i| ((i * (initial_len as u64 - 4)) / 10_000).min(initial_len as u64 - 5))
            .collect();
        offsets.sort_unstable();
        offsets.dedup();
        let count = offsets.len();

        splice_markers(&mut v, &offsets, "kasane-fp-");

        // All markers should appear before the trailing `</p>`.
        let output = String::from_utf8(v).unwrap();
        let marker_count = output.matches("<a id=\"kasane-fp-").count();
        assert_eq!(marker_count, count);
        assert!(output.ends_with("</p>"));
    }

    #[test]
    fn nested_same_digit_headings_avoids_quadratic_close_scan() {
        // Adversarial: deeply nested <h1> tags with an offset at each opening.
        // Without memo per digit, each close-tag scan would run to the far </h1>.
        let mut buffer = String::new();
        let depth = 5_000;
        for _ in 0..depth {
            buffer.push_str("<h1>");
        }
        for _ in 0..depth {
            buffer.push_str("</h1>");
        }
        let mut v = buffer.as_bytes().to_vec();

        // Offset at each <h1> opening.
        let offsets: Vec<u64> = (0..depth).map(|i| (i * 4) as u64).collect();

        splice_markers(&mut v, &offsets, "kasane-fp-");

        // Should produce exactly `depth` markers, all positioned after their </h1>.
        let output = String::from_utf8(v).unwrap();
        let marker_count = output.matches("<a id=\"kasane-fp-").count();
        assert_eq!(marker_count, depth);
    }

    #[test]
    fn regression_memoized_close_scan_does_not_match_orphan_tags() {
        // Critical regression test for Fix round 2 memoization bug.
        // Buffer with orphan closing tag: <h1>A</h1>X</h1><h1>B</h1>
        // When processing offset 16 (second <h1>), memoized close from first heading
        // must not cause rescan to match the orphan </h1> at position 11.
        // Marker for offset 16 must land after the real </h1> at position 25.
        let input = "<h1>A</h1>X</h1><h1>B</h1>";
        let output = splice(input, &[0, 16]);

        // Marker for offset 0 should be before the text between headings.
        // Marker for offset 16 should be after the second heading's closing tag.
        assert!(output.contains("<h1>A</h1><a id=\"kasane-fp-0\"></a>"));
        assert!(output.contains("</h1><a id=\"kasane-fp-16\"></a>"));
        // Verify marker for offset 16 is NOT before the heading.
        assert!(!output.contains("<h1><a id=\"kasane-fp-16\"></a>"));
    }

    #[test]
    fn mixed_digit_interleaving_markers_land_after_correct_heading() {
        // Test interleaving of h1 and h2 tags: <h1>A</h1><h2>B</h2><h1>C</h1>
        // with offsets at each opening tag.
        // Each marker should land after its own heading's closing tag.
        let input = "<h1>A</h1><h2>B</h2><h1>C</h1>";
        let output = splice(input, &[0, 10, 20]);

        // Extract the positions of all markers.
        let h1_marker_0_pos = output.find("</h1><a id=\"kasane-fp-0\"></a>");
        let h2_marker_1_pos = output.find("</h2><a id=\"kasane-fp-10\"></a>");
        let h1_marker_2_pos = output.find("</h1><a id=\"kasane-fp-20\"></a>");

        // All markers should be found (not None).
        assert!(
            h1_marker_0_pos.is_some(),
            "Marker for offset 0 not after first </h1>"
        );
        assert!(
            h2_marker_1_pos.is_some(),
            "Marker for offset 10 not after </h2>"
        );
        assert!(
            h1_marker_2_pos.is_some(),
            "Marker for offset 20 not after second </h1>"
        );

        // Verify we have exactly 3 markers.
        let marker_count = output.matches("<a id=\"kasane-fp-").count();
        assert_eq!(marker_count, 3);
    }
}
