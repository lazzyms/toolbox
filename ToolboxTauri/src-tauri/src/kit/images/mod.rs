use std::path::PathBuf;
use std::fs::File;
use std::io::BufReader;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{DynamicImage, ImageEncoder};
use image::AnimationDecoder;

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::{OutcomeWarning, SkipCode, SkipReason, TiffCompression, ToolError, WarningKind};

pub(crate) mod source;
pub(crate) use source::decode;
pub(crate) use source::{DecodedImage, ImageFamily, TiffPages};

pub(crate) const DEFAULT_QUALITY: u8 = 90;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutputFormat {
    Jpeg,
    Png,
    WebP,
    Heic,
    Tiff(TiffCompression),
}

impl OutputFormat {
    pub(crate) fn extension(self) -> &'static str {
        match self {
            OutputFormat::Jpeg => "jpg",
            OutputFormat::Png => "png",
            OutputFormat::WebP => "webp",
            OutputFormat::Heic => "heic",
            OutputFormat::Tiff(_) => "tiff",
        }
    }

    fn name(self) -> &'static str {
        match self {
            OutputFormat::Jpeg => "JPEG",
            OutputFormat::Png => "PNG",
            OutputFormat::WebP => "WebP",
            OutputFormat::Heic => "HEIC",
            OutputFormat::Tiff(_) => "TIFF",
        }
    }
}

// HEIC/HEIF, JPG/JPEG and TIF/TIFF each name one format, so a file already in the
// target family makes no output.
fn already_in_format(path: &std::path::Path, format: OutputFormat) -> Option<String> {
    let aliases = match (ImageFamily::of(path), format) {
        (ImageFamily::Jpeg, OutputFormat::Jpeg) => "JPG and JPEG",
        (ImageFamily::Heif, OutputFormat::Heic) => "HEIC and HEIF",
        (ImageFamily::Tiff, OutputFormat::Tiff(_)) => "TIF and TIFF",
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

fn lower_ext(path: &std::path::Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
}

/// A source decoded the way a viewer shows it, plus the HEIF bit depth when it
/// is above 8 so callers that write 8-bit output can say so.
pub(crate) struct UprightImage {
    pub image: image::DynamicImage,
    pub high_bit_depth: Option<u8>,
    /// The family the source was decoded as, which picks the format an edit writes back in.
    pub family: source::ImageFamily,
    /// Whether a non-neutral orientation was baked into the pixels.
    pub oriented: bool,
}

/// Decodes a source upright, applying its orientation exactly once. libheif
/// already applies HEIF irot/imir transforms while decoding, so `decode`
/// reports no orientation for HEIC/HEIF sources and they are never rotated
/// again here; every other format applies its EXIF orientation tag after
/// decoding. Only HEIF reports a bit depth above 8: the image crate's
/// decoders hand back 8-bit samples for the formats the 8-bit tools accept.
pub(crate) fn load_upright_image(path: &std::path::Path) -> Result<UprightImage, String> {
    let decoded = decode(path)?;
    let high_bit_depth = (decoded.family == source::ImageFamily::Heif && decoded.bits_per_channel > 8).then_some(decoded.bits_per_channel);
    let orientation = decoded.orientation;
    let family = decoded.family;
    let mut image = decoded.into_first_page();
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    Ok(UprightImage { image, high_bit_depth, family, oriented: orientation.is_some() })
}

/// Whether any pixel is less than fully opaque. An alpha channel that is opaque
/// everywhere, as the tone and flip steps add, does not count.
pub(crate) fn has_transparency(img: &image::DynamicImage) -> bool {
    match img {
        image::DynamicImage::ImageRgba8(rgba) => rgba.pixels().any(|pixel| pixel.0[3] < u8::MAX),
        image::DynamicImage::ImageLumaA8(luma) => luma.pixels().any(|pixel| pixel.0[1] < u8::MAX),
        other if other.color().has_alpha() => other.to_rgba16().pixels().any(|pixel| pixel.0[3] < u16::MAX),
        _ => false,
    }
}

/// Whether the image stores more than 8 bits per channel.
pub(crate) fn has_high_bit_depth(color: image::ColorType) -> bool {
    u16::from(color.bits_per_pixel()) / u16::from(color.channel_count()) > 8
}

/// Composites the image onto white, the background every image tool uses for a
/// format without transparency.
pub(crate) fn flatten_onto_white(img: &image::DynamicImage) -> image::RgbImage {
    let rgba = img.to_rgba8();
    image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [red, green, blue, alpha] = rgba.get_pixel(x, y).0;
        let over_white = |channel: u8| ((u32::from(channel) * u32::from(alpha) + 255 * u32::from(u8::MAX - alpha) + 127) / 255) as u8;
        image::Rgb([over_white(red), over_white(green), over_white(blue)])
    })
}

/// Baseline JPEG at `quality`. A greyscale image stays one channel; anything else is written as RGB
/// with alpha dropped.
pub(crate) fn encode_jpeg(img: &image::DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    encode_jpeg_with_profile(img, quality, None)
}

fn encode_jpeg_with_profile(img: &image::DynamicImage, quality: u8, icc: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut buf, quality);
    if let Some(icc) = icc {
        encoder.set_icc_profile(icc.to_vec()).map_err(|error| format!("Could not keep the colour profile: {error}"))?;
    }
    match img {
        image::DynamicImage::ImageLuma8(grey) => encoder.write_image(grey.as_raw(), grey.width(), grey.height(), image::ExtendedColorType::L8),
        _ => {
            let rgb = img.to_rgb8();
            encoder.write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        }
    }
    .map_err(|e| e.to_string())?;
    Ok(buf)
}

// Encoders write to a buffer first because WebP and HEIC only expose buffer
// encoders; JPEG and PNG go through the same path.
pub(crate) fn encode(img: &image::DynamicImage, format: OutputFormat, quality: u8) -> Result<Vec<u8>, String> {
    encode_with_profile(img, format, quality, None)
}

/// The colour profile embedded in a JPEG, PNG or WebP file, if it has one. HEIC is not read
/// here, because its encoder cannot write a profile back.
pub(crate) fn icc_profile(path: &std::path::Path) -> Option<Vec<u8>> {
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_decoder().ok()?;
    decoder.icc_profile().ok().flatten()
}

/// Encodes like `encode`, embedding the given colour profile in JPEG, PNG and WebP. HEIC is saved
/// without it, because libheif's encoder here has no way to write one.
pub(crate) fn encode_with_profile(img: &image::DynamicImage, format: OutputFormat, quality: u8, icc: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let profile_error = |error: image::error::UnsupportedError| format!("Could not keep the colour profile: {error}");
    match format {
        OutputFormat::Jpeg => {
            let rgb = if img.color().has_alpha() { flatten_onto_white(img) } else { img.to_rgb8() };
            encode_jpeg_with_profile(&image::DynamicImage::ImageRgb8(rgb), quality, icc)
        }
        OutputFormat::Png => {
            let mut buf = Vec::new();
            let mut encoder = PngEncoder::new(&mut buf);
            if let Some(icc) = icc { encoder.set_icc_profile(icc.to_vec()).map_err(profile_error)?; }
            encoder
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
            let bytes = webpx::Encoder::new_rgba(rgba.as_raw(), rgba.width(), rgba.height())
                .quality(quality.min(100) as f32)
                .encode(webpx::Unstoppable)
                .map_err(|e| format!("WebP encode failed: {}", e))?;
            match icc {
                Some(icc) => webpx::embed_icc(&bytes, icc).map_err(|e| format!("Could not keep the colour profile: {}", e)),
                None => Ok(bytes),
            }
        }
        OutputFormat::Heic => {
            // libheif's bundled x265 is 8-bit only, so a high-bit-depth source
            // is reduced to 8 bits per channel here
            let rgba = img.to_rgba8();
            let mut buf = Vec::new();
            image::DynamicImage::ImageRgba8(rgba)
                .write_with_encoder(heif::HeifEncoder::new(&mut buf).with_quality(quality))
                .map_err(|e| format!("HEIC encode failed: {}", e))?;
            Ok(buf)
        }
        OutputFormat::Tiff(compression) => encode_tiff(img, compression),
    }
}

fn encode_tiff(img: &DynamicImage, compression: TiffCompression) -> Result<Vec<u8>, String> {
    let mut buf = std::io::Cursor::new(Vec::new());
    write_tiff_pages(&mut buf, std::slice::from_ref(img), compression)?;
    Ok(buf.into_inner())
}

/// Writes each page as one TIFF directory with the chosen compression.
/// TIFF keeps 8 or 16 bits per channel and alpha as unassociated extra
/// samples; grey with alpha is written as RGBA, which every reader accepts.
pub(crate) fn write_tiff_pages<W: std::io::Write + std::io::Seek>(writer: W, pages: &[DynamicImage], compression: TiffCompression) -> Result<(), String> {
    use tiff::encoder::{colortype, Compression, DeflateLevel, TiffEncoder};
    use tiff::tags::{ExtraSamples, Tag};

    fn write<C: colortype::ColorType, W: std::io::Write + std::io::Seek>(encoder: &mut TiffEncoder<W>, width: u32, height: u32, data: &[C::Inner], alpha: bool) -> tiff::TiffResult<()>
    where [C::Inner]: tiff::encoder::TiffValue {
        let mut image = encoder.new_image::<C>(width, height)?;
        if alpha { image.encoder().write_tag(Tag::ExtraSamples, ExtraSamples::UnassociatedAlpha.to_u16())?; }
        image.write_data(data)
    }

    let compression = match compression {
        TiffCompression::Lzw => Compression::Lzw,
        TiffCompression::None => Compression::Uncompressed,
        TiffCompression::Deflate => Compression::Deflate(DeflateLevel::Balanced),
    };
    let mut encoder = TiffEncoder::new(writer).map_err(|e| e.to_string())?.with_compression(compression);
    for img in pages {
        let (width, height) = (img.width(), img.height());
        match img {
            DynamicImage::ImageLuma8(gray) => write::<colortype::Gray8, _>(&mut encoder, width, height, gray.as_raw(), false),
            DynamicImage::ImageLuma16(gray) => write::<colortype::Gray16, _>(&mut encoder, width, height, gray.as_raw(), false),
            DynamicImage::ImageRgb8(rgb) => write::<colortype::RGB8, _>(&mut encoder, width, height, rgb.as_raw(), false),
            DynamicImage::ImageRgb16(rgb) => write::<colortype::RGB16, _>(&mut encoder, width, height, rgb.as_raw(), false),
            DynamicImage::ImageLumaA16(_) | DynamicImage::ImageRgba16(_) => write::<colortype::RGBA16, _>(&mut encoder, width, height, img.to_rgba16().as_raw(), true),
            other if other.color().has_alpha() => write::<colortype::RGBA8, _>(&mut encoder, width, height, other.to_rgba8().as_raw(), true),
            other => write::<colortype::RGB8, _>(&mut encoder, width, height, other.to_rgb8().as_raw(), false),
        }
        .map_err(|e| format!("TIFF encode failed: {e}"))?;
    }
    Ok(())
}

// Convert targets are written at 8 bits per channel, except TIFF from a
// 16-bit source; alpha survives the reduction so the per-target alpha rule
// still applies afterwards.
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
fn encode_lossless_webp(img: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let rgba = img.to_rgba8();
    webpx::Encoder::new_rgba(rgba.as_raw(), rgba.width(), rgba.height())
        .lossless(true)
        .exact(true)
        .encode(webpx::Unstoppable)
        .map_err(|e| format!("WebP encode failed: {}", e))
}

/// One image written by Convert, with the source facts its fidelity rules need.
struct ConvertPage {
    bits: u8,
    orientation: Option<image::metadata::Orientation>,
    lossless_source: bool,
    suffix: String,
}

impl ConvertPage {
    /// Writes and publishes the converted image, adding each warning once
    /// across the pages of a file.
    fn write(self, input_path: &std::path::Path, mut img: DynamicImage, format: OutputFormat, output_location: &OutputLocation, warnings: &mut Vec<OutcomeWarning>) -> Result<PathBuf, String> {
        let mut warn = |kind: WarningKind, message: String| {
            let warning = OutcomeWarning { kind, message };
            if !warnings.contains(&warning) { warnings.push(warning); }
        };
        if let Some(orientation) = self.orientation {
            img.apply_orientation(orientation);
        }
        let keeps_depth = matches!(format, OutputFormat::Tiff(_)) && self.bits == 16;
        if self.bits > 8 && !keeps_depth {
            img = reduce_to_8_bit(img);
            warn(WarningKind::PrecisionReduced, format!("Reduced from {}-bit to 8-bit sRGB.", self.bits));
        }
        if format == OutputFormat::Jpeg && has_transparency(&img) {
            warn(WarningKind::TransparencyFlattened, tools::TRANSPARENCY_FLATTENED.to_string());
        }
        let output_path = OutputNaming::reserve_destination(input_path, output_location, &self.suffix, format.extension())
            .map_err(|error| format!("Could not reserve output: {error}"))?;
        let bytes = if format == OutputFormat::WebP && self.lossless_source { encode_lossless_webp(&img) } else { encode(&img, format, DEFAULT_QUALITY) }?;
        std::fs::write(output_path.path(), bytes).map_err(|error| format!("Write failed: {error}"))?;
        validate_encoded_output(output_path.path(), format, img.width(), img.height())?;
        output_path.publish().map_err(|error| format!("Could not publish output: {error}"))
    }
}

impl ImageProcessor {
    pub fn convert(input_path: PathBuf, format: OutputFormat, output_location: OutputLocation) -> JobOutcome {
        if let Some(message) = already_in_format(&input_path, format) {
            return JobOutcome::skipped(input_path, SkipReason::new(SkipCode::AlreadyInTargetFormat, message));
        }
        if ImageFamily::of(&input_path) == ImageFamily::Tiff {
            return Self::convert_tiff(input_path, format, &output_location);
        }
        if let Err(error) = reject_animated_input(&input_path) {
            return JobOutcome::failure(input_path, ToolError::processing(error));
        }
        match decode(&input_path) {
            Ok(decoded) => Self::convert_decoded(input_path, decoded, format, &output_location),
            Err(error) => JobOutcome::failure(input_path, ToolError::processing(error)),
        }
    }

    /// Convert fidelity rules: orientation is baked into the pixels once, deep
    /// sources are reduced to 8 bits with a warning (a 16-bit source keeps 16
    /// bits in TIFF), JPEG flattens alpha onto
    /// white with a warning, and WebP follows the source (lossless from PNG,
    /// TIFF and other lossless families, quality 90 from JPEG and HEIC).
    pub(crate) fn convert_decoded(input_path: PathBuf, decoded: DecodedImage, format: OutputFormat, output_location: &OutputLocation) -> JobOutcome {
        let lossless_source = !matches!(decoded.family, ImageFamily::Jpeg | ImageFamily::Heif);
        let page = ConvertPage { bits: decoded.bits_per_channel, orientation: decoded.orientation, lossless_source, suffix: "-converted".to_string() };
        let mut warnings = Vec::new();
        match page.write(&input_path, decoded.into_first_page(), format, output_location, &mut warnings) {
            Ok(path) => warnings.into_iter().fold(JobOutcome::succeeded(input_path, vec![path], format!("Saved as {}", format.extension())), JobOutcome::with_warning),
            Err(error) => JobOutcome::failure(input_path, ToolError::processing(error)),
        }
    }

    /// A TIFF is read and saved one page at a time, in page order. A
    /// multi-page TIFF makes one file per page named "-converted-p1",
    /// "-converted-p2" and so on; if a page fails, the pages already saved
    /// stay listed on the failed result.
    fn convert_tiff(input_path: PathBuf, format: OutputFormat, output_location: &OutputLocation) -> JobOutcome {
        let mut tiff = match source::TiffPages::open(&input_path) {
            Ok(tiff) => tiff,
            Err(error) => return JobOutcome::failure(input_path, ToolError::processing(error)),
        };
        let total = tiff.total();
        let mut outputs = Vec::with_capacity(total);
        let mut warnings = Vec::new();
        for index in 0..total {
            let saved = tiff.read(index).and_then(|(img, orientation)| {
                let suffix = if total > 1 { format!("-converted-p{}", index + 1) } else { "-converted".to_string() };
                let page = ConvertPage { bits: source::channel_bits(img.color()), orientation, lossless_source: true, suffix };
                page.write(&input_path, img, format, output_location, &mut warnings)
            });
            match saved {
                Ok(path) => outputs.push(path),
                Err(error) => {
                    let message = match outputs.as_slice() {
                        [] => error,
                        [only] => format!("{error} Page 1 was saved as {}.", only.file_name().unwrap_or_default().to_string_lossy()),
                        saved => format!("{error} Pages 1 to {} were saved.", saved.len()),
                    };
                    return warnings.into_iter().fold(JobOutcome::failure_with_outputs(input_path, outputs, ToolError::processing(message)), JobOutcome::with_warning);
                }
            }
        }
        let detail = if total > 1 { format!("{total} files, one per page.") } else { format!("Saved as {}", format.extension()) };
        warnings.into_iter().fold(JobOutcome::succeeded(input_path, outputs, detail), JobOutcome::with_warning)
    }
}

/// Whether the file is a GIF with more than one frame. It reads the header and at most two frames.
pub fn is_animated_gif(path: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = File::open(path) else { return false };
    let mut signature = [0u8; 4];
    if file.read_exact(&mut signature).is_err() || &signature != b"GIF8" { return false; }
    let Ok(file) = File::open(path) else { return false };
    let Ok(decoder) = image::codecs::gif::GifDecoder::new(BufReader::new(file)) else { return false };
    decoder.into_frames().take(2).count() > 1
}

fn reject_animated_input(path: &std::path::Path) -> Result<(), String> {
    if lower_ext(path).as_deref() != Some("gif") { return Ok(()); }
    let file = File::open(path).map_err(|error| format!("Could not inspect GIF sequence: {error}"))?;
    let decoder = image::codecs::gif::GifDecoder::new(BufReader::new(file)).map_err(|error| format!("Could not inspect GIF sequence: {error}"))?;
    let frames = decoder.into_frames().collect_frames().map_err(|error| format!("Could not inspect GIF frames: {error}"))?;
    if frames.len() > 1 { return Err("Animated GIF conversion is unsupported because it would discard frames; extract frames first.".to_string()); }
    Ok(())
}

// A TIFF output is read back through the image crate, which also reads the
// 16-bit pages the TIFF page tools turn away.
fn validate_encoded_output(path: &std::path::Path, format: OutputFormat, width: u32, height: u32) -> Result<(), String> {
    if !path.is_file() { return Err("Encoder did not produce an output container.".to_string()); }
    let output = match format {
        OutputFormat::Tiff(_) => image::open(path).map_err(|error| error.to_string()),
        _ => decode(path).map(|decoded| decoded.into_first_page()),
    }
    .map_err(|error| format!("Encoded output could not be decoded: {error}"))?;
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
    fn conversion_never_overwrites_or_modifies_the_original() {
        let src = temp_path("same_format_src.png");
        image::RgbaImage::from_pixel(64, 64, Rgba([10, 10, 10, 255]))
            .save(&src)
            .unwrap();
        let before = std::fs::read(&src).unwrap();

        let out = ImageProcessor::convert_decoded(src.clone(), decode(&src).unwrap(), OutputFormat::Png, &OutputLocation::AlongsideInput);

        assert!(out.error().is_none(), "{}", out.error().cloned().unwrap_or_default());
        assert_eq!(out.output_paths().len(), 1);
        assert_ne!(&out.output_paths()[0], &src, "must never write over the input");
        assert_eq!(std::fs::read(&src).unwrap(), before, "originals must never be modified");

        cleanup(&out.output_paths());
        let _ = std::fs::remove_file(&src);
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
            pages: vec![image::DynamicImage::ImageRgb16(deep.clone())],
            family: ImageFamily::Heif,
            bits_per_channel: 10,
            orientation: None,
            baseline_jpeg: None,
        };
        // TIFF keeps only 16-bit sources at 16 bits, so a 10-bit HEIC is reduced too.
        for format in [OutputFormat::Png, OutputFormat::Jpeg, OutputFormat::Tiff(TiffCompression::Lzw)] {
            let out = ImageProcessor::convert_decoded(src.clone(), ten_bit_heic(), format, &OutputLocation::AlongsideInput);
            assert!(out.error().is_none(), "{}", out.error().cloned().unwrap_or_default());
            assert_eq!(out.warnings.len(), 1);
            assert_eq!(out.warnings[0].kind, WarningKind::PrecisionReduced);
            assert_eq!(out.warnings[0].message, "Reduced from 10-bit to 8-bit sRGB.");
            let written = image::open(&out.output_paths()[0]).unwrap();
            assert_eq!(written.color(), image::ColorType::Rgb8, "{format:?} must be written at 8 bits per channel");
            if format != OutputFormat::Jpeg {
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

    #[test]
    fn conversion_rejects_animated_gifs_without_mutating_source() {
        let src = temp_path("animated_conversion.gif");
        let file = std::fs::File::create(&src).unwrap();
        let mut encoder = image::codecs::gif::GifEncoder::new(file);
        let frames = (0..2).map(|value| image::Frame::from_parts(image::RgbaImage::from_pixel(2, 2, Rgba([value * 100, 0, 0, 255])), 0, 0, image::Delay::from_numer_denom_ms(100, 1))).collect::<Vec<_>>();
        encoder.encode_frames(frames).unwrap();
        let before = std::fs::read(&src).unwrap();
        let result = ImageProcessor::convert(src.clone(), OutputFormat::Png, OutputLocation::AlongsideInput);
        assert!(result.error().is_some());
        assert_eq!(std::fs::read(&src).unwrap(), before);
        cleanup(&result.output_paths());
        let _ = std::fs::remove_file(src);
    }

    #[test]
    fn generated_high_bit_depth_heic_is_a_real_ten_bit_image() {
        let source = fixtures::red_left_blue_right(64, 32);
        let ten = fixtures::heic_10bit(&source);
        assert_eq!(heif::probe(&ten).unwrap().bit_depth, heif::BitDepth::Ten);
        let decoded = heif::decode(&ten).unwrap();
        assert!(matches!(decoded.color(), image::ColorType::Rgb16 | image::ColorType::Rgba16), "{:?}", decoded.color());
        assert_eq!(heif::probe(&fixtures::heic(&source)).unwrap().bit_depth, heif::BitDepth::Eight);
    }

    #[test]
    fn upright_loader_applies_each_orientation_exactly_once() {
        let source = fixtures::red_left_blue_right(64, 32);
        assert_eq!(fixtures::red_edge(&source), "left");
        // EXIF 6 is "rotate 90 degrees clockwise to display"; irot 1 is a
        // 90 degree anti-clockwise HEIF transform that libheif applies itself.
        let cases: [(&str, Vec<u8>, &str); 4] = [
            ("upright.jpg", fixtures::jpeg_with_orientation(&source, 1), "left"),
            ("exif-6.jpg", fixtures::jpeg_with_orientation(&source, 6), "top"),
            ("exif-3.jpg", fixtures::jpeg_with_orientation(&source, 3), "right"),
            ("irot-1.heic", fixtures::heic_rotated(&source, 1), "bottom"),
        ];
        for (name, bytes, edge) in cases {
            let path = temp_path(&format!("upright-{name}"));
            std::fs::write(&path, &bytes).unwrap();
            let loaded = load_upright_image(&path).unwrap();
            let expected = if edge == "left" || edge == "right" { (64, 32) } else { (32, 64) };
            assert_eq!((loaded.image.width(), loaded.image.height()), expected, "{name}");
            assert_eq!(fixtures::red_edge(&loaded.image), edge, "{name} must be rotated exactly once");
            assert_eq!(loaded.high_bit_depth, None);
            if name.ends_with(".jpg") {
                // The tag is real: a decode that ignores it still sees the stored landscape pixels.
                assert_eq!(fixtures::red_edge(&image::open(&path).unwrap()), "left", "{name}");
            }
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "{name} original must stay byte-identical");
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn a_heic_that_holds_a_property_list_is_refused_with_a_plain_reason() {
        let src = temp_path("HEIF Image-plist.heic");
        let bytes = b"bplist00\xd4\x01\x02\x03\x04\x05\x06\x07\x0aX$versionY$archiverT$topX$objects".to_vec();
        std::fs::write(&src, &bytes).unwrap();
        let message = "This file isn't a HEIC image. It looks like an Apple property list, not a photo. Export the original from Photos and try again.";

        assert_eq!(decode(&src).err().as_deref(), Some(message));
        let preview = tools::inspect_preview(&tools::ImagePreviewRequest { path: src.clone() }).unwrap_err();
        assert!(preview.ends_with(message), "{preview}");
        let out = ImageProcessor::convert(src.clone(), OutputFormat::Png, OutputLocation::AlongsideInput);
        assert_eq!(out.error().map(|error| error.message.as_str()), Some(message));
        assert!(out.output_paths().is_empty());
        assert_eq!(std::fs::read(&src).unwrap(), bytes, "the original is never modified");
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn a_file_without_the_format_its_extension_claims_is_refused_by_that_name() {
        for (name, format) in [("empty.heic", "HEIC"), ("notes.HEIF", "HEIF"), ("scan.jpg", "JPEG"), ("logo.png", "PNG"), ("photo.webp", "WebP"), ("fax.tif", "TIFF")] {
            let src = temp_path(&format!("claims-{name}"));
            std::fs::write(&src, b"plain text, not an image").unwrap();
            assert_eq!(decode(&src).err(), Some(format!("This file isn't a valid {format} image.")), "{name}");
            let _ = std::fs::remove_file(&src);
        }
    }

    #[test]
    fn an_image_saved_under_another_extension_decodes_by_its_content() {
        let source = fixtures::red_left_blue_right(64, 32);
        let cases: [(&str, Vec<u8>, source::ImageFamily); 3] = [
            ("jpeg-named.heic", fixtures::jpeg_with_orientation(&source, 6), source::ImageFamily::Jpeg),
            ("heic-named.jpg", fixtures::heic(&source), source::ImageFamily::Heif),
            ("png-named.heic", { let mut png = Vec::new(); source.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap(); png }, source::ImageFamily::Png),
        ];
        for (name, bytes, family) in cases {
            let src = temp_path(name);
            std::fs::write(&src, &bytes).unwrap();
            let decoded = decode(&src).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(decoded.family, family, "{name}");
            let upright = load_upright_image(&src).unwrap();
            let expected = if family == source::ImageFamily::Jpeg { (32, 64) } else { (64, 32) };
            assert_eq!((upright.image.width(), upright.image.height()), expected, "{name} keeps its own orientation handling");
            let out = ImageProcessor::convert(src.clone(), OutputFormat::Png, OutputLocation::AlongsideInput);
            assert!(out.error().is_none(), "{name}: {:?}", out.error());
            assert_eq!(out.output_paths()[0].extension().and_then(|e| e.to_str()), Some("png"), "{name}");
            assert_eq!(std::fs::read(&src).unwrap(), bytes, "{name} original must stay byte-identical");
            cleanup(&out.output_paths());
            let _ = std::fs::remove_file(&src);
        }
    }

    #[test]
    fn a_heic_photo_previews_and_converts_to_png() {
        let source = fixtures::red_left_blue_right(64, 32);
        let cases: [(&str, Vec<u8>, (u32, u32)); 3] = [
            ("IMG_8bit.HEIC", fixtures::heic(&source), (64, 32)),
            ("IMG_10bit.heic", fixtures::heic_10bit(&source), (64, 32)),
            ("IMG_rotated.heic", fixtures::heic_rotated(&source, 1), (32, 64)),
        ];
        for (name, bytes, size) in cases {
            let src = temp_path(name);
            std::fs::write(&src, &bytes).unwrap();
            let preview = tools::inspect_preview(&tools::ImagePreviewRequest { path: src.clone() }).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!((preview.width, preview.height), size, "{name}");
            assert!(preview.data_url.starts_with("data:image/png;base64,"), "{name}");
            let out = ImageProcessor::convert(src.clone(), OutputFormat::Png, OutputLocation::AlongsideInput);
            assert!(out.error().is_none(), "{name}: {:?}", out.error());
            let png = &out.output_paths()[0];
            assert!(png.file_name().unwrap().to_string_lossy().ends_with("-converted.png"), "{}", png.display());
            let written = image::open(png).unwrap();
            assert_eq!((written.width(), written.height()), size, "{name}");
            assert_eq!(fixtures::red_edge(&written), fixtures::red_edge(&load_upright_image(&src).unwrap().image), "{name}");
            assert_eq!(std::fs::read(&src).unwrap(), bytes, "{name} original must stay byte-identical");
            cleanup(&out.output_paths());
            let _ = std::fs::remove_file(&src);
        }
    }

    #[test]
    fn upright_loader_reports_high_bit_depth_heic() {
        let path = temp_path("upright-10bit.heic");
        std::fs::write(&path, fixtures::heic_10bit(&fixtures::red_left_blue_right(64, 32))).unwrap();
        let loaded = load_upright_image(&path).unwrap();
        assert_eq!(loaded.high_bit_depth, Some(10));
        assert_eq!(fixtures::red_edge(&loaded.image), "left");
        let _ = std::fs::remove_file(path);
    }
}
pub mod compress;
pub mod gif_extract;
pub(crate) mod gif_frames;
pub mod metadata;
pub(crate) mod strip;
pub mod tools;
#[cfg(test)]
pub(crate) mod fixtures;
