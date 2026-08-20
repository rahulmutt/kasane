//! THROWAWAY probe #4: what family are the edge cells' text regressions?
//! Not for commit.

mod census_support;

use census_support::{alphabet, parsed_text, render};
use kasane_ir::Inline;
use kasane_writer::Ledger;

const WHOLE: u32 = 1 << 0;
const EMST_HEAD: u32 = 1 << 1;
const EMST_TAIL: u32 = 1 << 2;
const STEM_HEAD: u32 = 1 << 3;
const STEM_TAIL: u32 = 1 << 4;

fn families(bits: u32, label: &str, lens: &[u32]) {
    let a = alphabet();
    let n = a.len();
    let base = Ledger::from_bits(WHOLE);
    let test = Ledger::from_bits(bits);
    let mut backtick = 0usize;
    let mut asterisk = 0usize;
    let mut ex: Vec<String> = Vec::new();
    for &len in lens {
        for code in 0..n.pow(len) {
            let mut c = code;
            let mut seq = Vec::with_capacity(len as usize);
            for _ in 0..len {
                seq.push(a[c % n].clone());
                c /= n;
            }
            let base_md = render(&seq, base);
            let test_md = render(&seq, test);
            if base_md == test_md {
                continue;
            }
            let want = kasane_gfm::rendered_text(&seq);
            if parsed_text(&base_md).trim() != want.trim() {
                continue;
            }
            if parsed_text(&test_md).trim() == want.trim() {
                continue;
            }
            // Which delimiter is doing the damage? A recovered string that
            // contains a stray backtick is the known code-span adjacency
            // family; a stray asterisk is a delimiter-pairing failure.
            let rec = parsed_text(&test_md);
            if rec.contains('`') {
                backtick += 1;
            } else {
                asterisk += 1;
                if ex.len() < 5 {
                    ex.push(format!("{seq:?} -> {test_md:?} rec={:?}", rec.trim()));
                }
            }
        }
    }
    println!("{label}: backtick-family={backtick} asterisk-family={asterisk}");
    for e in &ex {
        println!("   {e}");
    }
}

#[test]
#[ignore]
fn regression_families() {
    for (n, b) in [
        ("EmStTail", WHOLE | EMST_TAIL),
        ("StEmTail", WHOLE | STEM_TAIL),
        ("EmStHead", WHOLE | EMST_HEAD),
        ("StEmHead", WHOLE | STEM_HEAD),
    ] {
        families(b, n, &[4, 5]);
    }
}

/// The proptest P13 shrink case, checked deterministically.
#[test]
#[ignore]
fn the_p13_shrink_case() {
    let t = |s: &str| Inline::Text(s.to_string());
    let em = |v: Vec<Inline>| Inline::Emph(v);
    let st = |v: Vec<Inline>| Inline::Strong(v);
    let seq = vec![
        st(vec![em(vec![t("a")])]),
        em(vec![t("a")]),
        em(vec![em(vec![t("a")])]),
    ];
    let five = WHOLE | EMST_HEAD | EMST_TAIL | STEM_HEAD | STEM_TAIL;
    for (n, b) in [("shipped bit0", WHOLE), ("five cells", five)] {
        let md = render(&seq, Ledger::from_bits(b));
        println!(
            "{n:<14} md={:?} recovered={:?} want={:?}",
            md.trim(),
            parsed_text(&md).trim(),
            kasane_gfm::rendered_text(&seq).trim()
        );
    }
}
