use lopdf::{dictionary, Document, Object};
use std::path::{Path, PathBuf};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageScope {
    All,
    Selected { pages: Vec<usize> },
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropPdfRequest {
    pub paths: Vec<PathBuf>,
    pub rectangle: PdfRect,
    pub scope: PageScope,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizePdfRequest {
    pub paths: Vec<PathBuf>,
    pub page_order: Vec<usize>,
    pub scope: PageScope,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignPdfRequest {
    pub paths: Vec<PathBuf>,
    pub page: usize,
    pub text: String,
    pub rectangle: PdfRect,
    pub output_location: OutputLocation,
}

pub fn crop(request: &CropPdfRequest, input: PathBuf) -> JobOutcome {
    transform_pdf(input, &request.output_location, "-cropped", |document, pages| {
        let selected = selected_pages(&request.scope, pages.len());
        validate_rect(&request.rectangle)?;
        for (index, page_id) in pages.iter().enumerate() {
            if selected(index) {
                let page = document.get_dictionary_mut(*page_id).map_err(|e| e.to_string())?;
                page.set("MediaBox", vec![
                    Object::Real(request.rectangle.x),
                    Object::Real(request.rectangle.y),
                    Object::Real(request.rectangle.x + request.rectangle.width),
                    Object::Real(request.rectangle.y + request.rectangle.height),
                ]);
            }
        }
        Ok(())
    })
}

pub fn organize(request: &OrganizePdfRequest, input: PathBuf) -> JobOutcome {
    transform_pdf(input, &request.output_location, "-organized", |document, pages| {
        let selected = selected_pages(&request.scope, pages.len());
        let mut order = pages.to_vec();
        let requested: Vec<_> = request.page_order.iter().copied().filter(|index| *index < pages.len()).collect();
        for (position, index) in requested.into_iter().enumerate() {
            if selected(index) && position < order.len() {
                order[position] = pages[index];
            }
        }
        let root = document.get_dictionary(pages[0]).map_err(|e| e.to_string())?.get(b"Parent").map_err(|e| e.to_string())?.as_reference().map_err(|e| e.to_string())?;
        document.get_dictionary_mut(root).map_err(|e| e.to_string())?.set("Kids", order.iter().map(|page| Object::Reference(*page)).collect::<Vec<_>>());
        Ok(())
    })
}

pub fn sign(request: &SignPdfRequest, input: PathBuf) -> JobOutcome {
    transform_pdf(input, &request.output_location, "-signed", |document, pages| {
        validate_rect(&request.rectangle)?;
        let Some(page_id) = pages.get(request.page).copied() else { return Err("Signature page is outside the document.".to_string()); };
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let stream = format!("BT /Fsig 24 Tf {} {} Td ({}) Tj ET", request.rectangle.x, request.rectangle.y, escape_text(&request.text));
        let stream_id = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, stream.into_bytes())));
        let page = document.get_dictionary_mut(page_id).map_err(|e| e.to_string())?;
        if !page.has(b"Resources") { page.set("Resources", Object::Dictionary(dictionary! {})); }
        let resources = page.get_mut(b"Resources").map_err(|e| e.to_string())?.as_dict_mut().map_err(|e| e.to_string())?;
        if !resources.has(b"Font") { resources.set("Font", Object::Dictionary(dictionary! {})); }
        resources.get_mut(b"Font").map_err(|e| e.to_string())?.as_dict_mut().map_err(|e| e.to_string())?.set("Fsig", font_id);
        let contents = page.get_mut(b"Contents");
        let Ok(contents) = contents else { page.set("Contents", Object::Reference(stream_id)); return Ok(()); };
        if let Object::Reference(_) = contents { return Ok(()); }
        let existing = contents.as_array().map_err(|e| e.to_string())?.to_vec();
        *contents = Object::Array(existing.into_iter().chain([Object::Reference(stream_id)]).collect());
        Ok(())
    })
}

fn transform_pdf<F>(input: PathBuf, location: &OutputLocation, suffix: &str, edit: F) -> JobOutcome
where F: FnOnce(&mut Document, &[lopdf::ObjectId]) -> Result<(), String> {
    let output = OutputNaming::get_destination(&input, location, suffix, "pdf");
    let mut document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let pages: Vec<_> = document.get_pages().values().copied().collect();
    if pages.is_empty() { return failure(input, "PDF has no pages".to_string()); }
    if let Err(error) = edit(&mut document, &pages) { return failure(input, error); }
    match document.save(&output) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: "PDF saved".to_string(), failure: None }, Err(error) => failure(input, format!("Save failed: {error}")) }
}

fn selected_pages(scope: &PageScope, count: usize) -> impl Fn(usize) -> bool + '_ {
    move |index| match scope { PageScope::All => true, PageScope::Selected { pages } => pages.contains(&index) && index < count }
}

fn validate_rect(rect: &PdfRect) -> Result<(), String> { if rect.width <= 0.0 || rect.height <= 0.0 { Err("Rectangle must have positive dimensions.".to_string()) } else { Ok(()) } }
fn escape_text(text: &str) -> String { text.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)") }
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome { input_path, output_paths: vec![], detail: String::new(), failure: Some(error) } }

#[allow(dead_code)]
fn _path(_: &Path) {}
