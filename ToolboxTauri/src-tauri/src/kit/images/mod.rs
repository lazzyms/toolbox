use std::path::PathBuf;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{ImageEncoder, ImageFormat};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};

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
            Err(e) => {
                return JobOutcome {
                    input_path,
                    output_paths: vec![],
                    detail: "".to_string(),
                    failure: Some(format!("Decode failed: {}", e)),
                };
            }
        };

        let format = match options.target_format {
            Some(f) => f,
            None => ImageFormat::from_path(&input_path).unwrap_or(ImageFormat::Png),
        };

        let extension = match format {
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Png => "png",
            ImageFormat::WebP => "webp",
            _ => "png",
        };

        let output_path = OutputNaming::get_destination(
            &input_path,
            &OutputLocation::AlongsideInput,
            &options.suffix,
            extension,
            None,
        );

        let quality = options.quality.clamp(1, 100);
        let encode = || -> Result<(), String> {
            let file = std::fs::File::create(&output_path).map_err(|e| e.to_string())?;
            match format {
                // JPEG has no alpha channel; flatten so RGBA sources convert instead of failing
                ImageFormat::Jpeg => {
                    let rgb = img.to_rgb8();
                    JpegEncoder::new_with_quality(&file, quality)
                        .write_image(
                            rgb.as_raw(),
                            rgb.width(),
                            rgb.height(),
                            image::ExtendedColorType::Rgb8,
                        )
                        .map_err(|e| e.to_string())
                }
                ImageFormat::WebP => {
                    WebPEncoder::new_lossless(&file)
                        .write_image(
                            img.as_bytes(),
                            img.width(),
                            img.height(),
                            image::ExtendedColorType::from(img.color()),
                        )
                        .map_err(|e| e.to_string())
                }
                ImageFormat::Png => PngEncoder::new(&file)
                    .write_image(
                        img.as_bytes(),
                        img.width(),
                        img.height(),
                        image::ExtendedColorType::from(img.color()),
                    )
                    .map_err(|e| e.to_string()),
                _ => img
                    .save_with_format(&output_path, ImageFormat::Png)
                    .map_err(|e| e.to_string()),
            }
        };

        if let Err(e) = encode() {
            let _ = std::fs::remove_file(&output_path);
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

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("toolbox_{}_{}", std::process::id(), name))
    }

    #[test]
    fn converts_png_to_jpeg() {
        let src = temp_path("convert_src.png");
        let img = image::RgbaImage::from_fn(16, 16, |x, y| Rgba([x as u8, y as u8, 99, 255]));
        img.save(&src).unwrap();

        let out = ImageProcessor::run(
            src.clone(),
            Options {
                target_format: Some(ImageFormat::Jpeg),
                quality: 80,
                keep_smaller_original: false,
                suffix: "-converted".to_string(),
            },
        );

        assert!(out.failure.is_none(), "{}", out.failure.clone().unwrap_or_default());
        assert_eq!(out.output_paths.len(), 1);
        let ext = out.output_paths[0].extension().and_then(|e| e.to_str());
        assert_eq!(ext, Some("jpg"));

        for p in &out.output_paths {
            let _ = std::fs::remove_file(p);
        }
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn compress_keeps_a_file_unmodified_never_overwrites() {
        let src = temp_path("compress_src.png");
        let img = image::RgbaImage::from_pixel(64, 64, Rgba([10, 10, 10, 255]));
        img.save(&src).unwrap();
        let before = std::fs::metadata(&src).unwrap().len();

        let out = ImageProcessor::run(
            src.clone(),
            Options {
                target_format: None,
                quality: 50,
                keep_smaller_original: true,
                suffix: "-compressed".to_string(),
            },
        );

        assert!(out.failure.is_none(), "{}", out.failure.clone().unwrap_or_default());
        assert_eq!(out.output_paths.len(), 1);
        assert_ne!(&out.output_paths[0], &src, "must never write over the input");

        let after = std::fs::metadata(&src).unwrap().len();
        assert_eq!(before, after, "originals must never be modified");

        for p in &out.output_paths {
            let _ = std::fs::remove_file(p);
        }
        let _ = std::fs::remove_file(&src);
    }
}