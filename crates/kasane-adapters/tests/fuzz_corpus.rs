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
/// failure -- `replay()` panics when `target()` returns `None` for a corpus
/// directory it finds on disk -- so a renamed target cannot silently stop
/// being replayed.
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
        "slug" => kasane_gfm::fuzz_entry::slug,
        "escape" => kasane_writer::fuzz_entry::escape,
        _ => return None,
    })
}

const TARGET_COUNT: usize = 14;

/// Reproducers whose underlying bug is still open, keyed on (target
/// directory, filename) rather than filename alone -- unambiguous if two
/// targets ever produce a same-named file. Quarantining one here is what
/// keeps `mise run test` (this file, the stable replay) green despite a
/// real, uncommitted-fix bug; removing the entry is what re-arms its
/// regression test once the bug is fixed -- do that as part of the fix, not
/// before.
///
/// This quarantine protects the stable replay test ONLY. It has no effect on
/// `mise run fuzz` / `mise run fuzz-all`, which still reproduce a quarantined
/// crash on nightly. That is intended, not a gap to close: a weekly fuzz job
/// going red on a real, unfixed bug is the correct signal, and fuzz targets
/// get no skip logic. The list is empty whenever every committed reproducer
/// has a landed fix behind it, which is the steady state.
const KNOWN_OPEN: &[(&str, &str)] = &[];

fn corpus_root(which: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz")
        .join(which)
}

/// Run every file under `fuzz/<which>/<target>/` through `<target>`, except
/// files quarantined in `KNOWN_OPEN`.
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
            let filename = path.file_name().unwrap().to_string_lossy().into_owned();
            if which == "artifacts" && KNOWN_OPEN.contains(&(name.as_str(), filename.as_str())) {
                println!(
                    "SKIPPING quarantined reproducer fuzz/artifacts/{name}/{filename}: \
                     bug still open (see KNOWN_OPEN in fuzz_corpus.rs), not replayed"
                );
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
fn known_open_entries_have_a_reproducer_on_disk() {
    for (target_dir, filename) in KNOWN_OPEN {
        let path = corpus_root("artifacts").join(target_dir).join(filename);
        assert!(
            path.is_file(),
            "KNOWN_OPEN names fuzz/artifacts/{target_dir}/{filename}, which no \
             longer exists on disk. Remove the stale entry from KNOWN_OPEN in \
             fuzz_corpus.rs — a quarantine entry with no reproducer behind it \
             silently suppresses nothing forever.",
        );
    }
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
        "slug",
        "escape",
    ];
    assert_eq!(names.len(), TARGET_COUNT);
    for n in names {
        assert!(target(n).is_some(), "target {n} is not mapped");
    }
    assert!(target("no_such_target").is_none());
}
