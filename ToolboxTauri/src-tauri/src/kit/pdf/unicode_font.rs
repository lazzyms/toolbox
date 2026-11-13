use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;

use flate2::{write::ZlibEncoder, Compression};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use rustybuzz::ttf_parser::{self, Tag};
use sha2::{Digest, Sha256};

use crate::kit::contracts::ToolError;

const REQUIRED_TABLES: [&[u8; 4]; 6] = [b"head", b"hhea", b"hmtx", b"maxp", b"loca", b"glyf"];
const HINTING_TABLES: [&[u8; 4]; 3] = [b"cvt ", b"fpgm", b"prep"];

pub struct UnicodeFont {
    data: Vec<u8>,
    units_per_em: f32,
    ascent: f32,
    descent: f32,
    slots: Vec<(u16, Option<String>)>,
    slot_ids: BTreeMap<(u16, Option<String>), u16>,
}

pub struct GlyphRun {
    pub operators: String,
    pub width: f32,
}

fn font_error(message: impl std::fmt::Display) -> ToolError {
    ToolError::processing(format!("Could not read the font: {message}"))
}

fn table<'a>(face: &ttf_parser::Face<'a>, tag: &[u8; 4]) -> Option<&'a [u8]> {
    face.raw_face().table(Tag::from_bytes(tag))
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    data.get(offset..offset + 2).map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4).map(|bytes| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

struct Tables<'a> {
    head: &'a [u8],
    hhea: &'a [u8],
    hmtx: &'a [u8],
    maxp: &'a [u8],
    loca: &'a [u8],
    glyf: &'a [u8],
    long_loca: bool,
    glyph_count: u16,
    metric_count: u16,
}

impl<'a> Tables<'a> {
    fn read(face: &ttf_parser::Face<'a>) -> Result<Self, ToolError> {
        let missing = REQUIRED_TABLES.iter().filter(|tag| table(face, tag).is_none()).map(|tag| String::from_utf8_lossy(&tag[..]).into_owned()).collect::<Vec<_>>();
        if !missing.is_empty() { return Err(font_error(format!("it has no TrueType outlines (missing {}).", missing.join(", ")))); }
        let [head, hhea, hmtx, maxp, loca, glyf] = REQUIRED_TABLES.map(|tag| table(face, tag).unwrap_or_default());
        let format = read_u16(head, 50).ok_or_else(|| font_error("its head table is truncated."))?;
        let glyph_count = read_u16(maxp, 4).ok_or_else(|| font_error("its maxp table is truncated."))?;
        let metric_count = read_u16(hhea, 34).ok_or_else(|| font_error("its hhea table is truncated."))?;
        let tables = Self { head, hhea, hmtx, maxp, loca, glyf, long_loca: format == 1, glyph_count, metric_count };
        if glyph_count == 0 || metric_count == 0 || metric_count > glyph_count { return Err(font_error("its glyph metrics are invalid.")); }
        if hmtx.len() < metric_count as usize * 4 + (glyph_count - metric_count) as usize * 2 { return Err(font_error("its hmtx table is truncated.")); }
        if (0..=glyph_count).any(|gid| tables.loca_offset(gid).is_none()) { return Err(font_error("its loca table is invalid.")); }
        Ok(tables)
    }

    fn loca_offset(&self, index: u16) -> Option<usize> {
        let index = index as usize;
        let offset = if self.long_loca { read_u32(self.loca, index * 4)? as usize } else { read_u16(self.loca, index * 2)? as usize * 2 };
        (offset <= self.glyf.len()).then_some(offset)
    }

    fn glyph(&self, gid: u16) -> Result<&'a [u8], ToolError> {
        let (start, end) = (self.loca_offset(gid), self.loca_offset(gid + 1));
        match (start, end) {
            (Some(start), Some(end)) if start <= end => Ok(&self.glyf[start..end]),
            _ => Err(font_error(format!("glyph {gid} is out of range."))),
        }
    }

    fn metrics(&self, gid: u16) -> [u8; 4] {
        let gid = gid as usize;
        let metrics = self.metric_count as usize;
        let advance = &self.hmtx[(gid.min(metrics - 1)) * 4..][..2];
        let bearing = if gid < metrics { &self.hmtx[gid * 4 + 2..][..2] } else { &self.hmtx[metrics * 4 + (gid - metrics) * 2..][..2] };
        [advance[0], advance[1], bearing[0], bearing[1]]
    }
}

fn components(glyph: &[u8]) -> Result<Vec<(usize, u16)>, ToolError> {
    if glyph.len() < 10 || i16::from_be_bytes([glyph[0], glyph[1]]) >= 0 { return Ok(Vec::new()); }
    let truncated = || font_error("a composite glyph is truncated.");
    let mut found = Vec::new();
    let mut offset = 10;
    loop {
        let flags = read_u16(glyph, offset).ok_or_else(truncated)?;
        found.push((offset + 2, read_u16(glyph, offset + 2).ok_or_else(truncated)?));
        offset += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
        offset += if flags & 0x0008 != 0 { 2 } else if flags & 0x0040 != 0 { 4 } else if flags & 0x0080 != 0 { 8 } else { 0 };
        if flags & 0x0020 == 0 { return Ok(found); }
    }
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0_u32, |sum, chunk| {
        let mut word = [0_u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn font_file(mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by(|left, right| left.0.cmp(&right.0));
    let count = tables.len() as u16;
    let power = 1_u16 << (15 - count.leading_zeros());
    let mut file = Vec::new();
    file.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    for value in [count, power * 16, power.trailing_zeros() as u16, count * 16 - power * 16] { file.extend_from_slice(&value.to_be_bytes()); }
    let mut offset = 12 + tables.len() * 16;
    let mut head_offset = 0;
    for (tag, data) in &tables {
        if tag == b"head" { head_offset = offset; }
        file.extend_from_slice(tag);
        file.extend_from_slice(&checksum(data).to_be_bytes());
        file.extend_from_slice(&(offset as u32).to_be_bytes());
        file.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        file.extend_from_slice(data);
        file.resize(file.len().next_multiple_of(4), 0);
    }
    let adjustment = 0xB1B0_AFBA_u32.wrapping_sub(checksum(&file));
    file[head_offset + 8..head_offset + 12].copy_from_slice(&adjustment.to_be_bytes());
    file
}

fn utf16_hex(text: &str) -> String {
    text.encode_utf16().map(|unit| format!("{unit:04X}")).collect()
}

fn compressed_stream(dictionary: lopdf::Dictionary, bytes: &[u8]) -> Result<Stream, ToolError> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).and_then(|_| encoder.finish()).map(|compressed| {
        let mut dictionary = dictionary;
        dictionary.set("Filter", "FlateDecode");
        Stream::new(dictionary, compressed).with_compression(false)
    }).map_err(|error| ToolError::processing(format!("Could not compress the embedded font: {error}")))
}

impl UnicodeFont {
    pub fn parse(data: Vec<u8>) -> Result<Self, ToolError> {
        let face = ttf_parser::Face::parse(&data, 0).map_err(font_error)?;
        Tables::read(&face)?;
        if rustybuzz::Face::from_slice(&data, 0).is_none() { return Err(font_error("it cannot be shaped.")); }
        let units_per_em = face.units_per_em() as f32;
        let (ascent, descent) = (face.ascender() as f32 / units_per_em, face.descender() as f32 / units_per_em);
        Ok(Self { data, units_per_em, ascent, descent, slots: vec![(0, None)], slot_ids: BTreeMap::from([((0, None), 0)]) })
    }

    fn slot(&mut self, gid: u16, text: Option<String>) -> Result<u16, ToolError> {
        if let Some(id) = self.slot_ids.get(&(gid, text.clone())) { return Ok(*id); }
        let id = u16::try_from(self.slots.len()).map_err(|_| ToolError::processing("The text uses more glyphs than one embedded font can hold."))?;
        self.slots.push((gid, text.clone()));
        self.slot_ids.insert((gid, text), id);
        Ok(id)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn show(&mut self, text: &str, font_size: f32) -> Result<GlyphRun, ToolError> {
        let shaped = {
            let face = rustybuzz::Face::from_slice(&self.data, 0).ok_or_else(|| font_error("it cannot be shaped."))?;
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(text);
            buffer.guess_segment_properties();
            let shaped = rustybuzz::shape(&face, &[], buffer);
            let missing = shaped.glyph_infos().iter().filter(|info| info.glyph_id == 0).filter_map(|info| text[info.cluster as usize..].chars().next()).collect::<BTreeSet<_>>();
            if !missing.is_empty() {
                return Err(ToolError::processing(format!("The font has no glyph for: {}", missing.into_iter().collect::<String>())));
            }
            shaped.glyph_infos().iter().zip(shaped.glyph_positions()).map(|(info, position)| {
                let gid = info.glyph_id as u16;
                (gid, info.cluster as usize, *position, face.glyph_hor_advance(ttf_parser::GlyphId(gid)).unwrap_or(0) as i32)
            }).collect::<Vec<_>>()
        };
        let clusters = shaped.iter().map(|(_, cluster, _, _)| *cluster).collect::<BTreeSet<_>>();
        let scale = font_size / self.units_per_em;
        let mut segments: Vec<(i32, Vec<(f32, u16)>)> = Vec::new();
        let mut shift = 0;
        let mut width = 0;
        let mut previous_cluster = None;
        for (gid, cluster, position, default_advance) in shaped {
            let cluster_text = (previous_cluster != Some(cluster)).then(|| {
                let end = clusters.range(cluster + 1..).next().copied().unwrap_or(text.len());
                text[cluster.min(end)..end].to_string()
            });
            previous_cluster = Some(cluster);
            let id = self.slot(gid, cluster_text)?;
            shift += position.x_offset;
            let adjustment = -(shift as f32) * 1000. / self.units_per_em;
            match segments.last_mut() {
                Some((rise, glyphs)) if *rise == position.y_offset => glyphs.push((adjustment, id)),
                _ => segments.push((position.y_offset, vec![(adjustment, id)])),
            }
            shift = position.x_advance - position.x_offset - default_advance;
            width += position.x_advance;
        }
        let mut operators = String::new();
        for (rise, glyphs) in &segments {
            if *rise != 0 || segments.len() > 1 { let _ = write!(operators, "{} Ts ", *rise as f32 * scale); }
            if glyphs.iter().all(|(adjustment, _)| *adjustment == 0.) {
                let _ = write!(operators, "<{}> Tj ", glyphs.iter().map(|(_, id)| format!("{id:04X}")).collect::<String>());
            } else {
                operators.push('[');
                for (adjustment, id) in glyphs {
                    if *adjustment != 0. { let _ = write!(operators, "{adjustment} "); }
                    let _ = write!(operators, "<{id:04X}> ");
                }
                operators.push_str("] TJ ");
            }
        }
        if segments.len() > 1 || segments.first().is_some_and(|(rise, _)| *rise != 0) { operators.push_str("0 Ts "); }
        Ok(GlyphRun { operators: operators.trim_end().to_string(), width: width as f32 * scale })
    }

    /// The font's ascent and descent as fractions of the font size; descent is negative.
    pub fn vertical_metrics(&self) -> (f32, f32) { (self.ascent, self.descent) }

    /// Shows text one character per glyph, without shaping or kerning, for a text layer
    /// that is read rather than seen. Every character keeps its own ToUnicode entry, so
    /// extraction gives back exactly this text; a character the font lacks is drawn
    /// with the empty .notdef glyph but still maps back to itself.
    pub fn show_unshaped(&mut self, text: &str, font_size: f32) -> Result<GlyphRun, ToolError> {
        let glyphs = {
            let face = ttf_parser::Face::parse(&self.data, 0).map_err(font_error)?;
            text.chars().map(|character| {
                let gid = face.glyph_index(character).map_or(0, |gid| gid.0);
                (gid, character.to_string(), face.glyph_hor_advance(ttf_parser::GlyphId(gid)).unwrap_or(0) as f32)
            }).collect::<Vec<_>>()
        };
        let mut codes = String::new();
        let mut width = 0.;
        for (gid, character, advance) in glyphs {
            let id = self.slot(gid, Some(character))?;
            let _ = write!(codes, "{id:04X}");
            width += advance;
        }
        Ok(GlyphRun { operators: format!("<{codes}> Tj"), width: width * font_size / self.units_per_em })
    }

    fn subset(&self) -> Result<(Vec<u8>, Vec<f32>), ToolError> {
        let face = ttf_parser::Face::parse(&self.data, 0).map_err(font_error)?;
        let tables = Tables::read(&face)?;
        let mut glyphs = self.slots.iter().map(|(gid, _)| *gid).collect::<Vec<_>>();
        let mut first_slot = BTreeMap::new();
        for (index, gid) in glyphs.iter().enumerate() { first_slot.entry(*gid).or_insert(index as u16); }
        let mut glyf = Vec::new();
        let mut loca = Vec::new();
        let mut hmtx = Vec::new();
        let mut widths = Vec::new();
        let mut index = 0;
        while index < glyphs.len() {
            let gid = glyphs[index];
            if gid >= tables.glyph_count { return Err(font_error(format!("glyph {gid} is out of range."))); }
            let mut glyph = tables.glyph(gid)?.to_vec();
            for (offset, component) in components(&glyph)? {
                if component >= tables.glyph_count { return Err(font_error(format!("glyph {component} is out of range."))); }
                let next = glyphs.len();
                let id = *first_slot.entry(component).or_insert_with(|| { glyphs.push(component); next as u16 });
                if glyphs.len() > u16::MAX as usize { return Err(ToolError::processing("The text uses more glyphs than one embedded font can hold.")); }
                glyph[offset..offset + 2].copy_from_slice(&id.to_be_bytes());
            }
            loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
            glyf.extend_from_slice(&glyph);
            glyf.resize(glyf.len().next_multiple_of(4), 0);
            let metrics = tables.metrics(gid);
            hmtx.extend_from_slice(&metrics);
            widths.push(u16::from_be_bytes([metrics[0], metrics[1]]) as f32 * 1000. / self.units_per_em);
            index += 1;
        }
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        let count = (glyphs.len() as u16).to_be_bytes();
        let mut head = tables.head.to_vec();
        head[8..12].fill(0);
        head[50..52].copy_from_slice(&1_u16.to_be_bytes());
        let mut hhea = tables.hhea.to_vec();
        hhea[34..36].copy_from_slice(&count);
        let mut maxp = tables.maxp.to_vec();
        maxp[4..6].copy_from_slice(&count);
        let mut output = vec![(*b"head", head), (*b"hhea", hhea), (*b"maxp", maxp), (*b"hmtx", hmtx), (*b"loca", loca), (*b"glyf", glyf)];
        output.extend(HINTING_TABLES.iter().filter_map(|tag| table(&face, tag).map(|data| (**tag, data.to_vec()))));
        Ok((font_file(output), widths))
    }

    fn base_font(&self, face: &ttf_parser::Face) -> String {
        let name = face.names().into_iter()
            .find(|name| name.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
            .and_then(|name| name.to_string().or_else(|| name.name.is_ascii().then(|| String::from_utf8_lossy(name.name).into_owned())))
            .map(|name| name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(56).collect::<String>())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Font".into());
        let mut digest = Sha256::new();
        for (gid, text) in &self.slots {
            digest.update(gid.to_be_bytes());
            digest.update(text.as_deref().unwrap_or("\u{FFFF}").as_bytes());
            digest.update([0]);
        }
        let tag = digest.finalize().iter().take(6).map(|byte| (b'A' + byte % 26) as char).collect::<String>();
        format!("{tag}+{name}")
    }

    fn to_unicode(&self) -> String {
        let entries = self.slots.iter().enumerate()
            .filter_map(|(id, (_, text))| text.as_deref().filter(|text| !text.is_empty()).map(|text| format!("<{id:04X}> <{}>", utf16_hex(text))))
            .collect::<Vec<_>>();
        let mut cmap = String::from("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
        for chunk in entries.chunks(100) {
            let _ = writeln!(cmap, "{} beginbfchar\n{}\nendbfchar", chunk.len(), chunk.join("\n"));
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        cmap
    }

    pub fn embed(&self, document: &mut Document) -> Result<ObjectId, ToolError> {
        let face = ttf_parser::Face::parse(&self.data, 0).map_err(font_error)?;
        let (font, widths) = self.subset()?;
        let base_font = self.base_font(&face);
        let scale = 1000. / self.units_per_em;
        let bbox = face.global_bounding_box();
        let font_file = document.add_object(compressed_stream(dictionary! { "Length1" => font.len() as i64 }, &font)?);
        let to_unicode = document.add_object(compressed_stream(dictionary! {}, self.to_unicode().as_bytes())?);
        let descriptor = document.add_object(dictionary! {
            "Type" => "FontDescriptor", "FontName" => Object::Name(base_font.clone().into_bytes()), "Flags" => 4,
            "FontBBox" => [bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max].iter().map(|value| Object::Real(*value as f32 * scale)).collect::<Vec<_>>(),
            "ItalicAngle" => Object::Real(face.italic_angle()), "Ascent" => Object::Real(face.ascender() as f32 * scale),
            "Descent" => Object::Real(face.descender() as f32 * scale),
            "CapHeight" => Object::Real(face.capital_height().unwrap_or(face.ascender()) as f32 * scale),
            "StemV" => 80, "FontFile2" => font_file,
        });
        let descendant = document.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => Object::Name(base_font.clone().into_bytes()),
            "CIDSystemInfo" => dictionary! { "Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Identity"), "Supplement" => 0 },
            "FontDescriptor" => descriptor, "CIDToGIDMap" => "Identity",
            "W" => vec![Object::Integer(0), Object::Array(widths.into_iter().map(Object::Real).collect())],
        });
        Ok(document.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => Object::Name(base_font.into_bytes()),
            "Encoding" => "Identity-H", "DescendantFonts" => vec![Object::Reference(descendant)], "ToUnicode" => to_unicode,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use crate::kit::contracts::ErrorKind;

    const DEJAVU: &[u8] = include_bytes!("../../../resources/fonts/DejaVuSans.ttf");
    const LINES: [&str; 3] = ["Toolbox AVATAR, Wave & Type", "Съешь же ещё этих мягких булок", "Ξεσκεπάζω την ψυχοφθόρα βδελυγμία"];

    fn sandbox(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("toolbox_unicode_font_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_pdf(font: Vec<u8>, lines: &[&str], output: &Path) -> Result<(), ToolError> {
        let mut font = UnicodeFont::parse(font)?;
        let mut content = String::new();
        for (index, line) in lines.iter().enumerate() {
            let run = font.show(line, 18.)?;
            assert!(run.width > 0. && run.width < 540., "{line} is {} points wide", run.width);
            content.push_str(&format!("BT /F1 18 Tf 72 {} Td {} ET\n", 720 - index * 28, run.operators));
        }
        let mut document = Document::with_version("1.7");
        let font_id = font.embed(&mut document)?;
        let pages_id = document.new_object_id();
        let content_id = document.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } }, "Contents" => content_id,
        });
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1 }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(output).map(|_| ()).map_err(|error| ToolError::processing(error.to_string()))
    }

    struct Outline(String);
    impl ttf_parser::OutlineBuilder for Outline {
        fn move_to(&mut self, x: f32, y: f32) { self.0 += &format!("M{x},{y}"); }
        fn line_to(&mut self, x: f32, y: f32) { self.0 += &format!("L{x},{y}"); }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) { self.0 += &format!("Q{x1},{y1},{x},{y}"); }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) { self.0 += &format!("C{x1},{y1},{x2},{y2},{x},{y}"); }
        fn close(&mut self) { self.0 += "Z"; }
    }

    fn outline(face: &ttf_parser::Face, gid: u16) -> String {
        let mut path = Outline(String::new());
        face.outline_glyph(ttf_parser::GlyphId(gid), &mut path);
        format!("{}:{}", face.glyph_hor_advance(ttf_parser::GlyphId(gid)).unwrap_or(0), path.0)
    }

    fn normalized(text: &str) -> String {
        text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
    }

    #[test]
    fn latin_cyrillic_and_greek_text_round_trips_through_text_extraction() {
        let root = sandbox("round_trip");
        let output = root.join("unicode.pdf");
        write_pdf(DEJAVU.to_vec(), &LINES, &output).unwrap();
        let document = Document::load(&output).unwrap();
        assert_eq!(normalized(&document.extract_text(&[1]).unwrap()), LINES.join("\n"));
        if let Ok(result) = Command::new("pdftotext").args(["-enc", "UTF-8"]).arg(&output).arg("-").output() {
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            assert_eq!(normalized(&String::from_utf8(result.stdout).unwrap()), LINES.join("\n"));
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn embedded_font_is_a_deterministic_composite_subset_of_only_the_used_glyphs() {
        let root = sandbox("subset");
        let (first, second) = (root.join("first.pdf"), root.join("second.pdf"));
        write_pdf(DEJAVU.to_vec(), &LINES, &first).unwrap();
        write_pdf(DEJAVU.to_vec(), &LINES, &second).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), std::fs::read(&second).unwrap());

        let document = Document::load(&first).unwrap();
        let page = *document.get_pages().get(&1).unwrap();
        let fonts = document.get_page_fonts(page).unwrap();
        let font = fonts.get(b"F1".as_slice()).unwrap();
        assert_eq!(font.get(b"Subtype").unwrap().as_name().unwrap(), b"Type0");
        assert_eq!(font.get(b"Encoding").unwrap().as_name().unwrap(), b"Identity-H");
        let base_font = String::from_utf8(font.get(b"BaseFont").unwrap().as_name().unwrap().to_vec()).unwrap();
        let (tag, name) = base_font.split_once('+').unwrap();
        assert!(tag.len() == 6 && tag.bytes().all(|byte| byte.is_ascii_uppercase()));
        assert_eq!(name, "DejaVuSans");
        assert!(document.get_object(font.get(b"ToUnicode").unwrap().as_reference().unwrap()).unwrap().as_stream().is_ok());

        let descendants = font.get(b"DescendantFonts").unwrap().as_array().unwrap();
        let descendant = document.get_dictionary(descendants[0].as_reference().unwrap()).unwrap();
        assert_eq!(descendant.get(b"Subtype").unwrap().as_name().unwrap(), b"CIDFontType2");
        assert_eq!(descendant.get(b"CIDToGIDMap").unwrap().as_name().unwrap(), b"Identity");
        let descriptor = document.get_dictionary(descendant.get(b"FontDescriptor").unwrap().as_reference().unwrap()).unwrap();
        let file = document.get_object(descriptor.get(b"FontFile2").unwrap().as_reference().unwrap()).unwrap().as_stream().unwrap();
        let subset = file.decompressed_content().unwrap();
        assert_eq!(file.dict.get(b"Length1").unwrap().as_i64().unwrap() as usize, subset.len());

        let original = ttf_parser::Face::parse(DEJAVU, 0).unwrap();
        let face = rustybuzz::Face::from_slice(DEJAVU, 0).unwrap();
        let mut used = BTreeSet::from([(0_u16, None::<String>)]);
        for line in LINES {
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(line);
            buffer.guess_segment_properties();
            let shaped = rustybuzz::shape(&face, &[], buffer);
            for info in shaped.glyph_infos() {
                let character = line[info.cluster as usize..].chars().next().unwrap();
                used.insert((info.glyph_id as u16, Some(character.to_string())));
            }
        }
        let tables = Tables::read(&original).unwrap();
        let mut glyphs = used.iter().map(|(gid, _)| *gid).collect::<BTreeSet<_>>();
        let mut pending = glyphs.iter().copied().collect::<Vec<_>>();
        while let Some(gid) = pending.pop() {
            for (_, component) in components(tables.glyph(gid).unwrap()).unwrap() {
                if glyphs.insert(component) { pending.push(component); }
            }
        }
        let components_only = glyphs.len() - used.iter().map(|(gid, _)| *gid).collect::<BTreeSet<_>>().len();
        let embedded = ttf_parser::Face::parse(&subset, 0).unwrap();
        assert_eq!(embedded.number_of_glyphs() as usize, used.len() + components_only);
        assert!(subset.len() * 20 < DEJAVU.len());
        let widths = descendant.get(b"W").unwrap().as_array().unwrap()[1].as_array().unwrap();
        assert_eq!(widths.len(), embedded.number_of_glyphs() as usize);
        let original_outlines = glyphs.iter().map(|gid| outline(&original, *gid)).collect::<BTreeSet<_>>();
        let embedded_outlines = (0..embedded.number_of_glyphs()).map(|gid| outline(&embedded, gid)).collect::<BTreeSet<_>>();
        assert_eq!(embedded_outlines, original_outlines);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unparseable_font_is_a_processing_error_and_writes_no_file() {
        let root = sandbox("invalid");
        for (name, bytes) in [("garbage", b"not a font".to_vec()), ("truncated", DEJAVU[..DEJAVU.len() / 3].to_vec())] {
            let output = root.join(format!("{name}.pdf"));
            let error = write_pdf(bytes, &LINES, &output).err().unwrap_or_else(|| panic!("{name} font must be rejected"));
            assert_eq!(error.kind, ErrorKind::Processing, "{name}");
            assert!(!output.exists(), "{name} must not write a file");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
