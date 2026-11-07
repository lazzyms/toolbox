pub mod batch_runner;
pub mod limits;
pub mod watermark;
#[cfg(all(test, unix))]
pub mod fake_helper;

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
pub use crate::kit::contracts::JobOutcome;
pub use crate::kit::contracts::OutputLocation;
use crate::kit::contracts::{OutcomeState, OutcomeWarning, WarningKind};

pub struct OutputNaming;

pub struct OutputReservation {
    path: PathBuf,
    temporary_path: PathBuf,
    claim_path: PathBuf,
}

impl OutputReservation {
    pub fn path(&self) -> &Path {
        &self.temporary_path
    }

    pub fn destination_path(&self) -> &Path {
        &self.path
    }

    pub fn publish(self) -> io::Result<PathBuf> {
        limits::publish_within_time(|| std::fs::hard_link(&self.temporary_path, &self.path).map(|()| self.path.clone()))
    }
}

impl Drop for OutputReservation {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.temporary_path);
        let _ = std::fs::remove_file(&self.claim_path);
    }
}

pub struct FolderReservation {
    path: PathBuf,
    published: bool,
}

impl FolderReservation {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn publish(mut self) -> PathBuf {
        self.published = true;
        self.path.clone()
    }
}

impl Drop for FolderReservation {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_dir(&self.path);
        }
    }
}

/// A folder output built privately and published whole. The caller writes files
/// and subfolders into `path()`, a hidden staging folder beside the destination,
/// then `publish` moves it to `destination_path()` in one rename that never
/// replaces anything. Discarding or dropping it unpublished removes the staging
/// folder and the name claim, so nothing appears in the destination.
pub struct StagedFolderReservation {
    path: PathBuf,
    staging_path: PathBuf,
    claim_path: PathBuf,
    published: bool,
}

impl StagedFolderReservation {
    pub fn path(&self) -> &Path {
        &self.staging_path
    }

    pub fn destination_path(&self) -> &Path {
        &self.path
    }

    pub fn publish(mut self) -> io::Result<PathBuf> {
        limits::publish_within_time(|| {
            OutputNaming::retry_while_busy(|| rename_no_replace(&self.staging_path, &self.path))?;
            self.published = true;
            Ok(self.path.clone())
        })
    }

    pub fn discard(self) {}
}

impl Drop for StagedFolderReservation {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_dir_all(&self.staging_path);
        }
        let _ = std::fs::remove_file(&self.claim_path);
    }
}

/// Moves `from` to `to` in one step and fails with `AlreadyExists` (or another
/// error) rather than replacing whatever is at `to`, including an empty folder.
#[cfg(target_os = "macos")]
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::{c_char, c_int, c_uint, CString};
    use std::os::unix::ffi::OsStrExt;

    extern "C" {
        fn renamex_np(from: *const c_char, to: *const c_char, flags: c_uint) -> c_int;
    }
    const RENAME_EXCL: c_uint = 0x0000_0004;

    let (source, target) = (CString::new(from.as_os_str().as_bytes())?, CString::new(to.as_os_str().as_bytes())?);
    if unsafe { renamex_np(source.as_ptr(), target.as_ptr(), RENAME_EXCL) } == 0 {
        return Ok(());
    }
    unsupported_exclusive_rename_fallback(io::Error::last_os_error(), from, to)
}

#[cfg(target_os = "linux")]
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::{c_char, c_int, c_uint, CString};
    use std::os::unix::ffi::OsStrExt;

    extern "C" {
        fn renameat2(old_dir: c_int, old_path: *const c_char, new_dir: c_int, new_path: *const c_char, flags: c_uint) -> c_int;
    }
    const AT_FDCWD: c_int = -100;
    const RENAME_NOREPLACE: c_uint = 1;

    let (source, target) = (CString::new(from.as_os_str().as_bytes())?, CString::new(to.as_os_str().as_bytes())?);
    if unsafe { renameat2(AT_FDCWD, source.as_ptr(), AT_FDCWD, target.as_ptr(), RENAME_NOREPLACE) } == 0 {
        return Ok(());
    }
    unsupported_exclusive_rename_fallback(io::Error::last_os_error(), from, to)
}

/// A volume that cannot do an exclusive rename (some network and FUSE file
/// systems) still gets a plain rename, which on POSIX refuses a file or a
/// non-empty folder at `to`; only an empty folder created in the instant between
/// the check and the rename could be replaced there.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn unsupported_exclusive_rename_fallback(error: io::Error, from: &Path, to: &Path) -> io::Result<()> {
    const ENOSYS: i32 = if cfg!(target_os = "macos") { 78 } else { 38 };
    const EINVAL: i32 = 22;
    const ENOTSUP: i32 = if cfg!(target_os = "macos") { 45 } else { 95 };
    match error.raw_os_error() {
        Some(code) if code == EINVAL || code == ENOTSUP || code == ENOSYS => checked_rename(from, to),
        _ => Err(error),
    }
}

#[cfg(windows)]
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }

    let wide = |path: &Path| path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (source, target) = (wide(from), wide(to));
    // No MOVEFILE_REPLACE_EXISTING, so an existing file or folder fails the move,
    // and no MOVEFILE_COPY_ALLOWED, so it is a rename and never a partial copy.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0) } != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    checked_rename(from, to)
}

#[cfg(not(windows))]
fn checked_rename(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", to.display())));
    }
    std::fs::rename(from, to)
}

impl OutputNaming {
    pub fn reserve_destination(
        input_path: &Path,
        location: &OutputLocation,
        suffix: &str,
        extension: &str,
    ) -> io::Result<OutputReservation> {
        Self::allocate(input_path, location, suffix, Some(extension), |path| {
            Self::reserve_candidate(path).map(|reservation| reservation.map(|(reservation, _)| reservation))
        })
    }

    pub fn reserve_folder(
        input_path: &Path,
        location: &OutputLocation,
        suffix: &str,
    ) -> io::Result<FolderReservation> {
        Self::allocate(input_path, location, suffix, None, Self::reserve_folder_candidate)
    }

    pub fn reserve_staged_folder(
        input_path: &Path,
        location: &OutputLocation,
        suffix: &str,
    ) -> io::Result<StagedFolderReservation> {
        Self::allocate(input_path, location, suffix, None, Self::reserve_staged_folder_candidate)
    }

    pub fn create_destination(
        input_path: &Path,
        location: &OutputLocation,
        suffix: &str,
        extension: &str,
    ) -> io::Result<(OutputReservation, File)> {
        Self::allocate(input_path, location, suffix, Some(extension), |path| {
            Self::reserve_candidate(path)
        })
    }

    pub fn reserve_named_candidate(path: &Path) -> io::Result<Option<OutputReservation>> {
        Self::reserve_candidate(path).map(|reservation| reservation.map(|(reservation, _)| reservation))
    }

    pub fn reserve_single_output(
        chosen_path: Option<&Path>,
        input_path: &Path,
        location: &OutputLocation,
        suffix: &str,
        extension: &str,
    ) -> io::Result<OutputReservation> {
        Self::create_single_output(chosen_path, input_path, location, suffix, extension).map(|(reservation, _)| reservation)
    }

    pub fn create_single_output(
        chosen_path: Option<&Path>,
        input_path: &Path,
        location: &OutputLocation,
        suffix: &str,
        extension: &str,
    ) -> io::Result<(OutputReservation, File)> {
        let Some(chosen) = chosen_path else {
            return Self::create_destination(input_path, location, suffix, extension);
        };
        let directory = chosen.parent().filter(|parent| chosen.is_absolute() && parent.is_dir()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("{} is not in a folder that exists", chosen.display()))
        })?;
        let name = chosen.file_name().and_then(|value| value.to_str()).filter(|value| !value.is_empty()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "the chosen file name is empty")
        })?;
        let extension = extension.trim_start_matches('.');
        let stem = match chosen.extension().and_then(|value| value.to_str()) {
            Some(existing) if existing.eq_ignore_ascii_case(extension) => chosen.file_stem().and_then(|value| value.to_str()).unwrap_or(name),
            _ => name,
        };
        Self::allocate_in(directory, stem, "", Some(extension), Self::reserve_candidate)
    }

    fn allocate<T, F>(
        input_path: &Path,
        location: &OutputLocation,
        suffix: &str,
        extension: Option<&str>,
        claim: F,
    ) -> io::Result<T>
    where
        F: FnMut(&Path) -> io::Result<Option<T>>,
    {
        let directory = location.directory_for(input_path);
        let stem = input_path.file_stem().and_then(|value| value.to_str()).unwrap_or("");
        Self::allocate_in(&directory, stem, suffix, extension, claim)
    }

    fn allocate_in<T, F>(
        directory: &Path,
        stem: &str,
        suffix: &str,
        extension: Option<&str>,
        mut claim: F,
    ) -> io::Result<T>
    where
        F: FnMut(&Path) -> io::Result<Option<T>>,
    {
        let extension = extension.map(|value| format!(".{}", value.trim_start_matches('.'))).unwrap_or_default();
        let mut counter = 0usize;
        loop {
            let name = if counter == 0 {
                format!("{stem}{suffix}{extension}")
            } else {
                format!("{stem}{suffix}-{counter}{extension}")
            };
            if let Some(reservation) = claim(&directory.join(name))? {
                return Ok(reservation);
            }
            counter += 1;
        }
    }

    fn reserve_candidate(path: &Path) -> io::Result<Option<(OutputReservation, File)>> {
        if path.exists() {
            return Ok(None);
        }
        let claim_path = Self::claim_path(path);
        let claim = match Self::retry_while_busy(|| OpenOptions::new().write(true).create_new(true).open(&claim_path)) {
            Ok(claim) => claim,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(None),
            Err(error) => return Err(error),
        };
        drop(claim);
        if path.exists() {
            let _ = std::fs::remove_file(&claim_path);
            return Ok(None);
        }
        let (temporary_path, file) = match Self::create_temporary_file(path) {
            Ok(value) => value,
            Err(error) => {
                let _ = std::fs::remove_file(&claim_path);
                return Err(error);
            }
        };
        if path.exists() {
            drop(file);
            let _ = std::fs::remove_file(temporary_path);
            let _ = std::fs::remove_file(claim_path);
            return Ok(None);
        }
        Ok(Some((OutputReservation {
            path: path.to_path_buf(),
            temporary_path,
            claim_path,
        }, file)))
    }

    fn reserve_folder_candidate(path: &Path) -> io::Result<Option<FolderReservation>> {
        if path.exists() {
            return Ok(None);
        }
        let claim_path = Self::claim_path(path);
        match Self::retry_while_busy(|| OpenOptions::new().write(true).create_new(true).open(&claim_path)) {
            Ok(claim) => drop(claim),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(None),
            Err(error) => return Err(error),
        }
        let created = Self::retry_while_busy(|| std::fs::create_dir(path));
        let _ = std::fs::remove_file(&claim_path);
        match created {
            Ok(()) => Ok(Some(FolderReservation { path: path.to_path_buf(), published: false })),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Claims the name like a file output and keeps the claim until publish or
    /// drop, so parallel reservations skip it while its contents are still being
    /// written in a hidden staging folder in the same directory (same volume, so
    /// publishing is one rename).
    fn reserve_staged_folder_candidate(path: &Path) -> io::Result<Option<StagedFolderReservation>> {
        if std::fs::symlink_metadata(path).is_ok() {
            return Ok(None);
        }
        let claim_path = Self::claim_path(path);
        match Self::retry_while_busy(|| OpenOptions::new().write(true).create_new(true).open(&claim_path)) {
            Ok(claim) => drop(claim),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(None),
            Err(error) => return Err(error),
        }
        if std::fs::symlink_metadata(path).is_ok() {
            let _ = std::fs::remove_file(&claim_path);
            return Ok(None);
        }
        match Self::create_staging_folder(path) {
            Ok(staging_path) => Ok(Some(StagedFolderReservation {
                path: path.to_path_buf(),
                staging_path,
                claim_path,
                published: false,
            })),
            Err(error) => {
                let _ = std::fs::remove_file(&claim_path);
                Err(error)
            }
        }
    }

    fn create_staging_folder(path: &Path) -> io::Result<PathBuf> {
        let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("output");
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        for attempt in 0..100 {
            let staging_path = path.with_file_name(format!(".{name}.toolbox-staging-{timestamp}-{attempt}"));
            match Self::retry_while_busy(|| std::fs::create_dir(&staging_path)) {
                Ok(()) => return Ok(staging_path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "could not allocate a unique staging folder"))
    }

    /// Windows reports "access denied" while another reservation's claim file or
    /// abandoned folder is still being deleted, so a busy name is retried briefly
    /// before the error is treated as real.
    fn retry_while_busy<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
        let attempts = if cfg!(windows) { 50 } else { 1 };
        let mut result = operation();
        for _ in 1..attempts {
            match &result {
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    result = operation();
                }
                _ => break,
            }
        }
        result
    }

    fn claim_path(path: &Path) -> PathBuf {
        let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("output");
        path.with_file_name(format!(".{name}.toolbox-reservation"))
    }

    fn create_temporary_file(path: &Path) -> io::Result<(PathBuf, File)> {
        let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("output");
        let extension = path.extension().and_then(|value| value.to_str());
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        for attempt in 0..100 {
            let name = match extension {
                Some(extension) => format!(".{stem}.toolbox-tmp-{}-{attempt}.{extension}", timestamp),
                None => format!(".{stem}.toolbox-tmp-{}-{attempt}", timestamp),
            };
            let temporary_path = path.with_file_name(name);
            match OpenOptions::new().write(true).read(true).create_new(true).open(&temporary_path) {
                Ok(file) => return Ok((temporary_path, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "could not allocate a unique temporary output"))
    }
}


pub fn with_measured_sizes(outcome: JobOutcome) -> JobOutcome {
    let OutcomeState::Succeeded { output_paths } = &outcome.state else { return outcome };
    let [output] = output_paths.as_slice() else { return outcome };
    match (std::fs::metadata(&outcome.input_path), std::fs::metadata(output)) {
        (Ok(original), Ok(output)) => outcome.with_sizes(original.len(), output.len()),
        _ => outcome,
    }
}

/// A folder only this user can read, under the system temporary folder, removed with everything
/// in it when dropped. It holds intermediate files that must never sit beside the user's files.
pub(crate) struct TempDir(pub(crate) PathBuf);

impl TempDir {
    pub(crate) fn new() -> Result<Self, String> {
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();
        for _ in 0..1000 {
            let path = std::env::temp_dir().join(format!("toolbox-{}-{nanos}-{}", std::process::id(), NONCE.fetch_add(1, Ordering::Relaxed)));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("Could not reserve a private temporary folder.".into())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn note_renamed(outcome: JobOutcome, chosen_path: Option<&Path>) -> JobOutcome {
    let Some(chosen) = chosen_path else { return outcome };
    let renamed = match &outcome.state {
        OutcomeState::Succeeded { output_paths } => output_paths.first().filter(|published| published.as_path() != chosen).cloned(),
        _ => None,
    };
    let Some(published) = renamed else { return outcome };
    let file_name = |path: &Path| path.file_name().map(|value| value.to_string_lossy().into_owned()).unwrap_or_default();
    let (chosen_name, final_name) = (file_name(chosen), file_name(&published));
    let message = format!("Saved as {final_name} so the existing {chosen_name} was not replaced.");
    outcome.with_warning(OutcomeWarning { kind: WarningKind::RenamedToAvoidReplacing { chosen_name, final_name }, message })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_does_not_replace_a_competing_extracted_image_destination() {
        let root = std::env::temp_dir().join(format!("toolbox_output_reservation_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let input = root.join("source.txt");
        std::fs::write(&input, b"source").unwrap();

        let reservation = OutputNaming::reserve_destination(&input, &OutputLocation::CustomFolder(root.clone()), "-image-2-7", "jpg").unwrap();
        let destination = reservation.destination_path().to_path_buf();
        let temporary = reservation.path().to_path_buf();
        let claim = OutputNaming::claim_path(&destination);
        std::fs::write(reservation.path(), b"operation bytes").unwrap();
        std::fs::write(&destination, b"competing bytes").unwrap();

        assert!(reservation.publish().is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"competing bytes");
        assert!(!temporary.exists());
        assert!(!claim.exists());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn successful_publication_removes_private_temp_and_claim() {
        let root = std::env::temp_dir().join(format!("toolbox_output_success_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let input = root.join("source.txt");
        std::fs::write(&input, b"source").unwrap();

        let reservation = OutputNaming::reserve_destination(&input, &OutputLocation::CustomFolder(root.clone()), "-copy", "txt").unwrap();
        let destination = reservation.destination_path().to_path_buf();
        let temporary = reservation.path().to_path_buf();
        let claim = OutputNaming::claim_path(&destination);
        std::fs::write(reservation.path(), b"operation bytes").unwrap();

        assert_eq!(reservation.publish().unwrap(), destination);
        assert_eq!(std::fs::read(&destination).unwrap(), b"operation bytes");
        assert!(!temporary.exists());
        assert!(!claim.exists());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn failed_private_write_leaves_no_public_temp_or_claim_artifacts() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("toolbox_output_write_failure_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let input = root.join("source.txt");
        std::fs::write(&input, b"source").unwrap();

        let reservation = OutputNaming::reserve_destination(&input, &OutputLocation::CustomFolder(root.clone()), "-copy", "txt").unwrap();
        let destination = reservation.destination_path().to_path_buf();
        let temporary = reservation.path().to_path_buf();
        let claim = OutputNaming::claim_path(&destination);
        let mut readonly = std::fs::metadata(&temporary).unwrap().permissions();
        readonly.set_mode(0o400);
        std::fs::set_permissions(&temporary, readonly).unwrap();

        assert!(std::fs::write(&temporary, b"partial bytes").is_err());

        let mut writable = std::fs::metadata(&temporary).unwrap().permissions();
        writable.set_mode(0o600);
        std::fs::set_permissions(&temporary, writable).unwrap();
        drop(reservation);
        assert!(!destination.exists());
        assert!(!temporary.exists());
        assert!(!claim.exists());

        std::fs::remove_dir_all(root).unwrap();
    }

    fn folder_root(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("toolbox_folder_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let input = root.join("scan.pdf");
        std::fs::write(&input, b"source").unwrap();
        (root, input)
    }

    #[test]
    fn folder_outputs_number_like_files_and_never_reuse_a_previous_run() {
        let (root, input) = folder_root("numbering");
        let alongside = OutputLocation::AlongsideInput;

        let first = OutputNaming::reserve_folder(&input, &alongside, "-images").unwrap().publish();
        assert_eq!(first, root.join("scan-images"));
        assert!(first.is_dir());
        let second = OutputNaming::reserve_folder(&input, &alongside, "-images").unwrap().publish();
        assert_eq!(second, root.join("scan-images-1"));
        std::fs::write(root.join("scan-images-2"), b"a file, not a folder").unwrap();
        let third = OutputNaming::reserve_folder(&input, &alongside, "-images").unwrap().publish();
        assert_eq!(third, root.join("scan-images-3"));
        let file = OutputNaming::reserve_destination(&input, &alongside, "-images", "png").unwrap();
        assert_eq!(file.destination_path(), root.join("scan-images.png"));
        drop(file);

        assert_eq!(std::fs::read_dir(&first).unwrap().count(), 0);
        let names = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            names,
            ["scan-images", "scan-images-1", "scan-images-2", "scan-images-3", "scan.pdf"]
                .into_iter()
                .map(str::to_string)
                .collect()
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folder_outputs_honor_custom_folders_and_pending_claims() {
        let (root, input) = folder_root("custom");
        let custom = root.join("exports");
        std::fs::create_dir(&custom).unwrap();
        let location = OutputLocation::CustomFolder(custom.clone());
        std::fs::write(OutputNaming::claim_path(&custom.join("scan-frames")), b"").unwrap();

        let folder = OutputNaming::reserve_folder(&input, &location, "-frames").unwrap().publish();
        assert_eq!(folder, custom.join("scan-frames-1"));
        assert!(!OutputNaming::claim_path(&folder).exists());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unpublished_empty_folder_reservations_are_removed() {
        let (root, input) = folder_root("drop");
        let reservation = OutputNaming::reserve_folder(&input, &OutputLocation::AlongsideInput, "-split").unwrap();
        let path = reservation.path().to_path_buf();
        assert!(path.is_dir());
        drop(reservation);
        assert!(!path.exists());
        assert!(!OutputNaming::claim_path(&path).exists());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn busy_names_are_retried_only_on_windows_and_real_denials_still_fail() {
        let denied = || io::Error::new(io::ErrorKind::PermissionDenied, "Access is denied.");
        let mut calls = 0;
        let transient = OutputNaming::retry_while_busy(|| {
            calls += 1;
            if calls < 3 { Err(denied()) } else { Ok(calls) }
        });
        if cfg!(windows) {
            assert_eq!(transient.unwrap(), 3);
        } else {
            assert_eq!(transient.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(calls, 1);
        }

        let mut calls = 0;
        let persistent: io::Result<()> = OutputNaming::retry_while_busy(|| {
            calls += 1;
            Err(denied())
        });
        assert_eq!(persistent.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(calls, if cfg!(windows) { 50 } else { 1 });

        let mut calls = 0;
        let other: io::Result<()> = OutputNaming::retry_while_busy(|| {
            calls += 1;
            Err(io::Error::new(io::ErrorKind::NotFound, "missing"))
        });
        assert_eq!(other.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(calls, 1);
    }

    fn entry_names(directory: &Path) -> std::collections::BTreeSet<String> {
        std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect()
    }

    fn names(values: &[&str]) -> std::collections::BTreeSet<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn generated_bytes(seed: u8) -> Vec<u8> {
        (0..4096u32).map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed)).collect()
    }

    fn write_icon_set(reservation: &StagedFolderReservation) {
        let staging = reservation.path();
        std::fs::write(staging.join("Contents.json"), b"{\"images\":[]}").unwrap();
        std::fs::create_dir_all(staging.join("AppIcon.appiconset/nested")).unwrap();
        std::fs::write(staging.join("AppIcon.appiconset/icon-16.png"), generated_bytes(1)).unwrap();
        std::fs::write(staging.join("AppIcon.appiconset/nested/icon-32.png"), generated_bytes(2)).unwrap();
    }

    fn assert_icon_set(folder: &Path) {
        assert_eq!(std::fs::read(folder.join("Contents.json")).unwrap(), b"{\"images\":[]}");
        assert_eq!(std::fs::read(folder.join("AppIcon.appiconset/icon-16.png")).unwrap(), generated_bytes(1));
        assert_eq!(std::fs::read(folder.join("AppIcon.appiconset/nested/icon-32.png")).unwrap(), generated_bytes(2));
        assert_eq!(entry_names(folder), names(&["AppIcon.appiconset", "Contents.json"]));
        assert_eq!(entry_names(&folder.join("AppIcon.appiconset")), names(&["icon-16.png", "nested"]));
    }

    fn staged_root(name: &str) -> (PathBuf, PathBuf, Vec<u8>) {
        let root = std::env::temp_dir().join(format!("toolbox_staged_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let input = root.join("logo.png");
        let original = generated_bytes(7);
        std::fs::write(&input, &original).unwrap();
        (root, input, original)
    }

    #[test]
    fn staged_folder_publishes_nested_contents_in_one_step_beside_the_input() {
        let (root, input, original) = staged_root("publish");
        let reservation = OutputNaming::reserve_staged_folder(&input, &OutputLocation::AlongsideInput, "-icons").unwrap();
        let destination = reservation.destination_path().to_path_buf();
        let staging = reservation.path().to_path_buf();
        assert_eq!(destination, root.join("logo-icons"));
        assert_eq!(staging.parent(), Some(root.as_path()), "staging sits in the destination folder, on the same volume");
        assert!(staging.file_name().unwrap().to_string_lossy().starts_with('.'), "staging is hidden");

        write_icon_set(&reservation);
        assert!(!destination.exists(), "nothing is visible before publish");

        assert_eq!(reservation.publish().unwrap(), destination);
        assert_icon_set(&destination);
        assert!(!staging.exists());
        assert_eq!(entry_names(&root), names(&["logo-icons", "logo.png"]), "no staging or claim leftovers");
        assert_eq!(std::fs::read(&input).unwrap(), original);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_folder_publishes_into_a_chosen_folder_with_numbering() {
        let (root, input, original) = staged_root("custom");
        let custom = root.join("exports");
        std::fs::create_dir(&custom).unwrap();
        std::fs::create_dir(custom.join("logo-icons")).unwrap();
        let location = OutputLocation::CustomFolder(custom.clone());

        let reservation = OutputNaming::reserve_staged_folder(&input, &location, "-icons").unwrap();
        assert_eq!(reservation.path().parent(), Some(custom.as_path()));
        write_icon_set(&reservation);
        let published = reservation.publish().unwrap();

        assert_eq!(published, custom.join("logo-icons-1"));
        assert_icon_set(&published);
        assert_eq!(entry_names(&custom), names(&["logo-icons", "logo-icons-1"]));
        assert_eq!(entry_names(&custom.join("logo-icons")).len(), 0, "the existing folder is untouched");
        assert_eq!(entry_names(&root), names(&["exports", "logo.png"]));
        assert_eq!(std::fs::read(&input).unwrap(), original);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_folder_numbering_skips_existing_folders_files_and_pending_claims() {
        let (root, input, original) = staged_root("numbering");
        std::fs::create_dir(root.join("logo-icons")).unwrap();
        std::fs::write(root.join("logo-icons/keep.txt"), b"earlier run").unwrap();
        std::fs::write(root.join("logo-icons-1"), b"a file, not a folder").unwrap();
        let pending = OutputNaming::reserve_staged_folder(&input, &OutputLocation::AlongsideInput, "-icons").unwrap();
        assert_eq!(pending.destination_path(), root.join("logo-icons-2"));

        let reservation = OutputNaming::reserve_staged_folder(&input, &OutputLocation::AlongsideInput, "-icons").unwrap();
        assert_eq!(reservation.destination_path(), root.join("logo-icons-3"), "a pending staged reservation keeps its name");
        write_icon_set(&reservation);
        let published = reservation.publish().unwrap();
        assert_eq!(published, root.join("logo-icons-3"));
        drop(pending);

        assert_icon_set(&published);
        assert_eq!(std::fs::read(root.join("logo-icons/keep.txt")).unwrap(), b"earlier run");
        assert_eq!(std::fs::read(root.join("logo-icons-1")).unwrap(), b"a file, not a folder");
        assert_eq!(entry_names(&root), names(&["logo-icons", "logo-icons-1", "logo-icons-3", "logo.png"]));
        assert_eq!(std::fs::read(&input).unwrap(), original);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discarded_or_dropped_staged_folders_leave_nothing_behind() {
        for location_name in ["alongside", "custom"] {
            let (root, input, original) = staged_root(&format!("discard_{location_name}"));
            let location = if location_name == "custom" {
                std::fs::create_dir(root.join("exports")).unwrap();
                OutputLocation::CustomFolder(root.join("exports"))
            } else {
                OutputLocation::AlongsideInput
            };
            let directory = location.directory_for(&input).to_path_buf();
            let before = entry_names(&directory);

            let discarded = OutputNaming::reserve_staged_folder(&input, &location, "-icons").unwrap();
            write_icon_set(&discarded);
            let (destination, staging) = (discarded.destination_path().to_path_buf(), discarded.path().to_path_buf());
            discarded.discard();
            assert!(!destination.exists() && !staging.exists());
            assert_eq!(entry_names(&directory), before);

            let failed = || -> io::Result<PathBuf> {
                let reservation = OutputNaming::reserve_staged_folder(&input, &location, "-icons")?;
                write_icon_set(&reservation);
                Err(io::Error::other("one size could not be rendered"))?;
                reservation.publish()
            };
            assert!(failed().is_err());
            assert_eq!(entry_names(&directory), before, "a failure before publish leaves no folder and no staging");

            assert_eq!(std::fs::read(&input).unwrap(), original);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn staged_publish_never_replaces_a_concurrently_created_folder_or_file() {
        let (root, input, original) = staged_root("concurrent");
        let cases: [(&str, fn(&Path)); 3] = [
            ("empty folder", |path| std::fs::create_dir(path).unwrap()),
            ("folder with contents", |path| {
                std::fs::create_dir(path).unwrap();
                std::fs::write(path.join("theirs.txt"), b"theirs").unwrap();
            }),
            ("file", |path| std::fs::write(path, b"theirs").unwrap()),
        ];
        for (label, create_competitor) in cases {
            let reservation = OutputNaming::reserve_staged_folder(&input, &OutputLocation::AlongsideInput, "-icons").unwrap();
            let destination = reservation.destination_path().to_path_buf();
            write_icon_set(&reservation);
            // Another program ignores the claim and creates the same name before publish.
            create_competitor(&destination);
            let before = entry_names(&root);

            let error = reservation.publish().expect_err(label);
            assert_ne!(error.kind(), io::ErrorKind::NotFound, "{label}: {error}");
            if destination.is_dir() {
                let expected: &[&str] = if label == "empty folder" { &[] } else { &["theirs.txt"] };
                assert_eq!(entry_names(&destination), names(expected), "{label} kept its own contents");
            } else {
                assert_eq!(std::fs::read(&destination).unwrap(), b"theirs", "{label} kept its bytes");
            }
            let mut after = before.clone();
            after.retain(|name| !name.contains(".toolbox-"));
            assert_eq!(entry_names(&root), after, "{label}: staging and claim removed");

            if destination.is_dir() { std::fs::remove_dir_all(&destination).unwrap() } else { std::fs::remove_file(&destination).unwrap() }
        }
        assert_eq!(entry_names(&root), names(&["logo.png"]));
        assert_eq!(std::fs::read(&input).unwrap(), original);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parallel_staged_folders_publish_to_distinct_names() {
        let (root, input, _) = staged_root("parallel");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles = (0..8)
            .map(|_| {
                let (input, barrier) = (input.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    let reservation = OutputNaming::reserve_staged_folder(&input, &OutputLocation::AlongsideInput, "-icons").unwrap();
                    write_icon_set(&reservation);
                    reservation.publish().unwrap()
                })
            })
            .collect::<Vec<_>>();
        let folders = handles.into_iter().map(|handle| handle.join().unwrap()).collect::<std::collections::BTreeSet<_>>();

        let mut expected = vec![root.join("logo-icons")];
        expected.extend((1..8).map(|counter| root.join(format!("logo-icons-{counter}"))));
        assert_eq!(folders, expected.into_iter().collect());
        folders.iter().for_each(|folder| assert_icon_set(folder));
        assert!(entry_names(&root).iter().all(|name| !name.contains(".toolbox-")));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parallel_folder_reservations_never_share_a_folder() {
        let (root, input) = folder_root("parallel");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let handles = (0..16)
            .map(|_| {
                let input = input.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    OutputNaming::reserve_folder(&input, &OutputLocation::AlongsideInput, "-pages").unwrap().publish()
                })
            })
            .collect::<Vec<_>>();
        let folders = handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>();

        let unique = folders.iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), 16);
        let mut expected = vec![root.join("scan-pages")];
        expected.extend((1..16).map(|counter| root.join(format!("scan-pages-{counter}"))));
        assert_eq!(unique, expected.iter().collect());
        assert!(std::fs::read_dir(&root)
            .unwrap()
            .all(|entry| !entry.unwrap().file_name().to_string_lossy().ends_with(".toolbox-reservation")));

        std::fs::remove_dir_all(root).unwrap();
    }
}
