use image::codecs::jpeg::JpegEncoder;
use image::DynamicImage;
use lopdf::{dictionary, Document, Object, Stream};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::ToolError;

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
    pub output_location: OutputLocation,
}

pub fn merge(request: &MergePdfRequest) -> JobOutcome {
    let Some(first) = request.paths.first().cloned() else { return failure(PathBuf::new(), "Select at least one PDF.".to_string()); };
    if request.paths.len() < 2 { return failure(first, "Select at least two PDFs to merge.".to_string()); }
    let mut expected_pages = 0usize;
    for path in &request.paths {
        if path.extension().and_then(|extension| extension.to_str()).is_none_or(|extension| !extension.eq_ignore_ascii_case("pdf")) { return failure(first, format!("Only PDF inputs can be merged: {}", path.display())); }
        let document = match Document::load(path) { Ok(document) => document, Err(error) => return failure(first, format!("Could not read {}: {error}", path.display())) };
        expected_pages += document.get_pages().len();
    }
    let output = OutputNaming::get_destination(&first, &request.output_location, "-merged", "pdf");
    let Some(qpdf) = qpdf() else { return failure(first, "qpdf is required to merge PDFs but was not found.".to_string()); };
    let result = Command::new(qpdf).arg("--empty").arg("--pages").args(&request.paths).arg("--").arg(&output).output();
    match result {
        Ok(result) if result.status.success() => match Document::load(&output) {
            Ok(document) if document.get_pages().len() == expected_pages => JobOutcome { input_path: first, output_paths: vec![output], detail: format!("{} PDFs merged in input order", request.paths.len()), failure: None },
            Ok(document) => { let _ = fs::remove_file(&output); failure(first, format!("Merged PDF page count mismatch: expected {expected_pages}, got {}.", document.get_pages().len())) },
            Err(error) => { let _ = fs::remove_file(&output); failure(first, format!("Merged PDF could not be verified: {error}")) },
        },
        Ok(result) => { let _ = fs::remove_file(&output); failure(first, stderr(result, "qpdf failed to merge the PDFs.")) },
        Err(error) => failure(first, format!("Could not run qpdf: {error}")),
    }
}

pub fn split(request: &PageSelectionRequest, input: PathBuf) -> JobOutcome {
    let location = &request.output_location;
    let output = OutputNaming::get_destination(&input, location, "-split", "pdf");
    let Some(qpdf) = qpdf() else { return failure(input, "qpdf is required to split PDFs but was not found.".to_string()); };
    if request.split_mode.as_deref() == Some("chunks") {
        let Some(size) = request.chunk_size.filter(|size| *size > 0) else { return failure(input, "Chunk size must be greater than zero.".to_string()); };
        let result = Command::new(&qpdf).arg(&input).arg(format!("--split-pages={size}" )).arg(&output).output();
        return match result { Ok(result) if result.status.success() => split_outputs(&input, output, "page"), Ok(result) => failure(input, stderr(result, "qpdf failed to split PDF chunks.")), Err(error) => failure(input, format!("Could not run qpdf: {error}")) };
    }
    if let Some(raw) = request.page_ranges.as_deref().filter(|value| !value.trim().is_empty()) {
        let page_count = match Document::load(&input) { Ok(document) => document.get_pages().len(), Err(error) => return failure(input, error.to_string()) };
        let ranges = match parse_split_ranges(raw, page_count) { Ok(ranges) => ranges, Err(error) => return failure(input, error) };
        let stem = output.file_stem().and_then(|value| value.to_str()).unwrap_or("split");
        let mut outputs = Vec::new();
        for (number, (start, end)) in ranges.into_iter().enumerate() {
            let target = OutputNaming::get_destination(&input, location, &format!("-split-{number}"), "pdf");
            let result = Command::new(&qpdf).arg(&input).arg("--pages").arg(&input).arg(format!("{start}-{end}")).arg("--").arg(&target).output();
            match result { Ok(result) if result.status.success() => outputs.push(target), Ok(result) => { remove_outputs(&outputs); return failure(input, stderr(result, "qpdf failed to split the selected ranges.")); }, Err(error) => { remove_outputs(&outputs); return failure(input, format!("Could not run qpdf: {error}")); } }
        }
        return JobOutcome { input_path: input, output_paths: outputs, detail: format!("PDF split into selected ranges from {stem}"), failure: None };
    }
    let pattern = output.with_file_name(format!("{}-page-%d.pdf", output.file_stem().and_then(|stem| stem.to_str()).unwrap_or("output")));
    let result = Command::new(qpdf).arg(&input).arg("--split-pages").arg(&pattern).output();
    match result {
        Ok(result) if result.status.success() => {
            let prefix = pattern.file_stem().and_then(|stem| stem.to_str()).unwrap_or("output").replace("%d", "");
            let mut outputs = fs::read_dir(pattern.parent().unwrap_or_else(|| std::path::Path::new(".")))
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.file_name().and_then(|name| name.to_str()).map(|name| name.starts_with(&prefix) && path.extension().is_some_and(|ext| ext == "pdf")).unwrap_or(false))
                .collect::<Vec<_>>();
            outputs.sort();
            if outputs.is_empty() { failure(input, "qpdf reported success but produced no split files.".to_string()) }
            else { JobOutcome { input_path: input, output_paths: outputs, detail: "PDF split".to_string(), failure: None } }
        }
        Ok(result) => failure(input, stderr(result, "qpdf failed to split the PDF.")),
        Err(error) => failure(input, format!("Could not run qpdf: {error}")),
    }
}

fn split_outputs(input: &PathBuf, output: PathBuf, kind: &str) -> JobOutcome {
    let prefix = output.file_stem().and_then(|stem| stem.to_str()).unwrap_or("split");
    let mut outputs = fs::read_dir(output.parent().unwrap_or_else(|| std::path::Path::new("."))).ok().into_iter().flatten().filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(prefix) && path.extension().is_some_and(|ext| ext == "pdf"))).collect::<Vec<_>>();
    outputs.sort();
    if outputs.is_empty() { failure(input.clone(), format!("qpdf reported success but produced no {kind} files.")) } else { JobOutcome { input_path: input.clone(), output_paths: outputs, detail: "PDF split".to_string(), failure: None } }
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
    let destination = OutputNaming::get_destination(&input, &request.output_location, "-images", extension);
    let prefix = destination.with_extension("");
    let mut command = Command::new(renderer);
    let renderer_format = if format == "jpg" { "jpeg" } else { "png" };
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
            outputs.sort();
            if outputs.is_empty() { failure(input, "PDF renderer produced no images.".to_string()) } else { JobOutcome { input_path: input, output_paths: outputs, detail: "PDF rendered to images".to_string(), failure: None } }
        }
        Ok(result) => failure(input, stderr(result, "pdftoppm failed to render the PDF.")),
        Err(error) => failure(input, format!("Could not run pdftoppm: {error}")),
    }
}

pub fn to_text(request: &PdfToTextRequest, input: PathBuf) -> JobOutcome {
    let output = OutputNaming::get_destination(&input, &request.output_location, "-text", "txt");
    let document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let pages = document.get_pages().keys().copied().collect::<Vec<_>>();
    match document.extract_text(&pages) {
        Ok(text) => {
            let text = normalize_pdf_text(&text);
            if text.is_empty() { return failure(input, "PDF contains no selectable text; scanned PDFs require OCR.".to_string()); }
            match fs::write(&output, text) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: "PDF text extracted in page order".to_string(), failure: None }, Err(error) => failure(input, format!("Could not write text output: {error}")) }
        },
        Err(error) => failure(input, format!("Could not extract selectable PDF text: {error}")),
    }
}

fn normalize_pdf_text(text: &str) -> String {
    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
}

pub fn extract_images(request: &PdfToTextRequest, input: PathBuf) -> JobOutcome {
    let document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let directory = match &request.output_location { OutputLocation::AlongsideInput => input.parent().unwrap_or_else(|| std::path::Path::new(".")), OutputLocation::CustomFolder(folder) => folder.as_path() };
    let stem = input.file_stem().and_then(|value| value.to_str()).unwrap_or("output");
    let mut outputs = Vec::new();
    for (page_number, page_id) in document.get_pages() {
        let images = match document.get_page_images(page_id) { Ok(images) => images, Err(error) => return failure(input, error.to_string()) };
        for (index, image) in images.iter().enumerate() {
            let Some(filters) = &image.filters else {
                remove_outputs(&outputs);
                return failure(input, "PDF contains an embedded image with no supported filter; extraction stopped without a complete result.".to_string());
            };
            if filters.iter().any(|filter| filter != "DCTDecode") {
                remove_outputs(&outputs);
                return failure(input, format!("PDF image on page {page_number} uses an unsupported filter; only original JPEG images can be extracted without recompression."));
            }
            let mut path = directory.join(format!("{stem}-image-{page_number}-{index}.jpg"));
            let mut counter = 1;
            while path.exists() { path = directory.join(format!("{stem}-image-{page_number}-{index}-{counter}.jpg")); counter += 1; }
            if let Err(error) = fs::write(&path, image.content) {
                remove_outputs(&outputs);
                return failure(input, format!("Could not write extracted image: {error}"));
            }
            outputs.push(path);
        }
    }
    if outputs.is_empty() { failure(input, "No embedded JPEG images were found. Non-JPEG PDF image filters are not extractable without recompression.".to_string()) } else { JobOutcome { input_path: input, output_paths: outputs, detail: "Embedded JPEG images extracted without recompression".to_string(), failure: None } }
}

fn remove_outputs(outputs: &[PathBuf]) { for output in outputs { let _ = fs::remove_file(output); } }

pub fn images_to_pdf(request: &ImagesToPdfRequest) -> JobOutcome {
    let Some(first) = request.paths.first().cloned() else { return failure(PathBuf::new(), "Select at least one image.".to_string()); };
    let output = OutputNaming::get_destination(&first, &request.output_location, "-combined", "pdf");
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let mut kids = Vec::new();
    for path in &request.paths {
        let (jpeg, width, height) = match image_as_jpeg(path) { Ok(value) => value, Err(error) => return failure(first, error) };
        let image_id = document.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64, "Height" => height as i64, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode" }, jpeg));
        let content_id = document.add_object(Stream::new(dictionary! {}, format!("q {width} 0 0 {height} 0 0 cm /Im0 Do Q").into_bytes()));
        let page_id = document.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), (width as f32).into(), (height as f32).into()], "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } }, "Contents" => content_id });
        kids.push(page_id.into());
    }
    document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => request.paths.len() as i64 }.into());
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    match document.save(&output) { Ok(_) => JobOutcome { input_path: first, output_paths: vec![output], detail: "Images combined into PDF".to_string(), failure: None }, Err(error) => failure(first, format!("Save failed: {error}")) }
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
    let output = OutputNaming::get_destination(&input, &request.output_location, "-compressed", "pdf");
    let source = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let page_count = source.get_pages().len();
    let Some(qpdf) = qpdf() else { return failure(input, "qpdf is required to compress PDFs but was not found. Set TOOLBOX_QPDF_PATH or add qpdf to PATH.".to_string()); };
    let quality = request.quality.clamp(1, 100);
    let level = ((100_u16.saturating_sub(quality as u16) * 8) / 99 + 1).to_string();
    let result = Command::new(qpdf).arg("--object-streams=generate").arg("--stream-data=compress").arg("--recompress-flate").arg(format!("--compression-level={level}")).arg(&input).arg(&output).output();
    match result {
        Ok(result) if result.status.success() => match Document::load(&output) {
            Ok(verified) if verified.get_pages().len() == page_count => JobOutcome { input_path: input, output_paths: vec![output], detail: format!("PDF compressed at quality {quality}"), failure: None },
            Ok(_) => { let _ = fs::remove_file(&output); failure(input, "Compressed PDF changed its page count.".to_string()) },
            Err(error) => { let _ = fs::remove_file(&output); failure(input, format!("Compressed PDF could not be verified: {error}")) },
        },
        Ok(result) => { let _ = fs::remove_file(&output); failure(input, stderr(result, "qpdf failed to compress the PDF.")) },
        Err(error) => { let _ = fs::remove_file(&output); failure(input, format!("Could not run qpdf: {error}")) },
    }
}

pub fn remove_pages(request: &PageSelectionRequest, input: PathBuf) -> JobOutcome {
    let output = OutputNaming::get_destination(&input, &request.output_location, "-pages-removed", "pdf");
    let mut document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let pages = document.get_pages();
    let selected = match selected_page_indices(request, pages.len()) { Ok(selected) => selected, Err(error) => return failure(input, error) };
    let delete = selected.into_iter().map(|page| page as u32 + 1).collect::<Vec<_>>();
    if delete.len() >= pages.len() { return failure(input, "The output must keep at least one page.".to_string()); }
    document.delete_pages(&delete);
    save(document, input, output, "PDF pages removed")
}

pub fn extract_pages(request: &PageSelectionRequest, input: PathBuf) -> JobOutcome {
    let output = OutputNaming::get_destination(&input, &request.output_location, "-extracted", "pdf");
    let mut document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let pages = document.get_pages();
    let keep = match selected_page_indices(request, pages.len()) { Ok(selected) => selected.into_iter().collect::<std::collections::BTreeSet<_>>(), Err(error) => return failure(input, error) };
    let delete = (0..pages.len()).filter(|page| !keep.contains(page)).map(|page| page as u32 + 1).collect::<Vec<_>>();
    if keep.is_empty() { return failure(input, "Select at least one page.".to_string()); }
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

fn save(mut document: Document, input: PathBuf, output: PathBuf, detail: &str) -> JobOutcome {
    match document.save(&output) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: detail.to_string(), failure: None }, Err(error) => failure(input, format!("Save failed: {error}")) }
}

fn overlay<F>(request: &PageOverlayRequest, input: PathBuf, suffix: &str, content: F, opacity: u8) -> JobOutcome
where F: Fn(usize, f32, f32) -> String {
    let output = OutputNaming::get_destination(&input, &request.output_location, suffix, "pdf");
    let mut document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    if let Err(error) = validate_overlay_scope(request.pages.as_deref(), document.get_pages().len()) { return failure(input, error); }
    let opacity_id = document.add_object(dictionary! { "Type" => "ExtGState", "ca" => opacity as f32 / 100.0, "CA" => opacity as f32 / 100.0 });
    for (number, page_id) in document.get_pages().values().copied().enumerate() {
        if request.pages.as_ref().is_some_and(|pages| !pages.contains(&number)) { continue; }
        let Ok(page) = document.get_dictionary(page_id) else { return failure(input, "Could not read PDF page".to_string()); };
        let Ok(media_box) = page.get(b"MediaBox").and_then(Object::as_array) else { return failure(input, "PDF page has no media box".to_string()); };
        let Ok(left) = media_box[0].as_f32() else { return failure(input, "PDF page left edge is invalid".to_string()); };
        let Ok(bottom) = media_box[1].as_f32() else { return failure(input, "PDF page bottom edge is invalid".to_string()); };
        let Ok(right) = media_box[2].as_f32() else { return failure(input, "PDF page right edge is invalid".to_string()); };
        let Ok(top) = media_box[3].as_f32() else { return failure(input, "PDF page top edge is invalid".to_string()); };
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
    match document.save(&output) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: "PDF saved".to_string(), failure: None }, Err(error) => failure(input, format!("Save failed: {error}")) }
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
    let output = OutputNaming::get_destination(&input, &request.output_location, "-watermarked", "pdf");
    let mut document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    if let Err(error) = validate_overlay_scope(request.pages.as_deref(), document.get_pages().len()) { return failure(input, error); }
    let (bytes, width, height) = match image_as_jpeg(logo) { Ok(value) => value, Err(error) => return failure(input, error) };
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
    match document.save(&output) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: "PDF image watermark applied".to_string(), failure: None }, Err(error) => failure(input, format!("Save failed: {error}")) }
}

fn validate_overlay_scope(pages: Option<&[usize]>, page_count: usize) -> Result<(), String> {
    if let Some(pages) = pages {
        if pages.is_empty() { return Err("Select at least one page for the watermark.".to_string()); }
        if pages.iter().any(|page| *page >= page_count) { return Err("A watermark page is outside the document.".to_string()); }
    }
    Ok(())
}

fn escape(text: &str) -> String { text.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)") }
fn qpdf() -> Option<PathBuf> { std::env::var_os("TOOLBOX_QPDF_PATH").map(PathBuf::from).filter(|path| path.is_file()).or_else(|| Command::new("qpdf").arg("--version").output().ok().filter(|result| result.status.success()).map(|_| PathBuf::from("qpdf"))) }
fn tool(variable: &str, command: &str) -> Option<PathBuf> {
    std::env::var_os(variable).map(PathBuf::from).filter(|path| path.is_file())
        .or_else(|| crate::kit::resources::application_resource_root().and_then(|root| [root.join("pdf-bin").join(command), root.join("resources").join(command), root.join(command)].into_iter().find(|path| path.is_file())))
        .or_else(|| Command::new(command).arg("-h").output().ok().map(|_| PathBuf::from(command)))
}
fn stderr(result: std::process::Output, fallback: &str) -> String { String::from_utf8_lossy(&result.stderr).trim().lines().last().filter(|line| !line.is_empty()).unwrap_or(fallback).to_string() }
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::processing(error)) }

#[cfg(test)]
mod tests {
    use super::*;

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
}
