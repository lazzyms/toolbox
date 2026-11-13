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
    let compression = decoder.find_tag_unsigned::<u16>(tiff::tags::Tag::Compression).ok().flatten().unwrap_or(1);
    if matches!(compression, 3 | 4) && decoder.find_tag_unsigned::<u32>(tiff::tags::Tag::TileWidth).ok().flatten().is_none() {
        return read_ccitt_page(decoder, compression, width, height);
    }
    if let Some(message) = unsupported_compression(compression) {
        return Err(message);
    }
    let color = decoder.colortype().map_err(|error| format!("Could not read TIFF color type: {error}"))?;
    let unsupported = |decoder: &mut tiff::decoder::Decoder<File>| format!("{} TIFF pages aren't supported.", describe_variant(decoder, color));
    if !matches!(color, tiff::ColorType::Gray(1 | 8 | 16) | tiff::ColorType::RGB(8 | 16) | tiff::ColorType::RGBA(8 | 16) | tiff::ColorType::YCbCr(8)) {
        return Err(unsupported(decoder));
    }
    let data = decoder.read_image().map_err(|error| format!("Unsupported TIFF page: {error}"))?;
    let page = match (color, data) {
        (tiff::ColorType::Gray(1), DecodingResult::U8(data)) => expand_bilevel(&data, width, height).map(DynamicImage::ImageLuma8),
        (tiff::ColorType::Gray(8), DecodingResult::U8(data)) => ImageBuffer::<Luma<u8>, _>::from_raw(width, height, data).map(DynamicImage::ImageLuma8),
        (tiff::ColorType::Gray(16), DecodingResult::U16(data)) => ImageBuffer::<Luma<u16>, _>::from_raw(width, height, data).map(DynamicImage::ImageLuma16),
        (tiff::ColorType::RGB(8), DecodingResult::U8(data)) => ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, data).map(DynamicImage::ImageRgb8),
        (tiff::ColorType::RGB(16), DecodingResult::U16(data)) => ImageBuffer::<Rgb<u16>, _>::from_raw(width, height, data).map(DynamicImage::ImageRgb16),
        (tiff::ColorType::RGBA(8), DecodingResult::U8(data)) => ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, data).map(DynamicImage::ImageRgba8),
        (tiff::ColorType::RGBA(16), DecodingResult::U16(data)) => ImageBuffer::<Rgba<u16>, _>::from_raw(width, height, data).map(DynamicImage::ImageRgba16),
        (tiff::ColorType::YCbCr(8), DecodingResult::U8(data)) => ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, ycbcr_to_rgb(data)).map(DynamicImage::ImageRgb8),
        // Signed or floating-point samples decode to other result types.
        _ => return Err(unsupported(decoder)),
    };
    page.ok_or_else(|| "TIFF page pixel data has an invalid length.".to_string())
}

/// Explains which compression a page uses when this build can't decode it.
fn unsupported_compression(compression: u16) -> Option<String> {
    let name = match compression {
        // None, Group 3 and 4 fax, LZW, JPEG, Deflate, PackBits and the older Deflate code.
        1 | 3 | 4 | 5 | 7 | 8 | 32773 | 32946 => return None,
        2 => "CCITT modified Huffman",
        6 => "Old-style JPEG",
        32809 => "ThunderScan",
        34712 => "JPEG 2000",
        34887 => "LERC",
        34925 => "LZMA",
        50000 => "Zstandard",
        50001 => "WebP",
        other => return Some(format!("TIFF compression scheme {other} isn't supported.")),
    };
    Some(format!("{name} compression isn't supported in TIFF pages."))
}

/// Names a page's pixel format, such as "16-bit CMYK" or "32-bit floating-point RGB".
fn describe_variant(decoder: &mut tiff::decoder::Decoder<File>, color: tiff::ColorType) -> String {
    use tiff::ColorType::*;
    let (bits, model) = match color {
        Gray(bits) => (bits, "greyscale".to_string()),
        GrayA(bits) => (bits, "greyscale with alpha".to_string()),
        RGB(bits) => (bits, "RGB".to_string()),
        RGBA(bits) => (bits, "RGBA".to_string()),
        Palette(bits) => (bits, "palette (indexed colour)".to_string()),
        CMYK(bits) => (bits, "CMYK".to_string()),
        CMYKA(bits) => (bits, "CMYK with alpha".to_string()),
        YCbCr(bits) => (bits, "YCbCr".to_string()),
        Lab(bits) => (bits, "CIELAB".to_string()),
        Multiband { bit_depth, num_samples } => (bit_depth, format!("{num_samples}-channel")),
        other => return format!("{other:?}"),
    };
    // SampleFormat holds one value per sample; they match on any page this reader meets.
    let format = match decoder.find_tag_unsigned_vec::<u16>(tiff::tags::Tag::SampleFormat).ok().flatten().and_then(|formats| formats.first().copied()) {
        Some(2) => "signed ",
        Some(3) => "floating-point ",
        _ => "",
    };
    format!("{bits}-bit {format}{model}")
}

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

/// The most pixels a fax page may decode to, matching the tiff crate's default decoding buffer limit.
const MAX_FAX_PIXELS: u64 = 256 * 1024 * 1024;

/// Decodes a stripped CCITT Group 3 (one- or two-dimensional, compression 3) or Group 4 (compression 4) page with
/// hayro-ccitt, in-process and in pure Rust, into 8-bit greyscale.
///
/// Each strip is decoded on its own, `RowsPerStrip` rows at a time. `FillOrder` 2 strips are bit-reversed first.
/// As in libtiff, a decoded black run is a sample bit of 1, so with `WhiteIsZero` (the fax default) it is black and
/// with `BlackIsZero` it is white. A strip that ends before its rows are decoded fails the page. Tiled fax pages go
/// through the tiff crate instead.
fn read_ccitt_page(decoder: &mut tiff::decoder::Decoder<File>, compression: u16, width: u32, height: u32) -> Result<DynamicImage, String> {
    use hayro_ccitt::{DecodeSettings, DecoderContext, EncodingMode};
    use std::io::{Read, Seek, SeekFrom};
    use tiff::tags::Tag;
    let group = if compression == 3 { "Group 3" } else { "Group 4" };
    let bits = decoder.find_tag_unsigned_vec::<u16>(Tag::BitsPerSample).ok().flatten().unwrap_or_else(|| vec![1]);
    let samples = decoder.find_tag_unsigned::<u16>(Tag::SamplesPerPixel).ok().flatten().unwrap_or(1);
    if bits != [1] || samples != 1 { return Err(format!("{group} fax pages must be 1-bit black and white.")); }
    if u64::from(width) * u64::from(height) > MAX_FAX_PIXELS { return Err(format!("The {group} fax page is too large to read.")); }
    // T4Options bit 0 marks two-dimensional Group 3 coding and bit 2 byte-aligned EOLs, which the decoder reads as
    // fill. Uncompressed mode (bit 1) isn't decoded, so its rows come up missing and fail the page.
    let encoding = match compression {
        3 if decoder.find_tag_unsigned::<u32>(Tag::Unknown(292)).ok().flatten().unwrap_or(0) & 1 != 0 => EncodingMode::Group3_2D,
        3 => EncodingMode::Group3_1D,
        _ => EncodingMode::Group4,
    };
    let white_is_zero = decoder.find_tag_unsigned::<u16>(Tag::PhotometricInterpretation).ok().flatten() != Some(1);
    let reversed = decoder.find_tag_unsigned::<u16>(Tag::FillOrder).ok().flatten() == Some(2);
    let rows_per_strip = decoder.find_tag_unsigned::<u32>(Tag::RowsPerStrip).ok().flatten().unwrap_or(height).clamp(1, height.max(1));
    let strips_failed = |error: tiff::TiffError| format!("Could not read the {group} fax strips: {error}");
    let offsets = decoder.get_tag_u64_vec(Tag::StripOffsets).map_err(strips_failed)?;
    let counts = decoder.get_tag_u64_vec(Tag::StripByteCounts).map_err(strips_failed)?;
    if offsets.len() != counts.len() { return Err(format!("The {group} fax strips are damaged.")); }
    let mut rows = FaxRows { width: width as usize, black: if white_is_zero { 0 } else { 255 }, wanted: 0, pixels: Vec::new() };
    for (offset, count) in offsets.into_iter().zip(counts) {
        let decoded = (rows.pixels.len() / (width as usize).max(1)) as u32;
        if decoded == height { break; }
        let strip_rows = rows_per_strip.min(height - decoded);
        let mut strip = Vec::new();
        let reader = decoder.inner();
        let read_failed = |error: std::io::Error| format!("Could not read the {group} fax strip: {error}");
        reader.seek(SeekFrom::Start(offset)).map_err(read_failed)?;
        reader.take(count).read_to_end(&mut strip).map_err(read_failed)?;
        if reversed { strip.iter_mut().for_each(|byte| *byte = byte.reverse_bits()); }
        let settings = DecodeSettings {
            columns: width,
            rows: strip_rows,
            end_of_block: true,
            // TIFF Group 3 rows start with EOL codes; Group 4 has none.
            end_of_line: encoding != EncodingMode::Group4,
            rows_are_byte_aligned: false,
            encoding,
            strict: false,
        };
        rows.wanted = rows.pixels.len() + strip_rows as usize * width as usize;
        // Rows decoded before an error are kept, so the page fails only when rows are missing.
        let _ = hayro_ccitt::decode(&strip, &mut rows, &mut DecoderContext::new(settings));
        if rows.pixels.len() < rows.wanted { break; }
    }
    ImageBuffer::<Luma<u8>, _>::from_raw(width, height, rows.pixels)
        .filter(|page| page.len() == width as usize * height as usize)
        .map(DynamicImage::ImageLuma8)
        .ok_or_else(|| format!("The {group} fax data ends before the page does."))
}

/// Expands the rows hayro-ccitt reports as colour changes into 8-bit greyscale rows, up to the strip's last row.
struct FaxRows {
    width: usize,
    black: u8,
    wanted: usize,
    pixels: Vec<u8>,
}

impl hayro_ccitt::Decoder for FaxRows {
    fn push_row(&mut self, changes: &[u32]) {
        if self.pixels.len() >= self.wanted { return; }
        let start = self.pixels.len();
        self.pixels.resize(start + self.width, 255 - self.black);
        let row = &mut self.pixels[start..];
        for span in changes.chunks(2) {
            let from = (span[0] as usize).min(self.width);
            let to = span.get(1).map_or(self.width, |end| (*end as usize).min(self.width));
            row[from..to].fill(self.black);
        }
    }
}
