//! THROWAWAY stress probe #2. Not for commit.

mod census_support;

use census_support::{alphabet, classify_with, parsed_text, render, Structure};
use kasane_ir::{BlockId, Inline, RefTarget};
use kasane_writer::Ledger;

const WHOLE: u32 = 1 << 0;
const EMST_HEAD: u32 = 1 << 1;
const EMST_TAIL: u32 = 1 << 2;
const STEM_HEAD: u32 = 1 << 3;
const STEM_TAIL: u32 = 1 << 4;

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
            if ex.len() < 3 {
                ex.push(format!("{seq:?} -> {test_md:?}"));
            }
        }
        let base_clean = base_ok && classify_with(&seq, base) == Structure::Clean;
        let test_clean = test_ok && classify_with(&seq, test) == Structure::Clean;
        if base_clean && !test_clean {
            structreg += 1;
        }
    }
    println!("{label:<28} len={len} shapes={total} txtreg={txtreg} structreg={structreg}");
    for e in &ex {
        println!("   {e}");
    }
}

#[test]
#[ignore]
fn every_cell_on_the_full_alphabet_at_length_4() {
    let a = alphabet();
    for (n, b) in [
        ("Whole (control)", WHOLE),
        ("Whole+EmStHead", WHOLE | EMST_HEAD),
        ("Whole+EmStTail", WHOLE | EMST_TAIL),
        ("Whole+StEmHead", WHOLE | STEM_HEAD),
        ("Whole+StEmTail", WHOLE | STEM_TAIL),
        (
            "Whole+all four",
            WHOLE | EMST_HEAD | EMST_TAIL | STEM_HEAD | STEM_TAIL,
        ),
    ] {
        sweep(&a, 4, b, n);
    }
}

#[test]
#[ignore]
fn the_tail_cell_regression_dissected() {
    let seq = vec![
        Inline::Code("x".into()),
        Inline::Emph(vec![Inline::Code("x".into())]),
        Inline::Emph(vec![Inline::Strong(vec![Inline::Text("a".into())])]),
        Inline::Text("a".into()),
    ];
    for (n, b) in [
        ("shipped bit0", WHOLE),
        ("EmStTail", WHOLE | EMST_TAIL),
        ("EmStHead", WHOLE | EMST_HEAD),
    ] {
        let l = Ledger::from_bits(b);
        let md = render(&seq, l);
        println!(
            "{n:<14} md={:<24} recovered={:<10} want={:<10} struct={:?}",
            format!("{:?}", md.trim()),
            format!("{:?}", parsed_text(&md).trim()),
            format!("{:?}", kasane_gfm::rendered_text(&seq).trim()),
            classify_with(&seq, l)
        );
    }
    // Is the collision the cell's doing, or a pre-existing code-span
    // adjacency defect that the cell merely reaches?
    let pre = vec![
        Inline::Code("x".into()),
        Inline::Emph(vec![Inline::Code("x".into())]),
    ];
    let l = Ledger::from_bits(WHOLE);
    println!(
        "control [Code, Emph[Code]]  md={:?} recovered={:?} want={:?}",
        render(&pre, l).trim(),
        parsed_text(&render(&pre, l)).trim(),
        kasane_gfm::rendered_text(&pre).trim()
    );
    let pre2 = vec![
        Inline::Code("x".into()),
        Inline::Emph(vec![Inline::Code("x".into())]),
        Inline::Text("a".into()),
    ];
    println!(
        "control [Code, Emph[Code], a] md={:?} recovered={:?} want={:?}",
        render(&pre2, l).trim(),
        parsed_text(&render(&pre2, l)).trim(),
        kasane_gfm::rendered_text(&pre2).trim()
    );
    let _ = RefTarget::Internal(BlockId(0));
}

#[test]
#[ignore]
fn tail_cell_full_alphabet_length_5() {
    let a = alphabet();
    sweep(&a, 5, WHOLE | EMST_TAIL, "Whole+EmStTail");
}
