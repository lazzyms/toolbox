//! Compress Files for JPEG: re-encode at the strength's quality, keep the JPEG format, and carry
//! the source's metadata segments over byte for byte. The ICC colour profile always stays; EXIF,
//! XMP, IPTC and comments stay unless Strip EXIF is on. Orientation stays as the EXIF tag when
//! EXIF is kept and is baked into the pixels when it is stripped, so photos display upright.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegDecoder;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::compress::NO_SAVINGS;
use crate::kit::contracts::{SkipCode, SkipReason, ToolError};

const SOI: [u8; 2] = [0xFF, 0xD8];
const APP0: u8 = 0xE0;
const APP1: u8 = 0xE1;
const APP2: u8 = 0xE2;
const APP13: u8 = 0xED;
const COM: u8 = 0xFE;
const SOS: u8 = 0xDA;
const EOI: u8 = 0xD9;

pub const ENDS_EARLY: &str = "This JPEG ends early, so it may be damaged. Nothing was saved.";
pub const CMYK_UNSUPPORTED: &str = "This JPEG uses CMYK colour, which Compress can't re-save without changing its colours.";

/// One marker segment before the image data, including its marker and length bytes.
#[derive(Clone, Copy)]
struct Segment<'a> {
    marker: u8,
    bytes: &'a [u8],
}

impl Segment<'_> {
    fn payload(&self) -> &[u8] { &self.bytes[4..] }
    fn is(&self, marker: u8, signature: &[u8]) -> bool { self.marker == marker && self.payload().starts_with(signature) }
    fn is_icc(&self) -> bool { self.is(APP2, b"ICC_PROFILE\0") }
    /// EXIF, XMP (and extended XMP), IPTC and comments: what Strip EXIF removes.
    fn is_descriptive(&self) -> bool {
        self.is(APP1, b"Exif\0\0") || self.is(APP1, b"http://ns.adobe.com/xap/1.0/\0")
            || self.is(APP1, b"http://ns.adobe.com/xmp/extension/\0") || self.is(APP13, b"Photoshop 3.0\0") || self.marker == COM
    }
    /// A JFIF header with no thumbnail only records pixel density, so it is carried over in every case.
    fn is_plain_jfif(&self) -> bool { self.is(APP0, b"JFIF\0") && self.payload().len() == 14 }
}

struct Header<'a> {
    segments: Vec<Segment<'a>>,
    components: Option<u8>,
}

/// Reads the marker segments up to the image data, and checks the image data is followed by an
/// end marker so a truncated file is refused before anything is decoded.
fn read_header(bytes: &[u8]) -> Result<Header<'_>, String> {
    if !bytes.starts_with(&SOI) { return Err("This file isn't a readable JPEG. Nothing was saved.".to_string()); }
    let mut segments = Vec::new();
    let mut components = None;
    let mut position = 2;
    loop {
        if bytes.get(position) != Some(&0xFF) { return Err(ENDS_EARLY.to_string()); }
        while bytes.get(position + 1) == Some(&0xFF) { position += 1; }
        let Some(&marker) = bytes.get(position + 1) else { return Err(ENDS_EARLY.to_string()) };
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) { position += 2; continue; }
        if marker == EOI { return Err(ENDS_EARLY.to_string()); }
        let length = match bytes.get(position + 2..position + 4) {
            Some(&[high, low]) => usize::from(u16::from_be_bytes([high, low])),
            _ => return Err(ENDS_EARLY.to_string()),
        };
        let end = position + 2 + length;
        if length < 2 || end > bytes.len() { return Err(ENDS_EARLY.to_string()); }
        let segment = Segment { marker, bytes: &bytes[position..end] };
        if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            components = segment.payload().get(5).copied();
        }
        if marker == SOS {
            if !bytes[end..].windows(2).any(|pair| pair == [0xFF, EOI]) { return Err(ENDS_EARLY.to_string()); }
            return Ok(Header { segments, components });
        }
        segments.push(segment);
        position = end;
    }
}

/// The encoder's output without its own SOI and APPn segments, ready to follow carried metadata.
fn after_app_segments(encoded: &[u8]) -> Result<&[u8], String> {
    let mut position = 2;
    while encoded.get(position) == Some(&0xFF) && encoded.get(position + 1).is_some_and(|marker| (APP0..=0xEF).contains(marker)) {
        let length = usize::from(u16::from_be_bytes([encoded[position + 2], encoded[position + 3]]));
        position += 2 + length;
    }
    encoded.get(position..).ok_or_else(|| "The JPEG encoder wrote an incomplete file.".to_string())
}

fn decode(bytes: &[u8]) -> Result<(DynamicImage, Orientation, Option<Vec<u8>>, bool), String> {
    let mut decoder = JpegDecoder::new(Cursor::new(bytes)).map_err(|error| format!("Could not read this JPEG: {error}"))?;
    let orientation = decoder.orientation().map_err(|error| format!("Could not read this JPEG: {error}"))?;
    let icc = decoder.icc_profile().map_err(|error| format!("Could not read this JPEG: {error}"))?;
    let exif = decoder.exif_metadata().map_err(|error| format!("Could not read this JPEG: {error}"))?.is_some();
    let image = DynamicImage::from_decoder(decoder).map_err(|error| format!("Could not read this JPEG: {error}"))?;
    Ok((image, orientation, icc, exif))
}

/// The width and height a viewer shows once the orientation tag is applied.
fn displayed(image: &DynamicImage, orientation: Orientation) -> (u32, u32) {
    match orientation {
        Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH => (image.height(), image.width()),
        _ => (image.width(), image.height()),
    }
}

/// Re-encodes the JPEG at `quality` and assembles the copy with the carried metadata segments.
pub(crate) fn recompress_jpeg(bytes: &[u8], quality: u8, strip_exif: bool) -> Result<Vec<u8>, ToolError> {
    let header = read_header(bytes).map_err(ToolError::processing)?;
    if header.components == Some(4) { return Err(ToolError::invalid_input(CMYK_UNSUPPORTED)); }
    let (mut image, orientation, _, _) = decode(bytes).map_err(ToolError::processing)?;
    if strip_exif { image.apply_orientation(orientation); }
    let encoded = super::encode_jpeg(&image, quality).map_err(|error| ToolError::processing(format!("Could not re-save this JPEG: {error}")))?;
    let body = after_app_segments(&encoded).map_err(ToolError::processing)?;
    let jfif = header.segments.iter().find(|segment| segment.is_plain_jfif()).map(|segment| segment.bytes)
        .unwrap_or_else(|| &encoded[2..encoded.len() - body.len()]);
    let carried = header.segments.iter().filter(|segment| segment.is_icc() || (!strip_exif && segment.is_descriptive()));
    let mut output = SOI.to_vec();
    output.extend_from_slice(jfif);
    carried.for_each(|segment| output.extend_from_slice(segment.bytes));
    output.extend_from_slice(body);
    Ok(output)
}

/// Checks a written copy decodes as a JPEG, displays at the source's size, keeps the colour
/// profile, and carries no EXIF when Strip EXIF is on.
fn verify(copy: &[u8], source: &[u8], strip_exif: bool) -> Result<(), String> {
    if image::guess_format(copy).ok() != Some(image::ImageFormat::Jpeg) { return Err("The compressed copy isn't a JPEG.".to_string()); }
    read_header(copy)?;
    let (source_image, source_orientation, source_icc, _) = decode(source)?;
    let (copy_image, copy_orientation, copy_icc, copy_exif) = decode(copy).map_err(|error| format!("The compressed copy could not be opened: {error}"))?;
    if displayed(&copy_image, copy_orientation) != displayed(&source_image, source_orientation) {
        return Err("The compressed copy changed the photo's size.".to_string());
    }
    if copy_icc != source_icc { return Err("The compressed copy lost its colour profile.".to_string()); }
    if strip_exif && copy_exif { return Err("The compressed copy still carries EXIF.".to_string()); }
    Ok(())
}

fn failure(input: PathBuf, message: String) -> JobOutcome { JobOutcome::failure(input, ToolError::processing(message)) }

/// Compresses one JPEG into a new "-compressed" copy beside the input or in the chosen folder.
/// The original is only read; a copy that would not be smaller is discarded and Skipped.
pub fn compress_jpeg(input: PathBuf, location: &OutputLocation, quality: u8, strip_exif: bool) -> JobOutcome {
    let source = match std::fs::read(&input) {
        Ok(bytes) => bytes,
        Err(error) => return failure(input, format!("Could not read this JPEG: {error}")),
    };
    let compressed = match recompress_jpeg(&source, quality, strip_exif) {
        Ok(bytes) => bytes,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let (original_bytes, output_bytes) = (source.len() as u64, compressed.len() as u64);
    if output_bytes >= original_bytes {
        return JobOutcome::skipped(input, SkipReason::new(SkipCode::NoSavings, NO_SAVINGS)).with_sizes(original_bytes, output_bytes);
    }
    let extension = extension_of(&input);
    let output = match OutputNaming::reserve_destination(&input, location, "-compressed", &extension) {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve compressed JPEG output: {error}")),
    };
    if let Err(error) = std::fs::write(output.path(), &compressed) {
        return failure(input, format!("Could not write the compressed JPEG: {error}"));
    }
    let written = match std::fs::read(output.path()) {
        Ok(bytes) => bytes,
        Err(error) => return failure(input, format!("Compressed JPEG could not be verified: {error}")),
    };
    if let Err(error) = verify(&written, &source, strip_exif) {
        return failure(input, format!("{error} Nothing was saved."));
    }
    match output.publish() {
        Ok(path) => JobOutcome::succeeded(input, vec![path], detail(quality, strip_exif)).with_sizes(original_bytes, output_bytes),
        Err(error) => failure(input, format!("Could not publish compressed JPEG: {error}")),
    }
}

fn extension_of(input: &Path) -> String {
    input.extension().and_then(|extension| extension.to_str()).unwrap_or("jpg").to_string()
}

fn detail(quality: u8, strip_exif: bool) -> String {
    if strip_exif { format!("Re-saved at quality {quality}. EXIF removed, colour profile kept.") }
    else { format!("Re-saved at quality {quality}. EXIF and colour profile kept.") }
}
