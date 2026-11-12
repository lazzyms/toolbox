//! Toolbox-owned page-text reader built on lopdf.
//!
//! It walks a page's content stream (and the form XObjects it draws) and returns the
//! text it shows as positioned runs, in content order, with font size, bold and italic
//! flags and whether each run's characters come from a reliable Unicode mapping. It runs
//! entirely in-process, so the result is the same on every supported target.
//!
//! Coordinates follow the existing `PdfTextRun` convention: PDF points relative to the
//! page's effective crop box, with the origin at its top-left corner and `y` growing down.
//! Page `/Rotate` is not applied; runs are reported in the unrotated page space.
#![allow(dead_code)] // First consumer is PDF to EPUB (#428).

use std::collections::HashMap;
use std::rc::Rc;

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document, Encoding, Object, ObjectId};
use serde::Serialize;

/// Decompressed bytes one page may read across its content and form XObject streams.
const MAX_PAGE_CONTENT_BYTES: usize = 64 * 1024 * 1024;
/// Decompressed bytes one font's `/ToUnicode` CMap may expand to.
const MAX_CMAP_BYTES: usize = 4 * 1024 * 1024;
/// How deep form XObjects may nest before the reader stops following them.
const MAX_FORM_DEPTH: usize = 12;
/// Runs one page may produce before the reader refuses it.
const MAX_RUNS_PER_PAGE: usize = 100_000;
/// A `TJ` gap at least this wide (in thousandths of an em) reads as a word space.
const TJ_WORD_GAP: f32 = 200.0;
/// Ascent and descent (in em) used when a font gives none.
const DEFAULT_ASCENT: f32 = 0.8;
const DEFAULT_DESCENT: f32 = -0.2;
/// Advance (in thousandths of an em) used when a font has no width for a glyph.
const DEFAULT_GLYPH_WIDTH: f32 = 500.0;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageTextRun {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    pub reliable_unicode: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageText {
    pub runs: Vec<PageTextRun>,
    /// True when at least one run has non-blank text with a reliable Unicode mapping.
    pub has_extractable_text: bool,
}

/// Reads every page in page order.
pub fn read_document_text(document: &Document) -> Result<Vec<PageText>, String> {
    let mut fonts = FontCache::default();
    document.get_pages().values().map(|page_id| read_page(document, *page_id, &mut fonts)).collect()
}

/// Reads one page by its 1-based page number.
pub fn read_page_text(document: &Document, page_number: u32) -> Result<PageText, String> {
    let page_id = *document.get_pages().get(&page_number).ok_or_else(|| format!("PDF has no page {page_number}."))?;
    read_page(document, page_id, &mut FontCache::default())
}

fn read_page(document: &Document, page_id: ObjectId, fonts: &mut FontCache) -> Result<PageText, String> {
    let crop = super::metadata::effective_crop_bounds(document, page_id)?;
    let content = document.get_page_content_with_limit(page_id, MAX_PAGE_CONTENT_BYTES).map_err(|error| format!("Could not read PDF page content: {error}"))?;
    let operations = Content::decode(&content).map_err(|error| format!("Could not parse PDF page content: {error}"))?.operations;
    let resources = match super::inherited(document, page_id, b"Resources")? {
        Some(Object::Dictionary(resources)) => Some(Rc::new(resources)),
        _ => None,
    };
    let mut reader = PageReader { document, fonts, runs: Vec::new(), budget: MAX_PAGE_CONTENT_BYTES.saturating_sub(content.len()), forms: Vec::new() };
    reader.run(&operations, resources, Matrix::IDENTITY, 0)?;
    let runs: Vec<PageTextRun> = reader.runs.into_iter().map(|run| run.into_page_run(crop)).collect();
    let has_extractable_text = runs.iter().any(|run| run.reliable_unicode && !run.text.trim().is_empty());
    Ok(PageText { runs, has_extractable_text })
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Matrix { a: f32, b: f32, c: f32, d: f32, e: f32, f: f32 }

impl Matrix {
    const IDENTITY: Matrix = Matrix { a: 1., b: 0., c: 0., d: 1., e: 0., f: 0. };

    fn translate(x: f32, y: f32) -> Matrix { Matrix { e: x, f: y, ..Matrix::IDENTITY } }

    /// `self × other` in PDF's row-vector convention: apply `self`, then `other`.
    fn then(self, other: Matrix) -> Matrix {
        Matrix {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            e: self.e * other.a + self.f * other.c + other.e,
            f: self.e * other.b + self.f * other.d + other.f,
        }
    }

    fn apply(self, x: f32, y: f32) -> (f32, f32) { (x * self.a + y * self.c + self.e, x * self.b + y * self.d + self.f) }

    fn from_operands(operands: &[Object]) -> Option<Matrix> {
        let values: Vec<f32> = operands.iter().map(number).collect::<Option<_>>()?;
        let [a, b, c, d, e, f] = values.try_into().ok()?;
        let matrix = Matrix { a, b, c, d, e, f };
        [a, b, c, d, e, f].iter().all(|value| value.is_finite()).then_some(matrix)
    }

    /// True when the transformed axes lean away from perpendicular, as synthetic italic does.
    fn is_sheared(self) -> bool {
        let x_length = (self.a * self.a + self.b * self.b).sqrt();
        let y_length = (self.c * self.c + self.d * self.d).sqrt();
        if x_length == 0. || y_length == 0. {
            return false;
        }
        let cosine = (self.a * self.c + self.b * self.d) / (x_length * y_length);
        cosine.abs() > 0.08 // about 5 degrees
    }
}

fn number(value: &Object) -> Option<f32> {
    match value {
        Object::Integer(value) => Some(*value as f32),
        Object::Real(value) => Some(*value),
        _ => None,
    }
}

#[derive(Clone)]
struct TextState {
    font: Option<Rc<FontInfo>>,
    size: f32,
    char_spacing: f32,
    word_spacing: f32,
    horizontal_scale: f32,
    leading: f32,
    rise: f32,
    render_mode: i64,
}

impl Default for TextState {
    fn default() -> Self {
        TextState { font: None, size: 0., char_spacing: 0., word_spacing: 0., horizontal_scale: 1., leading: 0., rise: 0., render_mode: 0 }
    }
}

#[derive(Clone)]
struct GraphicsState { ctm: Matrix, text: TextState }

struct RawRun {
    text: String,
    corners: [(f32, f32); 4],
    font_size: f32,
    bold: bool,
    italic: bool,
    reliable_unicode: bool,
}

impl RawRun {
    fn into_page_run(self, crop: [f32; 4]) -> PageTextRun {
        let xs = self.corners.map(|corner| corner.0);
        let ys = self.corners.map(|corner| corner.1);
        let min_x = xs.iter().copied().fold(f32::INFINITY, f32::min);
        let max_x = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let min_y = ys.iter().copied().fold(f32::INFINITY, f32::min);
        let max_y = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        PageTextRun {
            text: self.text,
            x: min_x - crop[0],
            y: crop[3] - max_y,
            width: max_x - min_x,
            height: max_y - min_y,
            font_size: self.font_size,
            bold: self.bold,
            italic: self.italic,
            reliable_unicode: self.reliable_unicode,
        }
    }
}

struct PageReader<'a, 'f> {
    document: &'a Document,
    fonts: &'f mut FontCache,
    runs: Vec<RawRun>,
    budget: usize,
    forms: Vec<ObjectId>,
}

impl PageReader<'_, '_> {
    fn run(&mut self, operations: &[Operation], resources: Option<Rc<Dictionary>>, ctm: Matrix, depth: usize) -> Result<(), String> {
        let mut state = GraphicsState { ctm, text: TextState::default() };
        let mut stack: Vec<GraphicsState> = Vec::new();
        let mut text_matrix = Matrix::IDENTITY;
        let mut line_matrix = Matrix::IDENTITY;
        for operation in operations {
            let operands = operation.operands.as_slice();
            let first = || operands.first().and_then(number);
            match operation.operator.as_str() {
                "q" => stack.push(state.clone()),
                "Q" => if let Some(saved) = stack.pop() { state = saved },
                "cm" => if let Some(matrix) = Matrix::from_operands(operands) { state.ctm = matrix.then(state.ctm) },
                "BT" => { text_matrix = Matrix::IDENTITY; line_matrix = Matrix::IDENTITY; }
                "Tc" => if let Some(value) = first() { state.text.char_spacing = value },
                "Tw" => if let Some(value) = first() { state.text.word_spacing = value },
                "Tz" => if let Some(value) = first() { state.text.horizontal_scale = value / 100. },
                "TL" => if let Some(value) = first() { state.text.leading = value },
                "Ts" => if let Some(value) = first() { state.text.rise = value },
                "Tr" => if let Some(Object::Integer(mode)) = operands.first() { state.text.render_mode = *mode },
                "Tf" => {
                    if let (Some(Object::Name(name)), Some(size)) = (operands.first(), operands.get(1).and_then(number)) {
                        state.text.font = self.font(resources.as_deref(), name);
                        state.text.size = size;
                    }
                }
                "Td" | "TD" => {
                    if let (Some(x), Some(y)) = (first(), operands.get(1).and_then(number)) {
                        if operation.operator == "TD" { state.text.leading = -y; }
                        line_matrix = Matrix::translate(x, y).then(line_matrix);
                        text_matrix = line_matrix;
                    }
                }
                "Tm" => if let Some(matrix) = Matrix::from_operands(operands) { line_matrix = matrix; text_matrix = matrix },
                "T*" => { line_matrix = Matrix::translate(0., -state.text.leading).then(line_matrix); text_matrix = line_matrix; }
                "Tj" => if let Some(string @ Object::String(..)) = operands.first() { self.show(&state, &mut text_matrix, std::slice::from_ref(string))? },
                "TJ" => if let Some(Object::Array(items)) = operands.first() { self.show(&state, &mut text_matrix, items)? },
                "'" | "\"" => {
                    if operation.operator == "\"" {
                        if let (Some(word), Some(character)) = (first(), operands.get(1).and_then(number)) {
                            state.text.word_spacing = word;
                            state.text.char_spacing = character;
                        }
                    }
                    line_matrix = Matrix::translate(0., -state.text.leading).then(line_matrix);
                    text_matrix = line_matrix;
                    if let Some(string @ Object::String(..)) = operands.last() {
                        self.show(&state, &mut text_matrix, std::slice::from_ref(string))?;
                    }
                }
                "Do" => if let Some(Object::Name(name)) = operands.first() { self.form(resources.clone(), name, state.ctm, depth)? },
                _ => {}
            }
        }
        Ok(())
    }

    fn font(&mut self, resources: Option<&Dictionary>, name: &[u8]) -> Option<Rc<FontInfo>> {
        let fonts = resources?.get(b"Font").ok().and_then(|value| self.document.dereference(value).ok())?.1.as_dict().ok()?;
        let (id, font) = self.document.dereference(fonts.get(name).ok()?).ok()?;
        let font = font.as_dict().ok()?;
        // Only indirect fonts are cached; inline font dictionaries have no stable identity.
        let Some(id) = id else { return Some(Rc::new(FontInfo::read(self.document, font))) };
        if let Some(info) = self.fonts.0.get(&id) {
            return Some(info.clone());
        }
        let info = Rc::new(FontInfo::read(self.document, font));
        self.fonts.0.insert(id, info.clone());
        Some(info)
    }

    fn form(&mut self, resources: Option<Rc<Dictionary>>, name: &[u8], ctm: Matrix, depth: usize) -> Result<(), String> {
        let Some(xobjects) = resources.as_deref().and_then(|resources| resources.get(b"XObject").ok()).and_then(|value| self.document.dereference(value).ok()).and_then(|(_, value)| value.as_dict().ok()) else { return Ok(()) };
        let Some(Object::Reference(id)) = xobjects.get(name).ok() else { return Ok(()) };
        let Ok(stream) = self.document.get_object(*id).and_then(Object::as_stream) else { return Ok(()) };
        if stream.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Form".as_slice()) || depth >= MAX_FORM_DEPTH || self.forms.contains(id) {
            return Ok(());
        }
        let content = match stream.get_plain_content_with_limit(self.budget) {
            Ok(content) => content,
            Err(lopdf::Error::Decompress(lopdf::DecompressError::MemoryLimitExceeded { .. })) => return Err("PDF page content exceeds its size limit.".to_string()),
            // An undecodable form draws nothing readable; skip it like a viewer would.
            Err(_) => return Ok(()),
        };
        self.budget = self.budget.saturating_sub(content.len());
        let Ok(content) = Content::decode(&content) else { return Ok(()) };
        let matrix = stream.dict.get(b"Matrix").ok().and_then(|value| value.as_array().ok()).and_then(|values| Matrix::from_operands(values)).unwrap_or(Matrix::IDENTITY);
        let form_resources = match stream.dict.get(b"Resources").ok().and_then(|value| self.document.dereference(value).ok()).and_then(|(_, value)| value.as_dict().ok()) {
            Some(own) => Some(Rc::new(own.clone())),
            None => resources.clone(),
        };
        self.forms.push(*id);
        let result = self.run(&content.operations, form_resources, matrix.then(ctm), depth + 1);
        self.forms.pop();
        result
    }

    fn show(&mut self, state: &GraphicsState, text_matrix: &mut Matrix, items: &[Object]) -> Result<(), String> {
        let Some(font) = state.text.font.clone() else { return Ok(()) };
        let text_state = &state.text;
        let start = *text_matrix;
        let mut text = String::new();
        let mut reliable = true;
        let mut advance = 0.;
        // Text-space span of the first to last non-blank glyph, so the box hugs the visible text.
        let mut ink: Option<(f32, f32)> = None;
        for item in items {
            match item {
                Object::String(bytes, _) => {
                    for glyph in font.decode(bytes) {
                        let glyph_width = glyph.width * font.width_scale * text_state.size * text_state.horizontal_scale;
                        if !glyph.text.trim().is_empty() {
                            reliable &= glyph.reliable;
                            ink = Some((ink.map_or(advance, |(start, _)| start), advance + glyph_width));
                        }
                        text.push_str(&glyph.text);
                        let word_spacing = if glyph.single_byte_space { text_state.word_spacing } else { 0. };
                        advance += glyph_width + (text_state.char_spacing + word_spacing) * text_state.horizontal_scale;
                    }
                }
                other => if let Some(adjustment) = number(other) {
                    if adjustment <= -TJ_WORD_GAP && !text.is_empty() && !text.ends_with(char::is_whitespace) {
                        text.push(' ');
                    }
                    advance -= adjustment / 1000. * text_state.size * text_state.horizontal_scale;
                },
            }
        }
        *text_matrix = Matrix::translate(advance, 0.).then(*text_matrix);
        let Some((ink_start, ink_end)) = ink else { return Ok(()) };
        let text = text.trim().to_string();
        if self.runs.len() >= MAX_RUNS_PER_PAGE {
            return Err("PDF page has too many text runs.".to_string());
        }
        let to_page = start.then(state.ctm);
        let size = text_state.size * font.size_scale;
        let bottom = font.descent * size + text_state.rise;
        let top = font.ascent * size + text_state.rise;
        let corners = [to_page.apply(ink_start, bottom), to_page.apply(ink_end, bottom), to_page.apply(ink_start, top), to_page.apply(ink_end, top)];
        if !corners.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
            return Ok(());
        }
        let font_size = (size * (to_page.c * to_page.c + to_page.d * to_page.d).sqrt()).abs();
        self.runs.push(RawRun {
            text,
            corners,
            font_size,
            // Fill-then-stroke (modes 2 and 6) is how producers fake bold.
            bold: font.bold || matches!(text_state.render_mode, 2 | 6),
            italic: font.italic || to_page.is_sheared(),
            reliable_unicode: reliable,
        });
        Ok(())
    }
}

#[derive(Default)]
struct FontCache(HashMap<ObjectId, Rc<FontInfo>>);

struct Glyph { text: String, reliable: bool, width: f32, single_byte_space: bool }

type UnicodeMap = Box<dyn Fn(u32, u8) -> Option<String>>;

enum Codes {
    /// One byte per code; each entry is the code's Unicode text when the encoding names it.
    Simple { table: Box<[Option<char>; 256]>, fallback: Box<[Option<char>; 256]> },
    /// Multi-byte codes of a Type0 font.
    Composite { cmap: CompositeCMap },
}

#[derive(Clone, Copy, PartialEq)]
enum CompositeCMap { Identity, Utf16, Other }

enum Widths {
    Simple { first: i64, widths: Vec<f32>, missing: f32 },
    Composite { ranges: Vec<(u32, u32, CidWidth)>, default: f32 },
}

enum CidWidth { Each(Vec<f32>), Same(f32) }

struct FontInfo {
    codes: Codes,
    to_unicode: Option<UnicodeMap>,
    widths: Widths,
    /// Converts glyph-space widths to text space (1/1000 except for Type3).
    width_scale: f32,
    /// Converts the `Tf` size to an em size (1 except for Type3).
    size_scale: f32,
    ascent: f32,
    descent: f32,
    bold: bool,
    italic: bool,
}

impl FontInfo {
    fn read(document: &Document, font: &Dictionary) -> FontInfo {
        let subtype = font.get(b"Subtype").and_then(Object::as_name).unwrap_or(b"Type1");
        let composite = subtype == b"Type0";
        let descendant = composite
            .then(|| font.get_deref(b"DescendantFonts", document).ok()?.as_array().ok()?.first().and_then(|value| document.dereference(value).ok()).and_then(|(_, value)| value.as_dict().ok()))
            .flatten();
        let descriptor = descendant
            .unwrap_or(font)
            .get_deref(b"FontDescriptor", document)
            .ok()
            .and_then(|value| value.as_dict().ok());
        let descriptor_number = |key: &[u8]| descriptor.and_then(|descriptor| descriptor.get_deref(key, document).ok()).and_then(number);
        let flags = descriptor_number(b"Flags").unwrap_or(0.) as i64;
        let name = [Some(font), descendant]
            .into_iter()
            .flatten()
            .filter_map(|dictionary| dictionary.get(b"BaseFont").and_then(Object::as_name).ok())
            .map(|name| String::from_utf8_lossy(name.rsplit(|byte| *byte == b'+').next().unwrap_or(name)).to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        let bold = descriptor_number(b"FontWeight").is_some_and(|weight| weight >= 600.)
            || flags & (1 << 18) != 0
            || ["bold", "black", "heavy"].iter().any(|word| name.contains(word));
        let italic = flags & (1 << 6) != 0
            || descriptor_number(b"ItalicAngle").is_some_and(|angle| angle.abs() >= 1.)
            || ["italic", "oblique"].iter().any(|word| name.contains(word));

        let font_matrix = (subtype == b"Type3")
            .then(|| font.get_deref(b"FontMatrix", document).ok()?.as_array().ok().and_then(|values| Matrix::from_operands(values)))
            .flatten();
        let (width_scale, size_scale) = match font_matrix {
            Some(matrix) if matrix.a != 0. && matrix.d != 0. => (matrix.a, (matrix.d * 1000.).abs()),
            _ => (0.001, 1.),
        };
        let ascent = descriptor_number(b"Ascent").map(|value| value / 1000.).filter(|value| *value > 0. && *value <= 2.).unwrap_or(DEFAULT_ASCENT);
        let descent = descriptor_number(b"Descent").map(|value| value / 1000.).filter(|value| *value <= 0. && *value >= -1.).unwrap_or(DEFAULT_DESCENT);

        let to_unicode = to_unicode_map(document, font);
        let (codes, widths) = if composite {
            let cmap = match font.get(b"Encoding").and_then(Object::as_name) {
                Ok(b"Identity-H" | b"Identity-V") => CompositeCMap::Identity,
                Ok(name) if name.starts_with(b"Uni") && (name.windows(4).any(|part| part == b"UCS2") || name.windows(5).any(|part| part == b"UTF16")) => CompositeCMap::Utf16,
                _ => CompositeCMap::Other,
            };
            (Codes::Composite { cmap }, composite_widths(document, descendant))
        } else {
            let symbolic = flags & (1 << 2) != 0 && flags & (1 << 5) == 0
                || (descriptor.is_none() && (name.starts_with("symbol") || name.starts_with("zapfdingbats")));
            (simple_codes(document, font, symbolic), simple_widths(document, font, &name, descriptor_number(b"MissingWidth")))
        };
        FontInfo { codes, to_unicode, widths, width_scale, size_scale, ascent, descent, bold, italic }
    }

    fn decode(&self, bytes: &[u8]) -> Vec<Glyph> {
        let mut glyphs = Vec::new();
        match &self.codes {
            Codes::Simple { table, fallback } => {
                for byte in bytes {
                    let mapped = self.to_unicode.as_ref().and_then(|map| map(*byte as u32, 1)).or_else(|| table[*byte as usize].map(String::from));
                    let reliable = mapped.is_some();
                    let text = mapped.or_else(|| fallback[*byte as usize].map(String::from)).unwrap_or_else(|| char::REPLACEMENT_CHARACTER.to_string());
                    glyphs.push(Glyph { text, reliable, width: self.width(*byte as u32), single_byte_space: *byte == b' ' });
                }
            }
            Codes::Composite { cmap } => {
                let mut index = 0;
                while index < bytes.len() {
                    let (code, length) = self.next_composite_code(*cmap, &bytes[index..]);
                    index += length as usize;
                    let mapped = self.to_unicode.as_ref().and_then(|map| map(code, length)).or_else(|| match cmap {
                        CompositeCMap::Utf16 => utf16_text(code, length),
                        _ => None,
                    });
                    let reliable = mapped.is_some();
                    let width = if *cmap == CompositeCMap::Identity { self.width(code) } else { self.width(u32::MAX) };
                    glyphs.push(Glyph { text: mapped.unwrap_or_else(|| char::REPLACEMENT_CHARACTER.to_string()), reliable, width, single_byte_space: length == 1 && code == 32 });
                }
            }
        }
        glyphs
    }

    fn next_composite_code(&self, cmap: CompositeCMap, bytes: &[u8]) -> (u32, u8) {
        let take = |length: usize| (bytes[..length].iter().fold(0_u32, |code, byte| code << 8 | *byte as u32), length as u8);
        match cmap {
            CompositeCMap::Identity => take(bytes.len().min(2)),
            CompositeCMap::Utf16 => {
                let surrogate = bytes.len() >= 4 && (0xD8..=0xDB).contains(&bytes[0]);
                take(if surrogate { 4 } else { bytes.len().min(2) })
            }
            CompositeCMap::Other => (1..=bytes.len().min(4))
                .find(|length| self.to_unicode.as_ref().is_some_and(|map| map(take(*length).0, *length as u8).is_some()))
                .map(take)
                .unwrap_or_else(|| take(bytes.len().min(2))),
        }
    }

    fn width(&self, code: u32) -> f32 {
        match &self.widths {
            Widths::Simple { first, widths, missing } => usize::try_from(code as i64 - first).ok().and_then(|index| widths.get(index)).copied().unwrap_or(*missing),
            Widths::Composite { ranges, default } => ranges
                .iter()
                .find(|(start, end, _)| (*start..=*end).contains(&code))
                .and_then(|(start, _, width)| match width {
                    CidWidth::Same(width) => Some(*width),
                    CidWidth::Each(widths) => widths.get((code - start) as usize).copied(),
                })
                .unwrap_or(*default),
        }
    }
}

fn utf16_text(code: u32, length: u8) -> Option<String> {
    let units: Vec<u16> = if length == 4 { vec![(code >> 16) as u16, code as u16] } else { vec![code as u16] };
    usable_text(&units)
}

/// Unicode text for a mapping, or `None` when it carries no real character.
fn usable_text(units: &[u16]) -> Option<String> {
    let text: String = char::decode_utf16(units.iter().copied()).collect::<Result<_, _>>().ok()?;
    (!text.is_empty() && !text.chars().any(|character| character == char::REPLACEMENT_CHARACTER || (character.is_control() && !character.is_whitespace()))).then_some(text)
}

/// The font's `/ToUnicode` CMap, parsed by lopdf on its own so it wins over `/Encoding`.
fn to_unicode_map(document: &Document, font: &Dictionary) -> Option<UnicodeMap> {
    let reference = font.get(b"ToUnicode").ok()?.clone();
    let probe = dictionary! { "Type" => "Font", "ToUnicode" => reference };
    match probe.get_font_encoding_with_limit(document, MAX_CMAP_BYTES).ok()? {
        Encoding::UnicodeMapEncoding(cmap) => Some(Box::new(move |code, length| cmap.get(code, length).and_then(|units| usable_text(&units)))),
        _ => None,
    }
}

/// A named base encoding's code table, read from lopdf.
fn named_table(document: &Document, name: &[u8]) -> Option<[Option<char>; 256]> {
    let probe = dictionary! { "Type" => "Font", "Encoding" => Object::Name(name.to_vec()) };
    match probe.get_font_encoding(document).ok()? {
        Encoding::OneByteEncoding(table) if matches!(name, b"StandardEncoding" | b"WinAnsiEncoding" | b"MacRomanEncoding" | b"MacExpertEncoding") => {
            Some((*table).map(|glyph| glyph.and_then(|glyph| char::from_u32(glyph.utf16_code_unit() as u32))))
        }
        _ => None,
    }
}

/// The Unicode character for a glyph name, using lopdf's glyph list plus `uniXXXX`/`uXXXX` names.
fn glyph_name_char(document: &Document, name: &[u8]) -> Option<char> {
    let hex = |digits: &[u8]| std::str::from_utf8(digits).ok().and_then(|digits| u32::from_str_radix(digits, 16).ok()).and_then(char::from_u32);
    if let Some(digits) = name.strip_prefix(b"uni").filter(|digits| digits.len() == 4 && digits.iter().all(u8::is_ascii_hexdigit)) {
        return hex(digits);
    }
    if let Some(digits) = name.strip_prefix(b"u").filter(|digits| (4..=6).contains(&digits.len()) && digits.iter().all(u8::is_ascii_hexdigit)) {
        return hex(digits);
    }
    let probe = dictionary! {
        "Type" => "Font",
        "Encoding" => dictionary! { "Type" => "Encoding", "Differences" => vec![Object::Integer(0), Object::Name(name.to_vec())] },
    };
    match probe.get_font_encoding(document).ok()? {
        Encoding::Differences(differences) => differences.map.get(&0).and_then(|glyph| char::from_u32(glyph.utf16_code_unit() as u32)),
        _ => None,
    }
}

fn simple_codes(document: &Document, font: &Dictionary, symbolic: bool) -> Codes {
    let standard = named_table(document, b"StandardEncoding").unwrap_or([None; 256]);
    let fallback = named_table(document, b"WinAnsiEncoding").unwrap_or([None; 256]);
    // A symbolic font without an explicit base encoding uses its own built-in one, which has no known Unicode meaning.
    let default_table = if symbolic { [None; 256] } else { standard };
    let encoding = font.get(b"Encoding").ok().and_then(|value| document.dereference(value).ok()).map(|(_, value)| value);
    let table = match encoding {
        Some(Object::Name(name)) => named_table(document, name).unwrap_or(default_table),
        Some(Object::Dictionary(encoding)) => {
            let mut table = encoding.get(b"BaseEncoding").and_then(Object::as_name).ok().and_then(|name| named_table(document, name)).unwrap_or(default_table);
            let mut code = 0_i64;
            for item in encoding.get_deref(b"Differences", document).and_then(Object::as_array).map(Vec::as_slice).unwrap_or_default() {
                match item {
                    Object::Integer(start) => code = *start,
                    Object::Name(name) => {
                        if let Ok(index) = usize::try_from(code) {
                            if index < 256 {
                                table[index] = glyph_name_char(document, name);
                            }
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
            table
        }
        _ => default_table,
    };
    Codes::Simple { table: Box::new(table), fallback: Box::new(fallback) }
}

fn simple_widths(document: &Document, font: &Dictionary, name: &str, missing: Option<f32>) -> Widths {
    let first = font.get_deref(b"FirstChar", document).ok().and_then(number).unwrap_or(0.) as i64;
    let widths: Vec<f32> = font
        .get_deref(b"Widths", document)
        .and_then(Object::as_array)
        .map(|values| values.iter().map(|value| document.dereference(value).ok().and_then(|(_, value)| number(value)).unwrap_or(0.)).collect())
        .unwrap_or_default();
    // Without /Widths the standard Courier faces are monospaced; others get an average advance.
    let missing = missing.filter(|width| *width > 0.).unwrap_or(if name.starts_with("courier") { 600. } else { DEFAULT_GLYPH_WIDTH });
    Widths::Simple { first, widths, missing }
}

fn composite_widths(document: &Document, descendant: Option<&Dictionary>) -> Widths {
    let default = descendant.and_then(|font| font.get_deref(b"DW", document).ok()).and_then(number).unwrap_or(1000.);
    let mut ranges = Vec::new();
    let items = descendant.and_then(|font| font.get_deref(b"W", document).ok()).and_then(|value| value.as_array().ok()).map(Vec::as_slice).unwrap_or_default();
    let value = |index: usize| items.get(index).and_then(|value| document.dereference(value).ok()).map(|(_, value)| value);
    let mut index = 0;
    while index < items.len() {
        let Some(start) = value(index).and_then(number).filter(|start| *start >= 0.) else { break };
        match value(index + 1) {
            Some(Object::Array(widths)) => {
                let widths: Vec<f32> = widths.iter().map(|width| document.dereference(width).ok().and_then(|(_, width)| number(width)).unwrap_or(default)).collect();
                let end = start as u32 + widths.len().saturating_sub(1) as u32;
                ranges.push((start as u32, end, CidWidth::Each(widths)));
                index += 2;
            }
            Some(end) => {
                let (Some(end), Some(width)) = (number(end), value(index + 2).and_then(number)) else { break };
                ranges.push((start as u32, end.max(start) as u32, CidWidth::Same(width)));
                index += 3;
            }
            None => break,
        }
    }
    Widths::Composite { ranges, default }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::Stream;

    struct Fixture {
        document: Document,
        pages_id: ObjectId,
        page_ids: Vec<Object>,
    }

    impl Fixture {
        fn new() -> Self {
            let mut document = Document::with_version("1.7");
            let pages_id = document.new_object_id();
            Fixture { document, pages_id, page_ids: Vec::new() }
        }

        fn add(&mut self, object: impl Into<Object>) -> ObjectId { self.document.add_object(object) }

        fn page(&mut self, resources: Dictionary, content: &str) {
            let content_id = self.add(Stream::new(Dictionary::new(), content.as_bytes().to_vec()));
            let page_id = self.add(dictionary! {
                "Type" => "Page",
                "Parent" => self.pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => resources,
                "Contents" => content_id,
            });
            self.page_ids.push(page_id.into());
        }

        /// Saves and reloads the document so the reader sees a real PDF file.
        fn finish(mut self) -> Document {
            let count = self.page_ids.len() as i64;
            self.document.objects.insert(self.pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => self.page_ids, "Count" => count }));
            let catalog_id = self.document.add_object(dictionary! { "Type" => "Catalog", "Pages" => self.pages_id });
            self.document.trailer.set("Root", catalog_id);
            let mut bytes = Vec::new();
            self.document.save_to(&mut bytes).unwrap();
            Document::load_mem(&bytes).unwrap()
        }
    }

    /// A WinAnsi Type1 font whose printable glyphs are all half an em wide.
    fn simple_font(fixture: &mut Fixture, base_font: &str, descriptor: Option<Dictionary>) -> ObjectId {
        let mut font = dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => base_font,
            "Encoding" => "WinAnsiEncoding",
            "FirstChar" => 32,
            "LastChar" => 126,
            "Widths" => vec![Object::Integer(500); 95],
        };
        if let Some(descriptor) = descriptor {
            let descriptor_id = fixture.add(descriptor);
            font.set("FontDescriptor", descriptor_id);
        }
        fixture.add(font)
    }

    fn fonts(entries: &[(&str, ObjectId)]) -> Dictionary {
        let mut fonts = Dictionary::new();
        for (name, id) in entries {
            fonts.set(*name, *id);
        }
        dictionary! { "Font" => fonts }
    }

    /// An Identity-H Type0 font with 600-unit glyphs, optionally carrying a ToUnicode CMap.
    fn type0_font(fixture: &mut Fixture, to_unicode: Option<&str>) -> ObjectId {
        let descriptor = fixture.add(dictionary! { "Type" => "FontDescriptor", "FontName" => "ABCDEF+NotoSans", "Flags" => 4, "Ascent" => 900, "Descent" => -300, "ItalicAngle" => 0 });
        let descendant = fixture.add(dictionary! {
            "Type" => "Font",
            "Subtype" => "CIDFontType2",
            "BaseFont" => "ABCDEF+NotoSans",
            "CIDSystemInfo" => dictionary! { "Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Identity"), "Supplement" => 0 },
            "FontDescriptor" => descriptor,
            "DW" => 1000,
            "W" => vec![Object::Integer(1), Object::Array(vec![Object::Integer(600); 8])],
        });
        let mut font = dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => "ABCDEF+NotoSans",
            "Encoding" => "Identity-H",
            "DescendantFonts" => vec![Object::Reference(descendant)],
        };
        if let Some(cmap) = to_unicode {
            let cmap_id = fixture.add(Stream::new(Dictionary::new(), cmap.as_bytes().to_vec()));
            font.set("ToUnicode", cmap_id);
        }
        fixture.add(font)
    }

    fn close(actual: f32, expected: f32) -> bool { (actual - expected).abs() < 0.01 }

    fn assert_run(run: &PageTextRun, text: &str, x: f32, y: f32, width: f32, height: f32, font_size: f32) {
        assert_eq!(run.text, text);
        assert!(close(run.x, x) && close(run.y, y) && close(run.width, width) && close(run.height, height), "{run:?} should sit at ({x}, {y}) sized {width}x{height}");
        assert!(close(run.font_size, font_size), "{run:?} should be {font_size}pt");
    }

    #[test]
    fn reads_a_simple_page_as_positioned_runs_in_content_order() {
        let mut fixture = Fixture::new();
        let font = simple_font(&mut fixture, "Helvetica", None);
        fixture.page(fonts(&[("F1", font)]), "BT /F1 12 Tf 72 700 Td (Hello) Tj 0 -20 Td (World) Tj ET BT /F1 12 Tf 72 640 Td [(Two) -300 (words)] TJ ET");
        let page = read_page_text(&fixture.finish(), 1).unwrap();

        assert!(page.has_extractable_text);
        assert_eq!(page.runs.len(), 3);
        // Default em box: ascent 0.8, descent -0.2. Top-left origin: y = 792 - (baseline + 0.8 * size).
        assert_run(&page.runs[0], "Hello", 72., 82.4, 30., 12., 12.);
        assert_run(&page.runs[1], "World", 72., 102.4, 30., 12., 12.);
        // A TJ gap wider than 0.2 em reads as a space; the gap itself counts toward the width.
        assert_run(&page.runs[2], "Two words", 72., 142.4, 8. * 6. + 3.6, 12., 12.);
        assert!(page.runs.iter().all(|run| run.reliable_unicode && !run.bold && !run.italic));
    }

    #[test]
    fn reports_each_font_size_including_sizes_scaled_by_the_text_matrix() {
        let mut fixture = Fixture::new();
        let font = simple_font(&mut fixture, "Helvetica", None);
        fixture.page(fonts(&[("F1", font)]), "BT /F1 24 Tf 72 700 Td (Title) Tj ET BT /F1 12 Tf 72 660 Td (Body) Tj ET q 1 0 0 1 10 0 cm BT /F1 6 Tf 2 0 0 2 62 600 Tm (Scaled) Tj ET Q");
        let page = read_page_text(&fixture.finish(), 1).unwrap();

        assert_eq!(page.runs.iter().map(|run| run.text.as_str()).collect::<Vec<_>>(), ["Title", "Body", "Scaled"]);
        assert_run(&page.runs[0], "Title", 72., 792. - 719.2, 60., 24., 24.);
        assert_run(&page.runs[1], "Body", 72., 792. - 669.6, 24., 12., 12.);
        // 6pt text drawn through a 2x text matrix and a 10pt translation renders as 12pt at x = 72.
        assert_run(&page.runs[2], "Scaled", 72., 792. - 609.6, 36., 12., 12.);
    }

    #[test]
    fn flags_bold_and_italic_runs_from_font_names_descriptors_and_render_tricks() {
        let mut fixture = Fixture::new();
        let regular = simple_font(&mut fixture, "Helvetica", None);
        let bold = simple_font(&mut fixture, "Helvetica-Bold", None);
        let heavy = simple_font(&mut fixture, "ABCDEF+Custom", Some(dictionary! { "Type" => "FontDescriptor", "FontName" => "ABCDEF+Custom", "Flags" => 32, "FontWeight" => 700, "ItalicAngle" => 0, "Ascent" => 700, "Descent" => -200 }));
        let italic = simple_font(&mut fixture, "Times-Italic", None);
        let slanted = simple_font(&mut fixture, "ABCDEF+Serif", Some(dictionary! { "Type" => "FontDescriptor", "FontName" => "ABCDEF+Serif", "Flags" => 32 | 64, "ItalicAngle" => -12 }));
        fixture.page(
            fonts(&[("R", regular), ("B", bold), ("H", heavy), ("I", italic), ("S", slanted)]),
            "BT /R 12 Tf 72 700 Td (plain ) Tj /B 12 Tf (bold) Tj /R 12 Tf ( text) Tj ET \
             BT /H 12 Tf 72 680 Td (weight) Tj /I 12 Tf (italic) Tj /S 12 Tf (flag) Tj ET \
             BT /R 12 Tf 2 Tr 72 660 Td (stroked) Tj 0 Tr 1 0 0.25 1 72 640 Tm (skewed) Tj ET",
        );
        let page = read_page_text(&fixture.finish(), 1).unwrap();
        let flags: Vec<(&str, bool, bool)> = page.runs.iter().map(|run| (run.text.as_str(), run.bold, run.italic)).collect();
        assert_eq!(flags, [
            ("plain", false, false),
            ("bold", true, false),
            ("text", false, false),
            ("weight", true, false),
            ("italic", false, true),
            ("flag", false, true),
            ("stroked", true, false),
            ("skewed", false, true),
        ]);
        // "bold" starts after six half-em glyphs of "plain ", and its run uses the descriptor-free em box.
        assert_run(&page.runs[1], "bold", 72. + 36., 82.4, 24., 12., 12.);
        // The descriptor's ascent and descent set the box height.
        assert_run(&page.runs[3], "weight", 72., 792. - (680. + 8.4), 36., 10.8, 12.);
    }

    #[test]
    fn flags_runs_in_fonts_without_a_unicode_mapping_as_unreliable() {
        let mut fixture = Fixture::new();
        let unmapped = type0_font(&mut fixture, None);
        let custom = fixture.add(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "ABCDEF+Glyphs",
            "Encoding" => dictionary! { "Type" => "Encoding", "Differences" => vec![Object::Integer(1), Object::Name(b"g1".to_vec()), Object::Name(b"g2".to_vec())] },
            "FirstChar" => 1,
            "LastChar" => 2,
            "Widths" => vec![Object::Integer(500), Object::Integer(500)],
        });
        fixture.page(fonts(&[("U", unmapped), ("C", custom)]), "BT /U 10 Tf 72 700 Td <00010002> Tj ET BT /C 10 Tf 72 680 Td <0102> Tj ET");
        let page = read_page_text(&fixture.finish(), 1).unwrap();

        assert_eq!(page.runs.len(), 2);
        assert!(page.runs.iter().all(|run| !run.reliable_unicode), "{:?}", page.runs);
        // Positions and widths still come through: two 600-unit CIDs at 10pt.
        assert!(close(page.runs[0].width, 12.) && close(page.runs[0].x, 72.));
        assert!(!page.has_extractable_text, "unmapped glyphs are not extractable text");
    }

    #[test]
    fn decodes_a_mapped_non_latin_run_to_the_right_characters() {
        let cmap = "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
            /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
            /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
            1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n\
            3 beginbfchar\n<0001> <041F>\n<0002> <0440>\n<0003> <0438>\nendbfchar\n\
            1 beginbfrange\n<0004> <0005> <65E5>\nendbfrange\n\
            endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n";
        let mut fixture = Fixture::new();
        let mapped = type0_font(&mut fixture, Some(cmap));
        let latin = simple_font(&mut fixture, "Helvetica", None);
        fixture.page(fonts(&[("M", mapped), ("L", latin)]), "BT /M 10 Tf 72 700 Td <000100020003> Tj /L 10 Tf ( ok) Tj /M 10 Tf <00040005> Tj ET");
        let page = read_page_text(&fixture.finish(), 1).unwrap();

        assert_eq!(page.runs.iter().map(|run| run.text.as_str()).collect::<Vec<_>>(), ["При", "ok", "日旦"]);
        assert!(page.runs.iter().all(|run| run.reliable_unicode));
        assert!(page.has_extractable_text);
        // Descriptor ascent 0.9 / descent -0.3, three 600-unit CIDs at 10pt.
        assert_run(&page.runs[0], "При", 72., 792. - 709., 18., 12., 10.);
    }

    #[test]
    fn reports_no_extractable_text_on_an_image_only_page() {
        let mut fixture = Fixture::new();
        let image = fixture.add(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 },
            vec![0x80],
        ));
        fixture.page(dictionary! { "XObject" => dictionary! { "Im1" => image } }, "q 612 0 0 792 0 0 cm /Im1 Do Q");
        let page = read_page_text(&fixture.finish(), 1).unwrap();
        assert_eq!(page, PageText { runs: Vec::new(), has_extractable_text: false });
    }

    #[test]
    fn follows_form_xobjects_in_content_order_with_their_own_resources_and_matrix() {
        let mut fixture = Fixture::new();
        let font = simple_font(&mut fixture, "Helvetica", None);
        let form = fixture.add(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 200.into(), 50.into()], "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 100.into(), 0.into()], "Resources" => fonts(&[("FF", font)]) },
            b"BT /FF 10 Tf 0 0 Td (inside) Tj ET".to_vec(),
        ));
        let mut resources = fonts(&[("F1", font)]);
        resources.set("XObject", dictionary! { "Fm1" => form });
        fixture.page(resources, "BT /F1 10 Tf 72 700 Td (before) Tj ET q 1 0 0 1 0 500 cm /Fm1 Do Q BT /F1 10 Tf 72 400 Td (after) Tj ET");
        let page = read_page_text(&fixture.finish(), 1).unwrap();

        assert_eq!(page.runs.iter().map(|run| run.text.as_str()).collect::<Vec<_>>(), ["before", "inside", "after"]);
        assert_run(&page.runs[1], "inside", 100., 792. - 508., 30., 10., 10.);
    }

    #[test]
    fn reads_every_page_and_measures_from_the_crop_box() {
        let mut fixture = Fixture::new();
        let font = simple_font(&mut fixture, "Helvetica", None);
        fixture.page(fonts(&[("F1", font)]), "BT /F1 10 Tf 100 700 Td (first) Tj ET");
        fixture.page(fonts(&[("F1", font)]), "");
        let mut document = fixture.finish();
        let first = *document.get_pages().get(&1).unwrap();
        document.get_dictionary_mut(first).unwrap().set("CropBox", vec![50.into(), 100.into(), 562.into(), 742.into()]);

        let pages = read_document_text(&document).unwrap();
        assert_eq!(pages.len(), 2);
        assert_run(&pages[0].runs[0], "first", 50., 742. - 708., 25., 10., 10.);
        assert!(!pages[1].has_extractable_text && pages[1].runs.is_empty());
        assert!(read_page_text(&document, 3).is_err());
    }
}
