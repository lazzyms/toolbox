//! PDF to EPUB: each PDF's selectable text becomes a reflowable EPUB 3.
//!
//! Text comes from the in-process page-text reader. Runs are joined into lines and lines into
//! paragraphs by their positions, one content document per page with text, in page order. The
//! package is built in memory with fixed timestamps and an identifier derived from the PDF's bytes,
//! so the same PDF always makes the same EPUB, then reopened and checked before it is published.

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
}

pub const NO_SELECTABLE_TEXT: &str = "PDF contains no selectable text; scanned PDFs require OCR. No EPUB was made.";
pub const PASSWORD_PROTECTED: &str = "This PDF is password-protected. Remove the password, then convert again.";

const MIMETYPE: &str = "application/epub+zip";
const PACKAGE_PATH: &str = "OEBPS/content.opf";
const NAV_PATH: &str = "OEBPS/nav.xhtml";
/// Every EPUB carries this modification time, so output never depends on when it was made.
const MODIFIED: &str = "1980-01-01T00:00:00Z";
/// The text's language isn't known, so the book says so rather than guessing.
const LANGUAGE: &str = "und";

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
    let chapters = pages.iter().enumerate().filter_map(|(index, page)| {
        let paragraphs = paragraphs(page);
        (!paragraphs.is_empty()).then(|| Chapter { page: index as u32 + 1, paragraphs })
    }).collect::<Vec<_>>();
    if chapters.is_empty() {
        return JobOutcome::skipped(input, SkipReason::new(SkipCode::NeedsOcr, NO_SELECTABLE_TEXT).with_pointer(OutcomePointer::Tool { tool_id: "pdf-ocr".into() }));
    }
    let title = input.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
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
    let blank = (1..=pages.len() as u32).filter(|page| !chapters.iter().any(|chapter| chapter.page == *page)).collect::<Vec<_>>();
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

/// One page's text as a content document.
struct Chapter { page: u32, paragraphs: Vec<String> }

/// A line being assembled from runs that share a baseline band.
struct Line { text: String, top: f32, bottom: f32, right: f32, size: f32 }

impl Line {
    fn start(run: &PageTextRun) -> Self {
        Self { text: run.text.clone(), top: run.y, bottom: run.y + run.height, right: run.x + run.width, size: run.font_size }
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
        self.right = self.right.max(run.x + run.width);
        self.size = self.size.max(run.font_size);
    }
}

/// Groups a page's runs into lines in content order, then lines into paragraphs: a new paragraph
/// starts after a gap taller than about half a line, when the text moves back up the page, or when
/// the text size changes markedly. Runs without a reliable Unicode mapping are left out.
fn paragraphs(page: &PageText) -> Vec<String> {
    let mut lines: Vec<Line> = Vec::new();
    for run in page.runs.iter().filter(|run| run.reliable_unicode && !run.text.trim().is_empty()) {
        match lines.last_mut() {
            Some(line) if line.takes(run) => line.push(run),
            _ => lines.push(Line::start(run)),
        }
    }
    let mut paragraphs: Vec<String> = Vec::new();
    let mut previous: Option<&Line> = None;
    for line in &lines {
        let text = clean(&line.text);
        if text.is_empty() { continue; }
        let joins = previous.is_some_and(|previous| {
            let size = previous.size.max(line.size);
            line.top >= previous.top && line.top - previous.bottom <= size * 0.5 && (line.size - previous.size).abs() <= size * 0.2
        });
        match paragraphs.last_mut() {
            Some(paragraph) if joins => { paragraph.push(' '); paragraph.push_str(&text); }
            _ => paragraphs.push(text),
        }
        previous = Some(line);
    }
    paragraphs
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

fn chapter_name(page: u32) -> String { format!("page-{page}.xhtml") }

fn xhtml(title: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\">\n<head>\n<meta charset=\"UTF-8\"/>\n<title>{}</title>\n</head>\n<body>\n{body}</body>\n</html>\n",
        escape(title),
    )
}

fn chapter_document(title: &str, chapter: &Chapter) -> String {
    let paragraphs = chapter.paragraphs.iter().map(|paragraph| format!("<p>{}</p>\n", escape(paragraph))).collect::<String>();
    xhtml(&format!("{title}, page {}", chapter.page), &format!("<section id=\"page-{}\">\n{paragraphs}</section>\n", chapter.page))
}

fn nav_document(title: &str, chapters: &[Chapter]) -> String {
    let entries = chapters.iter().map(|chapter| format!("<li><a href=\"{}\">Page {}</a></li>\n", chapter_name(chapter.page), chapter.page)).collect::<String>();
    xhtml(title, &format!("<nav epub:type=\"toc\" id=\"toc\">\n<h1>Contents</h1>\n<ol>\n{entries}</ol>\n</nav>\n"))
}

fn package_document(title: &str, identifier: &str, chapters: &[Chapter]) -> String {
    let items = chapters.iter().map(|chapter| format!("<item id=\"page-{0}\" href=\"{1}\" media-type=\"application/xhtml+xml\"/>\n", chapter.page, chapter_name(chapter.page))).collect::<String>();
    let spine = chapters.iter().map(|chapter| format!("<itemref idref=\"page-{}\"/>\n", chapter.page)).collect::<String>();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"book-id\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n<dc:identifier id=\"book-id\">{identifier}</dc:identifier>\n<dc:title>{}</dc:title>\n<dc:language>{LANGUAGE}</dc:language>\n<meta property=\"dcterms:modified\">{MODIFIED}</meta>\n</metadata>\n<manifest>\n<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n{items}</manifest>\n<spine>\n{spine}</spine>\n</package>\n",
        escape(title),
    )
}

const CONTAINER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n<rootfiles>\n<rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n</rootfiles>\n</container>\n";

/// Builds the package: `mimetype` first and stored, then the container, package, nav and pages.
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
        entry(&format!("OEBPS/{}", chapter_name(chapter.page)), deflated, chapter_document(title, chapter).as_bytes())?;
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

    #[test]
    fn runs_join_into_lines_and_lines_into_paragraphs() {
        let text = page(vec![
            run("The quick", 72., 100., 12.), run("brown fox", 72. + 9. * 6. + 3., 100., 12.),
            run("jumps over.", 72., 114., 12.),
            run("A new", 72., 150., 12.), run("paragraph", 72. + 5. * 6. + 4., 150., 12.),
            run("Heading", 72., 170., 24.),
        ]);
        assert_eq!(paragraphs(&text), ["The quick brown fox jumps over.", "A new paragraph", "Heading"]);
    }

    #[test]
    fn runs_without_a_reliable_mapping_and_xml_illegal_characters_are_left_out() {
        let mut garbled = run("\u{fffd}\u{fffd}", 72., 100., 12.);
        garbled.reliable_unicode = false;
        let text = page(vec![garbled, run("Bell\u{7} & <tags>", 200., 100., 12.)]);
        assert_eq!(paragraphs(&text), ["Bell & <tags>"]);
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
        let chapters = [Chapter { page: 1, paragraphs: vec!["Text".into()] }];
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
        let root = std::env::temp_dir().join(format!("toolbox-to-epub-blank-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let input = root.join("mixed.pdf");
        pdf_with_pages(&input, &["BT /F1 12 Tf 72 720 Td (First page) Tj ET", "", "BT /F1 12 Tf 72 720 Td (Third page) Tj ET"]);
        let outcome = convert(&PdfToEpubRequest { paths: vec![input.clone()], output_location: OutputLocation::AlongsideInput }, input);
        let OutcomeState::Succeeded { output_paths } = &outcome.state else { panic!("{outcome:?}") };
        assert_eq!(outcome.detail, "3 pages.");
        assert!(matches!(&outcome.warnings[0].kind, WarningKind::PagesWithoutText { pages } if pages == &[2]));
        let mut archive = ZipArchive::new(fs::File::open(&output_paths[0]).unwrap()).unwrap();
        assert!(archive.by_name("OEBPS/page-2.xhtml").is_err());
        assert!(archive.by_name("OEBPS/page-3.xhtml").is_ok());
        let _ = fs::remove_dir_all(root);
    }

    fn pdf_with_pages(path: &Path, pages: &[&str]) {
        use lopdf::{dictionary, Object, Stream};
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
        let kids = pages.iter().map(|content| {
            let content_id = document.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
            Object::Reference(document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } }, "Contents" => content_id,
            }))
        }).collect::<Vec<_>>();
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages.len() as i64 }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }
}
