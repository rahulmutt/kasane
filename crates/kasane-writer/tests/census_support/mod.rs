//! The census oracle, shared by every census tier.
//!
//! Two test binaries render the same shapes through the same parser and ask
//! the same structural question: `census.rs` (lengths 1-3, the ratchet) and
//! `census_probe.rs` (design spec §2's re-measurement). A copy of
//! `classify_with` in either would drift from the other, which is the same
//! reason `section::canonicalize_inlines` is a `#[doc(hidden)] pub` seam
//! rather than a rule re-spelled in a test.
//!
//! A third tier was designed and **never shipped**: `census_deep.rs`, design
//! spec §5's licensed-spelling tier. It was written, measured, and abandoned
//! with the rest of that design — as specified it fails 2,561 times on
//! unmodified `main`, and its corpus cannot witness the failure it exists to
//! catch. Read spec §5.4 before building anything in its place; this module is
//! deliberately general enough to serve such a tier, and nothing here assumes
//! one exists.
//!
//! Every tier renders through [`render`], which takes an explicit `Ledger`,
//! because the probe needs today's output and the output under one isolated
//! cell in the same process.

// Each tier uses a different subset of this module; Rust warns per test
// binary, not per workspace.
#![allow(dead_code)]

use kasane_ir::{AssetBag, Block, BlockId, Inline, NoteId, RefTarget};
use kasane_writer::Ledger;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::collections::BTreeSet;

pub fn alphabet() -> Vec<Inline> {
    let t = |s: &str| Inline::Text(s.to_string());
    let em = |i: Inline| Inline::Emph(vec![i]);
    let st = |i: Inline| Inline::Strong(vec![i]);
    vec![
        t("a"),
        t("b"),
        t(" "),
        t("*"),
        Inline::Code("x".into()),
        Inline::Code("y".into()),
        Inline::Math("m".into()),
        Inline::Math("a$b".into()),
        em(t("a")),
        st(t("a")),
        em(Inline::Code("x".into())),
        st(Inline::Code("x".into())),
        em(em(t("a"))),
        st(em(t("a"))),
        em(st(t("a"))),
        st(st(t("a"))),
        Inline::Link {
            target: RefTarget::Internal(BlockId(0)),
            inlines: vec![Inline::Code("x".into())],
        },
        Inline::Link {
            target: RefTarget::Internal(BlockId(0)),
            inlines: vec![],
        },
        Inline::FootnoteRef(NoteId(1)),
    ]
}

/// The oracle's options. Shared so the two parser walks cannot drift onto
/// different option sets — `ENABLE_MATH` in one and not the other would move
/// characters between `Event::Text` and `Event::InlineMath` and silently
/// change what each walk counts.
pub fn parser_options() -> Options {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_MATH);
    opts
}

/// The text a real parser recovers from `md`.
pub fn parsed_text(md: &str) -> String {
    let mut out = String::new();
    for ev in Parser::new_ext(md, parser_options()) {
        match ev {
            Event::Text(t) | Event::Code(t) | Event::InlineMath(t) | Event::DisplayMath(t) => {
                out.push_str(&t)
            }
            _ => {}
        }
    }
    out
}

/// Every character a real parser recovers, paired with the stack of emphasis
/// containers enclosing it.
///
/// The third guard (design spec §3) is the `assert!` at the end: an unbalanced
/// event stream means the comparison below it is meaningless, so it fails
/// rather than returning a half-built vector.
pub fn parsed_context(md: &str) -> ContextWalk {
    let mut stack: Vec<Emphasis> = Vec::new();
    let mut out = Vec::new();
    for ev in Parser::new_ext(md, parser_options()) {
        match ev {
            Event::Start(Tag::Emphasis) => stack.push(Emphasis::Em),
            Event::Start(Tag::Strong) => stack.push(Emphasis::St),
            Event::End(TagEnd::Emphasis | TagEnd::Strong) => {
                stack.pop();
            }
            Event::Text(t) | Event::Code(t) | Event::InlineMath(t) | Event::DisplayMath(t) => {
                for c in t.chars() {
                    out.push((c, stack.clone()));
                }
            }
            _ => {}
        }
    }
    assert!(
        stack.is_empty(),
        "unbalanced emphasis events parsing {md:?} — the structural \
         comparison for this shape would be meaningless"
    );
    out
}

/// The slice with leading and trailing whitespace dropped, matching the
/// `.trim()` the text assertion compares under. Without this the two vectors
/// would be off by however much whitespace the writer added or the parser ate.
pub fn trim_whitespace(v: &[(char, Vec<Emphasis>)]) -> &[(char, Vec<Emphasis>)] {
    let start = v
        .iter()
        .position(|(c, _)| !c.is_whitespace())
        .unwrap_or(v.len());
    let end = v
        .iter()
        .rposition(|(c, _)| !c.is_whitespace())
        .map_or(start, |i| i + 1);
    &v[start..end]
}

/// A context walk's characters, each paired with its enclosing emphasis stack.
pub type ContextWalk = Vec<(char, Vec<Emphasis>)>;

/// Render one shape as a paragraph, under a chosen ledger.
pub fn render(seq: &[Inline], ledger: Ledger) -> String {
    kasane_writer::blocks_to_markdown_with_ledger(
        &[Block::Para(seq.to_vec())],
        &AssetBag::default(),
        ledger,
    )
}

/// Whether the text tier passes for this shape under this ledger.
///
/// Separate from [`classify_with`] on purpose: `classify_with` returns `Clean`
/// when the text is already corrupt, because the structural tier is gated on
/// the text tier and the text assertion names those shapes itself. A caller
/// that is not the ratchet must ask both questions, and the probe does. (The
/// never-shipped `census_deep.rs` was the caller this sentence originally
/// named — design spec §5.4.)
pub fn text_is_clean(seq: &[Inline], ledger: Ledger) -> bool {
    let md = render(seq, ledger);
    parsed_text(&md).trim() == kasane_gfm::rendered_text(seq).trim()
}

/// Renders `seq`, gates on the text assertion, and returns both trimmed
/// context walks -- or `None` if the text is already corrupt, in which case
/// structure is not evaluated (design spec §2, "Gate").
///
/// Shared by [`classify_with`] here and `census.rs`'s alignment guard
/// (`the_two_context_walks_align_character_for_character`): the guard is only
/// evidence for what `classify_with` actually compares if both exercise the
/// same render/gate/walk setup. Two independent copies of it could drift
/// apart, and if they did, the guard would stop covering the walks
/// `classify_with` uses.
pub fn context_walks_with(seq: &[Inline], ledger: Ledger) -> Option<(ContextWalk, ContextWalk)> {
    let md = render(seq, ledger);
    let expected = kasane_gfm::rendered_text(seq);
    if parsed_text(&md).trim() != expected.trim() {
        return None;
    }

    let mut ir = Vec::new();
    ir_context(seq, 0, &mut Vec::new(), &mut ir);
    let ir = trim_whitespace(&ir).to_vec();
    let got = parsed_context(&md);
    let got = trim_whitespace(&got).to_vec();
    Some((ir, got))
}

/// Every sequence of length 1-3 over the alphabet.
pub fn shapes() -> Vec<Vec<Inline>> {
    let a = alphabet();
    let mut out: Vec<Vec<Inline>> = a.iter().map(|i| vec![i.clone()]).collect();
    for i in &a {
        for j in &a {
            out.push(vec![i.clone(), j.clone()]);
            for k in &a {
                out.push(vec![i.clone(), j.clone(), k.clone()]);
            }
        }
    }
    out
}

/// The census alphabet's size, and the radix every shape index is written in.
///
/// A shape of length `n` is a base-`ALPHABET_LEN` numeral with `n` digits, most
/// significant first, and that numeral is its index everywhere in this module.
/// `nonclean_bitset` keys on it and `is_novel` does deletion arithmetic with
/// it, both of which would be wrong rather than merely slow if this drifted
/// from `alphabet().len()`. `alphabet_len_matches_the_radix` pins them
/// together.
pub const ALPHABET_LEN: usize = 19;

/// `ALPHABET_LEN.pow(k)`, as a `usize`.
///
/// Written as a fold rather than `pow` so an overflow at length 7 and up
/// panics in debug on the multiply rather than wrapping silently: 19^7 fits a
/// `usize`, but nothing here bounds what a future caller passes.
pub fn pow19(k: usize) -> usize {
    (0..k).fold(1usize, |a, _| a * ALPHABET_LEN)
}

/// Every sequence of `len` elements over the census alphabet, in ascending
/// base-`ALPHABET_LEN` order, handed to `f` one at a time.
///
/// Streamed rather than materialized: a `Vec` of 19^5 shapes held at once is a
/// cost the odometer does not pay, and at 19^6 it is not payable at all.
///
/// `f` receives the digit slice as well as the shape, because the deep tiers
/// need the shape's index to do `is_novel`'s deletion arithmetic and
/// recomputing it from the shape would mean a reverse lookup per element.
/// Callers that do not need it take `|seq, _|`.
///
/// This is the **only** carry loop in the census. It lived in `census_len4.rs`
/// as `for_each_length_four_shape` until lengths 5 and 6 needed one too, and a
/// second copy there would have been exactly the drift this module exists to
/// prevent -- the same argument `blessing()`'s doc makes about itself.
pub fn for_each_shape(len: usize, mut f: impl FnMut(&[Inline], &[usize])) {
    let a = alphabet();
    assert_eq!(a.len(), ALPHABET_LEN);
    let mut idx = vec![0usize; len];
    loop {
        let seq: Vec<Inline> = idx.iter().map(|&k| a[k].clone()).collect();
        f(&seq, &idx);
        let mut k = len;
        loop {
            if k == 0 {
                return;
            }
            k -= 1;
            idx[k] += 1;
            if idx[k] < ALPHABET_LEN {
                break;
            }
            idx[k] = 0;
        }
    }
}

/// One emphasis container, as it appears on the stack enclosing a character.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Emphasis {
    Em,
    St,
}

/// Every character `rendered_text` contributes, paired with the stack of
/// emphasis containers enclosing it.
///
/// Mirrors `kasane_gfm::rendered_text`'s own walk arm for arm — its
/// `MAX_INLINE_DEPTH` cutoff, and its `[^n]` spelling for a footnote reference
/// — because the whole comparison is meaningless if this walks a different
/// projection from the one the text assertion uses. That mirroring is not
/// asserted in prose: `the_context_walk_reproduces_rendered_text_for_every_short_sequence`
/// re-derives `rendered_text` from this walk's own output every run.
///
/// `Link` pushes nothing. `flatten_into` (`markdown.rs:237-238`) splices every
/// non-`External` target away before the emit loop ever sees it, so a
/// transparent link is not a structural level in the output and must not be
/// one here (design spec §2).
pub fn ir_context(
    inlines: &[Inline],
    depth: usize,
    stack: &mut Vec<Emphasis>,
    out: &mut Vec<(char, Vec<Emphasis>)>,
) {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return;
    }
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => {
                for c in t.chars() {
                    out.push((c, stack.clone()));
                }
            }
            Inline::Emph(x) => {
                stack.push(Emphasis::Em);
                ir_context(x, depth + 1, stack, out);
                stack.pop();
            }
            Inline::Strong(x) => {
                stack.push(Emphasis::St);
                ir_context(x, depth + 1, stack, out);
                stack.pop();
            }
            Inline::Link { inlines, .. } => ir_context(inlines, depth + 1, stack, out),
            Inline::FootnoteRef(n) => {
                for c in format!("[^{}]", n.0).chars() {
                    out.push((c, stack.clone()));
                }
            }
        }
    }
}

/// The characters of a context walk, in order.
pub fn context_text(v: &[(char, Vec<Emphasis>)]) -> String {
    v.iter().map(|(c, _)| *c).collect()
}

/// How one shape's structure survived rendering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Structure {
    /// Structure preserved — or text already corrupt, in which case structure
    /// is not evaluated (design spec §2, "Gate").
    Clean,
    /// A real, fixable loss. Belongs in the queue.
    Corrupt,
    /// This writer does not express this shape at any level.
    ///
    /// Two reasons, and the split is measured, not assumed — see
    /// `census-inexpressible.txt`'s header. For almost all of them CommonMark's
    /// flanking rule stops either `*` or `_` opening or closing against the
    /// letter text beside the nested container, and only an HTML tag spells
    /// those. Those are **permanent**: no writer change reaches them.
    ///
    /// For five of them it is **not** permanent, and this line said it was
    /// until 2026-08-23. The writer *declines* a legal `_` there, because the
    /// child it would save would then fuse into a sibling of another delimiter
    /// class and come back wearing it; erasing a level is the lesser loss. That
    /// is a deliberate trade against a defect in the fuse, not a limit of
    /// Markdown or of this alphabet, and fixing the fuse at its source takes
    /// those five out of this file
    /// (`2026-08-23-delimiter-choice-ordering-design.md` §9). `permanence_ceiling`'s
    /// doc explains why the difference matters: permanence is the one claim in
    /// this census that nothing downstream re-examines.
    ///
    /// This said "Markdown cannot express this shape" until 2026-08-23, which
    /// was false in a way that cost an item its estimate: `_*x*_` is
    /// `<em><em>x</em></em>`. The 2026-08-17 correction reached the `.txt`
    /// headers and `AGENTS.md` and missed this line.
    Inexpressible,
}

/// Whether `seq` holds a container whose **sole** child is a container of the
/// same class — `Emph[Emph[…]]` or `Strong[Strong[…]]`.
///
/// Condition 1 of the inexpressible split (design spec §4), and the one that
/// does the work. `<em><em>x</em></em>` has no CommonMark spelling: `**x**` is
/// strong, not nested emphasis. Only *direct* nesting is inexpressible —
/// `Emph[a, Emph[b], c]` round-trips correctly today, so filing it as permanent
/// on the strength of condition 2 alone would bury a real regression if it ever
/// broke.
///
/// Scoped to the whole shape, not to the mismatching position: this asks
/// whether `seq` contains direct same-class nesting *anywhere*, not whether
/// the position condition 2 is explaining is inside it. See design spec §8's
/// residual risks for what that costs once the alphabet stops being
/// single-child-only.
fn nests_same_class_directly(seq: &[Inline]) -> bool {
    seq.iter().any(|i| match i {
        Inline::Emph(x) => {
            matches!(x.as_slice(), [Inline::Emph(_)]) || nests_same_class_directly(x)
        }
        Inline::Strong(x) => {
            matches!(x.as_slice(), [Inline::Strong(_)]) || nests_same_class_directly(x)
        }
        Inline::Link { inlines, .. } => nests_same_class_directly(inlines),
        _ => false,
    })
}

/// Whether `seq` holds a `Strong` whose **sole** child is an `Emph`.
///
/// The second disjunct of condition 1 (design spec §4). Directional on
/// purpose: `***x***` always resolves em-outermost, so
/// `<strong><em>x</em></strong>` has no `*`-only spelling, while
/// `<em><strong>x</strong></em>` has one and the writer now prints it.
/// Matching both orders here would let a regression of the fixed family
/// launder itself into the permanent file, which is the one failure this
/// split must not have.
///
/// Scoped to the whole shape for the same reason its sibling is, and carrying
/// the same residual risk — see `2026-08-16-structural-census-design.md` §8,
/// and §7 of this item's spec for what a second predicate costs the
/// per-position conversion.
fn nests_strong_over_emph_directly(seq: &[Inline]) -> bool {
    seq.iter().any(|i| match i {
        Inline::Strong(x) => {
            matches!(x.as_slice(), [Inline::Emph(_)]) || nests_strong_over_emph_directly(x)
        }
        Inline::Emph(x) => nests_strong_over_emph_directly(x),
        Inline::Link { inlines, .. } => nests_strong_over_emph_directly(inlines),
        _ => false,
    })
}

/// Whether every difference between the two walks disappears under the two
/// erasures `*` alone forces. Condition 2 of the split (design spec §4).
///
/// Two rules, not one: adjacent identical classes collapse
/// (`<em><em>x</em></em>` has no spelling), and an `Em` directly inside a `St`
/// is dropped (`<strong><em>x</em></strong>` has none either). A stack can need
/// both — `[St, Em, Em]` must reach `[St]`, and a pass applying only one of the
/// two rules, or applying them as two separate sweeps in the wrong order, would
/// stop at `[St, Em]` and file a genuinely unspellable shape corrupt.
///
/// Both rules run in one scan, each element tested against the last *kept*
/// element rather than its original predecessor, and that is what lets a single
/// pass reach the fixpoint: `[St, Em, Em]` drops both `Em`s against the same
/// kept `St` and yields `[St]` directly, never materialising the `[St, Em]` a
/// two-sweep version would. A pass's output is a fixpoint by construction —
/// nothing is pushed after an element it equals, or after a `St` it would be
/// dropped against — so the loop confirms and exits. It is kept as a cheap
/// guard rather than a live iteration: add a third rule or a third class and
/// one-pass sufficiency stops being obvious, while the loop is already correct.
///
/// A **drop**, not a reorder. The writer leaves `Strong[Emph[x]]` spliced, so
/// it prints `**x**` and a parser recovers `[St]` against an IR of `[St, Em]`
/// — the level is deleted, not swapped. Nothing prints `***x***` for a
/// `Strong`-outer shape, so a reorder normalization would never fire.
///
/// The drop's direction is half the laundering guard. If the writer regresses
/// and `Emph[Strong[x]]` loses its `<strong>`, the stacks are `[Em, St]`
/// against `[Em]`; this drops an `Em` that follows a `St`, not a `St` that
/// follows an `Em`, so the walks stay unequal and the shape lands in the
/// queue where the ratchet fails the build.
fn differs_only_by_erasure(ir: &[(char, Vec<Emphasis>)], got: &[(char, Vec<Emphasis>)]) -> bool {
    fn normalize(v: &[Emphasis]) -> Vec<Emphasis> {
        let mut cur = v.to_vec();
        loop {
            let mut out: Vec<Emphasis> = Vec::new();
            for &e in &cur {
                match out.last() {
                    Some(&last) if last == e => {}
                    Some(&Emphasis::St) if e == Emphasis::Em => {}
                    _ => out.push(e),
                }
            }
            if out == cur {
                return out;
            }
            cur = out;
        }
    }
    ir.iter()
        .zip(got)
        .all(|(x, y)| normalize(&x.1) == normalize(&y.1))
}

/// The relation, for one shape (design spec §2).
pub fn classify_with(seq: &[Inline], ledger: Ledger) -> Structure {
    let Some((ir, got)) = context_walks_with(seq, ledger) else {
        return Structure::Clean;
    };

    if ir.iter().zip(&got).all(|(x, y)| x.1 == y.1) {
        return Structure::Clean;
    }
    if (nests_same_class_directly(seq) || nests_strong_over_emph_directly(seq))
        && differs_only_by_erasure(&ir, &got)
    {
        return Structure::Inexpressible;
    }
    Structure::Corrupt
}

/// Whether this run is regenerating the ratchet files rather than checking
/// them.
///
/// Spelled once and shared by every tier, because two readers disagreeing
/// about what a bless is would let one of them write while the other asserts
/// against the file it just changed. It lived in `census.rs` until the
/// length-4 structural tier needed it too; a second copy there would have been
/// that same hazard, one file further away.
pub fn blessing() -> bool {
    std::env::var_os("KASANE_CENSUS_BLESS").is_some()
}

/// The most entries a permanent file may hold.
///
/// A **ceiling**, not a count: a permanent file shrinking is always an
/// improvement, so this is only ever compared as an upper bound and a shrink
/// needs no edit. A bless *lowers* it to match — safe, since lowering only
/// tightens the gate — and never raises it. Raising it is a hand edit, and
/// that asymmetry is the entire point; see `PERMANENT_CEILING`'s doc in
/// `census.rs` for what went wrong when the claim was made at scale without
/// one.
///
/// Takes its path rather than closing over one, because there is a permanent
/// file per length and a helper that could only read the length-3 one would be
/// copied rather than reused.
pub fn permanence_ceiling(path: &str) -> usize {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path} must exist and be readable: {e}"));
    raw.trim()
        .parse()
        .unwrap_or_else(|e| panic!("{path} must hold a single integer: {e}"))
}

/// Bless or check one ratchet file, two-directionally: a shape that is in
/// `found` but not the file fails, and a shape in the file but not `found`
/// fails too, so the file can neither grow silently nor rot into stale
/// excuses.
///
/// `#`-prefixed lines are comments, which is how a permanent file carries its
/// generated header.
///
/// Note that the first assertion **short-circuits the second**: a run that
/// finds newly-corrupt shapes panics before it can report shapes that are no
/// longer corrupt. A caller that needs both directions of one file in one
/// look must bless and diff, not read a single failure.
pub fn ratchet(path: &str, found: &BTreeSet<String>, noun: &str, header: Option<&str>) {
    if blessing() {
        let mut body = header.unwrap_or("").to_string();
        body.extend(found.iter().map(|l| format!("{l}\n")));
        std::fs::write(path, body).expect("writing the allowlist");
        return;
    }

    let known: BTreeSet<String> = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("{path} must exist -- bless it with KASANE_CENSUS_BLESS=1"))
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();

    let new: Vec<&String> = found.difference(&known).collect();
    let gone: Vec<&String> = known.difference(found).collect();

    assert!(
        new.is_empty(),
        "{} shape(s) newly {noun} -- bless them into {path} \
         (KASANE_CENSUS_BLESS=1 does it for you):\n{}",
        new.len(),
        new.iter()
            .take(10)
            .map(|s| format!("  {s}\n"))
            .collect::<String>()
    );
    assert!(
        gone.is_empty(),
        "{} listed shape(s) are no longer {noun} -- delete them from {path} \
         (KASANE_CENSUS_BLESS=1 does it for you):\n{}",
        gone.len(),
        gone.iter()
            .take(10)
            .map(|s| format!("  {s}\n"))
            .collect::<String>()
    );
}

/// Non-clean shapes of one length, as a bitset keyed by base-`ALPHABET_LEN`
/// index.
///
/// A bitset rather than a `BTreeSet<String>` of `format!("{seq:?}")`, which is
/// what the ratchet files use and what the design probes used first. At length
/// 5 that set is ~100 MB and materially slower to build and query, and the
/// length-6 novelty check needs the length-5 set **resident** while it walks
/// 47 million shapes. 19^5 bits is 310 KB.
///
/// This is an in-memory index, never a committed file. Nothing blesses it and
/// nothing reads it across revisions -- design spec §2.2 is why lengths 5 and
/// 6 commit no per-shape files at all.
pub struct NonClean {
    bits: Vec<u64>,
    len: usize,
}

impl NonClean {
    /// An empty bitset with room for every shape of `len`.
    pub fn new(len: usize) -> Self {
        NonClean {
            bits: vec![0u64; pow19(len) / 64 + 1],
            len,
        }
    }

    /// The shape length this bitset indexes.
    pub fn shape_len(&self) -> usize {
        self.len
    }

    pub fn set(&mut self, i: usize) {
        self.bits[i / 64] |= 1 << (i % 64);
    }

    pub fn get(&self, i: usize) -> bool {
        self.bits[i / 64] >> (i % 64) & 1 == 1
    }

    /// How many bits are set.
    pub fn count(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }
}

/// Classify every shape of `len` and record the non-clean ones.
///
/// "Non-clean" is `Structure::Corrupt` **or** `Structure::Inexpressible`, i.e.
/// the union the ratchet gates -- not the queue alone. `is_novel` asks whether
/// a shape's family is already visible to a shipped tier, and a shape filed as
/// permanent is just as visible as one in the queue.
pub fn nonclean_bitset(len: usize, ledger: Ledger) -> NonClean {
    let mut bits = NonClean::new(len);
    let mut value = 0usize;
    for_each_shape(len, |seq, _| {
        if classify_with(seq, ledger) != Structure::Clean {
            bits.set(value);
        }
        value += 1;
    });
    bits
}

/// Whether a shape is **novel**: non-clean for a reason no shorter shape shows.
///
/// `idx` is the shape's base-`ALPHABET_LEN` digits; `shorter` is the non-clean
/// bitset one length down. The shape is novel when **all** of its
/// single-deletion sub-shapes are clean. The caller has already established
/// that the shape itself is non-clean -- this function does not re-classify it.
///
/// **Deletion, not contiguous substring**, and that is measured rather than
/// chosen: of the 1,204,312 non-clean length-5 shapes, all 1,204,312 have a
/// non-clean single-deletion sub-shape but only 1,204,044 have a non-clean
/// contiguous one. A substring relation reports 268 false novelties on a clean
/// tree. `an_interior_deletion_is_enough_to_make_a_shape_derivative` is what
/// stops someone "simplifying" this into one.
///
/// Novelty is **zero at every length measured** -- 4 against <=3, 5 against
/// <=4, 6 against <=5 -- which is why lengths 5 and 6 assert zero and commit no
/// per-shape files (design spec §2.2). That zero is a property of this writer
/// today, not a theorem.
pub fn is_novel(idx: &[usize], shorter: &NonClean) -> bool {
    debug_assert_eq!(idx.len(), shorter.shape_len() + 1);
    for i in 0..idx.len() {
        let mut sub = 0usize;
        for (p, &d) in idx.iter().enumerate() {
            if p != i {
                sub = sub * ALPHABET_LEN + d;
            }
        }
        if shorter.get(sub) {
            return false;
        }
    }
    true
}

/// The three census counts at one length.
///
/// `union` is `queue + permanent` and is stored rather than derived, because
/// it is the number the ratchet **gates** and a reader of the committed file
/// should not have to add two numbers to find the gated one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Counts {
    pub queue: usize,
    pub permanent: usize,
    pub union: usize,
}

/// Everything one walk over a length yields.
///
/// The two zero-assertions carry their offending shapes rather than only
/// counts, because a failure of either is a design event -- see
/// `census_len5.rs`'s failure text -- and the first thing anyone will want is
/// an example.
pub struct DeepScan {
    pub text_corrupt: Vec<String>,
    pub counts: Counts,
    pub novel: Vec<String>,
}

/// How many offending shapes a failing assertion reports.
const DEEP_SCAN_SAMPLE: usize = 10;

/// One walk over every shape of `len`: both tiers and the novelty check.
///
/// `shorter` must be the non-clean bitset for `len - 1`.
///
/// **This renders each shape twice** -- `text_is_clean` and `classify_with`
/// each call `render` -- and the measured costs in design spec §2 already
/// include that. Folding them into one render means restructuring
/// `classify_with`, which is shared with the length-1-3 and length-4 tiers and
/// is not worth the risk to save minutes on a weekly job. §10 records that the
/// figure is reported rather than hidden.
pub fn deep_scan(len: usize, shorter: &NonClean, ledger: Ledger) -> DeepScan {
    let mut text_corrupt: Vec<String> = Vec::new();
    let mut novel: Vec<String> = Vec::new();
    let (mut queue, mut permanent) = (0usize, 0usize);
    let mut n_text = 0usize;
    let mut n_novel = 0usize;

    for_each_shape(len, |seq, idx| {
        if !text_is_clean(seq, ledger) {
            n_text += 1;
            if text_corrupt.len() < DEEP_SCAN_SAMPLE {
                text_corrupt.push(format!("{seq:?}"));
            }
        }
        match classify_with(seq, ledger) {
            Structure::Clean => return,
            Structure::Corrupt => queue += 1,
            Structure::Inexpressible => permanent += 1,
        }
        if is_novel(idx, shorter) {
            n_novel += 1;
            if novel.len() < DEEP_SCAN_SAMPLE {
                novel.push(format!("{seq:?}"));
            }
        }
    });

    // The samples are capped; the counts are not. A caller asserting zero needs
    // the true total in its message, so the capped vectors are padded with a
    // tail marker rather than silently under-reporting.
    if n_text > text_corrupt.len() {
        text_corrupt.push(format!("... and {} more", n_text - text_corrupt.len()));
    }
    if n_novel > novel.len() {
        novel.push(format!("... and {} more", n_novel - novel.len()));
    }

    DeepScan {
        text_corrupt,
        counts: Counts {
            queue,
            permanent,
            union: queue + permanent,
        },
        novel,
    }
}

/// Bless or check one counts file.
///
/// The counts analogue of [`ratchet`], and deliberately **not** a ratchet: it
/// asserts equality in both directions, exactly as `ratchet` does for a shape
/// file. Whether a count may only shrink is `mise run census-ratchet`'s
/// question, asked across revisions; this one asks only whether the committed
/// file still describes the writer. Design spec §5 is why the two must not be
/// merged: the ratchet takes this file's accuracy on trust, which is only
/// earned once this assertion has run.
pub fn counts_ratchet(path: &str, found: Counts, header: &str) {
    let body = format!(
        "{header}queue {}\npermanent {}\nunion {}\n",
        found.queue, found.permanent, found.union
    );
    if blessing() {
        std::fs::write(path, body).expect("writing the counts file");
        return;
    }

    let known = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("{path} must exist -- bless it with KASANE_CENSUS_BLESS=1"));
    let strip = |s: &str| -> String {
        s.lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| format!("{l}\n"))
            .collect()
    };
    assert_eq!(
        strip(&known),
        strip(&body),
        "{path} no longer describes this writer.\n\
         \n\
         Committed above, measured below. Every one of these numbers moving is \
         normal on a change that improves the writer -- re-bless with \
         `mise run census-bless`. What is NOT normal is `union` going UP: that \
         is a shape becoming corrupt that was not, and \
         `mise run census-ratchet` will refuse it against main."
    );
}
