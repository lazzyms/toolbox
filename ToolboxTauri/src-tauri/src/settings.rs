use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;

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
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut stored = read_stored(settings_path);
    stored.output = Some(settings.clone());
    let directory = settings_path.parent().ok_or("Settings path has no parent folder.")?;
    std::fs::create_dir_all(directory).map_err(|error| format!("Could not create the settings folder: {error}"))?;
    let bytes = serde_json::to_vec_pretty(&stored).map_err(|error| error.to_string())?;
    let staged = settings_path.with_extension("json.tmp");
    std::fs::write(&staged, bytes).map_err(|error| format!("Could not save settings: {error}"))?;
    std::fs::rename(&staged, settings_path).map_err(|error| format!("Could not save settings: {error}"))?;
    Ok(settings)
}

fn paths(app: &tauri::AppHandle) -> Result<(PathBuf, PathBuf), String> {
    let resolver = app.path();
    let settings_path = resolver.app_data_dir().map_err(|error| error.to_string())?.join(SETTINGS_FILE);
    let documents = resolver
        .document_dir()
        .or_else(|_| resolver.home_dir())
        .map_err(|error| error.to_string())?;
    Ok((settings_path, documents.join("Toolbox")))
}

#[tauri::command]
pub fn get_output_settings(app: tauri::AppHandle) -> Result<OutputSettings, String> {
    let (settings_path, default_folder) = paths(&app)?;
    Ok(load_output_settings(&settings_path, &default_folder))
}

#[tauri::command]
pub fn set_output_settings(app: tauri::AppHandle, settings: OutputSettings) -> Result<OutputSettings, String> {
    let (settings_path, _) = paths(&app)?;
    save_output_settings(&settings_path, settings)
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
    fn rejects_a_relative_custom_folder() {
        let root = sandbox("relative");
        let path = root.join("settings.json");
        let error = save_output_settings(&path, OutputSettings { mode: OutputMode::CustomFolder, folder: "relative".into() }).unwrap_err();
        assert!(error.contains("full path"));
        assert!(!path.exists());
    }
}
