use flate2::{write::ZlibEncoder, Compression};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::kit::common::limits::{self, StoppableHelper};
use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming, OutputReservation};
use crate::kit::common::watermark::{WatermarkOptions, WatermarkPlacement, WatermarkPosition};
use crate::kit::contracts::{LimitKind, OutcomePointer, OutcomeWarning, SkipCode, SkipReason, ToolError, WarningKind};
use crate::kit::images::source::{self, ImageFamily};
use super::metadata::{page_bounds, page_rotation};
use super::page_ranges::PageRanges;
use super::{has_signed_signature_field, mutation_preflight, note_signature_removed, remove_signatures, resolve, PdfMutationIntent};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageOverlayRequest {
    pub paths: Vec<PathBuf>,
    #[serde(default)] pub position: Option<String>,
    #[serde(default)] pub pages: Option<Vec<usize>>,
    #[serde(default)] pub start_number: Option<u32>,
    #[serde(default)] pub font_size: Option<u16>,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfWatermarkRequest {
    pub paths: Vec<PathBuf>,
    #[serde(flatten)] pub options: WatermarkOptions,
    #[serde(default)] pub pages: Option<Vec<usize>>,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressPdfRequest {
    pub paths: Vec<PathBuf>,
    pub quality: u8,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageSelectionRequest {
    pub paths: Vec<PathBuf>,
    pub pages: Vec<usize>,
    #[serde(default)] pub page_ranges: Option<String>,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitPdfRequest {
    pub paths: Vec<PathBuf>,
    #[serde(default)] pub mode: RequestedSplitMode,
    pub output_location: OutputLocation,
}

/// How a split divides the document. Each kind carries only its own fields.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum SplitMode {
    Ranges { ranges: String },
    EveryN { pages: usize },
}

/// The mode as the caller sent it. A missing or malformed mode stays a per-file
/// InvalidInput outcome instead of failing the whole command before any file runs.
#[derive(Debug, Clone)]
pub struct RequestedSplitMode(Result<SplitMode, String>);

impl Default for RequestedSplitMode {
    fn default() -> Self { Self(Err("Choose how to split: by page ranges or every N pages.".to_string())) }
}

impl From<SplitMode> for RequestedSplitMode {
    fn from(mode: SplitMode) -> Self { Self(Ok(mode)) }
}

impl<'de> serde::Deserialize<'de> for RequestedSplitMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = <serde_json::Value as serde::Deserialize>::deserialize(deserializer)?;
        Ok(Self(serde_json::from_value(value).map_err(|error| format!("The split mode is malformed: {error}."))))
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergePdfRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
    #[serde(default)]
    pub output_path: Option<PathBuf>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfToImagesRequest {
    pub paths: Vec<PathBuf>,
    pub dpi: u16,
    pub format: String,
    #[serde(default)] pub page_range: Option<String>,
    #[serde(default)] pub pages: Option<Vec<usize>>,
    pub output_location: OutputLocation,
}

impl PdfToImagesRequest {
    pub(crate) fn page_range(&self) -> Option<&str> {
        self.page_range.as_deref().filter(|value| !value.trim().is_empty())
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagesToPdfRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
    #[serde(default)]
    pub output_path: Option<PathBuf>,
    #[serde(flatten)]
    pub layout: PageLayout,
}

/// How each image sits on its page. Margin, fit and orientation only apply to
/// A4 and Letter; a "match image" page is always the image's own size.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageLayout {
    #[serde(default)] pub page_size: PageSize,
    #[serde(default)] pub margin: Option<PageMargin>,
    #[serde(default)] pub fit: Option<PageFit>,
    #[serde(default)] pub orientation: Option<PageOrientation>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageSize { #[default] MatchImage, A4, Letter }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageMargin { None, #[default] Small, Large }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageFit { #[default] Fit, Fill }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageOrientation { #[default] Automatic, Portrait, Landscape }

/// Portrait paper sizes in PDF points: A4 is 210 × 297 mm, Letter 8.5 × 11 in.
pub const A4_POINTS: (f32, f32) = (595.2756, 841.8898);
pub const LETTER_POINTS: (f32, f32) = (612.0, 792.0);

impl PageMargin {
    /// Small is half an inch (12.7 mm), large is one inch (25.4 mm), on every side.
    pub fn points(self) -> f32 {
        match self { PageMargin::None => 0.0, PageMargin::Small => 36.0, PageMargin::Large => 72.0 }
    }
}

/// Where one image lands: the page's media box size, an optional clip
/// rectangle (fill only), and the rectangle the image is drawn into, all in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PagePlacement {
    pub page: (f32, f32),
    pub clip: Option<[f32; 4]>,
    pub image: [f32; 4],
}

impl PageLayout {
    pub fn place(&self, width: u32, height: u32) -> PagePlacement {
        let (width, height) = (width as f32, height as f32);
        let (short, long) = match self.page_size {
            PageSize::MatchImage => return PagePlacement { page: (width, height), clip: None, image: [0.0, 0.0, width, height] },
            PageSize::A4 => A4_POINTS,
            PageSize::Letter => LETTER_POINTS,
        };
        let landscape = match self.orientation.unwrap_or_default() {
            PageOrientation::Automatic => width > height,
            PageOrientation::Portrait => false,
            PageOrientation::Landscape => true,
        };
        let (page_width, page_height) = if landscape { (long, short) } else { (short, long) };
        let margin = self.margin.unwrap_or_default().points();
        let (box_width, box_height) = (page_width - 2.0 * margin, page_height - 2.0 * margin);
        let fill = self.fit.unwrap_or_default() == PageFit::Fill;
        let (across, down) = (box_width / width, box_height / height);
        let scale = if fill { across.max(down) } else { across.min(down) };
        let (drawn_width, drawn_height) = (width * scale, height * scale);
        let image = [margin + (box_width - drawn_width) / 2.0, margin + (box_height - drawn_height) / 2.0, drawn_width, drawn_height];
        PagePlacement { page: (page_width, page_height), clip: fill.then_some([margin, margin, box_width, box_height]), image }
    }
}

impl PagePlacement {
    // Fill trims by clipping the drawn image to the page's content box; the
    // embedded image data itself is never cropped or resampled.
    fn content(&self) -> String {
        let mut content = String::from("q ");
        if let Some([x, y, width, height]) = self.clip {
            content.push_str(&format!("{} {} {} {} re W n ", points(x), points(y), points(width), points(height)));
        }
        let [x, y, width, height] = self.image;
        content.push_str(&format!("{} 0 0 {} {} {} cm /Im0 Do Q", points(width), points(height), points(x), points(y)));
        content
    }
}

fn points(value: f32) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_string() } else { text.to_string() }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairPdfRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparePdfsRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
    #[serde(default)]
    pub output_path: Option<PathBuf>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotatePdfRequest {
    pub paths: Vec<PathBuf>,
    pub degrees: i32,
    #[serde(default)] pub pages: Option<Vec<usize>>,
    pub output_location: OutputLocation,
}

/// A merge reports one combined outcome for the run's limits and, once it succeeds, one outcome per entry.
pub struct Merged {
    pub combined: JobOutcome,
    pub entries: Vec<JobOutcome>,
}

impl From<JobOutcome> for Merged {
    fn from(combined: JobOutcome) -> Self { Self { combined, entries: Vec::new() } }
}

pub(crate) type QpdfDeadline = limits::HelperDeadline;

pub(crate) const QPDF_DEADLINE: QpdfDeadline = QpdfDeadline { per_file: Duration::from_secs(15), per_page: Duration::from_millis(50) };

fn run_qpdf(command: Command, deadline: Instant, limit: Duration, action: &str) -> Result<std::process::Output, ToolError> {
    let timed_out = format!("{action} took longer than {} and was stopped. Nothing was saved.", limits::duration_label(limit));
    match super::run_bounded_helper(command, deadline, super::PDF_TEXT_MAX_ERROR_BYTES, super::PDF_TEXT_MAX_ERROR_BYTES, &timed_out) {
        Ok(output) => Ok(output),
        Err(message) if message == timed_out => Err(ToolError::limit_exceeded(LimitKind::Time, message)),
        Err(message) => Err(ToolError::processing(format!("Could not run qpdf: {message}"))),
    }
}

pub fn merge(request: &MergePdfRequest) -> Merged {
    merge_within(request, QPDF_DEADLINE)
}

fn merge_within(request: &MergePdfRequest, deadline: QpdfDeadline) -> Merged {
    let Some(first) = request.paths.first().cloned() else { return failure(PathBuf::new(), "Select at least one PDF.".to_string()).into(); };
    if request.paths.len() < 2 { return failure(first, "Select at least two PDFs to merge.".to_string()).into(); }
    let max_pages = limits::recorded("merge_pdfs").and_then(|limits| limits.max_pages).map_or(u64::MAX, u64::from);
    let mut inputs = Vec::with_capacity(request.paths.len());
    let mut expected_pages = 0usize;
    for path in &request.paths {
        if path.extension().and_then(|extension| extension.to_str()).is_none_or(|extension| !extension.eq_ignore_ascii_case("pdf")) { return failure(path.clone(), format!("Only PDF inputs can be merged: {}", path.display())).into(); }
        let input = match ReadableInput::inspect(path) {
            Ok(input) => input,
            Err(error) => return JobOutcome::failure(path.clone(), error).into(),
        };
        expected_pages += input.pages;
        if expected_pages as u64 > max_pages {
            let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
            return JobOutcome::failure(path.clone(), ToolError::limit_exceeded(LimitKind::PageCount, format!(
                "Adding {name} takes the merged PDF past {} pages, over the limit for one PDF. Merge fewer pages and try again.",
                limits::count_label(max_pages),
            ))).into();
        }
        inputs.push(input);
    }
    let qpdf = match qpdf() { Ok(qpdf) => qpdf, Err(error) => return JobOutcome::failure(first, ToolError::unavailable(error)).into() };
    let output = match OutputNaming::reserve_single_output(request.output_path.as_deref(), &first, &request.output_location, "-merged", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(first, format!("Could not reserve merged PDF output: {error}")).into(),
    };
    let mut command = Command::new(qpdf);
    command.arg("--empty").arg("--pages").args(&request.paths).arg("--").arg(output.path());
    let limit = deadline.limit(request.paths.len(), expected_pages);
    let verified = match run_qpdf(command, Instant::now() + limit, limit, "Merging") {
        Ok(result) if result.status.success() => match Document::load(output.path()) {
            Ok(document) if document.get_pages().len() == expected_pages => unsign_copy(document, output.path()),
            Ok(document) => Err(format!("Merged PDF page count mismatch: expected {expected_pages}, got {}.", document.get_pages().len())),
            Err(error) => Err(format!("Merged PDF could not be verified: {error}")),
        },
        Ok(result) => Err(stderr(result, "qpdf failed to merge the PDFs.")),
        Err(error) => return JobOutcome::failure(first, error).into(),
    };
    if let Err(error) = verified { return failure(first, error).into(); }
    let merged = match output.publish() {
        Ok(path) => path,
        Err(error) => return failure(first, format!("Could not publish merged PDF output: {error}")).into(),
    };
    let merged_name = merged.file_name().map_or_else(|| merged.display().to_string(), |name| name.to_string_lossy().into_owned());
    let mut next_page = 1;
    let entries = request.paths.iter().zip(&inputs).map(|(path, input)| {
        let (start, end) = (next_page, next_page + input.pages - 1);
        next_page = end + 1;
        let span = if start == end { format!("Page {start}") } else { format!("Pages {start}–{end}") };
        let outcome = JobOutcome::succeeded(path.clone(), vec![merged.clone()], format!("{span} of {merged_name}."));
        input.note_what_was_not_kept(outcome, "the merged file")
    }).collect();
    Merged {
        combined: JobOutcome::succeeded(first, vec![merged], format!("{} PDFs merged in input order", request.paths.len())),
        entries,
    }
}

pub fn split(request: &SplitPdfRequest, input: PathBuf) -> JobOutcome {
    split_within(request, input, QPDF_DEADLINE)
}

fn split_within(request: &SplitPdfRequest, input: PathBuf, deadline: QpdfDeadline) -> JobOutcome {
    let plan = match request.mode.0.as_ref().map_err(String::clone).and_then(SplitPlan::new) {
        Ok(plan) => plan,
        Err(error) => return JobOutcome::failure(input, ToolError::invalid_input(error)),
    };
    let readable = match ReadableInput::inspect(&input) {
        Ok(readable) => readable,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let page_count = readable.pages;
    let location = &request.output_location;
    let spans = match plan.spans(page_count) {
        Ok(spans) if spans.is_empty() => return failure(input, "The PDF has no pages to split.".to_string()),
        Ok(spans) => spans,
        Err(error) => return JobOutcome::failure(input, ToolError::invalid_input(error)),
    };
    let qpdf = match qpdf() { Ok(qpdf) => qpdf, Err(error) => return JobOutcome::failure(input, ToolError::unavailable(error)) };
    let workspace = match SplitWorkspace::new() {
        Ok(workspace) => workspace,
        Err(error) => return failure(input, format!("Could not create a private split workspace: {error}")),
    };
    let limit = deadline.limit(1, page_count);
    let generated = match write_split_spans(&input, &qpdf, workspace.path(), spans, Instant::now() + limit, limit) {
        Ok(generated) => generated,
        Err(error) => return JobOutcome::failure(input, error),
    };
    if readable.signed {
        if let Err(error) = generated.iter().try_for_each(|(_, part)| Document::load(part).map_err(|error| format!("A split part could not be verified: {error}")).and_then(|document| unsign_copy(document, part))) {
            return failure(input, error);
        }
    }
    match materialize_split_outputs(&input, location, &generated) {
        Ok(outputs) => {
            let detail = match plan {
                SplitPlan::Ranges(_) => {
                    let stem = input.file_stem().and_then(|value| value.to_str()).unwrap_or("split");
                    format!("PDF split into selected ranges from {stem}")
                }
                SplitPlan::EveryN(_) => "PDF split".to_string(),
            };
            readable.note_what_was_not_kept(JobOutcome::succeeded(input, outputs, detail), "the parts")
        }
        Err(outcome) => outcome,
    }
}

struct SplitWorkspace(PathBuf);

impl SplitWorkspace {
    fn new() -> io::Result<Self> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let root = std::env::temp_dir();
        for attempt in 0..100 {
            let path = root.join(format!("toolbox_pdf_split_{}_{}_{}", std::process::id(), timestamp, attempt));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "could not allocate a unique temporary directory"))
    }

    fn path(&self) -> &Path { &self.0 }
}

impl Drop for SplitWorkspace {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

struct SplitSpan { suffix: String, first: usize, last: usize }

/// One entry of a range string: a single page, a closed range, or an open-ended `N-`.
#[derive(Debug, Clone, Copy)]
struct SplitRange { start: usize, end: Option<usize> }

/// A split mode checked without the document, so malformed requests fail before any file is read.
enum SplitPlan { Ranges(Vec<(String, SplitRange)>), EveryN(usize) }

impl SplitPlan {
    fn new(mode: &SplitMode) -> Result<Self, String> {
        match mode {
            SplitMode::Ranges { ranges } => parse_split_ranges(ranges).map(Self::Ranges),
            SplitMode::EveryN { pages: 0 } => Err("Pages per file must be at least 1.".to_string()),
            SplitMode::EveryN { pages } => Ok(Self::EveryN(*pages)),
        }
    }

    fn spans(&self, page_count: usize) -> Result<Vec<SplitSpan>, String> {
        match self {
            Self::Ranges(ranges) => {
                let beyond = ranges.iter().filter(|(_, range)| range.start > page_count || range.end.is_some_and(|end| end > page_count)).map(|(token, _)| format!("{token} goes past the last page ({page_count}).")).collect::<Vec<_>>();
                if !beyond.is_empty() { return Err(beyond.join(" ")); }
                Ok(ranges.iter().map(|(_, range)| {
                    let (first, last) = (range.start, range.end.unwrap_or(page_count));
                    let suffix = if first == last { format!("-page-{first}") } else { format!("-pages-{first}-{last}") };
                    SplitSpan { suffix, first, last }
                }).collect())
            }
            // Every-N names stay on the shipped -split family until #541 settles them.
            Self::EveryN(size) => {
                let (size, width) = (*size, page_count.to_string().len());
                Ok((1..=page_count).step_by(size).map(|first| {
                    let last = (first + size - 1).min(page_count);
                    let suffix = if size == 1 { format!("-split-page-{first:0width$}") } else { format!("-split-{first:0width$}-{last:0width$}") };
                    SplitSpan { suffix, first, last }
                }).collect())
            }
        }
    }
}

/// Parses a comma-separated list of one-based pages and ranges. Every problem is
/// reported at once, and any problem rejects the whole string.
fn parse_split_ranges(raw: &str) -> Result<Vec<(String, SplitRange)>, String> {
    if raw.trim().is_empty() { return Err("Enter at least one page range to split.".to_string()); }
    let digits = |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    let mut ranges = Vec::new();
    let mut problems = Vec::new();
    for (index, token) in raw.split(',').map(str::trim).enumerate() {
        if token.is_empty() { problems.push(format!("Entry {} is empty.", index + 1)); continue; }
        let Some((start, end)) = token.split_once('-').map(|(start, end)| (start.trim(), end.trim())) else {
            match token.parse::<usize>() {
                Ok(0) if digits(token) => problems.push(format!("{token} isn't a page. Pages start at 1.")),
                Ok(page) if digits(token) => ranges.push((token.to_string(), SplitRange { start: page, end: Some(page) })),
                _ => problems.push(format!("{token} isn't a page.")),
            }
            continue;
        };
        let start = if digits(start) { start.parse::<usize>().ok() } else { None };
        let end = if end.is_empty() { Some(None) } else if digits(end) { end.parse::<usize>().ok().map(Some) } else { None };
        match (start, end) {
            (Some(start), Some(end)) if start == 0 || end == Some(0) => problems.push(format!("{token} isn't a range. Pages start at 1.")),
            (Some(start), Some(Some(end))) if start > end => problems.push(format!("{token} runs backwards (write {end}-{start}).")),
            (Some(start), Some(end)) => ranges.push((token.to_string(), SplitRange { start, end })),
            _ => problems.push(format!("{token} isn't a range.")),
        }
    }
    if problems.is_empty() { Ok(ranges) } else { Err(problems.join(" ")) }
}

fn write_split_spans(input: &Path, qpdf: &Path, workspace: &Path, spans: Vec<SplitSpan>, deadline: Instant, limit: Duration) -> Result<Vec<(String, PathBuf)>, ToolError> {
    spans.into_iter().enumerate().map(|(index, span)| {
        let part = workspace.join(format!("part-{index}.pdf"));
        let mut command = Command::new(qpdf);
        command.arg("--empty").arg("--pages").arg(input).arg(format!("{}-{}", span.first, span.last)).arg("--").arg(&part);
        let result = run_qpdf(command, deadline, limit, "Splitting")?;
        if !result.status.success() { return Err(ToolError::processing(stderr(result, "qpdf failed to split the PDF."))); }
        if !part.is_file() { return Err(ToolError::processing("qpdf reported success but produced no split file.")); }
        Ok((span.suffix, part))
    }).collect()
}

fn materialize_split_outputs(input: &Path, location: &OutputLocation, generated: &[(String, PathBuf)]) -> Result<Vec<PathBuf>, JobOutcome> {
    let mut outputs = Vec::with_capacity(generated.len());
    for (suffix, source) in generated {
        let reservation = match OutputNaming::reserve_destination(input, location, suffix, "pdf") {
            Ok(reservation) => reservation,
            Err(error) => return Err(JobOutcome::failure_with_outputs(input.to_path_buf(), outputs, ToolError::processing(format!("Could not reserve split output: {error}")))),
        };
        if let Err(error) = copy_into_reservation(source, reservation.path()) {
            return Err(JobOutcome::failure_with_outputs(input.to_path_buf(), outputs, ToolError::processing(format!("Could not move split output: {error}"))));
        }
        let destination = match reservation.publish() {
            Ok(path) => path,
            Err(error) => return Err(JobOutcome::failure_with_outputs(input.to_path_buf(), outputs, ToolError::processing(format!("Could not publish split output: {error}")))),
        };
        outputs.push(destination);
    }
    Ok(outputs)
}

fn copy_into_reservation(source: &Path, destination: &Path) -> io::Result<()> {
    fs::copy(source, destination)?;
    fs::remove_file(source)
}

/// Shown for every file when the bundled page renderer is missing, so the row blames this copy of
/// Toolbox rather than the user's PDF.
pub(crate) const PDF_RENDERER_UNAVAILABLE: &str = "Not converted. The page renderer is unavailable. It's a bundled helper that ships inside Toolbox, so this is a problem with this copy of the app, not with your PDF. Reinstall Toolbox to restore it.";
pub(crate) const PDF_TO_IMAGES_PASSWORD_PROTECTED: &str = "It's password-protected. Remove its password, then convert it again.";
/// Each file gets this much renderer time, plus `PDF_RENDER_TIME_PER_PAGE` for every page it renders.
pub(crate) const PDF_RENDER_TIME_PER_FILE: Duration = Duration::from_secs(15);
pub(crate) const PDF_RENDER_TIME_PER_PAGE: Duration = Duration::from_secs(2);
const PDF_RENDER_MAX_OUTPUT_BYTES: usize = 64 * 1024;

fn pdf_render_time_limit(page_count: usize) -> Duration {
    PDF_RENDER_TIME_PER_FILE.saturating_add(PDF_RENDER_TIME_PER_PAGE.saturating_mul(u32::try_from(page_count).unwrap_or(u32::MAX)))
}

pub fn to_images(request: &PdfToImagesRequest, input: PathBuf) -> JobOutcome {
    to_images_within(request, input, pdf_render_time_limit)
}

fn to_images_within(request: &PdfToImagesRequest, input: PathBuf, time_limit: fn(usize) -> Duration) -> JobOutcome {
    let format = request.format.to_lowercase();
    if format != "jpg" && format != "png" { return failure(input, "PDF images must be JPEG or PNG.".to_string()); }
    let ranges = match request.page_range().map(PageRanges::parse).transpose() {
        Ok(ranges) => ranges,
        Err(error) => return JobOutcome::failure(input, ToolError::invalid_input(error)),
    };
    let Ok(renderer) = crate::kit::resources::resolve_helper(&crate::kit::resources::PDF_RENDERER) else {
        return JobOutcome::failure(input, ToolError::unavailable(PDF_RENDERER_UNAVAILABLE));
    };
    let document = match Document::load(&input) {
        Ok(document) => document,
        Err(_) => return JobOutcome::failure(input, crate::kit::password::unreadable(crate::kit::password::ProtectedFormat::Pdf)),
    };
    if document.is_encrypted() {
        return JobOutcome::failure(input, ToolError::invalid_input(PDF_TO_IMAGES_PASSWORD_PROTECTED)
            .with_pointer(OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
    }
    let page_count = document.get_pages().len();
    let dpi = request.dpi.clamp(72, 300);
    let extension = if format == "jpg" { "jpg" } else { "png" };
    let workspace = match SplitWorkspace::new() {
        Ok(workspace) => workspace,
        Err(error) => return failure(input, format!("Could not create a private image workspace: {error}")),
    };
    let prefix = match unique_image_prefix(&input, &request.output_location, extension, &workspace) {
        Ok(prefix) => prefix,
        Err(error) => return failure(input, format!("Could not reserve PDF image output: {error}")),
    };
    let renderer_format = if format == "jpg" { "jpeg" } else { "png" };
    if let Some(requested_pages) = request.pages.as_ref() {
        let pages = match selected_pdf_pages(&document, Some(requested_pages)) {
            Ok(pages) if !pages.is_empty() => pages,
            Ok(_) => return failure(input, "Select at least one PDF page.".to_string()),
            Err(error) => return failure(input, error),
        };
        let deadline = RenderDeadline::new(time_limit(pages.len()));
        let generated = match render_selected_pages(&renderer, &input, &prefix, renderer_format, dpi, extension, &pages, &deadline) {
            Ok(outputs) => outputs,
            Err(error) => return JobOutcome::failure(input, error),
        };
        return materialize_rendered_images(&input, &request.output_location, &generated);
    }

    let spans = ranges.map_or_else(|| vec![None], |ranges| ranges.spans().iter().copied().map(Some).collect());
    let rendered_pages = spans.iter().map(|span| span.map_or(page_count, |span| (span.last.unwrap_or(page_count).min(page_count) + 1).saturating_sub(span.first))).sum::<usize>();
    let deadline = RenderDeadline::new(time_limit(rendered_pages));
    for span in spans {
        let mut command = Command::new(&renderer);
        command.arg(format!("-{renderer_format}")).arg("-r").arg(dpi.to_string());
        if let Some(span) = span {
            command.arg("-f").arg(span.first.to_string());
            if let Some(last) = span.last { command.arg("-l").arg(last.to_string()); }
        }
        command.arg(&input).arg(&prefix);
        match deadline.run(command) {
            Ok(result) if result.status.success() => {}
            Ok(result) => return failure(input, stderr(result, "pdftoppm failed to render the PDF.")),
            Err(error) => return JobOutcome::failure(input, error),
        }
    }
    let mut outputs = fs::read_dir(prefix.parent().unwrap_or_else(|| std::path::Path::new("."))).ok().into_iter().flatten().filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(prefix.file_name().and_then(|stem| stem.to_str()).unwrap_or("")) && path.extension().is_some_and(|ext| ext == extension))).collect::<Vec<_>>();
    outputs.sort_by_key(|path| rendered_page_index(path, &prefix));
    if outputs.is_empty() { return failure(input, "PDF renderer produced no images.".to_string()); }
    materialize_rendered_images(&input, &request.output_location, &outputs)
}

/// One deadline shared by every renderer call for a file. Passing it fails the file with a time limit.
struct RenderDeadline {
    at: Instant,
    timed_out: String,
}

impl RenderDeadline {
    fn new(limit: Duration) -> Self {
        Self { at: Instant::now() + limit, timed_out: format!("Rendering took longer than {} and was stopped. Nothing was saved.", crate::kit::common::limits::duration_label(limit)) }
    }

    fn run(&self, command: Command) -> Result<std::process::Output, ToolError> {
        match super::run_bounded_helper(command, self.at, PDF_RENDER_MAX_OUTPUT_BYTES, PDF_RENDER_MAX_OUTPUT_BYTES, &self.timed_out) {
            Ok(output) => Ok(output),
            Err(message) if message == self.timed_out => Err(ToolError::limit_exceeded(LimitKind::Time, message)),
            Err(message) => Err(ToolError::processing(format!("Could not run pdftoppm: {message}"))),
        }
    }
}

fn rendered_page_index(path: &Path, prefix: &Path) -> Option<u32> {
    let stem = path.file_stem()?.to_str()?;
    let prefix = prefix.file_name()?.to_str()?;
    stem.strip_prefix(&format!("{prefix}-"))?.parse().ok()
}

fn unique_image_prefix(input: &PathBuf, location: &OutputLocation, extension: &str, workspace: &SplitWorkspace) -> std::io::Result<PathBuf> {
    let mut counter = 0usize;
    loop {
        let suffix = if counter == 0 { "-images".to_string() } else { format!("-images-{counter}") };
        let destination = OutputNaming::reserve_destination(input, location, &suffix, extension)?;
        let stem = destination.destination_path().file_stem().and_then(|value| value.to_str()).unwrap_or("output").to_string();
        let has_existing_family = input.parent().unwrap_or_else(|| Path::new(".")).read_dir().ok().into_iter().flatten().filter_map(Result::ok).any(|entry| {
            let path = entry.path();
            path.extension().and_then(|value| value.to_str()).is_some_and(|value| value.eq_ignore_ascii_case(extension))
                && path.file_stem().and_then(|value| value.to_str()).is_some_and(|value| value.starts_with(&format!("{stem}-")))
        });
        if !has_existing_family {
            drop(destination);
            return Ok(workspace.path().join(stem));
        }
        drop(destination);
        counter += 1;
    }
}

fn render_selected_pages(
    renderer: &PathBuf,
    input: &PathBuf,
    prefix: &PathBuf,
    renderer_format: &str,
    dpi: u16,
    extension: &str,
    pages: &[u32],
    deadline: &RenderDeadline,
) -> Result<Vec<PathBuf>, ToolError> {
    let mut outputs = Vec::with_capacity(pages.len());
    let base = prefix.file_name().and_then(|value| value.to_str()).unwrap_or("output");
    for page in pages {
        let page_prefix = prefix.with_file_name(format!("{base}-page-{page}"));
        let output = page_prefix.with_extension(extension);
        let mut command = Command::new(renderer);
        command
            .arg(format!("-{renderer_format}"))
            .arg("-r")
            .arg(dpi.to_string())
            .arg("-f")
            .arg(page.to_string())
            .arg("-l")
            .arg(page.to_string())
            .arg("-singlefile")
            .arg(input)
            .arg(&page_prefix);
        let result = deadline.run(command)?;
        if !(result.status.success() && output.exists()) {
            return Err(ToolError::processing(stderr(result, "pdftoppm failed to render the selected PDF page.")));
        }
        outputs.push(output);
    }
    Ok(outputs)
}

fn materialize_rendered_images(input: &Path, location: &OutputLocation, generated: &[PathBuf]) -> JobOutcome {
    let input_stem = input.file_stem().and_then(|value| value.to_str()).unwrap_or("output");
    let mut outputs = Vec::with_capacity(generated.len());
    for source in generated {
        let extension = source.extension().and_then(|value| value.to_str()).unwrap_or("png");
        let stem = match source.file_stem().and_then(|value| value.to_str()).and_then(|value| value.strip_prefix(input_stem)) {
            Some(suffix) if suffix.starts_with('-') => suffix,
            _ => return JobOutcome::failure_with_outputs(input.to_path_buf(), outputs, ToolError::processing("PDF renderer produced an unexpected image file name.")),
        };
        let reservation = match OutputNaming::reserve_destination(input, location, stem, extension) {
            Ok(reservation) => reservation,
            Err(error) => return JobOutcome::failure_with_outputs(input.to_path_buf(), outputs, ToolError::processing(format!("Could not reserve PDF image output: {error}"))),
        };
        if let Err(error) = fs::copy(source, reservation.path()) {
            return JobOutcome::failure_with_outputs(input.to_path_buf(), outputs, ToolError::processing(format!("Could not copy rendered PDF image: {error}")));
        }
        let destination = match reservation.publish() {
            Ok(path) => path,
            Err(error) => return JobOutcome::failure_with_outputs(input.to_path_buf(), outputs, ToolError::processing(format!("Could not publish rendered PDF image: {error}"))),
        };
        outputs.push(destination);
    }
    JobOutcome::succeeded(input.to_path_buf(), outputs, "PDF rendered to images")
}

pub const NO_SELECTABLE_TEXT: &str = "PDF contains no selectable text; scanned PDFs require OCR.";
pub const NO_EMBEDDED_IMAGES: &str = "No embedded JPEG images were found. Non-JPEG PDF image filters are not extractable without recompression.";
pub const BOTH_WITHOUT_TEXT: &str = "No selectable text, so no text file was made. OCR can read scanned pages.";
pub const BOTH_WITHOUT_IMAGES: &str = "No images in this PDF, so only the text file was saved.";
pub const NOTHING_TO_EXTRACT: &str = "Nothing to extract: no selectable text and no images. Nothing was saved.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtractMode { Text, Images, Both }

impl ExtractMode {
    fn wants_text(self) -> bool { matches!(self, Self::Text | Self::Both) }
    fn wants_images(self) -> bool { matches!(self, Self::Images | Self::Both) }
}

/// One request for Extract from PDF. `pageRange` uses the shared syntax ("1-3, 5, 8-") and applies to every PDF.
/// `preserveLayout` is accepted for the Text part; today's text routine always follows reading order (#625 reads it).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractFromPdfRequest {
    pub paths: Vec<PathBuf>,
    pub mode: ExtractMode,
    #[serde(default)] pub page_range: Option<String>,
    pub output_location: OutputLocation,
    #[allow(dead_code)]
    #[serde(default)] pub preserve_layout: bool,
    #[serde(default)] pub min_image_size: Option<u32>,
}

/// Saves the text, the images, or both from one PDF and reports them as one outcome.
pub fn extract(request: &ExtractFromPdfRequest, input: PathBuf) -> JobOutcome {
    let document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let pages = match pages_in_range(&document, request.page_range.as_deref()) {
        Ok(pages) if !pages.is_empty() => pages,
        Ok(_) => return failure(input, "Select at least one PDF page.".to_string()),
        Err(error) => return failure(input, error),
    };
    let text = if request.mode.wants_text() {
        match write_text(&input, &document, &pages, &request.output_location) {
            Ok(text) => text,
            Err(error) => return failure(input, error),
        }
    } else { None };
    let images = if request.mode.wants_images() {
        match write_images(&input, &document, &pages, &request.output_location, request.min_image_size) {
            Ok(images) => images,
            Err((published, error)) => {
                let outputs = text.into_iter().chain(published).collect();
                return JobOutcome::failure_with_outputs(input, outputs, ToolError::processing(error));
            }
        }
    } else { Vec::new() };
    match (request.mode, text, images.is_empty()) {
        (ExtractMode::Text, Some(text), _) => JobOutcome::succeeded(input, vec![text], "PDF text extracted in page order"),
        (ExtractMode::Text, None, _) => failure(input, NO_SELECTABLE_TEXT.to_string()),
        (ExtractMode::Images, _, true) => failure(input, NO_EMBEDDED_IMAGES.to_string()),
        (ExtractMode::Images, _, false) => JobOutcome::succeeded(input, images, "Embedded JPEG images extracted without recompression"),
        (ExtractMode::Both, Some(text), false) => {
            let detail = format!("Saved the text and {} {}.", images.len(), if images.len() == 1 { "image" } else { "images" });
            JobOutcome::succeeded(input, std::iter::once(text).chain(images).collect(), detail)
        }
        (ExtractMode::Both, Some(text), true) => JobOutcome::succeeded(input, vec![text], BOTH_WITHOUT_IMAGES),
        (ExtractMode::Both, None, false) => JobOutcome::succeeded(input, images, BOTH_WITHOUT_TEXT),
        (ExtractMode::Both, None, true) => JobOutcome::skipped(input, SkipReason::new(SkipCode::NoEffectiveChange, NOTHING_TO_EXTRACT)),
    }
}

/// Resolves the shared range syntax against one PDF into 1-based page numbers in document order.
fn pages_in_range(document: &Document, range: Option<&str>) -> Result<Vec<u32>, String> {
    let count = document.get_pages().len();
    let Some(raw) = range.filter(|value| !value.trim().is_empty()) else { return Ok((1..=count as u32).collect()); };
    let spans = PageRanges::parse(raw)?.within(count)?;
    Ok(spans.into_iter().flat_map(|(first, last)| first as u32..=last as u32).collect::<std::collections::BTreeSet<_>>().into_iter().collect())
}

/// Writes the selected pages' text. `None` means the pages hold no selectable text, and nothing is written.
fn write_text(input: &Path, document: &Document, pages: &[u32], location: &OutputLocation) -> Result<Option<PathBuf>, String> {
    let text = document.extract_text(pages).map_err(|error| format!("Could not extract selectable PDF text: {error}"))?;
    let text = normalize_pdf_text(&text);
    if text.is_empty() { return Ok(None); }
    let output = OutputNaming::reserve_destination(input, location, "-text", "txt").map_err(|error| format!("Could not reserve PDF text output: {error}"))?;
    fs::write(output.path(), text).map_err(|error| format!("Could not write text output: {error}"))?;
    output.publish().map(Some).map_err(|error| format!("Could not publish text output: {error}"))
}

fn normalize_pdf_text(text: &str) -> String {
    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// One embedded image found in the selected pages, in discovery order.
/// The JPEG-only writer reads only page, index, filters, size and data; the rest is for the
/// decoding and deduplication work in #426.
#[derive(Debug)]
#[allow(dead_code)]
struct ImageCandidate<'a> {
    /// One-based page number.
    page: u32,
    /// Position among the page's images.
    index: usize,
    object_id: ObjectId,
    filters: Option<Vec<String>>,
    width: i64,
    height: i64,
    color_space: Option<String>,
    soft_mask: Option<ObjectId>,
    mask: Option<ObjectId>,
    data: &'a [u8],
}

/// Lists the embedded images of the selected one-based pages, in page order and then
/// in each page's resource order.
fn discover_images<'a>(document: &'a Document, pages: &[u32]) -> Result<Vec<ImageCandidate<'a>>, String> {
    let mut candidates = Vec::new();
    for (page_number, page_id) in document.get_pages() {
        if !pages.contains(&page_number) { continue; }
        let images = document.get_page_images(page_id).map_err(|error| error.to_string())?;
        for (index, image) in images.into_iter().enumerate() {
            let reference = |key: &[u8]| image.origin_dict.get(key).and_then(Object::as_reference).ok();
            candidates.push(ImageCandidate {
                page: page_number,
                index,
                object_id: image.id,
                soft_mask: reference(b"SMask"),
                mask: reference(b"Mask"),
                filters: image.filters,
                width: image.width,
                height: image.height,
                color_space: image.color_space,
                data: image.content,
            });
        }
    }
    Ok(candidates)
}

/// Copies the selected pages' embedded JPEG images as they are, skipping those smaller than
/// `min_size` on either side. An empty list means there were none to save.
/// On failure nothing new is published, except images already published when publishing itself fails.
fn write_images(input: &Path, document: &Document, pages: &[u32], location: &OutputLocation, min_size: Option<u32>) -> Result<Vec<PathBuf>, (Vec<PathBuf>, String)> {
    let candidates = discover_images(document, pages).map_err(|error| (Vec::new(), error))?;
    let mut reservations = Vec::new();
    for candidate in &candidates {
        if min_size.is_some_and(|min| candidate.width < i64::from(min) || candidate.height < i64::from(min)) { continue; }
        let Some(filters) = &candidate.filters else {
            return Err((Vec::new(), "PDF contains an embedded image with no supported filter; extraction stopped without a complete result.".to_string()));
        };
        if filters.iter().any(|filter| filter != "DCTDecode") {
            return Err((Vec::new(), format!("PDF image on page {} uses an unsupported filter; only original JPEG images can be extracted without recompression.", candidate.page)));
        }
        let reservation = OutputNaming::reserve_destination(input, location, &format!("-image-{}-{}", candidate.page, candidate.index), "jpg")
            .map_err(|error| (Vec::new(), format!("Could not reserve extracted image output: {error}")))?;
        fs::write(reservation.path(), candidate.data).map_err(|error| (Vec::new(), format!("Could not write extracted image: {error}")))?;
        reservations.push(reservation);
    }
    publish_extracted_images(reservations)
}

fn publish_extracted_images(reservations: Vec<OutputReservation>) -> Result<Vec<PathBuf>, (Vec<PathBuf>, String)> {
    let mut outputs = Vec::with_capacity(reservations.len());
    for reservation in reservations {
        match reservation.publish() {
            Ok(path) => outputs.push(path),
            Err(error) => return Err((outputs, format!("Could not publish extracted image: {error}"))),
        }
    }
    Ok(outputs)
}

fn selected_pdf_pages(document: &Document, requested: Option<&Vec<usize>>) -> Result<Vec<u32>, String> {
    let pages = document.get_pages().keys().copied().collect::<Vec<_>>();
    let Some(requested) = requested else { return Ok(pages); };
    if requested.is_empty() { return Ok(Vec::new()); }
    let requested = requested.iter().map(|page| (*page as u32).saturating_add(1)).collect::<std::collections::HashSet<_>>();
    if requested.iter().any(|page| !pages.contains(page)) {
        return Err("Selected PDF pages are outside the document.".to_string());
    }
    Ok(pages.into_iter().filter(|page| requested.contains(page)).collect())
}

pub fn images_to_pdf(request: &ImagesToPdfRequest) -> JobOutcome {
    let Some(first) = request.paths.first().cloned() else { return failure(PathBuf::new(), "Select at least one image.".to_string()); };
    let output = match OutputNaming::reserve_single_output(request.output_path.as_deref(), &first, &request.output_location, "-combined", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(first, format!("Could not reserve combined PDF output: {error}")),
    };
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let mut kids = Vec::new();
    let mut reduced = Vec::new();
    for path in &request.paths {
        let images = match embed_images(&mut document, path) { Ok(value) => value, Err(error) => return failure(path.clone(), format!("Could not process {}: {error}", path.display())) };
        for image in images {
            if let Some(bits) = image.reduced_from_bits { reduced.push(format!("page {} ({bits}-bit)", kids.len() + 1)); }
            let placement = request.layout.place(image.width, image.height);
            let content_id = document.add_object(Stream::new(dictionary! {}, placement.content().into_bytes()));
            let (page_width, page_height) = placement.page;
            let page_id = document.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), page_width.into(), page_height.into()], "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image.id } }, "Contents" => content_id });
            kids.push(page_id.into());
        }
    }
    let count = kids.len() as i64;
    document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }.into());
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    match document.save(output.path()) {
        Ok(_) => match output.publish() {
            Ok(path) => {
                let outcome = JobOutcome::succeeded(first, vec![path], "Images combined into PDF");
                if reduced.is_empty() { return outcome; }
                outcome.with_warning(OutcomeWarning {
                    kind: WarningKind::PrecisionReduced,
                    message: format!("Reduced to 8-bit sRGB to fit in the PDF: {}.", reduced.join(", ")),
                })
            }
            Err(error) => failure(first, format!("Could not publish combined PDF: {error}")),
        },
        Err(error) => failure(first, format!("Save failed: {error}")),
    }
}

struct EmbeddedImage {
    id: lopdf::ObjectId,
    width: u32,
    height: u32,
    reduced_from_bits: Option<u8>,
}

fn embed_image(document: &mut Document, path: &Path) -> Result<EmbeddedImage, String> {
    if !matches!(ImageFamily::of(path), ImageFamily::Jpeg | ImageFamily::Png | ImageFamily::WebP | ImageFamily::Heif) {
        return Err("Only PNG, JPEG, WebP, HEIC and HEIF images can be combined into a PDF.".to_string());
    }
    let mut images = embed_images(document, path)?;
    if images.len() != 1 { return Err("Multi-page images can't be combined into a PDF.".to_string()); }
    Ok(images.swap_remove(0))
}

// A baseline JPEG goes in as its own bytes; everything else is stored without
// loss as 8-bit Flate samples, with any alpha kept as a soft mask. A TIFF
// gives one image per page, in page order.
fn embed_images(document: &mut Document, path: &Path) -> Result<Vec<EmbeddedImage>, String> {
    if !matches!(ImageFamily::of(path), ImageFamily::Jpeg | ImageFamily::Png | ImageFamily::WebP | ImageFamily::Heif | ImageFamily::Tiff) {
        return Err("Only PNG, JPEG, WebP, HEIC, HEIF and TIFF images can be combined into a PDF.".to_string());
    }
    let decoded = source::decode(path)?;
    let reduced_from_bits = (decoded.bits_per_channel > 8).then_some(decoded.bits_per_channel);
    let baseline_jpeg = decoded.baseline_jpeg.and_then(|bytes| jpeg_color_space(&bytes).map(|space| (bytes, space)));
    let mut images = Vec::with_capacity(decoded.pages.len());
    for page in &decoded.pages {
        let (width, height) = (page.width(), page.height());
        let image = dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64, "Height" => height as i64, "BitsPerComponent" => 8 };
        if let Some((jpeg, color_space)) = &baseline_jpeg {
            let mut dict = image;
            dict.set("ColorSpace", *color_space);
            dict.set("Filter", "DCTDecode");
            images.push(EmbeddedImage { id: document.add_object(Stream::new(dict, jpeg.clone())), width, height, reduced_from_bits });
            continue;
        }
        let (samples, color_space) = if page.color().has_color() { (page.to_rgb8().into_raw(), "DeviceRGB") } else { (page.to_luma8().into_raw(), "DeviceGray") };
        let mut dict = image;
        dict.set("ColorSpace", color_space);
        if page.color().has_alpha() {
            let alpha = page.to_rgba8().pixels().map(|pixel| pixel[3]).collect::<Vec<_>>();
            let mask = dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64, "Height" => height as i64, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 };
            dict.set("SMask", document.add_object(flate_stream(mask, &alpha)?));
        }
        images.push(EmbeddedImage { id: document.add_object(flate_stream(dict, &samples)?), width, height, reduced_from_bits });
    }
    Ok(images)
}

fn flate_stream(mut dict: lopdf::Dictionary, samples: &[u8]) -> Result<Stream, String> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(samples).map_err(|error| format!("Could not compress image: {error}"))?;
    dict.set("Filter", "FlateDecode");
    Ok(Stream::new(dict, encoder.finish().map_err(|error| format!("Could not compress image: {error}"))?).with_compression(false))
}

// The component count in the SOF0 header picks the PDF color space; a
// four-component (CMYK) JPEG is stored losslessly instead.
fn jpeg_color_space(bytes: &[u8]) -> Option<&'static str> {
    let mut at = 2;
    while let Some(&[0xFF, marker, high, low]) = bytes.get(at..at + 4) {
        match marker {
            0xFF => at += 1,
            0xC0 => return match bytes.get(at + 9)? { 1 => Some("DeviceGray"), 3 => Some("DeviceRGB"), _ => None },
            _ => at += 2 + usize::from(u16::from_be_bytes([high, low])),
        }
    }
    None
}

pub fn add_page_numbers(request: &PageOverlayRequest, input: PathBuf) -> JobOutcome {
    overlay(request.pages.as_deref(), &request.output_location, input, "-numbered", |page_number, page, font_name, _state_name| {
        let number = request.start_number.unwrap_or(1).saturating_add(page_number as u32).saturating_sub(1);
        let size = request.font_size.unwrap_or(12).clamp(8, 72);
        let (x, y) = number_position(request.position.as_deref(), page.display_width, page.display_height);
        let (x, y) = page_point_from_display(page, x, y);
        format!("BT /{font_name} {size} Tf {x} {y} Td ({number}) Tj ET")
    }, 100)
}

pub fn watermark(request: &PdfWatermarkRequest, input: PathBuf) -> JobOutcome {
    let options = &request.options;
    if let Err(error) = options.validate() { return failure(input, error); }
    if let Some(path) = &options.logo_path { return watermark_image(request, input, path); }
    let text = escape(options.text().unwrap_or_default());
    overlay(request.pages.as_deref(), &request.output_location, input, "-watermarked", |_, page, font_name, _state_name| {
        let (x, y) = watermark_position(options.position, page.display_width, page.display_height);
        let (x, y) = page_point_from_display(page, x, y);
        format!("BT /{font_name} 48 Tf {x} {y} Td ({text}) Tj ET")
    }, options.opacity.clamp(1, 100))
}

pub fn compress(request: &CompressPdfRequest, input: PathBuf) -> JobOutcome {
    let source = match load_existing_document_for_mutation(&input) { Ok(document) => document, Err(error) => return failure(input, error) };
    let page_count = source.get_pages().len();
    let qpdf = match qpdf() { Ok(qpdf) => qpdf, Err(error) => return JobOutcome::failure(input, ToolError::unavailable(error)) };
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-compressed", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve compressed PDF output: {error}")),
    };
    let quality = request.quality.clamp(1, 100);
    let level = ((100_u16.saturating_sub(quality as u16) * 8) / 99 + 1).to_string();
    let result = Command::new(qpdf).arg("--object-streams=generate").arg("--stream-data=compress").arg("--recompress-flate").arg(format!("--compression-level={level}")).arg(&input).arg(output.path()).output_until_stopped();
    match result {
        Ok(result) if result.status.success() => match Document::load(output.path()) {
            Ok(verified) if verified.get_pages().len() == page_count => match output.publish() {
                Ok(path) => JobOutcome::succeeded(input, vec![path], format!("PDF compressed at quality {quality}")),
                Err(error) => failure(input, format!("Could not publish compressed PDF: {error}")),
            },
            Ok(_) => failure(input, "Compressed PDF changed its page count.".to_string()),
            Err(error) => failure(input, format!("Compressed PDF could not be verified: {error}")),
        },
        Ok(result) => failure(input, stderr(result, "qpdf failed to compress the PDF.")),
        Err(error) => failure(input, format!("Could not run qpdf: {error}")),
    }
}

pub fn rotate(request: &RotatePdfRequest, input: PathBuf) -> JobOutcome {
    let degrees = request.degrees.rem_euclid(360) as i64;
    if ![90, 180, 270].contains(&degrees) { return failure(input, "Choose a rotation of 90, 180, or 270 degrees.".to_string()); }
    let mut document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    if document.is_encrypted() { return failure(input, "Unlock the PDF before rotating it.".to_string()); }
    let pages = document.get_pages().values().copied().collect::<Vec<_>>();
    if let Some(selected) = &request.pages {
        if selected.is_empty() { return failure(input, "Select at least one page to rotate.".to_string()); }
        if selected.iter().any(|page| *page >= pages.len()) { return failure(input, "A page to rotate is outside the document.".to_string()); }
    }
    let mut rotated = 0;
    for (index, page_id) in pages.into_iter().enumerate() {
        if request.pages.as_ref().is_some_and(|selected| !selected.contains(&index)) { continue; }
        let rotation = (page_rotation(&document, page_id) + degrees).rem_euclid(360);
        match document.get_dictionary_mut(page_id) { Ok(page) => page.set("Rotate", rotation), Err(error) => return failure(input, error.to_string()) }
        rotated += 1;
    }
    let noun = if rotated == 1 { "page" } else { "pages" };
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-rotated", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve rotated PDF output: {error}")),
    };
    save(document, input, output, &format!("{rotated} {noun} rotated {degrees}°"))
}

pub fn repair(request: &RepairPdfRequest, input: PathBuf) -> JobOutcome {
    let qpdf = match qpdf() { Ok(qpdf) => qpdf, Err(error) => return JobOutcome::failure(input, ToolError::unavailable(error)) };
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-repaired", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve repaired PDF output: {error}")),
    };
    let result = match Command::new(qpdf).arg(&input).arg(output.path()).output() { Ok(result) => result, Err(error) => return failure(input, format!("Could not run qpdf: {error}")) };
    // qpdf exits 3 when it had to recover from damage but still wrote a usable file.
    let detail = match result.status.code() {
        Some(0) => "No damage found; saved a clean copy",
        Some(3) => "PDF repaired",
        _ => return failure(input, format!("This PDF is too damaged to repair. {}", stderr(result, ""))),
    };
    match Document::load(output.path()) {
        Ok(document) if !document.get_pages().is_empty() => match output.publish() {
            Ok(path) => JobOutcome::succeeded(input, vec![path], detail),
            Err(error) => failure(input, format!("Could not publish repaired PDF: {error}")),
        },
        Ok(_) => failure(input, "The repaired PDF has no readable pages.".to_string()),
        Err(error) => failure(input, format!("The repaired PDF could not be opened: {error}")),
    }
}

/// Line-level differences between two PDFs' selectable text, page by page, written as a text report.
pub fn compare(request: &ComparePdfsRequest) -> JobOutcome {
    let [first, second] = request.paths.as_slice() else {
        return failure(request.paths.first().cloned().unwrap_or_default(), "Select exactly two PDFs to compare.".to_string());
    };
    let first_pages = match page_texts(first) { Ok(pages) => pages, Err(error) => return failure(first.clone(), error) };
    let second_pages = match page_texts(second) { Ok(pages) => pages, Err(error) => return failure(first.clone(), error) };
    if first_pages.iter().chain(&second_pages).all(|lines| lines.is_empty()) {
        return failure(first.clone(), "Neither PDF has selectable text. Run OCR on scanned PDFs first.".to_string());
    }
    let name = |path: &PathBuf| path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let mut report = format!("Original: {} (lines marked -)\nNew version: {} (lines marked +)\n", name(first), name(second));
    let mut changed_pages = 0;
    for index in 0..first_pages.len().max(second_pages.len()) {
        let section = match (first_pages.get(index), second_pages.get(index)) {
            (Some(old), Some(new)) => diff_lines(old, new),
            (Some(_), None) => vec![format!("Only in {}", name(first))],
            (None, Some(_)) => vec![format!("Only in {}", name(second))],
            (None, None) => vec![],
        };
        if section.is_empty() { continue; }
        changed_pages += 1;
        report.push_str(&format!("\nPage {}\n{}\n", index + 1, section.join("\n")));
    }
    let detail = if changed_pages == 0 { report.push_str("\nNo text differences found.\n"); "No text differences found".to_string() } else { format!("{changed_pages} page(s) with differences") };
    let output = match OutputNaming::reserve_single_output(request.output_path.as_deref(), first, &request.output_location, "-comparison", "txt") {
        Ok(output) => output,
        Err(error) => return failure(first.clone(), format!("Could not reserve comparison report: {error}")),
    };
    if let Err(error) = fs::write(output.path(), report) { return failure(first.clone(), format!("Could not write comparison report: {error}")); }
    match output.publish() {
        Ok(path) => JobOutcome::succeeded(first.clone(), vec![path], detail),
        Err(error) => failure(first.clone(), format!("Could not publish comparison report: {error}")),
    }
}

fn page_texts(path: &PathBuf) -> Result<Vec<Vec<String>>, String> {
    let document = Document::load(path).map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    Ok(document.get_pages().keys().map(|page| {
        let text = document.extract_text(&[*page]).unwrap_or_default();
        normalize_pdf_text(&text).lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string).collect()
    }).collect())
}

/// Longest-common-subsequence line diff; very long pages fall back to listing both sides.
fn diff_lines(old: &[String], new: &[String]) -> Vec<String> {
    if old == new { return vec![]; }
    if old.len() * new.len() > 4_000_000 {
        return old.iter().map(|line| format!("- {line}")).chain(new.iter().map(|line| format!("+ {line}"))).collect();
    }
    let mut lengths = vec![vec![0u32; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lengths[i][j] = if old[i] == new[j] { lengths[i + 1][j + 1] + 1 } else { lengths[i + 1][j].max(lengths[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut lines) = (0, 0, Vec::new());
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] { i += 1; j += 1; }
        else if i < old.len() && (j == new.len() || lengths[i + 1][j] >= lengths[i][j + 1]) { lines.push(format!("- {}", old[i])); i += 1; }
        else { lines.push(format!("+ {}", new[j])); j += 1; }
    }
    lines
}

pub fn extract_pages(request: &PageSelectionRequest, input: PathBuf) -> JobOutcome {
    let mut document = match load_existing_document_for_mutation(&input) { Ok(document) => document, Err(error) => return failure(input, error) };
    let pages = document.get_pages();
    let keep = match selected_page_indices(request, pages.len()) { Ok(selected) => selected.into_iter().collect::<std::collections::BTreeSet<_>>(), Err(error) => return failure(input, error) };
    let delete = (0..pages.len()).filter(|page| !keep.contains(page)).map(|page| page as u32 + 1).collect::<Vec<_>>();
    if keep.is_empty() { return failure(input, "Select at least one page.".to_string()); }
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-extracted", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve page extraction output: {error}")),
    };
    document.delete_pages(&delete);
    save(document, input, output, "PDF pages extracted")
}

fn selected_page_indices(request: &PageSelectionRequest, page_count: usize) -> Result<Vec<usize>, String> {
    let Some(raw) = request.page_ranges.as_deref().filter(|value| !value.trim().is_empty()) else {
        if request.pages.is_empty() { return Err("Enter at least one page or range, for example 1-3, 7.".to_string()); }
        if request.pages.iter().any(|page| *page >= page_count) { return Err("A selected page is outside the document.".to_string()); }
        return Ok(request.pages.clone());
    };
    let pages = PageRanges::parse(raw)?.within(page_count)?;
    Ok(pages.into_iter().flat_map(|(first, last)| first - 1..last).collect::<std::collections::BTreeSet<_>>().into_iter().collect())
}

fn save(mut document: Document, input: PathBuf, output: OutputReservation, detail: &str) -> JobOutcome {
    match document.save(output.path()) {
        Ok(_) => match output.publish() {
            Ok(path) => JobOutcome::succeeded(input, vec![path], detail.to_string()),
            Err(error) => failure(input, format!("Could not publish PDF output: {error}")),
        },
        Err(error) => failure(input, format!("Save failed: {error}")),
    }
}

#[derive(Debug, Clone, Copy)]
struct OverlayPageGeometry {
    left: f32,
    bottom: f32,
    width: f32,
    height: f32,
    rotation: i32,
    display_width: f32,
    display_height: f32,
}

fn overlay_page_geometry(document: &Document, page_id: lopdf::ObjectId) -> Result<OverlayPageGeometry, String> {
    let (left, bottom, right, top) = page_bounds(document, page_id)?;
    let rotation = super::inherited(document, page_id, b"Rotate")?
        .map(|value| value.as_i64().map_err(|error| error.to_string()))
        .transpose()?
        .unwrap_or(0)
        .rem_euclid(360) as i32;
    if rotation % 90 != 0 { return Err("PDF page rotation must be a multiple of 90 degrees".to_string()); }
    let width = right - left;
    let height = top - bottom;
    let (display_width, display_height) = if matches!(rotation, 90 | 270) { (height, width) } else { (width, height) };
    Ok(OverlayPageGeometry { left, bottom, width, height, rotation, display_width, display_height })
}

fn page_point_from_display(page: OverlayPageGeometry, x: f32, y: f32) -> (f32, f32) {
    let (x, y) = match page.rotation {
        0 => (x, y),
        90 => (page.width - y, x),
        180 => (page.width - x, page.height - y),
        _ => (y, page.height - x),
    };
    (page.left + x, page.bottom + y)
}

fn image_origin_from_display(page: OverlayPageGeometry, x: f32, y: f32, image_width: f32, image_height: f32) -> (f32, f32) {
    let (x, y) = match page.rotation {
        0 => (x, y),
        90 => (page.width - image_width - y, x),
        180 => (page.width - image_width - x, page.height - image_height - y),
        _ => (y, page.height - image_height - x),
    };
    (page.left + x, page.bottom + y)
}

fn overlay<F>(pages: Option<&[usize]>, output_location: &OutputLocation, input: PathBuf, suffix: &str, content: F, opacity: u8) -> JobOutcome
where F: Fn(usize, OverlayPageGeometry, &str, &str) -> String {
    let mut document = match load_existing_document_for_mutation(&input) { Ok(document) => document, Err(error) => return failure(input, error) };
    if let Err(error) = validate_overlay_scope(pages, document.get_pages().len()) { return failure(input, error); }
    let output = match OutputNaming::reserve_destination(&input, output_location, suffix, "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve PDF overlay output: {error}")),
    };
    let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    let opacity_id = document.add_object(dictionary! { "Type" => "ExtGState", "ca" => opacity as f32 / 100.0, "CA" => opacity as f32 / 100.0 });
    for (number, page_id) in document.get_pages().values().copied().enumerate() {
        if pages.is_some_and(|pages| !pages.contains(&number)) { continue; }
        let page = match overlay_page_geometry(&document, page_id) {
            Ok(page) => page,
            Err(error) => return failure(input, error),
        };
        if page.width <= 0.0 || page.height <= 0.0 { return failure(input, "PDF page has invalid dimensions".to_string()); }
        let font_name = match super::editor::add_resource(&mut document, page_id, "Font", "Fnum", Object::Reference(font_id)) { Ok(name) => name, Err(error) => return failure(input, error) };
        let state_name = match super::editor::add_resource(&mut document, page_id, "ExtGState", "GSwm", Object::Reference(opacity_id)) { Ok(name) => name, Err(error) => return failure(input, error) };
        if let Err(error) = document.add_page_contents(page_id, format!("q /{state_name} gs {} Q", content(number + 1, page, &font_name, &state_name)).into_bytes()) { return failure(input, error.to_string()); }
    }
    match document.save(output.path()) {
        Ok(_) => match output.publish() {
            Ok(path) => JobOutcome::succeeded(input, vec![path], "PDF saved"),
            Err(error) => failure(input, format!("Could not publish PDF output: {error}")),
        },
        Err(error) => failure(input, format!("Save failed: {error}")),
    }
}

fn watermark_position(position: Option<WatermarkPosition>, width: f32, height: f32) -> (f32, f32) {
    let Some(WatermarkPosition::Placement(placement)) = position else { return (width / 4.0, height / 2.0) };
    match placement {
        WatermarkPlacement::TopLeft => (24.0, height - 48.0), WatermarkPlacement::TopRight => (width - 180.0, height - 48.0),
        WatermarkPlacement::BottomLeft => (24.0, 24.0), WatermarkPlacement::BottomRight => (width - 180.0, 24.0),
        _ => (width / 4.0, height / 2.0),
    }
}

fn number_position(position: Option<&str>, width: f32, height: f32) -> (f32, f32) {
    match position.unwrap_or("bottom-right") {
        "bottom-left" => (24.0, 24.0), "top-left" => (24.0, height - 24.0), "top-right" => (width - 60.0, height - 24.0),
        _ => (width - 60.0, 24.0),
    }
}

fn watermark_image(request: &PdfWatermarkRequest, input: PathBuf, logo: &PathBuf) -> JobOutcome {
    let mut document = match load_existing_document_for_mutation(&input) { Ok(document) => document, Err(error) => return failure(input, error) };
    if let Err(error) = validate_overlay_scope(request.pages.as_deref(), document.get_pages().len()) { return failure(input, error); }
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-watermarked", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve watermarked PDF output: {error}")),
    };
    let logo_image = match embed_image(&mut document, logo) { Ok(value) => value, Err(error) => return failure(input, error) };
    let (image_id, image_width, image_height) = (logo_image.id, logo_image.width, logo_image.height);
    let state_id = document.add_object(dictionary! { "Type" => "ExtGState", "ca" => request.options.opacity.clamp(1, 100) as f32 / 100.0, "CA" => request.options.opacity.clamp(1, 100) as f32 / 100.0 });
    for (number, page_id) in document.get_pages().values().copied().enumerate() {
        if request.pages.as_ref().is_some_and(|pages| !pages.contains(&number)) { continue; }
        let image_name = match super::editor::add_resource(&mut document, page_id, "XObject", "Iwm", Object::Reference(image_id)) { Ok(name) => name, Err(error) => return failure(input, error) };
        let state_name = match super::editor::add_resource(&mut document, page_id, "ExtGState", "GSwm", Object::Reference(state_id)) { Ok(name) => name, Err(error) => return failure(input, error) };
        let page = match overlay_page_geometry(&document, page_id) {
            Ok(page) => page,
            Err(error) => return failure(input, error),
        };
        let draw_width = image_width.min(180) as f32;
        let draw_height = image_height.min(100) as f32;
        let (x, y) = watermark_position(request.options.position, page.display_width, page.display_height);
        let (x, y) = image_origin_from_display(page, x, y, draw_width, draw_height);
        if let Err(error) = document.add_page_contents(page_id, format!("q /{state_name} gs {} 0 0 {} {} {} cm /{image_name} Do Q", draw_width, draw_height, x, y).into_bytes()) { return failure(input, error.to_string()); }
    }
    match document.save(output.path()) {
        Ok(_) => match output.publish() {
            Ok(path) => JobOutcome::succeeded(input, vec![path], "PDF image watermark applied"),
            Err(error) => failure(input, format!("Could not publish watermarked PDF: {error}")),
        },
        Err(error) => failure(input, format!("Save failed: {error}")),
    }
}

fn validate_overlay_scope(pages: Option<&[usize]>, page_count: usize) -> Result<(), String> {
    if let Some(pages) = pages {
        if pages.is_empty() { return Err("Select at least one page for the watermark.".to_string()); }
        if pages.iter().any(|page| *page >= page_count) { return Err("A watermark page is outside the document.".to_string()); }
    }
    Ok(())
}

pub const PASSWORD_PROTECTED: &str = "Password-protected, so it can't be read. Remove the password, then add it again.";

/// What Merge and Split learn from reading an input. They copy pages into a new file and never edit the input,
/// so they only need it to open without a password and to have pages. Anything the new file can't keep is noted on the result.
struct ReadableInput {
    pages: usize,
    signed: bool,
    bookmarks: bool,
    tags: bool,
}

impl ReadableInput {
    fn inspect(path: &Path) -> Result<Self, ToolError> {
        let document = Document::load(path).map_err(|error| ToolError::processing(format!("Could not read {}: {error}", path.display())))?;
        if document.is_encrypted() {
            return Err(ToolError::invalid_input(PASSWORD_PROTECTED).with_pointer(OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
        }
        let pages = document.get_pages().len();
        if pages == 0 {
            return Err(ToolError::invalid_input("Has no pages, so there's nothing to take from it. Nothing was saved."));
        }
        let catalog = document.catalog().ok();
        let entry = |key: &[u8]| catalog.and_then(|catalog| resolve(&document, catalog.get(key).ok()?).ok()?.as_dict().ok());
        Ok(Self {
            pages,
            signed: has_signed_signature_field(&document),
            bookmarks: entry(b"Outlines").is_some_and(|outlines| outlines.has(b"First")),
            tags: entry(b"StructTreeRoot").is_some(),
        })
    }

    fn note_what_was_not_kept(&self, outcome: JobOutcome, into: &str) -> JobOutcome {
        let mut outcome = note_signature_removed(outcome, self.signed);
        if self.bookmarks {
            outcome = outcome.with_warning(OutcomeWarning { kind: WarningKind::BookmarksNotCarried { into: into.into() }, message: format!("Bookmarks weren't carried into {into}.") });
        }
        if self.tags {
            outcome = outcome.with_warning(OutcomeWarning { kind: WarningKind::TagsNotCarried { into: into.into() }, message: format!("Accessibility tags weren't carried into {into}.") });
        }
        outcome
    }
}

/// qpdf keeps signature fields when it copies pages, but their signatures no longer match the new file, so the copy drops them.
fn unsign_copy(mut document: Document, path: &Path) -> Result<(), String> {
    if !has_signed_signature_field(&document) {
        return Ok(());
    }
    remove_signatures(&mut document)?;
    document.save(path).map(|_| ()).map_err(|error| format!("Could not save the unsigned copy: {error}"))
}

fn load_existing_document_for_mutation(input: &Path) -> Result<Document, String> {
    let document = Document::load(input).map_err(|error| error.to_string())?;
    mutation_preflight(&document, PdfMutationIntent::General)?;
    Ok(document)
}

fn escape(text: &str) -> String { text.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)") }
fn qpdf() -> Result<PathBuf, String> { crate::kit::resources::resolve_helper(&crate::kit::resources::QPDF) }
pub(super) fn pdftoppm() -> Option<PathBuf> { tool("TOOLBOX_PDFTOPPM_PATH", "pdftoppm") }
fn tool(variable: &str, command: &str) -> Option<PathBuf> {
    std::env::var_os(variable).map(PathBuf::from).filter(|path| !path.as_os_str().is_empty() && path.is_file())
        .or_else(|| crate::kit::resources::application_resource_root().and_then(|root| [root.join("pdf-bin").join(command), root.join("resources").join(command), root.join(command)].into_iter().find(|path| path.is_file())))
        .or_else(|| Command::new(command).arg("-h").output().ok().map(|_| PathBuf::from(command)))
}
fn stderr(result: std::process::Output, fallback: &str) -> String { String::from_utf8_lossy(&result.stderr).trim().lines().last().filter(|line| !line.is_empty()).unwrap_or(fallback).to_string() }
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::processing(error)) }

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::codecs::jpeg::JpegEncoder;
    use image::DynamicImage;
    use std::io::Cursor;

    /// Held by tests that create or count split workspaces in the shared temporary directory.
    pub(crate) static SPLIT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn watermark_request(input: &Path, text: &str, logo_path: Option<PathBuf>, position: Option<WatermarkPlacement>, output_location: OutputLocation) -> PdfWatermarkRequest {
        PdfWatermarkRequest {
            paths: vec![input.to_path_buf()],
            options: WatermarkOptions { text: Some(text.to_string()), logo_path, size: None, opacity: 70, color: None, position: position.map(WatermarkPosition::Placement), rotation: 0.0 },
            pages: None,
            output_location,
        }
    }

    fn request(page_ranges: Option<&str>, pages: Vec<usize>) -> PageSelectionRequest {
        PageSelectionRequest {
            paths: vec![],
            pages,
            page_ranges: page_ranges.map(str::to_string),
            output_location: OutputLocation::AlongsideInput,
        }
    }

    fn split_request(output_location: OutputLocation, mode: SplitMode) -> SplitPdfRequest {
        SplitPdfRequest { paths: vec![], mode: mode.into(), output_location }
    }

    fn ranges(ranges: &str) -> SplitMode { SplitMode::Ranges { ranges: ranges.to_string() } }

    fn split_fixture_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "toolbox_split_regression_{}_{}_{}",
            std::process::id(),
            name,
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn split_fixture_pdf(path: &PathBuf, page_count: usize) {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let mut kids = Vec::new();
        for number in 0..page_count {
            let contents_id = document.add_object(Stream::new(dictionary! {}, format!("BT /F1 24 Tf 72 720 Td (Split page {}) Tj ET", number + 1).into_bytes()));
            let page_id = document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![Object::Integer(0), Object::Integer(0), Object::Integer(612), Object::Integer(792)],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
                "Contents" => contents_id,
            });
            kids.push(Object::Reference(page_id));
        }
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => page_count as i64 }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    fn unsafe_mutation_fixture(path: &Path, kind: &str) {
        split_fixture_pdf(&path.to_path_buf(), 2);
        let mut document = Document::load(path).unwrap();
        let catalog_id = document.trailer.get(b"Root").unwrap().as_reference().unwrap();
        match kind {
            "signed" => {
                let signature = document.add_object(dictionary! { "Type" => "Sig" });
                document.get_dictionary_mut(catalog_id).unwrap().set("Perms", dictionary! { "DocMDP" => signature });
            }
            "outline" => {
                let outlines = document.add_object(dictionary! { "Type" => "Outlines", "Count" => 0 });
                document.get_dictionary_mut(catalog_id).unwrap().set("Outlines", outlines);
            }
            "tagged" => {
                let structure = document.add_object(dictionary! { "Type" => "StructTreeRoot", "K" => vec![] });
                document.get_dictionary_mut(catalog_id).unwrap().set("StructTreeRoot", structure);
            }
            _ => panic!("unknown unsafe fixture kind: {kind}"),
        }
        document.save(path).unwrap();
    }

    fn inherited_overlay_fixture(path: &Path) {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let fonts_id = document.add_object(dictionary! { "F1" => font_id });
        let resources_id = document.add_object(dictionary! { "Font" => Object::Reference(fonts_id) });
        let content = document.add_object(Stream::new(dictionary! {}, b"BT /F1 18 Tf 72 720 Td (Existing content) Tj ET".to_vec()));
        let inherited_page = document.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => content });
        let direct_resources = document.add_object(dictionary! { "Font" => Object::Reference(fonts_id) });
        let direct_page = document.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Resources" => direct_resources, "Contents" => content });
        document.objects.insert(pages_id, dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference(inherited_page), Object::Reference(direct_page)], "Count" => 2,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Resources" => Object::Reference(resources_id),
        }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    fn colliding_overlay_fixture(path: &Path) {
        inherited_overlay_fixture(path);
        let mut document = Document::load(path).unwrap();
        let page_ids = document.get_pages().values().copied().collect::<Vec<_>>();
        let pages_id = document.get_dictionary(page_ids[0]).unwrap().get(b"Parent").unwrap().as_reference().unwrap();
        let resources_id = document.get_dictionary(pages_id).unwrap().get(b"Resources").unwrap().as_reference().unwrap();
        let image_id = document.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8 }, vec![0, 0, 0]));
        let state_id = document.add_object(dictionary! { "Type" => "ExtGState", "ca" => 0.25, "CA" => 0.25 });
        let fonts_id = document.get_dictionary(resources_id).unwrap().get(b"Font").unwrap().as_reference().unwrap();
        let original_font = document.get_dictionary(fonts_id).unwrap().get(b"F1").unwrap().clone();
        let fonts = document.get_dictionary_mut(fonts_id).unwrap();
        for name in ["Fedit", "Fnum", "Fsig", "Fwm"] { fonts.set(name, original_font.clone()); }
        {
            let resources = document.get_dictionary_mut(resources_id).unwrap();
            resources.set("XObject", dictionary! { "Iwm" => image_id, "Isig" => image_id });
            resources.set("ExtGState", dictionary! { "GSwm" => state_id });
        }
        let direct_resources_id = document.get_dictionary(page_ids[1]).unwrap().get(b"Resources").unwrap().as_reference().unwrap();
        let direct_resources = document.get_dictionary_mut(direct_resources_id).unwrap();
        direct_resources.set("XObject", dictionary! { "Iwm" => image_id, "Isig" => image_id });
        direct_resources.set("ExtGState", dictionary! { "GSwm" => state_id });
        document.save(path).unwrap();
    }

    fn offset_rotated_overlay_fixture(path: &Path) {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let resources_id = document.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
        let content = document.add_object(Stream::new(dictionary! {}, Vec::new()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Rotate" => 90,
            "Resources" => Object::Reference(resources_id), "Contents" => content,
        });
        document.objects.insert(pages_id, dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1,
            "MediaBox" => vec![10.into(), 20.into(), 250.into(), 360.into()],
        }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    #[test]
    fn legacy_overlays_merge_inherited_and_indirect_resources_without_losing_content() {
        let cases = ["numbers", "watermark", "image-watermark"];
        for case in cases {
            let root = split_fixture_root(&format!("overlay-resources-{case}"));
            let input = root.join("document.pdf");
            let output = root.join("outputs");
            fs::create_dir_all(&output).unwrap();
            inherited_overlay_fixture(&input);
            let logo = root.join("logo.jpg");
            if case == "image-watermark" {
                let mut jpeg = Cursor::new(Vec::new());
                JpegEncoder::new(&mut jpeg).encode_image(&DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0])))).unwrap();
                fs::write(&logo, jpeg.into_inner()).unwrap();
            }
            let outcome = if case == "numbers" {
                add_page_numbers(&PageOverlayRequest {
                    paths: vec![input.clone()], position: Some("center".into()),
                    pages: None, start_number: None,
                    font_size: None, output_location: OutputLocation::CustomFolder(output),
                }, input.clone())
            } else {
                watermark(&watermark_request(&input, "TEST", (case == "image-watermark").then_some(logo), Some(WatermarkPlacement::Center), OutputLocation::CustomFolder(output)), input.clone())
            };
            assert!(outcome.error().is_none(), "{case}: {:?}", outcome.error());
            let output_document = Document::load(outcome.output_paths().first().unwrap()).unwrap();
            for page_id in output_document.get_pages().values().copied() {
                let page = output_document.get_dictionary(page_id).unwrap();
                let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
                let fonts = super::super::resolve(&output_document, resources.get(b"Font").unwrap()).unwrap().as_dict().unwrap();
                assert!(fonts.has(b"F1"));
                let content_bytes = output_document.get_page_content(page_id);
                let content = String::from_utf8_lossy(&content_bytes);
                assert!(content.contains("Existing content"));
                if case == "numbers" { assert!(fonts.has(b"Fnum")); }
                if case == "watermark" { assert!(fonts.has(b"Fnum")); assert!(super::super::resolve(&output_document, resources.get(b"ExtGState").unwrap()).unwrap().as_dict().unwrap().has(b"GSwm")); }
                if case == "image-watermark" { assert!(super::super::resolve(&output_document, resources.get(b"XObject").unwrap()).unwrap().as_dict().unwrap().has(b"Iwm")); assert!(super::super::resolve(&output_document, resources.get(b"ExtGState").unwrap()).unwrap().as_dict().unwrap().has(b"GSwm")); }
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn legacy_overlays_allocate_collision_free_names_and_preserve_original_resources() {
        let cases = ["numbers", "watermark", "image-watermark"];
        for case in cases {
            let root = split_fixture_root(&format!("overlay-resource-collisions-{case}"));
            let input = root.join("document.pdf");
            let output = root.join("outputs");
            fs::create_dir_all(&output).unwrap();
            colliding_overlay_fixture(&input);
            let logo = root.join("logo.jpg");
            if case == "image-watermark" {
                let mut jpeg = Cursor::new(Vec::new());
                JpegEncoder::new(&mut jpeg).encode_image(&DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0])))).unwrap();
                fs::write(&logo, jpeg.into_inner()).unwrap();
            }
            let outcome = if case == "numbers" {
                add_page_numbers(&PageOverlayRequest {
                    paths: vec![input.clone()], position: Some("center".into()),
                    pages: None, start_number: None,
                    font_size: None, output_location: OutputLocation::CustomFolder(output),
                }, input.clone())
            } else {
                watermark(&watermark_request(&input, "TEST", (case == "image-watermark").then_some(logo), Some(WatermarkPlacement::Center), OutputLocation::CustomFolder(output)), input.clone())
            };
            assert!(outcome.error().is_none(), "{case}: {:?}", outcome.error());
            let output_document = Document::load(outcome.output_paths().first().unwrap()).unwrap();
            for page_id in output_document.get_pages().values().copied() {
                let page = output_document.get_dictionary(page_id).unwrap();
                let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
                let fonts = super::super::resolve(&output_document, resources.get(b"Font").unwrap()).unwrap().as_dict().unwrap();
                assert!(fonts.has(b"Fnum"));
                if case != "image-watermark" { assert!(fonts.has(b"Fnum1"), "{case}: {:?}", fonts); }
                assert!(fonts.has(b"Fedit"));
                assert!(fonts.has(b"Fsig"));
                assert!(fonts.has(b"Fwm"));
                let content_bytes = output_document.get_page_content(page_id);
                let content = String::from_utf8_lossy(&content_bytes);
                assert!(content.contains("Existing content"));
                if case == "numbers" || case == "watermark" {
                    assert!(content.contains("/Fnum1"));
                    assert!(content.contains("/GSwm1"), "{case}: {content}");
                }
                if case == "image-watermark" {
                    assert!(content.contains("/Iwm1"));
                    assert!(content.contains("/GSwm1"));
                }
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn legacy_page_numbers_and_logo_watermarks_use_offset_rotated_page_coordinates() {
        let root = split_fixture_root("offset-rotated-overlay-coordinates");
        let input = root.join("document.pdf");
        let output = root.join("outputs");
        fs::create_dir_all(&output).unwrap();
        offset_rotated_overlay_fixture(&input);

        let numbered = add_page_numbers(&PageOverlayRequest {
            paths: vec![input.clone()],
            position: Some("bottom-right".into()), pages: None,
            start_number: None, font_size: None, output_location: OutputLocation::CustomFolder(output.clone()),
        }, input.clone());
        assert!(numbered.error().is_none(), "page numbers failed: {:?}", numbered.error());
        let numbered_document = Document::load(numbered.output_paths().first().unwrap()).unwrap();
        let numbered_page = numbered_document.get_pages().values().next().copied().unwrap();
        let numbered_content = String::from_utf8_lossy(&numbered_document.get_page_content(numbered_page)).into_owned();
        assert!(numbered_content.contains("226 300 Td"), "number coordinates: {numbered_content}");

        let logo = root.join("logo.jpg");
        let mut jpeg = Cursor::new(Vec::new());
        JpegEncoder::new(&mut jpeg).encode_image(&DynamicImage::ImageRgb8(image::RgbImage::from_pixel(40, 30, image::Rgb([255, 0, 0])))).unwrap();
        fs::write(&logo, jpeg.into_inner()).unwrap();
        let watermarked = watermark(&watermark_request(&input, "", Some(logo), Some(WatermarkPlacement::BottomRight), OutputLocation::CustomFolder(output)), input.clone());
        assert!(watermarked.error().is_none(), "logo watermark failed: {:?}", watermarked.error());
        let watermarked_document = Document::load(watermarked.output_paths().first().unwrap()).unwrap();
        let watermarked_page = watermarked_document.get_pages().values().next().copied().unwrap();
        let watermarked_content = String::from_utf8_lossy(&watermarked_document.get_page_content(watermarked_page)).into_owned();
        assert!(watermarked_content.contains("40 0 0 30 186 180 cm"), "logo coordinates: {watermarked_content}");

        for output in numbered.output_paths().into_iter().chain(watermarked.output_paths()) { let _ = fs::remove_file(output); }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_transparent_png_logo_watermark_keeps_its_alpha_as_a_soft_mask() {
        let root = split_fixture_root("transparent-logo-watermark");
        let input = root.join("document.pdf");
        let output = root.join("outputs");
        fs::create_dir_all(&output).unwrap();
        offset_rotated_overlay_fixture(&input);
        let logo = root.join("logo.png");
        let pixels = image::RgbaImage::from_fn(40, 30, |x, _| image::Rgba([255, 0, 0, if x < 20 { 0 } else { 255 }]));
        pixels.save(&logo).unwrap();
        let original = fs::read(&logo).unwrap();

        let watermarked = watermark(&watermark_request(&input, "", Some(logo.clone()), Some(WatermarkPlacement::BottomRight), OutputLocation::CustomFolder(output)), input.clone());

        assert!(watermarked.error().is_none(), "logo watermark failed: {:?}", watermarked.error());
        let document = Document::load(watermarked.output_paths().first().unwrap()).unwrap();
        let logo_image = document.objects.values().filter_map(|object| object.as_stream().ok())
            .find(|stream| stream.dict.get(b"Width").and_then(Object::as_i64).ok() == Some(40) && stream.dict.has(b"SMask"))
            .expect("the logo is stored with a soft mask");
        assert_eq!(logo_image.dict.get(b"Filter").unwrap().as_name().unwrap(), b"FlateDecode");
        assert_eq!(fs::read(&logo).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remaining_mutators_reject_unsafe_documents_before_reserving_output() {
        for kind in ["signed", "outline", "tagged"] {
            for operation in ["numbers", "watermark", "image-watermark", "compress", "extract"] {
                let root = split_fixture_root(&format!("unsafe-{kind}-{operation}"));
                let input = root.join("document.pdf");
                let output = root.join("outputs");
                fs::create_dir_all(&output).unwrap();
                unsafe_mutation_fixture(&input, kind);
                let output_location = OutputLocation::CustomFolder(output.clone());
                let logo = root.join("logo.jpg");
                if operation == "image-watermark" {
                    let mut jpeg = Cursor::new(Vec::new());
                    JpegEncoder::new(&mut jpeg).encode_image(&DynamicImage::ImageRgb8(image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0])))).unwrap();
                    fs::write(&logo, jpeg.into_inner()).unwrap();
                }
                let outcome = match operation {
                    "numbers" => add_page_numbers(&PageOverlayRequest {
                        paths: vec![input.clone()],
                        position: None, pages: None, start_number: None,
                        font_size: None, output_location: output_location.clone(),
                    }, input.clone()),
                    "watermark" => watermark(&watermark_request(&input, "TEST", None, None, output_location.clone()), input.clone()),
                    "image-watermark" => watermark(&watermark_request(&input, "", Some(logo), None, output_location.clone()), input.clone()),
                    "compress" => compress(&CompressPdfRequest {
                        paths: vec![input.clone()], quality: 80, output_location: output_location.clone(),
                    }, input.clone()),
                    "extract" => {
                        let mut request = request(None, vec![0]);
                        request.output_location = output_location;
                        extract_pages(&request, input.clone())
                    }
                    _ => unreachable!(),
                };
                assert!(outcome.error().is_some(), "{kind} {operation}: {:?}", outcome.error());
                assert!(outcome.output_paths().is_empty());
                assert_eq!(fs::read_dir(output).unwrap().count(), 0, "{kind} {operation}");
                fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn pdf_to_images_keeps_twelve_pages_in_numeric_order() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = crate::kit::lock_process_env();
        let root = split_fixture_root("numeric-images");
        let input = root.join("document.pdf");
        let output_folder = root.join("outputs");
        let renderer = root.join("renderer.sh");
        fs::create_dir_all(&output_folder).unwrap();
        split_fixture_pdf(&input, 12);
        fs::write(&renderer, "#!/bin/sh\nprefix=\nfor arg in \"$@\"; do prefix=\"$arg\"; done\ni=1\nwhile [ \"$i\" -le 12 ]; do printf 'page-%s' \"$i\" > \"$prefix-$i.png\"; i=$((i + 1)); done\n").unwrap();
        let mut permissions = fs::metadata(&renderer).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&renderer, permissions).unwrap();
        let previous = std::env::var_os("TOOLBOX_PDFTOPPM_PATH");
        std::env::set_var("TOOLBOX_PDFTOPPM_PATH", &renderer);

        let outcome = to_images(&PdfToImagesRequest {
            paths: vec![input.clone()], dpi: 72, format: "png".to_string(), page_range: None, pages: None,
            output_location: OutputLocation::CustomFolder(output_folder.clone()),
        }, input.clone());

        match previous {
            Some(value) => std::env::set_var("TOOLBOX_PDFTOPPM_PATH", value),
            None => std::env::remove_var("TOOLBOX_PDFTOPPM_PATH"),
        }
        assert!(outcome.error().is_none(), "numeric image export failed: {:?}", outcome.error());
        assert_eq!(outcome.output_paths().len(), 12);
        for (index, output) in outcome.output_paths().iter().enumerate() {
            assert_eq!(output.file_stem().unwrap().to_str().unwrap(), format!("document-images-{}", index + 1));
            assert_eq!(fs::read_to_string(output).unwrap(), format!("page-{}", index + 1));
        }
        fs::remove_dir_all(root).unwrap();
    }

    struct DiscoveryFixture { document: Document, photo: ObjectId, scan: ObjectId, soft_mask: ObjectId, stencil: ObjectId, logo: ObjectId }

    fn image_discovery_fixture() -> DiscoveryFixture {
        let mut document=Document::with_version("1.7");
        let pages_id=document.new_object_id();
        let soft_mask=document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>4, "Height"=>3, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8
        },vec![0;12]));
        let stencil=document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>2, "Height"=>2, "ImageMask"=>true, "BitsPerComponent"=>1
        },vec![0;2]));
        let photo=document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>4, "Height"=>3,
            "ColorSpace"=>"DeviceCMYK", "BitsPerComponent"=>8, "Filter"=>"DCTDecode", "SMask"=>soft_mask
        },b"photo".to_vec()));
        let scan=document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>2, "Height"=>2,
            "ColorSpace"=>vec![Object::Name(b"ICCBased".to_vec()), Object::Integer(0)], "BitsPerComponent"=>8,
            "Filter"=>vec![Object::Name(b"FlateDecode".to_vec()), Object::Name(b"DCTDecode".to_vec())], "Mask"=>stencil
        },b"scan".to_vec()));
        let logo=document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>7, "Height"=>5, "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8,
            "Mask"=>vec![Object::Integer(0), Object::Integer(0)]
        },b"logo".to_vec()));
        let form=document.add_object(Stream::new(dictionary! { "Type"=>"XObject", "Subtype"=>"Form", "BBox"=>vec![0.into(),0.into(),1.into(),1.into()] },Vec::new()));
        let mut kids=Vec::new();
        for xobjects in [
            dictionary! { "Im1"=>photo, "Fm1"=>form, "Im2"=>scan },
            dictionary! {},
            dictionary! { "Logo"=>logo },
        ] {
            let page_id=document.add_object(dictionary! {
                "Type"=>"Page", "Parent"=>pages_id, "MediaBox"=>vec![0.into(),0.into(),1.into(),1.into()],
                "Resources"=>dictionary! { "XObject"=>xobjects }
            });
            kids.push(Object::Reference(page_id));
        }
        document.objects.insert(pages_id,dictionary! { "Type"=>"Pages", "Kids"=>kids, "Count"=>3 }.into());
        let catalog_id=document.add_object(dictionary! { "Type"=>"Catalog", "Pages"=>pages_id });
        document.trailer.set("Root",catalog_id);
        DiscoveryFixture { document, photo, scan, soft_mask, stencil, logo }
    }

    #[test]
    fn image_discovery_lists_candidates_in_page_then_resource_order_with_their_properties() {
        let fixture=image_discovery_fixture();

        let candidates=discover_images(&fixture.document,&[1,2,3]).unwrap();

        let order=candidates.iter().map(|candidate| (candidate.page,candidate.index,candidate.object_id)).collect::<Vec<_>>();
        assert_eq!(order,vec![(1,0,fixture.photo),(1,1,fixture.scan),(3,0,fixture.logo)]);

        let photo=&candidates[0];
        assert_eq!(photo.filters,Some(vec!["DCTDecode".to_string()]));
        assert_eq!((photo.width,photo.height),(4,3));
        assert_eq!(photo.color_space.as_deref(),Some("DeviceCMYK"));
        assert_eq!((photo.soft_mask,photo.mask),(Some(fixture.soft_mask),None));
        assert_eq!(photo.data,b"photo");

        let scan=&candidates[1];
        assert_eq!(scan.filters,Some(vec!["FlateDecode".to_string(),"DCTDecode".to_string()]));
        assert_eq!((scan.width,scan.height),(2,2));
        assert_eq!(scan.color_space.as_deref(),Some("ICCBased"));
        assert_eq!((scan.soft_mask,scan.mask),(None,Some(fixture.stencil)));

        let logo=&candidates[2];
        assert_eq!(logo.filters,Some(Vec::new()));
        assert_eq!((logo.width,logo.height),(7,5));
        assert_eq!(logo.color_space.as_deref(),Some("DeviceRGB"));
        assert_eq!((logo.soft_mask,logo.mask),(None,None),"a colour-key mask array is not a mask reference");
    }

    #[test]
    fn image_discovery_only_lists_the_selected_pages() {
        let fixture=image_discovery_fixture();

        let third=discover_images(&fixture.document,&[3]).unwrap();
        assert_eq!(third.iter().map(|candidate| (candidate.page,candidate.index,candidate.object_id)).collect::<Vec<_>>(),vec![(3,0,fixture.logo)]);
        assert!(discover_images(&fixture.document,&[2]).unwrap().is_empty());
        assert!(discover_images(&fixture.document,&[]).unwrap().is_empty());
    }

    #[test]
    fn image_discovery_uses_one_based_pages_from_zero_based_requests() {
        let fixture=image_discovery_fixture();

        let pages=selected_pdf_pages(&fixture.document,Some(&vec![2])).unwrap();
        let candidates=discover_images(&fixture.document,&pages).unwrap();

        assert_eq!(pages,vec![3]);
        assert_eq!(candidates.iter().map(|candidate| candidate.page).collect::<Vec<_>>(),vec![3]);
    }

    fn image_extraction_fixture(path: &Path) {
        let mut jpeg=Cursor::new(Vec::new());
        JpegEncoder::new(&mut jpeg).encode_image(&DynamicImage::ImageRgb8(image::RgbImage::from_pixel(1,1,image::Rgb([255,0,0])))).unwrap();
        let mut document=Document::with_version("1.7");
        let pages_id=document.new_object_id();
        let jpeg_id=document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>1, "Height"=>1,
            "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8, "Filter"=>"DCTDecode"
        },jpeg.into_inner()));
        let unsupported_id=document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>1, "Height"=>1,
            "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8, "Filter"=>"FlateDecode"
        },b"unsupported".to_vec()));
        let page_id=document.add_object(dictionary! {
            "Type"=>"Page", "Parent"=>pages_id, "MediaBox"=>vec![0.into(),0.into(),1.into(),1.into()],
            "Resources"=>dictionary! { "XObject"=>dictionary! { "AFirst"=>jpeg_id, "ZUnsupported"=>unsupported_id } }
        });
        document.objects.insert(pages_id,dictionary! { "Type"=>"Pages", "Kids"=>vec![Object::Reference(page_id)], "Count"=>1 }.into());
        let catalog_id=document.add_object(dictionary! { "Type"=>"Catalog", "Pages"=>pages_id });
        document.trailer.set("Root",catalog_id);
        document.save(path).unwrap();
    }

    #[test]
    fn failed_image_extraction_publishes_no_partial_outputs() {
        let temp=std::env::temp_dir().join(format!("toolbox_extract_images_{}_{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&temp).unwrap();
        let input=temp.join("source.pdf");
        image_extraction_fixture(&input);
        let outcome=extract(&ExtractFromPdfRequest { paths:vec![input.clone()], mode:ExtractMode::Images, page_range:None, output_location:OutputLocation::AlongsideInput, preserve_layout:false, min_image_size:None },input.clone());

        assert!(outcome.error().is_some(),"expected unsupported image filter to fail");
        assert!(outcome.output_paths().is_empty());
        assert_eq!(fs::read_dir(&temp).unwrap().count(),1);
        assert!(!temp.join("source-image-1-0.jpg").exists());
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn an_abandoned_image_reservation_preserves_a_competing_replacement() {
        let temp=std::env::temp_dir().join(format!("toolbox_extract_failure_{}_{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&temp).unwrap();
        let input=temp.join("source.pdf");
        fs::write(&input,b"source").unwrap();
        let reservation=OutputNaming::reserve_destination(&input,&OutputLocation::AlongsideInput,"-image-1-0","jpg").unwrap();
        fs::write(reservation.path(),b"extracted bytes").unwrap();
        let destination=reservation.destination_path().to_path_buf();
        fs::write(&destination,b"competing replacement").unwrap();

        drop(reservation);

        assert_eq!(fs::read(destination).unwrap(),b"competing replacement");
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn extracted_image_publication_reports_outputs_published_before_a_later_conflict() {
        let temp = std::env::temp_dir().join(format!(
            "toolbox_extract_partial_{}_{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&temp).unwrap();
        let input = temp.join("source.pdf");
        fs::write(&input, b"source").unwrap();
        let location = OutputLocation::AlongsideInput;
        let first = OutputNaming::reserve_destination(&input, &location, "-image-1-0", "jpg").unwrap();
        let second = OutputNaming::reserve_destination(&input, &location, "-image-1-1", "jpg").unwrap();
        let first_destination = first.destination_path().to_path_buf();
        let second_destination = second.destination_path().to_path_buf();
        fs::write(first.path(), b"first extracted image").unwrap();
        fs::write(second.path(), b"second extracted image").unwrap();
        fs::write(&second_destination, b"competing replacement").unwrap();

        let (published, _) = publish_extracted_images(vec![first, second]).unwrap_err();

        assert_eq!(published, vec![first_destination.clone()]);
        assert_eq!(fs::read(&first_destination).unwrap(), b"first extracted image");
        assert_eq!(fs::read(&second_destination).unwrap(), b"competing replacement");
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn rendered_image_materialization_reports_outputs_published_before_a_later_invalid_file() {
        let temp = split_fixture_root("rendered-partial");
        let input = temp.join("source.pdf");
        let generated = temp.join("generated");
        let output_folder = temp.join("outputs");
        fs::create_dir_all(&generated).unwrap();
        fs::create_dir_all(&output_folder).unwrap();
        fs::write(&input, b"source").unwrap();
        let first = generated.join("source-page-1.png");
        let invalid = generated.join("unexpected.png");
        fs::write(&first, b"rendered image").unwrap();
        fs::write(&invalid, b"unexpected image").unwrap();

        let outcome = materialize_rendered_images(&input, &OutputLocation::CustomFolder(output_folder.clone()), &[first, invalid]);

        assert!(outcome.error().is_some());
        assert_eq!(outcome.output_paths().len(), 1);
        assert!(outcome.output_paths()[0].is_file());
        assert_eq!(fs::read(&outcome.output_paths()[0]).unwrap(), b"rendered image");
        assert_eq!(fs::read_dir(&output_folder).unwrap().count(), 1);
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn split_materialization_reports_outputs_published_before_a_later_invalid_file() {
        let temp = split_fixture_root("split-partial");
        let input = temp.join("source.pdf");
        let generated = temp.join("generated");
        let output_folder = temp.join("outputs");
        fs::create_dir_all(&generated).unwrap();
        fs::create_dir_all(&output_folder).unwrap();
        fs::write(&input, b"source").unwrap();
        let first = generated.join("part-0.pdf");
        let missing = generated.join("part-1.pdf");
        fs::write(&first, b"first split").unwrap();

        let outcome = materialize_split_outputs(&input, &OutputLocation::CustomFolder(output_folder.clone()), &[("-split-0".to_string(), first), ("-split-1".to_string(), missing)]).unwrap_err();

        assert!(outcome.error().is_some());
        assert_eq!(outcome.output_paths().len(), 1);
        assert!(outcome.output_paths()[0].is_file());
        assert_eq!(fs::read(&outcome.output_paths()[0]).unwrap(), b"first split");
        assert_eq!(fs::read_dir(&output_folder).unwrap().count(), 1);
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn split_spans_name_parts_by_mode() {
        let suffixes = |mode: SplitMode, pages: usize| SplitPlan::new(&mode).unwrap().spans(pages).unwrap().into_iter().map(|span| (span.suffix, span.first, span.last)).collect::<Vec<_>>();
        let span = |suffix: &str, first: usize, last: usize| (suffix.to_string(), first, last);

        assert_eq!(suffixes(SplitMode::EveryN { pages: 1 }, 3), vec![span("-split-page-1", 1, 1), span("-split-page-2", 2, 2), span("-split-page-3", 3, 3)]);
        assert_eq!(suffixes(SplitMode::EveryN { pages: 1 }, 12)[..2], [span("-split-page-01", 1, 1), span("-split-page-02", 2, 2)]);
        assert_eq!(suffixes(SplitMode::EveryN { pages: 2 }, 3), vec![span("-split-1-2", 1, 2), span("-split-3-3", 3, 3)]);
        assert_eq!(suffixes(SplitMode::EveryN { pages: 5 }, 12), vec![span("-split-01-05", 1, 5), span("-split-06-10", 6, 10), span("-split-11-12", 11, 12)]);
        assert_eq!(suffixes(ranges("3, 1-2, 2-"), 3), vec![span("-page-3", 3, 3), span("-pages-1-2", 1, 2), span("-pages-2-3", 2, 3)]);
        assert_eq!(suffixes(ranges("1-3, 5, 8-"), 10), vec![span("-pages-1-3", 1, 3), span("-page-5", 5, 5), span("-pages-8-10", 8, 10)]);
        assert_eq!(suffixes(ranges("4-4,10-"), 10), vec![span("-page-4", 4, 4), span("-page-10", 10, 10)]);
    }

    #[test]
    fn range_strings_follow_the_grammar_and_report_every_problem() {
        let parsed = |raw: &str| parse_split_ranges(raw).map(|ranges| ranges.into_iter().map(|(_, range)| (range.start, range.end)).collect::<Vec<_>>());
        assert_eq!(parsed("1-3, 5, 8-"), Ok(vec![(1, Some(3)), (5, Some(5)), (8, None)]));
        assert_eq!(parsed(" 2 - 4 ,7"), Ok(vec![(2, Some(4)), (7, Some(7))]));
        assert_eq!(parsed("1-3,2-5"), Ok(vec![(1, Some(3)), (2, Some(5))]));

        let rejected = |raw: &str| parse_split_ranges(raw).unwrap_err();
        assert_eq!(rejected(""), "Enter at least one page range to split.");
        assert_eq!(rejected("  "), "Enter at least one page range to split.");
        assert_eq!(rejected("3-1"), "3-1 runs backwards (write 1-3).");
        assert_eq!(rejected("1,,3"), "Entry 2 is empty.");
        assert_eq!(rejected("1-3,"), "Entry 2 is empty.");
        assert_eq!(rejected("x"), "x isn't a page.");
        assert_eq!(rejected("0"), "0 isn't a page. Pages start at 1.");
        assert_eq!(rejected("0-2"), "0-2 isn't a range. Pages start at 1.");
        for raw in ["-5", "1-2-3", "a-3", "1-b", "+1-3", "1--3", "-"] {
            assert_eq!(rejected(raw), format!("{raw} isn't a range."), "{raw}");
        }
        for raw in ["+3", "1.5", "two"] {
            assert_eq!(rejected(raw), format!("{raw} isn't a page."), "{raw}");
        }
        assert_eq!(rejected("3-1, , x"), "3-1 runs backwards (write 1-3). Entry 2 is empty. x isn't a page.");
    }

    #[test]
    fn split_request_keeps_only_the_selected_mode_fields() {
        let mode = |json: &str| serde_json::from_str::<SplitPdfRequest>(json).unwrap().mode.0;
        assert_eq!(mode(r#"{"paths":[],"mode":{"kind":"ranges","ranges":"1-3"},"outputLocation":"alongsideInput"}"#), Ok(ranges("1-3")));
        assert_eq!(mode(r#"{"paths":[],"mode":{"kind":"everyN","pages":4},"outputLocation":"alongsideInput"}"#), Ok(SplitMode::EveryN { pages: 4 }));
        for malformed in [
            r#"{"kind":"ranges"}"#,
            r#"{"kind":"ranges","ranges":3}"#,
            r#"{"kind":"ranges","ranges":"1","pages":2}"#,
            r#"{"kind":"everyN"}"#,
            r#"{"kind":"everyN","pages":-1}"#,
            r#"{"kind":"everyN","pages":2,"ranges":"1"}"#,
            r#"{"kind":"chunks","chunkSize":2}"#,
            r#"{"ranges":"1-3"}"#,
            r#""ranges""#,
            "null",
        ] {
            let json = format!(r#"{{"paths":[],"mode":{malformed},"outputLocation":"alongsideInput"}}"#);
            assert!(mode(&json).unwrap_err().starts_with("The split mode is malformed"), "{malformed}");
        }
        assert_eq!(mode(r#"{"paths":[],"outputLocation":"alongsideInput"}"#).unwrap_err(), "Choose how to split: by page ranges or every N pages.");
    }

    fn split_temp_entries() -> std::collections::HashSet<PathBuf> {
        let prefix = format!("toolbox_pdf_split_{}", std::process::id());
        fs::read_dir(std::env::temp_dir()).unwrap().filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(&prefix))).collect()
    }

    fn assert_split_outputs(outputs: &[PathBuf], expected_count: usize) {
        assert_eq!(outputs.len(), expected_count);
        assert_eq!(outputs.iter().collect::<std::collections::HashSet<_>>().len(), expected_count);
        for output in outputs {
            assert!(output.is_file(), "split reported missing output {}", output.display());
            assert!(Document::load(output).is_ok(), "split output is not a valid PDF: {}", output.display());
        }
    }

    fn run_split_with_sentinel(name: &str, mode: SplitMode, sentinel_name: &str, expected_count: usize) {
        let _lock = SPLIT_TEST_LOCK.lock().unwrap();
        if qpdf().is_err() {
            eprintln!("skipping: qpdf not available");
            return;
        }
        let root = split_fixture_root(name);
        let input = root.join("document.pdf");
        let output_folder = root.join("outputs");
        fs::create_dir_all(&output_folder).unwrap();
        split_fixture_pdf(&input, 3);
        let sentinel = output_folder.join(sentinel_name);
        fs::write(&sentinel, format!("{name} sentinel")).unwrap();
        let sentinel_bytes = fs::read(&sentinel).unwrap();
        let temp_before = split_temp_entries();

        let outcome = split(&split_request(OutputLocation::CustomFolder(output_folder.clone()), mode), input.clone());

        assert!(outcome.error().is_none(), "split failed: {:?}", outcome.error());
        assert_eq!(outcome.input_path, input);
        assert_split_outputs(&outcome.output_paths(), expected_count);
        assert!(!outcome.output_paths().contains(&sentinel));
        assert_eq!(fs::read(&sentinel).unwrap(), sentinel_bytes);
        let temp_after = split_temp_entries();
        assert_eq!(temp_after, temp_before, "split temporary directories were not cleaned");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn page_split_does_not_overwrite_existing_outputs() {
        run_split_with_sentinel("pages", SplitMode::EveryN { pages: 1 }, "document-split-page-1.pdf", 3);
    }

    #[test]
    fn chunk_split_does_not_overwrite_existing_outputs() {
        run_split_with_sentinel("chunks", SplitMode::EveryN { pages: 2 }, "document-split-1-2.pdf", 2);
    }

    #[test]
    fn range_split_does_not_overwrite_existing_outputs() {
        run_split_with_sentinel("ranges", ranges("1,3"), "document-page-1.pdf", 2);
    }

    #[test]
    fn range_split_rejects_blank_ranges_without_outputs() {
        let input = PathBuf::from("blank-ranges.pdf");
        let outcome = split(&split_request(OutputLocation::AlongsideInput, ranges("  ")), input.clone());

        assert_eq!(outcome.input_path, input);
        assert!(matches!(outcome.error().map(|error| &error.kind), Some(crate::kit::contracts::ErrorKind::InvalidInput)));
        assert!(outcome.output_paths().is_empty());
    }

    fn split_page_labels(path: &Path) -> Vec<String> {
        let document = Document::load(path).unwrap();
        document.get_pages().values().map(|page| {
            let content = String::from_utf8_lossy(&document.get_page_content(*page)).into_owned();
            let start = content.find("(Split page ").unwrap_or_else(|| panic!("{} has a page without a label", path.display())) + 1;
            let end = start + content[start..].find(')').unwrap();
            content[start..end].to_string()
        }).collect()
    }

    fn split_output_names(outcome: &JobOutcome) -> Vec<String> {
        outcome.output_paths().iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    fn labels(pages: &[usize]) -> Vec<String> { pages.iter().map(|page| format!("Split page {page}")).collect() }

    struct RangeSplitFixture { root: PathBuf, input: PathBuf, original: Vec<u8>, output_folder: PathBuf }

    impl RangeSplitFixture {
        fn new(name: &str, page_count: usize) -> Self {
            let root = split_fixture_root(name);
            let input = root.join("document.pdf");
            let output_folder = root.join("outputs");
            fs::create_dir_all(&output_folder).unwrap();
            split_fixture_pdf(&input, page_count);
            let original = fs::read(&input).unwrap();
            Self { root, input, original, output_folder }
        }

        fn split(&self, mode: impl Into<RequestedSplitMode>) -> JobOutcome {
            let request = SplitPdfRequest { paths: vec![self.input.clone()], mode: mode.into(), output_location: OutputLocation::CustomFolder(self.output_folder.clone()) };
            let outcome = split(&request, self.input.clone());
            assert_eq!(fs::read(&self.input).unwrap(), self.original, "split modified the original PDF");
            outcome
        }

        fn output_count(&self) -> usize { fs::read_dir(&self.output_folder).unwrap().count() }
    }

    impl Drop for RangeSplitFixture {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); }
    }

    #[test]
    fn mixed_range_split_writes_page_named_files_and_numbers_a_second_run() {
        let _lock = SPLIT_TEST_LOCK.lock().unwrap();
        if qpdf().is_err() { eprintln!("skipping: qpdf not available"); return; }
        let fixture = RangeSplitFixture::new("mixed-ranges", 10);

        let first = fixture.split(ranges("1-3, 5, 8-"));
        assert!(first.error().is_none(), "split failed: {:?}", first.error());
        assert_eq!(split_output_names(&first), ["document-pages-1-3.pdf", "document-page-5.pdf", "document-pages-8-10.pdf"]);
        let contents = first.output_paths().iter().map(|path| split_page_labels(path)).collect::<Vec<_>>();
        assert_eq!(contents, [labels(&[1, 2, 3]), labels(&[5]), labels(&[8, 9, 10])]);
        let first_bytes = first.output_paths().iter().map(|path| fs::read(path).unwrap()).collect::<Vec<_>>();

        let second = fixture.split(ranges("1-3, 5, 8-"));
        assert!(second.error().is_none(), "second split failed: {:?}", second.error());
        assert_eq!(split_output_names(&second), ["document-pages-1-3-1.pdf", "document-page-5-1.pdf", "document-pages-8-10-1.pdf"]);
        assert_eq!(second.output_paths().iter().map(|path| split_page_labels(path)).collect::<Vec<_>>(), contents);
        assert_eq!(first.output_paths().iter().map(|path| fs::read(path).unwrap()).collect::<Vec<_>>(), first_bytes, "a second split replaced an earlier output");
        assert_eq!(fixture.output_count(), 6);
    }

    #[test]
    fn overlapping_ranges_each_get_their_own_pages() {
        let _lock = SPLIT_TEST_LOCK.lock().unwrap();
        if qpdf().is_err() { eprintln!("skipping: qpdf not available"); return; }
        let fixture = RangeSplitFixture::new("overlapping-ranges", 6);

        let outcome = fixture.split(ranges("1-4, 3-6, 4"));

        assert!(outcome.error().is_none(), "split failed: {:?}", outcome.error());
        assert_eq!(split_output_names(&outcome), ["document-pages-1-4.pdf", "document-pages-3-6.pdf", "document-page-4.pdf"]);
        assert_eq!(outcome.output_paths().iter().map(|path| split_page_labels(path)).collect::<Vec<_>>(), [labels(&[1, 2, 3, 4]), labels(&[3, 4, 5, 6]), labels(&[4])]);
    }

    #[test]
    fn malformed_split_requests_are_rejected_before_any_output() {
        let _lock = SPLIT_TEST_LOCK.lock().unwrap();
        let fixture = RangeSplitFixture::new("malformed-ranges", 10);
        let temp_before = split_temp_entries();
        let malformed: Vec<RequestedSplitMode> = vec![
            ranges("3-1").into(),
            ranges("1,,3").into(),
            ranges("1-3,").into(),
            ranges("x").into(),
            ranges("0").into(),
            ranges("1-2-3").into(),
            ranges("-5").into(),
            ranges("  ").into(),
            ranges("1-3, 12").into(),
            ranges("11-").into(),
            SplitMode::EveryN { pages: 0 }.into(),
            RequestedSplitMode::default(),
            serde_json::from_str::<RequestedSplitMode>(r#"{"kind":"ranges","ranges":"1-3","pages":2}"#).unwrap(),
            serde_json::from_str::<RequestedSplitMode>(r#"{"kind":"ranges"}"#).unwrap(),
        ];

        for mode in malformed {
            let described = format!("{mode:?}");
            let outcome = fixture.split(mode);
            assert_eq!(outcome.input_path, fixture.input);
            assert!(matches!(outcome.error().map(|error| &error.kind), Some(crate::kit::contracts::ErrorKind::InvalidInput)), "{described}: {:?}", outcome.error());
            assert!(outcome.output_paths().is_empty(), "{described}");
            assert_eq!(fixture.output_count(), 0, "{described} wrote an output");
        }
        assert_eq!(split_temp_entries(), temp_before, "a rejected split left a temporary directory");
        assert_eq!(fixture.split(ranges("1-3, 12, 9-11")).error().unwrap().message, "12 goes past the last page (10). 9-11 goes past the last page (10).");
    }

    #[test]
    fn split_failure_leaves_no_partial_destinations() {
        let _lock = SPLIT_TEST_LOCK.lock().unwrap();
        if qpdf().is_err() {
            eprintln!("skipping: qpdf not available");
            return;
        }
        let root = split_fixture_root("failure");
        let input = root.join("invalid.pdf");
        let output_folder = root.join("outputs");
        fs::create_dir_all(&output_folder).unwrap();
        fs::write(&input, b"not a PDF").unwrap();
        let sentinel = output_folder.join("document-split-page-1.pdf");
        fs::write(&sentinel, b"sentinel").unwrap();
        let temp_before = split_temp_entries();

        let outcome = split(&split_request(OutputLocation::CustomFolder(output_folder.clone()), SplitMode::EveryN { pages: 1 }), input.clone());

        assert_eq!(outcome.input_path, input);
        assert!(outcome.error().is_some());
        assert!(outcome.output_paths().is_empty());
        assert_eq!(fs::read(&sentinel).unwrap(), b"sentinel");
        assert_eq!(fs::read_dir(&output_folder).unwrap().count(), 1);
        assert_eq!(split_temp_entries(), temp_before, "split temporary directories were not cleaned after failure");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn merge_and_split_qpdf_deadlines_are_fifteen_seconds_per_file_plus_fifty_milliseconds_per_page() {
        assert_eq!(QPDF_DEADLINE, QpdfDeadline { per_file: Duration::from_secs(15), per_page: Duration::from_millis(50) });
        assert_eq!(QPDF_DEADLINE.limit(1, 1), Duration::from_millis(15_050));
        assert_eq!(QPDF_DEADLINE.limit(2, 200), Duration::from_secs(40));
        let longest = limits::recorded("split_pdf").unwrap();
        assert_eq!(longest, limits::recorded("merge_pdfs").unwrap());
        assert_eq!(QPDF_DEADLINE.limit(1, longest.max_pages.unwrap() as usize), Duration::from_secs(265));
        assert!(QPDF_DEADLINE.limit(1, longest.max_pages.unwrap() as usize) < longest.max_duration, "qpdf's own deadline ends a long split before the job limit does");
    }

    #[cfg(unix)]
    fn with_qpdf_stalling_on<T>(root: &Path, marker: &str, run: impl FnOnce() -> T) -> Option<T> {
        use std::os::unix::fs::PermissionsExt;
        let _guard = crate::kit::lock_process_env();
        let Ok(real) = qpdf() else { eprintln!("skipping: qpdf not available"); return None };
        let script = root.join("qpdf");
        fs::write(&script, format!("#!/bin/sh\ncase \"$*\" in *'{marker}'*) for last; do :; done; printf partial > \"$last\"; printf %s \"$last\" > '{}'; exec sleep 30;; esac\nexec '{}' \"$@\"\n", root.join("stalled-output").display(), real.display())).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let previous = std::env::var_os("TOOLBOX_QPDF_PATH");
        std::env::set_var("TOOLBOX_QPDF_PATH", &script);
        let result = run();
        match previous {
            Some(value) => std::env::set_var("TOOLBOX_QPDF_PATH", value),
            None => std::env::remove_var("TOOLBOX_QPDF_PATH"),
        }
        Some(result)
    }

    #[cfg(unix)]
    fn assert_timed_out_with_nothing_saved(outcome: &JobOutcome, input: &Path, output_folder: &Path, elapsed: Duration, action: &str) {
        assert_eq!(outcome.input_path, input);
        let error = outcome.error().expect("the stalled helper fails the file");
        assert_eq!(error.kind, crate::kit::contracts::ErrorKind::LimitExceeded { limit: LimitKind::Time });
        assert_eq!(error.message, format!("{action} took longer than 500 milliseconds and was stopped. Nothing was saved."));
        assert!(elapsed < Duration::from_secs(10), "the helper is stopped at its deadline, not waited out");
        assert!(outcome.output_paths().is_empty());
        assert_eq!(fs::read_dir(output_folder).unwrap().count(), 0, "no partial output is left behind");
    }

    #[cfg(unix)]
    #[test]
    fn a_merge_whose_qpdf_passes_its_deadline_is_stopped_and_publishes_nothing() {
        let root = split_fixture_root("merge-deadline");
        let paths = vec![root.join("first.pdf"), root.join("stalls.pdf")];
        for path in &paths { split_fixture_pdf(path, 1); }
        let output_folder = root.join("outputs");
        fs::create_dir_all(&output_folder).unwrap();
        let request = MergePdfRequest { paths: paths.clone(), output_location: OutputLocation::CustomFolder(output_folder.clone()), output_path: None };
        let Some((outcome, elapsed)) = with_qpdf_stalling_on(&root, "stalls.pdf", || { let started = std::time::Instant::now(); (merge_within(&request, QpdfDeadline { per_file: Duration::from_millis(250), per_page: Duration::ZERO }).combined, started.elapsed()) }) else { return };
        assert_timed_out_with_nothing_saved(&outcome, &paths[0], &output_folder, elapsed, "Merging");
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn a_split_whose_qpdf_passes_its_deadline_is_stopped_and_publishes_no_part() {
        let _lock = SPLIT_TEST_LOCK.lock().unwrap();
        let root = split_fixture_root("split-deadline");
        let input = root.join("document.pdf");
        split_fixture_pdf(&input, 2);
        let output_folder = root.join("outputs");
        fs::create_dir_all(&output_folder).unwrap();
        let request = split_request(OutputLocation::CustomFolder(output_folder.clone()), SplitMode::EveryN { pages: 1 });
        let Some((outcome, elapsed)) = with_qpdf_stalling_on(&root, " 2-2 ", || { let started = std::time::Instant::now(); (split_within(&request, input.clone(), QpdfDeadline { per_file: Duration::from_millis(300), per_page: Duration::from_millis(100) }), started.elapsed()) }) else { return };
        assert_timed_out_with_nothing_saved(&outcome, &input, &output_folder, elapsed, "Splitting");
        let stalled_output = PathBuf::from(fs::read_to_string(root.join("stalled-output")).unwrap());
        assert!(!stalled_output.parent().unwrap().exists(), "the split workspace is removed");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parses_ranges_in_document_order() {
        let selected = selected_page_indices(&request(Some("1-3, 7"), vec![]), 7).unwrap();
        assert_eq!(selected, vec![0, 1, 2, 6]);
        assert_eq!(selected_page_indices(&request(Some("6-, 2"), vec![]), 7).unwrap(), vec![1, 5, 6]);
    }

    #[test]
    fn rejects_invalid_or_out_of_bounds_ranges() {
        for input in ["", "1-3, nope", "0", "4", "1-3,,2", "4-"] {
            assert!(selected_page_indices(&request(Some(input), vec![]), 3).is_err(), "{input}");
        }
    }

    #[test]
    fn supports_legacy_zero_based_page_selection() {
        let selected = selected_page_indices(&request(None, vec![0, 2]), 3).unwrap();
        assert_eq!(selected, vec![0, 2]);
    }

    #[test]
    fn normalizes_selectable_text_without_reordering_lines() {
        assert_eq!(normalize_pdf_text("first line  \nsecond line\n\n"), "first line\nsecond line");
        assert!(normalize_pdf_text("  \n\t").is_empty());
    }

    #[test]
    fn accepts_integer_square_media_boxes_for_pdf_edits() {
        let input = std::env::temp_dir().join(format!("toolbox_square_pdf_{}.pdf", std::process::id()));
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let contents_id = document.add_object(Stream::new(dictionary! {}, Vec::new()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![Object::Integer(0), Object::Integer(0), Object::Integer(256), Object::Integer(256)],
            "Contents" => contents_id,
        });
        document.objects.insert(pages_id, dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
        }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(&input).unwrap();

        let numbered = add_page_numbers(&PageOverlayRequest {
            paths: vec![input.clone()],
            position: Some("bottom-right".to_string()), pages: None,
            start_number: None, font_size: None, output_location: OutputLocation::AlongsideInput,
        }, input.clone());
        assert!(numbered.error().is_none(), "page numbers failed: {:?}", numbered.error());

        let watermark = watermark(&watermark_request(&input, "TEST", None, Some(WatermarkPlacement::Center), OutputLocation::AlongsideInput), input.clone());
        assert!(watermark.error().is_none(), "watermark failed: {:?}", watermark.error());

        let cropped = crate::kit::pdf::editor::crop(&crate::kit::pdf::editor::CropPdfRequest {
            paths: vec![input.clone()],
            spec: crate::kit::pdf::editor::CropSpec::Rectangle { rectangle: crate::kit::pdf::editor::PdfRect { x: 0.0, y: 0.0, width: 128.0, height: 128.0 } },
            scope: crate::kit::pdf::editor::RangedPageScope::All,
            output_location: OutputLocation::AlongsideInput,
        }, input.clone());
        assert!(cropped.error().is_none(), "crop failed: {:?}", cropped.error());

        for output in numbered.output_paths().into_iter().chain(watermark.output_paths()).chain(cropped.output_paths()) {
            let _ = std::fs::remove_file(output);
        }
        let _ = std::fs::remove_file(input);
    }

    fn text_pdf(name: &str, pages: &[&str]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("toolbox_{name}_{}.pdf", std::process::id()));
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let kids = pages.iter().map(|text| {
            let lines = text.lines().enumerate().map(|(index, line)| format!("BT /F1 12 Tf 72 {} Td ({line}) Tj ET", 700 - index * 20)).collect::<Vec<_>>().join("\n");
            let contents_id = document.add_object(Stream::new(dictionary! {}, lines.into_bytes()));
            Object::Reference(document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "Contents" => contents_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
            }))
        }).collect::<Vec<_>>();
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Count" => kids.len() as i64, "Kids" => kids }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(&path).unwrap();
        path
    }

    #[test]
    fn rotates_only_selected_pages_on_top_of_existing_rotation() {
        let input = text_pdf("rotate", &["one", "two"]);
        let mut document = Document::load(&input).unwrap();
        let first = *document.get_pages().get(&1).unwrap();
        document.get_dictionary_mut(first).unwrap().set("Rotate", 270);
        document.save(&input).unwrap();

        let outcome = rotate(&RotatePdfRequest { paths: vec![input.clone()], degrees: 180, pages: Some(vec![0]), output_location: OutputLocation::AlongsideInput }, input.clone());
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let rotated = Document::load(&outcome.output_paths()[0]).unwrap();
        let pages = rotated.get_pages();
        assert_eq!(page_rotation(&rotated, pages[&1]), 90);
        assert_eq!(page_rotation(&rotated, pages[&2]), 0);
        assert!(rotate(&RotatePdfRequest { paths: vec![], degrees: 45, pages: None, output_location: OutputLocation::AlongsideInput }, input.clone()).error().is_some());
        assert!(rotate(&RotatePdfRequest { paths: vec![], degrees: 90, pages: Some(vec![5]), output_location: OutputLocation::AlongsideInput }, input.clone()).error().is_some());
        let _ = fs::remove_file(&outcome.output_paths()[0]);
        let _ = fs::remove_file(input);
    }

    #[test]
    fn repairs_a_pdf_with_a_broken_cross_reference_table() {
        if qpdf().is_err() { return; }
        let input = text_pdf("repair", &["intact text"]);
        let mut bytes = fs::read(&input).unwrap();
        let xref = bytes.windows(4).rposition(|window| window == b"xref").unwrap();
        bytes.truncate(xref);
        bytes.extend_from_slice(b"xref\ngarbage\n%%EOF\n");
        fs::write(&input, &bytes).unwrap();

        let outcome = repair(&RepairPdfRequest { paths: vec![input.clone()], output_location: OutputLocation::AlongsideInput }, input.clone());
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        assert_eq!(outcome.detail, "PDF repaired");
        assert_eq!(fs::read(&input).unwrap(), bytes, "repair must not modify the original");
        let repaired = Document::load(&outcome.output_paths()[0]).unwrap();
        assert!(repaired.extract_text(&[1]).unwrap().contains("intact text"));
        let _ = fs::remove_file(&outcome.output_paths()[0]);

        fs::write(&input, b"not a pdf at all").unwrap();
        let broken = repair(&RepairPdfRequest { paths: vec![input.clone()], output_location: OutputLocation::AlongsideInput }, input.clone());
        assert!(broken.error().is_some());
        assert!(broken.output_paths().is_empty());
        let _ = fs::remove_file(input);
    }

    #[test]
    fn compares_text_page_by_page() {
        let old = text_pdf("compare_old", &["Title\nPrice 10\nFooter", "Same page"]);
        let new = text_pdf("compare_new", &["Title\nPrice 12\nFooter", "Same page", "Appendix"]);
        let outcome = compare(&ComparePdfsRequest { paths: vec![old.clone(), new.clone()], output_location: OutputLocation::AlongsideInput, output_path: None });
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        assert_eq!(outcome.detail, "2 page(s) with differences");
        let report = fs::read_to_string(&outcome.output_paths()[0]).unwrap();
        assert!(report.starts_with(&format!("Original: {} (lines marked -)\nNew version: {} (lines marked +)\n", old.file_name().unwrap().to_string_lossy(), new.file_name().unwrap().to_string_lossy())), "{report}");
        assert!(report.contains("Page 1\n- Price 10\n+ Price 12"), "{report}");
        assert!(!report.contains("Page 2"), "{report}");
        assert!(report.contains("Page 3\nOnly in"), "{report}");
        assert!(compare(&ComparePdfsRequest { paths: vec![old.clone()], output_location: OutputLocation::AlongsideInput, output_path: None }).error().is_some());
        for path in [&outcome.output_paths()[0], &old, &new] { let _ = fs::remove_file(path); }
    }

    #[test]
    fn diffs_lines_by_longest_common_subsequence() {
        let lines = |text: &str| text.split(' ').map(str::to_string).collect::<Vec<_>>();
        assert!(diff_lines(&lines("a b c"), &lines("a b c")).is_empty());
        assert_eq!(diff_lines(&lines("a b c"), &lines("a x c d")), vec!["- b", "+ x", "+ d"]);
    }

    #[test]
    fn redaction_removes_text_under_the_box_from_the_file() {
        let _guard = crate::kit::lock_process_env();
        if pdftoppm().is_none() { return; }
        let input = text_pdf("redact", &["SECRET123\nkeep this line", "OTHERPAGE"]);
        let outcome = crate::kit::pdf::editor::redact(&crate::kit::pdf::editor::RedactPdfRequest {
            paths: vec![input.clone()],
            rectangle: crate::kit::pdf::editor::PdfRect { x: 60.0, y: 690.0, width: 200.0, height: 30.0 },
            scope: crate::kit::pdf::editor::PageScope::Selected { pages: vec![0] },
            output_location: OutputLocation::AlongsideInput,
        }, input.clone());
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let output = &outcome.output_paths()[0];
        let redacted = Document::load(output).unwrap();
        assert_eq!(redacted.get_pages().len(), 2);
        assert!(!redacted.extract_text(&[1]).unwrap_or_default().contains("SECRET123"));
        assert!(redacted.extract_text(&[2]).unwrap().contains("OTHERPAGE"));
        let mut decompressed = redacted.clone();
        decompressed.decompress();
        let bytes = { let path = std::env::temp_dir().join(format!("toolbox_redact_check_{}.pdf", std::process::id())); decompressed.save(&path).unwrap(); let bytes = fs::read(&path).unwrap(); let _ = fs::remove_file(path); bytes };
        assert!(!bytes.windows(9).any(|window| window == b"SECRET123"), "redacted text is still in the file");
        assert!(fs::read(&input).unwrap().windows(9).any(|window| window == b"SECRET123"), "original must be unchanged");
        let _ = fs::remove_file(output);
        let _ = fs::remove_file(input);
    }

    #[test]
    fn signature_is_added_to_pages_with_a_single_content_stream() {
        let input = text_pdf("sign_single_stream", &["original text", "second page"]);
        let outcome = crate::kit::pdf::editor::sign(&crate::kit::pdf::editor::SignPdfRequest {
            paths: vec![input.clone()],
            page: 0,
            text: "ToolboxSignature".to_string(),
            signature_path: None,
            rectangle: crate::kit::pdf::editor::PdfRect { x: 72.0, y: 100.0, width: 200.0, height: 40.0 },
            scope: crate::kit::pdf::editor::PageScope::Selected { pages: vec![0] },
            output_location: OutputLocation::AlongsideInput,
        }, input.clone());
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let output = &outcome.output_paths()[0];
        let signed = Document::load(output).unwrap();
        let first = signed.extract_text(&[1]).unwrap();
        assert!(first.contains("original text"), "existing content must be kept: {first}");
        assert!(first.contains("ToolboxSignature"), "signature missing: {first}");
        assert!(!signed.extract_text(&[2]).unwrap().contains("ToolboxSignature"));
        assert!(!fs::read(&input).unwrap().windows(16).any(|window| window == b"ToolboxSignature"), "original must be unchanged");
        let _ = fs::remove_file(output);
        let _ = fs::remove_file(input);
    }

    /// Which document-level parts a generated structured PDF carries. Each page gets its own media box size.
    #[derive(Default)]
    pub(crate) struct Structure<'a> {
        pub sizes: &'a [(i64, i64)],
        pub bookmarks: bool,
        pub form: bool,
        pub tags: bool,
        pub signed: bool,
    }

    pub(crate) fn structured_pdf(path: &Path, structure: Structure) {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let catalog_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let page_ids = structure.sizes.iter().enumerate().map(|(index, (width, height))| {
            let content_id = document.add_object(Stream::new(dictionary! {}, format!("/P <</MCID 0>> BDC BT /F1 18 Tf 36 36 Td (Page {}) Tj ET EMC", index + 1).into_bytes()));
            document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "StructParents" => index as i64,
                "MediaBox" => vec![0.into(), 0.into(), Object::Integer(*width), Object::Integer(*height)],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } }, "Contents" => content_id,
            })
        }).collect::<Vec<_>>();
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => page_ids.iter().map(|id| Object::Reference(*id)).collect::<Vec<_>>(), "Count" => page_ids.len() as i64 }.into());
        let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
        if structure.bookmarks {
            let outlines_id = document.new_object_id();
            let item_id = document.add_object(dictionary! { "Title" => Object::string_literal("Chapter 1"), "Parent" => outlines_id, "Dest" => vec![Object::Reference(page_ids[0]), "Fit".into()] });
            document.objects.insert(outlines_id, dictionary! { "Type" => "Outlines", "First" => item_id, "Last" => item_id, "Count" => 1 }.into());
            catalog.set("Outlines", outlines_id);
        }
        if structure.tags {
            let root_id = document.new_object_id();
            let paragraphs = page_ids.iter().map(|page| Object::Reference(document.add_object(dictionary! { "Type" => "StructElem", "S" => "P", "P" => root_id, "Pg" => *page, "K" => 0 }))).collect::<Vec<_>>();
            let parent_tree = document.add_object(dictionary! { "Nums" => paragraphs.iter().enumerate().flat_map(|(index, paragraph)| [Object::Integer(index as i64), Object::Array(vec![paragraph.clone()])]).collect::<Vec<_>>() });
            document.objects.insert(root_id, dictionary! { "Type" => "StructTreeRoot", "K" => paragraphs, "ParentTree" => parent_tree }.into());
            catalog.set("StructTreeRoot", root_id);
            catalog.set("MarkInfo", dictionary! { "Marked" => true });
        }
        let mut fields = Vec::new();
        if structure.form {
            fields.push(dictionary! {
                "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "T" => Object::string_literal("Name"), "V" => Object::string_literal("Ada"),
                "Rect" => vec![72.into(), 72.into(), 272.into(), 96.into()], "P" => page_ids[0],
            });
        }
        if structure.signed {
            let value = document.add_object(dictionary! {
                "Type" => "Sig", "Filter" => "Adobe.PPKLite", "SubFilter" => "adbe.pkcs7.detached",
                "ByteRange" => vec![0.into(), 0.into(), 0.into(), 0.into()], "Contents" => Object::String(vec![0; 16], lopdf::StringFormat::Hexadecimal),
            });
            fields.push(dictionary! {
                "Type" => "Annot", "Subtype" => "Widget", "FT" => "Sig", "T" => Object::string_literal("Signature1"), "V" => value,
                "Rect" => vec![0.into(), 0.into(), 0.into(), 0.into()], "P" => page_ids[0],
            });
        }
        if !fields.is_empty() {
            let field_ids = fields.into_iter().map(|field| Object::Reference(document.add_object(field))).collect::<Vec<_>>();
            document.get_dictionary_mut(page_ids[0]).unwrap().set("Annots", field_ids.clone());
            let mut form = dictionary! { "Fields" => field_ids };
            if structure.signed { form.set("SigFlags", 3); }
            catalog.set("AcroForm", form);
        }
        document.objects.insert(catalog_id, catalog.into());
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    pub(crate) fn zero_page_pdf(path: &Path) {
        structured_pdf(path, Structure::default());
    }

    pub(crate) fn password_protected_pdf(path: &Path) {
        fs::write(path, crate::kit::password::tests::pdf(Some(("owner-secret", crate::kit::TEST_PASSWORD)))).unwrap();
    }

    pub(crate) fn media_boxes(path: &Path) -> Vec<(i64, i64)> {
        let document = Document::load(path).unwrap();
        document.get_pages().values().map(|page| {
            let (x0, y0, x1, y1) = page_bounds(&document, *page).unwrap();
            ((x1 - x0).round() as i64, (y1 - y0).round() as i64)
        }).collect()
    }

    /// The parts a copy must not keep: a live signature, bookmarks and the structure tree.
    pub(crate) fn assert_nothing_carried(path: &Path) {
        let document = Document::load(path).unwrap();
        assert!(!has_signed_signature_field(&document), "{} still carries a signature", path.display());
        let catalog = document.catalog().unwrap();
        assert!(!catalog.has(b"Outlines"), "{} still carries bookmarks", path.display());
        assert!(!catalog.has(b"StructTreeRoot"), "{} still carries tags", path.display());
    }

    pub(crate) fn warning_kinds(outcome: &JobOutcome) -> Vec<WarningKind> {
        outcome.warnings.iter().map(|warning| warning.kind.clone()).collect()
    }

    const MIXED_SIZES: &[(i64, i64)] = &[(612, 792), (842, 595), (420, 595)];

    #[test]
    fn split_accepts_structured_and_signed_pdfs_and_notes_what_the_parts_do_not_keep() {
        let _lock = SPLIT_TEST_LOCK.lock().unwrap();
        if qpdf().is_err() {
            eprintln!("skipping: qpdf not available");
            return;
        }
        let root = split_fixture_root("structured");
        let input = root.join("contract.pdf");
        structured_pdf(&input, Structure { sizes: MIXED_SIZES, bookmarks: true, form: true, tags: true, signed: true });
        let original = fs::read(&input).unwrap();
        for (mode, split_mode, sizes) in [
            ("pages", SplitMode::EveryN { pages: 1 }, vec![vec![MIXED_SIZES[0]], vec![MIXED_SIZES[1]], vec![MIXED_SIZES[2]]]),
            ("ranges", ranges("1-2,3"), vec![MIXED_SIZES[..2].to_vec(), vec![MIXED_SIZES[2]]]),
            ("chunks", SplitMode::EveryN { pages: 2 }, vec![MIXED_SIZES[..2].to_vec(), vec![MIXED_SIZES[2]]]),
        ] {
            let folder = root.join(mode);
            fs::create_dir_all(&folder).unwrap();
            let outcome = split(&split_request(OutputLocation::CustomFolder(folder), split_mode), input.clone());

            assert!(outcome.error().is_none(), "{mode}: {:?}", outcome.error());
            assert_eq!(warning_kinds(&outcome), [
                WarningKind::SignatureRemoved,
                WarningKind::BookmarksNotCarried { into: "the parts".into() },
                WarningKind::TagsNotCarried { into: "the parts".into() },
            ], "{mode}");
            assert_eq!(outcome.output_paths().iter().map(|part| media_boxes(part)).collect::<Vec<_>>(), sizes, "{mode} keeps each page's own size");
            for part in outcome.output_paths() { assert_nothing_carried(part); }
            assert_eq!(fs::read(&input).unwrap(), original, "{mode} must not change the original");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn split_of_a_plain_pdf_has_no_notes() {
        let _lock = SPLIT_TEST_LOCK.lock().unwrap();
        if qpdf().is_err() {
            eprintln!("skipping: qpdf not available");
            return;
        }
        let root = split_fixture_root("plain-notes");
        let input = root.join("plain.pdf");
        structured_pdf(&input, Structure { sizes: MIXED_SIZES, form: true, ..Structure::default() });
        let outcome = split(&split_request(OutputLocation::CustomFolder(root.clone()), SplitMode::EveryN { pages: 1 }), input);

        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn split_refuses_password_protected_and_empty_pdfs_without_writing() {
        let _lock = SPLIT_TEST_LOCK.lock().unwrap();
        let root = split_fixture_root("refused");
        let locked = root.join("locked.pdf");
        password_protected_pdf(&locked);
        let empty = root.join("empty.pdf");
        zero_page_pdf(&empty);
        for (input, pointer) in [(&locked, Some(crate::kit::contracts::OutcomePointer::Tool { tool_id: "lock-unlock".into() })), (&empty, None)] {
            let original = fs::read(input).unwrap();
            let folder = root.join(input.file_stem().unwrap());
            fs::create_dir_all(&folder).unwrap();
            let outcome = split(&split_request(OutputLocation::CustomFolder(folder.clone()), SplitMode::EveryN { pages: 1 }), input.clone());

            let error = outcome.error().expect("refused");
            assert_eq!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput, "{}", input.display());
            assert_eq!(error.pointer, pointer, "{}", input.display());
            if pointer.is_some() { assert_eq!(error.message, PASSWORD_PROTECTED); }
            assert!(outcome.output_paths().is_empty());
            assert_eq!(fs::read_dir(&folder).unwrap().count(), 0, "{} wrote something", input.display());
            assert_eq!(fs::read(input).unwrap(), original);
        }
        let _ = fs::remove_dir_all(root);
    }

    const BUNDLED_QPDF_CHILD: &str = "TOOLBOX_TEST_BUNDLED_QPDF_CHILD";

    // Other tests resolve qpdf without the env lock, so the lookup environment is changed only in a child test process.
    #[cfg(unix)]
    #[test]
    fn pdf_commands_find_qpdf_in_the_bundled_resources_location() {
        if std::env::var_os(BUNDLED_QPDF_CHILD).is_none() {
            use std::os::unix::fs::PermissionsExt;
            let _guard = crate::kit::lock_process_env();
            let root = split_fixture_root("bundled-qpdf");
            let bundled = root.join("qpdf-bin/qpdf");
            fs::create_dir_all(bundled.parent().unwrap()).unwrap();
            fs::write(&bundled, b"#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&bundled, fs::Permissions::from_mode(0o755)).unwrap();
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["kit::pdf::remaining::tests::pdf_commands_find_qpdf_in_the_bundled_resources_location", "--exact", "--nocapture", "--test-threads=1"])
                .env(BUNDLED_QPDF_CHILD, &bundled)
                .env("TOOLBOX_RESOURCE_ROOT", &root)
                .env_remove("TOOLBOX_QPDF_PATH")
                .env("PATH", root.join("missing-path"))
                .output()
                .unwrap();
            let report = format!("{}{}", String::from_utf8_lossy(&child.stdout), String::from_utf8_lossy(&child.stderr));
            let _ = fs::remove_dir_all(root);
            assert!(child.status.success() && report.contains("1 passed"), "{report}");
            return;
        }
        assert_eq!(qpdf(), Ok(PathBuf::from(std::env::var_os(BUNDLED_QPDF_CHILD).unwrap())));
    }

    #[test]
    fn pdf_render_deadlines_are_per_file_plus_per_page() {
        assert_eq!(PDF_RENDER_TIME_PER_FILE, Duration::from_secs(15));
        assert_eq!(PDF_RENDER_TIME_PER_PAGE, Duration::from_secs(2));
        assert_eq!(pdf_render_time_limit(1), Duration::from_secs(17));
        assert_eq!(pdf_render_time_limit(100), Duration::from_secs(215));
        assert!(pdf_render_time_limit(usize::MAX) > pdf_render_time_limit(100), "a huge page count saturates instead of overflowing");
    }

    #[cfg(unix)]
    #[test]
    fn pdf_to_images_stops_a_slow_renderer_at_the_deadline_and_fails_only_that_file() {
        use std::os::unix::fs::PermissionsExt;

        let _split = SPLIT_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _guard = crate::kit::lock_process_env();
        let root = split_fixture_root("slow-renderer");
        let input = root.join("document.pdf");
        let output_folder = root.join("outputs");
        let renderer = root.join("renderer.sh");
        fs::create_dir_all(&output_folder).unwrap();
        split_fixture_pdf(&input, 3);
        fs::write(&renderer, "#!/bin/sh\nexec sleep 30\n").unwrap();
        let mut permissions = fs::metadata(&renderer).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&renderer, permissions).unwrap();
        let previous = std::env::var_os("TOOLBOX_PDFTOPPM_PATH");
        std::env::set_var("TOOLBOX_PDFTOPPM_PATH", &renderer);

        let outcomes = [None, Some(vec![0, 2])].map(|pages| {
            let started = Instant::now();
            let outcome = to_images_within(&PdfToImagesRequest {
                paths: vec![input.clone()], dpi: 72, format: "png".to_string(), page_range: None, pages,
                output_location: OutputLocation::CustomFolder(output_folder.clone()),
            }, input.clone(), |_| Duration::from_millis(200));
            (outcome, started.elapsed())
        });

        match previous {
            Some(value) => std::env::set_var("TOOLBOX_PDFTOPPM_PATH", value),
            None => std::env::remove_var("TOOLBOX_PDFTOPPM_PATH"),
        }
        for (outcome, elapsed) in outcomes {
            let error = outcome.error().expect("a renderer past its deadline fails the file");
            assert_eq!(error.kind, crate::kit::contracts::ErrorKind::LimitExceeded { limit: LimitKind::Time }, "{}", error.message);
            assert_eq!(error.message, "Rendering took longer than 200 milliseconds and was stopped. Nothing was saved.");
            assert!(outcome.output_paths().is_empty());
            assert!(elapsed < Duration::from_secs(10), "the stalled renderer must be stopped, took {elapsed:?}");
        }
        assert_eq!(fs::read_dir(&output_folder).unwrap().count(), 0, "a timed-out render leaves nothing behind");
        fs::remove_dir_all(root).unwrap();
    }
}
