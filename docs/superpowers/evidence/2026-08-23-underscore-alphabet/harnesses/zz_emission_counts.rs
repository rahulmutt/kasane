//! Probe 1 spot check: containers present vs delimiters actually chosen.
//! Requires `probe-hook.patch` (mode 1 = forced assignment, with LOG).
//! This is the instrument behind spec §2.3 — it compares what the pipeline did
//! against what the input contained, rather than against an external standard.
use kasane_ir::{AssetBag, Block, Inline};
use kasane_writer::{blocks_to_markdown_with_ledger, probe, Ledger};
fn r(s: &[Inline]) -> String {
    blocks_to_markdown_with_ledger(&[Block::Para(s.to_vec())], &AssetBag::default(), Ledger::LICENSED)
}
fn t(s: &str) -> Inline { Inline::Text(s.into()) }
fn em(v: Vec<Inline>) -> Inline { Inline::Emph(v) }
fn st(v: Vec<Inline>) -> Inline { Inline::Strong(v) }
fn n(i: &Inline) -> usize {
    match i { Inline::Emph(x) | Inline::Strong(x) => 1 + x.iter().map(n).sum::<usize>(), _ => 0 }
}
#[test]
fn emission_counts() {
    let cases: Vec<(&str, Vec<Inline>)> = vec![
        ("Em[Em[a]]", vec![em(vec![em(vec![t("a")])])]),
        ("St[St[a]]", vec![st(vec![st(vec![t("a")])])]),
        ("St[Em[a]]", vec![st(vec![em(vec![t("a")])])]),
        ("Em[St[a]]", vec![em(vec![st(vec![t("a")])])]),
        ("Em[Em[a]] in text", vec![t("z"), em(vec![em(vec![t("a")])]), t("z")]),
        ("Em[a],Em[b]", vec![em(vec![t("a")]), em(vec![t("b")])]),
        ("Em[a b]", vec![em(vec![t("a"), st(vec![t("b")])])]),
    ];
    for (name, s) in cases {
        let containers: usize = s.iter().map(n).sum();
        probe::reset(0, 0);
        let md0 = r(&s);
        let emissions = probe::log().len();
        for b in 0u64..4 { probe::reset(1, b); println!("      bits={b} -> {:?}", r(&s)); }
        println!("{name:<20} containers={containers} emissions={emissions} md0={md0:?}");
    }
    panic!("show");
}
