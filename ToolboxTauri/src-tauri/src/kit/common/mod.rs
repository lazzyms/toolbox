pub mod batch_runner;

use std::path::PathBuf;
pub use crate::kit::contracts::JobOutcome;
pub use crate::kit::contracts::OutputLocation;

pub struct OutputNaming;

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
        while full_path.exists() {
            final_name = format!("{}{}-{}{}", stem, suffix, counter, ext);
            full_path = directory.join(&final_name);
            counter += 1;
        }

        full_path
    }
}
