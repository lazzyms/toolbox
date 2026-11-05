//! PNG: Low and Medium repack the file without loss through oxipng, so every pixel stays the
//! same. High cuts the image to a palette of at most 256 colours with Floyd–Steinberg dithering,
//! keeping transparency in the palette; an image that already has 256 colours or fewer is
//! repacked without loss instead. The ICC colour profile and colour-space chunks always stay;
//! EXIF, text and timestamp chunks stay unless Strip EXIF is on. An animated PNG is refused.
//! Every step is deterministic, so the same file and settings give the same bytes.

use std::collections::HashSet;
use std::io::Cursor;
use std::path::PathBuf;

use image::codecs::png::PngDecoder;
use image::{DynamicImage, ImageDecoder};
use oxipng::{BitDepth, ColorType, Options, RawImage, StripChunks, RGBA8};

use super::save_smaller_copy;
use crate::kit::common::{JobOutcome, OutputLocation};
use crate::kit::contracts::ToolError;

pub const DAMAGED: &str = "Couldn't read this PNG. The file may be damaged or incomplete. Nothing was saved.";
pub const ANIMATED: &str = "Animated PNG isn't supported. Only still images can be compressed.";

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];
/// Chunks Strip EXIF removes: EXIF, text (which also carries XMP) and the modification time.
const DESCRIPTIVE: [&[u8; 4]; 5] = [b"eXIf", b"tEXt", b"zTXt", b"iTXt", b"tIME"];
/// Chunks a palette copy carries over whatever Strip EXIF says: how its colours are meant to look,
/// and its pixel density.
const COLOUR_SPACE: [&[u8; 4]; 5] = [b"iCCP", b"gAMA", b"cHRM", b"sRGB", b"pHYs"];
const TRANSPARENT: [u8; 4] = [0, 0, 0, 0];

/// How a strength treats PNGs. `effort` is oxipng's preset: higher tries more filters and a
/// stronger deflate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngMethod {
    Lossless { effort: u8 },
    Palette { effort: u8 },
}

/// One chunk: its type and the whole chunk as stored, length, type, data and CRC.
struct Chunk<'a> {
    name: [u8; 4],
    bytes: &'a [u8],
}

impl Chunk<'_> {
    fn data(&self) -> &[u8] { &self.bytes[8..self.bytes.len() - 4] }
    fn is_descriptive(&self) -> bool { DESCRIPTIVE.contains(&&self.name) }
}

/// Splits a PNG into its chunks. The header must come first and the end chunk last, so a file that
/// was cut short is refused before it is decoded; an animation control chunk refuses the file.
fn read_chunks(bytes: &[u8]) -> Result<Vec<Chunk<'_>>, ToolError> {
    if !bytes.starts_with(&SIGNATURE) { return Err(ToolError::processing(DAMAGED)); }
    let mut chunks = Vec::new();
    let mut position = SIGNATURE.len();
    while position < bytes.len() {
        let Some(header) = bytes.get(position..position + 8) else { return Err(ToolError::processing(DAMAGED)) };
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let end = position.checked_add(12).and_then(|end| end.checked_add(length)).filter(|end| *end <= bytes.len())
            .ok_or_else(|| ToolError::processing(DAMAGED))?;
        let name = [header[4], header[5], header[6], header[7]];
        if &name == b"acTL" { return Err(ToolError::invalid_input(ANIMATED)); }
        chunks.push(Chunk { name, bytes: &bytes[position..end] });
        position = end;
        if &name == b"IEND" { break; }
    }
    let complete = chunks.first().is_some_and(|chunk| &chunk.name == b"IHDR") && chunks.last().is_some_and(|chunk| &chunk.name == b"IEND");
    if !complete { return Err(ToolError::processing(DAMAGED)); }
    Ok(chunks)
}

/// The PNG rebuilt from its own chunks without the ones Strip EXIF removes, byte for byte otherwise.
fn without_descriptive(chunks: &[Chunk]) -> Vec<u8> {
    let mut output = SIGNATURE.to_vec();
    chunks.iter().filter(|chunk| !chunk.is_descriptive()).for_each(|chunk| output.extend_from_slice(chunk.bytes));
    output
}

fn decode(bytes: &[u8]) -> Result<(DynamicImage, Option<Vec<u8>>), String> {
    let mut decoder = PngDecoder::new(Cursor::new(bytes)).map_err(|_| DAMAGED.to_string())?;
    let icc = decoder.icc_profile().map_err(|_| DAMAGED.to_string())?;
    let image = DynamicImage::from_decoder(decoder).map_err(|_| DAMAGED.to_string())?;
    Ok((image, icc))
}

fn options(effort: u8) -> Options {
    let mut options = Options::from_preset(effort);
    // Chunks are chosen here, never by oxipng, which would otherwise swap an ICC profile for sRGB.
    options.strip = StripChunks::None;
    options
}

fn optimize_error(error: oxipng::PngError) -> ToolError { ToolError::processing(format!("Could not re-save this PNG: {error}")) }

/// The colours a palette copy uses: one fully transparent entry when the image has invisible
/// pixels, and the rest learned by NeuQuant from the visible pixels. Each learned alpha is moved to
/// the nearest alpha the image really has, so opaque images stay fully opaque.
fn learn_palette(pixels: &[u8]) -> Vec<[u8; 4]> {
    let visible: Vec<u8> = pixels.chunks_exact(4).filter(|pixel| pixel[3] != 0).flatten().copied().collect();
    let has_transparent = visible.len() < pixels.len();
    let mut palette = Vec::with_capacity(256);
    if has_transparent { palette.push(TRANSPARENT); }
    if visible.is_empty() { return palette; }
    let mut alphas = [false; 256];
    visible.chunks_exact(4).for_each(|pixel| alphas[usize::from(pixel[3])] = true);
    let nearest_alpha = |alpha: u8| (1..=255u8).filter(|candidate| alphas[usize::from(*candidate)])
        .min_by_key(|candidate| candidate.abs_diff(alpha)).unwrap_or(255);
    let sample_factor = if visible.len() / 4 <= 1 << 20 { 3 } else { 10 };
    let quantizer = color_quant::NeuQuant::new(sample_factor, 256 - palette.len(), &visible);
    for colour in quantizer.color_map_rgba().chunks_exact(4) {
        palette.push([colour[0], colour[1], colour[2], nearest_alpha(colour[3])]);
    }
    palette
}

/// Maps colours to their nearest visible palette entry. Colours are grouped five bits per channel
/// and each group is matched once, at its centre, so large images stay quick.
struct Matcher<'a> {
    palette: &'a [[u8; 4]],
    first_visible: usize,
    cache: Vec<u16>,
}

impl<'a> Matcher<'a> {
    const EMPTY: u16 = u16::MAX;

    fn new(palette: &'a [[u8; 4]]) -> Self {
        let first_visible = usize::from(palette.first() == Some(&TRANSPARENT));
        Matcher { palette, first_visible, cache: vec![Self::EMPTY; 1 << 20] }
    }

    fn nearest(&mut self, colour: [u8; 4]) -> usize {
        let key = colour.iter().fold(0usize, |key, channel| (key << 5) | usize::from(channel >> 3));
        if self.cache[key] == Self::EMPTY {
            let centre = colour.map(|channel| (channel & !7) | 4);
            let distance = |entry: &[u8; 4]| entry.iter().zip(centre).map(|(a, b)| (i32::from(*a) - i32::from(b)).pow(2)).sum::<i32>();
            let best = (self.first_visible..self.palette.len()).min_by_key(|index| distance(&self.palette[*index])).unwrap_or(0);
            self.cache[key] = best as u16;
        }
        usize::from(self.cache[key])
    }
}

/// Floyd–Steinberg dithering onto the palette, in integer arithmetic. Invisible pixels take the
/// transparent entry and pass no error on; visible pixels never become invisible.
fn dither(pixels: &[u8], width: usize, palette: &[[u8; 4]]) -> Vec<u8> {
    let mut matcher = Matcher::new(palette);
    let mut indices = Vec::with_capacity(pixels.len() / 4);
    let mut current = vec![[0i32; 4]; width + 2];
    let mut next = vec![[0i32; 4]; width + 2];
    for row in pixels.chunks_exact(width * 4) {
        for (x, pixel) in row.chunks_exact(4).enumerate() {
            if pixel[3] == 0 {
                indices.push(0);
                continue;
            }
            let error = current[x + 1];
            let wanted: [u8; 4] = std::array::from_fn(|channel| (i32::from(pixel[channel]) + error[channel] / 16).clamp(0, 255) as u8);
            let index = matcher.nearest(wanted);
            indices.push(index as u8);
            for channel in 0..4 {
                let difference = i32::from(wanted[channel]) - i32::from(palette[index][channel]);
                current[x + 2][channel] += difference * 7;
                next[x][channel] += difference * 3;
                next[x + 1][channel] += difference * 5;
                next[x + 2][channel] += difference;
            }
        }
        std::mem::swap(&mut current, &mut next);
        next.iter_mut().for_each(|error| *error = [0; 4]);
    }
    indices
}

fn distinct_colours(image: &DynamicImage, limit: usize) -> usize {
    let mut colours = HashSet::new();
    for pixel in image.to_rgba16().pixels() {
        if colours.insert(pixel.0) && colours.len() > limit { break; }
    }
    colours.len()
}

/// Cuts the image to at most 256 colours and repacks it with the carried chunks.
fn quantize(image: &DynamicImage, chunks: &[Chunk], strip_exif: bool, effort: u8) -> Result<Vec<u8>, ToolError> {
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let palette = learn_palette(rgba.as_raw());
    let indices = dither(rgba.as_raw(), width as usize, &palette);
    let palette = palette.iter().map(|[r, g, b, a]| RGBA8::new(*r, *g, *b, *a)).collect();
    let mut raw = RawImage::new(width, height, ColorType::Indexed { palette }, BitDepth::Eight, indices).map_err(optimize_error)?;
    chunks.iter()
        .filter(|chunk| COLOUR_SPACE.contains(&&chunk.name) || (!strip_exif && chunk.is_descriptive()))
        .for_each(|chunk| raw.add_png_chunk(chunk.name, chunk.data().to_vec()));
    raw.create_optimized_png(&options(effort)).map_err(optimize_error)
}

/// The compressed bytes and whether the pixels were kept exactly.
fn recompress_png(source: &[u8], method: PngMethod, strip_exif: bool) -> Result<(Vec<u8>, bool), ToolError> {
    let chunks = read_chunks(source)?;
    let (image, _) = decode(source).map_err(ToolError::processing)?;
    let stripped;
    let input = if strip_exif { stripped = without_descriptive(&chunks); &stripped[..] } else { source };
    match method {
        PngMethod::Palette { effort } if distinct_colours(&image, 256) > 256 => Ok((quantize(&image, &chunks, strip_exif, effort)?, false)),
        PngMethod::Lossless { effort } | PngMethod::Palette { effort } => {
            Ok((oxipng::optimize_from_memory(input, &options(effort)).map_err(optimize_error)?, true))
        }
    }
}

/// Checks a written copy is a still PNG at the source's size that keeps the colour profile and,
/// when Strip EXIF is on, carries no EXIF or text. A lossless copy must have the same pixels; a
/// palette copy must use at most 256 colours, keep invisible pixels invisible, and keep alpha.
fn verify(copy: &[u8], source: &[u8], lossless: bool, strip_exif: bool) -> Result<(), String> {
    let chunks = read_chunks(copy).map_err(|_| "The compressed copy isn't a readable still PNG.".to_string())?;
    let (source_image, source_icc) = decode(source)?;
    let (copy_image, copy_icc) = decode(copy).map_err(|_| "The compressed copy could not be opened.".to_string())?;
    if (copy_image.width(), copy_image.height()) != (source_image.width(), source_image.height()) { return Err("The compressed copy changed the image's size.".to_string()); }
    if copy_icc != source_icc { return Err("The compressed copy lost its colour profile.".to_string()); }
    if strip_exif && chunks.iter().any(Chunk::is_descriptive) { return Err("The compressed copy still carries EXIF.".to_string()); }
    if lossless {
        if copy_image.to_rgba16() != source_image.to_rgba16() { return Err("The compressed copy changed the image's pixels.".to_string()); }
        return Ok(());
    }
    if distinct_colours(&copy_image, 256) > 256 { return Err("The compressed copy has more than 256 colours.".to_string()); }
    let (source_pixels, copy_pixels) = (source_image.to_rgba8(), copy_image.to_rgba8());
    let invisible_kept = source_pixels.pixels().zip(copy_pixels.pixels()).all(|(source, copy)| (source[3] == 0) == (copy[3] == 0));
    let translucent = source_pixels.pixels().any(|pixel| pixel[3] < 255);
    if !invisible_kept || (translucent && !copy_image.color().has_alpha()) {
        return Err("The compressed copy lost the image's transparency.".to_string());
    }
    Ok(())
}

fn detail(lossless: bool, strip_exif: bool) -> String {
    let effect = if lossless { "Lossless, pixels unchanged." } else { "Cut to 256 colours, dithered." };
    let metadata = if strip_exif { "EXIF removed, colour profile kept." } else { "EXIF and colour profile kept." };
    format!("{effect} {metadata}")
}

/// Compresses one PNG into a new "-compressed" copy beside the input or in the chosen folder.
/// The original is only read; a copy that would not be smaller is discarded and Skipped.
pub fn compress_png(input: PathBuf, location: &OutputLocation, method: PngMethod, strip_exif: bool) -> JobOutcome {
    let source = match std::fs::read(&input) {
        Ok(bytes) => bytes,
        Err(error) => return JobOutcome::failure(input, ToolError::processing(format!("Could not read this PNG: {error}"))),
    };
    let (compressed, lossless) = match recompress_png(&source, method, strip_exif) {
        Ok(result) => result,
        Err(error) => return JobOutcome::failure(input, error),
    };
    save_smaller_copy(input, location, "PNG", &source, compressed, |written| verify(written, &source, lossless, strip_exif), detail(lossless, strip_exif))
}
