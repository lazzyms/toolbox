use image::codecs::jpeg::JpegEncoder;
use image::ImageEncoder;
use lopdf::{dictionary, Document, Object, ObjectId};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::{SkipReason, ToolError};
use super::metadata::{effective_crop_bounds, page_bounds, page_rotation};
use super::page_ranges::PageRanges;
use super::{inherited, mutation_preflight, note_signature_removed, remove_signatures, PdfMutationIntent, PdfMutationPreflight, SignedPdf};

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
pub enum RangedPageScope {
    All,
    Selected { pages: Vec<usize> },
    Range { ranges: String },
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropPdfRequest {
    pub paths: Vec<PathBuf>,
    pub spec: CropSpec,
    pub scope: RangedPageScope,
    pub output_location: OutputLocation,
}

/// What to keep on each page, measured on the page as the reader sees it (after /Rotate). The rectangle is
/// fractions of the displayed page with a top-left origin, so it lands in the same relative spot on any page
/// size; margins are points inset from the displayed edges.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CropSpec {
    Rectangle { left: f32, top: f32, width: f32, height: f32 },
    Margins { top: f32, right: f32, bottom: f32, left: f32 },
}

/// Slack for fractions that were rounded on their way from the preview.
const FRACTION_TOLERANCE: f32 = 1e-4;

impl CropSpec {
    fn validate(&self) -> Result<(), ToolError> {
        match *self {
            CropSpec::Rectangle { left, top, width, height } => {
                let finite = [left, top, width, height].iter().all(|value| value.is_finite());
                if !finite || width <= 0.0 || height <= 0.0 {
                    return Err(ToolError::invalid_input("The crop rectangle needs a width and height above zero."));
                }
                if left < -FRACTION_TOLERANCE || top < -FRACTION_TOLERANCE || left + width > 1.0 + FRACTION_TOLERANCE || top + height > 1.0 + FRACTION_TOLERANCE {
                    return Err(ToolError::invalid_input("The crop rectangle must stay inside the page."));
                }
            }
            CropSpec::Margins { top, right, bottom, left } => {
                if [top, right, bottom, left].iter().any(|margin| !margin.is_finite() || *margin < 0.0) {
                    return Err(ToolError::invalid_input("Margins must be zero or more points."));
                }
            }
        }
        Ok(())
    }
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

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactPdfRequest {
    pub paths: Vec<PathBuf>,
    pub rectangle: PdfRect,
    #[serde(default = "default_scope")] pub scope: PageScope,
    pub output_location: OutputLocation,
}

fn default_scope() -> PageScope { PageScope::All }
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
        validate_operation_targets(operation, &plan.delete_pages, page_count)?;
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

fn reject_annotated_pages(
    preflight: &PdfMutationPreflight,
    pages: &[ObjectId],
    page_indices: impl IntoIterator<Item = usize>,
    operation: &str,
) -> Result<(), String> {
    let Some(page_index) = page_indices.into_iter().find(|index| {
        pages.get(*index).is_some_and(|page| preflight.annotated_pages.contains(page))
    }) else {
        return Ok(());
    };
    Err(format!(
        "PDF page {} has annotations and cannot be {operation}; mutation was rejected before output",
        page_index + 1
    ))
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

fn validate_operation_targets(operation: &PdfEditOperation, deleted_pages: &[usize], page_count: usize) -> Result<(), String> {
    let targets = match operation {
        PdfEditOperation::Crop { scope, .. } => scoped_indices(scope, page_count)?,
        PdfEditOperation::Overlay { overlay } => match overlay {
            PdfOverlay::Edit { pages, .. } => optional_indices(pages, page_count, "edit pages")?,
            PdfOverlay::Watermark { pages, .. } => optional_indices(pages, page_count, "watermark pages")?,
            PdfOverlay::Sign { page, scope, .. } => match scope {
                PageScope::All => vec![*page],
                PageScope::Selected { .. } => scoped_indices(scope, page_count)?,
            },
            PdfOverlay::PageNumbers { pages, .. } => optional_indices(pages, page_count, "page-number pages")?,
        },
        PdfEditOperation::AddPages { .. } => return Ok(()),
    };
    if let Some(page) = targets.into_iter().find(|page| deleted_pages.contains(page)) {
        return Err(format!("PDF edit operation targets page {} that is also scheduled for deletion.", page + 1));
    }
    Ok(())
}

pub fn crop(request: &CropPdfRequest, input: PathBuf) -> JobOutcome {
    transform_or_skip_pdf(input, &request.output_location, "-cropped", PdfMutationIntent::ValidateOnly, SignedPdf::RemoveSignature, |document, pages, _preflight| {
        request.spec.validate()?;
        let (selected, notes) = match &request.scope {
            RangedPageScope::All => ((0..pages.len()).collect(), Vec::new()),
            RangedPageScope::Selected { pages: selected } => (selected_indices(selected, pages.len()).map_err(ToolError::invalid_input)?, Vec::new()),
            RangedPageScope::Range { ranges } => match PageRanges::parse(ranges).map_err(ToolError::invalid_input)?.clamp_or_skip(pages.len()) {
                Ok(clamped) => (clamped.page_indices(), clamped.notes),
                Err(reason) => return Ok(Edited::Skipped(reason)),
            },
        };
        // Only the crop box changes, so page objects, annotations, links, forms and structure stay valid.
        for page_index in selected {
            let page_id = pages[page_index];
            let crop_box = displayed_crop_box(document, page_id, page_index, &request.spec)?;
            let page = document.get_dictionary_mut(page_id).map_err(|error| ToolError::processing(error.to_string()))?;
            page.set("CropBox", crop_box.iter().copied().map(Object::Real).collect::<Vec<_>>());
        }
        Ok(Edited::Saved(if notes.is_empty() { "PDF saved".to_string() } else { format!("PDF saved. {}", notes.join(" ")) }))
    })
}

/// Insets the page's visible area from its displayed top, right, bottom and left edges: by fixed lengths for
/// margins, or by the parts of the displayed page outside a fractional rectangle.
fn displayed_crop_box(document: &Document, page_id: ObjectId, page_index: usize, spec: &CropSpec) -> Result<[f32; 4], ToolError> {
    let [x0, y0, x1, y1] = effective_crop_bounds(document, page_id).map_err(ToolError::processing)?;
    let user_unit = match document.get_dictionary(page_id).ok().and_then(|page| page.get(b"UserUnit").ok()) {
        Some(value) => super::resolve(document, value).and_then(super::metadata::number).map_err(ToolError::processing)?,
        None => 1.0,
    };
    if !user_unit.is_finite() || user_unit <= 0.0 { return Err(ToolError::processing("PDF page has an invalid UserUnit.")); }
    let rotation = page_rotation(document, page_id);
    let [top, right, bottom, left] = match *spec {
        CropSpec::Margins { top, right, bottom, left } => [top, right, bottom, left].map(|inset| inset / user_unit),
        CropSpec::Rectangle { left, top, width, height } => {
            // Displayed width and height in user space: a quarter turn swaps the page's sides.
            let (displayed_width, displayed_height) = if rotation % 180 == 90 { (y1 - y0, x1 - x0) } else { (x1 - x0, y1 - y0) };
            let clamp = |fraction: f32| fraction.clamp(0.0, 1.0);
            [clamp(top) * displayed_height, clamp(1.0 - left - width) * displayed_width, clamp(1.0 - top - height) * displayed_height, clamp(left) * displayed_width]
        }
    };
    // User-space insets (left, bottom, right, top) for each clockwise /Rotate value.
    let [inset_left, inset_bottom, inset_right, inset_top] = match rotation {
        0 => [left, bottom, right, top],
        90 => [top, left, bottom, right],
        180 => [right, top, left, bottom],
        270 => [bottom, right, top, left],
        rotation => return Err(ToolError::processing(format!("Page {} has an unsupported rotation of {rotation} degrees.", page_index + 1))),
    };
    let crop_box = [x0 + inset_left, y0 + inset_bottom, x1 - inset_right, y1 - inset_top];
    if crop_box[2] <= crop_box[0] || crop_box[3] <= crop_box[1] {
        let cause = if matches!(spec, CropSpec::Margins { .. }) { "these margins" } else { "this rectangle" };
        return Err(ToolError::invalid_input(format!("Page {} has no area left after {cause}.", page_index + 1)));
    }
    Ok(crop_box)
}

pub fn sign(request: &SignPdfRequest, input: PathBuf) -> JobOutcome {
    transform_pdf(input, &request.output_location, "-signed", PdfMutationIntent::General, SignedPdf::Reject, |document, pages, _preflight| {
        validate_rect(&request.rectangle)?;
        if request.page >= pages.len() { return Err("Signature page is outside the document.".to_string()); }
        let targets = match &request.scope {
            PageScope::All => vec![request.page],
            PageScope::Selected { .. } => scoped_indices(&request.scope, pages.len())?,
        };
        for page_index in targets {
        let page_id = pages[page_index];
        let stream_id = if let Some(path) = &request.signature_path {
            let (bytes, width, height) = signature_jpeg(path)?;
            let image_id = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64,
                "Height" => height as i64, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
                "Filter" => "DCTDecode",
            }, bytes)));
            let resource_name = add_xobject(document, page_id, "Isig", image_id)?;
            let content = format!("q {} 0 0 {} {} {} cm /{resource_name} Do Q", request.rectangle.width, request.rectangle.height, request.rectangle.x, request.rectangle.y);
            document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, content.into_bytes())))
        } else {
            let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
            let resource_name = add_font(document, page_id, "Fsig", font_id)?;
            let content = format!("BT /{resource_name} 24 Tf {} {} Td ({}) Tj ET", request.rectangle.x, request.rectangle.y, escape_text(&request.text));
            document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, content.into_bytes())))
        };
        append_content_stream(document, page_id, stream_id)?;
        }
        Ok(())
    })
}

pub fn edit(request: &EditPdfRequest, input: PathBuf) -> JobOutcome {
    transform_pdf(input, &request.output_location, "-edited", PdfMutationIntent::General, SignedPdf::Reject, |document, pages, _preflight| {
        validate_rect(&request.rectangle)?;
        if request.mode != "shape" && request.text.trim().is_empty() { return Err("Text is required for this edit mode.".to_string()); }
        let targets = optional_indices(&request.pages, pages.len(), "edit pages")?;
        let font_id = (request.mode != "shape").then(|| document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" }));
        for page_index in targets {
            let page_id = pages[page_index];
            let content = if let Some(font_id) = font_id {
                let font_name = add_font(document, page_id, "Fedit", font_id)?;
                if request.mode == "highlight" {
                    format!("q 1 1 0 rg {} {} {} {} re f Q BT /{font_name} 12 Tf {} {} Td ({}) Tj ET", request.rectangle.x, request.rectangle.y, request.rectangle.width, request.rectangle.height, request.rectangle.x + 4.0, request.rectangle.y + request.rectangle.height - 16.0, escape_text(&request.text))
                } else {
                    format!("BT /{font_name} 12 Tf {} {} Td ({}) Tj ET", request.rectangle.x, request.rectangle.y, escape_text(&request.text))
                }
            } else {
                format!("q 0 0 0 RG 1 w {} {} {} {} re S Q", request.rectangle.x, request.rectangle.y, request.rectangle.width, request.rectangle.height)
            };
            append_content(document, page_id, content.as_bytes())?;
        }
        Ok(())
    })
}

/// Permanently removes everything under the rectangle: each target page is flattened to an image with
/// the area painted black, and its original text, vectors, and annotations are dropped from the file.
pub fn redact(request: &RedactPdfRequest, input: PathBuf) -> JobOutcome {
    let mut outcome = transform_pdf(input, &request.output_location, "-redacted", PdfMutationIntent::General, SignedPdf::Reject, |document, pages, _preflight| {
        validate_rect(&request.rectangle)?;
        let targets = scoped_indices(&request.scope, pages.len())?;
        let renderer = super::remaining::pdftoppm().ok_or("The PDF renderer (pdftoppm) is required to redact but was not found.")?;
        let mut rotations = Vec::new();
        for index in &targets {
            let page_id = pages[*index];
            rotations.push(page_rotation(document, page_id));
            document.get_dictionary_mut(page_id).map_err(|e| e.to_string())?.set("Rotate", 0);
        }
        let workspace = RedactionWorkspace::new()?;
        let unrotated = workspace.0.join("source.pdf");
        document.save(&unrotated).map_err(|error| format!("Could not prepare the PDF for redaction: {error}"))?;
        for (index, rotation) in targets.into_iter().zip(rotations) {
            let page_id = pages[index];
            let [left, bottom, right, top] = effective_crop_bounds(document, page_id)?;
            let (x0, y0) = (request.rectangle.x.max(left), request.rectangle.y.max(bottom));
            let (x1, y1) = ((request.rectangle.x + request.rectangle.width).min(right), (request.rectangle.y + request.rectangle.height).min(top));
            if x0 >= x1 || y0 >= y1 { return Err(format!("The redaction box is outside page {}.", index + 1)); }
            let prefix = workspace.0.join(format!("page-{}", index + 1));
            let number = (index + 1).to_string();
            let result = Command::new(&renderer).args(["-r", "200", "-png", "-singlefile", "-f", &number, "-l", &number]).arg(&unrotated).arg(&prefix).output()
                .map_err(|error| format!("Could not run the PDF renderer: {error}"))?;
            if !result.status.success() { return Err(format!("Could not render page {} for redaction.", index + 1)); }
            let mut image = image::open(prefix.with_extension("png")).map_err(|error| format!("Could not read rendered page {}: {error}", index + 1))?.to_rgb8();
            let (width, height) = image.dimensions();
            let (scale_x, scale_y) = (width as f32 / (right - left), height as f32 / (top - bottom));
            let pixel = |value: f32, limit: u32| (value.max(0.0) as u32).min(limit);
            let (px0, px1) = (pixel(((x0 - left) * scale_x).floor(), width), pixel(((x1 - left) * scale_x).ceil(), width));
            let (py0, py1) = (pixel(((top - y1) * scale_y).floor(), height), pixel(((top - y0) * scale_y).ceil(), height));
            for y in py0..py1 { for x in px0..px1 { image.put_pixel(x, y, image::Rgb([0, 0, 0])); } }
            let mut bytes = Vec::new();
            JpegEncoder::new_with_quality(&mut bytes, 90).write_image(image.as_raw(), width, height, image::ExtendedColorType::Rgb8).map_err(|error| format!("Could not encode redacted page: {error}"))?;
            let image_id = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64,
                "Height" => height as i64, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
                "Filter" => "DCTDecode",
            }, bytes)));
            let content = format!("q {} 0 0 {} {left} {bottom} cm /Iredact Do Q", right - left, top - bottom);
            let content_id = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, content.into_bytes())));
            let page = document.get_dictionary_mut(page_id).map_err(|e| e.to_string())?;
            page.set("Contents", content_id);
            page.set("Resources", dictionary! { "XObject" => dictionary! { "Iredact" => image_id } });
            page.set("Rotate", rotation);
            page.remove(b"Annots");
        }
        document.prune_objects();
        Ok(())
    });
    if outcome.error().is_none() { outcome.detail = "Area permanently removed; redacted pages are now images".to_string(); }
    outcome
}

/// A private temp folder for redaction renders, removed when dropped.
struct RedactionWorkspace(PathBuf);

impl RedactionWorkspace {
    fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("toolbox-redact-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).map_err(|error| format!("Could not create a temporary folder: {error}"))?;
        Ok(Self(path))
    }
}

impl Drop for RedactionWorkspace {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

pub fn apply_session(request: &PdfEditSessionRequest, input: PathBuf) -> JobOutcome {
    let mut document = match Document::load(&input) {
        Ok(document) => document,
        Err(error) => return failure(input, error.to_string()),
    };
    if document.is_encrypted() {
        return failure(input, "Unlock the PDF before editing".to_string());
    }
    let pages = document.get_pages().values().copied().collect::<Vec<_>>();
    if pages.is_empty() {
        return failure(input, "PDF has no pages".to_string());
    }
    let preflight = match mutation_preflight(&document, PdfMutationIntent::ValidateOnly) {
        Ok(preflight) => preflight,
        Err(error) => return failure(input, error),
    };
    let plan = match normalize_session_plan(&request.plan, pages.len()) {
        Ok(plan) => plan,
        Err(error) => return failure(input, error),
    };
    let intent = if plan.delete_pages.is_empty() && plan.rotate_pages.is_empty() && plan.operations.is_empty() {
        PdfMutationIntent::ReorderOnly
    } else {
        PdfMutationIntent::General
    };
    if let Err(error) = preflight.enforce_policy(intent) {
        return failure(input, error);
    }
    if let Err(error) = reject_annotated_pages(&preflight, &pages, plan.delete_pages.iter().copied(), "deleted") {
        return failure(input, error);
    }
    if let Err(error) = reject_annotated_pages(&preflight, &pages, plan.rotate_pages.iter().map(|rotation| rotation.page), "rotated") {
        return failure(input, error);
    }
    for operation in &plan.operations {
        if let PdfEditOperation::Crop { scope, .. } = operation {
            let targets = match scoped_indices(scope, pages.len()) {
                Ok(targets) => targets,
                Err(error) => return failure(input, error),
            };
            if let Err(error) = reject_annotated_pages(&preflight, &pages, targets, "cropped") {
                return failure(input, error);
            }
        }
    }
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-edited", "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve PDF edit output: {error}")),
    };
    let mut current_pages = match apply_page_plan(&mut document, &pages, preflight.pages_root, &plan) {
        Ok(order) => order,
        Err(error) => return failure(input, error),
    };
    for operation in &plan.operations {
        let result = match operation {
            PdfEditOperation::Crop { rectangle, scope } => apply_crop_operation(&mut document, &pages, rectangle, scope),
            PdfEditOperation::Overlay { overlay } => apply_overlay(&mut document, &pages, overlay),
            PdfEditOperation::AddPages { page, position, count } => apply_add_pages_operation(&mut document, &mut current_pages, &pages, preflight.pages_root, *page, position, *count),
        };
        if let Err(error) = result {
            return failure(input, error);
        }
    }
    match document.save(output.path()) {
        Ok(_) => match output.publish() {
            Ok(path) => JobOutcome::succeeded(input, vec![path], "PDF edit session saved"),
            Err(error) => failure(input, format!("Could not publish PDF edit output: {error}")),
        },
        Err(error) => failure(input, format!("Save failed: {error}")),
    }
}

fn apply_page_plan(document: &mut Document, pages: &[lopdf::ObjectId], parent: lopdf::ObjectId, plan: &PdfEditSessionPlan) -> Result<Vec<lopdf::ObjectId>, String> {
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
    parent: lopdf::ObjectId,
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
        super::set_page_box_family(document, pages[page_index], [
            rectangle.x,
            rectangle.y,
            rectangle.x + rectangle.width,
            rectangle.y + rectangle.height,
        ], true)?;
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
            let font_id = (!matches!(mode, PdfEditMode::Shape)).then(|| document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" }));
            for page_index in targets {
                let page_id = pages[page_index];
                let content = if let Some(font_id) = font_id {
                    let font_name = add_font(document, page_id, "Fedit", font_id)?;
                    match mode {
                        PdfEditMode::Highlight => format!("q 1 1 0 rg {} {} {} {} re f Q BT /{font_name} 12 Tf {} {} Td ({}) Tj ET", rectangle.x, rectangle.y, rectangle.width, rectangle.height, rectangle.x + 4.0, rectangle.y + rectangle.height - 16.0, escape_text(text)),
                        PdfEditMode::Text | PdfEditMode::Note => format!("BT /{font_name} 12 Tf {} {} Td ({}) Tj ET", rectangle.x, rectangle.y, escape_text(text)),
                        PdfEditMode::Shape => unreachable!(),
                    }
                } else {
                    format!("q 0 0 0 RG 1 w {} {} {} {} re S Q", rectangle.x, rectangle.y, rectangle.width, rectangle.height)
                };
                append_content(document, page_id, content.as_bytes())?;
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
                    let image_name = add_xobject(document, pages[page_index], "Iwm", image_id)?;
                    let content = format!("q {} 0 0 {} {} {} cm /{image_name} Do Q", 160.0, 80.0, left + x, bottom + y);
                    append_content(document, pages[page_index], content.as_bytes())?;
                }
            } else {
                let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
                for page_index in targets {
                    let (left, bottom, right, top) = page_bounds(document, pages[page_index])?;
                    let (x, y) = watermark_position(position.as_ref(), right - left, top - bottom);
                    let font_name = add_font(document, pages[page_index], "Fwm", font_id)?;
                    let graphics_state = document.add_object(dictionary! { "Type" => "ExtGState", "ca" => (*opacity as f32 / 100.0).clamp(0.01, 1.0), "CA" => (*opacity as f32 / 100.0).clamp(0.01, 1.0) });
                    let state_name = add_ext_gstate(document, pages[page_index], "GSwm", graphics_state)?;
                    let content = format!("q /{state_name} gs 0 0 0 rg BT /{font_name} 48 Tf {} {} Td ({}) Tj ET Q", left + x, bottom + y, escape_text(text));
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
                    let image_name = add_xobject(document, pages[page_index], "Isig", image_id)?;
                    let content = format!("q {} 0 0 {} {} {} cm /{image_name} Do Q", rectangle.width, rectangle.height, rectangle.x, rectangle.y);
                    append_content(document, pages[page_index], content.as_bytes())?;
                }
            } else {
                let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
                for page_index in targets {
                    let font_name = add_font(document, pages[page_index], "Fsig", font_id)?;
                    let content = format!("BT /{font_name} 24 Tf {} {} Td ({}) Tj ET", rectangle.x, rectangle.y, escape_text(text));
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
                let font_name = add_font(document, pages[page_index], "Fnum", font_id)?;
                let content = format!("BT /{font_name} {size} Tf {} {} Td ({number}) Tj ET", left + x, bottom + y);
                append_content(document, pages[page_index], content.as_bytes())?;
            }
            Ok(())
        }
    }
}

fn scoped_indices(scope: &PageScope, page_count: usize) -> Result<Vec<usize>, String> {
    match scope {
        PageScope::All => Ok((0..page_count).collect()),
        PageScope::Selected { pages } => selected_indices(pages, page_count),
    }
}

fn selected_indices(pages: &[usize], page_count: usize) -> Result<Vec<usize>, String> {
    validate_unique_page_refs("selected-page scope", pages, page_count)?;
    if pages.is_empty() { return Err("A selected-page scope must contain at least one page.".to_string()); }
    Ok(pages.to_vec())
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
    append_content_stream(document, page_id, stream_id)
}

fn append_content_stream(document: &mut Document, page_id: lopdf::ObjectId, stream_id: lopdf::ObjectId) -> Result<(), String> {
    let prior = document.get_dictionary(page_id).map_err(|error| error.to_string())?.get(b"Contents").ok().cloned();
    let contents = match prior {
        Some(Object::Array(contents)) => Object::Array(contents.into_iter().chain([Object::Reference(stream_id)]).collect()),
        Some(Object::Reference(existing)) => Object::Array(vec![Object::Reference(existing), Object::Reference(stream_id)]),
        Some(existing) => {
            let existing_id = document.add_object(existing);
            Object::Array(vec![Object::Reference(existing_id), Object::Reference(stream_id)])
        }
        None => Object::Reference(stream_id),
    };
    document.get_dictionary_mut(page_id).map_err(|error| error.to_string())?.set("Contents", contents);
    Ok(())
}

fn next_resource_name(entries: &lopdf::Dictionary, category: &str, base: &str) -> Result<String, String> {
    if !entries.has(base.as_bytes()) { return Ok(base.to_string()); }
    for suffix in 1..=10000 {
        let candidate = format!("{base}{suffix}");
        if !entries.has(candidate.as_bytes()) { return Ok(candidate); }
    }
    Err(format!("Could not allocate a unique {category} resource name"))
}

pub(crate) fn add_resource(document: &mut Document, page_id: lopdf::ObjectId, category: &str, name: &str, value: Object) -> Result<String, String> {
    let mut resources = inherited(document, page_id, b"Resources")?
        .map(|value| value.as_dict().map(|dictionary| dictionary.clone()).map_err(|error| error.to_string()))
        .transpose()?
        .unwrap_or_default();
    let mut entries = match resources.get(category.as_bytes()) {
        Ok(value) => super::resolve(document, value).and_then(|value| value.as_dict().map(|dictionary| dictionary.clone()).map_err(|error| error.to_string()))?,
        Err(_) => lopdf::Dictionary::new(),
    };
    let resource_name = next_resource_name(&entries, category, name)?;
    entries.set(resource_name.as_str(), value);
    resources.set(category, entries);
    document.get_dictionary_mut(page_id).map_err(|error| error.to_string())?.set("Resources", Object::Dictionary(resources));
    Ok(resource_name)
}

fn add_font(document: &mut Document, page_id: lopdf::ObjectId, name: &str, font_id: lopdf::ObjectId) -> Result<String, String> {
    add_resource(document, page_id, "Font", name, Object::Reference(font_id))
}

fn add_xobject(document: &mut Document, page_id: lopdf::ObjectId, name: &str, image_id: lopdf::ObjectId) -> Result<String, String> {
    add_resource(document, page_id, "XObject", name, Object::Reference(image_id))
}

fn add_ext_gstate(document: &mut Document, page_id: lopdf::ObjectId, name: &str, state_id: lopdf::ObjectId) -> Result<String, String> {
    add_resource(document, page_id, "ExtGState", name, Object::Reference(state_id))
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

enum Edited {
    Saved(String),
    Skipped(SkipReason),
}

fn transform_pdf<F>(input: PathBuf, location: &OutputLocation, suffix: &str, intent: PdfMutationIntent, signed: SignedPdf, edit: F) -> JobOutcome
where F: FnOnce(&mut Document, &[lopdf::ObjectId], &PdfMutationPreflight) -> Result<(), String> {
    transform_or_skip_pdf(input, location, suffix, intent, signed, |document, pages, preflight| edit(document, pages, preflight).map(|()| Edited::Saved("PDF saved".to_string())).map_err(ToolError::processing))
}

/// Runs one PDF edit. The edit chooses its error kind, such as InvalidInput for a bad request, and may skip
/// the file with a reason instead of writing an output.
fn transform_or_skip_pdf<F>(input: PathBuf, location: &OutputLocation, suffix: &str, intent: PdfMutationIntent, signed: SignedPdf, edit: F) -> JobOutcome
where F: FnOnce(&mut Document, &[lopdf::ObjectId], &PdfMutationPreflight) -> Result<Edited, ToolError> {
    let mut document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    if document.is_encrypted() { return failure(input, "Unlock the PDF before editing".to_string()); }
    let pages: Vec<_> = document.get_pages().values().copied().collect();
    if pages.is_empty() { return failure(input, "PDF has no pages".to_string()); }
    let signature_removed = match signed {
        SignedPdf::Reject => false,
        SignedPdf::RemoveSignature => match remove_signatures(&mut document) { Ok(removed) => removed, Err(error) => return failure(input, error) },
    };
    let preflight = match mutation_preflight(&document, intent) {
        Ok(preflight) => preflight,
        Err(error) => return failure(input, error),
    };
    let detail = match edit(&mut document, &pages, &preflight) {
        Ok(Edited::Saved(detail)) => detail,
        Ok(Edited::Skipped(reason)) => return JobOutcome::skipped(input, reason),
        Err(error) => return JobOutcome::failure(input, error),
    };
    let output = match OutputNaming::reserve_destination(&input, location, suffix, "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve PDF output: {error}")),
    };
    match document.save(output.path()) {
        Ok(_) => match output.publish() {
            Ok(path) => note_signature_removed(JobOutcome::succeeded(input, vec![path], detail), signature_removed),
            Err(error) => failure(input, format!("Could not publish PDF output: {error}")),
        },
        Err(error) => failure(input, format!("Save failed: {error}")),
    }
}

fn validate_rect(rect: &PdfRect) -> Result<(), String> { if !rect.x.is_finite() || !rect.y.is_finite() || !rect.width.is_finite() || !rect.height.is_finite() || rect.width <= 0.0 || rect.height <= 0.0 { Err("Rectangle must have positive finite dimensions.".to_string()) } else { Ok(()) } }
fn validate_rect_for_page(document: &Document, page_id: lopdf::ObjectId, rect: &PdfRect) -> Result<(), String> {
    let [left, bottom, right, top] = effective_crop_bounds(document, page_id).map_err(|_| "PDF page has an invalid effective CropBox.".to_string())?;
    if rect.x < left || rect.y < bottom || rect.x + rect.width > right || rect.y + rect.height > top { return Err("Crop rectangle must stay within every selected page's effective CropBox.".to_string()); }
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

    fn make_nested_pdf(path: &Path) {
        let mut document = Document::with_version("1.7");
        let root = document.new_object_id();
        let nested = document.new_object_id();
        let content = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, Vec::new())));
        let nested_page = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => nested, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Contents" => content,
        });
        let direct_page = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => root, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Contents" => content,
        });
        document.objects.insert(nested, dictionary! {
            "Type" => "Pages", "Parent" => root, "Kids" => vec![Object::Reference(nested_page)], "Count" => 1,
        }.into());
        document.objects.insert(root, dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference(nested), Object::Reference(direct_page)], "Count" => 2,
        }.into());
        let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => root });
        document.trailer.set("Root", catalog);
        document.save(path).expect("nested fixture PDF should save");
    }

    fn make_duplicate_page_kids_pdf(path: &Path) {
        make_pdf_with_pages(path, 2);
        let mut document = Document::load(path).expect("fixture should load");
        let pages = document.get_pages().values().copied().collect::<Vec<_>>();
        let pages_root = document
            .trailer
            .get(b"Root")
            .unwrap()
            .as_reference()
            .unwrap();
        let pages_root = document.get_dictionary(pages_root).unwrap().get(b"Pages").unwrap().as_reference().unwrap();
        document.get_dictionary_mut(pages_root).unwrap().set(
            "Kids",
            vec![Object::Reference(pages[0]), Object::Reference(pages[0])],
        );
        document.save(path).expect("duplicate page-tree fixture should save");
    }

    fn encrypt_pdf(path: &Path) {
        let mut document = Document::load(path).expect("fixture should load");
        document.trailer.set(
            "ID",
            Object::Array(vec![
                Object::String(b"toolbox-test-id-1".to_vec(), lopdf::StringFormat::Literal),
                Object::String(b"toolbox-test-id-2".to_vec(), lopdf::StringFormat::Literal),
            ]),
        );
        let encryption_state = lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V2 {
            document: &document,
            owner_password: "owner",
            user_password: "user",
            key_length: 128,
            permissions: lopdf::Permissions::all(),
        })
        .expect("encryption fixture should create an encryption state");
        document.encrypt(&encryption_state).expect("fixture should encrypt");
        document.save(path).expect("encrypted fixture should save");
    }

    fn add_page_annotation(path: &Path, page_index: usize, annotation: Object) {
        let mut document = Document::load(path).expect("fixture should load");
        let page = document.get_pages().values().copied().collect::<Vec<_>>()[page_index];
        let mut annotation = annotation;
        if let Object::Dictionary(dictionary) = &mut annotation {
            dictionary.set("P", Object::Reference(page));
        }
        let annotation_id = document.add_object(annotation);
        document.get_dictionary_mut(page).unwrap().set("Annots", vec![Object::Reference(annotation_id)]);
        document.save(path).expect("annotated fixture should save");
    }

    fn make_navigation_and_form_pdf(path: &Path) {
        make_pdf_with_pages(path, 2);
        let mut document = Document::load(path).expect("fixture should load");
        let pages = document.get_pages().values().copied().collect::<Vec<_>>();
        let link = document.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => vec![20.into(), 20.into(), 80.into(), 50.into()],
            "P" => Object::Reference(pages[1]),
            "Dest" => vec![Object::Reference(pages[0]), Object::Name(b"Fit".to_vec())],
        });
        let widget = document.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Widget",
            "FT" => "Tx",
            "Rect" => vec![90.into(), 20.into(), 180.into(), 50.into()],
            "P" => Object::Reference(pages[0]),
        });
        document.get_dictionary_mut(pages[0]).unwrap().set("Annots", vec![Object::Reference(widget)]);
        document.get_dictionary_mut(pages[1]).unwrap().set("Annots", vec![Object::Reference(link)]);

        let names = document.add_object(dictionary! {
            "Dests" => dictionary! {
                "first" => vec![Object::Reference(pages[0]), Object::Name(b"Fit".to_vec())],
            },
        });
        let outlines = document.add_object(dictionary! {
            "Type" => "Outlines",
            "Count" => 0,
        });
        let acro_form = document.add_object(dictionary! {
            "Fields" => vec![Object::Reference(widget)],
        });
        let catalog = document.trailer.get(b"Root").unwrap().as_reference().unwrap();
        document.get_dictionary_mut(catalog).unwrap().set("Names", names);
        document.get_dictionary_mut(catalog).unwrap().set("Outlines", outlines);
        document.get_dictionary_mut(catalog).unwrap().set("AcroForm", acro_form);
        document.save(path).expect("navigation and form fixture should save");
    }

    fn assert_reordered_navigation_and_form_references(path: &Path, original_pages: &[ObjectId], widget: ObjectId) {
        let document = Document::load(path).expect("output should load");
        let pages = document.get_pages().values().copied().collect::<Vec<_>>();
        assert_eq!(pages, vec![original_pages[1], original_pages[0]]);

        let catalog_id = document.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let catalog = document.get_dictionary(catalog_id).unwrap();
        let names = catalog.get(b"Names").unwrap().as_reference().unwrap();
        let dests = document.get_dictionary(names).unwrap().get(b"Dests").unwrap().as_dict().unwrap();
        assert_eq!(dests.get(b"first").unwrap().as_array().unwrap()[0].as_reference().unwrap(), original_pages[0]);

        assert!(catalog.get(b"Outlines").is_ok());
        let acro_form = catalog.get(b"AcroForm").unwrap().as_reference().unwrap();
        let fields = document.get_dictionary(acro_form).unwrap().get(b"Fields").unwrap().as_array().unwrap();
        assert_eq!(fields, &[Object::Reference(widget)]);
        assert_eq!(document.get_dictionary(widget).unwrap().get(b"P").unwrap().as_reference().unwrap(), original_pages[0]);
        assert_eq!(document.get_dictionary(pages[1]).unwrap().get(b"Annots").unwrap().as_array().unwrap(), &[Object::Reference(widget)]);
        let link = document.get_dictionary(pages[0]).unwrap().get(b"Annots").unwrap().as_array().unwrap()[0].as_reference().unwrap();
        let link = document.get_dictionary(link).unwrap();
        assert_eq!(link.get(b"P").unwrap().as_reference().unwrap(), original_pages[1]);
        assert_eq!(link.get(b"Dest").unwrap().as_array().unwrap()[0].as_reference().unwrap(), original_pages[0]);
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

        assert!(outcome.error().is_some());
        assert!(outcome.output_paths().is_empty());
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

        assert!(outcome.error().is_some());
        assert!(outcome.output_paths().is_empty());
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

        assert!(sign_outcome.error().is_some());
        assert!(edit_outcome.error().is_some());
        assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn encrypted_session_and_edit_report_unlock_before_empty_page_checks() {
        let source = temp_path("encrypted-editor-entry.pdf");
        let output_dir = temp_path("encrypted-editor-entry-output");
        std::fs::create_dir_all(&output_dir).expect("output directory should be created");
        make_pdf_with_pages(&source, 1);
        encrypt_pdf(&source);
        assert!(Document::load(&source).unwrap().is_encrypted());
        let original = std::fs::read(&source).unwrap();

        let session_outcome = apply_session(
            &PdfEditSessionRequest {
                paths: vec![source.clone()],
                plan: PdfEditSessionPlan { page_order: vec![], delete_pages: vec![], rotate_pages: vec![], operations: vec![] },
                output_location: location(&output_dir),
            },
            source.clone(),
        );
        let edit_outcome = edit(
            &EditPdfRequest {
                paths: vec![source.clone()],
                mode: "shape".to_string(),
                text: String::new(),
                pages: None,
                rectangle: PdfRect { x: 10.0, y: 10.0, width: 100.0, height: 30.0 },
                output_location: location(&output_dir),
            },
            source.clone(),
        );

        for (name, outcome) in [("session", session_outcome), ("edit", edit_outcome)] {
            assert!(outcome.error().is_some_and(|error| error.message == "Unlock the PDF before editing"), "{name}: {:?}", outcome.error());
            assert!(outcome.output_paths().is_empty(), "{name} must not produce an output");
        }
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn sign_appends_to_indirect_contents_and_preserves_the_prior_stream() {
        let source = temp_path("indirect-contents-sign.pdf");
        let output_dir = temp_path("indirect-contents-sign-output");
        std::fs::create_dir_all(&output_dir).expect("output directory should be created");
        make_pdf_with_pages(&source, 1);
        let source_document = Document::load(&source).unwrap();
        let source_page = source_document.get_pages().values().next().copied().unwrap();
        let prior_contents = source_document.get_dictionary(source_page).unwrap().get(b"Contents").unwrap().as_reference().unwrap();

        let outcome = sign(
            &SignPdfRequest {
                paths: vec![source.clone()],
                page: 0,
                text: "signed".to_string(),
                signature_path: None,
                rectangle: PdfRect { x: 10.0, y: 10.0, width: 100.0, height: 30.0 },
                scope: PageScope::All,
                output_location: editor_request_location(&output_dir),
            },
            source.clone(),
        );

        assert!(outcome.error().is_none(), "sign failed: {:?}", outcome.error());
        let output = outcome.output_paths().first().expect("sign should produce an output");
        let output_document = Document::load(output).unwrap();
        let output_page = output_document.get_pages().values().next().copied().unwrap();
        let contents = output_document.get_dictionary(output_page).unwrap().get(b"Contents").unwrap().as_array().unwrap();
        assert_eq!(contents.first().unwrap().as_reference().unwrap(), prior_contents);
        assert!(String::from_utf8_lossy(&output_document.get_page_content(output_page)).contains("source"));
        assert!(String::from_utf8_lossy(&output_document.get_page_content(output_page)).contains("signed"));

        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(output);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn standalone_sign_scope_all_only_signs_the_requested_page() {
        let source = temp_path("sign-requested-page.pdf");
        let output_dir = temp_path("sign-requested-page-output");
        std::fs::create_dir_all(&output_dir).expect("output directory should be created");
        make_pdf_with_pages(&source, 3);

        let outcome = sign(
            &SignPdfRequest {
                paths: vec![source.clone()], page: 1, text: "target signature".into(), signature_path: None,
                rectangle: PdfRect { x: 10.0, y: 10.0, width: 100.0, height: 30.0 }, scope: PageScope::All,
                output_location: editor_request_location(&output_dir),
            },
            source.clone(),
        );

        assert!(outcome.error().is_none(), "sign failed: {:?}", outcome.error());
        let output = outcome.output_paths().first().expect("sign should produce an output");
        let document = Document::load(output).unwrap();
        for (index, page_id) in document.get_pages().values().copied().enumerate() {
            let content_bytes = document.get_page_content(page_id);
            let content = String::from_utf8_lossy(&content_bytes);
            assert_eq!(content.contains("target signature"), index == 1, "page {index} content: {content}");
        }

        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(output);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn overlays_clone_inherited_indirect_resources_before_adding_entries() {
        let source = temp_path("inherited-resources.pdf");
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier" });
        let inherited_fonts = document.add_object(dictionary! { "F0" => font_id, "Fedit" => font_id, "Fnum" => font_id, "Fsig" => font_id, "Fwm" => font_id });
        let existing_xobject = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
        }, vec![0, 0, 0])));
        let inherited_xobjects = document.add_object(dictionary! { "Existing" => existing_xobject, "Iwm" => existing_xobject, "Isig" => existing_xobject });
        let existing_state = document.add_object(dictionary! { "Type" => "ExtGState", "ca" => 0.25, "CA" => 0.25 });
        let inherited_states = document.add_object(dictionary! { "GSwm" => existing_state });
        let inherited_resources = document.add_object(dictionary! {
            "Font" => inherited_fonts,
            "XObject" => inherited_xobjects,
            "ExtGState" => inherited_states,
        });
        let content = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, b"BT /F0 12 Tf 10 10 Td (source) Tj ET".to_vec())));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => content,
        });
        document.objects.insert(pages_id, dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1,
            "Resources" => inherited_resources,
        }.into());
        let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog);

        apply_overlay(&mut document, &[page_id], &PdfOverlay::PageNumbers {
            start_number: Some(1), font_size: Some(12), position: None, pages: None,
        }).unwrap();
        apply_overlay(&mut document, &[page_id], &PdfOverlay::Edit {
            mode: PdfEditMode::Text, text: "edit".to_string(), pages: None,
            rectangle: PdfRect { x: 10.0, y: 10.0, width: 20.0, height: 20.0 },
        }).unwrap();
        apply_overlay(&mut document, &[page_id], &PdfOverlay::Sign {
            page: 0, text: "signed".to_string(), signature_path: None,
            rectangle: PdfRect { x: 10.0, y: 40.0, width: 100.0, height: 30.0 }, scope: PageScope::All,
        }).unwrap();
        apply_overlay(&mut document, &[page_id], &PdfOverlay::Watermark {
            text: "mark".to_string(), opacity: 50, position: None, logo_path: None, pages: None,
        }).unwrap();
        add_xobject(&mut document, page_id, "Added", existing_xobject).unwrap();

        let page = document.get_dictionary(page_id).unwrap();
        let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
        let fonts = resources.get(b"Font").unwrap().as_dict().unwrap();
        assert!(fonts.has(b"F0"));
        for name in [b"Fnum".as_slice(), b"Fedit", b"Fsig", b"Fwm", b"Fnum1", b"Fedit1", b"Fsig1", b"Fwm1"] { assert!(fonts.has(name), "missing font {name:?}"); }
        let xobjects = resources.get(b"XObject").unwrap().as_dict().unwrap();
        assert!(xobjects.has(b"Existing"));
        assert!(xobjects.has(b"Added"));
        let states = resources.get(b"ExtGState").unwrap().as_dict().unwrap();
        assert!(states.has(b"GSwm"));
        assert!(states.has(b"GSwm1"));
        let content_bytes = document.get_page_content(page_id);
        let content = String::from_utf8_lossy(&content_bytes);
        for name in ["/Fnum1", "/Fedit1", "/Fsig1", "/Fwm1", "/GSwm1"] { assert!(content.contains(name), "missing generated resource reference {name}: {content}"); }

        let _ = std::fs::remove_file(source);
    }

    #[test]
    fn crop_rectangle_sets_only_the_crop_box_and_keeps_every_other_page_box() {
        let source = temp_path("crop-box-family.pdf");
        let output_dir = temp_path("crop-box-family-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_pdf_with_pages(&source, 1);
        let mut document = Document::load(&source).unwrap();
        let page = document.get_pages().values().next().copied().unwrap();
        document.get_dictionary_mut(page).unwrap().set("CropBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
        document.get_dictionary_mut(page).unwrap().set("BleedBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
        document.get_dictionary_mut(page).unwrap().set("TrimBox", vec![50.into(), 50.into(), 150.into(), 200.into()]);
        document.get_dictionary_mut(page).unwrap().set("ArtBox", vec![500.into(), 700.into(), 700.into(), 900.into()]);
        document.save(&source).unwrap();

        let outcome = crop(&CropPdfRequest {
            paths: vec![source.clone()],
            // Fractions of the displayed 612 x 792 page, top-left origin: user space 100,100 to 300,400.
            spec: CropSpec::Rectangle { left: 100.0 / 612.0, top: 392.0 / 792.0, width: 200.0 / 612.0, height: 300.0 / 792.0 },
            scope: RangedPageScope::All,
            output_location: editor_request_location(&output_dir),
        }, source.clone());
        assert!(outcome.error().is_none(), "crop failed: {:?}", outcome.error());
        let output_document = Document::load(outcome.output_paths().first().unwrap()).unwrap();
        let page = output_document.get_pages().values().next().copied().unwrap();
        assert_box_near(page_box(&output_document, page, b"CropBox"), [100.0, 100.0, 300.0, 400.0], "rectangle");
        assert_eq!(page_box(&output_document, page, b"MediaBox"), [0.0, 0.0, 612.0, 792.0]);
        assert_eq!(page_box(&output_document, page, b"BleedBox"), [0.0, 0.0, 612.0, 792.0]);
        assert_eq!(page_box(&output_document, page, b"TrimBox"), [50.0, 50.0, 150.0, 200.0]);
        assert_eq!(page_box(&output_document, page, b"ArtBox"), [500.0, 700.0, 700.0, 900.0]);

        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(outcome.output_paths().first().unwrap());
        let _ = std::fs::remove_dir(output_dir);
    }

    fn page_box(document: &Document, page: ObjectId, key: &[u8]) -> [f32; 4] {
        let values = document.get_dictionary(page).unwrap().get(key).unwrap().as_array().unwrap();
        let values = values.iter().map(|value| super::super::metadata::number(value).unwrap()).collect::<Vec<_>>();
        [values[0], values[1], values[2], values[3]]
    }

    fn assert_box_near(actual: [f32; 4], expected: [f32; 4], label: &str) {
        assert!(actual.iter().zip(expected).all(|(a, e)| (a - e).abs() < 0.01), "{label}: {actual:?} != {expected:?}");
    }

    const MM: f32 = 72.0 / 25.4;

    /// One page with the given media box, every ancillary box, an optional crop box and rotation.
    fn make_boxed_pdf(path: &Path, media: [f32; 4], crop_box: Option<[f32; 4]>, rotate: Option<i64>) {
        make_pdf_with_pages(path, 1);
        let mut document = Document::load(path).unwrap();
        let page = document.get_pages().values().next().copied().unwrap();
        let page = document.get_dictionary_mut(page).unwrap();
        let real = |values: [f32; 4]| values.iter().copied().map(Object::Real).collect::<Vec<_>>();
        page.set("MediaBox", real(media));
        page.set("BleedBox", real(media));
        page.set("TrimBox", real([media[0] + 9.0, media[1] + 9.0, media[2] - 9.0, media[3] - 9.0]));
        page.set("ArtBox", real([media[0] + 18.0, media[1] + 18.0, media[2] - 18.0, media[3] - 18.0]));
        if let Some(crop_box) = crop_box { page.set("CropBox", real(crop_box)); }
        if let Some(rotate) = rotate { page.set("Rotate", rotate); }
        document.save(path).unwrap();
    }

    fn margins(top: f32, right: f32, bottom: f32, left: f32) -> CropSpec { CropSpec::Margins { top, right, bottom, left } }

    fn crop_margins(source: &Path, output_dir: &Path, spec: CropSpec) -> JobOutcome {
        crop(&CropPdfRequest {
            paths: vec![source.to_path_buf()], spec, scope: RangedPageScope::All, output_location: location(output_dir),
        }, source.to_path_buf())
    }

    #[test]
    fn crop_margins_inset_the_visible_page_and_leave_every_other_box_and_the_content() {
        let a4 = [0.0, 0.0, 595.2756, 841.8898];
        let letter = [0.0, 0.0, 612.0, 792.0];
        let offset_existing_crop = [0.0, 0.0, 612.0, 792.0];
        for (name, media, existing, rotate, spec, expected) in [
            // Displayed top, right, bottom, left of 10, 20, 30, 40 mm on an upright A4 page.
            ("a4", a4, None, None, margins(10.0 * MM, 20.0 * MM, 30.0 * MM, 40.0 * MM), [40.0 * MM, 30.0 * MM, a4[2] - 20.0 * MM, a4[3] - 10.0 * MM]),
            ("letter", letter, None, None, margins(72.0, 36.0, 18.0, 9.0), [9.0, 18.0, 576.0, 720.0]),
            // /Rotate 90 shows the user-space left edge at the top and the top edge on the right.
            ("rotate-90", letter, None, Some(90), margins(72.0, 36.0, 18.0, 9.0), [72.0, 9.0, 594.0, 756.0]),
            ("rotate-180", letter, None, Some(180), margins(72.0, 36.0, 18.0, 9.0), [36.0, 72.0, 603.0, 774.0]),
            // /Rotate 270 shows the user-space right edge at the top and the top edge on the left.
            ("rotate-270", letter, None, Some(-90), margins(72.0, 36.0, 18.0, 9.0), [18.0, 36.0, 540.0, 783.0]),
            // An existing crop box is the starting point, not the media box.
            ("existing-crop", [-50.0, -50.0, 700.0, 900.0], Some([100.0, 100.0, 500.0, 700.0]), None, margins(10.0, 20.0, 30.0, 40.0), [140.0, 130.0, 480.0, 690.0]),
            ("existing-crop-rotated", [-50.0, -50.0, 700.0, 900.0], Some([100.0, 100.0, 500.0, 700.0]), Some(90), margins(10.0, 20.0, 30.0, 40.0), [110.0, 140.0, 470.0, 680.0]),
            // A crop box larger than the media box is clipped to it first.
            ("crop-past-media", offset_existing_crop, Some([-100.0, -100.0, 900.0, 900.0]), None, margins(1.0, 2.0, 3.0, 4.0), [4.0, 3.0, 610.0, 791.0]),
            ("zero", letter, None, None, margins(0.0, 0.0, 0.0, 0.0), [0.0, 0.0, 612.0, 792.0]),
        ] {
            let source = temp_path(&format!("crop-margins-{name}.pdf"));
            let output_dir = temp_path(&format!("crop-margins-{name}-output"));
            std::fs::create_dir_all(&output_dir).unwrap();
            make_boxed_pdf(&source, media, existing, rotate);
            let original = std::fs::read(&source).unwrap();
            let before = Document::load(&source).unwrap();
            let before_page = before.get_pages().values().next().copied().unwrap();

            let outcome = crop_margins(&source, &output_dir, spec);

            assert!(outcome.error().is_none(), "{name}: {:?}", outcome.error());
            let output = outcome.output_paths().first().cloned().unwrap();
            assert!(output.file_name().unwrap().to_string_lossy().ends_with("-cropped.pdf"), "{name}: {output:?}");
            let after = Document::load(&output).unwrap();
            let page = after.get_pages().values().next().copied().unwrap();
            assert_box_near(page_box(&after, page, b"CropBox"), expected, name);
            for key in [b"MediaBox".as_slice(), b"BleedBox", b"TrimBox", b"ArtBox"] {
                assert_eq!(page_box(&after, page, key), page_box(&before, before_page, key), "{name}: {}", String::from_utf8_lossy(key));
            }
            assert_eq!(after.get_page_content(page), before.get_page_content(before_page), "{name}: content must stay");
            assert_eq!(std::fs::read(&source).unwrap(), original, "{name}: original must stay");
            let _ = std::fs::remove_file(source);
            let _ = std::fs::remove_dir_all(output_dir);
        }
    }

    fn rectangle(left: f32, top: f32, width: f32, height: f32) -> CropSpec { CropSpec::Rectangle { left, top, width, height } }

    #[test]
    fn crop_rectangle_keeps_the_same_relative_area_of_the_displayed_page_on_any_size_or_rotation() {
        let a4 = [0.0, 0.0, 595.2756, 841.8898];
        let letter = [0.0, 0.0, 612.0, 792.0];
        // Keep from 10% in and 20% down, 50% wide and 60% tall, as seen on screen.
        let spec = rectangle(0.1, 0.2, 0.5, 0.6);
        for (name, media, existing, rotate, expected) in [
            ("a4", a4, None, None, [0.1 * a4[2], 0.2 * a4[3], 0.6 * a4[2], 0.8 * a4[3]]),
            ("letter", letter, None, None, [61.2, 158.4, 367.2, 633.6]),
            // Rotated a quarter turn, the page shows 792 wide and 612 tall; its displayed top is the user-space left edge.
            ("rotate-90", letter, None, Some(90), [122.4, 79.2, 489.6, 475.2]),
            ("rotate-180", letter, None, Some(180), [244.8, 158.4, 550.8, 633.6]),
            // At 270 the displayed top is the user-space right edge and the displayed left is the top edge.
            ("rotate-270", letter, None, Some(270), [122.4, 316.8, 489.6, 712.8]),
            // Fractions are of the visible page, so an existing crop box is the frame they measure.
            ("existing-crop", [-50.0, -50.0, 700.0, 900.0], Some([100.0, 100.0, 500.0, 700.0]), None, [140.0, 220.0, 340.0, 580.0]),
        ] {
            let source = temp_path(&format!("crop-rectangle-{name}.pdf"));
            let output_dir = temp_path(&format!("crop-rectangle-{name}-output"));
            std::fs::create_dir_all(&output_dir).unwrap();
            make_boxed_pdf(&source, media, existing, rotate);
            let original = std::fs::read(&source).unwrap();
            let before = Document::load(&source).unwrap();
            let before_page = before.get_pages().values().next().copied().unwrap();

            let outcome = crop_margins(&source, &output_dir, spec.clone());

            assert!(outcome.error().is_none(), "{name}: {:?}", outcome.error());
            let after = Document::load(outcome.output_paths().first().unwrap()).unwrap();
            let page = after.get_pages().values().next().copied().unwrap();
            assert_box_near(page_box(&after, page, b"CropBox"), expected, name);
            for key in [b"MediaBox".as_slice(), b"BleedBox", b"TrimBox", b"ArtBox"] {
                assert_eq!(page_box(&after, page, key), page_box(&before, before_page, key), "{name}: {}", String::from_utf8_lossy(key));
            }
            assert_eq!(after.get_dictionary(page).unwrap().get(b"Rotate").ok(), before.get_dictionary(before_page).unwrap().get(b"Rotate").ok(), "{name}: rotation must stay");
            assert_eq!(after.get_page_content(page), before.get_page_content(before_page), "{name}: content must stay");
            assert_eq!(std::fs::read(&source).unwrap(), original, "{name}: original must stay");
            let _ = std::fs::remove_file(source);
            let _ = std::fs::remove_dir_all(output_dir);
        }
    }

    #[test]
    fn crop_rectangle_that_is_empty_or_leaves_the_page_fails_with_invalid_input_and_writes_nothing() {
        let source = temp_path("crop-rectangle-invalid.pdf");
        let output_dir = temp_path("crop-rectangle-invalid-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_pdf_with_pages(&source, 1);
        let original = std::fs::read(&source).unwrap();
        for (name, spec) in [
            ("zero-width", rectangle(0.1, 0.1, 0.0, 0.5)),
            ("zero-height", rectangle(0.1, 0.1, 0.5, 0.0)),
            ("past-right", rectangle(0.6, 0.1, 0.5, 0.5)),
            ("past-bottom", rectangle(0.1, 0.7, 0.5, 0.5)),
            ("negative", rectangle(-0.1, 0.1, 0.5, 0.5)),
            ("nan", rectangle(f32::NAN, 0.1, 0.5, 0.5)),
        ] {
            let outcome = crop_margins(&source, &output_dir, spec);
            let error = outcome.error().unwrap_or_else(|| panic!("{name} must fail"));
            assert!(matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput), "{name}: {error:?}");
            assert!(outcome.output_paths().is_empty(), "{name}");
            assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0, "{name}");
        }
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir_all(output_dir);
    }

    #[test]
    fn crop_margins_on_annotated_pages_keep_links_comments_and_form_fields_unchanged() {
        let source = temp_path("crop-margins-annotated.pdf");
        let output_dir = temp_path("crop-margins-annotated-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_navigation_and_form_pdf(&source);
        let mut document = Document::load(&source).unwrap();
        let pages = document.get_pages().values().copied().collect::<Vec<_>>();
        let comment = document.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![300.into(), 300.into(), 320.into(), 320.into()],
            "Contents" => Object::string_literal("Check this"), "P" => Object::Reference(pages[1]),
        });
        let annots = document.get_dictionary(pages[1]).unwrap().get(b"Annots").unwrap().as_array().unwrap().clone();
        document.get_dictionary_mut(pages[1]).unwrap().set("Annots", annots.into_iter().chain([Object::Reference(comment)]).collect::<Vec<_>>());
        document.save(&source).unwrap();
        let original = std::fs::read(&source).unwrap();
        let before = Document::load(&source).unwrap();

        let outcome = crop_margins(&source, &output_dir, margins(36.0, 36.0, 36.0, 36.0));

        assert!(outcome.error().is_none(), "annotated crop failed: {:?}", outcome.error());
        let after = Document::load(outcome.output_paths().first().unwrap()).unwrap();
        for page in after.get_pages().values().copied() {
            assert_eq!(page_box(&after, page, b"CropBox"), [36.0, 36.0, 576.0, 756.0]);
            let annots = after.get_dictionary(page).unwrap().get(b"Annots").unwrap().as_array().unwrap();
            assert_eq!(annots, before.get_dictionary(page).unwrap().get(b"Annots").unwrap().as_array().unwrap());
            for annotation in annots {
                let id = annotation.as_reference().unwrap();
                assert_eq!(after.get_dictionary(id).unwrap(), before.get_dictionary(id).unwrap(), "annotation {id:?} must stay");
            }
        }
        let catalog = |document: &Document| document.get_dictionary(document.trailer.get(b"Root").unwrap().as_reference().unwrap()).unwrap().clone();
        for key in [b"AcroForm".as_slice(), b"Names", b"Outlines"] {
            assert_eq!(catalog(&after).get(key).unwrap(), catalog(&before).get(key).unwrap());
        }
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir_all(output_dir);
    }

    #[test]
    fn crop_margins_that_leave_no_area_fail_with_invalid_input_naming_the_page_and_write_nothing() {
        let source = temp_path("crop-margins-too-wide.pdf");
        let output_dir = temp_path("crop-margins-too-wide-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_pdf_with_pages(&source, 3);
        let mut document = Document::load(&source).unwrap();
        let third = document.get_pages().values().copied().nth(2).unwrap();
        document.get_dictionary_mut(third).unwrap().set("MediaBox", vec![0.into(), 0.into(), 200.into(), 792.into()]);
        document.save(&source).unwrap();
        let original = std::fs::read(&source).unwrap();

        // 250 pt from both sides fits a 612 pt Letter page but not the 200 pt third page.
        let outcome = crop_margins(&source, &output_dir, margins(0.0, 100.0, 0.0, 150.0));

        let error = outcome.error().expect("too-wide margins must fail");
        assert!(matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput), "{error:?}");
        assert!(error.message.contains("Page 3"), "{}", error.message);
        assert!(outcome.output_paths().is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);

        for (name, spec) in [("negative", margins(-1.0, 0.0, 0.0, 0.0)), ("nan", margins(0.0, f32::NAN, 0.0, 0.0))] {
            let outcome = crop_margins(&source, &output_dir, spec);
            assert!(outcome.error().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput)), "{name}: {:?}", outcome.error());
            assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0, "{name}");
        }
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir_all(output_dir);
    }

    #[test]
    fn crop_margins_outputs_never_overwrite_an_earlier_crop() {
        let source = temp_path("crop-collision.pdf");
        let output_dir = temp_path("crop-collision-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_pdf_with_pages(&source, 1);
        let stem = source.file_stem().unwrap().to_string_lossy().to_string();
        let names = (0..3).map(|_| {
            let outcome = crop_margins(&source, &output_dir, margins(10.0, 10.0, 10.0, 10.0));
            assert!(outcome.error().is_none(), "{:?}", outcome.error());
            outcome.output_paths()[0].file_name().unwrap().to_string_lossy().to_string()
        }).collect::<Vec<_>>();
        assert_eq!(names, vec![format!("{stem}-cropped.pdf"), format!("{stem}-cropped-1.pdf"), format!("{stem}-cropped-2.pdf")]);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir_all(output_dir);
    }

    #[test]
    fn crop_keeps_internal_page_links_and_rejects_malformed_page_counts_before_output() {
        for (name, action) in [("dest", false), ("action", true)] {
            let source = temp_path(&format!("internal-link-{name}.pdf"));
            let output_dir = temp_path(&format!("internal-link-{name}-output"));
            std::fs::create_dir_all(&output_dir).unwrap();
            make_pdf_with_pages(&source, 1);
            let mut document = Document::load(&source).unwrap();
            let page = document.get_pages().values().next().copied().unwrap();
            let destination = vec![Object::Reference(page), Object::Name(b"Fit".to_vec())];
            let annotation = if action {
                document.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()], "A" => dictionary! { "S" => "GoTo", "D" => destination } })
            } else {
                document.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()], "Dest" => destination })
            };
            document.get_dictionary_mut(page).unwrap().set("Annots", vec![Object::Reference(annotation)]);
            document.save(&source).unwrap();
            let outcome = crop(&CropPdfRequest {
                paths: vec![source.clone()], spec: CropSpec::Rectangle { left: 0.0, top: 0.0, width: 0.5, height: 0.5 },
                scope: RangedPageScope::All, output_location: editor_request_location(&output_dir),
            }, source.clone());
            assert!(outcome.error().is_none(), "{name}: {:?}", outcome.error());
            let output = Document::load(outcome.output_paths().first().unwrap()).unwrap();
            let link = output.get_dictionary(annotation).unwrap();
            let target = if action { link.get(b"A").unwrap().as_dict().unwrap().get(b"D").unwrap() } else { link.get(b"Dest").unwrap() };
            assert_eq!(target.as_array().unwrap()[0].as_reference().unwrap(), page, "{name}: link must still target its page");
            let _ = std::fs::remove_file(source);
            let _ = std::fs::remove_dir_all(output_dir);
        }

        for count in [-1, 2] {
            let source = temp_path(&format!("malformed-count-{count}.pdf"));
            let output_dir = temp_path(&format!("malformed-count-{count}-output"));
            std::fs::create_dir_all(&output_dir).unwrap();
            make_pdf_with_pages(&source, 1);
            let mut document = Document::load(&source).unwrap();
            let pages = document.trailer.get(b"Root").unwrap().as_reference().unwrap();
            let pages = document.get_dictionary(pages).unwrap().get(b"Pages").unwrap().as_reference().unwrap();
            document.get_dictionary_mut(pages).unwrap().set("Count", count);
            document.save(&source).unwrap();
            let outcome = crop(&CropPdfRequest {
                paths: vec![source.clone()], spec: CropSpec::Rectangle { left: 0.0, top: 0.0, width: 0.5, height: 0.5 },
                scope: RangedPageScope::All, output_location: editor_request_location(&output_dir),
            }, source.clone());
            assert!(outcome.error().is_some_and(|error| error.message.contains("page-tree")), "count {count}: {:?}", outcome.error());
            assert!(outcome.output_paths().is_empty());
            assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
            let _ = std::fs::remove_file(source);
            let _ = std::fs::remove_dir(output_dir);
        }
    }

    #[test]
    fn crop_rejects_duplicate_flat_page_tree_references_before_output() {
        let source = temp_path("duplicate-page-kid");
        let output_dir = temp_path("duplicate-page-kid-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_duplicate_page_kids_pdf(&source);
        let original = std::fs::read(&source).unwrap();

        let outcome = crop(&CropPdfRequest {
            paths: vec![source.clone()],
            spec: CropSpec::Rectangle { left: 0.0, top: 0.0, width: 0.5, height: 0.5 },
            scope: RangedPageScope::All,
            output_location: location(&output_dir),
        }, source.clone());

        assert!(outcome.error().is_some_and(|error| error.message.contains("duplicate")), "unexpected result: {:?}", outcome.error());
        assert!(outcome.output_paths().is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn crop_keeps_plain_annotations_and_preserves_the_original() {
        let source = temp_path("crop-annotation-output");
        let output_dir = temp_path("crop-annotation-output-dir");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_pdf_with_pages(&source, 1);
        add_page_annotation(&source, 0, dictionary! {
            "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![20.into(), 20.into(), 80.into(), 50.into()],
        }.into());
        let original = std::fs::read(&source).unwrap();

        let outcome = crop(&CropPdfRequest {
            paths: vec![source.clone()], spec: CropSpec::Rectangle { left: 0.0, top: 0.0, width: 0.5, height: 0.5 },
            scope: RangedPageScope::All, output_location: location(&output_dir),
        }, source.clone());

        assert!(outcome.error().is_none(), "unexpected result: {:?}", outcome.error());
        let output = Document::load(outcome.output_paths().first().unwrap()).unwrap();
        let page = output.get_pages().values().next().copied().unwrap();
        assert_eq!(output.get_dictionary(page).unwrap().get(b"Annots").unwrap().as_array().unwrap().len(), 1);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir_all(output_dir);
    }

    #[test]
    fn pure_session_reorder_allows_navigation_and_form_references() {
        let source = temp_path("navigation-form-reorder-session");
        let output_dir = temp_path("navigation-form-reorder-session-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_navigation_and_form_pdf(&source);
        let original = std::fs::read(&source).unwrap();
        let source_document = Document::load(&source).unwrap();
        let original_pages = source_document.get_pages().values().copied().collect::<Vec<_>>();
        let catalog = source_document.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let acro_form = source_document.get_dictionary(catalog).unwrap().get(b"AcroForm").unwrap().as_reference().unwrap();
        let widget = source_document.get_dictionary(acro_form).unwrap().get(b"Fields").unwrap().as_array().unwrap()[0].as_reference().unwrap();

        let outcome = apply_session(&PdfEditSessionRequest {
            paths: vec![source.clone()],
            plan: PdfEditSessionPlan { page_order: vec![1, 0], delete_pages: vec![], rotate_pages: vec![], operations: vec![] },
            output_location: location(&output_dir),
        }, source.clone());

        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let output = outcome.output_paths().first().unwrap();
        assert_reordered_navigation_and_form_references(output, &original_pages, widget);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(output);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn session_rejects_annotation_with_a_non_owner_page_reference_before_output() {
        let source = temp_path("annotation-wrong-page");
        let output_dir = temp_path("annotation-wrong-page-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_pdf_with_pages(&source, 2);
        let mut document = Document::load(&source).unwrap();
        let pages = document.get_pages().values().copied().collect::<Vec<_>>();
        let annotation = document.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Text",
            "Rect" => vec![20.into(), 20.into(), 80.into(), 50.into()],
            "P" => Object::Reference(pages[1]),
        });
        document.get_dictionary_mut(pages[0]).unwrap().set("Annots", vec![Object::Reference(annotation)]);
        document.save(&source).unwrap();
        let original = std::fs::read(&source).unwrap();

        let outcome = apply_session(&PdfEditSessionRequest {
            paths: vec![source.clone()],
            plan: PdfEditSessionPlan { page_order: vec![1, 0], delete_pages: vec![], rotate_pages: vec![], operations: vec![] },
            output_location: location(&output_dir),
        }, source.clone());

        assert!(outcome.error().is_some_and(|error| error.message.contains("owning page")), "unexpected result: {:?}", outcome.error());
        assert!(outcome.output_paths().is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn malformed_annotation_entries_and_orphan_widgets_fail_closed_without_output() {
        for (name, annotation) in [
            ("direct-entry", dictionary! { "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![20.into(), 20.into(), 80.into(), 50.into()] }.into()),
            ("orphan-widget", dictionary! { "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "Rect" => vec![20.into(), 20.into(), 80.into(), 50.into()] }.into()),
        ] {
            let source = temp_path(&format!("malformed-annotation-{name}"));
            let output_dir = temp_path(&format!("malformed-annotation-{name}-output"));
            std::fs::create_dir_all(&output_dir).unwrap();
            make_pdf_with_pages(&source, 1);
            let mut document = Document::load(&source).unwrap();
            let page = document.get_pages().values().next().copied().unwrap();
            document.get_dictionary_mut(page).unwrap().set("Annots", vec![annotation]);
            document.save(&source).unwrap();
            let original = std::fs::read(&source).unwrap();

            let outcome = crop(&CropPdfRequest {
                paths: vec![source.clone()], spec: CropSpec::Rectangle { left: 0.0, top: 0.0, width: 0.5, height: 0.5 },
                scope: RangedPageScope::All, output_location: location(&output_dir),
            }, source.clone());

            assert!(outcome.error().is_some(), "{name}: malformed annotation must fail");
            assert!(outcome.output_paths().is_empty());
            assert_eq!(std::fs::read(&source).unwrap(), original);
            assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
            let _ = std::fs::remove_file(source);
            let _ = std::fs::remove_dir(output_dir);
        }
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
    fn session_plan_rejects_crop_and_overlay_targets_deleted_pages() {
        let crop = PdfEditSessionPlan {
            page_order: vec![],
            delete_pages: vec![0],
            rotate_pages: vec![],
            operations: vec![PdfEditOperation::Crop {
                rectangle: PdfRect { x: 0.0, y: 0.0, width: 10.0, height: 10.0 },
                scope: PageScope::Selected { pages: vec![0] },
            }],
        };
        assert!(normalize_session_plan(&crop, 2).is_err());

        let overlay = PdfEditSessionPlan {
            page_order: vec![],
            delete_pages: vec![1],
            rotate_pages: vec![],
            operations: vec![PdfEditOperation::Overlay {
                overlay: PdfOverlay::PageNumbers { start_number: None, font_size: None, position: None, pages: Some(vec![1]) },
            }],
        };
        assert!(normalize_session_plan(&overlay, 2).is_err());
    }

    #[test]
    fn session_rejects_deleted_page_targets_before_reserving_output() {
        for (name, operation) in [
            ("crop", PdfEditOperation::Crop {
                rectangle: PdfRect { x: 0.0, y: 0.0, width: 10.0, height: 10.0 },
                scope: PageScope::Selected { pages: vec![0] },
            }),
            ("overlay", PdfEditOperation::Overlay {
                overlay: PdfOverlay::PageNumbers { start_number: None, font_size: None, position: None, pages: Some(vec![0]) },
            }),
        ] {
            let source = temp_path(&format!("deleted-target-{name}.pdf"));
            let output_dir = temp_path(&format!("deleted-target-{name}-output"));
            std::fs::create_dir_all(&output_dir).expect("output directory should be created");
            make_pdf(&source);
            let outcome = apply_session(&PdfEditSessionRequest {
                paths: vec![source.clone()],
                plan: PdfEditSessionPlan { page_order: vec![], delete_pages: vec![0], rotate_pages: vec![], operations: vec![operation] },
                output_location: location(&output_dir),
            }, source.clone());
            assert!(outcome.error().is_some_and(|error| error.message.contains("scheduled for deletion")), "{name}: {:?}", outcome.error());
            assert!(outcome.output_paths().is_empty());
            assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
            let _ = std::fs::remove_file(source);
            let _ = std::fs::remove_dir(output_dir);
        }
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
        assert!(outcome.error().is_none(), "composed session failed: {:?}", outcome.error());
        let output = outcome.output_paths().first().expect("session should produce one output");
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
    fn session_rejects_overlay_outside_the_effective_crop_box_without_public_output() {
        let source = temp_path("crop-overlay-current-box.pdf");
        let output_dir = temp_path("crop-overlay-current-box-output");
        std::fs::create_dir_all(&output_dir).expect("output directory should be created");
        make_pdf_with_pages(&source, 1);
        let mut source_document = Document::load(&source).expect("source fixture should load");
        let page = source_document.get_pages().values().next().copied().unwrap();
        source_document.get_dictionary_mut(page).unwrap().set("CropBox", vec![50.into(), 50.into(), 550.into(), 742.into()]);
        source_document.save(&source).expect("cropped source fixture should save");
        let source_bytes = std::fs::read(&source).expect("source should be readable");
        let outcome = apply_session(&PdfEditSessionRequest {
            paths: vec![source.clone()],
            plan: PdfEditSessionPlan {
                page_order: vec![], delete_pages: vec![], rotate_pages: vec![],
                operations: vec![PdfEditOperation::Overlay {
                    overlay: PdfOverlay::Edit {
                        mode: PdfEditMode::Text, text: "outside crop".into(), pages: Some(vec![0]),
                        rectangle: PdfRect { x: 0., y: 0., width: 20., height: 20. },
                    },
                }],
            },
            output_location: location(&output_dir),
        }, source.clone());

        assert!(outcome.error().is_some_and(|error| error.message.contains("CropBox")), "unexpected result: {:?}", outcome.error());
        assert!(outcome.output_paths().is_empty());
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        assert_eq!(std::fs::read(&source).expect("source should remain readable"), source_bytes);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }

    fn add_catalog_marker(path: &Path, key: &[u8], value: Object) {
        let mut document = Document::load(path).expect("fixture should load");
        let catalog = document.trailer.get(b"Root").unwrap().as_reference().unwrap();
        document.get_dictionary_mut(catalog).unwrap().set(key, value);
        document.save(path).expect("fixture marker should save");
    }

    fn empty_plan() -> PdfEditSessionPlan {
        PdfEditSessionPlan { page_order: vec![], delete_pages: vec![], rotate_pages: vec![], operations: vec![] }
    }

    #[test]
    fn session_rejects_signed_pdfs_before_reserving_output() {
        let source = temp_path("signed-session.pdf");
        let output_dir = temp_path("signed-session-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_pdf(&source);
        let signature = {
            let mut document = Document::load(&source).unwrap();
            document.add_object(dictionary! { "Type" => "Sig" })
        };
        add_catalog_marker(&source, b"Perms", dictionary! { "DocMDP" => signature }.into());

        let outcome = apply_session(&PdfEditSessionRequest {
            paths: vec![source.clone()], plan: empty_plan(), output_location: location(&output_dir),
        }, source.clone());

        assert!(outcome.error().is_some_and(|error| error.message.contains("Digitally signed")), "{:?}", outcome.error());
        assert!(outcome.output_paths().is_empty());
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn session_overlay_rejects_navigational_pdfs_before_reserving_output() {
        let source = temp_path("navigation-session-overlay.pdf");
        let output_dir = temp_path("navigation-session-overlay-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_pdf(&source);
        let outlines = {
            let mut document = Document::load(&source).unwrap();
            document.add_object(dictionary! { "Type" => "Outlines", "Count" => 0 })
        };
        add_catalog_marker(&source, b"Outlines", Object::Reference(outlines));
        let original = std::fs::read(&source).unwrap();

        let outcome = apply_session(&PdfEditSessionRequest {
            paths: vec![source.clone()],
            plan: PdfEditSessionPlan {
                page_order: vec![1, 0],
                delete_pages: vec![],
                rotate_pages: vec![],
                operations: vec![PdfEditOperation::Overlay {
                    overlay: PdfOverlay::PageNumbers { start_number: None, font_size: None, position: None, pages: None },
                }],
            },
            output_location: location(&output_dir),
        }, source.clone());

        assert!(outcome.error().is_some_and(|error| error.message.contains("navigation")), "{:?}", outcome.error());
        assert!(outcome.output_paths().is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn session_rejects_tagged_pdfs_before_reserving_output() {
        let source = temp_path("tagged-session.pdf");
        let output_dir = temp_path("tagged-session-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_pdf(&source);
        let structure = {
            let mut document = Document::load(&source).unwrap();
            document.add_object(dictionary! { "Type" => "StructTreeRoot", "K" => vec![] })
        };
        add_catalog_marker(&source, b"StructTreeRoot", Object::Reference(structure));

        let outcome = apply_session(&PdfEditSessionRequest {
            paths: vec![source.clone()], plan: empty_plan(), output_location: location(&output_dir),
        }, source.clone());

        assert!(outcome.error().is_some_and(|error| error.message.contains("tagged PDF")), "{:?}", outcome.error());
        assert!(outcome.output_paths().is_empty());
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn session_rejects_nested_page_trees_before_output() {
        let source = temp_path("nested-session.pdf");
        let output_dir = temp_path("nested-session-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_nested_pdf(&source);

        let outcome = apply_session(&PdfEditSessionRequest {
            paths: vec![source.clone()], plan: empty_plan(), output_location: location(&output_dir),
        }, source.clone());

        assert!(outcome.error().is_some_and(|error| error.message.contains("nested or unsupported")), "{:?}", outcome.error());
        assert!(outcome.output_paths().is_empty());
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }
}
