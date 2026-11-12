use flate2::{write::ZlibEncoder, Compression};
use image::metadata::Orientation;
use image::{DynamicImage, GrayImage, Luma};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::kit::common::limits::{self, bytes_label, duration_label, StoppableHelper};
use crate::kit::common::{renamed_warning, FolderReservation, JobOutcome, OutputLocation, OutputNaming, OutputReservation};
use crate::kit::common::watermark::{WatermarkOptions, WatermarkPlacement, WatermarkPosition};
use crate::kit::contracts::{JpegColour, LimitKind, OutcomePointer, OutcomeWarning, SkipCode, SkipReason, ToolError, WarningKind};
use crate::kit::images::source::{self, ImageFamily};
use crate::kit::images::{self, OutputFormat};
use super::embedded_image::{self, Extracted, NotSaved};
use super::inline_image;
use super::metadata::{page_bounds, page_rotation};
use super::page_ranges::{pages_label, PageRanges};
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
    #[serde(default)] pub picks: Vec<PagePicks>,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PagePicks {
    pub path: PathBuf,
    pub pages: Vec<usize>,
}

impl PdfToImagesRequest {
    pub(crate) fn page_range(&self) -> Option<&str> {
        self.page_range.as_deref().filter(|value| !value.trim().is_empty())
    }

    fn picks_for(&self, input: &Path) -> Option<&Vec<usize>> {
        self.picks.iter().find(|picks| picks.path == input).map(|picks| &picks.pages).or(self.pages.as_ref())
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
    // embedded image data itself is never cropped or resampled. A camera
    // orientation still pending on the embedded data is turned in the matrix.
    fn content(&self, orientation: Orientation) -> String {
        let mut content = String::from("q ");
        if let Some([x, y, width, height]) = self.clip {
            content.push_str(&format!("{} {} {} {} re W n ", points(x), points(y), points(width), points(height)));
        }
        let matrix = CornerMap::of(orientation).matrix(self.image).map(points);
        content.push_str(&format!("{} cm /Im0 Do Q", matrix.join(" ")));
        content
    }
}

/// Where image's own `apply_orientation` moves a stored image's corners, so
/// pixels oriented before embedding and a JPEG turned by its placement follow
/// one rule. Positions are fractions of the upright image from its top-left:
/// the stored top-left corner, then one step across and one step down.
struct CornerMap { origin: (f32, f32), across: (f32, f32), down: (f32, f32) }

impl CornerMap {
    fn of(orientation: Orientation) -> Self {
        let mut probe = DynamicImage::ImageLuma8(GrayImage::from_fn(2, 2, |column, row| Luma([(column + 2 * row) as u8])));
        probe.apply_orientation(orientation);
        let probe = probe.into_luma8();
        let upright = |corner: u8| probe.enumerate_pixels().find(|(_, _, pixel)| pixel[0] == corner).map(|(column, row, _)| (column as f32, row as f32)).expect("orienting keeps every corner");
        let (origin, across, down) = (upright(0), upright(1), upright(2));
        CornerMap { origin, across: (across.0 - origin.0, across.1 - origin.1), down: (down.0 - origin.0, down.1 - origin.1) }
    }

    fn swaps_sides(&self) -> bool { self.across.0 == 0.0 }

    // The `cm` operands that draw the stored image's unit square (rows bottom
    // to top) upright into the rectangle [x, y, width, height].
    fn matrix(&self, [x, y, width, height]: [f32; 4]) -> [f32; 6] {
        let CornerMap { origin: (p0, q0), across: (p1, q1), down: (p2, q2) } = *self;
        [width * p1, -height * q1, -width * p2, height * q2, x + width * (p0 + p2), y + height * (1.0 - q0 - q2)]
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
    let Some(format) = rendered_image_format(&request.format.to_lowercase()) else { return failure(input, "PDF images must be JPEG, PNG or WebP.".to_string()); };
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
    let page_digits = page_count.to_string().len().max(2);
    let workspace = match SplitWorkspace::new() {
        Ok(workspace) => workspace,
        Err(error) => return failure(input, format!("Could not create a private image workspace: {error}")),
    };
    let prefix = workspace.path().join("page");
    if let Some(requested_pages) = request.picks_for(&input) {
        let pages = match selected_pdf_pages(&document, Some(requested_pages)) {
            Ok(pages) if !pages.is_empty() => pages,
            Ok(_) => return failure(input, "Select at least one PDF page.".to_string()),
            Err(error) => return failure(input, error),
        };
        let deadline = RenderDeadline::new(time_limit(pages.len()));
        let generated = match render_selected_pages(&renderer, &input, &prefix, dpi, &pages, &deadline) {
            Ok(outputs) => outputs,
            Err(error) => return JobOutcome::failure(input, error),
        };
        return materialize_rendered_images(&input, &request.output_location, &generated, format, page_digits);
    }

    let clamped = ranges.as_ref().map(|ranges| ranges.clamp(page_count));
    if let (Some(ranges), Some(clamped)) = (&ranges, &clamped) {
        if clamped.is_empty() {
            let message = format!("No pages left in {}. It has {}, so nothing was made.", ranges.spans().iter().map(ToString::to_string).collect::<Vec<_>>().join(", "), pages_label(page_count));
            return JobOutcome::skipped(input, SkipReason::new(SkipCode::RangeEmptyAfterClamping, message));
        }
    }
    let spans = clamped.as_ref().map_or_else(|| vec![None], |clamped| clamped.spans.iter().copied().map(Some).collect());
    let rendered_pages = spans.iter().map(|span| span.map_or(page_count, |(first, last)| (last + 1).saturating_sub(first))).sum::<usize>();
    let deadline = RenderDeadline::new(time_limit(rendered_pages));
    for span in spans {
        let mut command = Command::new(&renderer);
        command.arg("-png").arg("-r").arg(dpi.to_string());
        if let Some((first, last)) = span { command.arg("-f").arg(first.to_string()).arg("-l").arg(last.to_string()); }
        command.arg(&input).arg(&prefix);
        match deadline.run(command) {
            Ok(result) if result.status.success() => {}
            Ok(result) => return failure(input, stderr(result, "pdftoppm failed to render the PDF.")),
            Err(error) => return JobOutcome::failure(input, error),
        }
    }
    let mut outputs = fs::read_dir(workspace.path()).ok().into_iter().flatten().filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|ext| ext == "png")).filter_map(|path| rendered_page_index(&path, &prefix).map(|page| (page, path))).collect::<Vec<_>>();
    outputs.sort();
    if outputs.is_empty() { return failure(input, "PDF renderer produced no images.".to_string()); }
    let outcome = materialize_rendered_images(&input, &request.output_location, &outputs, format, page_digits);
    match clamped {
        Some(clamped) if !clamped.notes.is_empty() && outcome.error().is_none() => {
            let pages = outputs.iter().map(|(page, _)| *page).collect::<Vec<_>>();
            let name = input.file_name().map(|value| value.to_string_lossy().into_owned()).unwrap_or_default();
            let detail = format!("{} {}. Range shortened to {}, because {name} has {}.", pages.len(), if pages.len() == 1 { "image" } else { "images" }, describe_pages(&pages), pages_label(page_count));
            JobOutcome { detail, ..outcome }
        }
        _ => outcome,
    }
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

fn rendered_image_format(format: &str) -> Option<OutputFormat> {
    let declared = crate::kit::contracts::native_capability("pdf_to_images")
        .and_then(|capability| capability.output_formats.as_deref())
        .is_some_and(|formats| formats.iter().any(|extension| extension.strip_prefix('.') == Some(format)));
    match format {
        _ if !declared => None,
        "jpg" => Some(OutputFormat::Jpeg),
        "png" => Some(OutputFormat::Png),
        "webp" => Some(OutputFormat::WebP),
        _ => None,
    }
}

fn rendered_page_index(path: &Path, prefix: &Path) -> Option<u32> {
    let stem = path.file_stem()?.to_str()?;
    let prefix = prefix.file_name()?.to_str()?;
    stem.strip_prefix(&format!("{prefix}-"))?.parse().ok()
}

fn render_selected_pages(renderer: &PathBuf, input: &PathBuf, prefix: &Path, dpi: u16, pages: &[u32], deadline: &RenderDeadline) -> Result<Vec<(u32, PathBuf)>, ToolError> {
    let mut outputs = Vec::with_capacity(pages.len());
    let base = prefix.file_name().and_then(|value| value.to_str()).unwrap_or("page");
    for page in pages {
        let page_prefix = prefix.with_file_name(format!("{base}-{page}"));
        let output = page_prefix.with_extension("png");
        let mut command = Command::new(renderer);
        command
            .arg("-png")
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
        outputs.push((*page, output));
    }
    Ok(outputs)
}

fn materialize_rendered_images(input: &Path, location: &OutputLocation, generated: &[(u32, PathBuf)], format: OutputFormat, page_digits: usize) -> JobOutcome {
    let mut outputs = Vec::with_capacity(generated.len());
    let mut renamed = Vec::new();
    let stem = input.file_stem().map(|value| value.to_string_lossy().into_owned()).unwrap_or_default();
    for (page, source) in generated {
        let bytes = match encode_rendered_page(source, format) {
            Ok(bytes) => bytes,
            Err(error) => return JobOutcome::failure_with_outputs(input.to_path_buf(), outputs, ToolError::processing(error)),
        };
        let suffix = format!("-page-{page:0page_digits$}");
        let reservation = match OutputNaming::reserve_destination(input, location, &suffix, format.extension()) {
            Ok(reservation) => reservation,
            Err(error) => return JobOutcome::failure_with_outputs(input.to_path_buf(), outputs, ToolError::processing(format!("Could not reserve PDF image output: {error}"))),
        };
        if let Err(error) = fs::write(reservation.path(), bytes) {
            return JobOutcome::failure_with_outputs(input.to_path_buf(), outputs, ToolError::processing(format!("Could not write rendered PDF image: {error}")));
        }
        let destination = match reservation.publish() {
            Ok(path) => path,
            Err(error) => return JobOutcome::failure_with_outputs(input.to_path_buf(), outputs, ToolError::processing(format!("Could not publish rendered PDF image: {error}"))),
        };
        let chosen_name = format!("{stem}{suffix}.{}", format.extension());
        let final_name = destination.file_name().map(|value| value.to_string_lossy().into_owned()).unwrap_or_default();
        if final_name != chosen_name { renamed.push(renamed_warning(chosen_name, final_name)); }
        outputs.push(destination);
    }
    let pages = generated.iter().map(|(page, _)| *page).collect::<Vec<_>>();
    let detail = format!("{} {}, {}", outputs.len(), if outputs.len() == 1 { "image" } else { "images" }, describe_pages(&pages));
    renamed.into_iter().fold(JobOutcome::succeeded(input.to_path_buf(), outputs, detail), JobOutcome::with_warning)
}

fn describe_pages(pages: &[u32]) -> String {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &page in pages {
        match runs.last_mut() {
            Some((_, last)) if *last + 1 == page => *last = page,
            _ => runs.push((page, page)),
        }
    }
    let parts = runs.iter().flat_map(|&(first, last)| match last - first {
        0 => vec![first.to_string()],
        1 => vec![first.to_string(), last.to_string()],
        _ => vec![format!("{first}-{last}")],
    }).collect::<Vec<_>>();
    let noun = if pages.len() == 1 { "page" } else { "pages" };
    match parts.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{noun} {} and {last}", rest.join(", ")),
        _ => format!("{noun} {}", parts.join("")),
    }
}

fn encode_rendered_page(source: &Path, format: OutputFormat) -> Result<Vec<u8>, String> {
    if format == OutputFormat::Png {
        return fs::read(source).map_err(|error| format!("Could not read rendered PDF image: {error}"));
    }
    let page = image::open(source).map_err(|error| format!("Could not read rendered PDF image: {error}"))?;
    images::encode(&page, format, images::DEFAULT_QUALITY)
}

/// Text mode on a PDF without a text layer: the file is skipped and pointed at OCR, so nothing is written.
pub const NEEDS_OCR_MESSAGE: &str = "This PDF has no text layer. Run OCR to read its text.";
pub const NO_EMBEDDED_IMAGES: &str = "No images in the selected pages. No folder was made.";
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
/// `preserveLayout` keeps columns lined up in the text; off, the text follows reading order.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractFromPdfRequest {
    pub paths: Vec<PathBuf>,
    pub mode: ExtractMode,
    #[serde(default)] pub page_range: Option<String>,
    pub output_location: OutputLocation,
    #[serde(default)] pub preserve_layout: bool,
    #[serde(default)] pub min_image_size: Option<u32>,
}

pub const PDF_DAMAGED: &str = "This file is damaged or isn't a PDF, so it couldn't be read.";
pub const EXTRACT_PASSWORD_PROTECTED: &str = "This PDF is password-protected. Remove the password, then extract again.";
pub const PDF_TEXT_HELPER_MISSING: &str = "Not read: the text helper is missing. Reinstall Toolbox to restore it.";
pub(crate) const PDF_TEXT_TIME_PER_FILE: Duration = Duration::from_secs(20);
pub(crate) const PDF_TEXT_TIME_PER_PAGE: Duration = Duration::from_millis(50);
pub(crate) const PDF_TEXT_FILE_MAX_BYTES: usize = 100_000_000;

/// The pdftotext that Extract from PDF runs, and how long it may take for one PDF.
struct TextHelper {
    path: Result<PathBuf, String>,
    per_file: Duration,
    per_page: Duration,
}

impl TextHelper {
    fn bundled() -> Self {
        Self { path: crate::kit::resources::resolve_helper(&crate::kit::resources::PDF_TEXT_EXTRACTOR), per_file: PDF_TEXT_TIME_PER_FILE, per_page: PDF_TEXT_TIME_PER_PAGE }
    }

    fn time_limit(&self, page_count: usize) -> Duration {
        self.per_file + self.per_page * u32::try_from(page_count).unwrap_or(u32::MAX)
    }
}

/// Saves the text, the images, or both from one PDF and reports them as one outcome.
pub fn extract(request: &ExtractFromPdfRequest, input: PathBuf) -> JobOutcome {
    extract_with(request, input, &TextHelper::bundled())
}

fn extract_with(request: &ExtractFromPdfRequest, input: PathBuf, helper: &TextHelper) -> JobOutcome {
    let document = match Document::load(&input) {
        Ok(document) if document.is_encrypted() => return password_protected(input),
        Ok(document) => document,
        Err(lopdf::Error::InvalidPassword | lopdf::Error::Decryption(_)) => return password_protected(input),
        Err(_) => return JobOutcome::failure(input, ToolError::invalid_input(PDF_DAMAGED)),
    };
    let pages = match pages_in_range(&document, request.page_range.as_deref()) {
        Ok(pages) if !pages.is_empty() => pages,
        Ok(_) => return JobOutcome::failure(input, ToolError::invalid_input("Select at least one PDF page.")),
        Err(PageSelection::Empty(reason)) => return JobOutcome::skipped(input, reason),
        Err(PageSelection::Invalid(error)) => return JobOutcome::failure(input, ToolError::invalid_input(error)),
    };
    let text = if request.mode.wants_text() {
        match write_text(&input, &pages, helper, request.preserve_layout, &request.output_location) {
            Ok(text) => text,
            Err(error) => return JobOutcome::failure(input, error),
        }
    } else { None };
    let images = if request.mode.wants_images() {
        let written = discover_images(&document, &pages).map_err(|error| ImageFailure { folder: None, message: error, warnings: Vec::new() })
            .and_then(|candidates| write_images(&document, &input, &request.output_location, &candidates, request.min_image_size));
        match written {
            Ok(images) => images,
            Err(ImageFailure { folder, message, warnings }) => {
                let outputs = text.map(|text| text.path).into_iter().chain(folder).collect();
                return warnings.into_iter().fold(JobOutcome::failure_with_outputs(input, outputs, ToolError::processing(message)), JobOutcome::with_warning);
            }
        }
    } else { ImageRun::default() };
    let tally = images.tally();
    let all_small = images.all_small();
    let skipped = |message: String| JobOutcome::skipped(input.clone(), SkipReason::new(SkipCode::NoEffectiveChange, message));
    let blank_pages = text.as_ref().map(|text| text.blank_pages.clone()).unwrap_or_default();
    let text = text.map(|text| text.path);
    let outcome = match (request.mode, text, images.saved) {
        (ExtractMode::Text, Some(text), _) => JobOutcome::succeeded(input, vec![text], if request.preserve_layout { "PDF text extracted with its layout preserved" } else { "PDF text extracted in reading order" }),
        (ExtractMode::Text, None, _) => JobOutcome::skipped(input, SkipReason::new(SkipCode::NeedsOcr, NEEDS_OCR_MESSAGE).with_pointer(ocr_pointer())),
        (ExtractMode::Images, _, None) => skipped(all_small.map_or(NO_EMBEDDED_IMAGES.to_string(), |lead| format!("{lead}. No folder was made."))),
        (ExtractMode::Images, _, Some(saved)) => JobOutcome::succeeded(input, vec![saved.folder], format!("Saved {tally}.")),
        (ExtractMode::Both, Some(text), Some(saved)) => JobOutcome::succeeded(input, vec![text, saved.folder], format!("Saved the text and {tally}.")),
        (ExtractMode::Both, Some(text), None) => JobOutcome::succeeded(input, vec![text], all_small.map_or(BOTH_WITHOUT_IMAGES.to_string(), |lead| format!("{lead}, so only the text file was saved."))),
        (ExtractMode::Both, None, Some(saved)) => JobOutcome::succeeded(input, vec![saved.folder], format!("{BOTH_WITHOUT_TEXT} Saved {tally}.")),
        (ExtractMode::Both, None, None) => skipped(all_small.map_or(NOTHING_TO_EXTRACT.to_string(), |lead| format!("No selectable text, and {}{}. Nothing was saved.", lead[..1].to_lowercase(), &lead[1..]))),
    };
    let outcome = if blank_pages.is_empty() { outcome } else { outcome.with_warning(pages_without_text(blank_pages)) };
    images.warnings.into_iter().fold(outcome, JobOutcome::with_warning)
}

fn ocr_pointer() -> OutcomePointer { OutcomePointer::Tool { tool_id: "pdf-ocr".into() } }

/// Why a PDF's pages could not be taken from the shared range.
#[derive(Debug, PartialEq)]
enum PageSelection {
    /// The range itself is malformed.
    Invalid(String),
    /// One range applies to a whole batch, so a PDF too short for every page in it is skipped, not failed.
    Empty(SkipReason),
}

/// Pages the text helper read as blank while other pages had text: likely scanned, so OCR can read them.
fn pages_without_text(pages: Vec<u32>) -> OutcomeWarning {
    let message = if pages.len() == 1 { format!("{} has no selectable text. OCR can read it.", page_list(&pages)) } else { format!("{} have no selectable text. OCR can read them.", page_list(&pages)) };
    OutcomeWarning { kind: WarningKind::PagesWithoutText { pages }, message }
}

/// Resolves the shared range syntax against one PDF into 1-based page numbers in document order.
/// The range is clamped to this PDF's length: entries past its last page are dropped and open or long runs end there.
fn pages_in_range(document: &Document, range: Option<&str>) -> Result<Vec<u32>, PageSelection> {
    let count = document.get_pages().len();
    let Some(raw) = range.filter(|value| !value.trim().is_empty()) else { return Ok((1..=count as u32).collect()); };
    let clamped = match PageRanges::parse(raw).map_err(PageSelection::Invalid)?.clamp_or_skip(count) {
        Ok(clamped) => clamped,
        Err(reason) => return Err(PageSelection::Empty(reason)),
    };
    Ok(clamped.page_indices().into_iter().map(|index| index as u32 + 1).collect())
}

fn password_protected(input: PathBuf) -> JobOutcome {
    JobOutcome::failure(input, ToolError::invalid_input(EXTRACT_PASSWORD_PROTECTED).with_pointer(OutcomePointer::Tool { tool_id: "lock-unlock".into() }))
}

/// A written text file, and the selected pages that had no selectable text in it.
struct TextFile {
    path: PathBuf,
    blank_pages: Vec<u32>,
}

/// Writes the selected pages' text with pdftotext. `None` means the pages hold no selectable text, and nothing is written.
fn write_text(input: &Path, pages: &[u32], helper: &TextHelper, preserve_layout: bool, location: &OutputLocation) -> Result<Option<TextFile>, ToolError> {
    let Ok(pdftotext) = &helper.path else { return Err(ToolError::unavailable(PDF_TEXT_HELPER_MISSING)) };
    let limit = helper.time_limit(pages.len());
    let deadline = Instant::now() + limit;
    let timed_out = format!("Reading text took longer than {}, so it was stopped. The other PDFs weren't affected.", duration_label(Duration::from_secs(limit.as_secs())));
    let mut text = Vec::new();
    for (first, last) in page_runs(pages) {
        let mut command = Command::new(pdftotext);
        command.args(["-enc", "UTF-8", "-eol", "unix"]);
        if preserve_layout { command.arg("-layout"); }
        command.arg("-f").arg(first.to_string()).arg("-l").arg(last.to_string()).arg(input).arg("-");
        let output = match super::run_bounded_helper(command, deadline, PDF_TEXT_FILE_MAX_BYTES - text.len(), super::PDF_TEXT_MAX_ERROR_BYTES, &timed_out) {
            Ok(output) => output,
            Err(message) if message == timed_out => return Err(ToolError::processing(message)),
            Err(message) if message.contains("stdout output exceeds") => return Err(ToolError::processing(format!("The text is over {}, so nothing was saved.", bytes_label(PDF_TEXT_FILE_MAX_BYTES as u64)))),
            Err(message) => return Err(ToolError::processing(message)),
        };
        if !output.status.success() { return Err(ToolError::processing(stderr(output, "The text helper couldn't read this PDF."))); }
        text.extend_from_slice(&output.stdout);
    }
    let text = unix_text(&String::from_utf8_lossy(&text));
    if is_blank(&text) { return Ok(None); }
    let blank_pages = blank_pages(&text, pages);
    let output = OutputNaming::reserve_destination(input, location, "-text", "txt").map_err(|error| ToolError::processing(format!("Could not reserve PDF text output: {error}")))?;
    fs::write(output.path(), text).map_err(|error| ToolError::processing(format!("Could not write text output: {error}")))?;
    let path = output.publish().map_err(|error| ToolError::processing(format!("Could not publish text output: {error}")))?;
    Ok(Some(TextFile { path, blank_pages }))
}

fn is_blank(text: &str) -> bool { text.trim_matches(|character: char| character.is_whitespace() || character == '\u{c}').is_empty() }

/// pdftotext ends every page with a form feed, so the text splits into one part per selected page in order.
/// When the parts and pages don't line up, no page is named rather than naming the wrong ones.
fn blank_pages(text: &str, pages: &[u32]) -> Vec<u32> {
    let parts = text.split('\u{c}').collect::<Vec<_>>();
    if parts.len() != pages.len() { return Vec::new(); }
    pages.iter().zip(parts).filter(|(_, part)| is_blank(part)).map(|(page, _)| *page).collect()
}

/// Groups sorted page numbers into the contiguous runs pdftotext reads with -f and -l.
fn page_runs(pages: &[u32]) -> Vec<(u32, u32)> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &page in pages {
        match runs.last_mut() {
            Some((_, last)) if *last + 1 == page => *last = page,
            _ => runs.push((page, page)),
        }
    }
    runs
}

/// LF line endings only, with pdftotext's form feed between pages but not after the last one.
fn unix_text(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    text.strip_suffix('\u{c}').map(str::to_string).unwrap_or(text)
}


fn normalize_pdf_text(text: &str) -> String {
    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// Where one discovered image's data lives: an image XObject, or an inline image copied out of a content stream.
#[derive(Debug)]
enum ImageSource {
    Object(ObjectId),
    Inline(Stream),
}

/// One image found in the selected pages, in discovery order.
#[derive(Debug)]
struct ImageCandidate {
    /// One-based page number.
    page: u32,
    /// Position among the page's images.
    index: usize,
    source: ImageSource,
    /// Zero when the image doesn't state its size.
    width: i64,
    height: i64,
}

/// Form XObjects nested deeper than this are not searched.
const MAX_FORM_DEPTH: usize = 16;

/// Lists the images of the selected one-based pages in page order. Within a page, the image XObjects in its
/// resources come first in resource order, with each form XObject's images in its place, then the page's
/// inline images in content order. A form lists its own resources' images and then its inline images.
fn discover_images(document: &Document, pages: &[u32]) -> Result<Vec<ImageCandidate>, String> {
    let mut candidates = Vec::new();
    for (page_number, page_id) in document.get_pages() {
        if !pages.contains(&page_number) { continue; }
        let (direct, inherited) = document.get_page_resources(page_id).map_err(|error| error.to_string())?;
        let resources = direct.or_else(|| inherited.first().and_then(|id| document.get_dictionary(*id).ok()));
        let content = document.get_page_content_with_limit(page_id, embedded_image::MAX_DECODED_BYTES as usize).unwrap_or_default();
        let mut found = Vec::new();
        let resources = resources.into_iter().collect::<Vec<_>>();
        collect_images(document, &resources, &resources, &content, &mut Vec::new(), &mut found);
        candidates.extend(found.into_iter().enumerate().map(|(index, (source, width, height))| ImageCandidate { page: page_number, index, source, width, height }));
    }
    Ok(candidates)
}

fn collect_images(document: &Document, own: &[&lopdf::Dictionary], scope: &[&lopdf::Dictionary], content: &[u8], open_forms: &mut Vec<ObjectId>, found: &mut Vec<(ImageSource, i64, i64)>) {
    let side = |dict: &lopdf::Dictionary, key: &[u8]| dict.get(key).and_then(Object::as_i64).unwrap_or(0);
    let xobjects = own.iter().filter_map(|resources| resources.get(b"XObject").ok().and_then(|xobjects| document.dereference(xobjects).ok()).and_then(|(_, xobjects)| xobjects.as_dict().ok()));
    for (_, value) in xobjects.flat_map(|xobjects| xobjects.iter()) {
        let Ok(id) = value.as_reference() else { continue };
        let Ok(stream) = document.get_object(id).and_then(Object::as_stream) else { continue };
        match stream.dict.get(b"Subtype").and_then(Object::as_name) {
            Ok(b"Image") => found.push((ImageSource::Object(id), side(&stream.dict, b"Width"), side(&stream.dict, b"Height"))),
            Ok(b"Form") if !open_forms.contains(&id) && open_forms.len() < MAX_FORM_DEPTH => {
                let form = stream.dict.get(b"Resources").ok().and_then(|resources| document.dereference(resources).ok()).and_then(|(_, resources)| resources.as_dict().ok());
                let form = form.into_iter().collect::<Vec<_>>();
                let scope = form.iter().chain(scope).copied().collect::<Vec<_>>();
                let content = stream.decompressed_content_with_limit(embedded_image::MAX_DECODED_BYTES as usize).unwrap_or_default();
                open_forms.push(id);
                collect_images(document, &form, &scope, &content, open_forms, found);
                open_forms.pop();
            }
            _ => {}
        }
    }
    let named_space = |name: &[u8]| scope.iter().find_map(|resources| {
        let (_, spaces) = document.dereference(resources.get(b"ColorSpace").ok()?).ok()?;
        spaces.as_dict().ok()?.get(name).ok().cloned()
    });
    for image in inline_image::inline_images(content, named_space) {
        let (width, height) = (side(&image.dict, b"Width"), side(&image.dict, b"Height"));
        found.push((ImageSource::Inline(image), width, height));
    }
}

/// One PDF's images folder and how many images were published into it.
#[derive(Debug, PartialEq)]
struct SavedImages {
    folder: PathBuf,
    count: usize,
}

fn image_count(count: usize) -> String {
    format!("{count} {}", if count == 1 { "image" } else { "images" })
}

/// One PDF's images run: the folder saved, the images left out as duplicates, as smaller than the minimum size
/// or as undecodable, and a warning for each undecodable image followed by one per kind of JPEG kept with
/// colours some viewers get wrong.
#[derive(Debug, Default)]
struct ImageRun {
    saved: Option<SavedImages>,
    duplicates: usize,
    small: usize,
    undecodable: usize,
    warnings: Vec<OutcomeWarning>,
}

impl ImageRun {
    /// "17 images · 4 duplicates · 6 small left out · 2 couldn't be saved", leaving out counts of zero.
    fn tally(&self) -> String {
        let saved = self.saved.as_ref().map_or(0, |saved| saved.count);
        let mut parts = vec![image_count(saved)];
        if self.duplicates > 0 { parts.push(format!("{} {}", self.duplicates, if self.duplicates == 1 { "duplicate" } else { "duplicates" })); }
        if self.small > 0 { parts.push(format!("{} small left out", self.small)); }
        if self.undecodable > 0 { parts.push(format!("{} couldn't be saved", self.undecodable)); }
        parts.join(" · ")
    }

    /// Why no images were saved when every image found was smaller than the minimum size.
    fn all_small(&self) -> Option<String> {
        match self.small {
            0 => None,
            1 => Some("The 1 image is smaller than the minimum size".to_string()),
            count => Some(format!("All {count} images are smaller than the minimum size")),
        }
    }
}

#[derive(Debug)]
struct ImageFailure {
    folder: Option<PathBuf>,
    message: String,
    warnings: Vec<OutcomeWarning>,
}

/// Saves each distinct candidate at least `min_size` on both sides into one new "{stem}-images" folder: JPEG and
/// JPEG 2000 data as it is, every other image as a lossless PNG. An image object used again, and an image whose
/// saved bytes match one already saved, is a duplicate and saved once, under the first page where it appears.
/// A small image is counted once however often it is used. An image that can't be decoded is reported with its
/// page and left out; the rest are still saved. No folder is made when nothing is saved, and it fails when
/// images were found but none could be saved. On failure nothing is published, except the images published
/// before a later publish failed, whose folder is then reported with the error.
fn write_images(document: &Document, input: &Path, location: &OutputLocation, candidates: &[ImageCandidate], min_size: Option<u32>) -> Result<ImageRun, ImageFailure> {
    use sha2::{Digest, Sha256};
    let mut run = ImageRun::default();
    let mut seen_objects = std::collections::HashSet::new();
    let mut seen_bytes = std::collections::HashSet::new();
    let mut colours = [(JpegColour::Cmyk, Vec::new()), (JpegColour::Inverted, Vec::new())];
    let mut folder = None;
    let mut reservations = Vec::new();
    for candidate in candidates {
        let first_use = match candidate.source { ImageSource::Object(id) => seen_objects.insert(id), ImageSource::Inline(_) => true };
        let small = min_size.is_some_and(|min| (candidate.width > 0 && candidate.width < i64::from(min)) || (candidate.height > 0 && candidate.height < i64::from(min)));
        if small {
            if first_use { run.small += 1; }
            continue;
        }
        if !first_use { run.duplicates += 1; continue; }
        let (page, image) = (candidate.page, candidate.index as u32 + 1);
        let stream = match &candidate.source {
            ImageSource::Object(id) => document.get_object(*id).and_then(Object::as_stream).map_err(|_| NotSaved::Damaged),
            ImageSource::Inline(stream) => Ok(stream),
        };
        let extracted = match stream.and_then(|stream| embedded_image::extract(document, stream)) {
            Ok(extracted) => extracted,
            Err(reason) => {
                run.undecodable += 1;
                run.warnings.push(not_saved_warning(page, image, reason));
                continue;
            }
        };
        if !seen_bytes.insert(Sha256::new().chain_update(extracted.extension()).chain_update([0]).chain_update(extracted.bytes()).finalize()) {
            run.duplicates += 1;
            continue;
        }
        if let Extracted::Jpeg { cmyk, inverted, .. } = extracted {
            if cmyk { colours[0].1.push(page); }
            if inverted { colours[1].1.push(page); }
        }
        let fail = |message: String, warnings: Vec<OutcomeWarning>| ImageFailure { folder: None, message, warnings };
        let folder = match &mut folder {
            Some(folder) => folder,
            None => match OutputNaming::reserve_folder(input, location, "-images") {
                Ok(reserved) => folder.insert(reserved),
                Err(error) => return Err(fail(format!("Could not create the images folder: {error}"), run.warnings)),
            },
        };
        let name = format!("page-{page}-image-{image}.{}", extracted.extension());
        let reservation = match OutputNaming::reserve_named_candidate(&folder.path().join(&name)) {
            Ok(Some(reservation)) => reservation,
            Ok(None) => return Err(fail(format!("Could not reserve extracted image output: {name} already exists"), run.warnings)),
            Err(error) => return Err(fail(format!("Could not reserve extracted image output: {error}"), run.warnings)),
        };
        if let Err(error) = fs::write(reservation.path(), extracted.bytes()) { return Err(fail(format!("Could not write extracted image: {error}"), run.warnings)); }
        reservations.push(reservation);
    }
    run.warnings.extend(colours.into_iter().filter(|(_, pages)| !pages.is_empty()).map(|(colour, pages)| colour_warning(colour, pages)));
    let Some(folder) = folder else {
        if run.undecodable == 0 { return Ok(run); }
        let message = if run.undecodable == 1 { "The image couldn't be decoded, so nothing was saved.".to_string() } else { format!("None of the {} could be decoded, so nothing was saved.", image_count(run.undecodable)) };
        return Err(ImageFailure { folder: None, message, warnings: run.warnings });
    };
    match publish_extracted_images(folder, reservations) {
        Ok(saved) => Ok(ImageRun { saved: Some(saved), ..run }),
        Err((folder, message)) => Err(ImageFailure { folder, message, warnings: run.warnings }),
    }
}

fn not_saved_warning(page: u32, image: u32, reason: NotSaved) -> OutcomeWarning {
    let (kind, reason) = match reason {
        NotSaved::Unsupported(feature) => {
            let reason = format!("{feature} isn't supported in this version");
            (WarningKind::ImageUnsupported { page, image, feature }, reason)
        }
        NotSaved::Damaged => (WarningKind::ImageDamaged { page, image }, "The image data is damaged".to_string()),
        NotSaved::TooLarge { width, height } => (
            WarningKind::ImageTooLarge { page, image, width, height },
            format!("Would be {} × {} px when decoded, over the size limit", grouped(width), grouped(height)),
        ),
    };
    OutcomeWarning { kind, message: format!("Page {page}, image {image} — {reason}. Not saved.") }
}

fn colour_warning(colour: JpegColour, mut pages: Vec<u32>) -> OutcomeWarning {
    let images = pages.len() as u32;
    pages.dedup();
    let place = page_list(&pages);
    let message = match (colour, images) {
        (JpegColour::Cmyk, 1) => format!("{place} — 1 JPEG uses CMYK colour and was copied as it is. Some viewers show it with the wrong colours."),
        (JpegColour::Cmyk, _) => format!("{place} — {images} JPEGs use CMYK colour and were copied as they are. Some viewers show them with the wrong colours."),
        (JpegColour::Inverted, 1) => format!("{place} — 1 inverted JPEG, copied as it is. Some viewers show it in negative."),
        (JpegColour::Inverted, _) => format!("{place} — {images} inverted JPEGs, copied as they are. Some viewers show them in negative."),
    };
    OutcomeWarning { kind: WarningKind::JpegColourKept { colour, pages, images }, message }
}

/// "Page 4" or "Pages 1-3, 7", with runs of consecutive pages joined.
fn page_list(pages: &[u32]) -> String {
    let runs = page_runs(pages).into_iter().map(|(first, last)| if first == last { first.to_string() } else { format!("{first}-{last}") }).collect::<Vec<_>>();
    format!("{} {}", if pages.len() == 1 { "Page" } else { "Pages" }, runs.join(", "))
}

fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 { grouped.push(','); }
        grouped.push(digit);
    }
    grouped
}

/// Publishes each image into its reserved folder in order. The folder is kept once one image is published.
fn publish_extracted_images(folder: FolderReservation, reservations: Vec<OutputReservation>) -> Result<SavedImages, (Option<PathBuf>, String)> {
    let mut count = 0;
    let mut pending = reservations.into_iter();
    while let Some(reservation) = pending.next() {
        if let Err(error) = reservation.publish() {
            drop(pending);
            if count == 0 { return Err((None, format!("Could not publish extracted image: {error}"))); }
            let folder = folder.publish();
            let name = folder.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            return Err((Some(folder), format!("Could not publish extracted image: {error}. The {} before it {} saved in {name}.", image_count(count), if count == 1 { "was" } else { "were" })));
        }
        count += 1;
    }
    Ok(SavedImages { folder: folder.publish(), count })
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
            let content_id = document.add_object(Stream::new(dictionary! {}, placement.content(image.orientation).into_bytes()));
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

// Width and height are the upright size; orientation is what the page
// placement still has to apply to the embedded data.
struct EmbeddedImage {
    id: lopdf::ObjectId,
    width: u32,
    height: u32,
    orientation: Orientation,
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

// A baseline JPEG goes in as its own bytes and is turned upright by its
// placement; everything else is oriented, then stored without loss as 8-bit
// Flate samples, with any alpha kept as a soft mask. A TIFF gives one image
// per page, in page order, and its orientation tag applies to every page.
fn embed_images(document: &mut Document, path: &Path) -> Result<Vec<EmbeddedImage>, String> {
    if !matches!(ImageFamily::of(path), ImageFamily::Jpeg | ImageFamily::Png | ImageFamily::WebP | ImageFamily::Heif | ImageFamily::Tiff) {
        return Err("Only PNG, JPEG, WebP, HEIC, HEIF and TIFF images can be combined into a PDF.".to_string());
    }
    let decoded = source::decode(path)?;
    let reduced_from_bits = (decoded.bits_per_channel > 8).then_some(decoded.bits_per_channel);
    let baseline_jpeg = decoded.baseline_jpeg.and_then(|bytes| jpeg_color_space(&bytes).map(|space| (bytes, space)));
    let orientation = decoded.orientation.unwrap_or(Orientation::NoTransforms);
    let mut images = Vec::with_capacity(decoded.pages.len());
    for mut page in decoded.pages {
        if let Some((jpeg, color_space)) = &baseline_jpeg {
            let (width, height) = (page.width(), page.height());
            let dict = dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64, "Height" => height as i64, "BitsPerComponent" => 8, "ColorSpace" => *color_space, "Filter" => "DCTDecode" };
            let (width, height) = if CornerMap::of(orientation).swaps_sides() { (height, width) } else { (width, height) };
            images.push(EmbeddedImage { id: document.add_object(Stream::new(dict, jpeg.clone())), width, height, orientation, reduced_from_bits });
            continue;
        }
        page.apply_orientation(orientation);
        let (width, height) = (page.width(), page.height());
        let image = dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64, "Height" => height as i64, "BitsPerComponent" => 8 };
        let (samples, color_space) = if page.color().has_color() { (page.to_rgb8().into_raw(), "DeviceRGB") } else { (page.to_luma8().into_raw(), "DeviceGray") };
        let mut dict = image;
        dict.set("ColorSpace", color_space);
        if page.color().has_alpha() {
            let alpha = page.to_rgba8().pixels().map(|pixel| pixel[3]).collect::<Vec<_>>();
            let mask = dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64, "Height" => height as i64, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 };
            dict.set("SMask", document.add_object(flate_stream(mask, &alpha)?));
        }
        images.push(EmbeddedImage { id: document.add_object(flate_stream(dict, &samples)?), width, height, orientation: Orientation::NoTransforms, reduced_from_bits });
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

pub const PDF_PASSWORD_PROTECTED: &str = "This PDF is password-protected. Unlock it first, then compress the unlocked copy.";
pub const NO_SAVINGS: &str = "No savings: already well compressed";
const PAGE_BOXES: [&[u8]; 5] = [b"MediaBox", b"CropBox", b"BleedBox", b"TrimBox", b"ArtBox"];

/// Repacks a PDF without loss: object streams, recompressed flate streams at level 9. The copy is
/// published only when it opens with the same pages, page boxes and rotation and is smaller.
pub fn compress_lossless(input: PathBuf, location: &OutputLocation) -> JobOutcome {
    let source = match Document::load(&input) {
        Ok(document) => document,
        Err(_) => return JobOutcome::failure(input, crate::kit::password::unreadable(crate::kit::password::ProtectedFormat::Pdf)),
    };
    if source.is_encrypted() || source.was_encrypted() {
        return JobOutcome::failure(input, ToolError::invalid_input(PDF_PASSWORD_PROTECTED)
            .with_pointer(OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
    }
    match super::is_signed(&source) {
        Ok(false) => {}
        Ok(true) => return JobOutcome::failure(input, ToolError::invalid_input("Digitally signed PDFs cannot be compressed because rewriting the file would invalidate the signature; use an unsigned copy")),
        Err(error) => return failure(input, error),
    }
    let geometry = match page_geometry(&source) { Ok(geometry) => geometry, Err(error) => return failure(input, error) };
    let qpdf = match qpdf() { Ok(qpdf) => qpdf, Err(error) => return JobOutcome::failure(input, ToolError::unavailable(error)) };
    let output = match OutputNaming::reserve_destination(&input, location, "-compressed", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve compressed PDF output: {error}")),
    };
    let result = Command::new(qpdf).arg("--object-streams=generate").arg("--recompress-flate").arg("--compression-level=9")
        .arg(&input).arg(output.path()).output_until_stopped();
    match result {
        // qpdf exits 3 when it succeeded with warnings; verification below decides whether the copy is sound.
        Ok(result) if result.status.success() || result.status.code() == Some(3) => {}
        Ok(result) => return failure(input, stderr(result, "qpdf failed to compress the PDF.")),
        Err(error) => return failure(input, format!("Could not run qpdf: {error}")),
    }
    match Document::load(output.path()).map_err(|error| error.to_string()).and_then(|copy| page_geometry(&copy)) {
        Ok(copy) if copy == geometry => {}
        Ok(_) => return failure(input, "The compressed copy changed its pages, page boxes or rotation, so nothing was saved.".to_string()),
        Err(error) => return failure(input, format!("Compressed PDF could not be verified: {error}")),
    }
    let sizes = fs::metadata(&input).and_then(|original| fs::metadata(output.path()).map(|copy| (original.len(), copy.len())));
    let (original_bytes, output_bytes) = match sizes { Ok(sizes) => sizes, Err(error) => return failure(input, format!("Could not measure the compressed PDF: {error}")) };
    if output_bytes >= original_bytes {
        drop(output);
        return JobOutcome::skipped(input, SkipReason::new(SkipCode::NoSavings, NO_SAVINGS)).with_sizes(original_bytes, output_bytes);
    }
    match output.publish() {
        Ok(path) => JobOutcome::succeeded(input, vec![path], "PDF repacked without loss").with_sizes(original_bytes, output_bytes),
        Err(error) => failure(input, format!("Could not publish compressed PDF: {error}")),
    }
}

#[derive(Debug, PartialEq)]
struct PageGeometry { boxes: Vec<Option<Vec<f32>>>, rotation: i64 }

fn page_geometry(document: &Document) -> Result<Vec<PageGeometry>, String> {
    document.get_pages().into_values().map(|page| {
        let boxes = PAGE_BOXES.iter().map(|key| super::inherited(document, page, key)?.map(|value| {
            value.as_array().map_err(|error| error.to_string())?.iter()
                .map(|number| super::resolve(document, number).and_then(super::metadata::number))
                .collect::<Result<Vec<_>, _>>()
        }).transpose()).collect::<Result<Vec<_>, String>>()?;
        Ok(PageGeometry { boxes, rotation: page_rotation(document, page) })
    }).collect()
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
    use crate::kit::pdf::jbig2_fixtures::{self, Jbig2Fixture};
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
                // Compressing repacks without loss, so only a signature, which any rewrite breaks, stops it.
                if operation == "compress" && kind != "signed" { continue; }
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
                    "compress" => compress_lossless(input.clone(), &output_location),
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
            paths: vec![input.clone()], dpi: 72, format: "png".to_string(), page_range: None, pages: None, picks: Vec::new(),
            output_location: OutputLocation::CustomFolder(output_folder.clone()),
        }, input.clone());

        match previous {
            Some(value) => std::env::set_var("TOOLBOX_PDFTOPPM_PATH", value),
            None => std::env::remove_var("TOOLBOX_PDFTOPPM_PATH"),
        }
        assert!(outcome.error().is_none(), "numeric image export failed: {:?}", outcome.error());
        assert_eq!(outcome.output_paths().len(), 12);
        for (index, output) in outcome.output_paths().iter().enumerate() {
            assert_eq!(output.file_stem().unwrap().to_str().unwrap(), format!("document-page-{:02}", index + 1));
            assert_eq!(fs::read_to_string(output).unwrap(), format!("page-{}", index + 1));
        }
        fs::remove_dir_all(root).unwrap();
    }

    fn render_with_bundled_pdftoppm(input: &Path, folder: &Path, format: &str, dpi: u16, pages: Option<Vec<usize>>, page_range: Option<&str>) -> JobOutcome {
        to_images(&PdfToImagesRequest {
            paths: vec![input.to_path_buf()], dpi, format: format.to_string(), page_range: page_range.map(str::to_string), pages, picks: Vec::new(),
            output_location: OutputLocation::CustomFolder(folder.to_path_buf()),
        }, input.to_path_buf())
    }

    fn output_names(outcome: &JobOutcome) -> Vec<String> {
        outcome.output_paths().iter().map(|path| path.file_name().unwrap().to_str().unwrap().to_string()).collect()
    }

    #[test]
    fn pdf_to_images_renders_only_selected_pages_at_the_requested_dpi_in_every_format() {
        let _guard = crate::kit::lock_process_env();
        if pdftoppm().is_none() { return; }
        let root = split_fixture_root("formats-images");
        let input = root.join("document.pdf");
        split_fixture_pdf(&input, 12);
        let original = fs::read(&input).unwrap();

        for (format, dpi, width, height) in [("jpg", 72, 612, 792), ("png", 200, 1700, 2200), ("webp", 150, 1275, 1650)] {
            let folder = root.join(format);
            fs::create_dir_all(&folder).unwrap();
            let outcome = render_with_bundled_pdftoppm(&input, &folder, format, dpi, Some(vec![1, 10]), None);
            assert!(outcome.error().is_none(), "{format} export failed: {:?}", outcome.error());
            assert_eq!(output_names(&outcome), [format!("document-page-02.{format}"), format!("document-page-11.{format}")]);
            assert_eq!(fs::read_dir(&folder).unwrap().count(), 2, "only the selected pages are written");
            for output in outcome.output_paths() {
                let decoded = image::open(output).unwrap_or_else(|error| panic!("{} does not decode: {error}", output.display()));
                assert_eq!((decoded.width(), decoded.height()), (width, height), "{format} at {dpi} dpi");
            }
        }
        assert_eq!(fs::read(&input).unwrap(), original, "the original PDF is never modified");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pdf_to_images_encodes_webp_lossy_and_jpeg_at_quality_ninety_from_the_lossless_render() {
        let _guard = crate::kit::lock_process_env();
        if pdftoppm().is_none() { return; }
        let root = split_fixture_root("quality-images");
        let input = root.join("document.pdf");
        split_fixture_pdf(&input, 1);
        let rendered = |format: &str| {
            let folder = root.join(format);
            fs::create_dir_all(&folder).unwrap();
            let outcome = render_with_bundled_pdftoppm(&input, &folder, format, 72, None, None);
            assert!(outcome.error().is_none(), "{format} export failed: {:?}", outcome.error());
            fs::read(&outcome.output_paths()[0]).unwrap()
        };
        let lossless = image::load_from_memory(&rendered("png")).unwrap();
        let webp = rendered("webp");
        assert_eq!(&webp[..4], b"RIFF");
        assert!(webp.windows(4).any(|chunk| chunk == b"VP8 "), "WebP output must use the lossy VP8 bitstream");
        assert!(!webp.windows(4).any(|chunk| chunk == b"VP8L"), "WebP output must not be lossless");
        assert_eq!(webp, images::encode(&lossless, OutputFormat::WebP, 90).unwrap());
        assert_eq!(rendered("jpg"), images::encode(&lossless, OutputFormat::Jpeg, 90).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pdf_to_images_pads_page_numbers_and_numbers_a_rerun_without_replacing() {
        let _guard = crate::kit::lock_process_env();
        if pdftoppm().is_none() { return; }
        let root = split_fixture_root("padded-images");
        let input = root.join("document.pdf");
        let folder = root.join("outputs");
        fs::create_dir_all(&folder).unwrap();
        split_fixture_pdf(&input, 12);
        let original = fs::read(&input).unwrap();

        let first = render_with_bundled_pdftoppm(&input, &folder, "png", 72, None, None);
        assert!(first.error().is_none(), "{:?}", first.error());
        let names = output_names(&first);
        assert_eq!(names, (1..=12).map(|page| format!("document-page-{page:02}.png")).collect::<Vec<_>>());
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(sorted, names, "names sort in page order");
        assert_eq!(first.detail, "12 images, pages 1-12");
        assert!(first.warnings.is_empty());
        let earlier = first.output_paths().iter().map(|path| fs::read(path).unwrap()).collect::<Vec<_>>();

        let second = render_with_bundled_pdftoppm(&input, &folder, "png", 72, None, None);
        assert!(second.error().is_none(), "{:?}", second.error());
        assert_eq!(output_names(&second), (1..=12).map(|page| format!("document-page-{page:02}-1.png")).collect::<Vec<_>>());
        assert_eq!(second.warnings.iter().map(|warning| warning.kind.clone()).collect::<Vec<_>>(), (1..=12).map(|page| crate::kit::contracts::WarningKind::RenamedToAvoidReplacing {
            chosen_name: format!("document-page-{page:02}.png"),
            final_name: format!("document-page-{page:02}-1.png"),
        }).collect::<Vec<_>>());
        for (path, bytes) in first.output_paths().iter().zip(&earlier) {
            assert_eq!(&fs::read(path).unwrap(), bytes, "a re-run keeps {}", path.display());
        }

        let long = root.join("long.pdf");
        split_fixture_pdf(&long, 100);
        let ranged = render_with_bundled_pdftoppm(&long, &folder, "png", 72, None, Some("8-9"));
        assert!(ranged.error().is_none(), "{:?}", ranged.error());
        assert_eq!(output_names(&ranged), ["long-page-008.png", "long-page-009.png"]);
        assert_eq!(ranged.detail, "2 images, pages 8 and 9");

        let partial = root.join("partial");
        fs::create_dir_all(&partial).unwrap();
        for name in ["long-page-003.webp", "long-page-004.webp"] { fs::write(partial.join(name), name).unwrap(); }
        let renamed = render_with_bundled_pdftoppm(&long, &partial, "webp", 72, Some(vec![2, 3, 4]), None);
        assert!(renamed.error().is_none(), "{:?}", renamed.error());
        assert_eq!(output_names(&renamed), ["long-page-003-1.webp", "long-page-004-1.webp", "long-page-005.webp"]);
        assert_eq!(renamed.detail, "3 images, pages 3-5");
        assert_eq!(renamed.warnings.iter().map(|warning| warning.kind.clone()).collect::<Vec<_>>(), ["003", "004"].map(|page| crate::kit::contracts::WarningKind::RenamedToAvoidReplacing {
            chosen_name: format!("long-page-{page}.webp"),
            final_name: format!("long-page-{page}-1.webp"),
        }));
        for name in ["long-page-003.webp", "long-page-004.webp"] { assert_eq!(fs::read(partial.join(name)).unwrap(), name.as_bytes()); }
        assert_eq!(fs::read(&input).unwrap(), original, "the original PDF is never modified");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rendered_page_lists_join_runs_like_the_results_row() {
        assert_eq!(describe_pages(&[3]), "page 3");
        assert_eq!(describe_pages(&[3, 4]), "pages 3 and 4");
        assert_eq!(describe_pages(&[3, 4, 5, 8, 9]), "pages 3-5, 8 and 9");
        assert_eq!(describe_pages(&[3, 4, 5, 8, 9, 10, 11, 12, 13, 14]), "pages 3-5 and 8-14");
        assert_eq!(describe_pages(&[1, 3, 5]), "pages 1, 3 and 5");
    }

    #[test]
    fn pdf_to_images_rejects_formats_the_registry_does_not_declare() {
        let root = split_fixture_root("undeclared-images");
        let input = root.join("document.pdf");
        split_fixture_pdf(&input, 1);
        for format in ["gif", "heic", "tiff"] {
            let outcome = render_with_bundled_pdftoppm(&input, &root, format, 72, None, None);
            assert_eq!(outcome.error().map(|error| error.message.as_str()), Some("PDF images must be JPEG, PNG or WebP."));
        }
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    struct DiscoveryFixture { document: Document, photo: ObjectId, scan: ObjectId, logo: ObjectId, form: ObjectId, in_form: ObjectId }

    fn image_discovery_fixture() -> DiscoveryFixture {
        let mut document=Document::with_version("1.7");
        let pages_id=document.new_object_id();
        let soft_mask=document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>4, "Height"=>3, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8
        },vec![0;12]));
        let photo=document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>4, "Height"=>3,
            "ColorSpace"=>"DeviceCMYK", "BitsPerComponent"=>8, "Filter"=>"DCTDecode", "SMask"=>soft_mask
        },b"photo".to_vec()));
        let scan=document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>2, "Height"=>2, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8
        },b"scan".to_vec()));
        let logo=document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>7, "Height"=>5, "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8
        },b"logo".to_vec()));
        let in_form=document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>6, "Height"=>1, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8
        },vec![9;6]));
        let form_id=document.new_object_id();
        document.objects.insert(form_id,Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Form", "BBox"=>vec![0.into(),0.into(),1.into(),1.into()],
            "Resources"=>dictionary! { "XObject"=>dictionary! { "Inner"=>in_form, "Loop"=>form_id }, "ColorSpace"=>dictionary! { "Cs9"=>"DeviceRGB" } }
        },b"/Inner Do BI /W 1 /H 1 /BPC 8 /CS /Cs9 ID \x01\x02\x03 EI".to_vec()).into());
        let mut kids=Vec::new();
        for (xobjects,content) in [
            (dictionary! { "Im1"=>photo, "Fm1"=>form_id, "Im2"=>scan },b"/Im1 Do /Fm1 Do /Im2 Do".to_vec()),
            (dictionary! {},b"q BI /W 2 /H 2 /BPC 8 /CS /G ID \x05\x06\x07\x08 EI Q".to_vec()),
            (dictionary! { "Logo"=>logo },b"/Logo Do".to_vec()),
        ] {
            let contents=document.add_object(Stream::new(dictionary! {},content));
            let page_id=document.add_object(dictionary! {
                "Type"=>"Page", "Parent"=>pages_id, "MediaBox"=>vec![0.into(),0.into(),1.into(),1.into()],
                "Resources"=>dictionary! { "XObject"=>xobjects }, "Contents"=>contents
            });
            kids.push(Object::Reference(page_id));
        }
        document.objects.insert(pages_id,dictionary! { "Type"=>"Pages", "Kids"=>kids, "Count"=>3 }.into());
        let catalog_id=document.add_object(dictionary! { "Type"=>"Catalog", "Pages"=>pages_id });
        document.trailer.set("Root",catalog_id);
        DiscoveryFixture { document, photo, scan, logo, form: form_id, in_form }
    }

    fn listed(candidates: &[ImageCandidate]) -> Vec<(u32, usize, Option<ObjectId>, i64, i64)> {
        candidates.iter().map(|candidate| {
            let id=match candidate.source { ImageSource::Object(id) => Some(id), ImageSource::Inline(_) => None };
            (candidate.page,candidate.index,id,candidate.width,candidate.height)
        }).collect()
    }

    #[test]
    fn image_discovery_lists_page_form_and_inline_images_in_page_then_resource_order() {
        let fixture=image_discovery_fixture();

        let candidates=discover_images(&fixture.document,&[1,2,3]).unwrap();

        assert_eq!(listed(&candidates),vec![
            (1,0,Some(fixture.photo),4,3),
            (1,1,Some(fixture.in_form),6,1),
            (1,2,None,1,1),
            (1,3,Some(fixture.scan),2,2),
            (2,0,None,2,2),
            (3,0,Some(fixture.logo),7,5),
        ]);
        assert!(!listed(&candidates).iter().any(|(_,_,id,_,_)| *id==Some(fixture.form)),"a form is searched, not listed");
        let ImageSource::Inline(in_form)=&candidates[2].source else { panic!("expected an inline image") };
        assert_eq!(in_form.dict.get(b"ColorSpace").unwrap().as_name().unwrap(),b"DeviceRGB","a named colour space resolves through the form's resources");
        assert_eq!(in_form.content,b"\x01\x02\x03");
        let ImageSource::Inline(on_page)=&candidates[4].source else { panic!("expected an inline image") };
        assert_eq!(on_page.content,b"\x05\x06\x07\x08");
    }

    #[test]
    fn image_discovery_only_lists_the_selected_pages() {
        let fixture=image_discovery_fixture();

        let third=discover_images(&fixture.document,&[3]).unwrap();
        assert_eq!(listed(&third),vec![(3,0,Some(fixture.logo),7,5)]);
        assert_eq!(listed(&discover_images(&fixture.document,&[2]).unwrap()),vec![(2,0,None,2,2)]);
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

    pub(super) fn image_stream(entries: lopdf::Dictionary, data: impl Into<Vec<u8>>) -> Stream {
        let mut dict=dictionary! { "Type"=>"XObject", "Subtype"=>"Image" };
        for (key,value) in entries { dict.set(key,value); }
        Stream::new(dict,data.into())
    }

    fn flate(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut encoder=flate2::write::ZlibEncoder::new(Vec::new(),flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn ascii85(data: &[u8]) -> Vec<u8> {
        let mut encoded=Vec::new();
        for chunk in data.chunks(4) {
            let mut word=[0u8;4];
            word[..chunk.len()].copy_from_slice(chunk);
            let mut value=u32::from_be_bytes(word);
            let mut digits=[0u8;5];
            for digit in digits.iter_mut().rev() { *digit=(value%85) as u8+b'!'; value/=85; }
            encoded.extend_from_slice(&digits[..chunk.len()+1]);
        }
        encoded.extend_from_slice(b"~>");
        encoded
    }

    /// LZW with 9-bit codes only: a clear code, one literal code per byte, then end of data.
    fn lzw_literals(data: &[u8]) -> Vec<u8> {
        let codes=std::iter::once(256u32).chain(data.iter().map(|byte| u32::from(*byte))).chain(std::iter::once(257));
        let (mut bits,mut count,mut packed)=(0u64,0,Vec::new());
        for code in codes {
            bits=bits<<9|u64::from(code);
            count+=9;
            while count>=8 { count-=8; packed.push((bits>>count) as u8); }
        }
        if count>0 { packed.push((bits<<(8-count)) as u8); }
        packed
    }

    /// Saves a PDF whose pages each hold the images `build` adds, in the order given.
    pub(super) fn image_pdf(path: &Path, build: impl FnOnce(&mut Document) -> Vec<Vec<Stream>>) {
        let mut document=Document::with_version("1.7");
        let pages_id=document.new_object_id();
        let pages=build(&mut document);
        let mut kids=Vec::new();
        for images in pages {
            let mut xobjects=lopdf::Dictionary::new();
            for (index,image) in images.into_iter().enumerate() { xobjects.set(format!("Im{index:02}"),document.add_object(image)); }
            kids.push(Object::Reference(document.add_object(dictionary! {
                "Type"=>"Page", "Parent"=>pages_id, "MediaBox"=>vec![0.into(),0.into(),1.into(),1.into()],
                "Resources"=>dictionary! { "XObject"=>xobjects }
            })));
        }
        let count=kids.len() as i64;
        document.objects.insert(pages_id,dictionary! { "Type"=>"Pages", "Kids"=>kids, "Count"=>count }.into());
        let catalog_id=document.add_object(dictionary! { "Type"=>"Catalog", "Pages"=>pages_id });
        document.trailer.set("Root",catalog_id);
        document.save(path).unwrap();
    }

    fn extract_images(input: &Path) -> JobOutcome {
        extract(&ExtractFromPdfRequest { paths:vec![input.to_path_buf()], mode:ExtractMode::Images, page_range:None, output_location:OutputLocation::AlongsideInput, preserve_layout:false, min_image_size:None },input.to_path_buf())
    }

    fn decoded(path: PathBuf) -> DynamicImage {
        image::load_from_memory_with_format(&fs::read(path).unwrap(),image::ImageFormat::Png).unwrap()
    }

    #[test]
    fn lossless_and_uncompressed_images_become_png_at_native_size_in_each_colour_space() {
        let temp=split_fixture_root("images-colour-spaces");
        let input=temp.join("colours.pdf");
        image_pdf(&input,|document| {
            let mut profile=|components: i64| Object::Reference(document.add_object(Stream::new(dictionary! { "N"=>components },Vec::new())));
            let grey_profile=profile(1);
            let cmyk_profile=profile(4);
            vec![vec![
                image_stream(dictionary! { "Width"=>2, "Height"=>1, "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8, "Filter"=>"FlateDecode" },flate(&[255,0,0,0,0,255])),
                image_stream(dictionary! { "Width"=>3, "Height"=>2, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>1 },vec![0b1010_0000,0b0100_0000]),
                image_stream(dictionary! {
                    "Width"=>2, "Height"=>1, "BitsPerComponent"=>4,
                    "ColorSpace"=>vec![Object::Name(b"Indexed".to_vec()),Object::Name(b"DeviceRGB".to_vec()),1.into(),Object::string_literal(vec![10,20,30,200,210,220])]
                },vec![0x10]),
                image_stream(dictionary! { "Width"=>3, "Height"=>1, "ColorSpace"=>"DeviceCMYK", "BitsPerComponent"=>8, "Filter"=>"FlateDecode" },flate(&[0,0,0,0, 0,255,255,0, 51,0,0,102])),
                image_stream(dictionary! { "Width"=>2, "Height"=>1, "ColorSpace"=>vec![Object::Name(b"ICCBased".to_vec()),grey_profile], "BitsPerComponent"=>16 },vec![0x12,0x34,0xFF,0xFF]),
                image_stream(dictionary! { "Width"=>1, "Height"=>1, "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>16 },vec![1,2,3,4,5,6]),
                image_stream(dictionary! { "Width"=>1, "Height"=>1, "ColorSpace"=>vec![Object::Name(b"ICCBased".to_vec()),cmyk_profile], "BitsPerComponent"=>8, "Decode"=>vec![1.into(),0.into(),0.into(),1.into(),0.into(),1.into(),0.into(),1.into()] },vec![255,0,255,0]),
            ]]
        });
        let original=fs::read(&input).unwrap();

        let outcome=extract_images(&input);

        let folder=temp.join("colours-images");
        assert_eq!(outcome.output_paths(),[folder.clone()]);
        assert_eq!(outcome.detail,"Saved 7 images.");
        assert!(outcome.warnings.is_empty(),"{:?}",outcome.warnings);
        assert_eq!(sorted_names(&folder),(1..=7).map(|index| format!("page-1-image-{index}.png")).collect::<Vec<_>>());
        let image=|index: usize| decoded(folder.join(format!("page-1-image-{index}.png")));
        assert_eq!(image(1).as_rgb8().unwrap().as_raw(),&vec![255,0,0,0,0,255]);
        let grey=image(2);
        assert_eq!((grey.width(),grey.height()),(3,2));
        assert_eq!(grey.as_luma8().unwrap().as_raw(),&vec![255,0,255,0,255,0]);
        assert_eq!(image(3).as_rgb8().unwrap().as_raw(),&vec![200,210,220,10,20,30]);
        assert_eq!(image(4).as_rgb8().unwrap().as_raw(),&vec![255,255,255,255,0,0,122,153,153],"CMYK uses round((255-C)(255-K)/255)");
        assert_eq!(image(5).as_luma16().unwrap().as_raw(),&vec![0x1234,0xFFFF]);
        assert_eq!(image(6).as_rgb16().unwrap().as_raw(),&vec![0x0102,0x0304,0x0506]);
        assert_eq!(image(7).as_rgb8().unwrap().as_raw(),&vec![255,255,0],"the Decode array inverts cyan before conversion");

        let again=extract_images(&input);
        let numbered=temp.join("colours-images-1");
        assert_eq!(again.output_paths(),[numbered.clone()]);
        for name in sorted_names(&folder) { assert_eq!(fs::read(folder.join(&name)).unwrap(),fs::read(numbered.join(&name)).unwrap(),"{name} is deterministic"); }
        assert_eq!(fs::read(&input).unwrap(),original);
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn soft_masks_stencil_masks_and_colour_keys_become_alpha() {
        let temp=split_fixture_root("images-alpha");
        let input=temp.join("alpha.pdf");
        image_pdf(&input,|document| {
            let soft=document.add_object(image_stream(dictionary! { "Width"=>2, "Height"=>1, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8 },vec![0,128]));
            let small_soft=document.add_object(image_stream(dictionary! { "Width"=>1, "Height"=>1, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8, "Filter"=>"FlateDecode" },flate(&[77])));
            let stencil=document.add_object(image_stream(dictionary! { "Width"=>2, "Height"=>1, "ImageMask"=>true },vec![0b0100_0000]));
            vec![vec![
                image_stream(dictionary! { "Width"=>2, "Height"=>1, "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8, "SMask"=>soft },vec![1,2,3,4,5,6]),
                image_stream(dictionary! { "Width"=>2, "Height"=>2, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8, "SMask"=>small_soft },vec![1,2,3,4]),
                image_stream(dictionary! { "Width"=>2, "Height"=>1, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8, "Mask"=>stencil },vec![10,20]),
                image_stream(dictionary! { "Width"=>2, "Height"=>1, "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8, "Mask"=>vec![0.into(),0.into(),0.into(),0.into(),0.into(),0.into()] },vec![0,0,0,9,9,9]),
                image_stream(dictionary! { "Width"=>2, "Height"=>1, "ImageMask"=>true },vec![0b0100_0000]),
            ]]
        });

        let outcome=extract_images(&input);

        let folder=temp.join("alpha-images");
        assert_eq!(outcome.detail,"Saved 5 images.");
        let image=|index: usize| decoded(folder.join(format!("page-1-image-{index}.png")));
        assert_eq!(image(1).as_rgba8().unwrap().as_raw(),&vec![1,2,3,0,4,5,6,128]);
        assert_eq!(image(2).as_luma_alpha8().unwrap().as_raw(),&vec![1,77,2,77,3,77,4,77],"a smaller soft mask is scaled to the image");
        assert_eq!(image(3).as_luma_alpha8().unwrap().as_raw(),&vec![10,255,20,0],"stencil 0 paints, 1 masks");
        assert_eq!(image(4).as_rgba8().unwrap().as_raw(),&vec![0,0,0,0,9,9,9,255]);
        assert_eq!(image(5).as_luma_alpha8().unwrap().as_raw(),&vec![0,255,0,0]);
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn every_supported_filter_chain_is_undone_before_decoding() {
        let temp=split_fixture_root("images-filters");
        let input=temp.join("filters.pdf");
        let pixel=|image: u8| vec![image,2,3];
        let hex=|data: &[u8]| { let mut text=data.iter().map(|byte| format!("{byte:02X}")).collect::<String>(); text.push('>'); text.into_bytes() };
        image_pdf(&input,|_| {
            let rgb=|extra: lopdf::Dictionary,data: Vec<u8>| {
                let mut entries=dictionary! { "Width"=>1, "Height"=>1, "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8 };
                for (key,value) in extra { entries.set(key,value); }
                image_stream(entries,data)
            };
            let name=|name: &str| Object::Name(name.as_bytes().to_vec());
            vec![vec![
                rgb(dictionary! {},pixel(1)),
                rgb(dictionary! { "Filter"=>"ASCIIHexDecode" },hex(&pixel(2))),
                rgb(dictionary! { "Filter"=>"RunLengthDecode" },[vec![2],pixel(3),vec![128]].concat()),
                rgb(dictionary! { "Filter"=>"LZWDecode" },lzw_literals(&pixel(4))),
                rgb(dictionary! { "Filter"=>"ASCII85Decode" },ascii85(&pixel(5))),
                rgb(dictionary! {
                    "Filter"=>vec![name("AHx"),name("Fl")],
                    "DecodeParms"=>vec![Object::Null,dictionary! { "Predictor"=>15, "Colors"=>3, "Columns"=>1 }.into()]
                },hex(&flate(&[vec![0],pixel(6)].concat()))),
            ]]
        });

        let outcome=extract_images(&input);

        let folder=temp.join("filters-images");
        assert_eq!(outcome.detail,"Saved 6 images.","{:?}",outcome.warnings);
        for index in 1..=6u8 {
            assert_eq!(decoded(folder.join(format!("page-1-image-{index}.png"))).as_rgb8().unwrap().as_raw(),&pixel(index),"image {index}");
        }
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn jpeg_and_jpeg_2000_are_copied_byte_for_byte_with_one_colour_warning_per_kind() {
        let temp=split_fixture_root("images-copied");
        let input=temp.join("copied.pdf");
        let jpegs=[[10,20,30],[60,90,120],[200,40,80],[30,160,60]].map(jpeg_bytes);
        let jp2=b"\x00\x00\x00\x0cjP  \r\n\x87\nnot decoded".to_vec();
        let decode=|count: usize| (0..count).flat_map(|_| [Object::from(1),Object::from(0)]).collect::<Vec<_>>();
        image_pdf(&input,|_| {
            let dct=|jpeg: &Vec<u8>,space: &str,extra: Option<Vec<Object>>| {
                let mut entries=dictionary! { "Width"=>2, "Height"=>2, "ColorSpace"=>space, "BitsPerComponent"=>8, "Filter"=>"DCTDecode" };
                if let Some(decode)=extra { entries.set("Decode",decode); }
                image_stream(entries,jpeg.clone())
            };
            vec![
                vec![dct(&jpegs[0],"DeviceRGB",None),image_stream(dictionary! { "Width"=>2, "Height"=>2, "Filter"=>"JPXDecode" },jp2.clone())],
                vec![dct(&jpegs[1],"DeviceCMYK",None)],
                vec![dct(&jpegs[2],"DeviceCMYK",Some(decode(4))),dct(&jpegs[3],"DeviceGray",Some(decode(1)))],
            ]
        });

        let outcome=extract_images(&input);

        let folder=temp.join("copied-images");
        assert_eq!(outcome.detail,"Saved 5 images.");
        assert_eq!(sorted_names(&folder),vec!["page-1-image-1.jpg","page-1-image-2.jp2","page-2-image-1.jpg","page-3-image-1.jpg","page-3-image-2.jpg"]);
        assert_eq!(fs::read(folder.join("page-1-image-2.jp2")).unwrap(),jp2);
        for (name,jpeg) in ["page-1-image-1.jpg","page-2-image-1.jpg","page-3-image-1.jpg","page-3-image-2.jpg"].iter().zip(&jpegs) { assert_eq!(&fs::read(folder.join(name)).unwrap(),jpeg,"{name}"); }
        assert_eq!(warning_kinds(&outcome),vec![
            WarningKind::JpegColourKept { colour:JpegColour::Cmyk, pages:vec![2,3], images:2 },
            WarningKind::JpegColourKept { colour:JpegColour::Inverted, pages:vec![3], images:2 },
        ]);
        assert_eq!(outcome.warnings[0].message,"Pages 2-3 — 2 JPEGs use CMYK colour and were copied as they are. Some viewers show them with the wrong colours.");
        assert_eq!(outcome.warnings[1].message,"Page 3 — 2 inverted JPEGs, copied as they are. Some viewers show them in negative.");
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn undecodable_and_unsupported_images_are_reported_with_their_page_and_the_rest_are_saved() {
        let temp=split_fixture_root("images-partial");
        let input=temp.join("partial.pdf");
        let jpeg=jpeg_bytes([255,0,0]);
        image_pdf(&input,|document| {
            let broken_mask=document.add_object(image_stream(dictionary! { "Width"=>1, "Height"=>1, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8, "Filter"=>"FlateDecode" },b"broken".to_vec()));
            let rgb=|extra: lopdf::Dictionary,data: Vec<u8>| {
                let mut entries=dictionary! { "Width"=>1, "Height"=>1, "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8 };
                for (key,value) in extra { entries.set(key,value); }
                image_stream(entries,data)
            };
            vec![
                vec![rgb(dictionary! { "Width"=>2, "Height"=>2, "Filter"=>"DCTDecode" },jpeg.clone()),rgb(dictionary! { "Filter"=>"FlateDecode" },b"broken".to_vec())],
                vec![
                    rgb(dictionary! { "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>1, "Filter"=>"JBIG2Decode" },jbig2_colour_extension()),
                    rgb(dictionary! { "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>1, "Filter"=>"CCITTFaxDecode" },vec![0;8]),
                    rgb(dictionary! { "ColorSpace"=>vec![Object::Name(b"Lab".to_vec()),dictionary! {}.into()] },vec![0;3]),
                    rgb(dictionary! { "Width"=>61000, "Height"=>48000 },vec![0;3]),
                    rgb(dictionary! { "SMask"=>broken_mask },vec![0;3]),
                ],
            ]
        });
        let original=fs::read(&input).unwrap();

        let outcome=extract_images(&input);

        let folder=temp.join("partial-images");
        assert!(matches!(outcome.state,crate::kit::contracts::OutcomeState::Succeeded { .. }));
        assert_eq!(outcome.output_paths(),[folder.clone()]);
        assert_eq!(sorted_names(&folder),vec!["page-1-image-1.jpg"]);
        assert_eq!(outcome.detail,"Saved 1 image · 6 couldn't be saved.");
        assert_eq!(fs::read(folder.join("page-1-image-1.jpg")).unwrap(),jpeg);
        assert_eq!(warning_kinds(&outcome),vec![
            WarningKind::ImageDamaged { page:1, image:2 },
            WarningKind::ImageUnsupported { page:2, image:1, feature:"JBIG2".into() },
            WarningKind::ImageDamaged { page:2, image:2 },
            WarningKind::ImageUnsupported { page:2, image:3, feature:"Lab colour".into() },
            WarningKind::ImageTooLarge { page:2, image:4, width:61000, height:48000 },
            WarningKind::ImageDamaged { page:2, image:5 },
        ]);
        assert_eq!(outcome.warnings.iter().map(|warning| warning.message.as_str()).collect::<Vec<_>>(),vec![
            "Page 1, image 2 — The image data is damaged. Not saved.",
            "Page 2, image 1 — JBIG2 isn't supported in this version. Not saved.",
            "Page 2, image 2 — The image data is damaged. Not saved.",
            "Page 2, image 3 — Lab colour isn't supported in this version. Not saved.",
            "Page 2, image 4 — Would be 61,000 × 48,000 px when decoded, over the size limit. Not saved.",
            "Page 2, image 5 — The image data is damaged. Not saved.",
        ]);
        assert_eq!(fs::read(&input).unwrap(),original);
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn a_pdf_fails_only_when_none_of_its_images_could_be_saved() {
        let temp=split_fixture_root("images-none-decoded");
        let jbig2=|data: Vec<u8>| image_stream(dictionary! { "Width"=>1, "Height"=>1, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>1, "Filter"=>"JBIG2Decode" },data);
        let fax=temp.join("fax.pdf");
        image_pdf(&fax,|_| vec![vec![jbig2(jbig2_colour_extension()),jbig2(vec![0;8])],vec![jbig2(jbig2_colour_extension())]]);
        let single=temp.join("single.pdf");
        image_pdf(&single,|_| vec![vec![image_stream(dictionary! { "Width"=>1, "Height"=>1, "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8, "Filter"=>"FlateDecode" },b"broken".to_vec())]]);

        let outcome=extract_images(&fax);
        let error=outcome.error().unwrap();
        assert_eq!((&error.kind,error.message.as_str()),(&crate::kit::contracts::ErrorKind::Processing,"None of the 3 images could be decoded, so nothing was saved."));
        assert!(outcome.output_paths().is_empty());
        assert_eq!(warning_kinds(&outcome),vec![
            WarningKind::ImageUnsupported { page:1, image:1, feature:"JBIG2".into() },
            WarningKind::ImageDamaged { page:1, image:2 },
            WarningKind::ImageUnsupported { page:2, image:1, feature:"JBIG2".into() },
        ]);

        let outcome=extract_images(&single);
        assert_eq!(outcome.error().map(|error| error.message.as_str()),Some("The image couldn't be decoded, so nothing was saved."));
        assert_eq!(warning_kinds(&outcome),vec![WarningKind::ImageDamaged { page:1, image:1 }]);
        assert_eq!(sorted_names(&temp),vec!["fax.pdf","single.pdf"],"no images folder is made");
        fs::remove_dir_all(temp).unwrap();
    }

    /// The fax fixture: 10 × 4, `#` is black. Rows 1 and 2 match, so two-dimensional coding can refer to the row above.
    const FAX_ROWS: [&str; 4] = ["##....####", "##....####", "..#####...", "#........#"];

    fn fax_luma() -> Vec<u8> {
        FAX_ROWS.iter().flat_map(|row| row.bytes().map(|pixel| if pixel == b'#' { 0 } else { 255 })).collect()
    }

    /// Writes CCITT codes given as strings of 0 and 1.
    #[derive(Default)]
    struct FaxBits { bytes: Vec<u8>, used: usize }

    impl FaxBits {
        fn put(&mut self, code: &str) {
            for bit in code.bytes() {
                if self.used % 8 == 0 { self.bytes.push(0); }
                if bit == b'1' { *self.bytes.last_mut().unwrap() |= 0x80 >> (self.used % 8); }
                self.used += 1;
            }
        }

        /// An EOL whose last bit ends a byte, after zero fill bits.
        fn aligned_eol(&mut self) {
            while (self.used + 12) % 8 != 0 { self.put("0"); }
            self.put("000000000001");
        }

        /// Terminating run codes, enough for the 10-pixel fixture.
        fn run(&mut self, black: bool, length: usize) {
            const WHITE: [&str; 11] = ["00110101","000111","0111","1000","1011","1100","1110","1111","10011","10100","00111"];
            const BLACK: [&str; 11] = ["0000110111","010","11","10","011","0011","0010","00011","000101","000100","0000100"];
            self.put(if black { BLACK[length] } else { WHITE[length] });
        }

        /// One row coded one-dimensionally: alternating runs that start white.
        fn row_1d(&mut self, row: &str) {
            let (mut black, mut length) = (false, 0);
            for pixel in row.bytes().map(|pixel| pixel == b'#') {
                if pixel != black { self.run(black, length); black = pixel; length = 0; }
                length += 1;
            }
            self.run(black, length);
        }

        /// One row coded two-dimensionally: V0 for every change when it repeats the row above, horizontal mode otherwise.
        fn row_2d(&mut self, row: &str, above: &str) {
            let changes = fax_changes(row);
            if row == above { for _ in 0..=changes.len() { self.put("1"); } return; }
            let (mut a0, mut index) = (0, 0);
            while a0 < row.len() {
                let a1 = changes.get(index).copied().unwrap_or(row.len());
                let a2 = changes.get(index + 1).copied().unwrap_or(row.len());
                self.put("001");
                self.run(false, a1 - a0);
                self.run(true, a2 - a1);
                (a0, index) = (a2, index + 2);
            }
        }
    }

    fn fax_changes(row: &str) -> Vec<usize> {
        let mut previous = false;
        row.bytes().enumerate().filter_map(|(x, pixel)| { let black = pixel == b'#'; let change = black != previous; previous = black; change.then_some(x) }).collect()
    }

    fn fax_group_4() -> Vec<u8> {
        let mut encoder = fax::encoder::Encoder::new(fax::VecWriter::new());
        for row in FAX_ROWS {
            encoder.encode_line(row.bytes().map(|pixel| if pixel == b'#' { fax::Color::Black } else { fax::Color::White }), 10).unwrap();
        }
        encoder.finish().unwrap().finish()
    }

    fn fax_image(parms: lopdf::Dictionary, extra: lopdf::Dictionary, data: Vec<u8>) -> Stream {
        let mut entries = dictionary! { "Width"=>10, "Height"=>4, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>1, "Filter"=>"CCITTFaxDecode" };
        let mut decode_parms = dictionary! { "Columns"=>10, "Rows"=>4 };
        for (key, value) in parms { decode_parms.set(key, value); }
        entries.set("DecodeParms", decode_parms);
        for (key, value) in extra { entries.set(key, value); }
        image_stream(entries, data)
    }

    #[test]
    fn group_3_and_group_4_fax_images_become_png_with_their_encoding_parameters_honoured() {
        let temp=split_fixture_root("images-ccitt");
        let mut plain=FaxBits::default();
        FAX_ROWS.iter().for_each(|row| plain.row_1d(row));
        let mut lined=FaxBits::default();
        for row in FAX_ROWS { lined.aligned_eol(); lined.row_1d(row); }
        for _ in 0..6 { lined.aligned_eol(); }
        let mut mixed=FaxBits::default();
        for (index,row) in FAX_ROWS.iter().enumerate() {
            mixed.put("000000000001");
            if index % 2 == 0 { mixed.put("1"); mixed.row_1d(row); } else { mixed.put("0"); mixed.row_2d(row,FAX_ROWS[index-1]); }
        }
        for _ in 0..6 { mixed.put("0000000000011"); }
        let inverted=|| vec![Object::from(1),Object::from(0)];
        let variants=[
            ("group-4",fax_image(dictionary! { "K"=>-1 },dictionary! {},fax_group_4())),
            ("group-3-black-is-1",fax_image(dictionary! { "K"=>0, "BlackIs1"=>true },dictionary! { "Decode"=>inverted() },plain.bytes)),
            ("group-3-aligned-eol",fax_image(dictionary! { "K"=>0, "EndOfLine"=>true, "EncodedByteAlign"=>true },dictionary! {},lined.bytes)),
            ("group-3-mixed-2d",fax_image(dictionary! { "K"=>2, "EndOfLine"=>true },dictionary! {},mixed.bytes)),
        ];

        // Identical pixels would be merged as duplicates, so each variant gets its own PDF.
        for (name,image) in variants {
            let input=temp.join(format!("{name}.pdf"));
            image_pdf(&input,|_| vec![vec![image]]);
            let original=fs::read(&input).unwrap();

            let outcome=extract_images(&input);

            let folder=temp.join(format!("{name}-images"));
            assert!(outcome.warnings.is_empty(),"{name}: {:?}",outcome.warnings);
            assert_eq!(outcome.detail,"Saved 1 image.","{name}");
            assert_eq!(sorted_names(&folder),vec!["page-1-image-1.png"],"{name}");
            let image=decoded(folder.join("page-1-image-1.png"));
            assert_eq!((image.width(),image.height()),(10,4),"{name} keeps its native size");
            assert_eq!(image.as_luma8().unwrap().as_raw(),&fax_luma(),"{name}");
            assert_eq!(fs::read(&input).unwrap(),original,"{name} is left as it was");
        }

        let input=temp.join("stencil.pdf");
        image_pdf(&input,|_| vec![vec![
            fax_image(dictionary! { "K"=>-1 },dictionary! { "ImageMask"=>true },fax_group_4()),
            fax_image(dictionary! { "K"=>-1 },dictionary! { "Width"=>8 },fax_group_4()),
        ]]);
        let outcome=extract_images(&input);
        assert_eq!(outcome.detail,"Saved 2 images.");
        let narrow=decoded(temp.join("stencil-images").join("page-1-image-2.png"));
        assert_eq!((narrow.width(),narrow.height()),(8,4));
        assert_eq!(narrow.as_luma8().unwrap().as_raw(),&fax_luma().chunks(10).flat_map(|row| row[..8].to_vec()).collect::<Vec<_>>(),"rows wider than the image are cut to its width");
        let mask=decoded(temp.join("stencil-images").join("page-1-image-1.png"));
        assert_eq!(mask.as_luma_alpha8().unwrap().as_raw(),&fax_luma().iter().flat_map(|luma| [0,255-luma]).collect::<Vec<_>>(),"a fax stencil mask paints its black pixels");
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn a_malformed_fax_image_is_reported_on_its_own_while_the_other_images_are_saved() {
        let temp=split_fixture_root("images-ccitt-malformed");
        let input=temp.join("fax-bundle.pdf");
        let group_4=fax_group_4();
        image_pdf(&input,|_| vec![
            vec![fax_image(dictionary! { "K"=>-1 },dictionary! {},vec![0;8]),fax_image(dictionary! { "K"=>-1 },dictionary! {},group_4.clone())],
            vec![fax_image(dictionary! { "K"=>-1 },dictionary! {},group_4[..1].to_vec())],
        ]);

        let outcome=extract_images(&input);

        let folder=temp.join("fax-bundle-images");
        assert!(matches!(outcome.state,crate::kit::contracts::OutcomeState::Succeeded { .. }));
        assert_eq!(outcome.output_paths(),[folder.clone()]);
        assert_eq!(sorted_names(&folder),vec!["page-1-image-2.png"]);
        assert_eq!(decoded(folder.join("page-1-image-2.png")).as_luma8().unwrap().as_raw(),&fax_luma());
        assert_eq!(outcome.detail,"Saved 1 image · 2 couldn't be saved.");
        assert_eq!(warning_kinds(&outcome),vec![WarningKind::ImageDamaged { page:1, image:1 },WarningKind::ImageDamaged { page:2, image:1 }]);
        assert_eq!(outcome.warnings.iter().map(|warning| warning.message.as_str()).collect::<Vec<_>>(),vec![
            "Page 1, image 1 — The image data is damaged. Not saved.",
            "Page 2, image 1 — The image data is damaged. Not saved.",
        ]);
        fs::remove_dir_all(temp).unwrap();
    }

    fn jbig2_luma(fixture: &Jbig2Fixture) -> Vec<u8> {
        fixture.black.iter().map(|black| if *black { 0 } else { 255 }).collect()
    }

    fn jbig2_image(fixture: &Jbig2Fixture, globals: Option<lopdf::ObjectId>, extra: lopdf::Dictionary) -> Stream {
        let mut entries=dictionary! { "Width"=>fixture.width as i64, "Height"=>fixture.height as i64, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>1, "Filter"=>"JBIG2Decode" };
        if let Some(globals)=globals { entries.set("DecodeParms",dictionary! { "JBIG2Globals"=>globals }); }
        for (key,value) in extra { entries.set(key,value); }
        image_stream(entries,fixture.stream.clone())
    }

    /// The generic region fixture with its region's colour extension flag set, a JBIG2 feature the decoder does not handle.
    fn jbig2_colour_extension() -> Vec<u8> {
        let mut stream=jbig2_fixtures::generic_region().stream;
        // A page information segment (11-byte header, 19 bytes of data), then the region's 11-byte header and 17-byte region info.
        assert_eq!(stream[30+4],38,"the second segment is the immediate generic region");
        stream[30+11+16]|=0x08;
        stream
    }

    #[test]
    fn jbig2_images_become_png_with_their_exact_pixels_including_ones_that_use_global_segments() {
        let temp=split_fixture_root("images-jbig2");
        let fixtures=jbig2_fixtures::all();

        // Identical pixels would be merged as duplicates, so each fixture gets its own PDF, written as the spike wrote it.
        for (index,fixture) in fixtures.iter().enumerate() {
            let input=temp.join(format!("jbig2-{index}.pdf"));
            jbig2_fixtures::pdf_with_images(std::slice::from_ref(fixture)).save(&input).unwrap();
            let original=fs::read(&input).unwrap();

            let outcome=extract_images(&input);

            let folder=temp.join(format!("jbig2-{index}-images"));
            assert!(outcome.warnings.is_empty(),"{}: {:?}",fixture.name,outcome.warnings);
            assert_eq!(outcome.detail,"Saved 1 image.","{}",fixture.name);
            assert_eq!(sorted_names(&folder),vec!["page-1-image-1.png"],"{}",fixture.name);
            let image=decoded(folder.join("page-1-image-1.png"));
            assert_eq!((image.width(),image.height()),(fixture.width,fixture.height),"{} keeps its native size",fixture.name);
            assert_eq!(image.as_luma8().unwrap().as_raw(),&jbig2_luma(fixture),"{}",fixture.name);
            assert_eq!(fs::read(&input).unwrap(),original,"{} is left as it was",fixture.name);
        }

        // A compressed globals stream, a Decode array, a stencil mask and a page wider than the image.
        let input=temp.join("variants.pdf");
        let [generic,_,global]=&fixtures[..] else { unreachable!() };
        image_pdf(&input,|document| {
            let globals=document.add_object(Stream::new(dictionary! { "Filter"=>"FlateDecode" },flate(global.globals.as_ref().unwrap())));
            vec![
                vec![jbig2_image(global,Some(globals),dictionary! {})],
                vec![jbig2_image(generic,None,dictionary! { "Decode"=>vec![Object::from(1),Object::from(0)] })],
                vec![jbig2_image(generic,None,dictionary! { "ImageMask"=>true })],
                vec![jbig2_image(generic,None,dictionary! { "Width"=>20 })],
            ]
        });

        let outcome=extract_images(&input);

        let folder=temp.join("variants-images");
        assert!(outcome.warnings.is_empty(),"{:?}",outcome.warnings);
        assert_eq!(outcome.detail,"Saved 4 images.");
        assert_eq!(decoded(folder.join("page-1-image-1.png")).as_luma8().unwrap().as_raw(),&jbig2_luma(global),"globals behind a filter are decoded");
        assert_eq!(decoded(folder.join("page-2-image-1.png")).as_luma8().unwrap().as_raw(),&jbig2_luma(generic).iter().map(|luma| 255-luma).collect::<Vec<_>>(),"Decode [1 0] inverts");
        assert_eq!(decoded(folder.join("page-3-image-1.png")).as_luma_alpha8().unwrap().as_raw(),&jbig2_luma(generic).iter().flat_map(|luma| [0,255-luma]).collect::<Vec<_>>(),"a JBIG2 stencil mask paints its black pixels");
        let narrow=decoded(folder.join("page-4-image-1.png"));
        assert_eq!((narrow.width(),narrow.height()),(20,generic.height));
        assert_eq!(narrow.as_luma8().unwrap().as_raw(),&jbig2_luma(generic).chunks(generic.width as usize).flat_map(|row| row[..20].to_vec()).collect::<Vec<_>>(),"rows wider than the image are cut to its width");
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn a_jbig2_image_that_cannot_be_decoded_is_reported_on_its_own_while_the_other_images_are_saved() {
        let temp=split_fixture_root("images-jbig2-malformed");
        let input=temp.join("jbig2-bundle.pdf");
        let generic=jbig2_fixtures::generic_region();
        let global=jbig2_fixtures::text_region_with_embedded_globals();
        image_pdf(&input,|document| {
            let not_globals=document.add_object(Stream::new(dictionary! { "Filter"=>"JBIG2Decode", "DecodeParms"=>dictionary! {} },global.globals.clone().unwrap()));
            let mut truncated=jbig2_image(&generic,None,dictionary! {});
            truncated.set_content(generic.stream[..40].to_vec());
            let mut colour=jbig2_image(&generic,None,dictionary! {});
            colour.set_content(jbig2_colour_extension());
            vec![
                vec![jbig2_image(&global,None,dictionary! {}),jbig2_image(&generic,None,dictionary! {})],
                vec![truncated,colour],
                vec![jbig2_image(&generic,None,dictionary! { "Height"=>10 }),jbig2_image(&global,Some(not_globals),dictionary! {})],
            ]
        });
        let original=fs::read(&input).unwrap();

        let outcome=extract_images(&input);

        let folder=temp.join("jbig2-bundle-images");
        assert!(matches!(outcome.state,crate::kit::contracts::OutcomeState::Succeeded { .. }));
        assert_eq!(sorted_names(&folder),vec!["page-1-image-2.png"]);
        assert_eq!(decoded(folder.join("page-1-image-2.png")).as_luma8().unwrap().as_raw(),&jbig2_luma(&generic));
        assert_eq!(outcome.detail,"Saved 1 image · 5 couldn't be saved.");
        assert_eq!(warning_kinds(&outcome),vec![
            WarningKind::ImageDamaged { page:1, image:1 },
            WarningKind::ImageDamaged { page:2, image:1 },
            WarningKind::ImageUnsupported { page:2, image:2, feature:"JBIG2".into() },
            WarningKind::ImageDamaged { page:3, image:1 },
            WarningKind::ImageDamaged { page:3, image:2 },
        ]);
        assert_eq!(outcome.warnings[2].message,"Page 2, image 2 — JBIG2 isn't supported in this version. Not saved.");
        assert_eq!(fs::read(&input).unwrap(),original);
        fs::remove_dir_all(temp).unwrap();
    }

    fn jpeg_bytes(rgb: [u8; 3]) -> Vec<u8> {
        let mut jpeg=Cursor::new(Vec::new());
        JpegEncoder::new(&mut jpeg).encode_image(&DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2,2,image::Rgb(rgb)))).unwrap();
        jpeg.into_inner()
    }

    /// Page 1 holds two JPEGs, page 2 none, page 3 one.
    fn images_folder_fixture(path: &Path) -> Vec<Vec<u8>> {
        let streams=vec![jpeg_bytes([255,0,0]),jpeg_bytes([0,255,0]),jpeg_bytes([0,0,255])];
        let mut document=Document::with_version("1.7");
        let pages_id=document.new_object_id();
        let ids=streams.iter().map(|data| document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Image", "Width"=>2, "Height"=>2,
            "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8, "Filter"=>"DCTDecode"
        },data.clone()))).collect::<Vec<_>>();
        let mut kids=Vec::new();
        for xobjects in [dictionary! { "A"=>ids[0], "B"=>ids[1] }, dictionary! {}, dictionary! { "C"=>ids[2] }] {
            let page_id=document.add_object(dictionary! {
                "Type"=>"Page", "Parent"=>pages_id, "MediaBox"=>vec![0.into(),0.into(),1.into(),1.into()],
                "Resources"=>dictionary! { "XObject"=>xobjects }
            });
            kids.push(Object::Reference(page_id));
        }
        document.objects.insert(pages_id,dictionary! { "Type"=>"Pages", "Kids"=>kids, "Count"=>3 }.into());
        let catalog_id=document.add_object(dictionary! { "Type"=>"Catalog", "Pages"=>pages_id });
        document.trailer.set("Root",catalog_id);
        document.save(path).unwrap();
        streams
    }

    fn sorted_names(folder: &Path) -> Vec<String> {
        let mut names=fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn images_land_in_one_numbered_images_folder_with_one_based_names_and_original_bytes() {
        let temp=split_fixture_root("images-folder");
        let input=temp.join("report.pdf");
        let streams=images_folder_fixture(&input);
        let original=fs::read(&input).unwrap();
        let request=ExtractFromPdfRequest { paths:vec![input.clone()], mode:ExtractMode::Images, page_range:None, output_location:OutputLocation::AlongsideInput, preserve_layout:false, min_image_size:None };

        let first=extract(&request,input.clone());
        let folder=temp.join("report-images");
        assert_eq!(first.output_paths(),[folder.clone()]);
        assert_eq!(first.detail,"Saved 3 images.");
        assert_eq!(sorted_names(&folder),vec!["page-1-image-1.jpg","page-1-image-2.jpg","page-3-image-1.jpg"]);
        assert_eq!(fs::read(folder.join("page-1-image-1.jpg")).unwrap(),streams[0]);
        assert_eq!(fs::read(folder.join("page-1-image-2.jpg")).unwrap(),streams[1]);
        assert_eq!(fs::read(folder.join("page-3-image-1.jpg")).unwrap(),streams[2]);
        let first_run=folder.join("page-1-image-1.jpg");
        let first_modified=fs::metadata(&first_run).unwrap().modified().unwrap();

        let second=extract(&request,input.clone());
        let numbered=temp.join("report-images-1");
        assert_eq!(second.output_paths(),[numbered.clone()]);
        assert_eq!(sorted_names(&numbered),vec!["page-1-image-1.jpg","page-1-image-2.jpg","page-3-image-1.jpg"]);
        assert_eq!(sorted_names(&folder),vec!["page-1-image-1.jpg","page-1-image-2.jpg","page-3-image-1.jpg"],"the first folder is never written into");
        assert_eq!(fs::metadata(&first_run).unwrap().modified().unwrap(),first_modified);
        assert_eq!(sorted_names(&temp),vec!["report-images","report-images-1","report.pdf"]);
        assert_eq!(fs::read(&input).unwrap(),original);
        fs::remove_dir_all(temp).unwrap();
    }

    fn reuse_fixture(path: &Path) {
        let mut document=Document::with_version("1.7");
        let pages_id=document.new_object_id();
        let mut image=|width: i64, fill: u8| document.add_object(image_stream(dictionary! { "Width"=>width, "Height"=>width, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8 },vec![fill;(width*width) as usize]));
        let (shared,twin_a,twin_b,tiny,in_form)=(image(8,10),image(8,20),image(8,20),image(2,30),image(8,40));
        let broken=document.add_object(image_stream(dictionary! { "Width"=>8, "Height"=>8, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8, "Filter"=>"FlateDecode" },b"broken".to_vec()));
        let form=document.add_object(Stream::new(dictionary! {
            "Type"=>"XObject", "Subtype"=>"Form", "BBox"=>vec![0.into(),0.into(),1.into(),1.into()],
            "Resources"=>dictionary! { "XObject"=>dictionary! { "Inner"=>in_form } }
        },b"/Inner Do".to_vec()));
        let inline=[b"q BI /W 8 /H 8 /BPC 8 /CS /G ID ".to_vec(),vec![50;64],b" EI Q".to_vec()].concat();
        let mut kids=Vec::new();
        for (xobjects,content) in [
            (dictionary! { "S"=>shared, "T"=>twin_a, "Tiny"=>tiny },b"BI /W 1 /H 1 /BPC 8 /CS /G ID \x09 EI".to_vec()),
            (dictionary! { "S"=>shared, "T"=>twin_b, "F"=>form },Vec::new()),
            (dictionary! { "S"=>shared, "Tiny"=>tiny, "X"=>broken },inline),
        ] {
            let contents=document.add_object(Stream::new(dictionary! {},content));
            kids.push(Object::Reference(document.add_object(dictionary! {
                "Type"=>"Page", "Parent"=>pages_id, "MediaBox"=>vec![0.into(),0.into(),1.into(),1.into()],
                "Resources"=>dictionary! { "XObject"=>xobjects }, "Contents"=>contents
            })));
        }
        document.objects.insert(pages_id,dictionary! { "Type"=>"Pages", "Kids"=>kids, "Count"=>3 }.into());
        let catalog_id=document.add_object(dictionary! { "Type"=>"Catalog", "Pages"=>pages_id });
        document.trailer.set("Root",catalog_id);
        document.save(path).unwrap();
    }

    #[test]
    fn reused_and_identical_images_are_saved_once_under_their_first_page_and_small_ones_are_counted_once() {
        let temp=split_fixture_root("images-reuse");
        let input=temp.join("reuse.pdf");
        reuse_fixture(&input);
        let original=fs::read(&input).unwrap();
        let request=|min_image_size| ExtractFromPdfRequest { paths:vec![input.clone()], mode:ExtractMode::Images, page_range:None, output_location:OutputLocation::AlongsideInput, preserve_layout:false, min_image_size };

        let filtered=extract(&request(Some(4)),input.clone());

        let folder=temp.join("reuse-images");
        assert_eq!(filtered.detail,"Saved 4 images · 3 duplicates · 2 small left out · 1 couldn't be saved.");
        assert_eq!(sorted_names(&folder),vec!["page-1-image-1.png","page-1-image-2.png","page-2-image-3.png","page-3-image-4.png"]);
        let gray=|name: &str| decoded(folder.join(name)).as_luma8().unwrap().as_raw().clone();
        assert_eq!(gray("page-1-image-1.png"),vec![10;64]);
        assert_eq!(gray("page-1-image-2.png"),vec![20;64]);
        assert_eq!(gray("page-2-image-3.png"),vec![40;64],"an image inside a form object");
        assert_eq!(gray("page-3-image-4.png"),vec![50;64],"an inline image");
        assert_eq!(warning_kinds(&filtered),vec![WarningKind::ImageDamaged { page:3, image:3 }]);

        let everything=extract(&request(None),input.clone());

        let folder=temp.join("reuse-images-1");
        assert_eq!(everything.detail,"Saved 6 images · 4 duplicates · 1 couldn't be saved.");
        assert_eq!(sorted_names(&folder),vec!["page-1-image-1.png","page-1-image-2.png","page-1-image-3.png","page-1-image-4.png","page-2-image-3.png","page-3-image-4.png"]);
        assert_eq!(gray_in(&folder,"page-1-image-3.png"),vec![30;4]);
        assert_eq!(gray_in(&folder,"page-1-image-4.png"),vec![9]);
        assert_eq!(fs::read(&input).unwrap(),original);
        fs::remove_dir_all(temp).unwrap();
    }

    fn gray_in(folder: &Path, name: &str) -> Vec<u8> {
        decoded(folder.join(name)).as_luma8().unwrap().as_raw().clone()
    }

    #[test]
    fn no_images_folder_is_made_when_no_image_is_written() {
        let temp=split_fixture_root("images-folder-none");
        let input=temp.join("report.pdf");
        images_folder_fixture(&input);
        let request=|page_range: &str, min_image_size| ExtractFromPdfRequest { paths:vec![input.clone()], mode:ExtractMode::Images, page_range:Some(page_range.to_string()), output_location:OutputLocation::AlongsideInput, preserve_layout:false, min_image_size };

        let imageless_page=extract(&request("2",None),input.clone());
        let all_too_small=extract(&request("1-3",Some(3)),input.clone());

        for (outcome,message) in [(imageless_page,NO_EMBEDDED_IMAGES),(all_too_small,"All 3 images are smaller than the minimum size. No folder was made.")] {
            assert!(matches!(&outcome.state,crate::kit::contracts::OutcomeState::Skipped { reason } if reason.message==message),"{outcome:?}");
        }
        assert_eq!(sorted_names(&temp),vec!["report.pdf"]);
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
    fn extracted_image_publication_reports_the_folder_of_images_published_before_a_later_conflict() {
        let temp=split_fixture_root("images-folder-partial");
        let input=temp.join("source.pdf");
        fs::write(&input,b"source").unwrap();
        let reserve=|folder: &FolderReservation, name: &str, data: &[u8]| {
            let reservation=OutputNaming::reserve_named_candidate(&folder.path().join(name)).unwrap().unwrap();
            fs::write(reservation.path(),data).unwrap();
            reservation
        };

        let folder=OutputNaming::reserve_folder(&input,&OutputLocation::AlongsideInput,"-images").unwrap();
        let reservations=vec![
            reserve(&folder,"page-1-image-1.jpg",b"first extracted image"),
            reserve(&folder,"page-1-image-2.jpg",b"second extracted image"),
            reserve(&folder,"page-2-image-1.jpg",b"third extracted image"),
        ];
        let images=temp.join("source-images");
        fs::write(images.join("page-1-image-2.jpg"),b"competing replacement").unwrap();

        let (published,message)=publish_extracted_images(folder,reservations).unwrap_err();

        assert_eq!(published,Some(images.clone()));
        assert!(message.starts_with("Could not publish extracted image: "),"{message}");
        assert!(message.ends_with("The 1 image before it was saved in source-images."),"{message}");
        assert_eq!(sorted_names(&images),vec!["page-1-image-1.jpg","page-1-image-2.jpg"]);
        assert_eq!(fs::read(images.join("page-1-image-1.jpg")).unwrap(),b"first extracted image");
        assert_eq!(fs::read(images.join("page-1-image-2.jpg")).unwrap(),b"competing replacement");

        let folder=OutputNaming::reserve_folder(&input,&OutputLocation::AlongsideInput,"-images").unwrap();
        let blocked=reserve(&folder,"page-1-image-1.jpg",b"only image");
        let numbered=temp.join("source-images-1");
        fs::create_dir(numbered.join("page-1-image-1.jpg")).unwrap();

        let (published,_)=publish_extracted_images(folder,vec![blocked]).unwrap_err();

        assert_eq!(published,None,"a folder with nothing published in it is not reported");
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
        let first = generated.join("page-1.png");
        let invalid = generated.join("page-2.png");
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([255, 0, 0]))).save(&first).unwrap();
        fs::write(&invalid, b"unexpected image").unwrap();

        let outcome = materialize_rendered_images(&input, &OutputLocation::CustomFolder(output_folder.clone()), &[(1, first), (2, invalid)], OutputFormat::WebP, 2);

        assert!(outcome.error().is_some());
        assert_eq!(outcome.output_paths().len(), 1);
        assert_eq!(outcome.output_paths()[0], output_folder.join("source-page-01.webp"));
        assert_eq!(image::open(&outcome.output_paths()[0]).unwrap().width(), 4);
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
                paths: vec![input.clone()], dpi: 72, format: "png".to_string(), page_range: None, pages, picks: Vec::new(),
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

#[cfg(test)]
mod text_extraction_tests {
    use super::*;
    use crate::kit::contracts::{ErrorKind, OutcomeState};
    use lopdf::StringFormat;

    enum Run<'a> { Latin(&'a str), Cid(&'a str) }
    struct Line<'a> { x: f32, y: f32, run: Run<'a> }

    fn sandbox(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("toolbox_pdftotext_{name}_{}_{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// Writes a PDF whose pages hold the given lines. Latin runs use WinAnsi Helvetica; CID runs use an
    /// Identity-H font whose ToUnicode map covers every character, so Greek, Cyrillic and CJK read back exactly.
    fn lines_pdf(path: &Path, pages: &[Vec<Line>]) {
        let mut characters: Vec<char> = Vec::new();
        for line in pages.iter().flatten() {
            if let Run::Cid(text) = line.run { for character in text.chars() { if !characters.contains(&character) { characters.push(character); } } }
        }
        let cid = |character: char| characters.iter().position(|known| *known == character).unwrap() as u16 + 1;
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let latin = document.add_object(dictionary! { "Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica", "Encoding"=>"WinAnsiEncoding" });
        let mut cmap = format!("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CMapName /Toolbox-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n{} beginbfchar\n", characters.len());
        for character in &characters {
            let mut units = [0u16; 2];
            let hex: String = character.encode_utf16(&mut units).iter().map(|unit| format!("{unit:04X}")).collect();
            cmap.push_str(&format!("<{:04X}> <{hex}>\n", cid(*character)));
        }
        cmap.push_str("endbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        let to_unicode = document.add_object(Stream::new(dictionary! {}, cmap.into_bytes()));
        let descendant = document.add_object(dictionary! {
            "Type"=>"Font", "Subtype"=>"CIDFontType2", "BaseFont"=>"ToolboxSans",
            "CIDSystemInfo"=>dictionary! { "Registry"=>Object::String(b"Adobe".to_vec(), StringFormat::Literal), "Ordering"=>Object::String(b"Identity".to_vec(), StringFormat::Literal), "Supplement"=>0 },
            "DW"=>600, "CIDToGIDMap"=>"Identity"
        });
        let unicode = document.add_object(dictionary! { "Type"=>"Font", "Subtype"=>"Type0", "BaseFont"=>"ToolboxSans", "Encoding"=>"Identity-H", "DescendantFonts"=>vec![Object::Reference(descendant)], "ToUnicode"=>to_unicode });
        let mut kids = Vec::new();
        for lines in pages {
            let mut content = Vec::new();
            for line in lines {
                content.extend_from_slice(format!("BT {} 12 Tf {} {} Td ", if matches!(line.run, Run::Latin(_)) { "/F1" } else { "/F2" }, line.x, line.y).as_bytes());
                match line.run {
                    Run::Latin(text) => {
                        content.push(b'(');
                        content.extend(text.chars().map(|character| u8::try_from(u32::from(character)).unwrap()));
                        content.extend_from_slice(b") Tj ET\n");
                    }
                    Run::Cid(text) => content.extend_from_slice(format!("<{}> Tj ET\n", text.chars().map(|character| format!("{:04X}", cid(character))).collect::<String>()).as_bytes()),
                }
            }
            let contents = document.add_object(Stream::new(dictionary! {}, content));
            kids.push(Object::Reference(document.add_object(dictionary! {
                "Type"=>"Page", "Parent"=>pages_id, "Contents"=>contents, "MediaBox"=>vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources"=>dictionary! { "Font"=>dictionary! { "F1"=>latin, "F2"=>unicode } }
            })));
        }
        let count = kids.len() as i64;
        document.objects.insert(pages_id, dictionary! { "Type"=>"Pages", "Kids"=>kids, "Count"=>count }.into());
        let catalog = document.add_object(dictionary! { "Type"=>"Catalog", "Pages"=>pages_id });
        document.trailer.set("Root", catalog);
        document.save(path).unwrap();
    }

    fn multi_language_pdf(path: &Path) {
        lines_pdf(path, &[vec![
            Line { x: 72., y: 720., run: Run::Latin("Café naïve façade Ångström") },
            Line { x: 72., y: 690., run: Run::Cid("Καλημέρα κόσμε") },
            Line { x: 72., y: 660., run: Run::Cid("Привет мир") },
            Line { x: 72., y: 630., run: Run::Cid("你好 世界 こんにちは 안녕하세요") },
        ]]);
    }

    fn two_column_pdf(path: &Path) {
        let left = ["Left alpha", "Left bravo", "Left charlie"];
        let right = ["Right delta", "Right echo", "Right foxtrot"];
        let mut lines = Vec::new();
        for (row, (left, right)) in left.iter().zip(right).enumerate() {
            let y = 720. - row as f32 * 16.;
            lines.push(Line { x: 72., y, run: Run::Latin(left) });
            lines.push(Line { x: 340., y, run: Run::Latin(right) });
        }
        lines_pdf(path, &[lines]);
    }

    fn pages_pdf(path: &Path, count: usize) {
        let texts: Vec<String> = (1..=count).map(|page| format!("Toolbox page {page}")).collect();
        lines_pdf(path, &texts.iter().map(|text| vec![Line { x: 72., y: 720., run: Run::Latin(text) }]).collect::<Vec<_>>());
    }

    fn request(input: &Path, mode: ExtractMode, page_range: Option<&str>, preserve_layout: bool) -> ExtractFromPdfRequest {
        ExtractFromPdfRequest { paths: vec![input.to_path_buf()], mode, page_range: page_range.map(str::to_string), output_location: OutputLocation::AlongsideInput, preserve_layout, min_image_size: None }
    }

    fn real_helper() -> Option<TextHelper> {
        let helper = TextHelper::bundled();
        helper.path.is_ok().then_some(helper)
    }

    fn text_of(outcome: &JobOutcome) -> String {
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        fs::read_to_string(&outcome.output_paths()[0]).unwrap()
    }

    #[cfg(unix)]
    fn stand_in(root: &Path, script: &str) -> TextHelper {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join("pdftotext-stand-in");
        fs::write(&path, format!("#!/bin/sh\n{script}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        TextHelper { path: Ok(path), per_file: PDF_TEXT_TIME_PER_FILE, per_page: PDF_TEXT_TIME_PER_PAGE }
    }

    fn files_in(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn accented_greek_cyrillic_and_cjk_text_survives_with_its_word_spacing() {
        let Some(helper) = real_helper() else { return };
        let root = sandbox("languages");
        let input = root.join("languages.pdf");
        multi_language_pdf(&input);
        let text = text_of(&extract_with(&request(&input, ExtractMode::Text, None, false), input.clone(), &helper));
        let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
        assert_eq!(lines, ["Café naïve façade Ångström", "Καλημέρα κόσμε", "Привет мир", "你好 世界 こんにちは 안녕하세요"], "{text:?}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reading_order_is_the_default_and_preserve_layout_keeps_two_columns_aligned() {
        let Some(helper) = real_helper() else { return };
        let root = sandbox("columns");
        let input = root.join("columns.pdf");
        two_column_pdf(&input);
        let reading = extract_with(&request(&input, ExtractMode::Text, None, false), input.clone(), &helper);
        assert_eq!(reading.detail, ("PDF text extracted in reading order"));
        let text = text_of(&reading);
        assert!(text.contains("Left alpha\nLeft bravo\nLeft charlie\n\nRight delta\nRight echo\nRight foxtrot"), "{text:?}");
        let layout = extract_with(&request(&input, ExtractMode::Text, None, true), input.clone(), &helper);
        assert_eq!(layout.detail, ("PDF text extracted with its layout preserved"));
        let text = text_of(&layout);
        let rows: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
        assert_eq!(rows.len(), 3, "{text:?}");
        for (row, (left, right)) in rows.iter().zip([("Left alpha", "Right delta"), ("Left bravo", "Right echo"), ("Left charlie", "Right foxtrot")]) {
            assert!(row.trim_start().starts_with(left) && row.ends_with(right), "{row:?}");
        }
        let column = |row: &str, right: &str| row.find(right).unwrap();
        assert_eq!(column(rows[0], "Right"), column(rows[2], "Right"), "{text:?}");
        assert_eq!(files_in(&root), ["columns-text-1.txt", "columns-text.txt", "columns.pdf"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pages_are_split_by_form_feeds_with_lf_line_endings_and_repeat_byte_for_byte() {
        let Some(helper) = real_helper() else { return };
        let root = sandbox("pages");
        let input = root.join("pages.pdf");
        pages_pdf(&input, 3);
        let original = fs::read(&input).unwrap();
        let first = extract_with(&request(&input, ExtractMode::Text, None, false), input.clone(), &helper);
        let text = text_of(&first);
        assert_eq!(text, "Toolbox page 1\n\n\u{c}Toolbox page 2\n\n\u{c}Toolbox page 3\n\n");
        assert!(!text.contains('\r'));
        let second = extract_with(&request(&input, ExtractMode::Text, None, false), input.clone(), &helper);
        assert_eq!(first.output_paths(), [root.join("pages-text.txt")]);
        assert_eq!(second.output_paths(), [root.join("pages-text-1.txt")]);
        assert_eq!(fs::read(&first.output_paths()[0]).unwrap(), fs::read(&second.output_paths()[0]).unwrap());
        assert_eq!(fs::read(&input).unwrap(), original);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn only_the_selected_pages_are_read() {
        let Some(helper) = real_helper() else { return };
        let root = sandbox("selected");
        let input = root.join("selected.pdf");
        pages_pdf(&input, 5);
        let text = text_of(&extract_with(&request(&input, ExtractMode::Text, Some("4, 1-2"), false), input.clone(), &helper));
        assert_eq!(text, "Toolbox page 1\n\n\u{c}Toolbox page 2\n\n\u{c}Toolbox page 4\n\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn one_range_is_clamped_to_each_pdf_and_a_pdf_outside_it_is_skipped() {
        let root = sandbox("clamped");
        let helper = TextHelper { path: Ok(root.join("never-run")), per_file: PDF_TEXT_TIME_PER_FILE, per_page: PDF_TEXT_TIME_PER_PAGE };
        let [two, five] = [2, 5].map(|count| {
            let path = root.join(format!("pages-{count}.pdf"));
            pages_pdf(&path, count);
            (path, Document::load(root.join(format!("pages-{count}.pdf"))).unwrap())
        });
        assert_eq!(pages_in_range(&five.1, Some("2-4, 9, 4-")), Ok(vec![2, 3, 4, 5]));
        assert_eq!(pages_in_range(&two.1, Some("2-4, 9, 4-")), Ok(vec![2]));
        let empty = SkipReason::new(SkipCode::RangeEmptyAfterClamping, "No pages in 3-5, 8- are in this PDF, which has 2 pages.");
        assert_eq!(pages_in_range(&two.1, Some("3-5, 8-")), Err(PageSelection::Empty(empty.clone())));
        assert_eq!(pages_in_range(&two.1, Some("4-3")), Err(PageSelection::Invalid("\"4-3\" runs backwards. Put the lower page first, for example 3-4.".to_string())));
        assert_eq!(pages_in_range(&two.1, Some("0")), Err(PageSelection::Invalid("\"0\" uses page 0. Page numbers start at 1.".to_string())));
        for mode in [ExtractMode::Text, ExtractMode::Images, ExtractMode::Both] {
            let outcome = extract_with(&request(&two.0, mode, Some(" 3-5, 8- "), false), two.0.clone(), &helper);
            assert_eq!(outcome.state, OutcomeState::Skipped { reason: empty.clone() });
        }
        assert_eq!(files_in(&root), ["pages-2.pdf", "pages-5.pdf"]);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn blank_pages_are_listed_while_the_text_is_saved_and_a_textless_pdf_points_at_ocr() {
        let root = sandbox("blank_pages");
        let input = root.join("partial.pdf");
        pages_pdf(&input, 6);
        // Prints each page in the -f/-l run, leaving pages 2, 3 and 6 without text, the way pdftotext reads scanned pages.
        let helper = stand_in(&root, "i=$6\nwhile [ $i -le $8 ]; do case $i in 2|3|6) printf '\\n\\f' ;; *) printf 'Page %s\\n\\f' $i ;; esac; i=$((i+1)); done\n");
        let outcome = extract_with(&request(&input, ExtractMode::Text, None, false), input.clone(), &helper);
        assert_eq!(text_of(&outcome), "Page 1\n\u{c}\n\u{c}\n\u{c}Page 4\n\u{c}Page 5\n\u{c}\n");
        assert_eq!(outcome.warnings, [pages_without_text(vec![2, 3, 6])]);
        assert_eq!(outcome.warnings[0].message, "Pages 2-3, 6 have no selectable text. OCR can read them.");
        let selected = extract_with(&request(&input, ExtractMode::Text, Some("1, 3, 5"), false), input.clone(), &helper);
        assert_eq!(selected.warnings[0].message, "Page 3 has no selectable text. OCR can read it.");
        let full = extract_with(&request(&input, ExtractMode::Text, Some("4-5"), false), input.clone(), &helper);
        assert!(full.warnings.is_empty(), "{full:?}");
        let scanned = extract_with(&request(&input, ExtractMode::Text, Some("2-3, 6"), false), input.clone(), &helper);
        assert_eq!(scanned.state, OutcomeState::Skipped { reason: SkipReason::new(SkipCode::NeedsOcr, NEEDS_OCR_MESSAGE).with_pointer(OutcomePointer::Tool { tool_id: "pdf-ocr".into() }) });
        let mut names = files_in(&root);
        names.retain(|name| name.ends_with(".txt"));
        assert_eq!(names, ["partial-text-1.txt", "partial-text-2.txt", "partial-text.txt"], "the textless run writes nothing");
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn the_helper_gets_utf8_unix_line_endings_the_layout_flag_and_one_call_per_page_run() {
        let root = sandbox("arguments");
        let input = root.join("arguments.pdf");
        pages_pdf(&input, 5);
        let helper = stand_in(&root, "printf '%s\\r\\n\\f' \"$*\"\n");
        let argument_lines = |preserve_layout: bool| {
            let outcome = extract_with(&request(&input, ExtractMode::Text, Some("1-2, 4"), preserve_layout), input.clone(), &helper);
            let text = text_of(&outcome);
            assert!(!text.contains('\r') && !text.ends_with('\u{c}'), "{text:?}");
            text.split('\u{c}').map(|line| line.trim_end().replace(&*input.to_string_lossy(), "INPUT")).collect::<Vec<_>>()
        };
        assert_eq!(argument_lines(false), ["-enc UTF-8 -eol unix -f 1 -l 2 INPUT -", "-enc UTF-8 -eol unix -f 4 -l 4 INPUT -"]);
        assert_eq!(argument_lines(true), ["-enc UTF-8 -eol unix -layout -f 1 -l 2 INPUT -", "-enc UTF-8 -eol unix -layout -f 4 -l 4 INPUT -"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn encrypted_and_damaged_pdfs_fail_as_invalid_input_before_the_helper_runs() {
        let root = sandbox("unreadable");
        let helper = TextHelper { path: Ok(root.join("never-run")), per_file: PDF_TEXT_TIME_PER_FILE, per_page: PDF_TEXT_TIME_PER_PAGE };
        let locked = root.join("locked.pdf");
        fs::write(&locked, crate::kit::password::tests::pdf(Some(("owner", "user")))).unwrap();
        let damaged = root.join("damaged.pdf");
        fs::write(&damaged, b"%PDF-1.7\nthis is not really a PDF").unwrap();
        for mode in [ExtractMode::Text, ExtractMode::Images, ExtractMode::Both] {
            let outcome = extract_with(&request(&locked, mode, None, false), locked.clone(), &helper);
            let error = outcome.error().unwrap();
            assert_eq!((&error.kind, error.message.as_str()), (&ErrorKind::InvalidInput, EXTRACT_PASSWORD_PROTECTED));
            assert_eq!(error.pointer, Some(OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
            let outcome = extract_with(&request(&damaged, mode, None, false), damaged.clone(), &helper);
            let error = outcome.error().unwrap();
            assert_eq!((&error.kind, error.message.as_str(), &error.pointer), (&ErrorKind::InvalidInput, PDF_DAMAGED, &None));
        }
        assert_eq!(files_in(&root), ["damaged.pdf", "locked.pdf"]);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn a_pdf_with_only_an_owner_password_is_read() {
        let root = sandbox("owner_only");
        let input = root.join("restricted.pdf");
        fs::write(&input, crate::kit::password::tests::pdf(Some(("owner", "")))).unwrap();
        let helper = stand_in(&root, "printf 'Restricted\\n\\f'\n");
        let outcome = extract_with(&request(&input, ExtractMode::Text, None, false), input.clone(), &helper);
        assert_eq!(text_of(&outcome), "Restricted\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_helper_is_unavailable_with_a_reinstall_hint_and_images_mode_still_runs() {
        let root = sandbox("missing");
        let input = root.join("missing.pdf");
        pages_pdf(&input, 1);
        let helper = TextHelper { path: Err("missing".into()), per_file: PDF_TEXT_TIME_PER_FILE, per_page: PDF_TEXT_TIME_PER_PAGE };
        for mode in [ExtractMode::Text, ExtractMode::Both] {
            let outcome = extract_with(&request(&input, mode, None, false), input.clone(), &helper);
            let error = outcome.error().unwrap();
            assert_eq!((&error.kind, error.message.as_str()), (&ErrorKind::Unavailable, PDF_TEXT_HELPER_MISSING));
            assert!(error.message.contains("Reinstall Toolbox"));
        }
        let images = extract_with(&request(&input, ExtractMode::Images, None, false), input.clone(), &helper);
        assert!(matches!(&images.state, crate::kit::contracts::OutcomeState::Skipped { reason } if reason.message == NO_EMBEDDED_IMAGES), "{images:?}");
        assert_eq!(files_in(&root), ["missing.pdf"]);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn when_every_image_is_too_small_the_result_says_so_in_each_mode() {
        let root = sandbox("all_small");
        let input = root.join("small.pdf");
        super::tests::image_pdf(&input, |_| vec![vec![super::tests::image_stream(dictionary! { "Width"=>2, "Height"=>2, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8 }, vec![0; 4])]]);
        let small = |mode| ExtractFromPdfRequest { min_image_size: Some(64), ..request(&input, mode, None, false) };
        let skipped = |outcome: &JobOutcome| match &outcome.state { crate::kit::contracts::OutcomeState::Skipped { reason } => reason.message.clone(), state => format!("{state:?}") };
        let blank = stand_in(&root, "printf '\\f'\n");

        assert_eq!(skipped(&extract_with(&small(ExtractMode::Images), input.clone(), &blank)), "The 1 image is smaller than the minimum size. No folder was made.");
        assert_eq!(skipped(&extract_with(&small(ExtractMode::Both), input.clone(), &blank)), "No selectable text, and the 1 image is smaller than the minimum size. Nothing was saved.");
        assert_eq!(files_in(&root), ["pdftotext-stand-in", "small.pdf"]);

        let words = stand_in(&root, "printf 'Words\\n\\f'\n");
        let outcome = extract_with(&small(ExtractMode::Both), input.clone(), &words);
        assert_eq!(outcome.detail, "The 1 image is smaller than the minimum size, so only the text file was saved.");
        assert_eq!(text_of(&outcome), "Words\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_text_deadline_is_twenty_seconds_per_file_plus_fifty_milliseconds_per_page() {
        assert_eq!(PDF_TEXT_TIME_PER_FILE, Duration::from_secs(20));
        assert_eq!(PDF_TEXT_TIME_PER_PAGE, Duration::from_millis(50));
        let helper = TextHelper { path: Err(String::new()), per_file: PDF_TEXT_TIME_PER_FILE, per_page: PDF_TEXT_TIME_PER_PAGE };
        assert_eq!(helper.time_limit(1), Duration::from_millis(20_050));
        assert_eq!(helper.time_limit(200), Duration::from_secs(30));
        let longest = crate::kit::common::limits::recorded("extract_from_pdf").unwrap();
        assert_eq!(helper.time_limit(longest.max_pages.unwrap() as usize), Duration::from_secs(270));
        assert!(helper.time_limit(longest.max_pages.unwrap() as usize) < longest.max_duration, "the helper deadline ends a long read before the job limit does");
    }

    #[cfg(unix)]
    #[test]
    fn a_read_past_its_deadline_fails_alone_with_nothing_saved() {
        let root = sandbox("deadline");
        let slow = root.join("slow.pdf");
        let quick = root.join("quick.pdf");
        pages_pdf(&slow, 1);
        pages_pdf(&quick, 1);
        let mut helper = stand_in(&root, "case \"$*\" in *slow.pdf*) exec /bin/sleep 5;; esac\nprintf 'Quick\\n\\f'\n");
        helper.per_file = Duration::from_millis(1_000);
        helper.per_page = Duration::from_millis(20);
        let started = Instant::now();
        let outcome = extract_with(&request(&slow, ExtractMode::Text, None, false), slow.clone(), &helper);
        assert!(started.elapsed() < Duration::from_secs(4), "the helper must be stopped at its deadline");
        let error = outcome.error().unwrap();
        assert_eq!(error.kind, ErrorKind::Processing);
        assert_eq!(error.message, "Reading text took longer than 1 second, so it was stopped. The other PDFs weren't affected.");
        assert!(outcome.output_paths().is_empty());
        assert_eq!(text_of(&extract_with(&request(&quick, ExtractMode::Text, None, false), quick.clone(), &helper)), "Quick\n");
        assert_eq!(files_in(&root), ["pdftotext-stand-in", "quick-text.txt", "quick.pdf", "slow.pdf"]);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn text_past_its_size_cap_fails_with_nothing_saved() {
        let root = sandbox("size_cap");
        let input = root.join("huge.pdf");
        pages_pdf(&input, 1);
        let helper = stand_in(&root, &format!("exec /bin/dd if=/dev/zero bs=1048576 count={} 2>/dev/null\n", PDF_TEXT_FILE_MAX_BYTES / 1_048_576 + 1));
        let outcome = extract_with(&request(&input, ExtractMode::Text, None, false), input.clone(), &helper);
        let error = outcome.error().unwrap();
        assert_eq!((&error.kind, error.message.as_str()), (&ErrorKind::Processing, "The text is over 100 MB, so nothing was saved."));
        assert_eq!(files_in(&root), ["huge.pdf", "pdftotext-stand-in"]);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    const BUNDLED_CHILD: &str = "TOOLBOX_EXTRACT_TEXT_BUNDLED_CHILD";

    #[cfg(unix)]
    #[test]
    fn extraction_uses_the_bundled_pdftotext_when_path_is_empty() {
        if std::env::var_os(BUNDLED_CHILD).is_none() {
            use std::os::unix::fs::PermissionsExt;
            let _guard = crate::kit::lock_process_env();
            let root = sandbox("bundled");
            let pdf_bin = root.join("Resources").join("pdf-bin");
            fs::create_dir_all(&pdf_bin).unwrap();
            let helper = pdf_bin.join("pdftotext");
            fs::write(&helper, "#!/bin/sh\nprintf 'BundledStandIn\\n\\f'\n").unwrap();
            fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["kit::pdf::remaining::text_extraction_tests::extraction_uses_the_bundled_pdftotext_when_path_is_empty", "--exact", "--nocapture", "--test-threads=1"])
                .env(BUNDLED_CHILD, root.join("work"))
                .env("PATH", "")
                .env("TOOLBOX_RESOURCE_ROOT", root.join("Resources"))
                .env_remove("TOOLBOX_PDFTOTEXT_PATH")
                .output()
                .unwrap();
            let report = format!("{}{}", String::from_utf8_lossy(&child.stdout), String::from_utf8_lossy(&child.stderr));
            assert!(child.status.success() && report.contains("1 passed"), "{report}");
            let _ = fs::remove_dir_all(root);
            return;
        }
        assert_eq!(std::env::var_os("PATH").unwrap_or_default(), "");
        let root = PathBuf::from(std::env::var_os(BUNDLED_CHILD).unwrap());
        fs::create_dir_all(&root).unwrap();
        let input = root.join("bundled.pdf");
        pages_pdf(&input, 1);
        assert_eq!(text_of(&extract(&request(&input, ExtractMode::Text, None, false), input.clone())), "BundledStandIn\n");
    }
}
