//! Formatted PDF: a document typed in Toolbox, sent as ordered blocks rather than markup, laid out into a new PDF.

use std::path::PathBuf;

use serde::Deserialize;

use crate::kit::common::limits::{self, ToolLimits};
use crate::kit::common::{note_renamed, OutputLocation, OutputNaming};
use crate::kit::contracts::{JobOutcome, LimitKind, ToolError};
use crate::kit::pdf::convert::{page_count, Margins, Orientation, PageSize};
use crate::kit::pdf::font_resolver::{FontResolver, SystemFontLookup};
use crate::kit::pdf::layout::{assemble, Align, Assembly, Block, FontBook, Flow, Geometry, Paragraph, Span, Style, Theme, PAGE_COUNT, PAGE_NUMBER};
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
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum DocumentBlock {
    Heading1 { text: String },
    Heading2 { text: String },
    Heading3 { text: String },
    Paragraph { runs: Vec<Run> },
    BulletedList { items: Vec<ListItem> },
    NumberedList { items: Vec<ListItem> },
    PageBreak,
}

impl DocumentBlock {
    fn text_bytes(&self) -> usize {
        let runs = |runs: &[Run]| runs.iter().map(|run| run.text.len()).sum::<usize>();
        match self {
            Self::Heading1 { text } | Self::Heading2 { text } | Self::Heading3 { text } => text.len(),
            Self::Paragraph { runs: paragraph } => runs(paragraph),
            Self::BulletedList { items } | Self::NumberedList { items } => items.iter().map(|item| runs(&item.runs)).sum(),
            Self::PageBreak => 0,
        }
    }
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
        let blocks = layout_blocks(&document.blocks);
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
        let mut flow = Flow::new(&mut book, theme, &[], geometry);
        flow.blocks(&blocks)?;
        let footer = page.footer_blocks((&PAGE_NUMBER.to_string(), &PAGE_COUNT.to_string()));
        flow.bands(&header, &footer, inset, inset)?;
        let laid = flow.finish();
        let count = laid.pages.len();
        limits.check_made_pages(count)?;
        let mut pdf = assemble(&book, Assembly {
            size, pages: laid.pages, pictures: &[], bookmarks: &laid.bookmarks, anchors: &laid.anchors,
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

/// The document's blocks for layout, in order. Blank blocks and list items are left out, and so are page breaks
/// with nothing before or after them. A numbered list that follows another across page breaks continues its numbering.
fn layout_blocks(blocks: &[DocumentBlock]) -> Vec<Block> {
    let mut laid: Vec<Block> = Vec::new();
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
        };
        laid.extend(std::iter::repeat_n(Block::PageBreak, std::mem::take(&mut breaks)));
        laid.push(made);
        continued = None;
    }
    laid
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
            json!([{ "kind": "table", "rows": [] }]),
        ] {
            assert!(serde_json::from_value::<FormattedPdfRequest>(document(rejected.clone())).is_err(), "{rejected} must be refused");
        }
        let blank_title = request(short_blocks(), Path::new("/tmp/x.pdf"));
        assert_eq!(layout_blocks(&blank_title.document.blocks).len(), 5);
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
}
