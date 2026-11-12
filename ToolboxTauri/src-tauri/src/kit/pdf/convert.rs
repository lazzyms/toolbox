use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8, WINDOWS_1252};
use lopdf::Document;
use serde::Deserialize;

use crate::kit::common::batch_runner::{BatchRunner, RunEvents};
use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::common::limits;
use crate::kit::contracts::{native_capabilities, OutcomePointer, OutcomeWarning, SkipCode, SkipReason, ToolError, WarningKind};
use crate::kit::office;
use crate::kit::pdf::font_resolver::{FontResolver, FontSource, FontStyle, SystemFontLookup};
use crate::kit::pdf::layout::{assemble, expand_tabs, Align, Assembly, Block, Bookmark, Face, FontBook, Flow, Geometry, Omitted, PageOut, PageSetup, Paragraph, Span, Style, Theme};
use crate::kit::pdf::presentation::Include;
use crate::kit::pdf::{delimited, docx, markdown, odp, ods, odt, pptx, rtf, xlsx};

pub const COMMAND: &str = "convert_to_pdf";
const ALREADY_PDF: &str = "Already a PDF. Nothing was made.";
const FONT_SIZE: f32 = 10.;
const LEADING: f32 = 13.;
const MARKDOWN_SIZE: f32 = 11.;
const CODE_FAMILY: &str = "Courier New";
const RTF_FAMILY: &str = "Times New Roman";
const RTF_SIZE: f32 = 12.;
const BAND_DISTANCE: f32 = 36.;
const LOCKED: &str = "It's password-protected. Unlock it in Lock & Unlock, then convert the unlocked copy.";
const MIN_PAGE_SIDE: f32 = 72.;
const MAX_PAGE_SIDE: f32 = 14_400.;
const TABLE_FAMILY: &str = "Arial";
const TABLE_SIZE: f32 = 10.;
const MIN_TABLE_SIZE: f32 = 4.;
const SLIDE_FAMILY: &str = "Arial";
const SLIDE_SIZE: f32 = 18.;
const NOTES_SIZE: f32 = 12.;
const NOTES_MARGIN: f32 = 36.;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Delimiter { #[default] Auto, Comma, Semicolon, Tab, Pipe }

impl Delimiter {
    fn resolve(self, extension: &str) -> (u8, &'static str) {
        match self {
            Self::Auto if extension == "tsv" => (b'\t', "tab"),
            Self::Auto | Self::Comma => (b',', "comma"),
            Self::Semicolon => (b';', "semicolon"),
            Self::Tab => (b'\t', "tab"),
            Self::Pipe => (b'|', "pipe"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Orientation { Portrait, #[default] Landscape }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TableFit { #[default] Width, None }

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableOptions {
    pub delimiter: Delimiter,
    pub header_row: bool,
    pub encoding: TextEncoding,
    pub orientation: Orientation,
    pub fit: TableFit,
}

impl Default for TableOptions {
    fn default() -> Self { Self { delimiter: Delimiter::default(), header_row: true, encoding: TextEncoding::default(), orientation: Orientation::default(), fit: TableFit::default() } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SheetChoice { All, #[default] Visible }

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpreadsheetOptions {
    pub sheets: SheetChoice,
    pub print_areas: bool,
    pub fit_to_width: bool,
}

impl Default for SpreadsheetOptions {
    fn default() -> Self { Self { sheets: SheetChoice::default(), print_areas: true, fit_to_width: true } }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationOptions {
    pub speaker_notes: bool,
    pub hidden_slides: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConvertToPdfRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
    #[serde(default)] pub page_defaults: PageDefaults,
    #[serde(default)] pub plain_text: Option<PlainTextOptions>,
    #[serde(default)] pub markdown: Option<MarkdownOptions>,
    #[serde(default)] pub tables: Option<TableOptions>,
    #[serde(default)] pub spreadsheets: Option<SpreadsheetOptions>,
    #[serde(default)] pub presentations: Option<PresentationOptions>,
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
        "docx" | "odt" => word_file(job, &input, &extension),
        "csv" | "tsv" => table_file(job, &job.request.tables.unwrap_or_default(), &input, &extension),
        "xlsx" | "ods" => spreadsheet_file(job, &job.request.spreadsheets.unwrap_or_default(), &input, &extension),
        "pptx" | "odp" => presentation_file(job, &job.request.presentations.unwrap_or_default(), &input, &extension),
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

fn page_geometry(page: PageSetup, defaults: &PageDefaults) -> Geometry {
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
    let geometry = page_geometry(parsed.page, &job.request.page_defaults);
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

    let outcome = JobOutcome::succeeded(input.to_path_buf(), vec![path], format!("{}.", page_count(count))).with_fonts(book.reports());
    Ok(with_notes(outcome, [omitted_content(Omitted { pictures: parsed.omitted_pictures, ..Omitted::default() })]))
}

fn with_notes(outcome: JobOutcome, notes: impl IntoIterator<Item = Option<OutcomeWarning>>) -> JobOutcome {
    notes.into_iter().flatten().fold(outcome, JobOutcome::with_warning)
}

fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn counted(count: usize, one: &str, many: &str) -> Option<String> {
    match count {
        0 => None,
        1 => Some(format!("1 {one}")),
        count => Some(format!("{count} {many}")),
    }
}

fn omitted_content(omitted: Omitted) -> Option<OutcomeWarning> {
    let mut sentences = Vec::new();
    let unstored = [
        counted(omitted.charts, "chart", "charts"),
        counted(omitted.diagrams, "diagram", "diagrams"),
        counted(omitted.objects, "embedded object", "embedded objects"),
    ].into_iter().flatten().collect::<Vec<_>>();
    if !unstored.is_empty() {
        let single = omitted.charts + omitted.diagrams + omitted.objects == 1;
        let (has, left) = if single { ("has", "it was") } else { ("have", "they were") };
        sentences.push(format!("{} {has} no stored image, so {left} left out.", listed(&unstored)));
    }
    match omitted.pictures {
        0 => {}
        1 => sentences.push("1 picture couldn't be shown and was left out.".to_string()),
        count => sentences.push(format!("{count} pictures couldn't be shown and were left out.")),
    }
    (!sentences.is_empty()).then(|| OutcomeWarning { kind: WarningKind::OmittedContent, message: format!("Omitted content: {}", sentences.join(" ")) })
}

fn approximate_layout(floats: &[usize]) -> Option<OutcomeWarning> {
    approximate_placement(floats, ("floating object", "floating objects"), ("page", "pages"))
}

/// Notes items in `floats` (one page or slide index each, counted from 0) that were placed approximately, naming the pages or slides they were on.
fn approximate_placement(floats: &[usize], (one, many): (&str, &str), (unit, units): (&str, &str)) -> Option<OutcomeWarning> {
    let mut pages = floats.iter().map(|page| page + 1).collect::<Vec<_>>();
    pages.sort_unstable();
    pages.dedup();
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for page in pages {
        match runs.last_mut() {
            Some((_, end)) if *end + 1 == page => *end = page,
            _ => runs.push((page, page)),
        }
    }
    let spans = runs.iter().flat_map(|&(start, end)| match end - start {
        0 => vec![start.to_string()],
        1 => vec![start.to_string(), end.to_string()],
        _ => vec![format!("{start}\u{2013}{end}")],
    }).collect::<Vec<_>>();
    let pages = match spans.as_slice() {
        [] => return None,
        [page] if !page.contains('\u{2013}') => format!("{unit} {page}"),
        _ => format!("{units} {}", listed(&spans)),
    };
    let objects = if floats.len() == 1 { format!("1 {one} on {pages} was") } else { format!("{} {many} on {pages} were", floats.len()) };
    Some(OutcomeWarning { kind: WarningKind::ApproximateLayout, message: format!("Approximate layout: {objects} placed as close as possible.") })
}

fn word_file<S: SystemFontLookup>(job: &ConvertJob<S>, input: &Path, extension: &str) -> Result<JobOutcome, ToolError> {
    let bytes = fs::read(input).map_err(|error| ToolError::processing(format!("Could not read the file: {error}")))?;
    let locked = ToolError::invalid_input(LOCKED).with_pointer(OutcomePointer::Tool { tool_id: "lock-unlock".into() });
    let limits = limits::recorded(COMMAND).ok_or_else(|| ToolError::processing("No size limits are recorded for converting to PDF."))?;
    let parsed = if extension == "odt" {
        odt::parse(&bytes, limits.max_expanded_bytes, limits.expansion_error(input), locked)?
    } else {
        match office::is_encrypted(&bytes, "docx") {
            Ok(false) => {}
            Ok(true) => return Err(locked),
            Err(_) => return Err(ToolError::invalid_input(docx::DAMAGED)),
        }
        docx::parse(&bytes, limits.max_expanded_bytes, limits.expansion_error(input))?
    };
    let geometry = page_geometry(parsed.page, &job.request.page_defaults);
    let mut theme = Theme::new(parsed.body_family.as_deref().unwrap_or(RTF_FAMILY), CODE_FAMILY, parsed.body_size, false);
    theme.families = parsed.families.clone();
    theme.leading = 1.2;

    let mut book = FontBook::with_embedded(&job.fonts, parsed.embedded);
    let mut flow = Flow::new(&mut book, theme, &parsed.pictures, geometry);
    flow.blocks(&parsed.blocks)?;
    flow.bands(&parsed.header, &parsed.footer, parsed.header_distance.unwrap_or(BAND_DISTANCE), parsed.footer_distance.unwrap_or(BAND_DISTANCE))?;
    let laid = flow.finish();
    let count = laid.pages.len();
    let layout = approximate_layout(&laid.floats);
    let assembly = Assembly { size: (geometry.width, geometry.height), pages: laid.pages, pictures: &parsed.pictures, bookmarks: &laid.bookmarks, anchors: &laid.anchors, title: parsed.title.as_deref(), author: parsed.author.as_deref() };
    let document = assemble(&book, assembly)?;
    let path = save(&job.request.output_location, input, document)?;

    let reports = book.reports();
    let mut detail = format!("{}.", page_count(count));
    if !reports.is_empty() && reports.iter().all(|report| report.source == FontSource::Embedded) {
        let fonts = reports.iter().map(|report| report.font_used.as_str()).collect::<std::collections::BTreeSet<_>>().len();
        detail.push_str(&if fonts == 1 { " 1 font, embedded from the file.".to_string() } else { format!(" {fonts} fonts, all embedded from the file.") });
    }
    let outcome = JobOutcome::succeeded(input.to_path_buf(), vec![path], detail).with_fonts(reports);
    Ok(with_notes(outcome, [omitted_content(parsed.omitted), layout]))
}

fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 { out.push(','); }
        out.push(digit);
    }
    out
}

fn table_file<S: SystemFontLookup>(job: &ConvertJob<S>, options: &TableOptions, input: &Path, extension: &str) -> Result<JobOutcome, ToolError> {
    let bytes = fs::read(input).map_err(|error| ToolError::processing(format!("Could not read the file: {error}")))?;
    let (text, encoding) = decode(&bytes, options.encoding)?;
    let (delimiter, delimiter_name) = options.delimiter.resolve(extension);
    let delimited::Table { block, rows } = delimited::parse(&text, delimiter, options.header_row)?;
    let (short, long) = job.request.page_defaults.page_size.points();
    let (width, height) = if options.orientation == Orientation::Landscape { (long, short) } else { (short, long) };
    let geometry = Geometry::uniform(width, height, job.request.page_defaults.margins.points());

    let mut book = FontBook::new(&job.fonts);
    let mut size = TABLE_SIZE;
    if options.fit == TableFit::Width {
        if let Block::Table { head, rows, .. } = &block {
            let mut measure = Flow::new(&mut book, Theme::new(TABLE_FAMILY, CODE_FAMILY, TABLE_SIZE, false), &[], geometry);
            size = (TABLE_SIZE * measure.table_scale(head, rows)? * 0.995).clamp(MIN_TABLE_SIZE, TABLE_SIZE);
        }
    }
    let mut flow = Flow::new(&mut book, Theme::new(TABLE_FAMILY, CODE_FAMILY, size, false), &[], geometry);
    flow.blocks(std::slice::from_ref(&block))?;
    let laid = flow.finish();
    let count = laid.pages.len();
    let document = assemble(&book, Assembly { size: (width, height), pages: laid.pages, pictures: &[], bookmarks: &[], anchors: &BTreeMap::new(), title: None, author: None })?;
    let path = save(&job.request.output_location, input, document)?;
    let rows = if rows == 1 { "1 row".to_string() } else { format!("{} rows", grouped(rows)) };
    let detail = format!("{}. {rows}, {delimiter_name}. Read as {encoding}.", page_count(count));
    Ok(JobOutcome::succeeded(input.to_path_buf(), vec![path], detail).with_fonts(book.reports()))
}

fn spreadsheet_file<S: SystemFontLookup>(job: &ConvertJob<S>, options: &SpreadsheetOptions, input: &Path, extension: &str) -> Result<JobOutcome, ToolError> {
    let bytes = fs::read(input).map_err(|error| ToolError::processing(format!("Could not read the file: {error}")))?;
    let locked = ToolError::invalid_input(LOCKED).with_pointer(OutcomePointer::Tool { tool_id: "lock-unlock".into() });
    let limits = limits::recorded(COMMAND).ok_or_else(|| ToolError::processing("No size limits are recorded for converting to PDF."))?;
    let workbook = if extension == "ods" {
        ods::parse(&bytes, limits.max_expanded_bytes, limits.expansion_error(input), locked)?
    } else {
        match office::is_encrypted(&bytes, "xlsx") {
            Ok(false) => {}
            Ok(true) => return Err(locked),
            Err(_) => return Err(ToolError::invalid_input(xlsx::DAMAGED)),
        }
        xlsx::parse(&bytes, limits.max_expanded_bytes, limits.expansion_error(input))?
    };
    let mut theme = Theme::new(workbook.body_family.as_deref().unwrap_or(TABLE_FAMILY), CODE_FAMILY, TABLE_SIZE, false);
    theme.families = workbook.families.clone();

    let mut book = FontBook::new(&job.fonts);
    let (mut pages, mut bookmarks, mut omitted) = (Vec::new(), Vec::new(), Omitted::default());
    let (mut printed, mut left_out) = (0, 0);
    for sheet in &workbook.sheets {
        if sheet.hidden && options.sheets == SheetChoice::Visible {
            if !sheet.is_empty() { left_out += 1; }
            continue;
        }
        let tables = sheet.tables(options.print_areas)?;
        if tables.is_empty() { continue; }
        omitted = Omitted { pictures: omitted.pictures + sheet.omitted.pictures, charts: omitted.charts + sheet.omitted.charts, diagrams: omitted.diagrams + sheet.omitted.diagrams, objects: omitted.objects + sheet.omitted.objects };
        let paper = sheet.paper.map(|(width, height)| PageSetup { width: Some(width), height: Some(height), ..PageSetup::default() }).unwrap_or_default();
        let portrait = page_geometry(paper, &job.request.page_defaults);
        let (width, height) = if sheet.landscape { (portrait.width.max(portrait.height), portrait.width.min(portrait.height)) } else { (portrait.width, portrait.height) };
        let geometry = Geometry::uniform(width, height, job.request.page_defaults.margins.points().min(width / 4.).min(height / 4.));

        let mut size = TABLE_SIZE;
        if options.fit_to_width || sheet.fit {
            let mut measure = Flow::new(&mut book, Theme { body_size: TABLE_SIZE, ..theme.clone() }, &[], geometry);
            for table in &tables {
                if let Block::Table { head, rows, .. } = table { size = size.min(TABLE_SIZE * measure.table_scale(head, rows)? * 0.995); }
            }
            size = size.clamp(MIN_TABLE_SIZE, TABLE_SIZE);
        }
        let blocks = tables.into_iter().enumerate().flat_map(|(index, table)| (index > 0).then_some(Block::PageBreak).into_iter().chain([table])).collect::<Vec<_>>();
        let mut flow = Flow::new(&mut book, Theme { body_size: size, ..theme.clone() }, &[], geometry);
        flow.blocks(&blocks)?;
        let laid = flow.finish();
        bookmarks.push(Bookmark { level: 1, title: sheet.name.clone(), page: pages.len(), y: height });
        pages.extend(laid.pages.into_iter().map(|page| PageOut { size: Some((width, height)), ..page }));
        printed += 1;
    }
    if pages.is_empty() {
        let hint = if left_out > 0 { " Its hidden sheets have content; choose All sheets to include them." } else { "" };
        return Err(ToolError::invalid_input(format!("This spreadsheet has nothing to print, so no PDF was made.{hint}")));
    }
    let count = pages.len();
    let first = pages[0].size.unwrap_or_default();
    let assembly = Assembly { size: first, pages, pictures: &[], bookmarks: &bookmarks, anchors: &BTreeMap::new(), title: workbook.title.as_deref(), author: workbook.author.as_deref() };
    let document = assemble(&book, assembly)?;
    let path = save(&job.request.output_location, input, document)?;

    let mut detail = format!("{}. {}.", page_count(count), if printed == 1 { "1 sheet".to_string() } else { format!("{printed} sheets") });
    match left_out {
        0 => {}
        1 => detail.push_str(" 1 hidden sheet left out."),
        count => detail.push_str(&format!(" {count} hidden sheets left out.")),
    }
    let outcome = JobOutcome::succeeded(input.to_path_buf(), vec![path], detail).with_fonts(book.reports());
    Ok(with_notes(outcome, [omitted_content(omitted)]))
}

fn presentation_file<S: SystemFontLookup>(job: &ConvertJob<S>, options: &PresentationOptions, input: &Path, extension: &str) -> Result<JobOutcome, ToolError> {
    let bytes = fs::read(input).map_err(|error| ToolError::processing(format!("Could not read the file: {error}")))?;
    let locked = ToolError::invalid_input(LOCKED).with_pointer(OutcomePointer::Tool { tool_id: "lock-unlock".into() });
    let limits = limits::recorded(COMMAND).ok_or_else(|| ToolError::processing("No size limits are recorded for converting to PDF."))?;
    let include = Include { hidden: options.hidden_slides, notes: options.speaker_notes };
    let deck = if extension == "odp" {
        odp::parse(&bytes, limits.max_expanded_bytes, limits.expansion_error(input), locked, include)?
    } else {
        match office::is_encrypted(&bytes, "pptx") {
            Ok(false) => {}
            Ok(true) => return Err(locked),
            Err(_) => return Err(ToolError::invalid_input(pptx::DAMAGED)),
        }
        pptx::parse(&bytes, limits.max_expanded_bytes, limits.expansion_error(input), include)?
    };
    if deck.slides.is_empty() {
        let hint = if deck.hidden_left_out > 0 { " Its slides are all hidden; choose Include hidden slides to include them." } else { "" };
        return Err(ToolError::invalid_input(format!("This presentation has nothing to print, so no PDF was made.{hint}")));
    }
    let mut theme = Theme::new(deck.body_family.as_deref().unwrap_or(SLIDE_FAMILY), CODE_FAMILY, SLIDE_SIZE, false);
    theme.families = deck.families.clone();
    theme.leading = 1.2;

    let mut book = FontBook::with_embedded(&job.fonts, deck.embedded);
    let (mut pages, mut bookmarks, mut omitted, mut approximate) = (Vec::new(), Vec::new(), Omitted::default(), Vec::new());
    let heading = Block::Formatted(Paragraph {
        spans: vec![Span { text: "Speaker notes".into(), style: Style { bold: true, ..Style::default() }, link: None }],
        align: Align::Left, indent: 0., right_indent: 0., before: 0., after: 6., size: NOTES_SIZE, leading: 1.2,
    });
    for slide in &deck.slides {
        let (width, height) = slide.size;
        let mut flow = Flow::new(&mut book, theme.clone(), &deck.pictures, Geometry::uniform(width, height, 0.));
        flow.blocks(&slide.shapes)?;
        let laid = flow.finish();
        bookmarks.push(Bookmark { level: 1, title: slide.title.clone().unwrap_or_else(|| format!("Slide {}", slide.number)), page: pages.len(), y: height });
        pages.extend(laid.pages.into_iter().take(1).map(|page| PageOut { size: Some((width, height)), ..page }));
        approximate.extend(std::iter::repeat_n(slide.number - 1, slide.approximate));
        omitted = Omitted { pictures: omitted.pictures + slide.omitted.pictures, charts: omitted.charts + slide.omitted.charts, diagrams: omitted.diagrams + slide.omitted.diagrams, objects: omitted.objects + slide.omitted.objects };
        if !slide.notes.is_empty() {
            let blocks = std::iter::once(heading.clone()).chain(slide.notes.iter().cloned()).collect::<Vec<_>>();
            let geometry = Geometry::uniform(width, height, NOTES_MARGIN.min(width / 4.).min(height / 4.));
            let mut flow = Flow::new(&mut book, Theme { body_size: NOTES_SIZE, ..theme.clone() }, &deck.pictures, geometry);
            flow.blocks(&blocks)?;
            pages.extend(flow.finish().pages.into_iter().map(|page| PageOut { size: Some((width, height)), ..page }));
        }
    }
    let count = pages.len();
    let first = pages[0].size.unwrap_or_default();
    let assembly = Assembly { size: first, pages, pictures: &deck.pictures, bookmarks: &bookmarks, anchors: &BTreeMap::new(), title: deck.title.as_deref(), author: deck.author.as_deref() };
    let document = assemble(&book, assembly)?;
    let path = save(&job.request.output_location, input, document)?;

    let printed = deck.slides.len();
    let mut detail = format!("{}. {}.", page_count(count), if printed == 1 { "1 slide".to_string() } else { format!("{printed} slides") });
    match deck.hidden_left_out {
        0 => {}
        1 => detail.push_str(" 1 hidden slide left out."),
        count => detail.push_str(&format!(" {count} hidden slides left out.")),
    }
    let layout = approximate_placement(&approximate, ("shape", "shapes"), ("slide", "slides"));
    let outcome = JobOutcome::succeeded(input.to_path_buf(), vec![path], detail).with_fonts(book.reports());
    Ok(with_notes(outcome, [omitted_content(omitted), layout]))
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
        let outcomes = convert_paths(&[&input], json!({ "ebooks": {} }));
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::Unavailable);
        assert_eq!(outcomes[0].error().unwrap().message, "Ebooks options come in a later update.");
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
        assert_eq!(outcome.detail, "2 pages.");
        assert_eq!(outcome.warnings, [OutcomeWarning { kind: WarningKind::OmittedContent, message: "Omitted content: 1 picture couldn't be shown and was left out.".into() }]);
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

    fn docx_fixture() -> Vec<u8> {
        let mut png = Vec::new();
        image::RgbaImage::from_fn(8, 6, |x, _| image::Rgba([30, 90, 200, if x < 4 { 255 } else { 128 }]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let paragraph = |text: &str| format!("<w:p><w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r></w:p>");
        let styled = |style: &str, text: &str| format!(r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#);
        let listed = |id: u8, text: &str| format!(r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="{id}"/></w:numPr></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#);
        let body = [
            r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:bookmarkStart w:id="0" w:name="summary"/><w:r><w:t>Quarterly summary</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p>"#.to_string(),
            r#"<w:p><w:r><w:t xml:space="preserve">Plain text with </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r><w:r><w:t xml:space="preserve"> and </w:t></w:r><w:r><w:rPr><w:color w:val="C80000"/></w:rPr><w:t>red</w:t></w:r><w:r><w:rPr><w:vanish/></w:rPr><w:t>hidden words</w:t></w:r><w:del w:id="1" w:author="A"><w:r><w:delText>removed text</w:delText></w:r></w:del><w:ins w:id="2" w:author="A"><w:r><w:t xml:space="preserve"> added text</w:t></w:r></w:ins></w:p>"#.to_string(),
            r#"<w:p><w:hyperlink r:id="rIdLink"><w:r><w:t>Example link</w:t></w:r></w:hyperlink><w:r><w:t xml:space="preserve"> and </w:t></w:r><w:hyperlink r:id="rIdFile"><w:r><w:t>local file</w:t></w:r></w:hyperlink><w:r><w:t xml:space="preserve"> and </w:t></w:r><w:hyperlink w:anchor="summary"><w:r><w:t>back to top</w:t></w:r></w:hyperlink></w:p>"#.to_string(),
            listed(1, "Bullet item"),
            listed(1, "Second bullet"),
            listed(2, "Step one"),
            listed(2, "Step two"),
            r#"<w:tbl><w:tr><w:trPr><w:tblHeader/></w:trPr><w:tc><w:p><w:r><w:t>Region</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Revenue</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>North</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>1,200</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#.to_string(),
            r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="762000" cy="571500"/><a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed="rIdImage"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#.to_string(),
            r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="762000" cy="571500"/><a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:link="rIdRemote"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#.to_string(),
            paragraph("After the table"),
            r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#.to_string(),
            styled("Heading2", "Second page"),
            paragraph("Closing words"),
            r#"<w:sectPr><w:headerReference w:type="default" r:id="rIdHeader"/><w:footerReference w:type="default" r:id="rIdFooter"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>"#.to_string(),
        ].concat();
        let styles = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi" w:hAnsiTheme="minorHAnsi"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:rFonts w:asciiTheme="majorHAnsi" w:hAnsiTheme="majorHAnsi"/><w:b/><w:sz w:val="32"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="26"/></w:rPr></w:style></w:styles>"#;
        let theme = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:fontScheme><a:majorFont><a:latin typeface="Cambria"/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#;
        let numbering = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#;
        let header = format!(r#"<w:hdr {}><w:p><w:r><w:t>Header text</w:t></w:r></w:p></w:hdr>"#, docx::fixtures::W);
        let footer = format!(r#"<w:ftr {}><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t xml:space="preserve">Page </w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>9</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve"> of </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> NUMPAGES </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>9</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p></w:ftr>"#, docx::fixtures::W);
        let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Quarterly report</dc:title><dc:creator>Dana Whitfield</dc:creator></cp:coreProperties>"#;
        docx::fixtures::docx(&body, &[
            ("rIdStyles", "styles", "styles.xml", false),
            ("rIdTheme", "theme", "theme/theme1.xml", false),
            ("rIdNumbering", "numbering", "numbering.xml", false),
            ("rIdHeader", "header", "header1.xml", false),
            ("rIdFooter", "footer", "footer1.xml", false),
            ("rIdImage", "image", "media/image1.png", false),
            ("rIdRemote", "image", "http://127.0.0.1:9/remote.png", true),
            ("rIdLink", "hyperlink", "https://example.com/docx", true),
            ("rIdFile", "hyperlink", "file:///etc/hosts", true),
        ], &[
            ("word/styles.xml", styles.as_bytes()),
            ("word/theme/theme1.xml", theme.as_bytes()),
            ("word/numbering.xml", numbering.as_bytes()),
            ("word/header1.xml", header.as_bytes()),
            ("word/footer1.xml", footer.as_bytes()),
            ("word/media/image1.png", &png),
            ("docProps/core.xml", core.as_bytes()),
        ])
    }

    #[test]
    fn word_keeps_headings_lists_tables_pictures_bands_links_and_page_setup() {
        let root = sandbox("docx");
        let input = root.join("report.docx");
        let source = docx_fixture();
        fs::write(&input, &source).unwrap();

        let outcomes = convert_paths(&[&input], json!({ "pageDefaults": { "pageSize": "a4", "margins": "wide" } }));
        let outcome = &outcomes[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert_eq!(outcome.detail, "2 pages.");
        assert_eq!(outcome.warnings, [OutcomeWarning { kind: WarningKind::OmittedContent, message: "Omitted content: 1 picture couldn't be shown and was left out.".into() }]);
        for family in ["Calibri", "Cambria"] {
            assert!(outcome.fonts.iter().any(|font| font.requested_family == family), "{family} missing from {:?}", outcome.fonts);
        }

        let document = Document::load(root.join("report.pdf")).unwrap();
        assert_eq!(document.get_pages().len(), 2);
        assert_eq!(media_box(&document, 1), [0., 0., 612., 792.], "the document's own page setup wins");
        let first = document.extract_text(&[1]).unwrap();
        assert!(in_order(&first, &["Quarterly summary", "Plain text with", "bold", "red", "added text", "Example link", "local file", "back to top", "\u{2022}", "Bullet item", "Second bullet", "1.", "Step one", "2.", "Step two", "Region", "Revenue", "North", "1,200", "After the table", "Header text", "Page 1 of 2"]), "{first}");
        for absent in ["hidden words", "removed text", "Second page", "Closing words"] {
            assert!(!first.contains(absent), "{absent:?} should not be on page 1: {first}");
        }
        let second = document.extract_text(&[2]).unwrap();
        assert!(in_order(&second, &["Second page", "Closing words", "Header text", "Page 2 of 2"]), "{second}");

        let content = String::from_utf8_lossy(&document.get_page_content(document.get_pages()[&1])).into_owned();
        assert!(content.contains("60.00 0 0 45.00 72.00"), "the picture keeps its stored size and the document's left margin");
        assert!(content.contains("0.78431374 0 0 rg"), "the red colour is applied");
        assert!(font_sizes(&document, 1).contains(&16.), "Heading 1 keeps its 16 pt style size");

        let links = links(&document);
        let uris = links.iter().filter_map(|link| link.get(b"A").and_then(Object::as_dict).and_then(|action| action.get(b"URI")).and_then(Object::as_str).ok().map(<[u8]>::to_vec)).collect::<Vec<_>>();
        assert_eq!(uris, [b"https://example.com/docx".to_vec()], "only the web link is linked; file links are left as text");
        let internal = links.iter().find_map(|link| link.get(b"Dest").ok().cloned()).expect("the bookmark link points inside the document");
        assert_eq!(page_of(&document, &internal), 1);
        assert_eq!(outline_titles(&document), ["Quarterly summary", "Second page"]);
        assert_eq!(info(&document, b"Title"), b"Quarterly report");
        assert_eq!(info(&document, b"Author"), b"Dana Whitfield");

        assert!(succeeded(&convert_paths(&[&input], json!({}))[0]));
        assert_eq!(fs::read(&input).unwrap(), source);
        assert_eq!(names(&root), ["report-1.pdf", "report.docx", "report.pdf"]);
    }

    #[test]
    fn word_uses_fonts_embedded_in_the_document_first() {
        let root = sandbox("docx_fonts");
        let input = root.join("brand.docx");
        let key = "{6C0C5D0E-1B2A-4F3E-9D8C-7B6A59483726}";
        let mut font = fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/fonts/Pacifico-Regular.ttf")).unwrap();
        docx::deobfuscate(&mut font, key);
        let body = r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="Brand Script" w:hAnsi="Brand Script"/></w:rPr><w:t>Hello from the brand</w:t></w:r></w:p>"#;
        let table = format!(r#"<w:fonts {}><w:font w:name="Brand Script"><w:embedRegular r:id="rIdFont" w:fontKey="{key}"/></w:font></w:fonts>"#, docx::fixtures::W);
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdFont" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/font" Target="fonts/font1.odttf"/></Relationships>"#;
        let styles = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Brand Script" w:hAnsi="Brand Script"/></w:rPr></w:rPrDefault></w:docDefaults></w:styles>"#;
        fs::write(&input, docx::fixtures::docx(body, &[("rIdFonts", "fontTable", "fontTable.xml", false), ("rIdStyles", "styles", "styles.xml", false)], &[
            ("word/fontTable.xml", table.as_bytes()),
            ("word/_rels/fontTable.xml.rels", rels.as_bytes()),
            ("word/fonts/font1.odttf", &font),
            ("word/styles.xml", styles.as_bytes()),
        ])).unwrap();

        let outcome = &convert_paths(&[&input], json!({}))[0];
        assert!(succeeded(outcome), "{outcome:?}");
        let report = outcome.fonts.iter().find(|font| font.requested_family == "Brand Script").expect("the embedded family is reported");
        assert_eq!(report.source, FontSource::Embedded, "{:?}", outcome.fonts);
        assert_eq!(outcome.detail, "1 page. 1 font, embedded from the file.");
        let text = Document::load(root.join("brand.pdf")).unwrap().extract_text(&[1]).unwrap();
        assert!(text.contains("ello from the brand"), "{text:?}");
    }

    fn png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut png = Vec::new();
        image::RgbImage::from_pixel(width, height, image::Rgb(rgb)).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        png
    }

    fn page_content(document: &Document, page: u32) -> String {
        String::from_utf8_lossy(&document.get_page_content(document.get_pages()[&page])).into_owned()
    }

    fn page_images(document: &Document, page: u32) -> usize {
        page_content(document, page).matches(" Do Q").count()
    }

    #[test]
    fn word_draws_charts_from_their_stored_image_and_names_the_ones_left_out() {
        let root = sandbox("docx_charts");
        let input = root.join("charts.docx");
        let chart = r#"<w:drawing><wp:inline><wp:extent cx="2540000" cy="1270000"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rIdChart"/></a:graphicData></a:graphic></wp:inline></w:drawing>"#;
        let stored = r#"<w:drawing><wp:inline><wp:extent cx="2540000" cy="1270000"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:embed="rIdStored"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing>"#;
        let body = format!(
            r#"<w:p><w:r><w:t>Revenue by region</w:t></w:r></w:p><w:p><w:r><mc:AlternateContent><mc:Choice Requires="c14">{chart}</mc:Choice><mc:Fallback>{stored}</mc:Fallback></mc:AlternateContent></w:r></w:p><w:p><w:r><w:t>Costs by region</w:t></w:r></w:p><w:p><w:r>{chart}</w:r></w:p><w:p><w:r><w:object><v:shape style="width:120pt;height:60pt"/><o:OLEObject ProgID="Excel.Sheet.12" r:id="rIdOle"/></w:object></w:r></w:p>"#
        );
        let chart_part = br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#;
        fs::write(&input, docx::fixtures::docx(&body, &[
            ("rIdChart", "chart", "charts/chart1.xml", false),
            ("rIdStored", "image", "media/chart1.png", false),
            ("rIdOle", "oleObject", "embeddings/sheet.xlsx", false),
        ], &[("word/charts/chart1.xml", chart_part), ("word/media/chart1.png", &png(20, 10, [10, 120, 60])), ("word/embeddings/sheet.xlsx", b"never opened")])).unwrap();

        let outcome = &convert_paths(&[&input], json!({}))[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert_eq!(outcome.detail, "1 page.");
        assert_eq!(outcome.warnings, [OutcomeWarning {
            kind: WarningKind::OmittedContent,
            message: "Omitted content: 1 chart and 1 embedded object have no stored image, so they were left out.".into(),
        }]);
        let document = Document::load(root.join("charts.pdf")).unwrap();
        assert_eq!(page_images(&document, 1), 1, "only the chart with a stored image is drawn");
        assert!(page_content(&document, 1).contains("200.00 0 0 100.00"), "the stored image keeps the chart's size");
        let text = document.extract_text(&[1]).unwrap();
        assert!(in_order(&text, &["Revenue by region", "Costs by region"]), "{text}");
    }

    #[test]
    fn omitted_content_and_layout_notes_count_what_they_name() {
        let message = |omitted: Omitted| omitted_content(omitted).map(|warning| warning.message);
        assert_eq!(message(Omitted::default()), None);
        assert_eq!(message(Omitted { charts: 1, ..Omitted::default() }).unwrap(), "Omitted content: 1 chart has no stored image, so it was left out.");
        assert_eq!(
            message(Omitted { pictures: 2, charts: 2, diagrams: 1, objects: 3 }).unwrap(),
            "Omitted content: 2 charts, 1 diagram and 3 embedded objects have no stored image, so they were left out. 2 pictures couldn't be shown and were left out.",
        );
        let note = |floats: &[usize]| approximate_layout(floats).map(|warning| (warning.kind, warning.message));
        assert_eq!(note(&[]), None);
        assert_eq!(note(&[0]).unwrap(), (WarningKind::ApproximateLayout, "Approximate layout: 1 floating object on page 1 was placed as close as possible.".into()));
        assert_eq!(note(&[3, 0, 1, 2, 5]).unwrap().1, "Approximate layout: 5 floating objects on pages 1\u{2013}4 and 6 were placed as close as possible.");
    }

    #[test]
    fn word_renders_tracked_changes_as_accepted_and_leaves_comments_out() {
        let root = sandbox("docx_tracked");
        let input = root.join("review.docx");
        let body = concat!(
            r#"<w:p><w:r><w:t xml:space="preserve">Kept </w:t></w:r><w:ins w:id="1" w:author="A"><w:r><w:t>inserted words</w:t></w:r></w:ins><w:del w:id="2" w:author="A"><w:r><w:delText>deleted words</w:delText></w:r></w:del>"#,
            r#"<w:commentRangeStart w:id="7"/><w:r><w:t xml:space="preserve"> commented</w:t></w:r><w:commentRangeEnd w:id="7"/><w:r><w:commentReference w:id="7"/></w:r></w:p>"#,
            r#"<w:p><w:moveFrom w:id="3" w:author="A"><w:r><w:t>moved away</w:t></w:r></w:moveFrom><w:r><w:rPr><w:b/><w:rPrChange w:id="4" w:author="A"><w:rPr><w:vanish/></w:rPr></w:rPrChange></w:rPr><w:t>Reformatted</w:t></w:r></w:p>"#,
            r#"<w:p><w:pPr><w:rPr><w:del w:id="5" w:author="A"/></w:rPr></w:pPr><w:r><w:t xml:space="preserve">Joined </w:t></w:r></w:p><w:p><w:r><w:t>sentence</w:t></w:r><w:moveTo w:id="6" w:author="A"><w:r><w:t xml:space="preserve"> moved here</w:t></w:r></w:moveTo></w:p>"#,
            r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Kept row</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:trPr><w:del w:id="8" w:author="A"/></w:trPr><w:tc><w:p><w:del w:id="9" w:author="A"><w:r><w:delText>Deleted row</w:delText></w:r></w:del></w:p></w:tc></w:tr><w:tr><w:trPr><w:ins w:id="10" w:author="A"/></w:trPr><w:tc><w:p><w:ins w:id="11" w:author="A"><w:r><w:t>Inserted row</w:t></w:r></w:ins></w:p></w:tc></w:tr></w:tbl>"#,
        );
        let comments = format!(r#"<w:comments {}><w:comment w:id="7" w:author="Reviewer"><w:p><w:r><w:annotationRef/></w:r><w:r><w:t>Secret reviewer remark</w:t></w:r></w:p></w:comment></w:comments>"#, docx::fixtures::W);
        fs::write(&input, docx::fixtures::docx(body, &[("rIdComments", "comments", "comments.xml", false)], &[("word/comments.xml", comments.as_bytes())])).unwrap();

        let outcome = &convert_paths(&[&input], json!({}))[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert!(outcome.warnings.is_empty(), "comments are left out without a warning: {:?}", outcome.warnings);
        let text = Document::load(root.join("review.pdf")).unwrap().extract_text(&[1]).unwrap();
        assert!(in_order(&text, &["Kept", "inserted words", "commented", "Reformatted", "Joined sentence moved here", "Kept row", "Inserted row"]), "{text}");
        for absent in ["deleted words", "moved away", "Deleted row", "Secret reviewer remark", "Reviewer"] {
            assert!(!text.contains(absent), "{absent:?} should be left out: {text}");
        }
    }

    #[test]
    fn word_places_floating_objects_at_their_anchors_with_an_approximate_layout_note() {
        let root = sandbox("docx_floating");
        let input = root.join("brochure.docx");
        let anchor = |horizontal: &str, vertical: &str, size: (u32, u32), wrap: &str, graphic: &str| format!(
            r#"<w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="1" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="{}"><wp:posOffset>{}</wp:posOffset></wp:positionH><wp:positionV relativeFrom="{}"><wp:posOffset>{}</wp:posOffset></wp:positionV><wp:extent cx="{}" cy="{}"/>{wrap}<a:graphic>{graphic}</a:graphic></wp:anchor></w:drawing>"#,
            horizontal.split(':').next().unwrap(), horizontal.split(':').nth(1).unwrap(), vertical.split(':').next().unwrap(), vertical.split(':').nth(1).unwrap(), size.0, size.1,
        );
        let text_box = anchor("page:3810000", "page:1270000", (1905000, 635000), "<wp:wrapSquare wrapText=\"bothSides\"/>",
            r#"<a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="FFF2CC"/></a:solidFill><a:ln><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>Boxed callout</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData>"#);
        let shape = anchor("margin:0", "paragraph:0", (1270000, 254000), "<wp:wrapNone/>",
            r#"<a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="ellipse"/><a:solidFill><a:srgbClr val="2E75B6"/></a:solidFill></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData>"#);
        let picture = anchor("column:0", "paragraph:0", (1270000, 1270000), "<wp:wrapTopAndBottom/>",
            r#"<a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:embed="rIdPhoto"/></pic:blipFill></pic:pic></a:graphicData>"#);
        let body = format!(concat!(
            r#"<w:p><w:r>{}</w:r><w:r><w:t>Front page text</w:t></w:r></w:p>"#,
            r#"<w:p><w:r>{}</w:r><w:r><w:t>Banner paragraph</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#,
            r#"<w:p><w:r>{}</w:r><w:r><w:t>Photo caption</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t>Text after the photo</w:t></w:r></w:p>"#,
        ), text_box, shape, picture);
        fs::write(&input, docx::fixtures::docx(&body, &[("rIdPhoto", "image", "media/photo.png", false)], &[("word/media/photo.png", &png(10, 10, [200, 40, 40]))])).unwrap();

        let outcome = &convert_paths(&[&input], json!({ "pageDefaults": { "pageSize": "letter", "margins": "normal" } }))[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert_eq!(outcome.detail, "2 pages.");
        assert_eq!(outcome.warnings, [OutcomeWarning {
            kind: WarningKind::ApproximateLayout,
            message: "Approximate layout: 3 floating objects on pages 1 and 2 were placed as close as possible.".into(),
        }]);

        let document = Document::load(root.join("brochure.pdf")).unwrap();
        let first = page_content(&document, 1);
        assert!(first.contains("1 0.9490196 0.8 rg 300.00 642.00 150.00 50.00 re f"), "the text box sits at its page offset: {first}");
        assert!(first.contains("0 0 0 RG 0.75 w 300.00 642.00 150.00 50.00 re S"), "the text box keeps its outline");
        assert!(first.contains("0.18039216 0.45882353 0.7137255 rg"), "the anchored shape is filled");
        let text = document.extract_text(&[1]).unwrap();
        assert!(in_order(&text, &["Boxed callout", "Front page text", "Banner paragraph"]), "{text}");
        assert_eq!(page_images(&document, 1), 0);
        assert_eq!(page_images(&document, 2), 1, "the wrapped picture stays on its anchor's page");
        let second = page_content(&document, 2);
        let picture_bottom = second.split("100.00 0 0 100.00 ").nth(1).and_then(|rest| rest.split_whitespace().nth(1)).and_then(|value| value.parse::<f32>().ok()).expect(&second);
        let second_text = document.extract_text(&[2]).unwrap();
        assert!(in_order(&second_text, &["Photo caption", "Text after the photo"]), "{second_text}");
        assert_eq!(picture_bottom, 792. - 72. - 100., "the picture sits at the top of its anchor paragraph on page 2");
        let first_line = second.split(" Tm ").next().and_then(|head| head.split_whitespace().last()).and_then(|value| value.parse::<f32>().ok()).expect(&second);
        assert!(first_line < picture_bottom, "wrapped text flows below the picture, not under it: {first_line}");
    }

    #[test]
    fn word_reads_legacy_vml_pictures_inline_and_text_boxes_as_floats() {
        let root = sandbox("docx_vml");
        let input = root.join("legacy.docx");
        let body = concat!(
            r#"<w:p><w:r><w:pict><v:shape style="width:60pt;height:30pt"><v:imagedata r:id="rIdLogo"/></v:shape></w:pict></w:r><w:r><w:t>After the logo</w:t></w:r></w:p>"#,
            r##"<w:p><w:r><w:pict><v:shape style="position:absolute;margin-left:200pt;margin-top:150pt;width:120pt;height:40pt;mso-position-horizontal-relative:page;mso-position-vertical-relative:page" fillcolor="#ffcc00"><v:textbox><w:txbxContent><w:p><w:r><w:t>Legacy box</w:t></w:r></w:p></w:txbxContent></v:textbox><w10:wrap type="none"/></v:shape></w:pict></w:r></w:p>"##,
        );
        fs::write(&input, docx::fixtures::docx(body, &[("rIdLogo", "image", "media/logo.png", false)], &[("word/media/logo.png", &png(6, 3, [0, 0, 255]))])).unwrap();

        let outcome = &convert_paths(&[&input], json!({ "pageDefaults": { "pageSize": "letter", "margins": "normal" } }))[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert_eq!(outcome.warnings, [OutcomeWarning {
            kind: WarningKind::ApproximateLayout,
            message: "Approximate layout: 1 floating object on page 1 was placed as close as possible.".into(),
        }]);
        let document = Document::load(root.join("legacy.pdf")).unwrap();
        let content = page_content(&document, 1);
        assert!(content.contains("60.00 0 0 30.00"), "the inline VML picture keeps its size: {content}");
        assert!(content.contains("1 0.8 0 rg 200.00 602.00 120.00 40.00 re f"), "the VML text box sits at its page offset: {content}");
        let text = document.extract_text(&[1]).unwrap();
        assert!(in_order(&text, &["After the logo"]) && text.contains("Legacy box"), "{text}");
    }

    #[test]
    fn word_without_floating_objects_carries_no_layout_note() {
        let root = sandbox("docx_flowing");
        let input = root.join("plain.docx");
        fs::write(&input, docx::fixtures::docx("<w:p><w:r><w:t>Only flowing text</w:t></w:r></w:p>", &[], &[])).unwrap();
        let outcome = &convert_paths(&[&input], json!({}))[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    }

    #[test]
    fn locked_damaged_and_oversized_word_documents_fail_alone() {
        let root = sandbox("docx_batch");
        let plain = root.join("plain.docx");
        fs::write(&plain, crate::kit::office::test_minimal_ooxml_package("docx")).unwrap();
        fs::create_dir_all(root.join("locked")).unwrap();
        let protected = crate::kit::office::OfficeProcessor::protect(plain.clone(), crate::kit::TEST_PASSWORD, &OutputLocation::CustomFolder(root.join("locked")));
        let locked = protected.output_paths().first().cloned().unwrap_or_else(|| panic!("{protected:?}"));
        let damaged = root.join("damaged.docx");
        fs::write(&damaged, b"PK\x03\x04 not really a package").unwrap();
        let bomb = root.join("bomb.docx");
        fs::write(&bomb, crate::kit::common::limits::fixtures::zip_with_deflated("word/document.xml", &crate::kit::common::limits::fixtures::zlib_of_zeros(500_000_001), 500_000_001)).unwrap();
        let good = root.join("letter.docx");
        fs::write(&good, docx::fixtures::docx("<w:p><w:r><w:t>Dear reader,</w:t></w:r></w:p>", &[], &[])).unwrap();
        let originals = [&locked, &damaged, &bomb, &good].map(|path| fs::read(path).unwrap());

        let outcomes = convert_paths(&[&locked, &good, &damaged, &bomb], json!({ "pageDefaults": { "pageSize": "letter", "margins": "narrow" } }));
        let error = outcomes[0].error().unwrap();
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        assert_eq!(error.message, "It's password-protected. Unlock it in Lock & Unlock, then convert the unlocked copy.");
        assert_eq!(error.pointer, Some(OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
        assert_eq!(outcomes[2].error().unwrap().message, docx::DAMAGED);
        let limited = outcomes[3].error().unwrap();
        assert_eq!(limited.kind, ErrorKind::LimitExceeded { limit: crate::kit::contracts::LimitKind::Decompression }, "{limited:?}");
        assert!(limited.message.contains("500 MB"), "{}", limited.message);
        assert!(succeeded(&outcomes[1]), "{outcomes:?}");
        assert_eq!(outcomes[1].detail, "1 page.");
        assert_eq!(media_box(&Document::load(root.join("letter.pdf")).unwrap(), 1), [0., 0., 612., 792.], "without page setup the page defaults apply");
        assert_eq!([&locked, &damaged, &bomb, &good].map(|path| fs::read(path).unwrap()), originals);
        assert!(!root.join("damaged.pdf").exists() && !root.join("bomb.pdf").exists());
    }

    fn odt_styles(automatic: &str, styles: &str, masters: &str) -> String {
        format!(r#"<office:document-styles {}><office:font-face-decls><style:font-face style:name="Calibri" svg:font-family="Calibri"/><style:font-face style:name="Cambria" svg:font-family="Cambria"/></office:font-face-decls><office:styles><style:default-style style:family="paragraph"><style:text-properties style:font-name="Calibri" fo:font-size="11pt"/></style:default-style><style:style style:name="Standard" style:family="paragraph"/>{styles}</office:styles><office:automatic-styles>{automatic}</office:automatic-styles><office:master-styles>{masters}</office:master-styles></office:document-styles>"#, odt::fixtures::NS)
    }

    fn odt_fixture() -> Vec<u8> {
        let mut png = Vec::new();
        image::RgbaImage::from_fn(8, 6, |x, _| image::Rgba([30, 90, 200, if x < 4 { 255 } else { 128 }]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let automatic = concat!(
            r#"<style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>"#,
            r##"<style:style style:name="T2" style:family="text"><style:text-properties fo:color="#c80000"/></style:style>"##,
            r#"<style:style style:name="T3" style:family="text"><style:text-properties text:display="none"/></style:style>"#,
            r#"<style:style style:name="P1" style:family="paragraph" style:parent-style-name="Heading_20_2"><style:paragraph-properties fo:break-before="page"/></style:style>"#,
            r#"<text:list-style style:name="L1"><text:list-level-style-bullet text:level="1" text:bullet-char="•"/></text:list-style>"#,
            r#"<text:list-style style:name="L2"><text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix="."/></text:list-style>"#,
        );
        let body = concat!(
            r#"<text:tracked-changes><text:changed-region text:id="ct1"><text:deletion><office:change-info><dc:creator>A</dc:creator></office:change-info><text:p>removed text</text:p></text:deletion></text:changed-region><text:changed-region text:id="ct2"><text:insertion><office:change-info><dc:creator>A</dc:creator></office:change-info></text:insertion></text:changed-region></text:tracked-changes>"#,
            r#"<text:h text:style-name="Heading_20_1" text:outline-level="1"><text:bookmark text:name="summary"/>Quarterly summary</text:h>"#,
            r#"<text:p>Plain text with <text:span text:style-name="T1">bold</text:span> and <text:span text:style-name="T2">red</text:span><text:span text:style-name="T3">hidden words</text:span><text:change text:change-id="ct1"/><text:change-start text:change-id="ct2"/> added text<text:change-end text:change-id="ct2"/></text:p>"#,
            r##"<text:p><text:a xlink:type="simple" xlink:href="https://example.com/odt">Example link</text:a> and <text:a xlink:type="simple" xlink:href="file:///etc/hosts">local file</text:a> and <text:a xlink:type="simple" xlink:href="#summary">back to top</text:a></text:p>"##,
            r#"<text:list text:style-name="L1"><text:list-item><text:p>Bullet item</text:p></text:list-item><text:list-item><text:p>Second bullet</text:p></text:list-item></text:list>"#,
            r#"<text:list text:style-name="L2"><text:list-item><text:p>Step one</text:p></text:list-item><text:list-item><text:p>Step two</text:p></text:list-item></text:list>"#,
            r#"<table:table table:name="Sales"><table:table-column table:number-columns-repeated="2"/><table:table-header-rows><table:table-row><table:table-cell><text:p>Region</text:p></table:table-cell><table:table-cell><text:p>Revenue</text:p></table:table-cell></table:table-row></table:table-header-rows><table:table-row><table:table-cell><text:p>North</text:p></table:table-cell><table:table-cell><text:p>1,200</text:p></table:table-cell></table:table-row></table:table>"#,
            r#"<text:p><draw:frame draw:name="Image1" text:anchor-type="as-char" svg:width="60pt" svg:height="45pt"><draw:image xlink:href="Pictures/image1.png" xlink:type="simple"/></draw:frame></text:p>"#,
            r#"<text:p><draw:frame draw:name="Image2" text:anchor-type="as-char" svg:width="60pt" svg:height="45pt"><draw:image xlink:href="http://127.0.0.1:9/remote.png" xlink:type="simple"/></draw:frame></text:p>"#,
            r#"<text:p>After the table</text:p>"#,
            r#"<text:h text:style-name="P1" text:outline-level="2">Second page</text:h>"#,
            r#"<text:p>Closing words</text:p>"#,
        );
        let styles = odt_styles(
            r#"<style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" fo:margin-top="0.5in" fo:margin-bottom="0.5in" fo:margin-left="1in" fo:margin-right="1in"/><style:header-style><style:header-footer-properties fo:min-height="0.4in" fo:margin-bottom="0.1in"/></style:header-style><style:footer-style><style:header-footer-properties fo:min-height="0.4in" fo:margin-top="0.1in"/></style:footer-style></style:page-layout><style:style style:name="MP1" style:family="paragraph"><style:paragraph-properties fo:text-align="center"/></style:style>"#,
            r#"<style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:parent-style-name="Standard" style:default-outline-level="1"><style:text-properties style:font-name="Cambria" fo:font-size="16pt" fo:font-weight="bold"/></style:style><style:style style:name="Heading_20_2" style:display-name="Heading 2" style:family="paragraph" style:parent-style-name="Standard" style:default-outline-level="2"><style:text-properties fo:font-size="13pt" fo:font-weight="bold"/></style:style>"#,
            r#"<style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Header text</text:p></style:header><style:footer><text:p text:style-name="MP1">Page <text:page-number text:select-page="current">9</text:page-number> of <text:page-count>9</text:page-count></text:p></style:footer></style:master-page>"#,
        );
        let meta = format!(r#"<office:document-meta {}><office:meta><dc:title>Quarterly report</dc:title><meta:initial-creator>Dana Whitfield</meta:initial-creator><dc:creator>Someone Else</dc:creator></office:meta></office:document-meta>"#, odt::fixtures::NS);
        odt::fixtures::odt(automatic, body, &[("styles.xml", styles.as_bytes()), ("meta.xml", meta.as_bytes()), ("Pictures/image1.png", &png)], &[("Pictures/image1.png", "image/png")])
    }

    #[test]
    fn odt_matches_word_for_headings_lists_tables_pictures_bands_links_and_page_setup() {
        let root = sandbox("odt");
        let input = root.join("report.odt");
        let source = odt_fixture();
        fs::write(&input, &source).unwrap();
        let word = root.join("word.docx");
        fs::write(&word, docx_fixture()).unwrap();

        let outcomes = convert_paths(&[&input, &word], json!({ "pageDefaults": { "pageSize": "a4", "margins": "wide" } }));
        let outcome = &outcomes[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert_eq!(outcome.detail, "2 pages.");
        assert_eq!(outcome.warnings, outcomes[1].warnings, "the same content gives the Word warnings");
        assert_eq!(outcome.warnings, [OutcomeWarning { kind: WarningKind::OmittedContent, message: "Omitted content: 1 picture couldn't be shown and was left out.".into() }]);
        for family in ["Calibri", "Cambria"] {
            assert!(outcome.fonts.iter().any(|font| font.requested_family == family), "{family} missing from {:?}", outcome.fonts);
        }

        let document = Document::load(root.join("report.pdf")).unwrap();
        let docx = Document::load(root.join("word.pdf")).unwrap();
        assert_eq!(document.get_pages().len(), docx.get_pages().len());
        assert_eq!(media_box(&document, 1), [0., 0., 612., 792.], "the document's own page setup wins");
        let first = document.extract_text(&[1]).unwrap();
        assert!(in_order(&first, &["Quarterly summary", "Plain text with", "bold", "red", "added text", "Example link", "local file", "back to top", "\u{2022}", "Bullet item", "Second bullet", "1.", "Step one", "2.", "Step two", "Region", "Revenue", "North", "1,200", "After the table", "Header text", "Page 1 of 2"]), "{first}");
        for absent in ["hidden words", "removed text", "Second page", "Closing words"] {
            assert!(!first.contains(absent), "{absent:?} should not be on page 1: {first}");
        }
        let second = document.extract_text(&[2]).unwrap();
        assert!(in_order(&second, &["Second page", "Closing words", "Header text", "Page 2 of 2"]), "{second}");
        for page in [1, 2] {
            assert_eq!(document.extract_text(&[page]).unwrap().split_whitespace().collect::<Vec<_>>(), docx.extract_text(&[page]).unwrap().split_whitespace().collect::<Vec<_>>(), "page {page} recovers the Word text");
        }

        let content = page_content(&document, 1);
        assert!(content.contains("60.00 0 0 45.00 72.00"), "the picture keeps its stored size and the document's left margin");
        assert!(content.contains("0.78431374 0 0 rg"), "the red colour is applied");
        assert!(font_sizes(&document, 1).contains(&16.), "Heading 1 keeps its 16 pt style size");

        let links = links(&document);
        let uris = links.iter().filter_map(|link| link.get(b"A").and_then(Object::as_dict).and_then(|action| action.get(b"URI")).and_then(Object::as_str).ok().map(<[u8]>::to_vec)).collect::<Vec<_>>();
        assert_eq!(uris, [b"https://example.com/odt".to_vec()], "only the web link is linked; file links are left as text");
        let internal = links.iter().find_map(|link| link.get(b"Dest").ok().cloned()).expect("the bookmark link points inside the document");
        assert_eq!(page_of(&document, &internal), 1);
        assert_eq!(outline_titles(&document), outline_titles(&docx));
        assert_eq!(outline_titles(&document), ["Quarterly summary", "Second page"]);
        assert_eq!(info(&document, b"Title"), b"Quarterly report");
        assert_eq!(info(&document, b"Author"), b"Dana Whitfield");

        assert!(succeeded(&convert_paths(&[&input], json!({}))[0]));
        assert_eq!(fs::read(&input).unwrap(), source);
        assert_eq!(names(&root), ["report-1.pdf", "report.odt", "report.pdf", "word.docx", "word.pdf"]);
    }

    #[test]
    fn odt_draws_charts_from_their_stored_image_and_names_the_ones_left_out() {
        let root = sandbox("odt_charts");
        let input = root.join("charts.odt");
        let body = concat!(
            r#"<text:p>Revenue by region</text:p><text:p><draw:frame text:anchor-type="as-char" svg:width="200pt" svg:height="100pt"><draw:object xlink:href="./Object 1" xlink:type="simple"/><draw:image xlink:href="./ObjectReplacements/Object 1" xlink:type="simple"/></draw:frame></text:p>"#,
            r#"<text:p>Costs by region</text:p><text:p><draw:frame text:anchor-type="as-char" svg:width="200pt" svg:height="100pt"><draw:object xlink:href="./Object 2" xlink:type="simple"/></draw:frame></text:p>"#,
            r#"<text:p><draw:frame text:anchor-type="as-char" svg:width="120pt" svg:height="60pt"><draw:object-ole xlink:href="./Object 3" xlink:type="simple"/></draw:frame></text:p>"#,
        );
        let chart = br#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"><office:body><office:chart/></office:body></office:document-content>"#;
        fs::write(&input, odt::fixtures::odt("", body, &[
            ("Object 1/content.xml", chart), ("ObjectReplacements/Object 1", &png(20, 10, [10, 120, 60])),
            ("Object 2/content.xml", chart), ("Object 3", b"never opened"),
        ], &[("Object 1/", "application/vnd.oasis.opendocument.chart"), ("Object 2/", "application/vnd.oasis.opendocument.chart"), ("Object 3", "application/vnd.sun.star.oleobject")])).unwrap();

        let outcome = &convert_paths(&[&input], json!({}))[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert_eq!(outcome.detail, "1 page.");
        assert_eq!(outcome.warnings, [OutcomeWarning {
            kind: WarningKind::OmittedContent,
            message: "Omitted content: 1 chart and 1 embedded object have no stored image, so they were left out.".into(),
        }]);
        let document = Document::load(root.join("charts.pdf")).unwrap();
        assert_eq!(page_images(&document, 1), 1, "only the chart with a stored image is drawn");
        assert!(page_content(&document, 1).contains("200.00 0 0 100.00"), "the stored image keeps the chart's size");
        let text = document.extract_text(&[1]).unwrap();
        assert!(in_order(&text, &["Revenue by region", "Costs by region"]), "{text}");
    }

    #[test]
    fn odt_renders_tracked_changes_as_accepted_and_leaves_comments_out() {
        let root = sandbox("odt_tracked");
        let input = root.join("review.odt");
        let body = concat!(
            r#"<text:tracked-changes><text:changed-region text:id="d1"><text:deletion><office:change-info><dc:creator>A</dc:creator></office:change-info><text:p>deleted words</text:p></text:deletion></text:changed-region><text:changed-region text:id="i1"><text:insertion><office:change-info><dc:creator>A</dc:creator></office:change-info></text:insertion></text:changed-region><text:changed-region text:id="f1"><text:format-change><office:change-info><dc:creator>A</dc:creator></office:change-info></text:format-change></text:changed-region></text:tracked-changes>"#,
            r#"<text:p>Kept <text:change-start text:change-id="i1"/>inserted words<text:change-end text:change-id="i1"/><text:change text:change-id="d1"/><office:annotation office:name="c7"><dc:creator>Reviewer</dc:creator><dc:date>2026-01-01T00:00:00</dc:date><text:p>Secret reviewer remark</text:p></office:annotation> commented<office:annotation-end office:name="c7"/></text:p>"#,
            r#"<text:p><text:change-start text:change-id="f1"/>Reformatted<text:change-end text:change-id="f1"/></text:p>"#,
            r#"<table:table><table:table-row><table:table-cell><text:p>Kept row</text:p></table:table-cell></table:table-row><table:table-row><table:table-cell><text:p><text:change-start text:change-id="i1"/>Inserted row<text:change-end text:change-id="i1"/></text:p></table:table-cell></table:table-row></table:table>"#,
        );
        fs::write(&input, odt::fixtures::odt("", body, &[], &[])).unwrap();

        let outcome = &convert_paths(&[&input], json!({}))[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert!(outcome.warnings.is_empty(), "comments are left out without a warning: {:?}", outcome.warnings);
        let text = Document::load(root.join("review.pdf")).unwrap().extract_text(&[1]).unwrap();
        assert!(in_order(&text, &["Kept", "inserted words", "commented", "Reformatted", "Kept row", "Inserted row"]), "{text}");
        for absent in ["deleted words", "Secret reviewer remark", "Reviewer"] {
            assert!(!text.contains(absent), "{absent:?} should be left out: {text}");
        }
    }

    #[test]
    fn odt_places_anchored_frames_at_their_anchors_with_an_approximate_layout_note() {
        let root = sandbox("odt_floating");
        let input = root.join("brochure.odt");
        let automatic = concat!(
            r##"<style:style style:name="gr1" style:family="graphic"><style:graphic-properties draw:fill="solid" draw:fill-color="#fff2cc" draw:stroke="solid" svg:stroke-color="#000000" style:wrap="parallel" style:horizontal-pos="from-left" style:horizontal-rel="page" style:vertical-pos="from-top" style:vertical-rel="page"/></style:style>"##,
            r##"<style:style style:name="gr2" style:family="graphic"><style:graphic-properties draw:fill="solid" draw:fill-color="#2e75b6" draw:stroke="none" style:wrap="run-through" style:horizontal-pos="from-left" style:horizontal-rel="page-content" style:vertical-pos="from-top" style:vertical-rel="paragraph"/></style:style>"##,
            r#"<style:style style:name="fr1" style:family="graphic"><style:graphic-properties style:wrap="none" style:horizontal-pos="from-left" style:horizontal-rel="paragraph" style:vertical-pos="from-top" style:vertical-rel="paragraph"/></style:style>"#,
            r#"<style:style style:name="P1" style:family="paragraph"><style:paragraph-properties fo:break-before="page"/></style:style>"#,
        );
        let body = concat!(
            r#"<text:p><draw:frame draw:style-name="gr1" text:anchor-type="paragraph" svg:x="300pt" svg:y="100pt" svg:width="150pt" svg:height="50pt"><draw:text-box><text:p>Boxed callout</text:p></draw:text-box></draw:frame>Front page text</text:p>"#,
            r#"<text:p><draw:ellipse draw:style-name="gr2" text:anchor-type="paragraph" svg:x="0pt" svg:y="0pt" svg:width="100pt" svg:height="20pt"/>Banner paragraph</text:p>"#,
            r#"<text:p text:style-name="P1"><draw:frame draw:style-name="fr1" text:anchor-type="paragraph" svg:x="0pt" svg:y="0pt" svg:width="100pt" svg:height="100pt"><draw:image xlink:href="Pictures/photo.png"/></draw:frame>Photo caption</text:p>"#,
            r#"<text:p>Text after the photo</text:p>"#,
        );
        fs::write(&input, odt::fixtures::odt(automatic, body, &[("Pictures/photo.png", &png(10, 10, [200, 40, 40]))], &[])).unwrap();

        let outcome = &convert_paths(&[&input], json!({ "pageDefaults": { "pageSize": "letter", "margins": "normal" } }))[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert_eq!(outcome.detail, "2 pages.");
        assert_eq!(outcome.warnings, [OutcomeWarning {
            kind: WarningKind::ApproximateLayout,
            message: "Approximate layout: 3 floating objects on pages 1 and 2 were placed as close as possible.".into(),
        }]);

        let document = Document::load(root.join("brochure.pdf")).unwrap();
        let first = page_content(&document, 1);
        assert!(first.contains("1 0.9490196 0.8 rg 300.00 642.00 150.00 50.00 re f"), "the text box sits at its page offset: {first}");
        assert!(first.contains("0 0 0 RG 0.75 w 300.00 642.00 150.00 50.00 re S"), "the text box keeps its outline");
        assert!(first.contains("0.18039216 0.45882353 0.7137255 rg"), "the anchored shape is filled");
        let text = document.extract_text(&[1]).unwrap();
        assert!(in_order(&text, &["Boxed callout", "Front page text", "Banner paragraph"]), "{text}");
        assert_eq!(page_images(&document, 1), 0);
        assert_eq!(page_images(&document, 2), 1, "the wrapped picture stays on its anchor's page");
        let second = page_content(&document, 2);
        let picture_bottom = second.split("100.00 0 0 100.00 ").nth(1).and_then(|rest| rest.split_whitespace().nth(1)).and_then(|value| value.parse::<f32>().ok()).expect(&second);
        let second_text = document.extract_text(&[2]).unwrap();
        assert!(in_order(&second_text, &["Photo caption", "Text after the photo"]), "{second_text}");
        assert_eq!(picture_bottom, 792. - 72. - 100., "the picture sits at the top of its anchor paragraph on page 2");
        let first_line = second.split(" Tm ").next().and_then(|head| head.split_whitespace().last()).and_then(|value| value.parse::<f32>().ok()).expect(&second);
        assert!(first_line < picture_bottom, "wrapped text flows below the picture, not under it: {first_line}");
    }

    fn encrypted_open_document(mimetype: &str) -> Vec<u8> {
        let manifest = br#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"><manifest:encryption-data manifest:checksum-type="SHA1/1K" manifest:checksum="AA=="><manifest:algorithm manifest:algorithm-name="Blowfish CFB" manifest:initialisation-vector="AA=="/></manifest:encryption-data></manifest:file-entry></manifest:manifest>"#;
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in [("mimetype", mimetype.as_bytes()), ("META-INF/manifest.xml", &manifest[..]), ("content.xml", b"\x9c\x01 ciphertext")] {
            archive.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut archive, data).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }

    #[test]
    fn locked_damaged_and_oversized_odt_documents_fail_alone() {
        let root = sandbox("odt_batch");
        let locked = root.join("locked.odt");
        fs::write(&locked, encrypted_open_document("application/vnd.oasis.opendocument.text")).unwrap();
        let damaged = root.join("damaged.odt");
        fs::write(&damaged, b"PK\x03\x04 not really a package").unwrap();
        let bomb = root.join("bomb.odt");
        fs::write(&bomb, crate::kit::common::limits::fixtures::zip_with_deflated("content.xml", &crate::kit::common::limits::fixtures::zlib_of_zeros(500_000_001), 500_000_001)).unwrap();
        let good = root.join("letter.odt");
        fs::write(&good, odt::fixtures::odt("", "<text:p>Dear reader,</text:p>", &[], &[])).unwrap();
        let originals = [&locked, &damaged, &bomb, &good].map(|path| fs::read(path).unwrap());

        let outcomes = convert_paths(&[&locked, &good, &damaged, &bomb], json!({ "pageDefaults": { "pageSize": "letter", "margins": "narrow" } }));
        let error = outcomes[0].error().unwrap();
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        assert_eq!(error.message, "It's password-protected. Unlock it in Lock & Unlock, then convert the unlocked copy.");
        assert_eq!(error.pointer, Some(OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
        assert_eq!(outcomes[2].error().unwrap().message, odt::DAMAGED);
        let limited = outcomes[3].error().unwrap();
        assert_eq!(limited.kind, ErrorKind::LimitExceeded { limit: crate::kit::contracts::LimitKind::Decompression }, "{limited:?}");
        assert!(limited.message.contains("500 MB"), "{}", limited.message);
        assert!(succeeded(&outcomes[1]), "{outcomes:?}");
        assert_eq!(outcomes[1].detail, "1 page.");
        assert_eq!(media_box(&Document::load(root.join("letter.pdf")).unwrap(), 1), [0., 0., 612., 792.], "without page setup the page defaults apply");
        assert_eq!([&locked, &damaged, &bomb, &good].map(|path| fs::read(path).unwrap()), originals);
        assert!(!root.join("locked.pdf").exists() && !root.join("damaged.pdf").exists() && !root.join("bomb.pdf").exists());
    }

    fn media_box(document: &Document, page: u32) -> Vec<f32> {
        document.get_dictionary(document.get_pages()[&page]).unwrap().get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|value| value.as_float().unwrap()).collect()
    }

    fn font_sizes(document: &Document, page: u32) -> Vec<f32> {
        let content = String::from_utf8_lossy(&document.get_page_content(document.get_pages()[&page])).into_owned();
        let tokens = content.split_whitespace().collect::<Vec<_>>();
        tokens.windows(2).filter(|pair| pair[1] == "Tf").filter_map(|pair| pair[0].parse().ok()).collect()
    }

    fn in_order(text: &str, needles: &[&str]) -> bool {
        let mut rest = text;
        needles.iter().all(|needle| match rest.find(needle) { Some(at) => { rest = &rest[at + needle.len()..]; true } None => false })
    }

    #[test]
    fn quoted_csv_with_a_custom_delimiter_and_default_tsv_keep_cells_in_order() {
        let root = sandbox("tables");
        let csv = root.join("ledger.csv");
        let tsv = root.join("people.tsv");
        let csv_bytes = b"Name;Note;Amount\n\"Smith; Jo\";\"said \"\"hi\"\"\";1200\nLee;plain;-3\n".to_vec();
        let tsv_bytes = b"City\tCountry\nOslo\tNorway\nLima, Centro\tPeru\n".to_vec();
        fs::write(&csv, &csv_bytes).unwrap();
        fs::write(&tsv, &tsv_bytes).unwrap();

        let outcomes = convert_paths(&[&csv], json!({ "tables": { "delimiter": "semicolon", "headerRow": true, "encoding": "detect", "orientation": "portrait", "fit": "width" } }));
        assert!(succeeded(&outcomes[0]), "{:?}", outcomes[0]);
        assert_eq!(outcomes[0].detail, "1 page. 2 rows, semicolon. Read as UTF-8.");
        assert_eq!(outcomes[0].fonts[0].requested_family, "Arial");
        let text = Document::load(root.join("ledger.pdf")).unwrap().extract_text(&[1]).unwrap();
        assert!(in_order(&text, &["Name", "Note", "Amount", "Smith; Jo", "said \"hi\"", "1200", "Lee", "plain", "-3"]), "{text}");

        let outcomes = convert_paths(&[&tsv], json!({}));
        assert!(succeeded(&outcomes[0]), "{:?}", outcomes[0]);
        assert_eq!(outcomes[0].detail, "1 page. 2 rows, tab. Read as UTF-8.");
        let document = Document::load(root.join("people.pdf")).unwrap();
        assert_eq!(media_box(&document, 1), [0., 0., 841.89, 595.28]);
        let text = document.extract_text(&[1]).unwrap();
        assert!(in_order(&text, &["City", "Country", "Oslo", "Norway", "Lima, Centro", "Peru"]), "{text}");

        let comma = convert_paths(&[&tsv], json!({ "tables": { "delimiter": "comma", "headerRow": true, "encoding": "detect", "orientation": "landscape", "fit": "width" } }));
        assert_eq!(comma[0].detail, "1 page. 2 rows, comma. Read as UTF-8.");
        assert_eq!(fs::read(&csv).unwrap(), csv_bytes);
        assert_eq!(fs::read(&tsv).unwrap(), tsv_bytes);
        assert_eq!(names(&root), ["ledger.csv", "ledger.pdf", "people-1.pdf", "people.pdf", "people.tsv"]);
    }

    #[test]
    fn long_tables_repeat_the_header_on_every_page() {
        let root = sandbox("table_pages");
        let input = root.join("orders.tsv");
        let mut text = String::from("Order\tClient\tTotal\n");
        for index in 0..300 { text.push_str(&format!("{index}\tCustomer {index}\t{}.50\n", index * 3)); }
        fs::write(&input, &text).unwrap();
        let outcomes = convert_paths(&[&input], json!({}));
        assert!(succeeded(&outcomes[0]), "{:?}", outcomes[0]);
        let document = Document::load(root.join("orders.pdf")).unwrap();
        let pages = document.get_pages().len() as u32;
        assert!(pages > 3, "{pages}");
        assert_eq!(outcomes[0].detail, format!("{pages} pages. 300 rows, tab. Read as UTF-8."));
        let mut seen = 0;
        for page in 1..=pages {
            let text = document.extract_text(&[page]).unwrap();
            assert!(in_order(&text, &["Order", "Client", "Total"]), "page {page}: {text}");
            assert_eq!(text.matches("Total").count(), 1, "page {page}");
            assert!(text.contains(&format!("Customer {seen}")), "page {page} continues at row {seen}");
            seen += text.matches("Customer ").count();
        }
        assert_eq!(seen, 300);
        assert_eq!([grouped(1), grouped(999), grouped(1240), grouped(1_234_567)], ["1", "999", "1,240", "1,234,567"]);

        let headless = convert_paths(&[&input], json!({ "tables": { "delimiter": "auto", "headerRow": false, "encoding": "detect", "orientation": "landscape", "fit": "width" } }));
        assert!(headless[0].detail.ends_with(" pages. 301 rows, tab. Read as UTF-8."), "{}", headless[0].detail);
        let document = Document::load(root.join("orders-1.pdf")).unwrap();
        assert!(!document.extract_text(&[2]).unwrap().contains("Total"));
    }

    #[test]
    fn orientation_and_fit_change_page_geometry_and_text_size() {
        let root = sandbox("table_fit");
        let input = root.join("wide.csv");
        let header = (0..10).map(|index| format!("Column heading {index}")).collect::<Vec<_>>().join(",");
        let row = (0..10).map(|index| format!("value number {index}")).collect::<Vec<_>>().join(",");
        fs::write(&input, format!("{header}\n{row}\n")).unwrap();
        let options = |orientation: &str, fit: &str| json!({ "pageDefaults": { "pageSize": "letter", "margins": "narrow" }, "tables": { "delimiter": "auto", "headerRow": true, "encoding": "detect", "orientation": orientation, "fit": fit } });

        for (orientation, fit) in [("landscape", "width"), ("portrait", "width"), ("portrait", "none")] {
            assert!(succeeded(&convert_paths(&[&input], options(orientation, fit))[0]));
        }
        let landscape = Document::load(root.join("wide.pdf")).unwrap();
        let portrait = Document::load(root.join("wide-1.pdf")).unwrap();
        let unfitted = Document::load(root.join("wide-2.pdf")).unwrap();
        assert_eq!(media_box(&landscape, 1), [0., 0., 792., 612.]);
        assert_eq!(media_box(&portrait, 1), [0., 0., 612., 792.]);
        let largest = |document: &Document| font_sizes(document, 1).into_iter().fold(0f32, f32::max);
        assert!(largest(&portrait) < largest(&landscape), "{} vs {}", largest(&portrait), largest(&landscape));
        assert!(largest(&landscape) < largest(&unfitted));
        assert_eq!(largest(&unfitted), 9.5);
        let text = portrait.extract_text(&[1]).unwrap();
        assert!(in_order(&text, &["Column heading 0", "Column heading 9", "value number 0", "value number 9"]), "{text}");
    }

    #[test]
    fn invalid_delimiters_and_empty_tables_fail_without_output() {
        let root = sandbox("table_invalid");
        let input = root.join("data.csv");
        let empty = root.join("empty.tsv");
        fs::write(&input, "a,b\n1,2\n").unwrap();
        fs::write(&empty, "\n\n").unwrap();
        for delimiter in [json!("colon"), json!(";;"), json!(9)] {
            let outcomes = convert_paths(&[&input], json!({ "tables": { "delimiter": delimiter, "headerRow": true, "encoding": "detect", "orientation": "landscape", "fit": "width" } }));
            assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::InvalidInput, "{delimiter}");
        }
        let outcomes = convert_paths(&[&empty, &input], json!({}));
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::InvalidInput);
        assert_eq!(outcomes[0].error().unwrap().message, "This table has no rows, so no PDF was made.");
        assert!(succeeded(&outcomes[1]));
        assert_eq!(names(&root), ["data.csv", "data.pdf", "empty.tsv"]);
    }

    fn xlsx_workbook() -> Vec<u8> {
        let styles = r#"<fonts count="1"><font><sz val="11"/><name val="Arial"/></font></fonts><cellXfs count="2"><xf numFmtId="0" fontId="0"/><xf numFmtId="4" fontId="0"/></cellXfs>"#;
        let summary = xlsx::fixtures::sheet("", concat!(
            r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" t="s"><v>2</v></c></row>"#,
            r#"<row r="2"><c r="A2" t="s"><v>3</v></c><c r="B2" s="1"><f>SUM(40,2)</f><v>42</v></c></row>"#,
            r#"<row r="3"><c r="A3" t="s"><v>4</v></c><c r="B3" s="1"><v>7</v></c></row>"#,
            r#"<row r="10"><c r="A10" t="s"><v>5</v></c></row>"#,
        ), r#"<pageSetup orientation="portrait"/>"#);
        let secret = xlsx::fixtures::sheet("", r#"<row r="1"><c r="A1" t="s"><v>6</v></c></row>"#, "");
        let wide = xlsx::fixtures::sheet("", r#"<row r="1"><c r="A1" t="s"><v>7</v></c></row>"#, r#"<pageSetup orientation="landscape"/>"#);
        xlsx::fixtures::xlsx(
            &[("Summary", "visible", &summary), ("Secret", "hidden", &secret), ("Wide", "visible", &wide)],
            r#"<definedName name="_xlnm.Print_Area" localSheetId="0">Summary!$A$1:$B$3</definedName><definedName name="_xlnm.Print_Titles" localSheetId="0">Summary!$1:$1</definedName>"#,
            styles,
            &["Item", "Total", "Beyond", "Pens", "Ink", "Outside", "Hidden note", "Wide data"],
        )
    }

    fn ods_workbook() -> Vec<u8> {
        let styles = concat!(
            r#"<office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" style:print-orientation="portrait"/></style:page-layout>"#,
            r#"<style:page-layout style:name="pm2"><style:page-layout-properties fo:page-width="29.7cm" fo:page-height="21cm" style:print-orientation="landscape"/></style:page-layout></office:automatic-styles>"#,
            r#"<office:master-styles><style:master-page style:name="Default" style:page-layout-name="pm1"/><style:master-page style:name="Landscape" style:page-layout-name="pm2"/></office:master-styles>"#,
        );
        let automatic = concat!(
            r#"<style:style style:name="ta1" style:family="table" style:master-page-name="Default"><style:table-properties table:display="true"/></style:style>"#,
            r#"<style:style style:name="ta2" style:family="table" style:master-page-name="Default"><style:table-properties table:display="false"/></style:style>"#,
            r#"<style:style style:name="ta3" style:family="table" style:master-page-name="Landscape"><style:table-properties table:display="true"/></style:style>"#,
        );
        let cell = |text: &str| format!("<table:table-cell><text:p>{text}</text:p></table:table-cell>");
        let number = |value: &str, text: &str, formula: &str| format!(r#"<table:table-cell {formula} office:value-type="float" office:value="{value}"><text:p>{text}</text:p></table:table-cell>"#);
        let body = format!(
            concat!(
                r#"<table:table table:name="Summary" table:style-name="ta1" table:print-ranges="Summary.A1:Summary.B3"><table:table-column table:number-columns-repeated="3"/>"#,
                "<table:table-header-rows><table:table-row>{}{}{}</table:table-row></table:table-header-rows>",
                "<table:table-row>{}{}</table:table-row><table:table-row>{}{}</table:table-row>",
                r#"<table:table-row table:number-rows-repeated="6"><table:table-cell table:number-columns-repeated="3"/></table:table-row><table:table-row>{}</table:table-row></table:table>"#,
                r#"<table:table table:name="Secret" table:style-name="ta2"><table:table-row>{}</table:table-row></table:table>"#,
                r#"<table:table table:name="Wide" table:style-name="ta3"><table:table-row>{}</table:table-row></table:table>"#,
            ),
            cell("Item"), cell("Total"), cell("Beyond"),
            cell("Pens"), number("42", "42.00", r#"table:formula="of:=SUM(40;2)""#), cell("Ink"), number("7", "7.00", ""),
            cell("Outside"), cell("Hidden note"), cell("Wide data"),
        );
        ods::fixtures::ods(&styles, automatic, &body)
    }

    fn sheets_option(sheets: &str, print_areas: bool) -> serde_json::Value {
        json!({ "spreadsheets": { "sheets": sheets, "printAreas": print_areas, "fitToWidth": true } })
    }

    fn assert_spreadsheet_outcomes(root: &Path, input: &Path) {
        let original = fs::read(input).unwrap();
        let outcomes = convert_paths(&[input], json!({}));
        assert!(succeeded(&outcomes[0]), "{:?}", outcomes[0]);
        assert_eq!(outcomes[0].detail, "2 pages. 2 sheets. 1 hidden sheet left out.");
        let document = Document::load(root.join("book.pdf")).unwrap();
        assert_eq!(document.get_pages().len(), 2);
        let summary = document.extract_text(&[1]).unwrap();
        assert!(in_order(&summary, &["Item", "Total", "Pens", "42.00", "Ink", "7.00"]), "{summary}");
        for absent in ["Beyond", "Outside", "SUM", "Hidden note"] { assert!(!summary.contains(absent), "{absent} in {summary}"); }
        let (portrait, landscape) = (media_box(&document, 1), media_box(&document, 2));
        assert!(portrait[2] < portrait[3], "{portrait:?}");
        assert!(landscape[2] > landscape[3], "{landscape:?}");
        assert!(document.extract_text(&[2]).unwrap().contains("Wide data"));
        assert_eq!(outline_titles(&document), ["Summary", "Wide"]);

        let outcomes = convert_paths(&[input], sheets_option("all", true));
        assert_eq!(outcomes[0].detail, "3 pages. 3 sheets.");
        let document = Document::load(root.join("book-1.pdf")).unwrap();
        assert!(document.extract_text(&[2]).unwrap().contains("Hidden note"));
        assert_eq!(outline_titles(&document), ["Summary", "Secret", "Wide"]);

        let outcomes = convert_paths(&[input], sheets_option("visible", false));
        assert_eq!(outcomes[0].detail, "2 pages. 2 sheets. 1 hidden sheet left out.");
        let whole = Document::load(root.join("book-2.pdf")).unwrap().extract_text(&[1]).unwrap();
        assert!(in_order(&whole, &["Item", "Total", "Beyond", "Pens", "42.00", "Outside"]), "{whole}");
        assert_eq!(fs::read(input).unwrap(), original);
    }

    #[test]
    fn excel_workbooks_respect_hidden_sheets_print_areas_orientation_and_stored_results() {
        let root = sandbox("xlsx");
        let input = root.join("book.xlsx");
        fs::write(&input, xlsx_workbook()).unwrap();
        assert_spreadsheet_outcomes(&root, &input);
        assert_eq!(names(&root), ["book-1.pdf", "book-2.pdf", "book.pdf", "book.xlsx"]);
    }

    #[test]
    fn opendocument_spreadsheets_match_excel() {
        let root = sandbox("ods");
        let input = root.join("book.ods");
        fs::write(&input, ods_workbook()).unwrap();
        assert_spreadsheet_outcomes(&root, &input);
        assert_eq!(names(&root), ["book-1.pdf", "book-2.pdf", "book.ods", "book.pdf"]);
    }

    #[test]
    fn wide_sheets_fit_the_page_width_unless_turned_off() {
        let root = sandbox("xlsx_fit");
        let cells = (0..26u8).map(|column| format!(r#"<c r="{}1" t="inlineStr"><is><t>Column heading {column}</t></is></c>"#, (b'A' + column) as char)).collect::<String>();
        let wide = xlsx::fixtures::sheet("", &format!(r#"<row r="1">{cells}</row>"#), "");
        let input = root.join("wide.xlsx");
        fs::write(&input, xlsx::fixtures::xlsx(&[("Wide", "visible", &wide)], "", "", &[])).unwrap();
        let fit = convert_paths(&[&input], json!({}));
        assert!(succeeded(&fit[0]), "{:?}", fit[0]);
        let fitted = font_sizes(&Document::load(root.join("wide.pdf")).unwrap(), 1);
        assert!(fitted.iter().all(|size| *size < TABLE_SIZE * 0.95 * 0.6), "{fitted:?}");
        convert_paths(&[&input], json!({ "spreadsheets": { "sheets": "visible", "printAreas": true, "fitToWidth": false } }));
        let natural = font_sizes(&Document::load(root.join("wide-1.pdf")).unwrap(), 1);
        assert!(natural.iter().all(|size| *size == TABLE_SIZE * 0.95), "{natural:?}");
    }

    #[test]
    fn locked_damaged_oversized_and_blank_spreadsheets_fail_alone() {
        let root = sandbox("xlsx_batch");
        let plain = root.join("plain.xlsx");
        fs::write(&plain, crate::kit::office::test_minimal_ooxml_package("xlsx")).unwrap();
        fs::create_dir_all(root.join("locked")).unwrap();
        let protected = crate::kit::office::OfficeProcessor::protect(plain.clone(), crate::kit::TEST_PASSWORD, &OutputLocation::CustomFolder(root.join("locked")));
        let locked = protected.output_paths().first().cloned().unwrap_or_else(|| panic!("{protected:?}"));
        let damaged = root.join("damaged.xlsx");
        fs::write(&damaged, b"PK\x03\x04 not really a package").unwrap();
        let bomb = root.join("bomb.xlsx");
        fs::write(&bomb, crate::kit::common::limits::fixtures::zip_with_deflated("xl/workbook.xml", &crate::kit::common::limits::fixtures::zlib_of_zeros(500_000_001), 500_000_001)).unwrap();
        let hidden_only = root.join("hidden.xlsx");
        let note = xlsx::fixtures::sheet("", r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Only here</t></is></c></row>"#, "");
        fs::write(&hidden_only, xlsx::fixtures::xlsx(&[("Blank", "visible", &xlsx::fixtures::sheet("", "", "")), ("Notes", "hidden", &note)], "", "", &[])).unwrap();
        let good = root.join("good.xlsx");
        fs::write(&good, xlsx_workbook()).unwrap();
        let locked_ods = root.join("locked.ods");
        fs::write(&locked_ods, encrypted_open_document("application/vnd.oasis.opendocument.spreadsheet")).unwrap();
        let inputs = [&locked, &damaged, &bomb, &hidden_only, &good, &locked_ods];
        let originals = inputs.map(|path| fs::read(path).unwrap());

        let outcomes = convert_paths(&inputs.map(|path| path.as_path()), json!({}));
        let error = outcomes[0].error().unwrap();
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        assert_eq!(error.message, LOCKED);
        assert_eq!(error.pointer, Some(OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
        assert_eq!(outcomes[1].error().unwrap().message, xlsx::DAMAGED);
        assert_eq!(outcomes[2].error().unwrap().kind, ErrorKind::LimitExceeded { limit: crate::kit::contracts::LimitKind::Decompression });
        assert_eq!(outcomes[3].error().unwrap().message, "This spreadsheet has nothing to print, so no PDF was made. Its hidden sheets have content; choose All sheets to include them.");
        assert!(succeeded(&outcomes[4]), "{:?}", outcomes[4]);
        assert_eq!((outcomes[5].error().unwrap().message.as_str(), &outcomes[5].error().unwrap().pointer), (LOCKED, &error.pointer));
        assert_eq!(inputs.map(|path| fs::read(path).unwrap()), originals);
        assert!(["locked.pdf", "locked-1.pdf", "damaged.pdf", "bomb.pdf", "hidden.pdf"].iter().all(|name| !root.join(name).exists() && !root.join("locked").join(name).exists()));
    }

    const PICTURE: &str = r#"<p:pic><p:nvPicPr><p:cNvPr id="3" name="Picture"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rIdImage"/></p:blipFill><p:spPr><a:xfrm><a:off x="6350000" y="2540000"/><a:ext cx="2540000" cy="1270000"/></a:xfrm></p:spPr></p:pic>"#;
    const CHART_FRAME: &str = r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="5" name="Chart"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="635000" y="3810000"/><a:ext cx="2540000" cy="1270000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rIdChart"/></a:graphicData></a:graphic></p:graphicFrame>"#;

    fn pptx_deck() -> Vec<u8> {
        use pptx::fixtures::{text_shape, FixtureSlide};
        let first = format!("{}{}{PICTURE}", text_shape(Some("title"), (36, 24, 600, 60), "Quarterly review", ""), text_shape(None, (36, 120, 400, 200), "Revenue grew", ""));
        let hidden = text_shape(Some("title"), (36, 24, 600, 60), "Draft numbers", "");
        let third = format!(
            r#"{}{}<mc:AlternateContent><mc:Choice Requires="c14">{CHART_FRAME}</mc:Choice><mc:Fallback>{PICTURE}</mc:Fallback></mc:AlternateContent>{CHART_FRAME}"#,
            text_shape(Some("title"), (36, 24, 600, 60), "Charts", ""),
            text_shape(None, (500, 300, 200, 80), "Tilted", r#" rot="1800000""#),
        );
        let media: &[(&str, &str, &str)] = &[("rIdImage", "image", "../media/image1.png"), ("rIdChart", "chart", "../charts/chart1.xml")];
        pptx::fixtures::pptx((960, 540), &[
            FixtureSlide { shapes: &first, notes: Some("Mention the new region"), rels: media, ..FixtureSlide::default() },
            FixtureSlide { shapes: &hidden, hidden: true, notes: Some("Hidden notes"), rels: media },
            FixtureSlide { shapes: &third, rels: media, ..FixtureSlide::default() },
        ], &[("ppt/media/image1.png", &png(20, 10, [10, 120, 60])), ("ppt/charts/chart1.xml", br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#)])
    }

    fn odp_deck() -> Vec<u8> {
        let styles = concat!(
            r#"<office:automatic-styles><style:page-layout style:name="PM1"><style:page-layout-properties fo:page-width="960pt" fo:page-height="540pt"/></style:page-layout></office:automatic-styles>"#,
            r#"<office:master-styles><style:master-page style:name="Default" style:page-layout-name="PM1"/></office:master-styles>"#,
        );
        let automatic = r#"<style:style style:name="dp1" style:family="drawing-page"/><style:style style:name="dp2" style:family="drawing-page"><style:drawing-page-properties presentation:visibility="hidden"/></style:style>"#;
        let text = |class: &str, (x, y, width, height): (u32, u32, u32, u32), text: &str, extra: &str| {
            let class = if class.is_empty() { String::new() } else { format!(r#" presentation:class="{class}""#) };
            format!(r#"<draw:frame{class} svg:x="{x}pt" svg:y="{y}pt" svg:width="{width}pt" svg:height="{height}pt"{extra}><draw:text-box><text:p>{text}</text:p></draw:text-box></draw:frame>"#)
        };
        let notes = |words: &str| format!("<presentation:notes>{}</presentation:notes>", text("notes", (36, 400, 600, 100), words, ""));
        let picture = r#"<draw:frame svg:x="500pt" svg:y="200pt" svg:width="200pt" svg:height="100pt"><draw:image xlink:href="Pictures/image1.png"/></draw:frame>"#;
        let stored_chart = r#"<draw:frame svg:x="500pt" svg:y="200pt" svg:width="200pt" svg:height="100pt"><draw:object xlink:href="./Object 1"/><draw:image xlink:href="Pictures/image1.png"/></draw:frame>"#;
        let chart = r#"<draw:frame svg:x="50pt" svg:y="300pt" svg:width="200pt" svg:height="100pt"><draw:object><office:document><office:body><office:chart/></office:body></office:document></draw:object></draw:frame>"#;
        let body = format!(
            concat!(
                r#"<draw:page draw:name="One" draw:style-name="dp1" draw:master-page-name="Default">{}{}{}{}</draw:page>"#,
                r#"<draw:page draw:name="Two" draw:style-name="dp2" draw:master-page-name="Default">{}{}</draw:page>"#,
                r#"<draw:page draw:name="Three" draw:style-name="dp1" draw:master-page-name="Default">{}{}{}{}</draw:page>"#,
            ),
            text("title", (36, 24, 600, 60), "Quarterly review", ""), text("", (36, 120, 400, 200), "Revenue grew", ""), picture, notes("Mention the new region"),
            text("title", (36, 24, 600, 60), "Draft numbers", ""), notes("Hidden notes"),
            text("title", (36, 24, 600, 60), "Charts", ""), text("", (500, 300, 200, 80), "Tilted", r#" draw:transform="rotate (0.5) translate (500pt 300pt)""#).replace(r#" svg:x="500pt" svg:y="300pt""#, ""), stored_chart, chart,
        );
        odp::fixtures::odp(styles, automatic, &body, &[("Pictures/image1.png", &png(20, 10, [10, 120, 60]))])
    }

    fn presentation_options(speaker_notes: bool, hidden_slides: bool) -> serde_json::Value {
        json!({ "presentations": { "speakerNotes": speaker_notes, "hiddenSlides": hidden_slides } })
    }

    fn assert_presentation_outcomes(root: &Path, input: &Path) {
        let original = fs::read(input).unwrap();
        let outcomes = convert_paths(&[input], json!({}));
        let outcome = &outcomes[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert_eq!(outcome.detail, "2 pages. 2 slides. 1 hidden slide left out.");
        assert_eq!(outcome.warnings, [
            OutcomeWarning { kind: WarningKind::OmittedContent, message: "Omitted content: 1 chart has no stored image, so it was left out.".into() },
            OutcomeWarning { kind: WarningKind::ApproximateLayout, message: "Approximate layout: 1 shape on slide 3 was placed as close as possible.".into() },
        ]);
        let document = Document::load(root.join("deck.pdf")).unwrap();
        assert_eq!(document.get_pages().len(), 2);
        for page in [1, 2] { assert_eq!(media_box(&document, page), [0., 0., 960., 540.]); }
        let first = document.extract_text(&[1]).unwrap();
        assert!(in_order(&first, &["Quarterly review", "Revenue grew"]), "{first}");
        for absent in ["Mention the new region", "Draft numbers", "Speaker notes"] {
            let all = document.extract_text(&[1, 2]).unwrap();
            assert!(!all.contains(absent), "{absent} in {all}");
        }
        assert!(document.extract_text(&[2]).unwrap().contains("Tilted"));
        assert_eq!((page_images(&document, 1), page_images(&document, 2)), (1, 1), "pictures and the chart's stored image are drawn");
        assert_eq!(outline_titles(&document), ["Quarterly review", "Charts"]);

        let outcomes = convert_paths(&[input], presentation_options(true, true));
        assert_eq!(outcomes[0].detail, "5 pages. 3 slides.");
        let document = Document::load(root.join("deck-1.pdf")).unwrap();
        assert_eq!(document.get_pages().len(), 5);
        for page in 1..=5 { assert_eq!(media_box(&document, page), [0., 0., 960., 540.]); }
        let notes = document.extract_text(&[2]).unwrap();
        assert!(in_order(&notes, &["Speaker notes", "Mention the new region"]), "{notes}");
        assert!(document.extract_text(&[3]).unwrap().contains("Draft numbers"));
        assert!(document.extract_text(&[4]).unwrap().contains("Hidden notes"));
        assert!(document.extract_text(&[5]).unwrap().contains("Charts"));
        assert_eq!(outline_titles(&document), ["Quarterly review", "Draft numbers", "Charts"]);

        let outcomes = convert_paths(&[input], presentation_options(true, false));
        assert_eq!(outcomes[0].detail, "3 pages. 2 slides. 1 hidden slide left out.");
        let all = Document::load(root.join("deck-2.pdf")).unwrap().extract_text(&[1, 2, 3]).unwrap();
        assert!(in_order(&all, &["Quarterly review", "Mention the new region", "Charts"]), "{all}");
        assert!(!all.contains("Hidden notes"), "{all}");
        assert_eq!(fs::read(input).unwrap(), original);
    }

    #[test]
    fn powerpoint_decks_print_one_page_per_visible_slide_with_notes_and_hidden_slides_on_request() {
        let root = sandbox("pptx");
        let input = root.join("deck.pptx");
        fs::write(&input, pptx_deck()).unwrap();
        assert_presentation_outcomes(&root, &input);
        assert_eq!(names(&root), ["deck-1.pdf", "deck-2.pdf", "deck.pdf", "deck.pptx"]);
        let document = Document::load(root.join("deck.pdf")).unwrap();
        let info = document.trailer.get(b"Info").and_then(Object::as_reference).and_then(|id| document.get_dictionary(id)).unwrap();
        assert_eq!(info.get(b"Title").unwrap().as_str().unwrap(), b"Quarterly review");
    }

    #[test]
    fn opendocument_presentations_match_powerpoint() {
        let root = sandbox("odp");
        let input = root.join("deck.odp");
        fs::write(&input, odp_deck()).unwrap();
        assert_presentation_outcomes(&root, &input);
        assert_eq!(names(&root), ["deck-1.pdf", "deck-2.pdf", "deck.odp", "deck.pdf"]);
    }

    #[test]
    fn locked_damaged_oversized_and_hidden_only_presentations_fail_alone() {
        let root = sandbox("pptx_batch");
        let plain = root.join("plain.pptx");
        fs::write(&plain, crate::kit::office::test_minimal_ooxml_package("pptx")).unwrap();
        fs::create_dir_all(root.join("locked")).unwrap();
        let protected = crate::kit::office::OfficeProcessor::protect(plain.clone(), crate::kit::TEST_PASSWORD, &OutputLocation::CustomFolder(root.join("locked")));
        let locked = protected.output_paths().first().cloned().unwrap_or_else(|| panic!("{protected:?}"));
        let damaged = root.join("damaged.pptx");
        fs::write(&damaged, b"PK\x03\x04 not really a package").unwrap();
        let bomb = root.join("bomb.pptx");
        fs::write(&bomb, crate::kit::common::limits::fixtures::zip_with_deflated("ppt/presentation.xml", &crate::kit::common::limits::fixtures::zlib_of_zeros(500_000_001), 500_000_001)).unwrap();
        let hidden_only = root.join("hidden.pptx");
        let shape = pptx::fixtures::text_shape(None, (36, 36, 200, 50), "Secret", "");
        fs::write(&hidden_only, pptx::fixtures::pptx((720, 540), &[pptx::fixtures::FixtureSlide { shapes: &shape, hidden: true, ..Default::default() }], &[])).unwrap();
        let good = root.join("good.pptx");
        fs::write(&good, pptx_deck()).unwrap();
        let locked_odp = root.join("locked.odp");
        fs::write(&locked_odp, encrypted_open_document("application/vnd.oasis.opendocument.presentation")).unwrap();
        let damaged_odp = root.join("damaged.odp");
        fs::write(&damaged_odp, b"PK\x03\x04 not really a package").unwrap();
        let inputs = [&locked, &damaged, &bomb, &hidden_only, &good, &locked_odp, &damaged_odp];
        let originals = inputs.map(|path| fs::read(path).unwrap());

        let outcomes = convert_paths(&inputs.map(|path| path.as_path()), json!({}));
        let error = outcomes[0].error().unwrap();
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        assert_eq!(error.message, LOCKED);
        assert_eq!(error.pointer, Some(OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
        assert_eq!(outcomes[1].error().unwrap().message, pptx::DAMAGED);
        assert_eq!(outcomes[2].error().unwrap().kind, ErrorKind::LimitExceeded { limit: crate::kit::contracts::LimitKind::Decompression });
        assert_eq!(outcomes[3].error().unwrap().message, "This presentation has nothing to print, so no PDF was made. Its slides are all hidden; choose Include hidden slides to include them.");
        assert!(succeeded(&outcomes[4]), "{:?}", outcomes[4]);
        assert_eq!((outcomes[5].error().unwrap().message.as_str(), &outcomes[5].error().unwrap().pointer), (LOCKED, &error.pointer));
        assert_eq!(outcomes[6].error().unwrap().message, odp::DAMAGED);
        assert_eq!(inputs.map(|path| fs::read(path).unwrap()), originals);
        assert!(["locked.pdf", "damaged.pdf", "bomb.pdf", "hidden.pdf"].iter().all(|name| !root.join(name).exists() && !root.join("locked").join(name).exists()));

        let outcomes = convert_paths(&[&hidden_only], presentation_options(false, true));
        assert_eq!(outcomes[0].detail, "1 page. 1 slide.");
    }

    #[test]
    fn presentation_options_reject_unknown_fields() {
        let root = sandbox("pptx_options");
        let input = root.join("deck.pptx");
        fs::write(&input, pptx_deck()).unwrap();
        let outcomes = convert_paths(&[&input], json!({ "presentations": { "speakerNotes": true, "hiddenSlides": false, "handouts": 2 } }));
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::InvalidInput);
        assert_eq!(names(&root), ["deck.pptx"]);
    }
}
