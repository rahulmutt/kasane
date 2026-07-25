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
