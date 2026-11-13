//! Recompresses the photographs embedded in a PDF for Compress Files at Medium and High.
//!
//! Only image XObject streams change. Text, fonts, vector content, page boxes and rotation are
//! never touched, and an image is replaced only when its new stream is smaller than the old one.

use std::collections::{HashMap, HashSet};

use image::imageops::FilterType;
use image::{DynamicImage, GrayImage, RgbImage};
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

/// JPEG quality and the highest resolution, in pixels per inch as drawn on the page, that a
/// recompressed PDF image keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSettings {
    pub quality: u8,
    pub max_ppi: u32,
}

pub const MEDIUM: ImageSettings = ImageSettings { quality: 75, max_ppi: 150 };
pub const HIGH: ImageSettings = ImageSettings { quality: 50, max_ppi: 96 };

/// Decoded samples above this are left alone rather than decoded into memory.
const MAX_DECODED_BYTES: usize = 200_000_000;
/// Flat graphics, charts and scanned line art have few colours and stay lossless; JPEG would blur them.
const MIN_PHOTO_COLOURS: usize = 257;
const MIN_PHOTO_GREYS: usize = 65;
/// Form XObjects nest at most this deep before their images count as unmeasured.
const MAX_FORM_DEPTH: usize = 12;

/// Re-encodes every eligible photographic image in `document` as JPEG at `settings`, downsampling
/// images drawn above `settings.max_ppi`. Returns how many image streams were replaced.
pub fn recompress_images(document: &mut Document, settings: ImageSettings) -> usize {
    let masks = mask_streams(document);
    let drawn = drawn_sizes(document);
    let candidates = document.objects.iter()
        .filter(|(id, object)| !masks.contains(id) && object.as_stream().is_ok_and(is_image))
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    let mut replaced = 0;
    for id in candidates {
        let Ok(stream) = document.get_object(id).and_then(Object::as_stream) else { continue };
        let size = drawn.as_ref().and_then(|drawn| drawn.get(&id).copied());
        let Some((jpeg, width, height)) = recompressed(document, stream, size, settings) else { continue };
        let Ok(stream) = document.get_object_mut(id).and_then(Object::as_stream_mut) else { continue };
        stream.dict.set("Filter", "DCTDecode");
        stream.dict.remove(b"DecodeParms");
        stream.dict.set("Width", width as i64);
        stream.dict.set("Height", height as i64);
        stream.dict.set("BitsPerComponent", 8);
        stream.set_content(jpeg);
        stream.allows_compression = false;
        replaced += 1;
    }
    replaced
}

fn is_image(stream: &Stream) -> bool {
    stream.dict.get(b"Subtype").and_then(Object::as_name).is_ok_and(|name| name == b"Image")
}

/// Every stream another image uses as its soft mask or stencil mask.
fn mask_streams(document: &Document) -> HashSet<ObjectId> {
    document.objects.values()
        .filter_map(|object| object.as_stream().ok().filter(|stream| is_image(stream)))
        .flat_map(|stream| [b"SMask".as_slice(), b"Mask"].into_iter().filter_map(|key| stream.dict.get(key).and_then(Object::as_reference).ok()))
        .collect()
}

/// The new JPEG and its pixel size, or None when the image must stay as it is.
fn recompressed(document: &Document, stream: &Stream, drawn: Option<(f64, f64)>, settings: ImageSettings) -> Option<(Vec<u8>, u32, u32)> {
    let dict = &stream.dict;
    if dict.get(b"ImageMask").and_then(Object::as_bool).unwrap_or(false)
        || dict.has(b"Mask") || dict.has(b"Decode") || dict.has(b"SMaskInData") {
        return None;
    }
    // A soft mask with Matte must keep its parent's pixel size, so its parent is not resized or re-encoded.
    if let Ok(smask) = dict.get(b"SMask") {
        let smask = super::resolve(document, smask).ok()?.as_stream().ok()?;
        if smask.dict.has(b"Matte") { return None; }
    }
    if integer(document, dict, b"BitsPerComponent")? != 8 { return None; }
    let width = u32::try_from(integer(document, dict, b"Width")?).ok().filter(|width| *width > 0)?;
    let height = u32::try_from(integer(document, dict, b"Height")?).ok().filter(|height| *height > 0)?;
    let channels = channels(document, dict.get(b"ColorSpace").ok()?)?;
    let samples = (width as usize).checked_mul(height as usize)?.checked_mul(channels)?;
    if samples > MAX_DECODED_BYTES { return None; }

    let image = match filters(document, dict)?.as_slice() {
        [] | [b"FlateDecode"] => {
            let data = stream.get_plain_content_with_limit(MAX_DECODED_BYTES).ok()?;
            let data = data.get(..samples)?.to_vec();
            let image = if channels == 1 {
                DynamicImage::ImageLuma8(GrayImage::from_raw(width, height, data)?)
            } else {
                DynamicImage::ImageRgb8(RgbImage::from_raw(width, height, data)?)
            };
            if !is_photographic(&image) { return None; }
            image
        }
        [b"DCTDecode"] if !dict.has(b"DecodeParms") => {
            let image = image::load_from_memory_with_format(&stream.content, image::ImageFormat::Jpeg).ok()?;
            let matches = match &image {
                DynamicImage::ImageLuma8(_) => channels == 1,
                DynamicImage::ImageRgb8(_) => channels == 3,
                _ => false,
            };
            if !matches || image.width() != width || image.height() != height { return None; }
            image
        }
        _ => return None,
    };

    let (target_width, target_height) = match drawn {
        Some((inches_wide, inches_high)) => (target(width, inches_wide, settings.max_ppi), target(height, inches_high, settings.max_ppi)),
        None => (width, height),
    };
    let image = if (target_width, target_height) == (width, height) { image } else { image.resize_exact(target_width, target_height, FilterType::Lanczos3) };
    let jpeg = crate::kit::images::encode_jpeg(&image, settings.quality).ok()?;
    (jpeg.len() < stream.content.len()).then_some((jpeg, target_width, target_height))
}

/// Pixels needed along one axis to show `inches` at `max_ppi`, never more than the image has.
pub(crate) fn target(pixels: u32, inches: f64, max_ppi: u32) -> u32 {
    let needed = (inches * max_ppi as f64 - 1e-6).ceil();
    if !needed.is_finite() || needed >= pixels as f64 { pixels } else { (needed as u32).max(1) }
}

fn integer(document: &Document, dict: &Dictionary, key: &[u8]) -> Option<i64> {
    super::resolve(document, dict.get(key).ok()?).ok()?.as_i64().ok()
}

/// Filter names in decoding order; None when a filter entry is malformed.
fn filters<'a>(document: &'a Document, dict: &'a Dictionary) -> Option<Vec<&'a [u8]>> {
    let Ok(filter) = dict.get(b"Filter") else { return Some(Vec::new()) };
    match super::resolve(document, filter).ok()? {
        Object::Name(name) => Some(vec![name.as_slice()]),
        Object::Array(names) => names.iter().map(|name| name.as_name().ok()).collect(),
        _ => None,
    }
}

/// 1 or 3 for colour spaces a JPEG round-trips as the same colours; None for everything else
/// (CMYK, indexed, separations, Lab and calibrated spaces).
fn channels(document: &Document, colour_space: &Object) -> Option<usize> {
    match super::resolve(document, colour_space).ok()? {
        Object::Name(name) if name == b"DeviceGray" => Some(1),
        Object::Name(name) if name == b"DeviceRGB" => Some(3),
        Object::Array(parts) => match parts.as_slice() {
            [family, profile] if family.as_name().is_ok_and(|name| name == b"ICCBased") => {
                let profile = super::resolve(document, profile).ok()?.as_stream().ok()?;
                match integer(document, &profile.dict, b"N")? { 1 => Some(1), 3 => Some(3), _ => None }
            }
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn is_photographic(image: &DynamicImage) -> bool {
    fn distinct_at_least<T: std::hash::Hash + Eq>(values: impl Iterator<Item = T>, wanted: usize) -> bool {
        let mut seen = HashSet::new();
        values.into_iter().any(|value| { seen.insert(value); seen.len() >= wanted })
    }
    match image {
        DynamicImage::ImageLuma8(grey) => distinct_at_least(grey.as_raw().iter(), MIN_PHOTO_GREYS),
        DynamicImage::ImageRgb8(rgb) => distinct_at_least(rgb.as_raw().chunks_exact(3), MIN_PHOTO_COLOURS),
        _ => false,
    }
}

type Matrix = [f64; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `first` then `second`, in PDF's row-vector order.
fn multiply(first: Matrix, second: Matrix) -> Matrix {
    let [a, b, c, d, e, f] = first;
    let [a2, b2, c2, d2, e2, f2] = second;
    [a * a2 + b * c2, a * b2 + b * d2, c * a2 + d * c2, c * b2 + d * d2, e * a2 + f * c2 + e2, e * b2 + f * d2 + f2]
}

/// The largest width and height, in inches, each image XObject is drawn at by page content and the
/// forms it calls. None when some page content cannot be read, so nothing is downsampled.
fn drawn_sizes(document: &Document) -> Option<HashMap<ObjectId, (f64, f64)>> {
    let mut sizes = HashMap::new();
    for page in document.get_pages().into_values() {
        let user_unit = super::inherited(document, page, b"UserUnit").ok()?
            .and_then(|unit| number(&unit)).filter(|unit| *unit > 0.0).unwrap_or(1.0);
        let resources = super::inherited(document, page, b"Resources").ok()?.and_then(|value| value.as_dict().ok().cloned()).unwrap_or_default();
        let content = Content::decode(&document.get_page_content_with_limit(page, MAX_DECODED_BYTES).ok()?).ok()?;
        let scale = [user_unit, 0.0, 0.0, user_unit, 0.0, 0.0];
        walk(document, &content, &resources, scale, &mut Vec::new(), &mut sizes)?;
    }
    Some(sizes)
}

fn walk(document: &Document, content: &Content, resources: &Dictionary, base: Matrix, forms: &mut Vec<ObjectId>, sizes: &mut HashMap<ObjectId, (f64, f64)>) -> Option<()> {
    let xobjects = resources.get(b"XObject").ok().and_then(|value| super::resolve(document, value).ok()).and_then(|value| value.as_dict().ok());
    let mut stack = Vec::new();
    let mut ctm = base;
    for operation in &content.operations {
        match operation.operator.as_str() {
            "q" => stack.push(ctm),
            "Q" => ctm = stack.pop().unwrap_or(base),
            "cm" => ctm = multiply(matrix(&operation.operands)?, ctm),
            "Do" => {
                let name = operation.operands.first()?.as_name().ok()?;
                let Some(id) = xobjects.and_then(|xobjects| xobjects.get(name).ok()).and_then(|value| value.as_reference().ok()) else { continue };
                let Ok(stream) = document.get_object(id).and_then(Object::as_stream) else { continue };
                if is_image(stream) {
                    let inches = |x: f64, y: f64| x.hypot(y) / 72.0;
                    let (wide, high) = (inches(ctm[0], ctm[1]), inches(ctm[2], ctm[3]));
                    let entry = sizes.entry(id).or_insert((0.0, 0.0));
                    *entry = (entry.0.max(wide), entry.1.max(high));
                } else if stream.dict.get(b"Subtype").and_then(Object::as_name).is_ok_and(|name| name == b"Form") {
                    if forms.contains(&id) { continue; }
                    if forms.len() >= MAX_FORM_DEPTH { return None; }
                    let form_matrix = stream.dict.get(b"Matrix").ok().map(|value| super::resolve(document, value).ok().and_then(|value| value.as_array().ok()).and_then(|values| matrix(values))).unwrap_or(Some(IDENTITY))?;
                    let form_resources = match stream.dict.get(b"Resources") {
                        Ok(value) => super::resolve(document, value).ok()?.as_dict().ok()?.clone(),
                        Err(_) => resources.clone(),
                    };
                    let form_content = Content::decode(&stream.get_plain_content_with_limit(MAX_DECODED_BYTES).ok()?).ok()?;
                    forms.push(id);
                    walk(document, &form_content, &form_resources, multiply(form_matrix, ctm), forms, sizes)?;
                    forms.pop();
                }
            }
            _ => {}
        }
    }
    Some(())
}

fn matrix(operands: &[Object]) -> Option<Matrix> {
    let values = operands.iter().map(number).collect::<Option<Vec<_>>>()?;
    values.try_into().ok()
}

fn number(value: &Object) -> Option<f64> {
    match value {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some(*value as f64),
        _ => None,
    }
}
