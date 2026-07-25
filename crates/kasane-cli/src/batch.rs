use crate::convert::{convert_one, ConvertOptions, Converted, WorkItem};
use anyhow::{Context, Result};
use rayon::prelude::*;

/// One document's fate in a batch run.
pub struct Outcome {
    // Not yet read outside this file: the run summary (Task 4) is what
    // surfaces it, to name which document a failure belongs to.
    #[allow(dead_code)]
    pub item: WorkItem,
    pub result: Result<Converted>,
}

/// Convert every item across `jobs` workers.
///
/// `into_par_iter().collect()` preserves input order, so the summary and the
/// library index are deterministic no matter which document finishes first.
/// Per-document failures are carried in each `Outcome`; the `Result` here is
/// only for a thread pool that cannot be built.
pub fn run_batch(items: Vec<WorkItem>, jobs: usize, opts: &ConvertOptions) -> Result<Vec<Outcome>> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .context("create worker thread pool")?;

    Ok(pool.install(|| {
        items
            .into_par_iter()
            .map(|item| {
                let result = convert_one(&item, opts);
                // Printed as each document finishes, so this is completion
                // order; the end-of-run summary is in input order.
                match &result {
                    Ok(done) => eprintln!(
                        "  {} -> {} ({} files)",
                        item.rel,
                        item.out_dir.display(),
                        done.files
                    ),
                    Err(e) => eprintln!("  {} FAILED: {e:#}", item.rel),
                }
                Outcome { item, result }
            })
            .collect()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> ConvertOptions {
        ConvertOptions {
            max_tokens: 2000,
            min_tokens: 200,
            force: false,
            ocr: false,
            ocr_lang: "eng".into(),
            ocr_no_image: false,
        }
    }

    /// `into_par_iter().collect()` is documented to preserve input order, but
    /// nothing else pins that guarantee — the summary (Task 4) and the library
    /// index (Task 5) both depend on `Outcome`s coming back in input order
    /// regardless of which worker finishes first. Every input here points at
    /// a nonexistent file, so every conversion fails fast and independently;
    /// only the ordering of the returned `Vec<Outcome>` is under test.
    #[test]
    fn outcomes_preserve_input_order_across_workers() {
        let items: Vec<WorkItem> = (0..20)
            .map(|i| WorkItem {
                input: format!("no-such-file-{i}").into(),
                out_dir: format!("out-{i}").into(),
                rel: format!("no-such-file-{i}"),
            })
            .collect();
        let expected: Vec<String> = items.iter().map(|i| i.rel.clone()).collect();

        let outcomes = run_batch(items, 4, &opts()).unwrap();

        assert!(
            outcomes.iter().all(|o| o.result.is_err()),
            "every input is a nonexistent file; all should fail"
        );
        let got: Vec<String> = outcomes.iter().map(|o| o.item.rel.clone()).collect();
        assert_eq!(got, expected, "run_batch must preserve input order");
    }
}
