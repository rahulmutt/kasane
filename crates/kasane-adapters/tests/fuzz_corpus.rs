//! Replays committed fuzz corpora through the same `fuzz_entry` functions the
//! libFuzzer targets call — on the pinned **stable** toolchain, inside
//! `mise run test`, on every PR.
//!
//! `fuzz/artifacts/<target>/` holds crash reproducers. Committing one is
//! mandatory when the fuzzer finds a crash: that is what turns a one-off find
//! into a permanent regression test.
//!
//! `fuzz/seeds/<target>/` holds hand-written starting inputs. They are replayed
//! too, so these functions are exercised here from day one rather than staying
//! dead code until the first crash lands.

use kasane_adapters::fuzz_entry;
use std::path::Path;

/// Every fuzz target, by the directory name its corpus lives under.
///
/// Adding a target means adding it here. An unrecognized directory is a test
/// failure (see `unknown_corpus_directory_is_a_failure`), so a renamed target
/// cannot silently stop being replayed.
fn target(name: &str) -> Option<fn(&[u8])> {
    Some(match name {
        "epub" => fuzz_entry::epub,
        "pptx" => fuzz_entry::pptx,
        "mobi" => fuzz_entry::mobi,
        "pdf" => fuzz_entry::pdf,
        "djvu" => fuzz_entry::djvu,
        "epub_zip" => fuzz_entry::epub_zip,
        "pptx_zip" => fuzz_entry::pptx_zip,
        "detect" => fuzz_entry::detect,
        "math_island" => fuzz_entry::math_island,
        "palmdoc" => fuzz_entry::palmdoc,
        "guards" => fuzz_entry::guards,
        "xmltext" => fuzz_entry::xmltext,
        _ => return None,
    })
}

const TARGET_COUNT: usize = 12;

fn corpus_root(which: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz")
        .join(which)
}

/// Run every file under `fuzz/<which>/<target>/` through `<target>`.
fn replay(which: &str) -> usize {
    let root = corpus_root(which);
    if !root.is_dir() {
        return 0;
    }
    let mut ran = 0;
    for entry in std::fs::read_dir(&root).expect("corpus root is readable") {
        let dir = entry.expect("readable dir entry").path();
        if !dir.is_dir() {
            continue; // .gitkeep and friends
        }
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let f = target(&name).unwrap_or_else(|| {
            panic!(
                "{}/{name}/ has no matching fuzz target. Rename the directory or \
                 add the target to `target()` — a silent skip would stop replaying it.",
                root.display()
            )
        });
        for file in std::fs::read_dir(&dir).expect("corpus dir is readable") {
            let path = file.expect("readable file entry").path();
            if !path.is_file() {
                continue;
            }
            let bytes = std::fs::read(&path).expect("corpus file is readable");
            // A panic here is the point: it means a previously-found crash has
            // regressed, and the panic message names the offending input.
            f(&bytes);
            ran += 1;
        }
    }
    ran
}

#[test]
fn replays_committed_seeds() {
    let ran = replay("seeds");
    assert!(
        ran > 0,
        "no seed inputs were replayed — fuzz/seeds/ is empty or missing, which \
         would make this whole test pass vacuously"
    );
}

#[test]
fn replays_committed_crash_artifacts() {
    // Legitimately zero until the fuzzer finds something. Once a reproducer is
    // committed it is replayed forever.
    replay("artifacts");
}

#[test]
fn every_target_is_reachable_by_name() {
    let names = [
        "epub",
        "pptx",
        "mobi",
        "pdf",
        "djvu",
        "epub_zip",
        "pptx_zip",
        "detect",
        "math_island",
        "palmdoc",
        "guards",
        "xmltext",
    ];
    assert_eq!(names.len(), TARGET_COUNT);
    for n in names {
        assert!(target(n).is_some(), "target {n} is not mapped");
    }
    assert!(target("no_such_target").is_none());
}
