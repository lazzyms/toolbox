use std::collections::BTreeMap;

use encoding_rs::{Encoding, BIG5, EUC_KR, GBK, MACINTOSH, SHIFT_JIS, UTF_8, WINDOWS_1250, WINDOWS_1251, WINDOWS_1252, WINDOWS_1253, WINDOWS_1254, WINDOWS_1255, WINDOWS_1256, WINDOWS_1257, WINDOWS_1258, WINDOWS_874};

use crate::kit::contracts::ToolError;
use crate::kit::pdf::layout::{Align, Block, PageSetup, Paragraph, Picture, Span, Style};

const DAMAGED: &str = "This RTF file is damaged or incomplete, so no PDF was made.";
const MAX_DEPTH: usize = 1024;
const DEFAULT_HALF_POINTS: u16 = 24;

pub struct Parsed {
    pub blocks: Vec<Block>,
    pub pictures: Vec<Picture>,
    pub families: Vec<String>,
    pub body_family: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub page: PageSetup,
    pub omitted_pictures: usize,
}

enum Token<'a> { Open, Close, Word(&'a str, Option<i32>), Symbol(u8), Hex(u8), Text(u8), Binary(&'a [u8]) }

struct Lexer<'a> { bytes: &'a [u8], at: usize }

impl<'a> Lexer<'a> {
    fn next(&mut self) -> Result<Option<Token<'a>>, ToolError> {
        let Some(&byte) = self.bytes.get(self.at) else { return Ok(None) };
        self.at += 1;
        Ok(Some(match byte {
            b'{' => Token::Open,
            b'}' => Token::Close,
            b'\\' => self.control()?,
            other => Token::Text(other),
        }))
    }

    fn control(&mut self) -> Result<Token<'a>, ToolError> {
        let Some(&first) = self.bytes.get(self.at) else { return Err(damaged()) };
        if !first.is_ascii_alphabetic() {
            self.at += 1;
            return Ok(match first {
                b'\'' => {
                    let digits = self.bytes.get(self.at..self.at + 2).ok_or_else(damaged)?;
                    self.at += 2;
                    let text = std::str::from_utf8(digits).map_err(|_| damaged())?;
                    Token::Hex(u8::from_str_radix(text, 16).map_err(|_| damaged())?)
                }
                b'\n' | b'\r' => Token::Word("par", None),
                other => Token::Symbol(other),
            });
        }
        let start = self.at;
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_alphabetic) { self.at += 1; }
        let word = std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| damaged())?;
        let number_start = self.at;
        if self.bytes.get(self.at) == Some(&b'-') { self.at += 1; }
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) { self.at += 1; }
        let digits = &self.bytes[number_start..self.at];
        let parameter = match digits {
            [] => None,
            [b'-'] => { self.at = number_start; None }
            digits => Some(std::str::from_utf8(digits).ok().and_then(|text| text.parse::<i64>().ok()).unwrap_or(0).clamp(i32::MIN as i64, i32::MAX as i64) as i32),
        };
        if self.bytes.get(self.at) == Some(&b' ') { self.at += 1; }
        if word == "bin" {
            let length = parameter.unwrap_or(0).max(0) as usize;
            let data = self.bytes.get(self.at..self.at.saturating_add(length)).ok_or_else(damaged)?;
            self.at += length;
            return Ok(Token::Binary(data));
        }
        Ok(Token::Word(word, parameter))
    }
}

fn damaged() -> ToolError { ToolError::invalid_input(DAMAGED) }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Destination { Body, FontTable, ColorTable, Info, Title, Author, Picture, Instruction, Skip }

#[derive(Debug, Clone)]
struct State {
    destination: Destination,
    font: Option<i32>,
    half_points: u16,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    hidden: bool,
    deleted: bool,
    color: usize,
    skip_after_unicode: usize,
    align: Align,
    left_indent: i32,
    right_indent: i32,
    space_before: i32,
    space_after: i32,
    line_spacing: i32,
    line_multiple: bool,
    in_table: bool,
    link: Option<String>,
}

impl State {
    fn new() -> Self {
        Self {
            destination: Destination::Body, font: None, half_points: DEFAULT_HALF_POINTS, bold: false, italic: false, underline: false, strike: false,
            hidden: false, deleted: false, color: 0, skip_after_unicode: 1, align: Align::Left, left_indent: 0, right_indent: 0,
            space_before: 0, space_after: 0, line_spacing: 0, line_multiple: false, in_table: false, link: None,
        }
    }

    fn reset_character(&mut self) {
        let fresh = Self::new();
        (self.font, self.half_points, self.bold, self.italic, self.underline, self.strike, self.hidden, self.deleted, self.color) =
            (None, fresh.half_points, false, false, false, false, false, false, 0);
    }

    fn reset_paragraph(&mut self) {
        let fresh = Self::new();
        (self.align, self.left_indent, self.right_indent, self.space_before, self.space_after, self.line_spacing, self.line_multiple, self.in_table) =
            (fresh.align, 0, 0, 0, 0, 0, false, false);
    }
}

struct Font { name: String, encoding: Option<&'static Encoding>, symbol: bool }

#[derive(Default)]
struct PictureData { kind: Option<bool>, data: Vec<u8>, nibble: Option<u8>, goal: (i32, i32), scale: (i32, i32) }

#[derive(Default)]
struct Table { rows: Vec<Vec<Vec<Span>>>, row: Vec<Vec<Span>>, cell: Vec<Span>, aligns: Vec<Align> }

struct Parser {
    stack: Vec<State>,
    star: bool,
    skip: usize,
    surrogate: Option<u32>,
    pending: Vec<u8>,
    code_page: &'static Encoding,
    default_font: Option<i32>,
    fonts: BTreeMap<i32, Font>,
    font_entry: Option<(i32, Vec<u8>, Option<&'static Encoding>, bool)>,
    colors: Vec<Option<[u8; 3]>>,
    color_entry: [Option<u8>; 3],
    families: Vec<String>,
    family_index: BTreeMap<String, u16>,
    instruction: String,
    picture: PictureData,
    spans: Vec<Span>,
    table: Option<Table>,
    blocks: Vec<Block>,
    pictures: Vec<Picture>,
    omitted_pictures: usize,
    title: String,
    author: String,
    page: PageSetup,
    section_page: PageSetup,
}

pub fn parse(bytes: &[u8]) -> Result<Parsed, ToolError> {
    let start = bytes.iter().position(|byte| !byte.is_ascii_whitespace()).unwrap_or(bytes.len());
    if !bytes[start..].starts_with(b"{\\rtf") { return Err(damaged()); }
    let mut lexer = Lexer { bytes: &bytes[start..], at: 0 };
    let mut parser = Parser {
        stack: Vec::new(), star: false, skip: 0, surrogate: None, pending: Vec::new(), code_page: WINDOWS_1252, default_font: None, fonts: BTreeMap::new(),
        font_entry: None, colors: Vec::new(), color_entry: [None; 3], families: Vec::new(), family_index: BTreeMap::new(), instruction: String::new(),
        picture: PictureData::default(), spans: Vec::new(), table: None, blocks: Vec::new(), pictures: Vec::new(), omitted_pictures: 0,
        title: String::new(), author: String::new(), page: PageSetup::default(), section_page: PageSetup::default(),
    };
    while let Some(token) = lexer.next()? {
        match token {
            Token::Open => {
                parser.flush();
                if parser.stack.len() >= MAX_DEPTH { return Err(damaged()); }
                let state = parser.stack.last().cloned().unwrap_or_else(State::new);
                parser.stack.push(state);
                parser.star = false;
                parser.skip = 0;
            }
            Token::Close => {
                parser.flush();
                let closed = parser.stack.pop().ok_or_else(damaged)?;
                parser.star = false;
                parser.skip = 0;
                if closed.destination == Destination::FontTable { parser.finish_font(); }
                if closed.destination == Destination::Picture && parser.stack.last().is_none_or(|state| state.destination != Destination::Picture) {
                    parser.finish_picture(&closed);
                }
                if parser.stack.is_empty() { break; }
            }
            _ if parser.stack.is_empty() => return Err(damaged()),
            Token::Word(word, parameter) => { parser.flush(); parser.word(word, parameter); }
            Token::Symbol(symbol) => { parser.flush(); parser.symbol(symbol); }
            Token::Hex(byte) => parser.byte(byte),
            Token::Text(b'\r' | b'\n') => {}
            Token::Text(byte) => parser.byte(byte),
            Token::Binary(data) => {
                parser.flush();
                if parser.state().destination == Destination::Picture { parser.picture.data.extend_from_slice(data); }
            }
        }
    }
    if !parser.stack.is_empty() { return Err(damaged()); }
    Ok(parser.finish())
}

fn toggle(parameter: Option<i32>) -> bool { parameter != Some(0) }

fn twips(value: i32) -> f32 { value as f32 / 20. }

fn charset(value: i32) -> Option<&'static Encoding> {
    Some(match value {
        77 => MACINTOSH,
        128 => SHIFT_JIS,
        129 => EUC_KR,
        134 => GBK,
        136 => BIG5,
        161 => WINDOWS_1253,
        162 => WINDOWS_1254,
        163 => WINDOWS_1258,
        177 => WINDOWS_1255,
        178 => WINDOWS_1256,
        186 => WINDOWS_1257,
        204 => WINDOWS_1251,
        222 => WINDOWS_874,
        238 => WINDOWS_1250,
        _ => return None,
    })
}

fn code_page(value: i32) -> &'static Encoding {
    match value {
        874 => WINDOWS_874,
        932 => SHIFT_JIS,
        936 => GBK,
        949 => EUC_KR,
        950 => BIG5,
        1250 => WINDOWS_1250,
        1251 => WINDOWS_1251,
        1253 => WINDOWS_1253,
        1254 => WINDOWS_1254,
        1255 => WINDOWS_1255,
        1256 => WINDOWS_1256,
        1257 => WINDOWS_1257,
        1258 => WINDOWS_1258,
        10000 => MACINTOSH,
        65001 => UTF_8,
        _ => WINDOWS_1252,
    }
}

fn hyperlink(instruction: &str) -> Option<String> {
    let rest = instruction.trim_start().strip_prefix("HYPERLINK")?;
    let mut parts = rest.split('"');
    let before = parts.next()?;
    if before.contains("\\l") { return None; }
    let target = parts.next()?.trim();
    let lower = target.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")).then(|| target.to_string())
}

impl Parser {
    fn state(&self) -> &State { self.stack.last().expect("tokens are only handled inside a group") }
    fn state_mut(&mut self) -> &mut State { self.stack.last_mut().expect("tokens are only handled inside a group") }

    fn encoding(&self) -> &'static Encoding {
        let font = self.state().font.or(self.default_font);
        font.and_then(|font| self.fonts.get(&font)).and_then(|font| font.encoding).unwrap_or(self.code_page)
    }

    fn byte(&mut self, byte: u8) {
        if self.skip > 0 { self.skip -= 1; return; }
        match self.state().destination {
            Destination::FontTable => {
                if byte == b';' { self.finish_font(); } else if let Some(entry) = &mut self.font_entry { entry.1.push(byte); }
            }
            Destination::ColorTable => {
                if byte == b';' {
                    let [red, green, blue] = self.color_entry;
                    let color = (red.is_some() || green.is_some() || blue.is_some()).then(|| [red.unwrap_or(0), green.unwrap_or(0), blue.unwrap_or(0)]);
                    self.colors.push(color);
                    self.color_entry = [None; 3];
                }
            }
            Destination::Picture => {
                let Some(digit) = (byte as char).to_digit(16) else { return };
                match self.picture.nibble.take() {
                    Some(high) => self.picture.data.push(high << 4 | digit as u8),
                    None => self.picture.nibble = Some(digit as u8),
                }
            }
            Destination::Body | Destination::Title | Destination::Author | Destination::Instruction => self.pending.push(byte),
            Destination::Info | Destination::Skip => {}
        }
    }

    fn flush(&mut self) {
        if self.pending.is_empty() { return; }
        let bytes = std::mem::take(&mut self.pending);
        let font = self.state().font.or(self.default_font).and_then(|font| self.fonts.get(&font));
        let text = if font.is_some_and(|font| font.symbol) {
            bytes.iter().map(|byte| match byte { 0xB7 => '\u{2022}', 0xA7 => '\u{25AA}', other => WINDOWS_1252.decode_without_bom_handling(&[*other]).0.chars().next().unwrap_or(' ') }).collect::<String>()
        } else {
            self.encoding().decode_without_bom_handling(&bytes).0.into_owned()
        };
        self.text(&text);
    }

    fn text(&mut self, text: &str) {
        match self.state().destination {
            Destination::Body => self.body_text(text),
            Destination::Title => self.title.push_str(text),
            Destination::Author => self.author.push_str(text),
            Destination::Instruction => self.instruction.push_str(text),
            _ => {}
        }
    }

    fn family(&mut self, font: Option<i32>) -> u16 {
        let Some(font) = font.and_then(|font| self.fonts.get(&font)).filter(|font| !font.symbol && !font.name.is_empty()) else { return 0 };
        let name = font.name.clone();
        if let Some(index) = self.family_index.get(&name) { return *index; }
        self.families.push(name.clone());
        let index = self.families.len() as u16;
        self.family_index.insert(name, index);
        index
    }

    fn style(&mut self) -> Style {
        let state = self.state().clone();
        let family = self.family(state.font.or(self.default_font));
        Style {
            bold: state.bold,
            italic: state.italic,
            underline: state.underline,
            strike: state.strike,
            family,
            half_points: state.half_points,
            color: self.colors.get(state.color).copied().flatten(),
            code: false,
        }
    }

    fn close_table_unless_in_table(&mut self) {
        if self.state().in_table { return; }
        let Some(mut table) = self.table.take() else { return };
        if !table.cell.is_empty() { table.row.push(std::mem::take(&mut table.cell)); }
        if !table.row.is_empty() { table.rows.push(std::mem::take(&mut table.row)); }
        if !table.rows.is_empty() { self.blocks.push(Block::Table { aligns: table.aligns, head: Vec::new(), rows: table.rows }); }
    }

    fn body_text(&mut self, text: &str) {
        if text.is_empty() || self.state().hidden || self.state().deleted { return; }
        self.close_table_unless_in_table();
        let style = self.style();
        let link = self.state().link.clone();
        let spans = if self.state().in_table { &mut self.table.get_or_insert_with(Table::default).cell } else { &mut self.spans };
        match spans.last_mut() {
            Some(last) if last.style == style && last.link == link => last.text.push_str(text),
            _ => spans.push(Span { text: text.to_string(), style, link }),
        }
    }

    fn end_paragraph(&mut self) {
        self.close_table_unless_in_table();
        if self.state().in_table { self.body_text("\n"); return; }
        let state = self.state().clone();
        let spans = std::mem::take(&mut self.spans);
        let size = spans.iter().map(|span| span.style.half_points).filter(|half| *half > 0).min().unwrap_or(state.half_points.max(1)) as f32 / 2.;
        let leading = match (state.line_spacing, state.line_multiple) {
            (0, _) => 1.2,
            (spacing, true) => 1.2 * spacing.abs() as f32 / 240.,
            (spacing, false) if spacing > 0 => (twips(spacing) / size).max(1.2),
            (spacing, false) => twips(-spacing) / size,
        }.clamp(0.9, 4.);
        self.blocks.push(Block::Formatted(Paragraph {
            spans,
            align: state.align,
            indent: twips(state.left_indent.max(0)),
            right_indent: twips(state.right_indent.max(0)),
            before: twips(state.space_before.max(0)),
            after: twips(state.space_after.max(0)),
            size,
            leading,
        }));
    }

    fn finish_font(&mut self) {
        let Some((id, name, encoding, symbol)) = self.font_entry.take() else { return };
        let name = encoding.unwrap_or(self.code_page).decode_without_bom_handling(&name).0.trim().to_string();
        self.fonts.entry(id).or_insert(Font { name, encoding, symbol });
    }

    fn finish_picture(&mut self, closed: &State) {
        let picture = std::mem::take(&mut self.picture);
        if closed.hidden || closed.deleted { return; }
        let Some(true) = picture.kind else { self.omitted_pictures += 1; return };
        if closed.in_table { self.omitted_pictures += 1; return; }
        let scale = |value: i32| if value > 0 { value as f32 / 100. } else { 1. };
        let size = (picture.goal.0 > 0 && picture.goal.1 > 0).then(|| (twips(picture.goal.0) * scale(picture.scale.0), twips(picture.goal.1) * scale(picture.scale.1)));
        match Picture::from_bytes(&picture.data, size) {
            Ok(decoded) => {
                self.close_table_unless_in_table();
                if self.spans.iter().any(|span| !span.text.trim().is_empty()) { self.end_paragraph(); } else { self.spans.clear(); }
                self.pictures.push(decoded);
                self.blocks.push(Block::Image(self.pictures.len() - 1));
            }
            Err(_) => self.omitted_pictures += 1,
        }
    }

    fn set_destination(&mut self, destination: Destination) {
        self.state_mut().destination = destination;
        if destination == Destination::Picture { self.picture = PictureData::default(); }
    }

    fn symbol(&mut self, symbol: u8) {
        match symbol {
            b'\\' | b'{' | b'}' => self.byte(symbol),
            b'*' => self.star = true,
            _ if self.skip > 0 => self.skip -= 1,
            b'~' => self.text("\u{a0}"),
            b'_' => self.text("-"),
            _ => {}
        }
    }

    fn word(&mut self, word: &str, parameter: Option<i32>) {
        let star = std::mem::take(&mut self.star);
        let destination = self.state().destination;
        if destination == Destination::Skip { return; }
        if self.skip > 0 {
            if matches!(word, "par" | "sect" | "page" | "cell" | "row" | "u") { self.skip = 0; } else { self.skip -= 1; return; }
        }
        let value = parameter.unwrap_or(0);
        if star {
            match word {
                "fldinst" => { self.instruction.clear(); self.set_destination(Destination::Instruction); }
                "shppict" => {}
                _ => self.set_destination(Destination::Skip),
            }
            return;
        }
        if destination == Destination::Picture {
            match word {
                "pngblip" | "jpegblip" => self.picture.kind = Some(true),
                "wmetafile" | "emfblip" | "macpict" | "dibitmap" | "wbitmap" | "pmmetafile" => self.picture.kind = Some(false),
                "picwgoal" => self.picture.goal.0 = value,
                "pichgoal" => self.picture.goal.1 = value,
                "picscalex" => self.picture.scale.0 = value,
                "picscaley" => self.picture.scale.1 = value,
                _ => {}
            }
            return;
        }
        match word {
            "fonttbl" => self.set_destination(Destination::FontTable),
            "colortbl" => self.set_destination(Destination::ColorTable),
            "info" => self.set_destination(Destination::Info),
            "title" if destination == Destination::Info => self.set_destination(Destination::Title),
            "author" if destination == Destination::Info => self.set_destination(Destination::Author),
            "pict" => self.set_destination(Destination::Picture),
            "field" => self.instruction.clear(),
            "fldinst" => { self.instruction.clear(); self.set_destination(Destination::Instruction); }
            "fldrslt" => {
                let link = hyperlink(&self.instruction);
                let state = self.state_mut();
                state.destination = Destination::Body;
                state.link = link;
            }
            "stylesheet" | "header" | "headerl" | "headerr" | "headerf" | "footer" | "footerl" | "footerr" | "footerf" | "footnote" | "annotation"
            | "pn" | "xe" | "tc" | "txe" | "rxe" | "private" | "template" | "filetbl" | "listtable" | "listoverridetable" | "revtbl" | "rsidtbl"
            | "objdata" | "nonshppict" | "atnid" | "atnauthor" | "atnicn" | "atnref" | "atntime" | "atndate" | "comment" | "operator" | "keywords"
            | "subject" | "doccomm" | "company" | "category" | "manager" | "hlinkbase" | "creatim" | "revtim" | "printim" | "buptim" => {
                self.set_destination(Destination::Skip)
            }
            _ if destination == Destination::Info => {}
            "f" if destination == Destination::FontTable => { self.finish_font(); self.font_entry = Some((value, Vec::new(), None, false)); }
            "fcharset" if destination == Destination::FontTable => {
                if let Some(entry) = &mut self.font_entry {
                    entry.2 = charset(value);
                    entry.3 = value == 2;
                }
            }
            "red" if destination == Destination::ColorTable => self.color_entry[0] = Some(value.clamp(0, 255) as u8),
            "green" if destination == Destination::ColorTable => self.color_entry[1] = Some(value.clamp(0, 255) as u8),
            "blue" if destination == Destination::ColorTable => self.color_entry[2] = Some(value.clamp(0, 255) as u8),
            _ if matches!(destination, Destination::FontTable | Destination::ColorTable) => {}
            "ansicpg" => self.code_page = code_page(value),
            "mac" => self.code_page = MACINTOSH,
            "deff" => self.default_font = Some(value),
            "paperw" => self.page.width = Some(twips(value)),
            "paperh" => self.page.height = Some(twips(value)),
            "margl" => self.page.left = Some(twips(value)),
            "margr" => self.page.right = Some(twips(value)),
            "margt" => self.page.top = Some(twips(value)),
            "margb" => self.page.bottom = Some(twips(value)),
            "pgwsxn" => { self.section_page.width.get_or_insert(twips(value)); }
            "pghsxn" => { self.section_page.height.get_or_insert(twips(value)); }
            "marglsxn" => { self.section_page.left.get_or_insert(twips(value)); }
            "margrsxn" => { self.section_page.right.get_or_insert(twips(value)); }
            "margtsxn" => { self.section_page.top.get_or_insert(twips(value)); }
            "margbsxn" => { self.section_page.bottom.get_or_insert(twips(value)); }
            "uc" => self.state_mut().skip_after_unicode = value.max(0) as usize,
            "u" if parameter.is_some() => {
                let code = if value < 0 { value + 65536 } else { value } as u32;
                self.unicode(code);
                self.skip = self.state().skip_after_unicode;
            }
            "plain" => self.state_mut().reset_character(),
            "pard" => self.state_mut().reset_paragraph(),
            "f" => self.state_mut().font = Some(value),
            "fs" => self.state_mut().half_points = if value > 0 { value.min(3200) as u16 } else { DEFAULT_HALF_POINTS },
            "b" => self.state_mut().bold = toggle(parameter),
            "i" => self.state_mut().italic = toggle(parameter),
            "ul" | "uld" | "uldash" | "uldashd" | "uldashdd" | "uldb" | "ulth" | "ulw" | "ulwave" | "ulhwave" | "ululdbwave" => self.state_mut().underline = toggle(parameter),
            "ulnone" => self.state_mut().underline = false,
            "strike" | "striked" => self.state_mut().strike = toggle(parameter),
            "v" => self.state_mut().hidden = toggle(parameter),
            "deleted" => self.state_mut().deleted = toggle(parameter),
            "cf" => self.state_mut().color = value.max(0) as usize,
            "ql" | "qj" | "qd" => self.state_mut().align = Align::Left,
            "qc" => self.state_mut().align = Align::Center,
            "qr" => self.state_mut().align = Align::Right,
            "li" => self.state_mut().left_indent = value,
            "ri" => self.state_mut().right_indent = value,
            "sb" => self.state_mut().space_before = value,
            "sa" => self.state_mut().space_after = value,
            "sl" => self.state_mut().line_spacing = value,
            "slmult" => self.state_mut().line_multiple = value == 1,
            "intbl" => self.state_mut().in_table = true,
            "par" | "sect" => self.end_paragraph(),
            "page" => {
                if !self.state().in_table {
                    if !self.spans.is_empty() { self.end_paragraph(); }
                    self.close_table_unless_in_table();
                    self.blocks.push(Block::PageBreak);
                }
            }
            "line" => self.text("\n"),
            "tab" => self.text("\t"),
            "emdash" => self.text("\u{2014}"),
            "endash" => self.text("\u{2013}"),
            "emspace" | "enspace" | "qmspace" => self.text(" "),
            "bullet" => self.text("\u{2022}"),
            "lquote" => self.text("\u{2018}"),
            "rquote" => self.text("\u{2019}"),
            "ldblquote" => self.text("\u{201C}"),
            "rdblquote" => self.text("\u{201D}"),
            "cell" | "nestcell" => self.end_cell(word == "nestcell"),
            "row" | "nestrow" => self.end_row(word == "nestrow"),
            _ => {}
        }
    }

    fn unicode(&mut self, code: u32) {
        if (0xD800..0xDC00).contains(&code) {
            self.surrogate = Some(code);
            return;
        }
        let character = match (self.surrogate.take(), code) {
            (Some(high), low) if (0xDC00..0xE000).contains(&low) => char::from_u32(0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)),
            (_, code) => char::from_u32(code),
        };
        if let Some(character) = character { self.text(character.encode_utf8(&mut [0; 4])); }
    }

    fn end_cell(&mut self, nested: bool) {
        if !self.state().in_table { return; }
        if nested { self.body_text(" "); return; }
        let align = self.state().align;
        let table = self.table.get_or_insert_with(Table::default);
        while table.cell.last().is_some_and(|span| span.text.ends_with('\n')) {
            let last = table.cell.last_mut().expect("checked above");
            last.text.pop();
            if last.text.is_empty() { table.cell.pop(); }
        }
        let column = table.row.len();
        if table.rows.is_empty() && table.aligns.len() == column { table.aligns.push(align); }
        let cell = std::mem::take(&mut table.cell);
        table.row.push(cell);
    }

    fn end_row(&mut self, nested: bool) {
        if nested { if self.state().in_table { self.body_text("\n"); } return; }
        let Some(table) = &mut self.table else { return };
        if !table.cell.is_empty() { table.row.push(std::mem::take(&mut table.cell)); }
        let row = std::mem::take(&mut table.row);
        if !row.is_empty() { table.rows.push(row); }
    }

    fn finish(mut self) -> Parsed {
        self.stack.push(State::new());
        if self.spans.iter().any(|span| !span.text.trim().is_empty()) { self.end_paragraph(); }
        self.close_table_unless_in_table();
        let pick = |section: Option<f32>, document: Option<f32>| section.or(document);
        let page = PageSetup {
            width: pick(self.section_page.width, self.page.width),
            height: pick(self.section_page.height, self.page.height),
            left: pick(self.section_page.left, self.page.left),
            right: pick(self.section_page.right, self.page.right),
            top: pick(self.section_page.top, self.page.top),
            bottom: pick(self.section_page.bottom, self.page.bottom),
        };
        let body_family = self.default_font.and_then(|font| self.fonts.get(&font)).filter(|font| !font.symbol && !font.name.is_empty()).map(|font| font.name.clone());
        let clean = |text: String| { let text = text.split_whitespace().collect::<Vec<_>>().join(" "); (!text.is_empty()).then_some(text) };
        Parsed {
            blocks: self.blocks,
            pictures: self.pictures,
            families: self.families,
            body_family,
            title: clean(self.title),
            author: clean(self.author),
            page,
            omitted_pictures: self.omitted_pictures,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(blocks: &[Block]) -> Vec<String> {
        blocks.iter().filter_map(|block| match block {
            Block::Formatted(paragraph) => Some(paragraph.spans.iter().map(|span| span.text.as_str()).collect()),
            _ => None,
        }).collect()
    }

    #[test]
    fn unicode_code_pages_and_escapes_decode_to_the_written_text() {
        let parsed = parse(br"{\rtf1\ansi\ansicpg1251{\fonttbl{\f0 Arial;}{\f1\fcharset238 Arial CE;}}\uc2 \u-3913\'3f\'3f\u-10179??\u-8704??\u8364\'80 {\uc0 \u955}\'c4\'e0\par
{\f1 \'b9\'f3d\'9f}\par
\{braces\} and \\ back\~slash\emdash done\par}").unwrap();
        assert_eq!(text(&parsed.blocks), ["\u{f0b7}\u{1f600}\u{20ac}\u{3bb}\u{414}\u{430}", "\u{105}\u{f3}d\u{17a}", "{braces} and \\ back\u{a0}slash\u{2014}done"]);
        assert_eq!(parsed.families, ["Arial CE"]);
    }

    #[test]
    fn page_setup_prefers_the_first_section_and_missing_braces_are_damage() {
        let parsed = parse(br"{\rtf1\paperw11906\paperh16838\margl1000\sectd\pgwsxn16838\pghsxn11906\marglsxn720 Text\par}").unwrap();
        assert_eq!(parsed.page, PageSetup { width: Some(841.9), height: Some(595.3), left: Some(36.), right: None, top: None, bottom: None });
        for damaged in [&br"{\rtf1 open"[..], br"plain", br"{\rtf1 \'zz}", br"{\rtf1 \bin99 ab}"] {
            assert_eq!(parse(damaged).err().map(|error| error.message), Some(DAMAGED.to_string()), "{}", String::from_utf8_lossy(damaged));
        }
    }
}
