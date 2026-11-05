use image::codecs::jpeg::JpegEncoder;
use image::DynamicImage;
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::kit::common::limits::{bytes_label, duration_label, StoppableHelper};
use crate::kit::common::{FolderReservation, JobOutcome, OutputLocation, OutputNaming, OutputReservation};
use crate::kit::contracts::{JpegColour, OutcomePointer, OutcomeWarning, SkipCode, SkipReason, ToolError, WarningKind};
use super::embedded_image::{self, Extracted, NotSaved};
use super::inline_image;
use super::metadata::{page_bounds, page_rotation};
use super::{mutation_preflight, PdfMutationIntent};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageOverlayRequest {
    pub paths: Vec<PathBuf>,
    pub text: String,
    #[serde(default)] pub opacity: u8,
    #[serde(default)] pub position: Option<String>,
    #[serde(default)] pub logo_path: Option<PathBuf>,
    #[serde(default)] pub pages: Option<Vec<usize>>,
    #[serde(default)] pub start_number: Option<u32>,
    #[serde(default)] pub font_size: Option<u16>,
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
    #[serde(default)] pub split_mode: Option<String>,
    #[serde(default)] pub chunk_size: Option<usize>,
    pub output_location: OutputLocation,
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

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagesToPdfRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
    #[serde(default)]
    pub output_path: Option<PathBuf>,
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

pub fn merge(request: &MergePdfRequest) -> JobOutcome {
    let Some(first) = request.paths.first().cloned() else { return failure(PathBuf::new(), "Select at least one PDF.".to_string()); };
    if request.paths.len() < 2 { return failure(first, "Select at least two PDFs to merge.".to_string()); }
    let mut expected_pages = 0usize;
    for path in &request.paths {
        if path.extension().and_then(|extension| extension.to_str()).is_none_or(|extension| !extension.eq_ignore_ascii_case("pdf")) { return failure(path.clone(), format!("Only PDF inputs can be merged: {}", path.display())); }
        let document = match load_existing_document_for_mutation(path) {
            Ok(document) => document,
            Err(error) => return failure(path.clone(), format!("Could not read {}: {error}", path.display())),
        };
        expected_pages += document.get_pages().len();
    }
    let output = match OutputNaming::reserve_single_output(request.output_path.as_deref(), &first, &request.output_location, "-merged", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(first, format!("Could not reserve merged PDF output: {error}")),
    };
    let qpdf = match qpdf() { Ok(qpdf) => qpdf, Err(error) => return JobOutcome::failure(first, ToolError::unavailable(error)) };
    let result = Command::new(qpdf).arg("--empty").arg("--pages").args(&request.paths).arg("--").arg(output.path()).output_until_stopped();
    match result {
        Ok(result) if result.status.success() => match Document::load(output.path()) {
            Ok(document) if document.get_pages().len() == expected_pages => match output.publish() {
                Ok(path) => JobOutcome::succeeded(first, vec![path], format!("{} PDFs merged in input order", request.paths.len())),
                Err(error) => failure(first, format!("Could not publish merged PDF output: {error}")),
            },
            Ok(document) => failure(first, format!("Merged PDF page count mismatch: expected {expected_pages}, got {}.", document.get_pages().len())),
            Err(error) => failure(first, format!("Merged PDF could not be verified: {error}")),
        },
        Ok(result) => failure(first, stderr(result, "qpdf failed to merge the PDFs.")),
        Err(error) => failure(first, format!("Could not run qpdf: {error}")),
    }
}

pub fn split(request: &PageSelectionRequest, input: PathBuf) -> JobOutcome {
    if let Err(error) = validate_split_request(request) {
        return JobOutcome::failure(input, ToolError::invalid_input(error));
    }
    if let Err(error) = load_existing_document_for_mutation(&input) {
        return failure(input, error);
    }
    let location = &request.output_location;
    let qpdf = match qpdf() { Ok(qpdf) => qpdf, Err(error) => return JobOutcome::failure(input, ToolError::unavailable(error)) };
    let workspace = match SplitWorkspace::new() {
        Ok(workspace) => workspace,
        Err(error) => return failure(input, format!("Could not create a private split workspace: {error}")),
    };
    let generated = match generate_split_outputs(request, &input, &qpdf, &workspace) {
        Ok(generated) => generated,
        Err(error) => return failure(input, error),
    };
    match materialize_split_outputs(&input, location, &generated) {
        Ok(outputs) => {
            let detail = if request.page_ranges.as_deref().is_some_and(|value| !value.trim().is_empty()) {
                let stem = input.file_stem().and_then(|value| value.to_str()).unwrap_or("split");
                format!("PDF split into selected ranges from {stem}")
            } else {
                "PDF split".to_string()
            };
            JobOutcome::succeeded(input, outputs, detail)
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

fn generate_split_outputs(request: &PageSelectionRequest, input: &Path, qpdf: &Path, workspace: &SplitWorkspace) -> Result<Vec<PathBuf>, String> {
    let stem = input.file_stem().and_then(|value| value.to_str()).unwrap_or("output");
    if request.split_mode.as_deref() == Some("chunks") {
        let Some(size) = request.chunk_size.filter(|size| *size > 0) else { return Err("Chunk size must be greater than zero.".to_string()); };
        let output = workspace.path().join(format!("{stem}-split.pdf"));
        let result = Command::new(qpdf).arg(input).arg(format!("--split-pages={size}")).arg(&output).output_until_stopped().map_err(|error| format!("Could not run qpdf: {error}"))?;
        if !result.status.success() { return Err(stderr(result, "qpdf failed to split PDF chunks.")); }
        return collect_split_outputs(workspace.path(), "qpdf reported success but produced no chunk files.");
    }
    if let Some(raw) = request.page_ranges.as_deref().filter(|value| !value.trim().is_empty()) {
        let page_count = Document::load(input).map_err(|error| error.to_string())?.get_pages().len();
        let ranges = parse_split_ranges(raw, page_count)?;
        let mut outputs = Vec::with_capacity(ranges.len());
        for (number, (start, end)) in ranges.into_iter().enumerate() {
            let output = workspace.path().join(format!("{stem}-split-{number}.pdf"));
            let result = Command::new(qpdf).arg(input).arg("--pages").arg(input).arg(format!("{start}-{end}")).arg("--").arg(&output).output_until_stopped().map_err(|error| format!("Could not run qpdf: {error}"))?;
            if !result.status.success() { return Err(stderr(result, "qpdf failed to split the selected ranges.")); }
            if !output.is_file() { return Err("qpdf reported success but produced no range file.".to_string()); }
            outputs.push(output);
        }
        return Ok(outputs);
    }
    let pattern = workspace.path().join(format!("{stem}-split-page-%d.pdf"));
    let result = Command::new(qpdf).arg(input).arg("--split-pages").arg(&pattern).output_until_stopped().map_err(|error| format!("Could not run qpdf: {error}"))?;
    if !result.status.success() { return Err(stderr(result, "qpdf failed to split the PDF.")); }
    collect_split_outputs(workspace.path(), "qpdf reported success but produced no split files.")
}

fn collect_split_outputs(workspace: &Path, empty_message: &str) -> Result<Vec<PathBuf>, String> {
    let mut outputs = fs::read_dir(workspace).map_err(|error| format!("Could not inspect qpdf split outputs: {error}"))?.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.is_file() && path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))).collect::<Vec<_>>();
    outputs.sort();
    if outputs.is_empty() { Err(empty_message.to_string()) } else { Ok(outputs) }
}

fn materialize_split_outputs(input: &Path, location: &OutputLocation, generated: &[PathBuf]) -> Result<Vec<PathBuf>, JobOutcome> {
    let mut outputs = Vec::with_capacity(generated.len());
    for source in generated {
        let suffix = match split_output_suffix(input, source) {
            Ok(suffix) => suffix,
            Err(error) => return Err(JobOutcome::failure_with_outputs(input.to_path_buf(), outputs, ToolError::processing(error))),
        };
        let reservation = match OutputNaming::reserve_destination(input, location, &suffix, "pdf") {
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

fn split_output_suffix(input: &Path, generated: &Path) -> Result<String, String> {
    let input_stem = input.file_stem().and_then(|value| value.to_str()).unwrap_or("output");
    let generated_stem = generated.file_stem().and_then(|value| value.to_str()).ok_or_else(|| "qpdf produced an unnamed split file.".to_string())?;
    let suffix = generated_stem.strip_prefix(input_stem).ok_or_else(|| "qpdf produced an unexpected split file name.".to_string())?;
    if !suffix.starts_with("-split") { return Err("qpdf produced an unexpected split file name.".to_string()); }
    Ok(suffix.to_string())
}

fn copy_into_reservation(source: &Path, destination: &Path) -> io::Result<()> {
    fs::copy(source, destination)?;
    fs::remove_file(source)
}

fn validate_split_request(request: &PageSelectionRequest) -> Result<(), String> {
    if request.split_mode.as_deref() == Some("ranges") && request.page_ranges.as_deref().is_none_or(|value| value.trim().is_empty()) {
        return Err("Enter at least one page range when split mode is Ranges.".to_string());
    }
    Ok(())
}

fn parse_split_ranges(raw: &str, page_count: usize) -> Result<Vec<(usize, usize)>, String> {
    raw.split(',').map(str::trim).map(|token| {
        let (start, end) = token.split_once('-').unwrap_or((token, token));
        let start = parse_page_number(start)?;
        let end = if end.trim().is_empty() { page_count } else { parse_page_number(end)? };
        if start > end || end > page_count { return Err(format!("Invalid page range: {token}.")); }
        Ok((start, end))
    }).collect()
}

pub fn to_images(request: &PdfToImagesRequest, input: PathBuf) -> JobOutcome {
    let format = request.format.to_lowercase();
    if format != "jpg" && format != "png" { return failure(input, "PDF images must be JPEG or PNG.".to_string()); }
    let Some(renderer) = tool("TOOLBOX_PDFTOPPM_PATH", "pdftoppm") else { return failure(input, "pdftoppm is required to render PDFs. Set TOOLBOX_PDFTOPPM_PATH or add pdftoppm to PATH.".to_string()); };
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
        let document = match Document::load(&input) {
            Ok(document) => document,
            Err(error) => return failure(input, error.to_string()),
        };
        let pages = match selected_pdf_pages(&document, Some(requested_pages)) {
            Ok(pages) if !pages.is_empty() => pages,
            Ok(_) => return failure(input, "Select at least one PDF page.".to_string()),
            Err(error) => return failure(input, error),
        };
        let generated = match render_selected_pages(&renderer, &input, &prefix, renderer_format, dpi, extension, &pages) {
            Ok(outputs) => outputs,
            Err(error) => return failure(input, error),
        };
        return materialize_rendered_images(&input, &request.output_location, &generated);
    }

    let mut command = Command::new(&renderer);
    command.arg(format!("-{renderer_format}")).arg("-r").arg(dpi.to_string());
    if let Some(range) = request.page_range.as_deref().filter(|value| !value.trim().is_empty()) {
        let (first, last) = range.split_once('-').unwrap_or((range, range));
        let first = match first.trim().parse::<usize>() { Ok(page) if page > 0 => page, _ => return failure(input, "Page range start must be a positive number.".to_string()) };
        let last = match last.trim().parse::<usize>() { Ok(page) if page >= first => page, _ => return failure(input, "Page range must be ascending and use positive page numbers.".to_string()) };
        command.arg("-f").arg(first.to_string()).arg("-l").arg(last.to_string());
    }
    let output = command.arg(&input).arg(&prefix).output_until_stopped();
    match output {
        Ok(result) if result.status.success() => {
            let mut outputs = fs::read_dir(prefix.parent().unwrap_or_else(|| std::path::Path::new("."))).ok().into_iter().flatten().filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(prefix.file_name().and_then(|stem| stem.to_str()).unwrap_or("")) && path.extension().is_some_and(|ext| ext == extension))).collect::<Vec<_>>();
            outputs.sort_by_key(|path| rendered_page_index(path, &prefix));
            if outputs.is_empty() {
                failure(input, "PDF renderer produced no images.".to_string())
            } else {
                materialize_rendered_images(&input, &request.output_location, &outputs)
            }
        }
        Ok(result) => failure(input, stderr(result, "pdftoppm failed to render the PDF.")),
        Err(error) => failure(input, format!("Could not run pdftoppm: {error}")),
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
) -> Result<Vec<PathBuf>, String> {
    let mut outputs = Vec::with_capacity(pages.len());
    let base = prefix.file_name().and_then(|value| value.to_str()).unwrap_or("output");
    for page in pages {
        let page_prefix = prefix.with_file_name(format!("{base}-page-{page}"));
        let output = page_prefix.with_extension(extension);
        let result = Command::new(renderer)
            .arg(format!("-{renderer_format}"))
            .arg("-r")
            .arg(dpi.to_string())
            .arg("-f")
            .arg(page.to_string())
            .arg("-l")
            .arg(page.to_string())
            .arg("-singlefile")
            .arg(input)
            .arg(&page_prefix)
            .output_until_stopped();
        match result {
            Ok(result) if result.status.success() && output.exists() => outputs.push(output),
            Ok(result) => return Err(stderr(result, "pdftoppm failed to render the selected PDF page.")),
            Err(error) => return Err(format!("Could not run pdftoppm: {error}")),
        }
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
pub const PDF_PASSWORD_PROTECTED: &str = "This PDF is password-protected. Remove the password, then extract again.";
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
        Ok(_) => return failure(input, "Select at least one PDF page.".to_string()),
        Err(error) => return failure(input, error),
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
                let outputs = text.into_iter().chain(folder).collect();
                return warnings.into_iter().fold(JobOutcome::failure_with_outputs(input, outputs, ToolError::processing(message)), JobOutcome::with_warning);
            }
        }
    } else { ImageRun::default() };
    let tally = images.tally();
    let all_small = images.all_small();
    let skipped = |message: String| JobOutcome::skipped(input.clone(), SkipReason::new(SkipCode::NoEffectiveChange, message));
    let outcome = match (request.mode, text, images.saved) {
        (ExtractMode::Text, Some(text), _) => JobOutcome::succeeded(input, vec![text], if request.preserve_layout { "PDF text extracted with its layout preserved" } else { "PDF text extracted in reading order" }),
        (ExtractMode::Text, None, _) => failure(input, NO_SELECTABLE_TEXT.to_string()),
        (ExtractMode::Images, _, None) => skipped(all_small.map_or(NO_EMBEDDED_IMAGES.to_string(), |lead| format!("{lead}. No folder was made."))),
        (ExtractMode::Images, _, Some(saved)) => JobOutcome::succeeded(input, vec![saved.folder], format!("Saved {tally}.")),
        (ExtractMode::Both, Some(text), Some(saved)) => JobOutcome::succeeded(input, vec![text, saved.folder], format!("Saved the text and {tally}.")),
        (ExtractMode::Both, Some(text), None) => JobOutcome::succeeded(input, vec![text], all_small.map_or(BOTH_WITHOUT_IMAGES.to_string(), |lead| format!("{lead}, so only the text file was saved."))),
        (ExtractMode::Both, None, Some(saved)) => JobOutcome::succeeded(input, vec![saved.folder], format!("{BOTH_WITHOUT_TEXT} Saved {tally}.")),
        (ExtractMode::Both, None, None) => skipped(all_small.map_or(NOTHING_TO_EXTRACT.to_string(), |lead| format!("No selectable text, and {}{}. Nothing was saved.", lead[..1].to_lowercase(), &lead[1..]))),
    };
    images.warnings.into_iter().fold(outcome, JobOutcome::with_warning)
}

/// Resolves the shared range syntax against one PDF into 1-based page numbers in document order.
fn pages_in_range(document: &Document, range: Option<&str>) -> Result<Vec<u32>, String> {
    let count = document.get_pages().len();
    let Some(raw) = range.filter(|value| !value.trim().is_empty()) else { return Ok((1..=count as u32).collect()); };
    let mut selected = std::collections::BTreeSet::new();
    for token in raw.split(',').map(str::trim) {
        if token.is_empty() { return Err("Page ranges cannot contain empty entries.".to_string()); }
        let (start, end) = match token.split_once('-') {
            Some((start, end)) if end.trim().is_empty() => (parse_page_number(start)?, count.max(1)),
            Some((start, end)) => (parse_page_number(start)?, parse_page_number(end)?),
            None => { let page = parse_page_number(token)?; (page, page) },
        };
        if start > count || end > count { return Err(format!("Page range {token} is outside the document.")); }
        if start > end { return Err(format!("{token} runs backwards. Write it as {end}-{start}.")); }
        selected.extend((start as u32)..=(end as u32));
    }
    Ok(selected.into_iter().collect())
}

fn password_protected(input: PathBuf) -> JobOutcome {
    JobOutcome::failure(input, ToolError::invalid_input(PDF_PASSWORD_PROTECTED).with_pointer(OutcomePointer::Tool { tool_id: "lock-unlock".into() }))
}

/// Writes the selected pages' text with pdftotext. `None` means the pages hold no selectable text, and nothing is written.
fn write_text(input: &Path, pages: &[u32], helper: &TextHelper, preserve_layout: bool, location: &OutputLocation) -> Result<Option<PathBuf>, ToolError> {
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
    if text.trim_matches(|character: char| character.is_whitespace() || character == '\u{c}').is_empty() { return Ok(None); }
    let output = OutputNaming::reserve_destination(input, location, "-text", "txt").map_err(|error| ToolError::processing(format!("Could not reserve PDF text output: {error}")))?;
    fs::write(output.path(), text).map_err(|error| ToolError::processing(format!("Could not write text output: {error}")))?;
    output.publish().map(Some).map_err(|error| ToolError::processing(format!("Could not publish text output: {error}")))
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
    for path in &request.paths {
        let (jpeg, width, height) = match image_as_jpeg(path) { Ok(value) => value, Err(error) => return failure(path.clone(), format!("Could not process {}: {error}", path.display())) };
        let image_id = document.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64, "Height" => height as i64, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode" }, jpeg));
        let content_id = document.add_object(Stream::new(dictionary! {}, format!("q {width} 0 0 {height} 0 0 cm /Im0 Do Q").into_bytes()));
        let page_id = document.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), (width as f32).into(), (height as f32).into()], "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } }, "Contents" => content_id });
        kids.push(page_id.into());
    }
    document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => request.paths.len() as i64 }.into());
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    match document.save(output.path()) {
        Ok(_) => match output.publish() {
            Ok(path) => JobOutcome::succeeded(first, vec![path], "Images combined into PDF"),
            Err(error) => failure(first, format!("Could not publish combined PDF: {error}")),
        },
        Err(error) => failure(first, format!("Save failed: {error}")),
    }
}

fn image_as_jpeg(path: &PathBuf) -> Result<(Vec<u8>, u32, u32), String> {
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_lowercase();
    if matches!(extension.as_str(), "gif" | "tif" | "tiff") { return Err("Animated or multi-frame image inputs are not supported for image-to-PDF conversion.".to_string()); }
    let image = if matches!(extension.as_str(), "heic" | "heif") {
        let converted = std::env::temp_dir().join(format!("toolbox_heic_{}_{}.jpg", std::process::id(), path.file_name().and_then(|name| name.to_str()).unwrap_or("input")));
        let result = Command::new("sips").arg("-s").arg("format").arg("jpeg").arg(path).arg("--out").arg(&converted).output_until_stopped()
            .map_err(|error| format!("Could not run the HEIC decoder: {error}"))?;
        if !result.status.success() { return Err(stderr(result, "HEIC decoding is unavailable on this system.")); }
        let decoded = image::open(&converted).map_err(|error| format!("Could not decode HEIC image: {error}"));
        let _ = fs::remove_file(&converted);
        decoded?
    } else { image::open(path).map_err(|error| format!("Could not decode image: {error}"))? };
    let (width, height) = (image.width(), image.height());
    let mut output = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut output, 92).encode_image(&DynamicImage::ImageRgb8(image.to_rgb8())).map_err(|error| format!("Could not encode image as JPEG: {error}"))?;
    Ok((output.into_inner(), width, height))
}

pub fn add_page_numbers(request: &PageOverlayRequest, input: PathBuf) -> JobOutcome {
    overlay(request, input, "-numbered", |page_number, page, font_name, _state_name| {
        let number = request.start_number.unwrap_or(1).saturating_add(page_number as u32).saturating_sub(1);
        let size = request.font_size.unwrap_or(12).clamp(8, 72);
        let (x, y) = number_position(request.position.as_deref(), page.display_width, page.display_height);
        let (x, y) = page_point_from_display(page, x, y);
        format!("BT /{font_name} {size} Tf {x} {y} Td ({number}) Tj ET")
    }, 100)
}

pub fn watermark(request: &PageOverlayRequest, input: PathBuf) -> JobOutcome {
    if let Some(path) = &request.logo_path { return watermark_image(request, input, path); }
    overlay(request, input, "-watermarked", |_, page, font_name, _state_name| {
        let (x, y) = watermark_position(request.position.as_deref(), page.display_width, page.display_height);
        let (x, y) = page_point_from_display(page, x, y);
        format!("BT /{font_name} 48 Tf {x} {y} Td ({}) Tj ET", escape(&request.text))
    }, request.opacity.clamp(1, 100))
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

pub fn remove_pages(request: &PageSelectionRequest, input: PathBuf) -> JobOutcome {
    let mut document = match load_existing_document_for_mutation(&input) { Ok(document) => document, Err(error) => return failure(input, error) };
    let pages = document.get_pages();
    let selected = match selected_page_indices(request, pages.len()) { Ok(selected) => selected, Err(error) => return failure(input, error) };
    let delete = selected.into_iter().map(|page| page as u32 + 1).collect::<Vec<_>>();
    if delete.len() >= pages.len() { return failure(input, "The output must keep at least one page.".to_string()); }
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-pages-removed", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve page removal output: {error}")),
    };
    document.delete_pages(&delete);
    save(document, input, output, "PDF pages removed")
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
    let mut selected = std::collections::BTreeSet::new();
    for token in raw.split(',').map(str::trim) {
        if token.is_empty() { return Err("Page ranges cannot contain empty entries.".to_string()); }
        let (start, end) = match token.split_once('-') {
            Some((start, end)) => (parse_page_number(start)?, parse_page_number(end)?),
            None => { let page = parse_page_number(token)?; (page, page) },
        };
        if start > end { return Err(format!("Page range {token} is reversed.")); }
        if end > page_count { return Err(format!("Page range {token} is outside the document.")); }
        selected.extend((start - 1)..end);
    }
    if selected.is_empty() { return Err("Select at least one page.".to_string()); }
    Ok(selected.into_iter().collect())
}

fn parse_page_number(value: &str) -> Result<usize, String> {
    let page = value.trim().parse::<usize>().map_err(|_| format!("Invalid page number: {value}."))?;
    if page == 0 { return Err("Page numbers start at 1.".to_string()); }
    Ok(page)
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

#[derive(Clone, Copy)]
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

fn overlay<F>(request: &PageOverlayRequest, input: PathBuf, suffix: &str, content: F, opacity: u8) -> JobOutcome
where F: Fn(usize, OverlayPageGeometry, &str, &str) -> String {
    let mut document = match load_existing_document_for_mutation(&input) { Ok(document) => document, Err(error) => return failure(input, error) };
    if let Err(error) = validate_overlay_scope(request.pages.as_deref(), document.get_pages().len()) { return failure(input, error); }
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, suffix, "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve PDF overlay output: {error}")),
    };
    let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    let opacity_id = document.add_object(dictionary! { "Type" => "ExtGState", "ca" => opacity as f32 / 100.0, "CA" => opacity as f32 / 100.0 });
    for (number, page_id) in document.get_pages().values().copied().enumerate() {
        if request.pages.as_ref().is_some_and(|pages| !pages.contains(&number)) { continue; }
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

fn watermark_position(position: Option<&str>, width: f32, height: f32) -> (f32, f32) {
    match position.unwrap_or("center") {
        "top-left" => (24.0, height - 48.0), "top-right" => (width - 180.0, height - 48.0),
        "bottom-left" => (24.0, 24.0), "bottom-right" => (width - 180.0, 24.0),
        _ => (width / 4.0, height / 2.0),
    }
}

fn number_position(position: Option<&str>, width: f32, height: f32) -> (f32, f32) {
    match position.unwrap_or("bottom-right") {
        "bottom-left" => (24.0, 24.0), "top-left" => (24.0, height - 24.0), "top-right" => (width - 60.0, height - 24.0),
        _ => (width - 60.0, 24.0),
    }
}

fn watermark_image(request: &PageOverlayRequest, input: PathBuf, logo: &PathBuf) -> JobOutcome {
    let mut document = match load_existing_document_for_mutation(&input) { Ok(document) => document, Err(error) => return failure(input, error) };
    if let Err(error) = validate_overlay_scope(request.pages.as_deref(), document.get_pages().len()) { return failure(input, error); }
    let (bytes, image_width, image_height) = match image_as_jpeg(logo) { Ok(value) => value, Err(error) => return failure(input, error) };
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-watermarked", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve watermarked PDF output: {error}")),
    };
    let image_id = document.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => image_width as i64, "Height" => image_height as i64, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode" }, bytes));
    let state_id = document.add_object(dictionary! { "Type" => "ExtGState", "ca" => request.opacity.clamp(1, 100) as f32 / 100.0, "CA" => request.opacity.clamp(1, 100) as f32 / 100.0 });
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
        let (x, y) = watermark_position(request.position.as_deref(), page.display_width, page.display_height);
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
mod tests {
    use super::*;

    static SPLIT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn request(page_ranges: Option<&str>, pages: Vec<usize>) -> PageSelectionRequest {
        PageSelectionRequest {
            paths: vec![],
            pages,
            page_ranges: page_ranges.map(str::to_string),
            split_mode: None,
            chunk_size: None,
            output_location: OutputLocation::AlongsideInput,
        }
    }

    fn split_request(output_location: OutputLocation, split_mode: &str, page_ranges: Option<&str>, chunk_size: Option<usize>) -> PageSelectionRequest {
        PageSelectionRequest {
            paths: vec![],
            pages: vec![],
            page_ranges: page_ranges.map(str::to_string),
            split_mode: Some(split_mode.to_string()),
            chunk_size,
            output_location,
        }
    }

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
            let request = PageOverlayRequest {
                paths: vec![input.clone()], text: "TEST".into(), opacity: 70, position: Some("center".into()),
                logo_path: (case == "image-watermark").then_some(logo), pages: None, start_number: None,
                font_size: None, output_location: OutputLocation::CustomFolder(output),
            };
            let outcome = if case == "numbers" { add_page_numbers(&request, input.clone()) } else { watermark(&request, input.clone()) };
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
            let request = PageOverlayRequest {
                paths: vec![input.clone()], text: "TEST".into(), opacity: 70, position: Some("center".into()),
                logo_path: (case == "image-watermark").then_some(logo), pages: None, start_number: None,
                font_size: None, output_location: OutputLocation::CustomFolder(output),
            };
            let outcome = if case == "numbers" { add_page_numbers(&request, input.clone()) } else { watermark(&request, input.clone()) };
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
            paths: vec![input.clone()], text: String::new(), opacity: 100,
            position: Some("bottom-right".into()), logo_path: None, pages: None,
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
        let watermarked = watermark(&PageOverlayRequest {
            paths: vec![input.clone()], text: String::new(), opacity: 70,
            position: Some("bottom-right".into()), logo_path: Some(logo), pages: None,
            start_number: None, font_size: None, output_location: OutputLocation::CustomFolder(output),
        }, input.clone());
        assert!(watermarked.error().is_none(), "logo watermark failed: {:?}", watermarked.error());
        let watermarked_document = Document::load(watermarked.output_paths().first().unwrap()).unwrap();
        let watermarked_page = watermarked_document.get_pages().values().next().copied().unwrap();
        let watermarked_content = String::from_utf8_lossy(&watermarked_document.get_page_content(watermarked_page)).into_owned();
        assert!(watermarked_content.contains("40 0 0 30 186 180 cm"), "logo coordinates: {watermarked_content}");

        for output in numbered.output_paths().into_iter().chain(watermarked.output_paths()) { let _ = fs::remove_file(output); }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remaining_mutators_reject_unsafe_documents_before_reserving_output() {
        for kind in ["signed", "outline", "tagged"] {
            for operation in ["numbers", "watermark", "image-watermark", "compress", "remove", "extract"] {
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
                        paths: vec![input.clone()], text: "1".to_string(), opacity: 100,
                        position: None, logo_path: None, pages: None, start_number: None,
                        font_size: None, output_location: output_location.clone(),
                    }, input.clone()),
                    "watermark" => watermark(&PageOverlayRequest {
                        paths: vec![input.clone()], text: "TEST".to_string(), opacity: 70,
                        position: None, logo_path: None, pages: None, start_number: None,
                        font_size: None, output_location: output_location.clone(),
                    }, input.clone()),
                    "image-watermark" => watermark(&PageOverlayRequest {
                        paths: vec![input.clone()], text: String::new(), opacity: 70,
                        position: None, logo_path: Some(logo), pages: None, start_number: None,
                        font_size: None, output_location: output_location.clone(),
                    }, input.clone()),
                    "compress" => compress(&CompressPdfRequest {
                        paths: vec![input.clone()], quality: 80, output_location: output_location.clone(),
                    }, input.clone()),
                    "remove" => {
                        let mut request = request(None, vec![0]);
                        request.output_location = output_location.clone();
                        remove_pages(&request, input.clone())
                    }
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

    fn warning_kinds(outcome: &JobOutcome) -> Vec<WarningKind> {
        outcome.warnings.iter().map(|warning| warning.kind.clone()).collect()
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
                    rgb(dictionary! { "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>1, "Filter"=>"JBIG2Decode" },vec![0;8]),
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
            WarningKind::ImageUnsupported { page:2, image:2, feature:"CCITT fax".into() },
            WarningKind::ImageUnsupported { page:2, image:3, feature:"Lab colour".into() },
            WarningKind::ImageTooLarge { page:2, image:4, width:61000, height:48000 },
            WarningKind::ImageDamaged { page:2, image:5 },
        ]);
        assert_eq!(outcome.warnings.iter().map(|warning| warning.message.as_str()).collect::<Vec<_>>(),vec![
            "Page 1, image 2 — The image data is damaged. Not saved.",
            "Page 2, image 1 — JBIG2 isn't supported in this version. Not saved.",
            "Page 2, image 2 — CCITT fax isn't supported in this version. Not saved.",
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
        let jbig2=|| image_stream(dictionary! { "Width"=>1, "Height"=>1, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>1, "Filter"=>"JBIG2Decode" },vec![0;8]);
        let fax=temp.join("fax.pdf");
        image_pdf(&fax,|_| vec![vec![jbig2(),jbig2()],vec![jbig2()]]);
        let single=temp.join("single.pdf");
        image_pdf(&single,|_| vec![vec![image_stream(dictionary! { "Width"=>1, "Height"=>1, "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8, "Filter"=>"FlateDecode" },b"broken".to_vec())]]);

        let outcome=extract_images(&fax);
        let error=outcome.error().unwrap();
        assert_eq!((&error.kind,error.message.as_str()),(&crate::kit::contracts::ErrorKind::Processing,"None of the 3 images could be decoded, so nothing was saved."));
        assert!(outcome.output_paths().is_empty());
        assert_eq!(warning_kinds(&outcome),[(1,1),(1,2),(2,1)].map(|(page,image)| WarningKind::ImageUnsupported { page, image, feature:"JBIG2".into() }).to_vec());

        let outcome=extract_images(&single);
        assert_eq!(outcome.error().map(|error| error.message.as_str()),Some("The image couldn't be decoded, so nothing was saved."));
        assert_eq!(warning_kinds(&outcome),vec![WarningKind::ImageDamaged { page:1, image:1 }]);
        assert_eq!(sorted_names(&temp),vec!["fax.pdf","single.pdf"],"no images folder is made");
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
        let first = generated.join("source-split-0.pdf");
        let invalid = generated.join("unexpected.pdf");
        fs::write(&first, b"first split").unwrap();
        fs::write(&invalid, b"unexpected split").unwrap();

        let outcome = materialize_split_outputs(&input, &OutputLocation::CustomFolder(output_folder.clone()), &[first, invalid]).unwrap_err();

        assert!(outcome.error().is_some());
        assert_eq!(outcome.output_paths().len(), 1);
        assert!(outcome.output_paths()[0].is_file());
        assert_eq!(fs::read(&outcome.output_paths()[0]).unwrap(), b"first split");
        assert_eq!(fs::read_dir(&output_folder).unwrap().count(), 1);
        fs::remove_dir_all(temp).unwrap();
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

    fn run_split_with_sentinel(name: &str, split_mode: &str, page_ranges: Option<&str>, chunk_size: Option<usize>, sentinel_name: &str, expected_count: usize) {
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

        let outcome = split(&split_request(OutputLocation::CustomFolder(output_folder.clone()), split_mode, page_ranges, chunk_size), input.clone());

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
        run_split_with_sentinel("pages", "pages", None, None, "document-split-page-1.pdf", 3);
    }

    #[test]
    fn chunk_split_does_not_overwrite_existing_outputs() {
        run_split_with_sentinel("chunks", "chunks", None, Some(2), "document-split-1-2.pdf", 2);
    }

    #[test]
    fn range_split_does_not_overwrite_existing_outputs() {
        run_split_with_sentinel("ranges", "pages", Some("1,3"), None, "document-split-0.pdf", 2);
    }

    #[test]
    fn range_split_rejects_blank_ranges_without_outputs() {
        let input = PathBuf::from("blank-ranges.pdf");
        let outcome = split(&split_request(OutputLocation::AlongsideInput, "ranges", Some("  "), None), input.clone());

        assert_eq!(outcome.input_path, input);
        assert!(matches!(outcome.error().map(|error| &error.kind), Some(crate::kit::contracts::ErrorKind::InvalidInput)));
        assert!(outcome.output_paths().is_empty());
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

        let outcome = split(&split_request(OutputLocation::CustomFolder(output_folder.clone()), "pages", None, None), input.clone());

        assert_eq!(outcome.input_path, input);
        assert!(outcome.error().is_some());
        assert!(outcome.output_paths().is_empty());
        assert_eq!(fs::read(&sentinel).unwrap(), b"sentinel");
        assert_eq!(fs::read_dir(&output_folder).unwrap().count(), 1);
        assert_eq!(split_temp_entries(), temp_before, "split temporary directories were not cleaned after failure");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parses_ranges_in_document_order() {
        let selected = selected_page_indices(&request(Some("1-3, 7"), vec![]), 7).unwrap();
        assert_eq!(selected, vec![0, 1, 2, 6]);
    }

    #[test]
    fn rejects_invalid_or_out_of_bounds_ranges() {
        for input in ["", "1-3, nope", "0", "4"] {
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
            paths: vec![input.clone()], text: "1".to_string(), opacity: 100,
            position: Some("bottom-right".to_string()), logo_path: None, pages: None,
            start_number: None, font_size: None, output_location: OutputLocation::AlongsideInput,
        }, input.clone());
        assert!(numbered.error().is_none(), "page numbers failed: {:?}", numbered.error());

        let watermark = watermark(&PageOverlayRequest {
            paths: vec![input.clone()], text: "TEST".to_string(), opacity: 70,
            position: Some("center".to_string()), logo_path: None, pages: None,
            start_number: None, font_size: None, output_location: OutputLocation::AlongsideInput,
        }, input.clone());
        assert!(watermark.error().is_none(), "watermark failed: {:?}", watermark.error());

        let cropped = crate::kit::pdf::editor::crop(&crate::kit::pdf::editor::CropPdfRequest {
            paths: vec![input.clone()],
            rectangle: crate::kit::pdf::editor::PdfRect { x: 0.0, y: 0.0, width: 128.0, height: 128.0 },
            scope: crate::kit::pdf::editor::PageScope::All,
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
}

#[cfg(test)]
mod text_extraction_tests {
    use super::*;
    use crate::kit::contracts::ErrorKind;
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
            assert_eq!((&error.kind, error.message.as_str()), (&ErrorKind::InvalidInput, PDF_PASSWORD_PROTECTED));
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
