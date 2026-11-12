//! Writing text in any script into a PDF: script itemization, bidi reordering, shaping, and
//! embedded subset fonts.
//!
//! The code is built into the app. Latin, Cyrillic and Greek are written with the fonts bundled in
//! the app. Every other script that a pack in the catalog writes (`scriptFonts`) is written with
//! that pack's font; without that pack the text is refused before anything is written, with a
//! pointer to the pack. Scripts no pack writes are left to the bundled fonts, which name the
//! characters they can't show. Each output embeds a subset of every font it uses, so it never depends on
//! the pack staying installed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use flate2::{write::ZlibEncoder, Compression};
use lopdf::{dictionary, Document, Object, ObjectId, Stream, StringFormat};
use rustybuzz::ttf_parser::GlyphId;
use sha2::Digest as _;
use std::io::Write as _;
use unicode_bidi::ParagraphBidiInfo;
use unicode_script::{Script, UnicodeScript};

use crate::kit::contracts::{OutcomePointer, PackNeed, ToolError};
use crate::packs::uninstall::{RequiredPack, RunPacks};

/// The font the app writes Latin, Cyrillic and Greek with.
const BASE_FONT: &[u8] = crate::kit::images::tools::INCLUDED_WATERMARK_FONT;
const BASE_FONT_NAME: &str = "DejaVuSans";
/// Development builds read a script's font from this folder, at the path the catalog gives it in
/// the pack, when the pack is not installed.
const SCRIPT_FONT_DIR_ENV: &str = "TOOLBOX_SCRIPT_FONT_DIR";

/// The scripts the app's own font writes. Every other script needs a pack.
fn is_base_script(script: Script) -> bool {
    matches!(script, Script::Latin | Script::Cyrillic | Script::Greek | Script::Common | Script::Inherited)
}

/// The script a pack may add, by its full Unicode name, as in `Devanagari`. Scripts the app writes
/// with its own font are not pack scripts.
pub fn pack_script(name: &str) -> Option<Script> {
    Script::from_full_name(name).filter(|script| !is_base_script(*script) && *script != Script::Unknown)
}

/// The scripts in `text` that a pack may write, by name, skipping characters in no script.
fn known_pack_scripts(text: &str) -> BTreeSet<&'static str> {
    text.chars().map(|character| character.script()).filter(|script| !is_base_script(*script) && *script != Script::Unknown).map(|script| script.full_name()).collect()
}

/// The scripts in `text` that need a pack, by name.
#[cfg(test)]
fn pack_scripts(text: &str) -> Result<BTreeSet<&'static str>, String> {
    let mut scripts = BTreeSet::new();
    for character in text.chars() {
        match character.script() {
            Script::Unknown => return Err(format!("Toolbox can't write the character U+{:04X} into a PDF.", character as u32)),
            script if is_base_script(script) => {}
            script => { scripts.insert(script.full_name()); }
        }
    }
    Ok(scripts)
}

/// The fonts a scene's text is written with: the app's own font, then one font per pack font file.
pub struct ScriptFonts {
    fonts: Vec<FontFile>,
    /// A pack script's name and the index of its font in `fonts`.
    by_script: BTreeMap<&'static str, usize>,
}

struct FontFile {
    name: String,
    bytes: Vec<u8>,
}

/// One shaped glyph, placed in em units from the start of its line on the baseline, y up.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedGlyph {
    pub font: usize,
    pub glyph: u16,
    pub x: f32,
    pub y: f32,
    /// The text this glyph stands for, on the first glyph of each cluster.
    pub text: Option<String>,
    /// Whether the glyph belongs to a right-to-left run.
    pub rtl: bool,
}

impl ScriptFonts {
    /// Only the app's own font, for text in Latin, Cyrillic and Greek.
    pub fn base() -> Self {
        Self { fonts: vec![FontFile { name: BASE_FONT_NAME.into(), bytes: BASE_FONT.to_vec() }], by_script: BTreeMap::new() }
    }

    /// The fonts `texts` need. A script whose pack is not installed, is damaged or does not fit this
    /// app fails with a pointer to that pack. A script no pack in the catalog writes is skipped, so
    /// the bundled fonts name its characters. Each font is read whole, so the run never reads the
    /// pack again.
    pub fn resolve<'a>(texts: impl IntoIterator<Item = &'a str>, packs: &RunPacks) -> Result<Self, ToolError> {
        let mut scripts = BTreeSet::new();
        for text in texts {
            scripts.extend(known_pack_scripts(text));
        }
        let mut fonts = Self::base();
        if scripts.is_empty() {
            return Ok(fonts);
        }
        let catalog = packs.catalog()?;
        let mut by_path: BTreeMap<(String, String), usize> = BTreeMap::new();
        for script in scripts {
            let Some((pack, font_path)) = catalog.packs.iter()
                .find_map(|pack| pack.script_fonts.get(script).map(|font| (pack, font))) else { continue };
            if let Some(index) = by_path.get(&(pack.id.clone(), font_path.clone())) {
                fonts.by_script.insert(script, *index);
                continue;
            }
            let development_override = cfg!(debug_assertions).then(|| std::env::var_os(SCRIPT_FONT_DIR_ENV)).flatten()
                .map(|folder| PathBuf::from(folder).join(font_path));
            let resolved = packs.resolve(font_path, RequiredPack { id: &pack.id, name: &pack.name }, None, development_override)
                .map_err(|mut error| {
                    if matches!(error.pointer, Some(OutcomePointer::Pack { reason: PackNeed::NotInstalled, .. })) {
                        error.message = format!("{script} text needs the {} pack, which isn't installed. Nothing was saved.", pack.name);
                    }
                    error
                })?;
            let unreadable = || ToolError::unavailable(format!("The {} pack's font for {script} can't be read. Nothing was saved.", pack.name));
            let bytes = std::fs::read(&resolved.path).map_err(|_| unreadable())?;
            if rustybuzz::Face::from_slice(&bytes, 0).is_none() || table_directory(&bytes).is_err() {
                return Err(unreadable());
            }
            let name = font_path.rsplit('/').next().unwrap_or(font_path).trim_end_matches(".ttf")
                .chars().filter(|character| character.is_ascii_alphanumeric() || *character == '-').collect();
            fonts.fonts.push(FontFile { name, bytes });
            by_path.insert((pack.id.clone(), font_path.clone()), fonts.fonts.len() - 1);
            fonts.by_script.insert(script, fonts.fonts.len() - 1);
        }
        Ok(fonts)
    }

    /// Whether `text` has a script only a pack writes, and this set has a pack font for each such
    /// script. Other text is written with the bundled fonts.
    pub fn writes_with_packs(&self, text: &str) -> bool {
        let scripts = known_pack_scripts(text);
        !scripts.is_empty() && scripts.iter().all(|script| self.by_script.contains_key(script))
    }

    /// How many fonts the scene can use.
    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    fn face(&self, font: usize) -> Result<rustybuzz::Face<'_>, String> {
        rustybuzz::Face::from_slice(&self.fonts[font].bytes, 0).ok_or_else(|| format!("The {} font can't be read.", self.fonts[font].name))
    }

    /// Itemizes one line by script and bidi level, shapes each run, and places its glyphs in visual
    /// order. Returns the glyphs and the line's advance, in em units.
    pub fn shape_line(&self, line: &str) -> Result<(Vec<PlacedGlyph>, f32), String> {
        if line.is_empty() {
            return Ok((Vec::new(), 0.));
        }
        let faces = (0..self.fonts.len()).map(|font| self.face(font)).collect::<Result<Vec<_>, _>>()?;
        let fonts = self.line_fonts(line, &faces)?;
        let bidi = ParagraphBidiInfo::new(line, None);
        let (levels, runs) = bidi.visual_runs(0..line.len());
        let mut glyphs = Vec::new();
        let mut pen = 0f32;
        for run in runs {
            // Runs of one font inside one bidi run, in logical order, then in visual order.
            let mut pieces: Vec<(usize, std::ops::Range<usize>)> = Vec::new();
            for (offset, _) in line[run.clone()].char_indices() {
                let index = run.start + offset;
                let font = fonts[&index];
                match pieces.last_mut() {
                    Some((last, range)) if *last == font => range.end = index,
                    _ => pieces.push((font, index..index)),
                }
            }
            let mut ends = pieces.iter().skip(1).map(|(_, range)| range.start).collect::<Vec<_>>();
            ends.push(run.end);
            for ((_, range), end) in pieces.iter_mut().zip(ends) { range.end = end; }
            let rtl = levels[run.start].is_rtl();
            if rtl { pieces.reverse(); }
            for (font, range) in pieces {
                pen = shape_piece(&faces[font], font, &line[range], rtl, pen, &mut glyphs)?;
            }
        }
        Ok((glyphs, pen))
    }

    /// The font each character of `line` is written with, by byte offset. Characters shared between
    /// scripts, such as spaces and digits, follow the script before them when its font has them.
    fn line_fonts(&self, line: &str, faces: &[rustybuzz::Face]) -> Result<BTreeMap<usize, usize>, String> {
        let strong = |character: char| -> Result<Option<usize>, String> {
            match character.script() {
                Script::Common | Script::Inherited => Ok(None),
                script if is_base_script(script) => Ok(Some(0)),
                script => self.by_script.get(script.full_name()).copied().map(Some)
                    .ok_or_else(|| format!("No font for {} text", script.full_name())),
            }
        };
        let characters: Vec<(usize, char)> = line.char_indices().collect();
        let mut fonts = BTreeMap::new();
        let mut previous: Option<usize> = None;
        for (position, (index, character)) in characters.iter().enumerate() {
            let font = match strong(*character)? {
                Some(font) => font,
                None => {
                    let has = |font: usize| faces[font].glyph_index(*character).is_some();
                    let next = characters[position + 1..].iter().find_map(|(_, next)| strong(*next).ok().flatten());
                    if character.script() == Script::Inherited && previous.is_some() {
                        previous.unwrap()
                    } else if let Some(font) = previous.filter(|font| has(*font)) {
                        font
                    } else if let Some(font) = next.filter(|font| previous.is_none() && has(*font)) {
                        font
                    } else {
                        previous.or(next).filter(|font| !has(0) && has(*font)).unwrap_or(0)
                    }
                }
            };
            fonts.insert(*index, font);
            previous = Some(font);
        }
        Ok(fonts)
    }

    /// Adds Type 0 fonts that embed a subset of each font for the glyphs `used` holds.
    pub fn write_fonts(&self, doc: &mut Document, used: UsedGlyphs) -> Result<(), String> {
        for (font, glyphs) in used.glyphs {
            let id = used.ids[&font];
            let face = self.face(font)?;
            let name = &self.fonts[font].name;
            let subset = subset_truetype(&self.fonts[font].bytes, &glyphs.keys().copied().collect())?;
            let mut tag_hash = sha2::Sha256::new();
            tag_hash.update(name.as_bytes());
            for glyph in glyphs.keys() { tag_hash.update(glyph.to_be_bytes()); }
            let tag: String = tag_hash.finalize().iter().take(6).map(|byte| (b'A' + byte % 26) as char).collect();
            let base_font = format!("{tag}+{name}");
            let scale = 1000. / face.units_per_em() as f32;
            let em = |value: i16| (value as f32 * scale).round() as i64;
            let bbox = face.global_bounding_box();
            let mut file = ZlibEncoder::new(Vec::new(), Compression::default());
            file.write_all(&subset).map_err(|error| error.to_string())?;
            let compressed = file.finish().map_err(|error| error.to_string())?;
            let font_file = doc.add_object(Stream::new(dictionary! { "Length1" => subset.len() as i64, "Filter" => "FlateDecode" }, compressed));
            let descriptor = doc.add_object(dictionary! {
                "Type" => "FontDescriptor", "FontName" => Object::Name(base_font.clone().into_bytes()), "Flags" => 4,
                "FontBBox" => vec![em(bbox.x_min).into(), em(bbox.y_min).into(), em(bbox.x_max).into(), em(bbox.y_max).into()],
                "ItalicAngle" => 0, "Ascent" => em(face.ascender()), "Descent" => em(face.descender()),
                "CapHeight" => em(face.capital_height().unwrap_or(face.ascender())), "StemV" => 80, "FontFile2" => font_file,
            });
            let widths: Vec<Object> = glyphs.keys().flat_map(|glyph| {
                let advance = face.glyph_hor_advance(GlyphId(*glyph)).unwrap_or(0);
                [Object::Integer(*glyph as i64), Object::Array(vec![Object::Integer((advance as f32 * scale).round() as i64)])]
            }).collect();
            let descendant = doc.add_object(dictionary! {
                "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => Object::Name(base_font.clone().into_bytes()),
                "CIDSystemInfo" => dictionary! { "Registry" => Object::String(b"Adobe".to_vec(), StringFormat::Literal), "Ordering" => Object::String(b"Identity".to_vec(), StringFormat::Literal), "Supplement" => 0 },
                "FontDescriptor" => descriptor, "W" => widths, "CIDToGIDMap" => "Identity",
            });
            let to_unicode = doc.add_object(Stream::new(dictionary! {}, to_unicode_cmap(&glyphs).into_bytes()));
            doc.objects.insert(id, Object::Dictionary(dictionary! {
                "Type" => "Font", "Subtype" => "Type0", "BaseFont" => Object::Name(base_font.into_bytes()), "Encoding" => "Identity-H",
                "DescendantFonts" => vec![Object::Reference(descendant)], "ToUnicode" => to_unicode,
            }));
        }
        Ok(())
    }
}

fn shape_piece(face: &rustybuzz::Face, font: usize, text: &str, rtl: bool, mut pen: f32, glyphs: &mut Vec<PlacedGlyph>) -> Result<f32, String> {
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(if rtl { rustybuzz::Direction::RightToLeft } else { rustybuzz::Direction::LeftToRight });
    buffer.guess_segment_properties();
    let shaped = rustybuzz::shape(face, &[], buffer);
    let units = face.units_per_em() as f32;
    let clusters: BTreeSet<usize> = shaped.glyph_infos().iter().map(|info| info.cluster as usize).collect();
    let mut seen = BTreeSet::new();
    for (info, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        let cluster = info.cluster as usize;
        let end = clusters.range(cluster + 1..).next().copied().unwrap_or(text.len());
        let cluster_text = &text[cluster..end];
        if info.glyph_id == 0 && !cluster_text.chars().all(is_default_ignorable) {
            let missing = cluster_text.chars().find(|character| face.glyph_index(*character).is_none()).unwrap_or(' ');
            return Err(format!("Toolbox can't write “{missing}” with the font it has for this text."));
        }
        glyphs.push(PlacedGlyph {
            font,
            glyph: info.glyph_id as u16,
            x: pen + position.x_offset as f32 / units,
            y: position.y_offset as f32 / units,
            text: seen.insert(cluster).then(|| cluster_text.to_string()),
            rtl,
        });
        pen += position.x_advance as f32 / units;
    }
    Ok(pen)
}

fn is_default_ignorable(character: char) -> bool {
    matches!(character, '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}'..='\u{1160}' | '\u{17B4}'..='\u{17B5}' | '\u{180B}'..='\u{180F}'
        | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}')
}

/// The fonts and glyphs a document's text uses, with the object each font will be written to.
#[derive(Default)]
pub struct UsedGlyphs {
    ids: BTreeMap<usize, ObjectId>,
    /// Each font's glyphs and the text the first use of each glyph stood for.
    glyphs: BTreeMap<usize, BTreeMap<u16, String>>,
}

impl UsedGlyphs {
    /// The object the font will be written to, reserved on first use.
    pub fn font_object(&mut self, doc: &mut Document, font: usize) -> ObjectId {
        *self.ids.entry(font).or_insert_with(|| doc.new_object_id())
    }

    fn record(&mut self, glyph: &PlacedGlyph) {
        let entry = self.glyphs.entry(glyph.font).or_default().entry(glyph.glyph).or_default();
        if entry.is_empty() {
            if let Some(text) = &glyph.text { *entry = text.clone(); }
        }
    }
}

/// Content operators that draw `glyphs` at `size` along the text matrix `[a b c d e f]`, inside
/// marked content whose actual text is `line`, so the line extracts in reading order.
pub fn draw_line(glyphs: &[PlacedGlyph], line: &str, size: f32, [a, b, c, d, e, f]: [f32; 6], resource: &dyn Fn(usize) -> String, used: &mut UsedGlyphs) -> String {
    if glyphs.is_empty() {
        return String::new();
    }
    // Glyphs that reorder or merge characters, such as Devanagari vowel signs, extract by the
    // line's actual text. Lines with right-to-left runs extract by each glyph's text instead: the
    // bundled extractor reverses actual text it finds in a right-to-left line.
    let marked = !glyphs.iter().any(|glyph| glyph.rtl);
    let mut content = if marked {
        let actual: String = std::iter::once(0xFEFFu16).chain(line.encode_utf16()).map(|unit| format!("{unit:04X}")).collect();
        format!("/Span <</ActualText <{actual}>>> BDC BT\n")
    } else {
        String::from("BT\n")
    };
    let mut current = None;
    for glyph in glyphs {
        used.record(glyph);
        if current != Some(glyph.font) {
            content.push_str(&format!("/{} {size} Tf\n", resource(glyph.font)));
            current = Some(glyph.font);
        }
        let (x, y) = (glyph.x * size, glyph.y * size);
        content.push_str(&format!("{a} {b} {c} {d} {} {} Tm <{:04X}> Tj\n", e + a * x + c * y, f + b * x + d * y, glyph.glyph));
    }
    content.push_str(if marked { "ET EMC\n" } else { "ET\n" });
    content
}

/// A ToUnicode CMap from each glyph to the text it first stood for.
fn to_unicode_cmap(glyphs: &BTreeMap<u16, String>) -> String {
    let mut cmap = String::from("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
    let mapped: Vec<_> = glyphs.iter().filter(|(_, text)| !text.is_empty()).collect();
    for chunk in mapped.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (glyph, text) in chunk {
            let units: String = text.encode_utf16().map(|unit| format!("{unit:04X}")).collect();
            cmap.push_str(&format!("<{glyph:04X}> <{units}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    cmap
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, String> {
    bytes.get(at..at + 2).map(|value| u16::from_be_bytes([value[0], value[1]])).ok_or_else(|| "The font is truncated.".to_string())
}
fn read_u32(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes.get(at..at + 4).map(|value| u32::from_be_bytes([value[0], value[1], value[2], value[3]])).ok_or_else(|| "The font is truncated.".to_string())
}

/// The tables of a single TrueType font, by tag.
fn table_directory(bytes: &[u8]) -> Result<BTreeMap<[u8; 4], &[u8]>, String> {
    if read_u32(bytes, 0)? != 0x0001_0000 {
        return Err("Only TrueType outline fonts can be embedded.".into());
    }
    let count = read_u16(bytes, 4)? as usize;
    let mut tables = BTreeMap::new();
    for index in 0..count {
        let record = 12 + index * 16;
        let tag: [u8; 4] = bytes.get(record..record + 4).ok_or("The font is truncated.")?.try_into().map_err(|_| "The font is truncated.")?;
        let (offset, length) = (read_u32(bytes, record + 8)? as usize, read_u32(bytes, record + 12)? as usize);
        tables.insert(tag, bytes.get(offset..offset.checked_add(length).ok_or("The font is truncated.")?).ok_or("The font is truncated.")?);
    }
    for required in [b"head", b"hhea", b"maxp", b"hmtx", b"loca", b"glyf"] {
        if !tables.contains_key(required) { return Err("The font has no TrueType outlines.".into()); }
    }
    Ok(tables)
}

/// A TrueType font that keeps only `glyphs`, glyph 0 and the glyphs they are built from. Glyph ids
/// are kept, so the font maps codes to glyphs with an identity map.
fn subset_truetype(bytes: &[u8], glyphs: &BTreeSet<u16>) -> Result<Vec<u8>, String> {
    let tables = table_directory(bytes)?;
    let head = tables[b"head"];
    let long_offsets = read_u16(head, 50)? == 1;
    let count = read_u16(tables[b"maxp"], 4)? as usize;
    let (loca, glyf) = (tables[b"loca"], tables[b"glyf"]);
    let glyph_range = |glyph: usize| -> Result<std::ops::Range<usize>, String> {
        let (start, end) = if long_offsets {
            (read_u32(loca, glyph * 4)? as usize, read_u32(loca, glyph * 4 + 4)? as usize)
        } else {
            (read_u16(loca, glyph * 2)? as usize * 2, read_u16(loca, glyph * 2 + 2)? as usize * 2)
        };
        if start > end || end > glyf.len() { return Err("The font's glyph table is damaged.".into()); }
        Ok(start..end)
    };
    let mut keep: BTreeSet<usize> = glyphs.iter().map(|glyph| *glyph as usize).chain([0]).filter(|glyph| *glyph < count).collect();
    let mut pending: Vec<usize> = keep.iter().copied().collect();
    while let Some(glyph) = pending.pop() {
        let data = &glyf[glyph_range(glyph)?];
        if data.len() < 10 || (read_u16(data, 0)? as i16) >= 0 { continue; }
        let mut at = 10;
        loop {
            let flags = read_u16(data, at)?;
            let component = read_u16(data, at + 2)? as usize;
            if component < count && keep.insert(component) { pending.push(component); }
            at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
            at += if flags & 0x0008 != 0 { 2 } else if flags & 0x0040 != 0 { 4 } else if flags & 0x0080 != 0 { 8 } else { 0 };
            if flags & 0x0020 == 0 { break; }
        }
    }
    let mut new_glyf = Vec::new();
    let mut new_loca = Vec::with_capacity((count + 1) * 4);
    for glyph in 0..count {
        new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
        if keep.contains(&glyph) {
            new_glyf.extend_from_slice(&glyf[glyph_range(glyph)?]);
            while new_glyf.len() % 4 != 0 { new_glyf.push(0); }
        }
    }
    new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
    let mut new_head = head.to_vec();
    new_head.get_mut(8..12).ok_or("The font is truncated.")?.copy_from_slice(&[0; 4]);
    new_head.get_mut(50..52).ok_or("The font is truncated.")?.copy_from_slice(&1u16.to_be_bytes());
    let mut out: BTreeMap<[u8; 4], Vec<u8>> = BTreeMap::new();
    for tag in [*b"cvt ", *b"fpgm", *b"hhea", *b"hmtx", *b"maxp", *b"prep"] {
        if let Some(table) = tables.get(&tag) { out.insert(tag, table.to_vec()); }
    }
    out.insert(*b"head", new_head);
    out.insert(*b"loca", new_loca);
    out.insert(*b"glyf", new_glyf);
    let checksum = |data: &[u8]| data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    });
    let tables_count = out.len() as u16;
    let entry_selector = 15 - tables_count.leading_zeros() as u16;
    let search_range = (1u16 << entry_selector) * 16;
    let mut font = Vec::new();
    font.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    for value in [tables_count, search_range, entry_selector, tables_count * 16 - search_range] { font.extend_from_slice(&value.to_be_bytes()); }
    let mut offset = 12 + out.len() * 16;
    for (tag, data) in &out {
        font.extend_from_slice(tag);
        font.extend_from_slice(&checksum(data).to_be_bytes());
        font.extend_from_slice(&(offset as u32).to_be_bytes());
        font.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().div_ceil(4) * 4;
    }
    let mut head_at = 0;
    for (tag, data) in &out {
        if tag == b"head" { head_at = font.len(); }
        font.extend_from_slice(data);
        while font.len() % 4 != 0 { font.push(0); }
    }
    let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&font));
    font[head_at + 8..head_at + 12].copy_from_slice(&adjustment.to_be_bytes());
    // Keep the face readable after subsetting.
    rustybuzz::ttf_parser::Face::parse(&font, 0).map_err(|error| format!("The font subset is not valid: {error}"))?;
    Ok(font)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_scripts_never_need_a_pack_and_other_scripts_do() {
        assert!(pack_scripts("Hello, мир, κόσμος, Ågård – 1 ½ €").unwrap().is_empty());
        assert_eq!(pack_scripts("Received from आशा, 你好, مرحبا").unwrap(), BTreeSet::from(["Arabic", "Devanagari", "Han"]));
        assert_eq!(pack_script("Devanagari"), Some(Script::Devanagari));
        for base in ["Latin", "Cyrillic", "Greek", "Common", "Inherited", "Unknown", "Klingon"] {
            assert_eq!(pack_script(base), None, "{base}");
        }
        assert!(pack_scripts("\u{E000}").is_err());
    }

    #[test]
    fn the_subset_keeps_only_the_used_glyphs_and_their_components_under_the_same_ids() {
        let face = rustybuzz::Face::from_slice(BASE_FONT, 0).unwrap();
        let glyph_data = |font: &[u8], glyph: u16| -> Vec<u8> {
            let tables = table_directory(font).unwrap();
            let loca = tables[b"loca"];
            let at = |index: usize| if read_u16(tables[b"head"], 50).unwrap() == 1 { read_u32(loca, index * 4).unwrap() as usize } else { read_u16(loca, index * 2).unwrap() as usize * 2 };
            tables[b"glyf"][at(glyph as usize)..at(glyph as usize + 1)].to_vec()
        };
        // The first composite glyph in the font, and the first glyph it is built from.
        let composite = (1..face.number_of_glyphs()).find(|glyph| {
            let data = glyph_data(BASE_FONT, *glyph);
            data.len() > 10 && (read_u16(&data, 0).unwrap() as i16) < 0
        }).unwrap();
        let component = read_u16(&glyph_data(BASE_FONT, composite), 12).unwrap();
        let unused = face.glyph_index('Z').unwrap().0;
        assert!(unused != composite && unused != component);
        let subset = subset_truetype(BASE_FONT, &BTreeSet::from([composite])).unwrap();
        assert!(subset.len() < BASE_FONT.len() / 4, "{} bytes", subset.len());
        let parsed = rustybuzz::ttf_parser::Face::parse(&subset, 0).unwrap();
        assert_eq!(parsed.number_of_glyphs(), face.number_of_glyphs());
        assert!(glyph_data(&subset, composite).starts_with(&glyph_data(BASE_FONT, composite)));
        assert!(glyph_data(&subset, component).starts_with(&glyph_data(BASE_FONT, component)), "the composite's component is kept");
        assert!(glyph_data(&subset, unused).is_empty() && !glyph_data(BASE_FONT, unused).is_empty(), "unused glyphs are dropped");
        assert_eq!(subset, subset_truetype(BASE_FONT, &BTreeSet::from([composite])).unwrap(), "subsetting is deterministic");
    }

    #[test]
    fn mixed_direction_lines_are_placed_in_visual_order_with_their_text_in_logical_clusters() {
        let fonts = ScriptFonts::base();
        let (glyphs, width) = fonts.shape_line("Привет κόσμε").unwrap();
        assert!(width > 0. && glyphs.windows(2).all(|pair| pair[0].x < pair[1].x));
        let text: String = glyphs.iter().filter_map(|glyph| glyph.text.clone()).collect();
        assert_eq!(text, "Привет κόσμε");
    }
}
