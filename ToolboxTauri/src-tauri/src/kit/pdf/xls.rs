//! Reads an Excel 97-2003 (XLS, BIFF8) workbook into the shared sheet model: stored cell values, number formats, fonts and print settings.
//! Formulas are never recalculated and macro sheets are never read.

use std::collections::HashMap;

use crate::kit::contracts::ToolError;
use crate::kit::pdf::compound::{f64_at, u16_at, u32_at, Compound};
use crate::kit::pdf::layout::{Span, Style};
use crate::kit::pdf::spreadsheet::{Area, Cell, Sheet, Workbook};
use crate::kit::pdf::wordprocessing::Families;
use crate::kit::pdf::xlsx::{builtin_format, format_number, paper};

pub const DAMAGED: &str = "This Excel 97-2003 workbook is damaged or incomplete, so no PDF was made.";
pub const TOO_OLD: &str = "This workbook was saved by Excel 95 or earlier, which Convert to PDF doesn't read. Save it as a newer workbook, then convert that.";

const BOF: u16 = 0x0809;
const EOF: u16 = 0x000A;
const CONTINUE: u16 = 0x003C;
const BIFF8: u16 = 0x0600;
const MAX_STRINGS: usize = 1 << 20;

fn damaged() -> ToolError { ToolError::invalid_input(DAMAGED) }

struct Record<'a> { id: u16, data: &'a [u8] }

/// The BIFF records from `offset` onwards; a record that runs past the end of the stream makes the workbook damaged.
fn records(stream: &[u8], mut offset: usize) -> impl Iterator<Item = Result<Record<'_>, ToolError>> {
    std::iter::from_fn(move || {
        if offset >= stream.len() { return None; }
        let (Some(id), Some(size)) = (u16_at(stream, offset), u16_at(stream, offset + 2)) else { offset = stream.len(); return Some(Err(damaged())) };
        let start = offset + 4;
        let Some(data) = stream.get(start..start + size as usize) else { offset = stream.len(); return Some(Err(damaged())) };
        offset = start + size as usize;
        Some(Ok(Record { id, data }))
    })
}

/// Characters stored one byte each (the low byte of UTF-16) or as UTF-16LE.
fn characters(bytes: &[u8], wide: bool) -> String {
    if wide { String::from_utf16_lossy(&bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect::<Vec<_>>()) } else { bytes.iter().map(|byte| char::from(*byte)).collect() }
}

/// A string with a 16-bit character count, an options byte and its characters (XLUnicodeString).
fn long_string(data: &[u8], offset: usize) -> Option<String> {
    let count = u16_at(data, offset)? as usize;
    let wide = data.get(offset + 2)? & 1 != 0;
    let length = if wide { count * 2 } else { count };
    Some(characters(data.get(offset + 3..offset + 3 + length)?, wide))
}

/// A string with an 8-bit character count, an options byte and its characters (ShortXLUnicodeString).
fn short_string(data: &[u8], offset: usize) -> Option<String> {
    let count = *data.get(offset)? as usize;
    let wide = data.get(offset + 1)? & 1 != 0;
    let length = if wide { count * 2 } else { count };
    Some(characters(data.get(offset + 2..offset + 2 + length)?, wide))
}

/// The shared string table's bytes across the SST record and the CONTINUE records after it. A string's characters may break across records; each continuation then starts with a new options byte.
struct Segments<'a> { parts: Vec<&'a [u8]>, part: usize, position: usize }

impl<'a> Segments<'a> {
    fn advance(&mut self) -> bool {
        while self.parts.get(self.part).is_some_and(|part| self.position >= part.len()) {
            self.part += 1;
            self.position = 0;
        }
        self.part < self.parts.len()
    }

    fn take(&mut self, mut count: usize) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(count.min(1 << 16));
        while count > 0 {
            if !self.advance() { return None; }
            let part = self.parts[self.part];
            let available = (part.len() - self.position).min(count);
            out.extend_from_slice(&part[self.position..self.position + available]);
            self.position += available;
            count -= available;
        }
        Some(out)
    }

    fn skip(&mut self, count: usize) -> Option<()> {
        let mut count = count;
        while count > 0 {
            if !self.advance() { return None; }
            let available = (self.parts[self.part].len() - self.position).min(count);
            self.position += available;
            count -= available;
        }
        Some(())
    }

    fn u16(&mut self) -> Option<u16> { self.take(2).map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]])) }

    fn u32(&mut self) -> Option<u32> { self.take(4).map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])) }

    fn characters(&mut self, mut count: usize, mut wide: bool) -> Option<String> {
        let mut text = String::new();
        while count > 0 {
            let part = self.parts.get(self.part)?;
            if self.position >= part.len() {
                self.part += 1;
                self.position = 0;
                wide = *self.parts.get(self.part)?.first()? & 1 != 0;
                self.position = 1;
                continue;
            }
            let width = if wide { 2 } else { 1 };
            let here = ((part.len() - self.position) / width).min(count);
            if here == 0 { return None; }
            text.push_str(&characters(&part[self.position..self.position + here * width], wide));
            self.position += here * width;
            count -= here;
        }
        Some(text)
    }

    /// The shared strings, as many as the table holds before it ends.
    fn strings(mut self) -> Vec<String> {
        let mut strings = Vec::new();
        let Some(unique) = self.skip(4).and_then(|_| self.u32()) else { return strings };
        for _ in 0..(unique as usize).min(MAX_STRINGS) {
            let Some(string) = self.string() else { break };
            strings.push(string);
        }
        strings
    }

    fn string(&mut self) -> Option<String> {
        let count = self.u16()? as usize;
        let options = self.take(1)?[0];
        let runs = if options & 0x08 != 0 { self.u16()? as usize } else { 0 };
        let extended = if options & 0x04 != 0 { self.u32()? as usize } else { 0 };
        let text = self.characters(count, options & 1 != 0)?;
        self.skip(runs * 4)?;
        self.skip(extended)?;
        Some(text)
    }
}

/// The number shape Excel stores to save space (RkNumber): a 30-bit integer or the high bits of a double, optionally divided by 100.
fn rk(value: u32) -> f64 {
    let number = if value & 2 != 0 { ((value as i32) >> 2) as f64 } else { f64::from_bits(((value & 0xFFFF_FFFC) as u64) << 32) };
    if value & 1 != 0 { number / 100. } else { number }
}

fn error_text(code: u8) -> &'static str {
    match code {
        0x00 => "#NULL!",
        0x07 => "#DIV/0!",
        0x0F => "#VALUE!",
        0x17 => "#REF!",
        0x1D => "#NAME?",
        0x24 => "#NUM!",
        _ => "#N/A",
    }
}

/// One cell format: the font style and number format code it applies.
struct Format { style: Style, code: String }

struct Globals {
    formats: Vec<Format>,
    strings: Vec<String>,
    date1904: bool,
}

impl Globals {
    fn format(&self, index: u16) -> (Style, &str) {
        self.formats.get(index as usize).map_or((Style::default(), "General"), |format| (format.style, format.code.as_str()))
    }

    fn text(&self, text: String, index: u16) -> Cell {
        Cell { spans: vec![Span { text, style: self.format(index).0, link: None }], numeric: false }
    }

    fn number(&self, value: f64, index: u16) -> Option<Cell> {
        if !value.is_finite() { return None; }
        let (style, code) = self.format(index);
        Some(Cell { spans: vec![Span { text: format_number(value, code, self.date1904), style, link: None }], numeric: true })
    }
}

struct Entry { name: String, position: usize, hidden: bool, kind: u8 }

/// Parses an Excel 97-2003 workbook. `budget` caps the bytes read from the compound file; going over returns `exceeded`.
pub fn parse(bytes: &[u8], budget: u64, exceeded: ToolError) -> Result<Workbook, ToolError> {
    let mut file = Compound::open(bytes, budget, exceeded, DAMAGED)?;
    let Some(stream) = file.stream("/Workbook")? else {
        return Err(if file.stream("/Book")?.is_some() { ToolError::invalid_input(TOO_OLD) } else { damaged() });
    };
    let (title, author) = file.summary()?;

    let mut all = records(&stream, 0);
    let first = all.next().ok_or_else(damaged)??;
    if first.id != BOF { return Err(damaged()); }
    if u16_at(first.data, 0) != Some(BIFF8) { return Err(ToolError::invalid_input(TOO_OLD)); }

    let mut fonts = Vec::new();
    let mut font_names = Vec::new();
    let mut cell_formats = Vec::new();
    let mut codes: HashMap<u16, String> = HashMap::new();
    let mut entries = Vec::new();
    let mut names: Vec<(u16, u8, Vec<Area>)> = Vec::new();
    let mut sst: Option<Vec<&[u8]>> = None;
    let mut strings = Vec::new();
    let mut date1904 = false;
    for record in all {
        let record = record?;
        if record.id != CONTINUE {
            if let Some(parts) = sst.take() { strings = Segments { parts, part: 0, position: 0 }.strings(); }
        }
        match record.id {
            EOF => break,
            0x00FC => sst = Some(vec![record.data]),
            CONTINUE => if let Some(parts) = sst.as_mut() { parts.push(record.data) },
            0x0022 => date1904 = u16_at(record.data, 0) == Some(1),
            0x0031 => {
                let data = record.data;
                let options = u16_at(data, 2).unwrap_or(0);
                fonts.push(Style {
                    bold: u16_at(data, 6).is_some_and(|weight| weight >= 600),
                    italic: options & 0x02 != 0,
                    strike: options & 0x08 != 0,
                    underline: data.get(10).is_some_and(|underline| *underline != 0),
                    ..Style::default()
                });
                font_names.push(short_string(data, 14).filter(|name| !name.is_empty()));
            }
            0x041E => if let (Some(id), Some(code)) = (u16_at(record.data, 0), long_string(record.data, 2)) { codes.insert(id, code); },
            0x00E0 => cell_formats.push((u16_at(record.data, 0).unwrap_or(0), u16_at(record.data, 2).unwrap_or(0))),
            0x0085 => {
                let data = record.data;
                let (Some(position), Some(name)) = (u32_at(data, 0), short_string(data, 6)) else { return Err(damaged()) };
                entries.push(Entry { name, position: position as usize, hidden: data.get(4).is_some_and(|state| state & 3 != 0), kind: data.get(5).copied().unwrap_or(0) });
            }
            0x0018 => if let Some(name) = builtin_name(record.data) { names.push(name) },
            _ => {}
        }
    }
    if let Some(parts) = sst.take() { strings = Segments { parts, part: 0, position: 0 }.strings(); }

    let mut families = Families { body: font_names.first().cloned().flatten(), others: Vec::new() };
    let fonts = fonts.into_iter().zip(&font_names).map(|(style, name)| Style { family: families.index(name.as_deref()), ..style }).collect::<Vec<_>>();
    let formats = cell_formats.into_iter().map(|(font, number)| Format {
        // Font 4 is never stored, so later fonts are numbered one past their place in the list.
        style: fonts.get(if font >= 4 { font as usize - 1 } else { font as usize }).copied().unwrap_or_default(),
        code: codes.get(&number).cloned().unwrap_or_else(|| builtin_format(number.into()).to_string()),
    }).collect();
    let globals = Globals { formats, strings, date1904 };

    let mut sheets = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let mut sheet = Sheet { name: entry.name, hidden: entry.hidden, ..Sheet::default() };
        match entry.kind {
            0 => {
                read_sheet(&stream, entry.position, &globals, &mut sheet)?;
                for (sheet_index, kind, areas) in &names {
                    if usize::from(*sheet_index) != index + 1 { continue; }
                    match kind {
                        0x06 => sheet.areas = areas.clone(),
                        0x07 => sheet.titles = areas.iter().find(|area| area.left.is_none()).and_then(|area| area.top),
                        _ => {}
                    }
                }
            }
            2 => sheet.omitted.charts = 1,
            _ => continue,
        }
        sheets.push(sheet);
    }
    Ok(Workbook { sheets, families: families.others, body_family: families.body, title, author })
}

/// A built-in Print_Area (6) or Print_Titles (7) name: the sheet it belongs to and the ranges it covers.
fn builtin_name(data: &[u8]) -> Option<(u16, u8, Vec<Area>)> {
    let options = u16_at(data, 0)?;
    if options & 0x20 == 0 { return None; }
    let formula_length = u16_at(data, 4)? as usize;
    let sheet = u16_at(data, 8)?;
    let wide = *data.get(14)? & 1 != 0;
    let kind = *data.get(15)?;
    let formula = data.get(15 + if wide { 2 } else { 1 }..)?.get(..formula_length)?;
    let mut areas = Vec::new();
    let mut offset = 0;
    while let Some(token) = formula.get(offset) {
        match token {
            0x3B | 0x5B | 0x7B => {
                let (top, bottom, left, right) = (u16_at(formula, offset + 3)?, u16_at(formula, offset + 5)?, u16_at(formula, offset + 7)? & 0x3FFF, u16_at(formula, offset + 9)? & 0x3FFF);
                let whole_columns = top == 0 && bottom == 0xFFFF;
                let whole_rows = left == 0 && right >= 0xFF;
                areas.push(Area {
                    top: (!whole_columns).then_some(u32::from(top)),
                    bottom: (!whole_columns).then_some(u32::from(bottom)),
                    left: (!whole_rows).then_some(u32::from(left)),
                    right: (!whole_rows).then_some(u32::from(right)),
                });
                offset += 11;
            }
            0x29 | 0x49 | 0x69 => offset += 3,
            0x10 => offset += 1,
            _ => break,
        }
    }
    (!areas.is_empty()).then_some((sheet, kind, areas))
}

fn read_sheet(stream: &[u8], position: usize, globals: &Globals, sheet: &mut Sheet) -> Result<(), ToolError> {
    let mut all = records(stream, position);
    let first = all.next().ok_or_else(damaged)??;
    if first.id != BOF { return Err(damaged()); }
    let mut nested = 0usize;
    let mut pending: Option<(u16, u16, u16)> = None;
    let (mut fit, mut fit_width) = (false, true);
    for record in all {
        let record = record?;
        if nested > 0 {
            match record.id { BOF => nested += 1, EOF => nested -= 1, _ => {} }
            continue;
        }
        let data = record.data;
        let place = || Some((u16_at(data, 0)?, u16_at(data, 2)?, u16_at(data, 4)?));
        match record.id {
            EOF => break,
            BOF => nested = 1,
            0x00FD => if let (Some((row, column, format)), Some(index)) = (place(), u32_at(data, 6)) {
                if let Some(text) = globals.strings.get(index as usize) { sheet.put(row.into(), column.into(), globals.text(text.clone(), format))?; }
            },
            0x0204 | 0x00D6 => if let (Some((row, column, format)), Some(text)) = (place(), long_string(data, 6)) {
                sheet.put(row.into(), column.into(), globals.text(text, format))?;
            },
            0x0203 => if let (Some((row, column, format)), Some(value)) = (place(), f64_at(data, 6)) {
                if let Some(cell) = globals.number(value, format) { sheet.put(row.into(), column.into(), cell)?; }
            },
            0x027E => if let (Some((row, column, format)), Some(value)) = (place(), u32_at(data, 6)) {
                if let Some(cell) = globals.number(rk(value), format) { sheet.put(row.into(), column.into(), cell)?; }
            },
            0x00BD => if let (Some(row), Some(first)) = (u16_at(data, 0), u16_at(data, 2)) {
                for (index, item) in data.get(4..data.len().saturating_sub(2)).unwrap_or_default().chunks_exact(6).enumerate() {
                    let (Some(format), Some(value)) = (u16_at(item, 0), u32_at(item, 2)) else { continue };
                    if let Some(cell) = globals.number(rk(value), format) { sheet.put(row.into(), u32::from(first) + index as u32, cell)?; }
                }
            },
            0x0006 => if let Some((row, column, format)) = place() {
                pending = None;
                let Some(result) = data.get(6..14) else { continue };
                if result[6] == 0xFF && result[7] == 0xFF {
                    match result[0] {
                        0 => pending = Some((row, column, format)),
                        1 => sheet.put(row.into(), column.into(), globals.text(if result[2] != 0 { "TRUE" } else { "FALSE" }.into(), format))?,
                        2 => sheet.put(row.into(), column.into(), globals.text(error_text(result[2]).into(), format))?,
                        _ => {}
                    }
                } else if let Some(cell) = f64_at(result, 0).and_then(|value| globals.number(value, format)) {
                    sheet.put(row.into(), column.into(), cell)?;
                }
            },
            0x0207 => if let (Some((row, column, format)), Some(text)) = (pending.take(), long_string(data, 0)) {
                sheet.put(row.into(), column.into(), globals.text(text, format))?;
            },
            0x0205 => if let (Some((row, column, format)), Some(value), Some(error)) = (place(), data.get(6), data.get(7)) {
                let text = if *error != 0 { error_text(*value) } else if *value != 0 { "TRUE" } else { "FALSE" };
                sheet.put(row.into(), column.into(), globals.text(text.into(), format))?;
            },
            0x0208 => if let (Some(row), Some(options)) = (u16_at(data, 0), u16_at(data, 12)) {
                let row = u32::from(row);
                if options & 0x20 != 0 {
                    match sheet.hidden_rows.last_mut() {
                        Some((_, end)) if end.saturating_add(1) == row => *end = row,
                        _ => sheet.hidden_rows.push((row, row)),
                    }
                }
            },
            0x007D => if let (Some(first), Some(last), Some(options)) = (u16_at(data, 0), u16_at(data, 2), u16_at(data, 8)) {
                if options & 1 != 0 { sheet.hidden_columns.push((first.into(), last.into())); }
            },
            0x00A1 => {
                let options = u16_at(data, 10).unwrap_or(0);
                if options & 0x04 == 0 {
                    sheet.landscape = options & 0x02 == 0;
                    sheet.paper = u16_at(data, 0).and_then(paper);
                }
                fit_width = u16_at(data, 6) != Some(0);
            }
            0x0081 => fit = u16_at(data, 0).is_some_and(|options| options & 0x100 != 0),
            0x005D => match (u16_at(data, 0), u16_at(data, 4)) {
                (Some(0x15), Some(5)) => sheet.omitted.charts += 1,
                (Some(0x15), Some(8)) => sheet.omitted.pictures += 1,
                _ => {}
            },
            _ => {}
        }
    }
    sheet.fit = fit && fit_width;
    Ok(())
}

#[cfg(test)]
pub(crate) mod fixtures {
    use crate::kit::office::tests::{biff_record, cfb_with_streams};
    use crate::kit::pdf::compound::fixtures::summary;

    pub(crate) enum Value<'a> { Text(&'a str), Number(f64) }

    fn wide(text: &str) -> Vec<u8> { text.encode_utf16().flat_map(u16::to_le_bytes).collect() }

    fn bof(kind: u16) -> Vec<u8> { biff_record(0x0809, &[&0x0600u16.to_le_bytes()[..], &kind.to_le_bytes(), &[0; 12]].concat()) }

    /// An Excel 97-2003 workbook whose sheets each have a name, a hidden flag and cells; every string is stored as UTF-16 in the shared string table.
    pub(crate) fn workbook(sheets: &[(&str, bool, Vec<(u16, u16, Value)>)]) -> Vec<u8> {
        let mut strings = Vec::new();
        for (_, _, cells) in sheets {
            for (_, _, value) in cells { if let Value::Text(text) = value { strings.push(*text); } }
        }
        let mut font = vec![0u8; 14];
        font[0..2].copy_from_slice(&200u16.to_le_bytes());
        font[6..8].copy_from_slice(&400u16.to_le_bytes());
        font.extend([5, 0]);
        font.extend(b"Arial");
        let mut sst = Vec::new();
        sst.extend((strings.len() as u32).to_le_bytes());
        sst.extend((strings.len() as u32).to_le_bytes());
        for text in &strings {
            sst.extend((text.encode_utf16().count() as u16).to_le_bytes());
            sst.push(1);
            sst.extend(wide(text));
        }
        let entry = |name: &str, position: u32, hidden: bool| {
            let mut data = position.to_le_bytes().to_vec();
            data.extend([u8::from(hidden), 0, name.encode_utf16().count() as u8, 1]);
            data.extend(wide(name));
            biff_record(0x0085, &data)
        };
        let head = [bof(0x0005), biff_record(0x0031, &font), biff_record(0x00E0, &[0u8; 20])].concat();
        let tail = [biff_record(0x00FC, &sst), biff_record(0x000A, &[])].concat();
        let entries_length: usize = sheets.iter().map(|(name, hidden, _)| entry(name, 0, *hidden).len()).sum();
        let mut position = head.len() + entries_length + tail.len();
        let mut bodies = Vec::new();
        let mut entries = Vec::new();
        let mut next_string = 0u32;
        for (name, hidden, cells) in sheets {
            entries.extend(entry(name, position as u32, *hidden));
            let mut body = bof(0x0010);
            for (row, column, value) in cells {
                let mut data = [row.to_le_bytes(), column.to_le_bytes(), 0u16.to_le_bytes()].concat();
                match value {
                    Value::Text(_) => {
                        data.extend(next_string.to_le_bytes());
                        next_string += 1;
                        body.extend(biff_record(0x00FD, &data));
                    }
                    Value::Number(number) => {
                        data.extend(number.to_le_bytes());
                        body.extend(biff_record(0x0203, &data));
                    }
                }
            }
            body.extend(biff_record(0x000A, &[]));
            position += body.len();
            bodies.push(body);
        }
        let stream = [head, entries, tail, bodies.concat()].concat();
        cfb_with_streams(&[("/Workbook", &stream), ("/\u{5}SummaryInformation", &summary("Toolbox notes", "Ana"))])
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{workbook, Value};
    use super::*;
    use crate::kit::office::tests::{biff_record, cfb_with_streams, locked_xls, LegacyEncryption};
    use crate::kit::pdf::layout::{plain_text, Block};

    fn exceeded() -> ToolError { ToolError::invalid_input("too big") }

    fn texts(sheet: &Sheet) -> Vec<String> {
        sheet.tables(false).unwrap().iter().flat_map(|table| match table {
            Block::Table { head, rows, .. } => head.iter().chain(rows.iter().flatten()).map(|cell| plain_text(cell)).collect::<Vec<_>>(),
            _ => Vec::new(),
        }).collect()
    }

    #[test]
    fn sheets_keep_their_names_hidden_flags_text_and_numbers() {
        let bytes = workbook(&[
            ("Notes", false, vec![(0, 0, Value::Text("Caf\u{e9} \u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}")), (0, 1, Value::Number(42.5)), (2, 0, Value::Number(1.)) ]),
            ("Secret", true, vec![(0, 0, Value::Text("hidden value"))]),
        ]);
        let book = parse(&bytes, 1 << 20, exceeded()).unwrap();
        assert_eq!(book.sheets.len(), 2);
        assert_eq!((book.sheets[0].name.as_str(), book.sheets[0].hidden), ("Notes", false));
        assert_eq!((book.sheets[1].name.as_str(), book.sheets[1].hidden), ("Secret", true));
        let cells = texts(&book.sheets[0]);
        assert!(cells.contains(&"Caf\u{e9} \u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}".to_string()), "{cells:?}");
        assert!(cells.contains(&"42.5".to_string()) && cells.contains(&"1".to_string()), "{cells:?}");
        assert_eq!(book.body_family.as_deref(), Some("Arial"));
        assert_eq!((book.title.as_deref(), book.author.as_deref()), (Some("Toolbox notes"), Some("Ana")));
    }

    #[test]
    fn shared_strings_continue_across_records_and_switch_width() {
        let first = [&2u32.to_le_bytes()[..], &2u32.to_le_bytes(), &5u16.to_le_bytes(), &[0], b"Hel"].concat();
        let second = [&[1u8][..], &wide("lo"), &3u16.to_le_bytes(), &[0], b"abc"].concat();
        let strings = Segments { parts: vec![&first, &second], part: 0, position: 0 }.strings();
        assert_eq!(strings, ["Hello", "abc"]);
        fn wide(text: &str) -> Vec<u8> { text.encode_utf16().flat_map(u16::to_le_bytes).collect() }
    }

    #[test]
    fn stored_numbers_decode_and_formats_apply() {
        assert_eq!(rk(0x3FF0_0000), 1.);
        assert_eq!(rk((1234 << 2) | 2 | 1), 12.34);
        let globals = Globals { formats: vec![Format { style: Style::default(), code: "0%".into() }, Format { style: Style::default(), code: "m/d/yyyy".into() }], strings: Vec::new(), date1904: false };
        assert_eq!(plain_text(&globals.number(0.25, 0).unwrap().spans), "25%");
        assert_eq!(plain_text(&globals.number(45292., 1).unwrap().spans), "1/1/2024");
        assert!(globals.number(f64::NAN, 0).is_none());
    }

    #[test]
    fn print_areas_and_titles_come_from_built_in_names() {
        let mut name = vec![0u8; 14];
        name[0..2].copy_from_slice(&0x20u16.to_le_bytes());
        name[3] = 1;
        name[4..6].copy_from_slice(&11u16.to_le_bytes());
        name[8..10].copy_from_slice(&1u16.to_le_bytes());
        name.extend([0, 0x06, 0x3B, 0, 0]);
        for value in [1u16, 4, 0, 2] { name.extend(value.to_le_bytes()); }
        assert_eq!(builtin_name(&name), Some((1, 6, vec![Area { top: Some(1), bottom: Some(4), left: Some(0), right: Some(2) }])));
    }

    #[test]
    fn excel_95_workbooks_and_damaged_files_are_named() {
        let old = cfb_with_streams(&[("/Book", &biff_record(0x0809, &[0, 5, 0x10, 0]))]);
        assert_eq!(parse(&old, 1 << 20, exceeded()).err().unwrap().message, TOO_OLD);
        let truncated = cfb_with_streams(&[("/Workbook", &[0x09, 0x08, 0x10])]);
        assert_eq!(parse(&truncated, 1 << 20, exceeded()).err().unwrap().message, DAMAGED);
        assert_eq!(parse(&workbook(&[("A", false, vec![])]), 10, exceeded()).err().unwrap().message, "too big");
    }

    #[test]
    fn encrypted_workbooks_are_detected_before_parsing() {
        for encryption in [LegacyEncryption::Xor, LegacyEncryption::Rc4, LegacyEncryption::CryptoApi] {
            assert!(crate::kit::office::is_encrypted(&locked_xls(encryption), "xls").unwrap());
        }
        assert!(!crate::kit::office::is_encrypted(&workbook(&[("A", false, vec![])]), "xls").unwrap());
    }
}
