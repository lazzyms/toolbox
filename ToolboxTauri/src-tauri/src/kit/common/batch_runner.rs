use rayon::prelude::*;
use crate::kit::common::JobOutcome;
use crate::kit::contracts::{partition_command_inputs, OutcomePointer, OutputLocation, ToolError};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct BatchRunner;

pub const READ_ONLY_OUTPUT_MESSAGE: &str = "The output folder is read-only. Nothing was saved.";

impl BatchRunner {
    pub fn run<F>(
        command: &str,
        inputs: Vec<PathBuf>,
        location: &OutputLocation,
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
                let mut writable = HashMap::new();
                let mut runnable = Vec::with_capacity(validation.accepted.len());
                for (index, path) in validation.accepted {
                    let directory = location.directory_for(&path).to_path_buf();
                    let is_writable = *writable.entry(directory).or_insert_with_key(|directory| !is_read_only(directory));
                    if is_writable {
                        runnable.push((index, path));
                    } else {
                        outcomes[index] = Some(JobOutcome::failure(
                            path,
                            ToolError::processing(READ_ONLY_OUTPUT_MESSAGE).with_pointer(OutcomePointer::OutputFolder),
                        ));
                    }
                }
                let processed = runnable.into_par_iter().map(|(index, path)| (index, job(path))).collect::<Vec<_>>();
                for (index, outcome) in processed { outcomes[index] = Some(outcome); }
                outcomes.into_iter().map(Option::unwrap).collect()
            }
            Err(outcomes) => outcomes,
        }
    }
}

// Permission bits miss ACLs, sandboxing and read-only mounts, so only a real create answers the question.
fn is_read_only(directory: &Path) -> bool {
    static PROBE: AtomicUsize = AtomicUsize::new(0);
    let probe = directory.join(format!(
        ".toolbox-write-check-{}-{}",
        std::process::id(),
        PROBE.fetch_add(1, Ordering::Relaxed),
    ));
    match OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            false
        }
        Err(error) => matches!(error.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem),
    }
}
