//! Design spec §5: every spelling this item newly licensed round-trips.
//!
//! A second corpus, seven delimiter-bearing elements at lengths 4 and 5. Each
//! shape renders twice — under `Ledger::LICENSED` and under
//! `Ledger::CONSERVATIVE` — and **only shapes whose renderings differ** are
//! kept and asserted clean. The kept set is therefore exactly the spellings
//! this item licensed, computed by differencing rather than by matching cases
//! someone anticipated. That is deliberate: design spec §4.3's flanking
//! interaction changes the rendering, so it lands in the kept set whether or
//! not anyone predicted it.
//!
//! No allowlist and no queue. A corrupt shape here is a wrong cell in
//! `may_abut`, not a residual.
//!
//! Lengths 4 and 5 rather than 3 because the licensed configurations appear at
//! length 3 but their *consequences* do not: a flanking flip needs a licensed
//! abutment with a neighbour on both sides (four elements), and a second
//! abutment whose flank class the first one's shortened run changed needs five.

mod census_support;

use census_support::{classify_with, render, text_is_clean, Structure};
use kasane_ir::Inline;
use kasane_writer::Ledger;

/// Seven elements, chosen to put both emphasis classes beside each other and
/// beside a non-emphasis delimiter. Smaller than the length-3 census's
/// nineteen because 7^5 is already 16,807 shapes.
fn deep_alphabet() -> Vec<Inline> {
    let t = |s: &str| Inline::Text(s.to_string());
    let em = |i: Inline| Inline::Emph(vec![i]);
    let st = |i: Inline| Inline::Strong(vec![i]);
    vec![
        t("a"),
        t("*"),
        Inline::Code("x".into()),
        em(t("a")),
        st(t("a")),
        em(st(t("a"))),
        st(em(t("a"))),
    ]
}

/// Every sequence of length 4 and 5 over [`deep_alphabet`].
///
/// 7^4 + 7^5 = 2,401 + 16,807 = 19,208 shapes. Each index is one digit of a
/// base-7 counter, which is simply an exhaustive product and is worth keeping
/// obviously correct: a subtly wrong generator would silently shrink the
/// corpus, and this tier's only guard against that is `kept > 0`.
fn deep_shapes() -> Vec<Vec<Inline>> {
    let a = deep_alphabet();
    let n = a.len();
    let mut out = Vec::new();
    for len in 4..=5u32 {
        for code in 0..n.pow(len) {
            let mut code = code;
            let mut seq = Vec::with_capacity(len as usize);
            for _ in 0..len {
                seq.push(a[code % n].clone());
                code /= n;
            }
            out.push(seq);
        }
    }
    out
}

#[test]
fn every_newly_licensed_spelling_round_trips() {
    let mut kept = 0usize;
    let mut bad: Vec<String> = Vec::new();

    for seq in deep_shapes() {
        let licensed = render(&seq, Ledger::LICENSED);
        if licensed == render(&seq, Ledger::CONSERVATIVE) {
            continue;
        }
        kept += 1;

        // Both tiers, and the text tier first: `classify_with` returns `Clean`
        // when the text is already corrupt, because the length-3 census gates
        // structure on text and names text corruption with its own assertion.
        // Nothing here names it, so this must ask both questions.
        if !text_is_clean(&seq, Ledger::LICENSED) {
            bad.push(format!("text: {seq:?} -> {licensed:?}"));
        } else if classify_with(&seq, Ledger::LICENSED) != Structure::Clean {
            bad.push(format!("structure: {seq:?} -> {licensed:?}"));
        }
    }

    // The filter is the tier. If it ever matches nothing -- a cell turned off,
    // a corpus that stopped reaching the licensed configurations -- this test
    // would pass vacuously and go on passing forever.
    assert!(
        kept > 0,
        "the differencing filter matched no shapes, so this tier proved nothing"
    );

    assert!(
        bad.is_empty(),
        "{} newly-licensed spelling(s) do not round-trip -- a cell in \
         `may_abut` is wrong, not a residual to be recorded:\n{}",
        bad.len(),
        bad.iter()
            .take(10)
            .map(|s| format!("  {s}\n"))
            .collect::<String>()
    );
}
