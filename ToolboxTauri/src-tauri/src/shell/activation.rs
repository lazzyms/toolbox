use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;

use super::events::{ShellEvent, WorkspaceId};

static NEXT_ACTIVATION: AtomicU64 = AtomicU64::new(1);

#[derive(Deserialize)]
struct FileTypes {
    #[serde(rename = "pdf-editor")]
    pdf_editor: Vec<String>,
    #[serde(rename = "image-editor")]
    image_editor: Vec<String>,
}

pub fn parse_external_paths(paths: impl IntoIterator<Item = String>) -> ShellEvent {
    let paths = paths.into_iter().map(normalize_external_path).collect::<Vec<_>>();
    let path_strings = paths.iter().map(|path| path.to_string_lossy().into_owned()).collect::<Vec<_>>();
    let file_types: FileTypes = serde_json::from_str(include_str!("../../../shared/shell-file-types.json"))
        .expect("shared shell file types must be valid JSON");
    let mut workspaces = Vec::with_capacity(paths.len());
    let mut unsupported = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        match workspace_for_path(path, &file_types) {
            Some(workspace) => workspaces.push((index, workspace)),
            None => unsupported.push(path.extension().and_then(|extension| extension.to_str()).unwrap_or("").to_ascii_lowercase()),
        }
    }
    let reason = if !unsupported.is_empty() {
        let extensions = unsupported.iter().map(|extension| if extension.is_empty() { "(no extension)".to_string() } else { format!(".{extension}") }).collect::<Vec<_>>();
        Some(format!("Unsupported file type: {}", extensions.join(", ")))
    } else if workspaces.iter().any(|(_, workspace)| *workspace != workspaces[0].1) {
        Some("Choose PDF and image files separately.".to_string())
    } else if paths.len() > 1 {
        Some("Open one file at a time in this editor.".to_string())
    } else {
        None
    };
    if let Some(reason) = reason {
        return ShellEvent::RejectedFiles { paths: path_strings, reason };
    }
    let Some((_, workspace)) = workspaces.first() else {
        return ShellEvent::RejectedFiles { paths: path_strings, reason: "Choose a PDF or supported image file.".to_string() };
    };
    ShellEvent::Files {
        activation_id: format!("activation-{}-{}", std::process::id(), NEXT_ACTIVATION.fetch_add(1, Ordering::Relaxed)),
        workspace: *workspace,
        paths: path_strings,
    }
}

fn workspace_for_path(path: &Path, file_types: &FileTypes) -> Option<WorkspaceId> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if file_types.pdf_editor.iter().any(|item| item == &extension) {
        return Some(WorkspaceId::PdfEditor);
    }
    if file_types.image_editor.iter().any(|item| item == &extension) {
        return Some(WorkspaceId::ImageEditor);
    }
    None
}

fn normalize_external_path(path: String) -> PathBuf {
    url::Url::parse(&path)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .unwrap_or_else(|| PathBuf::from(path))
}
