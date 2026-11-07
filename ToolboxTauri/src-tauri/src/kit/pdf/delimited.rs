use crate::kit::contracts::ToolError;
use crate::kit::pdf::layout::{Align, Block, Span, Style};

pub struct Table { pub block: Block, pub rows: usize }

fn cell(text: &str) -> Vec<Span> {
    vec![Span { text: text.to_string(), style: Style::default(), link: None }]
}

fn numeric(text: &str) -> bool {
    let text = text.trim();
    let text = text.strip_prefix(['-', '+']).unwrap_or(text);
    let text = text.strip_prefix(['$', '\u{20ac}', '\u{a3}', '\u{a5}']).unwrap_or(text);
    let text = text.strip_suffix('%').unwrap_or(text);
    text.chars().any(|character| character.is_ascii_digit()) && text.chars().all(|character| character.is_ascii_digit() || matches!(character, ',' | '.'))
}

pub fn parse(text: &str, delimiter: u8, header: bool) -> Result<Table, ToolError> {
    let mut reader = csv::ReaderBuilder::new().delimiter(delimiter).has_headers(false).flexible(true).from_reader(text.as_bytes());
    let mut records = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|error| ToolError::invalid_input(format!("This table can't be read: {error}")))?;
        records.push(record.iter().map(str::to_string).collect::<Vec<_>>());
    }
    while records.last().is_some_and(|record| record.iter().all(|value| value.trim().is_empty())) { records.pop(); }
    if records.is_empty() { return Err(ToolError::invalid_input("This table has no rows, so no PDF was made.")); }

    let head = if header { records.remove(0) } else { Vec::new() };
    let columns = records.iter().map(Vec::len).chain([head.len()]).max().unwrap_or(0);
    let aligns = (0..columns).map(|column| {
        let mut values = records.iter().filter_map(|record| record.get(column)).filter(|value| !value.trim().is_empty()).peekable();
        if values.peek().is_some() && values.all(|value| numeric(value)) { Align::Right } else { Align::Left }
    }).collect();
    let rows = records.len();
    let block = Block::Table {
        aligns,
        head: head.iter().map(|value| cell(value)).collect(),
        rows: records.iter().map(|record| record.iter().map(|value| cell(value)).collect()).collect(),
    };
    Ok(Table { block, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(table: &Table) -> (Vec<String>, Vec<Vec<String>>) {
        let Block::Table { head, rows, .. } = &table.block else { panic!("not a table") };
        let text = |cell: &Vec<Span>| cell.iter().map(|span| span.text.as_str()).collect::<String>();
        (head.iter().map(text).collect(), rows.iter().map(|row| row.iter().map(text).collect()).collect())
    }

    #[test]
    fn quoted_fields_keep_delimiters_quotes_and_line_breaks() {
        let table = parse("name;note;amount\n\"Smith; Jo\";\"said \"\"hi\"\"\nthen left\";1,200.50\nLee;;-3\n\n", b';', true).unwrap();
        let (head, rows) = cells(&table);
        assert_eq!(head, ["name", "note", "amount"]);
        assert_eq!(rows, [vec!["Smith; Jo", "said \"hi\"\nthen left", "1,200.50"], vec!["Lee", "", "-3"]]);
        assert_eq!(table.rows, 2);
        let Block::Table { aligns, .. } = &table.block else { unreachable!() };
        assert_eq!(aligns, &[Align::Left, Align::Left, Align::Right]);
    }

    #[test]
    fn ragged_rows_without_a_header_and_empty_tables() {
        let table = parse("a\tb\tc\nd\n", b'\t', false).unwrap();
        let (head, rows) = cells(&table);
        assert!(head.is_empty());
        assert_eq!(rows, [vec!["a", "b", "c"], vec!["d"]]);
        let Block::Table { aligns, .. } = &table.block else { unreachable!() };
        assert_eq!(aligns.len(), 3);
        assert!(parse("\n,,\n", b',', true).is_err());
    }
}
