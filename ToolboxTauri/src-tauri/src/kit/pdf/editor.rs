use image::codecs::jpeg::JpegEncoder;
use image::ImageEncoder;
use lopdf::{dictionary, Document, Object};
use std::path::{Path, PathBuf};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::ToolError;
use super::metadata::page_bounds;

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
pub struct AddPdfPagesRequest {
    pub paths: Vec<PathBuf>,
    #[serde(default = "default_add_page_position")]
    pub position: String,
    #[serde(default)]
    pub page: usize,
    #[serde(default = "default_add_page_count")]
    pub count: usize,
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
    #[serde(default = "default_scope")]
    pub scope: PageScope,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditPdfRequest {
    pub paths: Vec<PathBuf>,
    pub mode: String,
    #[serde(default)] pub text: String,
    #[serde(default)] pub pages: Option<Vec<usize>>,
    pub rectangle: PdfRect,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfEditSessionRequest {
    pub paths: Vec<PathBuf>,
    pub plan: PdfEditSessionPlan,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfEditSessionPlan {
    #[serde(default)]
    pub page_order: Vec<usize>,
    #[serde(default)]
    pub delete_pages: Vec<usize>,
    #[serde(default)]
    pub rotate_pages: Vec<RotatePage>,
    #[serde(default)]
    pub operations: Vec<PdfEditOperation>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PdfEditOperation {
    Crop {
        rectangle: PdfRect,
        scope: PageScope,
    },
    Overlay {
        overlay: PdfOverlay,
    },
    AddPages {
        page: usize,
        position: String,
        count: usize,
    },
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PdfEditMode {
    Text,
    Note,
    Highlight,
    Shape,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PdfOverlayPosition {
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PdfOverlay {
    Edit {
        mode: PdfEditMode,
        #[serde(default)]
        text: String,
        #[serde(default)]
        pages: Option<Vec<usize>>,
        rectangle: PdfRect,
    },
    Watermark {
        text: String,
        #[serde(default = "default_opacity")]
        opacity: u8,
        #[serde(default)]
        position: Option<PdfOverlayPosition>,
        #[serde(default)]
        logo_path: Option<PathBuf>,
        #[serde(default)]
        pages: Option<Vec<usize>>,
    },
    Sign {
        page: usize,
        text: String,
        #[serde(default)]
        signature_path: Option<PathBuf>,
        rectangle: PdfRect,
        #[serde(default = "default_scope")]
        scope: PageScope,
    },
    PageNumbers {
        #[serde(default)]
        start_number: Option<u32>,
        #[serde(default)]
        font_size: Option<u16>,
        #[serde(default)]
        position: Option<PdfOverlayPosition>,
        #[serde(default)]
        pages: Option<Vec<usize>>,
    },
}

fn default_scope() -> PageScope { PageScope::All }
fn default_add_page_position() -> String { "after".to_string() }
fn default_add_page_count() -> usize { 1 }
fn default_opacity() -> u8 { 100 }

pub fn normalize_session_plan(plan: &PdfEditSessionPlan, page_count: usize) -> Result<PdfEditSessionPlan, String> {
    if page_count == 0 {
        return Err("The PDF must contain at least one page.".to_string());
    }

    let mut page_order = if plan.page_order.is_empty() {
        (0..page_count).collect::<Vec<_>>()
    } else {
        plan.page_order.clone()
    };
    validate_unique_page_refs("page order", &page_order, page_count)?;
    for page in 0..page_count {
        if !page_order.contains(&page) {
            page_order.push(page);
        }
    }

    validate_unique_page_refs("deleted pages", &plan.delete_pages, page_count)?;
    validate_unique_rotations(&plan.rotate_pages, page_count)?;
    for operation in &plan.operations {
        validate_operation(operation, page_count)?;
    }
    if page_order.iter().filter(|page| !plan.delete_pages.contains(page)).count() == 0 {
        return Err("The PDF edit plan must keep at least one page.".to_string());
    }

    Ok(PdfEditSessionPlan {
        page_order,
        delete_pages: plan.delete_pages.clone(),
        rotate_pages: plan.rotate_pages.iter().map(|rotation| RotatePage {
            page: rotation.page,
            degrees: rotation.degrees.rem_euclid(360),
        }).collect(),
        operations: plan.operations.clone(),
    })
}

fn validate_unique_page_refs(label: &str, pages: &[usize], page_count: usize) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for page in pages {
        if *page >= page_count {
            return Err(format!("{label} references page {} outside the document.", page + 1));
        }
        if !seen.insert(*page) {
            return Err(format!("{label} contains page {} more than once.", page + 1));
        }
    }
    Ok(())
}

fn validate_unique_rotations(rotations: &[RotatePage], page_count: usize) -> Result<(), String> {
    let pages = rotations.iter().map(|rotation| rotation.page).collect::<Vec<_>>();
    validate_unique_page_refs("rotations", &pages, page_count)
}

fn validate_scope(scope: &PageScope, page_count: usize) -> Result<(), String> {
    match scope {
        PageScope::All => Ok(()),
        PageScope::Selected { pages } => {
            if pages.is_empty() {
                return Err("A selected-page scope must contain at least one page.".to_string());
            }
            validate_unique_page_refs("selected-page scope", pages, page_count)
        }
    }
}

fn validate_optional_pages(label: &str, pages: &Option<Vec<usize>>, page_count: usize) -> Result<(), String> {
    if let Some(pages) = pages {
        if pages.is_empty() {
            return Err(format!("{label} must contain at least one page when provided."));
        }
        validate_unique_page_refs(label, pages, page_count)?;
    }
    Ok(())
}

fn validate_operation(operation: &PdfEditOperation, page_count: usize) -> Result<(), String> {
    match operation {
        PdfEditOperation::Crop { rectangle, scope } => {
            validate_rect(rectangle)?;
            validate_scope(scope, page_count)
        }
        PdfEditOperation::Overlay { overlay } => match overlay {
            PdfOverlay::Edit { mode, text, pages, rectangle } => {
                validate_rect(rectangle)?;
                if !matches!(mode, PdfEditMode::Shape) && text.trim().is_empty() {
                    return Err("Text is required for this edit mode.".to_string());
                }
                validate_optional_pages("edit pages", pages, page_count)
            }
            PdfOverlay::Watermark { text, pages, .. } => {
                if text.trim().is_empty() {
                    return Err("Watermark text is required.".to_string());
                }
                validate_optional_pages("watermark pages", pages, page_count)
            }
            PdfOverlay::Sign { page, text, signature_path, rectangle, scope } => {
                if *page >= page_count {
                    return Err(format!("Signature page {} is outside the document.", page + 1));
                }
                if signature_path.is_none() && text.trim().is_empty() {
                    return Err("Signature text is required when no signature image is provided.".to_string());
                }
                validate_rect(rectangle)?;
                validate_scope(scope, page_count)
            }
            PdfOverlay::PageNumbers { pages, .. } => validate_optional_pages("page-number pages", pages, page_count),
        },
        PdfEditOperation::AddPages { page, position, count } => {
            if *count == 0 {
                return Err("Add at least one blank page.".to_string());
            }
            if *count > 100 {
                return Err("You can add at most 100 blank pages at once.".to_string());
            }
            match position.as_str() {
                "end" => Ok(()),
                "before" | "after" if *page < page_count => Ok(()),
                "before" | "after" => Err("The selected page is outside the document.".to_string()),
                _ => Err("Page insertion position must be before, after, or end.".to_string()),
            }
        }
    }
}

pub fn crop(request: &CropPdfRequest, input: PathBuf) -> JobOutcome {
    transform_pdf(input, &request.output_location, "-cropped", |document, pages| {
        let selected = selected_pages(&request.scope, pages.len());
        validate_rect(&request.rectangle)?;
        for (index, page_id) in pages.iter().enumerate() {
            if selected(index) {
                validate_rect_for_page(document, *page_id, &request.rectangle)?;
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
        validate_unique_page_refs("page order", &request.page_order, pages.len())?;
        validate_unique_page_refs("deleted pages", &request.delete_pages, pages.len())?;
        validate_unique_rotations(&request.rotate_pages, pages.len())?;
        let selected = scoped_indices(&request.scope, pages.len())?;
        let selected_set = selected.iter().copied().collect::<std::collections::HashSet<_>>();
        let deleted = request.delete_pages.iter().copied().filter(|index| selected_set.contains(index)).collect::<std::collections::HashSet<_>>();
        let mut selected_order = request.page_order.iter().copied().filter(|index| selected_set.contains(index) && !deleted.contains(index)).collect::<Vec<_>>();
        selected_order.extend(selected.iter().copied().filter(|index| !deleted.contains(index) && !request.page_order.contains(index)));
        let mut selected_iter = selected_order.into_iter();
        let mut order = Vec::new();
        for (index, page_id) in pages.iter().enumerate() {
            if !selected_set.contains(&index) { order.push(*page_id); }
            else if !deleted.contains(&index) { order.push(pages[selected_iter.next().ok_or("The selected organize scope could not be represented")?]); }
        }
        let root = document.get_dictionary(pages[0]).map_err(|e| e.to_string())?.get(b"Parent").map_err(|e| e.to_string())?.as_reference().map_err(|e| e.to_string())?;
        for operation in &request.rotate_pages {
            if selected_set.contains(&operation.page) {
                if let Some(page_id) = pages.get(operation.page).copied() {
                let page = document.get_dictionary_mut(page_id).map_err(|e| e.to_string())?;
                let degrees = operation.degrees.rem_euclid(360);
                page.set("Rotate", degrees as i64);
                }
            }
        }
        if order.is_empty() { return Err("The organize plan must keep at least one page.".to_string()); }
        let root = document.get_dictionary_mut(root).map_err(|e| e.to_string())?;
        root.set("Kids", order.iter().map(|page| Object::Reference(*page)).collect::<Vec<_>>());
        root.set("Count", order.len() as i64);
        Ok(())
    })
}

pub fn add_pages(request: &AddPdfPagesRequest, input: PathBuf) -> JobOutcome {
    transform_pdf(input, &request.output_location, "-pages-added", |document, pages| {
        if request.count == 0 {
            return Err("Add at least one blank page.".to_string());
        }
        if request.count > 100 {
            return Err("You can add at most 100 blank pages at once.".to_string());
        }

        let insertion_index = match request.position.as_str() {
            "before" if request.page < pages.len() => request.page,
            "after" if request.page < pages.len() => request.page + 1,
            "end" => pages.len(),
            "before" | "after" => return Err("The selected page is outside the document.".to_string()),
            _ => return Err("Page insertion position must be before, after, or end.".to_string()),
        };
        let parent = document
            .get_dictionary(pages[0])
            .map_err(|error| error.to_string())?
            .get(b"Parent")
            .map_err(|error| error.to_string())?
            .as_reference()
            .map_err(|error| error.to_string())?;
        let template_index = insertion_index.saturating_sub(1).min(pages.len() - 1);
        let template = document
            .get_dictionary(pages[template_index])
            .map_err(|error| error.to_string())?
            .clone();
        let (left, bottom, right, top) = page_bounds(document, pages[template_index])?;
        let mut inserted = Vec::with_capacity(request.count);

        for _ in 0..request.count {
            let mut blank = dictionary! {
                "Type" => "Page",
                "Parent" => Object::Reference(parent),
                "MediaBox" => vec![
                    Object::Real(left),
                    Object::Real(bottom),
                    Object::Real(right),
                    Object::Real(top),
                ],
            };
            for key in ["CropBox", "BleedBox", "TrimBox", "ArtBox", "Rotate", "UserUnit"] {
                if let Ok(value) = template.get(key.as_bytes()) {
                    blank.set(key, value.clone());
                }
            }
            inserted.push(document.add_object(blank));
        }

        let mut order = pages.to_vec();
        order.splice(insertion_index..insertion_index, inserted);
        let root = document.get_dictionary_mut(parent).map_err(|error| error.to_string())?;
        root.set("Kids", order.iter().map(|page| Object::Reference(*page)).collect::<Vec<_>>());
        root.set("Count", order.len() as i64);
        Ok(())
    })
}

pub fn sign(request: &SignPdfRequest, input: PathBuf) -> JobOutcome {
    transform_pdf(input, &request.output_location, "-signed", |document, pages| {
        validate_rect(&request.rectangle)?;
        let targets = scoped_indices(&request.scope, pages.len())?;
        if request.page >= pages.len() { return Err("Signature page is outside the document.".to_string()); }
        let stream = format!("BT /Fsig 24 Tf {} {} Td ({}) Tj ET", request.rectangle.x, request.rectangle.y, escape_text(&request.text));
        for page_index in targets {
        let page_id = pages[page_index];
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
            let content_id = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, stream.clone().into_bytes())));
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
        let Ok(contents) = contents else { page.set("Contents", Object::Reference(stream_id)); continue; };
        if let Object::Reference(_) = contents { continue; }
        let existing = contents.as_array().map_err(|e| e.to_string())?.to_vec();
        *contents = Object::Array(existing.into_iter().chain([Object::Reference(stream_id)]).collect());
        }
        Ok(())
    })
}

pub fn edit(request: &EditPdfRequest, input: PathBuf) -> JobOutcome {
    transform_pdf(input, &request.output_location, "-edited", |document, pages| {
        validate_rect(&request.rectangle)?;
        if request.mode != "shape" && request.text.trim().is_empty() { return Err("Text is required for this edit mode.".to_string()); }
        let targets = optional_indices(&request.pages, pages.len(), "edit pages")?;
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let content = if request.mode == "shape" {
            format!("q 0 0 0 RG 1 w {} {} {} {} re S Q", request.rectangle.x, request.rectangle.y, request.rectangle.width, request.rectangle.height)
        } else if request.mode == "highlight" {
            format!("q 1 1 0 rg {} {} {} {} re f Q BT /Fedit 12 Tf {} {} Td ({}) Tj ET", request.rectangle.x, request.rectangle.y, request.rectangle.width, request.rectangle.height, request.rectangle.x + 4.0, request.rectangle.y + request.rectangle.height - 16.0, escape_text(&request.text))
        } else {
            format!("BT /Fedit 12 Tf {} {} Td ({}) Tj ET", request.rectangle.x, request.rectangle.y, escape_text(&request.text))
        };
        for page_index in targets {
            let stream_id = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, content.clone().into_bytes())));
            let page_id = pages[page_index];
            let page = document.get_dictionary_mut(page_id).map_err(|e| e.to_string())?;
            if !page.has(b"Resources") { page.set("Resources", Object::Dictionary(dictionary! {})); }
            let resources = page.get_mut(b"Resources").map_err(|e| e.to_string())?.as_dict_mut().map_err(|e| e.to_string())?;
            resources.set("Font", resources.get(b"Font").cloned().unwrap_or_else(|_| Object::Dictionary(dictionary! {})));
            resources.get_mut(b"Font").map_err(|e| e.to_string())?.as_dict_mut().map_err(|e| e.to_string())?.set("Fedit", font_id);
            match page.get_mut(b"Contents") {
                Ok(Object::Array(contents)) => contents.push(Object::Reference(stream_id)),
                Ok(Object::Reference(existing)) => { let prior = Object::Reference(*existing); page.set("Contents", Object::Array(vec![prior, Object::Reference(stream_id)])); },
                _ => page.set("Contents", Object::Reference(stream_id)),
            }
        }
        Ok(())
    })
}

pub fn apply_session(request: &PdfEditSessionRequest, input: PathBuf) -> JobOutcome {
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-edited", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve PDF edit output: {error}")),
    };
    let mut document = match Document::load(&input) {
        Ok(document) => document,
        Err(error) => return failure(input, error.to_string()),
    };
    let pages = document.get_pages().values().copied().collect::<Vec<_>>();
    if pages.is_empty() {
        return failure(input, "PDF has no pages".to_string());
    }
    let plan = match normalize_session_plan(&request.plan, pages.len()) {
        Ok(plan) => plan,
        Err(error) => return failure(input, error),
    };
    let mut current_pages = match apply_page_plan(&mut document, &pages, &plan) {
        Ok(order) => order,
        Err(error) => return failure(input, error),
    };
    for operation in &plan.operations {
        let result = match operation {
            PdfEditOperation::Crop { rectangle, scope } => apply_crop_operation(&mut document, &pages, rectangle, scope),
            PdfEditOperation::Overlay { overlay } => apply_overlay(&mut document, &pages, overlay),
            PdfEditOperation::AddPages { page, position, count } => apply_add_pages_operation(&mut document, &mut current_pages, &pages, *page, position, *count),
        };
        if let Err(error) = result {
            return failure(input, error);
        }
    }
    match document.save(output.path()) {
        Ok(_) => match output.publish() {
            Ok(path) => JobOutcome {
                input_path: input,
                output_paths: vec![path],
                detail: "PDF edit session saved".to_string(),
                failure: None,
            },
            Err(error) => failure(input, format!("Could not publish PDF edit output: {error}")),
        },
        Err(error) => failure(input, format!("Save failed: {error}")),
    }
}

fn apply_page_plan(document: &mut Document, pages: &[lopdf::ObjectId], plan: &PdfEditSessionPlan) -> Result<Vec<lopdf::ObjectId>, String> {
    let parent = document
        .get_dictionary(pages[0])
        .map_err(|error| error.to_string())?
        .get(b"Parent")
        .map_err(|error| error.to_string())?
        .as_reference()
        .map_err(|error| error.to_string())?;
    let order = plan
        .page_order
        .iter()
        .filter(|page| !plan.delete_pages.contains(page))
        .map(|page| pages[*page])
        .collect::<Vec<_>>();
    if order.is_empty() {
        return Err("The PDF edit plan must keep at least one page.".to_string());
    }
    for rotation in &plan.rotate_pages {
        let page_id = pages[rotation.page];
        let page = document.get_dictionary_mut(page_id).map_err(|error| error.to_string())?;
        page.set("Rotate", rotation.degrees.rem_euclid(360) as i64);
    }
    let root = document.get_dictionary_mut(parent).map_err(|error| error.to_string())?;
    root.set("Kids", order.iter().map(|page| Object::Reference(*page)).collect::<Vec<_>>());
    root.set("Count", order.len() as i64);
    Ok(order)
}

fn apply_add_pages_operation(
    document: &mut Document,
    current_pages: &mut Vec<lopdf::ObjectId>,
    original_pages: &[lopdf::ObjectId],
    page: usize,
    position: &str,
    count: usize,
) -> Result<(), String> {
    let insertion_index = match position {
        "end" => current_pages.len(),
        "before" | "after" => {
            let page_id = *original_pages.get(page).ok_or_else(|| "The selected page is outside the document.".to_string())?;
            let index = current_pages.iter().position(|candidate| *candidate == page_id).ok_or_else(|| "The selected page is not available in the current document order.".to_string())?;
            if position == "after" { index + 1 } else { index }
        }
        _ => return Err("Page insertion position must be before, after, or end.".to_string()),
    };
    let template_index = insertion_index.saturating_sub(1).min(current_pages.len().saturating_sub(1));
    let template_id = *current_pages.get(template_index).ok_or_else(|| "The PDF must contain at least one page.".to_string())?;
    let parent = document
        .get_dictionary(template_id)
        .map_err(|error| error.to_string())?
        .get(b"Parent")
        .map_err(|error| error.to_string())?
        .as_reference()
        .map_err(|error| error.to_string())?;
    let template = document.get_dictionary(template_id).map_err(|error| error.to_string())?.clone();
    let (left, bottom, right, top) = page_bounds(document, template_id)?;
    let mut inserted = Vec::with_capacity(count);
    for _ in 0..count {
        let mut blank = dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference(parent),
            "MediaBox" => vec![
                Object::Real(left),
                Object::Real(bottom),
                Object::Real(right),
                Object::Real(top),
            ],
        };
        for key in ["CropBox", "BleedBox", "TrimBox", "ArtBox", "Rotate", "UserUnit"] {
            if let Ok(value) = template.get(key.as_bytes()) {
                blank.set(key, value.clone());
            }
        }
        inserted.push(document.add_object(blank));
    }
    current_pages.splice(insertion_index..insertion_index, inserted);
    let root = document.get_dictionary_mut(parent).map_err(|error| error.to_string())?;
    root.set("Kids", current_pages.iter().map(|page| Object::Reference(*page)).collect::<Vec<_>>());
    root.set("Count", current_pages.len() as i64);
    Ok(())
}

fn apply_crop_operation(document: &mut Document, pages: &[lopdf::ObjectId], rectangle: &PdfRect, scope: &PageScope) -> Result<(), String> {
    validate_rect(rectangle)?;
    for page_index in scoped_indices(scope, pages.len())? {
        validate_rect_for_page(document, pages[page_index], rectangle)?;
        let page = document.get_dictionary_mut(pages[page_index]).map_err(|error| error.to_string())?;
        page.set("MediaBox", vec![
            Object::Real(rectangle.x),
            Object::Real(rectangle.y),
            Object::Real(rectangle.x + rectangle.width),
            Object::Real(rectangle.y + rectangle.height),
        ]);
    }
    Ok(())
}

fn apply_overlay(document: &mut Document, pages: &[lopdf::ObjectId], overlay: &PdfOverlay) -> Result<(), String> {
    match overlay {
        PdfOverlay::Edit { mode, text, pages: targets, rectangle } => {
            validate_rect(rectangle)?;
            let targets = optional_indices(targets, pages.len(), "edit pages")?;
            for page_index in &targets {
                validate_rect_for_page(document, pages[*page_index], rectangle)?;
            }
            let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
            let content = match mode {
                PdfEditMode::Shape => format!("q 0 0 0 RG 1 w {} {} {} {} re S Q", rectangle.x, rectangle.y, rectangle.width, rectangle.height),
                PdfEditMode::Highlight => format!("q 1 1 0 rg {} {} {} {} re f Q BT /Fedit 12 Tf {} {} Td ({}) Tj ET", rectangle.x, rectangle.y, rectangle.width, rectangle.height, rectangle.x + 4.0, rectangle.y + rectangle.height - 16.0, escape_text(text)),
                PdfEditMode::Text | PdfEditMode::Note => format!("BT /Fedit 12 Tf {} {} Td ({}) Tj ET", rectangle.x, rectangle.y, escape_text(text)),
            };
            for page_index in targets {
                add_font(document, pages[page_index], "Fedit", font_id)?;
                append_content(document, pages[page_index], content.as_bytes())?;
            }
            Ok(())
        }
        PdfOverlay::Watermark { text, opacity, position, logo_path, pages: targets } => {
            let targets = optional_indices(targets, pages.len(), "watermark pages")?;
            if let Some(path) = logo_path {
                let (bytes, width, height) = signature_jpeg(path)?;
                let image_id = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {
                    "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64,
                    "Height" => height as i64, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
                    "Filter" => "DCTDecode",
                }, bytes)));
                for page_index in targets {
                    let (left, bottom, right, top) = page_bounds(document, pages[page_index])?;
                    let (x, y) = watermark_position(position.as_ref(), right - left, top - bottom);
                    add_xobject(document, pages[page_index], "Iwm", image_id)?;
                    let content = format!("q {} 0 0 {} {} {} cm /Iwm Do Q", 160.0, 80.0, left + x, bottom + y);
                    append_content(document, pages[page_index], content.as_bytes())?;
                }
            } else {
                let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
                for page_index in targets {
                    let (left, bottom, right, top) = page_bounds(document, pages[page_index])?;
                    let (x, y) = watermark_position(position.as_ref(), right - left, top - bottom);
                    add_font(document, pages[page_index], "Fwm", font_id)?;
                    let graphics_state = document.add_object(dictionary! { "Type" => "ExtGState", "ca" => (*opacity as f32 / 100.0).clamp(0.01, 1.0), "CA" => (*opacity as f32 / 100.0).clamp(0.01, 1.0) });
                    add_ext_gstate(document, pages[page_index], "GSwm", graphics_state)?;
                    let content = format!("q /GSwm gs 0 0 0 rg BT /Fwm 48 Tf {} {} Td ({}) Tj ET Q", left + x, bottom + y, escape_text(text));
                    append_content(document, pages[page_index], content.as_bytes())?;
                }
            }
            Ok(())
        }
        PdfOverlay::Sign { page, text, signature_path, rectangle, scope, .. } => {
            validate_rect(rectangle)?;
            let targets = match scope {
                PageScope::All => {
                    if *page >= pages.len() { return Err(format!("Signature page {} is outside the document.", page + 1)); }
                    vec![*page]
                }
                PageScope::Selected { .. } => scoped_indices(scope, pages.len())?,
            };
            for page_index in &targets {
                validate_rect_for_page(document, pages[*page_index], rectangle)?;
            }
            if let Some(path) = signature_path {
                let (bytes, width, height) = signature_jpeg(path)?;
                let image_id = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {
                    "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64,
                    "Height" => height as i64, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
                    "Filter" => "DCTDecode",
                }, bytes)));
                for page_index in targets {
                    add_xobject(document, pages[page_index], "Isig", image_id)?;
                    let content = format!("q {} 0 0 {} {} {} cm /Isig Do Q", rectangle.width, rectangle.height, rectangle.x, rectangle.y);
                    append_content(document, pages[page_index], content.as_bytes())?;
                }
            } else {
                let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
                for page_index in targets {
                    add_font(document, pages[page_index], "Fsig", font_id)?;
                    let content = format!("BT /Fsig 24 Tf {} {} Td ({}) Tj ET", rectangle.x, rectangle.y, escape_text(text));
                    append_content(document, pages[page_index], content.as_bytes())?;
                }
            }
            Ok(())
        }
        PdfOverlay::PageNumbers { start_number, font_size, position, pages: targets } => {
            let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
            for page_index in optional_indices(targets, pages.len(), "page-number pages")? {
                let (left, bottom, right, top) = page_bounds(document, pages[page_index])?;
                let size = font_size.unwrap_or(12).clamp(8, 72);
                let (x, y) = page_number_position(position.as_ref(), right - left, top - bottom);
                let number = start_number.unwrap_or(1).saturating_add(page_index as u32).saturating_sub(1);
                add_font(document, pages[page_index], "Fnum", font_id)?;
                let content = format!("BT /Fnum {size} Tf {} {} Td ({number}) Tj ET", left + x, bottom + y);
                append_content(document, pages[page_index], content.as_bytes())?;
            }
            Ok(())
        }
    }
}

fn scoped_indices(scope: &PageScope, page_count: usize) -> Result<Vec<usize>, String> {
    match scope {
        PageScope::All => Ok((0..page_count).collect()),
        PageScope::Selected { pages } => {
            validate_unique_page_refs("selected-page scope", pages, page_count)?;
            if pages.is_empty() { return Err("A selected-page scope must contain at least one page.".to_string()); }
            Ok(pages.clone())
        }
    }
}

fn optional_indices(pages: &Option<Vec<usize>>, page_count: usize, label: &str) -> Result<Vec<usize>, String> {
    match pages {
        Some(pages) => {
            validate_unique_page_refs(label, pages, page_count)?;
            if pages.is_empty() { return Err(format!("{label} must contain at least one page when provided.")); }
            Ok(pages.clone())
        }
        None => Ok((0..page_count).collect()),
    }
}

fn append_content(document: &mut Document, page_id: lopdf::ObjectId, content: &[u8]) -> Result<(), String> {
    let stream_id = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, content.to_vec())));
    let page = document.get_dictionary_mut(page_id).map_err(|error| error.to_string())?;
    match page.get_mut(b"Contents") {
        Ok(Object::Array(contents)) => contents.push(Object::Reference(stream_id)),
        Ok(Object::Reference(existing)) => {
            let prior = Object::Reference(*existing);
            page.set("Contents", Object::Array(vec![prior, Object::Reference(stream_id)]));
        }
        _ => page.set("Contents", Object::Reference(stream_id)),
    }
    Ok(())
}

fn add_font(document: &mut Document, page_id: lopdf::ObjectId, name: &str, font_id: lopdf::ObjectId) -> Result<(), String> {
    let page = document.get_dictionary_mut(page_id).map_err(|error| error.to_string())?;
    if !page.has(b"Resources") { page.set("Resources", Object::Dictionary(dictionary! {})); }
    let resources = page.get_mut(b"Resources").map_err(|error| error.to_string())?.as_dict_mut().map_err(|error| error.to_string())?;
    resources.set("Font", resources.get(b"Font").cloned().unwrap_or_else(|_| Object::Dictionary(dictionary! {})));
    resources.get_mut(b"Font").map_err(|error| error.to_string())?.as_dict_mut().map_err(|error| error.to_string())?.set(name, font_id);
    Ok(())
}

fn add_xobject(document: &mut Document, page_id: lopdf::ObjectId, name: &str, image_id: lopdf::ObjectId) -> Result<(), String> {
    let page = document.get_dictionary_mut(page_id).map_err(|error| error.to_string())?;
    if !page.has(b"Resources") { page.set("Resources", Object::Dictionary(dictionary! {})); }
    let resources = page.get_mut(b"Resources").map_err(|error| error.to_string())?.as_dict_mut().map_err(|error| error.to_string())?;
    resources.set("XObject", resources.get(b"XObject").cloned().unwrap_or_else(|_| Object::Dictionary(dictionary! {})));
    resources.get_mut(b"XObject").map_err(|error| error.to_string())?.as_dict_mut().map_err(|error| error.to_string())?.set(name, image_id);
    Ok(())
}

fn add_ext_gstate(document: &mut Document, page_id: lopdf::ObjectId, name: &str, state_id: lopdf::ObjectId) -> Result<(), String> {
    let page = document.get_dictionary_mut(page_id).map_err(|error| error.to_string())?;
    if !page.has(b"Resources") { page.set("Resources", Object::Dictionary(dictionary! {})); }
    let resources = page.get_mut(b"Resources").map_err(|error| error.to_string())?.as_dict_mut().map_err(|error| error.to_string())?;
    resources.set("ExtGState", resources.get(b"ExtGState").cloned().unwrap_or_else(|_| Object::Dictionary(dictionary! {})));
    resources.get_mut(b"ExtGState").map_err(|error| error.to_string())?.as_dict_mut().map_err(|error| error.to_string())?.set(name, state_id);
    Ok(())
}

fn watermark_position(position: Option<&PdfOverlayPosition>, width: f32, height: f32) -> (f32, f32) {
    let x = match position { Some(PdfOverlayPosition::TopLeft) | Some(PdfOverlayPosition::BottomLeft) => 24.0, Some(PdfOverlayPosition::TopRight) | Some(PdfOverlayPosition::BottomRight) => (width - 240.0).max(24.0), _ => (width - 240.0).max(24.0) / 2.0 };
    let y = match position { Some(PdfOverlayPosition::TopLeft) | Some(PdfOverlayPosition::TopRight) => (height - 60.0).max(24.0), Some(PdfOverlayPosition::BottomLeft) | Some(PdfOverlayPosition::BottomRight) => 24.0, _ => (height - 60.0).max(24.0) / 2.0 };
    (x, y)
}

fn page_number_position(position: Option<&PdfOverlayPosition>, width: f32, height: f32) -> (f32, f32) {
    match position {
        Some(PdfOverlayPosition::TopLeft) => (24.0, (height - 24.0).max(24.0)),
        Some(PdfOverlayPosition::TopRight) => ((width - 48.0).max(24.0), (height - 24.0).max(24.0)),
        Some(PdfOverlayPosition::BottomLeft) => (24.0, 24.0),
        _ => ((width - 48.0).max(24.0), 24.0),
    }
}

fn transform_pdf<F>(input: PathBuf, location: &OutputLocation, suffix: &str, edit: F) -> JobOutcome
where F: FnOnce(&mut Document, &[lopdf::ObjectId]) -> Result<(), String> {
    let output = match OutputNaming::reserve_destination(&input, location, suffix, "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve PDF output: {error}")),
    };
    let mut document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    let pages: Vec<_> = document.get_pages().values().copied().collect();
    if pages.is_empty() { return failure(input, "PDF has no pages".to_string()); }
    if let Err(error) = edit(&mut document, &pages) { return failure(input, error); }
    match document.save(output.path()) {
        Ok(_) => match output.publish() {
            Ok(path) => JobOutcome { input_path: input, output_paths: vec![path], detail: "PDF saved".to_string(), failure: None },
            Err(error) => failure(input, format!("Could not publish PDF output: {error}")),
        },
        Err(error) => failure(input, format!("Save failed: {error}")),
    }
}

fn selected_pages(scope: &PageScope, count: usize) -> impl Fn(usize) -> bool + '_ {
    move |index| match scope { PageScope::All => true, PageScope::Selected { pages } => pages.contains(&index) && index < count }
}

fn validate_rect(rect: &PdfRect) -> Result<(), String> { if !rect.x.is_finite() || !rect.y.is_finite() || !rect.width.is_finite() || !rect.height.is_finite() || rect.width <= 0.0 || rect.height <= 0.0 { Err("Rectangle must have positive finite dimensions.".to_string()) } else { Ok(()) } }
fn validate_rect_for_page(document: &Document, page_id: lopdf::ObjectId, rect: &PdfRect) -> Result<(), String> {
    let (left, bottom, right, top) = page_bounds(document, page_id).map_err(|_| "PDF page has an invalid media box.".to_string())?;
    if rect.x < left || rect.y < bottom || rect.x + rect.width > right || rect.y + rect.height > top { return Err("Crop rectangle must stay within every selected page's media box.".to_string()); }
    Ok(())
}
fn escape_text(text: &str) -> String { text.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)") }
fn signature_jpeg(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let image = image::open(path).map_err(|error| format!("Could not read signature image: {error}"))?;
    let rgb = image.to_rgb8();
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, 95).write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8).map_err(|error| format!("Could not encode signature image: {error}"))?;
    Ok((bytes, rgb.width(), rgb.height()))
}
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::processing(error)) }

#[allow(dead_code)]
fn _path(_: &Path) {}

#[cfg(test)]
mod session_tests {
    use super::*;
    use lopdf::dictionary;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("toolbox_pdf_session_{}_{}", std::process::id(), name))
    }

    fn location(folder: &Path) -> OutputLocation {
        OutputLocation::CustomFolder(folder.to_path_buf())
    }

    fn make_pdf(path: &Path) {
        make_pdf_with_pages(path, 2);
    }

    fn make_pdf_with_pages(path: &Path, page_count: usize) {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let content_id = document.add_object(Object::Stream(lopdf::Stream::new(
            dictionary! {},
            b"BT /F0 12 Tf 36 720 Td (source) Tj ET".to_vec(),
        )));
        let mut kids = Vec::new();
        for _ in 0..page_count {
            let page_id = document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F0" => font_id } },
                "Contents" => content_id,
            });
            kids.push(Object::Reference(page_id));
        }
        document.objects.insert(pages_id, dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => page_count as i64,
        }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).expect("fixture PDF should save");
    }

    fn editor_request_location(folder: &Path) -> OutputLocation {
        OutputLocation::CustomFolder(folder.to_path_buf())
    }

    #[test]
    fn sign_rejects_a_mixed_valid_and_out_of_bounds_scope_without_writing() {
        let source = temp_path("sign-invalid-scope.pdf");
        let output_dir = temp_path("sign-invalid-scope-output");
        std::fs::create_dir_all(&output_dir).expect("output directory should be created");
        make_pdf_with_pages(&source, 1);
        let source_bytes = std::fs::read(&source).expect("source should be readable");
        let outcome = sign(
            &SignPdfRequest {
                paths: vec![source.clone()],
                page: 0,
                text: "signed".to_string(),
                signature_path: None,
                rectangle: PdfRect { x: 10.0, y: 10.0, width: 100.0, height: 30.0 },
                scope: PageScope::Selected { pages: vec![0, 99] },
                output_location: editor_request_location(&output_dir),
            },
            source.clone(),
        );

        assert!(outcome.failure.is_some());
        assert!(outcome.output_paths.is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn edit_rejects_a_mixed_valid_and_out_of_bounds_selection_without_writing() {
        let source = temp_path("edit-invalid-selection.pdf");
        let output_dir = temp_path("edit-invalid-selection-output");
        std::fs::create_dir_all(&output_dir).expect("output directory should be created");
        make_pdf_with_pages(&source, 1);
        let source_bytes = std::fs::read(&source).expect("source should be readable");
        let outcome = edit(
            &EditPdfRequest {
                paths: vec![source.clone()],
                mode: "shape".to_string(),
                text: String::new(),
                pages: Some(vec![0, 99]),
                rectangle: PdfRect { x: 10.0, y: 10.0, width: 100.0, height: 30.0 },
                output_location: editor_request_location(&output_dir),
            },
            source.clone(),
        );

        assert!(outcome.failure.is_some());
        assert!(outcome.output_paths.is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn sign_and_edit_reject_empty_page_selections_without_writing() {
        let source = temp_path("empty-selection.pdf");
        let output_dir = temp_path("empty-selection-output");
        std::fs::create_dir_all(&output_dir).expect("output directory should be created");
        make_pdf_with_pages(&source, 1);
        let source_bytes = std::fs::read(&source).expect("source should be readable");
        let sign_outcome = sign(
            &SignPdfRequest {
                paths: vec![source.clone()],
                page: 0,
                text: "signed".to_string(),
                signature_path: None,
                rectangle: PdfRect { x: 10.0, y: 10.0, width: 100.0, height: 30.0 },
                scope: PageScope::Selected { pages: vec![] },
                output_location: editor_request_location(&output_dir),
            },
            source.clone(),
        );
        let edit_outcome = edit(
            &EditPdfRequest {
                paths: vec![source.clone()],
                mode: "shape".to_string(),
                text: String::new(),
                pages: Some(vec![]),
                rectangle: PdfRect { x: 10.0, y: 10.0, width: 100.0, height: 30.0 },
                output_location: editor_request_location(&output_dir),
            },
            source.clone(),
        );

        assert!(sign_outcome.failure.is_some());
        assert!(edit_outcome.failure.is_some());
        assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn session_plan_normalization_rejects_invalid_references_and_empty_output() {
        let invalid = PdfEditSessionPlan {
            page_order: vec![0, 2],
            delete_pages: vec![],
            rotate_pages: vec![],
            operations: vec![],
        };
        assert!(normalize_session_plan(&invalid, 2).is_err());

        let empty = PdfEditSessionPlan {
            page_order: vec![0, 1],
            delete_pages: vec![0, 1],
            rotate_pages: vec![],
            operations: vec![],
        };
        assert!(normalize_session_plan(&empty, 2).is_err());

        let invalid_overlay = PdfEditSessionPlan {
            page_order: vec![],
            delete_pages: vec![],
            rotate_pages: vec![],
            operations: vec![PdfEditOperation::Overlay {
                overlay: PdfOverlay::Edit {
                    mode: PdfEditMode::Text,
                    text: "text".to_string(),
                    pages: Some(vec![2]),
                    rectangle: PdfRect { x: 0.0, y: 0.0, width: 10.0, height: 10.0 },
                },
            }],
        };
        assert!(normalize_session_plan(&invalid_overlay, 2).is_err());

        let invalid_insert = PdfEditSessionPlan {
            page_order: vec![],
            delete_pages: vec![],
            rotate_pages: vec![],
            operations: vec![PdfEditOperation::AddPages {
                page: 0,
                position: "sideways".to_string(),
                count: 1,
            }],
        };
        assert!(normalize_session_plan(&invalid_insert, 2).is_err());
    }

    #[test]
    fn composed_session_applies_structure_and_multiple_overlays_without_mutating_source() {
        let source = temp_path("source.pdf");
        let output_dir = temp_path("output");
        std::fs::create_dir_all(&output_dir).expect("output directory should be created");
        make_pdf(&source);
        let source_bytes = std::fs::read(&source).expect("source should be readable");
        let request = PdfEditSessionRequest {
            paths: vec![source.clone()],
            plan: PdfEditSessionPlan {
                page_order: vec![1, 0],
                delete_pages: vec![],
                rotate_pages: vec![RotatePage { page: 0, degrees: 90 }],
                operations: vec![
                    PdfEditOperation::Crop {
                        rectangle: PdfRect { x: 0.0, y: 0.0, width: 500.0, height: 700.0 },
                        scope: PageScope::Selected { pages: vec![0] },
                    },
                    PdfEditOperation::Overlay {
                        overlay: PdfOverlay::Edit {
                            mode: PdfEditMode::Text,
                            text: "session text".to_string(),
                            pages: Some(vec![0]),
                            rectangle: PdfRect { x: 40.0, y: 600.0, width: 180.0, height: 40.0 },
                        },
                    },
                    PdfEditOperation::Overlay {
                        overlay: PdfOverlay::Watermark {
                            text: "session mark".to_string(),
                            opacity: 60,
                            position: Some(PdfOverlayPosition::Center),
                            logo_path: None,
                            pages: Some(vec![1]),
                        },
                    },
                    PdfEditOperation::AddPages {
                        page: 1,
                        position: "after".to_string(),
                        count: 1,
                    },
                ],
            },
            output_location: location(&output_dir),
        };

        let outcome = apply_session(&request, source.clone());
        assert!(outcome.failure.is_none(), "composed session failed: {:?}", outcome.failure);
        let output = outcome.output_paths.first().expect("session should produce one output");
        let document = Document::load(output).expect("composed output should reopen");
        assert_eq!(document.get_pages().len(), 3);
        let output_bytes = std::fs::read(output).expect("composed output should be readable");
        assert!(output_bytes.windows(b"session text".len()).any(|bytes| bytes == b"session text"));
        assert!(output_bytes.windows(b"session mark".len()).any(|bytes| bytes == b"session mark"));
        assert_eq!(std::fs::read(&source).expect("source should remain readable"), source_bytes);

        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(output);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn organize_selected_scope_preserves_unselected_pages_and_order() {
        let source = temp_path("organize-selected.pdf");
        let output_dir = temp_path("organize-selected-output");
        std::fs::create_dir_all(&output_dir).expect("output directory should be created");
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let mut kids = Vec::new();
        for index in 0..4 {
            let content = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, format!("BT /F0 12 Tf 36 720 Td (page-{index}) Tj ET").into_bytes())));
            kids.push(Object::Reference(document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Contents" => content,
            })));
        }
        document.objects.insert(pages_id, dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => 4}.into());
        let catalog = document.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
        document.trailer.set("Root", catalog);
        document.save(&source).expect("fixture PDF should save");

        let outcome = organize(&OrganizePdfRequest {
            paths: vec![source.clone()], page_order: vec![3, 1], delete_pages: vec![],
            rotate_pages: vec![], scope: PageScope::Selected { pages: vec![1, 3] }, output_location: location(&output_dir),
        }, source.clone());
        assert!(outcome.failure.is_none(), "selected organize failed: {:?}", outcome.failure);
        let output = outcome.output_paths.first().expect("organize should produce an output");
        let output_document = Document::load(output).expect("organized PDF should reopen");
        let order = output_document.get_pages().values().copied().collect::<Vec<_>>();
        let contents = order.iter().map(|page| String::from_utf8_lossy(&output_document.get_page_content(*page)).into_owned()).collect::<Vec<_>>();
        assert_eq!(contents, vec!["BT /F0 12 Tf 36 720 Td (page-0) Tj ET\n", "BT /F0 12 Tf 36 720 Td (page-3) Tj ET\n", "BT /F0 12 Tf 36 720 Td (page-2) Tj ET\n", "BT /F0 12 Tf 36 720 Td (page-1) Tj ET\n"]);

        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(output);
        let _ = std::fs::remove_dir(output_dir);
    }
}
