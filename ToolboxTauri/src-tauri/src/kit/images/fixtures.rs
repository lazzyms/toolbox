//! Code-generated image fixtures for orientation and HEIC tests.
//!
//! The bundled x265 encoder is an 8-bit build, so a high-bit-depth HEIC is made
//! by encoding an 8-bit HEIC and rewriting its HEVC sequence parameter set and
//! HEIF properties to declare 10 bits. libde265 then decodes the same slices as
//! a real 10-bit Main 10 stream. HEIF rotation is added the same way, as an
//! `irot` property, so libheif applies it while decoding.

use image::DynamicImage;

/// A landscape source whose left half is red and right half is blue, so every
/// orientation (and a double rotation) puts red somewhere distinct.
pub(crate) fn red_left_blue_right(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(image::RgbImage::from_fn(width, height, |x, _| if x < width / 2 { image::Rgb([220, 20, 20]) } else { image::Rgb([20, 20, 220]) }))
}

/// Which edge of a decoded image is red: "top", "bottom", "left" or "right".
pub(crate) fn red_edge(image: &DynamicImage) -> &'static str {
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let red = |x: u32, y: u32| { let pixel = rgba.get_pixel(x, y).0; pixel[0] > 150 && pixel[2] < 100 };
    let (mid_x, mid_y) = (width / 2, height / 2);
    match (red(mid_x, height / 8), red(mid_x, height - 1 - height / 8), red(width / 8, mid_y), red(width - 1 - width / 8, mid_y)) {
        (true, false, _, _) => "top",
        (false, true, _, _) => "bottom",
        (_, _, true, false) => "left",
        (_, _, false, true) => "right",
        _ => "unclear",
    }
}

/// Encodes `image` as a JPEG carrying an EXIF Orientation tag.
pub(crate) fn jpeg_with_orientation(image: &DynamicImage, orientation: u16) -> Vec<u8> {
    let mut jpeg = Vec::new();
    image.to_rgb8().write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
    let mut tiff = b"MM\0\x2a\0\0\0\x08".to_vec();
    tiff.extend_from_slice(&1u16.to_be_bytes());
    tiff.extend_from_slice(&0x0112u16.to_be_bytes());
    tiff.extend_from_slice(&3u16.to_be_bytes());
    tiff.extend_from_slice(&1u32.to_be_bytes());
    tiff.extend_from_slice(&orientation.to_be_bytes());
    tiff.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let mut segment = b"Exif\0\0".to_vec();
    segment.extend_from_slice(&tiff);
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((segment.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&segment);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// An 8-bit HEIC from the bundled encoder.
pub(crate) fn heic(image: &DynamicImage) -> Vec<u8> {
    heif::encode(image).unwrap()
}

/// A 10-bit HEIC: the 8-bit encode re-declared as Main 10 in every place
/// libheif and libde265 read the bit depth from.
pub(crate) fn heic_10bit(image: &DynamicImage) -> Vec<u8> {
    rewrite(&heic(image), |ipco, _| {
        for property in ipco.iter_mut() {
            match &property.kind {
                b"hvcC" => property.payload = ten_bit_hvcc(&property.payload),
                b"pixi" => {
                    for bits in &mut property.payload[5..] { *bits = 10; }
                }
                _ => {}
            }
        }
    })
}

/// A HEIC whose primary item carries an `irot` property turning it
/// `quarter_turns` x 90 degrees anti-clockwise.
pub(crate) fn heic_rotated(image: &DynamicImage, quarter_turns: u8) -> Vec<u8> {
    rewrite(&heic(image), |ipco, ipma| {
        ipco.push(Bx { kind: *b"irot", payload: vec![quarter_turns & 3], children: None });
        add_association(ipma, ipco.len());
    })
}

struct Bx {
    kind: [u8; 4],
    payload: Vec<u8>,
    // Container boxes keep their own header bytes (full-box version/flags) in payload.
    children: Option<Vec<Bx>>,
}

fn parse(bytes: &[u8]) -> Vec<Bx> {
    let mut boxes = Vec::new();
    let mut at = 0;
    while at + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = bytes[at + 4..at + 8].try_into().unwrap();
        assert!(size >= 8, "fixture boxes use 32-bit sizes");
        let body = &bytes[at + 8..at + size];
        let header = match &kind { b"meta" => 4, b"iprp" | b"ipco" => 0, _ => usize::MAX };
        boxes.push(if header == usize::MAX {
            Bx { kind, payload: body.to_vec(), children: None }
        } else {
            Bx { kind, payload: body[..header].to_vec(), children: Some(parse(&body[header..])) }
        });
        at += size;
    }
    boxes
}

fn write(boxes: &[Bx], out: &mut Vec<u8>) {
    for bx in boxes {
        let mut body = bx.payload.clone();
        if let Some(children) = &bx.children { write(children, &mut body); }
        out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
        out.extend_from_slice(&bx.kind);
        out.extend_from_slice(&body);
    }
}

fn child<'a>(boxes: &'a mut [Bx], kind: &[u8; 4]) -> &'a mut Bx {
    boxes.iter_mut().find(|bx| &bx.kind == kind).unwrap_or_else(|| panic!("missing {}", String::from_utf8_lossy(kind)))
}

/// Rewrites the item properties, then shifts the file offsets in `iloc` by
/// however much the `meta` box grew, since media data follows it.
fn rewrite(bytes: &[u8], edit: impl FnOnce(&mut Vec<Bx>, &mut Bx)) -> Vec<u8> {
    let mut top = parse(bytes);
    let before = bytes.len();
    {
        let meta = child(&mut top, b"meta").children.as_mut().unwrap();
        let iprp = child(meta, b"iprp").children.as_mut().unwrap();
        let (ipco, rest): (Vec<_>, Vec<_>) = std::mem::take(iprp).into_iter().partition(|bx| &bx.kind == b"ipco");
        let mut ipco = ipco.into_iter().next().unwrap();
        let mut rest = rest;
        edit(ipco.children.as_mut().unwrap(), child(&mut rest, b"ipma"));
        iprp.push(ipco);
        iprp.extend(rest);
    }
    let mut grown = Vec::new();
    write(&top, &mut grown);
    let delta = grown.len() as i64 - before as i64;
    let meta = child(&mut top, b"meta").children.as_mut().unwrap();
    shift_iloc(&mut child(meta, b"iloc").payload, delta);
    let mut out = Vec::new();
    write(&top, &mut out);
    out
}

fn shift_iloc(iloc: &mut [u8], delta: i64) {
    let version = iloc[0];
    let (offset_size, length_size) = ((iloc[4] >> 4) as usize, (iloc[4] & 15) as usize);
    let (base_size, index_size) = ((iloc[5] >> 4) as usize, if version > 0 { (iloc[5] & 15) as usize } else { 0 });
    let mut at = 6;
    let read = |bytes: &[u8], at: usize, size: usize| bytes[at..at + size].iter().fold(0u64, |value, byte| value << 8 | u64::from(*byte));
    let item_count = if version < 2 { at += 2; read(iloc, 4 + 2, 2) } else { at += 4; read(iloc, 6, 4) };
    let shift = |bytes: &mut [u8], at: usize, size: usize| {
        let value = (read(bytes, at, size) as i64 + delta) as u64;
        for index in 0..size { bytes[at + index] = (value >> (8 * (size - 1 - index))) as u8; }
    };
    for _ in 0..item_count {
        at += if version < 2 { 2 } else { 4 };
        let method = if version > 0 { let method = iloc[at + 1] & 15; at += 2; method } else { 0 };
        at += 2;
        let base_at = at;
        at += base_size;
        let extents = read(iloc, at, 2);
        at += 2;
        if method == 0 && base_size > 0 { shift(iloc, base_at, base_size); }
        for _ in 0..extents {
            at += index_size;
            if method == 0 && base_size == 0 { shift(iloc, at, offset_size); }
            at += offset_size + length_size;
        }
    }
}

fn add_association(ipma: &mut Bx, property_index: usize) {
    let (version, wide) = (ipma.payload[0], ipma.payload[3] & 1 == 1);
    let entries = u32::from_be_bytes(ipma.payload[4..8].try_into().unwrap());
    assert_eq!(entries, 1, "fixtures carry a single image item");
    let count_at = 8 + if version < 1 { 2 } else { 4 };
    let count = ipma.payload[count_at] as usize;
    let end = count_at + 1 + count * if wide { 2 } else { 1 };
    let association = if wide { (0x8000 | property_index as u16).to_be_bytes().to_vec() } else { vec![0x80 | property_index as u8] };
    ipma.payload.splice(end..end, association);
    ipma.payload[count_at] += 1;
}

fn ten_bit_hvcc(hvcc: &[u8]) -> Vec<u8> {
    let mut out = hvcc[..23].to_vec();
    out[1] = (out[1] & 0xE0) | 2; // general_profile_idc: Main 10
    out[2] |= 0x20; // general_profile_compatibility_flag[2]
    out[17] = 0xF8 | 2; // bitDepthLumaMinus8
    out[18] = 0xF8 | 2; // bitDepthChromaMinus8
    let mut at = 23;
    for _ in 0..hvcc[22] {
        let nal_type = hvcc[at] & 0x3F;
        let count = u16::from_be_bytes([hvcc[at + 1], hvcc[at + 2]]);
        out.extend_from_slice(&hvcc[at..at + 3]);
        at += 3;
        for _ in 0..count {
            let length = u16::from_be_bytes([hvcc[at], hvcc[at + 1]]) as usize;
            let nal = &hvcc[at + 2..at + 2 + length];
            let nal = if nal_type == 33 { ten_bit_sps(nal) } else { nal.to_vec() };
            out.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            out.extend_from_slice(&nal);
            at += 2 + length;
        }
    }
    out
}

struct Bits<'a> { bytes: &'a [u8], at: usize }

impl Bits<'_> {
    fn bit(&mut self) -> u32 { let bit = (self.bytes[self.at / 8] >> (7 - self.at % 8)) & 1; self.at += 1; u32::from(bit) }
    fn bits(&mut self, count: usize) -> u32 { (0..count).fold(0, |value, _| value << 1 | self.bit()) }
    fn ue(&mut self) -> u32 {
        let mut zeros = 0;
        while self.bit() == 0 { zeros += 1; }
        (1 << zeros) - 1 + self.bits(zeros)
    }
}

#[derive(Default)]
struct Writer { bytes: Vec<u8>, at: usize }

impl Writer {
    fn bit(&mut self, bit: u32) {
        if self.at % 8 == 0 { self.bytes.push(0); }
        if bit & 1 == 1 { *self.bytes.last_mut().unwrap() |= 1 << (7 - self.at % 8); }
        self.at += 1;
    }
    fn bits(&mut self, value: u32, count: usize) { for index in (0..count).rev() { self.bit(value >> index); } }
    fn ue(&mut self, value: u32) {
        let coded = value + 1;
        let length = 32 - coded.leading_zeros() as usize;
        self.bits(0, length - 1);
        self.bits(coded, length);
    }
}

/// Re-writes an HEVC SPS NAL unit with a Main 10 profile and 10-bit luma and
/// chroma, copying every other syntax element bit for bit.
fn ten_bit_sps(nal: &[u8]) -> Vec<u8> {
    let (mut rbsp, mut zeros) = (Vec::new(), 0);
    for &byte in &nal[2..] {
        if byte == 3 && zeros >= 2 { zeros = 0; continue; }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        rbsp.push(byte);
    }
    let stop = rbsp.len() * 8 - 1 - rbsp.last().unwrap().trailing_zeros() as usize;
    let mut reader = Bits { bytes: &rbsp, at: 0 };
    let mut writer = Writer::default();
    let header = reader.bits(8);
    assert_eq!((header >> 1) & 7, 0, "fixture SPS has a single sub-layer");
    writer.bits(header, 8);
    reader.bits(8);
    writer.bits(2, 8); // profile_space 0, tier 0, profile_idc 2 (Main 10)
    let compatibility = reader.bits(32) | 1 << 29;
    writer.bits(compatibility, 32);
    for _ in 0..7 { let value = reader.bits(8); writer.bits(value, 8); } // constraints + level
    let sps_id = reader.ue();
    writer.ue(sps_id);
    let chroma_format = reader.ue();
    writer.ue(chroma_format);
    if chroma_format == 3 { let separate_planes = reader.bit(); writer.bit(separate_planes); }
    for _ in 0..2 { let value = reader.ue(); writer.ue(value); } // width, height
    let window = reader.bit();
    writer.bit(window);
    if window == 1 { for _ in 0..4 { let value = reader.ue(); writer.ue(value); } }
    reader.ue();
    reader.ue();
    writer.ue(2);
    writer.ue(2);
    while reader.at < stop { let bit = reader.bit(); writer.bit(bit); }
    writer.bit(1);
    let mut out = nal[..2].to_vec();
    let mut zeros = 0;
    for byte in writer.bytes {
        if zeros >= 2 && byte <= 3 { out.push(3); zeros = 0; }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    out
}
