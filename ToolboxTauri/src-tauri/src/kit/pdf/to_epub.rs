//! PDF to EPUB: each PDF's selectable text becomes a reflowable EPUB 3.
//!
//! Text comes from the in-process page-text reader. Runs are joined into lines by their positions.
//! Running headers, footers and page numbers are dropped unless the request keeps them; lines join
//! into paragraphs, across page breaks too, with words broken at line ends rejoined; and text set
//! larger or bolder than the body becomes headings, which split the book into chapter documents.
//! The table of contents follows the PDF's bookmarks, or the headings when it has none. Links to
//! pages become links into the book and web links are kept as written. Title and author come from
//! the PDF unless the request replaces them, and the first page can be rendered as the cover. Images
//! drawn on the pages are kept as Extract from PDF saves them, each placed where it is drawn among the
//! text and stored once however often it is drawn. The package is built in memory with fixed timestamps and an identifier derived from the PDF's bytes,
//! so the same PDF always makes the same EPUB, then reopened and checked before it is published.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use lopdf::{Dictionary, Document, Object, ObjectId};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, System, ZipArchive, ZipWriter};

use crate::kit::common::{note_renamed, JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::{OutcomeDetail, OutcomePointer, OutcomeWarning, SkipCode, SkipReason, ToolError, WarningKind};
use super::document_info;
use super::embedded_image::{self, Extracted, NotSaved};
use super::page_text::{read_document_text, PageImageSource, PageText, PageTextRun};
use super::remaining::{not_saved_warning, pages_without_text, PDF_DAMAGED};
use super::resolve;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfToEpubRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
    /// Leaves out running headers, footers and page numbers. On unless the request turns it off.
    #[serde(default = "on")]
    pub remove_headers_and_footers: bool,
    /// Whether the book opens with its first page as the cover. The first page unless the request says otherwise.
    #[serde(default)]
    pub cover: EpubCover,
    /// Replaces the PDF's title in the book when it isn't blank. The app offers it for a single PDF only.
    #[serde(default)]
    pub title: Option<String>,
    /// Replaces the PDF's author in the book when it isn't blank.
    #[serde(default)]
    pub author: Option<String>,
}

fn on() -> bool { true }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EpubCover {
    /// The first page, rendered as an image.
    #[default]
    FirstPage,
    #[serde(rename = "none")]
    NoCover,
}

pub const NO_SELECTABLE_TEXT: &str = "PDF contains no selectable text; scanned PDFs require OCR. No EPUB was made.";
pub const PASSWORD_PROTECTED: &str = "This PDF is password-protected. Remove the password, then convert again.";
pub const COVER_NOT_RENDERED: &str = "Omitted content: the cover, because the first page couldn't be rendered.";
/// Closes the warning for an image that couldn't be read.
pub const IMAGE_LEFT_OUT: &str = "Left out; the rest of the book converted.";

const MIMETYPE: &str = "application/epub+zip";
const PACKAGE_PATH: &str = "OEBPS/content.opf";
const NAV_PATH: &str = "OEBPS/nav.xhtml";
const COVER_IMAGE: &str = "cover.jpg";
const COVER_PAGE: &str = "cover.xhtml";
/// The folder, beside the content documents, that holds the book's images.
const IMAGES: &str = "images";
/// The cover's JPEG quality: the rendered page is mostly flat colour and text.
const COVER_QUALITY: u8 = 85;
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
/// Bookmarks nested deeper than this, or past this many, are left out of the contents.
const BOOKMARK_MAX_DEPTH: usize = 64;
const BOOKMARK_MAX_COUNT: usize = 10_000;

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
    let page_ids = document.get_pages().into_values().collect::<Vec<_>>();
    let page_numbers = page_ids.iter().enumerate().map(|(index, id)| (*id, index)).collect::<HashMap<_, _>>();
    let mut lines = pages.iter().zip(&page_ids).map(|(page, id)| page_lines(page, &page_links(&document, *id, &page_numbers))).collect::<Vec<_>>();
    let blank = lines.iter().enumerate().filter(|(_, lines)| lines.is_empty()).map(|(index, _)| index as u32 + 1).collect::<Vec<_>>();
    if blank.len() == lines.len() {
        return JobOutcome::skipped(input, SkipReason::new(SkipCode::NeedsOcr, NO_SELECTABLE_TEXT).with_pointer(OutcomePointer::Tool { tool_id: "pdf-ocr".into() }));
    }
    if request.remove_headers_and_footers { remove_running_text(&mut lines); }
    let stem = input.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let details = BookDetails::new(document_info::from_document(&document).merged(), request, &stem);
    let images = book_images(&document, &pages);
    let chapters = chapters(blocks(&lines, &images.placed));
    let homes = page_homes(&chapters, pages.len());
    let bookmarks = bookmarks(&document, &page_numbers);
    let (source, contents) = if bookmarks.is_empty() {
        let headings = heading_contents(&chapters);
        (if headings.is_empty() { Source::Title } else { Source::Headings }, headings)
    } else {
        (Source::Bookmarks, bookmarks.iter().map(|bookmark| (bookmark.level, page_href(bookmark.page, &homes), bookmark.title.clone())).collect())
    };
    let mut anchors = bookmarks.iter().map(|bookmark| bookmark.page).collect::<BTreeSet<_>>();
    anchors.extend(chapters.iter().flat_map(|chapter| &chapter.items).filter_map(Item::body).flat_map(|body| &body.links).filter_map(|link| match link.target {
        Target::Page(page) => Some(page),
        Target::Uri(_) => None,
    }));
    let cover = (request.cover == EpubCover::FirstPage).then(|| cover_image(&input));
    let book = Book {
        title: details.title.clone(),
        author: details.author.clone(),
        identifier: identifier(&bytes),
        chapters,
        contents,
        homes,
        anchors,
        cover: cover.clone().flatten(),
        images: images.files,
    };
    let epub = match package(&book).and_then(|epub| verify(&epub).map(|()| epub)) {
        Ok(epub) => epub,
        Err(error) => return JobOutcome::failure(input, ToolError::processing(format!("Could not build the EPUB: {error}"))),
    };
    let path = match save(&input, &request.output_location, &epub) {
        Ok(path) => path,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let chosen = path.with_file_name(format!("{stem}.epub"));
    let entries = book.contents.len();
    let page_count = count(pages.len(), "page", "pages");
    let (kept, found) = (book.images.len(), book.images.len() + images.warnings.len());
    let detail = match (kept, images.warnings.len()) {
        (0, 0) => format!("{page_count}. {}", source.sentence(entries)),
        (_, 0) => format!("{page_count}, {}. {}", count(kept, "image", "images"), source.sentence(entries)),
        (_, unreadable) => format!("{page_count}, {kept} of {}. {} couldn't be read.", count(found, "image", "images"), count(unreadable, "image", "images")),
    };
    let cover_detail = match &cover {
        Some(Some(_)) => "Page 1",
        Some(None) => "None; the first page couldn't be rendered",
        None => "None",
    };
    let images_kept = (found > 0).then(|| OutcomeDetail { label: "Images kept".into(), value: format!("{kept} of {found}") });
    let outcome = note_renamed(JobOutcome::succeeded(input, vec![path], detail), Some(&chosen)).with_page_details(images_kept.into_iter().chain([
        OutcomeDetail { label: "Contents".into(), value: source.detail(entries) },
        OutcomeDetail { label: "Cover".into(), value: cover_detail.into() },
        OutcomeDetail { label: "Title and author".into(), value: details.describe() },
    ]).collect());
    let outcome = if matches!(cover, Some(None)) { outcome.with_warning(OutcomeWarning { kind: WarningKind::OmittedContent, message: COVER_NOT_RENDERED.into() }) } else { outcome };
    let outcome = images.warnings.into_iter().fold(outcome, JobOutcome::with_warning);
    if blank.is_empty() { outcome } else { outcome.with_warning(pages_without_text(blank)) }
}

fn password_protected(input: PathBuf) -> JobOutcome {
    JobOutcome::failure(input, ToolError::invalid_input(PASSWORD_PROTECTED).with_pointer(OutcomePointer::Tool { tool_id: "lock-unlock".into() }))
}

/// One image file in the book.
#[derive(Debug, Clone, PartialEq)]
struct BookImage { name: String, media_type: &'static str, data: Vec<u8> }

/// The book's images: each distinct image once, where every page draws one (its top and its file, by
/// page in draw order), and a warning for each image that couldn't be read.
struct BookImages { files: Vec<BookImage>, placed: Vec<Vec<(f32, usize)>>, warnings: Vec<OutcomeWarning> }

/// Reads every image the pages draw the way Extract from PDF saves it: JPEG data byte for byte and
/// anything else as a lossless PNG. JPEG 2000 is left out, since EPUB readers needn't show it. An image
/// object drawn again, or an image whose saved bytes match one already kept, is stored once. Images are
/// numbered on each page in the order the page draws them; an unreadable one is reported once, where it is
/// first drawn, and left out.
fn book_images(document: &Document, pages: &[PageText]) -> BookImages {
    let mut images = BookImages { files: Vec::new(), placed: Vec::new(), warnings: Vec::new() };
    let mut by_object: HashMap<ObjectId, Option<usize>> = HashMap::new();
    let mut by_content: HashMap<Vec<u8>, usize> = HashMap::new();
    for (page, text) in pages.iter().enumerate() {
        let mut placed = Vec::new();
        for (number, drawn) in text.images.iter().enumerate() {
            let object = match &drawn.source { PageImageSource::Object(id) => Some(*id), PageImageSource::Inline(_) => None };
            let file = match object.and_then(|id| by_object.get(&id)) {
                Some(known) => *known,
                None => {
                    let extracted = match &drawn.source {
                        PageImageSource::Object(id) => document.get_object(*id).and_then(Object::as_stream).map_err(|_| NotSaved::Damaged).and_then(|stream| embedded_image::extract(document, stream)),
                        PageImageSource::Inline(stream) => embedded_image::extract(document, stream),
                    };
                    let file = match extracted {
                        Ok(Extracted::Jpeg2000(_)) => Err(NotSaved::Unsupported("JPEG 2000".into())),
                        Ok(extracted) => {
                            let key = [extracted.extension().as_bytes(), &[0], &Sha256::digest(extracted.bytes())].concat();
                            Ok(*by_content.entry(key).or_insert_with(|| {
                                let media_type = if matches!(extracted, Extracted::Jpeg { .. }) { "image/jpeg" } else { "image/png" };
                                let name = format!("image-{}.{}", images.files.len() + 1, extracted.extension());
                                images.files.push(BookImage { name, media_type, data: extracted.bytes().to_vec() });
                                images.files.len() - 1
                            }))
                        }
                        Err(reason) => Err(reason),
                    };
                    let file = file.map_err(|reason| images.warnings.push(not_saved_warning(page as u32 + 1, number as u32 + 1, reason, IMAGE_LEFT_OUT))).ok();
                    if let Some(id) = object { by_object.insert(id, file); }
                    file
                }
            };
            if let Some(file) = file { placed.push((drawn.y, file)); }
        }
        images.placed.push(placed);
    }
    images
}

fn save(input: &Path, location: &OutputLocation, epub: &[u8]) -> Result<PathBuf, ToolError> {
    let output = OutputNaming::reserve_destination(input, location, "", "epub").map_err(|error| ToolError::processing(format!("Could not reserve the EPUB output: {error}")))?;
    fs::write(output.path(), epub).map_err(|error| ToolError::processing(format!("Could not write the EPUB: {error}")))?;
    output.publish().map_err(|error| ToolError::processing(format!("Could not save the EPUB: {error}")))
}

/// The book's title and author: the request's when given, else the PDF's, else the file name as the title.
#[derive(Debug, Clone, PartialEq)]
struct BookDetails { title: String, author: Option<String>, changed: bool }

impl BookDetails {
    fn new(properties: document_info::PdfProperties, request: &PdfToEpubRequest, stem: &str) -> Self {
        let given = |value: &Option<String>| value.as_deref().map(clean).filter(|value| !value.is_empty());
        let (pdf_title, pdf_author) = (given(&properties.title), given(&properties.author));
        let (title, author) = (given(&request.title), given(&request.author));
        let changed = title.as_ref().is_some_and(|title| Some(title) != pdf_title.as_ref()) || author.as_ref().is_some_and(|author| Some(author) != pdf_author.as_ref());
        Self { title: title.or(pdf_title).unwrap_or_else(|| clean(stem)), author: author.or(pdf_author), changed }
    }

    fn describe(&self) -> String {
        let names = match &self.author { Some(author) => format!("{}, {author}", self.title), None => self.title.clone() };
        if self.changed { format!("{names} (changed for this book)") } else { names }
    }
}

/// Where the table of contents came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source { Bookmarks, Headings, Title }

impl Source {
    fn sentence(self, entries: usize) -> String {
        match self {
            Source::Bookmarks => format!("Contents from the bookmarks, {}.", count(entries, "entry", "entries")),
            Source::Headings => format!("Contents from headings in the text, {}.", count(entries, "entry", "entries")),
            Source::Title => "No bookmarks or headings, so the contents list the title only.".into(),
        }
    }

    fn detail(self, entries: usize) -> String {
        match self {
            Source::Bookmarks => format!("From the PDF's bookmarks, {}", count(entries, "entry", "entries")),
            Source::Headings => format!("From headings in the text, {}", count(entries, "entry", "entries")),
            Source::Title => "The title only; the PDF has no bookmarks and no headings were found".into(),
        }
    }
}

fn count(number: usize, one: &str, many: &str) -> String {
    if number == 1 { format!("1 {one}") } else { format!("{number} {many}") }
}

/// Renders the first page and encodes it as a JPEG, or `None` when it can't be rendered.
fn cover_image(input: &Path) -> Option<Vec<u8>> {
    let png = super::metadata::render_page_png(input, 1)?;
    let page = image::load_from_memory_with_format(&png, image::ImageFormat::Png).ok()?.to_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, COVER_QUALITY).encode_image(&page).ok()?;
    Some(jpeg)
}

/// Where a link goes: a page of the PDF, counted from 0, or a URI kept as written.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target { Page(usize), Uri(String) }

/// Where a bookmark, link annotation or action goes. Destinations that don't open a page of this
/// PDF, other actions, and script URIs give nothing.
fn link_target(document: &Document, item: &Dictionary, pages: &HashMap<ObjectId, usize>) -> Option<Target> {
    let page = |destination: &Object| super::scene::destination_page(document, destination, 0).and_then(|id| pages.get(&id)).map(|page| Target::Page(*page));
    if let Ok(destination) = item.get(b"Dest") { return page(destination); }
    let action = resolve(document, item.get(b"A").ok()?).ok()?.as_dict().ok()?;
    match action.get(b"S").ok().and_then(|kind| kind.as_name().ok())? {
        b"GoTo" => page(action.get(b"D").ok()?),
        b"URI" => {
            let uri = std::str::from_utf8(resolve(document, action.get(b"URI").ok()?).ok()?.as_str().ok()?).ok()?;
            let usable = !uri.trim().is_empty() && !uri.chars().any(char::is_control) && !uri.trim_start().to_ascii_lowercase().starts_with("javascript:");
            usable.then(|| Target::Uri(uri.to_string()))
        }
        _ => None,
    }
}

/// A link annotation's area, measured like the page text: from the crop box's top-left corner, `y` growing down.
#[derive(Debug, Clone, PartialEq)]
struct PageLink { left: f32, top: f32, right: f32, bottom: f32, target: Target }

impl PageLink {
    /// A run belongs to a link when its middle sits inside the link's area, give or take a point.
    fn holds(&self, run: &PageTextRun) -> bool {
        let (x, y) = (run.x + run.width / 2., run.y + run.height / 2.);
        x >= self.left - 1. && x <= self.right + 1. && y >= self.top - 1. && y <= self.bottom + 1.
    }
}

/// A page's link annotations that go somewhere the book can follow.
fn page_links(document: &Document, page_id: ObjectId, pages: &HashMap<ObjectId, usize>) -> Vec<PageLink> {
    let Ok(crop) = super::metadata::effective_crop_bounds(document, page_id) else { return Vec::new() };
    let annotations = document.get_dictionary(page_id).ok().and_then(|page| page.get(b"Annots").ok()).and_then(|value| resolve(document, value).ok()).and_then(|value| value.as_array().ok());
    annotations.into_iter().flatten().filter_map(|annotation| {
        let annotation = resolve(document, annotation).ok()?.as_dict().ok()?;
        if annotation.get(b"Subtype").ok()?.as_name().ok()? != b"Link" { return None; }
        let rect = resolve(document, annotation.get(b"Rect").ok()?).ok()?.as_array().ok()?;
        let values = rect.iter().map(|value| resolve(document, value).ok()?.as_float().ok().filter(|value| value.is_finite())).collect::<Option<Vec<_>>>()?;
        let [x1, y1, x2, y2] = values[..] else { return None };
        let target = link_target(document, annotation, pages)?;
        Some(PageLink { left: x1.min(x2) - crop[0], top: crop[3] - y1.max(y2), right: x1.max(x2) - crop[0], bottom: crop[3] - y1.min(y2), target })
    }).collect()
}

/// A bookmark that opens a page: its depth from 1, its title and the page, counted from 0.
#[derive(Debug, Clone, PartialEq)]
struct Bookmark { level: usize, title: String, page: usize }

/// The PDF's bookmarks in reading order. A bookmark that doesn't open a page of this PDF is left
/// out and its children stay; one without a title is named after its page.
fn bookmarks(document: &Document, pages: &HashMap<ObjectId, usize>) -> Vec<Bookmark> {
    let outlines = document.catalog().ok().and_then(|catalog| catalog.get(b"Outlines").ok()).and_then(|value| resolve(document, value).ok()).and_then(|value| value.as_dict().ok());
    let mut found = Vec::new();
    walk_bookmarks(document, outlines.and_then(|outlines| outlines.get(b"First").ok()), 1, pages, &mut found, &mut HashSet::new());
    found
}

fn walk_bookmarks(document: &Document, first: Option<&Object>, level: usize, pages: &HashMap<ObjectId, usize>, found: &mut Vec<Bookmark>, seen: &mut HashSet<ObjectId>) {
    if level > BOOKMARK_MAX_DEPTH { return; }
    let mut next = first.and_then(|value| value.as_reference().ok());
    while let Some(id) = next {
        if found.len() >= BOOKMARK_MAX_COUNT || !seen.insert(id) { return; }
        let Ok(item) = document.get_dictionary(id) else { return };
        if let Some(Target::Page(page)) = link_target(document, item, pages) {
            let title = item.get(b"Title").ok().and_then(|title| resolve(document, title).ok()).and_then(document_info::decode_text).map(|title| clean(&title));
            found.push(Bookmark { level, title: title.filter(|title| !title.is_empty()).unwrap_or_else(|| format!("Page {}", page + 1)), page });
        }
        walk_bookmarks(document, item.get(b"First").ok(), level + 1, pages, found, seen);
        next = item.get(b"Next").ok().and_then(|value| value.as_reference().ok());
    }
}

/// Linked text: bytes `start..end` of a text go to `target`.
#[derive(Debug, Clone, PartialEq)]
struct Span { start: usize, end: usize, target: Target }

/// Adds a link over `start..end`, extending the last one when it goes to the same place and at most a space lies between.
fn add_link(links: &mut Vec<Span>, start: usize, end: usize, target: &Target) {
    if start >= end { return; }
    match links.last_mut() {
        Some(last) if last.target == *target && last.end + 1 >= start => last.end = end,
        _ => links.push(Span { start, end, target: target.clone() }),
    }
}

/// A block's text with its links, and where each page's text starts in it: `(page, byte offset)`.
#[derive(Debug, Clone, Default, PartialEq)]
struct Text { text: String, links: Vec<Span>, pages: Vec<(usize, usize)> }

impl Text {
    #[cfg(test)]
    fn plain(text: &str) -> Self { Self { text: text.into(), ..Self::default() } }

    /// Drops soft hyphens, moving the links and page starts with the text.
    fn without_soft_hyphens(self) -> Self {
        if !self.text.contains('\u{ad}') { return self; }
        let mut text = String::with_capacity(self.text.len());
        let mut moved = Vec::with_capacity(self.text.len() + 1);
        for character in self.text.chars() {
            moved.extend(std::iter::repeat_n(text.len(), character.len_utf8()));
            if character != '\u{ad}' { text.push(character); }
        }
        moved.push(text.len());
        let links = self.links.into_iter().map(|link| Span { start: moved[link.start], end: moved[link.end], target: link.target }).filter(|link| link.start < link.end).collect();
        let pages = self.pages.into_iter().map(|(page, offset)| (page, moved[offset])).collect();
        Self { text, links, pages }
    }
}

/// A line being assembled from runs that share a baseline band. `bold` holds when every run is bold;
/// `spaced` when the last run ended in white space, which the line keeps as one space before the next run.
#[derive(Debug, Clone)]
struct Line { text: String, links: Vec<Span>, spaced: bool, top: f32, bottom: f32, left: f32, right: f32, size: f32, bold: bool }

impl Line {
    fn start(run: &PageTextRun, text: String, target: Option<&Target>) -> Self {
        let links = target.map(|target| vec![Span { start: 0, end: text.len(), target: target.clone() }]).unwrap_or_default();
        Self { text, links, spaced: run.text.ends_with(char::is_whitespace), top: run.y, bottom: run.y + run.height, left: run.x, right: run.x + run.width, size: run.font_size, bold: run.bold }
    }

    /// A run continues this line when its middle sits inside the line's band and it doesn't jump back left.
    fn takes(&self, run: &PageTextRun) -> bool {
        let middle = run.y + run.height / 2.;
        middle >= self.top && middle <= self.bottom && run.x + run.font_size >= self.right
    }

    fn push(&mut self, run: &PageTextRun, text: &str, target: Option<&Target>) {
        let gap = run.x - self.right;
        if gap > run.font_size * 0.15 || self.spaced || run.text.starts_with(char::is_whitespace) { self.text.push(' '); }
        let start = self.text.len();
        self.text.push_str(text);
        if let Some(target) = target { add_link(&mut self.links, start, self.text.len(), target); }
        self.spaced = run.text.ends_with(char::is_whitespace);
        self.top = self.top.min(run.y);
        self.bottom = self.bottom.max(run.y + run.height);
        self.left = self.left.min(run.x);
        self.right = self.right.max(run.x + run.width);
        self.size = self.size.max(run.font_size);
        self.bold &= run.bold;
    }
}

/// Groups a page's runs into lines in content order, each run cleaned of white space XML can't
/// carry and linked when it sits inside a link's area. Runs without a reliable Unicode mapping are left out.
fn page_lines(page: &PageText, links: &[PageLink]) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    for run in page.runs.iter().filter(|run| run.reliable_unicode) {
        let text = clean(&run.text);
        if text.is_empty() { continue; }
        let target = links.iter().find(|link| link.holds(run)).map(|link| &link.target);
        match lines.last_mut() {
            Some(line) if line.takes(run) => line.push(run, &text, target),
            _ => lines.push(Line::start(run, text, target)),
        }
    }
    lines
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

/// Text the book reads in order: a heading with its style, or a paragraph.
#[derive(Debug, Clone, PartialEq)]
struct Block { heading: Option<Style>, body: Text }

/// What the book reads in order: a block of text, or an image by its index among the book's images.
#[derive(Debug, Clone, PartialEq)]
enum Piece { Block(Block), Figure(usize) }

/// A block being assembled from lines.
struct Draft { heading: Option<Style>, body: Text, lines: usize, page: usize, top: f32, bottom: f32, left: f32, right: f32, widest: f32, size: f32 }

impl Draft {
    fn start(heading: Option<Style>, page: usize, line: &Line) -> Self {
        let body = Text { text: line.text.clone(), links: line.links.clone(), pages: Vec::new() };
        Self { heading, body, lines: 1, page, top: line.top, bottom: line.bottom, left: line.left, right: line.right, widest: line.right, size: line.size }
    }

    /// The next line on the same page continues this block when it sits just below the last line
    /// (a gap of at most half a line), at a similar size, and doesn't start a new paragraph after
    /// a finished sentence by being indented or by following a short last line.
    fn continues_on_page(&self, line: &Line) -> bool {
        let size = self.size.max(line.size);
        let below = line.top >= self.top && line.top - self.bottom <= size * 0.5 && (line.size - self.size).abs() <= size * 0.2;
        let new_paragraph = ends_sentence(&self.body.text) && (line.left > self.left + size * 0.8 || self.right < self.widest - size * 3.);
        below && !new_paragraph
    }

    /// A paragraph continues on a later page when it ended mid-word with a hyphen, or mid-sentence
    /// and the next page starts in lower case.
    fn continues_across_pages(&self, line: &Line) -> bool {
        self.heading.is_none() && (ends_with_break(&self.body.text) || (!ends_sentence(&self.body.text) && line.text.starts_with(char::is_lowercase)))
    }

    /// Joins the line on and returns where its text starts. A hyphen dropped by the join leaves the links before it.
    fn push(&mut self, page: usize, line: &Line, words: &HashSet<String>) -> usize {
        let before = self.body.text.len();
        join(&mut self.body.text, &line.text, words);
        let offset = self.body.text.len() - line.text.len();
        let kept = before.min(offset);
        for link in &mut self.body.links { link.end = link.end.min(kept); }
        self.body.links.retain(|link| link.start < link.end);
        for (_, start) in &mut self.body.pages { *start = (*start).min(kept); }
        for link in &line.links { add_link(&mut self.body.links, offset + link.start, offset + link.end, &link.target); }
        self.lines += 1;
        if page != self.page { self.widest = line.right; self.left = line.left; }
        self.page = page;
        self.top = line.top;
        self.bottom = line.bottom;
        self.left = self.left.min(line.left);
        self.right = line.right;
        self.widest = self.widest.max(line.right);
        self.size = line.size;
        offset
    }

    fn finish(self) -> Block {
        let body = self.body.without_soft_hyphens();
        let heading = self.heading.filter(|_| self.lines <= HEADING_MAX_LINES && body.text.chars().count() <= HEADING_MAX_CHARS);
        Block { heading, body }
    }
}

/// Builds the book's blocks from every page's lines in order. The body style is the size and weight
/// carrying the most characters; a line is in a heading style when it is at least 15% larger than
/// the body, or bold at about body size or larger when the body isn't bold. Lines of one style join
/// into blocks, and paragraphs continue across page breaks. Every page is marked where its text
/// starts; a page without text is marked where the next text starts, or at the end of the book.
///
/// `figures` holds each page's images by their top. An image goes before the first block that starts
/// after the line it sits above, so it never splits a paragraph; images left after the last block close the book.
fn blocks(pages: &[Vec<Line>], figures: &[Vec<(f32, usize)>]) -> Vec<Piece> {
    let mut weights: BTreeMap<Style, usize> = BTreeMap::new();
    for line in pages.iter().flatten() { *weights.entry(style(line)).or_default() += line.text.chars().count(); }
    let Some((&(body_size, body_bold), _)) = weights.iter().max_by_key(|(_, weight)| **weight) else {
        return figures.iter().flatten().map(|&(_, image)| Piece::Figure(image)).collect();
    };
    let body_size = body_size as f32 / 2.;
    let heading = |line: &Line| (line.size >= body_size * 1.15 || (line.bold && !body_bold && line.size >= body_size * 0.95)).then(|| style(line));
    let words = inline_words(pages);
    let mut drafts: Vec<Draft> = Vec::new();
    // Each draft's index among the pieces, and the images waiting for the next draft.
    let mut order: Vec<Result<usize, usize>> = Vec::new();
    let mut waiting: Vec<usize> = Vec::new();
    let mut unmarked = 0;
    for (page, lines) in pages.iter().enumerate() {
        let mut page_figures = figures.get(page).cloned().unwrap_or_default();
        page_figures.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut page_figures = page_figures.into_iter().peekable();
        for line in lines {
            while let Some((_, image)) = page_figures.next_if(|(top, _)| *top <= line.top) { waiting.push(image); }
            let kind = heading(line);
            let continues = drafts.last().is_some_and(|draft| draft.heading == kind && if draft.page == page { draft.continues_on_page(line) } else { draft.continues_across_pages(line) });
            let offset = match drafts.last_mut() {
                Some(draft) if continues => draft.push(page, line, &words),
                _ => {
                    order.extend(waiting.drain(..).map(Err));
                    order.push(Ok(drafts.len()));
                    drafts.push(Draft::start(kind, page, line));
                    0
                }
            };
            let draft = drafts.last_mut().expect("a draft was just pushed or extended");
            draft.body.pages.extend((unmarked..=page).map(|marked| (marked, offset)));
            unmarked = unmarked.max(page + 1);
        }
        waiting.extend(page_figures.map(|(_, image)| image));
    }
    order.extend(waiting.into_iter().map(Err));
    if let Some(draft) = drafts.last_mut() {
        let end = draft.body.text.len();
        draft.body.pages.extend((unmarked..pages.len()).map(|marked| (marked, end)));
    }
    let mut blocks = drafts.into_iter().map(Draft::finish);
    order.into_iter().map(|piece| match piece {
        Ok(_) => Piece::Block(blocks.next().expect("every draft has its place")),
        Err(image) => Piece::Figure(image),
    }).collect()
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

/// A content document's text and images: headings carry their level and an anchor for the navigation,
/// and an image its index among the book's images.
#[derive(Debug, Clone, PartialEq)]
enum Item { Heading { level: usize, anchor: String, body: Text }, Paragraph(Text), Image(usize) }

impl Item {
    fn body(&self) -> Option<&Text> {
        match self { Item::Heading { body, .. } | Item::Paragraph(body) => Some(body), Item::Image(_) => None }
    }
}

/// One spine document.
#[derive(Debug, Clone, PartialEq)]
struct Chapter { number: usize, items: Vec<Item> }

impl Chapter {
    fn title<'a>(&'a self, book: &'a str) -> &'a str {
        self.items.iter().find_map(|item| match item { Item::Heading { body, .. } => Some(body.text.as_str()), _ => None }).unwrap_or(book)
    }
}

/// Gives headings levels and splits the blocks into chapters.
///
/// Heading styles rank by size, then bold before regular, and the ranks become levels 1 to 6. The
/// book splits at headings of the shallowest level that occurs more than once (or the shallowest
/// level when none repeats) and of any level above it, so a lone title doesn't hold the whole book.
/// A heading right after another heading stays in its chapter, and a long chapter continues in a
/// new document at a paragraph.
fn chapters(pieces: Vec<Piece>) -> Vec<Chapter> {
    let headings = || pieces.iter().filter_map(|piece| match piece { Piece::Block(block) => block.heading, Piece::Figure(_) => None });
    let mut styles = headings().collect::<Vec<_>>();
    styles.sort_by_key(|&(size, bold)| (std::cmp::Reverse(size), std::cmp::Reverse(bold)));
    styles.dedup();
    let level = |style: &Style| (styles.iter().position(|known| known == style).unwrap_or(0) + 1).min(6);
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for style in headings() { *counts.entry(level(&style)).or_default() += 1; }
    let split = counts.iter().find(|(_, count)| **count >= 2).or(counts.iter().next()).map_or(0, |(level, _)| *level);

    let mut chapters = vec![Chapter { number: 1, items: Vec::new() }];
    let mut length = 0;
    let mut anchors = 0;
    for piece in pieces {
        let current = chapters.last().expect("there is always a chapter");
        let has_text = current.items.iter().any(|item| matches!(item, Item::Paragraph(_)));
        let item = match piece {
            Piece::Block(Block { heading: Some(style), body }) => { anchors += 1; Item::Heading { level: level(&style), anchor: format!("heading-{anchors}"), body } }
            Piece::Block(Block { heading: None, body }) => Item::Paragraph(body),
            Piece::Figure(image) => Item::Image(image),
        };
        let breaks = match &item {
            Item::Heading { level, .. } => *level <= split && has_text,
            Item::Paragraph(_) => length > DOCUMENT_TEXT_LIMIT,
            Item::Image(_) => false,
        };
        if breaks {
            chapters.push(Chapter { number: chapters.len() + 1, items: Vec::new() });
            length = 0;
        }
        length += item.body().map_or(0, |body| body.text.len());
        chapters.last_mut().expect("there is always a chapter").items.push(item);
    }
    chapters
}

/// The chapter each page's text starts in, by page; a page no block marks stays in the first chapter.
fn page_homes(chapters: &[Chapter], pages: usize) -> Vec<usize> {
    let mut homes = vec![1; pages];
    for chapter in chapters {
        for (page, _) in chapter.items.iter().filter_map(Item::body).flat_map(|body| &body.pages) {
            if let Some(home) = homes.get_mut(*page) { *home = chapter.number; }
        }
    }
    homes
}

/// The table of contents from the headings, nested by level.
fn heading_contents(chapters: &[Chapter]) -> Vec<(usize, String, String)> {
    chapters.iter().flat_map(|chapter| chapter.items.iter().filter_map(move |item| match item {
        Item::Heading { level, anchor, body } => Some((*level, format!("{}#{anchor}", chapter_name(chapter.number)), body.text.clone())),
        _ => None,
    })).collect()
}

fn page_href(page: usize, homes: &[usize]) -> String {
    format!("{}#page-{}", chapter_name(homes.get(page).copied().unwrap_or(1)), page + 1)
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

/// Everything that goes into one EPUB. `contents` holds each table of contents entry's level, link
/// and label; `homes` the chapter each page starts in; `anchors` the pages something links to.
struct Book { title: String, author: Option<String>, identifier: String, chapters: Vec<Chapter>, contents: Vec<(usize, String, String)>, homes: Vec<usize>, anchors: BTreeSet<usize>, cover: Option<Vec<u8>>, images: Vec<BookImage> }

/// A block's text as XHTML: links become `a` elements, to the page's anchor in the book or to the
/// URI as written, and each page something links to gets an anchor where its text starts.
fn inline(body: &Text, book: &Book) -> String {
    let mut marks = body.pages.iter().filter(|(page, _)| book.anchors.contains(page)).peekable();
    let mut out = String::new();
    let mut segment = |out: &mut String, from: usize, to: usize, last: bool| {
        let mut cursor = from;
        while let Some(&&(page, offset)) = marks.peek() {
            if offset > to || (offset == to && !last) { break; }
            let offset = offset.max(cursor);
            out.push_str(&quick_xml::escape::escape(&body.text[cursor..offset]));
            out.push_str(&format!("<span id=\"page-{}\"></span>", page + 1));
            cursor = offset;
            marks.next();
        }
        out.push_str(&quick_xml::escape::escape(&body.text[cursor..to]));
    };
    let mut cursor = 0;
    for link in &body.links {
        segment(&mut out, cursor, link.start, false);
        let href = match &link.target { Target::Page(page) => page_href(*page, &book.homes), Target::Uri(uri) => uri.clone() };
        out.push_str(&format!("<a href=\"{}\">", quick_xml::escape::escape(&href)));
        segment(&mut out, link.start, link.end, false);
        out.push_str("</a>");
        cursor = link.end;
    }
    segment(&mut out, cursor, body.text.len(), true);
    out
}

fn chapter_document(book: &Book, chapter: &Chapter) -> String {
    let items = chapter.items.iter().map(|item| match item {
        Item::Heading { level, anchor, body } => format!("<h{level} id=\"{anchor}\">{}</h{level}>\n", inline(body, book)),
        Item::Paragraph(body) => format!("<p>{}</p>\n", inline(body, book)),
        // The PDF carries no description of the image, so it is marked as having none.
        Item::Image(image) => format!("<figure><img src=\"{IMAGES}/{}\" alt=\"\"/></figure>\n", book.images[*image].name),
    }).collect::<String>();
    xhtml(chapter.title(&book.title), &format!("<section id=\"chapter-{}\">\n{items}</section>\n", chapter.number))
}

fn cover_document(title: &str) -> String {
    xhtml(title, &format!("<section epub:type=\"cover\" id=\"cover\">\n<img src=\"{COVER_IMAGE}\" alt=\"Cover\"/>\n</section>\n"))
}

/// The table of contents, nested by level, or the book's title when there are no entries.
/// A level never goes more than one deeper than the entry before it, so the lists nest validly.
fn nav_document(book: &Book) -> String {
    let mut entries = book.contents.iter().map(|(level, href, label)| (*level, format!("<a href=\"{}\">{}</a>", quick_xml::escape::escape(href), escape(label)))).collect::<Vec<_>>();
    if entries.is_empty() { entries.push((1, format!("<a href=\"{}\">{}</a>", chapter_name(1), escape(&book.title)))); }
    let shallowest = entries.iter().map(|(level, _)| *level).min().unwrap_or(1);
    let mut list = String::from("<ol>\n<li>");
    let mut depth = 0usize;
    for (index, (level, link)) in entries.iter().enumerate() {
        let wanted = if index == 0 { 0 } else { (level - shallowest).min(depth + 1) };
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
    xhtml(&book.title, &format!("<nav epub:type=\"toc\" id=\"toc\">\n<h1>Contents</h1>\n{list}</nav>\n"))
}

fn package_document(book: &Book) -> String {
    let creator = book.author.as_deref().map(|author| format!("<dc:creator>{}</dc:creator>\n", escape(author))).unwrap_or_default();
    let (cover_meta, cover_items, cover_spine) = if book.cover.is_some() {
        (
            "<meta name=\"cover\" content=\"cover-image\"/>\n".to_string(),
            format!("<item id=\"cover-image\" href=\"{COVER_IMAGE}\" media-type=\"image/jpeg\" properties=\"cover-image\"/>\n<item id=\"cover\" href=\"{COVER_PAGE}\" media-type=\"application/xhtml+xml\"/>\n"),
            "<itemref idref=\"cover\"/>\n".to_string(),
        )
    } else {
        Default::default()
    };
    let images = book.images.iter().enumerate().map(|(index, image)| format!("<item id=\"image-{}\" href=\"{IMAGES}/{}\" media-type=\"{}\"/>\n", index + 1, image.name, image.media_type));
    let chapters = book.chapters.iter().map(|chapter| format!("<item id=\"chapter-{0}\" href=\"{1}\" media-type=\"application/xhtml+xml\"/>\n", chapter.number, chapter_name(chapter.number)));
    let items = images.chain(chapters).collect::<String>();
    let spine = book.chapters.iter().map(|chapter| format!("<itemref idref=\"chapter-{}\"/>\n", chapter.number)).collect::<String>();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"book-id\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n<dc:identifier id=\"book-id\">{}</dc:identifier>\n<dc:title>{}</dc:title>\n{creator}<dc:language>{LANGUAGE}</dc:language>\n<meta property=\"dcterms:modified\">{MODIFIED}</meta>\n{cover_meta}</metadata>\n<manifest>\n<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n{cover_items}{items}</manifest>\n<spine>\n{cover_spine}{spine}</spine>\n</package>\n",
        book.identifier,
        escape(&book.title),
    )
}

const CONTAINER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n<rootfiles>\n<rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n</rootfiles>\n</container>\n";

/// Builds the package: `mimetype` first and stored, then the container, package, nav, cover, images and chapters.
/// Images are stored as they are, since JPEG and PNG data is already compressed.
/// Every entry has the same fixed time, permissions and host system, so the bytes match on every platform.
fn package(book: &Book) -> Result<Vec<u8>, String> {
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
    entry(PACKAGE_PATH, deflated, package_document(book).as_bytes())?;
    entry(NAV_PATH, deflated, nav_document(book).as_bytes())?;
    if let Some(cover) = &book.cover {
        entry(&format!("OEBPS/{COVER_IMAGE}"), stored, cover)?;
        entry(&format!("OEBPS/{COVER_PAGE}"), deflated, cover_document(&book.title).as_bytes())?;
    }
    for image in &book.images {
        entry(&format!("OEBPS/{IMAGES}/{}", image.name), stored, &image.data)?;
    }
    for chapter in &book.chapters {
        entry(&format!("OEBPS/{}", chapter_name(chapter.number)), deflated, chapter_document(book, chapter).as_bytes())?;
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
        PageText { runs, has_extractable_text, images: Vec::new() }
    }

    fn paragraph(text: &str) -> (Option<Style>, String) { (None, text.into()) }

    /// Each block's heading style and text.
    fn shapes(pieces: Vec<Piece>) -> Vec<(Option<Style>, String)> {
        pieces.into_iter().filter_map(|piece| match piece { Piece::Block(block) => Some((block.heading, block.body.text)), Piece::Figure(_) => None }).collect()
    }

    fn lines(page: &PageText) -> Vec<Line> { page_lines(page, &[]) }

    #[test]
    fn runs_join_into_lines_and_lines_into_paragraphs() {
        let text = page(vec![
            run("The quick", 72., 100., 12.), run("brown fox", 72. + 9. * 6. + 3., 100., 12.),
            run("jumps over.", 72., 114., 12.),
            run("A new", 72., 150., 12.), run("paragraph", 72. + 5. * 6. + 4., 150., 12.),
            run("Heading", 72., 170., 24.),
        ]);
        assert_eq!(shapes(blocks(&[lines(&text)], &[])), [paragraph("The quick brown fox jumps over."), paragraph("A new paragraph"), (Some((48, false)), "Heading".into())]);
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
        assert_eq!(shapes(blocks(&[lines(&text)], &[])), [
            paragraph("A line that fills the measure ends here."),
            paragraph("Next paragraph starts flush and fills the whole measure."),
            paragraph("Indented one follows."),
        ]);
    }

    #[test]
    fn a_paragraph_continues_on_the_next_page_only_mid_sentence() {
        let pages = [
            lines(&page(vec![run("Body text that stops in the", 72., 100., 12.)])),
            lines(&page(vec![run("middle of a sentence.", 72., 100., 12.), run("Then text with an ending.", 72., 140., 12.)])),
            lines(&page(vec![run("Capitalised start.", 72., 100., 12.)])),
        ];
        assert_eq!(shapes(blocks(&pages, &[])), [
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
        assert_eq!(shapes(blocks(&[lines(&text)], &[])), [paragraph("Bell & <tags>")]);
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
        let chapters = vec![Chapter { number: 1, items: vec![Item::Paragraph(Text::plain("Text"))] }];
        let book = Book { title: "Book".into(), author: None, identifier: identifier(b"x"), chapters, contents: Vec::new(), homes: vec![1], anchors: BTreeSet::new(), cover: None, images: Vec::new() };
        let good = package(&book).unwrap();
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
        assert_eq!(outcome.detail, "3 pages. No bookmarks or headings, so the contents list the title only.");
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
        assert_eq!(outcome.detail, "2 pages. Contents from headings in the text, 6 entries.");
        assert_eq!(outcome.page_details[0], OutcomeDetail { label: "Contents".into(), value: "From headings in the text, 6 entries".into() });
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
        for (name, pages) in [("handbook", &HANDBOOK[..]), ("guide", &FIELD_GUIDE[..]), ("illustrated", &ILLUSTRATED[..])] {
            let input = root.join(format!("{name}.pdf"));
            if name == "illustrated" { illustrated_pdf(&input) } else { pdf_with_pages(&input, pages) }
            let outputs = (0..2).map(|index| {
                let destination = root.join(format!("{name}-{index}"));
                fs::create_dir_all(&destination).unwrap();
                let outcome = convert(&PdfToEpubRequest { paths: vec![input.clone()], output_location: OutputLocation::CustomFolder(destination), remove_headers_and_footers: true, cover: EpubCover::NoCover, title: None, author: None }, input.clone());
                let OutcomeState::Succeeded { output_paths } = &outcome.state else { panic!("{outcome:?}") };
                fs::read(&output_paths[0]).unwrap()
            }).collect::<Vec<_>>();
            assert!(outputs[0] == outputs[1], "{name} differs between runs");
        }
        let _ = fs::remove_dir_all(root);
    }

    fn navigation(epub: &Path) -> Vec<(usize, String, String)> {
        let book = super::super::epub::parse(&fs::read(epub).unwrap(), u64::MAX, ToolError::processing("too big"), 1.).unwrap_or_else(|error| panic!("{error:?}"));
        book.navigation.iter().map(|entry| (entry.level as usize, entry.title.clone(), entry.target.clone())).collect()
    }

    fn names(epub: &Path) -> Vec<String> {
        ZipArchive::new(fs::File::open(epub).unwrap()).unwrap().file_names().map(str::to_string).collect()
    }

    #[test]
    fn bookmarks_become_the_contents_and_open_the_page_they_point_at() {
        use lopdf::{dictionary, Object};
        let root = folder("bookmarks");
        let input = root.join("guide.pdf");
        pdf_with(&input, &FIELD_GUIDE, |document, pages, catalog| {
            let outlines = document.new_object_id();
            let (first, child, second) = (document.new_object_id(), document.new_object_id(), document.new_object_id());
            document.objects.insert(first, dictionary! { "Title" => Object::string_literal("Getting out"), "Parent" => outlines, "Next" => second, "First" => child, "Last" => child, "Count" => 1, "Dest" => vec![pages[0].into(), "Fit".into()] }.into());
            document.objects.insert(child, dictionary! { "Title" => Object::string_literal("At the water"), "Parent" => first, "A" => dictionary! { "S" => "GoTo", "D" => vec![pages[1].into(), "XYZ".into(), Object::Null, Object::Null, Object::Null] } }.into());
            document.objects.insert(second, dictionary! { "Title" => Object::string_literal(""), "Parent" => outlines, "Prev" => first, "Dest" => vec![pages[2].into(), "Fit".into()] }.into());
            document.objects.insert(outlines, dictionary! { "Type" => "Outlines", "First" => first, "Last" => second, "Count" => 3 }.into());
            catalog.set("Outlines", outlines);
        });
        let outcome = convert(&request(&input, true), input);
        let OutcomeState::Succeeded { output_paths } = &outcome.state else { panic!("{outcome:?}") };
        assert_eq!(outcome.detail, "3 pages. Contents from the bookmarks, 3 entries.");
        assert_eq!(outcome.page_details[0], OutcomeDetail { label: "Contents".into(), value: "From the PDF's bookmarks, 3 entries".into() });
        assert_eq!(navigation(&output_paths[0]), [
            (1, "Getting out".into(), "OEBPS/chapter-1.xhtml#page-1".into()),
            (2, "At the water".into(), "OEBPS/chapter-1.xhtml#page-2".into()),
            (1, "Page 3".into(), "OEBPS/chapter-1.xhtml#page-3".into()),
        ]);
        let chapter = entry(&output_paths[0], "OEBPS/chapter-1.xhtml");
        assert!(chapter.contains("<p><span id=\"page-1\"></span>Good birding takes patience"), "{chapter}");
        assert!(chapter.contains("We wait until the <span id=\"page-2\"></span>light fades over the water.</p>"), "{chapter}");
        assert!(chapter.contains("<p><span id=\"page-3\"></span>The last page closes the book.</p>"), "{chapter}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn links_to_pages_open_that_page_in_the_book_and_web_links_are_kept_as_written() {
        use lopdf::{dictionary, Object};
        let root = folder("links");
        let input = root.join("map.pdf");
        let pages = [
            "BT /F1 12 Tf 72 700 Td (Read) Tj ET BT /F1 12 Tf 110 700 Td (the map) Tj ET BT /F1 12 Tf 158 700 Td (first.) Tj ET \
             BT /F1 12 Tf 72 660 Td (Visit) Tj ET BT /F1 12 Tf 110 660 Td (our site) Tj ET BT /F1 12 Tf 160 660 Td (today.) Tj ET",
            "BT /F1 12 Tf 72 700 Td (The map shows the shore.) Tj ET",
        ];
        pdf_with(&input, &pages, |document, pages, _| {
            let link = |rect: [i64; 4], target: (&str, Object)| dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => rect.iter().map(|value| Object::Integer(*value)).collect::<Vec<_>>(), target.0 => target.1 };
            let to_page = document.add_object(link([105, 690, 155, 715], ("Dest", vec![pages[1].into(), "Fit".into()].into())));
            let web = document.add_object(link([105, 650, 152, 675], ("A", dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com/birds?region=north&season=spring") }.into())));
            let script = document.add_object(link([157, 650, 200, 675], ("A", dictionary! { "S" => "URI", "URI" => Object::string_literal("javascript:alert(1)") }.into())));
            document.get_dictionary_mut(pages[0]).unwrap().set("Annots", vec![to_page.into(), web.into(), script.into()]);
        });
        let outcome = convert(&request(&input, false), input);
        let OutcomeState::Succeeded { output_paths } = &outcome.state else { panic!("{outcome:?}") };
        let chapter = entry(&output_paths[0], "OEBPS/chapter-1.xhtml");
        assert!(chapter.contains(
            "<p>Read <a href=\"chapter-1.xhtml#page-2\">the map</a> first.</p>\n\
             <p>Visit <a href=\"https://example.com/birds?region=north&amp;season=spring\">our site</a> today.</p>\n\
             <p><span id=\"page-2\"></span>The map shows the shore.</p>"
        ), "{chapter}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn title_and_author_come_from_the_pdf_unless_the_request_replaces_them() {
        use lopdf::{dictionary, Object};
        let root = folder("details");
        let input = root.join("handbook.pdf");
        pdf_with(&input, &HANDBOOK, |document, _, _| {
            let info = document.add_object(dictionary! { "Title" => Object::string_literal("The Long Road Home"), "Author" => Object::string_literal("Mira Okafor") });
            document.trailer.set("Info", info);
        });
        let from_pdf = convert(&request(&input, true), input.clone());
        let OutcomeState::Succeeded { output_paths } = &from_pdf.state else { panic!("{from_pdf:?}") };
        assert_eq!(output_paths[0].file_name().unwrap(), "handbook.epub", "the file is still named after the PDF");
        let package = entry(&output_paths[0], PACKAGE_PATH);
        assert!(package.contains("<dc:title>The Long Road Home</dc:title>\n<dc:creator>Mira Okafor</dc:creator>\n"), "{package}");
        assert_eq!(from_pdf.page_details[2], OutcomeDetail { label: "Title and author".into(), value: "The Long Road Home, Mira Okafor".into() });

        let destination = root.join("changed");
        fs::create_dir_all(&destination).unwrap();
        let changed = PdfToEpubRequest { output_location: OutputLocation::CustomFolder(destination), title: Some(" Road  Home ".into()), author: Some("  ".into()), ..request(&input, true) };
        let overridden = convert(&changed, input);
        let OutcomeState::Succeeded { output_paths } = &overridden.state else { panic!("{overridden:?}") };
        let package = entry(&output_paths[0], PACKAGE_PATH);
        assert!(package.contains("<dc:title>Road Home</dc:title>\n<dc:creator>Mira Okafor</dc:creator>\n"), "a blank author keeps the PDF's: {package}");
        assert!(entry(&output_paths[0], NAV_PATH).contains("<title>Road Home</title>"));
        assert_eq!(overridden.page_details[2], OutcomeDetail { label: "Title and author".into(), value: "Road Home, Mira Okafor (changed for this book)".into() });
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_pdf_without_details_is_titled_after_its_file_and_has_no_author() {
        let root = folder("untitled");
        let input = root.join("guide.pdf");
        pdf_with_pages(&input, &FIELD_GUIDE);
        let outcome = convert(&request(&input, true), input);
        let OutcomeState::Succeeded { output_paths } = &outcome.state else { panic!("{outcome:?}") };
        let package = entry(&output_paths[0], PACKAGE_PATH);
        assert!(package.contains("<dc:title>guide</dc:title>\n<dc:language>") && !package.contains("dc:creator"), "{package}");
        assert_eq!(outcome.page_details[2].value, "guide");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_first_page_becomes_the_cover_unless_the_request_leaves_it_out() {
        if crate::kit::resources::resolve_helper(&crate::kit::resources::PDF_RENDERER).is_err() { return; }
        let root = folder("cover");
        let input = root.join("handbook.pdf");
        pdf_with_pages(&input, &HANDBOOK);
        let covered = (0..2).map(|index| {
            let destination = root.join(format!("cover-{index}"));
            fs::create_dir_all(&destination).unwrap();
            let outcome = convert(&PdfToEpubRequest { output_location: OutputLocation::CustomFolder(destination), cover: EpubCover::FirstPage, ..request(&input, true) }, input.clone());
            let OutcomeState::Succeeded { output_paths } = &outcome.state else { panic!("{outcome:?}") };
            assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
            assert_eq!(outcome.page_details[1], OutcomeDetail { label: "Cover".into(), value: "Page 1".into() });
            output_paths[0].clone()
        }).collect::<Vec<_>>();
        let package = entry(&covered[0], PACKAGE_PATH);
        assert!(package.contains("<meta name=\"cover\" content=\"cover-image\"/>"), "{package}");
        assert!(package.contains("<item id=\"cover-image\" href=\"cover.jpg\" media-type=\"image/jpeg\" properties=\"cover-image\"/>"), "{package}");
        assert!(package.contains("<spine>\n<itemref idref=\"cover\"/>\n<itemref idref=\"chapter-1\"/>"), "{package}");
        let mut image = Vec::new();
        ZipArchive::new(fs::File::open(&covered[0]).unwrap()).unwrap().by_name("OEBPS/cover.jpg").unwrap().read_to_end(&mut image).unwrap();
        let decoded = image::load_from_memory_with_format(&image, image::ImageFormat::Jpeg).unwrap();
        assert!(decoded.height() > decoded.width(), "a portrait page makes a portrait cover");
        assert!(fs::read(&covered[0]).unwrap() == fs::read(&covered[1]).unwrap(), "the cover is deterministic");

        let plain = convert(&request(&input, true), input);
        let OutcomeState::Succeeded { output_paths } = &plain.state else { panic!("{plain:?}") };
        assert!(!names(&output_paths[0]).iter().any(|name| name.contains("cover")));
        assert!(!entry(&output_paths[0], PACKAGE_PATH).contains("cover"));
        assert_eq!(plain.page_details[1], OutcomeDetail { label: "Cover".into(), value: "None".into() });
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_request_without_the_new_choices_makes_a_cover_and_keeps_the_pdfs_details() {
        let request: PdfToEpubRequest = serde_json::from_str(r#"{"paths":["/a.pdf"],"outputLocation":"alongsideInput"}"#).unwrap();
        assert_eq!((request.cover, request.title, request.author), (EpubCover::FirstPage, None, None));
        let request: PdfToEpubRequest = serde_json::from_str(r#"{"paths":["/a.pdf"],"outputLocation":"alongsideInput","cover":"none","title":"T","author":"A"}"#).unwrap();
        assert_eq!((request.cover, request.title.as_deref(), request.author.as_deref()), (EpubCover::NoCover, Some("T"), Some("A")));
    }

    /// A small RGB JPEG, as a camera or scanner would embed it.
    fn jpeg() -> Vec<u8> {
        let pixels = (0..8 * 8).flat_map(|index| [(index * 4) as u8, 128, 255 - (index * 4) as u8]).collect::<Vec<_>>();
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90).encode(&pixels, 8, 8, image::ExtendedColorType::Rgb8).unwrap();
        bytes
    }

    fn jpeg_object(document: &mut Document, data: Vec<u8>) -> ObjectId {
        use lopdf::{dictionary, Stream};
        document.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => 8, "Height" => 8, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode" }, data))
    }

    /// Gives a page of a `pdf_with` document these image XObjects.
    fn page_images(document: &mut Document, page: ObjectId, images: &[(&str, ObjectId)]) {
        let mut xobjects = Dictionary::new();
        for (name, id) in images { xobjects.set(*name, *id); }
        let resources = document.get_dictionary_mut(page).unwrap().get_mut(b"Resources").unwrap().as_dict_mut().unwrap();
        resources.set("XObject", xobjects);
    }

    /// A paragraph, a JPEG drawn below it, and a paragraph below the JPEG.
    const ILLUSTRATED: [&str; 1] = [
        "BT /F1 12 Tf 72 700 Td (Before the picture.) Tj ET q 200 0 0 100 72 560 cm /Im1 Do Q BT /F1 12 Tf 72 520 Td (After the picture.) Tj ET",
    ];

    fn illustrated_pdf(path: &Path) {
        pdf_with(path, &ILLUSTRATED, |document, pages, _| {
            let image = jpeg_object(document, jpeg());
            page_images(document, pages[0], &[("Im1", image)]);
        });
    }

    fn bytes_of(epub: &Path, name: &str) -> Vec<u8> {
        let mut archive = ZipArchive::new(fs::File::open(epub).unwrap()).unwrap();
        let mut bytes = Vec::new();
        archive.by_name(name).unwrap_or_else(|error| panic!("{name}: {error}")).read_to_end(&mut bytes).unwrap();
        bytes
    }

    fn image_names(epub: &Path) -> Vec<String> {
        names(epub).into_iter().filter(|name| name.starts_with("OEBPS/images/")).collect()
    }

    fn page_detail(outcome: &JobOutcome, label: &str) -> Option<String> {
        outcome.page_details.iter().find(|detail| detail.label == label).map(|detail| detail.value.clone())
    }

    #[test]
    fn an_embedded_jpeg_is_kept_byte_for_byte_between_the_text_around_it() {
        let root = folder("jpeg");
        let input = root.join("illustrated.pdf");
        illustrated_pdf(&input);
        let outcome = convert(&request(&input, true), input);
        let OutcomeState::Succeeded { output_paths } = &outcome.state else { panic!("{outcome:?}") };
        let chapter = entry(&output_paths[0], "OEBPS/chapter-1.xhtml");
        assert!(chapter.contains("<p>Before the picture.</p>\n<figure><img src=\"images/image-1.jpg\" alt=\"\"/></figure>\n<p>After the picture.</p>"), "{chapter}");
        assert_eq!(bytes_of(&output_paths[0], "OEBPS/images/image-1.jpg"), jpeg());
        let package = entry(&output_paths[0], PACKAGE_PATH);
        assert!(package.contains("<item id=\"image-1\" href=\"images/image-1.jpg\" media-type=\"image/jpeg\"/>"), "{package}");
        assert_eq!(outcome.detail, "1 page, 1 image. No bookmarks or headings, so the contents list the title only.");
        assert_eq!(page_detail(&outcome, "Images kept").as_deref(), Some("1 of 1"));
        assert_eq!(outcome.page_details[0].label, "Images kept");
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_image_used_again_or_with_the_same_content_is_stored_once() {
        let root = folder("duplicates");
        let input = root.join("repeated.pdf");
        pdf_with(&input, &[
            "BT /F1 12 Tf 72 700 Td (A logo on the first page.) Tj ET q 50 0 0 50 72 600 cm /Im1 Do Q \
             BT /F1 12 Tf 72 560 Td (Then a small grey swatch.) Tj ET q 30 0 0 30 72 500 cm BI /W 2 /H 2 /CS /G /BPC 8 ID abcd EI Q \
             BT /F1 12 Tf 72 460 Td (The end of the first page.) Tj ET",
            "BT /F1 12 Tf 72 700 Td (The same logo again.) Tj ET q 50 0 0 50 72 600 cm /Im1 Do Q \
             BT /F1 12 Tf 72 560 Td (And a copy of it.) Tj ET q 50 0 0 50 72 480 cm /Im2 Do Q \
             BT /F1 12 Tf 72 440 Td (The end of the book.) Tj ET",
        ], |document, pages, _| {
            let logo = jpeg_object(document, jpeg());
            let copy = jpeg_object(document, jpeg());
            page_images(document, pages[0], &[("Im1", logo)]);
            page_images(document, pages[1], &[("Im1", logo), ("Im2", copy)]);
        });
        let outcome = convert(&request(&input, true), input);
        let OutcomeState::Succeeded { output_paths } = &outcome.state else { panic!("{outcome:?}") };
        assert_eq!(image_names(&output_paths[0]), ["OEBPS/images/image-1.jpg", "OEBPS/images/image-2.png"]);
        let chapter = entry(&output_paths[0], "OEBPS/chapter-1.xhtml");
        assert_eq!(chapter.matches("images/image-1.jpg").count(), 3, "{chapter}");
        assert_eq!(chapter.matches("images/image-2.png").count(), 1, "{chapter}");
        let swatch = image::load_from_memory(&bytes_of(&output_paths[0], "OEBPS/images/image-2.png")).unwrap().to_luma8();
        assert_eq!(swatch.into_raw(), b"abcd", "the inline image is decoded losslessly");
        assert_eq!(outcome.detail, "2 pages, 2 images. No bookmarks or headings, so the contents list the title only.");
        assert_eq!(page_detail(&outcome, "Images kept").as_deref(), Some("2 of 2"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_image_that_cannot_be_decoded_is_named_and_the_rest_of_the_book_converts() {
        use lopdf::{dictionary, Stream};
        let root = folder("damaged");
        let input = root.join("damaged.pdf");
        pdf_with(&input, &[
            "BT /F1 12 Tf 72 700 Td (Text before both pictures.) Tj ET q 50 0 0 50 72 600 cm /Im1 Do Q q 50 0 0 50 72 500 cm /Im2 Do Q \
             BT /F1 12 Tf 72 460 Td (Text after both pictures.) Tj ET",
        ], |document, pages, _| {
            let good = jpeg_object(document, jpeg());
            let damaged = document.add_object(Stream::new(
                dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => 2, "Height" => 2, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8, "Filter" => "FlateDecode" },
                b"this is not zlib data".to_vec(),
            ));
            page_images(document, pages[0], &[("Im1", good), ("Im2", damaged)]);
        });
        let outcome = convert(&request(&input, true), input);
        let OutcomeState::Succeeded { output_paths } = &outcome.state else { panic!("{outcome:?}") };
        assert_eq!(image_names(&output_paths[0]), ["OEBPS/images/image-1.jpg"]);
        let chapter = entry(&output_paths[0], "OEBPS/chapter-1.xhtml");
        assert!(chapter.contains("<p>Text before both pictures.</p>\n<figure><img src=\"images/image-1.jpg\" alt=\"\"/></figure>\n<p>Text after both pictures.</p>"), "{chapter}");
        assert_eq!(outcome.detail, "1 page, 1 of 2 images. 1 image couldn't be read.");
        assert_eq!(page_detail(&outcome, "Images kept").as_deref(), Some("1 of 2"));
        let warning = &outcome.warnings[0];
        assert_eq!(warning.kind, WarningKind::ImageDamaged { page: 1, image: 2 });
        assert_eq!(warning.message, "Page 1, image 2 — The image data is damaged. Left out; the rest of the book converted.");
        let _ = fs::remove_dir_all(root);
    }

    fn request(input: &Path, remove_headers_and_footers: bool) -> PdfToEpubRequest {
        PdfToEpubRequest { paths: vec![input.to_path_buf()], output_location: OutputLocation::AlongsideInput, remove_headers_and_footers, cover: EpubCover::NoCover, title: None, author: None }
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
    fn pdf_with_pages(path: &Path, pages: &[&str]) { pdf_with(path, pages, |_, _, _| {}); }

    /// Like `pdf_with_pages`, with `extra` given the document, its page ids and the catalog before saving.
    fn pdf_with(path: &Path, pages: &[&str], extra: impl FnOnce(&mut Document, &[ObjectId], &mut Dictionary)) {
        use lopdf::{dictionary, Object, Stream};
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let regular = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
        let bold = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica-Bold", "Encoding" => "WinAnsiEncoding" });
        let ids = pages.iter().map(|content| {
            let content_id = document.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
            document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => regular, "F2" => bold } }, "Contents" => content_id,
            })
        }).collect::<Vec<_>>();
        let kids = ids.iter().map(|id| Object::Reference(*id)).collect::<Vec<_>>();
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages.len() as i64 }.into());
        let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
        extra(&mut document, &ids, &mut catalog);
        let catalog_id = document.add_object(catalog);
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }
}
