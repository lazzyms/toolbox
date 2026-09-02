use image::codecs::jpeg::JpegEncoder;
use image::ImageEncoder;
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
    pub delete_pages: Vec<usize>,
    pub rotate_pages: Vec<RotatePage>,
    pub scope: PageScope,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotatePage {
    pub page: usize,
    pub degrees: i32,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignPdfRequest {
    pub paths: Vec<PathBuf>,
    pub page: usize,
    pub text: String,
    pub signature_path: Option<PathBuf>,
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
        let requested: Vec<_> = request.page_order.iter().copied().filter(|index| *index < pages.len() && selected(*index) && !request.delete_pages.contains(index)).collect();
        let mut order = requested.into_iter().map(|index| pages[index]).collect::<Vec<_>>();
        for (index, page_id) in pages.iter().enumerate() {
            if selected(index) && !request.delete_pages.contains(&index) && !request.page_order.contains(&index) { order.push(*page_id); }
        }
        let root = document.get_dictionary(pages[0]).map_err(|e| e.to_string())?.get(b"Parent").map_err(|e| e.to_string())?.as_reference().map_err(|e| e.to_string())?;
        for operation in &request.rotate_pages {
            if let Some(page_id) = pages.get(operation.page).copied() {
                let page = document.get_dictionary_mut(page_id).map_err(|e| e.to_string())?;
                let degrees = operation.degrees.rem_euclid(360);
                page.set("Rotate", degrees as i64);
            }
        }
        if order.is_empty() { return Err("The organize plan must keep at least one page.".to_string()); }
        let root = document.get_dictionary_mut(root).map_err(|e| e.to_string())?;
        root.set("Kids", order.iter().map(|page| Object::Reference(*page)).collect::<Vec<_>>());
        root.set("Count", order.len() as i64);
        Ok(())
    })
}

pub fn sign(request: &SignPdfRequest, input: PathBuf) -> JobOutcome {
    transform_pdf(input, &request.output_location, "-signed", |document, pages| {
        validate_rect(&request.rectangle)?;
        let Some(page_id) = pages.get(request.page).copied() else { return Err("Signature page is outside the document.".to_string()); };
        let stream = format!("BT /Fsig 24 Tf {} {} Td ({}) Tj ET", request.rectangle.x, request.rectangle.y, escape_text(&request.text));
        let (stream_id, resource_id, resource_name) = if let Some(path) = &request.signature_path {
            let (bytes, width, height) = signature_jpeg(path)?;
            let image_id = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64,
                "Height" => height as i64, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
                "Filter" => "DCTDecode",
            }, bytes)));
            let content = format!("q {} 0 0 {} {} {} cm /Isig Do Q", request.rectangle.width, request.rectangle.height, request.rectangle.x, request.rectangle.y);
            (document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, content.into_bytes()))), image_id, "XObject")
        } else {
            let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
            let content_id = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, stream.into_bytes())));
            (content_id, font_id, "Font")
        };
        let page = document.get_dictionary_mut(page_id).map_err(|e| e.to_string())?;
        if !page.has(b"Resources") { page.set("Resources", Object::Dictionary(dictionary! {})); }
        let resources = page.get_mut(b"Resources").map_err(|e| e.to_string())?.as_dict_mut().map_err(|e| e.to_string())?;
        if resource_name == "Font" {
            resources.set("Font", resources.get(b"Font").cloned().unwrap_or_else(|_| Object::Dictionary(dictionary! {})));
            resources.get_mut(b"Font").map_err(|e| e.to_string())?.as_dict_mut().map_err(|e| e.to_string())?.set("Fsig", resource_id);
        } else {
            resources.set("XObject", resources.get(b"XObject").cloned().unwrap_or_else(|_| Object::Dictionary(dictionary! {})));
            resources.get_mut(b"XObject").map_err(|e| e.to_string())?.as_dict_mut().map_err(|e| e.to_string())?.set("Isig", resource_id);
        }
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
fn signature_jpeg(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let image = image::open(path).map_err(|error| format!("Could not read signature image: {error}"))?;
    let rgb = image.to_rgb8();
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, 95).write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8).map_err(|error| format!("Could not encode signature image: {error}"))?;
    Ok((bytes, rgb.width(), rgb.height()))
}
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome { input_path, output_paths: vec![], detail: String::new(), failure: Some(error) } }

#[allow(dead_code)]
fn _path(_: &Path) {}
