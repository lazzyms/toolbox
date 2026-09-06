use std::path::{Path, PathBuf};

fn validate_output_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err("Output path must not be empty.".to_string());
    }

    let metadata = path.metadata().map_err(|error| {
        format!(
            "Output file does not exist or cannot be accessed: {} ({error})",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "Output path is not a regular file: {}",
            path.display()
        ));
    }

    Ok(())
}

#[tauri::command]
pub fn open_output_path(path: PathBuf) -> Result<(), String> {
    validate_output_path(&path)?;
    open_file(&path)
}

#[tauri::command]
pub fn reveal_output_path(path: PathBuf) -> Result<(), String> {
    validate_output_path(&path)?;
    reveal_file(&path)
}

#[cfg(target_os = "macos")]
fn open_file(path: &Path) -> Result<(), String> {
    run_open([path.as_os_str()], "open", path)
}

#[cfg(target_os = "macos")]
fn reveal_file(path: &Path) -> Result<(), String> {
    run_open([std::ffi::OsStr::new("-R"), path.as_os_str()], "reveal", path)
}

#[cfg(target_os = "macos")]
fn run_open<const N: usize>(
    arguments: [&std::ffi::OsStr; N],
    action: &str,
    path: &Path,
) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/open")
        .args(arguments)
        .status()
        .map_err(|error| format!("Could not {action} output file {}: {error}", path.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Could not {action} output file {}: /usr/bin/open exited with {status}",
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
fn reveal_file(path: &Path) -> Result<(), String> {
    let mut argument = std::ffi::OsString::from("/select,");
    argument.push(path.as_os_str());
    std::process::Command::new("explorer.exe")
        .arg(argument)
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
fn reveal_file(path: &Path) -> Result<(), String> {
    Err(format!(
        "Revealing output files is unsupported on this platform: {}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::validate_output_path;
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
    fn rejects_an_empty_path() {
        let error = validate_output_path(Path::new("")).unwrap_err();
        assert!(error.contains("must not be empty"));
    }

    #[test]
    fn rejects_a_missing_path() {
        let error = validate_output_path(&unique_path("missing")).unwrap_err();
        assert!(error.contains("does not exist"));
    }

    #[test]
    fn accepts_a_regular_file() {
        let path = unique_path("file");
        std::fs::write(&path, b"output").unwrap();

        assert!(validate_output_path(&path).is_ok());

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn rejects_a_non_file_path() {
        let path = unique_path("directory");
        std::fs::create_dir(&path).unwrap();

        let error = validate_output_path(&path).unwrap_err();

        std::fs::remove_dir(&path).unwrap();
        assert!(error.contains("not a regular file"));
    }
}
