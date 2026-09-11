pub mod batch_runner;

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
pub use crate::kit::contracts::JobOutcome;
pub use crate::kit::contracts::OutputLocation;

pub struct OutputNaming;

pub struct OutputReservation {
    path: PathBuf,
    claim_path: Option<PathBuf>,
    committed: bool,
}

impl OutputReservation {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn commit(mut self) -> PathBuf {
        self.committed = true;
        self.release_claim();
        self.path.clone()
    }

    fn release_claim(&mut self) {
        if let Some(claim_path) = self.claim_path.take() {
            let _ = std::fs::remove_file(claim_path);
        }
    }
}

impl Drop for OutputReservation {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
        self.release_claim();
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
            let claim_path = Self::claim_path(path);
            match OpenOptions::new().write(true).create_new(true).open(&claim_path) {
                Ok(_) if !path.exists() => Ok(Some(OutputReservation {
                    path: path.to_path_buf(),
                    claim_path: Some(claim_path),
                    committed: false,
                })),
                Ok(_) => {
                    let _ = std::fs::remove_file(claim_path);
                    Ok(None)
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(None),
                Err(error) => Err(error),
            }
        })
    }

    pub fn create_destination(
        input_path: &Path,
        location: &OutputLocation,
        suffix: &str,
        extension: &str,
    ) -> io::Result<(OutputReservation, File)> {
        Self::allocate(input_path, location, suffix, extension, |path| {
            match OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(file) => Ok(Some((
                    OutputReservation {
                        path: path.to_path_buf(),
                        claim_path: None,
                        committed: false,
                    },
                    file,
                ))),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(None),
                Err(error) => Err(error),
            }
        })
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

    fn claim_path(path: &Path) -> PathBuf {
        let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("output");
        path.with_file_name(format!(".{name}.toolbox-reservation"))
    }
}
