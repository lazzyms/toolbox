use rayon::prelude::*;
use crate::kit::common::limits::{self, CancelFlag, ToolLimits};
use crate::kit::common::JobOutcome;
use crate::kit::contracts::{partition_command_inputs, OutcomePointer, OutputLocation, SkipCode, SkipReason, ToolError};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

pub struct BatchRunner;

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum FileEvent {
    Started { index: usize },
    Finished { index: usize, outcome: JobOutcome },
    /// How far a long single-file job has got, such as the page a PDF export is on.
    Progress { index: usize, completed: usize, total: usize },
}

pub(crate) type Sink = Arc<dyn Fn(FileEvent) + Send + Sync>;

thread_local! {
    static PROGRESS: std::cell::RefCell<Option<(usize, Sink)>> = const { std::cell::RefCell::new(None) };
}

/// Sends progress this thread reports to `sink` as the file at `index`. Each job runs on its own thread, so the reporter belongs to that file alone.
pub(crate) fn report_progress_to(index: usize, sink: Option<Sink>) {
    PROGRESS.with(|progress| *progress.borrow_mut() = sink.map(|sink| (index, sink)));
}

/// Reports progress for the file the current job is processing. Outside a batch run, such as a preview, it does nothing.
pub fn report_progress(completed: usize, total: usize) {
    PROGRESS.with(|progress| {
        if let Some((index, sink)) = &*progress.borrow() { sink(FileEvent::Progress { index: *index, completed, total }) }
    });
}

/// A run's link to the view that started it. Progress goes out through the sink and a cancel comes back in by run id.
pub struct RunEvents {
    sink: Option<Sink>,
    cancel: RunCancel,
}

/// A cancel flag that [`cancel_run`] reaches by run id for as long as it lives.
pub struct RunCancel {
    flag: CancelFlag,
    run_id: Option<String>,
}

impl RunCancel {
    pub fn unregistered() -> Self { Self { flag: CancelFlag::default(), run_id: None } }
    pub fn register(run_id: String) -> Self {
        let flag = CancelFlag::default();
        running().insert(run_id.clone(), flag.clone());
        Self { flag, run_id: Some(run_id) }
    }
    pub fn flag(&self) -> &CancelFlag { &self.flag }
}

impl Drop for RunCancel {
    fn drop(&mut self) {
        let Some(run_id) = &self.run_id else { return };
        let mut running = running();
        if running.get(run_id).is_some_and(|flag| flag.same_as(&self.flag)) { running.remove(run_id); }
    }
}

static RUNNING: LazyLock<Mutex<HashMap<String, CancelFlag>>> = LazyLock::new(Default::default);

fn running() -> std::sync::MutexGuard<'static, HashMap<String, CancelFlag>> {
    RUNNING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Asks the run with this id to stop. False when no such run is in progress.
pub fn cancel_run(run_id: &str) -> bool {
    running().get(run_id).map(CancelFlag::cancel).is_some()
}

pub const CANCELLED_BEFORE_START: &str = "Cancelled before it started";

impl RunEvents {
    pub fn silent() -> Self { Self { sink: None, cancel: RunCancel::unregistered() } }
    pub fn new(sink: impl Fn(FileEvent) + Send + Sync + 'static) -> Self {
        Self { sink: Some(Arc::new(sink)), cancel: RunCancel::unregistered() }
    }
    pub fn for_run(run_id: String, sink: impl Fn(FileEvent) + Send + Sync + 'static) -> Self {
        Self { sink: Some(Arc::new(sink)), cancel: RunCancel::register(run_id) }
    }
    fn emit(&self, event: FileEvent) { if let Some(sink) = &self.sink { sink(event) } }
    fn settled(&self, index: usize, outcome: &JobOutcome) {
        self.emit(FileEvent::Started { index });
        self.emit(FileEvent::Finished { index, outcome: outcome.clone() });
    }
}

pub const READ_ONLY_OUTPUT_MESSAGE: &str = "The output folder is read-only. Nothing was saved.";

impl BatchRunner {
    pub fn run<R, F>(
        events: &RunEvents,
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
        Self::run_with_limits(events, command, inputs, location, limits::recorded(command), request, job)
    }

    /// Runs a job that only reads its inputs. It saves nothing, so no output folder is probed and nothing is created.
    pub fn inspect<R, F>(events: &RunEvents, command: &str, inputs: Vec<PathBuf>, request: &Arc<R>, job: F) -> Vec<JobOutcome>
    where
        R: Send + Sync + 'static,
        F: Fn(&R, PathBuf) -> JobOutcome + Send + Sync + 'static,
    {
        let limits = limits::recorded(command);
        Self::run_batch(events, command, inputs, None, move |_| limits, request, job)
    }

    pub fn run_with_limits<R, F>(
        events: &RunEvents,
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
        Self::run_batch(events, command, inputs, Some(location), move |_| limits, request, job)
    }

    /// For a command whose limits depend on the input, such as Convert to PDF, which takes many formats.
    pub fn run_limited<R, L, F>(
        events: &RunEvents,
        command: &str,
        inputs: Vec<PathBuf>,
        location: &OutputLocation,
        limits_for: L,
        request: &Arc<R>,
        job: F,
    ) -> Vec<JobOutcome>
    where
        R: Send + Sync + 'static,
        L: Fn(&Path) -> Option<ToolLimits> + Sync,
        F: Fn(&R, PathBuf) -> JobOutcome + Send + Sync + 'static,
    {
        Self::run_batch(events, command, inputs, Some(location), limits_for, request, job)
    }

    /// `location` is where outputs go; `None` means the job writes nothing.
    fn run_batch<R, L, F>(
        events: &RunEvents,
        command: &str,
        inputs: Vec<PathBuf>,
        location: Option<&OutputLocation>,
        limits_for: L,
        request: &Arc<R>,
        job: F,
    ) -> Vec<JobOutcome>
    where
        R: Send + Sync + 'static,
        L: Fn(&Path) -> Option<ToolLimits> + Sync,
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
                    let is_writable = location.is_none_or(|location| {
                        let directory = location.directory_for(&path).to_path_buf();
                        *writable.entry(directory).or_insert_with_key(|directory| !is_read_only(directory))
                    });
                    if is_writable {
                        runnable.push((index, path));
                    } else {
                        outcomes[index] = Some(JobOutcome::failure(
                            path,
                            ToolError::processing(READ_ONLY_OUTPUT_MESSAGE).with_pointer(OutcomePointer::OutputFolder),
                        ));
                    }
                }
                for (index, outcome) in outcomes.iter().enumerate() {
                    if let Some(outcome) = outcome { events.settled(index, outcome); }
                }
                let job = Arc::new(job);
                let processed = runnable.into_par_iter().map(|(index, path)| {
                    if events.cancel.flag().is_cancelled() {
                        let outcome = JobOutcome::skipped(path, SkipReason::new(SkipCode::Cancelled, CANCELLED_BEFORE_START));
                        events.settled(index, &outcome);
                        return (index, outcome);
                    }
                    events.emit(FileEvent::Started { index });
                    let outcome = match limits_for(&path) {
                        Some(limits) => {
                            let (job, request, sink) = (Arc::clone(&job), Arc::clone(request), events.sink.clone());
                            limits.run_each(path, events.cancel.flag(), move |path| {
                                report_progress_to(index, sink);
                                job(&request, path)
                            })
                        }
                        None => JobOutcome::failure(path, ToolError::processing(format!("No processing limits are recorded for {command}."))),
                    };
                    events.emit(FileEvent::Finished { index, outcome: outcome.clone() });
                    (index, outcome)
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
    fn file_events_use_the_phase_tagged_wire_shape() {
        let outcome = JobOutcome::failure(PathBuf::from("/in/a.pdf"), ToolError::processing("broken"));
        assert_eq!(serde_json::to_value(FileEvent::Started { index: 2 }).unwrap(), serde_json::json!({ "phase": "started", "index": 2 }));
        let finished = serde_json::to_value(FileEvent::Finished { index: 0, outcome: outcome.clone() }).unwrap();
        assert_eq!(finished["phase"], "finished");
        assert_eq!(finished["index"], 0);
        assert_eq!(finished["outcome"], serde_json::to_value(&outcome).unwrap());
        assert_eq!(
            serde_json::to_value(FileEvent::Progress { index: 0, completed: 4, total: 9 }).unwrap(),
            serde_json::json!({ "phase": "progress", "index": 0, "completed": 4, "total": 9 })
        );
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
