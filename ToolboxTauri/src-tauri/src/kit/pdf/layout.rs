use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};

use std::fmt::Write as _;

use unicode_bidi::ParagraphBidiInfo;
use unicode_script::{Script, UnicodeScript};

use crate::kit::contracts::ToolError;
use crate::kit::pdf::font_resolver::{EmbeddedFont, FontReport, FontResolver, FontStyle, SystemFontLookup};
use crate::kit::pdf::script_text::{character_pack_script, pack_font};
use crate::kit::pdf::unicode_font::UnicodeFont;
use crate::packs::uninstall::RunPacks;

const MAX_PICTURE_SIDE: u32 = 20_000;
const MAX_PICTURE_PIXELS: u64 = 40_000_000;
const BLACK: [f32; 3] = [0., 0., 0.];
const QUOTE_INK: [f32; 3] = [0.3, 0.3, 0.3];
const LINK_INK: [f32; 3] = [0.05, 0.33, 0.75];
const SHADE: [f32; 3] = [0.94, 0.94, 0.94];
const RULE: [f32; 3] = [0.7, 0.7, 0.7];
const TABLE_PAD: f32 = 4.;
pub const PAGE_NUMBER: char = '\u{E000}';
pub const PAGE_COUNT: char = '\u{E001}';

/// A font to write with: `oblique` and `heavy` slant and thicken a font that lacks the asked-for
/// italic or bold, and `italic` and `bold` say what was asked for, for text in a script pack's font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Face { index: usize, oblique: bool, heavy: bool, italic: bool, bold: bool }

/// The fonts a document is written with. Latin, Cyrillic and Greek are written with the font each
/// face resolves to. Text in any other script is written with the font the script pack has for
/// it, read the first time the script appears; without the pack, laying the text out fails with a
/// pointer to the pack, before anything is saved.
pub struct FontBook<'r, S: SystemFontLookup> {
    resolver: &'r FontResolver<S>,
    packs: &'r RunPacks,
    embedded: Vec<EmbeddedFont>,
    requests: BTreeMap<(String, FontStyle), Face>,
    files: BTreeMap<(String, FontStyle), usize>,
    fonts: Vec<UnicodeFont>,
    reports: Vec<FontReport>,
    char_widths: HashMap<(usize, char), f32>,
    /// Each pack script's font in `fonts`, by script name.
    scripts: BTreeMap<&'static str, usize>,
    /// Each pack font file's index in `fonts`, by pack and path.
    pack_files: BTreeMap<(String, String), usize>,
}

/// Whether `text` has a character in a script the app's own fonts don't write.
fn needs_pack(text: &str) -> bool { text.chars().any(|character| character_pack_script(character).is_some()) }

impl<'r, S: SystemFontLookup> FontBook<'r, S> {
    pub fn new(resolver: &'r FontResolver<S>, packs: &'r RunPacks) -> Self { Self::with_embedded(resolver, packs, Vec::new()) }

    pub fn with_embedded(resolver: &'r FontResolver<S>, packs: &'r RunPacks, embedded: Vec<EmbeddedFont>) -> Self {
        Self { resolver, packs, embedded, requests: BTreeMap::new(), files: BTreeMap::new(), fonts: Vec::new(), reports: Vec::new(), char_widths: HashMap::new(), scripts: BTreeMap::new(), pack_files: BTreeMap::new() }
    }

    pub fn face(&mut self, family: &str, style: FontStyle) -> Result<Face, ToolError> {
        let key = (family.to_string(), style);
        if let Some(face) = self.requests.get(&key) { return Ok(*face); }
        let resolved = self.resolver.resolve(family, style, &self.embedded)?;
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
        let face = Face { index, oblique: italic(style) && !italic(report.style_used), heavy: bold(style) && !bold(report.style_used), italic: italic(style), bold: bold(style) };
        self.reports.push(report);
        self.requests.insert(key, face);
        Ok(face)
    }

    pub fn reports(&self) -> Vec<FontReport> { self.reports.clone() }

    /// The script pack's font for `script`, read from the pack the first time.
    fn script_font(&mut self, script: &'static str) -> Result<usize, ToolError> {
        if let Some(index) = self.scripts.get(script) { return Ok(*index); }
        let font = pack_font(script, self.packs)?;
        let index = match self.pack_files.get(&font.file) {
            Some(index) => *index,
            None => {
                self.fonts.push(UnicodeFont::parse(font.bytes)?);
                self.pack_files.insert(font.file, self.fonts.len() - 1);
                self.fonts.len() - 1
            }
        };
        self.scripts.insert(script, index);
        Ok(index)
    }

    /// `text` in runs of one font each, in logical order, with each run's byte range. Characters
    /// shared between scripts, such as spaces and digits, follow the character before them when
    /// its font has them.
    fn runs(&mut self, face: Face, text: &str) -> Result<Vec<(usize, std::ops::Range<usize>)>, ToolError> {
        if !needs_pack(text) { return Ok(vec![(face.index, 0..text.len())]); }
        let mut strong = Vec::new();
        for (offset, character) in text.char_indices() {
            let font = match (character_pack_script(character), character.script()) {
                (Some(script), _) => Some(self.script_font(script)?),
                (None, Script::Common | Script::Inherited) => None,
                (None, _) => Some(face.index),
            };
            strong.push((offset, character, font));
        }
        let mut runs: Vec<(usize, std::ops::Range<usize>)> = Vec::new();
        for (position, (offset, character, font)) in strong.iter().enumerate() {
            let previous = runs.last().map(|(font, _)| *font);
            let has = |font: usize| self.fonts[font].has_glyph(*character);
            let font = font.unwrap_or_else(|| match previous {
                Some(previous) if character.script() == Script::Inherited || has(previous) => previous,
                _ if has(face.index) => face.index,
                _ => strong[position + 1..].iter().find_map(|(_, _, font)| *font).filter(|font| has(*font)).unwrap_or(face.index),
            });
            let end = offset + character.len_utf8();
            match runs.last_mut() {
                Some((last, range)) if *last == font => range.end = end,
                _ => runs.push((font, *offset..end)),
            }
        }
        Ok(runs)
    }

    pub fn width(&mut self, face: Face, text: &str, size: f32) -> Result<f32, ToolError> {
        let mut width = 0.;
        for (font, range) in self.runs(face, text)? { width += self.fonts[font].show(&text[range], size)?.width; }
        Ok(width)
    }

    pub fn char_width(&mut self, face: Face, character: char, size: f32) -> Result<f32, ToolError> {
        let text = character.to_string();
        let font = self.runs(face, &text)?.first().map_or(face.index, |(font, _)| *font);
        if let Some(width) = self.char_widths.get(&(font, character)) { return Ok(width * size); }
        let width = self.fonts[font].show(&text, 1.)?.width;
        self.char_widths.insert((font, character), width);
        Ok(width * size)
    }

    pub fn text(&mut self, face: Face, size: f32, x: f32, y: f32, text: &str) -> Result<String, ToolError> {
        let style = |oblique: bool, heavy: bool| {
            let skew = if oblique { 0.2 } else { 0. };
            let (heavy, light) = if heavy { (format!("2 Tr {:.2} w ", size * 0.03), " 0 Tr") } else { (String::new(), "") };
            (skew, heavy, light)
        };
        if !needs_pack(text) {
            let run = self.fonts[face.index].show(text, size)?;
            let (skew, heavy, light) = style(face.oblique, face.heavy);
            return Ok(format!("BT /F{} {size} Tf {heavy}1 0 {skew} 1 {x:.2} {y:.2} Tm {}{light} ET\n", face.index, run.operators));
        }
        // Each bidi run in visual order, and the font runs inside it in visual order too.
        let runs = self.runs(face, text)?;
        let font_at = |offset: usize| runs.iter().find(|(_, range)| range.contains(&offset)).map_or(face.index, |(font, _)| *font);
        let bidi = ParagraphBidiInfo::new(text, None);
        let (levels, visual) = bidi.visual_runs(0..text.len());
        let mut content = String::from("BT ");
        let mut pen = x;
        for line_run in visual {
            let rtl = levels[line_run.start].is_rtl();
            let mut pieces: Vec<(usize, std::ops::Range<usize>)> = Vec::new();
            for (offset, character) in text[line_run.clone()].char_indices() {
                let (offset, font) = (line_run.start + offset, font_at(line_run.start + offset));
                let end = offset + character.len_utf8();
                match pieces.last_mut() {
                    Some((last, range)) if *last == font => range.end = end,
                    _ => pieces.push((font, offset..end)),
                }
            }
            if rtl { pieces.reverse(); }
            for (font, range) in pieces {
                let run = self.fonts[font].show_directed(&text[range], size, Some(rtl))?;
                // A pack font has one style, so it is slanted and thickened for italic and bold.
                let (skew, heavy, light) = if font == face.index { style(face.oblique, face.heavy) } else { style(face.italic, face.bold) };
                let _ = write!(content, "/F{font} {size} Tf {heavy}1 0 {skew} 1 {pen:.2} {y:.2} Tm {}{light} ", run.operators);
                pen += run.width;
            }
        }
        content.push_str("ET\n");
        Ok(content)
    }
}

pub struct Picture { rgb: Vec<u8>, alpha: Option<Vec<u8>>, width: u32, height: u32, size: Option<(f32, f32)> }

impl Picture {
    pub fn load(path: &Path) -> Result<Self, String> {
        let reader = image::ImageReader::open(path).map_err(|error| error.to_string())?;
        Self::decode(reader, None)
    }

    pub fn from_bytes(bytes: &[u8], size: Option<(f32, f32)>) -> Result<Self, String> {
        Self::decode(image::ImageReader::new(std::io::Cursor::new(bytes)), size.filter(|(width, height)| *width > 0. && *height > 0.))
    }

    fn decode<R: std::io::BufRead + std::io::Seek>(reader: image::ImageReader<R>, size: Option<(f32, f32)>) -> Result<Self, String> {
        let mut reader = reader.with_guessed_format().map_err(|error| error.to_string())?;
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
        Ok(Self { rgb, alpha, width: image.width(), height: image.height(), size })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub underline: bool,
    pub family: u16,
    pub half_points: u16,
    pub color: Option<[u8; 3]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span { pub text: String, pub style: Style, pub link: Option<String> }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Align { #[default] Left, Center, Right }

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
    Formatted(Paragraph),
    PageBreak,
    Float(Float),
}

/// What a floating object is placed against: the page edge, the page margins, or the surrounding text (column and paragraph).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame { Page, Margin, Text }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place { Offset(f32), Start, Center, End }

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position { pub frame: Frame, pub place: Place }

/// An object anchored at a position instead of flowing with the text. Text that follows is pushed below it when `wrap` is set.
#[derive(Debug, Clone, PartialEq)]
pub struct Float {
    pub x: Position,
    pub y: Position,
    pub width: f32,
    pub height: f32,
    pub fill: Option<[u8; 3]>,
    pub line: Option<[u8; 3]>,
    pub picture: Option<usize>,
    pub blocks: Vec<Block>,
    pub wrap: bool,
}

/// Content left out because nothing renderable is stored for it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Omitted { pub pictures: usize, pub charts: usize, pub diagrams: usize, pub objects: usize }

#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph { pub spans: Vec<Span>, pub align: Align, pub indent: f32, pub right_indent: f32, pub before: f32, pub after: f32, pub size: f32, pub leading: f32 }

pub fn plain_text(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, PartialEq)]
pub enum Target { Uri(String), Anchor(String), Page { page: usize, y: f32 } }

#[derive(Debug, Clone, PartialEq)]
pub struct Link { pub rect: [f32; 4], pub target: Target }

/// A laid-out page; `size` overrides the document's page size for this page.
#[derive(Debug, Default)]
pub struct PageOut { pub content: String, pub links: Vec<Link>, pub size: Option<(f32, f32)> }

#[derive(Debug, Clone, PartialEq)]
pub struct Bookmark { pub level: u8, pub title: String, pub page: usize, pub y: f32 }

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct PageSetup { pub width: Option<f32>, pub height: Option<f32>, pub left: Option<f32>, pub right: Option<f32>, pub top: Option<f32>, pub bottom: Option<f32> }

#[derive(Debug, Clone, Copy)]
pub struct Geometry { pub width: f32, pub height: f32, pub left: f32, pub right: f32, pub top: f32, pub bottom: f32 }

impl Geometry {
    pub fn uniform(width: f32, height: f32, margin: f32) -> Self { Self { width, height, left: margin, right: margin, top: margin, bottom: margin } }
    fn content_height(&self) -> f32 { self.height - self.top - self.bottom }
    fn top(&self) -> f32 { self.height - self.top }
}

#[derive(Debug, Clone)]
pub struct Theme { pub body_family: String, pub code_family: String, pub body_size: f32, pub shaded_code: bool, pub families: Vec<String>, pub leading: f32 }

impl Theme {
    pub fn new(body_family: &str, code_family: &str, body_size: f32, shaded_code: bool) -> Self {
        Self { body_family: body_family.to_string(), code_family: code_family.to_string(), body_size, shaded_code, families: Vec::new(), leading: 1.45 }
    }
}

#[derive(Clone)]
struct Piece { text: String, face: Face, size: f32, width: f32, style: Style, link: Option<String>, space: bool }

struct Line { pieces: Vec<Piece>, width: f32, size: f32, leading: f32 }

impl Line {
    fn new(size: f32, leading: f32) -> Self { Self { pieces: Vec::new(), width: 0., size, leading } }
    fn height(&self) -> f32 { self.size * self.leading }
    fn push(&mut self, piece: Piece) {
        self.width += piece.width;
        if !piece.space { self.size = self.size.max(piece.size); }
        self.pieces.push(piece);
    }
    fn trim(&mut self) {
        while self.pieces.last().is_some_and(|piece| piece.space) {
            let piece = self.pieces.pop().expect("checked above");
            self.width -= piece.width;
        }
    }
}

/// `floats` lists the page index of every floating object placed, in placement order.
pub struct Laid { pub pages: Vec<PageOut>, pub bookmarks: Vec<Bookmark>, pub anchors: BTreeMap<String, (usize, f32)>, pub floats: Vec<usize> }

pub struct Flow<'b, 'r, S: SystemFontLookup> {
    book: &'b mut FontBook<'r, S>,
    theme: Theme,
    pictures: &'b [Picture],
    geometry: Geometry,
    pages: Vec<PageOut>,
    cursor: f32,
    ink: [f32; 3],
    depth: usize,
    leading: f32,
    bookmarks: Vec<Bookmark>,
    anchors: BTreeMap<String, (usize, f32)>,
    floats: Vec<usize>,
    keep_out: Vec<(usize, f32, f32)>,
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
        let leading = theme.leading;
        Self { book, theme, pictures, geometry, pages: vec![PageOut::default()], cursor: 0., ink: BLACK, depth: 0, leading, bookmarks: Vec::new(), anchors: BTreeMap::new(), floats: Vec::new(), keep_out: Vec::new() }
    }

    pub fn finish(self) -> Laid { Laid { pages: self.pages, bookmarks: self.bookmarks, anchors: self.anchors, floats: self.floats } }

    pub fn bands(&mut self, header: &[Block], footer: &[Block], header_top: f32, footer_bottom: f32) -> Result<(), ToolError> {
        let total = self.pages.len();
        for index in 0..total {
            let numbers = [(PAGE_NUMBER, (index + 1).to_string()), (PAGE_COUNT, total.to_string())];
            if !header.is_empty() {
                let (band, _, floats) = self.band(&numbered(header, &numbers), header_top)?;
                self.overlay(index, band, floats);
            }
            if !footer.is_empty() {
                let footer = numbered(footer, &numbers);
                let (_, height, _) = self.band(&footer, 0.)?;
                let (band, _, floats) = self.band(&footer, (self.geometry.height - footer_bottom - height).max(0.))?;
                self.overlay(index, band, floats);
            }
        }
        Ok(())
    }

    /// The height a header or footer band of `blocks` takes, without drawing it.
    pub fn band_height(&mut self, blocks: &[Block]) -> Result<f32, ToolError> { self.band(blocks, 0.).map(|(_, height, _)| height) }

    fn band(&mut self, blocks: &[Block], top: f32) -> Result<(PageOut, f32, usize), ToolError> {
        let geometry = Geometry { top, bottom: 0., ..self.geometry };
        let (x, width) = (geometry.left, geometry.width - geometry.left - geometry.right);
        self.apart(geometry, |flow| flow.block_list(blocks, x, width))
    }

    /// Lays `blocks` out on a scratch page with `geometry`, returning that page, the height used and how many floats it placed.
    fn apart(&mut self, geometry: Geometry, draw: impl FnOnce(&mut Self) -> Result<(), ToolError>) -> Result<(PageOut, f32, usize), ToolError> {
        let pages = std::mem::replace(&mut self.pages, vec![PageOut::default()]);
        let geometry = std::mem::replace(&mut self.geometry, geometry);
        let cursor = std::mem::replace(&mut self.cursor, 0.);
        let floats = std::mem::take(&mut self.floats);
        let keep_out = std::mem::take(&mut self.keep_out);
        let (bookmarks, anchors) = (self.bookmarks.len(), self.anchors.clone());
        let drawn = draw(self);
        let height = self.cursor;
        let page = std::mem::replace(&mut self.pages, pages).into_iter().next().unwrap_or_default();
        let placed = std::mem::replace(&mut self.floats, floats).len();
        self.geometry = geometry;
        self.cursor = cursor;
        self.keep_out = keep_out;
        self.bookmarks.truncate(bookmarks);
        self.anchors = anchors;
        drawn?;
        Ok((page, height, placed))
    }

    fn overlay(&mut self, index: usize, band: PageOut, floats: usize) {
        let page = &mut self.pages[index];
        page.content.push_str(&band.content);
        page.links.extend(band.links);
        self.floats.extend(std::iter::repeat_n(index, floats));
    }

    fn left(&self) -> f32 { self.geometry.left }
    fn text_width(&self) -> f32 { self.geometry.width - self.geometry.left - self.geometry.right }
    fn y(&self) -> f32 { self.geometry.top() - self.cursor }
    fn page(&mut self) -> &mut PageOut { self.pages.last_mut().expect("a flow always has a page") }
    fn page_index(&self) -> usize { self.pages.len() - 1 }

    /// The page and height the next block starts at, for links that point at a place between blocks.
    pub fn position(&self) -> (usize, f32) { (self.page_index(), self.y()) }

    fn need(&mut self, height: f32) -> bool {
        let broke = self.cursor > 0. && self.cursor + height > self.geometry.content_height();
        if broke {
            self.pages.push(PageOut::default());
            self.cursor = 0.;
        }
        let page = self.page_index();
        let below = self.keep_out.iter()
            .filter(|(on, top, bottom)| *on == page && *top < self.cursor + height && *bottom > self.cursor)
            .map(|(_, _, bottom)| *bottom)
            .fold(None, |lowest: Option<f32>, bottom| Some(lowest.map_or(bottom, |lowest| lowest.max(bottom))));
        match below {
            Some(bottom) => {
                self.cursor = bottom;
                self.need(height) || broke
            }
            None => broke,
        }
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
        let family = match style.family {
            0 => self.theme.body_family.clone(),
            index => self.theme.families.get(index as usize - 1).cloned().unwrap_or_else(|| self.theme.body_family.clone()),
        };
        let size = if style.half_points > 0 { style.half_points as f32 / 2. } else { size };
        Ok((self.book.face(&family, FontStyle::new(style.bold || bold, style.italic))?, size))
    }

    fn lines(&mut self, spans: &[Span], width: f32, size: f32, bold: bool) -> Result<Vec<Line>, ToolError> {
        let leading = self.leading;
        let mut lines = Vec::new();
        let mut line = Line::new(size, leading);
        for span in spans {
            let (face, piece_size) = self.face_for(span.style, size, bold)?;
            for token in tokens(&span.text) {
                let piece = |text: &str, width: f32, space: bool| Piece { text: text.to_string(), face, size: piece_size, width, style: span.style, link: span.link.clone(), space };
                if token == "\n" {
                    line.trim();
                    lines.push(std::mem::replace(&mut line, Line::new(size, leading)));
                    continue;
                }
                if token.chars().all(char::is_whitespace) {
                    if line.pieces.is_empty() { continue; }
                    let width = self.book.width(face, " ", piece_size)?;
                    line.push(piece(" ", width, true));
                    continue;
                }
                let word_width = self.book.width(face, token, piece_size)?;
                if line.width + word_width > width && line.pieces.iter().any(|piece| !piece.space) {
                    line.trim();
                    lines.push(std::mem::replace(&mut line, Line::new(size, leading)));
                }
                if word_width <= width {
                    line.push(piece(token, word_width, false));
                    continue;
                }
                let mut chunk = String::new();
                let mut chunk_width = 0.;
                for character in token.chars() {
                    let character_width = self.book.char_width(face, character, piece_size)?;
                    if line.width + chunk_width + character_width > width && !(chunk.is_empty() && line.pieces.is_empty()) {
                        if !chunk.is_empty() { line.push(piece(&chunk, chunk_width, false)); }
                        line.trim();
                        lines.push(std::mem::replace(&mut line, Line::new(size, leading)));
                        chunk.clear();
                        chunk_width = 0.;
                    }
                    chunk.push(character);
                    chunk_width += character_width;
                }
                if !chunk.is_empty() { line.push(piece(&chunk, chunk_width, false)); }
            }
        }
        line.trim();
        if !line.pieces.is_empty() || lines.is_empty() { lines.push(line); }
        Ok(lines)
    }

    fn draw_line(&mut self, line: &Line, x: f32, width: f32, align: Align) -> Result<(), ToolError> {
        let baseline = self.y() - line.size * (line.leading - 0.35);
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
            let ink = match (&link, style.color) {
                (Some(_), _) => LINK_INK,
                (None, Some(rgb)) => rgb.map(|channel| channel as f32 / 255.),
                (None, None) => self.ink,
            };
            let operators = self.book.text(face, size, pen, baseline, &text)?;
            let page = self.page();
            page.content.push_str(&color(ink));
            page.content.push_str(&operators);
            if style.strike { self.stroke(ink, size * 0.06, (pen, baseline + size * 0.28), (pen + run_width, baseline + size * 0.28)); }
            if style.underline { self.stroke(ink, size * 0.05, (pen, baseline - size * 0.12), (pen + run_width, baseline - size * 0.12)); }
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
                let natural = picture.size.unwrap_or((picture.width as f32 * 0.75, picture.height as f32 * 0.75));
                let scale = (width / natural.0).min(self.geometry.content_height() / natural.1).min(1.);
                let (image_width, image_height) = (natural.0 * scale, natural.1 * scale);
                self.need(image_height);
                let bottom = self.y() - image_height;
                self.page().content.push_str(&format!("q {image_width:.2} 0 0 {image_height:.2} {x:.2} {bottom:.2} cm /Im{index} Do Q\n"));
                self.cursor += image_height;
                self.gap(size * 0.6);
            }
            Block::Formatted(paragraph) => {
                self.gap(paragraph.before);
                let leading = std::mem::replace(&mut self.leading, paragraph.leading);
                let x = x + paragraph.indent;
                let width = (width - paragraph.indent - paragraph.right_indent).max(width * 0.25);
                let drawn = if paragraph.spans.iter().all(|span| span.text.trim().is_empty()) {
                    let height = paragraph.size * paragraph.leading;
                    self.need(height);
                    self.cursor += height;
                    Ok(())
                } else {
                    self.paragraph(&paragraph.spans, x, width, paragraph.size, false, paragraph.align)
                };
                self.leading = leading;
                drawn?;
                self.cursor += paragraph.after;
            }
            Block::PageBreak => {
                self.pages.push(PageOut::default());
                self.cursor = 0.;
            }
            Block::Float(float) => self.float(float, x, width)?,
            Block::Rule => {
                self.need(size);
                let y = self.y() - size * 0.5;
                self.stroke(RULE, 0.75, (x, y), (x + width, y));
                self.cursor += size;
            }
        }
        Ok(())
    }

    fn float(&mut self, float: &Float, x: f32, width: f32) -> Result<(), ToolError> {
        const INSET: f32 = 5.;
        const GAP: f32 = 4.;
        let geometry = self.geometry;
        let (box_width, box_height) = (float.width.clamp(1., geometry.width), float.height.clamp(1., geometry.height));
        let place = |position: Position, start: f32, span: f32, size: f32, limit: f32| {
            let at = match position.place {
                Place::Offset(offset) => start + offset,
                Place::Start => start,
                Place::Center => start + (span - size) / 2.,
                Place::End => start + span - size,
            };
            at.clamp(0., (limit - size).max(0.))
        };
        let (start, span) = match float.x.frame {
            Frame::Page => (0., geometry.width),
            Frame::Margin => (geometry.left, self.text_width()),
            Frame::Text => (x, width),
        };
        let left = place(float.x, start, span, box_width, geometry.width);
        let (start, span) = match float.y.frame {
            Frame::Page => (0., geometry.height),
            Frame::Margin => (geometry.top, geometry.content_height()),
            Frame::Text => (geometry.top + self.cursor, geometry.content_height() - self.cursor),
        };
        let top = place(float.y, start, span, box_height, geometry.height);
        let bottom = geometry.height - top - box_height;
        let page = self.page_index();

        let mut content = String::new();
        if let Some(fill) = float.fill {
            let [r, g, b] = fill.map(|channel| channel as f32 / 255.);
            content.push_str(&format!("q {r} {g} {b} rg {left:.2} {bottom:.2} {box_width:.2} {box_height:.2} re f Q\n"));
        }
        if let Some(index) = float.picture {
            content.push_str(&format!("q {box_width:.2} 0 0 {box_height:.2} {left:.2} {bottom:.2} cm /Im{index} Do Q\n"));
        }
        if let Some(line) = float.line {
            let [r, g, b] = line.map(|channel| channel as f32 / 255.);
            content.push_str(&format!("q {r} {g} {b} RG 0.75 w {left:.2} {bottom:.2} {box_width:.2} {box_height:.2} re S Q\n"));
        }
        self.page().content.push_str(&content);
        if !float.blocks.is_empty() {
            let inset = INSET.min(box_width / 4.);
            let inner = Geometry { left: left + inset, right: (geometry.width - left - box_width + inset).max(0.), top: top + inset, bottom: 0., ..geometry };
            let (inner_x, inner_width) = (inner.left, (box_width - 2. * inset).max(1.));
            let (text, _, _) = self.apart(inner, |flow| flow.block_list(&float.blocks, inner_x, inner_width))?;
            let page = self.page();
            page.content.push_str(&text.content);
            page.links.extend(text.links);
        }
        if float.wrap { self.keep_out.push((page, top - geometry.top - GAP, top + box_height - geometry.top + GAP)); }
        self.floats.push(page);
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

    fn column_widths(&mut self, head: &[Vec<Span>], rows: &[Vec<Vec<Span>>], columns: usize, size: f32) -> Result<(Vec<f32>, Vec<f32>), ToolError> {
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
        Ok((natural, minimum))
    }

    pub fn table_scale(&mut self, head: &[Vec<Span>], rows: &[Vec<Vec<Span>>]) -> Result<f32, ToolError> {
        let columns = rows.iter().map(Vec::len).chain([head.len()]).max().unwrap_or(0);
        let (natural, _) = self.column_widths(head, rows, columns, self.theme.body_size * 0.95)?;
        let text = natural.iter().sum::<f32>();
        let room = self.text_width() - 2. * TABLE_PAD * columns as f32;
        Ok(if text <= 0. { 1. } else { (room / text).clamp(0., 1.) })
    }

    fn table(&mut self, aligns: &[Align], head: &[Vec<Span>], rows: &[Vec<Vec<Span>>], x: f32, width: f32) -> Result<(), ToolError> {
        let size = self.theme.body_size * 0.95;
        let pad = TABLE_PAD;
        let columns = rows.iter().map(Vec::len).chain([head.len(), aligns.len()]).max().unwrap_or(0);
        if columns == 0 { return Ok(()); }
        let (natural, minimum) = self.column_widths(head, rows, columns, size)?;
        // A hair over the natural width, so the widest cell is not wrapped by rounding when the padding comes off again.
        let wanted = natural.iter().map(|value| value + 2. * pad + 0.01).collect::<Vec<_>>();
        let total = wanted.iter().sum::<f32>();
        let mut widths = if total <= width { wanted } else {
            let floors = minimum.iter().map(|value| (value + 2. * pad).min(width / columns as f32)).collect::<Vec<_>>();
            wanted.iter().zip(&floors).map(|(want, floor)| (width * want / total).max(*floor)).collect::<Vec<_>>()
        };
        let sum = widths.iter().sum::<f32>();
        if sum > width { for value in &mut widths { *value *= width / sum; } }
        let align = |column: usize| aligns.get(column).copied().unwrap_or(Align::Left);

        let has_head = head.iter().any(|cell| !cell.is_empty());
        if has_head {
            // The header starts the next page rather than sit alone at the foot of this one.
            let first = match rows.first() { Some(row) => self.row_height(row, &widths, size, pad, false)?, None => 0. };
            let head_height = self.row_height(head, &widths, size, pad, true)?;
            self.need(head_height + first);
            self.table_row(head, &widths, x, size, pad, true, &align)?;
        }
        for row in rows {
            let height = self.row_height(row, &widths, size, pad, false)?;
            if self.need(height) && has_head { self.table_row(head, &widths, x, size, pad, true, &align)?; }
            self.table_row(row, &widths, x, size, pad, false, &align)?;
        }
        self.gap(self.theme.body_size * 0.6);
        Ok(())
    }

    fn row_height(&mut self, row: &[Vec<Span>], widths: &[f32], size: f32, pad: f32, bold: bool) -> Result<f32, ToolError> {
        let mut height = size * self.leading;
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

fn numbered(blocks: &[Block], numbers: &[(char, String)]) -> Vec<Block> {
    let spans = |spans: &[Span]| spans.iter().map(|span| {
        let text = numbers.iter().fold(span.text.clone(), |text, (field, value)| text.replace(*field, value));
        Span { text, ..span.clone() }
    }).collect::<Vec<_>>();
    let rows = |rows: &[Vec<Vec<Span>>]| rows.iter().map(|row| row.iter().map(|cell| spans(cell)).collect()).collect();
    blocks.iter().map(|block| match block {
        Block::Heading { level, spans: content, anchor } => Block::Heading { level: *level, spans: spans(content), anchor: anchor.clone() },
        Block::Paragraph(content) => Block::Paragraph(spans(content)),
        Block::Formatted(paragraph) => Block::Formatted(Paragraph { spans: spans(&paragraph.spans), ..paragraph.clone() }),
        Block::Quote(inner) => Block::Quote(numbered(inner, numbers)),
        Block::List { start, items } => Block::List { start: *start, items: items.iter().map(|item| numbered(item, numbers)).collect() },
        Block::Table { aligns, head, rows: body } => Block::Table { aligns: aligns.clone(), head: head.iter().map(|cell| spans(cell)).collect(), rows: rows(body) },
        Block::Float(float) => Block::Float(Float { blocks: numbered(&float.blocks, numbers), ..float.clone() }),
        other => other.clone(),
    }).collect()
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
    pub author: Option<&'a str>,
}

pub fn assemble<S: SystemFontLookup>(book: &FontBook<'_, S>, assembly: Assembly<'_>) -> Result<Document, ToolError> {
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
        let (width, height) = page.size.unwrap_or(assembly.size);
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
    let mut info = Dictionary::new();
    if let Some(title) = assembly.title.filter(|title| !title.trim().is_empty()) { info.set("Title", text_string(title.trim())); }
    if let Some(author) = assembly.author.filter(|author| !author.trim().is_empty()) { info.set("Author", text_string(author.trim())); }
    if !info.is_empty() {
        let info = document.add_object(info);
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
