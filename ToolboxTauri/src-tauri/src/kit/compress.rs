use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::kit::common::{JobOutcome, OutputLocation};
use crate::kit::contracts::ToolError;
use crate::kit::images;
use crate::kit::pdf::{recompress, remaining};

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

impl CompressLevel {
    /// The JPEG quality and maximum resolution PDF images get at this level; None at Low, which
    /// leaves every PDF image untouched.
    pub fn pdf_images(self) -> Option<recompress::ImageSettings> {
        match self {
            CompressLevel::Low => None,
            CompressLevel::Medium => Some(recompress::MEDIUM),
            CompressLevel::High => Some(recompress::HIGH),
        }
    }

    /// The encoder quality JPEGs are re-saved at: high, medium and low quality.
    pub fn jpeg_quality(self) -> u8 {
        match self {
            CompressLevel::Low => 85,
            CompressLevel::Medium => 75,
            CompressLevel::High => 50,
        }
    }
}

pub const NO_SAVINGS: &str = "No savings: already well compressed";
pub const NOT_COMPRESSIBLE: &str = "Not a PDF or image. Compress takes PDF, JPEG, PNG and WebP.";

/// Compresses one file by its type. Every PDF is repacked without loss; at Medium and High its
/// photographs are also recompressed and downsampled first. JPEGs are re-saved at the level's
/// quality and stay JPEG. Strip EXIF applies to images only.
pub fn compress_file(request: &CompressFilesRequest, input: PathBuf) -> JobOutcome {
    let extension = input.extension().and_then(|extension| extension.to_str()).map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("pdf") => remaining::compress_pdf(input, &request.output_location, request.level.pdf_images()),
        Some("jpg" | "jpeg") => images::compress::compress_jpeg(input, &request.output_location, request.level.jpeg_quality(), request.strip_exif),
        _ => JobOutcome::failure(input, ToolError::invalid_input(NOT_COMPRESSIBLE)),
    }
}
