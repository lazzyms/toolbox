use lopdf::{dictionary, Document, Object};
use std::path::PathBuf;

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
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome { input_path, output_paths: vec![], detail: String::new(), failure: Some(error) } }
