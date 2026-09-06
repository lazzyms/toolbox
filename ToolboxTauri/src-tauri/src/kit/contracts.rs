use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputLocation {
    AlongsideInput,
    CustomFolder(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub completed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfRequest {
    pub paths: Vec<PathBuf>,
    pub password: String,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordRequest {
    pub paths: Vec<PathBuf>,
    pub password: String,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressImagesRequest {
    pub paths: Vec<PathBuf>,
    pub quality: u8,
    #[serde(default)]
    pub lossless: bool,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertImagesRequest {
    pub paths: Vec<PathBuf>,
    pub format: String,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolError {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    InvalidInput,
    Unavailable,
    LimitExceeded,
    Processing,
}

impl ToolError {
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self { kind: ErrorKind::InvalidInput, message: message.into() }
    }

    pub fn processing(message: impl Into<String>) -> Self {
        Self { kind: ErrorKind::Processing, message: message.into() }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self { kind: ErrorKind::Unavailable, message: message.into() }
    }
}

impl Default for ToolError {
    fn default() -> Self {
        Self::processing("")
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobOutcome {
    pub input_path: PathBuf,
    pub output_paths: Vec<PathBuf>,
    pub detail: String,
    pub failure: Option<ToolError>,
}

impl JobOutcome {
    pub fn failure(input_path: PathBuf, error: ToolError) -> Self {
        Self { input_path, output_paths: Vec::new(), detail: String::new(), failure: Some(error) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_result_uses_the_frontend_field_names() {
        let outcome = JobOutcome {
            input_path: PathBuf::from("source.pdf"),
            output_paths: vec![PathBuf::from("source-unlocked.pdf")],
            detail: "PDF Unlocked".to_string(),
            failure: None,
        };
        let value = serde_json::to_value(outcome).unwrap();
        assert_eq!(value["inputPath"], "source.pdf");
        assert_eq!(value["outputPaths"][0], "source-unlocked.pdf");
    }

    #[test]
    fn failures_are_structured_for_frontend_handling() {
        let outcome = JobOutcome::failure(
            PathBuf::from("missing.pdf"),
            ToolError::invalid_input("Input file does not exist."),
        );
        let value = serde_json::to_value(outcome).unwrap();
        assert_eq!(value["failure"]["kind"], "invalidInput");
        assert_eq!(value["failure"]["message"], "Input file does not exist.");
    }

    #[test]
    fn request_and_progress_are_serializable_contracts() {
        let request = ToolRequest {
            paths: vec![PathBuf::from("source.pdf")],
            output_location: OutputLocation::AlongsideInput,
        };
        let progress = Progress { completed: 1, total: 2 };
        assert_eq!(serde_json::to_value(request).unwrap()["outputLocation"], "alongsideInput");
        assert_eq!(serde_json::to_value(progress).unwrap()["completed"], 1);
    }

    #[test]
    fn custom_folder_is_part_of_the_request_shape() {
        let request = ToolRequest {
            paths: vec![PathBuf::from("source.pdf")],
            output_location: OutputLocation::CustomFolder(PathBuf::from("exports")),
        };
        assert_eq!(serde_json::to_value(request).unwrap()["outputLocation"]["customFolder"], "exports");
    }
}
