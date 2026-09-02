use lopdf::{dictionary, Document, Object};
use std::path::PathBuf;
use std::process::Command;

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};

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
    match result { Ok(result) if result.status.success() => JobOutcome { input_path: input, output_paths: vec![pattern], detail: "PDF split".to_string(), failure: None }, Ok(result) => failure(input, stderr(result, "qpdf failed to split the PDF.")), Err(error) => failure(input, format!("Could not run qpdf: {error}")) }
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
fn stderr(result: std::process::Output, fallback: &str) -> String { String::from_utf8_lossy(&result.stderr).trim().lines().last().filter(|line| !line.is_empty()).unwrap_or(fallback).to_string() }
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome { input_path, output_paths: vec![], detail: String::new(), failure: Some(error) } }
