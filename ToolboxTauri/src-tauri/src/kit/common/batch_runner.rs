use rayon::prelude::*;
use crate::kit::common::JobOutcome;
use std::path::PathBuf;

pub struct BatchRunner;

impl BatchRunner {
    pub fn run<F>(
        inputs: Vec<PathBuf>,
        job: F,
    ) -> Vec<JobOutcome>
    where
        F: Fn(PathBuf) -> JobOutcome + Sync + Send,
    {
        // rayon::into_par_iter automatically uses the system core count for the thread pool,
        // matching the original Swift `ProcessInfo.processInfo.activeProcessorCount` behavior.
        inputs.into_par_iter().map(job).collect()
    }
}
