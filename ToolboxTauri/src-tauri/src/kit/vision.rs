use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use image::GenericImageView;
use lopdf::{dictionary, Document, Object, ObjectId, Stream};

use crate::kit::common::limits::{self, HelperDeadline, StoppableHelper};
use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::{LimitKind, OutcomeDetail, OutcomePointer, SkipCode, SkipReason, ToolError};
use crate::kit::pdf::metadata::{effective_crop_bounds, page_bounds, page_rotation};
use crate::kit::pdf::text_layer::{self, PageText};
use crate::kit::pdf::unicode_font::UnicodeFont;
use crate::kit::pdf::page_ranges::{validate_page_ranges, PageRanges};
use crate::kit::resources;

const OCR_MAX_INPUT_BYTES: u64 = 100 * 1024 * 1024;
const OCR_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;
const OCR_RENDER_DPI: u32 = 200;
const OCR_MAX_SINGLE_PIXELS: u64 = 40_000_000;
const OCR_MAX_TOTAL_PIXELS: u64 = 120_000_000;
const OCR_MAX_RENDERED_BYTES: u64 = 50 * 1024 * 1024;
const OCR_DEADLINE: HelperDeadline = HelperDeadline { per_file: Duration::from_secs(30), per_page: Duration::from_secs(30) };
const OCR_LANGUAGES: &[&str] = &["eng"];
const OCR_TEXT_SUFFIX: &str = "-ocr";
const OCR_PAGE_SEGMENTATION: &str = "3";
const OCR_NO_TEXT: &str = "OCR completed but found no readable text.";
const OCR_PASSWORD_PROTECTED: &str = "It's password-protected. Remove its password in File Security, then run OCR again.";
/// The resource name the invisible text layer's font is registered under on each page.
const OCR_LAYER_FONT: &str = "ToolboxOcr";

#[derive(Debug, Clone, Copy)]
struct OcrPage {
    number: u32,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrMode { SearchablePdf, Text }

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrRequest {
    pub paths: Vec<PathBuf>,
    #[serde(default)] pub page_range: Option<String>,
    pub mode: String,
    #[serde(default)] pub force: bool,
    pub languages: Vec<String>,
    pub output_location: OutputLocation,
}

impl OcrRequest {
    fn page_range(&self) -> Option<&str> { self.page_range.as_deref().filter(|range| !range.trim().is_empty()) }

    pub fn mode(&self) -> Result<OcrMode, String> {
        match self.mode.as_str() {
            "searchablePdf" => Ok(OcrMode::SearchablePdf),
            "text" => Ok(OcrMode::Text),
            other => Err(format!("\"{other}\" is not an OCR output. Choose a searchable PDF or a text file.")),
        }
    }

    fn check_languages(&self) -> Result<(), String> {
        if self.languages.is_empty() { return Err("Choose at least one recognition language.".to_string()); }
        match self.languages.iter().find(|code| !OCR_LANGUAGES.contains(&code.as_str())) {
            Some(code) => Err(format!("\"{code}\" is not a recognition language Toolbox knows.")),
            None => Ok(()),
        }
    }
}

pub fn validate_ocr_request(request: &OcrRequest) -> Result<(), Vec<JobOutcome>> {
    validate_page_ranges(&request.paths, request.page_range())?;
    let Err(error) = request.mode().and_then(|_| request.check_languages()) else { return Ok(()) };
    let error = ToolError::invalid_input(error);
    if request.paths.is_empty() { return Err(vec![JobOutcome::failure(PathBuf::new(), error)]); }
    Err(request.paths.iter().cloned().map(|path| JobOutcome::failure(path, error.clone())).collect())
}

pub fn ocr_pdf(request: &OcrRequest, input: PathBuf) -> JobOutcome {
    ocr_pdf_within(request, input, OCR_DEADLINE)
}

fn ocr_pdf_within(request: &OcrRequest, input: PathBuf, deadline: HelperDeadline) -> JobOutcome {
    match request.mode() {
        Ok(OcrMode::SearchablePdf) => searchable_pdf_within(request, input, deadline),
        Ok(OcrMode::Text) => text_file_within(request, input, deadline),
        Err(error) => JobOutcome::failure(input, ToolError::invalid_input(error)),
    }
}

fn text_file_within(request: &OcrRequest, input: PathBuf, deadline: HelperDeadline) -> JobOutcome {
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, OCR_TEXT_SUFFIX, "txt") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve OCR output: {error}")),
    };
    // A missing engine affects every file the same way, so it is reported before anything is read from the file.
    let resources = match OcrResources::resolve() {
        Ok(resources) => resources,
        Err(error) => return unavailable(input, error.to_string()),
    };
    let input_size = match std::fs::metadata(&input) {
        Ok(metadata) => metadata.len(),
        Err(error) => return failure(input, format!("Could not inspect PDF for OCR: {error}")),
    };
    if input_size > OCR_MAX_INPUT_BYTES { return failure(input, "PDF exceeds the 100 MiB OCR input limit.".to_string()); }
    let document = match load_ocr_document(&input) {
        Ok(document) => document,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let plan = match PagePlan::of(&document, request.page_range(), request.force) {
        Ok(plan) => plan,
        Err(NoPlan::Failed(error)) => return failure(input, error),
        Err(NoPlan::Skipped(reason)) => return JobOutcome::skipped(input, reason),
    };
    let recognized = plan.recognized();
    if recognized.is_empty() { return JobOutcome::skipped(input, plan.text_present()); }
    if let Err(error) = check_render_limits(&document, &recognized) { return failure(input, error); }
    let mut ocr = match run_ocr(&resources, &input, &recognized, &request.languages, &[], deadline) {
        Ok(pages) => pages.into_iter(),
        Err(error) => return JobOutcome::failure(input, error),
    };
    let kept = plan.kept();
    let mut existing = match existing_text(&input, &kept, deadline) {
        Ok(pages) => pages.into_iter(),
        Err(error) => return JobOutcome::failure(input, error),
    };
    // Recognised and kept pages are each in page order, so taking from whichever list
    // the page belongs to puts every page's text back in page order.
    let pages = plan.scope.iter().filter_map(|(_, text)| if plan.recognizes(*text) { ocr.next() } else { existing.next() }).collect::<Vec<_>>();
    let text = match ocr_text(&pages) {
        Ok(text) => text,
        Err(error) => return failure(input, error),
    };
    if text.len() > OCR_MAX_OUTPUT_BYTES { return failure(input, "OCR output exceeds the 10 MiB text limit.".to_string()); }
    if text.is_empty() { return failure(input, OCR_NO_TEXT.to_string()); }
    match std::fs::write(output.path(), text) {
        Ok(_) => match output.publish() {
            Ok(path) => plan.text_file_details(success(input, path, &plan.text_file_detail()), &request.languages, request.page_range()),
            Err(error) => failure(input, format!("Could not publish OCR output: {error}")),
        },
        Err(error) => failure(input, format!("Could not write OCR output: {error}")),
    }
}

fn load_ocr_document(input: &Path) -> Result<Document, ToolError> {
    let document = Document::load(input).map_err(|error| ToolError::processing(format!("Could not inspect PDF for OCR: {error}")))?;
    if document.is_encrypted() {
        return Err(ToolError::invalid_input(OCR_PASSWORD_PROTECTED).with_pointer(OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
    }
    Ok(document)
}

fn recognized_detail(pages: usize) -> String {
    format!("{} recognized. English.", count(pages, "page", "pages"))
}

fn count(number: usize, one: &str, many: &str) -> String { format!("{number} {}", if number == 1 { one } else { many }) }

/// Why a file has no pages to work on: it can't be read, or the range leaves none of its pages.
#[derive(Debug)]
enum NoPlan { Failed(String), Skipped(SkipReason) }

/// The pages in scope, in page order, and the text each one already has.
struct PagePlan {
    scope: Vec<(OcrPage, PageText)>,
    /// Pages of the file outside the range, which a text file leaves out.
    outside: Vec<u32>,
    force: bool,
}

impl PagePlan {
    /// Checks every page in scope for text before anything is recognized. Without
    /// Force any text keeps a page as it is; with Force a page whose only text is an
    /// invisible layer is read again. Visible text is never read again.
    ///
    /// One range applies to every file in a batch, so it is clamped to this file's
    /// length; a file with no pages left is skipped. Pages are always taken in page
    /// order, whatever order the range was typed in.
    fn of(document: &Document, range: Option<&str>, force: bool) -> Result<Self, NoPlan> {
        let pages = document.get_pages();
        if pages.is_empty() { return Err(NoPlan::Failed("PDF has no pages to OCR.".to_string())); }
        let requested = match range {
            Some(range) => {
                let ranges = PageRanges::parse(range).map_err(NoPlan::Failed)?;
                let clamped = ranges.clamp_or_skip(pages.len()).map_err(|skip| NoPlan::Skipped(SkipReason::new(
                    skip.code,
                    format!("No pages left in {ranges}. It has {}, so nothing was made.", count(pages.len(), "page", "pages")),
                )))?;
                Some(clamped.page_indices())
            }
            None => None,
        };
        let mut scope = Vec::new();
        let mut outside = Vec::new();
        for (number, page_id) in pages {
            if requested.as_ref().is_some_and(|indices| indices.binary_search(&(number as usize - 1)).is_err()) {
                outside.push(number);
                continue;
            }
            let text = text_layer::page_text(document, page_id).map_err(|error| NoPlan::Failed(format!("Page {number}'s text could not be checked: {error}")))?;
            scope.push((OcrPage { number }, text));
        }
        if scope.is_empty() { return Err(NoPlan::Failed("Select at least one PDF page for OCR.".to_string())); }
        Ok(Self { scope, outside, force })
    }

    fn recognizes(&self, text: PageText) -> bool { text == PageText::None || (self.force && text == PageText::Invisible) }

    fn recognized(&self) -> Vec<OcrPage> { self.scope.iter().filter(|(_, text)| self.recognizes(*text)).map(|(page, _)| *page).collect() }

    fn kept(&self) -> Vec<OcrPage> { self.scope.iter().filter(|(_, text)| !self.recognizes(*text)).map(|(page, _)| *page).collect() }

    fn numbers(&self, class: PageText) -> Vec<u32> { self.scope.iter().filter(|(_, text)| *text == class).map(|(page, _)| page.number).collect() }

    /// Pages read again that had an earlier invisible layer, which the new one replaces.
    fn replaced(&self) -> Vec<u32> { if self.force { self.numbers(PageText::Invisible) } else { Vec::new() } }

    /// Pages kept as they are because of an earlier invisible layer.
    fn kept_invisible(&self) -> Vec<u32> { if self.force { Vec::new() } else { self.numbers(PageText::Invisible) } }

    /// Why nothing was made when every page in scope keeps its text.
    fn text_present(&self) -> SkipReason {
        let (visible, invisible) = (self.numbers(PageText::Visible), self.kept_invisible());
        let classes = [(visible, "visible"), (invisible.clone(), "from an earlier OCR run")].into_iter()
            .filter(|(pages, _)| !pages.is_empty())
            .map(|(pages, class)| format!("{} {class}", page_list(&pages).to_lowercase()))
            .collect::<Vec<_>>();
        let mut message = format!("Every page already has text: {}. Nothing was made.", classes.join(", "));
        if !invisible.is_empty() { message.push_str(" Turn on Force to replace the earlier OCR text."); }
        SkipReason::new(SkipCode::TextAlreadyPresent, message)
    }

    /// "5 of 9 pages recognized, 4 skipped for their text." when pages kept their text.
    fn searchable_detail(&self) -> String {
        let (recognized, kept, replaced) = (self.recognized().len(), self.kept().len(), self.replaced().len());
        if kept > 0 {
            format!("{recognized} of {} recognized, {kept} skipped for {} text.", count(self.scope.len(), "page", "pages"), if kept == 1 { "its" } else { "their" })
        } else if replaced > 0 {
            format!("{} recognized, {replaced} with earlier OCR text replaced.", count(recognized, "page", "pages"))
        } else {
            recognized_detail(recognized)
        }
    }

    /// "5 pages recognized, 3 pages of text added as they are" when pages kept their text.
    fn text_file_detail(&self) -> String {
        let (recognized, kept, replaced) = (self.recognized().len(), self.kept().len(), self.replaced().len());
        if kept > 0 {
            format!("{} recognized, {}", count(recognized, "page", "pages"), count(kept, "page of text added as it is", "pages of text added as they are"))
        } else if replaced > 0 {
            format!("{} recognized, {replaced} with earlier OCR text replaced.", count(recognized, "page", "pages"))
        } else {
            recognized_detail(recognized)
        }
    }

    /// The page breakdown, only when some page kept its text or had it replaced.
    fn details(&self, kept_rows: [(&str, Vec<u32>, &str); 2], outside: Option<(String, Vec<u32>)>, languages: &[String]) -> Vec<OutcomeDetail> {
        if self.kept().is_empty() && self.replaced().is_empty() { return Vec::new(); }
        let row = |label: &str, pages: &[u32], after: &str| (!pages.is_empty()).then(|| OutcomeDetail { label: label.to_string(), value: format!("{}{after}", page_list(pages)) });
        let [first, second] = kept_rows;
        [
            row("Recognized", &self.numbers(PageText::None), ""),
            row("Earlier OCR text replaced", &self.replaced(), ""),
            row(first.0, &first.1, first.2),
            row(second.0, &second.1, second.2),
            outside.and_then(|(label, pages)| row(&label, &pages, "")),
            Some(OutcomeDetail { label: "Languages".to_string(), value: language_names(languages) }),
        ].into_iter().flatten().collect()
    }

    fn searchable_details(&self, outcome: JobOutcome, languages: &[String]) -> JobOutcome {
        let details = self.details([
            ("Skipped, visible text", self.numbers(PageText::Visible), ""),
            ("Skipped, earlier OCR text", self.kept_invisible(), ". Turn on Force to replace it."),
        ], None, languages);
        outcome.with_page_details(details)
    }

    fn text_file_details(&self, outcome: JobOutcome, languages: &[String], range: Option<&str>) -> JobOutcome {
        let outside = range.map(|range| (format!("Outside {}, left out", range.trim()), self.outside.clone()));
        let details = self.details([
            ("Visible text, added as it is", self.numbers(PageText::Visible), ""),
            ("Earlier OCR text, added as it is", self.kept_invisible(), ""),
        ], outside, languages);
        outcome.with_page_details(details)
    }
}

/// "Page 8", "Pages 1-4 and 9", "Pages 1, 3-5 and 9". Pages are in page order.
fn page_list(pages: &[u32]) -> String {
    let mut spans: Vec<(u32, u32)> = Vec::new();
    for &page in pages {
        match spans.last_mut() {
            Some((_, last)) if *last + 1 == page => *last = page,
            _ => spans.push((page, page)),
        }
    }
    // Two pages in a row read as "2 and 3"; three or more as "5-7".
    let spans = spans.into_iter().flat_map(|(first, last)| match last - first {
        0 => vec![first.to_string()],
        1 => vec![first.to_string(), last.to_string()],
        _ => vec![format!("{first}-{last}")],
    }).collect::<Vec<_>>();
    let joined = match spans.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    };
    format!("{} {joined}", if pages.len() == 1 { "Page" } else { "Pages" })
}

fn language_names(languages: &[String]) -> String {
    languages.iter().map(|code| match code.as_str() { "eng" => "English", other => other }).collect::<Vec<_>>().join(", ")
}

/// The text a kept page already has, read with the bundled text reader one page at a
/// time, in page order. It is normalized the same way as recognised text.
fn existing_text(input: &Path, pages: &[OcrPage], deadline: HelperDeadline) -> Result<Vec<Vec<u8>>, ToolError> {
    if pages.is_empty() { return Ok(Vec::new()); }
    let reader = resources::resolve_helper(&resources::PDF_TEXT).map_err(|_| ToolError::unavailable(OcrComponent::TextReader.to_string()))?;
    let limit = deadline.limit(1, pages.len());
    let stop_at = Instant::now() + limit;
    let timed_out = format!("Reading the existing text took longer than {} and was stopped. Nothing was saved.", limits::duration_label(limit));
    let mut texts = Vec::with_capacity(pages.len());
    let mut total = 0usize;
    for page in pages {
        let mut command = Command::new(&reader);
        command.args(["-enc", "UTF-8", "-f"]).arg(page.number.to_string()).arg("-l").arg(page.number.to_string()).arg(input).arg("-");
        let result = run_ocr_helper(command, stop_at, &timed_out)?;
        if !result.status.success() { return Err(ToolError::processing(stderr(result, "The PDF text reader could not read the existing text."))); }
        total = total.saturating_add(result.stdout.len()).saturating_add(1);
        if total > OCR_MAX_OUTPUT_BYTES { return Err(ToolError::processing("OCR output exceeds the 10 MiB text limit.")); }
        texts.push(result.stdout);
    }
    Ok(texts)
}

/// Only the pages that are rendered count toward the render limits.
fn check_render_limits(document: &Document, pages: &[OcrPage]) -> Result<(), String> {
    let page_ids = document.get_pages();
    let mut total_pixels = 0u64;
    for page in pages {
        let id = *page_ids.get(&page.number).ok_or_else(|| "PDF page is missing.".to_string())?;
        let (left, bottom, right, top) = page_bounds(document, id)?;
        let pixels = rendered_pixels(right - left, top - bottom)?;
        if pixels > OCR_MAX_SINGLE_PIXELS {
            return Err("Selected PDF page exceeds the 40 million pixel OCR limit.".to_string());
        }
        total_pixels = total_pixels.checked_add(pixels).ok_or_else(|| "Selected PDF pages exceed the OCR pixel limit.".to_string())?;
        if total_pixels > OCR_MAX_TOTAL_PIXELS {
            return Err("Selected PDF pages exceed the 120 million pixel OCR limit.".to_string());
        }
    }
    Ok(())
}

fn rendered_pixels(width: f32, height: f32) -> Result<u64, String> {
    let width = (width / 72.0 * OCR_RENDER_DPI as f32).ceil();
    let height = (height / 72.0 * OCR_RENDER_DPI as f32).ceil();
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("Selected PDF page has invalid OCR dimensions.".to_string());
    }
    (width as u64).checked_mul(height as u64).ok_or_else(|| "Selected PDF pages exceed the OCR pixel limit.".to_string())
}

struct OcrWorkspace(PathBuf);

impl OcrWorkspace {
    fn new() -> Result<Self, String> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        for attempt in 0..100 {
            let path = std::env::temp_dir().join(format!("toolbox_ocr_{}_{}_{}", std::process::id(), timestamp, attempt));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("Could not create private OCR workspace: {error}")),
            }
        }
        Err("Could not allocate a unique private OCR workspace.".to_string())
    }

    fn page_prefix(&self, page: u32) -> PathBuf { self.0.join(format!("page-{page}")) }
}

impl Drop for OcrWorkspace {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

struct OcrResources {
    engine: PathBuf,
    tessdata: PathBuf,
    renderer: PathBuf,
}

/// The OCR pieces a file needs. Each one names itself when it is missing, so the
/// person knows reinstalling is the fix rather than trying another file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OcrComponent { Engine, Renderer, EnglishData, TextReader }

impl std::fmt::Display for OcrComponent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Engine => "The text recognition engine is missing. Reinstall Toolbox.",
            Self::Renderer => "The PDF page renderer is missing. Reinstall Toolbox.",
            Self::EnglishData => "The English recognition data is missing. Reinstall Toolbox.",
            Self::TextReader => "The PDF text reader is missing. Reinstall Toolbox.",
        })
    }
}

impl OcrResources {
    fn resolve() -> Result<Self, OcrComponent> {
        let root = ocr_resource_root();
        Ok(Self {
            engine: resources::resolve("tesseract", &root, "TOOLBOX_TESSERACT_PATH", "tesseract").map_err(|_| OcrComponent::Engine)?.path,
            tessdata: find_resource("engTraineddata", "TOOLBOX_TESSDATA_PATH", &root).map_err(|_| OcrComponent::EnglishData)?,
            renderer: find_pdf_renderer().map_err(|_| OcrComponent::Renderer)?,
        })
    }
}

/// Recognises each page on its own, so the text keeps page order and page breaks.
/// The engine runs on one thread with a fixed segmentation mode, resolution and
/// language data, so the same file gives the same text every time.
/// `settings` are extra engine arguments, such as `WORD_BOX_OUTPUT`; none gives plain text.
fn run_ocr(resources: &OcrResources, input: &Path, pages: &[OcrPage], languages: &[String], settings: &[&str], deadline: HelperDeadline) -> Result<Vec<Vec<u8>>, ToolError> {
    let limit = deadline.limit(1, pages.len());
    let stop_at = Instant::now() + limit;
    let timed_out = format!("Recognising took longer than {} and was stopped. Nothing was saved.", limits::duration_label(limit));
    let workspace = OcrWorkspace::new().map_err(ToolError::processing)?;
    let tessdata_dir = resources.tessdata.parent().ok_or_else(|| ToolError::processing("OCR language data has no parent directory."))?;
    let mut recognized = Vec::with_capacity(pages.len());
    let mut total = 0usize;
    for page in pages {
        let image = render_ocr_page(&resources.renderer, input, &workspace, *page, stop_at, &timed_out)?;
        let mut command = Command::new(&resources.engine);
        command.env("OMP_THREAD_LIMIT", "1")
            .arg(&image).arg("stdout")
            .arg("--tessdata-dir").arg(tessdata_dir)
            .arg("-l").arg(languages.join("+"))
            .arg("--psm").arg(OCR_PAGE_SEGMENTATION)
            .arg("--dpi").arg(OCR_RENDER_DPI.to_string())
            .args(settings);
        let result = run_ocr_helper(command, stop_at, &timed_out)?;
        if !result.status.success() { return Err(ToolError::processing(stderr(result, "OCR engine failed."))); }
        total = total.saturating_add(result.stdout.len()).saturating_add(1);
        if total > OCR_MAX_OUTPUT_BYTES { return Err(ToolError::processing("OCR output exceeds the 10 MiB text limit.")); }
        recognized.push(result.stdout);
    }
    Ok(recognized)
}

/// Joins recognised pages with a form feed between them. Lines end in LF. A page
/// with no text stays an empty page, so page numbers still line up. When no page
/// has text the result is empty.
fn ocr_text(pages: &[Vec<u8>]) -> Result<String, String> {
    let pages = pages.iter().map(|page| normalize_ocr_text(page)).collect::<Result<Vec<_>, _>>()?;
    if pages.iter().all(String::is_empty) { return Ok(String::new()); }
    Ok(pages.iter().map(|page| if page.is_empty() { String::new() } else { format!("{page}\n") }).collect::<Vec<_>>().join("\u{c}"))
}

/// Makes a copy of the scan whose pages look the same but carry an invisible text
/// layer placed from the engine's word boxes, so the text can be selected, copied and
/// searched. The original content streams, resources and page boxes are kept as they
/// are; each recognised page only gains a font and one more content stream.
fn searchable_pdf_within(request: &OcrRequest, input: PathBuf, deadline: HelperDeadline) -> JobOutcome {
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, OCR_TEXT_SUFFIX, "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve OCR output: {error}")),
    };
    let resources = match OcrResources::resolve() {
        Ok(resources) => resources,
        Err(error) => return unavailable(input, error.to_string()),
    };
    let input_size = match std::fs::metadata(&input) {
        Ok(metadata) => metadata.len(),
        Err(error) => return failure(input, format!("Could not inspect PDF for OCR: {error}")),
    };
    if input_size > OCR_MAX_INPUT_BYTES { return failure(input, "PDF exceeds the 100 MiB OCR input limit.".to_string()); }
    let mut document = match load_ocr_document(&input) {
        Ok(document) => document,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let plan = match PagePlan::of(&document, request.page_range(), request.force) {
        Ok(plan) => plan,
        Err(NoPlan::Failed(error)) => return failure(input, error),
        Err(NoPlan::Skipped(reason)) => return JobOutcome::skipped(input, reason),
    };
    let pages = plan.recognized();
    if pages.is_empty() { return JobOutcome::skipped(input, plan.text_present()); }
    let pages = match check_render_limits(&document, &pages).and_then(|()| upright_uncropped_pages(&document, pages)) {
        Ok(pages) => pages,
        Err(error) => return failure(input, error),
    };
    let layouts = match run_ocr(&resources, &input, &pages, &request.languages, WORD_BOX_OUTPUT, deadline)
        .and_then(|recognized| recognized.iter().map(|tsv| word_boxes(tsv)).collect::<Result<Vec<_>, _>>().map_err(ToolError::processing))
    {
        Ok(layouts) => layouts,
        Err(error) => return JobOutcome::failure(input, error),
    };
    if layouts.iter().all(|layout| layout.lines.is_empty()) { return failure(input, OCR_NO_TEXT.to_string()); }
    let page_ids = document.get_pages();
    // An earlier invisible layer goes before the new one is added, so each word is
    // found once. Visible content and every other page stay byte for byte as they were,
    // which the check below holds the output to.
    for number in plan.replaced() {
        let Some(id) = page_ids.get(&number) else { continue };
        if let Err(error) = text_layer::remove_invisible_text(&mut document, *id) {
            return failure(input, format!("Page {number}'s earlier OCR text couldn't be replaced: {error}"));
        }
    }
    let original = document.clone();
    let signature_removed = match crate::kit::pdf::remove_signatures(&mut document) {
        Ok(removed) => removed,
        Err(error) => return failure(input, error),
    };
    let layered = pages.iter().zip(layouts).filter_map(|(page, layout)| page_ids.get(&page.number).map(|id| (page.number, *id, layout))).collect::<Vec<_>>();
    let expected = match crate::kit::images::tools::bundled_text_font_bytes().map_err(ToolError::processing).and_then(|font| add_text_layer(&mut document, &layered, font)) {
        Ok(expected) => expected,
        Err(error) => return JobOutcome::failure(input, error),
    };
    if let Err(error) = document.save(output.path()) { return failure(input, format!("Could not write OCR output: {error}")); }
    #[cfg(test)]
    if BREAK_SEARCHABLE_OUTPUT_FOR.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_deref() == Some(input.as_path()) {
        let _ = std::fs::write(output.path(), b"%PDF-1.7\nnot a PDF any more\n");
    }
    if let Err(error) = validate_searchable_pdf(&original, output.path(), &expected) {
        return failure(input, format!("The searchable PDF didn't pass its check, so it wasn't saved: {error}"));
    }
    match output.publish() {
        Ok(path) => crate::kit::pdf::note_signature_removed(plan.searchable_details(success(input, path, &plan.searchable_detail()), &request.languages), signature_removed),
        Err(error) => failure(input, format!("Could not publish OCR output: {error}")),
    }
}

/// Asks the engine for word boxes as TSV through settings rather than its `tsv` config
/// file, because only the language data ships with the app, not the engine's configs.
const WORD_BOX_OUTPUT: &[&str] = &["-c", "tessedit_create_tsv=1", "-c", "tessedit_create_txt=0"];

/// Test seam: the searchable PDF written for this input is damaged after it is saved,
/// so tests can see reopen validation keep it from being published.
#[cfg(test)]
pub(crate) static BREAK_SEARCHABLE_OUTPUT_FOR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Rotated and cropped pages render to a picture that no longer lines up with the
/// MediaBox, so their word boxes would land in the wrong place. They come later.
fn upright_uncropped_pages(document: &Document, pages: Vec<OcrPage>) -> Result<Vec<OcrPage>, String> {
    let page_ids = document.get_pages();
    for page in &pages {
        let id = *page_ids.get(&page.number).ok_or_else(|| "PDF page is missing.".to_string())?;
        let (left, bottom, right, top) = page_bounds(document, id)?;
        if page_rotation(document, id) != 0 || effective_crop_bounds(document, id)? != [left, bottom, right, top] {
            return Err(format!("Page {} is rotated or cropped. Searchable PDFs support upright, uncropped pages for now. Save a text file instead.", page.number));
        }
    }
    Ok(pages)
}

#[derive(Debug, Clone, PartialEq)]
struct OcrWord { text: String, left: f32, width: f32 }

#[derive(Debug, Clone, PartialEq)]
struct OcrLine { top: f32, bottom: f32, words: Vec<OcrWord> }

/// Recognised words on one rendered page, in reading order, in raster pixels.
#[derive(Debug, Clone, PartialEq)]
struct OcrLayout { width: f32, height: f32, lines: Vec<OcrLine> }

/// Reads the engine's TSV: the page row gives the raster size, line rows give each
/// line's height, and word rows give each word and its box.
fn word_boxes(tsv: &[u8]) -> Result<OcrLayout, String> {
    let tsv = std::str::from_utf8(tsv).map_err(|_| "OCR helper produced invalid UTF-8 text.".to_string())?;
    let mut size = None;
    let mut lines: Vec<((u32, u32, u32), OcrLine)> = Vec::new();
    for row in tsv.lines().map(|row| row.trim_end_matches('\r')).filter(|row| !row.is_empty() && !row.starts_with("level")) {
        let fields = row.splitn(12, '\t').collect::<Vec<_>>();
        if fields.len() < 11 { return Err("OCR helper produced unreadable word boxes.".to_string()); }
        let number = |index: usize| fields[index].trim().parse::<f32>().map_err(|_| "OCR helper produced unreadable word boxes.".to_string());
        let key = |index: usize| fields[index].trim().parse::<u32>().unwrap_or(0);
        let (level, left, top, width, height) = (number(0)? as u32, number(6)?, number(7)?, number(8)?, number(9)?);
        let line_key = (key(2), key(3), key(4));
        match level {
            1 => size = Some((width, height)),
            4 => lines.push((line_key, OcrLine { top, bottom: top + height, words: Vec::new() })),
            5 => {
                let text = fields.get(11).map(|text| text.trim()).unwrap_or_default();
                if text.is_empty() || width <= 0. || height <= 0. { continue; }
                let word = OcrWord { text: text.to_string(), left, width };
                match lines.iter_mut().rev().find(|(key, _)| *key == line_key) {
                    Some((_, line)) => {
                        line.top = line.top.min(top);
                        line.bottom = line.bottom.max(top + height);
                        line.words.push(word);
                    }
                    None => lines.push((line_key, OcrLine { top, bottom: top + height, words: vec![word] })),
                }
            }
            _ => {}
        }
    }
    let lines = lines.into_iter().map(|(_, line)| line).filter(|line| !line.words.is_empty() && line.bottom > line.top).collect::<Vec<_>>();
    match size {
        Some((width, height)) if width > 0. && height > 0. => Ok(OcrLayout { width, height, lines }),
        _ if lines.is_empty() => Ok(OcrLayout { width: 1., height: 1., lines }),
        _ => Err("OCR helper reported words without a page size.".to_string()),
    }
}

/// What the text layer on one page must give back when the output is reopened.
struct ExpectedLayer { page: u32, words: Vec<String> }

/// Writes each page's words in invisible text (rendering mode 3) with an embedded,
/// subset Unicode font. A word starts at its box's left edge on its line's baseline and
/// is stretched to its box's width, so selecting it highlights the word in the scan.
/// The page's own content runs first inside q/Q, so its graphics state can't move the
/// text. Numbers have fixed precision, so the same words always give the same bytes.
fn add_text_layer(document: &mut Document, pages: &[(u32, ObjectId, OcrLayout)], font: Vec<u8>) -> Result<Vec<ExpectedLayer>, ToolError> {
    let mut font = UnicodeFont::parse(font)?;
    let (ascent, descent) = font.vertical_metrics();
    let mut layers = Vec::new();
    for (number, page_id, layout) in pages {
        if layout.lines.is_empty() { continue; }
        let (left, bottom, right, top) = page_bounds(document, *page_id).map_err(ToolError::processing)?;
        let (scale_x, scale_y) = ((right - left) / layout.width, (top - bottom) / layout.height);
        let name = layer_font_name(document, *page_id).map_err(ToolError::processing)?;
        let mut content = String::new();
        let mut words = Vec::new();
        for line in &layout.lines {
            let size = (line.bottom - line.top) * scale_y / (ascent - descent);
            let baseline = top - line.bottom * scale_y - descent * size;
            content.push_str(&format!("BT\n3 Tr\n/{name} {size:.2} Tf\n"));
            for word in &line.words {
                let natural = font.show_unshaped(&word.text, size)?.width;
                let run = font.show_unshaped(&format!("{} ", word.text), size)?;
                let stretch = if natural > 0. { 100. * word.width * scale_x / natural } else { 100. };
                content.push_str(&format!("{stretch:.2} Tz 1 0 0 1 {:.2} {baseline:.2} Tm {}\n", left + word.left * scale_x, run.operators));
                words.push(word.text.clone());
            }
            content.push_str("ET\n");
        }
        layers.push((*page_id, name, content, ExpectedLayer { page: *number, words }));
    }
    let font_id = font.embed(document)?;
    for (page_id, name, content, _) in &layers {
        attach_text_layer(document, *page_id, name, font_id, content).map_err(ToolError::processing)?;
    }
    Ok(layers.into_iter().map(|(_, _, _, expected)| expected).collect())
}

/// The page's resources as the page sees them, inherited or its own.
fn page_resources(document: &Document, page_id: ObjectId) -> Result<(lopdf::Dictionary, lopdf::Dictionary), String> {
    let resources = text_layer::page_resources(document, page_id)?;
    let fonts = match resources.get(b"Font") {
        Ok(fonts) => crate::kit::pdf::resolve(document, fonts)?.as_dict().map_err(|_| "PDF page fonts are invalid.".to_string())?.clone(),
        Err(_) => lopdf::Dictionary::new(),
    };
    Ok((resources, fonts))
}

fn layer_font_name(document: &Document, page_id: ObjectId) -> Result<String, String> {
    let (_, fonts) = page_resources(document, page_id)?;
    (0..).map(|index| if index == 0 { OCR_LAYER_FONT.to_string() } else { format!("{OCR_LAYER_FONT}{index}") })
        .find(|name| !fonts.has(name.as_bytes()))
        .ok_or_else(|| "PDF page has no free font name.".to_string())
}

/// Gives the page its own copy of its resources with the layer font added, so
/// resources shared with other pages stay as they were, then wraps the original
/// content streams, untouched, between a q stream and the text layer.
fn attach_text_layer(document: &mut Document, page_id: ObjectId, name: &str, font_id: ObjectId, content: &str) -> Result<(), String> {
    let (mut resources, mut fonts) = page_resources(document, page_id)?;
    fonts.set(name.as_bytes().to_vec(), Object::Reference(font_id));
    resources.set("Font", fonts);
    let original = document.get_page_contents(page_id);
    let open = document.add_object(Stream::new(dictionary! {}, b"q\n".to_vec()));
    let mut layer = Stream::new(dictionary! {}, format!("\nQ\n{content}").into_bytes());
    layer.compress().map_err(|error| format!("Could not compress the OCR text layer: {error}"))?;
    let layer = document.add_object(layer);
    let contents = std::iter::once(open).chain(original).chain(std::iter::once(layer)).map(Object::Reference).collect::<Vec<_>>();
    let page = document.get_dictionary_mut(page_id).map_err(|error| error.to_string())?;
    page.set("Resources", resources);
    page.set("Contents", contents);
    Ok(())
}

/// Reopens the written PDF and checks it before anyone can see it: the same pages,
/// the same boxes, every original content stream byte for byte, and a text layer that
/// gives back the recognized words through text extraction.
fn validate_searchable_pdf(original: &Document, output: &Path, expected: &[ExpectedLayer]) -> Result<(), String> {
    let reopened = Document::load(output).map_err(|error| format!("it could not be reopened ({error})."))?;
    let (before, after) = (original.get_pages(), reopened.get_pages());
    if before.len() != after.len() { return Err("its page count changed.".to_string()); }
    let streams = |document: &Document, page: ObjectId| document.get_page_contents(page).into_iter()
        .map(|id| document.get_object(id).and_then(Object::as_stream).map(|stream| stream.content.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("its content could not be read ({error})."));
    for ((number, old), new) in before.iter().zip(after.values()) {
        if page_bounds(original, *old)? != page_bounds(&reopened, *new)? || effective_crop_bounds(original, *old)? != effective_crop_bounds(&reopened, *new)? {
            return Err(format!("page {number}'s boxes changed."));
        }
        let (old_streams, new_streams) = (streams(original, *old)?, streams(&reopened, *new)?);
        if !old_streams.is_empty() && !new_streams.windows(old_streams.len()).any(|window| window == old_streams.as_slice()) {
            return Err(format!("page {number}'s original content changed."));
        }
    }
    for layer in expected {
        let text = reopened.extract_text(&[layer.page]).map_err(|error| format!("page {}'s text could not be read ({error}).", layer.page))?;
        if !text.split_whitespace().collect::<Vec<_>>().join(" ").contains(&layer.words.join(" ")) {
            return Err(format!("page {}'s text layer doesn't give back the recognized words.", layer.page));
        }
    }
    Ok(())
}

fn render_ocr_page(renderer: &Path, input: &Path, workspace: &OcrWorkspace, page: OcrPage, deadline: Instant, timed_out: &str) -> Result<PathBuf, ToolError> {
    let prefix = workspace.page_prefix(page.number);
    let output = prefix.with_extension("png");
    let mut command = Command::new(renderer);
    command.arg("-png").arg("-r").arg(OCR_RENDER_DPI.to_string()).arg("-f").arg(page.number.to_string()).arg("-l").arg(page.number.to_string()).arg("-singlefile").arg(input).arg(&prefix);
    let result = run_ocr_helper(command, deadline, timed_out)?;
    if result.status.success() && output.is_file() {
        let size = std::fs::metadata(&output).map_err(|error| ToolError::processing(format!("Could not inspect rendered OCR page: {error}")))?.len();
        if size > OCR_MAX_RENDERED_BYTES {
            return Err(ToolError::processing("Rendered OCR page exceeds the 50 MiB file-size limit."));
        }
        Ok(output)
    } else {
        Err(ToolError::processing(stderr(result, "PDF renderer failed to render the selected OCR page.")))
    }
}

fn run_ocr_helper(command: Command, deadline: Instant, timed_out: &str) -> Result<std::process::Output, ToolError> {
    match crate::kit::pdf::run_bounded_helper(command, deadline, OCR_MAX_OUTPUT_BYTES, crate::kit::pdf::PDF_TEXT_MAX_ERROR_BYTES, timed_out) {
        Ok(output) => Ok(output),
        Err(message) if message == timed_out => Err(ToolError::limit_exceeded(LimitKind::Time, message)),
        Err(message) => Err(ToolError::processing(message)),
    }
}

fn find_pdf_renderer() -> Result<PathBuf, String> {
    find_pdf_renderer_with_override(std::env::var_os("TOOLBOX_PDFTOPPM_PATH").map(PathBuf::from), || {
        let root = crate::kit::resources::application_resource_root().unwrap_or_default();
        let executable = if cfg!(windows) { "pdftoppm.exe" } else { "pdftoppm" };
        [root.join("pdf-bin").join(executable), root.join("resources").join(executable), root.join(executable)]
            .into_iter()
            .find(|path| path.is_file())
            .or_else(|| Command::new(executable).arg("-h").output().ok().filter(|result| result.status.success()).map(|_| PathBuf::from(executable)))
            .ok_or_else(|| "PDF rasterizer is unavailable. Bundle pdftoppm or set TOOLBOX_PDFTOPPM_PATH.".to_string())
    })
}

fn find_pdf_renderer_with_override<F>(override_path: Option<PathBuf>, fallback: F) -> Result<PathBuf, String>
where
    F: FnOnce() -> Result<PathBuf, String>,
{
    if let Some(path) = override_path.filter(|path| !path.as_os_str().is_empty()) {
        if path.is_file() { return Ok(path); }
        return Err("TOOLBOX_PDFTOPPM_PATH does not point to a file.".to_string());
    }
    fallback()
}

fn normalize_ocr_text(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "OCR helper produced invalid UTF-8 text.".to_string())?
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(text)
}

pub fn blur_faces(request: &VisionRequest, input: PathBuf) -> JobOutcome {
    run_image_adapter(request, input, "face-blur", "faceBlur", "faceBlurModel", "TOOLBOX_FACE_BLUR_PATH", "TOOLBOX_FACE_BLUR_MODEL_PATH", "toolbox-face-blur", "-blurred", "Blurred faces", ImageAdapterValidation::FaceBlur)
}

pub fn remove_background(request: &VisionRequest, input: PathBuf) -> JobOutcome {
    run_image_adapter(request, input, "background-removal", "backgroundRemoval", "backgroundRemovalModel", "TOOLBOX_BACKGROUND_REMOVAL_PATH", "TOOLBOX_BACKGROUND_REMOVAL_MODEL_PATH", "toolbox-background-removal", "-cutout", "Background removed", ImageAdapterValidation::Cutout)
}

enum ImageAdapterValidation { FaceBlur, Cutout }

fn run_image_adapter(request: &VisionRequest, input: PathBuf, mode: &str, resource_name: &str, model_name: &str, variable: &str, model_variable: &str, command: &str, suffix: &str, detail: &str, validation: ImageAdapterValidation) -> JobOutcome {
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, suffix, "png") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve vision output: {error}")),
    };
    let engine = match find_engine(resource_name, variable, command) {
        Ok(engine) => engine,
        Err(error) => return unavailable(input, error),
    };
    let model = match find_model(model_name, model_variable) {
        Ok(model) => model,
        Err(error) => return unavailable(input, error),
    };
    match Command::new(engine).arg("--mode").arg(mode).arg("--input").arg(&input).arg("--output").arg(output.path()).arg("--model").arg(model).output_until_stopped() {
        Ok(result) if result.status.success() && output.path().is_file() => {
            let validation = match validation {
                ImageAdapterValidation::FaceBlur => validate_face_blur(&input, output.path()),
                ImageAdapterValidation::Cutout => validate_cutout(output.path()),
            };
            match validation {
                Ok(()) => match output.publish() {
                    Ok(path) => success(input, path, detail),
                    Err(error) => failure(input, format!("Could not publish vision output: {error}")),
                },
                Err(error) => failure(input, error),
            }
        }
        Ok(result) if result.status.success() => {
            failure(input, "Vision adapter reported success but produced no output.".to_string())
        }
        Ok(result) => failure(input, stderr(result, "Vision adapter failed.")),
        Err(error) => failure(input, format!("Could not run vision adapter: {error}")),
    }
}

fn find_engine(name: &str, variable: &str, command: &str) -> Result<PathBuf, String> {
    resources::resolve(name, &vision_resource_root(), variable, command).map(|resource| resource.path)
}

fn find_model(name: &str, variable: &str) -> Result<PathBuf, String> {
    let root = vision_resource_root();
    resources::resolve_bundled_or_override(name, &root, variable)
        .and_then(|resource| resource.map(|resource| resource.path).ok_or_else(|| format!("Resource {name} is unavailable.")))
}

fn find_resource(name: &str, variable: &str, root: &std::path::Path) -> Result<PathBuf, String> {
    resources::resolve_bundled_or_override(name, root, variable)
        .and_then(|resource| resource.map(|resource| resource.path).ok_or_else(|| format!("Resource {name} is unavailable.")))
}

fn vision_resource_root() -> std::path::PathBuf {
    let root = resources::application_resource_root().unwrap_or_default();
    let direct = root.join("vision");
    if direct.is_dir() || !root.join("resources").join("vision").is_dir() { direct } else { root.join("resources").join("vision") }
}

fn ocr_resource_root() -> std::path::PathBuf {
    let root = resources::application_resource_root().unwrap_or_default();
    let direct = root.join("ocr");
    if direct.is_dir() || !root.join("resources").join("ocr").is_dir() { direct } else { root.join("resources").join("ocr") }
}

fn validate_cutout(path: &std::path::Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|error| format!("Background adapter output could not be read: {error}"))?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Background adapter must produce a PNG with a transparent subject mask.".to_string());
    }
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .map_err(|error| format!("Background adapter produced an unreadable PNG: {error}"))?
        .to_rgba8();
    let has_visible = image.pixels().any(|pixel| pixel.0[3] > 0);
    let has_soft_edge = image.pixels().any(|pixel| pixel.0[3] < 255);
    if !has_visible || !has_soft_edge { return Err("Background adapter produced no transparent subject mask.".to_string()); }
    Ok(())
}

fn validate_face_blur(input: &std::path::Path, output: &std::path::Path) -> Result<(), String> {
    let input_image = image::open(input).map_err(|error| format!("Face adapter input could not be read: {error}"))?;
    let output_bytes = std::fs::read(output).map_err(|error| format!("Face adapter output could not be read: {error}"))?;
    if !output_bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Face adapter must produce a PNG output.".to_string());
    }
    let output_image = image::load_from_memory_with_format(&output_bytes, image::ImageFormat::Png)
        .map_err(|error| format!("Face adapter produced an unreadable PNG: {error}"))?;
    if input_image.dimensions() != output_image.dimensions() {
        return Err("Face adapter changed the image dimensions.".to_string());
    }
    if input_image.to_rgba8() == output_image.to_rgba8() {
        return Err("Face adapter reported success without detecting or blurring a face.".to_string());
    }
    Ok(())
}

fn success(input_path: PathBuf, output: PathBuf, detail: &str) -> JobOutcome { JobOutcome::succeeded(input_path, vec![output], detail.to_string()) }
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::processing(error)) }
fn unavailable(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::unavailable(error)) }
fn stderr(result: std::process::Output, fallback: &str) -> String { String::from_utf8_lossy(&result.stderr).trim().lines().last().filter(|line| !line.is_empty()).unwrap_or(fallback).to_string() }

#[cfg(test)]
mod tests {
    use super::{normalize_ocr_text, ocr_pdf, ocr_pdf_within, validate_cutout, validate_face_blur, validate_ocr_request, OcrRequest, OCR_DEADLINE};
    use crate::kit::common::limits::HelperDeadline;
    use crate::kit::common::OutputLocation;
    use crate::kit::contracts::{ErrorKind, LimitKind};
    use image::{Rgba, RgbaImage};
    use lopdf::{dictionary, Document, Object, Stream};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;


    fn path(name: &str) -> PathBuf { std::env::temp_dir().join(format!("toolbox_vision_{}_{}", std::process::id(), name)) }

    fn ocr_request(input: &Path, page_range: Option<&str>, output_root: &Path) -> OcrRequest {
        OcrRequest {
            paths: vec![input.to_path_buf()],
            page_range: page_range.map(str::to_string),
            mode: "text".to_string(),
            force: false,
            languages: vec!["eng".to_string()],
            output_location: OutputLocation::CustomFolder(output_root.to_path_buf()),
        }
    }

    /// Scanned pages: something is drawn, but no text.
    fn make_pdf(path: &PathBuf, page_count: usize) {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let mut kids = Vec::new();
        for number in 0..page_count {
            let content_id = document.add_object(Stream::new(dictionary! {}, format!("q 0 0 0 rg 72 720 {} 10 re f Q", number + 1).into_bytes()));
            let page_id = document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } }, "Contents" => content_id,
            });
            kids.push(Object::Reference(page_id));
        }
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => page_count as i64 }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    #[cfg(unix)]
    fn fake_ocr_engine(fail_on_page_two: bool) -> PathBuf {
        let engine = path("fake-ocr.sh");
        let page_two = if fail_on_page_two { "  *page-2.png) printf 'OCR failure\\n' >&2; exit 1 ;;" } else { "  *page-2.png) printf 'page two\\n' ;;" };
        fs::write(&engine, format!("#!/bin/sh\ncase \"$1\" in\n  *page-1.png) printf 'selected page one\\n' ;;\n{page_two}\n  *.pdf) printf 'selected page one\\npage two\\n' ;;\n  *) exit 2 ;;\nesac\n")).unwrap();
        let mut permissions = fs::metadata(&engine).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&engine, permissions).unwrap();
        engine
    }

    #[test]
    fn selected_ocr_pages_preserve_document_order_and_clamp_the_range_to_the_file() {
        let _guard = crate::kit::lock_process_env();
        let input = path("selected-pages-scope.pdf");
        make_pdf(&input, 2);

        let document = Document::load(&input).unwrap();
        let selected = |range| super::PagePlan::of(&document, Some(range), false).map(|plan| plan.recognized().iter().map(|page| page.number).collect::<Vec<_>>());
        assert!(matches!(selected("2, 1"), Ok(pages) if pages == vec![1, 2]));
        assert!(matches!(selected("2-"), Ok(pages) if pages == vec![2]));
        assert!(matches!(selected("2-9"), Ok(pages) if pages == vec![2]), "a range past the end is shortened");
        assert!(matches!(selected("3, 5-"), Err(super::NoPlan::Skipped(reason)) if reason.code == crate::kit::contracts::SkipCode::RangeEmptyAfterClamping && reason.message == "No pages left in 3, 5-. It has 2 pages, so nothing was made."));

        let _ = fs::remove_file(input);
    }

    #[test]
    fn selected_ocr_pages_reject_excessive_render_pixels_before_starting_helpers() {
        let _guard = crate::kit::lock_process_env();
        let input = path("oversized-page.pdf");
        make_pdf(&input, 1);
        let mut document = Document::load(&input).unwrap();
        let page = document.get_pages().values().next().copied().unwrap();
        let root = document.get_dictionary(page).unwrap().get(b"Parent").unwrap().as_reference().unwrap();
        document.get_dictionary_mut(page).unwrap().remove(b"MediaBox");
        document.get_dictionary_mut(root).unwrap().set("MediaBox", vec![0.into(), 0.into(), 14400.into(), 14400.into()]);
        document.save(&input).unwrap();

        let document = Document::load(&input).unwrap();
        let error = super::check_render_limits(&document, &super::PagePlan::of(&document, None, false).unwrap().recognized()).unwrap_err();
        assert!(error.contains("pixel"), "{error}");
        let _ = fs::remove_file(input);
    }

    #[cfg(unix)]
    #[test]
    fn oversized_rendered_page_is_rejected_before_ocr_runs() {
        // Holds the lock because sibling tests count private OCR workspaces.
        let _guard = crate::kit::lock_process_env();
        let renderer = path("oversized-renderer.sh");
        let engine = path("oversized-engine.sh");
        let input = path("oversized-render.pdf");
        make_pdf(&input, 1);
        fs::write(&renderer, format!("#!/bin/sh\ndd if=/dev/zero of=\"${{10}}.png\" bs=1 count=0 seek={} 2>/dev/null\n", super::OCR_MAX_RENDERED_BYTES + 1)).unwrap();
        fs::write(&engine, "#!/bin/sh\nprintf 'ocr should not run\\n'\n").unwrap();
        for helper in [&renderer, &engine] {
            let mut permissions = fs::metadata(helper).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(helper, permissions).unwrap();
        }
        let workspace = super::OcrWorkspace::new().unwrap();
        let result = super::render_ocr_page(&renderer, &input, &workspace, super::OcrPage { number: 1 }, Instant::now() + Duration::from_secs(5), "timed out");
        let error = result.unwrap_err();
        assert!(error.message.contains("file-size limit"), "{error}");

        drop(workspace);
        let _ = fs::remove_file(renderer);
        let _ = fs::remove_file(engine);
        let _ = fs::remove_file(input);
    }

    #[cfg(unix)]
    #[test]
    fn selected_ocr_page_cannot_process_an_excluded_page() {
        let _guard = crate::kit::lock_process_env();
        let input = path("selected-pages.pdf");
        let output_root = path("selected-pages-output");
        make_pdf(&input, 2);
        let original = fs::read(&input).unwrap();
        fs::create_dir_all(&output_root).unwrap();
        let engine = fake_ocr_engine(false);
        let tessdata = path("selected-eng.traineddata");
        fs::write(&tessdata, b"fake tessdata").unwrap();
        let old_engine = std::env::var_os("TOOLBOX_TESSERACT_PATH");
        let old_tessdata = std::env::var_os("TOOLBOX_TESSDATA_PATH");
        std::env::set_var("TOOLBOX_TESSERACT_PATH", &engine);
        std::env::set_var("TOOLBOX_TESSDATA_PATH", &tessdata);

        let outcome = ocr_pdf(&ocr_request(&input, Some("1"), &output_root), input.clone());
        let text = outcome.output_paths().first().and_then(|path| fs::read_to_string(path).ok());
        let output_path = outcome.output_paths().first().cloned();
        let input_after = fs::read(&input).unwrap();

        match old_engine {
            Some(value) => std::env::set_var("TOOLBOX_TESSERACT_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSERACT_PATH"),
        }
        match old_tessdata {
            Some(value) => std::env::set_var("TOOLBOX_TESSDATA_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSDATA_PATH"),
        }
        let _ = fs::remove_file(&engine);
        let _ = fs::remove_file(&tessdata);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_dir_all(&output_root);

        assert!(outcome.error().is_none(), "selected OCR failed: {:?}", outcome.error());
        assert_eq!(text.as_deref(), Some("selected page one\n"));
        assert_eq!(output_path.as_deref().and_then(Path::parent), Some(output_root.as_path()));
        assert_eq!(input_after, original);
    }

    #[cfg(unix)]
    #[test]
    fn failed_selected_ocr_cleans_private_workspace_and_reserved_output() {
        let _guard = crate::kit::lock_process_env();
        let input = path("failed-selected-pages.pdf");
        let output_root = path("failed-selected-pages-output");
        make_pdf(&input, 2);
        let original = fs::read(&input).unwrap();
        fs::create_dir_all(&output_root).unwrap();
        let engine = fake_ocr_engine(true);
        let tessdata = path("failed-eng.traineddata");
        fs::write(&tessdata, b"fake tessdata").unwrap();
        let before_workspaces = ocr_workspaces();
        let old_engine = std::env::var_os("TOOLBOX_TESSERACT_PATH");
        let old_tessdata = std::env::var_os("TOOLBOX_TESSDATA_PATH");
        std::env::set_var("TOOLBOX_TESSERACT_PATH", &engine);
        std::env::set_var("TOOLBOX_TESSDATA_PATH", &tessdata);

        let outcome = ocr_pdf(&ocr_request(&input, Some("2"), &output_root), input.clone());
        let after_workspaces = ocr_workspaces();
        let input_after = fs::read(&input).unwrap();
        let output_entries = fs::read_dir(&output_root).unwrap().count();

        match old_engine {
            Some(value) => std::env::set_var("TOOLBOX_TESSERACT_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSERACT_PATH"),
        }
        match old_tessdata {
            Some(value) => std::env::set_var("TOOLBOX_TESSDATA_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSDATA_PATH"),
        }
        let _ = fs::remove_file(&engine);
        let _ = fs::remove_file(&tessdata);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_dir_all(&output_root);

        assert!(outcome.error().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::Processing)));
        assert!(outcome.output_paths().is_empty());
        assert_eq!(before_workspaces, after_workspaces);
        assert_eq!(original, input_after);
        assert_eq!(output_entries, 0);
    }

    #[cfg(unix)]
    #[test]
    fn ocr_workspace_is_private_and_removed_on_drop() {
        let _guard = crate::kit::lock_process_env();
        let workspace = super::OcrWorkspace::new().unwrap();
        let path = workspace.0.clone();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o700);
        fs::write(path.join("secret.png"), b"private OCR input").unwrap();
        drop(workspace);
        assert!(!path.exists());
    }

    #[test]
    fn empty_pdf_renderer_override_uses_path_fallback() {
        let result = super::find_pdf_renderer_with_override(Some(PathBuf::new()), || Ok(PathBuf::from("pdftoppm")));
        assert_eq!(result.unwrap(), PathBuf::from("pdftoppm"));
    }

    #[cfg(unix)]
    fn ocr_workspaces() -> Vec<PathBuf> {
        let prefix = format!("toolbox_ocr_{}", std::process::id());
        let mut paths = fs::read_dir(std::env::temp_dir()).unwrap().filter_map(Result::ok).map(|entry| entry.path()).filter(|path| {
            path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(&prefix))
        }).collect::<Vec<_>>();
        paths.sort();
        paths
    }

    #[test]
    fn accepts_cutouts_with_soft_transparency() {
        let output = path("soft.png");
        let mut image = RgbaImage::from_pixel(2, 1, Rgba([20, 30, 40, 255]));
        image.put_pixel(1, 0, Rgba([20, 30, 40, 120]));
        image.save(&output).unwrap();
        assert!(validate_cutout(&output).is_ok());
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn rejects_opaque_adapter_outputs() {
        let output = path("opaque.png");
        RgbaImage::from_pixel(2, 1, Rgba([20, 30, 40, 255])).save(&output).unwrap();
        assert!(validate_cutout(&output).is_err());
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn rejects_identity_face_blur_outputs() {
        let input = path("face-input.png");
        let output = path("face-output.png");
        let image = RgbaImage::from_pixel(2, 1, Rgba([20, 30, 40, 255]));
        image.save(&input).unwrap();
        image.save(&output).unwrap();
        assert!(validate_face_blur(&input, &output).is_err());
        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn normalizes_ocr_lines_without_reordering_them() {
        assert_eq!(normalize_ocr_text(b"  first   line\r\n\n second line  \n").unwrap(), "first line\nsecond line");
    }

    #[test]
    fn ocr_text_ends_lines_in_lf_and_separates_pages_with_a_form_feed() {
        let pages = [b"first  page\r\nline two\r\n".to_vec(), b" \n\n".to_vec(), b"third\n".to_vec()];
        assert_eq!(super::ocr_text(&pages).unwrap(), "first page\nline two\n\u{c}\u{c}third\n");
        assert_eq!(super::ocr_text(&[b"\n".to_vec(), b"  \x0c".to_vec()]).unwrap(), "");
    }

    #[test]
    fn page_lists_name_runs_of_pages_the_way_the_results_read() {
        assert_eq!(super::page_list(&[8]), "Page 8");
        assert_eq!(super::page_list(&[1, 2, 3, 4, 9]), "Pages 1-4 and 9");
        assert_eq!(super::page_list(&[1, 3, 4, 5, 9]), "Pages 1, 3-5 and 9");
        assert_eq!(super::page_list(&[5, 6, 7]), "Pages 5-7");
        assert_eq!(super::page_list(&[2, 3]), "Pages 2 and 3");
        assert_eq!(super::page_list(&[1, 2, 5, 6]), "Pages 1, 2, 5 and 6");
    }

    #[test]
    fn missing_ocr_components_name_themselves() {
        assert_eq!(super::OcrComponent::Engine.to_string(), "The text recognition engine is missing. Reinstall Toolbox.");
        assert_eq!(super::OcrComponent::Renderer.to_string(), "The PDF page renderer is missing. Reinstall Toolbox.");
        assert_eq!(super::OcrComponent::EnglishData.to_string(), "The English recognition data is missing. Reinstall Toolbox.");
        assert_eq!(super::OcrComponent::TextReader.to_string(), "The PDF text reader is missing. Reinstall Toolbox.");
    }

    #[test]
    fn rejects_invalid_utf8_ocr_output() {
        assert!(normalize_ocr_text(b"valid\xfftext").is_err());
    }

    #[test]
    fn ocr_deadline_is_thirty_seconds_per_file_plus_thirty_seconds_per_page() {
        assert_eq!(OCR_DEADLINE, HelperDeadline { per_file: Duration::from_secs(30), per_page: Duration::from_secs(30) });
        assert_eq!(OCR_DEADLINE.limit(1, 1), Duration::from_secs(60));
        assert_eq!(OCR_DEADLINE.limit(1, 3), Duration::from_secs(120));
    }

    #[test]
    fn ocr_requests_are_rejected_at_the_boundary_for_bad_languages_modes_and_ranges() {
        let input = PathBuf::from("scan.pdf");
        let output = PathBuf::from("out");
        let cases: [(&str, fn(&mut OcrRequest), &str); 5] = [
            ("empty languages", |request| request.languages.clear(), "at least one recognition language"),
            ("unknown language", |request| request.languages.push("klingon".into()), "\"klingon\" is not a recognition language"),
            ("unknown mode", |request| request.mode = "hocr".into(), "\"hocr\" is not an OCR output"),
            ("backwards range", |request| request.page_range = Some("3-1".into()), "runs backwards"),
            ("malformed range", |request| request.page_range = Some("1, x".into()), "isn't a page or range"),
        ];
        for (name, change, message) in cases {
            let mut request = ocr_request(&input, None, &output);
            change(&mut request);
            let outcomes = validate_ocr_request(&request).expect_err(name);
            assert_eq!(outcomes.len(), 1, "{name}");
            let error = outcomes[0].error().expect(name);
            assert_eq!(error.kind, ErrorKind::InvalidInput, "{name}");
            assert!(error.message.contains(message), "{name}: {}", error.message);
        }
        let mut searchable = ocr_request(&input, Some("1-3, 5, 8-"), &output);
        searchable.mode = "searchablePdf".into();
        searchable.force = true;
        assert!(validate_ocr_request(&searchable).is_ok());
    }

    #[test]
    fn word_boxes_keep_reading_order_and_take_line_heights_from_line_rows() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
1\t1\t0\t0\t0\t0\t0\t0\t1700\t2200\t-1\t\n\
4\t1\t1\t1\t1\t0\t100\t200\t500\t60\t-1\t\n\
5\t1\t1\t1\t1\t1\t100\t210\t200\t40\t96.1\tFirst\n\
5\t1\t1\t1\t1\t2\t320\t205\t280\t50\t12.0\tline\n\
5\t1\t1\t1\t1\t3\t620\t205\t0\t0\t-1\t \n\
5\t1\t2\t1\t1\t1\t100\t400\t240\t50\t91.0\tПривет\r\n";
        let layout = super::word_boxes(tsv.as_bytes()).unwrap();
        assert_eq!((layout.width, layout.height), (1700., 2200.));
        let lines = layout.lines.iter().map(|line| (line.top, line.bottom, line.words.iter().map(|word| (word.text.as_str(), word.left, word.width)).collect::<Vec<_>>())).collect::<Vec<_>>();
        assert_eq!(lines, vec![
            (200., 260., vec![("First", 100., 200.), ("line", 320., 280.)]),
            (400., 450., vec![("Привет", 100., 240.)]),
        ]);
        assert_eq!(super::word_boxes(b"level\tpage_num\n1\t1\t0\t0\t0\t0\t0\t0\t1700\t2200\t-1\t\n").unwrap().lines, Vec::new());
        assert!(super::word_boxes(b"5\t1\t1\t1\t1\t1\t1\t1\t9\t9\t90\tlost\n").is_err(), "words without a page size can't be placed");
        assert!(super::word_boxes(b"5\t1\t1\tnot a box\n").is_err());
    }

    #[test]
    fn the_shared_ocr_request_sample_parses_and_passes_the_boundary() {
        let request: OcrRequest = serde_json::from_str(include_str!("../../../shared/ocr-request-sample.json")).unwrap();
        assert_eq!(request.paths, vec![PathBuf::from("/scans/contract.pdf")]);
        assert_eq!(request.page_range.as_deref(), Some("1-3, 5, 8-"));
        assert_eq!(request.mode(), Ok(super::OcrMode::Text));
        assert!(!request.force);
        assert_eq!(request.languages, vec!["eng"]);
        assert!(matches!(request.output_location, OutputLocation::AlongsideInput));
        assert!(validate_ocr_request(&request).is_ok());
    }

    #[cfg(unix)]
    struct TimedOcrHelpers {
        renderer: PathBuf,
        engine: PathBuf,
        tessdata: PathBuf,
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    #[cfg(unix)]
    impl TimedOcrHelpers {
        fn install(name: &str, renderer_body: &str, engine_body: &str) -> Self {
            let renderer = path(&format!("{name}-renderer.sh"));
            let engine = path(&format!("{name}-engine.sh"));
            let tessdata = path(&format!("{name}-eng.traineddata"));
            fs::write(&renderer, format!("#!/bin/sh\n{renderer_body}\ntouch \"${{10}}.png\"\n")).unwrap();
            fs::write(&engine, format!("#!/bin/sh\n{engine_body}\n")).unwrap();
            fs::write(&tessdata, b"fake tessdata").unwrap();
            for helper in [&renderer, &engine] {
                fs::set_permissions(helper, fs::Permissions::from_mode(0o755)).unwrap();
            }
            let saved = ["TOOLBOX_PDFTOPPM_PATH", "TOOLBOX_TESSERACT_PATH", "TOOLBOX_TESSDATA_PATH"].into_iter().map(|key| (key, std::env::var_os(key))).collect();
            std::env::set_var("TOOLBOX_PDFTOPPM_PATH", &renderer);
            std::env::set_var("TOOLBOX_TESSERACT_PATH", &engine);
            std::env::set_var("TOOLBOX_TESSDATA_PATH", &tessdata);
            Self { renderer, engine, tessdata, saved }
        }
    }

    #[cfg(unix)]
    impl Drop for TimedOcrHelpers {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                match value { Some(value) => std::env::set_var(key, value), None => std::env::remove_var(key) }
            }
            for file in [&self.renderer, &self.engine, &self.tessdata] { let _ = fs::remove_file(file); }
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_multi_page_file_gets_a_budget_for_every_page() {
        let _guard = crate::kit::lock_process_env();
        let input = path("per-page-budget.pdf");
        let output_root = path("per-page-budget-output");
        make_pdf(&input, 3);
        fs::create_dir_all(&output_root).unwrap();
        let helpers = TimedOcrHelpers::install("per-page-budget", "sleep 0.6", "sleep 0.6\nprintf 'page\\n'");
        let deadline = HelperDeadline { per_file: Duration::from_millis(500), per_page: Duration::from_secs(2) };

        let started = Instant::now();
        let outcome = ocr_pdf_within(&ocr_request(&input, None, &output_root), input.clone(), deadline);
        let elapsed = started.elapsed();
        let text = outcome.output_paths().first().and_then(|path| fs::read_to_string(path).ok());
        drop(helpers);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_dir_all(&output_root);

        assert!(outcome.error().is_none(), "budgeted OCR failed: {:?}", outcome.error());
        assert_eq!(text.as_deref(), Some("page\n\u{c}page\n\u{c}page\n"));
        assert!(elapsed > deadline.limit(1, 1), "three pages ran longer than one page's budget, so a single shared deadline would have stopped them: {elapsed:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_page_that_passes_its_budget_fails_the_file_with_a_time_limit() {
        let _guard = crate::kit::lock_process_env();
        let input = path("page-over-budget.pdf");
        let output_root = path("page-over-budget-output");
        make_pdf(&input, 2);
        let original = fs::read(&input).unwrap();
        fs::create_dir_all(&output_root).unwrap();
        let before_workspaces = ocr_workspaces();
        let helpers = TimedOcrHelpers::install("page-over-budget", ":", "case \"$1\" in *page-2.png) exec sleep 30 ;; esac\nprintf 'page one\\n'");

        let started = Instant::now();
        let outcome = ocr_pdf_within(&ocr_request(&input, None, &output_root), input.clone(), HelperDeadline { per_file: Duration::from_millis(300), per_page: Duration::from_millis(200) });
        let elapsed = started.elapsed();
        let after_workspaces = ocr_workspaces();
        let output_entries = fs::read_dir(&output_root).unwrap().count();
        let input_after = fs::read(&input).unwrap();
        drop(helpers);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_dir_all(&output_root);

        let error = outcome.error().expect("the stalled page fails the file");
        assert_eq!(error.kind, ErrorKind::LimitExceeded { limit: LimitKind::Time });
        assert_eq!(error.message, "Recognising took longer than 700 milliseconds and was stopped. Nothing was saved.");
        assert!(elapsed < Duration::from_secs(10), "the engine is stopped at its deadline, not waited out: {elapsed:?}");
        assert!(outcome.output_paths().is_empty());
        assert_eq!(output_entries, 0);
        assert_eq!(before_workspaces, after_workspaces);
        assert_eq!(input_after, original);
    }
}
