pub mod batch_runner;

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
pub use crate::kit::contracts::JobOutcome;
pub use crate::kit::contracts::OutputLocation;

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
        std::fs::hard_link(&self.temporary_path, &self.path)?;
        Ok(self.path.clone())
    }
}

impl Drop for OutputReservation {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.temporary_path);
        let _ = std::fs::remove_file(&self.claim_path);
    }
}

impl OutputNaming {
    pub fn reserve_destination(
        input_path: &Path,
        location: &OutputLocation,
        suffix: &str,
        extension: &str,
    ) -> io::Result<OutputReservation> {
        Self::allocate(input_path, location, suffix, extension, |path| {
            Self::reserve_candidate(path).map(|reservation| reservation.map(|(reservation, _)| reservation))
        })
    }

    pub fn create_destination(
        input_path: &Path,
        location: &OutputLocation,
        suffix: &str,
        extension: &str,
    ) -> io::Result<(OutputReservation, File)> {
        Self::allocate(input_path, location, suffix, extension, |path| {
            Self::reserve_candidate(path)
        })
    }

    pub fn reserve_named_candidate(path: &Path) -> io::Result<Option<OutputReservation>> {
        Self::reserve_candidate(path).map(|reservation| reservation.map(|(reservation, _)| reservation))
    }

    fn allocate<T, F>(
        input_path: &Path,
        location: &OutputLocation,
        suffix: &str,
        extension: &str,
        mut claim: F,
    ) -> io::Result<T>
    where
        F: FnMut(&Path) -> io::Result<Option<T>>,
    {
        let directory = match location {
            OutputLocation::AlongsideInput => input_path.parent().unwrap_or_else(|| Path::new(".")),
            OutputLocation::CustomFolder(folder) => folder,
        };
        let stem = input_path.file_stem().and_then(|value| value.to_str()).unwrap_or("");
        let extension = extension.trim_start_matches('.');
        let mut counter = 0usize;
        loop {
            let name = if counter == 0 {
                format!("{stem}{suffix}.{extension}")
            } else {
                format!("{stem}{suffix}-{counter}.{extension}")
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
        let claim = match OpenOptions::new().write(true).create_new(true).open(&claim_path) {
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
        std::fs::write(reservation.path(), b"operation bytes").unwrap();
        std::fs::write(&destination, b"competing bytes").unwrap();

        assert!(reservation.publish().is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"competing bytes");
        assert!(!temporary.exists());

        std::fs::remove_dir_all(root).unwrap();
    }
}
