//! Formatted PDF: a document typed in Toolbox, sent as ordered blocks rather than markup, laid out into a new PDF.

use std::path::PathBuf;

use serde::Deserialize;

use crate::kit::common::limits::{self, ToolLimits};
use crate::kit::common::{note_renamed, OutputLocation, OutputNaming};
use crate::kit::contracts::{JobOutcome, LimitKind, ToolError};
use crate::kit::pdf::convert::{page_count, PageDefaults, PageSize};
use crate::kit::pdf::font_resolver::{FontResolver, SystemFontLookup};
use crate::kit::pdf::layout::{assemble, Assembly, Block, FontBook, Flow, Geometry, Span, Style, Theme};
use crate::packs::uninstall::RunPacks;

pub const COMMAND: &str = "create_formatted_pdf";
/// Headings and paragraphs have no code, but the theme names a family for it.
const CODE_FAMILY: &str = "Liberation Mono";
const FONT_SIZES: std::ops::RangeInclusive<f32> = 6.0..=72.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlockKind { Heading1, Heading2, Heading3, Paragraph }

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentBlock {
    pub kind: BlockKind,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentFont {
    pub family: String,
    pub size: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormattedDocument {
    pub title: String,
    pub author: String,
    pub blocks: Vec<DocumentBlock>,
    pub page: PageDefaults,
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
        let bytes = document.title.len() + document.author.len() + document.blocks.iter().map(|block| block.text.len()).sum::<usize>();
        if bytes as u64 > limits.max_bytes {
            return Err(ToolError::limit_exceeded(
                LimitKind::FileSize,
                format!("The document is larger than the {} limit for one PDF. Split it into smaller documents and try again.", limits::bytes_label(limits.max_bytes)),
            ));
        }

        let size = document.page.page_size.points();
        let geometry = Geometry::uniform(size.0, size.1, document.page.margins.points());
        let mut book = FontBook::new(fonts, packs);
        let mut flow = Flow::new(&mut book, Theme::new(&document.font.family, CODE_FAMILY, document.font.size, false), &[], geometry);
        flow.blocks(&blocks)?;
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
        let detail = format!("{}, {}.", page_count(count), page_size_label(document.page.page_size));
        Ok(JobOutcome::succeeded(request.output_path.clone(), vec![path], detail).with_fonts(book.reports()))
    })();
    result.unwrap_or_else(|error| JobOutcome::failure(request.output_path.clone(), error))
}

/// The document's blocks for layout, in order; blank blocks are left out.
fn layout_blocks(blocks: &[DocumentBlock]) -> Vec<Block> {
    blocks.iter().filter(|block| !block.text.trim().is_empty()).enumerate().map(|(index, block)| {
        let spans = vec![Span { text: block.text.trim_end().to_string(), style: Style::default(), link: None }];
        match block.kind {
            BlockKind::Heading1 => Block::Heading { level: 1, spans, anchor: format!("heading-{index}") },
            BlockKind::Heading2 => Block::Heading { level: 2, spans, anchor: format!("heading-{index}") },
            BlockKind::Heading3 => Block::Heading { level: 3, spans, anchor: format!("heading-{index}") },
            BlockKind::Paragraph => Block::Paragraph(spans),
        }
    }).collect()
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
                "page": { "pageSize": "a4", "margins": "normal" },
                "font": { "family": "Liberation Serif", "size": 11 },
            },
            "outputPath": output_path,
        })).unwrap()
    }

    fn short_blocks() -> serde_json::Value {
        json!([
            { "kind": "heading1", "text": "Move-out checklist" },
            { "kind": "paragraph", "text": "Return the keys to the front desk." },
            { "kind": "heading2", "text": "Kitchen" },
            { "kind": "heading3", "text": "Oven" },
            { "kind": "paragraph", "text": "Clean the oven and the hob." },
        ])
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
        let paragraph = "Every room is swept, every shelf is wiped, and every light works before the keys go back. ".repeat(6);
        let blocks = (0..60).map(|index| json!({ "kind": if index % 10 == 0 { "heading2" } else { "paragraph" }, "text": format!("Part {index}. {paragraph}") })).collect::<Vec<_>>();
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
        for blocks in [json!([]), json!([{ "kind": "paragraph", "text": "  \n " }, { "kind": "heading1", "text": "" }])] {
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
        let rejected = serde_json::from_value::<FormattedPdfRequest>(json!({
            "document": { "title": "", "author": "", "blocks": [{ "kind": "heading1", "text": "x", "html": "<b>x</b>" }], "page": { "pageSize": "a4", "margins": "normal" }, "font": { "family": "Liberation Serif", "size": 11 } },
            "outputPath": "/tmp/x.pdf",
        }));
        assert!(rejected.is_err(), "unknown fields are refused");
        let blank_title = request(short_blocks(), Path::new("/tmp/x.pdf"));
        assert_eq!(layout_blocks(&blank_title.document.blocks).len(), 5);
    }
}
