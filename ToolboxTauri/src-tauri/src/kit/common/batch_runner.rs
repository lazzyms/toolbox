use rayon::prelude::*;
use crate::kit::common::limits::{self, ToolLimits};
use crate::kit::common::JobOutcome;
use crate::kit::contracts::{partition_command_inputs, OutcomePointer, OutputLocation, ToolError};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub struct BatchRunner;

pub const READ_ONLY_OUTPUT_MESSAGE: &str = "The output folder is read-only. Nothing was saved.";

impl BatchRunner {
    pub fn run<R, F>(
        command: &str,
        inputs: Vec<PathBuf>,
        location: &OutputLocation,
        request: &Arc<R>,
        job: F,
    ) -> Vec<JobOutcome>
    where
        R: Send + Sync + 'static,
        F: Fn(&R, PathBuf) -> JobOutcome + Send + Sync + 'static,
    {
        Self::run_with_limits(command, inputs, location, limits::recorded(command), request, job)
    }

    pub fn run_with_limits<R, F>(
        command: &str,
        inputs: Vec<PathBuf>,
        location: &OutputLocation,
        limits: Option<ToolLimits>,
        request: &Arc<R>,
        job: F,
    ) -> Vec<JobOutcome>
    where
        R: Send + Sync + 'static,
        F: Fn(&R, PathBuf) -> JobOutcome + Send + Sync + 'static,
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
                let job = Arc::new(job);
                let processed = runnable.into_par_iter().map(|(index, path)| {
                    let Some(limits) = limits else {
                        return (index, JobOutcome::failure(path, ToolError::processing(format!("No processing limits are recorded for {command}."))));
                    };
                    let (job, request) = (Arc::clone(&job), Arc::clone(request));
                    (index, limits.run_each(path, move |path| job(&request, path)))
                }).collect::<Vec<_>>();
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderCheck { Ready, Missing, Unwritable }

pub fn check_output_folder(folder: &Path) -> FolderCheck {
    if !folder.is_dir() { FolderCheck::Missing }
    else if is_read_only(folder) { FolderCheck::Unwritable }
    else { FolderCheck::Ready }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("toolbox-folder-check-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn writable_folder_is_ready_and_left_empty() {
        let root = sandbox("ready");
        assert_eq!(check_output_folder(&root), FolderCheck::Ready);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0, "the probe leaves nothing behind");
    }

    #[test]
    fn missing_folder_and_file_path_report_missing() {
        let root = sandbox("missing");
        let file = root.join("notes.txt");
        std::fs::write(&file, b"keep").unwrap();
        assert_eq!(check_output_folder(&root.join("Protected")), FolderCheck::Missing);
        assert_eq!(check_output_folder(&file), FolderCheck::Missing);
        assert!(!root.join("Protected").exists(), "the check never creates the folder");
        assert_eq!(std::fs::read(&file).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[test]
    fn read_only_folder_reports_unwritable() {
        use std::os::unix::fs::PermissionsExt;
        let root = sandbox("unwritable");
        let locked = root.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let check = check_output_folder(&locked);
        let listing = std::fs::read_dir(&locked).unwrap().count();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(check, FolderCheck::Unwritable);
        assert_eq!(listing, 0);
    }
}
