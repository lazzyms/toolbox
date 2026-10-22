#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputKind {
    File,
    Folder,
}

fn output_kind(path: &Path) -> Result<OutputKind, String> {
    if path.as_os_str().is_empty() {
        return Err("Output path must not be empty.".to_string());
    }

    let metadata = path.metadata().map_err(|error| {
        format!(
            "Output file does not exist or cannot be accessed: {} ({error})",
            path.display()
        )
    })?;
    if metadata.is_file() {
        Ok(OutputKind::File)
    } else if metadata.is_dir() {
        Ok(OutputKind::Folder)
    } else {
        Err(format!(
            "Output path is not a regular file or folder: {}",
            path.display()
        ))
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[derive(Debug, PartialEq, Eq)]
struct RevealCommand {
    program: &'static str,
    arguments: Vec<OsString>,
}

#[cfg(target_os = "macos")]
fn reveal_command(path: &Path, kind: OutputKind) -> RevealCommand {
    let arguments = match kind {
        OutputKind::File => vec![OsString::from("-R"), path.as_os_str().to_owned()],
        OutputKind::Folder => vec![path.as_os_str().to_owned()],
    };
    RevealCommand { program: "/usr/bin/open", arguments }
}

#[cfg(target_os = "windows")]
fn reveal_command(path: &Path, kind: OutputKind) -> RevealCommand {
    let argument = match kind {
        OutputKind::File => {
            let mut argument = OsString::from("/select,");
            argument.push(path.as_os_str());
            argument
        }
        OutputKind::Folder => path.as_os_str().to_owned(),
    };
    RevealCommand { program: "explorer.exe", arguments: vec![argument] }
}

#[tauri::command]
pub fn open_output_path(path: PathBuf) -> Result<(), String> {
    output_kind(&path)?;
    open_file(&path)
}

#[tauri::command]
pub fn reveal_output_path(path: PathBuf) -> Result<(), String> {
    let kind = output_kind(&path)?;
    reveal_file(&path, kind)
}

const NOTICES_FILE: &str = "THIRD-PARTY-NOTICES.txt";

// Bundles keep resources either directly under the resource root or under a
// nested resources/ folder, depending on the platform's bundle layout.
fn third_party_notices_path(root: &Path) -> Option<PathBuf> {
    [root.join("licenses"), root.join("resources").join("licenses")]
        .into_iter()
        .map(|folder| folder.join(NOTICES_FILE))
        .find(|path| path.is_file())
}

#[tauri::command]
pub fn open_third_party_notices() -> Result<(), String> {
    let path = crate::kit::resources::application_resource_root()
        .and_then(|root| third_party_notices_path(&root))
        .ok_or_else(|| "The third-party notices are missing from this copy of Toolbox. Reinstall Toolbox to restore them.".to_string())?;
    open_file(&path)
}

#[cfg(target_os = "macos")]
fn open_file(path: &Path) -> Result<(), String> {
    run_open("/usr/bin/open", &[path.as_os_str().to_owned()], "open", path)
}

#[cfg(target_os = "macos")]
fn reveal_file(path: &Path, kind: OutputKind) -> Result<(), String> {
    let command = reveal_command(path, kind);
    run_open(command.program, &command.arguments, "reveal", path)
}

#[cfg(target_os = "macos")]
fn run_open(program: &str, arguments: &[OsString], action: &str, path: &Path) -> Result<(), String> {
    let status = std::process::Command::new(program)
        .args(arguments)
        .status()
        .map_err(|error| format!("Could not {action} output file {}: {error}", path.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Could not {action} output file {}: {program} exited with {status}",
            path.display()
        ))
    }
}

#[cfg(target_os = "windows")]
fn open_file(path: &Path) -> Result<(), String> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show_command: i32,
        ) -> isize;
    }

    let operation: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    let file: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            ptr::null(),
            ptr::null(),
            1,
        )
    };
    if result > 32 {
        Ok(())
    } else {
        Err(format!(
            "Could not open output file {}: ShellExecuteW returned {result}",
            path.display()
        ))
    }
}

#[cfg(target_os = "windows")]
fn reveal_file(path: &Path, kind: OutputKind) -> Result<(), String> {
    let command = reveal_command(path, kind);
    std::process::Command::new(command.program)
        .args(command.arguments)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not reveal output file {}: {error}", path.display()))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn open_file(path: &Path) -> Result<(), String> {
    Err(format!(
        "Opening output files is unsupported on this platform: {}",
        path.display()
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn reveal_file(path: &Path, _kind: OutputKind) -> Result<(), String> {
    Err(format!(
        "Revealing output files is unsupported on this platform: {}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::{output_kind, third_party_notices_path, OutputKind};
    use std::path::{Path, PathBuf};

    fn unique_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "toolbox_file_actions_{}_{}_{}",
            std::process::id(),
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn finds_the_bundled_third_party_notices() {
        let root = crate::kit::resources::application_resource_root().unwrap();
        let path = third_party_notices_path(&root).unwrap();
        let notices = std::fs::read_to_string(&path).unwrap();
        assert!(notices.starts_with("Toolbox third-party notices"));
        assert!(notices.contains("x265"));

        let nested = unique_path("nested_notices");
        std::fs::create_dir_all(nested.join("resources").join("licenses")).unwrap();
        std::fs::write(nested.join("resources").join("licenses").join("THIRD-PARTY-NOTICES.txt"), b"notices").unwrap();
        let found = third_party_notices_path(&nested);
        let missing = third_party_notices_path(&nested.join("resources").join("licenses"));
        std::fs::remove_dir_all(&nested).unwrap();
        assert_eq!(found, Some(nested.join("resources").join("licenses").join("THIRD-PARTY-NOTICES.txt")));
        assert_eq!(missing, None);
    }

    #[test]
    fn rejects_an_empty_path() {
        let error = output_kind(Path::new("")).unwrap_err();
        assert!(error.contains("must not be empty"));
    }

    #[test]
    fn rejects_a_missing_path() {
        let error = output_kind(&unique_path("missing")).unwrap_err();
        assert!(error.contains("does not exist"));
    }

    #[test]
    fn accepts_a_regular_file() {
        let path = unique_path("file");
        std::fs::write(&path, b"output").unwrap();

        let kind = output_kind(&path);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(kind, Ok(OutputKind::File));
    }

    #[test]
    fn accepts_a_folder_output() {
        let path = unique_path("directory");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("frame-001.png"), b"frame").unwrap();

        let kind = output_kind(&path);

        std::fs::remove_dir_all(&path).unwrap();
        assert_eq!(kind, Ok(OutputKind::Folder));
    }

    #[test]
    fn a_failed_outcome_lists_its_published_outputs_as_actionable_paths() {
        use crate::kit::contracts::{JobOutcome, ToolError};

        let folder = unique_path("partial_outputs");
        std::fs::create_dir(&folder).unwrap();
        let frame = folder.join("source-frame-1.png");
        std::fs::write(&frame, b"frame").unwrap();
        let input = folder.join("source.gif");

        let outcome = JobOutcome::failure_with_outputs(
            input,
            vec![frame.clone(), folder.clone()],
            ToolError::processing("Could not publish the next frame."),
        );
        let value = serde_json::to_value(&outcome).unwrap();
        let kinds: Vec<_> = outcome.output_paths().iter().map(|path| output_kind(path)).collect();

        std::fs::remove_dir_all(&folder).unwrap();
        assert_eq!(value["state"], "failed");
        assert_eq!(value["outputPaths"], serde_json::json!([frame, folder]));
        assert_eq!(kinds, vec![Ok(OutputKind::File), Ok(OutputKind::Folder)]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reveal_selects_a_file_and_opens_a_folder() {
        use super::reveal_command;
        use std::ffi::OsString;

        let folder = unique_path("reveal_folder");
        std::fs::create_dir(&folder).unwrap();
        let file = folder.join("page-01.png");
        std::fs::write(&file, b"page").unwrap();

        let file_command = reveal_command(&file, output_kind(&file).unwrap());
        let folder_command = reveal_command(&folder, output_kind(&folder).unwrap());

        std::fs::remove_dir_all(&folder).unwrap();
        assert_eq!(file_command.program, "/usr/bin/open");
        assert_eq!(file_command.arguments, vec![OsString::from("-R"), file.into_os_string()]);
        assert_eq!(folder_command.program, "/usr/bin/open");
        assert_eq!(folder_command.arguments, vec![folder.into_os_string()]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn reveal_selects_a_file_and_opens_a_folder() {
        use super::reveal_command;
        use std::ffi::OsString;

        let folder = unique_path("reveal_folder");
        std::fs::create_dir(&folder).unwrap();
        let file = folder.join("page-01.png");
        std::fs::write(&file, b"page").unwrap();

        let file_command = reveal_command(&file, output_kind(&file).unwrap());
        let folder_command = reveal_command(&folder, output_kind(&folder).unwrap());

        std::fs::remove_dir_all(&folder).unwrap();
        let mut selected = OsString::from("/select,");
        selected.push(file.as_os_str());
        assert_eq!(file_command.program, "explorer.exe");
        assert_eq!(file_command.arguments, vec![selected]);
        assert_eq!(folder_command.program, "explorer.exe");
        assert_eq!(folder_command.arguments, vec![folder.into_os_string()]);
    }
}
