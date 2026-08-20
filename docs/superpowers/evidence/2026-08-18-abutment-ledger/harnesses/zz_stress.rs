//! THROWAWAY stress probe for the surviving cell. Not for commit.

mod census_support;

use census_support::{alphabet, classify_with, parsed_text, render, Structure};
use kasane_ir::Inline;
use kasane_writer::Ledger;

const WHOLE: u32 = 1 << 0;
const EMST_HEAD: u32 = 1 << 1;
const EMST_TAIL: u32 = 1 << 2;
const STEM_TAIL: u32 = 1 << 4;

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

fn sweep(alpha: &[Inline], len: u32, bits: u32, label: &str) {
    let n = alpha.len();
    let base = Ledger::from_bits(WHOLE);
    let test = Ledger::from_bits(bits);
    let mut txtreg = 0usize;
    let mut structreg = 0usize;
    let mut ex: Vec<String> = Vec::new();
    let total = n.pow(len);
    for code in 0..total {
        let mut c = code;
        let mut seq = Vec::with_capacity(len as usize);
        for _ in 0..len {
            seq.push(alpha[c % n].clone());
            c /= n;
        }
        let want = kasane_gfm::rendered_text(&seq);
        let base_md = render(&seq, base);
        let test_md = render(&seq, test);
        if base_md == test_md {
            continue;
        }
        let base_ok = parsed_text(&base_md).trim() == want.trim();
        let test_ok = parsed_text(&test_md).trim() == want.trim();
        if base_ok && !test_ok {
            txtreg += 1;
            if ex.len() < 5 {
                ex.push(format!("{seq:?} -> {test_md:?}"));
            }
        }
        let base_clean = base_ok && classify_with(&seq, base) == Structure::Clean;
        let test_clean = test_ok && classify_with(&seq, test) == Structure::Clean;
        if base_clean && !test_clean {
            structreg += 1;
        }
    }
    println!("{label}: len={len} shapes={total} txtreg={txtreg} structreg={structreg}");
    for e in &ex {
        println!("   {e}");
    }
}

#[test]
#[ignore]
fn tail_only_deep_alphabet_lengths_6_7() {
    let a = deep_alphabet();
    sweep(&a, 6, WHOLE | EMST_TAIL, "EmStTail/7elem");
    sweep(&a, 7, WHOLE | EMST_TAIL, "EmStTail/7elem");
}

#[test]
#[ignore]
fn tail_only_full_alphabet_length_4() {
    let a = alphabet();
    sweep(&a, 4, WHOLE | EMST_TAIL, "EmStTail/19elem");
    sweep(&a, 4, WHOLE | EMST_HEAD, "EmStHead/19elem");
    sweep(&a, 4, WHOLE | STEM_TAIL, "StEmTail/19elem");
}

#[test]
#[ignore]
fn tail_only_full_alphabet_length_5() {
    let a = alphabet();
    sweep(&a, 5, WHOLE | EMST_TAIL, "EmStTail/19elem");
}
