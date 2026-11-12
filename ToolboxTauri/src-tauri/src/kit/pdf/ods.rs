//! Reads an OpenDocument spreadsheet (ODS) into the shared sheet model, using the text each cell stores for its value.

use crate::kit::contracts::ToolError;
use crate::kit::pdf::odt::{link_target, push_collapsed, text_style, Props, Styles};
use crate::kit::pdf::spreadsheet::{area, split_quoted, Cell, Sheet, Workbook};
use crate::kit::pdf::wordprocessing::{self as shared, css_points, Child, Collected, Families, Node, Package};

pub const DAMAGED: &str = "This OpenDocument spreadsheet is damaged or incomplete, so no PDF was made.";
const NUMERIC: &[&str] = &["float", "percentage", "currency", "date", "time"];
const MAX_SPACES: usize = 1_000;
const MAX_ROW: u32 = 1_048_575;
const MAX_COLUMN: u32 = 16_383;

fn damaged() -> ToolError { ToolError::invalid_input(DAMAGED) }

fn tree(bytes: &[u8]) -> Result<Node, ToolError> { shared::tree(bytes, DAMAGED) }

fn in_body(open: &[Node]) -> bool { open.len() == 3 && open[1].name == "body" && open[2].name == "spreadsheet" }

fn repeated(node: &Node, name: &str) -> u32 { node.attr(name).and_then(|count| count.parse::<u32>().ok()).filter(|count| *count > 0).unwrap_or(1) }

fn hidden(node: &Node) -> bool { matches!(node.attr("visibility"), Some("collapse" | "filter")) }

/// Reads an ODS package. `budget` caps the bytes expanded across every part read; going over returns `exceeded`, and a password-protected package returns `locked`.
pub fn parse(bytes: &[u8], budget: u64, exceeded: ToolError, locked: ToolError) -> Result<Workbook, ToolError> {
    let mut package = Package::open(bytes, budget, exceeded, DAMAGED)?;
    if let Some(manifest) = package.part("META-INF/manifest.xml")? {
        if tree(&manifest)?.find("encryption-data").is_some() { return Err(locked); }
    }
    let mut styles = Styles::default();
    if let Some(bytes) = package.part("styles.xml")? { styles.read(&tree(&bytes)?); }
    let content = package.part("content.xml")?.ok_or_else(damaged)?;
    let head = shared::walk(&content, in_body, DAMAGED, |_| Ok(()))?;
    if head.name != "document-content" || head.child("body").and_then(|body| body.child("spreadsheet")).is_none() { return Err(damaged()); }
    styles.read(&head);

    let mut reader = Reader { fonts: Families { body: styles.font(&styles.cell_text(None)), others: Vec::new() }, styles };
    let mut sheets = Vec::new();
    shared::walk(&content, in_body, DAMAGED, |node| {
        if node.name == "table" { sheets.push(reader.sheet(&node)?); }
        Ok(())
    })?;

    let (title, author) = match package.part("meta.xml")? {
        Some(bytes) => {
            let meta = tree(&bytes)?;
            let field = |name: &str| meta.find(name).map(|node| node.text().trim().to_string()).filter(|text| !text.is_empty());
            (field("title"), field("initial-creator").or_else(|| field("creator")))
        }
        None => (None, None),
    };
    Ok(Workbook { sheets, families: reader.fonts.others, body_family: reader.fonts.body, title, author })
}

struct Reader { styles: Styles, fonts: Families }

#[derive(Default)]
struct Cursor { row: u32, columns: Vec<(u32, Option<String>)> }

impl Reader {
    fn sheet(&mut self, table: &Node) -> Result<Sheet, ToolError> {
        let (properties, master) = self.styles.table(table.attr("style-name"));
        let mut sheet = Sheet {
            name: table.attr("name").unwrap_or_default().to_string(),
            hidden: properties.get("display").map(String::as_str) == Some("false"),
            ..Sheet::default()
        };
        if let Some(page) = self.styles.page_layout(master.as_deref()) {
            let side = |name: &str| page.attr(name).and_then(css_points).filter(|side| *side > 0.);
            let (width, height) = (side("page-width"), side("page-height"));
            sheet.landscape = page.attr("print-orientation") == Some("landscape") || matches!((width, height), (Some(width), Some(height)) if width > height);
            sheet.paper = width.zip(height).map(|(width, height)| (width.min(height), width.max(height)));
            let scaled = |name: &str| page.attr(name).is_some_and(|count| count != "0");
            sheet.fit = scaled("scale-to-X") || scaled("scale-to-pages");
        }
        sheet.areas = table.attr("print-ranges").map(|ranges| split_quoted(ranges, ' ').into_iter().filter_map(|range| {
            let (start, end) = range.split_once(':').unwrap_or((range, range));
            let cells = |end: &str| end.rsplit_once('.').map_or(end, |(_, cells)| cells).to_string();
            area(&format!("{}:{}", cells(start), cells(end)))
        }).collect()).unwrap_or_default();

        let mut cursor = Cursor::default();
        self.rows(table, &mut sheet, &mut cursor, false)?;
        Ok(sheet)
    }

    fn rows(&mut self, parent: &Node, sheet: &mut Sheet, cursor: &mut Cursor, header: bool) -> Result<(), ToolError> {
        for node in parent.nodes() {
            match node.name.as_str() {
                "table-column" => {
                    let count = repeated(node, "number-columns-repeated");
                    let start = cursor.columns.last().map_or(0, |(end, _)| end + 1);
                    let end = start.saturating_add(count - 1).min(MAX_COLUMN);
                    if hidden(node) { sheet.hidden_columns.push((start, end)); }
                    if start <= MAX_COLUMN { cursor.columns.push((end, node.attr("default-cell-style-name").map(str::to_string))); }
                }
                "table-columns" | "table-header-columns" | "table-column-group" | "table-rows" | "table-row-group" => self.rows(node, sheet, cursor, header)?,
                "table-header-rows" => self.rows(node, sheet, cursor, true)?,
                "table-row" => self.row(node, sheet, cursor, header)?,
                "shapes" => count_frames(node, sheet),
                _ => {}
            }
        }
        Ok(())
    }

    fn row(&mut self, row: &Node, sheet: &mut Sheet, cursor: &mut Cursor, header: bool) -> Result<(), ToolError> {
        let count = repeated(row, "number-rows-repeated");
        let start = cursor.row;
        cursor.row = start.saturating_add(count);
        if start > MAX_ROW { return Ok(()); }
        let end = start.saturating_add(count - 1).min(MAX_ROW);
        if hidden(row) { sheet.hidden_rows.push((start, end)); }
        if header && sheet.titles.is_none() { sheet.titles = Some(start); }

        let mut cells = Vec::new();
        let mut column = 0u32;
        for cell in row.nodes().filter(|node| matches!(node.name.as_str(), "table-cell" | "covered-table-cell")) {
            let span = repeated(cell, "number-columns-repeated");
            count_frames(cell, sheet);
            if cell.name == "table-cell" && column <= MAX_COLUMN {
                let style = cell.attr("style-name").map(str::to_string)
                    .or_else(|| row.attr("default-cell-style-name").map(str::to_string))
                    .or_else(|| cursor.columns.iter().find(|(end, _)| *end >= column).and_then(|(_, style)| style.clone()));
                if let Some(value) = self.cell(cell, style.as_deref()) {
                    for offset in 0..span.min(MAX_COLUMN + 1 - column) { cells.push((column + offset, value.clone())); }
                }
            }
            column = column.saturating_add(span);
        }
        if cells.is_empty() { return Ok(()); }
        for index in start..=end {
            for (column, value) in &cells { sheet.put(index, *column, value.clone())?; }
        }
        Ok(())
    }

    fn cell(&mut self, cell: &Node, style: Option<&str>) -> Option<Cell> {
        let base = self.styles.cell_text(style);
        let mut out = Collected::new();
        for paragraph in cell.nodes().filter(|node| matches!(node.name.as_str(), "p" | "h")) {
            if out.segments.iter().flatten().next().is_some() {
                let (style, _) = self.style(&base);
                out.text("\n", style, None);
            }
            self.inline(paragraph, &base, None, &mut out);
        }
        let spans = out.segments.into_iter().flatten().collect::<Vec<_>>();
        if spans.is_empty() { return None; }
        Some(Cell { spans, numeric: cell.attr("value-type").is_some_and(|kind| NUMERIC.contains(&kind)) })
    }

    fn style(&mut self, text: &Props) -> (crate::kit::pdf::layout::Style, bool) {
        let (mut style, hidden) = text_style(&self.styles, &mut self.fonts, text);
        style.half_points = 0;
        (style, hidden)
    }

    fn inline(&mut self, node: &Node, text: &Props, link: Option<&str>, out: &mut Collected) {
        let (style, hidden) = self.style(text);
        for child in &node.children {
            let child = match child {
                Child::Text(value) => {
                    if !hidden { push_collapsed(out, value, style, link); }
                    continue;
                }
                Child::Node(child) => child,
            };
            match child.name.as_str() {
                "span" => {
                    let text = self.styles.text(text, child.attr("style-name"));
                    self.inline(child, &text, link, out);
                }
                "a" => {
                    let target = child.attr("href").and_then(link_target).filter(|target| !target.starts_with('#'));
                    self.inline(child, text, target.as_deref().or(link), out);
                }
                "s" if !hidden => out.text(&" ".repeat(child.attr("c").and_then(|count| count.parse::<usize>().ok()).unwrap_or(1).min(MAX_SPACES)), style, link),
                "tab" if !hidden => out.text(" ", style, link),
                "line-break" if !hidden => out.text("\n", style, link),
                "annotation" | "note" | "frame" => {}
                _ => self.inline(child, text, link, out),
            }
        }
    }
}

fn count_frames(node: &Node, sheet: &mut Sheet) {
    for child in node.nodes() {
        if child.name == "frame" {
            if child.child("object").is_some() || child.child("object-ole").is_some() { sheet.omitted.charts += 1 } else if child.child("image").is_some() { sheet.omitted.pictures += 1 }
        } else if child.name != "p" {
            count_frames(child, sheet);
        }
    }
}

#[cfg(test)]
pub mod fixtures {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    pub use crate::kit::pdf::odt::fixtures::NS;

    /// Builds an ODS package from shared styles, automatic styles and the `office:spreadsheet` body.
    pub fn ods(styles: &str, automatic: &str, body: &str) -> Vec<u8> {
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        archive.start_file("mimetype", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
        archive.write_all(b"application/vnd.oasis.opendocument.spreadsheet").unwrap();
        let mut add = |name: &str, data: String| {
            archive.start_file(name, SimpleFileOptions::default()).unwrap();
            archive.write_all(data.as_bytes()).unwrap();
        };
        add("META-INF/manifest.xml", r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#.into());
        add("styles.xml", format!(r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles {NS}>{styles}</office:document-styles>"#));
        add("content.xml", format!(r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS}><office:automatic-styles>{automatic}</office:automatic-styles><office:body><office:spreadsheet>{body}</office:spreadsheet></office:body></office:document-content>"#));
        archive.finish().unwrap().into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::contracts::LimitKind;
    use crate::kit::pdf::layout::Block;

    fn exceeded() -> ToolError { ToolError::limit_exceeded(LimitKind::Decompression, "over") }
    fn locked() -> ToolError { ToolError::invalid_input("locked") }

    #[test]
    fn repeated_rows_and_cells_header_rows_and_hidden_columns_are_read() {
        let body = r#"<table:table table:name="Data"><table:table-column table:number-columns-repeated="2"/><table:table-column table:visibility="collapse"/><table:table-header-rows><table:table-row><table:table-cell><text:p>Head</text:p></table:table-cell></table:table-row></table:table-header-rows><table:table-row table:number-rows-repeated="2"><table:table-cell office:value-type="float" office:value="1" table:number-columns-repeated="2"><text:p>1.00</text:p></table:table-cell><table:table-cell><text:p>gone</text:p></table:table-cell></table:table-row><table:table-row table:number-rows-repeated="1048570"><table:table-cell table:number-columns-repeated="1024"/></table:table-row></table:table>"#;
        let workbook = parse(&fixtures::ods("", "", body), 10_000_000, exceeded(), locked()).unwrap();
        let tables = workbook.sheets[0].tables(true).unwrap();
        let Block::Table { head, rows, aligns } = &tables[0] else { panic!() };
        let text = |cell: &Vec<crate::kit::pdf::layout::Span>| cell.iter().map(|span| span.text.as_str()).collect::<String>();
        assert_eq!(head.iter().map(text).collect::<Vec<_>>(), ["Head", ""]);
        assert_eq!(rows.iter().map(|row| row.iter().map(text).collect::<Vec<_>>()).collect::<Vec<_>>(), [["1.00", "1.00"], ["1.00", "1.00"]]);
        assert_eq!(aligns.len(), 2);
    }

    #[test]
    fn text_documents_are_not_spreadsheets() {
        let text = crate::kit::pdf::odt::fixtures::odt("", "<text:p>Hi</text:p>", &[], &[]);
        assert_eq!(parse(&text, 10_000_000, exceeded(), locked()).err().unwrap().message, DAMAGED);
    }
}
