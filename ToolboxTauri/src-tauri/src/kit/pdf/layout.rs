use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};

use crate::kit::contracts::ToolError;
use crate::kit::pdf::font_resolver::{FontReport, FontResolver, FontStyle, SystemFontLookup};
use crate::kit::pdf::unicode_font::UnicodeFont;

const MAX_PICTURE_SIDE: u32 = 20_000;
const MAX_PICTURE_PIXELS: u64 = 40_000_000;
const BLACK: [f32; 3] = [0., 0., 0.];
const QUOTE_INK: [f32; 3] = [0.3, 0.3, 0.3];
const LINK_INK: [f32; 3] = [0.05, 0.33, 0.75];
const SHADE: [f32; 3] = [0.94, 0.94, 0.94];
const RULE: [f32; 3] = [0.7, 0.7, 0.7];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Face { index: usize, oblique: bool, heavy: bool }

pub struct FontBook<'r, S: SystemFontLookup> {
    resolver: &'r FontResolver<S>,
    requests: BTreeMap<(String, FontStyle), Face>,
    files: BTreeMap<(String, FontStyle), usize>,
    fonts: Vec<UnicodeFont>,
    reports: Vec<FontReport>,
    char_widths: HashMap<(usize, char), f32>,
}

impl<'r, S: SystemFontLookup> FontBook<'r, S> {
    pub fn new(resolver: &'r FontResolver<S>) -> Self {
        Self { resolver, requests: BTreeMap::new(), files: BTreeMap::new(), fonts: Vec::new(), reports: Vec::new(), char_widths: HashMap::new() }
    }

    pub fn face(&mut self, family: &str, style: FontStyle) -> Result<Face, ToolError> {
        let key = (family.to_string(), style);
        if let Some(face) = self.requests.get(&key) { return Ok(*face); }
        let resolved = self.resolver.resolve(family, style, &[])?;
        let report = resolved.report;
        let file = (format!("{:?}:{}", report.source, report.font_used), report.style_used);
        let index = match self.files.get(&file) {
            Some(index) => *index,
            None => {
                self.fonts.push(UnicodeFont::parse(resolved.data)?);
                self.files.insert(file, self.fonts.len() - 1);
                self.fonts.len() - 1
            }
        };
        let bold = |style: FontStyle| matches!(style, FontStyle::Bold | FontStyle::BoldItalic);
        let italic = |style: FontStyle| matches!(style, FontStyle::Italic | FontStyle::BoldItalic);
        let face = Face { index, oblique: italic(style) && !italic(report.style_used), heavy: bold(style) && !bold(report.style_used) };
        self.reports.push(report);
        self.requests.insert(key, face);
        Ok(face)
    }

    pub fn reports(&self) -> Vec<FontReport> { self.reports.clone() }

    pub fn width(&mut self, face: Face, text: &str, size: f32) -> Result<f32, ToolError> {
        Ok(self.fonts[face.index].show(text, size)?.width)
    }

    pub fn char_width(&mut self, face: Face, character: char, size: f32) -> Result<f32, ToolError> {
        if let Some(width) = self.char_widths.get(&(face.index, character)) { return Ok(width * size); }
        let width = self.fonts[face.index].show(character.encode_utf8(&mut [0; 4]), 1.)?.width;
        self.char_widths.insert((face.index, character), width);
        Ok(width * size)
    }

    pub fn text(&mut self, face: Face, size: f32, x: f32, y: f32, text: &str) -> Result<String, ToolError> {
        let run = self.fonts[face.index].show(text, size)?;
        let skew = if face.oblique { 0.2 } else { 0. };
        let (heavy, light) = if face.heavy { (format!("2 Tr {:.2} w ", size * 0.03), " 0 Tr") } else { (String::new(), "") };
        Ok(format!("BT /F{} {size} Tf {heavy}1 0 {skew} 1 {x:.2} {y:.2} Tm {}{light} ET\n", face.index, run.operators))
    }
}

pub struct Picture { rgb: Vec<u8>, alpha: Option<Vec<u8>>, width: u32, height: u32 }

impl Picture {
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut reader = image::ImageReader::open(path).map_err(|error| error.to_string())?.with_guessed_format().map_err(|error| error.to_string())?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_PICTURE_SIDE);
        limits.max_image_height = Some(MAX_PICTURE_SIDE);
        reader.limits(limits);
        let image = reader.decode().map_err(|error| error.to_string())?;
        if image.width() == 0 || image.height() == 0 || image.width() as u64 * image.height() as u64 > MAX_PICTURE_PIXELS {
            return Err("the image is too large".into());
        }
        let image = image.to_rgba8();
        let mut rgb = Vec::with_capacity(image.len() / 4 * 3);
        let mut alpha = Vec::with_capacity(image.len() / 4);
        for pixel in image.pixels() {
            rgb.extend_from_slice(&pixel.0[..3]);
            alpha.push(pixel.0[3]);
        }
        let alpha = alpha.iter().any(|value| *value < 255).then_some(alpha);
        Ok(Self { rgb, alpha, width: image.width(), height: image.height() })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style { pub bold: bool, pub italic: bool, pub code: bool, pub strike: bool }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span { pub text: String, pub style: Style, pub link: Option<String> }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align { Left, Center, Right }

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading { level: u8, spans: Vec<Span>, anchor: String },
    Paragraph(Vec<Span>),
    Code(String),
    Quote(Vec<Block>),
    List { start: Option<u64>, items: Vec<Vec<Block>> },
    Table { aligns: Vec<Align>, head: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>> },
    Image(usize),
    Rule,
}

pub fn plain_text(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, PartialEq)]
pub enum Target { Uri(String), Anchor(String), Page { page: usize, y: f32 } }

#[derive(Debug, Clone, PartialEq)]
pub struct Link { pub rect: [f32; 4], pub target: Target }

#[derive(Debug, Default)]
pub struct PageOut { pub content: String, pub links: Vec<Link> }

#[derive(Debug, Clone, PartialEq)]
pub struct Bookmark { pub level: u8, pub title: String, pub page: usize, pub y: f32 }

#[derive(Debug, Clone, Copy)]
pub struct Geometry { pub width: f32, pub height: f32, pub margin: f32 }

impl Geometry {
    fn content_height(&self) -> f32 { self.height - 2. * self.margin }
    fn top(&self) -> f32 { self.height - self.margin }
}

#[derive(Debug, Clone)]
pub struct Theme { pub body_family: String, pub code_family: String, pub body_size: f32, pub shaded_code: bool }

#[derive(Clone)]
struct Piece { text: String, face: Face, size: f32, width: f32, style: Style, link: Option<String>, space: bool }

#[derive(Default)]
struct Line { pieces: Vec<Piece>, width: f32, size: f32 }

impl Line {
    fn new(size: f32) -> Self { Self { pieces: Vec::new(), width: 0., size } }
    fn height(&self) -> f32 { self.size * 1.45 }
    fn trim(&mut self) {
        while self.pieces.last().is_some_and(|piece| piece.space) {
            let piece = self.pieces.pop().expect("checked above");
            self.width -= piece.width;
        }
    }
}

pub struct Laid { pub pages: Vec<PageOut>, pub bookmarks: Vec<Bookmark>, pub anchors: BTreeMap<String, (usize, f32)> }

pub struct Flow<'b, 'r, S: SystemFontLookup> {
    book: &'b mut FontBook<'r, S>,
    theme: Theme,
    pictures: &'b [Picture],
    geometry: Geometry,
    pages: Vec<PageOut>,
    cursor: f32,
    ink: [f32; 3],
    depth: usize,
    bookmarks: Vec<Bookmark>,
    anchors: BTreeMap<String, (usize, f32)>,
}

fn tokens(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut previous: Option<u8> = None;
    for (index, character) in text.char_indices() {
        let kind = if character == '\n' { 2 } else if character.is_whitespace() { 1 } else { 0 };
        if previous.is_some_and(|previous| previous != kind || kind == 2) {
            tokens.push(&text[start..index]);
            start = index;
        }
        previous = Some(kind);
    }
    if start < text.len() { tokens.push(&text[start..]); }
    tokens
}

fn color(ink: [f32; 3]) -> String {
    let [r, g, b] = ink;
    format!("{r} {g} {b} rg {r} {g} {b} RG\n")
}

impl<'b, 'r, S: SystemFontLookup> Flow<'b, 'r, S> {
    pub fn new(book: &'b mut FontBook<'r, S>, theme: Theme, pictures: &'b [Picture], geometry: Geometry) -> Self {
        Self { book, theme, pictures, geometry, pages: vec![PageOut::default()], cursor: 0., ink: BLACK, depth: 0, bookmarks: Vec::new(), anchors: BTreeMap::new() }
    }

    pub fn finish(self) -> Laid { Laid { pages: self.pages, bookmarks: self.bookmarks, anchors: self.anchors } }

    fn left(&self) -> f32 { self.geometry.margin }
    fn text_width(&self) -> f32 { self.geometry.width - 2. * self.geometry.margin }
    fn y(&self) -> f32 { self.geometry.top() - self.cursor }
    fn page(&mut self) -> &mut PageOut { self.pages.last_mut().expect("a flow always has a page") }
    fn page_index(&self) -> usize { self.pages.len() - 1 }

    fn need(&mut self, height: f32) -> bool {
        if self.cursor > 0. && self.cursor + height > self.geometry.content_height() {
            self.pages.push(PageOut::default());
            self.cursor = 0.;
            return true;
        }
        false
    }

    fn gap(&mut self, height: f32) { if self.cursor > 0. { self.cursor += height; } }

    fn rect(&mut self, ink: [f32; 3], x: f32, y: f32, width: f32, height: f32) {
        let [r, g, b] = ink;
        self.page().content.push_str(&format!("q {r} {g} {b} rg {x:.2} {y:.2} {width:.2} {height:.2} re f Q\n"));
    }

    fn stroke(&mut self, ink: [f32; 3], line_width: f32, from: (f32, f32), to: (f32, f32)) {
        let [r, g, b] = ink;
        self.page().content.push_str(&format!("q {r} {g} {b} RG {line_width} w {:.2} {:.2} m {:.2} {:.2} l S Q\n", from.0, from.1, to.0, to.1));
    }

    fn face_for(&mut self, style: Style, size: f32, bold: bool) -> Result<(Face, f32), ToolError> {
        if style.code {
            let family = self.theme.code_family.clone();
            return Ok((self.book.face(&family, FontStyle::new(style.bold || bold, false))?, size * 0.9));
        }
        let family = self.theme.body_family.clone();
        Ok((self.book.face(&family, FontStyle::new(style.bold || bold, style.italic))?, size))
    }

    fn lines(&mut self, spans: &[Span], width: f32, size: f32, bold: bool) -> Result<Vec<Line>, ToolError> {
        let mut lines = Vec::new();
        let mut line = Line::new(size);
        for span in spans {
            let (face, piece_size) = self.face_for(span.style, size, bold)?;
            for token in tokens(&span.text) {
                let piece = |text: &str, width: f32, space: bool| Piece { text: text.to_string(), face, size: piece_size, width, style: span.style, link: span.link.clone(), space };
                if token == "\n" {
                    line.trim();
                    lines.push(std::mem::replace(&mut line, Line::new(size)));
                    continue;
                }
                if token.chars().all(char::is_whitespace) {
                    if line.pieces.is_empty() { continue; }
                    let width = self.book.width(face, " ", piece_size)?;
                    line.width += width;
                    line.pieces.push(piece(" ", width, true));
                    continue;
                }
                let word_width = self.book.width(face, token, piece_size)?;
                if line.width + word_width > width && line.pieces.iter().any(|piece| !piece.space) {
                    line.trim();
                    lines.push(std::mem::replace(&mut line, Line::new(size)));
                }
                if word_width <= width {
                    line.width += word_width;
                    line.pieces.push(piece(token, word_width, false));
                    continue;
                }
                let mut chunk = String::new();
                let mut chunk_width = 0.;
                for character in token.chars() {
                    let character_width = self.book.char_width(face, character, piece_size)?;
                    if line.width + chunk_width + character_width > width && !(chunk.is_empty() && line.pieces.is_empty()) {
                        if !chunk.is_empty() { line.pieces.push(piece(&chunk, chunk_width, false)); }
                        line.width += chunk_width;
                        line.trim();
                        lines.push(std::mem::replace(&mut line, Line::new(size)));
                        chunk.clear();
                        chunk_width = 0.;
                    }
                    chunk.push(character);
                    chunk_width += character_width;
                }
                if !chunk.is_empty() {
                    line.width += chunk_width;
                    line.pieces.push(piece(&chunk, chunk_width, false));
                }
            }
        }
        line.trim();
        if !line.pieces.is_empty() || lines.is_empty() { lines.push(line); }
        Ok(lines)
    }

    fn draw_line(&mut self, line: &Line, x: f32, width: f32, align: Align) -> Result<(), ToolError> {
        let baseline = self.y() - line.size * 1.1;
        let mut pen = x + match align { Align::Left => 0., Align::Center => (width - line.width) / 2., Align::Right => width - line.width };
        let mut index = 0;
        while index < line.pieces.len() {
            let first = &line.pieces[index];
            let mut text = first.text.clone();
            let mut run_width = first.width;
            let mut next = index + 1;
            while let Some(piece) = line.pieces.get(next) {
                if piece.face != first.face || piece.size != first.size || piece.style != first.style || piece.link != first.link { break; }
                text.push_str(&piece.text);
                run_width += piece.width;
                next += 1;
            }
            let (face, size, style, link) = (first.face, first.size, first.style, first.link.clone());
            if style.code && self.theme.shaded_code {
                self.rect(SHADE, pen - 1., baseline - size * 0.28, run_width + 2., size * 1.2);
            }
            let ink = if link.is_some() { LINK_INK } else { self.ink };
            let operators = self.book.text(face, size, pen, baseline, &text)?;
            let page = self.page();
            page.content.push_str(&color(ink));
            page.content.push_str(&operators);
            if style.strike { self.stroke(ink, size * 0.06, (pen, baseline + size * 0.28), (pen + run_width, baseline + size * 0.28)); }
            if let Some(link) = link {
                let target = match link.strip_prefix('#') { Some(anchor) => Target::Anchor(anchor.to_string()), None => Target::Uri(link) };
                self.page().links.push(Link { rect: [pen, baseline - size * 0.25, pen + run_width, baseline + size * 0.9], target });
            }
            pen += run_width;
            index = next;
        }
        self.cursor += line.height();
        Ok(())
    }

    fn paragraph(&mut self, spans: &[Span], x: f32, width: f32, size: f32, bold: bool, align: Align) -> Result<(), ToolError> {
        for line in self.lines(spans, width, size, bold)? {
            self.need(line.height());
            self.draw_line(&line, x, width, align)?;
        }
        Ok(())
    }

    pub fn blocks(&mut self, blocks: &[Block]) -> Result<(), ToolError> {
        let (x, width) = (self.left(), self.text_width());
        self.block_list(blocks, x, width)
    }

    fn block_list(&mut self, blocks: &[Block], x: f32, width: f32) -> Result<(), ToolError> {
        for block in blocks { self.block(block, x, width)?; }
        Ok(())
    }

    fn block(&mut self, block: &Block, x: f32, width: f32) -> Result<(), ToolError> {
        let size = self.theme.body_size;
        match block {
            Block::Heading { level, spans, anchor } => {
                let heading_size = size * [2., 1.6, 1.3, 1.15, 1.05, 1.][(*level as usize).clamp(1, 6) - 1];
                self.gap(heading_size * 0.7);
                let lines = self.lines(spans, width, heading_size, true)?;
                let height = lines.iter().map(Line::height).sum::<f32>();
                self.need((height + size * 3.).min(self.geometry.content_height()));
                let destination = (self.page_index(), self.y());
                self.anchors.entry(anchor.clone()).or_insert(destination);
                self.bookmarks.push(Bookmark { level: *level, title: plain_text(spans), page: destination.0, y: destination.1 });
                for line in lines {
                    self.need(line.height());
                    self.draw_line(&line, x, width, Align::Left)?;
                }
                self.gap(heading_size * 0.3);
            }
            Block::Paragraph(spans) => {
                self.paragraph(spans, x, width, size, false, Align::Left)?;
                self.gap(if self.depth > 0 { size * 0.25 } else { size * 0.6 });
            }
            Block::Code(text) => self.code(text, x, width)?,
            Block::Quote(blocks) => {
                let start = (self.page_index(), self.cursor);
                let ink = std::mem::replace(&mut self.ink, QUOTE_INK);
                self.block_list(blocks, x + 14., width - 14.)?;
                self.ink = ink;
                let end = (self.page_index(), self.cursor);
                let top = self.geometry.top();
                for page in start.0..=end.0 {
                    let from = if page == start.0 { start.1 } else { 0. };
                    let to = if page == end.0 { end.1 - size * 0.6 } else { self.geometry.content_height() };
                    if to <= from { continue; }
                    let [r, g, b] = RULE;
                    self.pages[page].content.push_str(&format!("q {r} {g} {b} RG 2 w {:.2} {:.2} m {:.2} {:.2} l S Q\n", x + 3., top - from, x + 3., top - to));
                }
            }
            Block::List { start, items } => {
                self.depth += 1;
                let (face, _) = self.face_for(Style::default(), size, false)?;
                for (index, item) in items.iter().enumerate() {
                    let marker = match start {
                        Some(first) => format!("{}.", first + index as u64),
                        None => if self.depth % 2 == 1 { "\u{2022}".to_string() } else { "\u{2013}".to_string() },
                    };
                    let marker_width = self.book.width(face, &marker, size)?;
                    let indent = (marker_width + 8.).max(18.);
                    self.need(size * 1.45);
                    let baseline = self.y() - size * 1.1;
                    let operators = self.book.text(face, size, x + indent - 6. - marker_width, baseline, &marker)?;
                    let ink = self.ink;
                    let page = self.page();
                    page.content.push_str(&color(ink));
                    page.content.push_str(&operators);
                    let before = self.cursor;
                    self.block_list(item, x + indent, width - indent)?;
                    if self.cursor == before && self.pages.len() > 0 { self.cursor += size * 1.45; }
                }
                self.depth -= 1;
                if self.depth == 0 { self.gap(size * 0.35); }
            }
            Block::Table { aligns, head, rows } => self.table(aligns, head, rows, x, width)?,
            Block::Image(index) => {
                let picture = &self.pictures[*index];
                let natural = (picture.width as f32 * 0.75, picture.height as f32 * 0.75);
                let scale = (width / natural.0).min(self.geometry.content_height() / natural.1).min(1.);
                let (image_width, image_height) = (natural.0 * scale, natural.1 * scale);
                self.need(image_height);
                let bottom = self.y() - image_height;
                self.page().content.push_str(&format!("q {image_width:.2} 0 0 {image_height:.2} {x:.2} {bottom:.2} cm /Im{index} Do Q\n"));
                self.cursor += image_height;
                self.gap(size * 0.6);
            }
            Block::Rule => {
                self.need(size);
                let y = self.y() - size * 0.5;
                self.stroke(RULE, 0.75, (x, y), (x + width, y));
                self.cursor += size;
            }
        }
        Ok(())
    }

    fn code(&mut self, text: &str, x: f32, width: f32) -> Result<(), ToolError> {
        let size = self.theme.body_size * 0.85;
        let row_height = size * 1.4;
        let pad = 6.;
        let face = self.book.face(&self.theme.code_family.clone(), FontStyle::Regular)?;
        let mut rows = Vec::new();
        for line in text.trim_end_matches('\n').split('\n') {
            let line = expand_tabs(line);
            let mut row = String::new();
            let mut row_width = 0.;
            for character in line.chars() {
                let character_width = self.book.char_width(face, character, size)?;
                if row_width + character_width > width - 2. * pad && !row.is_empty() {
                    rows.push(std::mem::take(&mut row));
                    row_width = 0.;
                }
                row.push(character);
                row_width += character_width;
            }
            rows.push(row);
        }
        let shaded = self.theme.shaded_code;
        self.need(row_height + pad);
        if shaded { let y = self.y(); self.rect(SHADE, x, y - pad, width, pad); }
        self.cursor += pad;
        for row in rows {
            if self.need(row_height) && shaded { let y = self.y(); self.rect(SHADE, x, y - pad / 2., width, pad / 2.); self.cursor += pad / 2.; }
            let top = self.y();
            if shaded { self.rect(SHADE, x, top - row_height, width, row_height); }
            if !row.trim().is_empty() {
                let operators = self.book.text(face, size, x + pad, top - size * 1.05, &row)?;
                let ink = self.ink;
                let page = self.page();
                page.content.push_str(&color(ink));
                page.content.push_str(&operators);
            }
            self.cursor += row_height;
        }
        if shaded { let y = self.y(); self.rect(SHADE, x, y - pad, width, pad); }
        self.cursor += pad;
        self.gap(self.theme.body_size * 0.6);
        Ok(())
    }

    fn table(&mut self, aligns: &[Align], head: &[Vec<Span>], rows: &[Vec<Vec<Span>>], x: f32, width: f32) -> Result<(), ToolError> {
        let size = self.theme.body_size * 0.95;
        let pad = 4.;
        let columns = rows.iter().map(Vec::len).chain([head.len(), aligns.len()]).max().unwrap_or(0);
        if columns == 0 { return Ok(()); }
        let mut natural = vec![0f32; columns];
        let mut minimum = vec![0f32; columns];
        for (row, bold) in std::iter::once((head, true)).chain(rows.iter().map(|row| (row.as_slice(), false))) {
            for (column, cell) in row.iter().enumerate() {
                for line in self.lines(cell, f32::INFINITY, size, bold)? {
                    natural[column] = natural[column].max(line.width);
                    for piece in &line.pieces { minimum[column] = minimum[column].max(piece.width); }
                }
            }
        }
        let wanted = natural.iter().map(|value| value + 2. * pad).collect::<Vec<_>>();
        let total = wanted.iter().sum::<f32>();
        let mut widths = if total <= width { wanted } else {
            let floors = minimum.iter().map(|value| (value + 2. * pad).min(width / columns as f32)).collect::<Vec<_>>();
            wanted.iter().zip(&floors).map(|(want, floor)| (width * want / total).max(*floor)).collect::<Vec<_>>()
        };
        let sum = widths.iter().sum::<f32>();
        if sum > width { for value in &mut widths { *value *= width / sum; } }
        let align = |column: usize| aligns.get(column).copied().unwrap_or(Align::Left);

        let has_head = head.iter().any(|cell| !cell.is_empty());
        if has_head { self.table_row(head, &widths, x, size, pad, true, &align)?; }
        for row in rows {
            let height = self.row_height(row, &widths, size, pad, false)?;
            if self.need(height) && has_head { self.table_row(head, &widths, x, size, pad, true, &align)?; }
            self.table_row(row, &widths, x, size, pad, false, &align)?;
        }
        self.gap(self.theme.body_size * 0.6);
        Ok(())
    }

    fn row_height(&mut self, row: &[Vec<Span>], widths: &[f32], size: f32, pad: f32, bold: bool) -> Result<f32, ToolError> {
        let mut height = size * 1.45;
        for (column, cell) in row.iter().enumerate() {
            let lines = self.lines(cell, widths[column] - 2. * pad, size, bold)?;
            height = height.max(lines.iter().map(Line::height).sum());
        }
        Ok(height + 2. * pad)
    }

    #[allow(clippy::too_many_arguments)]
    fn table_row(&mut self, row: &[Vec<Span>], widths: &[f32], x: f32, size: f32, pad: f32, bold: bool, align: &dyn Fn(usize) -> Align) -> Result<(), ToolError> {
        let height = self.row_height(row, widths, size, pad, bold)?;
        self.need(height);
        let top = self.y();
        let start = self.cursor;
        let mut left = x;
        for (column, cell_width) in widths.iter().enumerate() {
            if bold { self.rect(SHADE, left, top - height, *cell_width, height); }
            let [r, g, b] = RULE;
            self.page().content.push_str(&format!("q {r} {g} {b} RG 0.5 w {left:.2} {:.2} {cell_width:.2} {height:.2} re S Q\n", top - height));
            if let Some(cell) = row.get(column) {
                self.cursor = start + pad;
                for line in self.lines(cell, cell_width - 2. * pad, size, bold)? {
                    self.draw_line(&line, left + pad, cell_width - 2. * pad, align(column))?;
                }
            }
            left += cell_width;
        }
        self.cursor = start + height;
        Ok(())
    }

    pub fn contents(&mut self, title: &str, entries: &[Bookmark], offset: usize) -> Result<(), ToolError> {
        let size = self.theme.body_size;
        self.paragraph(&[Span { text: title.to_string(), style: Style::default(), link: None }], self.left(), self.text_width(), size * 1.6, true, Align::Left)?;
        self.gap(size * 0.6);
        let (face, _) = self.face_for(Style::default(), size, false)?;
        let number_column = 36.;
        for entry in entries {
            let indent = (entry.level.saturating_sub(1) as f32) * 14.;
            let x = self.left() + indent;
            let width = self.text_width() - indent - number_column;
            let target = Target::Page { page: entry.page + offset, y: entry.y };
            let spans = [Span { text: entry.title.clone(), style: Style::default(), link: None }];
            let lines = self.lines(&spans, width, size, entry.level == 1)?;
            let count = lines.len();
            for (index, line) in lines.into_iter().enumerate() {
                self.need(line.height());
                let top = self.y();
                self.draw_line(&line, x, width, Align::Left)?;
                let mut right = x + line.width;
                if index + 1 == count {
                    let number = (entry.page + offset + 1).to_string();
                    let number_width = self.book.width(face, &number, size)?;
                    let number_x = self.left() + self.text_width() - number_width;
                    let operators = self.book.text(face, size, number_x, top - size * 1.1, &number)?;
                    let ink = self.ink;
                    let page = self.page();
                    page.content.push_str(&color(ink));
                    page.content.push_str(&operators);
                    right = number_x + number_width;
                }
                self.page().links.push(Link { rect: [x, top - line.height(), right, top], target: target.clone() });
            }
        }
        Ok(())
    }
}

pub fn expand_tabs(line: &str) -> String {
    const TAB_WIDTH: usize = 8;
    let mut expanded = String::with_capacity(line.len());
    let mut column = 0;
    for character in line.chars() {
        if character == '\t' {
            let spaces = TAB_WIDTH - column % TAB_WIDTH;
            expanded.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            expanded.push(character);
            column += 1;
        }
    }
    expanded
}

pub fn text_string(text: &str) -> Object {
    if text.is_ascii() { return Object::string_literal(text); }
    let mut bytes = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() { bytes.extend_from_slice(&unit.to_be_bytes()); }
    Object::String(bytes, StringFormat::Hexadecimal)
}

pub struct Assembly<'a> {
    pub size: (f32, f32),
    pub pages: Vec<PageOut>,
    pub pictures: &'a [Picture],
    pub bookmarks: &'a [Bookmark],
    pub anchors: &'a BTreeMap<String, (usize, f32)>,
    pub title: Option<&'a str>,
}

pub fn assemble<S: SystemFontLookup>(book: &FontBook<'_, S>, assembly: Assembly<'_>) -> Result<Document, ToolError> {
    let (width, height) = assembly.size;
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let page_ids = (0..assembly.pages.len()).map(|_| document.new_object_id()).collect::<Vec<_>>();
    let destination = |page: usize, y: f32| Object::Array(vec![page_ids[page].into(), "XYZ".into(), Object::Null, Object::Real(y), Object::Null]);

    let mut fonts = Dictionary::new();
    for (index, font) in book.fonts.iter().enumerate() { fonts.set(format!("F{index}"), font.embed(&mut document)?); }
    let mut resources = dictionary! { "Font" => fonts };
    if !assembly.pictures.is_empty() {
        let mut images = Dictionary::new();
        for (index, picture) in assembly.pictures.iter().enumerate() {
            let image = |color_space: &str, data: Vec<u8>| Stream::new(dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => picture.width as i64, "Height" => picture.height as i64,
                "ColorSpace" => color_space, "BitsPerComponent" => 8,
            }, data);
            let mut stream = image("DeviceRGB", picture.rgb.clone());
            if let Some(alpha) = &picture.alpha { stream.dict.set("SMask", document.add_object(image("DeviceGray", alpha.clone()))); }
            images.set(format!("Im{index}"), document.add_object(stream));
        }
        resources.set("XObject", images);
    }
    let resources_id = document.add_object(resources);

    for (index, page) in assembly.pages.into_iter().enumerate() {
        let content_id = document.add_object(Stream::new(dictionary! {}, page.content.into_bytes()));
        let mut annotations = Vec::new();
        for link in page.links {
            let mut annotation = dictionary! {
                "Type" => "Annot", "Subtype" => "Link",
                "Rect" => link.rect.iter().map(|value| Object::Real(*value)).collect::<Vec<_>>(),
                "Border" => vec![0.into(), 0.into(), 0.into()],
            };
            match link.target {
                Target::Uri(uri) => annotation.set("A", dictionary! { "S" => "URI", "URI" => Object::string_literal(uri) }),
                Target::Page { page, y } => annotation.set("Dest", destination(page, y)),
                Target::Anchor(anchor) => match assembly.anchors.get(&anchor) {
                    Some((page, y)) => annotation.set("Dest", destination(*page, *y)),
                    None => continue,
                },
            }
            annotations.push(Object::Reference(document.add_object(annotation)));
        }
        let mut page = dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(width), Object::Real(height)],
            "Resources" => resources_id, "Contents" => content_id,
        };
        if !annotations.is_empty() { page.set("Annots", annotations); }
        document.objects.insert(page_ids[index], page.into());
    }
    let page_count = page_ids.len() as i64;
    document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => page_ids.iter().map(|id| Object::Reference(*id)).collect::<Vec<_>>(), "Count" => page_count }.into());
    let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
    if !assembly.bookmarks.is_empty() {
        catalog.set("Outlines", outlines(&mut document, assembly.bookmarks, &page_ids));
        catalog.set("PageMode", "UseOutlines");
    }
    let catalog_id = document.add_object(catalog);
    document.trailer.set("Root", catalog_id);
    if let Some(title) = assembly.title.filter(|title| !title.trim().is_empty()) {
        let info = document.add_object(dictionary! { "Title" => text_string(title.trim()) });
        document.trailer.set("Info", info);
    }
    document.compress();
    Ok(document)
}

fn outlines(document: &mut Document, bookmarks: &[Bookmark], page_ids: &[ObjectId]) -> ObjectId {
    let root = document.new_object_id();
    let ids = bookmarks.iter().map(|_| document.new_object_id()).collect::<Vec<_>>();
    let mut parents = vec![None; bookmarks.len()];
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); bookmarks.len()];
    let mut top = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for (index, bookmark) in bookmarks.iter().enumerate() {
        while stack.last().is_some_and(|open| bookmarks[*open].level >= bookmark.level) { stack.pop(); }
        match stack.last() {
            Some(parent) => { parents[index] = Some(*parent); children[*parent].push(index); }
            None => top.push(index),
        }
        stack.push(index);
    }
    fn descendants(index: usize, children: &[Vec<usize>]) -> i64 {
        children[index].iter().map(|child| 1 + descendants(*child, children)).sum()
    }
    let siblings = |index: usize| -> &Vec<usize> { match parents[index] { Some(parent) => &children[parent], None => &top } };
    for (index, bookmark) in bookmarks.iter().enumerate() {
        let family = siblings(index);
        let position = family.iter().position(|sibling| *sibling == index).expect("a bookmark is among its siblings");
        let mut item = dictionary! {
            "Title" => text_string(&bookmark.title),
            "Parent" => parents[index].map(|parent| ids[parent]).unwrap_or(root),
            "Dest" => vec![page_ids[bookmark.page].into(), "XYZ".into(), Object::Null, Object::Real(bookmark.y), Object::Null],
        };
        if position > 0 { item.set("Prev", ids[family[position - 1]]); }
        if let Some(next) = family.get(position + 1) { item.set("Next", ids[*next]); }
        if let (Some(first), Some(last)) = (children[index].first(), children[index].last()) {
            item.set("First", ids[*first]);
            item.set("Last", ids[*last]);
            item.set("Count", descendants(index, &children));
        }
        document.objects.insert(ids[index], item.into());
    }
    let total = bookmarks.len() as i64;
    document.objects.insert(root, dictionary! {
        "Type" => "Outlines", "First" => ids[top[0]], "Last" => ids[*top.last().expect("bookmarks are not empty")], "Count" => total,
    }.into());
    root
}
