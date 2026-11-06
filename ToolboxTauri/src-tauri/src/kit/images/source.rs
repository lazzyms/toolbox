use std::fs::File;
use std::path::Path;

use image::metadata::Orientation;
use image::{ColorType, DynamicImage, ImageBuffer, ImageDecoder, ImageReader, Luma, Rgb, Rgba};

use super::{lower_ext, OutputFormat};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ImageFamily {
    Heif,
    Jpeg,
    Png,
    WebP,
    Tiff,
    Other,
}

impl ImageFamily {
    pub(crate) fn of(path: &Path) -> Self {
        match lower_ext(path).as_deref() {
            Some("heic" | "heif") => ImageFamily::Heif,
            Some("jpg" | "jpeg") => ImageFamily::Jpeg,
            Some("png") => ImageFamily::Png,
            Some("webp") => ImageFamily::WebP,
            Some("tif" | "tiff") => ImageFamily::Tiff,
            _ => ImageFamily::Other,
        }
    }

    pub(crate) fn output_format(self) -> OutputFormat {
        match self {
            ImageFamily::Jpeg => OutputFormat::Jpeg,
            ImageFamily::WebP => OutputFormat::WebP,
            ImageFamily::Heif => OutputFormat::Heic,
            _ => OutputFormat::Png,
        }
    }
}

// Facts beyond the pages and family are read by Images to PDF (#550) and the
// orientation-aware tools that follow it.
#[allow(dead_code)]
pub(crate) struct DecodedImage {
    pub pages: Vec<DynamicImage>,
    pub family: ImageFamily,
    pub bits_per_channel: u8,
    pub has_alpha: bool,
    pub orientation: Option<Orientation>,
    pub baseline_jpeg: Option<Vec<u8>>,
}

impl DecodedImage {
    pub(crate) fn into_first_page(mut self) -> DynamicImage {
        self.pages.swap_remove(0)
    }
}

pub(crate) fn decode(path: &Path) -> Result<DecodedImage, String> {
    let family = ImageFamily::of(path);
    let (pages, orientation, source_bits) = match family {
        ImageFamily::Tiff => {
            let (pages, orientation) = read_tiff_pages(path)?;
            (pages, orientation, None)
        }
        // HEIC has no decoder in the image crate, so it goes through the
        // bundled libheif adapter, which reports the true 10/12-bit depth
        // that its 16-bit samples would otherwise hide.
        ImageFamily::Heif => {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            let page = heif::decode(&bytes).map_err(|e| format!("HEIC decode failed: {}", e))?;
            let bits = match heif::probe(&bytes).map_err(|e| format!("HEIC decode failed: {}", e))?.bit_depth {
                heif::BitDepth::Ten => 10,
                heif::BitDepth::Twelve => 12,
                _ => 8,
            };
            (vec![page], None, Some(bits))
        }
        _ => {
            let (page, orientation) = read_single_image(path).map_err(|e| format!("Decode failed: {}", e))?;
            (vec![page], orientation, None)
        }
    };
    let bits_per_channel = source_bits.unwrap_or_else(|| pages.iter().map(|page| channel_bits(page.color())).max().unwrap_or(8));
    let has_alpha = pages.iter().any(|page| page.color().has_alpha());
    let baseline_jpeg = match family {
        ImageFamily::Jpeg => std::fs::read(path).ok().filter(|bytes| is_baseline_jpeg(bytes)),
        _ => None,
    };
    Ok(DecodedImage { pages, family, bits_per_channel, has_alpha, orientation, baseline_jpeg })
}

// Same steps as image::open, keeping the decoder long enough to read its
// orientation tag.
fn read_single_image(path: &Path) -> image::ImageResult<(DynamicImage, Option<Orientation>)> {
    let mut decoder = ImageReader::open(path)?.into_decoder()?;
    let mut limits = image::Limits::default();
    limits.reserve(decoder.total_bytes())?;
    decoder.set_limits(limits)?;
    let orientation = decoder.orientation().ok().filter(|orientation| *orientation != Orientation::NoTransforms);
    Ok((DynamicImage::from_decoder(decoder)?, orientation))
}

fn channel_bits(color: ColorType) -> u8 {
    color.bytes_per_pixel() / color.channel_count() * 8
}

fn is_baseline_jpeg(bytes: &[u8]) -> bool {
    if !bytes.starts_with(&[0xFF, 0xD8]) { return false; }
    let mut at = 2;
    while let Some(&[0xFF, marker, high, low]) = bytes.get(at..at + 4) {
        match marker {
            0xFF => at += 1,
            0xC0 => return true,
            0xC1..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF | 0xDA => return false,
            _ => at += 2 + usize::from(u16::from_be_bytes([high, low])),
        }
    }
    false
}

fn read_tiff_pages(path: &Path) -> Result<(Vec<DynamicImage>, Option<Orientation>), String> {
    let file = File::open(path).map_err(|error| format!("Could not open TIFF: {error}"))?;
    let mut decoder = tiff::decoder::Decoder::new(file).map_err(|error| format!("Could not read TIFF: {error}"))?;
    let orientation = decoder
        .find_tag_unsigned::<u8>(tiff::tags::Tag::Orientation)
        .ok()
        .flatten()
        .and_then(Orientation::from_exif)
        .filter(|orientation| *orientation != Orientation::NoTransforms);
    let mut pages = Vec::new();
    loop {
        let (width, height) = decoder.dimensions().map_err(|error| format!("Could not read TIFF dimensions: {error}"))?;
        let color = decoder.colortype().map_err(|error| format!("Could not read TIFF color type: {error}"))?;
        let data = decoder.read_image().map_err(|error| format!("Unsupported TIFF page: {error}"))?;
        let page = match (color, data) {
            (tiff::ColorType::Gray(8), tiff::decoder::DecodingResult::U8(data)) => ImageBuffer::<Luma<u8>, _>::from_raw(width, height, data).map(DynamicImage::ImageLuma8),
            (tiff::ColorType::RGB(8), tiff::decoder::DecodingResult::U8(data)) => ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, data).map(DynamicImage::ImageRgb8),
            (tiff::ColorType::RGBA(8), tiff::decoder::DecodingResult::U8(data)) => ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, data).map(DynamicImage::ImageRgba8),
            (color, _) => return Err(format!("Unsupported TIFF color type or bit depth: {color:?}")),
        };
        pages.push(page.ok_or_else(|| "TIFF page pixel data has an invalid length.".to_string())?);
        if !decoder.more_images() { break; }
        decoder.next_image().map_err(|error| format!("Could not advance to TIFF page: {error}"))?;
    }
    Ok((pages, orientation))
}
