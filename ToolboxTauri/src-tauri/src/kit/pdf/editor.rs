use image::codecs::jpeg::JpegEncoder;
use image::ImageEncoder;
use lopdf::{dictionary, Document, Object, ObjectId};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::{SkipCode, SkipReason, ToolError};
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
pub struct ResizePdfRequest {
    pub paths: Vec<PathBuf>,
    pub size: TargetSize,
    pub orientation: ResizeOrientation,
    pub placement: ResizePlacement,
    pub scope: RangedPageScope,
    pub output_location: OutputLocation,
}

/// The paper every page in scope is set to: a preset from the shared paper-size table, by id, or a custom
/// width and height in points, within the table's custom bounds.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetSize {
    Preset(String),
    Custom { width: f32, height: f32 },
}

/// How the paper is turned: `Match` gives each page the paper in its own displayed orientation;
/// `Portrait` and `Landscape` turn every page in scope that way.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResizeOrientation {
    Match,
    Portrait,
    Landscape,
}

/// Where the visible page lands on the paper: `Fit` scales it uniformly by the smaller ratio and centres it;
/// `ActualSize` keeps it at scale 1, centred, so whatever lies past the new page is cut off.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResizePlacement {
    Fit,
    ActualSize,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperSizes {
    /// Pages within this many points of the paper on both sides already count as that paper.
    pub tolerance_points: f32,
    /// The smallest and largest side a custom size may have, in points.
    pub custom_bounds_points: CustomBounds,
    pub sizes: Vec<PaperSize>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct CustomBounds {
    pub min: f32,
    pub max: f32,
}

/// A paper size in points, portrait for the presets.
#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct PaperSize {
    pub id: String,
    pub name: String,
    pub width: f32,
    pub height: f32,
}

/// The paper sizes the Crop and Resize view offers, shared with the UI so both use the same points.
pub(crate) fn paper_sizes() -> &'static PaperSizes {
    static SIZES: std::sync::OnceLock<PaperSizes> = std::sync::OnceLock::new();
    SIZES.get_or_init(|| serde_json::from_str(include_str!("../../../../shared/paper-sizes.json")).expect("the shared paper-size table is valid JSON"))
}

/// A length in points as the view's sentences show it: millimetres to a tenth, then points, with thousands separators.
fn describe_bound(points: f32) -> String {
    let group = |value: f32, decimals: usize| {
        let text = format!("{value:.decimals$}");
        let text = if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text };
        let (whole, fraction) = text.split_once('.').map_or((text.as_str(), None), |(whole, fraction)| (whole, Some(fraction)));
        let digits: Vec<char> = whole.chars().collect();
        let mut grouped = String::new();
        for (index, digit) in digits.iter().enumerate() {
            if index > 0 && (digits.len() - index) % 3 == 0 { grouped.push(','); }
            grouped.push(*digit);
        }
        fraction.map_or(grouped.clone(), |fraction| format!("{grouped}.{fraction}"))
    };
    format!("{} mm ({} pt)", group(points * 25.4 / 72.0, 1), group(points, 1))
}

impl TargetSize {
    fn paper(&self) -> Result<PaperSize, ToolError> {
        match self {
            TargetSize::Preset(id) => paper_sizes().sizes.iter().find(|paper| paper.id == *id).cloned()
                .ok_or_else(|| ToolError::invalid_input(format!("\"{id}\" isn't a paper size this tool offers."))),
            &TargetSize::Custom { width, height } => {
                let CustomBounds { min, max } = paper_sizes().custom_bounds_points;
                for (side, length) in [("Width", width), ("Height", height)] {
                    if length.is_nan() || length < min { return Err(ToolError::invalid_input(format!("{side} must be at least {}.", describe_bound(min)))); }
                    if length > max { return Err(ToolError::invalid_input(format!("{side} can be at most {}.", describe_bound(max)))); }
                }
                let millimetres = |points: f32| { let text = format!("{:.1}", points * 25.4 / 72.0); text.strip_suffix(".0").map(str::to_string).unwrap_or(text) };
                Ok(PaperSize { id: "custom".into(), name: format!("{} \u{d7} {} mm", millimetres(width), millimetres(height)), width, height })
            }
        }
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
        let (selected, notes) = match ranged_pages(&request.scope, pages.len())? {
            Ok(selection) => selection,
            Err(reason) => return Ok(Edited::Skipped(reason)),
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
    let user_unit = page_user_unit(document, page_id)?;
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

/// The 0-based pages a ranged scope selects in a file of `page_count` pages, with notes on clamped ranges,
/// or the reason to skip a file that has none of the typed pages.
fn ranged_pages(scope: &RangedPageScope, page_count: usize) -> Result<Result<(Vec<usize>, Vec<String>), SkipReason>, ToolError> {
    Ok(match scope {
        RangedPageScope::All => Ok(((0..page_count).collect(), Vec::new())),
        RangedPageScope::Selected { pages } => Ok((selected_indices(pages, page_count).map_err(ToolError::invalid_input)?, Vec::new())),
        RangedPageScope::Range { ranges } => PageRanges::parse(ranges).map_err(ToolError::invalid_input)?.clamp_or_skip(page_count).map(|clamped| (clamped.page_indices(), clamped.notes)),
    })
}

/// The page's /UserUnit: how many points one user-space unit spans.
fn page_user_unit(document: &Document, page_id: ObjectId) -> Result<f32, ToolError> {
    let user_unit = match document.get_dictionary(page_id).ok().and_then(|page| page.get(b"UserUnit").ok()) {
        Some(value) => super::resolve(document, value).and_then(super::metadata::number).map_err(ToolError::processing)?,
        None => 1.0,
    };
    if !user_unit.is_finite() || user_unit <= 0.0 { return Err(ToolError::processing("PDF page has an invalid UserUnit.")); }
    Ok(user_unit)
}

pub fn resize(request: &ResizePdfRequest, input: PathBuf) -> JobOutcome {
    transform_or_skip_pdf(input, &request.output_location, "-resized", PdfMutationIntent::ValidateOnly, SignedPdf::RemoveSignature, |document, pages, _preflight| {
        let paper = &request.size.paper()?;
        let (selected, notes) = match ranged_pages(&request.scope, pages.len())? {
            Ok(selection) => selection,
            Err(reason) => return Ok(Edited::Skipped(reason)),
        };
        let mut placements = Vec::new();
        let mut unchanged = Vec::new();
        for &page_index in &selected {
            match resize_placement(document, pages[page_index], page_index, paper, request.orientation, request.placement)? {
                Some(placement) => placements.push((page_index, placement)),
                None => unchanged.push(page_index),
            }
        }
        if placements.is_empty() {
            let message = if selected.len() == pages.len() {
                format!("Every page is already {}. Nothing was saved.", paper.name)
            } else {
                format!("{} {} already {}. Nothing was saved.", describe_pages(&unchanged), if unchanged.len() == 1 { "is" } else { "are" }, paper.name)
            };
            return Ok(Edited::Skipped(SkipReason::new(SkipCode::NoEffectiveChange, message)));
        }
        // Annotations shared between pages are moved once, by the first page that lists them.
        let mut moved_annotations = std::collections::HashSet::new();
        let changed: Vec<usize> = placements.iter().map(|(page_index, _)| *page_index).collect();
        for (page_index, placement) in placements {
            place_page_content(document, pages[page_index], &placement, &mut moved_annotations)?;
        }
        let mut detail = vec![format!("{} changed to {}.", describe_pages(&changed), paper.name)];
        if !unchanged.is_empty() {
            detail.push(format!("{} {} already {}.", describe_pages(&unchanged), if unchanged.len() == 1 { "was" } else { "were" }, paper.name));
        }
        detail.extend(notes);
        Ok(Edited::Saved(detail.join(" ")))
    })
}

/// Where one page's visible area lands on the new paper, in the page's user space.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PagePlacement {
    /// The page's visible area before resizing: [x0, y0, x1, y1].
    visible: [f32; 4],
    /// The new page size in user space: width and height before /Rotate is applied.
    target: (f32, f32),
    /// Uniform scale and the translation that centres the scaled visible area on the new page.
    scale: f32,
    offset: (f32, f32),
}

/// The new page for one page, or None when the page is already that paper as displayed.
fn resize_placement(document: &Document, page_id: ObjectId, page_index: usize, paper: &PaperSize, orientation: ResizeOrientation, placement: ResizePlacement) -> Result<Option<PagePlacement>, ToolError> {
    let visible @ [x0, y0, x1, y1] = effective_crop_bounds(document, page_id).map_err(ToolError::processing)?;
    let user_unit = page_user_unit(document, page_id)?;
    let rotation = page_rotation(document, page_id);
    if rotation % 90 != 0 { return Err(ToolError::processing(format!("Page {} has an unsupported rotation of {rotation} degrees.", page_index + 1))); }
    let quarter_turn = rotation % 180 == 90;
    let (width, height) = (x1 - x0, y1 - y0);
    if width <= 0.0 || height <= 0.0 { return Err(ToolError::processing(format!("Page {} has no visible area.", page_index + 1))); }
    // The page as displayed, in points.
    let (shown_width, shown_height) = if quarter_turn { (height * user_unit, width * user_unit) } else { (width * user_unit, height * user_unit) };
    let (paper_width, paper_height) = match orientation {
        // A landscape page gets landscape paper; a portrait or square page keeps the paper's portrait.
        ResizeOrientation::Match if (shown_width > shown_height) != (paper.width > paper.height) => (paper.height, paper.width),
        ResizeOrientation::Match => (paper.width, paper.height),
        ResizeOrientation::Portrait => (paper.width.min(paper.height), paper.width.max(paper.height)),
        ResizeOrientation::Landscape => (paper.width.max(paper.height), paper.width.min(paper.height)),
    };
    let tolerance = paper_sizes().tolerance_points;
    if (shown_width - paper_width).abs() <= tolerance && (shown_height - paper_height).abs() <= tolerance { return Ok(None); }
    // The new page in user space: /Rotate stays, so a quarter-turned page stores the paper's sides swapped.
    let (target_width, target_height) = if quarter_turn { (paper_height / user_unit, paper_width / user_unit) } else { (paper_width / user_unit, paper_height / user_unit) };
    let scale = match placement {
        ResizePlacement::Fit => (target_width / width).min(target_height / height),
        // Scale 1 in user space is actual size, because the page keeps its UserUnit. The new page boxes cut off
        // whatever lies past them.
        ResizePlacement::ActualSize => 1.0,
    };
    let offset = ((target_width - scale * width) / 2.0 - scale * x0, (target_height - scale * height) / 2.0 - scale * y0);
    Ok(Some(PagePlacement { visible, target: (target_width, target_height), scale, offset }))
}

/// Sets every page box to the new paper, draws the old visible area scaled and centred on it, and moves the
/// page's annotations by the same mapping so links, comments and form fields stay on their content.
fn place_page_content(document: &mut Document, page_id: ObjectId, placement: &PagePlacement, moved_annotations: &mut std::collections::HashSet<ObjectId>) -> Result<(), ToolError> {
    let processing = |error: String| ToolError::processing(error);
    let [x0, y0, x1, y1] = placement.visible;
    let PagePlacement { scale, offset: (e, f), target: (target_width, target_height), .. } = *placement;
    // New streams wrap the existing content, which may be shared with other pages, so it is never rewritten.
    // The clip keeps content that was cropped away hidden.
    let prefix = format!("q\n{scale} 0 0 {scale} {e} {f} cm\n{x0} {y0} {} {} re W n\n", x1 - x0, y1 - y0);
    let prefix_id = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, prefix.into_bytes())));
    let suffix_id = document.add_object(Object::Stream(lopdf::Stream::new(dictionary! {}, b"\nQ\n".to_vec())));
    let prior = document.get_dictionary(page_id).map_err(|error| processing(error.to_string()))?.get(b"Contents").ok().cloned();
    let existing = match prior {
        Some(Object::Array(contents)) => contents,
        Some(Object::Reference(existing)) => vec![Object::Reference(existing)],
        Some(existing) => vec![Object::Reference(document.add_object(existing))],
        None => Vec::new(),
    };
    let contents: Vec<Object> = std::iter::once(Object::Reference(prefix_id)).chain(existing).chain([Object::Reference(suffix_id)]).collect();
    let paper_box = || Object::Array(vec![Object::Integer(0), Object::Integer(0), Object::Real(target_width), Object::Real(target_height)]);
    let page = document.get_dictionary_mut(page_id).map_err(|error| processing(error.to_string()))?;
    page.set("Contents", contents);
    for key in ["MediaBox", "CropBox", "BleedBox", "TrimBox", "ArtBox"] { page.set(key, paper_box()); }
    let annotations = match page.get(b"Annots").ok().cloned() {
        Some(Object::Reference(array_id)) => document.get_object(array_id).and_then(Object::as_array).map(|array| array.clone()).unwrap_or_default(),
        Some(Object::Array(array)) => array,
        _ => Vec::new(),
    };
    let map = |x: f32, y: f32| (scale * x + e, scale * y + f);
    // The shared PDF preflight only lets indirect annotations through.
    for annotation_id in annotations.iter().filter_map(|annotation| annotation.as_reference().ok()) {
        if !moved_annotations.insert(annotation_id) { continue; }
        if let Ok(dictionary) = document.get_dictionary_mut(annotation_id) { move_annotation(dictionary, map); }
    }
    Ok(())
}

/// Maps the annotation's rectangle and point lists. An array with a value that isn't a number is left as is.
fn move_annotation(annotation: &mut lopdf::Dictionary, map: impl Fn(f32, f32) -> (f32, f32)) {
    let map_points = |values: &[Object]| -> Option<Vec<Object>> {
        if values.len() % 2 != 0 { return None; }
        let numbers = values.iter().map(|value| super::metadata::number(value).ok()).collect::<Option<Vec<f32>>>()?;
        Some(numbers.chunks(2).flat_map(|pair| { let (x, y) = map(pair[0], pair[1]); [Object::Real(x), Object::Real(y)] }).collect())
    };
    if let Ok(Object::Array(rect)) = annotation.get_mut(b"Rect") {
        if let Some(mapped) = map_points(rect) {
            // Keep the rectangle normalised: lower-left then upper-right.
            if let [Object::Real(ax), Object::Real(ay), Object::Real(bx), Object::Real(by)] = mapped.as_slice() {
                *rect = vec![Object::Real(ax.min(*bx)), Object::Real(ay.min(*by)), Object::Real(ax.max(*bx)), Object::Real(ay.max(*by))];
            }
        }
    }
    for key in [&b"QuadPoints"[..], b"L", b"Vertices", b"CL"] {
        if let Ok(Object::Array(points)) = annotation.get_mut(key) {
            if let Some(mapped) = map_points(points) { *points = mapped; }
        }
    }
    if let Ok(Object::Array(strokes)) = annotation.get_mut(b"InkList") {
        let mapped: Option<Vec<Object>> = strokes.iter().map(|stroke| stroke.as_array().ok().and_then(|points| map_points(points)).map(Object::Array)).collect();
        if let Some(mapped) = mapped { *strokes = mapped; }
    }
}

/// Names pages by 0-based index in runs, such as "Page 4" or "Pages 3–14 and 20".
fn describe_pages(indices: &[usize]) -> String {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &index in indices {
        let page = index + 1;
        match runs.last_mut() {
            Some((_, end)) if *end + 1 == page => *end = page,
            _ => runs.push((page, page)),
        }
    }
    let parts: Vec<String> = runs.iter().map(|&(start, end)| if start == end { start.to_string() } else { format!("{start}\u{2013}{end}") }).collect();
    let listed = match parts.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    format!("{} {listed}", if indices.len() == 1 { "Page" } else { "Pages" })
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

    fn resize_to(source: &Path, output_dir: &Path, preset: &str, scope: RangedPageScope) -> JobOutcome {
        resize_with(source, output_dir, TargetSize::Preset(preset.into()), ResizeOrientation::Match, ResizePlacement::Fit)(scope)
    }

    fn resize_with(source: &Path, output_dir: &Path, size: TargetSize, orientation: ResizeOrientation, placement: ResizePlacement) -> impl Fn(RangedPageScope) -> JobOutcome {
        let (source, output_dir) = (source.to_path_buf(), output_dir.to_path_buf());
        move |scope| resize(&ResizePdfRequest {
            paths: vec![source.clone()], size: size.clone(), orientation, placement, scope, output_location: location(&output_dir),
        }, source.clone())
    }

    fn stream_text(document: &Document, object: &Object) -> String {
        let stream = document.get_object(object.as_reference().unwrap()).unwrap().as_stream().unwrap();
        String::from_utf8(stream.decompressed_content().unwrap_or_else(|_| stream.content.clone())).unwrap()
    }

    /// The scale and translation of the resize wrapper, and the clip rectangle, read from the page's first content stream.
    fn resize_wrapper(document: &Document, page: ObjectId) -> ([f32; 3], [f32; 4]) {
        let contents = document.get_dictionary(page).unwrap().get(b"Contents").unwrap().as_array().unwrap();
        let prefix = stream_text(document, &contents[0]);
        assert_eq!(stream_text(document, contents.last().unwrap()).trim(), "Q");
        let numbers = prefix.split_whitespace().filter_map(|token| token.parse::<f32>().ok()).collect::<Vec<_>>();
        assert!(prefix.starts_with("q\n") && prefix.contains(" cm\n") && prefix.trim_end().ends_with("re W n"), "{prefix}");
        assert_eq!((numbers[1], numbers[2], numbers[0]), (0.0, 0.0, numbers[3]), "uniform scale: {prefix}");
        ([numbers[0], numbers[4], numbers[5]], [numbers[6], numbers[7], numbers[8], numbers[9]])
    }

    fn only_page(document: &Document) -> ObjectId { document.get_pages().values().copied().next().unwrap() }

    fn assert_near(actual: f32, expected: f32, label: &str) {
        assert!((actual - expected).abs() < 1e-3, "{label}: {actual} != {expected}");
    }

    #[test]
    fn resize_sets_each_preset_exactly_on_every_box_and_fits_the_page_centred() {
        let output_dir = temp_path("resize-presets-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        // Neither shape matches any preset, so every preset changes both.
        for (shape, media) in [("portrait", [0.0, 0.0, 500.0, 700.0]), ("landscape", [10.0, 20.0, 710.0, 520.0])] {
            let source = temp_path(&format!("resize-preset-{shape}.pdf"));
            make_boxed_pdf(&source, media, None, None);
            let original = std::fs::read(&source).unwrap();
            for paper in &paper_sizes().sizes {
                let outcome = resize_to(&source, &output_dir, &paper.id, RangedPageScope::All);
                assert!(outcome.error().is_none(), "{shape} {}: {:?}", paper.id, outcome.error());
                assert_eq!(outcome.detail, format!("Page 1 changed to {}.", paper.name));
                let output = &outcome.output_paths()[0];
                assert!(output.file_name().unwrap().to_string_lossy().contains("-resized"));
                let after = Document::load(output).unwrap();
                let page = only_page(&after);
                let (width, height) = if shape == "landscape" { (paper.height, paper.width) } else { (paper.width, paper.height) };
                for key in [&b"MediaBox"[..], b"CropBox", b"BleedBox", b"TrimBox", b"ArtBox"] {
                    let actual = page_box(&after, page, key);
                    for (value, expected) in actual.iter().zip([0.0, 0.0, width, height]) {
                        assert_near(*value, expected, &format!("{shape} {} {}", paper.id, String::from_utf8_lossy(key)));
                    }
                }
                let ([scale, e, f], clip) = resize_wrapper(&after, page);
                let (source_width, source_height) = (media[2] - media[0], media[3] - media[1]);
                assert_near(scale, (width / source_width).min(height / source_height), "fit uses the smaller ratio");
                // The scaled page sits centred: equal gaps on each side.
                let (left, bottom) = (scale * media[0] + e, scale * media[1] + f);
                let (right, top) = (scale * media[2] + e, scale * media[3] + f);
                assert_near(left, width - right, &format!("{shape} {} centred horizontally", paper.id));
                assert_near(bottom, height - top, &format!("{shape} {} centred vertically", paper.id));
                assert!(left.abs() < 1e-3 || bottom.abs() < 1e-3, "one side touches the paper: nothing is cut off");
                assert_eq!(clip, [media[0], media[1], source_width, source_height]);
                assert!(after.extract_text(&[1]).unwrap().contains("source"), "text stays extractable");
            }
            assert_eq!(std::fs::read(&source).unwrap(), original, "the original is never modified");
            let _ = std::fs::remove_file(source);
        }
        let _ = std::fs::remove_dir_all(output_dir);
    }

    #[test]
    fn resize_matches_each_page_as_displayed_including_rotated_pages() {
        let output_dir = temp_path("resize-rotated-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        let a4 = &paper_sizes().sizes.iter().find(|paper| paper.id == "a4").unwrap();
        // A portrait page turned a quarter is displayed landscape, so it gets landscape A4: in user space the
        // MediaBox keeps the page's own portrait sides and /Rotate stays.
        for (rotate, media, user_space) in [
            (90, [0.0, 0.0, 500.0, 700.0], (a4.width, a4.height)),
            (270, [0.0, 0.0, 500.0, 700.0], (a4.width, a4.height)),
            (180, [0.0, 0.0, 500.0, 700.0], (a4.width, a4.height)),
            (90, [0.0, 0.0, 700.0, 500.0], (a4.height, a4.width)),
        ] {
            let source = temp_path(&format!("resize-rotated-{rotate}-{}.pdf", media[2]));
            make_boxed_pdf(&source, media, None, Some(rotate));
            let outcome = resize_to(&source, &output_dir, "a4", RangedPageScope::All);
            assert!(outcome.error().is_none(), "{rotate}: {:?}", outcome.error());
            let after = Document::load(&outcome.output_paths()[0]).unwrap();
            let page = only_page(&after);
            assert_box_near(page_box(&after, page, b"MediaBox"), [0.0, 0.0, user_space.0, user_space.1], &format!("rotate {rotate} {media:?}"));
            assert_eq!(after.get_dictionary(page).unwrap().get(b"Rotate").unwrap().as_i64().unwrap(), rotate);
            let ([scale, _, _], _) = resize_wrapper(&after, page);
            assert_near(scale, (user_space.0 / (media[2] - media[0])).min(user_space.1 / (media[3] - media[1])), "rotated fit");
            let _ = std::fs::remove_file(source);
        }
        let _ = std::fs::remove_dir_all(output_dir);
    }

    #[test]
    fn resize_fits_only_the_visible_crop_box_and_clips_what_was_cropped_away() {
        let source = temp_path("resize-cropped.pdf");
        let output_dir = temp_path("resize-cropped-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_boxed_pdf(&source, [0.0, 0.0, 612.0, 792.0], Some([100.0, 100.0, 400.0, 500.0]), None);
        let outcome = resize_to(&source, &output_dir, "a4", RangedPageScope::All);
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let after = Document::load(&outcome.output_paths()[0]).unwrap();
        let page = only_page(&after);
        let ([scale, e, f], clip) = resize_wrapper(&after, page);
        let a4 = (595.2756_f32, 841.8898_f32);
        let expected = (a4.0 / 300.0).min(a4.1 / 400.0);
        assert_near(scale, expected, "scale of the 300 x 400 visible area");
        assert_near(e, (a4.0 - expected * 300.0) / 2.0 - expected * 100.0, "x offset");
        assert_near(f, (a4.1 - expected * 400.0) / 2.0 - expected * 100.0, "y offset");
        assert_eq!(clip, [100.0, 100.0, 300.0, 400.0]);
        assert_box_near(page_box(&after, page, b"CropBox"), [0.0, 0.0, a4.0, a4.1], "crop box is the new paper");
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir_all(output_dir);
    }

    #[test]
    fn resize_moves_links_comments_and_form_fields_with_the_content_and_keeps_shared_content_intact() {
        let source = temp_path("resize-annotated.pdf");
        let output_dir = temp_path("resize-annotated-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_navigation_and_form_pdf(&source);
        let mut document = Document::load(&source).unwrap();
        let pages = document.get_pages().values().copied().collect::<Vec<_>>();
        let highlight = document.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Highlight", "Rect" => vec![100.into(), 600.into(), 200.into(), 620.into()],
            "QuadPoints" => vec![100.into(), 620.into(), 200.into(), 620.into(), 100.into(), 600.into(), 200.into(), 600.into()],
        });
        // Listed on both pages: it must move once, not twice.
        let shared_comment = document.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![300.into(), 300.into(), 320.into(), 320.into()],
        });
        let ink = document.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Ink", "Rect" => vec![10.into(), 10.into(), 60.into(), 60.into()],
            "InkList" => vec![Object::Array(vec![10.into(), 10.into(), 60.into(), 60.into()])],
        });
        let first = document.get_dictionary(pages[0]).unwrap().get(b"Annots").unwrap().as_array().unwrap().clone();
        document.get_dictionary_mut(pages[0]).unwrap().set("Annots", first.into_iter().chain([Object::Reference(highlight), Object::Reference(shared_comment), Object::Reference(ink)]).collect::<Vec<_>>());
        let second = document.get_dictionary(pages[1]).unwrap().get(b"Annots").unwrap().as_array().unwrap().clone();
        let second_annots = document.add_object(second.into_iter().chain([Object::Reference(shared_comment)]).collect::<Vec<_>>());
        document.get_dictionary_mut(pages[1]).unwrap().set("Annots", Object::Reference(second_annots));
        document.save(&source).unwrap();
        let original = std::fs::read(&source).unwrap();
        let before = Document::load(&source).unwrap();

        let outcome = resize_to(&source, &output_dir, "a4", RangedPageScope::All);

        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        assert_eq!(outcome.detail, "Pages 1\u{2013}2 changed to A4.");
        let after = Document::load(&outcome.output_paths()[0]).unwrap();
        let after_pages = after.get_pages().values().copied().collect::<Vec<_>>();
        let ([scale, e, f], _) = resize_wrapper(&after, after_pages[0]);
        assert_eq!(resize_wrapper(&after, after_pages[1]).0, [scale, e, f], "both Letter pages use one mapping");
        let numbers = |document: &Document, dictionary: &lopdf::Dictionary, key: &[u8]| dictionary.get(key).unwrap().as_array().unwrap().iter()
            .map(|value| super::super::metadata::number(super::super::resolve(document, value).unwrap()).unwrap()).collect::<Vec<_>>();
        let mapped = |values: Vec<f32>| values.chunks(2).flat_map(|pair| [scale * pair[0] + e, scale * pair[1] + f]).collect::<Vec<_>>();
        let assert_moved = |label: &str, before_values: Vec<f32>, after_values: Vec<f32>| {
            for (actual, expected) in after_values.iter().zip(mapped(before_values)) { assert_near(*actual, expected, label); }
        };
        for (label, id) in [("widget", 0usize), ("highlight", 1), ("comment", 2)] {
            let id = after.get_dictionary(after_pages[0]).unwrap().get(b"Annots").unwrap().as_array().unwrap()[id].as_reference().unwrap();
            assert_moved(label, numbers(&before, before.get_dictionary(id).unwrap(), b"Rect"), numbers(&after, after.get_dictionary(id).unwrap(), b"Rect"));
        }
        assert_moved("quad points", numbers(&before, before.get_dictionary(highlight).unwrap(), b"QuadPoints"), numbers(&after, after.get_dictionary(highlight).unwrap(), b"QuadPoints"));
        let ink = after.get_dictionary(ink).unwrap();
        assert_moved("ink rect", vec![10.0, 10.0, 60.0, 60.0], numbers(&after, ink, b"Rect"));
        let stroke = ink.get(b"InkList").unwrap().as_array().unwrap()[0].as_array().unwrap().iter().map(|value| super::super::metadata::number(value).unwrap()).collect();
        assert_moved("ink stroke", vec![10.0, 10.0, 60.0, 60.0], stroke);
        let link = after.get_object(second_annots).unwrap().as_array().unwrap()[0].as_reference().unwrap();
        assert_moved("link", numbers(&before, before.get_dictionary(link).unwrap(), b"Rect"), numbers(&after, after.get_dictionary(link).unwrap(), b"Rect"));
        // Link destinations, the outline, named destinations and the form stay wired to the same objects.
        assert_eq!(after.get_dictionary(link).unwrap().get(b"Dest").unwrap(), before.get_dictionary(link).unwrap().get(b"Dest").unwrap());
        let catalog = |document: &Document| document.get_dictionary(document.trailer.get(b"Root").unwrap().as_reference().unwrap()).unwrap().clone();
        for key in [b"AcroForm".as_slice(), b"Names", b"Outlines"] {
            assert_eq!(catalog(&after).get(key).unwrap(), catalog(&before).get(key).unwrap());
        }
        // The shared source content stream is wrapped, not rewritten.
        let contents = after.get_dictionary(after_pages[0]).unwrap().get(b"Contents").unwrap().as_array().unwrap().clone();
        assert_eq!(stream_text(&after, &contents[1]), "BT /F0 12 Tf 36 720 Td (source) Tj ET");
        for page in [1, 2] { assert!(after.extract_text(&[page]).unwrap().contains("source"), "page {page} text"); }
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir_all(output_dir);
    }

    fn make_sized_pages_pdf(path: &Path, sizes: &[(f32, f32)]) {
        make_pdf_with_pages(path, sizes.len());
        let mut document = Document::load(path).unwrap();
        let pages = document.get_pages().values().copied().collect::<Vec<_>>();
        for (page, (width, height)) in pages.into_iter().zip(sizes) {
            document.get_dictionary_mut(page).unwrap().set("MediaBox", vec![0.into(), 0.into(), Object::Real(*width), Object::Real(*height)]);
        }
        document.save(path).unwrap();
    }

    #[test]
    fn resize_leaves_pages_already_at_the_target_unchanged_and_skips_files_with_nothing_to_change() {
        use crate::kit::contracts::OutcomeState;
        let output_dir = temp_path("resize-unchanged-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        let (a4, letter, a4_landscape) = ((595.2756, 841.8898), (612.0, 792.0), (841.8898, 595.2756));

        let mixed = temp_path("resize-mixed.pdf");
        make_sized_pages_pdf(&mixed, &[a4, letter, a4_landscape, letter]);
        let before = Document::load(&mixed).unwrap();
        let outcome = resize_to(&mixed, &output_dir, "a4", RangedPageScope::All);
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        assert_eq!(outcome.detail, "Pages 2 and 4 changed to A4. Pages 1 and 3 were already A4.");
        let after = Document::load(&outcome.output_paths()[0]).unwrap();
        let (before_pages, after_pages) = (before.get_pages().values().copied().collect::<Vec<_>>(), after.get_pages().values().copied().collect::<Vec<_>>());
        for index in [0, 2] {
            assert_eq!(after.get_dictionary(after_pages[index]).unwrap(), before.get_dictionary(before_pages[index]).unwrap(), "page {} is unchanged", index + 1);
        }
        assert_box_near(page_box(&after, after_pages[1], b"MediaBox"), [0.0, 0.0, a4.0, a4.1], "page 2");

        // Within the 1.5 pt tolerance a page already counts as A4.
        let all_a4 = temp_path("resize-all-a4.pdf");
        make_sized_pages_pdf(&all_a4, &[(595.0, 842.5), a4_landscape]);
        let outcome = resize_to(&all_a4, &output_dir, "a4", RangedPageScope::All);
        match &outcome.state {
            OutcomeState::Skipped { reason } => {
                assert_eq!(reason.code, SkipCode::NoEffectiveChange);
                assert_eq!(reason.message, "Every page is already A4. Nothing was saved.");
            }
            state => panic!("expected an all-A4 file to be skipped, got {state:?}"),
        }
        assert!(outcome.output_paths().is_empty());

        // Only the pages in the typed range count, and the range is clamped to this file.
        let scoped = temp_path("resize-scoped-a4.pdf");
        make_sized_pages_pdf(&scoped, &[letter, a4, a4]);
        let outcome = resize_to(&scoped, &output_dir, "a4", RangedPageScope::Range { ranges: "2-9".into() });
        match &outcome.state {
            OutcomeState::Skipped { reason } => {
                assert_eq!(reason.code, SkipCode::NoEffectiveChange);
                assert_eq!(reason.message, "Pages 2\u{2013}3 are already A4. Nothing was saved.");
            }
            state => panic!("expected the scoped A4 pages to be skipped, got {state:?}"),
        }
        let outcome = resize_to(&scoped, &output_dir, "a4", RangedPageScope::Range { ranges: "1, 3-9".into() });
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        assert!(outcome.detail.starts_with("Page 1 changed to A4. Page 3 was already A4."), "{}", outcome.detail);
        assert!(outcome.detail.contains("9"), "the clamping note names the typed range: {}", outcome.detail);
        let after = Document::load(&outcome.output_paths()[0]).unwrap();
        let pages = after.get_pages().values().copied().collect::<Vec<_>>();
        assert_box_near(page_box(&after, pages[0], b"MediaBox"), [0.0, 0.0, a4.0, a4.1], "page 1 in range");
        assert!(after.get_dictionary(pages[1]).unwrap().get(b"Contents").unwrap().as_reference().is_ok(), "page 2 is outside the range and untouched");

        let outcome = resize_to(&scoped, &output_dir, "a4", RangedPageScope::Range { ranges: "7-".into() });
        assert!(matches!(&outcome.state, OutcomeState::Skipped { reason } if reason.code == SkipCode::RangeEmptyAfterClamping), "{:?}", outcome.state);

        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 2, "only the two changed files were written");
        for path in [mixed, all_a4, scoped] { let _ = std::fs::remove_file(path); }
        let _ = std::fs::remove_dir_all(output_dir);
    }

    fn custom(width_mm: f32, height_mm: f32) -> TargetSize { TargetSize::Custom { width: width_mm * MM, height: height_mm * MM } }

    /// The page as displayed, in points: its MediaBox with /Rotate applied.
    fn displayed_size(document: &Document, page: ObjectId) -> (f32, f32) {
        let [x0, y0, x1, y1] = page_box(document, page, b"MediaBox");
        let rotate = document.get_dictionary(page).unwrap().get(b"Rotate").ok().and_then(|value| value.as_i64().ok()).unwrap_or(0);
        if rotate % 180 == 90 { (y1 - y0, x1 - x0) } else { (x1 - x0, y1 - y0) }
    }

    #[test]
    fn resize_sets_a_custom_size_exactly_on_every_box_and_names_it_in_millimetres() {
        let output_dir = temp_path("resize-custom-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        let source = temp_path("resize-custom.pdf");
        make_boxed_pdf(&source, [0.0, 0.0, 500.0, 700.0], None, None);
        let original = std::fs::read(&source).unwrap();
        // 200 x 250 mm and 8 x 10 in, each sent in points.
        for (size, (width, height), name) in [
            (custom(200.0, 250.0), (200.0 * MM, 250.0 * MM), "200 \u{d7} 250 mm"),
            (TargetSize::Custom { width: 8.0 * 72.0, height: 10.0 * 72.0 }, (576.0, 720.0), "203.2 \u{d7} 254 mm"),
        ] {
            let outcome = resize_with(&source, &output_dir, size, ResizeOrientation::Match, ResizePlacement::Fit)(RangedPageScope::All);
            assert!(outcome.error().is_none(), "{name}: {:?}", outcome.error());
            assert_eq!(outcome.detail, format!("Page 1 changed to {name}."));
            let after = Document::load(&outcome.output_paths()[0]).unwrap();
            let page = only_page(&after);
            for key in [&b"MediaBox"[..], b"CropBox", b"BleedBox", b"TrimBox", b"ArtBox"] {
                assert_box_near(page_box(&after, page, key), [0.0, 0.0, width, height], &format!("{name} {}", String::from_utf8_lossy(key)));
            }
            let ([scale, _, _], _) = resize_wrapper(&after, page);
            assert_near(scale, (width / 500.0).min(height / 700.0), "fit");
        }
        // A page already at the custom size, within the tolerance, is left alone.
        let already = temp_path("resize-custom-already.pdf");
        make_boxed_pdf(&already, [0.0, 0.0, 200.0 * MM + 1.0, 250.0 * MM - 1.0], None, None);
        let outcome = resize_with(&already, &output_dir, custom(200.0, 250.0), ResizeOrientation::Match, ResizePlacement::Fit)(RangedPageScope::All);
        assert!(matches!(&outcome.state, crate::kit::contracts::OutcomeState::Skipped { reason } if reason.message == "Every page is already 200 \u{d7} 250 mm. Nothing was saved."), "{:?}", outcome.state);
        assert_eq!(std::fs::read(&source).unwrap(), original, "the original is never modified");
        for path in [source, already] { let _ = std::fs::remove_file(path); }
        let _ = std::fs::remove_dir_all(output_dir);
    }

    #[test]
    fn resize_custom_sizes_must_be_within_72_and_14400_points_or_fail_with_invalid_input() {
        let bounds = &paper_sizes().custom_bounds_points;
        assert_eq!((bounds.min, bounds.max), (72.0, 14_400.0), "the bounds the UI reads from the shared table");
        let output_dir = temp_path("resize-bounds-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        let source = temp_path("resize-bounds.pdf");
        make_pdf_with_pages(&source, 1);
        for (width, height, message) in [
            (71.9, 500.0, "Width must be at least 25.4 mm (72 pt)."),
            (500.0, 71.9, "Height must be at least 25.4 mm (72 pt)."),
            (0.0, 500.0, "Width must be at least 25.4 mm (72 pt)."),
            (-200.0, 500.0, "Width must be at least 25.4 mm (72 pt)."),
            (14_400.5, 500.0, "Width can be at most 5,080 mm (14,400 pt)."),
            (500.0, 14_400.5, "Height can be at most 5,080 mm (14,400 pt)."),
            (f32::NAN, 500.0, "Width must be at least 25.4 mm (72 pt)."),
        ] {
            let outcome = resize_with(&source, &output_dir, TargetSize::Custom { width, height }, ResizeOrientation::Match, ResizePlacement::Fit)(RangedPageScope::All);
            let error = outcome.error().unwrap_or_else(|| panic!("{width} x {height} should fail"));
            assert!(matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput), "{width} x {height}: {error:?}");
            assert_eq!(error.message, message);
        }
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0, "nothing is written for a size out of bounds");
        // The bounds themselves are allowed.
        let outcome = resize_with(&source, &output_dir, TargetSize::Custom { width: 72.0, height: 14_400.0 }, ResizeOrientation::Match, ResizePlacement::Fit)(RangedPageScope::All);
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let after = Document::load(&outcome.output_paths()[0]).unwrap();
        assert_box_near(page_box(&after, only_page(&after), b"MediaBox"), [0.0, 0.0, 72.0, 14_400.0], "smallest and largest side");
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir_all(output_dir);
    }

    #[test]
    fn resize_portrait_and_landscape_turn_every_page_and_match_keeps_each_page_as_displayed() {
        let output_dir = temp_path("resize-orientation-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        let source = temp_path("resize-orientation.pdf");
        // Portrait, landscape, portrait turned a quarter (shown landscape) and landscape turned a quarter (shown portrait).
        make_sized_pages_pdf(&source, &[(500.0, 700.0), (700.0, 500.0), (500.0, 700.0), (700.0, 500.0)]);
        let mut document = Document::load(&source).unwrap();
        let pages = document.get_pages().values().copied().collect::<Vec<_>>();
        for page in [pages[2], pages[3]] { document.get_dictionary_mut(page).unwrap().set("Rotate", 90); }
        document.save(&source).unwrap();
        let a4 = (595.2756_f32, 841.8898_f32);
        let (portrait, landscape) = (a4, (a4.1, a4.0));
        // A landscape custom size is turned the same way: Portrait gives its short side as the width.
        let (wide, tall) = ((300.0 * MM, 200.0 * MM), (200.0 * MM, 300.0 * MM));
        for (size, orientation, expected) in [
            (TargetSize::Preset("a4".into()), ResizeOrientation::Portrait, [portrait; 4]),
            (TargetSize::Preset("a4".into()), ResizeOrientation::Landscape, [landscape; 4]),
            (TargetSize::Preset("a4".into()), ResizeOrientation::Match, [portrait, landscape, landscape, portrait]),
            (custom(300.0, 200.0), ResizeOrientation::Portrait, [tall; 4]),
            (custom(300.0, 200.0), ResizeOrientation::Landscape, [wide; 4]),
            (custom(300.0, 200.0), ResizeOrientation::Match, [tall, wide, wide, tall]),
        ] {
            let label = format!("{size:?} {orientation:?}");
            let outcome = resize_with(&source, &output_dir, size, orientation, ResizePlacement::Fit)(RangedPageScope::All);
            assert!(outcome.error().is_none(), "{label}: {:?}", outcome.error());
            let after = Document::load(&outcome.output_paths()[0]).unwrap();
            for (index, (page, expected)) in after.get_pages().values().copied().zip(expected).enumerate() {
                let (width, height) = displayed_size(&after, page);
                assert_near(width, expected.0, &format!("{label} page {} width", index + 1));
                assert_near(height, expected.1, &format!("{label} page {} height", index + 1));
                let rotate = after.get_dictionary(page).unwrap().get(b"Rotate").ok().and_then(|value| value.as_i64().ok()).unwrap_or(0);
                assert_eq!(rotate, if index >= 2 { 90 } else { 0 }, "/Rotate stays");
            }
        }
        // Forcing an orientation also changes a page that is already the paper the other way round.
        let turned = temp_path("resize-orientation-turned.pdf");
        make_sized_pages_pdf(&turned, &[landscape]);
        let outcome = resize_with(&turned, &output_dir, TargetSize::Preset("a4".into()), ResizeOrientation::Portrait, ResizePlacement::Fit)(RangedPageScope::All);
        assert_eq!(outcome.detail, "Page 1 changed to A4.");
        let outcome = resize_with(&turned, &output_dir, TargetSize::Preset("a4".into()), ResizeOrientation::Landscape, ResizePlacement::Fit)(RangedPageScope::All);
        assert!(matches!(&outcome.state, crate::kit::contracts::OutcomeState::Skipped { .. }), "{:?}", outcome.state);
        for path in [source, turned] { let _ = std::fs::remove_file(path); }
        let _ = std::fs::remove_dir_all(output_dir);
    }

    #[test]
    fn resize_actual_size_keeps_scale_one_centred_clips_to_the_new_page_and_moves_annotations_the_same_way() {
        let output_dir = temp_path("resize-actual-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        let a4 = (595.2756_f32, 841.8898_f32);
        // A page larger than A4 with an offset origin, a smaller one, and a quarter-turned one.
        for (label, media, rotate, user_space) in [
            ("larger", [50.0, 60.0, 1050.0, 1260.0], None, a4),
            ("smaller", [0.0, 0.0, 300.0, 400.0], None, a4),
            ("turned", [0.0, 0.0, 1000.0, 700.0], Some(90), (a4.1, a4.0)),
        ] {
            let source = temp_path(&format!("resize-actual-{label}.pdf"));
            make_boxed_pdf(&source, media, None, rotate);
            let mut document = Document::load(&source).unwrap();
            let page = only_page(&document);
            let comment = document.add_object(dictionary! {
                "Type" => "Annot", "Subtype" => "Square",
                "Rect" => vec![Object::Real(media[0] + 10.0), Object::Real(media[1] + 20.0), Object::Real(media[0] + 110.0), Object::Real(media[1] + 70.0)],
                "Vertices" => vec![Object::Real(media[0] + 10.0), Object::Real(media[1] + 20.0)],
            });
            document.get_dictionary_mut(page).unwrap().set("Annots", vec![Object::Reference(comment)]);
            document.save(&source).unwrap();
            let original = std::fs::read(&source).unwrap();

            let outcome = resize_with(&source, &output_dir, TargetSize::Preset("a4".into()), ResizeOrientation::Portrait, ResizePlacement::ActualSize)(RangedPageScope::All);

            assert!(outcome.error().is_none(), "{label}: {:?}", outcome.error());
            let after = Document::load(&outcome.output_paths()[0]).unwrap();
            let page = only_page(&after);
            let (width, height) = displayed_size(&after, page);
            assert_near(width, a4.0, &format!("{label} shown portrait width"));
            assert_near(height, a4.1, &format!("{label} shown portrait height"));
            for key in [&b"MediaBox"[..], b"CropBox", b"BleedBox", b"TrimBox", b"ArtBox"] {
                assert_box_near(page_box(&after, page, key), [0.0, 0.0, user_space.0, user_space.1], &format!("{label}: the new page clips the content"));
            }
            let ([scale, e, f], clip) = resize_wrapper(&after, page);
            assert_eq!(scale, 1.0, "{label}: actual size");
            // Centred: the old page's centre lands on the new page's centre.
            assert_near((media[0] + media[2]) / 2.0 + e, user_space.0 / 2.0, &format!("{label} centred horizontally"));
            assert_near((media[1] + media[3]) / 2.0 + f, user_space.1 / 2.0, &format!("{label} centred vertically"));
            assert_eq!(clip, [media[0], media[1], media[2] - media[0], media[3] - media[1]], "{label}: the old visible area stays the inner clip");
            let annotation = after.get_dictionary(comment).unwrap();
            let numbers = |key: &[u8]| annotation.get(key).unwrap().as_array().unwrap().iter().map(|value| super::super::metadata::number(value).unwrap()).collect::<Vec<_>>();
            let expected_rect = [media[0] + 10.0 + e, media[1] + 20.0 + f, media[0] + 110.0 + e, media[1] + 70.0 + f];
            for (actual, expected) in numbers(b"Rect").iter().zip(expected_rect) { assert_near(*actual, expected, &format!("{label} annotation rect")); }
            for (actual, expected) in numbers(b"Vertices").iter().zip(&expected_rect[..2]) { assert_near(*actual, *expected, &format!("{label} annotation vertices")); }
            assert_eq!(std::fs::read(&source).unwrap(), original, "the original is never modified");
            let _ = std::fs::remove_file(source);
        }
        let _ = std::fs::remove_dir_all(output_dir);
    }

    #[test]
    fn resize_rejects_an_unknown_preset_and_never_overwrites_earlier_outputs() {
        let source = temp_path("resize-collision.pdf");
        let output_dir = temp_path("resize-collision-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_pdf_with_pages(&source, 1);
        let outcome = resize_to(&source, &output_dir, "b5", RangedPageScope::All);
        assert!(outcome.error().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput)), "{:?}", outcome.error());
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let stem = source.file_stem().unwrap().to_string_lossy().to_string();
        let names = (0..3).map(|_| {
            let outcome = resize_to(&source, &output_dir, "a4", RangedPageScope::All);
            assert!(outcome.error().is_none(), "{:?}", outcome.error());
            outcome.output_paths()[0].file_name().unwrap().to_string_lossy().to_string()
        }).collect::<Vec<_>>();
        assert_eq!(names, vec![format!("{stem}-resized.pdf"), format!("{stem}-resized-1.pdf"), format!("{stem}-resized-2.pdf")]);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir_all(output_dir);
    }

    #[test]
    fn resize_paper_table_is_the_shared_one_the_ui_reads() {
        let table = paper_sizes();
        assert_eq!(table.tolerance_points, 1.5);
        let ids = table.sizes.iter().map(|paper| (paper.id.as_str(), paper.name.as_str())).collect::<Vec<_>>();
        assert_eq!(ids, vec![("a3", "A3"), ("a4", "A4"), ("a5", "A5"), ("letter", "Letter"), ("legal", "Legal"), ("tabloid", "Tabloid")]);
        for paper in &table.sizes { assert!(paper.width > 0.0 && paper.width < paper.height, "{} is portrait", paper.id); }
        let a4 = &table.sizes[1];
        assert_near(a4.width, 210.0 * MM, "A4 width");
        assert_near(a4.height, 297.0 * MM, "A4 height");
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
