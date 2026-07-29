//! Shared bodies for the fuzz targets in `fuzz/`.
//!
//! **This is a test seam, not public API.** It is `pub` only so the `fuzz/`
//! crate — a separate cargo workspace — can call it, and it lives *inside*
//! this crate so it can reach `pub(crate)` internals such as
//! `math::capture_island` and `mobi::palmdoc::decompress` without widening the
//! real public surface.
//!
//! Every function here has the same shape: `fn(&[u8])`. It takes arbitrary
//! bytes and either returns normally or panics. A panic **is** the finding —
//! these functions never return an error to report one. That uniformity is
//! what lets `tests/fuzz_corpus.rs` dispatch by directory name and keeps every
//! libFuzzer wrapper identical.

use crate::guard::{check_expansion, percent_decode, resolve_rel};
use crate::math::{capture_island, mathml_to_latex, omml_to_latex};
use crate::mobi::palmdoc::decompress;
use crate::xmltext::resolve_general_ref;
use crate::{Adapter, DjvuAdapter, EpubAdapter, MobiAdapter, PdfAdapter, PptxAdapter};
use kasane_ir::{AssetBag, Block, Document, Inline};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::Write as _;
use zip::write::{SimpleFileOptions, ZipWriter};
use zip::CompressionMethod;

pub fn epub(data: &[u8]) {
    adapter(&EpubAdapter, data, "fuzz.epub");
}

pub fn pptx(data: &[u8]) {
    adapter(&PptxAdapter, data, "fuzz.pptx");
}

/// Covers MOBI and AZW3/KF8 alike — they are one adapter.
pub fn mobi(data: &[u8]) {
    adapter(&MobiAdapter, data, "fuzz.mobi");
}

pub fn pdf(data: &[u8]) {
    adapter(&PdfAdapter, data, "fuzz.pdf");
}

pub fn djvu(data: &[u8]) {
    adapter(&DjvuAdapter, data, "fuzz.djvu");
}

/// A rejected parse is a perfectly good outcome — most fuzzer inputs are not
/// valid documents. Only a *successful* parse has assets worth checking.
fn adapter(a: &dyn Adapter, data: &[u8], source_path: &str) {
    if let Ok((doc, assets)) = a.parse(data, source_path) {
        assert_assets_contained(&assets);
        assert_inline_depth_bounded(&doc);
        // `assert_inline_depth_bounded` makes the WALK safe; without this,
        // the ordinary, compiler-derived `Drop` when `doc` falls out of scope
        // at the end of this `if let` would still abort the process on a
        // hostile input's block nesting -- see `teardown_document`'s doc
        // comment below, and `kasane_core::nav::teardown_document`, which
        // this mirrors for the identical reason.
        teardown_document(doc);
    }
}

/// Tear `doc` down with an explicit worklist rather than letting the
/// compiler-derived `Drop` on `Block`/`Inline` recurse on block/inline
/// nesting depth. Block nesting (`Block::List`/`Block::Footnote`) has no
/// depth bound anywhere in this codebase (see `assert_inline_depth_bounded`'s
/// doc comment), so a hostile or fuzzer-found document's `Document` value can
/// be arbitrarily deep, and dropping it the ordinary way would abort the
/// process on the way out of `adapter()` even though every walk over it
/// above is now bounded. This is the same fix, for the same reason, as
/// `kasane_core::nav::teardown_document` -- duplicated here rather than
/// shared because `kasane-core`'s copy is private to that crate and this is
/// a small, self-contained utility.
fn teardown_document(doc: Document) {
    let mut blocks: Vec<Block> = doc.nodes.into_iter().map(|n| n.block).collect();
    while let Some(b) = blocks.pop() {
        match b {
            Block::Heading { inlines, .. } | Block::Para(inlines) => teardown_inlines(inlines),
            Block::List { items, .. } => {
                for item in items {
                    blocks.extend(item);
                }
            }
            Block::Table(t) => {
                for c in t.header {
                    teardown_inlines(c);
                }
                for r in t.rows {
                    for c in r {
                        teardown_inlines(c);
                    }
                }
            }
            Block::Figure { caption, .. } => teardown_inlines(caption),
            Block::Footnote { blocks: inner, .. } => blocks.extend(inner),
            Block::CodeBlock { .. } | Block::MathBlock(_) | Block::Raw { .. } => {}
        }
    }
}

fn teardown_inlines(inls: Vec<Inline>) {
    let mut stack = inls;
    while let Some(i) = stack.pop() {
        match i {
            Inline::Emph(x) | Inline::Strong(x) => stack.extend(x),
            Inline::Link { inlines, .. } => stack.extend(inlines),
            Inline::Text(_) | Inline::Code(_) | Inline::Math(_) | Inline::FootnoteRef(_) => {}
        }
    }
}

/// Design spec `2026-07-29-core-property-tier-design.md` §2.2: `kasane-core`
/// and `kasane-writer` walk INLINE nesting (`Inline::Emph`/`Strong`/`Link`)
/// recursively, so IR nested past `kasane_ir::MAX_INLINE_DEPTH` aborts the
/// process on a stack overflow rather than failing recoverably. No adapter
/// may produce it. Asserted against the core's safety bound rather than any
/// one adapter's flattening bound, because the core's is the value that
/// decides whether the process survives.
///
/// BLOCK nesting (`Block::List`/`Block::Footnote`) is a different property
/// and is **not bounded anywhere in this codebase** -- not in the EPUB
/// parser's `frames` stack, not in `kasane-core`, not here. A hostile or
/// fuzzer-found document can nest lists or footnotes arbitrarily deep, and
/// this function does not check that depth at all (nesting a list contributes
/// nothing to the value asserted below -- only the inline content reachable
/// through it does).
///
/// Both traversals below are therefore iterative, over an explicit worklist,
/// rather than function-call recursion -- on the block side because that
/// nesting is unbounded by construction, and on the inline side because the
/// bound this assertion exists to check is exactly what a hostile input may
/// be violating: assuming inline nesting is already shallow before checking
/// whether it's shallow is circular, and disabling an adapter's own flattening
/// bound (as this task's design spec does, to prove its seed reaches the bug)
/// demonstrates a recursive inline walk is not safe either. A recursive
/// version of either traversal can overflow *this assertion's own* stack
/// before ever reaching the `assert!` below -- in a fuzz seam that reads as a
/// crash in the test code, not in the adapter or core code under test, which
/// sends anyone triaging it in the wrong direction.
fn assert_inline_depth_bounded(doc: &Document) {
    // Max nesting depth of Emph/Strong/Link wrappers reachable from `inls`,
    // via an explicit stack of (slice, depth-of-this-slice) frames. Each
    // wrapper records `depth + 1` as a candidate max and pushes its own
    // contents with that as their depth; the deepest wrapper chain anywhere
    // in the tree is exactly the largest value recorded.
    fn inline_depth(inls: &[Inline]) -> usize {
        let mut max_depth = 0;
        let mut stack: Vec<(&[Inline], usize)> = vec![(inls, 0)];
        while let Some((slice, depth)) = stack.pop() {
            for i in slice {
                match i {
                    Inline::Emph(x) | Inline::Strong(x) => {
                        max_depth = max_depth.max(depth + 1);
                        stack.push((x, depth + 1));
                    }
                    Inline::Link { inlines, .. } => {
                        max_depth = max_depth.max(depth + 1);
                        stack.push((inlines, depth + 1));
                    }
                    _ => {}
                }
            }
        }
        max_depth
    }

    // Max inline depth anywhere in the block tree rooted at `root`, via an
    // explicit worklist of block references. List/Footnote nesting itself
    // contributes nothing to the value (matching the original recursive
    // definition this replaces) -- it only needs to be walked, however deep,
    // to reach every leaf block's inline content.
    fn block_depth(root: &Block) -> usize {
        let mut max_depth = 0;
        let mut stack: Vec<&Block> = vec![root];
        while let Some(b) = stack.pop() {
            match b {
                Block::Heading { inlines, .. } | Block::Para(inlines) => {
                    max_depth = max_depth.max(inline_depth(inlines));
                }
                Block::Figure { caption, .. } => {
                    max_depth = max_depth.max(inline_depth(caption));
                }
                Block::List { items, .. } => stack.extend(items.iter().flatten()),
                Block::Footnote { blocks, .. } => stack.extend(blocks.iter()),
                Block::Table(t) => {
                    for cell in t.header.iter().chain(t.rows.iter().flatten()) {
                        max_depth = max_depth.max(inline_depth(cell));
                    }
                }
                _ => {}
            }
        }
        max_depth
    }

    for node in &doc.nodes {
        let d = block_depth(&node.block);
        assert!(
            d <= kasane_ir::MAX_INLINE_DEPTH,
            "inline nesting depth {} exceeds MAX_INLINE_DEPTH {}",
            d,
            kasane_ir::MAX_INLINE_DEPTH
        );
    }
}

/// AGENTS.md: "No path traversal: sanitize archive entry names and asset
/// filenames, confine writes to `_assets/`." `AssetItem::filename` is what the
/// writer actually creates inside `_assets/`, and `guard::safe_media_filename`
/// is supposed to reduce it to a bare basename. Nothing checked that until now.
fn assert_assets_contained(assets: &AssetBag) {
    for item in &assets.items {
        let f = item.filename.as_str();
        assert!(!f.is_empty(), "empty asset filename");
        assert!(
            !f.contains('/') && !f.contains('\\'),
            "asset filename contains a path separator: {f:?}"
        );
        assert!(
            f != "." && f != "..",
            "asset filename is a directory traversal: {f:?}"
        );
    }
}

/// Split on the first NUL. Lets a multi-argument target keep the uniform
/// `fn(&[u8])` signature without pulling `arbitrary` into the library crate.
fn split2(data: &[u8]) -> (&[u8], &[u8]) {
    match data.iter().position(|b| *b == 0) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &[]),
    }
}

/// First byte selects the extension hint; the rest is the content. Detection is
/// the first code in the process to touch hostile bytes.
pub fn detect(data: &[u8]) {
    const HINTS: [Option<&str>; 8] = [
        None,
        Some("epub"),
        Some("pptx"),
        Some("mobi"),
        Some("azw3"),
        Some("pdf"),
        Some("djvu"),
        Some("../../etc/passwd"),
    ];
    let (hint, body) = match data.split_first() {
        Some((h, rest)) => (HINTS[(*h as usize) % HINTS.len()], rest),
        None => (None, data),
    };
    let _ = crate::detect(body, hint);
}

/// The highest-value target in the set. `capture_island` is the only thing
/// standing between an over-deep island and a stack overflow inside roxmltree,
/// and a stack overflow aborts the process — no `Result` plumbing recovers from
/// it. Feed the captured island to both front-ends, which are entry points in
/// their own right and re-check the budget themselves.
pub fn math_island(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut reader = Reader::from_str(text);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => return,
            Ok(Event::Start(start)) => {
                let start = start.into_owned();
                if let Ok(island) = capture_island(&mut reader, &start) {
                    let _ = mathml_to_latex(&island);
                    let _ = omml_to_latex(&island);
                }
            }
            Ok(_) => {}
        }
    }
}

/// A hand-rolled LZ77-style decoder reading attacker-controlled back-reference
/// distances and lengths.
pub fn palmdoc(data: &[u8]) {
    let _ = decompress(data);
}

/// Three pure functions whose postconditions are security-critical and, until
/// now, asserted nowhere: `resolve_rel`, `check_expansion`, and `percent_decode`
/// -- the last checked through `resolve_rel`, since what matters about it is
/// that decoding upstream of the segment loop cannot defeat confinement.
pub fn guards(data: &[u8]) {
    let (base, target) = split2(data);
    let (Ok(base), Ok(target)) = (std::str::from_utf8(base), std::str::from_utf8(target)) else {
        return;
    };

    // Both target shapes an adapter can hand to resolve_rel: the raw one, and
    // the percent-decoded one. Decoding runs BEFORE resolve_rel at every EPUB
    // call site precisely so that decoded separators are still normalized and
    // confined by its segment loop, so the postconditions must hold for both.
    let decoded = percent_decode(target);
    for target in [target, decoded.as_str()] {
        if let Some(path) = resolve_rel(base, target) {
            // resolve_rel joins segments, and a `..` segment pops rather than being
            // emitted -- but a segment may legitimately *contain* `..` (e.g.
            // `..foo`). Check components, not substrings, or valid input reports as
            // a crash.
            assert!(
                !path.split('/').any(|s| s == ".."),
                "resolve_rel emitted a traversal component: {path:?} from base={base:?} target={target:?}"
            );
            assert!(
                !path.starts_with('/') && !path.is_empty(),
                "resolve_rel emitted an absolute or empty path: {path:?}"
            );
        }
    }

    // Monotonicity: the streaming callers re-check as `decompressed` grows, so
    // a predicate that could flip back to true would let a bomb through.
    let (c, d) = (
        u64::from_le_bytes(std::array::from_fn(|i| *data.get(i).unwrap_or(&0))),
        u64::from_le_bytes(std::array::from_fn(|i| *data.get(i + 8).unwrap_or(&0))),
    );
    if !check_expansion(c, d) {
        assert!(
            !check_expansion(c, d.saturating_add(1)) && !check_expansion(c, u64::MAX),
            "check_expansion is non-monotone at compressed={c} decompressed={d}"
        );
    }
}

/// The entity-expansion surface. Drive a real reader so the `BytesRef` values
/// are the ones the adapters actually see.
pub fn xmltext(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut reader = Reader::from_str(text);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => return,
            Ok(Event::GeneralRef(r)) => {
                let _ = resolve_general_ref(&r);
            }
            Ok(_) => {}
        }
    }
}

/// Maximum members the builder will emit from one input. Without a cap a single
/// fuzzer input could build an archive with thousands of entries, which costs
/// throughput without buying coverage.
const MAX_ZIP_ENTRIES: usize = 64;

/// Assemble a structurally valid ZIP -- correct local headers, central
/// directory and CRCs -- from fuzzer-controlled entry names and contents.
///
/// The `zip` crate verifies CRCs on read, so raw byte mutation is rejected at
/// the container before it ever reaches the parsers underneath. Generating a
/// valid container puts the mutation budget on entry names and member payloads
/// instead, which is where `resolve_rel`, the bomb guards, and the OPF /
/// XHTML / math parsers live.
///
/// Input is NUL-separated fields read pairwise: name, content, name, content...
fn build_zip(data: &[u8]) -> Vec<u8> {
    let mut out = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut rest = data;
    for _ in 0..MAX_ZIP_ENTRIES {
        if rest.is_empty() {
            break;
        }
        let (name, after) = split2(rest);
        let (content, after) = split2(after);
        rest = after;
        // Lossy is right here: an entry name is a string to the zip crate, and
        // invalid UTF-8 should still produce *an* entry rather than skipping it.
        let name = String::from_utf8_lossy(name).into_owned();
        // An empty name is a legal entry to the zip crate, but it's useless
        // coverage -- substitute rather than spend the mutation budget on an
        // entry the parsers underneath can't meaningfully distinguish.
        let name = if name.is_empty() {
            "_".to_string()
        } else {
            name
        };
        if out.start_file(name, opts).is_err() {
            continue;
        }
        let _ = out.write_all(content);
    }
    match out.finish() {
        Ok(c) => c.into_inner(),
        Err(_) => Vec::new(),
    }
}

/// EPUB past the container (see `build_zip`).
pub fn epub_zip(data: &[u8]) {
    adapter(&EpubAdapter, &build_zip(data), "fuzz.epub");
}

/// PPTX past the container (see `build_zip`).
pub fn pptx_zip(data: &[u8]) {
    adapter(&PptxAdapter, &build_zip(data), "fuzz.pptx");
}

#[cfg(test)]
mod tests {
    use super::*;
    use kasane_ir::{AssetBag, AssetItem, DocMeta, Node, Provenance};

    fn bag(filename: &str) -> AssetBag {
        AssetBag {
            items: vec![AssetItem {
                key: "k".into(),
                filename: filename.into(),
                bytes: vec![],
            }],
        }
    }

    /// Block nesting (`Block::List`/`Block::Footnote`) has no depth bound
    /// anywhere in this codebase -- see the doc comment on
    /// `assert_inline_depth_bounded`. Before that traversal was made
    /// iterative, walking a document shaped like this one overflowed the
    /// assertion helper's OWN stack before it ever reached an `assert!` --
    /// confirmed locally against the pre-fix recursive form at this depth:
    /// `thread '...' has overflowed its stack` / `SIGABRT`, not a clean test
    /// failure. In a fuzz seam that reads as a crash in the test code, not
    /// the adapter or core code this assertion exists to check.
    ///
    /// 100_000 is comfortably past where the overflow was observed by hand
    /// at 5_000 (see this task's report for the reproduction).
    #[test]
    fn assert_inline_depth_bounded_survives_deeply_nested_lists() {
        const DEPTH: usize = 100_000;
        let mut blocks = vec![Block::Para(vec![Inline::Text("bottom".into())])];
        for _ in 0..DEPTH {
            blocks = vec![Block::List {
                ordered: false,
                items: vec![blocks],
            }];
        }
        let doc = Document {
            meta: DocMeta {
                title: "T".into(),
                authors: vec![],
                language: None,
                source_format: "test".into(),
                source_path: "test".into(),
            },
            nodes: blocks
                .into_iter()
                .map(|block| Node {
                    block,
                    prov: Provenance::default(),
                })
                .collect(),
        };
        // Must return normally rather than aborting the process. Block
        // nesting depth is not itself bounded or measured here -- only
        // inline nesting is -- so a document with zero inline nesting but
        // 100_000-deep block nesting must pass cleanly.
        assert_inline_depth_bounded(&doc);
        // Mirrors `adapter()`: tear `doc` down explicitly rather than letting
        // it fall out of scope here, which -- independent of the assertion
        // above -- would recurse on this same 100_000-deep block nesting via
        // the compiler-derived `Drop` and abort the process regardless of
        // how the assertion traversal is implemented.
        teardown_document(doc);
    }

    #[test]
    fn assets_containment_accepts_a_sanitized_basename() {
        assert_assets_contained(&bag("001-image.png"));
    }

    #[test]
    #[should_panic(expected = "path separator")]
    fn assets_containment_rejects_a_separator() {
        assert_assets_contained(&bag("../../etc/passwd"));
    }

    #[test]
    #[should_panic(expected = "traversal")]
    fn assets_containment_rejects_dotdot() {
        assert_assets_contained(&bag(".."));
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn every_fixture_survives_its_adapter() {
        let cases: &[(&str, fn(&[u8]))] = &[
            ("epub/minimal.epub", epub as fn(&[u8])),
            ("epub/rich.epub", epub),
            ("pptx/minimal.pptx", pptx),
            ("mobi/minimal.mobi", mobi),
            ("mobi/minimal-drm.mobi", mobi),
            ("azw3/minimal.azw3", mobi),
            ("azw3/lying-skel.azw3", mobi),
            ("pdf/minimal.pdf", pdf),
            ("pdf/no-outline.pdf", pdf),
            ("pdf/image.pdf", pdf),
            ("pdf/scanned.pdf", pdf),
            ("djvu/sample.djvu", djvu),
            ("djvu/scanned.djvu", djvu),
        ];
        for (rel, f) in cases {
            let path = format!("../../tests/fixtures/{rel}");
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            f(&bytes);
        }
    }

    #[test]
    fn truncated_and_empty_inputs_survive() {
        let bytes = std::fs::read("../../tests/fixtures/epub/rich.epub").unwrap();
        for f in [epub as fn(&[u8]), pptx, mobi, pdf, djvu] {
            f(&[]);
            f(&bytes[..bytes.len() / 2]);
            f(&bytes[..1]);
        }
    }

    #[test]
    fn split2_splits_on_the_first_nul_only() {
        assert_eq!(split2(b"ab\0cd\0ef"), (&b"ab"[..], &b"cd\0ef"[..]));
        assert_eq!(split2(b"abc"), (&b"abc"[..], &b""[..]));
        assert_eq!(split2(b""), (&b""[..], &b""[..]));
    }

    #[test]
    fn sub_parsers_survive_hostile_shapes() {
        // Deeply nested XML: the case capture_island exists to stop. 4x the
        // MAX_ISLAND_NESTING bound of 128, which would abort the process via
        // stack overflow if it reached roxmltree unguarded.
        let deep = format!(
            "<math>{}{}</math>",
            "<mrow>".repeat(512),
            "</mrow>".repeat(512)
        );
        math_island(deep.as_bytes());

        // Unclosed island: exercises the rewind path.
        math_island(b"<math><mrow><mi>x</mi>");

        // Entity-expansion shapes.
        xmltext(b"<p>&lt;&amp;&undefined;&#x41;&#999999999;</p>");

        // Back-reference opcodes with distances pointing before the buffer.
        palmdoc(&[0x80, 0x00, 0xFF, 0xC0, 0x01, 0x02]);

        // Traversal attempts, NUL-separated base and target.
        guards(b"ppt/slides\0../../../../etc/passwd");
        guards(b"\0/abs/path");

        for f in [detect as fn(&[u8]), math_island, palmdoc, guards, xmltext] {
            f(&[]);
            f(&[0u8; 64]);
            f(b"<<<>>>&&&\0\0\0");
        }
    }

    #[test]
    fn build_zip_produces_a_readable_archive() {
        let raw = b"mimetype\0application/epub+zip\0META-INF/container.xml\0<container/>";
        let bytes = build_zip(raw);
        let mut ar = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..]))
            .expect("builder must emit a structurally valid archive");
        assert_eq!(ar.len(), 2);
        assert_eq!(ar.by_index(0).unwrap().name(), "mimetype");
        assert_eq!(ar.by_index(1).unwrap().name(), "META-INF/container.xml");
    }

    #[test]
    fn build_zip_tolerates_hostile_entry_names() {
        // Names the builder must not choke on -- rejecting them is the
        // *adapter's* job, not the builder's.
        let raw = b"../../etc/passwd\0x\0/abs\0y\0\0z";
        let bytes = build_zip(raw);
        assert!(zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).is_ok());
    }

    #[test]
    fn zip_targets_survive_arbitrary_input() {
        for f in [epub_zip as fn(&[u8]), pptx_zip] {
            f(&[]);
            f(b"mimetype\0application/epub+zip");
            f(b"ppt/slides/slide1.xml\0<p:sld><m:oMath/></p:sld>");
            f(b"../../escape\0<x/>\0OEBPS/a.xhtml\0<math><mrow/></math>");
            f(&[0u8; 256]);
        }
    }

    #[test]
    fn check_expansion_is_monotone_in_decompressed() {
        // The property every streaming caller depends on: once the guard says
        // no, growing the decompressed size never makes it say yes again.
        for compressed in [0u64, 1, 7, 1024, u64::MAX] {
            for decompressed in [0u64, 1, 200, 201, crate::guard::MAX_TOTAL_BYTES] {
                if !crate::guard::check_expansion(compressed, decompressed) {
                    for grown in [decompressed.saturating_add(1), u64::MAX] {
                        assert!(
                            !crate::guard::check_expansion(compressed, grown),
                            "non-monotone at compressed={compressed} decompressed={decompressed} grown={grown}"
                        );
                    }
                }
            }
        }
    }
}
