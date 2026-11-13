//! PDF to EPUB: each PDF's selectable text becomes a reflowable EPUB 3.
//!
//! Text comes from the in-process page-text reader. Runs are joined into lines by their positions.
//! Running headers, footers and page numbers are dropped unless the request keeps them; lines join
//! into paragraphs, across page breaks too, with words broken at line ends rejoined; and text set
//! larger or bolder than the body becomes headings, which split the book into chapter documents
//! and fill the table of contents. The package is built in memory with fixed timestamps and an identifier derived from the PDF's bytes,
//! so the same PDF always makes the same EPUB, then reopened and checked before it is published.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use lopdf::Document;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, System, ZipArchive, ZipWriter};

use crate::kit::common::{note_renamed, JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::{OutcomePointer, SkipCode, SkipReason, ToolError};
use super::page_text::{read_document_text, PageText, PageTextRun};
use super::remaining::{pages_without_text, PDF_DAMAGED};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfToEpubRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
    /// Leaves out running headers, footers and page numbers. On unless the request turns it off.
    #[serde(default = "on")]
    pub remove_headers_and_footers: bool,
}

fn on() -> bool { true }

pub const NO_SELECTABLE_TEXT: &str = "PDF contains no selectable text; scanned PDFs require OCR. No EPUB was made.";
pub const PASSWORD_PROTECTED: &str = "This PDF is password-protected. Remove the password, then convert again.";

const MIMETYPE: &str = "application/epub+zip";
const PACKAGE_PATH: &str = "OEBPS/content.opf";
const NAV_PATH: &str = "OEBPS/nav.xhtml";
/// Every EPUB carries this modification time, so output never depends on when it was made.
const MODIFIED: &str = "1980-01-01T00:00:00Z";
/// The text's language isn't known, so the book says so rather than guessing.
const LANGUAGE: &str = "und";
/// Lines at each page edge that can be part of a running header or footer band.
const BAND_LINES: usize = 2;
/// A heading is short: longer text in a heading style is a paragraph set large or bold.
const HEADING_MAX_LINES: usize = 3;
const HEADING_MAX_CHARS: usize = 200;
/// A chapter's text past this many characters continues in a new content document at the next paragraph.
const DOCUMENT_TEXT_LIMIT: usize = 100_000;

/// Converts one PDF and reports it as one outcome.
pub fn convert(request: &PdfToEpubRequest, input: PathBuf) -> JobOutcome {
    let bytes = match fs::read(&input) {
        Ok(bytes) => bytes,
        Err(error) => return JobOutcome::failure(input, ToolError::processing(format!("Could not read the PDF: {error}"))),
    };
    let document = match Document::load_mem(&bytes) {
        Ok(document) if document.is_encrypted() => return password_protected(input),
        Ok(document) => document,
        Err(lopdf::Error::InvalidPassword | lopdf::Error::Decryption(_)) => return password_protected(input),
        Err(_) => return JobOutcome::failure(input, ToolError::invalid_input(PDF_DAMAGED)),
    };
    let pages = match read_document_text(&document) {
        Ok(pages) => pages,
        Err(error) => return JobOutcome::failure(input, ToolError::processing(error)),
    };
    let mut lines = pages.iter().map(page_lines).collect::<Vec<_>>();
    let blank = lines.iter().enumerate().filter(|(_, lines)| lines.is_empty()).map(|(index, _)| index as u32 + 1).collect::<Vec<_>>();
    if blank.len() == lines.len() {
        return JobOutcome::skipped(input, SkipReason::new(SkipCode::NeedsOcr, NO_SELECTABLE_TEXT).with_pointer(OutcomePointer::Tool { tool_id: "pdf-ocr".into() }));
    }
    if request.remove_headers_and_footers { remove_running_text(&mut lines); }
    let title = input.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let chapters = chapters(blocks(&lines));
    let epub = match package(&title, &identifier(&bytes), &chapters).and_then(|epub| verify(&epub).map(|()| epub)) {
        Ok(epub) => epub,
        Err(error) => return JobOutcome::failure(input, ToolError::processing(format!("Could not build the EPUB: {error}"))),
    };
    let path = match save(&input, &request.output_location, &epub) {
        Ok(path) => path,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let chosen = path.with_file_name(format!("{title}.epub"));
    let detail = if pages.len() == 1 { "1 page.".to_string() } else { format!("{} pages.", pages.len()) };
    let outcome = note_renamed(JobOutcome::succeeded(input, vec![path], detail), Some(&chosen));
    if blank.is_empty() { outcome } else { outcome.with_warning(pages_without_text(blank)) }
}

fn password_protected(input: PathBuf) -> JobOutcome {
    JobOutcome::failure(input, ToolError::invalid_input(PASSWORD_PROTECTED).with_pointer(OutcomePointer::Tool { tool_id: "lock-unlock".into() }))
}

fn save(input: &Path, location: &OutputLocation, epub: &[u8]) -> Result<PathBuf, ToolError> {
    let output = OutputNaming::reserve_destination(input, location, "", "epub").map_err(|error| ToolError::processing(format!("Could not reserve the EPUB output: {error}")))?;
    fs::write(output.path(), epub).map_err(|error| ToolError::processing(format!("Could not write the EPUB: {error}")))?;
    output.publish().map_err(|error| ToolError::processing(format!("Could not save the EPUB: {error}")))
}

/// A line being assembled from runs that share a baseline band. `bold` holds when every run is bold.
#[derive(Debug, Clone)]
struct Line { text: String, top: f32, bottom: f32, left: f32, right: f32, size: f32, bold: bool }

impl Line {
    fn start(run: &PageTextRun) -> Self {
        Self { text: run.text.clone(), top: run.y, bottom: run.y + run.height, left: run.x, right: run.x + run.width, size: run.font_size, bold: run.bold }
    }

    /// A run continues this line when its middle sits inside the line's band and it doesn't jump back left.
    fn takes(&self, run: &PageTextRun) -> bool {
        let middle = run.y + run.height / 2.;
        middle >= self.top && middle <= self.bottom && run.x + run.font_size >= self.right
    }

    fn push(&mut self, run: &PageTextRun) {
        let gap = run.x - self.right;
        if gap > run.font_size * 0.15 && !self.text.ends_with(char::is_whitespace) && !run.text.starts_with(char::is_whitespace) { self.text.push(' '); }
        self.text.push_str(&run.text);
        self.top = self.top.min(run.y);
        self.bottom = self.bottom.max(run.y + run.height);
        self.left = self.left.min(run.x);
        self.right = self.right.max(run.x + run.width);
        self.size = self.size.max(run.font_size);
        self.bold &= run.bold;
    }
}

/// Groups a page's runs into lines in content order. Runs without a reliable Unicode mapping are left out.
fn page_lines(page: &PageText) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    for run in page.runs.iter().filter(|run| run.reliable_unicode && !run.text.trim().is_empty()) {
        match lines.last_mut() {
            Some(line) if line.takes(run) => line.push(run),
            _ => lines.push(Line::start(run)),
        }
    }
    lines.into_iter().filter_map(|mut line| {
        line.text = clean(&line.text);
        (!line.text.is_empty()).then_some(line)
    }).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Edge { Top, Bottom }

/// Drops running headers, footers and page numbers: lines among the two highest or lowest on a page
/// whose text, with every number read as the same number, recurs at about the same height on at
/// least two pages and at least 40% of the pages with text. Alternating left and right running
/// heads each repeat on about half the pages, so the bar sits below half. A page with a single line
/// has no band to drop, and when every line would go the text is kept, since it is the book itself.
fn remove_running_text(pages: &mut [Vec<Line>]) {
    let with_text = pages.iter().filter(|lines| !lines.is_empty()).count();
    let mut bands: BTreeMap<(Edge, String), Vec<(usize, usize)>> = BTreeMap::new();
    for (page, lines) in pages.iter().enumerate().filter(|(_, lines)| lines.len() >= 2) {
        let reach = BAND_LINES.min(lines.len() - 1);
        let mut by_height = (0..lines.len()).collect::<Vec<_>>();
        by_height.sort_by(|&a, &b| lines[a].top.total_cmp(&lines[b].top).then(a.cmp(&b)));
        let edges = by_height[..reach].iter().map(|&index| (Edge::Top, index)).chain(by_height[lines.len() - reach..].iter().map(|&index| (Edge::Bottom, index)));
        for (edge, index) in edges {
            bands.entry((edge, band_text(&lines[index].text))).or_default().push((page, index));
        }
    }
    let mut dropped = BTreeSet::new();
    for members in bands.values() {
        let mut tops = members.iter().map(|&(page, index)| pages[page][index].top).collect::<Vec<_>>();
        tops.sort_by(f32::total_cmp);
        let median = tops[tops.len() / 2];
        let aligned = members.iter().copied().filter(|&(page, index)| {
            let line = &pages[page][index];
            (line.top - median).abs() <= line.size.max(1.) * 0.5
        }).collect::<Vec<_>>();
        let repeats = aligned.iter().map(|&(page, _)| page).collect::<BTreeSet<_>>().len();
        if repeats >= 2 && repeats * 5 >= with_text * 2 { dropped.extend(aligned); }
    }
    if dropped.len() == pages.iter().map(Vec::len).sum::<usize>() { return; }
    for (page, lines) in pages.iter_mut().enumerate() {
        let mut index = 0;
        lines.retain(|_| { let keep = !dropped.contains(&(page, index)); index += 1; keep });
    }
}

/// A band line's text for comparison across pages: case folded, each run of digits read as `#`, and
/// a line that is only a small Roman numeral read as `#`, so "Page 3" matches "Page 4" and "iv" matches "v".
fn band_text(text: &str) -> String {
    let lower = text.to_lowercase();
    if !lower.is_empty() && lower.len() <= 7 && lower.chars().all(|character| "ivxlc".contains(character)) { return "#".into(); }
    let mut out = String::with_capacity(lower.len());
    for character in lower.chars() {
        if character.is_ascii_digit() {
            if !out.ends_with('#') { out.push('#'); }
        } else {
            out.push(character);
        }
    }
    out
}

/// A text style: the size in half points and whether it is bold.
type Style = (i32, bool);

fn style(line: &Line) -> Style { ((line.size * 2.).round() as i32, line.bold) }

/// Text the book reads in order: headings with their style, and paragraphs.
#[derive(Debug, Clone, PartialEq)]
enum Block { Heading(Style, String), Paragraph(String) }

/// A block being assembled from lines.
struct Draft { heading: Option<Style>, text: String, lines: usize, page: usize, top: f32, bottom: f32, left: f32, right: f32, widest: f32, size: f32 }

impl Draft {
    fn start(heading: Option<Style>, page: usize, line: &Line) -> Self {
        Self { heading, text: line.text.clone(), lines: 1, page, top: line.top, bottom: line.bottom, left: line.left, right: line.right, widest: line.right, size: line.size }
    }

    /// The next line on the same page continues this block when it sits just below the last line
    /// (a gap of at most half a line), at a similar size, and doesn't start a new paragraph after
    /// a finished sentence by being indented or by following a short last line.
    fn continues_on_page(&self, line: &Line) -> bool {
        let size = self.size.max(line.size);
        let below = line.top >= self.top && line.top - self.bottom <= size * 0.5 && (line.size - self.size).abs() <= size * 0.2;
        let new_paragraph = ends_sentence(&self.text) && (line.left > self.left + size * 0.8 || self.right < self.widest - size * 3.);
        below && !new_paragraph
    }

    /// A paragraph continues on a later page when it ended mid-word with a hyphen, or mid-sentence
    /// and the next page starts in lower case.
    fn continues_across_pages(&self, line: &Line) -> bool {
        self.heading.is_none() && (ends_with_break(&self.text) || (!ends_sentence(&self.text) && line.text.starts_with(char::is_lowercase)))
    }

    fn push(&mut self, page: usize, line: &Line, words: &HashSet<String>) {
        join(&mut self.text, &line.text, words);
        self.lines += 1;
        if page != self.page { self.widest = line.right; self.left = line.left; }
        self.page = page;
        self.top = line.top;
        self.bottom = line.bottom;
        self.left = self.left.min(line.left);
        self.right = line.right;
        self.widest = self.widest.max(line.right);
        self.size = line.size;
    }

    fn finish(self) -> Block {
        let text = self.text.replace('\u{ad}', "");
        match self.heading {
            Some(style) if self.lines <= HEADING_MAX_LINES && text.chars().count() <= HEADING_MAX_CHARS => Block::Heading(style, text),
            _ => Block::Paragraph(text),
        }
    }
}

/// Builds the book's blocks from every page's lines in order. The body style is the size and weight
/// carrying the most characters; a line is in a heading style when it is at least 15% larger than
/// the body, or bold at about body size or larger when the body isn't bold. Lines of one style join
/// into blocks, and paragraphs continue across page breaks.
fn blocks(pages: &[Vec<Line>]) -> Vec<Block> {
    let mut weights: BTreeMap<Style, usize> = BTreeMap::new();
    for line in pages.iter().flatten() { *weights.entry(style(line)).or_default() += line.text.chars().count(); }
    let Some((&(body_size, body_bold), _)) = weights.iter().max_by_key(|(_, weight)| **weight) else { return Vec::new() };
    let body_size = body_size as f32 / 2.;
    let heading = |line: &Line| (line.size >= body_size * 1.15 || (line.bold && !body_bold && line.size >= body_size * 0.95)).then(|| style(line));
    let words = inline_words(pages);
    let mut drafts: Vec<Draft> = Vec::new();
    for (page, lines) in pages.iter().enumerate() {
        for line in lines {
            let kind = heading(line);
            let continues = drafts.last().is_some_and(|draft| draft.heading == kind && if draft.page == page { draft.continues_on_page(line) } else { draft.continues_across_pages(line) });
            match drafts.last_mut() {
                Some(draft) if continues => draft.push(page, line, &words),
                _ => drafts.push(Draft::start(kind, page, line)),
            }
        }
    }
    drafts.into_iter().map(Draft::finish).collect()
}

/// Every word that appears inside a line rather than at its end, case folded, so a hyphen at a
/// line end can be checked against how the PDF writes the same word elsewhere.
fn inline_words(pages: &[Vec<Line>]) -> HashSet<String> {
    let mut words = HashSet::new();
    for line in pages.iter().flatten() {
        let tokens = line.text.split_whitespace().collect::<Vec<_>>();
        for token in &tokens[..tokens.len().saturating_sub(1)] {
            let word = word_core(token);
            if !word.is_empty() { words.insert(word); }
        }
    }
    words
}

/// A token without surrounding punctuation or soft hyphens, case folded; inner hyphens stay.
fn word_core(token: &str) -> String {
    token.replace('\u{ad}', "").trim_matches(|character: char| !character.is_alphanumeric() && character != '-').trim_matches('-').to_lowercase()
}

/// Strips closing quotes and brackets, then checks for a sentence end.
fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', '\u{201d}', '\u{2019}', '\u{bb}', ')', ']']).ends_with(['.', '!', '?', '\u{2026}'])
}

/// Ends with a soft hyphen, or a hyphen right after a letter: a word broken at the line end.
fn ends_with_break(text: &str) -> bool {
    if text.ends_with('\u{ad}') { return true; }
    let mut characters = text.chars().rev();
    matches!(characters.next(), Some('-' | '\u{2010}')) && characters.next().is_some_and(char::is_alphabetic)
}

/// Joins the next line onto a block's text.
///
/// A word broken at the line end is rejoined: a soft hyphen always goes; a hard hyphen goes when the
/// next line continues in lower case, unless the hyphen is a real compound's. It is kept when the
/// part before it is already hyphenated ("state-of-the-art"), or when the PDF writes the hyphenated
/// form inside a line somewhere and never the joined one ("well-known"). A hyphen followed by a
/// capital or a digit ("Jean-Paul", "COVID-19") is kept. A line ending in an em dash joins without a
/// space. Any other line joins with a space.
fn join(text: &mut String, next: &str, words: &HashSet<String>) {
    if text.ends_with('\u{ad}') {
        text.pop();
        text.push_str(next);
        return;
    }
    if ends_with_break(text) && next.starts_with(char::is_alphanumeric) {
        let hyphen = text.chars().next_back().map_or(0, char::len_utf8);
        let stem = &text[..text.len() - hyphen];
        let prefix = word_core(stem.rsplit(char::is_whitespace).next().unwrap_or_default());
        let suffix = word_core(next.split_whitespace().next().unwrap_or_default());
        let compound = prefix.contains('-') || (words.contains(&format!("{prefix}-{suffix}")) && !words.contains(&format!("{prefix}{suffix}")));
        if next.starts_with(char::is_lowercase) && !compound { text.truncate(text.len() - hyphen); }
        text.push_str(next);
        return;
    }
    if !text.ends_with('\u{2014}') { text.push(' '); }
    text.push_str(next);
}

/// A content document's text: headings carry their level and an anchor for the navigation.
#[derive(Debug, Clone, PartialEq)]
enum Item { Heading { level: u8, text: String, anchor: String }, Paragraph(String) }

/// One spine document.
#[derive(Debug, Clone, PartialEq)]
struct Chapter { number: usize, items: Vec<Item> }

impl Chapter {
    fn title<'a>(&'a self, book: &'a str) -> &'a str {
        self.items.iter().find_map(|item| match item { Item::Heading { text, .. } => Some(text.as_str()), Item::Paragraph(_) => None }).unwrap_or(book)
    }
}

/// Gives headings levels and splits the blocks into chapters.
///
/// Heading styles rank by size, then bold before regular, and the ranks become levels 1 to 6. The
/// book splits at headings of the shallowest level that occurs more than once (or the shallowest
/// level when none repeats) and of any level above it, so a lone title doesn't hold the whole book.
/// A heading right after another heading stays in its chapter, and a long chapter continues in a
/// new document at a paragraph.
fn chapters(blocks: Vec<Block>) -> Vec<Chapter> {
    let mut styles = blocks.iter().filter_map(|block| match block { Block::Heading(style, _) => Some(*style), Block::Paragraph(_) => None }).collect::<Vec<_>>();
    styles.sort_by_key(|&(size, bold)| (std::cmp::Reverse(size), std::cmp::Reverse(bold)));
    styles.dedup();
    let level = |style: &Style| (styles.iter().position(|known| known == style).unwrap_or(0) + 1).min(6) as u8;
    let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
    for block in &blocks { if let Block::Heading(style, _) = block { *counts.entry(level(style)).or_default() += 1; } }
    let split = counts.iter().find(|(_, count)| **count >= 2).or(counts.iter().next()).map_or(0, |(level, _)| *level);

    let mut chapters = vec![Chapter { number: 1, items: Vec::new() }];
    let mut length = 0;
    let mut anchors = 0;
    for block in blocks {
        let current = chapters.last().expect("there is always a chapter");
        let has_text = current.items.iter().any(|item| matches!(item, Item::Paragraph(_)));
        let item = match block {
            Block::Heading(style, text) => { anchors += 1; Item::Heading { level: level(&style), text, anchor: format!("heading-{anchors}") } }
            Block::Paragraph(text) => Item::Paragraph(text),
        };
        let breaks = match &item {
            Item::Heading { level, .. } => *level <= split && has_text,
            Item::Paragraph(_) => length > DOCUMENT_TEXT_LIMIT,
        };
        if breaks {
            chapters.push(Chapter { number: chapters.len() + 1, items: Vec::new() });
            length = 0;
        }
        length += match &item { Item::Heading { text, .. } | Item::Paragraph(text) => text.len() };
        chapters.last_mut().expect("there is always a chapter").items.push(item);
    }
    chapters
}

/// Collapses whitespace and drops characters XML 1.0 cannot carry.
fn clean(text: &str) -> String {
    let allowed = text.chars().filter(|character| !matches!(character, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}'));
    allowed.collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn escape(text: &str) -> String {
    quick_xml::escape::escape(clean(text).as_str()).into_owned()
}

/// A name-based UUID from the PDF's bytes, so the same PDF always gets the same identifier.
fn identifier(bytes: &[u8]) -> String {
    let mut hash: [u8; 16] = Sha256::digest(bytes)[..16].try_into().expect("a SHA-256 digest has 16 leading bytes");
    hash[6] = (hash[6] & 0x0f) | 0x80; // version 8: custom, name-based
    hash[8] = (hash[8] & 0x3f) | 0x80; // RFC 9562 variant
    let hex = hash.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    format!("urn:uuid:{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

fn chapter_name(number: usize) -> String { format!("chapter-{number}.xhtml") }

fn xhtml(title: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\">\n<head>\n<meta charset=\"UTF-8\"/>\n<title>{}</title>\n</head>\n<body>\n{body}</body>\n</html>\n",
        escape(title),
    )
}

fn chapter_document(title: &str, chapter: &Chapter) -> String {
    let items = chapter.items.iter().map(|item| match item {
        Item::Heading { level, text, anchor } => format!("<h{level} id=\"{anchor}\">{}</h{level}>\n", escape(text)),
        Item::Paragraph(text) => format!("<p>{}</p>\n", escape(text)),
    }).collect::<String>();
    xhtml(chapter.title(title), &format!("<section id=\"chapter-{}\">\n{items}</section>\n", chapter.number))
}

/// The table of contents: every heading, nested by level, or the book's title when it has none.
/// A level never goes more than one deeper than the entry before it, so the lists nest validly.
fn nav_document(title: &str, chapters: &[Chapter]) -> String {
    let mut entries = chapters.iter().flat_map(|chapter| chapter.items.iter().filter_map(move |item| match item {
        Item::Heading { level, text, anchor } => Some((*level, format!("<a href=\"{}#{anchor}\">{}</a>", chapter_name(chapter.number), escape(text)))),
        Item::Paragraph(_) => None,
    })).collect::<Vec<_>>();
    if entries.is_empty() { entries.push((1, format!("<a href=\"{}\">{}</a>", chapter_name(1), escape(title)))); }
    let shallowest = entries.iter().map(|(level, _)| *level).min().unwrap_or(1);
    let mut list = String::from("<ol>\n<li>");
    let mut depth = 0usize;
    for (index, (level, link)) in entries.iter().enumerate() {
        let wanted = if index == 0 { 0 } else { usize::from(level - shallowest).min(depth + 1) };
        if index > 0 && wanted > depth {
            list.push_str("\n<ol>\n<li>");
        } else if index > 0 {
            for _ in wanted..depth { list.push_str("</li>\n</ol>\n"); }
            list.push_str("</li>\n<li>");
        }
        depth = wanted;
        list.push_str(link);
    }
    for _ in 0..depth { list.push_str("</li>\n</ol>\n"); }
    list.push_str("</li>\n</ol>\n");
    xhtml(title, &format!("<nav epub:type=\"toc\" id=\"toc\">\n<h1>Contents</h1>\n{list}</nav>\n"))
}

fn package_document(title: &str, identifier: &str, chapters: &[Chapter]) -> String {
    let items = chapters.iter().map(|chapter| format!("<item id=\"chapter-{0}\" href=\"{1}\" media-type=\"application/xhtml+xml\"/>\n", chapter.number, chapter_name(chapter.number))).collect::<String>();
    let spine = chapters.iter().map(|chapter| format!("<itemref idref=\"chapter-{}\"/>\n", chapter.number)).collect::<String>();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"book-id\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n<dc:identifier id=\"book-id\">{identifier}</dc:identifier>\n<dc:title>{}</dc:title>\n<dc:language>{LANGUAGE}</dc:language>\n<meta property=\"dcterms:modified\">{MODIFIED}</meta>\n</metadata>\n<manifest>\n<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n{items}</manifest>\n<spine>\n{spine}</spine>\n</package>\n",
        escape(title),
    )
}

const CONTAINER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n<rootfiles>\n<rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n</rootfiles>\n</container>\n";

/// Builds the package: `mimetype` first and stored, then the container, package, nav and chapters.
/// Every entry has the same fixed time, permissions and host system, so the bytes match on every platform.
fn package(title: &str, identifier: &str, chapters: &[Chapter]) -> Result<Vec<u8>, String> {
    let fixed = SimpleFileOptions::default().last_modified_time(DateTime::default()).unix_permissions(0o644).system(System::Unix);
    let stored = fixed.compression_method(CompressionMethod::Stored);
    let deflated = fixed.compression_method(CompressionMethod::Deflated).compression_level(Some(6));
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    let mut entry = |name: &str, options: SimpleFileOptions, contents: &[u8]| -> Result<(), String> {
        archive.start_file(name, options).map_err(|error| error.to_string())?;
        archive.write_all(contents).map_err(|error| error.to_string())
    };
    entry("mimetype", stored, MIMETYPE.as_bytes())?;
    entry("META-INF/container.xml", deflated, CONTAINER.as_bytes())?;
    entry(PACKAGE_PATH, deflated, package_document(title, identifier, chapters).as_bytes())?;
    entry(NAV_PATH, deflated, nav_document(title, chapters).as_bytes())?;
    for chapter in chapters {
        entry(&format!("OEBPS/{}", chapter_name(chapter.number)), deflated, chapter_document(title, chapter).as_bytes())?;
    }
    archive.finish().map(Cursor::into_inner).map_err(|error| error.to_string())
}

/// Reopens a built EPUB and checks its structure: `mimetype` first, stored and exact; the container
/// pointing at the package; every part present and well-formed XML; and no embedded fonts.
fn verify(epub: &[u8]) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(epub)).map_err(|error| format!("it doesn't reopen as a ZIP: {error}"))?;
    {
        let mut first = archive.by_index(0).map_err(|error| error.to_string())?;
        if first.name() != "mimetype" || first.compression() != CompressionMethod::Stored { return Err("mimetype isn't the first, uncompressed entry".into()); }
        let mut mimetype = String::new();
        first.read_to_string(&mut mimetype).map_err(|error| error.to_string())?;
        if mimetype != MIMETYPE { return Err("mimetype is wrong".into()); }
    }
    let names = archive.file_names().map(str::to_string).collect::<Vec<_>>();
    if names.iter().any(|name| [".ttf", ".otf", ".woff", ".woff2"].iter().any(|extension| name.to_ascii_lowercase().ends_with(extension))) {
        return Err("it embeds a font".into());
    }
    for required in ["META-INF/container.xml", PACKAGE_PATH, NAV_PATH] {
        if !names.iter().any(|name| name == required) { return Err(format!("{required} is missing")); }
    }
    for name in names.iter().filter(|name| name.ends_with(".xml") || name.ends_with(".opf") || name.ends_with(".xhtml")) {
        let mut part = String::new();
        archive.by_name(name).map_err(|error| error.to_string())?.read_to_string(&mut part).map_err(|error| error.to_string())?;
        well_formed(&part).map_err(|error| format!("{name} isn't well-formed: {error}"))?;
        if name == PACKAGE_PATH {
            for href in attribute_values(&part, "href") {
                if !names.contains(&format!("OEBPS/{href}")) { return Err(format!("the package lists {href}, which is missing")); }
            }
        }
    }
    Ok(())
}

fn well_formed(xml: &str) -> Result<(), String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().check_end_names = true;
    let mut depth = 0usize;
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            quick_xml::events::Event::Start(_) => depth += 1,
            quick_xml::events::Event::End(_) => depth = depth.checked_sub(1).ok_or("an element closes twice")?,
            quick_xml::events::Event::Eof => return if depth == 0 { Ok(()) } else { Err("an element isn't closed".into()) },
            _ => {}
        }
    }
}

fn attribute_values(xml: &str, attribute: &str) -> Vec<String> {
    let needle = format!(" {attribute}=\"");
    xml.match_indices(&needle).filter_map(|(start, _)| {
        let rest = &xml[start + needle.len()..];
        rest.find('"').map(|end| rest[..end].to_string())
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::contracts::{OutcomeState, WarningKind};

    fn run(text: &str, x: f32, y: f32, size: f32) -> PageTextRun {
        PageTextRun { text: text.into(), x, y, width: text.chars().count() as f32 * size * 0.5, height: size, font_size: size, bold: false, italic: false, reliable_unicode: true }
    }

    fn page(runs: Vec<PageTextRun>) -> PageText {
        let has_extractable_text = runs.iter().any(|run| run.reliable_unicode && !run.text.trim().is_empty());
        PageText { runs, has_extractable_text }
    }

    fn paragraph(text: &str) -> Block { Block::Paragraph(text.into()) }

    #[test]
    fn runs_join_into_lines_and_lines_into_paragraphs() {
        let text = page(vec![
            run("The quick", 72., 100., 12.), run("brown fox", 72. + 9. * 6. + 3., 100., 12.),
            run("jumps over.", 72., 114., 12.),
            run("A new", 72., 150., 12.), run("paragraph", 72. + 5. * 6. + 4., 150., 12.),
            run("Heading", 72., 170., 24.),
        ]);
        assert_eq!(blocks(&[page_lines(&text)]), [paragraph("The quick brown fox jumps over."), paragraph("A new paragraph"), Block::Heading((48, false), "Heading".into())]);
    }

    #[test]
    fn an_indent_or_a_short_last_line_after_a_sentence_starts_a_paragraph() {
        let text = page(vec![
            run("A line that fills the measure", 72., 100., 12.),
            run("ends here.", 72., 114., 12.),
            run("Next paragraph starts flush", 72., 128., 12.),
            run("and fills the whole measure.", 72., 142., 12.),
            run("Indented one follows.", 90., 156., 12.),
        ]);
        assert_eq!(blocks(&[page_lines(&text)]), [
            paragraph("A line that fills the measure ends here."),
            paragraph("Next paragraph starts flush and fills the whole measure."),
            paragraph("Indented one follows."),
        ]);
    }

    #[test]
    fn a_paragraph_continues_on_the_next_page_only_mid_sentence() {
        let pages = [
            page_lines(&page(vec![run("Body text that stops in the", 72., 100., 12.)])),
            page_lines(&page(vec![run("middle of a sentence.", 72., 100., 12.), run("Then text with an ending.", 72., 140., 12.)])),
            page_lines(&page(vec![run("Capitalised start.", 72., 100., 12.)])),
        ];
        assert_eq!(blocks(&pages), [
            paragraph("Body text that stops in the middle of a sentence."),
            paragraph("Then text with an ending."),
            paragraph("Capitalised start."),
        ]);
    }

    #[test]
    fn words_broken_at_line_ends_are_rejoined_and_compounds_keep_their_hyphen() {
        let words = ["well-known".to_string()].into_iter().collect::<HashSet<_>>();
        let joined = |text: &str, next: &str| { let mut text = text.to_string(); join(&mut text, next, &words); text };
        assert_eq!(joined("takes pa-", "tience now"), "takes patience now");
        assert_eq!(joined("takes pa\u{2010}", "tience"), "takes patience");
        assert_eq!(joined("soft\u{ad}", "ware"), "software");
        assert_eq!(joined("a well-", "known fact"), "a well-known fact");
        assert_eq!(joined("a state-of-the-", "art lens"), "a state-of-the-art lens");
        assert_eq!(joined("met Jean-", "Paul"), "met Jean-Paul");
        assert_eq!(joined("during COVID-", "19"), "during COVID-19");
        assert_eq!(joined("a pause\u{2014}", "then"), "a pause\u{2014}then");
        assert_eq!(joined("a range 1 -", "2"), "a range 1 - 2");
        assert_eq!(joined("one line", "and the next"), "one line and the next");
        let both = ["well-known".to_string(), "wellknown".to_string()].into_iter().collect::<HashSet<_>>();
        let mut text = "a well-".to_string();
        join(&mut text, "known", &both);
        assert_eq!(text, "a wellknown", "the joined form written in-line wins");
    }

    #[test]
    fn band_text_reads_every_page_number_alike() {
        assert_eq!(band_text("Page 12 of 140"), "page # of #");
        assert_eq!(band_text("iv"), "#");
        assert_eq!(band_text("XII"), "#");
        assert_eq!(band_text("Chapter One"), "chapter one");
    }

    #[test]
    fn runs_without_a_reliable_mapping_and_xml_illegal_characters_are_left_out() {
        let mut garbled = run("\u{fffd}\u{fffd}", 72., 100., 12.);
        garbled.reliable_unicode = false;
        let text = page(vec![garbled, run("Bell\u{7} & <tags>", 200., 100., 12.)]);
        assert_eq!(blocks(&[page_lines(&text)]), [paragraph("Bell & <tags>")]);
        assert_eq!(escape("Bell\u{7} & <tags>"), "Bell &amp; &lt;tags&gt;");
    }

    #[test]
    fn the_identifier_is_a_stable_uuid_derived_from_the_bytes() {
        let first = identifier(b"one pdf");
        assert_eq!(first, identifier(b"one pdf"));
        assert_ne!(first, identifier(b"another pdf"));
        let uuid = first.strip_prefix("urn:uuid:").unwrap();
        assert_eq!(uuid.len(), 36);
        assert_eq!(&uuid[14..15], "8", "version 8");
        assert!(matches!(&uuid[19..20], "8" | "9" | "a" | "b"), "RFC 9562 variant");
    }

    #[test]
    fn verification_refuses_a_package_without_a_leading_stored_mimetype() {
        let chapters = [Chapter { number: 1, items: vec![Item::Paragraph("Text".into())] }];
        let good = package("Book", &identifier(b"x"), &chapters).unwrap();
        assert_eq!(verify(&good), Ok(()));
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        archive.start_file("META-INF/container.xml", SimpleFileOptions::default()).unwrap();
        archive.write_all(CONTAINER.as_bytes()).unwrap();
        let bad = archive.finish().unwrap().into_inner();
        assert!(verify(&bad).unwrap_err().contains("mimetype"));
    }

    #[test]
    fn a_page_with_no_text_is_named_and_left_out() {
        let root = folder("blank");
        let input = root.join("mixed.pdf");
        pdf_with_pages(&input, &["BT /F1 12 Tf 72 720 Td (First page) Tj ET", "", "BT /F1 12 Tf 72 720 Td (Third page) Tj ET"]);
        let outcome = convert(&request(&input, true), input);
        let OutcomeState::Succeeded { output_paths } = &outcome.state else { panic!("{outcome:?}") };
        assert_eq!(outcome.detail, "3 pages.");
        assert!(matches!(&outcome.warnings[0].kind, WarningKind::PagesWithoutText { pages } if pages == &[2]));
        let chapter = entry(&output_paths[0], "OEBPS/chapter-1.xhtml");
        assert!(chapter.contains("<p>First page</p>\n<p>Third page</p>"), "{chapter}");
        let _ = fs::remove_dir_all(root);
    }

    /// Three pages with a running head, page-number footers, and one paragraph that breaks words
    /// across lines and runs over a page break.
    const FIELD_GUIDE: [&str; 3] = [
        "BT /F1 10 Tf 72 760 Td (The Field Guide) Tj ET \
         BT /F1 12 Tf 72 700 Td (Good birding takes pa-) Tj ET \
         BT /F1 12 Tf 72 686 Td (tience and a well-aimed lens. A well-) Tj ET \
         BT /F1 12 Tf 72 672 Td (aimed lens finds the tar-) Tj ET \
         BT /F1 12 Tf 72 658 Td (get before the bird moves. We wait until the) Tj ET \
         BT /F1 10 Tf 300 40 Td (1) Tj ET",
        "BT /F1 10 Tf 72 760 Td (The Field Guide) Tj ET \
         BT /F1 12 Tf 72 700 Td (light fades over the water.) Tj ET \
         BT /F1 12 Tf 90 686 Td (A second paragraph begins here.) Tj ET \
         BT /F1 10 Tf 300 40 Td (2) Tj ET",
        "BT /F1 10 Tf 72 760 Td (The Field Guide) Tj ET \
         BT /F1 12 Tf 72 700 Td (The last page closes the book.) Tj ET \
         BT /F1 10 Tf 300 40 Td (3) Tj ET",
    ];

    #[test]
    fn lines_join_into_paragraphs_across_pages_with_broken_words_rejoined() {
        let root = folder("paragraphs");
        let input = root.join("guide.pdf");
        pdf_with_pages(&input, &FIELD_GUIDE);
        let outcome = convert(&request(&input, true), input);
        let OutcomeState::Succeeded { output_paths } = &outcome.state else { panic!("{outcome:?}") };
        let chapter = entry(&output_paths[0], "OEBPS/chapter-1.xhtml");
        assert!(chapter.contains(
            "<p>Good birding takes patience and a well-aimed lens. A well-aimed lens finds the target before the bird moves. We wait until the light fades over the water.</p>\n\
             <p>A second paragraph begins here.</p>\n<p>The last page closes the book.</p>\n</section>"
        ), "{chapter}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn running_headers_and_page_numbers_are_dropped_unless_the_request_keeps_them() {
        let root = folder("bands");
        let input = root.join("guide.pdf");
        pdf_with_pages(&input, &FIELD_GUIDE);
        let removed = convert(&request(&input, true), input.clone());
        let OutcomeState::Succeeded { output_paths } = &removed.state else { panic!("{removed:?}") };
        let chapter = entry(&output_paths[0], "OEBPS/chapter-1.xhtml");
        assert!(!chapter.contains("The Field Guide</p>") && !chapter.contains("<p>1</p>") && !chapter.contains("<p>3</p>"), "{chapter}");

        let kept = convert(&request(&input, false), input);
        let OutcomeState::Succeeded { output_paths } = &kept.state else { panic!("{kept:?}") };
        let chapter = entry(&output_paths[0], "OEBPS/chapter-1.xhtml");
        assert_eq!(chapter.matches("<p>The Field Guide</p>").count(), 3, "{chapter}");
        for number in 1..=3 { assert!(chapter.contains(&format!("<p>{number}</p>")), "{chapter}"); }
        assert!(chapter.contains("<p>Good birding takes patience and a well-aimed lens. A well-aimed lens finds the target before the bird moves. We wait until the</p>"), "{chapter}");
        let _ = fs::remove_dir_all(root);
    }

    /// A title, two sections and bold subsections across two pages.
    const HANDBOOK: [&str; 2] = [
        "BT /F1 24 Tf 72 700 Td (The Field Guide) Tj ET \
         BT /F1 18 Tf 72 650 Td (Getting started) Tj ET \
         BT /F1 12 Tf 72 620 Td (Bring water and patience.) Tj ET \
         BT /F2 12 Tf 72 590 Td (Gear) Tj ET \
         BT /F1 12 Tf 72 570 Td (Binoculars matter most.) Tj ET \
         BT /F2 12 Tf 72 540 Td (Timing) Tj ET \
         BT /F1 12 Tf 72 520 Td (Go out at dawn & stay late.) Tj ET",
        "BT /F1 18 Tf 72 700 Td (Finding birds) Tj ET \
         BT /F2 12 Tf 72 670 Td (Wetlands) Tj ET \
         BT /F1 12 Tf 72 650 Td (Herons wade in the shallows.) Tj ET",
    ];

    #[test]
    fn larger_and_bold_text_becomes_headings_that_split_chapters_and_fill_the_navigation() {
        let root = folder("headings");
        let input = root.join("handbook.pdf");
        pdf_with_pages(&input, &HANDBOOK);
        let outcome = convert(&request(&input, true), input);
        let OutcomeState::Succeeded { output_paths } = &outcome.state else { panic!("{outcome:?}") };
        let first = entry(&output_paths[0], "OEBPS/chapter-1.xhtml");
        assert!(first.contains(
            "<section id=\"chapter-1\">\n<h1 id=\"heading-1\">The Field Guide</h1>\n<h2 id=\"heading-2\">Getting started</h2>\n<p>Bring water and patience.</p>\n<h3 id=\"heading-3\">Gear</h3>\n"
        ), "{first}");
        assert!(first.contains("<p>Go out at dawn &amp; stay late.</p>\n</section>"), "{first}");
        let second = entry(&output_paths[0], "OEBPS/chapter-2.xhtml");
        assert!(second.contains("<title>Finding birds</title>") && second.contains("<h2 id=\"heading-5\">Finding birds</h2>\n<h3 id=\"heading-6\">Wetlands</h3>"), "{second}");

        let book = crate::kit::pdf::epub::parse(&fs::read(&output_paths[0]).unwrap(), u64::MAX, ToolError::processing("too big"), 1.).unwrap_or_else(|error| panic!("{error:?}"));
        assert_eq!(book.chapters.len(), 2);
        let navigation = book.navigation.iter().map(|entry| (entry.level, entry.title.as_str(), entry.target.as_str())).collect::<Vec<_>>();
        assert_eq!(navigation, [
            (1, "The Field Guide", "OEBPS/chapter-1.xhtml#heading-1"),
            (2, "Getting started", "OEBPS/chapter-1.xhtml#heading-2"),
            (3, "Gear", "OEBPS/chapter-1.xhtml#heading-3"),
            (3, "Timing", "OEBPS/chapter-1.xhtml#heading-4"),
            (2, "Finding birds", "OEBPS/chapter-2.xhtml#heading-5"),
            (3, "Wetlands", "OEBPS/chapter-2.xhtml#heading-6"),
        ]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_same_pdf_makes_byte_identical_epubs() {
        let root = folder("repeat");
        for (name, pages) in [("handbook", &HANDBOOK[..]), ("guide", &FIELD_GUIDE[..])] {
            let input = root.join(format!("{name}.pdf"));
            pdf_with_pages(&input, pages);
            let outputs = (0..2).map(|index| {
                let destination = root.join(format!("{name}-{index}"));
                fs::create_dir_all(&destination).unwrap();
                let outcome = convert(&PdfToEpubRequest { paths: vec![input.clone()], output_location: OutputLocation::CustomFolder(destination), remove_headers_and_footers: true }, input.clone());
                let OutcomeState::Succeeded { output_paths } = &outcome.state else { panic!("{outcome:?}") };
                fs::read(&output_paths[0]).unwrap()
            }).collect::<Vec<_>>();
            assert!(outputs[0] == outputs[1], "{name} differs between runs");
        }
        let _ = fs::remove_dir_all(root);
    }

    fn request(input: &Path, remove_headers_and_footers: bool) -> PdfToEpubRequest {
        PdfToEpubRequest { paths: vec![input.to_path_buf()], output_location: OutputLocation::AlongsideInput, remove_headers_and_footers }
    }

    fn folder(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("toolbox-to-epub-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn entry(epub: &Path, name: &str) -> String {
        let mut archive = ZipArchive::new(fs::File::open(epub).unwrap()).unwrap();
        let mut text = String::new();
        archive.by_name(name).unwrap_or_else(|error| panic!("{name}: {error}")).read_to_string(&mut text).unwrap();
        text
    }

    /// A US Letter PDF with one content stream per page; F1 is Helvetica and F2 Helvetica-Bold.
    fn pdf_with_pages(path: &Path, pages: &[&str]) {
        use lopdf::{dictionary, Object, Stream};
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let regular = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
        let bold = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica-Bold", "Encoding" => "WinAnsiEncoding" });
        let kids = pages.iter().map(|content| {
            let content_id = document.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
            Object::Reference(document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => regular, "F2" => bold } }, "Contents" => content_id,
            }))
        }).collect::<Vec<_>>();
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages.len() as i64 }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }
}
