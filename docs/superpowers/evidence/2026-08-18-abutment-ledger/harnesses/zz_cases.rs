//! THROWAWAY case-by-case probe (task-4 independent check). Not for commit.

mod census_support;

use census_support::{classify_with, parsed_text, render, Structure};
use kasane_ir::Inline;
use kasane_writer::Ledger;

const WHOLE: u32 = 1 << 0;
const EMST_HEAD: u32 = 1 << 1;
const EMST_TAIL: u32 = 1 << 2;
const STEM_HEAD: u32 = 1 << 3;
const STEM_TAIL: u32 = 1 << 4;

fn t(s: &str) -> Inline {
    Inline::Text(s.to_string())
}
fn em(v: Vec<Inline>) -> Inline {
    Inline::Emph(v)
}
fn st(v: Vec<Inline>) -> Inline {
    Inline::Strong(v)
}

fn show(label: &str, seq: &[Inline], bits: u32) {
    let l = Ledger::from_bits(bits);
    let md = render(seq, l);
    let rec = parsed_text(&md);
    let want = kasane_gfm::rendered_text(seq);
    println!(
        "{label:<34} md={:<26} recovered={:<12} want={:<10} textok={} struct={:?}",
        format!("{:?}", md.trim()),
        format!("{:?}", rec.trim()),
        format!("{:?}", want.trim()),
        rec.trim() == want.trim(),
        classify_with(seq, l),
    );
}

#[test]
#[ignore]
fn blocker_a_cases() {
    let four = WHOLE | EMST_HEAD | EMST_TAIL | STEM_HEAD | STEM_TAIL;
    let cases: Vec<(&str, Vec<Inline>)> = vec![
        (
            "EmSt/Em/EmSt",
            vec![
                em(vec![st(vec![t("a")])]),
                em(vec![t("a")]),
                em(vec![st(vec![t("a")])]),
            ],
        ),
        (
            "StEm/Em/StEm",
            vec![
                st(vec![em(vec![t("a")])]),
                em(vec![t("a")]),
                st(vec![em(vec![t("a")])]),
            ],
        ),
    ];
    for (n, seq) in &cases {
        show(&format!("{n} [shipped bit0]"), seq, WHOLE);
        show(&format!("{n} [4 cells]"), seq, four);
        show(&format!("{n} [head only]"), seq, WHOLE | EMST_HEAD);
        show(&format!("{n} [tail only]"), seq, WHOLE | EMST_TAIL);
        show(
            &format!("{n} [EmStHead+EmStTail]"),
            seq,
            WHOLE | EMST_HEAD | EMST_TAIL,
        );
        show(
            &format!("{n} [StEmHead+StEmTail]"),
            seq,
            WHOLE | STEM_HEAD | STEM_TAIL,
        );
        println!();
    }
}

#[test]
#[ignore]
fn blocker_b_case() {
    let seq = vec![t("a"), st(vec![em(vec![t("a")])]), st(vec![t("a")])];
    show("B [shipped bit0]", &seq, WHOLE);
    show("B [StEmHead]", &seq, WHOLE | STEM_HEAD);
    show("B [StEmTail]", &seq, WHOLE | STEM_TAIL);
    show("B [StEmHead+StEmTail]", &seq, WHOLE | STEM_HEAD | STEM_TAIL);
    show("B [EmStHead]", &seq, WHOLE | EMST_HEAD);
    show("B [EmStTail]", &seq, WHOLE | EMST_TAIL);
}

/// The head/tail asymmetry at length 4: does the mirror image of the
/// head-only failure exist for tail-only?
#[test]
#[ignore]
fn head_tail_asymmetry() {
    let hd = WHOLE | EMST_HEAD;
    let tl = WHOLE | EMST_TAIL;
    let emst = || em(vec![st(vec![t("a")])]);
    let ema = || em(vec![t("a")]);

    println!("-- head-only failing family and its mirror --");
    show(
        "H: [EmSt,Em,EmSt,Em] head",
        &[emst(), ema(), emst(), ema()],
        hd,
    );
    show(
        "H: [EmSt,Em,EmSt,Em] tail",
        &[emst(), ema(), emst(), ema()],
        tl,
    );
    show(
        "T: [Em,EmSt,Em,EmSt] head",
        &[ema(), emst(), ema(), emst()],
        hd,
    );
    show(
        "T: [Em,EmSt,Em,EmSt] tail",
        &[ema(), emst(), ema(), emst()],
        tl,
    );
    show(
        "T: [Em,EmSt,Em,EmSt] both",
        &[ema(), emst(), ema(), emst()],
        hd | tl,
    );
    println!();
    println!("-- minimal head-only cases --");
    show("[EmSt,Em] head", &[emst(), ema()], hd);
    show("[EmSt,Em,Em] head", &[emst(), ema(), ema()], hd);
    show("[EmSt,Em,EmSt] head", &[emst(), ema(), emst()], hd);
    show("[EmSt,Em,EmSt] tail", &[emst(), ema(), emst()], tl);
    show(
        "[EmSt,Em,EmSt,Em] head",
        &[emst(), ema(), emst(), ema()],
        hd,
    );
    show(
        "[EmSt,Em,Em,EmSt] head",
        &[emst(), ema(), ema(), emst()],
        hd,
    );
    show(
        "[EmSt,Em,Em,EmSt] tail",
        &[emst(), ema(), ema(), emst()],
        tl,
    );
}

/// Does the tail cell stay clean at length 6/7 too? (Not part of the brief's
/// corpora; a robustness probe for the survivor.)
#[test]
#[ignore]
fn tail_only_at_lengths_6_and_7() {
    let alpha = {
        let em1 = em(vec![t("a")]);
        let st1 = st(vec![t("a")]);
        vec![
            t("a"),
            t("*"),
            Inline::Code("x".into()),
            em1,
            st1,
            em(vec![st(vec![t("a")])]),
            st(vec![em(vec![t("a")])]),
        ]
    };
    let n = alpha.len();
    for len in [6u32, 7u32] {
        let mut txtreg = 0usize;
        let mut structreg = 0usize;
        let mut ex: Vec<String> = Vec::new();
        for code in 0..n.pow(len) {
            let mut c = code;
            let mut seq = Vec::with_capacity(len as usize);
            for _ in 0..len {
                seq.push(alpha[c % n].clone());
                c /= n;
            }
            let base = Ledger::from_bits(WHOLE);
            let test = Ledger::from_bits(WHOLE | EMST_TAIL);
            let bt = parsed_text(&render(&seq, base));
            let want = kasane_gfm::rendered_text(&seq);
            let tt = parsed_text(&render(&seq, test));
            let base_ok = bt.trim() == want.trim();
            let test_ok = tt.trim() == want.trim();
            if base_ok && !test_ok {
                txtreg += 1;
                if ex.len() < 5 {
                    ex.push(format!("{seq:?} -> {:?}", render(&seq, test)));
                }
            }
            if base_ok
                && classify_with(&seq, base) == Structure::Clean
                && !(test_ok && classify_with(&seq, test) == Structure::Clean)
            {
                structreg += 1;
            }
        }
        println!("tail-only, length {len}: txtreg={txtreg} structreg={structreg}");
        for e in &ex {
            println!("   {e}");
        }
    }
}
