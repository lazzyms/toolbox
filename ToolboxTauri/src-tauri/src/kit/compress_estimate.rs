//! Compress Files size estimates: roughly how big each level's copy would be. Nothing is written,
//! and a PDF is never loaded or recompressed as a whole. A PDF is read by scanning its stream
//! objects, a few streams are sampled to measure how well they recompress, and the images that
//! Medium and High would re-encode are sized from their dictionaries and the page size. Images
//! and PNG or WebP files are sized from their headers, with a small sample decoded when a
//! per-pixel cost has to be measured. Every value is capped at the original size and the levels
//! never rise from Low to High, so the estimate is monotonic by construction.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use image::imageops::FilterType;
use image::{DynamicImage, GrayImage, RgbImage};
use memchr::memmem;
use serde::Serialize;

use crate::kit::common::limits::map_file;
use crate::kit::images;
use crate::kit::pdf::recompress::{self, ImageSettings};

/// The estimated sizes, in bytes, of one file at each level, next to its current size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeEstimate {
    pub original_bytes: u64,
    pub low: u64,
    pub medium: u64,
    pub high: u64,
}

/// Each level's estimated size before it is capped at the original.
#[derive(Debug, Clone, Copy)]
struct Levels {
    low: f64,
    medium: f64,
    high: f64,
}

/// Share of a PDF's text outside its streams that remains after the object-stream repack.
const PDF_TEXT_KEPT: f64 = 0.25;
/// JPEG output bytes scale with (quantiser scale)^-exponent. Calibrated against the app's encoder.
const JPEG_BITS_EXPONENT: f64 = 0.5;
/// The quality an unreadable JPEG table is assumed to be at, and the quality a lossy WebP is assumed at.
const ASSUMED_SOURCE_QUALITY: u8 = 75;
/// Fallback bytes per pixel for an RGB photo at the 75 and 50 qualities, used when no sample decodes.
const FALLBACK_RGB_BYTES_PER_PIXEL_Q75: f64 = 0.20;
const FALLBACK_RGB_BYTES_PER_PIXEL_Q50: f64 = 0.13;
/// Greyscale photos cost this share of the RGB cost per pixel.
const GREY_SHARE: f64 = 0.6;
const WEBP_LOSSLESS_LOW: f64 = 0.97;
const WEBP_LOSSLESS_MEDIUM: f64 = 0.95;
/// Medium's lossless PNG effort saves this share of Low's output.
const PNG_MEDIUM_SHARE: f64 = 0.98;
/// Palette output costs this much over its own indexed bytes.
const PNG_PALETTE_OVERHEAD: f64 = 1.4;
/// A PNG is measured from at most this much of its compressed and inflated image data.
const PNG_IDAT_SAMPLE_BYTES: usize = 1 << 20;
const PNG_RAW_SAMPLE_BYTES: usize = 4 << 20;
const PNG_PALETTE_COLOURS: usize = 256;
/// The bytes inflated or deflated when measuring how well streams recompress.
const RATIO_SAMPLE_BYTES: usize = 1 << 20;
const RATIO_SAMPLE_STREAMS: usize = 8;
/// How many small photographs are decoded to measure JPEG output per pixel, and how many are tried.
const IMAGE_SAMPLES: usize = 3;
const IMAGE_SAMPLE_CANDIDATES: usize = 12;
/// Decoded image samples above this are not read, matching the recompression limit.
const MAX_DECODED_BYTES: usize = 200_000_000;
/// How far before a `stream` keyword the object header and dictionary are searched for.
const MAX_DICT_WINDOW: usize = 64 * 1024;
/// A PDF page size used when no MediaBox is found: US Letter in points.
const DEFAULT_PAGE_POINTS: (f64, f64) = (612.0, 792.0);

/// Estimates one file on a blocking thread, so a large PDF never holds up the UI. One file per call
/// keeps files apart: the list shows each estimate as it arrives, a removed file is never started,
/// and a file that cannot be estimated, even by a panic, only loses its own estimate.
#[tauri::command]
pub async fn estimate_compressed_size(path: PathBuf, strip_exif: bool) -> Option<SizeEstimate> {
    tauri::async_runtime::spawn_blocking(move || estimate_file(&path, strip_exif)).await.ok().flatten()
}

/// The estimate for one file, or None when Compress would not take it: unsupported or HEIC
/// types, encrypted PDFs, animated images, and anything that cannot be read.
pub fn estimate_file(path: &Path, strip_exif: bool) -> Option<SizeEstimate> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(extension.as_str(), "pdf" | "jpg" | "jpeg" | "png" | "webp") { return None; }
    let bytes = map_file(path).ok()?;
    if bytes.is_empty() { return None; }
    let levels = match extension.as_str() {
        "pdf" => pdf_levels(&bytes)?,
        "jpg" | "jpeg" => jpeg_levels(&bytes, strip_exif)?,
        "png" => png_levels(&bytes, strip_exif)?,
        "webp" => webp_levels(&bytes)?,
        _ => return None,
    };
    Some(finish(bytes.len() as u64, levels))
}

/// Caps every level at the original size and keeps Low >= Medium >= High.
fn finish(original: u64, levels: Levels) -> SizeEstimate {
    let cap = |bytes: f64| (bytes.max(0.0).round() as u64).min(original);
    let low = cap(levels.low);
    let medium = cap(levels.medium).min(low);
    let high = cap(levels.high).min(medium);
    SizeEstimate { original_bytes: original, low, medium, high }
}

// ---------------------------------------------------------------------------------------------
// JPEG
// ---------------------------------------------------------------------------------------------

fn jpeg_levels(bytes: &[u8], strip_exif: bool) -> Option<Levels> {
    let layout = images::compress::jpeg_layout(bytes, strip_exif).ok()?;
    let source = layout.quality.unwrap_or(ASSUMED_SOURCE_QUALITY);
    Some(Levels {
        low: jpeg_bytes(&layout, source, 85)?,
        medium: jpeg_bytes(&layout, source, 75)?,
        high: jpeg_bytes(&layout, source, 50)?,
    })
}

/// Kept metadata, the encoder's own headers, and the scan rescaled from the source quality.
fn jpeg_bytes(layout: &images::compress::JpegLayout, source: u8, quality: u8) -> Option<f64> {
    let ratio = (images::compress::quant_scale(source) / images::compress::quant_scale(quality)).powf(JPEG_BITS_EXPONENT);
    let headers = images::encode_jpeg(&DynamicImage::new_luma8(8, 8), quality).ok()?.len();
    Some(layout.carried_bytes as f64 + headers as f64 + layout.scan_bytes as f64 * ratio)
}

// ---------------------------------------------------------------------------------------------
// PNG
// ---------------------------------------------------------------------------------------------

fn png_levels(bytes: &[u8], strip_exif: bool) -> Option<Levels> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) { return None; }
    let mut position = SIGNATURE.len();
    let mut header = None;
    let mut image_data: Vec<&[u8]> = Vec::new();
    let mut image_data_bytes = 0usize;
    let mut metadata_bytes = 0usize;
    while position + 12 <= bytes.len() {
        let length = u32::from_be_bytes(bytes[position..position + 4].try_into().ok()?) as usize;
        let end = position.checked_add(12)?.checked_add(length)?;
        if end > bytes.len() { return None; }
        let kind = &bytes[position + 4..position + 8];
        let data = &bytes[position + 8..position + 8 + length];
        match kind {
            b"IHDR" if data.len() >= 13 => {
                let width = u32::from_be_bytes(data[0..4].try_into().ok()?);
                header = Some((width, data[8], data[9], data[12]));
            }
            b"IDAT" => {
                image_data.push(data);
                image_data_bytes += length;
            }
            b"acTL" => return None,
            b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" => metadata_bytes += 12 + length,
            _ => {}
        }
        position = end;
        if kind == b"IEND" { break; }
    }
    let (width, depth, colour, interlace) = header.map(|(width, depth, colour, interlace)| (width, depth, colour, interlace))?;
    let fixed = (bytes.len() - image_data_bytes) as f64 - if strip_exif { metadata_bytes as f64 } else { 0.0 };

    let mut compressed = Vec::new();
    for chunk in &image_data {
        if compressed.len() >= PNG_IDAT_SAMPLE_BYTES { break; }
        compressed.extend_from_slice(chunk);
    }
    let mut decoder = ZlibDecoder::new(&compressed[..]);
    let mut raw = Vec::new();
    let _ = (&mut decoder).take(PNG_RAW_SAMPLE_BYTES as u64).read_to_end(&mut raw);
    let used = decoder.total_in() as usize;
    if used == 0 { return None; }
    let ratio = deflate(&raw) as f64 / used as f64;
    let body = image_data_bytes as f64 * ratio;
    let low = fixed + body;
    let medium = fixed + body * PNG_MEDIUM_SHARE;

    // Palette output stores one byte per pixel, so it only helps truecolour PNGs that have more
    // than 256 colours in the sampled rows.
    let bytes_per_pixel = match (depth, colour) { (8, 2) => 3, (8, 6) => 4, _ => 0 };
    let palette = bytes_per_pixel > 0 && interlace == 0 && more_than_palette_colours(&raw, width as usize, bytes_per_pixel);
    let high = if palette { fixed + body * PNG_MEDIUM_SHARE / bytes_per_pixel as f64 * PNG_PALETTE_OVERHEAD } else { medium };
    Some(Levels { low, medium, high })
}

/// True when the unfiltered rows of `raw` hold more than the palette's colours.
fn more_than_palette_colours(raw: &[u8], width: usize, bytes_per_pixel: usize) -> bool {
    // A crafted width is never trusted: rows longer than the sample are not read at all.
    let Some(stride) = width.checked_mul(bytes_per_pixel).map(|row| row + 1) else { return false };
    if width == 0 || stride > raw.len() { return false; }
    let mut previous = vec![0u8; stride - 1];
    let mut colours = HashSet::new();
    for row in raw.chunks_exact(stride) {
        let filter = row[0];
        let mut current = vec![0u8; stride - 1];
        for index in 0..stride - 1 {
            let left = if index >= bytes_per_pixel { current[index - bytes_per_pixel] } else { 0 };
            let up = previous[index];
            let up_left = if index >= bytes_per_pixel { previous[index - bytes_per_pixel] } else { 0 };
            let predictor = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, up_left),
                _ => return false,
            };
            current[index] = row[1 + index].wrapping_add(predictor);
        }
        for pixel in current.chunks_exact(bytes_per_pixel) {
            colours.insert(pixel.iter().fold(0u32, |key, &byte| (key << 8) | u32::from(byte)));
        }
        if colours.len() > PNG_PALETTE_COLOURS { return true; }
        previous = current;
    }
    false
}

fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let (a, b, c) = (i16::from(left), i16::from(up), i16::from(up_left));
    let estimate = a + b - c;
    let (pa, pb, pc) = ((estimate - a).abs(), (estimate - b).abs(), (estimate - c).abs());
    if pa <= pb && pa <= pc { left } else if pb <= pc { up } else { up_left }
}

// ---------------------------------------------------------------------------------------------
// WebP
// ---------------------------------------------------------------------------------------------

fn webp_levels(bytes: &[u8]) -> Option<Levels> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" { return None; }
    let mut position = 12;
    let mut lossless = None;
    let mut dimensions = None;
    while position + 8 <= bytes.len() {
        let kind = &bytes[position..position + 4];
        let size = u32::from_le_bytes(bytes[position + 4..position + 8].try_into().ok()?) as usize;
        let data_start = position + 8;
        let data = &bytes[data_start..(data_start + size).min(bytes.len())];
        match kind {
            b"VP8X" if data.first().is_some_and(|flags| flags & 0x02 != 0) => return None,
            b"ANIM" | b"ANMF" => return None,
            b"VP8L" => {
                lossless = Some(true);
                dimensions = vp8l_dimensions(data);
            }
            b"VP8 " => {
                lossless = Some(false);
                dimensions = vp8_dimensions(data);
            }
            _ => {}
        }
        position = data_start.checked_add(size)?.checked_add(size & 1)?;
    }
    let (width, height) = dimensions?;
    let original = bytes.len() as f64;
    let pixels = (width * height) as f64;
    match lossless? {
        true => Some(Levels {
            low: original * WEBP_LOSSLESS_LOW,
            medium: original * WEBP_LOSSLESS_MEDIUM,
            high: fallback_rgb(50) * pixels,
        }),
        false => {
            let scale = |quality: u8| (images::compress::quant_scale(ASSUMED_SOURCE_QUALITY) / images::compress::quant_scale(quality)).powf(JPEG_BITS_EXPONENT);
            Some(Levels {
                low: original * scale(85),
                medium: original * scale(75),
                high: original * scale(50),
            })
        }
    }
}

fn vp8l_dimensions(data: &[u8]) -> Option<(usize, usize)> {
    if data.first() != Some(&0x2f) { return None; }
    let bits = u32::from_le_bytes(data.get(1..5)?.try_into().ok()?);
    Some(((bits & 0x3fff) as usize + 1, ((bits >> 14) & 0x3fff) as usize + 1))
}

fn vp8_dimensions(data: &[u8]) -> Option<(usize, usize)> {
    if data.get(3..6)? != [0x9d, 0x01, 0x2a] { return None; }
    let width = u16::from_le_bytes(data.get(6..8)?.try_into().ok()?) & 0x3fff;
    let height = u16::from_le_bytes(data.get(8..10)?.try_into().ok()?) & 0x3fff;
    Some((width as usize, height as usize))
}

// ---------------------------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------------------------

fn pdf_levels(bytes: &[u8]) -> Option<Levels> {
    if !bytes.starts_with(b"%PDF") || memmem::find(bytes, b"/Encrypt").is_some() { return None; }
    let parts = pdf_parts(bytes);
    let page = media_box_inches(bytes);
    let data_bytes: usize = parts.iter().map(|part| part.data.len()).sum();
    let text = bytes.len().saturating_sub(data_bytes) as f64 * PDF_TEXT_KEPT;

    let flate_ratio = sample_ratio(&parts, Encoding::Flate);
    let raw_ratio = sample_ratio(&parts, Encoding::Raw);
    let kept = |part: &Part| -> f64 {
        let length = part.data.len() as f64;
        match part.encoding {
            Encoding::Raw => length * raw_ratio,
            Encoding::Flate => length * flate_ratio,
            Encoding::Dct | Encoding::Other => length,
        }
    };
    let low = text + parts.iter().map(kept).sum::<f64>();
    let samples = decode_samples(&parts);
    let medium = text + images_cost(&parts, &samples, recompress::MEDIUM, page, &kept);
    let high = text + images_cost(&parts, &samples, recompress::HIGH, page, &kept);
    Some(Levels { low, medium, high })
}

/// How a stream's bytes are stored, and the image it holds when Medium and High would re-encode it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Raw,
    Flate,
    Dct,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct ImageInfo {
    width: u32,
    height: u32,
    channels: usize,
}

#[derive(Debug, Clone, Copy)]
struct Part<'a> {
    data: &'a [u8],
    encoding: Encoding,
    image: Option<ImageInfo>,
}

/// Every stream object's data and classification. When an object number appears more than once,
/// as after an incremental update, only the last copy counts because it is the one a reader uses.
fn pdf_parts(bytes: &[u8]) -> Vec<Part<'_>> {
    let streams = pdf_streams(bytes);
    let mut last_index: HashMap<u32, usize> = HashMap::new();
    for (index, stream) in streams.iter().enumerate() {
        if let Some(number) = stream.object { last_index.insert(number, index); }
    }
    let current: Vec<&PdfStream> = streams.iter().enumerate()
        .filter(|(index, stream)| stream.object.is_none_or(|number| last_index[&number] == *index))
        .map(|(_, stream)| stream)
        .collect();
    let objects: Objects = current.iter().filter_map(|stream| Some((stream.object?, *stream))).collect();
    // Like the recompression, a stream another image uses as its soft or stencil mask stays as it is.
    let masks: HashSet<u32> = current.iter()
        .filter(|stream| is_image(&stream.entries))
        .flat_map(|stream| [&b"SMask"[..], b"Mask"].into_iter().filter_map(|key| reference(&stream.entries, key)))
        .collect();
    current.iter().map(|stream| classify(bytes, stream, &objects, &masks)).collect()
}

type Entries<'a> = HashMap<&'a [u8], Value<'a>>;
/// The current stream objects by object number, to follow references to masks and ICC profiles.
type Objects<'s, 'a> = HashMap<u32, &'s PdfStream<'a>>;

struct PdfStream<'a> {
    object: Option<u32>,
    entries: Entries<'a>,
    data: &'a [u8],
}

fn is_image(entries: &Entries) -> bool {
    name(entries, b"Subtype") == Some(b"Image")
}

/// Finds every `stream` ... `endstream` pair and the dictionary of the object that holds it. Stream
/// objects are never inside object streams, so the file's own text is enough to find them.
fn pdf_streams(bytes: &[u8]) -> Vec<PdfStream<'_>> {
    let mut streams = Vec::new();
    let mut skip_until = 0;
    for keyword in memmem::find_iter(bytes, b"stream") {
        if keyword < skip_until || (keyword >= 3 && &bytes[keyword - 3..keyword] == b"end") { continue; }
        let mut data_start = keyword + 6;
        match bytes.get(data_start) {
            Some(b'\r') => {
                data_start += 1;
                if bytes.get(data_start) == Some(&b'\n') { data_start += 1; }
            }
            Some(b'\n') => data_start += 1,
            _ => continue,
        }
        let head = &bytes[keyword.saturating_sub(MAX_DICT_WINDOW)..keyword];
        let Some(obj_at) = memmem::rfind(head, b"obj") else { continue };
        let Some(entries) = dict_entries(&head[obj_at + 3..]) else { continue };
        let Some(end_offset) = memmem::find(&bytes[data_start..], b"endstream") else { continue };
        let mut data_end = data_start + end_offset;
        if data_end > data_start && bytes[data_end - 1] == b'\n' { data_end -= 1; }
        if data_end > data_start && bytes[data_end - 1] == b'\r' { data_end -= 1; }
        streams.push(PdfStream { object: object_number(&head[..obj_at]), entries, data: &bytes[data_start..data_end] });
        skip_until = data_end;
    }
    streams
}

/// The object number from the `N G` pair written just before `obj`.
fn object_number(before_obj: &[u8]) -> Option<u32> {
    let mut words = before_obj.rsplit(|byte| byte.is_ascii_whitespace()).filter(|word| !word.is_empty());
    let _generation = words.next()?;
    std::str::from_utf8(words.next()?).ok()?.parse().ok()
}

fn classify<'a>(bytes: &'a [u8], stream: &PdfStream<'a>, objects: &Objects<'_, 'a>, masks: &HashSet<u32>) -> Part<'a> {
    let entries = &stream.entries;
    let encoding = match names(entries, b"Filter").as_slice() {
        [] => Encoding::Raw,
        [b"FlateDecode" | b"Fl"] => Encoding::Flate,
        [b"DCTDecode" | b"DCT"] => Encoding::Dct,
        _ => Encoding::Other,
    };
    let is_mask = stream.object.is_some_and(|number| masks.contains(&number));
    let image = (is_image(entries) && !is_mask).then(|| image_info(bytes, entries, encoding, objects)).flatten();
    Part { data: stream.data, encoding, image }
}

/// The image's size and channels when Medium and High would re-encode it, following the same
/// rules as the recompression in kit::pdf::recompress for the cases the dictionary shows.
fn image_info(bytes: &[u8], entries: &Entries, encoding: Encoding, objects: &Objects) -> Option<ImageInfo> {
    if boolean(entries, b"ImageMask") == Some(true) { return None; }
    for key in [&b"Mask"[..], b"Decode", b"SMaskInData"] {
        if entries.contains_key(key) { return None; }
    }
    // A soft mask with Matte pins its parent's pixel size, so that parent is never re-encoded.
    if entries.contains_key(&b"SMask"[..]) {
        let smask = objects.get(&reference(entries, b"SMask")?)?;
        if smask.entries.contains_key(&b"Matte"[..]) { return None; }
    }
    if int(entries, b"BitsPerComponent")? != 8 { return None; }
    let width = u32::try_from(int(entries, b"Width")?).ok().filter(|width| *width > 0)?;
    let height = u32::try_from(int(entries, b"Height")?).ok().filter(|height| *height > 0)?;
    // Quartz and others write the colour space as its own object, as in `/ColorSpace 6 0 R`.
    let colour_space = match entries.get(&b"ColorSpace"[..])? {
        Value::Ref(number) => indirect_value(bytes, *number)?,
        value => value.clone(),
    };
    let channels = match colour_space {
        Value::Name(b"DeviceGray") => 1,
        Value::Name(b"DeviceRGB") => 3,
        // An ICC profile counts only with 1 or 3 components; a CMYK profile stays as it is.
        Value::Array { ref names, ref refs } if names.as_slice() == [&b"ICCBased"[..]] && refs.len() == 1 => {
            match int(&objects.get(&refs[0])?.entries, b"N")? { 1 => 1, 3 => 3, _ => return None }
        }
        _ => return None,
    };
    if u64::from(width) * u64::from(height) * channels as u64 > MAX_DECODED_BYTES as u64 { return None; }
    match encoding {
        Encoding::Raw | Encoding::Flate if !entries.contains_key(&b"DecodeParms"[..]) => {}
        Encoding::Dct if !entries.contains_key(&b"DecodeParms"[..]) => {}
        _ => return None,
    }
    Some(ImageInfo { width, height, channels })
}

/// The value of a non-stream object, read from the last `N 0 obj` header for that number in the file.
fn indirect_value(bytes: &[u8], number: u32) -> Option<Value<'_>> {
    let header = format!("{number} 0 obj");
    let at = memmem::find_iter(bytes, header.as_bytes())
        .filter(|&at| at == 0 || bytes[at - 1].is_ascii_whitespace())
        .last()?;
    let start = at + header.len();
    let tokens = lex(&bytes[start..(start + 512).min(bytes.len())]);
    match value_at(&tokens, 0).0 {
        Value::Ref(_) | Value::Other => None,
        value => Some(value),
    }
}

/// The width and height, in inches, of the first MediaBox in the file, or US Letter.
fn media_box_inches(bytes: &[u8]) -> (f64, f64) {
    let default = (DEFAULT_PAGE_POINTS.0 / 72.0, DEFAULT_PAGE_POINTS.1 / 72.0);
    let Some(at) = memmem::find(bytes, b"/MediaBox") else { return default };
    let window = &bytes[at + 9..(at + 9 + 256).min(bytes.len())];
    let tokens = lex(window);
    if tokens.first() != Some(&Tok::OpenArray) { return default; }
    let numbers: Vec<f64> = tokens.iter().filter_map(|token| match token { Tok::Num(value) => Some(*value), _ => None }).take(4).collect();
    if numbers.len() < 4 { return default; }
    let (width, height) = ((numbers[2] - numbers[0]).abs(), (numbers[3] - numbers[1]).abs());
    if width <= 0.0 || height <= 0.0 { return default; }
    (width / 72.0, height / 72.0)
}

/// A decoded photograph whose re-encoding is measured.
struct DecodedSample {
    info: ImageInfo,
    image: DynamicImage,
}

/// The first few RGB photographs that decode and pass the photographic check. They are decoded once
/// and shared by the Medium and High estimates.
fn decode_samples(parts: &[Part]) -> Vec<DecodedSample> {
    let mut samples = Vec::new();
    let candidates = parts.iter().filter(|part| part.image.is_some_and(|info| info.channels == 3)).take(IMAGE_SAMPLE_CANDIDATES);
    for part in candidates {
        let Some(info) = part.image else { continue };
        let Some(image) = decode_image(part) else { continue };
        if part.encoding != Encoding::Dct && !recompress::is_photographic(&image) { continue; }
        samples.push(DecodedSample { info, image });
        if samples.len() == IMAGE_SAMPLES { break; }
    }
    samples
}

/// The images at Medium or High: each is re-encoded at the settings' quality and its size on the page,
/// or kept as it is when that would not be smaller. Other streams cost what they do at Low.
fn images_cost(parts: &[Part], samples: &[DecodedSample], settings: ImageSettings, page: (f64, f64), kept: &impl Fn(&Part) -> f64) -> f64 {
    let rgb = measured_rgb_bytes_per_pixel(samples, settings, page).unwrap_or_else(|| fallback_rgb(settings.quality));
    parts.iter().map(|part| match part.image {
        Some(info) => {
            let (width, height) = target_size(info, page, settings.max_ppi);
            let per_pixel = if info.channels == 3 { rgb } else { rgb * GREY_SHARE };
            kept(part).min(width as f64 * height as f64 * per_pixel)
        }
        None => kept(part),
    }).sum()
}

fn fallback_rgb(quality: u8) -> f64 {
    if quality >= 75 { FALLBACK_RGB_BYTES_PER_PIXEL_Q75 } else { FALLBACK_RGB_BYTES_PER_PIXEL_Q50 }
}

/// The pixel size an image is kept at: its size, or the most the page needs at `max_ppi`.
fn target_size(info: ImageInfo, page: (f64, f64), max_ppi: u32) -> (u32, u32) {
    (recompress::target(info.width, page.0, max_ppi), recompress::target(info.height, page.1, max_ppi))
}

/// JPEG bytes per RGB pixel at the settings' quality, measured on the samples each resized to its
/// target size. None when there are no samples.
fn measured_rgb_bytes_per_pixel(samples: &[DecodedSample], settings: ImageSettings, page: (f64, f64)) -> Option<f64> {
    let mut bytes = 0usize;
    let mut pixels = 0usize;
    for sample in samples {
        let (width, height) = target_size(sample.info, page, settings.max_ppi);
        let resized;
        let image = if (width, height) == (sample.info.width, sample.info.height) {
            &sample.image
        } else {
            resized = sample.image.resize_exact(width, height, FilterType::Lanczos3);
            &resized
        };
        let Ok(encoded) = images::encode_jpeg(image, settings.quality) else { continue };
        bytes += encoded.len();
        pixels += width as usize * height as usize;
    }
    (pixels > 0).then(|| bytes as f64 / pixels as f64)
}

fn decode_image(part: &Part) -> Option<DynamicImage> {
    let info = part.image?;
    let (width, height, channels) = (info.width as usize, info.height as usize, info.channels);
    let needed = width * height * channels;
    if part.encoding == Encoding::Dct {
        let decoded = image::load_from_memory_with_format(part.data, image::ImageFormat::Jpeg).ok()?;
        return (decoded.width() == info.width && decoded.height() == info.height).then_some(decoded);
    }
    let mut plain = match part.encoding {
        Encoding::Flate => inflate_prefix(part.data, needed)?.1,
        Encoding::Raw => part.data.get(..needed)?.to_vec(),
        Encoding::Dct | Encoding::Other => return None,
    };
    if plain.len() < needed { return None; }
    plain.truncate(needed);
    if channels == 1 {
        GrayImage::from_raw(info.width, info.height, plain).map(DynamicImage::ImageLuma8)
    } else {
        RgbImage::from_raw(info.width, info.height, plain).map(DynamicImage::ImageRgb8)
    }
}

/// How many compressed bytes a stream recompresses to, measured on a sample of up to eight streams
/// of one encoding. Without a sample the stream is taken to stay the same size.
fn sample_ratio(parts: &[Part], encoding: Encoding) -> f64 {
    let mut source = 0usize;
    let mut recompressed = 0usize;
    for part in parts.iter().filter(|part| part.encoding == encoding && !part.data.is_empty()).take(RATIO_SAMPLE_STREAMS) {
        let sample = match encoding {
            Encoding::Flate => inflate_prefix(part.data, RATIO_SAMPLE_BYTES).map(|(used, plain)| (used, deflate(&plain))),
            _ => {
                let length = part.data.len().min(RATIO_SAMPLE_BYTES);
                Some((length, deflate(&part.data[..length])))
            }
        };
        let Some((used, deflated)) = sample else { continue };
        source += used;
        recompressed += deflated;
    }
    if source == 0 { 1.0 } else { recompressed as f64 / source as f64 }
}

/// The first `limit` bytes of a zlib stream, and how many compressed bytes they took.
fn inflate_prefix(data: &[u8], limit: usize) -> Option<(usize, Vec<u8>)> {
    let mut decoder = ZlibDecoder::new(data);
    let mut plain = Vec::new();
    let _ = (&mut decoder).take(limit as u64).read_to_end(&mut plain);
    (!plain.is_empty()).then(|| (decoder.total_in() as usize, plain))
}

/// The size of `plain` as a zlib stream at the highest compression level, as qpdf writes it.
fn deflate(plain: &[u8]) -> usize {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    if encoder.write_all(plain).is_err() { return plain.len(); }
    encoder.finish().map_or(plain.len(), |out| out.len())
}

// ---------------------------------------------------------------------------------------------
// PDF tokens
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok<'a> {
    Open,
    Close,
    OpenArray,
    CloseArray,
    Name(&'a [u8]),
    Num(f64),
    Word(&'a [u8]),
    Other,
}

#[derive(Debug, Clone, PartialEq)]
enum Value<'a> {
    Name(&'a [u8]),
    /// An array's names and the object numbers it refers to, in order; other items are left out.
    Array { names: Vec<&'a [u8]>, refs: Vec<u32> },
    Num(f64),
    Bool(bool),
    Ref(u32),
    Other,
}

/// The object number `N` of an `N G R` reference, read with the same tokens as a dictionary.
fn reference_at(tokens: &[Tok], i: usize) -> Option<u32> {
    match (tokens.get(i)?, tokens.get(i + 1)?, tokens.get(i + 2)?) {
        (Tok::Num(number), Tok::Num(_), Tok::Word(b"R")) if number.fract() == 0.0 && *number >= 0.0 => u32::try_from(*number as u64).ok(),
        _ => None,
    }
}

fn is_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

/// Splits PDF object syntax into tokens. Strings are skipped, so their contents are never read as keys.
fn lex(text: &[u8]) -> Vec<Tok<'_>> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let byte = text[i];
        match byte {
            _ if byte.is_ascii_whitespace() || byte == 0 => i += 1,
            b'%' => {
                while i < text.len() && text[i] != b'\n' && text[i] != b'\r' { i += 1; }
            }
            b'<' if text.get(i + 1) == Some(&b'<') => {
                tokens.push(Tok::Open);
                i += 2;
            }
            b'>' if text.get(i + 1) == Some(&b'>') => {
                tokens.push(Tok::Close);
                i += 2;
            }
            b'<' => {
                i = text[i..].iter().position(|&b| b == b'>').map_or(text.len(), |offset| i + offset + 1);
                tokens.push(Tok::Other);
            }
            b'[' => {
                tokens.push(Tok::OpenArray);
                i += 1;
            }
            b']' => {
                tokens.push(Tok::CloseArray);
                i += 1;
            }
            b'(' => {
                let mut depth = 0usize;
                while i < text.len() {
                    match text[i] {
                        b'\\' => i += 1,
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 { i += 1; break; }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                tokens.push(Tok::Other);
            }
            b'/' => {
                let start = i + 1;
                i = start;
                while i < text.len() && !is_delimiter(text[i]) { i += 1; }
                tokens.push(Tok::Name(&text[start..i]));
            }
            _ if is_delimiter(byte) => {
                i += 1;
                tokens.push(Tok::Other);
            }
            _ => {
                let start = i;
                while i < text.len() && !is_delimiter(text[i]) { i += 1; }
                let word = &text[start..i];
                tokens.push(match std::str::from_utf8(word).ok().and_then(|word| word.parse::<f64>().ok()) {
                    Some(number) => Tok::Num(number),
                    None => Tok::Word(word),
                });
            }
        }
    }
    tokens
}

/// The top-level entries of a dictionary; None when the text does not begin with one.
fn dict_entries(text: &[u8]) -> Option<HashMap<&[u8], Value<'_>>> {
    let tokens = lex(text);
    if tokens.first() != Some(&Tok::Open) { return None; }
    let mut entries = HashMap::new();
    let mut depth = 1usize;
    let mut i = 1;
    while i < tokens.len() {
        match tokens[i] {
            Tok::Open | Tok::OpenArray => {
                depth += 1;
                i += 1;
            }
            Tok::Close | Tok::CloseArray => {
                depth -= 1;
                i += 1;
                if depth == 0 { break; }
            }
            Tok::Name(key) if depth == 1 => {
                let (value, next) = value_at(&tokens, i + 1);
                entries.insert(key, value);
                i = next;
            }
            _ => i += 1,
        }
    }
    Some(entries)
}

/// The value starting at token `i`, and the index just after it. Nested dictionaries are left for the
/// caller to step into, so their keys are not mistaken for this dictionary's.
fn value_at<'a>(tokens: &[Tok<'a>], i: usize) -> (Value<'a>, usize) {
    match tokens.get(i) {
        Some(Tok::Name(name)) => (Value::Name(name), i + 1),
        Some(Tok::Num(number)) => match reference_at(tokens, i) {
            Some(object) => (Value::Ref(object), i + 3),
            None => (Value::Num(*number), i + 1),
        },
        Some(Tok::Word(word)) if *word == b"true" => (Value::Bool(true), i + 1),
        Some(Tok::Word(word)) if *word == b"false" => (Value::Bool(false), i + 1),
        Some(Tok::OpenArray) => {
            let (mut names, mut refs) = (Vec::new(), Vec::new());
            let mut depth = 0usize;
            let mut j = i;
            while let Some(token) = tokens.get(j) {
                if let Some(object) = reference_at(tokens, j) {
                    refs.push(object);
                    j += 3;
                    continue;
                }
                match token {
                    Tok::Open | Tok::OpenArray => depth += 1,
                    Tok::Close | Tok::CloseArray => {
                        depth -= 1;
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    Tok::Name(name) => names.push(*name),
                    _ => {}
                }
                j += 1;
            }
            (Value::Array { names, refs }, j)
        }
        _ => (Value::Other, i),
    }
}

fn name<'a>(entries: &HashMap<&[u8], Value<'a>>, key: &[u8]) -> Option<&'a [u8]> {
    match entries.get(key)? {
        Value::Name(name) => Some(name),
        _ => None,
    }
}

fn names<'a>(entries: &HashMap<&[u8], Value<'a>>, key: &[u8]) -> Vec<&'a [u8]> {
    match entries.get(key) {
        Some(Value::Name(name)) => vec![name],
        Some(Value::Array { names, .. }) => names.clone(),
        _ => Vec::new(),
    }
}

fn reference(entries: &HashMap<&[u8], Value<'_>>, key: &[u8]) -> Option<u32> {
    match entries.get(key)? {
        Value::Ref(object) => Some(*object),
        _ => None,
    }
}

fn int(entries: &HashMap<&[u8], Value<'_>>, key: &[u8]) -> Option<i64> {
    match entries.get(key)? {
        Value::Num(number) if number.fract() == 0.0 => Some(*number as i64),
        _ => None,
    }
}

fn boolean(entries: &HashMap<&[u8], Value<'_>>, key: &[u8]) -> Option<bool> {
    match entries.get(key)? {
        Value::Bool(value) => Some(*value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{estimate_file, SizeEstimate};
    use std::path::{Path, PathBuf};

    fn sandbox(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("toolbox_estimate_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A smooth photo-like gradient with mild noise, so JPEG and palette behave like real photographs.
    fn photo(width: u32, height: u32) -> image::RgbImage {
        let mut state = 0x9e37_79b9u32;
        image::RgbImage::from_fn(width, height, |x, y| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (state >> 29) as u8;
            image::Rgb([(x * 255 / width) as u8 / 2 + noise, (y * 255 / height) as u8 / 2 + noise, ((x + y) % 256) as u8])
        })
    }

    fn check_ordering(estimate: SizeEstimate) {
        assert!(estimate.high <= estimate.medium && estimate.medium <= estimate.low && estimate.low <= estimate.original_bytes, "{estimate:?}");
    }

    #[test]
    fn photo_jpeg_levels_are_monotonic_and_repeatable() {
        let root = sandbox("jpeg");
        let path = root.join("photo.jpg");
        photo(640, 480).save_with_format(&path, image::ImageFormat::Jpeg).unwrap();
        let first = estimate_file(&path, false).expect("a readable JPEG has an estimate");
        assert_eq!(estimate_file(&path, false), Some(first), "the same file gives the same estimate");
        check_ordering(first);
        assert!(first.medium < first.original_bytes, "a q95 photo shrinks at Medium: {first:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn palette_png_gains_most_at_high() {
        let root = sandbox("png");
        let path = root.join("photo.png");
        photo(300, 300).save_with_format(&path, image::ImageFormat::Png).unwrap();
        let estimate = estimate_file(&path, false).expect("a readable PNG has an estimate");
        check_ordering(estimate);
        assert!(estimate.high < estimate.medium, "more than 256 colours can be palettised: {estimate:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn lossless_webp_has_levels_and_lossy_high_shrinks_it() {
        let root = sandbox("webp");
        let path = root.join("photo.webp");
        let rgb = photo(256, 256);
        let file = std::fs::File::create(&path).unwrap();
        image::codecs::webp::WebPEncoder::new_lossless(file).encode(rgb.as_raw(), 256, 256, image::ExtendedColorType::Rgb8).unwrap();
        let estimate = estimate_file(&path, false).expect("a lossless WebP has an estimate");
        check_ordering(estimate);
        assert!(estimate.high < estimate.original_bytes, "High makes a lossless WebP lossy: {estimate:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn flate_photo_pdf_has_an_estimate_that_orders_the_levels() {
        use std::io::Write;
        let root = sandbox("pdf");
        let path = root.join("photo.pdf");
        let rgb = photo(1200, 900);
        let mut deflated = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        deflated.write_all(rgb.as_raw()).unwrap();
        let deflated = deflated.finish().unwrap();
        let mut pdf = b"%PDF-1.4\n1 0 obj\n<< /Type /Page /MediaBox [0 0 612 792] /Contents 2 0 R >>\nendobj\n".to_vec();
        pdf.extend_from_slice(b"2 0 obj\n<< /Length 0 >>\nstream\n\nendstream\nendobj\n");
        pdf.extend_from_slice(format!("3 0 obj\n<< /Type /XObject /Subtype /Image /Width 1200 /Height 900 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n", deflated.len()).as_bytes());
        pdf.extend_from_slice(&deflated);
        pdf.extend_from_slice(b"\nendstream\nendobj\n%%EOF\n");
        std::fs::write(&path, &pdf).unwrap();
        let estimate = estimate_file(&path, false).expect("a readable PDF has an estimate");
        check_ordering(estimate);
        assert!(estimate.medium < estimate.low, "the photo is re-encoded at Medium: {estimate:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unsupported_encrypted_and_animated_files_have_no_estimate() {
        let root = sandbox("none");
        let heic = root.join("shot.heic");
        std::fs::write(&heic, b"not really a heic").unwrap();
        let text = root.join("notes.txt");
        std::fs::write(&text, b"hello").unwrap();
        let empty = root.join("empty.jpg");
        std::fs::write(&empty, b"").unwrap();
        let encrypted = root.join("locked.pdf");
        std::fs::write(&encrypted, b"%PDF-1.4\n1 0 obj\n<< /Filter /Standard /V 2 >>\nendobj\ntrailer\n<< /Encrypt 1 0 R >>\n%%EOF\n").unwrap();
        let gif = root.join("animated.gif");
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(std::fs::File::create(&gif).unwrap());
            encoder.encode_frame(image::Frame::new(image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255])))).unwrap();
        }
        for path in [&heic, &text, &empty, &encrypted, &gif] {
            assert_eq!(estimate_file(path, false), None, "{}", path.display());
        }
        assert_eq!(estimate_file(Path::new("/nonexistent/photo.jpg"), false), None);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Which object numbers' streams count as images Medium and High would re-encode.
    fn re_encoded(pdf: &str) -> Vec<bool> {
        super::pdf_parts(pdf.as_bytes()).iter().map(|part| part.image.is_some()).collect()
    }

    const PHOTO: &str = "/Subtype /Image /Width 4 /Height 4 /BitsPerComponent 8 /Length 1";

    #[test]
    fn pdf_masks_matte_parents_and_cmyk_profiles_stay_as_they_are() {
        // Object 1 uses 2 as its soft mask: 1 is re-encoded and 2, a mask, is not.
        let soft_mask = format!("%PDF-1.4\n1 0 obj\n<< {PHOTO} /ColorSpace /DeviceRGB /SMask 2 0 R >>\nstream\nx\nendstream\nendobj\n\
            2 0 obj\n<< {PHOTO} /ColorSpace /DeviceGray >>\nstream\nx\nendstream\nendobj\n");
        assert_eq!(re_encoded(&soft_mask), [true, false]);
        // A soft mask with Matte keeps its parent too.
        let matte = soft_mask.replace("/ColorSpace /DeviceGray", "/ColorSpace /DeviceGray /Matte [0 0 0]");
        assert_eq!(re_encoded(&matte), [false, false]);
        // An ICC profile counts by its component count: 3 is RGB, 4 is CMYK and stays.
        let icc = |components: u8| format!("%PDF-1.4\n1 0 obj\n<< {PHOTO} /ColorSpace [/ICCBased 2 0 R] >>\nstream\nx\nendstream\nendobj\n\
            2 0 obj\n<< /N {components} /Length 1 >>\nstream\nx\nendstream\nendobj\n");
        assert_eq!(re_encoded(&icc(3)), [true, false]);
        assert_eq!(re_encoded(&icc(4)), [false, false]);
        // Quartz writes the colour space as its own object; it is followed like the inline one.
        let indirect = |components: u8| format!("%PDF-1.4\n1 0 obj\n<< {PHOTO} /ColorSpace 3 0 R >>\nstream\nx\nendstream\nendobj\n\
            2 0 obj\n<< /N {components} /Length 1 >>\nstream\nx\nendstream\nendobj\n3 0 obj\n[/ICCBased 2 0 R]\nendobj\n");
        assert_eq!(re_encoded(&indirect(3)), [true, false]);
        assert_eq!(re_encoded(&indirect(4)), [false, false]);
        let named = "%PDF-1.4\n1 0 obj\n<< {PHOTO} /ColorSpace 13 0 R >>\nstream\nx\nendstream\nendobj\n13 0 obj /DeviceGray endobj\n".replace("{PHOTO}", PHOTO);
        assert_eq!(re_encoded(&named), [true]);
        assert_eq!(re_encoded(&named.replace("13 0 obj /DeviceGray", "13 0 obj /DeviceCMYK")), [false]);
    }

    #[test]
    fn a_png_with_a_crafted_width_is_estimated_without_reading_its_rows() {
        // Rows wider than the sample are never allocated, so a huge IHDR width cannot exhaust memory.
        assert!(!super::more_than_palette_colours(&[0; 64], u32::MAX as usize, 4));
        assert!(!super::more_than_palette_colours(&[0; 64], usize::MAX / 2, 4));
    }

    #[test]
    fn estimating_writes_nothing() {
        let root = sandbox("readonly");
        let path = root.join("photo.jpg");
        photo(320, 240).save_with_format(&path, image::ImageFormat::Jpeg).unwrap();
        let before = std::fs::read(&path).unwrap();
        let names_before: Vec<_> = std::fs::read_dir(&root).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        let _ = estimate_file(&path, true);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let names_after: Vec<_> = std::fs::read_dir(&root).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(names_before, names_after);
        let _ = std::fs::remove_dir_all(root);
    }
}
