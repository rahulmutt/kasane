mod census_support;
use census_support::{alphabet, text_is_clean};
use kasane_ir::Inline;
use kasane_writer::{probe, Ledger};
#[test]
fn len4_under_reorder() {
    let a = alphabet();
    let n = a.len();
    for (mode, label) in [(0u64, "ship"), (3, "reorder")] {
        let mut bad: Vec<String> = Vec::new();
        let mut idx = [0usize; 4];
        loop {
            let seq: Vec<Inline> = idx.iter().map(|&k| a[k].clone()).collect();
            probe::reset(mode);
            if !text_is_clean(&seq, Ledger::LICENSED) { bad.push(format!("{seq:?}")); }
            let mut k = 4;
            loop {
                if k == 0 { break; }
                k -= 1;
                idx[k] += 1;
                if idx[k] < n { break; }
                idx[k] = 0;
                if k == 0 { idx = [n; 4]; break; }
            }
            if idx[0] >= n { break; }
        }
        println!("{label:<8} length-4 text-corrupt = {} / {}", bad.len(), n.pow(4));
        for x in bad.iter().take(8) { println!("    {x}"); }
    }
}
