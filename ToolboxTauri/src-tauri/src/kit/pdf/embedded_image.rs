use image::{DynamicImage, ImageBuffer, ImageFormat};
use lopdf::{dictionary, Dictionary, Document, Object, Stream};
use std::io::Cursor;

/// The most bytes one image may take once its filters are undone, or once its pixels are decoded.
pub(crate) const MAX_DECODED_BYTES: u64 = 500_000_000;

/// What Extract from PDF saves for one embedded image.
#[derive(Debug, PartialEq)]
pub(crate) enum Extracted {
    /// The original JPEG bytes. `cmyk` and `inverted` flag JPEGs that some viewers show with the wrong colours.
    Jpeg { data: Vec<u8>, cmyk: bool, inverted: bool },
    /// The original JPEG 2000 codestream or file bytes.
    Jpeg2000(Vec<u8>),
    /// A lossless PNG of the decoded pixels at the image's own size.
    Png(Vec<u8>),
}

impl Extracted {
    pub(crate) fn extension(&self) -> &'static str {
        match self {
            Self::Jpeg { .. } => "jpg",
            Self::Jpeg2000(_) => "jp2",
            Self::Png(_) => "png",
        }
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        match self {
            Self::Jpeg { data, .. } | Self::Jpeg2000(data) | Self::Png(data) => data,
        }
    }
}

/// Why one embedded image was not saved.
#[derive(Debug, PartialEq)]
pub(crate) enum NotSaved {
    Unsupported(String),
    Damaged,
    TooLarge { width: u64, height: u64 },
}

/// Converts one image XObject. JPEG and JPEG 2000 data is kept byte for byte; everything else is decoded to PNG.
///
/// CMYK becomes RGB with the fixed formula R = round((M − C)(M − K) / M), G and B likewise with magenta and
/// yellow, where M is 255 for 8-bit output and 65535 for 16-bit output. No colour profile is applied.
pub(crate) fn extract(document: &Document, image: &Stream) -> Result<Extracted, NotSaved> {
    let dict = &image.dict;
    let (width, height) = dimensions(dict)?;
    let (data, encoded) = undo_filters(document, image, (width, height))?;
    match encoded {
        Some(Encoded::Jpeg) => {
            let space = dict.get(b"ColorSpace").ok().map(|space| colour_space(document, space));
            let cmyk = matches!(space, Some(Ok(Space::Cmyk)));
            let inverted = numbers(document, dict.get(b"Decode").ok()).is_some_and(|decode| decode.len() >= 2 && decode[0] == 1.0 && decode[1] == 0.0);
            Ok(Extracted::Jpeg { data, cmyk, inverted })
        }
        Some(Encoded::Jpeg2000) => Ok(Extracted::Jpeg2000(data)),
        None => decode_pixels(document, dict, &data, (width, height)).and_then(encode_png).map(Extracted::Png),
    }
}

enum Encoded { Jpeg, Jpeg2000 }

fn dimensions(dict: &Dictionary) -> Result<(u64, u64), NotSaved> {
    let side = |key: &[u8]| dict.get(key).and_then(Object::as_i64).ok().filter(|value| *value > 0).map(|value| value as u64).ok_or(NotSaved::Damaged);
    Ok((side(b"Width")?, side(b"Height")?))
}

fn names<'a>(document: &'a Document, object: Option<&'a Object>) -> Result<Vec<&'a [u8]>, NotSaved> {
    match object.map(|object| document.dereference(object).map(|(_, object)| object)) {
        None => Ok(Vec::new()),
        Some(Ok(Object::Name(name))) => Ok(vec![name.as_slice()]),
        Some(Ok(Object::Array(items))) => items.iter().map(|item| document.dereference(item).ok().and_then(|(_, item)| item.as_name().ok()).ok_or(NotSaved::Damaged)).collect(),
        Some(_) => Err(NotSaved::Damaged),
    }
}

fn numbers(document: &Document, object: Option<&Object>) -> Option<Vec<f64>> {
    let (_, object) = document.dereference(object?).ok()?;
    object.as_array().ok()?.iter().map(|item| item.as_float().map(f64::from).or_else(|_| item.as_i64().map(|value| value as f64)).ok()).collect()
}

/// Undoes the filters in order. A final DCT or JPX filter is left in place and reported, so that data is kept as it is.
fn undo_filters(document: &Document, image: &Stream, (width, height): (u64, u64)) -> Result<(Vec<u8>, Option<Encoded>), NotSaved> {
    let filters = names(document, image.dict.get(b"Filter").ok())?;
    let params = image.dict.get(b"DecodeParms").ok().map(|object| document.dereference(object).map(|(_, object)| object.clone()));
    let mut data = image.content.clone();
    for (index, filter) in filters.iter().enumerate() {
        let last = index + 1 == filters.len();
        let parms = match &params {
            Some(Ok(Object::Array(items))) => items.get(index).and_then(|item| document.dereference(item).ok()).and_then(|(_, item)| item.as_dict().ok()).cloned(),
            Some(Ok(Object::Dictionary(parms))) if filters.len() == 1 => Some(parms.clone()),
            _ => None,
        };
        data = match *filter {
            b"DCTDecode" | b"DCT" if last => return Ok((data, Some(Encoded::Jpeg))),
            b"JPXDecode" if last => return Ok((data, Some(Encoded::Jpeg2000))),
            b"DCTDecode" | b"DCT" | b"JPXDecode" => return Err(NotSaved::Damaged),
            b"FlateDecode" | b"Fl" => lopdf_filter(b"FlateDecode", parms, data, (width, height))?,
            b"LZWDecode" | b"LZW" => lopdf_filter(b"LZWDecode", parms, data, (width, height))?,
            b"ASCII85Decode" | b"A85" => lopdf_filter(b"ASCII85Decode", parms, data, (width, height))?,
            b"ASCIIHexDecode" | b"AHx" => ascii_hex(&data)?,
            b"RunLengthDecode" | b"RL" => run_length(&data, (width, height))?,
            b"JBIG2Decode" => jbig2(document, parms.as_ref(), &data, (width, height))?,
            b"CCITTFaxDecode" | b"CCF" => ccitt_fax(parms.as_ref(), &data, (width, height))?,
            other => return Err(NotSaved::Unsupported(String::from_utf8_lossy(other).into_owned())),
        };
    }
    Ok((data, None))
}

fn lopdf_filter(filter: &[u8], parms: Option<Dictionary>, data: Vec<u8>, (width, height): (u64, u64)) -> Result<Vec<u8>, NotSaved> {
    let mut dict = dictionary! { "Filter" => Object::Name(filter.to_vec()) };
    if let Some(parms) = parms { dict.set("DecodeParms", parms); }
    match Stream::new(dict, data).decompressed_content_with_limit(MAX_DECODED_BYTES as usize) {
        Ok(data) => Ok(data),
        Err(lopdf::Error::Decompress(lopdf::DecompressError::MemoryLimitExceeded { .. })) => Err(NotSaved::TooLarge { width, height }),
        Err(_) => Err(NotSaved::Damaged),
    }
}

fn ascii_hex(data: &[u8]) -> Result<Vec<u8>, NotSaved> {
    let mut digits = Vec::new();
    for &byte in data {
        match byte {
            b'>' => break,
            byte if byte.is_ascii_whitespace() => {}
            byte => digits.push(char::from(byte).to_digit(16).ok_or(NotSaved::Damaged)? as u8),
        }
    }
    if digits.len() % 2 == 1 { digits.push(0); }
    Ok(digits.chunks(2).map(|pair| pair[0] << 4 | pair[1]).collect())
}

fn run_length(data: &[u8], (width, height): (u64, u64)) -> Result<Vec<u8>, NotSaved> {
    let mut output = Vec::new();
    let mut index = 0;
    while let Some(&length) = data.get(index) {
        match length {
            128 => break,
            0..=127 => {
                let run = data.get(index + 1..index + 2 + usize::from(length)).ok_or(NotSaved::Damaged)?;
                output.extend_from_slice(run);
                index += 2 + usize::from(length);
            }
            _ => {
                let byte = *data.get(index + 1).ok_or(NotSaved::Damaged)?;
                output.extend(std::iter::repeat_n(byte, 257 - usize::from(length)));
                index += 2;
            }
        }
        if output.len() as u64 > MAX_DECODED_BYTES { return Err(NotSaved::TooLarge { width, height }); }
    }
    Ok(output)
}

/// Decodes CCITT Group 3 (`K` = 0 one-dimensional, `K` > 0 mixed) or Group 4 (`K` < 0) fax data into 1-bit rows that
/// each start on a byte boundary, so the image's own colour space, `Decode` array or `ImageMask` then applies as usual.
///
/// Black is written as 0 and white as 1 unless `BlackIs1` is true. `EndOfLine`, `EncodedByteAlign` and `EndOfBlock` are
/// honoured. Rows are decoded `Columns` wide (1728 when absent) and then cut or padded with white to the image's width.
/// Data that ends before the image's height is reached counts as damaged.
fn ccitt_fax(parms: Option<&Dictionary>, data: &[u8], (width, height): (u64, u64)) -> Result<Vec<u8>, NotSaved> {
    use hayro_ccitt::{DecodeSettings, DecoderContext, EncodingMode};
    let entry = |key: &[u8]| parms.and_then(|parms| parms.get(key).ok());
    let number = |key: &[u8], default: i64| entry(key).and_then(|value| value.as_i64().ok()).unwrap_or(default);
    let flag = |key: &[u8], default: bool| entry(key).and_then(|value| value.as_bool().ok()).unwrap_or(default);
    let row_bytes = width.div_ceil(8);
    if row_bytes.saturating_mul(height) > MAX_DECODED_BYTES { return Err(NotSaved::TooLarge { width, height }); }
    let columns = u32::try_from(number(b"Columns", 1728)).ok().filter(|columns| *columns > 0).ok_or(NotSaved::Damaged)?;
    let k = number(b"K", 0);
    let settings = DecodeSettings {
        columns,
        rows: u32::try_from(height).map_err(|_| NotSaved::TooLarge { width, height })?,
        end_of_block: flag(b"EndOfBlock", true),
        end_of_line: flag(b"EndOfLine", false),
        rows_are_byte_aligned: flag(b"EncodedByteAlign", false),
        encoding: if k < 0 { EncodingMode::Group4 } else if k == 0 { EncodingMode::Group3_1D } else { EncodingMode::Group3_2D },
        strict: false,
    };
    let mut rows = FaxRows { width: width as usize, row_bytes: row_bytes as usize, height, black_is_one: flag(b"BlackIs1", false), rows: 0, data: Vec::new() };
    rows.data.reserve((row_bytes * height) as usize);
    // Rows decoded before an error are kept, so the image is damaged only when rows are missing.
    let _ = hayro_ccitt::decode(data, &mut rows, &mut DecoderContext::new(settings));
    if rows.rows < height { return Err(NotSaved::Damaged); }
    Ok(rows.data)
}

/// Packs the rows hayro-ccitt reports as colour changes into 1-bit rows of the image's width.
struct FaxRows {
    width: usize,
    row_bytes: usize,
    height: u64,
    black_is_one: bool,
    rows: u64,
    data: Vec<u8>,
}

impl hayro_ccitt::Decoder for FaxRows {
    fn push_row(&mut self, changes: &[u32]) {
        if self.rows == self.height { return; }
        self.rows += 1;
        let start = self.data.len();
        self.data.resize(start + self.row_bytes, if self.black_is_one { 0x00 } else { 0xFF });
        let row = &mut self.data[start..];
        for span in changes.chunks(2) {
            let from = (span[0] as usize).min(self.width);
            let to = span.get(1).map_or(self.width, |end| (*end as usize).min(self.width));
            for x in from..to {
                if self.black_is_one { row[x / 8] |= 0x80 >> (x % 8) } else { row[x / 8] &= !(0x80 >> (x % 8)) }
            }
        }
    }
}

/// Decodes a JBIG2 page in the embedded organisation, with the segments of its `JBIG2Globals` stream if it has one, into
/// 1-bit rows that each start on a byte boundary. Black is written as 0, as the PDF `JBIG2Decode` filter defines it, so
/// the image's own colour space, `Decode` array or `ImageMask` then applies as usual.
///
/// Decoding is strict: hayro-jbig2's lenient mode skips a region it cannot decode and returns the rest of the page, which
/// would save a blank or partial image without a word. So, as with fax data that ends early, any decoding error counts
/// as damaged, except JBIG2 features the decoder does not handle (colour extension regions, required extension
/// segments), which are reported as unsupported. Rows are cut or padded with white to the image's width; a page shorter
/// than the image counts as damaged.
fn jbig2(document: &Document, parms: Option<&Dictionary>, data: &[u8], (width, height): (u64, u64)) -> Result<Vec<u8>, NotSaved> {
    use hayro_jbig2::{DecodeError, DecodeSettings, Globals, Page};
    let row_bytes = width.div_ceil(8);
    if row_bytes.saturating_mul(height) > MAX_DECODED_BYTES { return Err(NotSaved::TooLarge { width, height }); }
    let failed = |error: DecodeError| match error {
        DecodeError::Unsupported => NotSaved::Unsupported("JBIG2".into()),
        _ => NotSaved::Damaged,
    };
    let settings = DecodeSettings { strict: true };
    let globals = match parms.and_then(|parms| parms.get(b"JBIG2Globals").ok()) {
        None => None,
        Some(object) => {
            let stream = document.dereference(object).ok().and_then(|(_, object)| object.as_stream().ok()).ok_or(NotSaved::Damaged)?;
            // A globals stream is plain segments; a JBIG2 filter on it could refer back to itself.
            if names(document, stream.dict.get(b"Filter").ok())?.contains(&&b"JBIG2Decode"[..]) { return Err(NotSaved::Damaged); }
            let (segments, encoded) = undo_filters(document, stream, (width, height))?;
            if encoded.is_some() { return Err(NotSaved::Damaged); }
            Some(Globals::new(&segments, &settings).map_err(failed)?)
        }
    };
    let page = Page::new_embedded(data, globals.as_ref(), &settings).map_err(failed)?;
    let (page_width, page_height) = (u64::from(page.width()), u64::from(page.height()));
    if page_width.div_ceil(8).saturating_mul(page_height) > MAX_DECODED_BYTES { return Err(NotSaved::TooLarge { width: page_width, height: page_height }); }
    if page_height < height { return Err(NotSaved::Damaged); }
    let mut rows = Jbig2Rows { width: width as usize, row_bytes: row_bytes as usize, height: height as usize, x: 0, y: 0, data: vec![0xFF; (row_bytes * height) as usize] };
    page.decode(&mut rows).map_err(failed)?;
    Ok(rows.data)
}

/// Packs the pixels hayro-jbig2 reports into 1-bit rows of the image's size, with 0 for black.
struct Jbig2Rows {
    width: usize,
    row_bytes: usize,
    height: usize,
    x: usize,
    y: usize,
    data: Vec<u8>,
}

impl Jbig2Rows {
    fn black(&mut self, from: usize, count: usize) {
        if self.y >= self.height { return; }
        let row = &mut self.data[self.y * self.row_bytes..(self.y + 1) * self.row_bytes];
        for x in from..(from + count).min(self.width) { row[x / 8] &= !(0x80 >> (x % 8)); }
    }
}

impl hayro_jbig2::Decoder for Jbig2Rows {
    fn push_pixel(&mut self, black: bool) {
        if black { self.black(self.x, 1); }
        self.x += 1;
    }

    fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
        let count = chunk_count as usize * 8;
        if black { self.black(self.x, count); }
        self.x += count;
    }

    fn next_line(&mut self) {
        self.x = 0;
        self.y += 1;
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Space {
    Gray,
    Rgb,
    Cmyk,
    Indexed { base: Box<Space>, high: usize, lookup: Vec<u8> },
}

impl Space {
    fn components(&self) -> usize {
        match self {
            Self::Gray | Self::Indexed { .. } => 1,
            Self::Rgb => 3,
            Self::Cmyk => 4,
        }
    }
}

fn unsupported_space(name: &[u8]) -> NotSaved {
    let name = match name {
        b"Lab" => "Lab",
        b"Separation" => "Separation",
        b"DeviceN" => "DeviceN",
        b"Pattern" => "Pattern",
        other => return NotSaved::Unsupported(format!("The {} colour space", String::from_utf8_lossy(other))),
    };
    NotSaved::Unsupported(format!("{name} colour"))
}

fn colour_space(document: &Document, object: &Object) -> Result<Space, NotSaved> {
    let (_, object) = document.dereference(object).map_err(|_| NotSaved::Damaged)?;
    let (family, items) = match object {
        Object::Name(name) => (name.as_slice(), &[][..]),
        Object::Array(items) => (items.first().and_then(|item| item.as_name().ok()).ok_or(NotSaved::Damaged)?, &items[1..]),
        _ => return Err(NotSaved::Damaged),
    };
    match family {
        b"DeviceGray" | b"G" | b"CalGray" => Ok(Space::Gray),
        b"DeviceRGB" | b"RGB" | b"CalRGB" => Ok(Space::Rgb),
        b"DeviceCMYK" | b"CMYK" => Ok(Space::Cmyk),
        b"ICCBased" => {
            let profile = items.first().and_then(|item| document.dereference(item).ok()).and_then(|(_, item)| item.as_stream().ok()).ok_or(NotSaved::Damaged)?;
            match profile.dict.get(b"N").and_then(Object::as_i64) {
                Ok(1) => Ok(Space::Gray),
                Ok(3) => Ok(Space::Rgb),
                Ok(4) => Ok(Space::Cmyk),
                _ => Err(NotSaved::Damaged),
            }
        }
        b"Indexed" | b"I" => {
            let [base, high, lookup] = items else { return Err(NotSaved::Damaged) };
            let base = colour_space(document, base)?;
            if matches!(base, Space::Indexed { .. }) { return Err(NotSaved::Damaged); }
            let high = document.dereference(high).ok().and_then(|(_, high)| high.as_i64().ok()).filter(|high| (0..=255).contains(high)).ok_or(NotSaved::Damaged)? as usize;
            let lookup = match document.dereference(lookup).map_err(|_| NotSaved::Damaged)?.1 {
                Object::String(bytes, _) => bytes.clone(),
                Object::Stream(stream) => undo_filters(document, stream, (1, 1)).and_then(|(data, encoded)| if encoded.is_none() { Ok(data) } else { Err(NotSaved::Damaged) })?,
                _ => return Err(NotSaved::Damaged),
            };
            if lookup.len() < (high + 1) * base.components() { return Err(NotSaved::Damaged); }
            Ok(Space::Indexed { base: Box::new(base), high, lookup })
        }
        other => Err(unsupported_space(other)),
    }
}

/// Decoded channels at one bit depth: 8-bit values in `narrow`, 16-bit values in `wide`.
struct Pixels {
    width: u32,
    height: u32,
    channels: usize,
    max: u32,
    narrow: Vec<u8>,
    wide: Vec<u16>,
}

impl Pixels {
    fn push(&mut self, value: u32) {
        if self.max == 255 { self.narrow.push(value as u8) } else { self.wide.push(value as u16) }
    }
}

/// The samples of a packed image: rows start on a byte boundary, components are interleaved.
struct Samples<'a> {
    data: &'a [u8],
    row_bytes: usize,
    bits: u8,
    per_pixel: usize,
}

impl Samples<'_> {
    fn new(data: &[u8], (width, height): (u64, u64), per_pixel: usize, bits: u8) -> Result<Samples<'_>, NotSaved> {
        let row_bits = width.checked_mul(per_pixel as u64 * u64::from(bits)).ok_or(NotSaved::TooLarge { width, height })?;
        let row_bytes = row_bits.div_ceil(8);
        let needed = row_bytes.checked_mul(height).ok_or(NotSaved::TooLarge { width, height })?;
        if needed > MAX_DECODED_BYTES { return Err(NotSaved::TooLarge { width, height }); }
        if (data.len() as u64) < needed { return Err(NotSaved::Damaged); }
        Ok(Samples { data, row_bytes: row_bytes as usize, bits, per_pixel })
    }

    fn get(&self, x: usize, y: usize, component: usize) -> u32 {
        let row = &self.data[y * self.row_bytes..];
        let index = x * self.per_pixel + component;
        match self.bits {
            8 => u32::from(row[index]),
            16 => u32::from(row[index * 2]) << 8 | u32::from(row[index * 2 + 1]),
            bits => {
                let bit = index * usize::from(bits);
                let byte = u32::from(row[bit / 8]);
                byte >> (8 - usize::from(bits) - bit % 8) & ((1 << bits) - 1)
            }
        }
    }

    fn top(&self) -> u32 {
        (1 << self.bits) - 1
    }
}

/// Maps a raw sample through the Decode range [low, high] onto 0..=max.
fn level(sample: u32, top: u32, (low, high): (f64, f64), max: u32) -> u32 {
    let value = low + f64::from(sample) * (high - low) / f64::from(top);
    (value.clamp(0.0, 1.0) * f64::from(max)).round() as u32
}

fn decode_ranges(document: &Document, dict: &Dictionary, components: usize, default: (f64, f64)) -> Result<Vec<(f64, f64)>, NotSaved> {
    match numbers(document, dict.get(b"Decode").ok()) {
        None => Ok(vec![default; components]),
        Some(values) if values.len() >= components * 2 => Ok(values.chunks(2).take(components).map(|pair| (pair[0], pair[1])).collect()),
        Some(_) => Err(NotSaved::Damaged),
    }
}

fn bits_per_component(dict: &Dictionary) -> Result<u8, NotSaved> {
    if dict.get(b"ImageMask").and_then(Object::as_bool).unwrap_or(false) { return Ok(1); }
    match dict.get(b"BitsPerComponent").and_then(Object::as_i64) {
        Ok(bits @ (1 | 2 | 4 | 8 | 16)) => Ok(bits as u8),
        _ => Err(NotSaved::Damaged),
    }
}

fn to_rgb(space: &Space, values: &[u32], max: u32) -> [u32; 3] {
    match space {
        Space::Gray => [values[0]; 3],
        Space::Cmyk => {
            let [c, m, y, k] = [values[0], values[1], values[2], values[3]].map(u64::from);
            let max = u64::from(max);
            [c, m, y].map(|ink| (((max - ink) * (max - k) + max / 2) / max) as u32)
        }
        _ => [values[0], values[1], values[2]],
    }
}

/// One alpha value per pixel of the base image, in the output's 0..=max range.
fn alpha_plane(document: &Document, dict: &Dictionary, samples: &Samples, base: (u64, u64), max: u32) -> Result<Option<Vec<u16>>, NotSaved> {
    let resolve = |key: &[u8]| dict.get(key).ok().map(|object| document.dereference(object).map(|(_, object)| object).map_err(|_| NotSaved::Damaged));
    if let Some(soft) = resolve(b"SMask") {
        let soft = soft?.as_stream().map_err(|_| NotSaved::Damaged)?;
        return gray_plane(document, soft, base, max, false).map(Some);
    }
    match resolve(b"Mask") {
        None => Ok(None),
        Some(Ok(Object::Stream(stencil))) => gray_plane(document, stencil, base, max, true).map(Some),
        Some(Ok(Object::Array(_))) => {
            let ranges = numbers(document, dict.get(b"Mask").ok()).ok_or(NotSaved::Damaged)?;
            if ranges.len() < samples.per_pixel * 2 { return Err(NotSaved::Damaged); }
            let (width, height) = (base.0 as usize, base.1 as usize);
            let mut alpha = Vec::with_capacity(width * height);
            for y in 0..height {
                for x in 0..width {
                    let keyed = (0..samples.per_pixel).all(|component| {
                        let sample = f64::from(samples.get(x, y, component));
                        ranges[component * 2] <= sample && sample <= ranges[component * 2 + 1]
                    });
                    alpha.push(if keyed { 0 } else { max as u16 });
                }
            }
            Ok(Some(alpha))
        }
        Some(_) => Err(NotSaved::Damaged),
    }
}

/// Decodes a soft mask or stencil mask and scales it to the base image's size by nearest neighbour.
/// A stencil sample that decodes to 0 is painted (opaque); one that decodes to 1 is masked out.
fn gray_plane(document: &Document, mask: &Stream, (width, height): (u64, u64), max: u32, stencil: bool) -> Result<Vec<u16>, NotSaved> {
    let (mask_width, mask_height) = dimensions(&mask.dict)?;
    if mask_width.saturating_mul(mask_height).max(width.saturating_mul(height)) > MAX_DECODED_BYTES / 2 { return Err(NotSaved::TooLarge { width, height }); }
    let (data, encoded) = undo_filters(document, mask, (mask_width, mask_height))?;
    let values: Vec<u16> = match encoded {
        Some(Encoded::Jpeg) => {
            let gray = image::load_from_memory_with_format(&data, ImageFormat::Jpeg).map_err(|_| NotSaved::Damaged)?.into_luma16();
            if u64::from(gray.width()) != mask_width || u64::from(gray.height()) != mask_height { return Err(NotSaved::Damaged); }
            gray.into_raw().into_iter().map(|value| level(u32::from(value), 65535, (0.0, 1.0), max) as u16).collect()
        }
        Some(Encoded::Jpeg2000) => return Err(NotSaved::Unsupported("A JPEG 2000 soft mask".into())),
        None => {
            let bits = if stencil { 1 } else { bits_per_component(&mask.dict)? };
            let samples = Samples::new(&data, (mask_width, mask_height), 1, bits)?;
            let [range] = decode_ranges(document, &mask.dict, 1, (0.0, 1.0))?[..] else { return Err(NotSaved::Damaged) };
            let mut values = Vec::with_capacity((mask_width * mask_height) as usize);
            for y in 0..mask_height as usize {
                for x in 0..mask_width as usize {
                    let value = level(samples.get(x, y, 0), samples.top(), range, max);
                    values.push((if stencil { max - value } else { value }) as u16);
                }
            }
            values
        }
    };
    let mut scaled = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let row = (y * mask_height / height) * mask_width;
        for x in 0..width {
            scaled.push(values[(row + x * mask_width / width) as usize]);
        }
    }
    Ok(scaled)
}

fn decode_pixels(document: &Document, dict: &Dictionary, data: &[u8], (width, height): (u64, u64)) -> Result<Pixels, NotSaved> {
    let stencil = dict.get(b"ImageMask").and_then(Object::as_bool).unwrap_or(false);
    let space = if stencil { Space::Gray } else { colour_space(document, dict.get(b"ColorSpace").map_err(|_| NotSaved::Damaged)?)? };
    let bits = bits_per_component(dict)?;
    let samples = Samples::new(data, (width, height), space.components(), bits)?;
    let max = if bits == 16 && !matches!(space, Space::Indexed { .. }) { 65535 } else { 255 };
    let colour = match &space {
        Space::Gray => 1,
        Space::Indexed { base, .. } if **base == Space::Gray => 1,
        _ => 3,
    };
    let alpha = if stencil { None } else { alpha_plane(document, dict, &samples, (width, height), max)? };
    let channels = colour + usize::from(stencil || alpha.is_some());
    let output_bytes = width.saturating_mul(height).saturating_mul(channels as u64).saturating_mul(if max == 255 { 1 } else { 2 });
    if output_bytes > MAX_DECODED_BYTES { return Err(NotSaved::TooLarge { width, height }); }
    let default = match &space { Space::Indexed { .. } => (0.0, f64::from(samples.top())), _ => (0.0, 1.0) };
    let ranges = decode_ranges(document, dict, space.components(), default)?;
    let mut pixels = Pixels { width: width as u32, height: height as u32, channels, max, narrow: Vec::new(), wide: Vec::new() };
    if max == 255 { pixels.narrow.reserve(output_bytes as usize) } else { pixels.wide.reserve(output_bytes as usize / 2) }
    let mut values = vec![0; space.components()];
    for y in 0..height as usize {
        for x in 0..width as usize {
            if stencil {
                pixels.push(0);
                pixels.push(max - level(samples.get(x, y, 0), samples.top(), ranges[0], max));
                continue;
            }
            let rgb = match &space {
                Space::Indexed { base, high, lookup } => {
                    let (low, top) = ranges[0];
                    let index = (low + f64::from(samples.get(x, y, 0)) * (top - low) / f64::from(samples.top())).round().clamp(0.0, *high as f64) as usize;
                    let entry: Vec<u32> = lookup[index * base.components()..(index + 1) * base.components()].iter().map(|value| u32::from(*value)).collect();
                    to_rgb(base, &entry, max)
                }
                space => {
                    for (component, value) in values.iter_mut().enumerate() {
                        *value = level(samples.get(x, y, component), samples.top(), ranges[component], max);
                    }
                    to_rgb(space, &values, max)
                }
            };
            for value in &rgb[..colour] { pixels.push(*value); }
            if let Some(alpha) = &alpha { pixels.push(u32::from(alpha[y * width as usize + x])); }
        }
    }
    Ok(pixels)
}

fn encode_png(pixels: Pixels) -> Result<Vec<u8>, NotSaved> {
    let Pixels { width, height, channels, max, narrow, wide } = pixels;
    let image = match (channels, max != 255) {
        (1, false) => ImageBuffer::from_raw(width, height, narrow).map(DynamicImage::ImageLuma8),
        (2, false) => ImageBuffer::from_raw(width, height, narrow).map(DynamicImage::ImageLumaA8),
        (3, false) => ImageBuffer::from_raw(width, height, narrow).map(DynamicImage::ImageRgb8),
        (4, false) => ImageBuffer::from_raw(width, height, narrow).map(DynamicImage::ImageRgba8),
        (1, true) => ImageBuffer::from_raw(width, height, wide).map(DynamicImage::ImageLuma16),
        (2, true) => ImageBuffer::from_raw(width, height, wide).map(DynamicImage::ImageLumaA16),
        (3, true) => ImageBuffer::from_raw(width, height, wide).map(DynamicImage::ImageRgb16),
        (_, true) => ImageBuffer::from_raw(width, height, wide).map(DynamicImage::ImageRgba16),
        _ => None,
    }.ok_or(NotSaved::Damaged)?;
    let mut png = Vec::new();
    image.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).map_err(|_| NotSaved::Damaged)?;
    Ok(png)
}
