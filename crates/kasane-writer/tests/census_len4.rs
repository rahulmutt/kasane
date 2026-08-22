//! The text tier at length 4, asserting zero.
//!
//! The length 1-3 census (`census.rs`) carries three allowlist files because
//! its answer is not zero. This one carries none, and cannot rot into stale
//! excuses, because it has no file to rot into.
//!
//! **Why length 4 specifically.** `2026-08-18-abutment-ledger-design.md` §2b.5
//! is that branch's most transferable finding: its structural counter read 0 in
//! every row of every table while text losses ran into the thousands, because
//! the census stops at length 3 and the losses lived at length >= 4. A guard at
//! 4 is the smallest one that would have spoken. Lengths 5 and 6 were swept
//! too (`2026-08-21-declined-run-rescan-design.md` §2.2, also zero) and are not
//! shipped: they cost minutes, not seconds.
//!
//! **What this does not cover.** The alphabet is the census's own 19 elements.
//! Zero here says nothing about text outside it, and the property tier
//! (`properties.rs`) remains the only guard there. That scope statement is
//! load-bearing: `census-inexpressible.txt` spent months asserting "Markdown
//! cannot express" when it meant "this alphabet cannot express", and 88% of it
//! was wrong.

mod census_support;

use census_support::{alphabet, text_is_clean};
use kasane_ir::Inline;
use kasane_writer::Ledger;

/// Every sequence of length 4 over the census alphabet round-trips its text.
///
/// Shapes are built by odometer rather than by `shapes()`, which is fixed at
/// lengths 1-3. The corrupt list is capped when reported so a regression prints
/// a readable failure instead of 130k lines.
#[test]
fn no_shape_of_length_four_loses_text() {
    let a = alphabet();
    let n = a.len();
    let mut corrupt: Vec<String> = Vec::new();
    let mut idx = [0usize; 4];
    loop {
        let seq: Vec<Inline> = idx.iter().map(|&k| a[k].clone()).collect();
        if !text_is_clean(&seq, Ledger::LICENSED) {
            corrupt.push(format!("{seq:?}"));
        }
        let mut k = 4;
        loop {
            if k == 0 {
                assert!(
                    corrupt.is_empty(),
                    "{} shape(s) of length 4 lose text; first 20:\n  {}",
                    corrupt.len(),
                    corrupt
                        .iter()
                        .take(20)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n  ")
                );
                return;
            }
            k -= 1;
            idx[k] += 1;
            if idx[k] < n {
                break;
            }
            idx[k] = 0;
        }
    }
}
