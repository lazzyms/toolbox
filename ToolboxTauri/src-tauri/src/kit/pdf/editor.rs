use image::codecs::jpeg::JpegEncoder;
use image::ImageEncoder;
use lopdf::{dictionary, Document, Object, ObjectId};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::{SkipReason, ToolError};
use super::metadata::{effective_crop_bounds, page_rotation};
use super::page_ranges::PageRanges;
use super::{mutation_preflight, note_signature_removed, remove_signatures, PdfMutationIntent, PdfMutationPreflight, SignedPdf};

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
pub struct RedactPdfRequest {
    pub paths: Vec<PathBuf>,
    pub rectangle: PdfRect,
    #[serde(default = "default_scope")] pub scope: PageScope,
    pub output_location: OutputLocation,
}

fn default_scope() -> PageScope { PageScope::All }

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

pub fn crop(request: &CropPdfRequest, input: PathBuf) -> JobOutcome {
    // Only the crop box changes, so page objects, annotations, links, forms and structure stay valid.
    transform_or_skip_pdf_checked(input, &request.output_location, "-cropped", PdfMutationIntent::ValidateOnly, SignedPdf::RemoveSignature, |document, pages, _preflight| {
        request.spec.validate()?;
        let (selected, notes) = match &request.scope {
            RangedPageScope::All => ((0..pages.len()).collect(), Vec::new()),
            RangedPageScope::Selected { pages: selected } => (selected_indices(selected, pages.len()).map_err(ToolError::invalid_input)?, Vec::new()),
            RangedPageScope::Range { ranges } => match PageRanges::parse(ranges).map_err(ToolError::invalid_input)?.clamp_or_skip(pages.len()) {
                Ok(clamped) => (clamped.page_indices(), clamped.notes),
                Err(reason) => return Ok(Edited::Skipped(reason)),
            },
        };
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

enum Edited {
    Saved(String),
    Skipped(SkipReason),
}

fn transform_pdf<F>(input: PathBuf, location: &OutputLocation, suffix: &str, intent: PdfMutationIntent, signed: SignedPdf, edit: F) -> JobOutcome
where F: FnOnce(&mut Document, &[lopdf::ObjectId], &PdfMutationPreflight) -> Result<(), String> {
    transform_or_skip_pdf(input, location, suffix, intent, signed, |document, pages, preflight| edit(document, pages, preflight).map(|()| Edited::Saved("PDF saved".to_string())))
}

fn transform_or_skip_pdf<F>(input: PathBuf, location: &OutputLocation, suffix: &str, intent: PdfMutationIntent, signed: SignedPdf, edit: F) -> JobOutcome
where F: FnOnce(&mut Document, &[lopdf::ObjectId], &PdfMutationPreflight) -> Result<Edited, String> {
    transform_or_skip_pdf_checked(input, location, suffix, intent, signed, |document, pages, preflight| edit(document, pages, preflight).map_err(ToolError::processing))
}

/// Like `transform_or_skip_pdf`, but the edit chooses its error kind, such as InvalidInput for a bad request.
fn transform_or_skip_pdf_checked<F>(input: PathBuf, location: &OutputLocation, suffix: &str, intent: PdfMutationIntent, signed: SignedPdf, edit: F) -> JobOutcome
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

    fn editor_request_location(folder: &Path) -> OutputLocation {
        OutputLocation::CustomFolder(folder.to_path_buf())
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

}
