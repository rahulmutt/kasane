//! THROWAWAY probe for the `_` alphabet item. Not shipped.
//! Requires the probe hook patch in `markdown.rs`.

mod census_support;

use census_support::{
    alphabet, ir_context, parsed_context, parsed_text, parser_options, trim_whitespace, Emphasis,
};
use kasane_ir::{AssetBag, Block, Inline};
use kasane_writer::{blocks_to_markdown_with_ledger, probe, Ledger};
use std::collections::{BTreeMap, BTreeSet};

fn t(s: &str) -> Inline {
    Inline::Text(s.to_string())
}
fn em(v: Vec<Inline>) -> Inline {
    Inline::Emph(v)
}
fn st(v: Vec<Inline>) -> Inline {
    Inline::Strong(v)
}
fn code(s: &str) -> Inline {
    Inline::Code(s.into())
}

/// Census 19 + the two holes: literal `_` text, and multi-child containers.
fn probe_alphabet() -> Vec<Inline> {
    let mut a = alphabet();
    a.push(t("_"));
    a.push(em(vec![t("a"), code("x")]));
    a.push(st(vec![code("x"), t("a")]));
    a.push(em(vec![st(vec![t("a")]), t("b")]));
    a
}

/// Enclosing contexts. `_`'s flanking rule is positional, so a shape measured
/// alone (the census's only context) sits in the single most permissive one.
fn contexts() -> Vec<(&'static str, Vec<Inline>, Vec<Inline>)> {
    vec![
        ("alone", vec![], vec![]),
        ("letter", vec![t("a")], vec![t("c")]),
        ("punct", vec![t(".")], vec![t(".")]),
        ("space", vec![t(" ")], vec![t(" ")]),
        ("letter/space", vec![t("a")], vec![t(" ")]),
    ]
}

fn render(seq: &[Inline]) -> String {
    blocks_to_markdown_with_ledger(
        &[Block::Para(seq.to_vec())],
        &AssetBag::default(),
        Ledger::LICENSED,
    )
}

/// Both invariants at once: text round-trips AND every character carries the
/// same emphasis stack. `None` = text lost (structure not evaluated).
fn ok(seq: &[Inline]) -> Option<bool> {
    let md = render(seq);
    if parsed_text(&md).trim() != kasane_gfm::rendered_text(seq).trim() {
        return None;
    }
    let mut ir = Vec::new();
    ir_context(seq, 0, &mut Vec::new(), &mut ir);
    let ir = trim_whitespace(&ir).to_vec();
    let got = parsed_context(&md);
    let got = trim_whitespace(&got).to_vec();
    if ir.len() != got.len() {
        return Some(false);
    }
    Some(ir.iter().zip(&got).all(|(x, y)| x.1 == y.1))
}

fn shapes_upto3(a: &[Inline]) -> Vec<Vec<Inline>> {
    let mut out: Vec<Vec<Inline>> = a.iter().map(|i| vec![i.clone()]).collect();
    for i in a {
        for j in a {
            out.push(vec![i.clone(), j.clone()]);
            for k in a {
                out.push(vec![i.clone(), j.clone(), k.clone()]);
            }
        }
    }
    out
}

#[test]
fn probe() {
    let _ = parser_options();
    let a = probe_alphabet();
    let shapes = shapes_upto3(&a);
    println!("alphabet {} elements, {} shapes", a.len(), shapes.len());

    let mut report = String::new();

    for (name, pre, post) in contexts() {
        // Baseline: ship behaviour (all `*`).
        let mut base_clean = BTreeSet::new();
        let mut base_bad = BTreeSet::new();
        for s in &shapes {
            let mut full = pre.clone();
            full.extend(s.iter().cloned());
            full.extend(post.iter().cloned());
            probe::reset(0, 0);
            match ok(&full) {
                Some(true) => base_clean.insert(format!("{s:?}")),
                _ => base_bad.insert(format!("{s:?}")),
            };
        }

        // M1 ceiling: does SOME `*`/`_` assignment fix a baseline-bad shape?
        let mut fixed = BTreeSet::new();
        let mut winning: BTreeMap<String, u64> = BTreeMap::new();
        for s in &shapes {
            let key = format!("{s:?}");
            if !base_bad.contains(&key) {
                continue;
            }
            let mut full = pre.clone();
            full.extend(s.iter().cloned());
            full.extend(post.iter().cloned());
            for bits in 0u64..16 {
                probe::reset(1, bits);
                if ok(&full) == Some(true) {
                    fixed.insert(key.clone());
                    winning.insert(key.clone(), bits);
                    break;
                }
            }
        }

        // M3 regression: the local rule applied to shapes clean today.
        let mut lost = BTreeSet::new();
        let mut rule_fixed = BTreeSet::new();
        for s in &shapes {
            let key = format!("{s:?}");
            let mut full = pre.clone();
            full.extend(s.iter().cloned());
            full.extend(post.iter().cloned());
            probe::reset(2, 0);
            let r = ok(&full) == Some(true);
            if base_clean.contains(&key) && !r {
                lost.insert(key.clone());
            }
            if base_bad.contains(&key) && r {
                rule_fixed.insert(key.clone());
            }
        }

        report.push_str(&format!(
            "\ncontext {name:<12} baseline: {} clean / {} wrong\n  \
             M1 ceiling  : {} of {} wrong shapes have a working assignment\n  \
             M3 rule     : fixes {} | BREAKS {} shapes that are clean today\n",
            base_clean.len(),
            base_bad.len(),
            fixed.len(),
            base_bad.len(),
            rule_fixed.len(),
            lost.len(),
        ));
        for x in lost.iter().take(5) {
            report.push_str(&format!("    lost: {x}\n"));
        }
        std::fs::write(
            format!("/tmp/claude-1000/-workspace/7732882d-b24b-4108-b3e3-e74ced3579f6/scratchpad/lost-{}.txt", name.replace('/', "-")),
            lost.iter().cloned().collect::<Vec<_>>().join("\n"),
        )
        .unwrap();
        std::fs::write(
            format!("/tmp/claude-1000/-workspace/7732882d-b24b-4108-b3e3-e74ced3579f6/scratchpad/fixed-{}.txt", name.replace('/', "-")),
            fixed.iter().cloned().collect::<Vec<_>>().join("\n"),
        )
        .unwrap();
    }

    println!("{report}");
    std::fs::write("/tmp/claude-1000/-workspace/7732882d-b24b-4108-b3e3-e74ced3579f6/scratchpad/report.txt", &report).unwrap();
    let _ = Emphasis::Em;
    panic!("probe output above");
}
