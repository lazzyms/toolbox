pub mod batch_runner;
pub mod limits;
#[cfg(all(test, unix))]
pub mod fake_helper;

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
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
