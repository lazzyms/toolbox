//! Formatted PDF: a document typed in Toolbox, sent as ordered blocks rather than markup, laid out into a new PDF.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::kit::common::limits::{self, ToolLimits};
use crate::kit::common::{note_renamed, OutputLocation, OutputNaming};
use crate::kit::contracts::{JobOutcome, LimitKind, ToolError};
use crate::kit::images::source::{self, ImageFamily};
use crate::kit::pdf::convert::{page_count, Margins, Orientation, PageSize};
use crate::kit::pdf::font_resolver::{FontResolver, SystemFontLookup};
use crate::kit::pdf::layout::{assemble, Align, Assembly, Block, FontBook, Flow, Geometry, Paragraph, Picture, Span, Style, Theme, PAGE_COUNT, PAGE_NUMBER};
use crate::packs::uninstall::RunPacks;

pub const COMMAND: &str = "create_formatted_pdf";
/// Headings and paragraphs have no code, but the theme names a family for it.
const CODE_FAMILY: &str = "Liberation Mono";
const FONT_SIZES: std::ops::RangeInclusive<f32> = 6.0..=72.0;
/// Header, footer and page numbers are drawn small and grey, in the document's font.
const BAND_SIZE: f32 = 9.;
const BAND_LEADING: f32 = 1.3;
const BAND_INK: [u8; 3] = [0x66, 0x66, 0x66];
/// How far into the margin, as a share of it, the header and footer sit from the page edge.
const BAND_INSET: f32 = 0.4;
/// The least space between a header or footer and the text.
const BAND_GAP: f32 = 6.;
/// The most a table may hold, checked when the request arrives.
const MAX_TABLE_COLUMNS: usize = 20;
const MAX_TABLE_ROWS: usize = 1000;
const MAX_CELL_CHARACTERS: usize = 1000;

/// A stretch of paragraph or list text in one style.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListItem {
    pub runs: Vec<Run>,
}

/// One block of the document. Headings are drawn bold, so they carry plain text; paragraphs and list items carry runs.
/// A table carries its rows of plain cell text, and with `header_row` its first row is the header. An image carries the
/// path of a file on this device, read when the PDF is saved.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum DocumentBlock {
    Heading1 { text: String },
    Heading2 { text: String },
    Heading3 { text: String },
    Paragraph { runs: Vec<Run> },
    BulletedList { items: Vec<ListItem> },
    NumberedList { items: Vec<ListItem> },
    Table { header_row: bool, rows: Vec<Vec<String>> },
    Image { path: PathBuf },
    PageBreak,
}

impl DocumentBlock {
    fn text_bytes(&self) -> usize {
        let runs = |runs: &[Run]| runs.iter().map(|run| run.text.len()).sum::<usize>();
        match self {
            Self::Heading1 { text } | Self::Heading2 { text } | Self::Heading3 { text } => text.len(),
            Self::Paragraph { runs: paragraph } => runs(paragraph),
            Self::BulletedList { items } | Self::NumberedList { items } => items.iter().map(|item| runs(&item.runs)).sum(),
            Self::Table { rows, .. } => rows.iter().flatten().map(String::len).sum(),
            Self::Image { path } => path.as_os_str().len(),
            Self::PageBreak => 0,
        }
    }

    /// Refuses a table that is not a grid of rows with the same number of cells, or that holds more than a table may,
    /// and an image that is not a file path on this device or not a kind of image a PDF can hold.
    fn check(&self) -> Result<(), ToolError> {
        if let Self::Image { path } = self { return check_image_path(path); }
        let Self::Table { rows, .. } = self else { return Ok(()) };
        let columns = rows.first().map_or(0, Vec::len);
        if rows.is_empty() || columns == 0 { return Err(ToolError::invalid_input("A table needs at least one row and one column.")); }
        if rows.iter().any(|row| row.len() != columns) { return Err(ToolError::invalid_input("Every row of a table needs the same number of cells.")); }
        if columns > MAX_TABLE_COLUMNS || rows.len() > MAX_TABLE_ROWS {
            return Err(ToolError::limit_exceeded(
                LimitKind::Dimensions,
                format!("A table can have up to {MAX_TABLE_COLUMNS} columns and {MAX_TABLE_ROWS} rows. Split it into smaller tables and try again."),
            ));
        }
        if rows.iter().flatten().any(|cell| cell.chars().count() > MAX_CELL_CHARACTERS) {
            return Err(ToolError::limit_exceeded(
                LimitKind::TextLength,
                format!("A table cell can hold up to {MAX_CELL_CHARACTERS} characters. Shorten the cell and try again."),
            ));
        }
        Ok(())
    }
}

/// The name an image is shown by in messages: its file name.
fn image_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// Images are only read from this device, so a web address, or any path that is not a full path, is refused.
fn check_image_path(path: &Path) -> Result<(), ToolError> {
    if !path.is_absolute() {
        return Err(ToolError::invalid_input(format!("Images are read from this device only, so {} can't be added. Save the image to your disk and choose it there.", path.display())));
    }
    if ImageFamily::of(path) == ImageFamily::Other {
        return Err(ToolError::invalid_input(format!("{} can't be added. Choose a PNG, JPEG, WebP, HEIC, HEIF or TIFF image.", image_name(path))));
    }
    Ok(())
}

/// Reads an image block's file as the PDF is saved, turned upright by its orientation tag. A TIFF gives its first page.
fn load_picture(path: &Path) -> Result<Picture, ToolError> {
    let name = image_name(path);
    if !path.is_file() {
        return Err(ToolError::invalid_input(format!("{name} is missing. It was at {}. Nothing was written. Choose the image again or remove its block, then save.", path.display())));
    }
    limits::SIGNATURE_IMAGE.check(path).map_err(|error| ToolError { message: format!("{name} can't be added. {}", error.message), ..error })?;
    let decoded = source::decode(path).map_err(|error| ToolError::invalid_input(format!("{name} can't be read as an image: {error}")))?;
    let orientation = decoded.orientation;
    let mut image = decoded.into_first_page();
    if let Some(orientation) = orientation { image.apply_orientation(orientation); }
    Picture::from_image(image, None).map_err(|_| ToolError::limit_exceeded(
        LimitKind::Dimensions,
        format!("{name} is too large to place in a PDF. The limit is 40 megapixels. Make it smaller and try again."),
    ))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentFont {
    pub family: String,
    pub size: f32,
}

/// How every page looks: its size, orientation and margins, and the header, footer and page numbers drawn on it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageSettings {
    pub page_size: PageSize,
    pub orientation: Orientation,
    pub margins: Margins,
    pub header: String,
    pub footer: String,
    pub page_numbers: bool,
}

impl PageSettings {
    /// The media box width and height, in points.
    fn size(&self) -> (f32, f32) {
        let (width, height) = self.page_size.points();
        match self.orientation { Orientation::Portrait => (width, height), Orientation::Landscape => (height, width) }
    }

    fn header_blocks(&self) -> Vec<Block> {
        band_line(&self.header, Align::Left).into_iter().collect()
    }

    /// The footer text, then the page number on its own line; `numbers` stands in for the page number and count.
    fn footer_blocks(&self, numbers: (&str, &str)) -> Vec<Block> {
        let page = self.page_numbers.then(|| format!("Page {} of {}", numbers.0, numbers.1));
        band_line(&self.footer, Align::Left).into_iter().chain(page.and_then(|page| band_line(&page, Align::Center))).collect()
    }

    fn label(&self) -> String {
        let size = page_size_label(self.page_size);
        match self.orientation { Orientation::Portrait => size.to_string(), Orientation::Landscape => format!("{size} landscape") }
    }
}

/// One line of a header or footer, or nothing for blank text.
fn band_line(text: &str, align: Align) -> Option<Block> {
    let text = text.trim();
    if text.is_empty() { return None; }
    let style = Style { color: Some(BAND_INK), ..Style::default() };
    Some(Block::Formatted(Paragraph {
        spans: vec![Span { text: text.to_string(), style, link: None }],
        align, indent: 0., right_indent: 0., before: 0., after: 0., size: BAND_SIZE, leading: BAND_LEADING,
    }))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormattedDocument {
    pub title: String,
    pub author: String,
    pub blocks: Vec<DocumentBlock>,
    pub page: PageSettings,
    pub font: DocumentFont,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormattedPdfRequest {
    pub document: FormattedDocument,
    pub output_path: PathBuf,
}

pub fn limits() -> ToolLimits { limits::CREATED_DOCUMENT }

/// Saves the document at the chosen path, or a numbered name beside it when that name is taken.
pub fn create(request: &FormattedPdfRequest, packs: &RunPacks) -> JobOutcome {
    // Only bundled fonts, so the same document makes the same PDF on every computer.
    let outcome = match FontResolver::bundled() {
        Ok(fonts) => create_with(request, &fonts, packs),
        Err(error) => JobOutcome::failure(request.output_path.clone(), error),
    };
    note_renamed(outcome, Some(&request.output_path))
}

fn create_with<S: SystemFontLookup>(request: &FormattedPdfRequest, fonts: &FontResolver<S>, packs: &RunPacks) -> JobOutcome {
    let result = (|| {
        let document = &request.document;
        document.blocks.iter().try_for_each(DocumentBlock::check)?;
        let (blocks, images) = layout_blocks(&document.blocks);
        if blocks.is_empty() { return Err(ToolError::invalid_input("The document is empty. Add some text and try again.")); }
        if document.font.family.trim().is_empty() { return Err(ToolError::invalid_input("Choose a font for the document.")); }
        if !document.font.size.is_finite() || !FONT_SIZES.contains(&document.font.size) {
            return Err(ToolError::invalid_input(format!("Text size must be between {} and {} pt.", FONT_SIZES.start(), FONT_SIZES.end())));
        }
        let limits = limits();
        let page = &document.page;
        let bytes = document.title.len() + document.author.len() + page.header.len() + page.footer.len() + document.blocks.iter().map(DocumentBlock::text_bytes).sum::<usize>();
        if bytes as u64 > limits.max_bytes {
            return Err(ToolError::limit_exceeded(
                LimitKind::FileSize,
                format!("The document is larger than the {} limit for one PDF. Split it into smaller documents and try again.", limits::bytes_label(limits.max_bytes)),
            ));
        }

        // Each image file is read once, now, so a missing one fails the save before anything is written.
        let pictures = images.iter().map(|path| load_picture(path)).collect::<Result<Vec<_>, _>>()?;

        let size = page.size();
        let theme = Theme::new(&document.font.family, CODE_FAMILY, document.font.size, false);
        let margin = page.margins.points();
        let inset = margin * BAND_INSET;
        let header = page.header_blocks();
        let mut book = FontBook::new(fonts, packs);
        // The text starts below the header and ends above the footer, even where a long one reaches past the margin.
        let geometry = {
            let mut measure = Flow::new(&mut book, theme.clone(), &[], Geometry::uniform(size.0, size.1, margin));
            let mut edge = |blocks: &[Block], name: &str| -> Result<f32, ToolError> {
                if blocks.is_empty() { return Ok(margin); }
                let reach = inset + measure.band_height(blocks)? + BAND_GAP;
                if reach > size.1 / 4. {
                    return Err(ToolError::invalid_input(format!("The {name} is too long to fit on the page. Shorten it and try again.")));
                }
                Ok(margin.max(reach))
            };
            let top = edge(&header, "header")?;
            let bottom = edge(&page.footer_blocks(("9999", "9999")), "footer")?;
            Geometry { top, bottom, ..Geometry::uniform(size.0, size.1, margin) }
        };
        let mut flow = Flow::new(&mut book, theme, &pictures, geometry);
        flow.blocks(&blocks)?;
        let footer = page.footer_blocks((&PAGE_NUMBER.to_string(), &PAGE_COUNT.to_string()));
        flow.bands(&header, &footer, inset, inset)?;
        let laid = flow.finish();
        let count = laid.pages.len();
        limits.check_made_pages(count)?;
        let mut pdf = assemble(&book, Assembly {
            size, pages: laid.pages, pictures: &pictures, bookmarks: &laid.bookmarks, anchors: &laid.anchors,
            title: Some(&document.title), author: Some(&document.author),
        })?;

        let output = OutputNaming::reserve_single_output(Some(&request.output_path), &request.output_path, &OutputLocation::AlongsideInput, "", "pdf")
            .map_err(|error| ToolError::invalid_input(format!("Could not save the PDF there: {error}")))?;
        pdf.save(output.path()).map_err(|error| ToolError::processing(format!("Could not write the PDF: {error}")))?;
        let path = output.publish().map_err(|error| ToolError::processing(format!("Could not save the PDF: {error}")))?;
        let detail = format!("{}, {}.", page_count(count), page.label());
        Ok(JobOutcome::succeeded(request.output_path.clone(), vec![path], detail).with_fonts(book.reports()))
    })();
    result.unwrap_or_else(|error| JobOutcome::failure(request.output_path.clone(), error))
}

/// The document's blocks for layout, in order. Blank blocks, list items and table rows are left out, and so are page breaks
/// with nothing before or after them. A numbered list that follows another across page breaks continues its numbering.
/// The image files come back in the order the image blocks first name them, each once; `Block::Image` indexes them.
fn layout_blocks(blocks: &[DocumentBlock]) -> (Vec<Block>, Vec<&Path>) {
    let mut laid: Vec<Block> = Vec::new();
    let mut images: Vec<&Path> = Vec::new();
    let mut breaks = 0;
    // The next number of the last numbered list, while only page breaks have followed it.
    let mut continued: Option<u64> = None;
    for block in blocks {
        let made = match block {
            DocumentBlock::PageBreak => {
                if !laid.is_empty() { breaks += 1; }
                continue;
            }
            DocumentBlock::Heading1 { text } | DocumentBlock::Heading2 { text } | DocumentBlock::Heading3 { text } => {
                if text.trim().is_empty() { continue; }
                let level = match block { DocumentBlock::Heading1 { .. } => 1, DocumentBlock::Heading2 { .. } => 2, _ => 3 };
                let spans = vec![Span { text: text.trim_end().to_string(), style: Style::default(), link: None }];
                Block::Heading { level, spans, anchor: format!("heading-{}", laid.len()) }
            }
            DocumentBlock::Paragraph { runs } => match spans(runs) {
                Some(spans) => Block::Paragraph(spans),
                None => continue,
            },
            DocumentBlock::BulletedList { items } | DocumentBlock::NumberedList { items } => {
                let items: Vec<Vec<Block>> = items.iter().filter_map(|item| spans(&item.runs)).map(|spans| vec![Block::Paragraph(spans)]).collect();
                if items.is_empty() { continue; }
                let start = matches!(block, DocumentBlock::NumberedList { .. }).then(|| continued.unwrap_or(1));
                let next = start.map(|first| first + items.len() as u64);
                laid.extend(std::iter::repeat_n(Block::PageBreak, std::mem::take(&mut breaks)));
                laid.push(Block::List { start, items });
                continued = next;
                continue;
            }
            DocumentBlock::Table { header_row, rows } => {
                let mut rows = rows.iter().map(|row| row.iter().map(|cell| cell_spans(cell)).collect::<Vec<_>>());
                let head = if *header_row { rows.next().unwrap_or_default() } else { Vec::new() };
                let body: Vec<Vec<Vec<Span>>> = rows.filter(|row| row.iter().any(|cell| !cell.is_empty())).collect();
                if body.is_empty() && head.iter().all(Vec::is_empty) { continue; }
                Block::Table { aligns: Vec::new(), head, rows: body }
            }
            DocumentBlock::Image { path } => Block::Image(images.iter().position(|seen| *seen == path.as_path()).unwrap_or_else(|| {
                images.push(path);
                images.len() - 1
            })),
        };
        laid.extend(std::iter::repeat_n(Block::PageBreak, std::mem::take(&mut breaks)));
        laid.push(made);
        continued = None;
    }
    (laid, images)
}

/// The runs as styled spans with trailing blank space removed, or nothing when they hold no text.
fn spans(runs: &[Run]) -> Option<Vec<Span>> {
    let mut spans: Vec<Span> = runs.iter().filter(|run| !run.text.is_empty())
        .map(|run| Span { text: run.text.clone(), style: Style { bold: run.bold, italic: run.italic, ..Style::default() }, link: None })
        .collect();
    while let Some(last) = spans.last_mut() {
        let kept = last.text.trim_end().len();
        if kept > 0 { last.text.truncate(kept); break; }
        spans.pop();
    }
    if spans.is_empty() { None } else { Some(spans) }
}

/// A table cell's text as one plain span, or no spans when it is blank.
fn cell_spans(text: &str) -> Vec<Span> {
    let text = text.trim();
    if text.is_empty() { Vec::new() } else { vec![Span { text: text.to_string(), style: Style::default(), link: None }] }
}

fn page_size_label(size: PageSize) -> &'static str {
    match size {
        PageSize::A4 => "A4",
        PageSize::Letter => "Letter",
        PageSize::Legal => "Legal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::contracts::{ErrorKind, WarningKind};
    use crate::kit::pdf::font_resolver::{BundledFonts, BundledOnly, FontSource};
    use lopdf::{Document, Object};
    use serde_json::json;
    use std::fs;
    use std::path::Path;

    fn no_packs() -> RunPacks { RunPacks::new(Err("No packs in this test.".into()), "0.0.0", Err("No catalog in this test.".into())) }

    fn fonts() -> FontResolver<BundledOnly> {
        FontResolver::new(BundledFonts::discover(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/fonts")).unwrap(), BundledOnly)
    }

    fn sandbox(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("toolbox_formatted_pdf_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// The exact document the view sends.
    fn request(blocks: serde_json::Value, output_path: &Path) -> FormattedPdfRequest {
        serde_json::from_value(json!({
            "document": {
                "title": "Move-out checklist",
                "author": "Sam Lee",
                "blocks": blocks,
                "page": { "pageSize": "a4", "orientation": "portrait", "margins": "normal", "header": "", "footer": "", "pageNumbers": false },
                "font": { "family": "Liberation Serif", "size": 11 },
            },
            "outputPath": output_path,
        })).unwrap()
    }

    fn plain(text: &str) -> serde_json::Value { json!({ "text": text, "bold": false, "italic": false }) }

    fn paragraph(text: &str) -> serde_json::Value { json!({ "kind": "paragraph", "runs": [plain(text)] }) }

    fn list(kind: &str, items: &[String]) -> serde_json::Value {
        json!({ "kind": kind, "items": items.iter().map(|text| json!({ "runs": [plain(text)] })).collect::<Vec<_>>() })
    }

    fn short_blocks() -> serde_json::Value {
        json!([
            { "kind": "heading1", "text": "Move-out checklist" },
            paragraph("Return the keys to the front desk."),
            { "kind": "heading2", "text": "Kitchen" },
            { "kind": "heading3", "text": "Oven" },
            paragraph("Clean the oven and the hob."),
        ])
    }

    /// Each text-showing operation on a page, as the font's PostScript name (without the subset tag) and the text it draws.
    fn drawn(document: &Document, page: u32) -> Vec<(String, String)> {
        let id = document.get_pages()[&page];
        let fonts = document.get_page_fonts(id).unwrap();
        let content = lopdf::content::Content::decode(&document.get_page_content(id)).unwrap();
        let mut font = None;
        let mut shown = Vec::new();
        for operation in content.operations {
            match operation.operator.as_str() {
                "Tf" => font = fonts.get(operation.operands[0].as_name().unwrap()).copied(),
                "Tj" | "TJ" => {
                    let font = font.expect("text is shown after a font is chosen");
                    let encoding = font.get_font_encoding(document).unwrap();
                    let strings = match &operation.operands[0] { Object::Array(parts) => parts.clone(), other => vec![other.clone()] };
                    let text = strings.iter().filter_map(|part| part.as_str().ok()).map(|bytes| Document::decode_text(&encoding, bytes).unwrap()).collect::<String>();
                    let name = String::from_utf8_lossy(font.get(b"BaseFont").unwrap().as_name().unwrap()).into_owned();
                    shown.push((name.split_once('+').map_or(name.clone(), |(_, name)| name.to_string()), text));
                }
                _ => {}
            }
        }
        shown
    }

    /// Extracted text with the line ends lopdf adds after every text object removed.
    fn flat(text: &str) -> String { text.replace('\n', "") }

    /// The fonts that draw text containing `needle` on a page.
    fn fonts_drawing(document: &Document, page: u32, needle: &str) -> Vec<String> {
        drawn(document, page).into_iter().filter(|(_, text)| text.contains(needle)).map(|(font, _)| font).collect()
    }

    fn run(request: &FormattedPdfRequest) -> JobOutcome { note_renamed(create_with(request, &fonts(), &no_packs()), Some(&request.output_path)) }

    fn info(document: &Document, key: &[u8]) -> Option<String> {
        let info = document.trailer.get(b"Info").and_then(Object::as_reference).ok()?;
        crate::kit::pdf::document_info::decode_text(document.get_dictionary(info).ok()?.get(key).ok()?)
    }

    fn media_box(document: &Document, page: lopdf::ObjectId) -> Vec<f32> {
        let page = document.get_dictionary(page).unwrap();
        page.get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|value| value.as_float().unwrap().round()).collect()
    }

    #[test]
    fn headings_and_paragraphs_make_an_a4_pdf_with_recoverable_text_title_and_author() {
        let root = sandbox("short");
        let path = root.join("Move-out checklist.pdf");
        let outcome = run(&request(short_blocks(), &path));
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        assert_eq!(outcome.output_paths(), [path.clone()]);
        assert_eq!(outcome.detail, "1 page, A4.");
        assert!(outcome.warnings.is_empty());
        assert!(outcome.fonts.iter().all(|font| font.font_used == "Liberation Serif" && matches!(font.source, FontSource::BundledFallback)), "{:?}", outcome.fonts);

        let document = Document::load(&path).unwrap();
        let pages = document.get_pages();
        assert_eq!(pages.len(), 1);
        assert_eq!(media_box(&document, pages[&1]), [0., 0., 595., 842.]);
        let text = document.extract_text(&[1]).unwrap();
        for expected in ["Move-out checklist", "Return the keys to the front desk.", "Kitchen", "Oven", "Clean the oven and the hob."] {
            assert!(text.contains(expected), "{expected} missing from {text:?}");
        }
        assert_eq!(info(&document, b"Title").as_deref(), Some("Move-out checklist"));
        assert_eq!(info(&document, b"Author").as_deref(), Some("Sam Lee"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn long_content_paginates_and_letter_pages_are_letter_sized() {
        let root = sandbox("long");
        let filler = "Every room is swept, every shelf is wiped, and every light works before the keys go back. ".repeat(6);
        let blocks = (0..60).map(|index| {
            let text = format!("Part {index}. {filler}");
            if index % 10 == 0 { json!({ "kind": "heading2", "text": text }) } else { paragraph(&text) }
        }).collect::<Vec<_>>();
        let path = root.join("long.pdf");
        let mut long = request(json!(blocks), &path);
        long.document.page.page_size = PageSize::Letter;
        let outcome = run(&long);
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let document = Document::load(&path).unwrap();
        let pages = document.get_pages();
        assert!(pages.len() >= 5, "60 long blocks fill several pages, got {}", pages.len());
        assert_eq!(outcome.detail, format!("{} pages, Letter.", pages.len()));
        for page in pages.values() {
            assert_eq!(media_box(&document, *page), [0., 0., 612., 792.]);
        }
        let last = pages.len() as u32;
        assert!(document.extract_text(&[1]).unwrap().contains("Part 0."));
        assert!(document.extract_text(&[last]).unwrap().contains("Part 59."));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_taken_name_gets_a_numbered_name_and_the_same_document_saves_byte_identical() {
        let root = sandbox("collision");
        let path = root.join("Move-out checklist.pdf");
        fs::write(&path, b"existing PDF").unwrap();
        let first = run(&request(short_blocks(), &path));
        let second = run(&request(short_blocks(), &path));
        assert_eq!(first.output_paths(), [root.join("Move-out checklist-1.pdf")]);
        assert_eq!(second.output_paths(), [root.join("Move-out checklist-2.pdf")]);
        assert_eq!(
            first.warnings.iter().map(|warning| warning.kind.clone()).collect::<Vec<_>>(),
            [WarningKind::RenamedToAvoidReplacing { chosen_name: "Move-out checklist.pdf".into(), final_name: "Move-out checklist-1.pdf".into() }],
        );
        assert_eq!(fs::read(&path).unwrap(), b"existing PDF", "the existing file must stay byte-identical");
        assert_eq!(fs::read(root.join("Move-out checklist-1.pdf")).unwrap(), fs::read(root.join("Move-out checklist-2.pdf")).unwrap(), "the same content must make the same bytes");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_empty_document_is_refused_and_writes_nothing() {
        let root = sandbox("empty");
        let path = root.join("Untitled.pdf");
        let blank_document = json!([
            paragraph("  \n "), { "kind": "heading1", "text": "" }, { "kind": "pageBreak" },
            { "kind": "paragraph", "runs": [] }, list("bulletedList", &["  ".into()]), { "kind": "numberedList", "items": [] }, { "kind": "pageBreak" },
        ]);
        for blocks in [json!([]), blank_document] {
            let outcome = run(&request(blocks, &path));
            let error = outcome.error().expect("an empty document must fail");
            assert!(matches!(error.kind, ErrorKind::InvalidInput) && error.message.contains("empty"), "{error:?}");
        }
        let mut tiny = request(short_blocks(), &path);
        tiny.document.font.size = 2.;
        assert!(run(&tiny).error().is_some_and(|error| error.message.contains("Text size")));
        assert!(fs::read_dir(&root).unwrap().next().is_none(), "a refused document writes nothing");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_request_is_a_structured_model_not_markup() {
        let document = |blocks: serde_json::Value| json!({
            "document": { "title": "", "author": "", "blocks": blocks, "page": { "pageSize": "a4", "orientation": "portrait", "margins": "normal", "header": "", "footer": "", "pageNumbers": false }, "font": { "family": "Liberation Serif", "size": 11 } },
            "outputPath": "/tmp/x.pdf",
        });
        for rejected in [
            json!([{ "kind": "heading1", "text": "x", "html": "<b>x</b>" }]),
            json!([{ "kind": "paragraph", "text": "<b>x</b>" }]),
            json!([{ "kind": "paragraph", "runs": [{ "text": "x", "bold": true, "italic": false, "underline": true }] }]),
            json!([{ "kind": "paragraph", "runs": [{ "text": "x" }] }]),
            json!([{ "kind": "bulletedList", "items": [{ "runs": [plain("x")], "level": 2 }] }]),
            json!([{ "kind": "table", "rows": [["Room"]] }]),
            json!([{ "kind": "table", "headerRow": true, "rows": [["Room"]], "caption": "Rooms" }]),
            json!([{ "kind": "table", "headerRow": true, "rows": [[{ "text": "Room", "bold": true, "italic": false }]] }]),
            json!([{ "kind": "table", "header_row": true, "rows": [["Room"]] }]),
            json!([{ "kind": "image" }]),
            json!([{ "kind": "image", "src": "https://example.com/floorplan.png" }]),
            json!([{ "kind": "image", "path": "/Users/sam/floorplan.png", "width": 200 }]),
            json!([{ "kind": "image", "path": "/Users/sam/floorplan.png", "data": "iVBORw0KGgo=" }]),
        ] {
            assert!(serde_json::from_value::<FormattedPdfRequest>(document(rejected.clone())).is_err(), "{rejected} must be refused");
        }
        let blank_title = request(short_blocks(), Path::new("/tmp/x.pdf"));
        assert_eq!(layout_blocks(&blank_title.document.blocks).0.len(), 5);
    }

    #[test]
    fn bold_and_italic_runs_are_drawn_in_the_bundled_bold_and_italic_styles_and_their_text_is_recoverable() {
        let root = sandbox("emphasis");
        let path = root.join("emphasis.pdf");
        let blocks = json!([
            { "kind": "paragraph", "runs": [
                plain("Return "),
                { "text": "every key", "bold": true, "italic": false },
                plain(" and "),
                { "text": "photograph each room", "bold": false, "italic": true },
                plain(", then "),
                { "text": "sign out", "bold": true, "italic": true },
                plain("."),
            ] },
            { "kind": "bulletedList", "items": [{ "runs": [plain("Kitchen "), { "text": "spotless", "bold": true, "italic": false }] }] },
        ]);
        let outcome = run(&request(blocks, &path));
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let document = Document::load(&path).unwrap();
        assert_eq!(fonts_drawing(&document, 1, "every key"), ["LiberationSerif-Bold"]);
        assert_eq!(fonts_drawing(&document, 1, "photograph each room"), ["LiberationSerif-Italic"]);
        assert_eq!(fonts_drawing(&document, 1, "sign out"), ["LiberationSerif-BoldItalic"]);
        assert_eq!(fonts_drawing(&document, 1, "spotless"), ["LiberationSerif-Bold"]);
        assert_eq!(fonts_drawing(&document, 1, "Return"), ["LiberationSerif"]);
        assert_eq!(fonts_drawing(&document, 1, "Kitchen"), ["LiberationSerif"]);
        let content = String::from_utf8_lossy(&document.get_page_content(document.get_pages()[&1])).into_owned();
        assert!(!content.contains(" Tr") && !content.contains("1 0 0.2 1"), "real bold and italic fonts are used, never thickened or slanted text: {content}");
        let shown = drawn(&document, 1).into_iter().map(|(_, text)| text).collect::<String>();
        assert!(shown.contains("Return every key and photograph each room, then sign out."), "{shown:?}");
        assert!(shown.contains("Kitchen spotless"), "{shown:?}");
        let text = flat(&document.extract_text(&[1]).unwrap());
        for words in ["Return", "every key", "photograph each room", "sign out", "Kitchen", "spotless"] {
            assert!(text.contains(words), "{words:?} is recoverable from {text:?}");
        }
        let mut styles = outcome.fonts.iter().map(|font| (font.font_used.as_str(), format!("{:?}", font.style_used))).collect::<Vec<_>>();
        styles.sort();
        styles.dedup();
        assert_eq!(styles.len(), 4, "regular, bold, italic and bold italic from the bundled family: {:?}", outcome.fonts);
        assert!(outcome.fonts.iter().all(|font| font.font_used == "Liberation Serif" && matches!(font.source, FontSource::BundledFallback)), "{:?}", outcome.fonts);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_numbered_list_keeps_its_numbering_across_pages_and_a_bulleted_list_draws_its_markers() {
        let root = sandbox("lists");
        let path = root.join("lists.pdf");
        let items = |from: usize, to: usize| (from..=to).map(|index| format!("Step {index} of the move")).collect::<Vec<_>>();
        let blocks = json!([
            list("bulletedList", &["Keys".into(), "Meters".into(), "Post".into()]),
            // Long enough to flow from page 1 onto page 2 by itself.
            list("numberedList", &items(1, 70)),
            // Continued after a page-break block, and again after a blank paragraph that is left out.
            { "kind": "pageBreak" },
            list("numberedList", &items(71, 72)),
            paragraph("   "),
            list("numberedList", &items(73, 73)),
            // Anything else in between starts a new list at 1.
            paragraph("Then the garden."),
            list("numberedList", &["Mow the lawn".into()]),
        ]);
        let outcome = run(&request(blocks, &path));
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let document = Document::load(&path).unwrap();
        let pages = document.get_pages().len() as u32;
        assert_eq!(pages, 3, "the long list flows onto page 2 and the page break starts page 3");
        let shown = |page: u32| drawn(&document, page).into_iter().map(|(_, text)| text).collect::<Vec<_>>();

        let first = shown(1);
        assert_eq!(first.iter().filter(|text| text.as_str() == "\u{2022}").count(), 3, "one bullet marker for each item: {first:?}");
        let markers = |texts: &[String]| texts.iter().filter_map(|text| text.strip_suffix('.').and_then(|number| number.parse::<u64>().ok())).collect::<Vec<_>>();
        let on_first = markers(&first);
        let on_second = markers(&shown(2));
        assert!(!on_first.is_empty() && !on_second.is_empty(), "the list is on both pages");
        let flowed = on_first.iter().chain(&on_second).copied().collect::<Vec<_>>();
        assert_eq!(flowed, (1..=70).collect::<Vec<_>>(), "numbering runs on across the page without restarting");
        for number in [on_second[0], 70] {
            assert!(shown(2).iter().any(|text| text == &format!("Step {number} of the move")), "item {number} is drawn beside its number");
        }
        let third = shown(3);
        assert_eq!(markers(&third), [71, 72, 73, 1], "{third:?}");
        assert!(third.iter().any(|text| text == "Step 71 of the move") && third.iter().any(|text| text == "Mow the lawn"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_page_break_block_starts_a_new_page_and_the_same_document_saves_byte_identical() {
        let root = sandbox("page_break");
        let blocks = json!([
            { "kind": "pageBreak" },
            { "kind": "heading1", "text": "Inventory" },
            paragraph("Living room."),
            { "kind": "pageBreak" },
            paragraph("Bedroom."),
            { "kind": "pageBreak" },
            { "kind": "pageBreak" },
            { "kind": "paragraph", "runs": [{ "text": "Signed", "bold": true, "italic": false }, plain(" by the tenant.")] },
            { "kind": "pageBreak" },
        ]);
        let first = root.join("first.pdf");
        let second = root.join("second.pdf");
        for path in [&first, &second] {
            let outcome = run(&request(blocks.clone(), path));
            assert!(outcome.error().is_none(), "{:?}", outcome.error());
            assert_eq!(outcome.detail, "4 pages, A4.", "breaks before the first and after the last block add no page");
        }
        let document = Document::load(&first).unwrap();
        assert_eq!(document.get_pages().len(), 4);
        let text = |page: u32| flat(&document.extract_text(&[page]).unwrap());
        assert!(text(1).contains("Inventory") && text(1).contains("Living room.") && !text(1).contains("Bedroom."), "{:?}", text(1));
        assert!(text(2).contains("Bedroom.") && !text(2).contains("Signed"), "{:?}", text(2));
        assert!(drawn(&document, 3).is_empty(), "two page breaks in a row leave a blank page");
        assert!(text(4).contains("Signed by the tenant."), "{:?}", text(4));
        assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap(), "the same content must make the same bytes");
        let _ = fs::remove_dir_all(root);
    }

    fn long_blocks(count: usize) -> serde_json::Value {
        let filler = "Every room is swept, every shelf is wiped, and every light works before the keys go back. ".repeat(4);
        json!((0..count).map(|index| paragraph(&format!("Part {index}. {filler}"))).collect::<Vec<_>>())
    }

    fn with_page(blocks: serde_json::Value, path: &Path, page: serde_json::Value) -> FormattedPdfRequest {
        serde_json::from_value(json!({
            "document": { "title": "Move-out checklist", "author": "Sam Lee", "blocks": blocks, "page": page_json(&page), "font": { "family": "Liberation Serif", "size": 11 } },
            "outputPath": path,
        })).unwrap()
    }

    /// The view's default page settings with `changes` applied.
    fn page_json(changes: &serde_json::Value) -> serde_json::Value {
        let mut page = json!({ "pageSize": "a4", "orientation": "portrait", "margins": "normal", "header": "", "footer": "", "pageNumbers": false });
        for (key, value) in changes.as_object().unwrap() { page[key] = value.clone(); }
        page
    }

    #[test]
    fn each_page_size_and_orientation_gives_its_media_box_on_every_page() {
        let root = sandbox("sizes");
        for (size, orientation, expected, label) in [
            ("a4", "portrait", [595., 842.], "A4"), ("a4", "landscape", [842., 595.], "A4 landscape"),
            ("letter", "portrait", [612., 792.], "Letter"), ("letter", "landscape", [792., 612.], "Letter landscape"),
            ("legal", "portrait", [612., 1008.], "Legal"), ("legal", "landscape", [1008., 612.], "Legal landscape"),
        ] {
            let path = root.join(format!("{size}-{orientation}.pdf"));
            let outcome = run(&with_page(long_blocks(30), &path, json!({ "pageSize": size, "orientation": orientation })));
            assert!(outcome.error().is_none(), "{size} {orientation}: {:?}", outcome.error());
            let document = Document::load(&path).unwrap();
            let pages = document.get_pages();
            assert!(pages.len() > 1, "{size} {orientation} flows onto more pages");
            assert_eq!(outcome.detail, format!("{} pages, {label}.", pages.len()));
            for page in pages.values() {
                assert_eq!(media_box(&document, *page), [0., 0., expected[0], expected[1]], "{size} {orientation}");
            }
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn larger_margins_give_more_pages_for_the_same_content() {
        let root = sandbox("margins");
        // A few long paragraphs, so the line count follows the text width rather than rounding up per paragraph.
        let text = "Every room is swept, every shelf is wiped, and every light works before the keys go back. ".repeat(60);
        let blocks = json!((0..6).map(|_| paragraph(&text)).collect::<Vec<_>>());
        let pages = ["narrow", "normal", "wide"].map(|margins| {
            let path = root.join(format!("{margins}.pdf"));
            let outcome = run(&with_page(blocks.clone(), &path, json!({ "margins": margins })));
            assert!(outcome.error().is_none(), "{margins}: {:?}", outcome.error());
            Document::load(&path).unwrap().get_pages().len()
        });
        assert!(pages[0] < pages[1] && pages[1] < pages[2], "narrow, normal and wide margins give {pages:?} pages");
        let _ = fs::remove_dir_all(root);
    }

    /// The lowest and highest baselines of the text drawn on a page whose text contains `needle`, from the text matrices.
    fn baselines(document: &Document, page: u32, needle: &str) -> Vec<f32> {
        let id = document.get_pages()[&page];
        let fonts = document.get_page_fonts(id).unwrap();
        let content = lopdf::content::Content::decode(&document.get_page_content(id)).unwrap();
        let (mut font, mut y, mut found) = (None, 0., Vec::new());
        for operation in content.operations {
            match operation.operator.as_str() {
                "Tf" => font = fonts.get(operation.operands[0].as_name().unwrap()).copied(),
                "Td" => y = operation.operands[1].as_float().unwrap(),
                "Tm" => y = operation.operands[5].as_float().unwrap(),
                "Tj" | "TJ" => {
                    let encoding = font.unwrap().get_font_encoding(document).unwrap();
                    let strings = match &operation.operands[0] { Object::Array(parts) => parts.clone(), other => vec![other.clone()] };
                    let text = strings.iter().filter_map(|part| part.as_str().ok()).map(|bytes| Document::decode_text(&encoding, bytes).unwrap()).collect::<String>();
                    if text.contains(needle) { found.push(y); }
                }
                _ => {}
            }
        }
        found
    }

    #[test]
    fn header_footer_and_page_numbers_are_on_every_page_outside_the_text_and_recoverable() {
        let root = sandbox("bands");
        let path = root.join("bands.pdf");
        let page = json!({ "header": "  Flat 4B, 12 Elm Street ", "footer": "Signed copy for the landlord", "pageNumbers": true });
        let outcome = run(&with_page(long_blocks(40), &path, page.clone()));
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let document = Document::load(&path).unwrap();
        let count = document.get_pages().len() as u32;
        assert!(count >= 3, "the content fills several pages, got {count}");
        for number in 1..=count {
            let text = flat(&document.extract_text(&[number]).unwrap());
            for expected in ["Flat 4B, 12 Elm Street", "Signed copy for the landlord", &format!("Page {number} of {count}")] {
                assert!(text.contains(expected), "page {number} is missing {expected:?}: {text:?}");
            }
            let header = baselines(&document, number, "Flat 4B");
            let footer = baselines(&document, number, "Signed copy");
            let numbers = baselines(&document, number, &format!("Page {number} of"));
            let body = baselines(&document, number, "Part ");
            assert!(header.len() == 1 && footer.len() == 1 && numbers.len() == 1 && !body.is_empty(), "page {number}");
            let (top, bottom) = (body.iter().copied().fold(f32::MIN, f32::max), body.iter().copied().fold(f32::MAX, f32::min));
            assert!(header[0] > top + 11. && footer[0] < bottom - 11. && numbers[0] < footer[0], "page {number}: header {header:?}, text {top}..{bottom}, footer {footer:?}, numbers {numbers:?}");
            assert!(header[0] > 842. - 72. && footer[0] < 72., "the header and footer sit in the margins on page {number}");
        }
        // The same document saves byte-identical.
        let again = root.join("again.pdf");
        assert!(run(&with_page(long_blocks(40), &again, page)).error().is_none());
        assert_eq!(fs::read(&path).unwrap(), fs::read(&again).unwrap());

        // Without them, nothing more is drawn.
        let plain_path = root.join("plain.pdf");
        assert!(run(&with_page(long_blocks(40), &plain_path, json!({ "header": "   ", "footer": "" }))).error().is_none());
        let plain = Document::load(&plain_path).unwrap();
        for number in 1..=plain.get_pages().len() as u32 {
            let text = flat(&plain.extract_text(&[number]).unwrap());
            assert!(!text.contains("Page ") && !text.contains("Flat 4B"), "{text:?}");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_header_too_long_for_the_page_is_refused_and_page_settings_are_required() {
        let root = sandbox("long_header");
        let path = root.join("x.pdf");
        let outcome = run(&with_page(short_blocks(), &path, json!({ "header": "Flat 4B, 12 Elm Street. ".repeat(200) })));
        let error = outcome.error().expect("a header taller than the page allows is refused");
        assert!(matches!(error.kind, ErrorKind::InvalidInput) && error.message.contains("header is too long"), "{error:?}");
        assert!(fs::read_dir(&root).unwrap().next().is_none(), "a refused document writes nothing");
        let document = |page: serde_json::Value| json!({
            "document": { "title": "", "author": "", "blocks": short_blocks(), "page": page, "font": { "family": "Liberation Serif", "size": 11 } },
            "outputPath": "/tmp/x.pdf",
        });
        let mut missing = page_json(&json!({}));
        missing.as_object_mut().unwrap().remove("pageNumbers");
        for rejected in [missing, page_json(&json!({ "orientation": "sideways" })), page_json(&json!({ "headerAlign": "right" })), page_json(&json!({ "pageSize": "a3" }))] {
            assert!(serde_json::from_value::<FormattedPdfRequest>(document(rejected.clone())).is_err(), "{rejected} must be refused");
        }
        let _ = fs::remove_dir_all(root);
    }

    fn table(header_row: bool, rows: &[&[&str]]) -> serde_json::Value {
        json!({ "kind": "table", "headerRow": header_row, "rows": rows })
    }

    /// The stroked cell borders on a page, as left, bottom, width and height.
    fn cell_borders(document: &Document, page: u32) -> Vec<[f32; 4]> {
        let content = lopdf::content::Content::decode(&document.get_page_content(document.get_pages()[&page])).unwrap();
        let mut rects = Vec::new();
        let mut last = None;
        for operation in content.operations {
            match operation.operator.as_str() {
                "re" => last = Some(<[f32; 4]>::try_from(operation.operands.iter().map(|value| value.as_float().unwrap()).collect::<Vec<_>>()).unwrap()),
                "S" => rects.extend(last.take()),
                _ => last = None,
            }
        }
        rects
    }

    #[test]
    fn a_table_draws_its_cell_text_in_row_order_with_a_bold_header_and_content_sized_columns() {
        let root = sandbox("table");
        let blocks = json!([
            paragraph("Condition of each room."),
            table(true, &[&["Room", "Condition", "Notes"], &["Kitchen", "Good", "Oven cleaned"], &["  ", "", " "], &["Bathroom", "Fair", " Grout stained "]]),
            paragraph("Signed by the tenant."),
        ]);
        let first = root.join("first.pdf");
        let second = root.join("second.pdf");
        for path in [&first, &second] {
            let outcome = run(&request(blocks.clone(), path));
            assert!(outcome.error().is_none(), "{:?}", outcome.error());
            assert_eq!(outcome.detail, "1 page, A4.");
        }
        let document = Document::load(&first).unwrap();
        let shown = drawn(&document, 1);
        let texts = shown.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>();
        assert_eq!(texts, [
            "Condition of each room.",
            "Room", "Condition", "Notes",
            "Kitchen", "Good", "Oven cleaned",
            "Bathroom", "Fair", "Grout stained",
            "Signed by the tenant.",
        ], "cells are drawn row by row, left to right, and the blank row is left out");
        let font_of = |cell: &str| shown.iter().find(|(_, text)| text == cell).map(|(font, _)| font.as_str());
        for header in ["Room", "Condition", "Notes"] { assert_eq!(font_of(header), Some("LiberationSerif-Bold"), "{header}"); }
        for cell in ["Kitchen", "Oven cleaned", "Grout stained"] { assert_eq!(font_of(cell), Some("LiberationSerif"), "{cell}"); }
        let text = flat(&document.extract_text(&[1]).unwrap());
        assert!(text.contains("room.RoomConditionNotesKitchenGoodOven cleanedBathroomFairGrout stainedSigned"), "the extracted text keeps row order: {text:?}");

        // Three rows of three cells, sized to their short text rather than stretched across the page.
        let borders = cell_borders(&document, 1);
        assert_eq!(borders.len(), 9, "{borders:?}");
        let right = borders.iter().map(|[x, _, width, _]| x + width).fold(f32::MIN, f32::max);
        assert!(right < 300., "short cells give a narrow table, its right edge is at {right}");
        assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap(), "the same table must make the same bytes");

        // Long cells share the text width and wrap, staying inside the margins.
        let wide = root.join("wide.pdf");
        let long = "Every room is swept, every shelf is wiped, and every light works. ".repeat(4);
        assert!(run(&request(json!([table(false, &[&["Kitchen", long.as_str(), long.as_str()]])]), &wide)).error().is_none());
        let wide = Document::load(&wide).unwrap();
        let borders = cell_borders(&wide, 1);
        let (left, right) = (borders.iter().map(|[x, ..]| *x).fold(f32::MAX, f32::min), borders.iter().map(|[x, _, width, _]| x + width).fold(f32::MIN, f32::max));
        let margin = Margins::Normal.points();
        assert!((left - margin).abs() < 0.5 && (right - (595.28 - margin)).abs() < 1., "the table fills the text width, {left}..{right}");
        assert!(drawn(&wide, 1).iter().all(|(font, _)| font == "LiberationSerif"), "without a header row every cell is regular");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_long_table_splits_across_pages_and_repeats_its_header_row_at_the_top_of_each() {
        let root = sandbox("long_table");
        let names = (1..=120).map(|index| format!("Item {index:03}")).collect::<Vec<_>>();
        let rows = std::iter::once(vec!["Room", "Condition", "Notes"])
            .chain(names.iter().map(|name| vec![name.as_str(), "Good", "Checked on handover"]))
            .collect::<Vec<_>>();
        let rows = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let path = root.join("long.pdf");
        let outcome = run(&request(json!([paragraph("Inventory."), table(true, &rows)]), &path));
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let document = Document::load(&path).unwrap();
        let pages = document.get_pages().len() as u32;
        assert!(pages >= 3, "120 rows fill several pages, got {pages}");
        assert_eq!(outcome.detail, format!("{pages} pages, A4."));
        let mut seen = Vec::new();
        for page in 1..=pages {
            let texts = drawn(&document, page).into_iter().map(|(_, text)| text).collect::<Vec<_>>();
            let table = if page == 1 { &texts[1..] } else { &texts[..] };
            assert_eq!(&table[..3], ["Room", "Condition", "Notes"], "page {page} starts the table with its header row: {texts:?}");
            assert_eq!(texts.iter().filter(|text| text.as_str() == "Room").count(), 1, "the header row is drawn once on page {page}");
            assert_eq!(fonts_drawing(&document, page, "Condition"), ["LiberationSerif-Bold"]);
            let body = &table[3..];
            assert_eq!(body.len() % 3, 0, "only whole rows are on page {page}: {body:?}");
            for row in body.chunks(3) {
                assert_eq!(&row[1..], ["Good", "Checked on handover"], "a row's cells stay together on page {page}");
                seen.push(row[0].clone());
            }
            assert!(!body.is_empty(), "page {page} has rows under its header");
        }
        assert_eq!(seen, names, "every row is drawn once, in order, across the pages");

        // Without a header row, the first row is an ordinary row and nothing repeats.
        let plain_path = root.join("plain.pdf");
        assert!(run(&request(json!([table(false, &rows)]), &plain_path)).error().is_none());
        let plain = Document::load(&plain_path).unwrap();
        let count = plain.get_pages().len() as u32;
        assert!(count >= 3);
        let rooms = (1..=count).map(|page| drawn(&plain, page).into_iter().filter(|(_, text)| text == "Room").count()).sum::<usize>();
        assert_eq!(rooms, 1);
        assert!(!fonts_drawing(&plain, 1, "Room").contains(&"LiberationSerif-Bold".to_string()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_header_row_never_sits_alone_at_the_foot_of_a_page() {
        let root = sandbox("table_foot");
        let mut moved = false;
        // Push the table further down page 1 each time, through the point where only its header row would fit.
        for filler in 24..=34 {
            let mut blocks = (0..filler).map(|index| paragraph(&format!("Line {index}."))).collect::<Vec<_>>();
            blocks.push(table(true, &[&["Room", "Condition"], &["Kitchen", "Good"], &["Bathroom", "Fair"]]));
            let path = root.join(format!("{filler}.pdf"));
            assert!(run(&request(json!(blocks), &path)).error().is_none());
            let document = Document::load(&path).unwrap();
            for page in 1..=document.get_pages().len() as u32 {
                let texts = drawn(&document, page).into_iter().map(|(_, text)| text).collect::<Vec<_>>();
                if let Some(at) = texts.iter().position(|text| text == "Room") {
                    assert!(texts.get(at + 2).is_some_and(|text| text == "Kitchen" || text == "Bathroom"), "{filler} lines: the header row on page {page} has its first row under it: {texts:?}");
                    moved |= page == 2 && at == 0;
                }
            }
        }
        assert!(moved, "one of the documents puts the whole table on page 2");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn table_rows_columns_and_cell_text_are_checked_when_the_request_arrives() {
        let root = sandbox("table_limits");
        let path = root.join("x.pdf");
        let wide = vec!["x"; 21];
        let tall = vec![["x"].as_slice(); 1001];
        let long = "x".repeat(1001);
        for (blocks, kind, message) in [
            (json!([table(true, &[])]), ErrorKind::InvalidInput, "at least one row and one column"),
            (json!([table(true, &[&[]])]), ErrorKind::InvalidInput, "at least one row and one column"),
            (json!([table(true, &[&["Room", "Condition"], &["Kitchen"]])]), ErrorKind::InvalidInput, "same number of cells"),
            (json!([paragraph("Rooms"), table(true, &[&wide[..]])]), ErrorKind::LimitExceeded { limit: LimitKind::Dimensions }, "up to 20 columns and 1000 rows"),
            (json!([table(false, &tall)]), ErrorKind::LimitExceeded { limit: LimitKind::Dimensions }, "up to 20 columns and 1000 rows"),
            (json!([table(false, &[&["Kitchen", long.as_str()]])]), ErrorKind::LimitExceeded { limit: LimitKind::TextLength }, "up to 1000 characters"),
        ] {
            let outcome = run(&request(blocks.clone(), &path));
            let error = outcome.error().unwrap_or_else(|| panic!("{blocks} must be refused"));
            assert!(error.kind == kind && error.message.contains(message), "{blocks}: {error:?}");
        }
        // At the limits, and a table of blank cells is left out like any blank block.
        let most = vec![vec!["x"; 20]; 2];
        let most = most.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let full = "x".repeat(1000);
        assert!(run(&request(json!([table(true, &most), table(false, &[&[full.as_str()]])]), &path)).error().is_none());
        let blank = run(&request(json!([table(true, &[&[" ", ""], &["", " "]])]), &root.join("blank.pdf")));
        assert!(blank.error().is_some_and(|error| error.message.contains("empty")), "{:?}", blank.error());
        let _ = fs::remove_dir_all(root);
    }

    /// Writes a solid PNG of the given pixel size.
    fn png(path: &Path, width: u32, height: u32) {
        image::RgbImage::from_pixel(width, height, image::Rgb([30, 90, 160])).save(path).unwrap();
    }

    /// Where each image is drawn on a page, as its name and the width, height, left and bottom of its placement.
    fn placed_images(document: &Document, page: u32) -> Vec<(String, [f32; 4])> {
        let content = lopdf::content::Content::decode(&document.get_page_content(document.get_pages()[&page])).unwrap();
        let mut placed = Vec::new();
        let mut matrix = None;
        for operation in content.operations {
            match operation.operator.as_str() {
                "cm" => {
                    let values = operation.operands.iter().map(|value| value.as_float().unwrap()).collect::<Vec<_>>();
                    matrix = Some([values[0], values[3], values[4], values[5]]);
                }
                "Do" => placed.push((String::from_utf8_lossy(operation.operands[0].as_name().unwrap()).into_owned(), matrix.take().unwrap())),
                _ => {}
            }
        }
        placed
    }

    /// The pixel width and height of every image XObject in the PDF, smallest first.
    fn image_objects(document: &Document) -> Vec<(i64, i64)> {
        let mut sizes = document.objects.values()
            .filter_map(|object| object.as_stream().ok())
            .filter(|stream| stream.dict.get(b"Subtype").and_then(Object::as_name).is_ok_and(|name| name == b"Image"))
            .filter(|stream| stream.dict.get(b"ColorSpace").and_then(Object::as_name).is_ok_and(|name| name == b"DeviceRGB"))
            .map(|stream| (stream.dict.get(b"Width").unwrap().as_i64().unwrap(), stream.dict.get(b"Height").unwrap().as_i64().unwrap()))
            .collect::<Vec<_>>();
        sizes.sort();
        sizes
    }

    #[test]
    fn an_image_block_is_embedded_once_scaled_to_fit_the_content_width_and_saves_byte_identical() {
        let root = sandbox("images");
        let wide = root.join("floorplan.png");
        let small = root.join("key.png");
        png(&wide, 2000, 500);
        png(&small, 200, 100);
        let blocks = json!([
            paragraph("Floor plan"),
            { "kind": "image", "path": wide },
            { "kind": "image", "path": small },
            { "kind": "image", "path": wide },
        ]);
        let path = root.join("plan.pdf");
        let outcome = run(&request(blocks.clone(), &path));
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let document = Document::load(&path).unwrap();
        // The same file is embedded once, at its full pixel size.
        assert_eq!(image_objects(&document), [(200, 100), (2000, 500)]);
        let placed = placed_images(&document, 1);
        assert_eq!(placed.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["Im0", "Im1", "Im0"]);
        let content_width = 595.28 - 2. * 72.;
        // A4 with normal margins: the wide image fills the content width and keeps its shape; the small one keeps its size.
        let [width, height, left, _] = placed[0].1;
        assert!((width - content_width).abs() < 0.05 && (height - content_width / 4.).abs() < 0.05 && (left - 72.).abs() < 0.05, "{placed:?}");
        let [width, height, left, _] = placed[1].1;
        assert!((width - 150.).abs() < 0.05 && (height - 75.).abs() < 0.05 && (left - 72.).abs() < 0.05, "{placed:?}");
        assert!(placed.iter().all(|(_, [width, _, left, _])| left + width <= 595.28 - 72. + 0.05), "{placed:?}");
        assert!(document.extract_text(&[1]).unwrap().contains("Floor plan"));

        // The same document with the same files saves the same bytes, and the image files are left as they were.
        let again = root.join("again.pdf");
        assert!(run(&request(blocks, &again)).error().is_none());
        assert_eq!(fs::read(&path).unwrap(), fs::read(&again).unwrap());
        assert_eq!(image::image_dimensions(&wide).unwrap(), (2000, 500));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_image_fails_the_save_naming_the_image_and_writes_nothing() {
        let root = sandbox("missing_image");
        let present = root.join("key.png");
        png(&present, 20, 20);
        let missing = root.join("Move").join("floorplan.png");
        let path = root.join("plan.pdf");
        let outcome = run(&request(json!([paragraph("Floor plan"), { "kind": "image", "path": present }, { "kind": "image", "path": missing }]), &path));
        let error = outcome.error().expect("a missing image fails the save");
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        assert!(error.message.starts_with("floorplan.png is missing."), "{error:?}");
        assert!(error.message.contains(&missing.display().to_string()), "{error:?}");
        assert!(outcome.output_paths().is_empty());
        let mut left = fs::read_dir(&root).unwrap().map(|entry| entry.unwrap().file_name()).collect::<Vec<_>>();
        left.sort();
        assert_eq!(left, ["key.png"], "nothing is written beside the image");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn images_are_only_read_from_this_device_and_must_be_images_a_pdf_can_hold() {
        let root = sandbox("image_checks");
        let path = root.join("plan.pdf");
        let broken = root.join("broken.png");
        fs::write(&broken, b"not a picture").unwrap();
        let large = root.join("large.png");
        fs::File::create(&large).unwrap().set_len(21 * 1024 * 1024).unwrap();
        let gif = root.join("animation.gif");
        png(&root.join("frame.png"), 4, 4);
        fs::copy(root.join("frame.png"), &gif).unwrap();
        for (image, kind, message) in [
            (json!("https://example.com/floorplan.png"), ErrorKind::InvalidInput, "read from this device only, so https://example.com/floorplan.png"),
            (json!("floorplan.png"), ErrorKind::InvalidInput, "read from this device only, so floorplan.png"),
            (json!(gif), ErrorKind::InvalidInput, "animation.gif can't be added. Choose a PNG, JPEG, WebP, HEIC, HEIF or TIFF image."),
            (json!(broken), ErrorKind::InvalidInput, "broken.png can't be read as an image"),
            (json!(large), ErrorKind::LimitExceeded { limit: LimitKind::FileSize }, "large.png can't be added. Larger than the 20 MB limit"),
        ] {
            let outcome = run(&request(json!([paragraph("Floor plan"), { "kind": "image", "path": image }]), &path));
            let error = outcome.error().unwrap_or_else(|| panic!("{image} must be refused"));
            assert!(error.kind == kind && error.message.contains(message), "{image}: {error:?}");
            assert!(!path.exists());
        }
        let _ = fs::remove_dir_all(root);
    }
}
