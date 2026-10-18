use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;
use crate::kit::contracts::{JobOutcome, OutcomeState};

const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputMode {
    AlongsideInput,
    CustomFolder,
}

// The folder is kept while the mode is alongside so switching back restores the last choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputSettings {
    pub mode: OutputMode,
    pub folder: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSettings {
    #[serde(default)]
    output: Option<OutputSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_save_folder: Option<PathBuf>,
}

static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn default_output(default_folder: &Path) -> OutputSettings {
    OutputSettings { mode: OutputMode::AlongsideInput, folder: default_folder.to_path_buf() }
}

fn read_stored(settings_path: &Path) -> StoredSettings {
    std::fs::read(settings_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn load_output_settings(settings_path: &Path, default_folder: &Path) -> OutputSettings {
    read_stored(settings_path).output.unwrap_or_else(|| default_output(default_folder))
}

pub fn save_output_settings(settings_path: &Path, settings: OutputSettings) -> Result<OutputSettings, String> {
    if settings.mode == OutputMode::CustomFolder {
        if !settings.folder.is_absolute() {
            return Err("Choose an output folder with a full path.".to_string());
        }
        std::fs::create_dir_all(&settings.folder)
            .map_err(|error| format!("Could not use {} as the output folder: {error}", settings.folder.display()))?;
    }
    update_stored(settings_path, |stored| stored.output = Some(settings.clone()))?;
    Ok(settings)
}

pub fn load_last_save_folder(settings_path: &Path, documents: &Path) -> PathBuf {
    read_stored(settings_path).last_save_folder.filter(|folder| folder.is_dir()).unwrap_or_else(|| documents.to_path_buf())
}

pub fn remember_save_folder(settings_path: &Path, outcomes: &[JobOutcome]) -> Result<(), String> {
    let saved = outcomes.iter().find_map(|outcome| match &outcome.state {
        OutcomeState::Succeeded { output_paths } => output_paths.first().and_then(|path| path.parent()).map(Path::to_path_buf),
        _ => None,
    });
    match saved {
        Some(folder) => update_stored(settings_path, |stored| stored.last_save_folder = Some(folder)),
        None => Ok(()),
    }
}

fn update_stored(settings_path: &Path, change: impl FnOnce(&mut StoredSettings)) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut stored = read_stored(settings_path);
    change(&mut stored);
    let directory = settings_path.parent().ok_or("Settings path has no parent folder.")?;
    std::fs::create_dir_all(directory).map_err(|error| format!("Could not create the settings folder: {error}"))?;
    let bytes = serde_json::to_vec_pretty(&stored).map_err(|error| error.to_string())?;
    let staged = settings_path.with_extension("json.tmp");
    std::fs::write(&staged, bytes).map_err(|error| format!("Could not save settings: {error}"))?;
    std::fs::rename(&staged, settings_path).map_err(|error| format!("Could not save settings: {error}"))
}

fn paths(app: &tauri::AppHandle) -> Result<(PathBuf, PathBuf), String> {
    let resolver = app.path();
    let settings_path = resolver.app_data_dir().map_err(|error| error.to_string())?.join(SETTINGS_FILE);
    let documents = resolver
        .document_dir()
        .or_else(|_| resolver.home_dir())
        .map_err(|error| error.to_string())?;
    Ok((settings_path, documents))
}

#[tauri::command]
pub fn get_output_settings(app: tauri::AppHandle) -> Result<OutputSettings, String> {
    let (settings_path, documents) = paths(&app)?;
    Ok(load_output_settings(&settings_path, &documents.join("Toolbox")))
}

#[tauri::command]
pub fn set_output_settings(app: tauri::AppHandle, settings: OutputSettings) -> Result<OutputSettings, String> {
    let (settings_path, _) = paths(&app)?;
    save_output_settings(&settings_path, settings)
}

#[tauri::command]
pub fn get_last_save_folder(app: tauri::AppHandle) -> Result<PathBuf, String> {
    let (settings_path, documents) = paths(&app)?;
    Ok(load_last_save_folder(&settings_path, &documents))
}

pub fn remember_chosen_save(app: &tauri::AppHandle, chosen_path: Option<&Path>, outcomes: Vec<JobOutcome>) -> Vec<JobOutcome> {
    if chosen_path.is_some() {
        if let Ok((settings_path, _)) = paths(app) {
            let _ = remember_save_folder(&settings_path, &outcomes);
        }
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("toolbox-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn returns_alongside_default_when_nothing_is_stored() {
        let root = sandbox("default");
        let settings = load_output_settings(&root.join("settings.json"), &root.join("Documents/Toolbox"));
        assert_eq!(settings, OutputSettings { mode: OutputMode::AlongsideInput, folder: root.join("Documents/Toolbox") });
    }

    #[test]
    fn returns_default_when_the_file_is_corrupt() {
        let root = sandbox("corrupt");
        let path = root.join("settings.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(load_output_settings(&path, &root).mode, OutputMode::AlongsideInput);
    }

    #[test]
    fn round_trips_both_modes() {
        let root = sandbox("round-trip");
        let path = root.join("app-data/settings.json");
        let folder = root.join("chosen");
        let custom = OutputSettings { mode: OutputMode::CustomFolder, folder: folder.clone() };
        assert_eq!(save_output_settings(&path, custom.clone()).unwrap(), custom);
        assert!(folder.is_dir());
        let loaded = load_output_settings(&path, &root.join("unused"));
        assert_eq!(loaded, custom);

        let alongside = OutputSettings { mode: OutputMode::AlongsideInput, folder: folder.clone() };
        save_output_settings(&path, alongside.clone()).unwrap();
        assert_eq!(load_output_settings(&path, &root.join("unused")), alongside);

        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["output"]["mode"], "alongsideInput");
    }

    #[test]
    fn last_save_folder_defaults_to_documents_then_reads_back_the_folder_of_a_successful_save() {
        let root = sandbox("last-save");
        let path = root.join("app-data/settings.json");
        let documents = root.join("Documents");
        assert_eq!(load_last_save_folder(&path, &documents), documents);

        let output = OutputSettings { mode: OutputMode::CustomFolder, folder: root.join("exports") };
        save_output_settings(&path, output.clone()).unwrap();
        let chosen = root.join("Leases");
        std::fs::create_dir_all(&chosen).unwrap();
        let failed = JobOutcome::failure(root.join("a.pdf"), crate::kit::contracts::ToolError::processing("no"));
        remember_save_folder(&path, &[failed]).unwrap();
        assert_eq!(load_last_save_folder(&path, &documents), documents);

        let saved = JobOutcome::succeeded(root.join("a.pdf"), vec![chosen.join("lease-merged.pdf")], "Saved");
        remember_save_folder(&path, &[saved]).unwrap();
        assert_eq!(load_last_save_folder(&path, &documents), chosen);
        assert_eq!(load_output_settings(&path, &root.join("unused")), output);

        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["output"]["mode"], "customFolder");
        assert_eq!(json["lastSaveFolder"], serde_json::json!(chosen));
    }

    #[test]
    fn a_settings_file_without_a_last_save_folder_still_reads_its_output() {
        let root = sandbox("legacy");
        let path = root.join("settings.json");
        std::fs::write(&path, br#"{"output":{"mode":"customFolder","folder":"/tmp/out"}}"#).unwrap();
        assert_eq!(load_output_settings(&path, &root).mode, OutputMode::CustomFolder);
        assert_eq!(load_last_save_folder(&path, &root.join("Documents")), root.join("Documents"));
    }

    #[test]
    fn rejects_a_relative_custom_folder() {
        let root = sandbox("relative");
        let path = root.join("settings.json");
        let error = save_output_settings(&path, OutputSettings { mode: OutputMode::CustomFolder, folder: "relative".into() }).unwrap_err();
        assert!(error.contains("full path"));
        assert!(!path.exists());
    }
}
