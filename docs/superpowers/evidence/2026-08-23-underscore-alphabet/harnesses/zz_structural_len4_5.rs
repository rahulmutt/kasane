//! THROWAWAY structural probe for Task 4's post-implementation verification.
//!
//! `census.rs`'s structural tier stops at length 3 and `census_len4.rs` is
//! text-only, so no shipped gate prices structure above length 3. This probe
//! classifies every length-4 (130,321) and length-5 (2,476,099) census shape
//! with the same `classify_with` the census ratchet uses, and writes one line
//! per shape -- `<CLEAN|CORRUPT|INEXPR>\t<shape debug>` -- to
//! `$KASANE_STRUCT_OUT/len{4,5}.tsv`, in the fixed enumeration order
//! `census_support::alphabet()` gives (least-significant digit first, same
//! as the brief's own `zz_len5_debug.rs`). Run once per revision (see the
//! sibling `structural-len4-5-sweep.sh`) and diff line-for-line -- the
//! enumeration order is identical across revisions because `census_support`,
//! `kasane-ir`, and `kasane-writer`'s public surface are byte-identical
//! across all three revisions this item compares; only `choose_mark`'s body
//! in `crates/kasane-writer/src/markdown.rs` differs between them.
//!
//! Not a test: never run in CI, never *committed* under `crates/`. Kept only
//! as evidence under `docs/superpowers/evidence/`. The sibling sweep script
//! does copy this file into `crates/kasane-writer/tests/` and compile it there
//! -- inside a disposable `git worktree` it removes afterwards -- which is the
//! only way it ever builds. This line said "never compiled as part of
//! `crates/`" until the whole-branch review on 2026-08-23, which was false.
mod census_support;
use census_support::{alphabet, classify_with, Structure};
use kasane_ir::Inline;
use kasane_writer::Ledger;
use std::io::Write;

fn tag(s: Structure) -> &'static str {
    match s {
        Structure::Clean => "CLEAN",
        Structure::Corrupt => "CORRUPT",
        Structure::Inexpressible => "INEXPR",
    }
}

fn run_len(len: usize, out_path: &str) {
    let a = alphabet();
    let n = a.len();
    let total = n.pow(len as u32);
    let mut f = std::io::BufWriter::new(std::fs::File::create(out_path).unwrap());
    let (mut clean, mut corrupt, mut inexpr) = (0usize, 0usize, 0usize);
    for m in 0..total {
        let mut x = m;
        let mut seq: Vec<Inline> = Vec::with_capacity(len);
        for _ in 0..len {
            seq.push(a[x % n].clone());
            x /= n;
        }
        let c = classify_with(&seq, Ledger::LICENSED);
        match c {
            Structure::Clean => clean += 1,
            Structure::Corrupt => corrupt += 1,
            Structure::Inexpressible => inexpr += 1,
        }
        writeln!(f, "{}\t{:?}", tag(c), seq).unwrap();
    }
    f.flush().unwrap();
    println!("length-{len}: clean={clean} corrupt={corrupt} inexpr={inexpr} total={total}");
}

#[test]
fn structural_len4_and_len5() {
    let out_dir = std::env::var("KASANE_STRUCT_OUT")
        .expect("set KASANE_STRUCT_OUT to a writable directory before running this probe");
    run_len(4, &format!("{out_dir}/len4.tsv"));
    run_len(5, &format!("{out_dir}/len5.tsv"));
}
