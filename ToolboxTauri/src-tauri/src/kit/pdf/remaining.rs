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
    let output = OutputNaming::get_destination(&first, &request.output_location, "-merged", "pdf");
    let Some(qpdf) = qpdf() else { return failure(first, "qpdf is required to merge PDFs but was not found.".to_string()); };
    let result = Command::new(qpdf).arg("--empty").arg("--pages").args(&request.paths).arg("--").arg(&output).output();
    match result { Ok(result) if result.status.success() => JobOutcome { input_path: first, output_paths: vec![output], detail: "PDFs merged".to_string(), failure: None }, Ok(result) => failure(first, stderr(result, "qpdf failed to merge the PDFs.")), Err(error) => failure(first, format!("Could not run qpdf: {error}")) }
}

pub fn split(input: PathBuf, location: &OutputLocation) -> JobOutcome {
    let output = OutputNaming::get_destination(&input, location, "-split", "pdf");
    let Some(qpdf) = qpdf() else { return failure(input, "qpdf is required to split PDFs but was not found.".to_string()); };
    let pattern = output.with_file_name(format!("{}-page-%d.pdf", output.file_stem().and_then(|stem| stem.to_str()).unwrap_or("output")));
    let result = Command::new(qpdf).arg(&input).arg("--split-pages").arg(&pattern).output();
    match result {
        Ok(result) if result.status.success() => {
            let mut outputs = fs::read_dir(pattern.parent().unwrap_or_else(|| std::path::Path::new(".")))
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.file_name().and_then(|name| name.to_str()).map(|name| name.starts_with(pattern.file_stem().and_then(|stem| stem.to_str()).unwrap_or("")) && path.extension().is_some_and(|ext| ext == "pdf")).unwrap_or(false))
                .collect::<Vec<_>>();
            outputs.sort();
            if outputs.is_empty() { failure(input, "qpdf reported success but produced no split files.".to_string()) }
            else { JobOutcome { input_path: input, output_paths: outputs, detail: "PDF split".to_string(), failure: None } }
        }
        Ok(result) => failure(input, stderr(result, "qpdf failed to split the PDF.")),
        Err(error) => failure(input, format!("Could not run qpdf: {error}")),
    }
}

pub fn to_images(request: &PdfToImagesRequest, input: PathBuf) -> JobOutcome {
    let format = request.format.to_lowercase();
    if format != "jpg" && format != "png" { return failure(input, "PDF images must be JPEG or PNG.".to_string()); }
    let Some(renderer) = tool("TOOLBOX_PDFTOPPM_PATH", "pdftoppm") else { return failure(input, "pdftoppm is required to render PDFs. Set TOOLBOX_PDFTOPPM_PATH or add pdftoppm to PATH.".to_string()); };
    let dpi = request.dpi.clamp(72, 300);
    let extension = if format == "jpg" { "jpg" } else { "png" };
    let destination = OutputNaming::get_destination(&input, &request.output_location, "-images", extension);
    let prefix = destination.with_extension("");
    let output = Command::new(renderer).arg(format!("-{format}")).arg("-r").arg(dpi.to_string()).arg(&input).arg(&prefix).output();
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
        Ok(text) => match fs::write(&output, text) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: "PDF text extracted".to_string(), failure: None }, Err(error) => failure(input, format!("Could not write text output: {error}")) },
        Err(error) => failure(input, format!("Could not extract selectable PDF text: {error}")),
    }
}

pub fn extract_images(request: &PdfToTextRequest, input: PathBuf) -> JobOutcome {
    let document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let directory = match &request.output_location { OutputLocation::AlongsideInput => input.parent().unwrap_or_else(|| std::path::Path::new(".")), OutputLocation::CustomFolder(folder) => folder.as_path() };
    let stem = input.file_stem().and_then(|value| value.to_str()).unwrap_or("output");
    let mut outputs = Vec::new();
    for (page_number, page_id) in document.get_pages() {
        let images = match document.get_page_images(page_id) { Ok(images) => images, Err(error) => return failure(input, error.to_string()) };
        for (index, image) in images.iter().enumerate() {
            let Some(filters) = &image.filters else { continue; };
            if filters.iter().any(|filter| filter != "DCTDecode") { continue; }
            let mut path = directory.join(format!("{stem}-image-{page_number}-{index}.jpg"));
            let mut counter = 1;
            while path.exists() { path = directory.join(format!("{stem}-image-{page_number}-{index}-{counter}.jpg")); counter += 1; }
            if let Err(error) = fs::write(&path, image.content) { return failure(input, format!("Could not write extracted image: {error}")); }
            outputs.push(path);
        }
    }
    if outputs.is_empty() { failure(input, "No embedded JPEG images were found. Non-JPEG PDF image filters are not extractable without recompression.".to_string()) } else { JobOutcome { input_path: input, output_paths: outputs, detail: "PDF images extracted".to_string(), failure: None } }
}

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
    let image = image::open(path).map_err(|error| format!("Could not decode image: {error}"))?;
    let (width, height) = (image.width(), image.height());
    let mut output = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut output, 92).encode_image(&DynamicImage::ImageRgb8(image.to_rgb8())).map_err(|error| format!("Could not encode image as JPEG: {error}"))?;
    Ok((output.into_inner(), width, height))
}

pub fn add_page_numbers(request: &PageOverlayRequest, input: PathBuf) -> JobOutcome {
    overlay(request, input, "-numbered", |page_number, width, height| {
        format!("BT /Fnum 12 Tf {} {} Td ({}) Tj ET", width - 50.0, 24.0_f32.min(height / 2.0), page_number)
    })
}

pub fn watermark(request: &PageOverlayRequest, input: PathBuf) -> JobOutcome {
    overlay(request, input, "-watermarked", |_, width, height| {
        format!("BT /Fnum 48 Tf {} {} Td ({}) Tj ET", width / 4.0, height / 2.0, escape(&request.text))
    })
}

pub fn compress(request: &CompressPdfRequest, input: PathBuf) -> JobOutcome {
    let output = OutputNaming::get_destination(&input, &request.output_location, "-compressed", "pdf");
    let mut document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let _quality = request.quality.clamp(1, 100);
    document.compress();
    match document.save(&output) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: "PDF compressed".to_string(), failure: None }, Err(error) => failure(input, format!("Save failed: {error}")) }
}

pub fn remove_pages(request: &PageSelectionRequest, input: PathBuf) -> JobOutcome {
    let output = OutputNaming::get_destination(&input, &request.output_location, "-pages-removed", "pdf");
    let mut document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let pages = document.get_pages();
    let delete = request.pages.iter().filter_map(|page| pages.get(&(*page as u32 + 1)).map(|_| *page as u32 + 1)).collect::<Vec<_>>();
    if delete.len() >= pages.len() { return failure(input, "The output must keep at least one page.".to_string()); }
    document.delete_pages(&delete);
    save(document, input, output, "PDF pages removed")
}

pub fn extract_pages(request: &PageSelectionRequest, input: PathBuf) -> JobOutcome {
    let output = OutputNaming::get_destination(&input, &request.output_location, "-extracted", "pdf");
    let mut document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let pages = document.get_pages();
    let keep = request.pages.iter().copied().filter(|page| *page < pages.len()).collect::<std::collections::BTreeSet<_>>();
    let delete = (0..pages.len()).filter(|page| !keep.contains(page)).map(|page| page as u32 + 1).collect::<Vec<_>>();
    if keep.is_empty() { return failure(input, "Select at least one page.".to_string()); }
    document.delete_pages(&delete);
    save(document, input, output, "PDF pages extracted")
}

fn save(mut document: Document, input: PathBuf, output: PathBuf, detail: &str) -> JobOutcome {
    match document.save(&output) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: detail.to_string(), failure: None }, Err(error) => failure(input, format!("Save failed: {error}")) }
}

fn overlay<F>(request: &PageOverlayRequest, input: PathBuf, suffix: &str, content: F) -> JobOutcome
where F: Fn(usize, f32, f32) -> String {
    let output = OutputNaming::get_destination(&input, &request.output_location, suffix, "pdf");
    let mut document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    for (number, page_id) in document.get_pages().values().copied().enumerate() {
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
        if let Err(error) = document.add_page_contents(page_id, content(number + 1, width, height).into_bytes()) { return failure(input, error.to_string()); }
    }
    match document.save(&output) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: "PDF saved".to_string(), failure: None }, Err(error) => failure(input, format!("Save failed: {error}")) }
}

fn escape(text: &str) -> String { text.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)") }
fn qpdf() -> Option<PathBuf> { std::env::var_os("TOOLBOX_QPDF_PATH").map(PathBuf::from).filter(|path| path.is_file()).or_else(|| Command::new("qpdf").arg("--version").output().ok().filter(|result| result.status.success()).map(|_| PathBuf::from("qpdf"))) }
fn tool(variable: &str, command: &str) -> Option<PathBuf> { std::env::var_os(variable).map(PathBuf::from).filter(|path| path.is_file()).or_else(|| Command::new(command).arg("-h").output().ok().map(|_| PathBuf::from(command))) }
fn stderr(result: std::process::Output, fallback: &str) -> String { String::from_utf8_lossy(&result.stderr).trim().lines().last().filter(|line| !line.is_empty()).unwrap_or(fallback).to_string() }
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::processing(error)) }
