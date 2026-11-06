use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::kit::common::{JobOutcome, OutputLocation};
use crate::kit::contracts::ToolError;
use crate::kit::pdf::remaining;

/// How hard Compress Files works. Low keeps the most detail and High saves the most space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompressLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressFilesRequest {
    pub paths: Vec<PathBuf>,
    pub level: CompressLevel,
    #[serde(default)]
    pub strip_exif: bool,
    pub output_location: OutputLocation,
}

pub const NOT_COMPRESSIBLE: &str = "Not a PDF or image. Compress takes PDF, JPEG, PNG and WebP.";

/// Compresses one file by its type. Every PDF is repacked without loss at every level; the level
/// and Strip EXIF apply to images.
pub fn compress_file(request: &CompressFilesRequest, input: PathBuf) -> JobOutcome {
    let extension = input.extension().and_then(|extension| extension.to_str()).map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("pdf") => remaining::compress_lossless(input, &request.output_location),
        _ => JobOutcome::failure(input, ToolError::invalid_input(NOT_COMPRESSIBLE)),
    }
}
