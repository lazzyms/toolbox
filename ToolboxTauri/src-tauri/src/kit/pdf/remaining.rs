use image::codecs::jpeg::JpegEncoder;
use image::DynamicImage;
use lopdf::{dictionary, Document, Object, Stream};
use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming, OutputReservation};
use crate::kit::contracts::ToolError;
use super::metadata::page_bounds;
use super::mutation_preflight;

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
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfToTextRequest {
    pub paths: Vec<PathBuf>,
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
    let output = match OutputNaming::reserve_destination(&first, &request.output_location, "-merged", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(first, format!("Could not reserve merged PDF output: {error}")),
    };
    let Some(qpdf) = qpdf() else { return failure(first, "qpdf is required to merge PDFs but was not found.".to_string()); };
    let result = Command::new(qpdf).arg("--empty").arg("--pages").args(&request.paths).arg("--").arg(output.path()).output();
    match result {
        Ok(result) if result.status.success() => match Document::load(output.path()) {
            Ok(document) if document.get_pages().len() == expected_pages => match output.publish() {
                Ok(path) => JobOutcome { input_path: first, output_paths: vec![path], detail: format!("{} PDFs merged in input order", request.paths.len()), failure: None },
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
    let Some(qpdf) = qpdf() else { return failure(input, "qpdf is required to split PDFs but was not found.".to_string()); };
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
            JobOutcome { input_path: input, output_paths: outputs, detail, failure: None }
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
        let result = Command::new(qpdf).arg(input).arg(format!("--split-pages={size}")).arg(&output).output().map_err(|error| format!("Could not run qpdf: {error}"))?;
        if !result.status.success() { return Err(stderr(result, "qpdf failed to split PDF chunks.")); }
        return collect_split_outputs(workspace.path(), "qpdf reported success but produced no chunk files.");
    }
    if let Some(raw) = request.page_ranges.as_deref().filter(|value| !value.trim().is_empty()) {
        let page_count = Document::load(input).map_err(|error| error.to_string())?.get_pages().len();
        let ranges = parse_split_ranges(raw, page_count)?;
        let mut outputs = Vec::with_capacity(ranges.len());
        for (number, (start, end)) in ranges.into_iter().enumerate() {
            let output = workspace.path().join(format!("{stem}-split-{number}.pdf"));
            let result = Command::new(qpdf).arg(input).arg("--pages").arg(input).arg(format!("{start}-{end}")).arg("--").arg(&output).output().map_err(|error| format!("Could not run qpdf: {error}"))?;
            if !result.status.success() { return Err(stderr(result, "qpdf failed to split the selected ranges.")); }
            if !output.is_file() { return Err("qpdf reported success but produced no range file.".to_string()); }
            outputs.push(output);
        }
        return Ok(outputs);
    }
    let pattern = workspace.path().join(format!("{stem}-split-page-%d.pdf"));
    let result = Command::new(qpdf).arg(input).arg("--split-pages").arg(&pattern).output().map_err(|error| format!("Could not run qpdf: {error}"))?;
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
    let output = command.arg(&input).arg(&prefix).output();
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
            .output();
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
    JobOutcome { input_path: input.to_path_buf(), output_paths: outputs, detail: "PDF rendered to images".to_string(), failure: None }
}

pub fn to_text(request: &PdfToTextRequest, input: PathBuf) -> JobOutcome {
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-text", "txt") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve PDF text output: {error}")),
    };
    let document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let pages = selected_pdf_pages(&document, request.pages.as_ref());
    let pages = match pages {
        Ok(pages) if !pages.is_empty() => pages,
        Ok(_) => return failure(input, "Select at least one PDF page.".to_string()),
        Err(error) => return failure(input, error),
    };
    match document.extract_text(&pages) {
        Ok(text) => {
            let text = normalize_pdf_text(&text);
            if text.is_empty() { return failure(input, "PDF contains no selectable text; scanned PDFs require OCR.".to_string()); }
            match fs::write(output.path(), text) {
                Ok(_) => match output.publish() {
                    Ok(path) => JobOutcome { input_path: input, output_paths: vec![path], detail: "PDF text extracted in page order".to_string(), failure: None },
                    Err(error) => failure(input, format!("Could not publish text output: {error}")),
                },
                Err(error) => failure(input, format!("Could not write text output: {error}")),
            }
        },
        Err(error) => failure(input, format!("Could not extract selectable PDF text: {error}")),
    }
}

fn normalize_pdf_text(text: &str) -> String {
    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
}

pub fn extract_images(request: &PdfToTextRequest, input: PathBuf) -> JobOutcome {
    let document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let selected = match selected_pdf_pages(&document, request.pages.as_ref()) {
        Ok(pages) => pages.into_iter().map(|page| page as usize).collect::<std::collections::HashSet<_>>(),
        Err(error) => return failure(input, error),
    };
    let mut reservations = Vec::new();
    for (page_number, page_id) in document.get_pages() {
        if request.pages.is_some() && !selected.contains(&(page_number as usize)) { continue; }
        let images = match document.get_page_images(page_id) { Ok(images) => images, Err(error) => return extraction_failure(&input, reservations, error.to_string()) };
        for (index, image) in images.iter().enumerate() {
            let Some(filters) = &image.filters else {
                return extraction_failure(&input, reservations, "PDF contains an embedded image with no supported filter; extraction stopped without a complete result.");
            };
            if filters.iter().any(|filter| filter != "DCTDecode") {
                return extraction_failure(&input, reservations, format!("PDF image on page {page_number} uses an unsupported filter; only original JPEG images can be extracted without recompression."));
            }
            let reservation = match OutputNaming::reserve_destination(&input, &request.output_location, &format!("-image-{page_number}-{index}"), "jpg") {
                Ok(reservation) => reservation,
                Err(error) => return extraction_failure(&input, reservations, format!("Could not reserve extracted image output: {error}")),
            };
            if let Err(error) = fs::write(reservation.path(), image.content) {
                return extraction_failure(&input, reservations, format!("Could not write extracted image: {error}"));
            }
            reservations.push(reservation);
        }
    }
    if reservations.is_empty() {
        return failure(input, "No embedded JPEG images were found. Non-JPEG PDF image filters are not extractable without recompression.".to_string());
    }
    publish_extracted_images(input, reservations)
}

fn publish_extracted_images(input: PathBuf, reservations: Vec<OutputReservation>) -> JobOutcome {
    let mut outputs = Vec::with_capacity(reservations.len());
    for reservation in reservations {
        match reservation.publish() {
            Ok(path) => outputs.push(path),
            Err(error) => return JobOutcome::failure_with_outputs(input, outputs, ToolError::processing(format!("Could not publish extracted image: {error}"))),
        }
    }
    JobOutcome { input_path: input, output_paths: outputs, detail: "Embedded JPEG images extracted without recompression".to_string(), failure: None }
}

fn extraction_failure(input: &Path, reservations: Vec<OutputReservation>, message: impl Into<String>) -> JobOutcome {
    drop(reservations);
    failure(input.to_path_buf(), message.into())
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
    let output = match OutputNaming::reserve_destination(&first, &request.output_location, "-combined", "pdf") {
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
            Ok(path) => JobOutcome { input_path: first, output_paths: vec![path], detail: "Images combined into PDF".to_string(), failure: None },
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
        let result = Command::new("sips").arg("-s").arg("format").arg("jpeg").arg(path).arg("--out").arg(&converted).output()
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
    overlay(request, input, "-numbered", |page_number, width, height| {
        let number = request.start_number.unwrap_or(1).saturating_add(page_number as u32).saturating_sub(1);
        let size = request.font_size.unwrap_or(12).clamp(8, 72);
        let (x, y) = number_position(request.position.as_deref(), width, height);
        format!("BT /Fnum {size} Tf {x} {y} Td ({number}) Tj ET")
    }, 100)
}

pub fn watermark(request: &PageOverlayRequest, input: PathBuf) -> JobOutcome {
    if let Some(path) = &request.logo_path { return watermark_image(request, input, path); }
    overlay(request, input, "-watermarked", |_, width, height| {
        let (x, y) = watermark_position(request.position.as_deref(), width, height);
        format!("BT /Fnum 48 Tf {x} {y} Td ({}) Tj ET", escape(&request.text))
    }, request.opacity.clamp(1, 100))
}

pub fn compress(request: &CompressPdfRequest, input: PathBuf) -> JobOutcome {
    let source = match load_existing_document_for_mutation(&input) { Ok(document) => document, Err(error) => return failure(input, error) };
    let page_count = source.get_pages().len();
    let Some(qpdf) = qpdf() else { return failure(input, "qpdf is required to compress PDFs but was not found. Set TOOLBOX_QPDF_PATH or add qpdf to PATH.".to_string()); };
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-compressed", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve compressed PDF output: {error}")),
    };
    let quality = request.quality.clamp(1, 100);
    let level = ((100_u16.saturating_sub(quality as u16) * 8) / 99 + 1).to_string();
    let result = Command::new(qpdf).arg("--object-streams=generate").arg("--stream-data=compress").arg("--recompress-flate").arg(format!("--compression-level={level}")).arg(&input).arg(output.path()).output();
    match result {
        Ok(result) if result.status.success() => match Document::load(output.path()) {
            Ok(verified) if verified.get_pages().len() == page_count => match output.publish() {
                Ok(path) => JobOutcome { input_path: input, output_paths: vec![path], detail: format!("PDF compressed at quality {quality}"), failure: None },
                Err(error) => failure(input, format!("Could not publish compressed PDF: {error}")),
            },
            Ok(_) => failure(input, "Compressed PDF changed its page count.".to_string()),
            Err(error) => failure(input, format!("Compressed PDF could not be verified: {error}")),
        },
        Ok(result) => failure(input, stderr(result, "qpdf failed to compress the PDF.")),
        Err(error) => failure(input, format!("Could not run qpdf: {error}")),
    }
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
            Ok(path) => JobOutcome { input_path: input, output_paths: vec![path], detail: detail.to_string(), failure: None },
            Err(error) => failure(input, format!("Could not publish PDF output: {error}")),
        },
        Err(error) => failure(input, format!("Save failed: {error}")),
    }
}

fn overlay<F>(request: &PageOverlayRequest, input: PathBuf, suffix: &str, content: F, opacity: u8) -> JobOutcome
where F: Fn(usize, f32, f32) -> String {
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
        let (left, bottom, right, top) = match page_bounds(&document, page_id) {
            Ok(bounds) => bounds,
            Err(error) => return failure(input, error),
        };
        let width = right - left;
        let height = top - bottom;
        if width <= 0.0 || height <= 0.0 { return failure(input, "PDF page has invalid dimensions".to_string()); }
        let page = match document.get_dictionary_mut(page_id) { Ok(page) => page, Err(error) => return failure(input, error.to_string()) };
        if !page.has(b"Resources") { page.set("Resources", Object::Dictionary(dictionary! {})); }
        let resources = match page.get_mut(b"Resources").and_then(Object::as_dict_mut) { Ok(resources) => resources, Err(error) => return failure(input, error.to_string()) };
        if !resources.has(b"Font") { resources.set("Font", Object::Dictionary(dictionary! {})); }
        if let Err(error) = resources.get_mut(b"Font").and_then(Object::as_dict_mut).map(|fonts| fonts.set("Fnum", font_id)) { return failure(input, error.to_string()); }
        resources.set("ExtGState", resources.get(b"ExtGState").cloned().unwrap_or_else(|_| Object::Dictionary(dictionary! {})));
        if let Err(error) = resources.get_mut(b"ExtGState").and_then(Object::as_dict_mut).map(|states| states.set("GSwm", opacity_id)) { return failure(input, error.to_string()); }
        if let Err(error) = document.add_page_contents(page_id, format!("q /GSwm gs {} Q", content(number + 1, width, height)).into_bytes()) { return failure(input, error.to_string()); }
    }
    match document.save(output.path()) {
        Ok(_) => match output.publish() {
            Ok(path) => JobOutcome { input_path: input, output_paths: vec![path], detail: "PDF saved".to_string(), failure: None },
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
    let (bytes, width, height) = match image_as_jpeg(logo) { Ok(value) => value, Err(error) => return failure(input, error) };
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-watermarked", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve watermarked PDF output: {error}")),
    };
    let image_id = document.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64, "Height" => height as i64, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode" }, bytes));
    let state_id = document.add_object(dictionary! { "Type" => "ExtGState", "ca" => request.opacity.clamp(1, 100) as f32 / 100.0, "CA" => request.opacity.clamp(1, 100) as f32 / 100.0 });
    for (number, page_id) in document.get_pages().values().copied().enumerate() {
        if request.pages.as_ref().is_some_and(|pages| !pages.contains(&number)) { continue; }
        let page = match document.get_dictionary_mut(page_id) { Ok(page) => page, Err(error) => return failure(input, error.to_string()) };
        if !page.has(b"Resources") { page.set("Resources", Object::Dictionary(dictionary! {})); }
        let resources = match page.get_mut(b"Resources").and_then(Object::as_dict_mut) { Ok(resources) => resources, Err(error) => return failure(input, error.to_string()) };
        resources.set("XObject", resources.get(b"XObject").cloned().unwrap_or_else(|_| Object::Dictionary(dictionary! {})));
        resources.set("ExtGState", resources.get(b"ExtGState").cloned().unwrap_or_else(|_| Object::Dictionary(dictionary! {})));
        resources.get_mut(b"XObject").and_then(Object::as_dict_mut).map(|objects| objects.set("Iwm", image_id)).map_err(|error| error.to_string()).ok();
        resources.get_mut(b"ExtGState").and_then(Object::as_dict_mut).map(|states| states.set("GSwm", state_id)).map_err(|error| error.to_string()).ok();
        let (x, y) = watermark_position(request.position.as_deref(), width as f32, height as f32);
        if let Err(error) = document.add_page_contents(page_id, format!("q /GSwm gs {} 0 0 {} {} {} cm /Iwm Do Q", width.min(180) as f32, height.min(100) as f32, x, y).into_bytes()) { return failure(input, error.to_string()); }
    }
    match document.save(output.path()) {
        Ok(_) => match output.publish() {
            Ok(path) => JobOutcome { input_path: input, output_paths: vec![path], detail: "PDF image watermark applied".to_string(), failure: None },
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
    mutation_preflight(&document, true, true)?;
    Ok(document)
}

fn escape(text: &str) -> String { text.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)") }
fn qpdf() -> Option<PathBuf> { std::env::var_os("TOOLBOX_QPDF_PATH").map(PathBuf::from).filter(|path| path.is_file()).or_else(|| Command::new("qpdf").arg("--version").output().ok().filter(|result| result.status.success()).map(|_| PathBuf::from("qpdf"))) }
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
    static RENDERER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
                assert!(outcome.failure.is_some(), "{kind} {operation}: {:?}", outcome.failure);
                assert!(outcome.output_paths.is_empty());
                assert_eq!(fs::read_dir(output).unwrap().count(), 0, "{kind} {operation}");
                fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn pdf_to_images_keeps_twelve_pages_in_numeric_order() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = RENDERER_TEST_LOCK.lock().unwrap();
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
        assert!(outcome.failure.is_none(), "numeric image export failed: {:?}", outcome.failure);
        assert_eq!(outcome.output_paths.len(), 12);
        for (index, output) in outcome.output_paths.iter().enumerate() {
            assert_eq!(output.file_stem().unwrap().to_str().unwrap(), format!("document-images-{}", index + 1));
            assert_eq!(fs::read_to_string(output).unwrap(), format!("page-{}", index + 1));
        }
        fs::remove_dir_all(root).unwrap();
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
        let outcome=extract_images(&PdfToTextRequest { paths:vec![input.clone()], pages:None, output_location:OutputLocation::AlongsideInput },input.clone());

        assert!(outcome.failure.is_some(),"expected unsupported image filter to fail");
        assert!(outcome.output_paths.is_empty());
        assert_eq!(fs::read_dir(&temp).unwrap().count(),1);
        assert!(!temp.join("source-image-1-0.jpg").exists());
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn extraction_failure_preserves_a_competing_replacement() {
        let temp=std::env::temp_dir().join(format!("toolbox_extract_failure_{}_{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&temp).unwrap();
        let input=temp.join("source.pdf");
        fs::write(&input,b"source").unwrap();
        let reservation=OutputNaming::reserve_destination(&input,&OutputLocation::AlongsideInput,"-image-1-0","jpg").unwrap();
        fs::write(reservation.path(),b"extracted bytes").unwrap();
        let destination=reservation.destination_path().to_path_buf();
        fs::write(&destination,b"competing replacement").unwrap();

        let outcome=extraction_failure(&input,vec![reservation],"extraction stopped");

        assert!(outcome.failure.is_some());
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

        let outcome = publish_extracted_images(input.clone(), vec![first, second]);

        assert!(outcome.failure.is_some());
        assert_eq!(outcome.output_paths, vec![first_destination.clone()]);
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

        assert!(outcome.failure.is_some());
        assert_eq!(outcome.output_paths.len(), 1);
        assert!(outcome.output_paths[0].is_file());
        assert_eq!(fs::read(&outcome.output_paths[0]).unwrap(), b"rendered image");
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

        assert!(outcome.failure.is_some());
        assert_eq!(outcome.output_paths.len(), 1);
        assert!(outcome.output_paths[0].is_file());
        assert_eq!(fs::read(&outcome.output_paths[0]).unwrap(), b"first split");
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
        if qpdf().is_none() {
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

        assert!(outcome.failure.is_none(), "split failed: {:?}", outcome.failure);
        assert_eq!(outcome.input_path, input);
        assert_split_outputs(&outcome.output_paths, expected_count);
        assert!(!outcome.output_paths.contains(&sentinel));
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
        assert!(matches!(outcome.failure.as_ref().map(|error| &error.kind), Some(crate::kit::contracts::ErrorKind::InvalidInput)));
        assert!(outcome.output_paths.is_empty());
    }

    #[test]
    fn split_failure_leaves_no_partial_destinations() {
        let _lock = SPLIT_TEST_LOCK.lock().unwrap();
        if qpdf().is_none() {
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
        assert!(outcome.failure.is_some());
        assert!(outcome.output_paths.is_empty());
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
        assert!(numbered.failure.is_none(), "page numbers failed: {:?}", numbered.failure);

        let watermark = watermark(&PageOverlayRequest {
            paths: vec![input.clone()], text: "TEST".to_string(), opacity: 70,
            position: Some("center".to_string()), logo_path: None, pages: None,
            start_number: None, font_size: None, output_location: OutputLocation::AlongsideInput,
        }, input.clone());
        assert!(watermark.failure.is_none(), "watermark failed: {:?}", watermark.failure);

        let cropped = crate::kit::pdf::editor::crop(&crate::kit::pdf::editor::CropPdfRequest {
            paths: vec![input.clone()],
            rectangle: crate::kit::pdf::editor::PdfRect { x: 0.0, y: 0.0, width: 128.0, height: 128.0 },
            scope: crate::kit::pdf::editor::PageScope::All,
            output_location: OutputLocation::AlongsideInput,
        }, input.clone());
        assert!(cropped.failure.is_none(), "crop failed: {:?}", cropped.failure);

        for output in numbered.output_paths.into_iter().chain(watermark.output_paths).chain(cropped.output_paths) {
            let _ = std::fs::remove_file(output);
        }
        let _ = std::fs::remove_file(input);
    }
}
