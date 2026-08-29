use image::{DynamicImage, ImageFormat, ImageOutputFormat};
use std::path::PathBuf;
use crate::kit::common::{JobOutcome, OutputNaming, OutputLocation};

pub struct ImageProcessor;

pub struct Options {
    pub target_format: Option<ImageFormat>,
    pub quality: u8, // 1-100
    pub keep_smaller_original: bool,
    pub suffix: String,
}

impl ImageProcessor {
    pub fn run(input_path: PathBuf, options: Options) -> JobOutcome {
        let original_bytes = std::fs::metadata(&input_path).map(|m| m.len()).unwrap_or(0);

        let img = match image::open(&input_path) {
            Ok(i) => i,
            Err(e) => return JobOutcome {
                input_path,
                output_paths: vec![],
                detail: "".to_string(),
                failure: Some(format!("Decode failed: {}", e)),
            },
        };

        let format = options.target_format.unwrap_or(img.color().into()); // Simplification for example
        let extension = match format {
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Png => "png",
            ImageFormat::WebP => "webp",
            _ => "bin",
        };

        let output_path = OutputNaming::get_destination(
            &input_path,
            &OutputLocation::AlongsideInput,
            &options.suffix,
            extension,
            None,
        );

        // In a real impl, we'd use a specific encoder for quality.
        // Image crate's save() handles format automatically.
        if let Err(e) = img.save(&output_path) {
            return JobOutcome {
                input_path,
                output_paths: vec![],
                detail: "".to_string(),
                failure: Some(format!("Encode failed: {}", e)),
            };
        }

        let new_bytes = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);

        if options.keep_smaller_original && new_bytes >= original_bytes {
            let _ = std::fs::remove_file(&output_path);
            let fallback_path = OutputNaming::get_destination(
                &input_path,
                &OutputLocation::AlongsideInput,
                &options.suffix,
                input_path.extension().and_then(|e| e.to_str()).unwrap_or("bin"),
                None,
            );
            if let Err(e) = std::fs::copy(&input_path, &fallback_path) {
                return JobOutcome {
                    input_path,
                    output_paths: vec![],
                    detail: "".to_string(),
                    failure: Some(format!("Fallback copy failed: {}", e)),
                };
            }
            return JobOutcome {
                input_path,
                output_paths: vec![fallback_path],
                detail: "Kept original (compressed version was larger)".to_string(),
                failure: None,
            };
        }

        JobOutcome {
            input_path,
            output_paths: vec![output_path],
            detail: format!("Saved as {}", extension),
            failure: None,
        }
    }
}
