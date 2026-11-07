use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8, WINDOWS_1252};
use lopdf::Document;
use serde::Deserialize;

use crate::kit::common::batch_runner::{BatchRunner, RunEvents};
use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::{native_capabilities, SkipCode, SkipReason, ToolError};
use crate::kit::pdf::font_resolver::{FontResolver, FontStyle, SystemFontLookup};
use crate::kit::pdf::layout::{assemble, expand_tabs, Assembly, Face, FontBook, Flow, Geometry, PageOut, Theme};
use crate::kit::pdf::{markdown, rtf};

pub const COMMAND: &str = "convert_to_pdf";
const ALREADY_PDF: &str = "Already a PDF. Nothing was made.";
const FONT_SIZE: f32 = 10.;
const LEADING: f32 = 13.;
const MARKDOWN_SIZE: f32 = 11.;
const CODE_FAMILY: &str = "Courier New";
const RTF_FAMILY: &str = "Times New Roman";
const RTF_SIZE: f32 = 12.;
const MIN_PAGE_SIDE: f32 = 72.;
const MAX_PAGE_SIDE: f32 = 14_400.;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PageSize { #[default] A4, Letter, Legal }

impl PageSize {
    fn points(self) -> (f32, f32) {
        match self {
            Self::A4 => (595.28, 841.89),
            Self::Letter => (612., 792.),
            Self::Legal => (612., 1008.),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Margins { Narrow, #[default] Normal, Wide }

impl Margins {
    fn points(self) -> f32 {
        match self {
            Self::Narrow => 36.,
            Self::Normal => 72.,
            Self::Wide => 108.,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextFont { #[default] Monospace, Sans, Serif }

impl TextFont {
    fn family(self) -> &'static str {
        match self {
            Self::Monospace => "Courier New",
            Self::Sans => "Arial",
            Self::Serif => "Times New Roman",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextEncoding { #[default] Detect, Utf8, Utf16le, Utf16be, Windows1252 }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CodeStyle { #[default] Shaded, Plain }

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageDefaults {
    #[serde(default)] pub page_size: PageSize,
    #[serde(default)] pub margins: Margins,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlainTextOptions {
    #[serde(default)] pub page_size: Option<PageSize>,
    #[serde(default)] pub margins: Option<Margins>,
    pub font: TextFont,
    pub encoding: TextEncoding,
    pub wrap_lines: bool,
}

impl Default for PlainTextOptions {
    fn default() -> Self { Self { page_size: None, margins: None, font: TextFont::default(), encoding: TextEncoding::default(), wrap_lines: true } }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarkdownOptions {
    #[serde(default)] pub page_size: Option<PageSize>,
    #[serde(default)] pub margins: Option<Margins>,
    pub body_font: TextFont,
    pub code_style: CodeStyle,
    pub table_of_contents: bool,
}

impl Default for MarkdownOptions {
    fn default() -> Self { Self { page_size: None, margins: None, body_font: TextFont::Sans, code_style: CodeStyle::default(), table_of_contents: false } }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConvertToPdfRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
    #[serde(default)] pub page_defaults: PageDefaults,
    #[serde(default)] pub plain_text: Option<PlainTextOptions>,
    #[serde(default)] pub markdown: Option<MarkdownOptions>,
}

pub struct ConvertJob<S: SystemFontLookup> {
    request: ConvertToPdfRequest,
    fonts: FontResolver<S>,
}

pub fn run(events: &RunEvents, request: serde_json::Value) -> Vec<JobOutcome> {
    run_with(events, request, FontResolver::installed)
}

pub fn run_with<S, F>(events: &RunEvents, request: serde_json::Value, fonts: F) -> Vec<JobOutcome>
where
    S: SystemFontLookup + Send + Sync + 'static,
    F: FnOnce() -> Result<FontResolver<S>, ToolError>,
{
    let paths = request_paths(&request);
    let fail_all = |error: ToolError| paths.iter().cloned().map(|path| JobOutcome::failure(path, error.clone())).collect();
    let request = match parse_request(request) { Ok(request) => request, Err(error) => return fail_all(error) };
    let fonts = match fonts() { Ok(fonts) => fonts, Err(error) => return fail_all(error) };
    let job = Arc::new(ConvertJob { request, fonts });
    let location = job.request.output_location.clone();
    BatchRunner::run(events, COMMAND, job.request.paths.clone(), &location, &job, convert)
}

fn request_paths(request: &serde_json::Value) -> Vec<PathBuf> {
    request.get("paths").and_then(serde_json::Value::as_array).map(|paths| {
        paths.iter().filter_map(serde_json::Value::as_str).map(PathBuf::from).collect()
    }).unwrap_or_default()
}

fn section_key(family_id: &str) -> String {
    let mut parts = family_id.split('-');
    let first = parts.next().unwrap_or_default().to_string();
    parts.fold(first, |mut key, part| {
        let mut chars = part.chars();
        if let Some(head) = chars.next() { key.extend(head.to_uppercase()); key.push_str(chars.as_str()); }
        key
    })
}

fn parse_request(request: serde_json::Value) -> Result<ConvertToPdfRequest, ToolError> {
    if let Some(fields) = request.as_object() {
        let families = native_capabilities().iter().find(|capability| capability.command == COMMAND).map(|capability| capability.format_families.as_slice()).unwrap_or_default();
        if let Some(family) = families.iter().find(|family| !family.native_available && fields.contains_key(&section_key(&family.id))) {
            let name = family.id.replace('-', " ");
            let mut chars = name.chars();
            let name = chars.next().map(|head| head.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default();
            return Err(ToolError::unavailable(format!("{name} options come in a later update.")));
        }
    }
    serde_json::from_value(request).map_err(|error| ToolError::invalid_input(format!("These conversion options can't be used: {error}")))
}

fn convert<S: SystemFontLookup>(job: &ConvertJob<S>, input: PathBuf) -> JobOutcome {
    let extension = input.extension().and_then(|extension| extension.to_str()).unwrap_or_default().to_ascii_lowercase();
    let result = match extension.as_str() {
        "pdf" => return JobOutcome::skipped(input, SkipReason::new(SkipCode::AlreadyInTargetFormat, ALREADY_PDF)),
        "md" | "markdown" => markdown_file(job, &job.request.markdown.unwrap_or_default(), &input),
        "rtf" => rich_text(job, &input),
        _ => plain_text(job, &job.request.plain_text.unwrap_or_default(), &input),
    };
    result.unwrap_or_else(|error| JobOutcome::failure(input, error))
}

fn page_count(count: usize) -> String {
    if count == 1 { "1 page".to_string() } else { format!("{count} pages") }
}

fn save(job_location: &OutputLocation, input: &Path, mut document: Document) -> Result<PathBuf, ToolError> {
    let output = OutputNaming::reserve_destination(input, job_location, "", "pdf")
        .map_err(|error| ToolError::processing(format!("Could not reserve the PDF output: {error}")))?;
    document.save(output.path()).map_err(|error| ToolError::processing(format!("Could not write the PDF: {error}")))?;
    output.publish().map_err(|error| ToolError::processing(format!("Could not save the PDF: {error}")))
}

fn plain_text<S: SystemFontLookup>(job: &ConvertJob<S>, options: &PlainTextOptions, input: &Path) -> Result<JobOutcome, ToolError> {
    let bytes = fs::read(input).map_err(|error| ToolError::processing(format!("Could not read the file: {error}")))?;
    let (text, encoding) = decode(&bytes, options.encoding)?;
    let mut book = FontBook::new(&job.fonts);
    let face = book.face(options.font.family(), FontStyle::Regular)?;
    let size = options.page_size.unwrap_or(job.request.page_defaults.page_size).points();
    let margin = options.margins.unwrap_or(job.request.page_defaults.margins).points();
    let (width, height) = size;
    let text_width = width - 2. * margin;
    let lines_per_page = (((height - 2. * margin) / LEADING).floor() as usize).max(1);

    let mut measure = Measure { book: &mut book, face, widths: HashMap::new() };
    let mut pages: Vec<Vec<String>> = vec![Vec::new()];
    for (page_index, page) in text.split('\u{c}').enumerate() {
        if page_index > 0 { pages.push(Vec::new()); }
        for line in page.lines() {
            let line = expand_tabs(line);
            let wrapped = if options.wrap_lines { measure.wrap(&line, text_width)? } else { vec![line] };
            for row in wrapped {
                if pages.last().is_some_and(|rows| rows.len() >= lines_per_page) { pages.push(Vec::new()); }
                pages.last_mut().expect("pages always has one page").push(row);
            }
        }
    }

    let mut out = Vec::with_capacity(pages.len());
    for rows in &pages {
        let mut page = PageOut::default();
        for (index, row) in rows.iter().enumerate() {
            if row.trim().is_empty() { continue; }
            let baseline = height - margin - FONT_SIZE - index as f32 * LEADING;
            page.content.push_str(&book.text(face, FONT_SIZE, margin, baseline, row)?);
        }
        out.push(page);
    }
    let count = out.len();
    let document = assemble(&book, Assembly { size, pages: out, pictures: &[], bookmarks: &[], anchors: &BTreeMap::new(), title: None, author: None })?;
    let path = save(&job.request.output_location, input, document)?;
    Ok(JobOutcome::succeeded(input.to_path_buf(), vec![path], format!("{}. Read as {encoding}.", page_count(count))).with_fonts(book.reports()))
}

fn markdown_file<S: SystemFontLookup>(job: &ConvertJob<S>, options: &MarkdownOptions, input: &Path) -> Result<JobOutcome, ToolError> {
    let bytes = fs::read(input).map_err(|error| ToolError::processing(format!("Could not read the file: {error}")))?;
    let (text, _) = decode(&bytes, TextEncoding::Detect)?;
    let parsed = markdown::parse(&text, input.parent().unwrap_or(Path::new(".")));
    let size = options.page_size.unwrap_or(job.request.page_defaults.page_size).points();
    let geometry = Geometry::uniform(size.0, size.1, options.margins.unwrap_or(job.request.page_defaults.margins).points());
    let theme = Theme::new(options.body_font.family(), CODE_FAMILY, MARKDOWN_SIZE, options.code_style == CodeStyle::Shaded);

    let mut book = FontBook::new(&job.fonts);
    let mut body = Flow::new(&mut book, theme.clone(), &parsed.pictures, geometry);
    body.blocks(&parsed.blocks)?;
    let mut laid = body.finish();

    let mut contents = Vec::new();
    if options.table_of_contents && !laid.bookmarks.is_empty() {
        let mut offset = 1;
        loop {
            let mut flow = Flow::new(&mut book, theme.clone(), &parsed.pictures, geometry);
            flow.contents("Contents", &laid.bookmarks, offset)?;
            let pages = flow.finish().pages;
            if pages.len() == offset { contents = pages; break; }
            offset = pages.len();
        }
        for bookmark in &mut laid.bookmarks { bookmark.page += contents.len(); }
        for (page, _) in laid.anchors.values_mut() { *page += contents.len(); }
    }
    let mut pages = contents;
    pages.append(&mut laid.pages);
    let count = pages.len();
    let document = assemble(&book, Assembly { size, pages, pictures: &parsed.pictures, bookmarks: &laid.bookmarks, anchors: &laid.anchors, title: parsed.title.as_deref(), author: None })?;
    let path = save(&job.request.output_location, input, document)?;

    let mut detail = format!("{}.", page_count(count));
    match parsed.remote_images {
        0 => {}
        1 => detail.push_str(" Remote content left out: 1 image from the internet wasn't loaded."),
        count => detail.push_str(&format!(" Remote content left out: {count} images from the internet weren't loaded.")),
    }
    match parsed.unreadable_images {
        0 => {}
        1 => detail.push_str(" 1 local image couldn't be read and was left out."),
        count => detail.push_str(&format!(" {count} local images couldn't be read and were left out.")),
    }
    Ok(JobOutcome::succeeded(input.to_path_buf(), vec![path], detail).with_fonts(book.reports()))
}

fn rtf_geometry(page: rtf::PageSetup, defaults: &PageDefaults) -> Geometry {
    let (default_width, default_height) = defaults.page_size.points();
    let margin = defaults.margins.points();
    let side = |value: Option<f32>, fallback: f32| value.filter(|value| (MIN_PAGE_SIDE..=MAX_PAGE_SIDE).contains(value)).unwrap_or(fallback);
    let (width, height) = match (page.width, page.height) {
        (None, None) => (default_width, default_height),
        (width, height) => (side(width, default_width), side(height, default_height)),
    };
    let edge = |value: Option<f32>| value.filter(|value| *value >= 0.).unwrap_or(margin);
    let geometry = Geometry { width, height, left: edge(page.left), right: edge(page.right), top: edge(page.top), bottom: edge(page.bottom) };
    if geometry.width - geometry.left - geometry.right < MIN_PAGE_SIDE || geometry.height - geometry.top - geometry.bottom < MIN_PAGE_SIDE {
        return Geometry::uniform(width, height, margin.min(width / 4.).min(height / 4.));
    }
    geometry
}

fn rich_text<S: SystemFontLookup>(job: &ConvertJob<S>, input: &Path) -> Result<JobOutcome, ToolError> {
    let bytes = fs::read(input).map_err(|error| ToolError::processing(format!("Could not read the file: {error}")))?;
    let parsed = rtf::parse(&bytes)?;
    let geometry = rtf_geometry(parsed.page, &job.request.page_defaults);
    let mut theme = Theme::new(parsed.body_family.as_deref().unwrap_or(RTF_FAMILY), CODE_FAMILY, RTF_SIZE, false);
    theme.families = parsed.families.clone();
    theme.leading = 1.2;

    let mut book = FontBook::new(&job.fonts);
    let mut flow = Flow::new(&mut book, theme, &parsed.pictures, geometry);
    flow.blocks(&parsed.blocks)?;
    let laid = flow.finish();
    let count = laid.pages.len();
    let assembly = Assembly { size: (geometry.width, geometry.height), pages: laid.pages, pictures: &parsed.pictures, bookmarks: &[], anchors: &BTreeMap::new(), title: parsed.title.as_deref(), author: parsed.author.as_deref() };
    let document = assemble(&book, assembly)?;
    let path = save(&job.request.output_location, input, document)?;

    let mut detail = format!("{}.", page_count(count));
    match parsed.omitted_pictures {
        0 => {}
        1 => detail.push_str(" Omitted content: 1 picture couldn't be shown and was left out."),
        count => detail.push_str(&format!(" Omitted content: {count} pictures couldn't be shown and were left out.")),
    }
    Ok(JobOutcome::succeeded(input.to_path_buf(), vec![path], detail).with_fonts(book.reports()))
}

fn decode(bytes: &[u8], encoding: TextEncoding) -> Result<(String, &'static str), ToolError> {
    let strict = |encoding: &'static Encoding, label: &'static str, bytes: &[u8]| {
        encoding.decode_without_bom_handling_and_without_replacement(bytes)
            .map(|text| (text.into_owned(), label))
            .ok_or_else(|| ToolError::invalid_input(format!("This file isn't valid {label}. Pick another encoding.")))
    };
    let without_bom = |target: &'static Encoding| match Encoding::for_bom(bytes) {
        Some((found, length)) if found == target => &bytes[length..],
        _ => bytes,
    };
    let (text, label) = match encoding {
        TextEncoding::Utf8 => strict(UTF_8, "UTF-8", without_bom(UTF_8))?,
        TextEncoding::Utf16le => strict(UTF_16LE, "UTF-16 LE", without_bom(UTF_16LE))?,
        TextEncoding::Utf16be => strict(UTF_16BE, "UTF-16 BE", without_bom(UTF_16BE))?,
        TextEncoding::Windows1252 => (WINDOWS_1252.decode_without_bom_handling(bytes).0.into_owned(), "Windows-1252"),
        TextEncoding::Detect => match Encoding::for_bom(bytes) {
            Some((found, length)) => {
                let label = if found == UTF_8 { "UTF-8" } else if found == UTF_16LE { "UTF-16 LE" } else { "UTF-16 BE" };
                strict(found, label, &bytes[length..])?
            }
            None if bytes.contains(&0) => return Err(ToolError::invalid_input("This file doesn't look like plain text, so no PDF was made.")),
            None => match std::str::from_utf8(bytes) {
                Ok(text) => (text.to_string(), "UTF-8"),
                Err(_) => (WINDOWS_1252.decode_without_bom_handling(bytes).0.into_owned(), "Windows-1252"),
            },
        },
    };
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    Ok((text.chars().filter(|character| !character.is_control() || matches!(character, '\n' | '\t' | '\u{c}')).collect(), label))
}

struct Measure<'a, 'r, S: SystemFontLookup> {
    book: &'a mut FontBook<'r, S>,
    face: Face,
    widths: HashMap<char, f32>,
}

impl<S: SystemFontLookup> Measure<'_, '_, S> {
    fn width(&mut self, character: char) -> Result<f32, ToolError> {
        if let Some(width) = self.widths.get(&character) { return Ok(*width); }
        let width = self.book.char_width(self.face, character, FONT_SIZE)?;
        self.widths.insert(character, width);
        Ok(width)
    }

    fn wrap(&mut self, line: &str, limit: f32) -> Result<Vec<String>, ToolError> {
        let mut rows = Vec::new();
        let mut row = String::new();
        let mut row_width = 0.;
        for word in line.split_inclusive(' ') {
            let mut word_width = 0.;
            for character in word.chars() { word_width += self.width(character)?; }
            let visible = word_width - if word.ends_with(' ') { self.width(' ')? } else { 0. };
            if row_width + visible <= limit {
                row.push_str(word);
                row_width += word_width;
                continue;
            }
            if !row.is_empty() {
                rows.push(row.trim_end().to_string());
                row = String::new();
                row_width = 0.;
            }
            for character in word.chars() {
                let width = self.width(character)?;
                if row_width + width > limit && !row.is_empty() && character != ' ' {
                    rows.push(std::mem::take(&mut row));
                    row_width = 0.;
                }
                row.push(character);
                row_width += width;
            }
        }
        rows.push(row.trim_end().to_string());
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::contracts::{ErrorKind, OutcomeState};

    fn succeeded(outcome: &JobOutcome) -> bool { matches!(outcome.state, OutcomeState::Succeeded { .. }) }
    fn skip(outcome: &JobOutcome) -> &SkipReason { match &outcome.state { OutcomeState::Skipped { reason } => reason, other => panic!("not skipped: {other:?}") } }
    use crate::kit::pdf::font_resolver::{BundledFonts, FontSource, SystemFace};
    use lopdf::Object;
    use serde_json::json;

    struct NoSystemFonts;
    impl SystemFontLookup for NoSystemFonts {
        fn faces(&self, _family: &str) -> Vec<SystemFace> { Vec::new() }
    }

    fn fonts() -> Result<FontResolver<NoSystemFonts>, ToolError> {
        Ok(FontResolver::new(BundledFonts::discover(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/fonts"))?, NoSystemFonts))
    }

    fn sandbox(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("toolbox_convert_to_pdf_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn convert_paths(paths: &[&Path], options: serde_json::Value) -> Vec<JobOutcome> {
        let mut request = json!({ "paths": paths, "outputLocation": "alongsideInput" });
        request.as_object_mut().unwrap().extend(options.as_object().unwrap().clone());
        run_with(&RunEvents::silent(), request, fonts)
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut names = fs::read_dir(folder).unwrap().flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        names.sort();
        names
    }

    fn plain(wrap: bool) -> serde_json::Value {
        json!({ "plainText": { "font": "monospace", "encoding": "detect", "wrapLines": wrap } })
    }

    #[test]
    fn non_utf8_text_with_a_long_line_becomes_a_wrapped_searchable_pdf() {
        let root = sandbox("long_line");
        let input = root.join("caf\u{e9}-log.txt");
        let long_line = (0..120).map(|index| format!("word{index}")).collect::<Vec<_>>().join(" ");
        let mut bytes = b"Caf\xe9 cr\xe8me na\xefve\n".to_vec();
        bytes.extend_from_slice(long_line.as_bytes());
        bytes.push(b'\n');
        let original = bytes.clone();
        fs::write(&input, &bytes).unwrap();

        let outcomes = convert_paths(&[&input], json!({ "plainText": { "pageSize": "letter", "font": "monospace", "encoding": "detect", "wrapLines": true } }));
        assert_eq!(outcomes.len(), 1);
        let outcome = &outcomes[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert_eq!(outcome.detail, "1 page. Read as Windows-1252.");
        let output = root.join("caf\u{e9}-log.pdf");
        assert_eq!(outcome.output_paths(), [output.clone()]);
        assert_eq!(outcome.fonts.len(), 1);
        assert_eq!(outcome.fonts[0].requested_family, "Courier New");
        assert_eq!(outcome.fonts[0].font_used, "Liberation Mono");
        assert_eq!(outcome.fonts[0].source, FontSource::BundledFallback);

        let document = Document::load(&output).unwrap();
        let pages = document.get_pages();
        assert_eq!(pages.len(), 1);
        let page = document.get_dictionary(pages[&1]).unwrap();
        let media_box = page.get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|value| value.as_float().unwrap()).collect::<Vec<_>>();
        assert_eq!(media_box, [0., 0., 612., 792.]);
        let text = document.extract_text(&[1]).unwrap();
        assert!(text.contains("Caf\u{e9} cr\u{e8}me na\u{ef}ve"), "{text}");
        assert!(text.contains("word0") && text.contains("word119"), "{text}");
        let content = document.get_page_content(pages[&1]);
        let rows = String::from_utf8_lossy(&content).matches("BT ").count();
        assert!(rows > 2, "the long line wraps onto several rows, found {rows}");
        assert_eq!(fs::read(&input).unwrap(), original);
        assert_eq!(names(&root), ["caf\u{e9}-log.pdf", "caf\u{e9}-log.txt"]);
    }

    #[test]
    fn many_lines_paginate_and_unwrapped_lines_stay_on_one_row() {
        let root = sandbox("paginate");
        let input = root.join("lines.txt");
        let text = (0..130).map(|index| format!("line {index}\tend")).collect::<Vec<_>>().join("\n");
        fs::write(&input, format!("{text}\n{}", "x".repeat(400))).unwrap();
        let outcomes = convert_paths(&[&input], json!({ "pageDefaults": { "pageSize": "a4", "margins": "normal" }, "plainText": { "font": "sans", "encoding": "utf8", "wrapLines": false } }));
        assert!(succeeded(&outcomes[0]), "{:?}", outcomes[0]);
        assert_eq!(outcomes[0].detail, "3 pages. Read as UTF-8.");
        assert_eq!(outcomes[0].fonts[0].font_used, "Liberation Sans");
        let document = Document::load(root.join("lines.pdf")).unwrap();
        assert_eq!(document.get_pages().len(), 3);
        assert!(document.extract_text(&[3]).unwrap().contains(&"x".repeat(400)));
    }

    #[test]
    fn pdf_inputs_are_skipped_and_a_damaged_file_fails_alone() {
        let root = sandbox("mixed");
        let pdf = root.join("already.PDF");
        let damaged = root.join("damaged.txt");
        let good = root.join("good.txt");
        fs::write(&pdf, b"%PDF-1.7 not really").unwrap();
        fs::write(&damaged, b"\x00\x01binary\x00junk").unwrap();
        fs::write(&good, "Hello").unwrap();
        let before = [fs::read(&pdf).unwrap(), fs::read(&damaged).unwrap(), fs::read(&good).unwrap()];

        let outcomes = convert_paths(&[&pdf, &damaged, &good], plain(true));
        assert_eq!(skip(&outcomes[0]), &SkipReason::new(SkipCode::AlreadyInTargetFormat, ALREADY_PDF));
        assert!(outcomes[0].output_paths().is_empty());
                assert_eq!(outcomes[1].error().unwrap().kind, ErrorKind::InvalidInput);
        assert!(succeeded(&outcomes[2]));
        assert_eq!(outcomes[2].detail, "1 page. Read as UTF-8.");
        assert_eq!([fs::read(&pdf).unwrap(), fs::read(&damaged).unwrap(), fs::read(&good).unwrap()], before);
        assert_eq!(names(&root), ["already.PDF", "damaged.txt", "good.pdf", "good.txt"]);
    }

    #[test]
    fn forced_encodings_decode_or_fail_without_leaving_files() {
        let root = sandbox("encodings");
        let utf16 = root.join("wide.txt");
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "\u{3b1}lpha".encode_utf16() { bytes.extend_from_slice(&unit.to_le_bytes()); }
        fs::write(&utf16, &bytes).unwrap();
        let detected = convert_paths(&[&utf16], plain(true));
        assert_eq!(detected[0].detail, "1 page. Read as UTF-16 LE.");
        assert!(Document::load(root.join("wide.pdf")).unwrap().extract_text(&[1]).unwrap().contains("\u{3b1}lpha"));

        let latin = root.join("latin.txt");
        fs::write(&latin, b"caf\xe9").unwrap();
        let forced = convert_paths(&[&latin], json!({ "plainText": { "font": "serif", "encoding": "utf8", "wrapLines": true } }));
        assert_eq!(forced[0].error().unwrap().kind, ErrorKind::InvalidInput);
        assert_eq!(forced[0].error().unwrap().message, "This file isn't valid UTF-8. Pick another encoding.");
        assert_eq!(names(&root), ["latin.txt", "wide.pdf", "wide.txt"]);
    }

    #[test]
    fn repeated_runs_never_overwrite_and_are_byte_identical() {
        let root = sandbox("repeat");
        let input = root.join("notes.txt");
        fs::write(&input, "Same text\nevery time\n").unwrap();
        for _ in 0..3 { assert!(succeeded(&convert_paths(&[&input], plain(true))[0])); }
        assert_eq!(names(&root), ["notes-1.pdf", "notes-2.pdf", "notes.pdf", "notes.txt"]);
        let first = fs::read(root.join("notes.pdf")).unwrap();
        assert_eq!(fs::read(root.join("notes-1.pdf")).unwrap(), first);
        assert_eq!(fs::read(root.join("notes-2.pdf")).unwrap(), first);
    }

    #[test]
    fn unknown_options_are_invalid_and_unshipped_families_are_unavailable() {
        let root = sandbox("options");
        let input = root.join("notes.txt");
        fs::write(&input, "text").unwrap();
        for options in [
            json!({ "plainText": { "font": "monospace", "encoding": "detect", "wrapLines": true, "fontSize": 12 } }),
            json!({ "plainText": { "font": "comic", "encoding": "detect", "wrapLines": true } }),
            json!({ "pageDefaults": { "pageSize": "a3" } }),
            json!({ "colour": true }),
        ] {
            let outcomes = convert_paths(&[&input], options.clone());
            assert_eq!(outcomes.len(), 1);
            assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::InvalidInput, "{options}");
        }
        let outcomes = convert_paths(&[&input], json!({ "markdown": { "bodyFont": "sans", "codeStyle": "shaded", "tableOfContents": false, "toc": true } }));
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::InvalidInput);
        let outcomes = convert_paths(&[&input], json!({ "tables": {} }));
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::Unavailable);
        assert_eq!(outcomes[0].error().unwrap().message, "Tables options come in a later update.");
        let outcomes = convert_paths(&[&input], json!({ "webPages": {} }));
        assert_eq!(outcomes[0].error().unwrap().message, "Web pages options come in a later update.");
        let mobi = root.join("book.mobi");
        fs::write(&mobi, "mobi").unwrap();
        let outcomes = convert_paths(&[&mobi, &input], plain(true));
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::Unavailable);
        assert!(succeeded(&outcomes[1]));
        assert_eq!(names(&root), ["book.mobi", "notes.pdf", "notes.txt"]);
    }

    #[test]
    fn missing_glyphs_and_unavailable_fonts_fail_without_temporary_files() {
        let root = sandbox("missing_glyph");
        let input = root.join("emoji.txt");
        fs::write(&input, "rocket \u{1F680}").unwrap();
        let outcomes = convert_paths(&[&input], plain(true));
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::Processing);
        assert!(outcomes[0].error().unwrap().message.starts_with("The font has no glyph for"));
        assert_eq!(names(&root), ["emoji.txt"]);

        let request = json!({ "paths": [input], "outputLocation": "alongsideInput" });
        let outcomes = run_with::<NoSystemFonts, _>(&RunEvents::silent(), request, || Err(ToolError::unavailable("fonts missing")));
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::Unavailable);
        assert_eq!(names(&root), ["emoji.txt"]);
    }

    fn markdown_options(table_of_contents: bool) -> serde_json::Value {
        json!({ "markdown": { "bodyFont": "sans", "codeStyle": "shaded", "tableOfContents": table_of_contents } })
    }

    fn links(document: &Document) -> Vec<lopdf::Dictionary> {
        document.objects.values().filter_map(|object| object.as_dict().ok())
            .filter(|dictionary| dictionary.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Link".as_slice()))
            .cloned().collect()
    }

    fn outline_titles(document: &Document) -> Vec<String> {
        let root = document.catalog().unwrap().get(b"Outlines").unwrap().as_reference().unwrap();
        let mut titles = Vec::new();
        let mut stack = vec![document.get_dictionary(root).unwrap().get(b"First").unwrap().as_reference().unwrap()];
        while let Some(id) = stack.pop() {
            let item = document.get_dictionary(id).unwrap();
            let title = item.get(b"Title").unwrap().as_str().unwrap();
            titles.push(match title.strip_prefix(&[0xFE, 0xFF]) {
                Some(units) => String::from_utf16(&units.chunks(2).map(|pair| u16::from_be_bytes([pair[0], pair[1]])).collect::<Vec<_>>()).unwrap(),
                None => String::from_utf8_lossy(title).into_owned(),
            });
            if let Ok(next) = item.get(b"Next").and_then(Object::as_reference) { stack.push(next); }
            if let Ok(first) = item.get(b"First").and_then(Object::as_reference) { stack.push(first); }
        }
        titles
    }

    fn page_of(document: &Document, destination: &Object) -> u32 {
        let target = destination.as_array().unwrap()[0].as_reference().unwrap();
        *document.get_pages().iter().find(|(_, id)| **id == target).unwrap().0
    }

    #[test]
    fn markdown_keeps_structure_local_images_bookmarks_and_links_without_fetching_remote_images() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let root = sandbox("markdown");
        fs::create_dir_all(root.join("images")).unwrap();
        image::RgbaImage::from_fn(8, 6, |x, _| image::Rgba([200, 40, 40, if x < 4 { 255 } else { 128 }])).save(root.join("images/chart one.png")).unwrap();
        let input = root.join("release-notes.md");
        let source = format!("---\ntitle: Release notes 2.0\n---\n# Overview\n\nIntro with **bold**, *italic*, ~~old~~, `inline code` and a [docs link](https://example.com/docs) plus a [jump](#install-steps).\n\n## Install steps\n\n| Step | Command |\n|------|:-------:|\n| fetch | `cargo build` |\n| run | toolbox |\n\n```rust\nfn main() {{\n    println!(\"hello\");\n}}\n```\n\n![Local chart](images/chart%20one.png)\n\n![Remote logo](http://127.0.0.1:{port}/logo.png)\n\n> Quoted advice\n\n1. first item\n2. second item\n   - nested point\n\n### Notes\n\nDone.\n");
        fs::write(&input, &source).unwrap();

        let outcomes = convert_paths(&[&input], markdown_options(false));
        let outcome = &outcomes[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert_eq!(outcome.detail, "1 page. Remote content left out: 1 image from the internet wasn't loaded.");
        assert!(matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock), "the remote image was requested");
        assert!(outcome.fonts.iter().any(|font| font.requested_family == "Arial"));
        assert!(outcome.fonts.iter().any(|font| font.requested_family == "Courier New"));

        let output = root.join("release-notes.pdf");
        let document = Document::load(&output).unwrap();
        let text = document.extract_text(&[1]).unwrap();
        for expected in ["Overview", "Intro with", "bold", "inline code", "docs link", "Install steps", "Step", "Command", "fetch", "cargo build", "toolbox", "println!(\"hello\");", "Quoted advice", "first item", "nested point", "Notes", "Done."] {
            assert!(text.contains(expected), "missing {expected:?} in {text}");
        }
        assert!(text.contains("Remote logo"), "the left-out image keeps its alt text");
        let images = document.objects.values().filter_map(|object| object.as_stream().ok()).filter(|stream| stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image".as_slice())).collect::<Vec<_>>();
        assert!(images.iter().any(|image| image.dict.get(b"Width").unwrap().as_i64().unwrap() == 8 && image.dict.has(b"SMask")), "the local image is embedded");
        assert_eq!(outline_titles(&document), ["Overview", "Install steps", "Notes"]);
        let links = links(&document);
        assert!(links.iter().any(|link| link.get(b"A").and_then(Object::as_dict).and_then(|action| action.get(b"URI")).and_then(Object::as_str).ok() == Some(b"https://example.com/docs".as_slice())));
        assert!(links.iter().any(|link| link.has(b"Dest")), "the in-document link jumps to its heading");
        let info = document.trailer.get(b"Info").and_then(Object::as_reference).unwrap();
        assert_eq!(document.get_dictionary(info).unwrap().get(b"Title").unwrap().as_str().unwrap(), b"Release notes 2.0");
        assert_eq!(fs::read_to_string(&input).unwrap(), source);
        assert_eq!(names(&root), ["images", "release-notes.md", "release-notes.pdf"]);
    }

    #[test]
    fn table_of_contents_is_off_by_default_and_lists_every_heading_when_on() {
        let root = sandbox("contents");
        let input = root.join("guide.markdown");
        let headings = ["Getting started", "Configuration", "Advanced use", "Caf\u{e9} appendix"];
        let source = headings.iter().enumerate().map(|(index, heading)| format!("{} {heading}\n\n{}\n", "#".repeat(index.min(2) + 1), "Paragraph text. ".repeat(200))).collect::<String>();
        fs::write(&input, &source).unwrap();

        let plain = convert_paths(&[&input], json!({}));
        assert!(succeeded(&plain[0]), "{:?}", plain[0]);
        let without = Document::load(root.join("guide.pdf")).unwrap();
        assert!(!without.extract_text(&[1]).unwrap().contains("Contents"));
        assert!(without.trailer.get(b"Info").and_then(Object::as_reference).is_ok(), "the first heading becomes the title");

        let with = convert_paths(&[&input], markdown_options(true));
        assert!(succeeded(&with[0]), "{:?}", with[0]);
        let document = Document::load(root.join("guide-1.pdf")).unwrap();
        assert_eq!(document.get_pages().len(), without.get_pages().len() + 1);
        assert_eq!(with[0].detail, format!("{} pages.", without.get_pages().len() + 1));
        let contents = document.extract_text(&[1]).unwrap();
        assert!(contents.contains("Contents"), "{contents}");
        for heading in headings { assert!(contents.contains(heading), "missing {heading:?} in {contents}"); }
        assert!(!document.extract_text(&[2]).unwrap().contains("Contents"));
        let contents_page = document.get_pages()[&1];
        let entries = document.get_dictionary(contents_page).unwrap().get(b"Annots").unwrap().as_array().unwrap().iter()
            .map(|annotation| document.get_dictionary(annotation.as_reference().unwrap()).unwrap().get(b"Dest").unwrap().clone()).collect::<Vec<_>>();
        assert_eq!(entries.len(), headings.len());
        assert!(entries.iter().all(|destination| page_of(&document, destination) > 1));
        assert_eq!(outline_titles(&document), headings);
        assert_eq!(names(&root), ["guide-1.pdf", "guide.markdown", "guide.pdf"]);
    }

    #[test]
    fn unreadable_local_images_are_noted_and_markdown_reruns_are_identical() {
        let root = sandbox("markdown_repeat");
        let input = root.join("notes.md");
        fs::write(root.join("broken.png"), "not an image").unwrap();
        fs::write(&input, "# Notes\n\n![missing](missing.png) ![broken](broken.png) ![a](https://example.com/a.png) ![b](//cdn.example.com/b.gif)\n\n```\nplain code\n```\n").unwrap();
        let first = convert_paths(&[&input], json!({ "markdown": { "pageSize": "letter", "margins": "narrow", "bodyFont": "serif", "codeStyle": "plain", "tableOfContents": false } }));
        assert_eq!(first[0].detail, "1 page. Remote content left out: 2 images from the internet weren't loaded. 2 local images couldn't be read and were left out.");
        assert!(first[0].fonts.iter().any(|font| font.requested_family == "Times New Roman"));
        let second = convert_paths(&[&input], json!({ "markdown": { "pageSize": "letter", "margins": "narrow", "bodyFont": "serif", "codeStyle": "plain", "tableOfContents": false } }));
        assert!(succeeded(&second[0]));
        assert_eq!(fs::read(root.join("notes.pdf")).unwrap(), fs::read(root.join("notes-1.pdf")).unwrap());
        let document = Document::load(root.join("notes.pdf")).unwrap();
        let page = document.get_dictionary(document.get_pages()[&1]).unwrap();
        let media_box = page.get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|value| value.as_float().unwrap()).collect::<Vec<_>>();
        assert_eq!(media_box, [0., 0., 612., 792.]);
        assert!(!String::from_utf8_lossy(&document.get_page_content(document.get_pages()[&1])).contains(" re f"), "plain code has no shading");
        assert_eq!(names(&root), ["broken.png", "notes-1.pdf", "notes.md", "notes.pdf"]);
    }

    fn rtf_fixture() -> String {
        let mut png = Vec::new();
        image::RgbaImage::from_fn(8, 6, |x, _| image::Rgba([30, 90, 200, if x < 4 { 255 } else { 128 }]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let hex = png.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        [
            r"{\rtf1\ansi\ansicpg1252\deff0",
            r"{\fonttbl{\f0\froman\fcharset0 Times New Roman;}{\f1\fswiss\fcharset0 Arial;}{\f2\fmodern\fcharset0 Courier New;}{\f3\fnil\fcharset2 Symbol;}}",
            r"{\colortbl;\red200\green0\blue0;}",
            r"{\info{\title Quarterly report}{\author Dana Whitfield}{\operator Someone Else}}",
            r"\paperw12240\paperh15840\margl1440\margr1440\margt1080\margb1080",
            r"{\header \pard\plain\f1 Header text left out\par}",
            r"\pard\qc\sa240{\f1\fs36\b Quarterly summary\par}",
            r"\pard\plain\f0\fs24 Plain serif with {\b bold}, {\i italic}, {\ul underlined}, {\strike struck} and {\cf1 red} words in Caf\'e9 Ren\u233?e{\v  hidden words}{\deleted  removed text}{\revised  added text}.",
            r"{\*\atnid A}{\*\atnauthor Reviewer}\chatn{\*\annotation{\*\atnref 1}\pard Comment text\par}\par",
            r#"\pard\li720{\f2 Monospace line}\line {\field{\*\fldinst HYPERLINK "https://example.com/rtf"}{\fldrslt Example link}}\par"#,
            r"{\pntext\f3\'b7\tab}\pard\li720 Bullet item\par",
            r"\trowd\cellx3000\cellx6000\pard\intbl\b Region\cell Revenue\cell\b0\row",
            r"\trowd\cellx3000\cellx6000\pard\intbl North\cell 1,200\cell\row",
            r"\pard After the table\par",
            &format!(r"{{\*\shppict{{\pict\pngblip\picw8\pich6\picwgoal1200\pichgoal900 {hex}}}}}{{\nonshppict{{\pict\wmetafile8 0102}}}}\par"),
            r"{\pict\wmetafile8\picw10\pich10 010203}\par",
            r"\page Second page\par",
            "}",
        ].join("\n")
    }

    fn info(document: &Document, key: &[u8]) -> Vec<u8> {
        let info = document.trailer.get(b"Info").and_then(Object::as_reference).unwrap();
        document.get_dictionary(info).unwrap().get(key).unwrap().as_str().unwrap().to_vec()
    }

    #[test]
    fn rich_text_keeps_styles_tables_pictures_page_setup_and_requested_fonts() {
        let root = sandbox("rtf");
        let input = root.join("report.rtf");
        let source = rtf_fixture();
        fs::write(&input, &source).unwrap();

        let outcomes = convert_paths(&[&input], json!({ "pageDefaults": { "pageSize": "a4", "margins": "wide" } }));
        let outcome = &outcomes[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert_eq!(outcome.detail, "2 pages. Omitted content: 1 picture couldn't be shown and was left out.");
        for family in ["Times New Roman", "Arial", "Courier New"] {
            assert!(outcome.fonts.iter().any(|font| font.requested_family == family && font.source == FontSource::BundledFallback), "{family} missing from {:?}", outcome.fonts);
        }
        assert!(!outcome.fonts.iter().any(|font| font.requested_family == "Symbol"));

        let document = Document::load(root.join("report.pdf")).unwrap();
        assert_eq!(document.get_pages().len(), 2);
        let text = document.extract_text(&[1]).unwrap();
        for expected in ["Quarterly summary", "Plain serif with", "bold", "italic", "underlined", "struck", "red", "Caf\u{e9} Ren\u{e9}e", "added text", "Monospace line", "Example link", "\u{2022}", "Bullet item", "Region", "Revenue", "North", "1,200", "After the table"] {
            assert!(text.contains(expected), "missing {expected:?} in {text}");
        }
        for absent in ["hidden words", "removed text", "Comment text", "Reviewer", "Header text"] {
            assert!(!text.contains(absent), "{absent:?} should be left out of {text}");
        }
        assert!(document.extract_text(&[2]).unwrap().contains("Second page"));
        let images = document.objects.values().filter_map(|object| object.as_stream().ok()).filter(|stream| stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image".as_slice())).collect::<Vec<_>>();
        let pictures = images.iter().filter(|image| image.dict.has(b"SMask")).collect::<Vec<_>>();
        assert_eq!(pictures.len(), 1, "only the PNG is embedded; the metafiles are left out");
        assert_eq!(pictures[0].dict.get(b"Width").unwrap().as_i64().unwrap(), 8);
        let content = String::from_utf8_lossy(&document.get_page_content(document.get_pages()[&1])).into_owned();
        assert!(content.contains("60.00 0 0 45.00 72.00"), "the picture keeps its stored size and the document's left margin");
        assert!(content.contains("0.78431374 0 0 rg"), "the red colour is applied");
        assert!(links(&document).iter().any(|link| link.get(b"A").and_then(Object::as_dict).and_then(|action| action.get(b"URI")).and_then(Object::as_str).ok() == Some(b"https://example.com/rtf".as_slice())));
        let media_box = document.get_dictionary(document.get_pages()[&1]).unwrap().get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|value| value.as_float().unwrap()).collect::<Vec<_>>();
        assert_eq!(media_box, [0., 0., 612., 792.], "the document's own page setup wins");
        assert_eq!(info(&document, b"Title"), b"Quarterly report");
        assert_eq!(info(&document, b"Author"), b"Dana Whitfield");

        assert!(succeeded(&convert_paths(&[&input], json!({ "pageDefaults": { "pageSize": "a4", "margins": "wide" } }))[0]));
        assert_eq!(fs::read(root.join("report.pdf")).unwrap(), fs::read(root.join("report-1.pdf")).unwrap());
        assert_eq!(fs::read_to_string(&input).unwrap(), source);
        assert_eq!(names(&root), ["report-1.pdf", "report.pdf", "report.rtf"]);
    }

    #[test]
    fn a_damaged_rtf_fails_alone_and_rtf_without_page_setup_uses_page_defaults() {
        let root = sandbox("rtf_batch");
        let unclosed = root.join("unclosed.rtf");
        let not_rtf = root.join("renamed.rtf");
        let good = root.join("letter.rtf");
        let notes = root.join("notes.txt");
        fs::write(&unclosed, r"{\rtf1\ansi {\b bold never closes\par").unwrap();
        fs::write(&not_rtf, "just text with an rtf name").unwrap();
        fs::write(&good, r"{\rtf1\ansi{\fonttbl{\f0\fswiss Helvetica;}}\f0 Dear reader,\par}").unwrap();
        fs::write(&notes, "notes").unwrap();

        let outcomes = convert_paths(&[&unclosed, &good, &not_rtf, &notes], json!({ "pageDefaults": { "pageSize": "letter", "margins": "narrow" } }));
        for outcome in [&outcomes[0], &outcomes[2]] {
            let error = outcome.error().unwrap();
            assert_eq!((error.kind.clone(), error.message.as_str()), (ErrorKind::InvalidInput, "This RTF file is damaged or incomplete, so no PDF was made."));
        }
        assert!(succeeded(&outcomes[1]) && succeeded(&outcomes[3]), "{outcomes:?}");
        assert_eq!(outcomes[1].detail, "1 page.");
        assert!(outcomes[1].fonts.iter().any(|font| font.requested_family == "Helvetica"));
        let document = Document::load(root.join("letter.pdf")).unwrap();
        assert!(document.extract_text(&[1]).unwrap().contains("Dear reader,"));
        assert!(document.trailer.get(b"Info").is_err(), "no title or author means no Info dictionary");
        let media_box = document.get_dictionary(document.get_pages()[&1]).unwrap().get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|value| value.as_float().unwrap()).collect::<Vec<_>>();
        assert_eq!(media_box, [0., 0., 612., 792.]);
        assert!(String::from_utf8_lossy(&document.get_page_content(document.get_pages()[&1])).contains(" 36.00 "), "narrow margins apply");
        assert_eq!(names(&root), ["letter.pdf", "letter.rtf", "notes.pdf", "notes.txt", "renamed.rtf", "unclosed.rtf"]);

        let outcomes = convert_paths(&[&good], json!({ "richText": {} }));
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::InvalidInput);
    }
}
