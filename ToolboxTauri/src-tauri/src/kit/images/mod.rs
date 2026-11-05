use std::path::PathBuf;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{DynamicImage, ImageEncoder};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::{OutcomeWarning, SkipCode, SkipReason, ToolError, WarningKind};

pub(crate) mod source;
pub(crate) use source::decode;
pub(crate) use source::{DecodedImage, ImageFamily};

pub(crate) const DEFAULT_QUALITY: u8 = 90;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutputFormat {
    Jpeg,
    Png,
    WebP,
    Heic,
}

impl OutputFormat {
    pub(crate) fn extension(self) -> &'static str {
        match self {
            OutputFormat::Jpeg => "jpg",
            OutputFormat::Png => "png",
            OutputFormat::WebP => "webp",
            OutputFormat::Heic => "heic",
        }
    }

    fn name(self) -> &'static str {
        match self {
            OutputFormat::Jpeg => "JPEG",
            OutputFormat::Png => "PNG",
            OutputFormat::WebP => "WebP",
            OutputFormat::Heic => "HEIC",
        }
    }
}

// HEIC/HEIF and JPG/JPEG each name one format, so a file already in the
// target family makes no output.
fn already_in_format(path: &std::path::Path, format: OutputFormat) -> Option<String> {
    let aliases = match (ImageFamily::of(path), format) {
        (ImageFamily::Jpeg, OutputFormat::Jpeg) => "JPG and JPEG",
        (ImageFamily::Heif, OutputFormat::Heic) => "HEIC and HEIF",
        (ImageFamily::Png, OutputFormat::Png) | (ImageFamily::WebP, OutputFormat::WebP) => "",
        _ => return None,
    };
    let name = format.name();
    if lower_ext(path).as_deref() == Some(format.extension()) {
        Some(format!("Already {name}, so no file was made."))
    } else {
        Some(format!("Already {name}. {aliases} count as one format."))
    }
}

pub struct ImageProcessor;

pub struct Options {
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

// Encoders write to a buffer first: WebP and HEIC only expose buffer encoders
// anyway, and buffering lets the no-inflation guard compare sizes before
// touching the destination (JPEG/PNG write through the same path).
pub(crate) fn encode(img: &image::DynamicImage, format: OutputFormat, quality: u8) -> Result<Vec<u8>, String> {
    match format {
        OutputFormat::Jpeg => {
            let rgb = if img.color().has_alpha() { flatten_onto_white(img) } else { img.to_rgb8() };
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

pub(crate) const TRANSPARENCY_FLATTENED_MESSAGE: &str = "Transparent areas were flattened onto white, because JPEG has none.";

pub(crate) fn has_transparency(img: &DynamicImage) -> bool {
    match img {
        DynamicImage::ImageRgba8(rgba) => rgba.pixels().any(|pixel| pixel.0[3] < u8::MAX),
        DynamicImage::ImageLumaA8(luma) => luma.pixels().any(|pixel| pixel.0[1] < u8::MAX),
        other if other.color().has_alpha() => other.to_rgba16().pixels().any(|pixel| pixel.0[3] < u16::MAX),
        _ => false,
    }
}

// JPEG has no alpha channel, so transparent pixels are composited over white
// instead of keeping whatever colour sat under them.
pub(crate) fn flatten_onto_white(img: &DynamicImage) -> image::RgbImage {
    let rgba = img.to_rgba8();
    image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [red, green, blue, alpha] = rgba.get_pixel(x, y).0;
        let over_white = |channel: u8| ((u32::from(channel) * u32::from(alpha) + 255 * u32::from(u8::MAX - alpha) + 127) / 255) as u8;
        image::Rgb([over_white(red), over_white(green), over_white(blue)])
    })
}

// Every Convert target is written at 8 bits per channel; alpha survives the
// reduction so the per-target alpha rule still applies afterwards.
fn reduce_to_8_bit(img: DynamicImage) -> DynamicImage {
    match img {
        DynamicImage::ImageLuma16(_) => DynamicImage::ImageLuma8(img.to_luma8()),
        DynamicImage::ImageLumaA16(_) => DynamicImage::ImageLumaA8(img.to_luma_alpha8()),
        DynamicImage::ImageRgb16(_) | DynamicImage::ImageRgb32F(_) => DynamicImage::ImageRgb8(img.to_rgb8()),
        DynamicImage::ImageRgba16(_) | DynamicImage::ImageRgba32F(_) => DynamicImage::ImageRgba8(img.to_rgba8()),
        other => other,
    }
}

// Lossless WebP keeps every pixel, including the colour under fully
// transparent areas, so a PNG or TIFF source decodes back unchanged.
fn encode_lossless_webp(img: &DynamicImage) -> Result<Vec<u8>, String> {
    let rgba = img.to_rgba8();
    webpx::Encoder::new_rgba(rgba.as_raw(), rgba.width(), rgba.height())
        .lossless(true)
        .exact(true)
        .encode(webpx::Unstoppable)
        .map_err(|e| format!("WebP encode failed: {}", e))
}

impl ImageProcessor {
    pub fn convert(input_path: PathBuf, format: OutputFormat, output_location: OutputLocation) -> JobOutcome {
        if let Some(message) = already_in_format(&input_path, format) {
            return JobOutcome::skipped(input_path, SkipReason::new(SkipCode::AlreadyInTargetFormat, message));
        }
        match decode(&input_path) {
            Ok(decoded) => Self::convert_decoded(input_path, decoded, format, &output_location),
            Err(error) => JobOutcome::failure(input_path, ToolError::processing(error)),
        }
    }

    /// Convert fidelity rules: orientation is baked into the pixels once, deep
    /// sources are reduced to 8 bits with a warning, JPEG flattens alpha onto
    /// white with a warning, and WebP follows the source (lossless from PNG,
    /// TIFF and other lossless families, quality 90 from JPEG and HEIC).
    pub(crate) fn convert_decoded(input_path: PathBuf, decoded: DecodedImage, format: OutputFormat, output_location: &OutputLocation) -> JobOutcome {
        let lossless_source = !matches!(decoded.family, ImageFamily::Jpeg | ImageFamily::Heif);
        let bits = decoded.bits_per_channel;
        let orientation = decoded.orientation;
        let mut img = decoded.into_first_page();
        if let Some(orientation) = orientation {
            img.apply_orientation(orientation);
        }
        let mut warnings = Vec::new();
        if bits > 8 {
            img = reduce_to_8_bit(img);
            warnings.push(OutcomeWarning { kind: WarningKind::PrecisionReduced, message: format!("Reduced from {bits}-bit to 8-bit sRGB.") });
        }
        if format == OutputFormat::Jpeg && has_transparency(&img) {
            warnings.push(OutcomeWarning { kind: WarningKind::TransparencyFlattened, message: TRANSPARENCY_FLATTENED_MESSAGE.to_string() });
        }

        let extension = format.extension();
        let output_path = match OutputNaming::reserve_destination(&input_path, output_location, "-converted", extension) {
            Ok(output) => output,
            Err(error) => return JobOutcome::failure(input_path, ToolError::processing(format!("Could not reserve output: {error}"))),
        };
        let encoded = if format == OutputFormat::WebP && lossless_source { encode_lossless_webp(&img) } else { encode(&img, format, DEFAULT_QUALITY) };
        let bytes = match encoded {
            Ok(bytes) => bytes,
            Err(error) => return JobOutcome::failure(input_path, ToolError::processing(error)),
        };
        if let Err(error) = std::fs::write(output_path.path(), bytes) {
            return JobOutcome::failure(input_path, ToolError::processing(format!("Write failed: {error}")));
        }
        if let Err(error) = validate_encoded_output(output_path.path(), img.width(), img.height()) {
            return JobOutcome::failure(input_path, ToolError::processing(error));
        }
        match output_path.publish() {
            Ok(path) => warnings.into_iter().fold(JobOutcome::succeeded(input_path, vec![path], format!("Saved as {extension}")), JobOutcome::with_warning),
            Err(error) => JobOutcome::failure(input_path, ToolError::processing(format!("Could not publish output: {error}"))),
        }
    }

    pub fn run(input_path: PathBuf, options: Options) -> JobOutcome {
        let original_bytes = std::fs::metadata(&input_path).map(|m| m.len()).unwrap_or(0);

        if options.quality == 0 {
            let extension = input_path.extension().and_then(|e| e.to_str()).unwrap_or("bin");
            let output_path = match OutputNaming::reserve_destination(&input_path, &options.output_location, &options.suffix, extension) {
                Ok(output) => output,
                Err(error) => return JobOutcome::failure(input_path, ToolError::processing(format!("Could not reserve output: {error}"))),
            };
            return match std::fs::copy(&input_path, output_path.path()) {
                Ok(_) => match output_path.publish() {
                    Ok(path) => JobOutcome::succeeded(input_path, vec![path], "Kept original bytes (lossless mode)"),
                    Err(error) => JobOutcome::failure(input_path, ToolError::processing(format!("Could not publish output: {error}"))),
                },
                Err(error) => JobOutcome::failure(input_path, ToolError::processing(format!("Lossless copy failed: {error}"))),
            };
        }

        let decoded = match decode(&input_path) {
            Ok(decoded) => decoded,
            Err(e) => {
                return JobOutcome::failure(input_path, ToolError::processing(e));
            }
        };

        let format = decoded.family.output_format();
        let img = decoded.into_first_page();
        let extension = format.extension();

        let output_path = match OutputNaming::reserve_destination(
            &input_path,
            &options.output_location,
            &options.suffix,
            extension,
        ) {
            Ok(output) => output,
            Err(error) => return JobOutcome::failure(input_path, ToolError::processing(format!("Could not reserve output: {error}"))),
        };

        let quality = options.quality.clamp(1, 100);
        let bytes = match encode(&img, format, quality) {
            Ok(b) => b,
            Err(e) => {
                return JobOutcome::failure(input_path, ToolError::processing(e));
            }
        };

        if let Err(e) = std::fs::write(output_path.path(), bytes) {
            return JobOutcome::failure(input_path, ToolError::processing(format!("Write failed: {}", e)));
        }

        if let Err(error) = validate_encoded_output(output_path.path(), img.width(), img.height()) {
            return JobOutcome::failure(input_path, ToolError::processing(error));
        }

        let new_bytes = std::fs::metadata(output_path.path()).map(|m| m.len()).unwrap_or(0);

        if options.keep_smaller_original && new_bytes >= original_bytes {
            drop(output_path);
            let fallback_path = match OutputNaming::reserve_destination(
                &input_path,
                &options.output_location,
                &options.suffix,
                input_path.extension().and_then(|e| e.to_str()).unwrap_or("bin"),
            ) {
                Ok(output) => output,
                Err(error) => return JobOutcome::failure(input_path, ToolError::processing(format!("Could not reserve fallback output: {error}"))),
            };
            if let Err(e) = std::fs::copy(&input_path, fallback_path.path()) {
                return JobOutcome::failure(input_path, ToolError::processing(format!("Fallback copy failed: {}", e)));
            }
            return match fallback_path.publish() {
                Ok(path) => JobOutcome::succeeded(input_path, vec![path], "Kept original (compressed version was larger)"),
                Err(error) => JobOutcome::failure(input_path, ToolError::processing(format!("Could not publish fallback output: {error}"))),
            };
        }

        match output_path.publish() {
            Ok(path) => JobOutcome::succeeded(input_path, vec![path], format!("Saved as {}", extension)),
            Err(error) => JobOutcome::failure(input_path, ToolError::processing(format!("Could not publish output: {error}"))),
        }
    }
}

fn validate_encoded_output(path: &std::path::Path, width: u32, height: u32) -> Result<(), String> {
    if !path.is_file() { return Err("Encoder did not produce an output container.".to_string()); }
    let output = decode(path).map_err(|error| format!("Encoded output could not be decoded: {error}"))?.into_first_page();
    if output.width() != width || output.height() != height { return Err("Encoded output dimensions do not match the source.".to_string()); }
    Ok(())
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

        let out = ImageProcessor::convert(src.clone(), OutputFormat::Jpeg, OutputLocation::AlongsideInput);

        assert!(out.error().is_none(), "{}", out.error().cloned().unwrap_or_default());
        assert_eq!(out.output_paths().len(), 1);
        assert_eq!(out.output_paths()[0].extension().and_then(|e| e.to_str()), Some("jpg"));
        assert!(out.warnings.is_empty(), "an opaque source has nothing to flatten");

        cleanup(&out.output_paths());
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
                quality: 50,
                keep_smaller_original: true,
                suffix: "-compressed".to_string(),
                output_location: OutputLocation::AlongsideInput,
            },
        );

        assert!(out.error().is_none(), "{}", out.error().cloned().unwrap_or_default());
        assert_eq!(out.output_paths().len(), 1);
        assert_ne!(&out.output_paths()[0], &src, "must never write over the input");

        let after = std::fs::metadata(&src).unwrap().len();
        assert_eq!(before, after, "originals must never be modified");

        cleanup(&out.output_paths());
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn lossless_compression_preserves_source_bytes() {
        let src = temp_path("lossless_src.png");
        let original = b"deliberate source bytes";
        std::fs::write(&src, original).unwrap();
        let out = ImageProcessor::run(src.clone(), Options { quality: 0, keep_smaller_original: true, suffix: "-compressed".to_string(), output_location: OutputLocation::AlongsideInput });
        assert!(out.error().is_none(), "{}", out.error().cloned().unwrap_or_default());
        assert_eq!(std::fs::read(&out.output_paths()[0]).unwrap(), original);
        cleanup(&out.output_paths());
        let _ = std::fs::remove_file(src);
    }

    // Photo-size image: x265 rejects tiny frames on some builds.
    #[test]
    fn heic_roundtrip_png_to_heic_to_png() {
        let src = temp_path("heic_src.png");
        image::RgbaImage::from_fn(256, 256, |x, y| Rgba([x as u8, y as u8, (x + y) as u8, 255]))
            .save(&src)
            .unwrap();

        let heic = ImageProcessor::convert(src.clone(), OutputFormat::Heic, OutputLocation::AlongsideInput);
        assert!(heic.error().is_none(), "{}", heic.error().cloned().unwrap_or_default());
        assert_eq!(heic.output_paths()[0].extension().and_then(|e| e.to_str()), Some("heic"));

        let back = ImageProcessor::convert(heic.output_paths()[0].clone(), OutputFormat::Png, OutputLocation::AlongsideInput);
        assert!(back.error().is_none(), "{}", back.error().cloned().unwrap_or_default());
        let back_img = image::open(&back.output_paths()[0]).unwrap();
        assert_eq!((back_img.width(), back_img.height()), (256, 256));

        cleanup(&heic.output_paths());
        cleanup(&back.output_paths());
        let _ = std::fs::remove_file(&src);
    }

    // libheif's bundled x265 only writes 8-bit HEIC, so a 10-bit HEIC decode
    // is described directly to exercise the precision rule.
    #[test]
    fn a_ten_bit_heic_is_reduced_to_eight_bits_with_a_precision_warning() {
        let src = temp_path("deep_floorplan.heic");
        std::fs::write(&src, b"stand-in for a 10-bit HEIC").unwrap();
        let deep = image::ImageBuffer::from_fn(32, 24, |x, y| image::Rgb([(x * 2000) as u16, (y * 2700) as u16, 40_000u16]));
        let ten_bit_heic = || DecodedImage {
            pages: vec![DynamicImage::ImageRgb16(deep.clone())],
            family: ImageFamily::Heif,
            bits_per_channel: 10,
            has_alpha: false,
            orientation: None,
            baseline_jpeg: None,
        };
        for format in [OutputFormat::Png, OutputFormat::Jpeg] {
            let out = ImageProcessor::convert_decoded(src.clone(), ten_bit_heic(), format, &OutputLocation::AlongsideInput);
            assert!(out.error().is_none(), "{}", out.error().cloned().unwrap_or_default());
            assert_eq!(out.warnings.len(), 1);
            assert_eq!(out.warnings[0].kind, WarningKind::PrecisionReduced);
            assert_eq!(out.warnings[0].message, "Reduced from 10-bit to 8-bit sRGB.");
            let written = image::open(&out.output_paths()[0]).unwrap();
            assert_eq!(written.color(), image::ColorType::Rgb8, "{format:?} must be written at 8 bits per channel");
            if format == OutputFormat::Png {
                assert_eq!(written.to_rgb8(), DynamicImage::ImageRgb16(deep.clone()).to_rgb8());
            }
            cleanup(&out.output_paths());
        }
        assert_eq!(std::fs::read(&src).unwrap(), b"stand-in for a 10-bit HEIC");
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn detection_maps_heic_extension() {
        assert_eq!(source::ImageFamily::of(std::path::Path::new("x.heic")).output_format(), OutputFormat::Heic);
        assert_eq!(source::ImageFamily::of(std::path::Path::new("x.HEIC")).output_format(), OutputFormat::Heic);
        assert_eq!(source::ImageFamily::of(std::path::Path::new("x.jpeg")).output_format(), OutputFormat::Jpeg);
    }
}
pub mod tools;
