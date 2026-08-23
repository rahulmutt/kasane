mod census_support;
use census_support::{parsed_text, alphabet};
use kasane_ir::{AssetBag, Block, Inline};
use kasane_writer::{blocks_to_markdown_with_ledger, probe, Ledger};
fn r(seq: &[Inline]) -> String {
    blocks_to_markdown_with_ledger(&[Block::Para(seq.to_vec())], &AssetBag::default(), Ledger::LICENSED)
}
#[test]
fn hookcheck() {
    let mut a = alphabet(); a.push(Inline::Text("_".into()));
    let n = a.len();
    for (mode, label) in [(0u64, "ship *"), (2, "flank-guarded _")] {
        let mut txt = 0; let mut tot = 0;
        for i in 0..n { for j in 0..n { for k in 0..n {
            let s = vec![a[i].clone(), a[j].clone(), a[k].clone()];
            // only shapes containing a literal underscore
            if !format!("{s:?}").contains("Text(\"_\")") { continue; }
            tot += 1;
            probe::reset(mode, 0);
            let md = r(&s);
            if parsed_text(&md).trim() != kasane_gfm::rendered_text(&s).trim() { txt += 1; }
        }}}
        println!("{label:<18} shapes-with-literal-_ = {tot}, text-corrupt = {txt}");
    }
    panic!("show");
}
