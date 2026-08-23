//! Probe 2 follow-up: dump the residual permanent set rather than count it.
//! Requires `probe-2-reorder.patch`. This is what produced spec §2.5.1 — and
//! what falsified the first draft's claim that the residual was a depth limit.
mod census_support;
use census_support::{classify_with, shapes, text_is_clean, Structure};
use kasane_writer::{probe, Ledger};
#[test]
fn resid() {
    let (mut perm, mut queue) = (Vec::new(), Vec::new());
    for s in shapes() {
        probe::reset(3);
        if !text_is_clean(&s, Ledger::LICENSED) { continue; }
        probe::reset(3);
        match classify_with(&s, Ledger::LICENSED) {
            Structure::Inexpressible => perm.push(format!("{s:?}")),
            Structure::Corrupt => queue.push(format!("{s:?}")),
            Structure::Clean => {}
        }
    }
    println!("residual permanent = {}, queue = {}", perm.len(), queue.len());
    std::fs::write("p2-residual-permanent.txt", perm.join("\n")).unwrap();
}
