use rayon::prelude::*;
use crate::kit::common::JobOutcome;
use crate::kit::contracts::partition_command_inputs;
use std::path::PathBuf;

pub struct BatchRunner;

impl BatchRunner {
    pub fn run<F>(
        command: &str,
        inputs: Vec<PathBuf>,
        job: F,
    ) -> Vec<JobOutcome>
    where
        F: Fn(PathBuf) -> JobOutcome + Sync + Send,
    {
        // rayon::into_par_iter automatically uses the system core count for the thread pool,
        // matching the original Swift `ProcessInfo.processInfo.activeProcessorCount` behavior.
        match partition_command_inputs(command, &inputs) {
            Ok(validation) => {
                let mut outcomes = vec![None; inputs.len()];
                for (index, outcome) in validation.rejected { outcomes[index] = Some(outcome); }
                let processed = validation.accepted.into_par_iter().map(|(index, path)| (index, job(path))).collect::<Vec<_>>();
                for (index, outcome) in processed { outcomes[index] = Some(outcome); }
                outcomes.into_iter().map(Option::unwrap).collect()
            }
            Err(outcomes) => outcomes,
        }
    }
}
