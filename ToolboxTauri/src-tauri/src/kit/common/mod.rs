pub mod batch_runner;

use std::path::PathBuf;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
pub use crate::kit::contracts::JobOutcome;
pub use crate::kit::contracts::OutputLocation;

pub struct OutputNaming;

static RESERVED_OUTPUTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

impl OutputNaming {
    pub fn get_destination(
        input_path: &PathBuf,
        location: &OutputLocation,
        suffix: &str,
        extension: &str,
    ) -> PathBuf {
        let directory = match location {
            OutputLocation::AlongsideInput => input_path.parent().unwrap_or_else(|| std::path::Path::new(".")),
            OutputLocation::CustomFolder(folder) => folder,
        };

        let stem = input_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let ext = if extension.starts_with('.') { extension.to_string() } else { format!(".{}", extension) };

        let mut final_name = format!("{}{}{}", stem, suffix, ext);
        let mut full_path = directory.join(&final_name);

        let mut counter = 1;
        let reserved = RESERVED_OUTPUTS.get_or_init(|| Mutex::new(HashSet::new()));
        let mut reserved = reserved.lock().expect("output reservation lock poisoned");
        while full_path.exists() || reserved.contains(&full_path) {
            final_name = format!("{}{}-{}{}", stem, suffix, counter, ext);
            full_path = directory.join(&final_name);
            counter += 1;
        }
        reserved.insert(full_path.clone());

        full_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_a_new_name_without_overwriting_an_existing_output() {
        let directory = std::env::temp_dir().join(format!("toolbox_output_naming_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let input = directory.join("source.png");
        let existing = directory.join("source-converted.jpg");
        std::fs::write(&input, b"source").unwrap();
        std::fs::write(&existing, b"existing").unwrap();

        let destination = OutputNaming::get_destination(&input, &OutputLocation::AlongsideInput, "-converted", "jpg");

        assert_eq!(destination, directory.join("source-converted-1.jpg"));
        assert_eq!(std::fs::read(&existing).unwrap(), b"existing");
        let _ = std::fs::remove_dir_all(directory);
    }
}
