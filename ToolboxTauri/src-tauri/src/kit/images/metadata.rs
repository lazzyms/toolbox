use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MetadataFormat { Jpeg, Png, Webp, Tiff, Heic }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageMetadata {
    pub format: MetadataFormat,
    pub exif: bool,
    pub gps: bool,
    pub xmp: bool,
    pub iptc: bool,
    pub comments: bool,
    pub icc: bool,
    pub orientation: Option<u8>,
    /// Whether EXIF holds anything beyond the orientation tag: camera, lens, date, thumbnail or
    /// location. An orientation-only block is what a cleaned copy keeps, so it does not count.
    #[serde(skip)]
    pub(crate) camera_details: bool,
}

impl ImageMetadata {
    fn empty(format: MetadataFormat) -> Self {
        Self { format, exif: false, gps: false, xmp: false, iptc: false, comments: false, icc: false, orientation: None, camera_details: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataErrorCode { Unreadable, UnsupportedFormat, Truncated, Corrupt }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataError { pub code: MetadataErrorCode, pub message: String }

impl std::fmt::Display for MetadataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.message) }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataReport {
    pub path: PathBuf,
    #[serde(flatten)]
    pub metadata: ImageMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataInspectionError {
    pub path: PathBuf,
    #[serde(flatten)]
    pub error: MetadataError,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum MetadataInspection {
    Report(MetadataReport),
    Error(MetadataInspectionError),
}

pub fn inspect(path: PathBuf) -> MetadataInspection {
    match read_file(&path) {
        Ok(metadata) => MetadataInspection::Report(MetadataReport { path, metadata }),
        Err(error) => MetadataInspection::Error(MetadataInspectionError { path, error }),
    }
}

pub fn read_file(path: &Path) -> Result<ImageMetadata, MetadataError> {
    let bytes = std::fs::read(path).map_err(|error| MetadataError { code: MetadataErrorCode::Unreadable, message: format!("Could not read image metadata: {error}") })?;
    read(&bytes)
}

pub fn read(bytes: &[u8]) -> Result<ImageMetadata, MetadataError> {
    if bytes.starts_with(&[0xFF, 0xD8]) { jpeg(bytes) }
    else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { png(bytes) }
    else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" { webp(bytes) }
    else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") { tiff(bytes) }
    else if is_heif(bytes) { heic(bytes) }
    else { Err(MetadataError { code: MetadataErrorCode::UnsupportedFormat, message: "Metadata can be read from JPEG, PNG, WebP, TIFF, and HEIC images only.".into() }) }
}

type Read<T> = Result<T, MetadataError>;

fn truncated(what: &str) -> MetadataError {
    MetadataError { code: MetadataErrorCode::Truncated, message: format!("The {what} ends before its structure is complete.") }
}

fn corrupt(what: &str) -> MetadataError {
    MetadataError { code: MetadataErrorCode::Corrupt, message: format!("The {what} is damaged and its metadata could not be read.") }
}

fn take(data: &[u8], start: usize, len: usize) -> Option<&[u8]> { data.get(start..start.checked_add(len)?) }
fn be16(data: &[u8], at: usize) -> Option<u16> { take(data, at, 2).map(|b| u16::from_be_bytes([b[0], b[1]])) }
fn be32(data: &[u8], at: usize) -> Option<u32> { take(data, at, 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])) }
fn le32(data: &[u8], at: usize) -> Option<u32> { take(data, at, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) }

fn strip_exif_prefix(data: &[u8]) -> &[u8] { data.strip_prefix(b"Exif\0\0").unwrap_or(data) }

#[derive(Clone, Copy)]
struct Endian { little: bool }

impl Endian {
    fn u16(self, data: &[u8], at: usize) -> Option<u16> { take(data, at, 2).map(|b| if self.little { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) }) }
    fn u32(self, data: &[u8], at: usize) -> Option<u32> { take(data, at, 4).map(|b| if self.little { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) }) }
}

struct Entry { tag: u16, kind: u16, count: u32, value_at: usize }

fn tiff_header(data: &[u8]) -> Option<(Endian, usize)> {
    let endian = match take(data, 0, 2)? { b"II" => Endian { little: true }, b"MM" => Endian { little: false }, _ => return None };
    if endian.u16(data, 2)? != 42 { return None; }
    Some((endian, endian.u32(data, 4)? as usize))
}

fn read_ifd(data: &[u8], at: usize, endian: Endian) -> Option<Vec<Entry>> {
    let count = endian.u16(data, at)? as usize;
    take(data, at + 2, count * 12 + 4)?;
    Some((0..count).map(|index| {
        let base = at + 2 + index * 12;
        Entry { tag: endian.u16(data, base).unwrap_or(0), kind: endian.u16(data, base + 2).unwrap_or(0), count: endian.u32(data, base + 4).unwrap_or(0), value_at: base + 8 }
    }).collect())
}

fn entry_values(data: &[u8], entry: &Entry, endian: Endian) -> Option<Vec<u32>> {
    let size = match entry.kind { 3 => 2, 4 | 13 => 4, _ => return None };
    let total = size * entry.count as usize;
    let start = if total <= 4 { entry.value_at } else { endian.u32(data, entry.value_at)? as usize };
    take(data, start, total)?;
    (0..entry.count as usize).map(|index| if size == 2 { endian.u16(data, start + index * 2).map(u32::from) } else { endian.u32(data, start + index * 4) }).collect()
}

fn entry_value(data: &[u8], entry: &Entry, endian: Endian) -> Option<u32> { entry_values(data, entry, endian)?.first().copied() }

fn read_ifd0_tags(data: &[u8], endian: Endian, entries: &[Entry], metadata: &mut ImageMetadata) -> Option<()> {
    for entry in entries {
        match entry.tag {
            0x0112 => metadata.orientation = Some(entry_value(data, entry, endian)?).filter(|value| (1..=8).contains(value)).map(|value| value as u8),
            0x8825 => metadata.gps = !read_ifd(data, entry_value(data, entry, endian)? as usize, endian)?.is_empty(),
            _ => {}
        }
    }
    Some(())
}

fn exif(payload: &[u8], metadata: &mut ImageMetadata) -> Read<()> {
    let (endian, at) = tiff_header(payload).ok_or_else(|| corrupt("EXIF block"))?;
    let entries = read_ifd(payload, at, endian).ok_or_else(|| corrupt("EXIF block"))?;
    read_ifd0_tags(payload, endian, &entries, metadata).ok_or_else(|| corrupt("EXIF block"))?;
    let next_ifd = endian.u32(payload, at + 2 + entries.len() * 12).unwrap_or(0);
    metadata.camera_details |= next_ifd != 0 || entries.iter().any(|entry| entry.tag != 0x0112);
    metadata.exif = true;
    Ok(())
}

fn photoshop_has_iptc(mut data: &[u8]) -> Read<bool> {
    let mut found = false;
    while data.len() >= 4 {
        let id = be16(data, 4).ok_or_else(|| corrupt("Photoshop resource block"))?;
        let name_len = *data.get(6).ok_or_else(|| corrupt("Photoshop resource block"))? as usize;
        let name_field = (name_len + 2) & !1;
        let size = be32(data, 6 + name_field).ok_or_else(|| corrupt("Photoshop resource block"))? as usize;
        let end = 10 + name_field + ((size + 1) & !1);
        take(data, 10 + name_field, size).ok_or_else(|| corrupt("Photoshop resource block"))?;
        found |= id == 0x0404 && size > 0;
        data = data.get(end.min(data.len())..).unwrap_or_default();
    }
    Ok(found)
}

fn jpeg(data: &[u8]) -> Read<ImageMetadata> {
    let mut metadata = ImageMetadata::empty(MetadataFormat::Jpeg);
    let mut pos = 2;
    loop {
        match data.get(pos) {
            Some(0xFF) => {}
            Some(_) => return Err(corrupt("JPEG image")),
            None => return Err(truncated("JPEG image")),
        }
        while data.get(pos) == Some(&0xFF) { pos += 1; }
        let marker = *data.get(pos).ok_or_else(|| truncated("JPEG image"))?;
        pos += 1;
        match marker {
            0x01 | 0xD0..=0xD7 => continue,
            0xD8 | 0xD9 => return Err(corrupt("JPEG image")),
            _ => {}
        }
        let length = be16(data, pos).ok_or_else(|| truncated("JPEG image"))? as usize;
        if length < 2 { return Err(corrupt("JPEG image")); }
        let body = take(data, pos + 2, length - 2).ok_or_else(|| truncated("JPEG image"))?;
        pos += length;
        match marker {
            0xDA => return if data[pos..].windows(2).any(|pair| pair == [0xFF, 0xD9]) { Ok(metadata) } else { Err(truncated("JPEG image")) },
            0xE1 if body.starts_with(b"Exif\0\0") => exif(&body[6..], &mut metadata)?,
            0xE1 if body.starts_with(b"http://ns.adobe.com/xap/1.0/\0") || body.starts_with(b"http://ns.adobe.com/xmp/extension/\0") => metadata.xmp = true,
            0xE2 if body.starts_with(b"ICC_PROFILE\0") => metadata.icc = true,
            0xED if body.starts_with(b"Photoshop 3.0\0") => metadata.iptc |= photoshop_has_iptc(&body[14..])?,
            0xFE => metadata.comments = true,
            _ => {}
        }
    }
}

fn png(data: &[u8]) -> Read<ImageMetadata> {
    let mut metadata = ImageMetadata::empty(MetadataFormat::Png);
    let mut pos = 8;
    loop {
        let length = be32(data, pos).ok_or_else(|| truncated("PNG image"))? as usize;
        let kind_and_data = take(data, pos + 4, length + 4).ok_or_else(|| truncated("PNG image"))?;
        let crc = be32(data, pos + 8 + length).ok_or_else(|| truncated("PNG image"))?;
        let mut check = flate2::Crc::new();
        check.update(kind_and_data);
        if check.sum() != crc { return Err(corrupt("PNG image")); }
        pos += 12 + length;
        let (kind, chunk) = kind_and_data.split_at(4);
        match kind {
            b"IEND" => return Ok(metadata),
            b"eXIf" => exif(strip_exif_prefix(chunk), &mut metadata)?,
            b"iCCP" => metadata.icc = true,
            b"tEXt" | b"zTXt" | b"iTXt" => match chunk.split(|byte| *byte == 0).next().unwrap_or_default() {
                b"XML:com.adobe.xmp" | b"Raw profile type xmp" => metadata.xmp = true,
                b"Raw profile type iptc" | b"Raw profile type 8bim" => metadata.iptc = true,
                b"Raw profile type exif" | b"Raw profile type APP1" => { metadata.exif = true; metadata.camera_details = true; }
                b"Raw profile type icc" | b"Raw profile type icm" => metadata.icc = true,
                _ => metadata.comments = true,
            },
            _ => {}
        }
    }
}

fn webp(data: &[u8]) -> Read<ImageMetadata> {
    let mut metadata = ImageMetadata::empty(MetadataFormat::Webp);
    let end = 8 + le32(data, 4).ok_or_else(|| truncated("WebP image"))? as usize;
    if data.len() < end { return Err(truncated("WebP image")); }
    let mut has_image = false;
    let mut pos = 12;
    while pos < end {
        let kind = take(data, pos, 4).ok_or_else(|| truncated("WebP image"))?;
        let size = le32(data, pos + 4).ok_or_else(|| truncated("WebP image"))? as usize;
        let chunk = data.get(pos + 8..end).and_then(|rest| rest.get(..size)).ok_or_else(|| truncated("WebP image"))?;
        pos += 8 + size + (size & 1);
        match kind {
            b"VP8 " | b"VP8L" | b"ANMF" => has_image = true,
            b"EXIF" => exif(strip_exif_prefix(chunk), &mut metadata)?,
            b"XMP " => metadata.xmp = true,
            b"ICCP" => metadata.icc = true,
            _ => {}
        }
    }
    if has_image { Ok(metadata) } else { Err(corrupt("WebP image")) }
}

fn tiff(data: &[u8]) -> Read<ImageMetadata> {
    let mut metadata = ImageMetadata::empty(MetadataFormat::Tiff);
    let (endian, at) = tiff_header(data).ok_or_else(|| truncated("TIFF image"))?;
    let entries = read_ifd(data, at, endian).ok_or_else(|| truncated("TIFF image"))?;
    read_ifd0_tags(data, endian, &entries, &mut metadata).ok_or_else(|| truncated("TIFF image"))?;
    let values = |tag: u16| entries.iter().find(|entry| entry.tag == tag).map(|entry| entry_values(data, entry, endian).ok_or_else(|| truncated("TIFF image"))).transpose();
    for entry in &entries {
        match entry.tag {
            0x8769 | 0x8825 => { metadata.exif = true; metadata.camera_details = true; }
            0x02BC => metadata.xmp = true,
            0x83BB => metadata.iptc = true,
            0x8773 => metadata.icc = true,
            0x010E | 0x9C9C => metadata.comments = true,
            _ => {}
        }
        if entry.count > 0 && !matches!(entry.kind, 1..=13 | 16..=18) { return Err(corrupt("TIFF image")); }
    }
    let (offsets, counts) = match (values(273)?, values(279)?) {
        (Some(offsets), Some(counts)) => (offsets, counts),
        _ => (values(324)?.ok_or_else(|| corrupt("TIFF image"))?, values(325)?.ok_or_else(|| corrupt("TIFF image"))?),
    };
    if offsets.len() != counts.len() { return Err(corrupt("TIFF image")); }
    for (offset, count) in offsets.into_iter().zip(counts) {
        take(data, offset as usize, count as usize).ok_or_else(|| truncated("TIFF image"))?;
    }
    Ok(metadata)
}

const HEIF_BRANDS: [&[u8; 4]; 9] = [b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"mif1", b"msf1", b"heif"];

fn is_heif(data: &[u8]) -> bool {
    if take(data, 4, 4) != Some(b"ftyp") { return false; }
    let size = be32(data, 0).unwrap_or(0) as usize;
    let brands = data.get(8..size.min(data.len())).unwrap_or_default();
    brands.chunks_exact(4).enumerate().any(|(index, brand)| index != 1 && HEIF_BRANDS.iter().any(|known| known.as_slice() == brand))
}

pub(crate) struct IsoBox<'a> { pub kind: [u8; 4], pub payload: &'a [u8] }

pub(crate) fn boxes(data: &[u8]) -> Read<Vec<IsoBox<'_>>> {
    let mut found = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let size = be32(data, pos).ok_or_else(|| truncated("HEIC image"))? as u64;
        let kind: [u8; 4] = take(data, pos + 4, 4).ok_or_else(|| truncated("HEIC image"))?.try_into().unwrap();
        let (header, size) = match size {
            0 => (8, (data.len() - pos) as u64),
            1 => (16, take(data, pos + 8, 8).map(|b| u64::from_be_bytes(b.try_into().unwrap())).ok_or_else(|| truncated("HEIC image"))?),
            size => (8, size),
        };
        if size < header as u64 { return Err(corrupt("HEIC image")); }
        let payload = take(data, pos + header, size as usize - header).ok_or_else(|| truncated("HEIC image"))?;
        found.push(IsoBox { kind, payload });
        pos += size as usize;
    }
    Ok(found)
}

fn child<'a>(children: &'a [IsoBox<'a>], kind: &[u8; 4]) -> Option<&'a IsoBox<'a>> { children.iter().find(|item| &item.kind == kind) }

pub(crate) struct ItemLocation { pub id: u32, pub method: u16, pub base: u64, pub extents: Vec<(u64, u64)> }

struct Fields<'a> { data: &'a [u8], pos: usize }

impl Fields<'_> {
    fn sized(&mut self, size: usize) -> Read<u64> {
        let bytes = take(self.data, self.pos, size).ok_or_else(|| corrupt("HEIC item table"))?;
        self.pos += size;
        Ok(bytes.iter().fold(0u64, |value, byte| (value << 8) | u64::from(*byte)))
    }
    fn cstr(&mut self) -> Read<&[u8]> {
        let rest = self.data.get(self.pos..).unwrap_or_default();
        let end = rest.iter().position(|byte| *byte == 0).ok_or_else(|| corrupt("HEIC item table"))?;
        self.pos += end + 1;
        Ok(&rest[..end])
    }
}

pub(crate) fn item_locations(iloc: &[u8]) -> Read<Vec<ItemLocation>> {
    let mut fields = Fields { data: iloc, pos: 0 };
    let version = fields.sized(1)?;
    fields.sized(3)?;
    let sizes = fields.sized(2)?;
    let (offset_size, length_size, base_size) = ((sizes >> 12) as usize, ((sizes >> 8) & 15) as usize, ((sizes >> 4) & 15) as usize);
    let index_size = if matches!(version, 1 | 2) { (sizes & 15) as usize } else { 0 };
    let count = fields.sized(if version < 2 { 2 } else { 4 })?;
    (0..count).map(|_| {
        let id = fields.sized(if version < 2 { 2 } else { 4 })? as u32;
        let method = if matches!(version, 1 | 2) { (fields.sized(2)? & 15) as u16 } else { 0 };
        fields.sized(2)?;
        let base = fields.sized(base_size)?;
        let extent_count = fields.sized(2)?;
        let extents = (0..extent_count).map(|_| {
            fields.sized(index_size)?;
            Ok((fields.sized(offset_size)?, fields.sized(length_size)?))
        }).collect::<Read<Vec<_>>>()?;
        Ok(ItemLocation { id, method, base, extents })
    }).collect()
}

fn item_extents(data: &[u8], idat: &[u8], location: &ItemLocation, mut visit: impl FnMut(&[u8])) -> Read<()> {
    let source = match location.method { 0 => data, 1 => idat, _ => return Ok(()) };
    for (offset, length) in &location.extents {
        let start = location.base.checked_add(*offset).ok_or_else(|| corrupt("HEIC image"))? as usize;
        let length = if *length == 0 { source.len().saturating_sub(start) } else { *length as usize };
        visit(take(source, start, length).ok_or_else(|| truncated("HEIC image"))?);
    }
    Ok(())
}

fn heic(data: &[u8]) -> Read<ImageMetadata> {
    let mut metadata = ImageMetadata::empty(MetadataFormat::Heic);
    let top = boxes(data)?;
    let meta = child(&top, b"meta").ok_or_else(|| corrupt("HEIC image"))?;
    let children = boxes(meta.payload.get(4..).ok_or_else(|| corrupt("HEIC image"))?)?;
    let idat = child(&children, b"idat").map(|item| item.payload).unwrap_or_default();
    let locations = item_locations(child(&children, b"iloc").ok_or_else(|| corrupt("HEIC image"))?.payload)?;
    for location in &locations { item_extents(data, idat, location, |_| {})?; }
    let iinf = child(&children, b"iinf").ok_or_else(|| corrupt("HEIC image"))?.payload;
    let entries_at = if iinf.first() == Some(&0) { 6 } else { 8 };
    for infe in boxes(iinf.get(entries_at..).ok_or_else(|| corrupt("HEIC item table"))?)?.iter().filter(|item| &item.kind == b"infe") {
        let mut fields = Fields { data: infe.payload, pos: 0 };
        let version = fields.sized(1)?;
        if version < 2 { continue; }
        fields.sized(3)?;
        let id = fields.sized(if version == 2 { 2 } else { 4 })? as u32;
        fields.sized(2)?;
        let kind = fields.sized(4)?.to_be_bytes();
        fields.cstr()?;
        match &kind[4..] {
            b"Exif" => {
                let mut payload = Vec::new();
                if let Some(location) = locations.iter().find(|location| location.id == id) { item_extents(data, idat, location, |bytes| payload.extend_from_slice(bytes))?; }
                let payload = payload.as_slice();
                let skip = be32(payload, 0).ok_or_else(|| corrupt("HEIC EXIF item"))? as usize;
                exif(strip_exif_prefix(payload.get(4 + skip..).ok_or_else(|| corrupt("HEIC EXIF item"))?), &mut metadata)?;
            }
            b"mime" if fields.cstr()? == b"application/rdf+xml" => metadata.xmp = true,
            _ => {}
        }
    }
    if let Some(ipco) = child(&children, b"iprp").map(|iprp| boxes(iprp.payload)).transpose()?.as_deref().and_then(|iprp| child(iprp, b"ipco").map(|ipco| ipco.payload)) {
        metadata.icc = boxes(ipco)?.iter().any(|item| &item.kind == b"colr" && matches!(item.payload.get(..4), Some(b"prof" | b"rICC")));
    }
    heif::probe(data).map_err(|error| MetadataError { code: MetadataErrorCode::Corrupt, message: format!("The HEIC image is damaged and its metadata could not be read: {error}") })?;
    Ok(metadata)
}

#[cfg(test)]
pub mod fixtures {
    use super::{boxes, item_locations};

    pub enum Field { Short(u16), Long(u32), Bytes(u16, Vec<u8>), Ifd(Vec<(u16, Field)>) }

    fn ifd(mut fields: Vec<(u16, Field)>, at: usize) -> Vec<u8> {
        fields.sort_by_key(|(tag, _)| *tag);
        let data_at = at + 2 + fields.len() * 12 + 4;
        let mut head = (fields.len() as u16).to_le_bytes().to_vec();
        let mut data = Vec::new();
        for (tag, field) in fields {
            let (kind, count, value) = match field {
                Field::Short(value) => (3u16, 1u32, [value.to_le_bytes()[0], value.to_le_bytes()[1], 0, 0]),
                Field::Long(value) => (4, 1, value.to_le_bytes()),
                Field::Bytes(kind, bytes) if bytes.len() <= 4 => {
                    let mut value = [0; 4];
                    value[..bytes.len()].copy_from_slice(&bytes);
                    (kind, bytes.len() as u32, value)
                }
                Field::Bytes(kind, bytes) => {
                    let offset = (data_at + data.len()) as u32;
                    data.extend_from_slice(&bytes);
                    if data.len() % 2 == 1 { data.push(0); }
                    (kind, bytes.len() as u32, offset.to_le_bytes())
                }
                Field::Ifd(sub) => {
                    let offset = data_at + data.len();
                    data.extend(ifd(sub, offset));
                    (4, 1, (offset as u32).to_le_bytes())
                }
            };
            head.extend(tag.to_le_bytes());
            head.extend(kind.to_le_bytes());
            head.extend(count.to_le_bytes());
            head.extend(value);
        }
        head.extend(0u32.to_le_bytes());
        head.extend(data);
        head
    }

    pub fn tiff_structure(fields: Vec<(u16, Field)>) -> Vec<u8> {
        let mut out = b"II*\0".to_vec();
        out.extend(8u32.to_le_bytes());
        out.extend(ifd(fields, 8));
        out
    }

    pub fn exif_fields(orientation: Option<u16>, gps: bool) -> Vec<(u16, Field)> {
        let mut fields = vec![(0x8769, Field::Ifd(vec![(0x9000, Field::Bytes(7, b"0232".to_vec()))]))];
        if let Some(orientation) = orientation { fields.push((0x0112, Field::Short(orientation))); }
        if gps { fields.push((0x8825, Field::Ifd(vec![(0x0000, Field::Bytes(1, vec![2, 3, 0, 0]))]))); }
        fields
    }

    pub fn exif(orientation: Option<u16>, gps: bool) -> Vec<u8> { tiff_structure(exif_fields(orientation, gps)) }

    pub const XMP: &[u8] = b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>";
    pub const ICC: &[u8] = b"fixture colour profile";

    #[derive(Default, Clone, Copy)]
    pub struct Wanted { pub exif: bool, pub gps: bool, pub orientation: Option<u16>, pub xmp: bool, pub iptc: bool, pub comments: bool, pub icc: bool }

    impl Wanted {
        fn exif_payload(&self) -> Option<Vec<u8>> { (self.exif || self.gps || self.orientation.is_some()).then(|| exif(self.orientation, self.gps)) }
    }

    fn rgb() -> image::DynamicImage { image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(16, 16, |x, y| image::Rgb([x as u8 * 16, y as u8 * 16, 128]))) }

    fn encoded(format: image::ImageFormat) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        rgb().write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }

    fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, marker];
        out.extend(((body.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    pub fn jpeg(wanted: Wanted) -> Vec<u8> {
        let base = encoded(image::ImageFormat::Jpeg);
        let mut out = base[..2].to_vec();
        if let Some(payload) = wanted.exif_payload() { out.extend(segment(0xE1, &[b"Exif\0\0".as_slice(), &payload].concat())); }
        if wanted.xmp { out.extend(segment(0xE1, &[b"http://ns.adobe.com/xap/1.0/\0".as_slice(), XMP].concat())); }
        if wanted.icc { out.extend(segment(0xE2, &[b"ICC_PROFILE\0\x01\x01".as_slice(), ICC].concat())); }
        if wanted.iptc {
            let record = [0x1C, 0x02, 0x78, 0x00, 0x05, b'h', b'e', b'l', b'l', b'o'];
            let mut body = b"Photoshop 3.0\08BIM\x04\x04\0\0".to_vec();
            body.extend((record.len() as u32).to_be_bytes());
            body.extend(record);
            out.extend(segment(0xED, &body));
        }
        if wanted.comments { out.extend(segment(0xFE, b"fixture comment")); }
        out.extend_from_slice(&base[2..]);
        out
    }

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend(kind);
        out.extend(data);
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        out.extend(crc.sum().to_be_bytes());
        out
    }

    pub fn png(wanted: Wanted) -> Vec<u8> {
        let base = encoded(image::ImageFormat::Png);
        let mut out = base[..33].to_vec();
        if let Some(payload) = wanted.exif_payload() { out.extend(chunk(b"eXIf", &payload)); }
        if wanted.icc {
            // iCCP holds the profile zlib-compressed, as decoders expect.
            let mut compressed = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            std::io::Write::write_all(&mut compressed, ICC).unwrap();
            out.extend(chunk(b"iCCP", &[b"fixture\0\0".as_slice(), &compressed.finish().unwrap()].concat()));
        }
        if wanted.xmp { out.extend(chunk(b"iTXt", &[b"XML:com.adobe.xmp\0\0\0\0\0".as_slice(), XMP].concat())); }
        if wanted.iptc { out.extend(chunk(b"tEXt", b"Raw profile type iptc\0\niptc\n       4\n1c020000\n")); }
        if wanted.comments { out.extend(chunk(b"tEXt", b"Comment\0fixture comment")); }
        out.extend_from_slice(&base[33..]);
        out
    }

    fn riff_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = kind.to_vec();
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        if data.len() % 2 == 1 { out.push(0); }
        out
    }

    pub fn webp(wanted: Wanted) -> Vec<u8> {
        let base = encoded(image::ImageFormat::WebP);
        let exif = wanted.exif_payload();
        let mut body = b"WEBP".to_vec();
        if exif.is_some() || wanted.xmp || wanted.icc {
            let flags = if wanted.icc { 0x20 } else { 0 } | if exif.is_some() { 0x08 } else { 0 } | if wanted.xmp { 0x04 } else { 0 };
            body.extend(riff_chunk(b"VP8X", &[flags, 0, 0, 0, 15, 0, 0, 15, 0, 0]));
            if wanted.icc { body.extend(riff_chunk(b"ICCP", ICC)); }
            body.extend_from_slice(&base[12..]);
            if let Some(exif) = &exif { body.extend(riff_chunk(b"EXIF", exif)); }
            if wanted.xmp { body.extend(riff_chunk(b"XMP ", XMP)); }
        } else {
            body.extend_from_slice(&base[12..]);
        }
        let mut out = b"RIFF".to_vec();
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }

    pub fn tiff(wanted: Wanted) -> Vec<u8> {
        let pixels = vec![200u8; 16 * 16];
        let fields = |strip_at: u32| {
            let mut fields = vec![
                (256, Field::Short(16)), (257, Field::Short(16)), (258, Field::Short(8)), (259, Field::Short(1)), (262, Field::Short(1)),
                (273, Field::Long(strip_at)), (277, Field::Short(1)), (278, Field::Short(16)), (279, Field::Long(pixels.len() as u32)),
            ];
            if wanted.exif { fields.push((0x8769, Field::Ifd(vec![(0x9000, Field::Bytes(7, b"0232".to_vec()))]))); }
            if let Some(orientation) = wanted.orientation { fields.push((0x0112, Field::Short(orientation))); }
            if wanted.gps { fields.push((0x8825, Field::Ifd(vec![(0x0000, Field::Bytes(1, vec![2, 3, 0, 0]))]))); }
            if wanted.xmp { fields.push((0x02BC, Field::Bytes(1, XMP.to_vec()))); }
            if wanted.iptc { fields.push((0x83BB, Field::Bytes(7, vec![0x1C, 0x02, 0x78, 0x00, 0x05, b'h', b'e', b'l', b'l', b'o']))); }
            if wanted.icc { fields.push((0x8773, Field::Bytes(7, ICC.to_vec()))); }
            if wanted.comments { fields.push((0x010E, Field::Bytes(2, b"fixture comment\0".to_vec()))); }
            fields
        };
        let strip_at = tiff_structure(fields(0)).len() as u32;
        let mut out = tiff_structure(fields(strip_at));
        out.extend(pixels);
        out
    }

    fn iso_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend(kind);
        out.extend(payload);
        out
    }

    fn infe(id: u16, kind: &[u8; 4], content_type: Option<&str>) -> Vec<u8> {
        let mut payload = vec![2, 0, 0, 0];
        payload.extend(id.to_be_bytes());
        payload.extend([0, 0]);
        payload.extend(kind);
        payload.push(0);
        if let Some(content_type) = content_type { payload.extend(content_type.as_bytes()); payload.push(0); }
        iso_box(b"infe", &payload)
    }

    pub fn heic_base() -> Vec<u8> { heif::encode(&rgb()).unwrap() }

    pub fn heic(wanted: Wanted) -> Vec<u8> {
        let base = heic_base();
        let exif = wanted.exif_payload().map(|payload| [0u32.to_be_bytes().as_slice(), b"Exif\0\0", &payload].concat());
        let mut appended: Vec<(&[u8; 4], Option<&str>, Vec<u8>)> = Vec::new();
        if let Some(exif) = exif { appended.push((b"Exif", None, exif)); }
        if wanted.xmp { appended.push((b"mime", Some("application/rdf+xml"), XMP.to_vec())); }
        if appended.is_empty() && !wanted.icc { return base; }

        let top = boxes(&base).unwrap();
        let meta_index = top.iter().position(|item| &item.kind == b"meta").unwrap();
        let meta = &top[meta_index];
        let meta_len = meta.payload.len() + 8;
        let meta_start: usize = top[..meta_index].iter().map(|item| item.payload.len() + 8).sum();
        let children = boxes(&meta.payload[4..]).unwrap();
        let locations = item_locations(children.iter().find(|item| &item.kind == b"iloc").unwrap().payload).unwrap();
        let iinf = children.iter().find(|item| &item.kind == b"iinf").unwrap().payload;
        let entries_at = if iinf[0] == 0 { 6 } else { 8 };
        let infes = boxes(&iinf[entries_at..]).unwrap();
        let first_new_id = locations.iter().map(|location| location.id).max().unwrap_or(0) + 1;

        let build_meta = |shift: i64, appended_at: u64| {
            let mut payload = meta.payload[..4].to_vec();
            for item in &children {
                match &item.kind {
                    b"iinf" => {
                        let mut iinf = vec![0, 0, 0, 0];
                        iinf.extend(((infes.len() + appended.len()) as u16).to_be_bytes());
                        for entry in &infes { iinf.extend(iso_box(&entry.kind, entry.payload)); }
                        for (index, (kind, content_type, _)) in appended.iter().enumerate() { iinf.extend(infe((first_new_id + index as u32) as u16, kind, *content_type)); }
                        payload.extend(iso_box(b"iinf", &iinf));
                    }
                    b"iloc" => {
                        let mut iloc = vec![1, 0, 0, 0, 0x44, 0x40];
                        iloc.extend(((locations.len() + appended.len()) as u16).to_be_bytes());
                        for location in &locations {
                            iloc.extend((location.id as u16).to_be_bytes());
                            iloc.extend(location.method.to_be_bytes());
                            iloc.extend([0, 0]);
                            let moves = location.method == 0;
                            let base = if moves && location.base > 0 { (location.base as i64 + shift) as u32 } else { location.base as u32 };
                            iloc.extend(base.to_be_bytes());
                            iloc.extend((location.extents.len() as u16).to_be_bytes());
                            for (offset, length) in &location.extents {
                                let offset = if moves && location.base == 0 { (*offset as i64 + shift) as u32 } else { *offset as u32 };
                                iloc.extend(offset.to_be_bytes());
                                iloc.extend((*length as u32).to_be_bytes());
                            }
                        }
                        let mut at = appended_at;
                        for (index, (_, _, bytes)) in appended.iter().enumerate() {
                            iloc.extend(((first_new_id + index as u32) as u16).to_be_bytes());
                            iloc.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
                            iloc.extend((at as u32).to_be_bytes());
                            iloc.extend((bytes.len() as u32).to_be_bytes());
                            at += bytes.len() as u64;
                        }
                        payload.extend(iso_box(b"iloc", &iloc));
                    }
                    b"iprp" if wanted.icc => {
                        let mut iprp = Vec::new();
                        for property in boxes(item.payload).unwrap() {
                            if &property.kind == b"ipco" {
                                let mut ipco = property.payload.to_vec();
                                ipco.extend(iso_box(b"colr", &[b"prof".as_slice(), ICC].concat()));
                                iprp.extend(iso_box(b"ipco", &ipco));
                            } else {
                                iprp.extend(iso_box(&property.kind, property.payload));
                            }
                        }
                        payload.extend(iso_box(b"iprp", &iprp));
                    }
                    kind => payload.extend(iso_box(kind, item.payload)),
                }
            }
            iso_box(b"meta", &payload)
        };
        let shift = build_meta(0, 0).len() as i64 - meta_len as i64;
        let appended_at = (base.len() as i64 + shift) as u64 + 8;
        let mut out = base[..meta_start].to_vec();
        out.extend(build_meta(shift, appended_at));
        out.extend_from_slice(&base[meta_start + meta_len..]);
        out.extend(iso_box(b"mdat", &appended.iter().flat_map(|(_, _, bytes)| bytes.iter().copied()).collect::<Vec<_>>()));
        out
    }
}
