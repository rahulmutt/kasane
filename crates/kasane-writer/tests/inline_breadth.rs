//! A wide paragraph must not cost more than linearly in its own breadth.
//!
//! `inline_depth.rs` guards the other axis. Every blowup this writer has
//! measured so far was in nesting *depth*, and every one of them was bounded
//! by `kasane_ir::MAX_INLINE_DEPTH` — bad, but not a denial of service. The
//! breadth axis has no such bound: nothing caps how many inlines an adapter
//! puts in a single `Block::Para`, and this writer sits behind the
//! EPUB/PDF/DjVu/MOBI untrusted-input boundary, so a cost that grows faster
//! than the input does is reachable from a hostile file.

use kasane_ir::*;

/// `k` copies of a container that declines and whose decline replaces one
/// slot in the working view with **two** children.
///
/// The `Emph` declines because its own last printed character is `y`'s closing
/// backtick (punctuation) and the `Text("a")` right after it is alphanumeric,
/// so `can_close(Punct, Other)` is false. Its two `Code` children then take
/// its single slot in `inlines_to_md_flat`'s view.
fn growing_declines(k: usize) -> Vec<Inline> {
    let mut v = Vec::with_capacity(k * 3);
    for _ in 0..k {
        v.push(Inline::Code("x".into()));
        v.push(Inline::Emph(vec![
            Inline::Code("x".into()),
            Inline::Code("y".into()),
        ]));
        v.push(Inline::Text("a".into()));
    }
    v
}

/// The control: the same paragraph with a **single**-child container, whose
/// decline replaces one slot with one child.
///
/// This is what makes the test above a measurement of the growing splice
/// rather than of declines, rollbacks or `run_end` in general. Both shapes
/// decline exactly `k` times and roll back exactly as often; they differ only
/// in whether the replacement grows the view. Before the fix this one stayed
/// linear (5.5 / 12.5 / 23 / 49 ms at n = 8k/16k/32k/64k) while the growing
/// one went 27 ms / 102 ms / 419 ms / 1.66 s.
fn flat_declines(k: usize) -> Vec<Inline> {
    let mut v = Vec::with_capacity(k * 3);
    for _ in 0..k {
        v.push(Inline::Code("x".into()));
        v.push(Inline::Emph(vec![Inline::Code("x".into())]));
        v.push(Inline::Text("a".into()));
    }
    v
}

fn render(seq: Vec<Inline>) -> (String, std::time::Duration) {
    let para = Block::Para(seq);
    let start = std::time::Instant::now();
    let md = kasane_writer::blocks_to_markdown(&[para], &AssetBag::default());
    (md, start.elapsed())
}

/// A declining container with more than one child must not make the paragraph
/// quadratic in its breadth.
///
/// The defect this pins: `inlines_to_md_flat` used to hold its working view as
/// one `Vec` and rewrite a declined run with `items.splice(i..end, children)`.
/// When the run is one container and it hands back two or more children, that
/// splice grows the vector and memmoves the entire untouched tail, so `k` such
/// declines in one paragraph cost O(k*n). The final review of
/// `declined-run-rescan` measured it in release at 27 ms / 102 ms / 419 ms /
/// 1.66 s for n = 8k/16k/32k/64k — 4.0x per doubling, against the
/// pre-decline renderer's exact 2.0x — while the single-child control above
/// stayed linear throughout, isolating the growing splice as the cause. The
/// fix splits the view at the cursor and holds the unscanned half reversed, so
/// a decline rewrites only the run's own slot and the tail never moves; the
/// same four points then read 7.4 / 14.6 / 26.8 / 54.4 ms.
///
/// **Ratio, not wall clock.** An absolute millisecond bound on a shared CI
/// runner is either so loose it catches nothing or so tight it flakes. What
/// separates linear from quadratic is the *shape* of the curve, so this
/// measures both breadths in the same process, back to back, and asserts the
/// growth factor. A doubling of `n` may cost at most `MAX_RATIO` times as
/// much; genuinely linear is 2.0, the defect was 4.0. The guard also compares
/// against the single-child control at the same breadth, which is the same
/// paragraph with the same number of declines and rollbacks and only the
/// splice growth removed — under the defect that ratio was 34x at n = 64k.
///
/// **What this does not cover.** One shape, one ledger (the shipped one), one
/// axis. It says nothing about nesting depth (`inline_depth.rs` owns that) and
/// nothing about the *census* corpus, which cannot reach this defect at all:
/// every container in the census alphabet has exactly one child, so its
/// declines splice one slot into one slot and never grow the view. Measured,
/// not assumed — instrumenting the decline arm over the census alphabet at
/// lengths 3 and 4 across all 128 ledgers counts 61,864 declines and **zero**
/// growing splices, against 95,194 growing splices over a multi-child corpus.
#[test]
fn a_paragraph_of_declining_multi_child_containers_scales_with_its_breadth() {
    // The doubling factor a linear pass is allowed. 2.0 is exact linearity;
    // the headroom absorbs allocator and cache noise on a shared runner. The
    // defect measured 4.0, so it fails this by a wide margin at every step.
    const MAX_RATIO: f64 = 2.9;
    // The control ratio: the growing-splice paragraph may cost at most this
    // much more than the equal-length single-child one. The defect was 34x.
    const MAX_VS_CONTROL: f64 = 4.0;

    let breadths = [8_000usize, 16_000, 32_000, 64_000];
    let mut grown = Vec::new();
    let mut flat = Vec::new();
    for &k in &breadths {
        let (md, t) = render(growing_declines(k));
        assert!(!md.is_empty(), "rendering must produce output");
        grown.push(t);
        let (md, t) = render(flat_declines(k));
        assert!(!md.is_empty(), "rendering must produce output");
        flat.push(t);
    }
    let report = || {
        breadths
            .iter()
            .zip(&grown)
            .zip(&flat)
            .map(|((k, g), f)| format!("n={k}: multi-child {g:?}, single-child control {f:?}"))
            .collect::<Vec<_>>()
            .join("\n  ")
    };

    for w in 1..breadths.len() {
        // Skip a step whose baseline is too small to divide by: on a fast
        // machine the first point can land near the timer's own resolution,
        // and a ratio taken there measures noise. The later steps are the
        // ones that separate 2x from 4x anyway.
        if grown[w - 1] < std::time::Duration::from_millis(1) {
            continue;
        }
        let ratio = grown[w].as_secs_f64() / grown[w - 1].as_secs_f64();
        assert!(
            ratio <= MAX_RATIO,
            "doubling the paragraph's breadth from {} to {} multiplied the render cost by \
             {ratio:.2}x, over the {MAX_RATIO}x a linear pass may cost -- this is the \
             growing-splice quadratic in paragraph breadth again\n  {}",
            breadths[w - 1],
            breadths[w],
            report()
        );
    }
    let widest = breadths.len() - 1;
    let vs_control = grown[widest].as_secs_f64() / flat[widest].as_secs_f64();
    assert!(
        vs_control <= MAX_VS_CONTROL,
        "at n={}, the multi-child paragraph cost {vs_control:.1}x the single-child control \
         with the same number of declines and rollbacks; only the splice growth differs, so \
         this is the growing-splice quadratic again\n  {}",
        breadths[widest],
        report()
    );
}
