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

pub(crate) struct DecodedImage {
    pub pages: Vec<DynamicImage>,
    pub family: ImageFamily,
    pub bits_per_channel: u8,
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
    let baseline_jpeg = match family {
        ImageFamily::Jpeg => std::fs::read(path).ok().filter(|bytes| is_baseline_jpeg(bytes)),
        _ => None,
    };
    Ok(DecodedImage { pages, family, bits_per_channel, orientation, baseline_jpeg })
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

pub(crate) fn channel_bits(color: ColorType) -> u8 {
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
    let mut tiff = TiffPages::open(path)?;
    let mut pages = Vec::with_capacity(tiff.total());
    let mut orientation = None;
    for index in 0..tiff.total() {
        let (page, page_orientation) = tiff.read(index)?;
        if index == 0 { orientation = page_orientation; }
        pages.push(page);
    }
    Ok((pages, orientation))
}

/// Reads a TIFF one page at a time, so a tool can check or use each page
/// before the next one is decoded.
pub(crate) struct TiffPages {
    decoder: tiff::decoder::Decoder<File>,
    total: usize,
}

impl TiffPages {
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let mut decoder = open_tiff(path)?;
        let total = count_tiff_pages(&mut decoder)?;
        Ok(Self { decoder, total })
    }

    pub(crate) fn total(&self) -> usize {
        self.total
    }

    /// Reads the width and height of the page at `index` from its header,
    /// without decoding its pixels.
    pub(crate) fn dimensions(&mut self, index: usize) -> Result<(u32, u32), String> {
        let total = self.total;
        self.decoder.seek_to_image(index)
            .and_then(|_| self.decoder.dimensions())
            .map_err(|error| format!("TIFF page {} of {total} can't be read. {error}", index + 1))
    }

    /// Decodes the page at `index` with its own orientation tag.
    pub(crate) fn read(&mut self, index: usize) -> Result<(DynamicImage, Option<Orientation>), String> {
        let total = self.total;
        let failed = |error: String| format!("TIFF page {} of {total} can't be read. {error}", index + 1);
        self.decoder.seek_to_image(index).map_err(|error| failed(error.to_string()))?;
        let orientation = self.decoder
            .find_tag_unsigned::<u8>(tiff::tags::Tag::Orientation)
            .ok()
            .flatten()
            .and_then(Orientation::from_exif)
            .filter(|orientation| *orientation != Orientation::NoTransforms);
        let page = read_tiff_page(&mut self.decoder).map_err(failed)?;
        Ok((page, orientation))
    }
}

pub(crate) fn page_count(path: &Path) -> Result<usize, String> {
    match ImageFamily::of(path) {
        ImageFamily::Tiff => count_tiff_pages(&mut open_tiff(path)?),
        _ => Ok(1),
    }
}

fn open_tiff(path: &Path) -> Result<tiff::decoder::Decoder<File>, String> {
    let file = File::open(path).map_err(|error| format!("Could not open TIFF: {error}"))?;
    tiff::decoder::Decoder::new(file).map_err(|error| format!("Could not read TIFF: {error}"))
}

fn count_tiff_pages(decoder: &mut tiff::decoder::Decoder<File>) -> Result<usize, String> {
    let mut total = 1;
    while decoder.more_images() {
        decoder.next_image().map_err(|error| format!("TIFF page {} can't be read. {error}", total + 1))?;
        total += 1;
    }
    Ok(total)
}

fn read_tiff_page(decoder: &mut tiff::decoder::Decoder<File>) -> Result<DynamicImage, String> {
    use tiff::decoder::DecodingResult;
    let (width, height) = decoder.dimensions().map_err(|error| format!("Could not read TIFF dimensions: {error}"))?;
    if decoder.find_tag_unsigned::<u16>(tiff::tags::Tag::Compression).ok().flatten() == Some(FAX3_COMPRESSION) {
        return read_fax3_page(decoder, width, height);
    }
    let color = decoder.colortype().map_err(|error| format!("Could not read TIFF color type: {error}"))?;
    let data = decoder.read_image().map_err(|error| format!("Unsupported TIFF page: {error}"))?;
    let page = match (color, data) {
        (tiff::ColorType::Gray(1), DecodingResult::U8(data)) => expand_bilevel(&data, width, height).map(DynamicImage::ImageLuma8),
        (tiff::ColorType::Gray(8), DecodingResult::U8(data)) => ImageBuffer::<Luma<u8>, _>::from_raw(width, height, data).map(DynamicImage::ImageLuma8),
        (tiff::ColorType::RGB(8), DecodingResult::U8(data)) => ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, data).map(DynamicImage::ImageRgb8),
        (tiff::ColorType::RGBA(8), DecodingResult::U8(data)) => ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, data).map(DynamicImage::ImageRgba8),
        (tiff::ColorType::YCbCr(8), DecodingResult::U8(data)) => ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, ycbcr_to_rgb(data)).map(DynamicImage::ImageRgb8),
        (color, _) => return Err(format!("Unsupported TIFF color type or bit depth: {color:?}")),
    };
    page.ok_or_else(|| "TIFF page pixel data has an invalid length.".to_string())
}

const FAX3_COMPRESSION: u16 = 3;

fn expand_bilevel(data: &[u8], width: u32, height: u32) -> Option<ImageBuffer<Luma<u8>, Vec<u8>>> {
    let stride = (width as usize).div_ceil(8);
    if data.len() < stride * height as usize { return None; }
    Some(ImageBuffer::from_fn(width, height, |x, y| {
        let byte = data[y as usize * stride + x as usize / 8];
        Luma([if byte & (0x80 >> (x % 8)) != 0 { 255 } else { 0 }])
    }))
}

fn ycbcr_to_rgb(mut data: Vec<u8>) -> Vec<u8> {
    for pixel in data.chunks_exact_mut(3) {
        let (y, cb, cr) = (f32::from(pixel[0]), f32::from(pixel[1]) - 128.0, f32::from(pixel[2]) - 128.0);
        pixel[0] = (y + 1.402 * cr).round().clamp(0.0, 255.0) as u8;
        pixel[1] = (y - 0.344_136 * cb - 0.714_136 * cr).round().clamp(0.0, 255.0) as u8;
        pixel[2] = (y + 1.772 * cb).round().clamp(0.0, 255.0) as u8;
    }
    data
}

// tiff 0.11 decodes Group 4 but not Group 3, so one-dimensional Group 3
// strips go through the fax crate that tiff already uses for Group 4.
fn read_fax3_page(decoder: &mut tiff::decoder::Decoder<File>, width: u32, height: u32) -> Result<DynamicImage, String> {
    use std::io::{Read, Seek, SeekFrom};
    use tiff::tags::Tag;
    let options = decoder.find_tag_unsigned::<u32>(Tag::Unknown(292)).ok().flatten().unwrap_or(0);
    if options & 1 != 0 { return Err("Two-dimensional Group 3 fax pages aren't supported.".to_string()); }
    let line_width = u16::try_from(width).map_err(|_| "The Group 3 fax page is too wide.".to_string())?;
    let white_is_zero = decoder.find_tag_unsigned::<u16>(Tag::PhotometricInterpretation).ok().flatten() != Some(1);
    let reversed = decoder.find_tag_unsigned::<u16>(Tag::FillOrder).ok().flatten() == Some(2);
    let rows_per_strip = decoder.find_tag_unsigned::<u32>(Tag::RowsPerStrip).ok().flatten().unwrap_or(height).max(1);
    let offsets = decoder.get_tag_u64_vec(Tag::StripOffsets).map_err(|error| format!("Could not read the Group 3 fax strips: {error}"))?;
    let counts = decoder.get_tag_u64_vec(Tag::StripByteCounts).map_err(|error| format!("Could not read the Group 3 fax strips: {error}"))?;
    if offsets.len() != counts.len() { return Err("The Group 3 fax strips are damaged.".to_string()); }
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for (offset, count) in offsets.into_iter().zip(counts) {
        let rows = rows_per_strip.min(height - (pixels.len() / width as usize) as u32);
        let mut strip = Vec::new();
        let reader = decoder.inner();
        reader.seek(SeekFrom::Start(offset)).map_err(|error| format!("Could not read the Group 3 fax strip: {error}"))?;
        reader.take(count).read_to_end(&mut strip).map_err(|error| format!("Could not read the Group 3 fax strip: {error}"))?;
        if reversed { strip.iter_mut().for_each(|byte| *byte = byte.reverse_bits()); }
        strip.extend_from_slice(&[0x00, 0x01]);
        let mut decoded = 0;
        fax::decoder::decode_g3(strip.into_iter(), |transitions| {
            if decoded < rows {
                pixels.extend(fax::decoder::pels(transitions, line_width).map(|color| if (color == fax::Color::Black) == white_is_zero { 0 } else { 255 }));
                decoded += 1;
            }
        });
        if decoded < rows { return Err("The Group 3 fax data ends before the page does.".to_string()); }
        if pixels.len() == width as usize * height as usize { break; }
    }
    ImageBuffer::<Luma<u8>, _>::from_raw(width, height, pixels)
        .map(DynamicImage::ImageLuma8)
        .ok_or_else(|| "The Group 3 fax data ends before the page does.".to_string())
}
