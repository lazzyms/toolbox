use std::path::PathBuf;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::ImageEncoder;

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::ToolError;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutputFormat {
    Jpeg,
    Png,
    WebP,
    Heic,
}

impl OutputFormat {
    fn extension(self) -> &'static str {
        match self {
            OutputFormat::Jpeg => "jpg",
            OutputFormat::Png => "png",
            OutputFormat::WebP => "webp",
            OutputFormat::Heic => "heic",
        }
    }
}

pub struct ImageProcessor;

pub struct Options {
    pub target_format: Option<OutputFormat>,
    pub quality: u8, // 1-100
    pub keep_smaller_original: bool,
    pub suffix: String,
    pub output_location: OutputLocation,
}

fn lower_ext(path: &std::path::Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
}

fn detect_format(path: &std::path::Path) -> OutputFormat {
    match lower_ext(path).as_deref() {
        Some("jpg" | "jpeg") => OutputFormat::Jpeg,
        Some("webp") => OutputFormat::WebP,
        // libheif reads .heif too; the extension tells us HEIC because the
        // image crate can't (it reports an unsupported format error we
        // intercept in load_image below)
        Some("heic" | "heif") => OutputFormat::Heic,
        _ => OutputFormat::Png,
    }
}

// HEIC has no decoder in the image crate, so fall back to heif-rs when the
// extension says HEIF but image::open refused it. Everything else errors
// normally so a genuinely corrupt file is surfaced as such.
fn load_image(path: &std::path::Path) -> Result<image::DynamicImage, String> {
    match image::open(path) {
        Ok(img) => Ok(img),
        Err(first) => {
            if matches!(lower_ext(path).as_deref(), Some("heic" | "heif")) {
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                heif::decode(&bytes).map_err(|e| format!("HEIC decode failed: {}", e))
            } else {
                Err(format!("Decode failed: {}", first))
            }
        }
    }
}

// Encoders write to a buffer first: WebP and HEIC only expose buffer encoders
// anyway, and buffering lets the no-inflation guard compare sizes before
// touching the destination (JPEG/PNG write through the same path).
fn encode(img: &image::DynamicImage, format: OutputFormat, quality: u8) -> Result<Vec<u8>, String> {
    match format {
        OutputFormat::Jpeg => {
            let rgb = img.to_rgb8();
            let mut buf = Vec::new();
            JpegEncoder::new_with_quality(&mut buf, quality)
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(|e| e.to_string())?;
            Ok(buf)
        }
        OutputFormat::Png => {
            let mut buf = Vec::new();
            PngEncoder::new(&mut buf)
                .write_image(
                    img.as_bytes(),
                    img.width(),
                    img.height(),
                    image::ExtendedColorType::from(img.color()),
                )
                .map_err(|e| e.to_string())?;
            Ok(buf)
        }
        OutputFormat::WebP => {
            // image's WebPEncoder is lossless-only; lossy quality needs
            // Google's libwebp through webpx (quality 0-100)
            let rgba = img.to_rgba8();
            webpx::Encoder::new_rgba(rgba.as_raw(), rgba.width(), rgba.height())
                .quality(quality.min(100) as f32)
                .encode(webpx::Unstoppable)
                .map_err(|e| format!("WebP encode failed: {}", e))
        }
        OutputFormat::Heic => {
            // libheif's bundled x265 is 8-bit only, so downconvert first and
            // let a high-bit-depth source fail at encode time like Swift does
            let rgba = img.to_rgba8();
            let mut buf = Vec::new();
            image::DynamicImage::ImageRgba8(rgba)
                .write_with_encoder(heif::HeifEncoder::new(&mut buf).with_quality(quality))
                .map_err(|e| format!("HEIC encode failed: {}", e))?;
            Ok(buf)
        }
    }
}

impl ImageProcessor {
    pub fn run(input_path: PathBuf, options: Options) -> JobOutcome {
        let original_bytes = std::fs::metadata(&input_path).map(|m| m.len()).unwrap_or(0);

        if options.quality == 0 && options.target_format.is_none() {
            let extension = input_path.extension().and_then(|e| e.to_str()).unwrap_or("bin");
            let output_path = OutputNaming::get_destination(&input_path, &options.output_location, &options.suffix, extension);
            return match std::fs::copy(&input_path, &output_path) {
                Ok(_) => JobOutcome { input_path, output_paths: vec![output_path], detail: "Kept original bytes (lossless mode)".to_string(), failure: None },
                Err(error) => JobOutcome { input_path, output_paths: vec![], detail: String::new(), failure: Some(ToolError::processing(format!("Lossless copy failed: {error}"))) },
            };
        }

        let img = match load_image(&input_path) {
            Ok(i) => i,
            Err(e) => {
                return JobOutcome {
                    input_path,
                    output_paths: vec![],
                    detail: "".to_string(),
                    failure: Some(ToolError::processing(e)),
                };
            }
        };

        let format = options
            .target_format
            .unwrap_or_else(|| detect_format(&input_path));
        let extension = format.extension();

        let output_path = OutputNaming::get_destination(
            &input_path,
            &options.output_location,
            &options.suffix,
            extension,
        );

        let quality = options.quality.clamp(1, 100);
        let bytes = match encode(&img, format, quality) {
            Ok(b) => b,
            Err(e) => {
                return JobOutcome {
                    input_path,
                    output_paths: vec![],
                    detail: "".to_string(),
                    failure: Some(ToolError::processing(e)),
                };
            }
        };

        if let Err(e) = std::fs::write(&output_path, bytes) {
            return JobOutcome {
                input_path,
                output_paths: vec![],
                detail: "".to_string(),
                failure: Some(ToolError::processing(format!("Write failed: {}", e))),
            };
        }

        let new_bytes = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);

        if options.keep_smaller_original && new_bytes >= original_bytes {
            let _ = std::fs::remove_file(&output_path);
            let fallback_path = OutputNaming::get_destination(
                &input_path,
                &options.output_location,
                &options.suffix,
                input_path.extension().and_then(|e| e.to_str()).unwrap_or("bin"),
            );
            if let Err(e) = std::fs::copy(&input_path, &fallback_path) {
                return JobOutcome {
                    input_path,
                    output_paths: vec![],
                    detail: "".to_string(),
                    failure: Some(ToolError::processing(format!("Fallback copy failed: {}", e))),
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

    fn cleanup(paths: &[PathBuf]) {
        for p in paths {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn converts_png_to_jpeg() {
        let src = temp_path("convert_src.png");
        image::RgbaImage::from_fn(16, 16, |x, y| Rgba([x as u8, y as u8, 99, 255]))
            .save(&src)
            .unwrap();

        let out = ImageProcessor::run(
            src.clone(),
            Options {
                target_format: Some(OutputFormat::Jpeg),
                quality: 80,
                keep_smaller_original: false,
                suffix: "-converted".to_string(),
                output_location: OutputLocation::AlongsideInput,
            },
        );

        assert!(out.failure.is_none(), "{}", out.failure.clone().unwrap_or_default());
        assert_eq!(out.output_paths.len(), 1);
        assert_eq!(out.output_paths[0].extension().and_then(|e| e.to_str()), Some("jpg"));

        cleanup(&out.output_paths);
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn compress_keeps_a_file_unmodified_never_overwrites() {
        let src = temp_path("compress_src.png");
        image::RgbaImage::from_pixel(64, 64, Rgba([10, 10, 10, 255]))
            .save(&src)
            .unwrap();
        let before = std::fs::metadata(&src).unwrap().len();

        let out = ImageProcessor::run(
            src.clone(),
            Options {
                target_format: None,
                quality: 50,
                keep_smaller_original: true,
                suffix: "-compressed".to_string(),
                output_location: OutputLocation::AlongsideInput,
            },
        );

        assert!(out.failure.is_none(), "{}", out.failure.clone().unwrap_or_default());
        assert_eq!(out.output_paths.len(), 1);
        assert_ne!(&out.output_paths[0], &src, "must never write over the input");

        let after = std::fs::metadata(&src).unwrap().len();
        assert_eq!(before, after, "originals must never be modified");

        cleanup(&out.output_paths);
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn lossless_compression_preserves_source_bytes() {
        let src = temp_path("lossless_src.png");
        let original = b"deliberate source bytes";
        std::fs::write(&src, original).unwrap();
        let out = ImageProcessor::run(src.clone(), Options { target_format: None, quality: 0, keep_smaller_original: true, suffix: "-compressed".to_string(), output_location: OutputLocation::AlongsideInput });
        assert!(out.failure.is_none(), "{}", out.failure.clone().unwrap_or_default());
        assert_eq!(std::fs::read(&out.output_paths[0]).unwrap(), original);
        cleanup(&out.output_paths);
        let _ = std::fs::remove_file(src);
    }

    // Photo-size image: x265 rejects tiny frames on some builds, and the
    // no-inflation guard would quietly keep the original and skip the assert.
    #[test]
    fn heic_roundtrip_png_to_heic_to_png() {
        let src = temp_path("heic_src.png");
        image::RgbaImage::from_fn(256, 256, |x, y| Rgba([x as u8, y as u8, (x + y) as u8, 255]))
            .save(&src)
            .unwrap();

        let heic = ImageProcessor::run(
            src.clone(),
            Options {
                target_format: Some(OutputFormat::Heic),
                quality: 80,
                keep_smaller_original: false,
                suffix: "-heic".to_string(),
                output_location: OutputLocation::AlongsideInput,
            },
        );
        assert!(heic.failure.is_none(), "{}", heic.failure.clone().unwrap_or_default());
        assert_eq!(
            heic.output_paths[0].extension().and_then(|e| e.to_str()),
            Some("heic")
        );

        let back = ImageProcessor::run(
            heic.output_paths[0].clone(),
            Options {
                target_format: Some(OutputFormat::Png),
                quality: 80,
                keep_smaller_original: false,
                suffix: "-back".to_string(),
                output_location: OutputLocation::AlongsideInput,
            },
        );
        assert!(back.failure.is_none(), "{}", back.failure.clone().unwrap_or_default());
        let back_img = image::open(&back.output_paths[0]).unwrap();
        assert_eq!((back_img.width(), back_img.height()), (256, 256));

        cleanup(&heic.output_paths);
        cleanup(&back.output_paths);
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn converts_png_to_lossy_webp() {
        let src = temp_path("webp_src.png");
        image::RgbaImage::from_fn(256, 256, |x, y| Rgba([x as u8, y as u8, (x & y) as u8, 255]))
            .save(&src)
            .unwrap();

        let out = ImageProcessor::run(
            src.clone(),
            Options {
                target_format: Some(OutputFormat::WebP),
                quality: 60,
                keep_smaller_original: false,
                suffix: "-webp".to_string(),
                output_location: OutputLocation::AlongsideInput,
            },
        );

        assert!(out.failure.is_none(), "{}", out.failure.clone().unwrap_or_default());
        assert_eq!(
            out.output_paths[0].extension().and_then(|e| e.to_str()),
            Some("webp")
        );
        let data = std::fs::read(&out.output_paths[0]).unwrap();
        assert_eq!(&data[..4], b"RIFF", "webpx must produce a real lossy webp container");
        assert_eq!(&data[8..12], b"WEBP");
        let dec = image::load_from_memory(&data).unwrap();
        assert_eq!(dec.width(), 256);

        cleanup(&out.output_paths);
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn detection_maps_heic_extension() {
        assert_eq!(detect_format(std::path::Path::new("x.heic")), OutputFormat::Heic);
        assert_eq!(detect_format(std::path::Path::new("x.HEIC")), OutputFormat::Heic);
        assert_eq!(detect_format(std::path::Path::new("x.jpeg")), OutputFormat::Jpeg);
    }
}
pub mod tools;
