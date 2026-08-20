//! THROWAWAY probe #3. Not for commit.

mod census_support;

use census_support::{alphabet, classify_with, parsed_text, render, text_is_clean, Structure};
use kasane_ir::Inline;
use kasane_writer::Ledger;

const WHOLE: u32 = 1 << 0;
const EMST_TAIL: u32 = 1 << 2;

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

fn deep_shapes() -> Vec<Vec<Inline>> {
    let a = deep_alphabet();
    let n = a.len();
    let mut out = Vec::new();
    for len in 4..=5u32 {
        for code in 0..n.pow(len) {
            let mut c = code;
            let mut seq = Vec::with_capacity(len as usize);
            for _ in 0..len {
                seq.push(a[c % n].clone());
                c /= n;
            }
            out.push(seq);
        }
    }
    out
}

fn has_em_over_st(seq: &[Inline]) -> bool {
    seq.iter()
        .any(|i| matches!(i, Inline::Emph(x) if matches!(x.as_slice(), [Inline::Strong(_)])))
}

#[test]
#[ignore]
fn deep_tier_composition_on_unmodified_main() {
    let mut kept = 0usize;
    let mut text_bad = 0usize;
    let mut struct_bad = 0usize;
    let mut bad_with_emst = 0usize;
    let mut bad_without: Vec<String> = Vec::new();
    let mut corrupt_kind = (0usize, 0usize); // (Corrupt, Inexpressible)
    for seq in deep_shapes() {
        let lic = render(&seq, Ledger::LICENSED);
        if lic == render(&seq, Ledger::CONSERVATIVE) {
            continue;
        }
        kept += 1;
        if !text_is_clean(&seq, Ledger::LICENSED) {
            text_bad += 1;
        } else {
            match classify_with(&seq, Ledger::LICENSED) {
                Structure::Clean => continue,
                Structure::Corrupt => corrupt_kind.0 += 1,
                Structure::Inexpressible => corrupt_kind.1 += 1,
            }
            struct_bad += 1;
            if has_em_over_st(&seq) {
                bad_with_emst += 1;
            } else if bad_without.len() < 5 {
                bad_without.push(format!("{seq:?} -> {lic:?}"));
            }
        }
    }
    println!("deep tier vs CONSERVATIVE on unmodified writer (LICENSED = bit0 only):");
    println!("  kept={kept} text_failures={text_bad} structure_failures={struct_bad}");
    println!(
        "  of the structure failures: Corrupt={} Inexpressible={} with a top-level Emph[Strong[..]]={}",
        corrupt_kind.0, corrupt_kind.1, bad_with_emst
    );
    for e in &bad_without {
        println!("  no-EmSt example: {e}");
    }
}

#[test]
#[ignore]
fn tail_cell_full_alphabet_length_5() {
    let a = alphabet();
    let n = a.len();
    let base = Ledger::from_bits(WHOLE);
    let test = Ledger::from_bits(WHOLE | EMST_TAIL);
    let mut txtreg = 0usize;
    let mut structreg = 0usize;
    let mut ex: Vec<String> = Vec::new();
    let total = n.pow(5);
    for code in 0..total {
        let mut c = code;
        let mut seq = Vec::with_capacity(5);
        for _ in 0..5 {
            seq.push(a[c % n].clone());
            c /= n;
        }
        let base_md = render(&seq, base);
        let test_md = render(&seq, test);
        if base_md == test_md {
            continue;
        }
        let want = kasane_gfm::rendered_text(&seq);
        let base_ok = parsed_text(&base_md).trim() == want.trim();
        let test_ok = parsed_text(&test_md).trim() == want.trim();
        if base_ok && !test_ok {
            txtreg += 1;
            if ex.len() < 5 {
                ex.push(format!("{seq:?} -> {test_md:?}"));
            }
        }
        if base_ok
            && classify_with(&seq, base) == Structure::Clean
            && !(test_ok && classify_with(&seq, test) == Structure::Clean)
        {
            structreg += 1;
        }
    }
    println!("EmStTail, 19-element alphabet, length 5: shapes={total} txtreg={txtreg} structreg={structreg}");
    for e in &ex {
        println!("   {e}");
    }
}
