mod census_support;
use census_support::{alphabet, text_is_clean};
use kasane_ir::Inline;
use kasane_writer::{probe, Ledger};
#[test]
fn len5_under_reorder() {
    let a = alphabet();
    let n = a.len();
    let mut bad: Vec<String> = Vec::new();
    let total = n.pow(5);
    for m in 0..total {
        let mut x = m;
        let mut seq = Vec::with_capacity(5);
        for _ in 0..5 { seq.push(a[x % n].clone()); x /= n; }
        probe::reset(3);
        if !text_is_clean(&seq, Ledger::LICENSED) { bad.push(format!("{seq:?}")); }
    }
    println!("reorder length-5 text-corrupt = {} / {}", bad.len(), total);
    for s in bad.iter().take(8) { println!("    {s}"); }
}
