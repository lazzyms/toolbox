//! WebP: a lossless source (VP8L) stays lossless at Low and Medium, re-encoded with more effort so
//! every pixel stays the same, and becomes lossy at High. A lossy source (VP8) is re-encoded at the
//! strength's quality. Transparency is kept in both cases. The container is rebuilt here, so the
//! ICC colour profile always stays and EXIF and XMP stay unless Strip EXIF is on. An animated WebP
//! is refused, never flattened. libwebp runs single-threaded with fixed settings, so the same file
//! and settings give the same bytes.

use std::path::PathBuf;

use webpx::{EncoderConfig, Unstoppable};

use super::save_smaller_copy;
use crate::kit::common::{JobOutcome, OutputLocation};
use crate::kit::contracts::ToolError;

pub const DAMAGED: &str = "Couldn't read this WebP. The file may be damaged or incomplete. Nothing was saved.";
pub const ANIMATED: &str = "Animated WebP isn't supported. Only still images can be compressed.";

const ICC_FLAG: u8 = 0x20;
const ALPHA_FLAG: u8 = 0x10;
const EXIF_FLAG: u8 = 0x08;
const XMP_FLAG: u8 = 0x04;
const ANIMATION_FLAG: u8 = 0x02;
/// libwebp's effort for lossy encoding: 6 is its slowest and smallest.
const LOSSY_METHOD: u8 = 6;

/// How a strength treats WebPs. A lossless source is re-encoded without loss at each of
/// `lossless_levels` (libwebp's 0–9 lossless presets) and the smallest result is kept; libwebp's
/// higher presets are not always smaller, so trying more of them is what makes a stronger level
/// never larger. With no levels a lossless source becomes lossy at `quality`. A lossy source is
/// always re-encoded at `quality`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebpMethod {
    pub lossless_levels: &'static [u8],
    pub quality: u8,
}

/// One chunk: its four-character code and its data, without the padding byte.
struct Chunk<'a> {
    name: [u8; 4],
    data: &'a [u8],
}

/// The parts of a still WebP that Compress uses.
struct Parts<'a> {
    lossless: bool,
    /// The image data: ALPH and VP8, or VP8L, in file order.
    image: Vec<Chunk<'a>>,
    icc: Option<&'a [u8]>,
    exif: Option<&'a [u8]>,
    xmp: Option<&'a [u8]>,
}

/// Splits a WebP into its chunks. A file cut short, or one with no image data, is refused before
/// it is decoded; an animated file is refused as invalid input.
fn read_parts(bytes: &[u8]) -> Result<Parts<'_>, ToolError> {
    let damaged = || ToolError::processing(DAMAGED);
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" { return Err(damaged()); }
    let riff_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let end = riff_size.checked_add(8).filter(|end| *end <= bytes.len() && *end >= 12).ok_or_else(damaged)?;
    let mut parts = Parts { lossless: false, image: Vec::new(), icc: None, exif: None, xmp: None };
    let mut position = 12;
    while position < end {
        let header = bytes.get(position..position + 8).filter(|_| position + 8 <= end).ok_or_else(damaged)?;
        let name = [header[0], header[1], header[2], header[3]];
        let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let data_end = (position + 8).checked_add(size).filter(|data_end| *data_end <= end).ok_or_else(damaged)?;
        let data = &bytes[position + 8..data_end];
        match &name {
            b"ANIM" | b"ANMF" => return Err(ToolError::invalid_input(ANIMATED)),
            b"VP8X" if data.first().is_some_and(|flags| flags & ANIMATION_FLAG != 0) => return Err(ToolError::invalid_input(ANIMATED)),
            b"VP8L" => { parts.lossless = true; parts.image.push(Chunk { name, data }); }
            b"VP8 " | b"ALPH" => parts.image.push(Chunk { name, data }),
            b"ICCP" => parts.icc = Some(data),
            b"EXIF" => parts.exif = Some(data),
            b"XMP " => parts.xmp = Some(data),
            _ => {}
        }
        position = data_end + (size & 1);
    }
    if !parts.image.iter().any(|chunk| matches!(&chunk.name, b"VP8 " | b"VP8L")) { return Err(damaged()); }
    Ok(parts)
}

fn decode(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    webpx::decode_rgba(bytes).map_err(|_| DAMAGED.to_string())
}

fn push_chunk(output: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(name);
    output.extend_from_slice(&(data.len() as u32).to_le_bytes());
    output.extend_from_slice(data);
    if data.len() % 2 == 1 { output.push(0); }
}

/// Wraps the encoder's image data with the carried metadata. With nothing to carry and no separate
/// alpha chunk the simple layout is used; otherwise an extended header lists what follows.
fn assemble(image: &[Chunk], width: u32, height: u32, has_alpha: bool, icc: Option<&[u8]>, exif: Option<&[u8]>, xmp: Option<&[u8]>) -> Vec<u8> {
    let separate_alpha = image.iter().any(|chunk| &chunk.name == b"ALPH");
    let mut body = b"WEBP".to_vec();
    if icc.is_some() || exif.is_some() || xmp.is_some() || separate_alpha {
        let mut flags = 0;
        if icc.is_some() { flags |= ICC_FLAG; }
        if has_alpha || separate_alpha { flags |= ALPHA_FLAG; }
        if exif.is_some() { flags |= EXIF_FLAG; }
        if xmp.is_some() { flags |= XMP_FLAG; }
        let mut header = vec![flags, 0, 0, 0];
        header.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        header.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        push_chunk(&mut body, b"VP8X", &header);
    }
    if let Some(icc) = icc { push_chunk(&mut body, b"ICCP", icc); }
    image.iter().for_each(|chunk| push_chunk(&mut body, &chunk.name, chunk.data));
    if let Some(exif) = exif { push_chunk(&mut body, b"EXIF", exif); }
    if let Some(xmp) = xmp { push_chunk(&mut body, b"XMP ", xmp); }
    let mut output = b"RIFF".to_vec();
    output.extend_from_slice(&(body.len() as u32).to_le_bytes());
    output.extend_from_slice(&body);
    output
}

fn encode_error(error: impl std::fmt::Display) -> ToolError { ToolError::processing(format!("Could not re-save this WebP: {error}")) }

/// The compressed bytes and whether they are lossless.
fn recompress_webp(source: &[u8], method: WebpMethod, strip_exif: bool) -> Result<(Vec<u8>, bool), ToolError> {
    let parts = read_parts(source)?;
    let (pixels, width, height) = decode(source).map_err(ToolError::processing)?;
    let lossless = parts.lossless && !method.lossless_levels.is_empty();
    // `exact` keeps the colour under invisible pixels, so a lossless copy decodes to the same pixels.
    let configs = if lossless {
        method.lossless_levels.iter().map(|level| EncoderConfig::new_lossless_level(*level).exact(true)).collect()
    } else {
        vec![EncoderConfig::new().quality(f32::from(method.quality)).method(LOSSY_METHOD)]
    };
    let mut encoded: Option<Vec<u8>> = None;
    for config in configs {
        let candidate = config.encode_rgba(&pixels, width, height, Unstoppable).map_err(encode_error)?;
        if encoded.as_ref().is_none_or(|best| candidate.len() < best.len()) { encoded = Some(candidate); }
    }
    let encoded = encoded.expect("at least one encoder setting");
    let image = read_parts(&encoded).map_err(|_| encode_error("the encoder wrote an unreadable file"))?.image;
    let has_alpha = pixels.chunks_exact(4).any(|pixel| pixel[3] != 255);
    let (exif, xmp) = if strip_exif { (None, None) } else { (parts.exif, parts.xmp) };
    Ok((assemble(&image, width, height, has_alpha, parts.icc, exif, xmp), lossless))
}

/// Checks a written copy is a still WebP at the source's size, encoded lossless or lossy as
/// intended, that keeps the colour profile and, when Strip EXIF is on, carries no EXIF or XMP. A
/// lossless copy must have the same pixels; a lossy copy must keep invisible pixels invisible and
/// keep the source's transparency.
fn verify(copy: &[u8], source: &[u8], lossless: bool, strip_exif: bool) -> Result<(), String> {
    let unreadable = |_| "The compressed copy isn't a readable still WebP.".to_string();
    let copy_parts = read_parts(copy).map_err(unreadable)?;
    let source_parts = read_parts(source).map_err(unreadable)?;
    if copy_parts.lossless != lossless { return Err("The compressed copy wasn't encoded as intended.".to_string()); }
    let (source_pixels, source_width, source_height) = decode(source)?;
    let (copy_pixels, copy_width, copy_height) = decode(copy).map_err(|_| "The compressed copy could not be opened.".to_string())?;
    if (copy_width, copy_height) != (source_width, source_height) { return Err("The compressed copy changed the image's size.".to_string()); }
    if copy_parts.icc != source_parts.icc { return Err("The compressed copy lost its colour profile.".to_string()); }
    if strip_exif && (copy_parts.exif.is_some() || copy_parts.xmp.is_some()) { return Err("The compressed copy still carries EXIF.".to_string()); }
    if lossless {
        if copy_pixels != source_pixels { return Err("The compressed copy changed the image's pixels.".to_string()); }
        return Ok(());
    }
    let (source_alpha, copy_alpha) = (source_pixels.chunks_exact(4).map(|pixel| pixel[3]), copy_pixels.chunks_exact(4).map(|pixel| pixel[3]));
    let invisible_kept = source_alpha.clone().zip(copy_alpha.clone()).all(|(source, copy)| (source == 0) == (copy == 0));
    let alpha_kept = !source_alpha.clone().any(|alpha| alpha != 255) || copy_alpha.clone().any(|alpha| alpha != 255);
    if !invisible_kept || !alpha_kept { return Err("The compressed copy lost the image's transparency.".to_string()); }
    Ok(())
}

fn detail(lossless: bool, quality: u8, strip_exif: bool) -> String {
    let effect = if lossless { "Lossless, pixels unchanged.".to_string() } else { format!("Lossy at quality {quality}.") };
    let metadata = if strip_exif { "EXIF removed, colour profile kept." } else { "EXIF and colour profile kept." };
    format!("{effect} {metadata}")
}

/// Compresses one WebP into a new "-compressed" copy beside the input or in the chosen folder.
/// The original is only read; a copy that would not be smaller is discarded and Skipped.
pub fn compress_webp(input: PathBuf, location: &OutputLocation, method: WebpMethod, strip_exif: bool) -> JobOutcome {
    let source = match std::fs::read(&input) {
        Ok(bytes) => bytes,
        Err(error) => return JobOutcome::failure(input, ToolError::processing(format!("Could not read this WebP: {error}"))),
    };
    let (compressed, lossless) = match recompress_webp(&source, method, strip_exif) {
        Ok(result) => result,
        Err(error) => return JobOutcome::failure(input, error),
    };
    save_smaller_copy(input, location, "WebP", &source, compressed, |written| verify(written, &source, lossless, strip_exif), detail(lossless, method.quality, strip_exif))
}
