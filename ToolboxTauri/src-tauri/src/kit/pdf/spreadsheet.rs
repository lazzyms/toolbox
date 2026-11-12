//! The sheet model shared by the Excel (XLSX) and OpenDocument spreadsheet (ODS) readers, laid out as tables.

use std::collections::BTreeMap;

use crate::kit::contracts::ToolError;
use crate::kit::pdf::layout::{Align, Block, Omitted, Span};

const MAX_CELLS: usize = 1_000_000;
const MAX_COLUMN: u32 = 16_383;
const MAX_ROW: u32 = 1_048_575;

pub struct Workbook {
    pub sheets: Vec<Sheet>,
    pub families: Vec<String>,
    pub body_family: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
}

/// One sheet's non-empty cells by `(row, column)`, counted from 0, with its print settings.
#[derive(Debug, Default)]
pub struct Sheet {
    pub name: String,
    pub hidden: bool,
    pub landscape: bool,
    pub fit: bool,
    pub paper: Option<(f32, f32)>,
    pub omitted: Omitted,
    pub(super) areas: Vec<Area>,
    pub(super) titles: Option<u32>,
    pub(super) cells: BTreeMap<(u32, u32), Cell>,
    pub(super) hidden_rows: Vec<(u32, u32)>,
    pub(super) hidden_columns: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Cell { pub(super) spans: Vec<Span>, pub(super) numeric: bool }

/// A cell range; a missing row or column bound spans the used part of the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Area { pub(super) top: Option<u32>, pub(super) left: Option<u32>, pub(super) bottom: Option<u32>, pub(super) right: Option<u32> }

fn too_large(name: &str) -> ToolError {
    ToolError::invalid_input(format!("The sheet \u{201c}{name}\u{201d} has too many cells to lay out, so no PDF was made."))
}

fn covers(ranges: &[(u32, u32)], index: u32) -> bool { ranges.iter().any(|(start, end)| (*start..=*end).contains(&index)) }

impl Sheet {
    pub(super) fn put(&mut self, row: u32, column: u32, cell: Cell) -> Result<(), ToolError> {
        if row > MAX_ROW || column > MAX_COLUMN || cell.spans.iter().all(|span| span.text.trim().is_empty()) { return Ok(()); }
        if self.cells.len() >= MAX_CELLS && !self.cells.contains_key(&(row, column)) { return Err(too_large(&self.name)); }
        self.cells.insert((row, column), cell);
        Ok(())
    }

    pub fn is_empty(&self) -> bool { self.cells.is_empty() }

    fn used(&self) -> Option<(u32, u32, u32, u32)> {
        let (top, bottom) = (self.cells.keys().next()?.0, self.cells.keys().next_back()?.0);
        let left = self.cells.keys().map(|(_, column)| *column).min()?;
        let right = self.cells.keys().map(|(_, column)| *column).max()?;
        Some((top, left, bottom, right))
    }

    /// One table per printed range, or for the used range when print areas are ignored or absent. Hidden rows and columns are left out and the print-title row repeats as the header.
    pub fn tables(&self, print_areas: bool) -> Result<Vec<Block>, ToolError> {
        let Some((top, left, bottom, right)) = self.used() else { return Ok(Vec::new()) };
        let ranges = if print_areas && !self.areas.is_empty() {
            self.areas.iter().map(|area| (
                area.top.unwrap_or(top),
                area.left.unwrap_or(left),
                area.bottom.map_or(bottom, |end| end.min(bottom)),
                area.right.map_or(right, |end| end.min(right)),
            )).collect()
        } else {
            vec![(top, left, bottom, right)]
        };
        let mut blocks = Vec::new();
        for (top, left, bottom, right) in ranges {
            if top > bottom || left > right { continue; }
            let rows = (top..=bottom).filter(|row| !covers(&self.hidden_rows, *row) && Some(*row) != self.titles).collect::<Vec<_>>();
            let columns = (left..=right).filter(|column| !covers(&self.hidden_columns, *column)).collect::<Vec<_>>();
            if rows.len().saturating_mul(columns.len()) > MAX_CELLS { return Err(too_large(&self.name)); }
            let line = |row: u32| columns.iter().map(|column| self.cells.get(&(row, *column)).map(|cell| cell.spans.clone()).unwrap_or_default()).collect::<Vec<_>>();
            let body = rows.iter().map(|row| line(*row)).collect::<Vec<_>>();
            if body.iter().flatten().all(Vec::is_empty) { continue; }
            let head = self.titles.filter(|row| !covers(&self.hidden_rows, *row)).map(line).unwrap_or_default();
            let aligns = columns.iter().map(|column| {
                let mut cells = rows.iter().filter_map(|row| self.cells.get(&(*row, *column))).peekable();
                if cells.peek().is_some() && cells.all(|cell| cell.numeric) { Align::Right } else { Align::Left }
            }).collect();
            blocks.push(Block::Table { aligns, head, rows: body });
        }
        Ok(blocks)
    }
}

/// Splits `text` at `separator` outside single-quoted sheet names.
pub(super) fn split_quoted(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if character == '\'' { quoted = !quoted; }
        if character == separator && !quoted {
            parts.push(&text[start..index]);
            start = index + character.len_utf8();
        }
    }
    parts.push(&text[start..]);
    parts.into_iter().map(str::trim).filter(|part| !part.is_empty()).collect()
}

/// A cell reference such as `$B$3`, `B` or `3` as `(row, column)`, either of which may be missing.
fn endpoint(text: &str) -> Option<(Option<u32>, Option<u32>)> {
    let text = text.trim().replace('$', "");
    let split = text.find(|character: char| character.is_ascii_digit()).unwrap_or(text.len());
    let (letters, digits) = text.split_at(split);
    if letters.len() > 3 || !letters.chars().all(|character| character.is_ascii_alphabetic()) { return None; }
    let column = (!letters.is_empty()).then(|| letters.chars().fold(0u32, |number, character| number * 26 + (character.to_ascii_uppercase() as u32 - 64)) - 1);
    let row = if digits.is_empty() { None } else { Some(digits.parse::<u32>().ok()?.checked_sub(1)?) };
    (column.is_some() || row.is_some()).then_some((row, column))
}

pub(super) fn reference(text: &str) -> Option<(u32, u32)> {
    match endpoint(text)? { (Some(row), Some(column)) => Some((row, column)), _ => None }
}

/// A range such as `A1:C10`, `A:C`, `1:5` or a single cell, with any sheet prefix already removed.
pub(super) fn area(text: &str) -> Option<Area> {
    let (start, end) = text.split_once(':').unwrap_or((text, text));
    let ((top, left), (bottom, right)) = (endpoint(start)?, endpoint(end)?);
    Some(Area { top, left, bottom, right })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::pdf::layout::Style;

    fn cell(text: &str, numeric: bool) -> Cell { Cell { spans: vec![Span { text: text.into(), style: Style::default(), link: None }], numeric } }

    fn texts(block: &Block) -> (Vec<String>, Vec<Vec<String>>, Vec<Align>) {
        let Block::Table { aligns, head, rows } = block else { panic!("not a table") };
        let join = |cell: &Vec<Span>| cell.iter().map(|span| span.text.as_str()).collect::<String>();
        (head.iter().map(join).collect(), rows.iter().map(|row| row.iter().map(join).collect()).collect(), aligns.clone())
    }

    #[test]
    fn references_and_ranges_parse_with_or_without_bounds() {
        assert_eq!(reference("$C$12"), Some((11, 2)));
        assert_eq!(reference("AA1"), Some((0, 26)));
        assert_eq!(reference("C"), None);
        assert_eq!(area("$A$1:$C$3"), Some(Area { top: Some(0), left: Some(0), bottom: Some(2), right: Some(2) }));
        assert_eq!(area("$2:$4"), Some(Area { top: Some(1), left: None, bottom: Some(3), right: None }));
        assert_eq!(area("#REF!"), None);
        assert_eq!(split_quoted("'a, b'!$A$1:$B$2,Sheet2!$C:$C", ','), vec!["'a, b'!$A$1:$B$2", "Sheet2!$C:$C"]);
    }

    #[test]
    fn print_areas_hidden_rows_and_title_rows_shape_the_tables() {
        let mut sheet = Sheet { name: "Data".into(), ..Sheet::default() };
        for (row, column, text, numeric) in [(0, 0, "Item", false), (0, 1, "Cost", false), (1, 0, "Pens", false), (1, 1, "3", true), (2, 0, "Ink", false), (2, 1, "9", true), (5, 4, "Far", false)] {
            sheet.put(row, column, cell(text, numeric)).unwrap();
        }
        sheet.put(3, 3, cell("  ", false)).unwrap();
        assert_eq!(sheet.cells.len(), 7);
        sheet.titles = Some(0);
        sheet.hidden_rows = vec![(2, 2)];
        sheet.areas = vec![Area { top: Some(0), left: Some(0), bottom: Some(2), right: Some(1) }];

        let tables = sheet.tables(true).unwrap();
        assert_eq!(tables.len(), 1);
        let (head, rows, aligns) = texts(&tables[0]);
        assert_eq!(head, ["Item", "Cost"]);
        assert_eq!(rows, [["Pens", "3"]]);
        assert_eq!(aligns, [Align::Left, Align::Right]);

        let (_, rows, _) = texts(&sheet.tables(false).unwrap()[0]);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[3], ["", "", "", "", "Far"]);
    }
}
