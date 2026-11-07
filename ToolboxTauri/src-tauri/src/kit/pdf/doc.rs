//! Reads a Word 97-2003 (DOC) document into the shared word-processing model: the main text with its basic character and
//! paragraph formatting, headings, tables and links. Macros and embedded objects are never run; pictures and objects are counted
//! as left out.

use std::collections::HashMap;

use encoding_rs::WINDOWS_1252;

use crate::kit::contracts::ToolError;
use crate::kit::pdf::compound::{i16_at, i32_at, u16_at, u32_at, utf16, Compound};
use crate::kit::pdf::layout::{plain_text, Align, Block, Omitted, PageSetup, Paragraph, Span, Style};
use crate::kit::pdf::rtf::hyperlink;
pub use crate::kit::pdf::wordprocessing::Parsed;
use crate::kit::pdf::wordprocessing::{Families, Out};

pub const DAMAGED: &str = "This Word 97-2003 document is damaged or incomplete, so no PDF was made.";
pub const TOO_OLD: &str = "This document was saved by Word 95 or earlier, which Convert to PDF doesn't read. Save it as a newer document, then convert that.";

const WORD_IDENT: u16 = 0xA5EC;
const WORD_97: u16 = 0x00C1;
const PAGE: usize = 512;
const DEFAULT_HALF_POINTS: u16 = 20;
const MAX_STYLE_CHAIN: usize = 16;
const NO_STYLE: u16 = 0x0FFF;
// Places in the FIB's table of (offset, length) pairs.
const STYLES: usize = 1;
const SECTIONS: usize = 6;
const CHARACTER_RUNS: usize = 12;
const PARAGRAPH_RUNS: usize = 13;
const FONTS: usize = 15;
const PIECES: usize = 33;

fn damaged() -> ToolError { ToolError::invalid_input(DAMAGED) }

fn twips(value: i32) -> f32 { value as f32 / 20. }

/// The parts of the File Information Block this reader uses.
struct Fib { table: &'static str, text_length: u32, places: Vec<(u32, u32)> }

impl Fib {
    fn read(word: &[u8]) -> Result<Self, ToolError> {
        if u16_at(word, 0) != Some(WORD_IDENT) { return Err(damaged()); }
        if u16_at(word, 2).ok_or_else(damaged)? < WORD_97 { return Err(ToolError::invalid_input(TOO_OLD)); }
        let table = if u16_at(word, 10).ok_or_else(damaged)? & 0x0200 != 0 { "/1Table" } else { "/0Table" };
        let shorts = u16_at(word, 32).ok_or_else(damaged)? as usize;
        let longs_at = 34 + shorts * 2;
        let longs = u16_at(word, longs_at).ok_or_else(damaged)? as usize;
        let text_length = u32_at(word, longs_at + 2 + 3 * 4).ok_or_else(damaged)?;
        let pairs_at = longs_at + 2 + longs * 4;
        let pairs = u16_at(word, pairs_at).ok_or_else(damaged)? as usize;
        let places = (0..pairs).map(|index| Some((u32_at(word, pairs_at + 2 + index * 8)?, u32_at(word, pairs_at + 6 + index * 8)?))).collect::<Option<Vec<_>>>().ok_or_else(damaged)?;
        Ok(Self { table, text_length, places })
    }

    /// The bytes of a table-stream structure, or `None` when the document doesn't have it.
    fn part<'t>(&self, table: &'t [u8], place: usize) -> Option<&'t [u8]> {
        let (offset, length) = *self.places.get(place)?;
        if length == 0 { return None; }
        table.get(offset as usize..offset as usize + length as usize)
    }
}

/// A piece of the main text: the character positions it covers and where its characters are stored.
struct Piece { start: u32, end: u32, offset: usize, compressed: bool }

fn pieces(clx: &[u8]) -> Option<Vec<Piece>> {
    let mut offset = 0;
    loop {
        match clx.get(offset)? {
            1 => offset += 3 + u16_at(clx, offset + 1)? as usize,
            2 => {
                let length = u32_at(clx, offset + 1)? as usize;
                let plc = clx.get(offset + 5..offset + 5 + length)?;
                if length < 4 || (length - 4) % 12 != 0 { return None; }
                let count = (length - 4) / 12;
                return (0..count).map(|index| {
                    let (start, end) = (u32_at(plc, index * 4)?, u32_at(plc, index * 4 + 4)?);
                    let place = u32_at(plc, (count + 1) * 4 + index * 8 + 2)?;
                    let compressed = place & 0x4000_0000 != 0;
                    let offset = if compressed { (place & 0x3FFF_FFFF) as usize / 2 } else { place as usize };
                    (start <= end).then_some(Piece { start, end, offset, compressed })
                }).collect();
            }
            _ => return None,
        }
    }
}

/// The main text as UTF-16 code units, each with the stream offset it was read from.
fn text(word: &[u8], pieces: &[Piece], length: u32) -> Result<Vec<(u16, u32)>, ToolError> {
    let mut units = Vec::new();
    let mut reached = 0;
    for piece in pieces {
        if piece.start < reached || piece.start >= length { continue; }
        let end = piece.end.min(length);
        let count = (end - piece.start) as usize;
        reached = end;
        if piece.compressed {
            let bytes = word.get(piece.offset..piece.offset + count).ok_or_else(damaged)?;
            let decoded = WINDOWS_1252.decode_without_bom_handling(bytes).0;
            units.extend(decoded.encode_utf16().zip(piece.offset as u32..));
        } else {
            let bytes = word.get(piece.offset..piece.offset + count * 2).ok_or_else(damaged)?;
            units.extend(bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).zip((piece.offset as u32..).step_by(2)));
        }
    }
    Ok(units)
}

/// Formatting that applies to a range of stream offsets.
struct Run { start: u32, end: u32, istd: u16, grpprl: Vec<u8> }

struct Runs(Vec<Run>);

impl Runs {
    /// Reads the formatted-disk-page runs listed by a bin table; `paragraphs` selects paragraph pages over character pages.
    fn read(word: &[u8], bins: Option<&[u8]>, paragraphs: bool) -> Self {
        let mut runs = Vec::new();
        let Some(bins) = bins.filter(|bins| bins.len() >= 4) else { return Self(runs) };
        let count = (bins.len() - 4) / 8;
        for index in 0..count {
            let Some(number) = u32_at(bins, (count + 1) * 4 + index * 4) else { break };
            let start = (number & 0x003F_FFFF) as usize * PAGE;
            let Some(page) = word.get(start..start + PAGE) else { continue };
            let entries = page[PAGE - 1] as usize;
            for entry in 0..entries {
                let (Some(from), Some(to)) = (u32_at(page, entry * 4), u32_at(page, entry * 4 + 4)) else { break };
                let at = if paragraphs { (entries + 1) * 4 + entry * 13 } else { (entries + 1) * 4 + entry };
                let place = page.get(at).copied().unwrap_or(0) as usize * 2;
                let (istd, grpprl) = if place == 0 { (0, Vec::new()) } else if paragraphs { paragraph_properties(page, place).unwrap_or_default() } else { (0, character_properties(page, place).unwrap_or_default()) };
                runs.push(Run { start: from, end: to, istd, grpprl });
            }
        }
        runs.sort_by_key(|run| run.start);
        Self(runs)
    }

    fn at(&self, offset: u32) -> Option<&Run> {
        let index = self.0.partition_point(|run| run.start <= offset).checked_sub(1)?;
        self.0.get(index).filter(|run| offset < run.end)
    }
}

fn character_properties(page: &[u8], place: usize) -> Option<Vec<u8>> {
    let size = *page.get(place)? as usize;
    Some(page.get(place + 1..place + 1 + size)?.to_vec())
}

fn paragraph_properties(page: &[u8], place: usize) -> Option<(u16, Vec<u8>)> {
    let (start, size) = match *page.get(place)? as usize {
        0 => (place + 2, *page.get(place + 1)? as usize * 2),
        size => (place + 1, size * 2 - 1),
    };
    let bytes = page.get(start..start + size)?;
    Some((u16_at(bytes, 0)?, bytes.get(2..)?.to_vec()))
}

/// Each property (sprm) in a list, with its operand.
fn sprms(grpprl: &[u8]) -> impl Iterator<Item = (u16, &[u8])> {
    let mut offset = 0;
    std::iter::from_fn(move || {
        let sprm = u16_at(grpprl, offset)?;
        let start = offset + 2;
        let size = match sprm >> 13 {
            0 | 1 => 1,
            2 | 4 | 5 => 2,
            3 => 4,
            7 => 3,
            _ if matches!(sprm, 0xD606 | 0xD608) => u16_at(grpprl, start)? as usize + 1,
            _ => *grpprl.get(start)? as usize + 1,
        };
        let operand = grpprl.get(start..start + size)?;
        offset = start + size;
        Some((sprm, operand))
    })
}

/// The Word 97 sixteen-colour palette.
fn palette(index: u8) -> Option<[u8; 3]> {
    Some(match index {
        2 => [0, 0, 255],
        3 => [0, 255, 255],
        4 => [0, 255, 0],
        5 => [255, 0, 255],
        6 => [255, 0, 0],
        7 => [255, 255, 0],
        8 => [255, 255, 255],
        9 => [0, 0, 128],
        10 => [0, 128, 128],
        11 => [0, 128, 0],
        12 => [128, 0, 128],
        13 => [128, 0, 0],
        14 => [128, 128, 0],
        15 => [128, 128, 128],
        16 => [192, 192, 192],
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy)]
struct Chp { bold: bool, italic: bool, strike: bool, underline: bool, hidden: bool, deleted: bool, special: bool, object: bool, half_points: u16, font: u16, color: Option<[u8; 3]>, style: Option<u16> }

impl Chp {
    fn new(font: u16) -> Self {
        Self { bold: false, italic: false, strike: false, underline: false, hidden: false, deleted: false, special: false, object: false, half_points: DEFAULT_HALF_POINTS, font, color: None, style: None }
    }

    /// Applies character properties; a toggle of 0x80 keeps the style's value and 0x81 flips it.
    fn apply(&mut self, grpprl: &[u8], style: &Chp) {
        let toggle = |operand: &[u8], inherited: bool| match operand.first() { Some(0x80) => inherited, Some(0x81) => !inherited, Some(value) => *value != 0, None => inherited };
        for (sprm, operand) in sprms(grpprl) {
            match sprm {
                0x0835 => self.bold = toggle(operand, style.bold),
                0x0836 => self.italic = toggle(operand, style.italic),
                0x0837 | 0x2A53 => self.strike = toggle(operand, style.strike),
                0x083C => self.hidden = toggle(operand, style.hidden),
                0x0800 => self.deleted = operand.first().is_some_and(|value| *value != 0),
                0x0855 => self.special = operand.first().is_some_and(|value| *value != 0),
                0x080A => self.object = operand.first().is_some_and(|value| *value != 0),
                0x2A3E => self.underline = operand.first().is_some_and(|value| *value != 0),
                0x4A43 => if let Some(size) = u16_at(operand, 0).filter(|size| (2..=3276).contains(size)) { self.half_points = size },
                0x4A4F => if let Some(font) = u16_at(operand, 0) { self.font = font },
                0x2A42 => self.color = operand.first().copied().and_then(palette),
                0x6870 => self.color = operand.get(..4).filter(|colour| colour[3] != 0xFF).map(|colour| [colour[0], colour[1], colour[2]]).filter(|colour| *colour != [0, 0, 0]),
                0x4A30 => self.style = u16_at(operand, 0),
                _ => {}
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Pap { align: Align, in_table: bool, row_end: bool, before: i32, after: i32, indent: i32, right_indent: i32 }

impl Pap {
    fn apply(&mut self, grpprl: &[u8]) {
        for (sprm, operand) in sprms(grpprl) {
            let byte = operand.first().copied().unwrap_or(0);
            match sprm {
                0x2403 | 0x2461 => self.align = match byte { 1 => Align::Center, 2 => Align::Right, _ => Align::Left },
                0x2416 => self.in_table = byte != 0,
                0x2417 => self.row_end = byte != 0,
                0x6649 => if let Some(depth) = i32_at(operand, 0) { self.in_table |= depth > 0 },
                0xA413 => self.before = u16_at(operand, 0).map_or(0, i32::from),
                0xA414 => self.after = u16_at(operand, 0).map_or(0, i32::from),
                0x840F | 0x845E => self.indent = i16_at(operand, 0).map_or(0, i32::from),
                0x840E | 0x845D => self.right_indent = i16_at(operand, 0).map_or(0, i32::from),
                _ => {}
            }
        }
    }
}

/// One stylesheet entry: its built-in identifier (1 to 9 are the headings), the style it is based on and its own properties.
#[derive(Default, Clone)]
struct StyleEntry { sti: u16, base: u16, paragraph: Vec<u8>, characters: Vec<u8> }

struct Styles { entries: Vec<Option<StyleEntry>>, default_font: u16, characters: HashMap<u16, Chp> }

impl Styles {
    fn read(bytes: Option<&[u8]>) -> Self {
        let mut styles = Self { entries: Vec::new(), default_font: 0, characters: HashMap::new() };
        let Some(bytes) = bytes else { return styles };
        let (Some(header), Some(count), Some(base_size)) = (u16_at(bytes, 0), u16_at(bytes, 2), u16_at(bytes, 4)) else { return styles };
        styles.default_font = u16_at(bytes, 2 + 12).filter(|_| header >= 14).unwrap_or(0);
        let mut offset = 2 + header as usize;
        for _ in 0..count {
            let Some(size) = u16_at(bytes, offset) else { break };
            let entry = bytes.get(offset + 2..offset + 2 + size as usize).and_then(|entry| style_entry(entry, base_size as usize));
            styles.entries.push(entry);
            offset += 2 + size as usize;
        }
        styles
    }

    fn entry(&self, istd: u16) -> Option<&StyleEntry> { self.entries.get(istd as usize)?.as_ref() }

    /// The styles from the root of `istd`'s chain down to `istd` itself.
    fn chain(&self, istd: u16) -> Vec<&StyleEntry> {
        let mut chain = Vec::new();
        let mut current = istd;
        while let Some(entry) = self.entry(current) {
            chain.push(entry);
            if chain.len() >= MAX_STYLE_CHAIN || entry.base == NO_STYLE { break; }
            current = entry.base;
        }
        chain.reverse();
        chain
    }

    fn characters(&mut self, istd: u16) -> Chp {
        if let Some(chp) = self.characters.get(&istd) { return *chp; }
        let mut chp = Chp::new(self.default_font);
        for entry in self.chain(istd) {
            let parent = chp;
            chp.apply(&entry.characters, &parent);
        }
        self.characters.insert(istd, chp);
        chp
    }

    fn paragraph(&self, istd: u16) -> Pap {
        let mut pap = Pap::default();
        for entry in self.chain(istd) { pap.apply(&entry.paragraph); }
        pap
    }

    fn heading(&self, istd: u16) -> Option<u8> { self.entry(istd).map(|entry| entry.sti).filter(|sti| (1..=9).contains(sti)).map(|sti| sti.min(6) as u8) }
}

fn style_entry(entry: &[u8], base_size: usize) -> Option<StyleEntry> {
    let sti = u16_at(entry, 0)? & 0x0FFF;
    let kind = u16_at(entry, 2)? & 0x000F;
    let base = u16_at(entry, 2)? >> 4;
    let names = u16_at(entry, base_size)? as usize;
    let mut offset = base_size + 2 + names * 2 + 2;
    let mut properties = Vec::new();
    for _ in 0..if kind == 1 { 2 } else { 1 } {
        let size = u16_at(entry, offset)? as usize;
        properties.push(entry.get(offset + 2..offset + 2 + size)?.to_vec());
        offset += 2 + size + size % 2;
    }
    let (paragraph, characters) = match kind {
        1 => (properties[0].get(2..).unwrap_or_default().to_vec(), properties.get(1).cloned().unwrap_or_default()),
        _ => (Vec::new(), properties.remove(0)),
    };
    Some(StyleEntry { sti, base, paragraph, characters })
}

/// Font names by font number, from the font table.
fn font_names(bytes: Option<&[u8]>) -> Vec<String> {
    let Some(bytes) = bytes else { return Vec::new() };
    let count = u16_at(bytes, 0).unwrap_or(0);
    let mut names = Vec::new();
    let mut offset = 4;
    for _ in 0..count {
        let Some(size) = bytes.get(offset).map(|size| *size as usize) else { break };
        names.push(bytes.get(offset + 40..offset + 1 + size).map(utf16).unwrap_or_default().trim().to_string());
        offset += 1 + size;
    }
    names
}

/// The first section's page size and margins.
fn page_setup(word: &[u8], sections: Option<&[u8]>) -> PageSetup {
    let mut page = PageSetup::default();
    let Some(sections) = sections.filter(|sections| sections.len() >= 16) else { return page };
    let count = (sections.len() - 4) / 16;
    let Some(place) = u32_at(sections, (count + 1) * 4 + 2) else { return page };
    let Some(size) = u16_at(word, place as usize) else { return page };
    let Some(grpprl) = word.get(place as usize + 2..place as usize + 2 + size as usize) else { return page };
    for (sprm, operand) in sprms(grpprl) {
        let Some(value) = u16_at(operand, 0).map(|value| twips(i32::from(value))).filter(|value| *value > 0.) else { continue };
        match sprm {
            0xB01F => page.width = Some(value),
            0xB020 => page.height = Some(value),
            0xB021 => page.left = Some(value),
            0xB022 => page.right = Some(value),
            0x9023 => page.top = Some(value),
            0x9024 => page.bottom = Some(value),
            _ => {}
        }
    }
    page
}

struct Field { instruction: String, link: Option<String>, result: bool }

/// Turns the main text into blocks, one paragraph (or table cell) at a time.
struct Builder<'a> {
    styles: Styles,
    fonts: &'a [String],
    families: Families,
    characters: &'a Runs,
    paragraphs: &'a Runs,
    fields: Vec<Field>,
    out: Out,
    cell: Vec<Span>,
    row: Vec<Vec<Span>>,
    rows: Vec<Vec<Vec<Span>>>,
    headings: usize,
    omitted: Omitted,
}

impl Builder<'_> {
    fn spans(&mut self, units: &[(u16, u32)], paragraph_style: u16) -> Vec<Span> {
        let mut spans: Vec<Span> = Vec::new();
        let mut pending: Vec<u16> = Vec::new();
        let mut current: Option<(Style, Option<String>)> = None;
        let flush = |spans: &mut Vec<Span>, pending: &mut Vec<u16>, current: &Option<(Style, Option<String>)>| {
            let Some((style, link)) = current else { return };
            if pending.is_empty() { return; }
            let text = String::from_utf16_lossy(pending);
            pending.clear();
            match spans.last_mut() {
                Some(last) if last.style == *style && last.link == *link => last.text.push_str(&text),
                _ => spans.push(Span { text, style: *style, link: link.clone() }),
            }
        };
        for &(unit, offset) in units {
            match unit {
                0x13 => { self.fields.push(Field { instruction: String::new(), link: None, result: false }); continue; }
                0x14 => {
                    if let Some(field) = self.fields.last_mut() {
                        field.result = true;
                        field.link = hyperlink(&field.instruction);
                    }
                    continue;
                }
                0x15 => { self.fields.pop(); continue; }
                _ => {}
            }
            if let Some(field) = self.fields.last_mut().filter(|field| !field.result) {
                field.instruction.push_str(&String::from_utf16_lossy(&[unit]));
                continue;
            }
            if self.fields.iter().any(|field| !field.result) { continue; }
            let chp = self.chp(offset, paragraph_style);
            if chp.hidden || chp.deleted { continue; }
            let text = match unit {
                0x01 | 0x08 if chp.special => {
                    if chp.object { self.omitted.objects += 1 } else { self.omitted.pictures += 1 }
                    continue;
                }
                0x0B => 0x0A,
                0x09 => 0x20,
                0x1E => 0x2D,
                0x00..=0x1F => continue,
                unit => unit,
            };
            let style = Style {
                bold: chp.bold,
                italic: chp.italic,
                strike: chp.strike,
                underline: chp.underline,
                family: self.families.index(self.fonts.get(chp.font as usize).map(String::as_str).filter(|name| !name.is_empty())),
                half_points: chp.half_points,
                color: chp.color,
                code: false,
            };
            let link = self.fields.iter().rev().find_map(|field| field.link.clone());
            let key = (style, link);
            if current.as_ref() != Some(&key) {
                flush(&mut spans, &mut pending, &current);
                current = Some(key);
            }
            pending.push(text);
        }
        flush(&mut spans, &mut pending, &current);
        spans
    }

    fn chp(&mut self, offset: u32, paragraph_style: u16) -> Chp {
        let base = self.styles.characters(paragraph_style);
        let Some(run) = self.characters.at(offset) else { return base };
        let mut chp = base;
        chp.apply(&run.grpprl, &base);
        if let Some(style) = chp.style.filter(|style| *style != 0) {
            let mut styled = self.styles.characters(style);
            styled.apply(&run.grpprl, &base);
            chp = styled;
        }
        chp
    }

    fn close_table(&mut self) {
        if !self.cell.is_empty() { let cell = std::mem::take(&mut self.cell); self.row.push(cell); }
        if !self.row.is_empty() { let row = std::mem::take(&mut self.row); self.rows.push(row); }
        if self.rows.is_empty() { return; }
        let mut rows = std::mem::take(&mut self.rows);
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        for row in &mut rows { row.resize_with(columns, Vec::new); }
        self.out.push(Block::Table { aligns: Vec::new(), head: Vec::new(), rows });
    }

    fn paragraph(&mut self, units: &[(u16, u32)], mark: (u16, u32)) {
        let (istd, grpprl) = self.paragraphs.at(mark.1).map_or((0, &[][..]), |run| (run.istd, run.grpprl.as_slice()));
        let mut pap = self.styles.paragraph(istd);
        pap.apply(grpprl);
        let spans = self.spans(units, istd);
        if mark.0 == 0x07 && pap.row_end {
            if !self.cell.is_empty() { let cell = std::mem::take(&mut self.cell); self.row.push(cell); }
            let row = std::mem::take(&mut self.row);
            self.rows.push(row);
            return;
        }
        if mark.0 == 0x07 || pap.in_table {
            if !self.cell.is_empty() { self.cell.push(Span { text: "\n".into(), style: Style::default(), link: None }); }
            self.cell.extend(spans);
            if mark.0 == 0x07 { let cell = std::mem::take(&mut self.cell); self.row.push(cell); }
            return;
        }
        self.close_table();
        let size = spans.iter().map(|span| span.style.half_points).filter(|half| *half > 0).min().unwrap_or(self.styles.characters(istd).half_points) as f32 / 2.;
        if let Some(level) = self.styles.heading(istd).filter(|_| !plain_text(&spans).is_empty()) {
            self.headings += 1;
            self.out.push(Block::Heading { level, spans, anchor: format!("heading-{}", self.headings) });
        } else {
            self.out.push(Block::Formatted(Paragraph {
                spans,
                align: pap.align,
                indent: twips(pap.indent.max(0)),
                right_indent: twips(pap.right_indent.max(0)),
                before: twips(pap.before.max(0)),
                after: twips(pap.after.max(0)),
                size,
                leading: 1.2,
            }));
        }
        if mark.0 == 0x0C { self.out.page_break(); }
    }
}

/// Parses a Word 97-2003 document. `budget` caps the bytes read from the compound file; going over returns `exceeded`.
pub fn parse(bytes: &[u8], budget: u64, exceeded: ToolError) -> Result<Parsed, ToolError> {
    let mut file = Compound::open(bytes, budget, exceeded, DAMAGED)?;
    let word = file.stream("/WordDocument")?.ok_or_else(damaged)?;
    let fib = Fib::read(&word)?;
    let table = file.stream(fib.table)?.ok_or_else(damaged)?;
    let (title, author) = file.summary()?;

    let pieces = fib.part(&table, PIECES).and_then(pieces).ok_or_else(damaged)?;
    let units = text(&word, &pieces, fib.text_length)?;
    let characters = Runs::read(&word, fib.part(&table, CHARACTER_RUNS), false);
    let paragraphs = Runs::read(&word, fib.part(&table, PARAGRAPH_RUNS), true);
    let mut styles = Styles::read(fib.part(&table, STYLES));
    let fonts = font_names(fib.part(&table, FONTS));
    let body = styles.characters(0);
    let families = Families { body: fonts.get(body.font as usize).filter(|name| !name.is_empty()).cloned(), others: Vec::new() };

    let mut builder = Builder {
        styles, fonts: &fonts, families, characters: &characters, paragraphs: &paragraphs, fields: Vec::new(),
        out: Out::default(), cell: Vec::new(), row: Vec::new(), rows: Vec::new(), headings: 0, omitted: Omitted::default(),
    };
    let mut start = 0;
    for (index, unit) in units.iter().enumerate() {
        if matches!(unit.0, 0x0D | 0x07 | 0x0C) {
            builder.paragraph(&units[start..index], *unit);
            start = index + 1;
        }
    }
    if start < units.len() { builder.paragraph(&units[start..], (0x0D, units[units.len() - 1].1)); }
    builder.close_table();

    let Builder { families, out, omitted, .. } = builder;
    Ok(Parsed {
        blocks: out.finish(),
        header: Vec::new(),
        footer: Vec::new(),
        header_distance: None,
        footer_distance: None,
        pictures: Vec::new(),
        families: families.others,
        body_family: families.body,
        body_size: body.half_points as f32 / 2.,
        embedded: Vec::new(),
        title,
        author,
        page: page_setup(&word, fib.part(&table, SECTIONS)),
        omitted,
    })
}

#[cfg(test)]
pub(crate) mod fixtures {
    use crate::kit::office::tests::cfb_with_streams;
    use crate::kit::pdf::compound::fixtures::summary;

    /// A run of text with its character properties; the paragraph and cell marks in it take `istd` and the paragraph properties.
    pub(crate) struct Run<'a> { pub(crate) text: &'a str, pub(crate) characters: &'a [u8], pub(crate) istd: u16, pub(crate) paragraph: &'a [u8] }

    impl<'a> Run<'a> {
        pub(crate) fn plain(text: &'a str) -> Self { Self { text, characters: &[], istd: 0, paragraph: &[] } }
    }

    pub(crate) const BOLD: &[u8] = &[0x35, 0x08, 0x01];
    pub(crate) const HIDDEN: &[u8] = &[0x3C, 0x08, 0x01];
    pub(crate) const IN_TABLE: &[u8] = &[0x16, 0x24, 0x01];
    pub(crate) const ROW_END: &[u8] = &[0x16, 0x24, 0x01, 0x17, 0x24, 0x01];
    pub(crate) const CENTERED: &[u8] = &[0x03, 0x24, 0x01];
    const TEXT_AT: usize = 1024;

    fn wide(text: &str) -> Vec<u8> { text.encode_utf16().flat_map(u16::to_le_bytes).collect() }

    fn style(sti: u16, name: &str, paragraph: &[u8], characters: &[u8]) -> Vec<u8> {
        let mut entry = Vec::new();
        entry.extend(sti.to_le_bytes());
        entry.extend((1u16 | (0x0FFF << 4)).to_le_bytes());
        entry.extend(2u16.to_le_bytes());
        entry.extend([0u8; 4]);
        entry.extend((name.encode_utf16().count() as u16).to_le_bytes());
        entry.extend(wide(name));
        entry.extend([0, 0]);
        let mut papx = 0u16.to_le_bytes().to_vec();
        papx.extend(paragraph);
        for upx in [papx, characters.to_vec()] {
            entry.extend((upx.len() as u16).to_le_bytes());
            entry.extend(&upx);
            if upx.len() % 2 == 1 { entry.push(0); }
        }
        let mut lp = (entry.len() as u16).to_le_bytes().to_vec();
        lp.extend(entry);
        lp
    }

    /// A Word 97-2003 document with the runs as its main text. The text is stored as UTF-16, or one byte per character when `compressed`.
    pub(crate) fn document(runs: &[Run], compressed: bool) -> Vec<u8> {
        assert!(runs.len() < 20, "one formatted disk page each");
        let text = runs.iter().map(|run| run.text).collect::<String>();
        let units = text.encode_utf16().count();
        let stored = if compressed { text.chars().map(|character| u8::try_from(u32::from(character)).expect("compressed fixtures are Latin-1")).collect::<Vec<_>>() } else { wide(&text) };
        let width = if compressed { 1 } else { 2 };
        let character_page = (TEXT_AT + stored.len()).div_ceil(512) * 512;
        let paragraph_page = character_page + 512;
        let mut word = vec![0u8; paragraph_page + 512];
        word[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
        word[2..4].copy_from_slice(&0x00C1u16.to_le_bytes());
        word[32..34].copy_from_slice(&14u16.to_le_bytes());
        word[62..64].copy_from_slice(&22u16.to_le_bytes());
        word[76..80].copy_from_slice(&(units as u32).to_le_bytes());
        word[152..154].copy_from_slice(&93u16.to_le_bytes());
        word[TEXT_AT..TEXT_AT + stored.len()].copy_from_slice(&stored);

        let mut bounds = vec![TEXT_AT as u32];
        for run in runs { bounds.push(bounds.last().unwrap() + (run.text.encode_utf16().count() * width) as u32); }
        let mut characters = vec![0u8; 512];
        let mut paragraphs = vec![0u8; 512];
        for (index, bound) in bounds.iter().enumerate() {
            characters[index * 4..index * 4 + 4].copy_from_slice(&bound.to_le_bytes());
            paragraphs[index * 4..index * 4 + 4].copy_from_slice(&bound.to_le_bytes());
        }
        let (mut character_end, mut paragraph_end) = (510usize, 510usize);
        for (index, run) in runs.iter().enumerate() {
            if !run.characters.is_empty() {
                character_end = (character_end - run.characters.len() - 1) & !1;
                characters[character_end] = run.characters.len() as u8;
                characters[character_end + 1..character_end + 1 + run.characters.len()].copy_from_slice(run.characters);
                characters[(runs.len() + 1) * 4 + index] = (character_end / 2) as u8;
            }
            let mut grpprl = run.istd.to_le_bytes().to_vec();
            grpprl.extend(run.paragraph);
            let header = if grpprl.len() % 2 == 1 { vec![grpprl.len().div_ceil(2) as u8] } else { vec![0, (grpprl.len() / 2) as u8] };
            paragraph_end = (paragraph_end - grpprl.len() - header.len()) & !1;
            paragraphs[paragraph_end..paragraph_end + header.len()].copy_from_slice(&header);
            paragraphs[paragraph_end + header.len()..paragraph_end + header.len() + grpprl.len()].copy_from_slice(&grpprl);
            paragraphs[(runs.len() + 1) * 4 + index * 13] = (paragraph_end / 2) as u8;
        }
        characters[511] = runs.len() as u8;
        paragraphs[511] = runs.len() as u8;
        word[character_page..character_page + 512].copy_from_slice(&characters);
        word[paragraph_page..paragraph_page + 512].copy_from_slice(&paragraphs);

        let mut table = Vec::new();
        let mut place = |table: &mut Vec<u8>, index: usize, bytes: &[u8]| {
            let at = 154 + index * 8;
            word[at..at + 4].copy_from_slice(&(table.len() as u32).to_le_bytes());
            word[at + 4..at + 8].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            table.extend_from_slice(bytes);
        };
        let mut stylesheet = [14u16, 2, 10, 1, 0, 0, 0, 0].iter().flat_map(|value| value.to_le_bytes()).collect::<Vec<_>>();
        stylesheet.extend(style(0, "Normal", &[], &[]));
        stylesheet.extend(style(1, "heading 1", &[], BOLD));
        place(&mut table, 1, &stylesheet);
        let bins = |text_end: u32, page: usize| [TEXT_AT as u32, text_end, (page / 512) as u32].iter().flat_map(|value| value.to_le_bytes()).collect::<Vec<_>>();
        place(&mut table, 12, &bins(*bounds.last().unwrap(), character_page));
        place(&mut table, 13, &bins(*bounds.last().unwrap(), paragraph_page));
        let name = wide("Times New Roman");
        let mut fonts = [1u16, 0].iter().flat_map(|value| value.to_le_bytes()).collect::<Vec<_>>();
        fonts.push((39 + name.len() + 2) as u8);
        fonts.extend([0u8; 39]);
        fonts.extend(&name);
        fonts.extend([0, 0]);
        place(&mut table, 15, &fonts);
        let stored_at = if compressed { ((TEXT_AT * 2) as u32) | 0x4000_0000 } else { TEXT_AT as u32 };
        let mut clx = vec![2u8];
        clx.extend(16u32.to_le_bytes());
        for value in [0u32, units as u32] { clx.extend(value.to_le_bytes()); }
        clx.extend(0u16.to_le_bytes());
        clx.extend(stored_at.to_le_bytes());
        clx.extend(0u16.to_le_bytes());
        place(&mut table, 33, &clx);
        cfb_with_streams(&[("/WordDocument", &word), ("/0Table", &table), ("/\u{5}SummaryInformation", &summary("Toolbox notes", "Ana"))])
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{document, Run, BOLD, CENTERED, HIDDEN, IN_TABLE, ROW_END};
    use super::*;
    use crate::kit::office::tests::{cfb_with_streams, locked_doc, LegacyEncryption};

    fn exceeded() -> ToolError { ToolError::invalid_input("too big") }

    fn texts(blocks: &[Block]) -> Vec<String> {
        blocks.iter().map(|block| match block {
            Block::Heading { spans, .. } => format!("# {}", plain_text(spans)),
            Block::Formatted(paragraph) => plain_text(&paragraph.spans),
            Block::Table { rows, .. } => rows.iter().map(|row| row.iter().map(|cell| plain_text(cell)).collect::<Vec<_>>().join("|")).collect::<Vec<_>>().join("/"),
            Block::PageBreak => "---".into(),
            other => format!("{other:?}"),
        }).collect()
    }

    #[test]
    fn text_headings_styles_tables_and_links_come_through() {
        let bytes = document(&[
            Run { text: "Plan\r", characters: &[], istd: 1, paragraph: &[] },
            Run::plain("Caf\u{e9} \u{41f}\u{440}\u{438}\u{432}\u{435}\u{442} "),
            Run { text: "bold", characters: BOLD, istd: 0, paragraph: &[] },
            Run { text: " secret", characters: HIDDEN, istd: 0, paragraph: &[] },
            Run { text: " see \u{13} HYPERLINK \"https://example.com\" \u{14}the site\u{15}.\r", characters: &[], istd: 0, paragraph: CENTERED },
            Run { text: "A1\u{7}B1\u{7}", characters: &[], istd: 0, paragraph: IN_TABLE },
            Run { text: "\u{7}", characters: &[], istd: 0, paragraph: ROW_END },
            Run::plain("After\u{c}Next page\r"),
        ], false);
        let parsed = parse(&bytes, 1 << 20, exceeded()).unwrap();
        assert_eq!(texts(&parsed.blocks), ["# Plan", "Caf\u{e9} \u{41f}\u{440}\u{438}\u{432}\u{435}\u{442} bold see the site.", "A1|B1", "After", "---", "Next page"]);
        let Block::Formatted(paragraph) = &parsed.blocks[1] else { panic!() };
        assert_eq!(paragraph.align, Align::Center);
        assert!(paragraph.spans.iter().any(|span| span.text == "bold" && span.style.bold));
        assert!(paragraph.spans.iter().any(|span| span.text == "the site" && span.link.as_deref() == Some("https://example.com")));
        assert_eq!(parsed.body_family.as_deref(), Some("Times New Roman"));
        assert_eq!(parsed.body_size, 10.);
        assert_eq!((parsed.title.as_deref(), parsed.author.as_deref()), (Some("Toolbox notes"), Some("Ana")));
    }

    #[test]
    fn one_byte_text_reads_as_windows_1252() {
        let parsed = parse(&document(&[Run::plain("Caf\u{e9} na\u{ef}ve\r")], true), 1 << 20, exceeded()).unwrap();
        assert_eq!(texts(&parsed.blocks), ["Caf\u{e9} na\u{ef}ve"]);
    }

    #[test]
    fn pictures_and_objects_are_counted_as_left_out() {
        const SPECIAL: &[u8] = &[0x55, 0x08, 0x01];
        let parsed = parse(&document(&[Run::plain("Chart: "), Run { text: "\u{1}\u{8}", characters: SPECIAL, istd: 0, paragraph: &[] }, Run::plain("\r")], false), 1 << 20, exceeded()).unwrap();
        assert_eq!(parsed.omitted.pictures, 2);
        assert_eq!(texts(&parsed.blocks), ["Chart:"]);
    }

    #[test]
    fn word_95_documents_and_damaged_files_are_named() {
        let mut old = vec![0u8; 64];
        old[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
        old[2..4].copy_from_slice(&0x0065u16.to_le_bytes());
        assert_eq!(parse(&cfb_with_streams(&[("/WordDocument", &old)]), 1 << 20, exceeded()).err().unwrap().message, TOO_OLD);
        assert_eq!(parse(&cfb_with_streams(&[("/WordDocument", &[0u8; 64])]), 1 << 20, exceeded()).err().unwrap().message, DAMAGED);
        assert_eq!(parse(&cfb_with_streams(&[("/Other", &[0u8; 4])]), 1 << 20, exceeded()).err().unwrap().message, DAMAGED);
        assert_eq!(parse(&document(&[Run::plain("x\r")], false), 100, exceeded()).err().unwrap().message, "too big");
    }

    #[test]
    fn encrypted_documents_are_detected_before_parsing() {
        for encryption in [LegacyEncryption::Xor, LegacyEncryption::Rc4, LegacyEncryption::CryptoApi] {
            assert!(crate::kit::office::is_encrypted(&locked_doc(encryption), "doc").unwrap());
        }
        assert!(!crate::kit::office::is_encrypted(&document(&[Run::plain("x\r")], false), "doc").unwrap());
    }
}
