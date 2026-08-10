//! Fuzz seams for `kasane-writer`.
//!
//! A test seam, not API — the same convention and the same rationale as
//! `kasane-core`'s module of this name: it lives inside the crate so it can
//! reach `pub(crate)` internals (`escape::*`) that the separate `fuzz/`
//! workspace cannot.
//!
//! Each function takes `&[u8]` and either returns or panics. A panic **is**
//! the finding.
//!
//! This target asserts **postconditions**, not a round trip through a parser,
//! and that is deliberate: the round trip is P7's job, and a parser here would
//! mean a production dependency on `pulldown-cmark` for every kasane build.
//! The postconditions are the same kind of argument `kasane-core`'s `slug`
//! target makes — untrusted text entering a closed output alphabet.

use crate::escape::{self, Ctx};

pub fn escape(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    for ctx in [Ctx::Flow, Ctx::Cell] {
        for at_line_start in [true, false] {
            let out = escape::text(text, ctx, at_line_start);
            assert!(
                !out.contains('\r'),
                "escape::text kept a CR: {out:?} from {text:?}"
            );
            assert!(
                !out.contains("\n\n"),
                "escape::text kept a blank line: {out:?} from {text:?}"
            );
            if ctx == Ctx::Cell {
                assert!(
                    !out.contains('\n'),
                    "a cell cannot carry a newline: {out:?} from {text:?}"
                );
            }
            assert_unescaped_specials_are_absent(&out, ctx, text);
        }
    }

    // An HTML context must leave no bare `<`, `>` or `&`, except as part of
    // the writer's own literal `<br>` for the first two (Task 2, design spec
    // §3.2: `html_text` renders a newline as `<br>`, unconditionally, the
    // same substitution `Ctx::Cell` makes with a backslash escape
    // everywhere else).
    let html = escape::text(text, Ctx::Html, false);
    assert_html_specials_are_closed(&html, text);

    // A code span's delimiter run must not appear inside its content.
    let span = escape::code_span(text, Ctx::Flow);
    let ticks = span.chars().take_while(|c| *c == '`').count();
    assert!(ticks >= 1, "code_span emitted no delimiter: {span:?}");
    let inner = &span[ticks..span.len() - ticks];
    assert!(
        !inner.contains(&"`".repeat(ticks)),
        "code_span content contains its own delimiter: {span:?} from {text:?}"
    );
    assert!(
        !inner.contains('\n'),
        "code_span kept a newline: {span:?} from {text:?}"
    );

    // A fenced block's fence must not appear at the start of a body line.
    let block = escape::fenced_block(text, Some(text));
    let fence_len = block.chars().take_while(|c| *c == '`').count();
    assert!(
        fence_len >= 3,
        "fence is too short: {fence_len} for {text:?}"
    );
    let fence = "`".repeat(fence_len);
    assert!(
        block.trim_end().ends_with(&fence),
        "fenced_block is not closed: {block:?}"
    );
    let body_start = block.find('\n').map(|i| i + 1).unwrap_or(block.len());
    let body_end = block.trim_end().len() - fence_len;
    if body_start < body_end {
        for line in block[body_start..body_end].lines() {
            assert!(
                !line.starts_with(&fence),
                "a body line reopens the fence: {line:?} from {text:?}"
            );
        }
    }

    // Destinations carry nothing that ends or nests a bare destination.
    for dest in [escape::dest_path(text), escape::dest_url(text)] {
        for bad in [' ', '(', ')', '<', '>', '"', '\\'] {
            assert!(
                !dest.contains(bad),
                "destination contains {bad:?}: {dest:?} from {text:?}"
            );
        }
        assert!(
            !dest.chars().any(|c| c.is_ascii_control()),
            "destination contains a control character: {dest:?} from {text:?}"
        );
    }

    // A YAML scalar is one quoted line with no unescaped interior quote.
    let yaml = escape::yaml_scalar(text);
    assert!(
        yaml.starts_with('"') && yaml.ends_with('"') && yaml.len() >= 2,
        "yaml_scalar is not quoted: {yaml:?} from {text:?}"
    );
    assert!(
        !yaml.contains('\n') && !yaml.chars().any(|c| c.is_control()),
        "yaml_scalar is not one line: {yaml:?} from {text:?}"
    );
    let body = &yaml[1..yaml.len() - 1];
    assert_no_unescaped(body, '"', text);
    assert_yaml_backslashes_are_paired(body, text);
}

/// Every character that can open an inline construct must carry a backslash,
/// except `<` when `ctx == Ctx::Cell` and it is the writer's own literal
/// `<br>` (see `assert_lt_escaped_or_br`). `Ctx::Flow` gets no such
/// exemption — nothing in `escape::text` ever substitutes `<br>` for a
/// newline outside `Ctx::Cell`, so a bare `<` there is unconditionally a
/// defect.
fn assert_unescaped_specials_are_absent(out: &str, ctx: Ctx, from: &str) {
    for c in ['`', '*', '_', '[', ']', '~', '$'] {
        assert_no_unescaped(out, c, from);
    }
    assert_lt_escaped_or_br(out, ctx, from);
}

/// `<` never appears in `s` except immediately after an odd-length run of
/// backslashes, or — only when `ctx == Ctx::Cell` — as the first character of
/// the exact substring `<br>`.
///
/// `Ctx::Cell` renders a newline as a literal `<br>`, not `\n` (a cell cannot
/// carry a newline at all — Task 2, design spec §3.2), so a bare `<` that
/// opens exactly that substring is not a defect *there*. `Ctx::Flow` carries
/// no such substitution — a `\n` in Flow output stays `\n` (asserted above,
/// where a blank line would fail the collapse check) — so in Flow every `<`
/// stays on the strict, unconditional path with no exemption at all.
///
/// Checked per occurrence, not as a whole-string carve-out: a stray
/// unescaped `<` anywhere else in the same output still panics, even when
/// the output elsewhere legitimately contains `<br>`.
fn assert_lt_escaped_or_br(s: &str, ctx: Ctx, from: &str) {
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'<' {
            continue;
        }
        if ctx == Ctx::Cell && bytes[i..].starts_with(b"<br>") {
            continue;
        }
        let run = preceding_backslash_run(bytes, i);
        assert!(
            run % 2 == 1,
            "unescaped '<' at byte {i} in {s:?} from {from:?} (ctx {ctx:?}, not a Cell <br>)"
        );
    }
}

/// `Ctx::Html` (`html_text`) entity-encodes every literal `<`, `>`, `&` and
/// `"`, so the only way one of those can survive bare into its output is
/// either the same `<br>` `html_text` substitutes for a newline (mechanism
/// differs from `Ctx::Cell` — entities, not backslashes — but the newline
/// carve-out is the same one), or, for `&`, being the first character of one
/// of the four entities `html_text` itself emits. A bare `<` must open
/// `<br>`; a bare `>` must close it; a bare `&` must open `&amp;`, `&lt;`,
/// `&gt;` or `&quot;`. Checked per occurrence, so a stray bare `<`/`>`/`&`
/// elsewhere still panics.
fn assert_html_specials_are_closed(html: &str, from: &str) {
    let bytes = html.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'<' => assert!(
                bytes[i..].starts_with(b"<br>"),
                "html escaping left a bare '<' not part of <br>: byte {i} in {html:?} from {from:?}"
            ),
            b'>' => assert!(
                i >= 3 && bytes[i - 3..].starts_with(b"<br>"),
                "html escaping left a bare '>' not part of <br>: byte {i} in {html:?} from {from:?}"
            ),
            b'&' => assert!(
                bytes[i..].starts_with(b"&amp;")
                    || bytes[i..].starts_with(b"&lt;")
                    || bytes[i..].starts_with(b"&gt;")
                    || bytes[i..].starts_with(b"&quot;"),
                "html escaping left a bare '&' not opening a known entity: byte {i} in {html:?} from {from:?}"
            ),
            _ => {}
        }
    }
}

/// Every `\` in a `yaml_scalar` body opens a two-byte pair whose second byte
/// is `\` or `"`.
///
/// This holds unconditionally, for every input — not just a backslash-only
/// one. `yaml_scalar` builds its body with a single left-to-right pass that
/// emits, per input character, one of: `\\` (an input `\`), `\"` (an input
/// `"`), a lone space (a control character), or the character itself
/// carrying no backslash byte at all (everything else, including multi-byte
/// UTF-8 — none of its bytes can equal the single-byte ASCII `\`). So a `\`
/// byte in the body can only ever be the first byte of one of the two escape
/// pairs; it can never originate from a pass-through chunk, and pairs are
/// never split across chunk boundaries.
///
/// The scan must be pairwise-*consuming* (advance by 2 after a match), not
/// position-by-position: a position-by-position check misreads the second
/// backslash of a `\\` pair as a fresh, unpaired backslash whose own next
/// character would then also have to pair, which double-counts every `\\`
/// pair and is exactly the mis-firing shape this replaces (see the doc on
/// `assert_no_unescaped`'s odd-length rule and the task-14 report).
fn assert_yaml_backslashes_are_paired(body: &str, text: &str) {
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            assert!(
                i + 1 < bytes.len() && matches!(bytes[i + 1], b'\\' | b'"'),
                "yaml_scalar left an unpaired backslash at byte {i} in {body:?} from {text:?}"
            );
            i += 2;
        } else {
            i += 1;
        }
    }
}

/// Length of the run of `\` bytes immediately preceding `bytes[i]`.
///
/// Byte-, not char-, indexed: every character this is ever asked about
/// (`` ` * _ [ ] ~ $ " < ``, and `\` itself) is single-byte ASCII, and a
/// UTF-8 continuation or lead byte (0x80..=0xFF) can never equal the
/// single-byte `\` (0x5C), so scanning raw bytes is exact for these targets
/// and avoids the `Vec<char>` collection a char-indexed version would need.
fn preceding_backslash_run(bytes: &[u8], i: usize) -> usize {
    let mut run = 0;
    let mut j = i;
    while j > 0 && bytes[j - 1] == b'\\' {
        run += 1;
        j -= 1;
    }
    run
}

/// `c` never appears in `s` except immediately after an odd-length run of
/// backslashes — i.e. it is always escaped. `c` must be single-byte ASCII
/// (true of every caller: `` ` * _ [ ] ~ $ " ``).
fn assert_no_unescaped(s: &str, c: char, from: &str) {
    debug_assert!(
        c.is_ascii(),
        "assert_no_unescaped target must be ASCII: {c:?}"
    );
    let bytes = s.as_bytes();
    let target = c as u8;
    for (i, &b) in bytes.iter().enumerate() {
        if b != target {
            continue;
        }
        let run = preceding_backslash_run(bytes, i);
        assert!(
            run % 2 == 1,
            "unescaped {c:?} at byte {i} in {s:?} from {from:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These pin the per-occurrence shape of the `<br>` exemptions: a
    // legitimate `<br>` must not turn off checking for the rest of the
    // string, unlike the brief's original `!out.contains(bad) ||
    // out.contains("<br>")` shape (Override 1) -- and the exemption itself
    // must not leak from `Ctx::Cell`, where it is justified, into
    // `Ctx::Flow`, where nothing substitutes `<br>` for a newline.

    #[test]
    fn lt_exemption_allows_the_writers_own_br_in_cell() {
        assert_lt_escaped_or_br("a<br>b", Ctx::Cell, "n/a");
    }

    #[test]
    fn lt_exemption_allows_an_escaped_lt_alongside_a_real_br_in_cell() {
        assert_lt_escaped_or_br("a\\<b<br>c", Ctx::Cell, "n/a");
    }

    #[test]
    #[should_panic(expected = "unescaped '<'")]
    fn lt_exemption_still_catches_a_stray_unescaped_lt_next_to_a_real_br_in_cell() {
        assert_lt_escaped_or_br("<br><x", Ctx::Cell, "n/a");
    }

    #[test]
    #[should_panic(expected = "unescaped '<'")]
    fn lt_exemption_does_not_apply_in_flow() {
        // Same literal `<br>` that Cell would wave through -- Flow has no
        // `<br>` substitution at all, so this must still panic.
        assert_lt_escaped_or_br("<br>", Ctx::Flow, "n/a");
    }

    #[test]
    fn escaped_lt_is_fine_in_flow() {
        assert_lt_escaped_or_br("a\\<b", Ctx::Flow, "n/a");
    }

    #[test]
    fn html_lt_gt_exemption_allows_the_writers_own_br() {
        assert_html_specials_are_closed("a<br>b<br>c", "n/a");
    }

    #[test]
    #[should_panic(expected = "bare '<'")]
    fn html_lt_exemption_still_catches_a_stray_bare_lt_next_to_a_real_br() {
        assert_html_specials_are_closed("<br>x<y", "n/a");
    }

    #[test]
    #[should_panic(expected = "bare '>'")]
    fn html_gt_exemption_still_catches_a_stray_bare_gt() {
        assert_html_specials_are_closed("a>b", "n/a");
    }

    #[test]
    fn html_amp_exemption_allows_the_four_real_entities() {
        assert_html_specials_are_closed("&amp;&lt;&gt;&quot;", "n/a");
    }

    #[test]
    #[should_panic(expected = "bare '&'")]
    fn html_amp_exemption_still_catches_a_bare_amp() {
        assert_html_specials_are_closed("Q&A", "n/a");
    }

    #[test]
    #[should_panic(expected = "bare '&'")]
    fn html_amp_exemption_rejects_an_unknown_entity() {
        // `&copy;` is a real HTML entity but not one `html_text` emits.
        assert_html_specials_are_closed("&copy;", "n/a");
    }

    // Pins the Important-2 fallback: every `\` in a yaml_scalar body pairs
    // with a following `\` or `"`, checked by a pairwise-consuming scan.

    #[test]
    fn yaml_backslashes_pair_for_backslash_only_input() {
        assert_yaml_backslashes_are_paired("\\\\", "\\");
        assert_yaml_backslashes_are_paired("\\\\\\\\", "\\\\");
    }

    #[test]
    fn yaml_backslashes_pair_for_mixed_backslash_and_quote_input() {
        // body for input `\"` (a literal backslash then a literal quote):
        // `\\` (from the backslash) followed by `\"` (from the quote).
        assert_yaml_backslashes_are_paired("\\\\\\\"", "\\\"");
    }

    #[test]
    fn yaml_backslashes_pair_for_quote_only_input() {
        // body for input `"`: `\"`. A single backslash, correctly paired --
        // this is exactly the case that made the old global-parity fallback
        // false in general (total count 1, odd) but is fine here because
        // the pairwise scan checks pairing, not overall parity.
        assert_yaml_backslashes_are_paired("\\\"", "\"");
    }

    #[test]
    #[should_panic(expected = "unpaired backslash")]
    fn yaml_backslashes_catches_a_trailing_unpaired_backslash() {
        assert_yaml_backslashes_are_paired("a\\", "n/a");
    }

    #[test]
    #[should_panic(expected = "unpaired backslash")]
    fn yaml_backslashes_catches_a_backslash_followed_by_something_else() {
        assert_yaml_backslashes_are_paired("\\x", "n/a");
    }

    #[test]
    fn escape_replays_the_backslash_seed_without_panicking() {
        // The exact input that first tripped the original odd-length-rule
        // finding on the yaml body (task-14 report).
        escape(b"a*b_c[d]e`f<g>h~i$j\\k");
    }
}
