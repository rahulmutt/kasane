//! THROWAWAY probe 2 sweep.
mod census_support;
use census_support::{alphabet, classify_with, shapes, text_is_clean, Structure};
use kasane_ir::Inline;
use kasane_writer::{probe, Ledger};
use std::collections::BTreeSet;

const SP: &str = "/tmp/claude-1000/-workspace/7732882d-b24b-4108-b3e3-e74ced3579f6/scratchpad";

fn t(s: &str) -> Inline { Inline::Text(s.to_string()) }

#[test]
fn sweep() {
    // ---- A. the census's own population, exactly as the queue files count it ----
    let mut base = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new()); // corrupt, inexpr, textbad
    for s in shapes() {
        probe::reset(0);
        if !text_is_clean(&s, Ledger::LICENSED) { base.2.insert(format!("{s:?}")); continue; }
        match classify_with(&s, Ledger::LICENSED) {
            Structure::Corrupt => { base.0.insert(format!("{s:?}")); }
            Structure::Inexpressible => { base.1.insert(format!("{s:?}")); }
            Structure::Clean => {}
        }
    }
    let mut now = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for s in shapes() {
        probe::reset(3);
        if !text_is_clean(&s, Ledger::LICENSED) { now.2.insert(format!("{s:?}")); continue; }
        probe::reset(3);
        match classify_with(&s, Ledger::LICENSED) {
            Structure::Corrupt => { now.0.insert(format!("{s:?}")); }
            Structure::Inexpressible => { now.1.insert(format!("{s:?}")); }
            Structure::Clean => {}
        }
    }
    println!("== census population (19-elem alphabet, len 1-3, 'alone' context) ==");
    println!("  text-corrupt   ship {:>5}  reorder {:>5}", base.2.len(), now.2.len());
    println!("  struct-corrupt ship {:>5}  reorder {:>5}", base.0.len(), now.0.len());
    println!("  inexpressible  ship {:>5}  reorder {:>5}", base.1.len(), now.1.len());
    let recovered: BTreeSet<_> = base.1.union(&base.0).cloned().collect::<BTreeSet<_>>()
        .difference(&now.1.union(&now.0).cloned().collect::<BTreeSet<_>>()).cloned().collect();
    let broken: BTreeSet<_> = now.1.union(&now.0).cloned().collect::<BTreeSet<_>>()
        .difference(&base.1.union(&base.0).cloned().collect::<BTreeSet<_>>()).cloned().collect();
    let newtext: BTreeSet<_> = now.2.difference(&base.2).cloned().collect();
    println!("  RECOVERED (was wrong, now clean) : {}", recovered.len());
    println!("  BROKEN    (was clean, now wrong) : {}", broken.len());
    println!("  NEW TEXT LOSS                    : {}", newtext.len());
    for x in broken.iter().take(6) { println!("    broken: {x}"); }
    for x in newtext.iter().take(6) { println!("    textloss: {x}"); }
    std::fs::write(format!("{}/p2-recovered.txt", SP), recovered.iter().cloned().collect::<Vec<_>>().join("\n")).unwrap();
    std::fs::write(format!("{}/p2-broken.txt", SP), broken.iter().cloned().collect::<Vec<_>>().join("\n")).unwrap();
    std::fs::write(format!("{}/p2-newtext.txt", SP), newtext.iter().cloned().collect::<Vec<_>>().join("\n")).unwrap();

    // ---- B. regression across the five enclosing contexts ----
    println!("\n== regression by enclosing context ==");
    for (name, pre, post) in [
        ("alone", vec![], vec![]),
        ("letter", vec![t("a")], vec![t("c")]),
        ("punct", vec![t(".")], vec![t(".")]),
        ("space", vec![t(" ")], vec![t(" ")]),
        ("letter/space", vec![t("a")], vec![t(" ")]),
    ] {
        let (mut rec, mut brk, mut txt) = (0usize, 0usize, 0usize);
        for s in shapes() {
            let mut full: Vec<Inline> = pre.clone();
            full.extend(s.iter().cloned());
            full.extend(post.iter().cloned());
            probe::reset(0);
            let b_txt = text_is_clean(&full, Ledger::LICENSED);
            probe::reset(0);
            let b_ok = b_txt && classify_with(&full, Ledger::LICENSED) == Structure::Clean;
            probe::reset(3);
            let n_txt = text_is_clean(&full, Ledger::LICENSED);
            probe::reset(3);
            let n_ok = n_txt && classify_with(&full, Ledger::LICENSED) == Structure::Clean;
            if !b_ok && n_ok { rec += 1; }
            if b_ok && !n_ok { brk += 1; }
            if b_txt && !n_txt { txt += 1; }
        }
        println!("  {name:<12} recovered {rec:>5}   broken {brk:>5}   new text loss {txt:>5}");
    }
}
