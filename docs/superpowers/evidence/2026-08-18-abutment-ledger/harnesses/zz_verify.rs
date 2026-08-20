//! THROWAWAY verification harness (task-4 independent check). Not for commit.

mod census_support;

use census_support::{classify_with, render, shapes, text_is_clean, Structure};
use kasane_ir::Inline;
use kasane_writer::Ledger;

const WHOLE: u32 = 1 << 0;
const EMST_HEAD: u32 = 1 << 1;
const EMST_TAIL: u32 = 1 << 2;
const STEM_HEAD: u32 = 1 << 3;
const STEM_TAIL: u32 = 1 << 4;
const EMST_SEAM: u32 = 1 << 5;
const STEM_SEAM: u32 = 1 << 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    TextCorrupt,
    Clean,
    Queue,
    Perm,
}

fn status(seq: &[Inline], l: Ledger) -> Status {
    if !text_is_clean(seq, l) {
        return Status::TextCorrupt;
    }
    match classify_with(seq, l) {
        Structure::Clean => Status::Clean,
        Structure::Corrupt => Status::Queue,
        Structure::Inexpressible => Status::Perm,
    }
}

fn deep_alphabet() -> Vec<Inline> {
    let t = |s: &str| Inline::Text(s.to_string());
    let em = |i: Inline| Inline::Emph(vec![i]);
    let st = |i: Inline| Inline::Strong(vec![i]);
    vec![
        t("a"),
        t("*"),
        Inline::Code("x".into()),
        em(t("a")),
        st(t("a")),
        em(st(t("a"))),
        st(em(t("a"))),
    ]
}

fn deep_shapes() -> Vec<Vec<Inline>> {
    let a = deep_alphabet();
    let n = a.len();
    let mut out = Vec::new();
    for len in 4..=5u32 {
        for code in 0..n.pow(len) {
            let mut code = code;
            let mut seq = Vec::with_capacity(len as usize);
            for _ in 0..len {
                seq.push(a[code % n].clone());
                code /= n;
            }
            out.push(seq);
        }
    }
    out
}

struct Counts {
    txtreg: usize,
    structreg: usize,
    q_clean: usize,
    p_clean: usize,
    q_p: usize,
    p_q: usize,
    examples: Vec<String>,
}

fn measure(corpus: &[Vec<Inline>], base: &[Status], bits: u32) -> Counts {
    let l = Ledger::from_bits(bits);
    let mut c = Counts {
        txtreg: 0,
        structreg: 0,
        q_clean: 0,
        p_clean: 0,
        q_p: 0,
        p_q: 0,
        examples: Vec::new(),
    };
    for (seq, &b) in corpus.iter().zip(base) {
        let t = status(seq, l);
        if b != Status::TextCorrupt && t == Status::TextCorrupt {
            c.txtreg += 1;
            if c.examples.len() < 6 {
                c.examples
                    .push(format!("TXT {seq:?} -> {:?}", render(seq, l)));
            }
        }
        if b == Status::Clean && t != Status::Clean {
            c.structreg += 1;
        }
        match (b, t) {
            (Status::Queue, Status::Clean) => c.q_clean += 1,
            (Status::Perm, Status::Clean) => c.p_clean += 1,
            (Status::Queue, Status::Perm) => c.q_p += 1,
            (Status::Perm, Status::Queue) => c.p_q += 1,
            _ => {}
        }
    }
    c
}

fn name(bits: u32) -> String {
    let mut v = Vec::new();
    for (n, b) in [
        ("Whole", WHOLE),
        ("EmStHead", EMST_HEAD),
        ("EmStTail", EMST_TAIL),
        ("StEmHead", STEM_HEAD),
        ("StEmTail", STEM_TAIL),
        ("EmStSeam", EMST_SEAM),
        ("StEmSeam", STEM_SEAM),
    ] {
        if bits & b != 0 {
            v.push(n);
        }
    }
    if v.is_empty() {
        "(none)".into()
    } else {
        v.join("+")
    }
}

#[test]
#[ignore]
fn verify_length3() {
    let corpus = shapes();
    let base: Vec<Status> = corpus
        .iter()
        .map(|s| status(s, Ledger::from_bits(WHOLE)))
        .collect();
    println!("length-3 corpus: {} shapes", corpus.len());
    println!(
        "baseline(bit0) statuses: textcorrupt={} clean={} queue={} perm={}",
        base.iter().filter(|s| **s == Status::TextCorrupt).count(),
        base.iter().filter(|s| **s == Status::Clean).count(),
        base.iter().filter(|s| **s == Status::Queue).count(),
        base.iter().filter(|s| **s == Status::Perm).count(),
    );
    println!(
        "{:<44}{:>8}{:>10}{:>10}{:>10}{:>8}{:>8}",
        "ledger (all include Whole)", "txtreg", "structreg", "q->clean", "p->clean", "q->p", "p->q"
    );
    let mut subsets: Vec<u32> = (0u32..16)
        .map(|m| {
            WHOLE
                | if m & 1 != 0 { EMST_HEAD } else { 0 }
                | if m & 2 != 0 { EMST_TAIL } else { 0 }
                | if m & 4 != 0 { STEM_HEAD } else { 0 }
                | if m & 8 != 0 { STEM_TAIL } else { 0 }
        })
        .collect();
    subsets.push(WHOLE | EMST_SEAM);
    subsets.push(WHOLE | STEM_SEAM);
    subsets.push(0x7f);
    for bits in subsets {
        let c = measure(&corpus, &base, bits);
        println!(
            "{:<44}{:>8}{:>10}{:>10}{:>10}{:>8}{:>8}",
            name(bits),
            c.txtreg,
            c.structreg,
            c.q_clean,
            c.p_clean,
            c.q_p,
            c.p_q
        );
        for e in &c.examples {
            println!("      {e}");
        }
    }
}

#[test]
#[ignore]
fn verify_deep() {
    let corpus = deep_shapes();
    let base: Vec<Status> = corpus
        .iter()
        .map(|s| status(s, Ledger::from_bits(WHOLE)))
        .collect();
    println!("deep corpus: {} shapes", corpus.len());
    println!(
        "baseline(bit0) statuses: textcorrupt={} clean={} queue={} perm={}",
        base.iter().filter(|s| **s == Status::TextCorrupt).count(),
        base.iter().filter(|s| **s == Status::Clean).count(),
        base.iter().filter(|s| **s == Status::Queue).count(),
        base.iter().filter(|s| **s == Status::Perm).count(),
    );
    println!(
        "{:<44}{:>8}{:>10}",
        "ledger (all include Whole)", "txtreg", "structreg"
    );
    for bits in [
        WHOLE,
        WHOLE | EMST_HEAD,
        WHOLE | EMST_TAIL,
        WHOLE | EMST_HEAD | EMST_TAIL,
        WHOLE | STEM_HEAD,
        WHOLE | STEM_TAIL,
        WHOLE | STEM_HEAD | STEM_TAIL,
        WHOLE | EMST_HEAD | EMST_TAIL | STEM_HEAD | STEM_TAIL,
    ] {
        let c = measure(&corpus, &base, bits);
        println!("{:<44}{:>8}{:>10}", name(bits), c.txtreg, c.structreg);
        for e in &c.examples {
            println!("      {e}");
        }
    }
}
