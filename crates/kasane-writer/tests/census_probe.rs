//! Design spec §2's re-measurement, committed rather than archived.
//!
//! The probe this replaces was a throwaway script, and its finer sub-split was
//! cut from `2026-08-16-cross-class-edge-splice-design.md` §6 for being
//! reproducible by nobody but its author. This one lives in the repo, prices
//! every cell separately, and is re-runnable by anyone:
//!
//! ```text
//! cargo test -p kasane-writer --test census_probe -- --ignored --nocapture
//! ```

mod census_support;

use census_support::{classify_with, shapes, text_is_clean, Structure};
use kasane_ir::Inline;
use kasane_writer::Ledger;
use std::collections::BTreeSet;

const STRUCTURE_QUEUE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-known-structure-corrupt.txt"
);
const PERMANENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/census-inexpressible.txt"
);

/// The shape keys listed in one census file.
fn keys(path: &str) -> BTreeSet<String> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("{path} must exist"))
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Whether `seq` is clean under `ledger`, on both the text and structural
/// tiers.
fn is_clean(seq: &[Inline], ledger: Ledger) -> bool {
    text_is_clean(seq, ledger) && classify_with(seq, ledger) == Structure::Clean
}

/// One cell's (or the combined ledger's) recovery, priced against both census
/// files.
///
/// Factored out of what the brief spells twice — once per cell, once for the
/// `ALL_CELLS` union — because the two loops are otherwise verbatim copies of
/// each other. The `was_clean` baseline is hoisted by the caller rather than
/// recomputed here, since it does not depend on `ledger`.
///
/// Returns `(queue_newly_clean, permanent_newly_clean, newly_corrupt)`.
fn measure(
    ledger: Ledger,
    all: &[Vec<Inline>],
    baseline: &[bool],
    queued: &BTreeSet<String>,
    permanent: &BTreeSet<String>,
) -> (usize, usize, usize) {
    let (mut q_clean, mut p_clean, mut broke) = (0usize, 0usize, 0usize);
    for (seq, &was_clean) in all.iter().zip(baseline) {
        let key = format!("{seq:?}");
        let clean = is_clean(seq, ledger);
        if clean && !was_clean {
            if queued.contains(&key) {
                q_clean += 1;
            } else if permanent.contains(&key) {
                p_clean += 1;
            }
        }
        if was_clean && !clean {
            broke += 1;
        }
    }
    (q_clean, p_clean, broke)
}

#[test]
#[ignore = "measurement, not an assertion: run with --ignored --nocapture"]
fn price_every_cell_against_both_census_files() {
    let queued = keys(STRUCTURE_QUEUE);
    let permanent = keys(PERMANENT);
    let all = shapes();

    // A sanity guard, not part of the measurement: this section exists
    // because a previous probe's numbers turned out to be untrustworthy, so
    // a silent key-format drift between `shapes()`'s `Debug` keys and the
    // census files' own keys must fail loudly rather than quietly produce a
    // plausible-looking but empty measurement.
    assert!(
        !queued.is_empty(),
        "{STRUCTURE_QUEUE} must list at least one key"
    );
    assert!(
        !permanent.is_empty(),
        "{PERMANENT} must list at least one key"
    );
    assert!(
        all.iter()
            .any(|seq| queued.contains(&format!("{seq:?}"))
                || permanent.contains(&format!("{seq:?}"))),
        "no key from `shapes()` matched either census file — the key format has \
         drifted and every count below would be meaningless"
    );

    // The baseline does not depend on the cell under test, so it is computed
    // once rather than once per cell (my ruling: this hoist changes no
    // printed number, since every per-cell and combined comparison below
    // still reads `Ledger::CONSERVATIVE`'s clean/not-clean verdict).
    //
    // `CONSERVATIVE` is pre-`0ac2c48` output, one cell below what `main`
    // ships as `LICENSED` (`markdown.rs`'s `Ledger` doc comment) -- so this
    // baseline is what the 292/97/389 table below is priced against, and it
    // is *not* "today's shipped output". A second baseline, against
    // `LICENSED` itself, is computed further down for the row that actually
    // answers "does any cell corrupt a shape that ships clean today".
    let baseline: Vec<bool> = all
        .iter()
        .map(|seq| is_clean(seq, Ledger::CONSERVATIVE))
        .collect();

    println!("cell,file,newly_clean,newly_corrupt");
    let mut union = Ledger::CONSERVATIVE.bits();
    for (name, bit) in Ledger::CELLS {
        let ledger = Ledger::from_bits(*bit);
        let (q_clean, p_clean, broke) = measure(ledger, &all, &baseline, &queued, &permanent);
        println!("{name},queue,{q_clean},{broke}");
        println!("{name},permanent,{p_clean},{broke}");
        union |= bit;
    }

    // The combined figure, which is what design spec §2 records. It is not the
    // sum of the per-cell rows: one shape can be recovered by more than one
    // cell, and a cell can recover a shape only once another cell has stopped
    // a fusion from swallowing it.
    let ledger = Ledger::from_bits(union);
    let (q, p, broke) = measure(ledger, &all, &baseline, &queued, &permanent);
    println!("ALL_CELLS,queue,{q},{broke}");
    println!("ALL_CELLS,permanent,{p},{broke}");
    println!("ALL_CELLS,total_recovered,{},{broke}", q + p);

    // A second baseline, against `Ledger::LICENSED` -- today's shipped
    // output -- rather than `CONSERVATIVE`. The rows above only count a
    // regression among shapes clean under `CONSERVATIVE`; a shape `LICENSED`
    // already recovers (clean under `LICENSED`, not clean under
    // `CONSERVATIVE`) is invisible to that `broke` column entirely, since a
    // regression there leaves both `clean` and `was_clean` false and moves
    // no counter. This row closes that blind spot: its `broke` column is the
    // early warning for "some cell licensed here corrupts a shape that ships
    // clean today", which the `CONSERVATIVE`-only rows above cannot see.
    let licensed_baseline: Vec<bool> = all
        .iter()
        .map(|seq| is_clean(seq, Ledger::LICENSED))
        .collect();
    let (lq, lp, lbroke) = measure(ledger, &all, &licensed_baseline, &queued, &permanent);
    println!(
        "ALL_CELLS_VS_LICENSED,shipped_baseline,{},{lbroke}",
        lq + lp
    );
}
