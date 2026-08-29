pub mod batch_runner;

use serde::{Serialize, Deserialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JobOutcome {
    pub input_path: PathBuf,
    pub output_paths: Vec<PathBuf>,
    pub detail: String,
    pub failure: Option<String>,
}

impl JobOutcome {
    pub fn succeeded(&self) -> bool {
        self.failure.is_none()
    }

    pub fn output(&self) -> Option<&PathBuf> {
        self.output_paths.first()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum OutputLocation {
    AlongsideInput,
    CustomFolder,
}

pub struct OutputNaming;

impl OutputNaming {
    pub fn get_destination(
        input_path: &PathBuf,
        location: &OutputLocation,
        suffix: &str,
        extension: &str,
        custom_folder: Option<&PathBuf>,
    ) -> PathBuf {
        let directory = match location {
            OutputLocation::AlongsideInput => input_path.parent().unwrap_or_else(|| std::path::Path::new(".")),
            OutputLocation::CustomFolder => custom_folder.expect("Custom folder must be provided"),
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
