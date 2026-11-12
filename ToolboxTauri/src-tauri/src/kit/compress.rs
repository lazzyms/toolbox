use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::kit::common::{JobOutcome, OutputLocation};
use crate::kit::contracts::{OutcomePointer, SkipCode, SkipReason, ToolError};
use crate::kit::images;
use crate::kit::images::compress::png::PngMethod;
use crate::kit::images::compress::webp::WebpMethod;
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

    /// How PNGs are re-saved: without loss at Low and Medium, Medium trying harder, and cut to a
    /// 256-colour palette at High.
    pub fn png_method(self) -> PngMethod {
        match self {
            CompressLevel::Low => PngMethod::Lossless { effort: 2 },
            CompressLevel::Medium => PngMethod::Lossless { effort: 4 },
            CompressLevel::High => PngMethod::Palette { effort: 4 },
        }
    }

    /// How WebPs are re-saved: a lossless WebP stays lossless at Low and Medium, Medium also trying
    /// libwebp's slowest preset and keeping the smaller, and becomes lossy at High. Lossy WebPs, and lossless ones at High, are re-encoded at
    /// the same qualities as JPEGs.
    pub fn webp_method(self) -> WebpMethod {
        let lossless_levels: &[u8] = match self {
            CompressLevel::Low => &[6],
            CompressLevel::Medium => &[6, 9],
            CompressLevel::High => &[],
        };
        WebpMethod { lossless_levels, quality: self.jpeg_quality() }
    }
}

pub const NO_SAVINGS: &str = "No savings: already well compressed";
pub const NOT_COMPRESSIBLE: &str = "Not a PDF or image. Compress takes PDF, JPEG, PNG and WebP.";
/// The registry id of Convert Image Format, which a skipped HEIC points to.
pub const CONVERT_IMAGE_TOOL_ID: &str = "heic-convert";

/// A HEIC or HEIF photo is Skipped, not Failed: nothing is wrong with it, Compress just doesn't
/// take it. Its row points to Convert Image Format, and no file is written.
fn heic_skipped(input: PathBuf, format: &str) -> JobOutcome {
    let reason = SkipReason::new(SkipCode::UnsupportedFormat, format!("{format} isn't compressed. Convert it to JPEG, then compress the copy. No file was written."))
        .with_pointer(OutcomePointer::Tool { tool_id: CONVERT_IMAGE_TOOL_ID.into() });
    JobOutcome::skipped(input, reason)
}

/// Compresses one file by its type. Every PDF is repacked without loss; at Medium and High its
/// photographs are also recompressed and downsampled first. JPEGs are re-saved at the level's
/// quality and stay JPEG. PNGs stay PNG: repacked without loss at Low and Medium, cut to 256
/// colours at High. WebPs stay WebP: lossless ones stay lossless at Low and Medium, and lossy ones
/// are re-encoded at the level's quality. Strip EXIF applies to images only.
pub fn compress_file(request: &CompressFilesRequest, input: PathBuf) -> JobOutcome {
    let extension = input.extension().and_then(|extension| extension.to_str()).map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("pdf") => remaining::compress_pdf(input, &request.output_location, request.level.pdf_images()),
        Some("jpg" | "jpeg") => images::compress::compress_jpeg(input, &request.output_location, request.level.jpeg_quality(), request.strip_exif),
        Some("png") => images::compress::png::compress_png(input, &request.output_location, request.level.png_method(), request.strip_exif),
        Some("webp") => images::compress::webp::compress_webp(input, &request.output_location, request.level.webp_method(), request.strip_exif),
        Some("heic") => heic_skipped(input, "HEIC"),
        Some("heif") => heic_skipped(input, "HEIF"),
        _ => JobOutcome::failure(input, ToolError::invalid_input(NOT_COMPRESSIBLE)),
    }
}
