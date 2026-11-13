use serde::Serialize;

pub const SHELL_EVENT: &str = "toolbox://shell-event";

#[derive(Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum WorkspaceId {
    PdfEditor,
    ImageEditor,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ShellCommand {
    Open,
    Undo,
    Redo,
    Export,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleTheme,
    Shortcuts,
}

/// Where files were dropped, in CSS pixels from the top left of the window.
#[derive(Clone, Copy, Serialize)]
pub struct DropPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub(crate) enum ShellEvent {
    Files { activation_id: String, workspace: WorkspaceId, paths: Vec<String> },
    DroppedFiles { paths: Vec<String>, point: DropPoint },
    Command { command: ShellCommand },
    RejectedFiles { paths: Vec<String>, reason: String },
}
