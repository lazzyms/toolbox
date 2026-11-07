//! Reads an Excel (XLSX) workbook into the shared sheet model: stored formula results, number formats, fonts and print settings.

use std::collections::HashMap;

use crate::kit::contracts::ToolError;
use crate::kit::pdf::layout::{Span, Style};
use crate::kit::pdf::spreadsheet::{area, reference, split_quoted, Cell, Sheet, Workbook};
use crate::kit::pdf::wordprocessing::{self as shared, hex_color, rels, Families, Node, Package};

pub const DAMAGED: &str = "This Excel workbook is damaged or incomplete, so no PDF was made.";
const LETTER: (f32, f32) = (612., 792.);
const MAX_SECTIONS: usize = 4;

fn damaged() -> ToolError { ToolError::invalid_input(DAMAGED) }

fn tree(bytes: &[u8]) -> Result<Node, ToolError> { shared::tree(bytes, DAMAGED) }

fn flag(node: &Node, name: &str) -> bool { matches!(node.attr(name), Some("1" | "true")) }

/// Reads an XLSX package. `budget` caps the bytes expanded across every part read; going over returns `exceeded`.
pub fn parse(bytes: &[u8], budget: u64, exceeded: ToolError) -> Result<Workbook, ToolError> {
    let mut package = Package::open(bytes, budget, exceeded, DAMAGED)?;
    let main = rels(&mut package, "")?.into_values().find(|rel| !rel.external && rel.kind.ends_with("/officeDocument")).map(|rel| rel.target).unwrap_or_else(|| "xl/workbook.xml".into());
    let workbook = tree(&package.part(&main)?.ok_or_else(damaged)?)?;
    if workbook.name != "workbook" { return Err(damaged()); }
    let links = rels(&mut package, &main)?;
    let part_of = |kind: &str| links.values().find(|rel| !rel.external && rel.kind.ends_with(kind)).map(|rel| rel.target.clone());

    let styles = match part_of("/styles") {
        Some(path) => package.part(&path)?.map(|bytes| tree(&bytes)).transpose()?,
        None => None,
    };
    let mut styles = Styles::read(styles.as_ref());
    let strings = match part_of("/sharedStrings").map(|path| package.part(&path)).transpose()?.flatten() {
        Some(bytes) => {
            let mut strings = Vec::new();
            shared::walk(&bytes, |open| open.len() == 1, DAMAGED, |item| {
                if item.name == "si" { strings.push(runs(&item, &mut styles.fonts)); }
                Ok(())
            })?;
            strings
        }
        None => Vec::new(),
    };
    let date1904 = workbook.child("workbookPr").is_some_and(|properties| flag(properties, "date1904"));

    let mut areas: HashMap<usize, String> = HashMap::new();
    let mut titles: HashMap<usize, String> = HashMap::new();
    for name in workbook.child("definedNames").into_iter().flat_map(Node::nodes).filter(|node| node.name == "definedName") {
        let Some(index) = name.attr("localSheetId").and_then(|index| index.parse().ok()) else { continue };
        match name.attr("name") {
            Some("_xlnm.Print_Area") => { areas.insert(index, name.text()); }
            Some("_xlnm.Print_Titles") => { titles.insert(index, name.text()); }
            _ => {}
        }
    }

    let mut sheets = Vec::new();
    let entries = workbook.child("sheets").ok_or_else(damaged)?.nodes().filter(|node| node.name == "sheet").cloned().collect::<Vec<_>>();
    for (index, entry) in entries.iter().enumerate() {
        let Some(rel) = entry.attr("id").and_then(|id| links.get(id)).filter(|rel| !rel.external) else { continue };
        let mut sheet = Sheet {
            name: entry.attr("name").unwrap_or_default().to_string(),
            hidden: matches!(entry.attr("state"), Some("hidden" | "veryHidden")),
            ..Sheet::default()
        };
        if rel.kind.ends_with("/chartsheet") {
            sheet.omitted.charts = 1;
        } else if rel.kind.ends_with("/worksheet") {
            let Some(bytes) = package.part(&rel.target)? else { return Err(damaged()) };
            let reader = Reader { styles: &styles, strings: &strings, date1904 };
            read_sheet(&mut package, &rel.target, &bytes, &reader, &mut sheet)?;
            sheet.areas = areas.get(&index).map(|ranges| split_quoted(ranges, ',').into_iter().filter_map(|range| area(without_sheet(range))).collect()).unwrap_or_default();
            sheet.titles = titles.get(&index).and_then(|ranges| split_quoted(ranges, ',').into_iter().filter_map(|range| area(without_sheet(range))).find(|area| area.left.is_none()).and_then(|area| area.top));
        } else {
            continue;
        }
        sheets.push(sheet);
    }

    let (title, author) = match rels(&mut package, "")?.into_values().find(|rel| !rel.external && rel.kind.ends_with("/core-properties")) {
        Some(rel) => match package.part(&rel.target)? {
            Some(bytes) => {
                let core = tree(&bytes)?;
                let field = |name: &str| core.child(name).map(|node| node.text().trim().to_string()).filter(|text| !text.is_empty());
                (field("title"), field("creator"))
            }
            None => (None, None),
        },
        None => (None, None),
    };
    Ok(Workbook { sheets, families: styles.fonts.others, body_family: styles.fonts.body, title, author })
}

fn without_sheet(range: &str) -> &str { range.rsplit_once('!').map_or(range, |(_, cells)| cells) }

struct Reader<'a> { styles: &'a Styles, strings: &'a [Vec<(String, Option<Style>)>], date1904: bool }

fn read_sheet(package: &mut Package, path: &str, bytes: &[u8], reader: &Reader, sheet: &mut Sheet) -> Result<(), ToolError> {
    let mut next_row = 0u32;
    let root = shared::walk(bytes, |open| open.len() == 2 && open[1].name == "sheetData", DAMAGED, |row| {
        if row.name != "row" { return Ok(()); }
        let index = row.attr("r").and_then(|index| index.parse::<u32>().ok()).and_then(|index| index.checked_sub(1)).unwrap_or(next_row);
        next_row = index.saturating_add(1);
        if flag(&row, "hidden") {
            match sheet.hidden_rows.last_mut() {
                Some((_, end)) if end.saturating_add(1) == index => *end = index,
                _ => sheet.hidden_rows.push((index, index)),
            }
        }
        let mut next_column = 0u32;
        for cell in row.nodes().filter(|node| node.name == "c") {
            let (row_index, column) = cell.attr("r").and_then(reference).unwrap_or((index, next_column));
            next_column = column.saturating_add(1);
            if let Some(value) = reader.cell(cell) { sheet.put(row_index, column, value)?; }
        }
        Ok(())
    })?;
    if root.name != "worksheet" { return Err(damaged()); }

    for column in root.child("cols").into_iter().flat_map(Node::nodes).filter(|node| node.name == "col" && flag(node, "hidden")) {
        let bound = |name: &str| column.attr(name).and_then(|value| value.parse::<u32>().ok()).and_then(|value| value.checked_sub(1));
        if let (Some(min), Some(max)) = (bound("min"), bound("max")) { sheet.hidden_columns.push((min, max)); }
    }
    if let Some(setup) = root.child("pageSetup") {
        sheet.landscape = setup.attr("orientation") == Some("landscape");
        sheet.paper = setup.attr("paperSize").and_then(|code| code.trim().parse().ok()).and_then(paper);
        sheet.fit = root.child("sheetPr").and_then(|properties| properties.child("pageSetUpPr")).is_some_and(|properties| flag(properties, "fitToPage"))
            && setup.attr("fitToWidth") != Some("0");
    }
    sheet.omitted.objects += root.child("oleObjects").map_or(0, |objects| objects.nodes().count());
    if let Some(id) = root.child("drawing").and_then(|drawing| drawing.attr("id")) {
        let drawing = rels(package, path)?.remove(id).filter(|rel| !rel.external);
        if let Some(bytes) = drawing.map(|rel| package.part(&rel.target)).transpose()?.flatten() {
            count_drawings(&tree(&bytes)?, sheet);
        }
    }
    Ok(())
}

fn count_drawings(node: &Node, sheet: &mut Sheet) {
    for child in node.nodes() {
        match child.name.as_str() {
            "pic" => sheet.omitted.pictures += 1,
            "graphicFrame" => {
                let uri = child.find("graphicData").and_then(|data| data.attr("uri")).unwrap_or_default();
                if uri.ends_with("/chart") || uri.contains("chartex") { sheet.omitted.charts += 1 }
                else if uri.ends_with("/diagram") { sheet.omitted.diagrams += 1 }
                else { sheet.omitted.objects += 1 }
            }
            _ => count_drawings(child, sheet),
        }
    }
}

impl Reader<'_> {
    fn cell(&self, cell: &Node) -> Option<Cell> {
        let format = cell.attr("s").and_then(|index| index.parse::<usize>().ok()).and_then(|index| self.styles.cells.get(index)).copied().unwrap_or_default();
        let style = self.styles.font(format.font);
        let value = cell.child("v").map(Node::text);
        let plain = |text: String| vec![Span { text, style, link: None }];
        let (spans, numeric) = match cell.attr("t").unwrap_or("n") {
            "s" => {
                let runs = self.strings.get(value?.trim().parse::<usize>().ok()?)?;
                (runs.iter().map(|(text, run)| Span { text: text.clone(), style: run.unwrap_or(style), link: None }).collect(), false)
            }
            "inlineStr" => {
                let mut fonts = Families::default();
                (runs(cell.child("is")?, &mut fonts).into_iter().map(|(text, _)| Span { text, style, link: None }).collect(), false)
            }
            "b" => (plain(if value?.trim() == "1" { "TRUE" } else { "FALSE" }.into()), false),
            "str" | "e" | "d" => (plain(value?), false),
            _ => {
                let number = value?.trim().parse::<f64>().ok().filter(|number| number.is_finite())?;
                (plain(format_number(number, self.styles.code(format.number), self.date1904)), true)
            }
        };
        Some(Cell { spans, numeric })
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Format { number: u32, font: usize }

struct Styles { formats: HashMap<u32, String>, fonts: Families, faces: Vec<Style>, cells: Vec<Format> }

impl Styles {
    fn read(root: Option<&Node>) -> Self {
        let section = |name: &str| root.and_then(|root| root.child(name)).into_iter().flat_map(Node::nodes);
        let formats = section("numFmts").filter_map(|format| Some((format.attr("numFmtId")?.parse().ok()?, format.attr("formatCode")?.to_string()))).collect();
        let fonts = section("fonts").collect::<Vec<_>>();
        let mut families = Families { body: fonts.first().and_then(|font| font.val("name")).map(str::to_string), others: Vec::new() };
        let faces = fonts.iter().map(|font| font_style(font, &mut families)).collect();
        let cells = section("cellXfs").map(|format| Format {
            number: format.attr("numFmtId").and_then(|id| id.parse().ok()).unwrap_or(0),
            font: format.attr("fontId").and_then(|id| id.parse().ok()).unwrap_or(0),
        }).collect();
        Self { formats, fonts: families, faces, cells }
    }

    fn font(&self, index: usize) -> Style { self.faces.get(index).copied().unwrap_or_default() }

    fn code(&self, id: u32) -> &str {
        self.formats.get(&id).map(String::as_str).unwrap_or_else(|| builtin_format(id))
    }
}

/// The format code of a built-in number format.
pub(super) fn builtin_format(id: u32) -> &'static str {
    match id {
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        14 => "m/d/yyyy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yyyy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mm:ss.0",
        48 => "##0.0E+0",
        _ => "General",
    }
}

/// The page size of a spreadsheet paper-size code, for the common sizes.
pub(super) fn paper(code: u16) -> Option<(f32, f32)> {
    match code {
        1 => Some(LETTER),
        5 => Some((612., 1008.)),
        8 => Some((841.89, 1190.55)),
        9 => Some((595.28, 841.89)),
        11 => Some((419.53, 595.28)),
        _ => None,
    }
}

/// The run style of a `font` or `rPr` element; table text keeps one size so the sheet can be scaled to fit.
fn font_style(font: &Node, families: &mut Families) -> Style {
    Style {
        bold: font.on("b").unwrap_or(false),
        italic: font.on("i").unwrap_or(false),
        strike: font.on("strike").unwrap_or(false),
        underline: font.on("u").unwrap_or(false),
        color: font.child("color").and_then(|color| color.attr("rgb")).and_then(|rgb| hex_color(&rgb[rgb.len().saturating_sub(6)..])),
        family: families.index(font.val("name").or_else(|| font.val("rFont"))),
        ..Style::default()
    }
}

/// A shared or inline string's text runs, each with its own style when the run sets one.
fn runs(item: &Node, fonts: &mut Families) -> Vec<(String, Option<Style>)> {
    let mut runs = Vec::new();
    for child in item.nodes() {
        match child.name.as_str() {
            "t" => runs.push((child.text(), None)),
            "r" => {
                let style = child.child("rPr").map(|properties| font_style(properties, fonts));
                runs.push((child.child("t").map(Node::text).unwrap_or_default(), style));
            }
            _ => {}
        }
    }
    runs
}

#[derive(Debug, Clone, PartialEq)]
enum Token { Literal(String), Digit(char), Point, Comma, Percent, Exponent(bool), Date(char, usize), Elapsed(char, usize), Meridiem(bool), General, Text }

fn tokens(section: &str) -> Vec<Token> {
    let characters = section.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0;
    let literal = |tokens: &mut Vec<Token>, text: &str| match tokens.last_mut() {
        Some(Token::Literal(existing)) => existing.push_str(text),
        _ => tokens.push(Token::Literal(text.to_string())),
    };
    while index < characters.len() {
        let character = characters[index];
        let rest = characters[index..].iter().collect::<String>();
        let upper = rest.to_ascii_uppercase();
        index += 1;
        match character {
            '"' => {
                let end = characters[index..].iter().position(|next| *next == '"').map_or(characters.len(), |offset| index + offset);
                literal(&mut tokens, &characters[index..end].iter().collect::<String>());
                index = end + 1;
            }
            '\\' => if let Some(next) = characters.get(index) { literal(&mut tokens, &next.to_string()); index += 1; },
            '_' => { literal(&mut tokens, " "); index += 1; }
            '*' => index += 1,
            '[' => {
                let end = characters[index..].iter().position(|next| *next == ']').map_or(characters.len(), |offset| index + offset);
                let inner = characters[index..end].iter().collect::<String>();
                index = end + 1;
                let lower = inner.to_ascii_lowercase();
                if let Some(currency) = inner.strip_prefix('$') {
                    literal(&mut tokens, currency.split('-').next().unwrap_or_default());
                } else if let Some(letter) = lower.chars().next().filter(|letter| matches!(letter, 'h' | 'm' | 's') && lower.chars().all(|next| next == *letter)) {
                    tokens.push(Token::Elapsed(letter, lower.len()));
                }
            }
            '0' | '#' | '?' => tokens.push(Token::Digit(character)),
            '.' => tokens.push(Token::Point),
            ',' => tokens.push(Token::Comma),
            '%' => tokens.push(Token::Percent),
            'E' | 'e' if matches!(characters.get(index), Some('+' | '-')) => {
                tokens.push(Token::Exponent(characters[index] == '+'));
                index += 1;
            }
            _ if upper.starts_with("GENERAL") => { tokens.push(Token::General); index += 6; }
            _ if upper.starts_with("AM/PM") => { tokens.push(Token::Meridiem(true)); index += 4; }
            _ if upper.starts_with("A/P") => { tokens.push(Token::Meridiem(false)); index += 2; }
            '@' => tokens.push(Token::Text),
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => {
                let letter = character.to_ascii_lowercase();
                let mut count = 1;
                while characters.get(index).is_some_and(|next| next.to_ascii_lowercase() == letter) { count += 1; index += 1; }
                tokens.push(Token::Date(letter, count));
            }
            _ => literal(&mut tokens, &character.to_string()),
        }
    }
    tokens
}

/// Splits a format code into its sections at semicolons outside quotes, escapes and brackets.
fn sections(code: &str) -> Vec<&str> {
    let mut sections = Vec::new();
    let (mut start, mut quoted, mut bracket, mut escaped) = (0, false, false, false);
    for (index, character) in code.char_indices() {
        match character {
            _ if escaped => escaped = false,
            '\\' if !quoted => escaped = true,
            '"' => quoted = !quoted,
            '[' if !quoted => bracket = true,
            ']' if !quoted => bracket = false,
            ';' if !quoted && !bracket => {
                sections.push(&code[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    sections.push(&code[start..]);
    sections.truncate(MAX_SECTIONS);
    sections
}

/// The text Excel shows for a stored number under a format code. Formulas are never recalculated; this only formats the stored value.
pub(super) fn format_number(value: f64, code: &str, date1904: bool) -> String {
    let sections = sections(code);
    let (section, magnitude, signed) = match sections.len() {
        _ if value < 0. && sections.len() >= 2 && !sections[1].is_empty() => (sections[1], -value, false),
        _ if value == 0. && sections.len() >= 3 => (sections[2], value, false),
        _ => (sections[0], value.abs(), value < 0.),
    };
    let tokens = tokens(section);
    let dated = tokens.iter().any(|token| matches!(token, Token::Date(..) | Token::Elapsed(..) | Token::Meridiem(_)));
    let text = if dated {
        match format_date(value, &tokens, date1904) {
            Some(text) => return text,
            None => return general(value),
        }
    } else if !tokens.iter().any(|token| matches!(token, Token::Digit(_) | Token::General)) {
        let text = tokens.iter().map(|token| match token { Token::Literal(text) => text.clone(), Token::Text => general(magnitude), _ => String::new() }).collect::<String>();
        if text.is_empty() && section.is_empty() { return String::new(); }
        text
    } else {
        format_digits(magnitude, &tokens)
    };
    if signed && text.chars().any(|character| character.is_ascii_digit() && character != '0') { format!("-{text}") } else { text }
}

/// Excel's General format: up to eleven characters, switching to scientific notation for very large or small numbers.
fn general(value: f64) -> String {
    if value == 0. { return "0".into(); }
    let magnitude = value.abs();
    if !(1e-9..1e11).contains(&magnitude) {
        let text = format!("{:.5E}", value);
        let (mantissa, exponent) = text.split_once('E').unwrap_or((&text, "0"));
        let mantissa = if mantissa.contains('.') { mantissa.trim_end_matches('0').trim_end_matches('.') } else { mantissa };
        let exponent = exponent.parse::<i32>().unwrap_or(0);
        return format!("{mantissa}E{}{:02}", if exponent < 0 { '-' } else { '+' }, exponent.abs());
    }
    let decimals = (10 - (magnitude.log10().floor() as i32 + 1).max(1)).max(0) as usize;
    let text = format!("{:.*}", decimals, value);
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text }
}

fn rounded(value: f64, decimals: usize) -> String {
    let scale = 10f64.powi(decimals as i32);
    let value = if (value * scale).abs() < 1e15 { (value * scale).round() / scale } else { value };
    format!("{:.*}", decimals, value)
}

fn format_digits(value: f64, tokens: &[Token]) -> String {
    let exponent_at = tokens.iter().position(|token| matches!(token, Token::Exponent(_)));
    let mantissa = &tokens[..exponent_at.unwrap_or(tokens.len())];
    let point = mantissa.iter().position(|token| *token == Token::Point);
    let integer = &mantissa[..point.unwrap_or(mantissa.len())];
    let last_digit = integer.iter().rposition(|token| matches!(token, Token::Digit(_)));
    let scaling = mantissa.iter().rposition(|token| matches!(token, Token::Digit(_))).map_or(0, |last| mantissa[last + 1..].iter().take_while(|token| **token == Token::Comma).count());
    let grouped = integer.iter().enumerate().any(|(index, token)| *token == Token::Comma && last_digit.is_some_and(|last| index < last));
    let fraction = point.map_or(Vec::new(), |point| mantissa[point + 1..].iter().filter_map(|token| match token { Token::Digit(kind) => Some(*kind), _ => None }).collect::<Vec<_>>());
    let percent = tokens.iter().filter(|token| **token == Token::Percent).count();

    let mut value = value * 100f64.powi(percent as i32) / 1000f64.powi(scaling as i32);
    let mut exponent = 0i32;
    if exponent_at.is_some() {
        let places = integer.iter().filter(|token| matches!(token, Token::Digit(_))).count().max(1) as i32;
        if value != 0. {
            exponent = value.log10().floor() as i32;
            if places > 1 { exponent -= exponent.rem_euclid(places); }
            value /= 10f64.powi(exponent);
            if rounded(value, fraction.len()).split('.').next().is_some_and(|whole| whole.len() as i32 > places) {
                exponent += places;
                value /= 10f64.powi(places);
            }
        }
    }
    let text = rounded(value, fraction.len());
    let (whole, decimals) = text.split_once('.').unwrap_or((&text, ""));
    let whole = whole.trim_start_matches('0');
    let integer_places = integer.iter().filter(|token| matches!(token, Token::Digit(_))).count();

    let mut out = String::new();
    let mut place = integer_places;
    let mut fraction_index = 0;
    let last_significant = decimals.rfind(|character| character != '0').map_or(0, |index| index + 1);
    let mut in_fraction = false;
    let mut exponent_digits = 0;
    let exponent_total = exponent_at.map_or(0, |at| tokens[at + 1..].iter().filter(|token| matches!(token, Token::Digit(_))).count());
    let exponent_text = format!("{:0width$}", exponent.abs(), width = exponent_total);
    let digit = |out: &mut String, character: char, position: usize| {
        out.push(character);
        if grouped && position > 0 && position % 3 == 0 { out.push(','); }
    };
    for (index, token) in tokens.iter().enumerate() {
        let in_exponent = exponent_at.is_some_and(|at| index > at);
        match token {
            Token::Digit(_) if in_exponent => {
                if exponent_digits == 0 { out.push_str(&exponent_text); }
                exponent_digits += 1;
            }
            Token::Digit(kind) if in_fraction => {
                let character = decimals.chars().nth(fraction_index).unwrap_or('0');
                match kind {
                    _ if fraction_index < last_significant => out.push(character),
                    '0' => out.push('0'),
                    '?' => out.push(' '),
                    _ => {}
                }
                fraction_index += 1;
            }
            Token::Digit(kind) => {
                place -= 1;
                let digits = whole.chars().collect::<Vec<_>>();
                if place == integer_places - 1 && digits.len() > integer_places {
                    for (offset, character) in digits[..digits.len() - integer_places].iter().enumerate() {
                        digit(&mut out, *character, digits.len() - 1 - offset);
                    }
                }
                match digits.len().checked_sub(place + 1).map(|at| digits[at]) {
                    Some(character) => digit(&mut out, character, place),
                    None if *kind == '0' => digit(&mut out, '0', place),
                    None if *kind == '?' => out.push(' '),
                    None => {}
                }
            }
            Token::Point if !in_exponent && exponent_at.is_none_or(|at| index < at) => { in_fraction = true; out.push('.'); }
            Token::Exponent(plus) => {
                in_fraction = false;
                out.push('E');
                if exponent < 0 { out.push('-'); } else if *plus { out.push('+'); }
            }
            Token::Literal(text) => out.push_str(text),
            Token::Percent => out.push('%'),
            Token::General => out.push_str(&general(value)),
            Token::Text => out.push_str(&general(value)),
            Token::Point | Token::Comma | Token::Date(..) | Token::Elapsed(..) | Token::Meridiem(_) => {}
        }
    }
    out
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    era * 146_097 + of_cycle - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let of_era = days - era * 146_097;
    let of_cycle = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * of_cycle + of_cycle / 4 - of_cycle / 100);
    let shifted = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 { shifted + 3 } else { shifted - 9 };
    (of_cycle + era * 400 + i64::from(month <= 2), month, day)
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// A date or time serial in the 1900 system (which counts a 29 February 1900 that never was) or the 1904 system.
fn format_date(value: f64, tokens: &[Token], date1904: bool) -> Option<String> {
    if !(0. ..2_958_466.).contains(&value) { return None; }
    let subsecond = tokens.iter().skip_while(|token| !matches!(token, Token::Point)).filter(|token| matches!(token, Token::Digit(_))).count().min(3);
    let unit = 10f64.powi(subsecond as i32);
    let ticks = (value * 86_400. * unit).round() as i64;
    let (serial, ticks) = (ticks.div_euclid(86_400 * unit as i64), ticks.rem_euclid(86_400 * unit as i64));
    let (seconds, fraction) = (ticks / unit as i64, ticks % unit as i64);
    let (year, month, day, weekday) = if date1904 {
        let days = days_from_civil(1904, 1, 1) + serial;
        let (year, month, day) = civil_from_days(days);
        (year, month, day, (days + 4).rem_euclid(7))
    } else if serial == 60 {
        (1900, 2, 29, 3)
    } else {
        let days = days_from_civil(1899, 12, if serial < 60 { 31 } else { 30 }) + serial;
        let (year, month, day) = civil_from_days(days);
        (year, month, day, (days + 4).rem_euclid(7))
    };
    let meridiem = tokens.iter().any(|token| matches!(token, Token::Meridiem(_)));
    let (hour, minute, second) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
    let minute_at = |index: usize| {
        let before = tokens[..index].iter().rev().find_map(|token| match token { Token::Date(letter, _) | Token::Elapsed(letter, _) => Some(*letter), _ => None });
        let after = tokens[index + 1..].iter().find_map(|token| match token { Token::Date(letter, _) | Token::Elapsed(letter, _) => Some(*letter), _ => None });
        before == Some('h') || after == Some('s')
    };
    let padded = |number: i64, count: usize| if count >= 2 { format!("{number:02}") } else { number.to_string() };
    let mut out = String::new();
    let mut fraction_written = false;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Date('y', count) => out.push_str(&if *count <= 2 { format!("{:02}", year.rem_euclid(100)) } else { year.to_string() }),
            Token::Date('m', count) if *count <= 2 && minute_at(index) => out.push_str(&padded(minute, *count)),
            Token::Date('m', count) => {
                let name = MONTHS[(month - 1) as usize];
                out.push_str(&match count { 1 | 2 => padded(month, *count), 3 => name[..3].to_string(), 5 => name[..1].to_string(), _ => name.to_string() });
            }
            Token::Date('d', count) => {
                let name = DAYS[weekday as usize];
                out.push_str(&match count { 1 | 2 => padded(day, *count), 3 => name[..3].to_string(), _ => name.to_string() });
            }
            Token::Date('h', count) => {
                let hour = if meridiem { (hour + 11) % 12 + 1 } else { hour };
                out.push_str(&padded(hour, *count));
            }
            Token::Date('s', count) => out.push_str(&padded(second, *count)),
            Token::Date(..) => {}
            Token::Elapsed(letter, count) => {
                let total = serial * 86_400 + seconds;
                let amount = match letter { 'h' => total / 3_600, 'm' => total / 60, _ => total };
                out.push_str(&format!("{amount:0count$}"));
            }
            Token::Meridiem(full) => out.push_str(match (hour < 12, full) { (true, true) => "AM", (false, true) => "PM", (true, false) => "A", (false, false) => "P" }),
            Token::Point => out.push('.'),
            Token::Digit(_) if subsecond > 0 => if !fraction_written {
                out.push_str(&format!("{fraction:0subsecond$}"));
                fraction_written = true;
            },
            Token::Literal(text) => out.push_str(text),
            Token::Percent => out.push('%'),
            Token::Comma => out.push(','),
            Token::Digit(_) | Token::Exponent(_) | Token::General | Token::Text => {}
        }
    }
    Some(out)
}

#[cfg(test)]
pub mod fixtures {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    pub const NS: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

    /// A sheet's XML part around `rows` (the `sheetData` content) and `extra` elements that follow it.
    pub fn sheet(before: &str, rows: &str, extra: &str) -> String {
        format!(r#"<?xml version="1.0" encoding="UTF-8"?><worksheet {NS}>{before}<sheetData>{rows}</sheetData>{extra}</worksheet>"#)
    }

    /// Builds an XLSX package from `(name, state, sheet xml)` entries, workbook-level `defined` names, styles and shared strings.
    pub fn xlsx(sheets: &[(&str, &str, &str)], defined: &str, styles: &str, strings: &[&str]) -> Vec<u8> {
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        let mut add = |name: &str, data: &str| {
            archive.start_file(name, SimpleFileOptions::default()).unwrap();
            archive.write_all(data.as_bytes()).unwrap();
        };
        add("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/></Types>"#);
        add("_rels/.rels", &format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#));
        let entries = sheets.iter().enumerate().map(|(index, (name, state, _))| format!(r#"<sheet name="{name}" sheetId="{}" state="{state}" r:id="rId{}"/>"#, index + 1, index + 10)).collect::<String>();
        add("xl/workbook.xml", &format!(r#"<?xml version="1.0" encoding="UTF-8"?><workbook {NS}><sheets>{entries}</sheets><definedNames>{defined}</definedNames></workbook>"#));
        let links = sheets.iter().enumerate().map(|(index, _)| format!(r#"<Relationship Id="rId{}" Type="{REL}/worksheet" Target="worksheets/sheet{}.xml"/>"#, index + 10, index + 1)).collect::<String>();
        add("xl/_rels/workbook.xml.rels", &format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{links}<Relationship Id="rId1" Type="{REL}/styles" Target="styles.xml"/><Relationship Id="rId2" Type="{REL}/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#));
        add("xl/styles.xml", &format!(r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet {NS}>{styles}</styleSheet>"#));
        let items = strings.iter().map(|text| format!("<si><t>{text}</t></si>")).collect::<String>();
        add("xl/sharedStrings.xml", &format!(r#"<?xml version="1.0" encoding="UTF-8"?><sst {NS}>{items}</sst>"#));
        for (index, (_, _, xml)) in sheets.iter().enumerate() { add(&format!("xl/worksheets/sheet{}.xml", index + 1), xml); }
        archive.finish().unwrap().into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::contracts::{ErrorKind, LimitKind};
    use crate::kit::pdf::layout::Block;

    fn exceeded() -> ToolError { ToolError::limit_exceeded(LimitKind::Decompression, "over") }

    fn format(value: f64, code: &str) -> String { format_number(value, code, false) }

    #[test]
    fn number_formats_render_like_excel() {
        assert_eq!(format(1234.5, "General"), "1234.5");
        assert_eq!(format(1.0 / 3.0, "General"), "0.333333333");
        assert_eq!(format(123_456_789_012., "General"), "1.23457E+11");
        assert_eq!(format(1234.567, "#,##0.00"), "1,234.57");
        assert_eq!(format(1_234_567., "#,##0"), "1,234,567");
        assert_eq!(format(-1234.5, "#,##0.00;(#,##0.00)"), "(1,234.50)");
        assert_eq!(format(-5., "0.0"), "-5.0");
        assert_eq!(format(0., "0.00;-0.00;\"zero\""), "zero");
        assert_eq!(format(0.256, "0.0%"), "25.6%");
        assert_eq!(format(12345., "0.00E+00"), "1.23E+04");
        assert_eq!(format(0.5, "#.##"), ".5");
        assert_eq!(format(7., "000"), "007");
        assert_eq!(format(1_500_000., "#,##0.0,,\"M\""), "1.5M");
        assert_eq!(format(42.5, "[$€-407] #,##0.00"), "€ 42.50");
        assert_eq!(format(3., "0_);[Red](0)"), "3 ");
        assert_eq!(format(5551234., "000-0000"), "555-1234");
    }

    #[test]
    fn dates_and_times_use_the_serial_systems() {
        assert_eq!(format(45_292., "yyyy-mm-dd"), "2024-01-01");
        assert_eq!(format(45_292., "dddd, mmmm d, yyyy"), "Monday, January 1, 2024");
        assert_eq!(format(45_292.75, "m/d/yyyy h:mm AM/PM"), "1/1/2024 6:00 PM");
        assert_eq!(format(60., "yyyy-mm-dd"), "1900-02-29");
        assert_eq!(format(61., "yyyy-mm-dd"), "1900-03-01");
        assert_eq!(format(1., "d-mmm-yy"), "1-Jan-00");
        assert_eq!(format(1.5, "[h]:mm:ss"), "36:00:00");
        assert_eq!(format(0.000_011_574, "mm:ss.0"), "00:01.0");
        assert_eq!(format_number(0., "yyyy-mm-dd", true), "1904-01-01");
    }

    #[test]
    fn cells_keep_their_stored_results_and_styles() {
        let styles = r#"<numFmts count="1"><numFmt numFmtId="164" formatCode="0.0"/></numFmts><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><color rgb="FFFF0000"/><name val="Georgia"/></font></fonts><cellXfs count="2"><xf numFmtId="0" fontId="0"/><xf numFmtId="164" fontId="1"/></cellXfs>"#;
        let rows = r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" s="1"><f>SUM(1,2)</f><v>3</v></c><c r="C1" t="b"><v>1</v></c><c r="D1" t="inlineStr"><is><t>inline</t></is></c><c r="E1" t="e"><f>1/0</f><v>#DIV/0!</v></c></row>"#;
        let bytes = fixtures::xlsx(&[("Data", "visible", &fixtures::sheet("", rows, ""))], "", styles, &["Label"]);
        let workbook = parse(&bytes, 10_000_000, exceeded()).unwrap();
        assert_eq!(workbook.body_family.as_deref(), Some("Calibri"));
        assert_eq!(workbook.families, ["Georgia"]);
        let tables = workbook.sheets[0].tables(true).unwrap();
        let Block::Table { rows, .. } = &tables[0] else { panic!() };
        let texts = rows[0].iter().map(|cell| cell.iter().map(|span| span.text.as_str()).collect::<String>()).collect::<Vec<_>>();
        assert_eq!(texts, ["Label", "3.0", "TRUE", "inline", "#DIV/0!"]);
        let style = rows[0][1][0].style;
        assert!(style.bold);
        assert_eq!((style.color, style.family, style.half_points), (Some([255, 0, 0]), 1, 0));
    }

    #[test]
    fn damaged_and_oversized_workbooks_fail() {
        assert_eq!(parse(b"not a zip", 1_000, exceeded()).err().unwrap().message, DAMAGED);
        let rows = format!(r#"<row r="1"><c r="A1" t="inlineStr"><is><t>{}</t></is></c></row>"#, "x".repeat(10_000));
        let bytes = fixtures::xlsx(&[("Data", "visible", &fixtures::sheet("", &rows, ""))], "", "", &[]);
        assert!(matches!(parse(&bytes, 5_000, exceeded()).err().unwrap().kind, ErrorKind::LimitExceeded { .. }));
    }
}
