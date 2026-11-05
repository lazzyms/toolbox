use image::codecs::jpeg::JpegEncoder;
use image::ImageEncoder;
use lopdf::{dictionary, Document, Object, ObjectId};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::ToolError;
use super::metadata::{effective_crop_bounds, page_bounds, page_rotation};
use super::{mutation_preflight, PdfMutationIntent, PdfMutationPreflight};

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
pub struct RedactPdfRequest {
    pub paths: Vec<PathBuf>,
    pub rectangle: PdfRect,
    #[serde(default = "default_scope")] pub scope: PageScope,
    pub output_location: OutputLocation,
}

fn default_scope() -> PageScope { PageScope::All }
fn default_add_page_position() -> String { "after".to_string() }
fn default_add_page_count() -> usize { 1 }

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

pub fn crop(request: &CropPdfRequest, input: PathBuf) -> JobOutcome {
    transform_pdf(input, &request.output_location, "-cropped", PdfMutationIntent::General, |document, pages, preflight| {
        validate_rect(&request.rectangle)?;
        let selected = scoped_indices(&request.scope, pages.len())?;
        reject_annotated_pages(preflight, pages, selected.iter().copied(), "cropped")?;
        for page_index in selected {
            let page_id = pages[page_index];
            validate_rect_for_page(document, page_id, &request.rectangle)?;
            super::set_page_box_family(document, page_id, [
                request.rectangle.x,
                request.rectangle.y,
                request.rectangle.x + request.rectangle.width,
                request.rectangle.y + request.rectangle.height,
            ], true)?;
        }
        Ok(())
    })
}

pub fn organize(request: &OrganizePdfRequest, input: PathBuf) -> JobOutcome {
    let intent = if request.delete_pages.is_empty() && request.rotate_pages.is_empty() {
        PdfMutationIntent::ReorderOnly
    } else {
        PdfMutationIntent::General
    };
    transform_pdf(input, &request.output_location, "-organized", intent, |document, pages, preflight| {
        let root = preflight.pages_root;
        validate_unique_page_refs("page order", &request.page_order, pages.len())?;
        validate_unique_page_refs("deleted pages", &request.delete_pages, pages.len())?;
        validate_unique_rotations(&request.rotate_pages, pages.len())?;
        let selected = scoped_indices(&request.scope, pages.len())?;
        let selected_set = selected.iter().copied().collect::<std::collections::HashSet<_>>();
        let deleted = request.delete_pages.iter().copied().filter(|index| selected_set.contains(index)).collect::<std::collections::HashSet<_>>();
        reject_annotated_pages(preflight, pages, request.delete_pages.iter().copied().filter(|index| selected_set.contains(index)), "deleted")?;
        reject_annotated_pages(preflight, pages, request.rotate_pages.iter().filter(|rotation| selected_set.contains(&rotation.page)).map(|rotation| rotation.page), "rotated")?;
        let mut selected_order = request.page_order.iter().copied().filter(|index| selected_set.contains(index) && !deleted.contains(index)).collect::<Vec<_>>();
        selected_order.extend(selected.iter().copied().filter(|index| !deleted.contains(index) && !request.page_order.contains(index)));
        let mut selected_iter = selected_order.into_iter();
        let mut order = Vec::new();
        for (index, page_id) in pages.iter().enumerate() {
            if !selected_set.contains(&index) { order.push(*page_id); }
            else if !deleted.contains(&index) { order.push(pages[selected_iter.next().ok_or("The selected organize scope could not be represented")?]); }
        }
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
    transform_pdf(input, &request.output_location, "-pages-added", PdfMutationIntent::General, |document, pages, preflight| {
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
        let parent = preflight.pages_root;
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

/// Permanently removes everything under the rectangle: each target page is flattened to an image with
/// the area painted black, and its original text, vectors, and annotations are dropped from the file.
pub fn redact(request: &RedactPdfRequest, input: PathBuf) -> JobOutcome {
    let mut outcome = transform_pdf(input, &request.output_location, "-redacted", PdfMutationIntent::General, |document, pages, _preflight| {
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
        PageScope::Selected { pages } => {
            validate_unique_page_refs("selected-page scope", pages, page_count)?;
            if pages.is_empty() { return Err("A selected-page scope must contain at least one page.".to_string()); }
            Ok(pages.clone())
        }
    }
}

fn transform_pdf<F>(input: PathBuf, location: &OutputLocation, suffix: &str, intent: PdfMutationIntent, edit: F) -> JobOutcome
where F: FnOnce(&mut Document, &[lopdf::ObjectId], &PdfMutationPreflight) -> Result<(), String> {
    let mut document = match Document::load(&input) { Ok(document) => document, Err(error) => return failure(input, error.to_string()) };
    if document.is_encrypted() { return failure(input, "Unlock the PDF before editing".to_string()); }
    let pages: Vec<_> = document.get_pages().values().copied().collect();
    if pages.is_empty() { return failure(input, "PDF has no pages".to_string()); }
    let preflight = match mutation_preflight(&document, intent) {
        Ok(preflight) => preflight,
        Err(error) => return failure(input, error),
    };
    if let Err(error) = edit(&mut document, &pages, &preflight) { return failure(input, error); }
    let output = match OutputNaming::reserve_destination(&input, location, suffix, "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve PDF output: {error}")),
    };
    match document.save(output.path()) {
        Ok(_) => match output.publish() {
            Ok(path) => JobOutcome::succeeded(input, vec![path], "PDF saved"),
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
    fn crop_clamps_all_existing_page_boxes_to_the_new_media_box() {
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
            rectangle: PdfRect { x: 100.0, y: 100.0, width: 200.0, height: 300.0 },
            scope: PageScope::All,
            output_location: editor_request_location(&output_dir),
        }, source.clone());
        assert!(outcome.error().is_none(), "crop failed: {:?}", outcome.error());
        let output_document = Document::load(outcome.output_paths().first().unwrap()).unwrap();
        let page = output_document.get_pages().values().next().copied().unwrap();
        let page = output_document.get_dictionary(page).unwrap();
        for key in [b"MediaBox".as_slice(), b"CropBox", b"BleedBox", b"ArtBox"] {
            assert_eq!(page.get(key).unwrap().as_array().unwrap().iter().map(|value| value.as_i64().unwrap() as f32).collect::<Vec<_>>(), vec![100.0, 100.0, 300.0, 400.0]);
        }
        assert_eq!(page.get(b"TrimBox").unwrap().as_array().unwrap().iter().map(|value| value.as_i64().unwrap() as f32).collect::<Vec<_>>(), vec![100.0, 100.0, 150.0, 200.0]);

        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(outcome.output_paths().first().unwrap());
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn crop_rejects_internal_page_links_and_malformed_page_counts_before_output() {
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
                paths: vec![source.clone()], rectangle: PdfRect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 },
                scope: PageScope::All, output_location: editor_request_location(&output_dir),
            }, source.clone());
            assert!(outcome.error().is_some_and(|error| error.message.contains("navigation")), "{name}: {:?}", outcome.error());
            assert!(outcome.output_paths().is_empty());
            assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
            let _ = std::fs::remove_file(source);
            let _ = std::fs::remove_dir(output_dir);
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
                paths: vec![source.clone()], rectangle: PdfRect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 },
                scope: PageScope::All, output_location: editor_request_location(&output_dir),
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
            rectangle: PdfRect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 },
            scope: PageScope::All,
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
    fn crop_rejects_plain_annotations_before_reserving_and_preserves_the_original() {
        let source = temp_path("crop-annotation-output");
        let output_dir = temp_path("crop-annotation-output-dir");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_pdf_with_pages(&source, 1);
        add_page_annotation(&source, 0, dictionary! {
            "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![20.into(), 20.into(), 80.into(), 50.into()],
        }.into());
        let original = std::fs::read(&source).unwrap();

        let outcome = crop(&CropPdfRequest {
            paths: vec![source.clone()], rectangle: PdfRect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 },
            scope: PageScope::All, output_location: location(&output_dir),
        }, source.clone());

        assert!(outcome.error().is_some_and(|error| error.message.contains("annotation")), "unexpected result: {:?}", outcome.error());
        assert!(outcome.output_paths().is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn organize_rejects_annotation_rotation_and_page_deletion_before_output() {
        for (name, rotate_pages, delete_pages) in [
            ("rotation", vec![RotatePage { page: 0, degrees: 90 }], vec![]),
            ("deletion", vec![], vec![0]),
        ] {
            let source = temp_path(&format!("annotation-page-plan-{name}"));
            let output_dir = temp_path(&format!("annotation-page-plan-{name}-output"));
            std::fs::create_dir_all(&output_dir).unwrap();
            make_pdf(&source);
            add_page_annotation(&source, 0, dictionary! {
                "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![20.into(), 20.into(), 80.into(), 50.into()],
            }.into());
            let original = std::fs::read(&source).unwrap();

            let outcome = organize(&OrganizePdfRequest {
                paths: vec![source.clone()], page_order: vec![1, 0], delete_pages, rotate_pages,
                scope: PageScope::All, output_location: location(&output_dir),
            }, source.clone());

            assert!(outcome.error().is_some_and(|error| error.message.contains("annotation")), "{name}: {:?}", outcome.error());
            assert!(outcome.output_paths().is_empty());
            assert_eq!(std::fs::read(&source).unwrap(), original);
            assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
            let _ = std::fs::remove_file(source);
            let _ = std::fs::remove_dir(output_dir);
        }
    }

    #[test]
    fn organize_reorders_pages_with_annotations_without_detaching_them() {
        let source = temp_path("annotation-page-reorder");
        let output_dir = temp_path("annotation-page-reorder-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_pdf(&source);
        add_page_annotation(&source, 0, dictionary! {
            "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![20.into(), 20.into(), 80.into(), 50.into()],
        }.into());

        let outcome = organize(&OrganizePdfRequest {
            paths: vec![source.clone()], page_order: vec![1, 0], delete_pages: vec![], rotate_pages: vec![],
            scope: PageScope::All, output_location: location(&output_dir),
        }, source.clone());

        assert!(outcome.error().is_none(), "reorder failed: {:?}", outcome.error());
        let output = outcome.output_paths().first().unwrap();
        let document = Document::load(output).unwrap();
        let pages = document.get_pages().values().copied().collect::<Vec<_>>();
        assert!(document.get_dictionary(pages[0]).unwrap().get(b"Annots").is_err());
        assert_eq!(document.get_dictionary(pages[1]).unwrap().get(b"Annots").unwrap().as_array().unwrap().len(), 1);
        let annotation = document.get_dictionary(pages[1]).unwrap().get(b"Annots").unwrap().as_array().unwrap()[0].as_reference().unwrap();
        assert_eq!(annotation.1, 0);
        assert_eq!(document.get_dictionary(annotation).unwrap().get(b"P").unwrap().as_reference().unwrap(), pages[1]);

        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(output);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn pure_reorder_allows_navigation_and_form_references_for_organize() {
        let source = temp_path("navigation-form-reorder-organize");
        let output_dir = temp_path("navigation-form-reorder-organize-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_navigation_and_form_pdf(&source);
        let original = std::fs::read(&source).unwrap();
        let source_document = Document::load(&source).unwrap();
        let original_pages = source_document.get_pages().values().copied().collect::<Vec<_>>();
        let catalog = source_document.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let acro_form = source_document.get_dictionary(catalog).unwrap().get(b"AcroForm").unwrap().as_reference().unwrap();
        let widget = source_document.get_dictionary(acro_form).unwrap().get(b"Fields").unwrap().as_array().unwrap()[0].as_reference().unwrap();

        let outcome = organize(&OrganizePdfRequest {
            paths: vec![source.clone()],
            page_order: vec![1, 0],
            delete_pages: vec![],
            rotate_pages: vec![],
            scope: PageScope::All,
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
    fn organize_rejects_annotation_with_a_non_owner_page_reference_before_output() {
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

        let outcome = organize(&OrganizePdfRequest {
            paths: vec![source.clone()],
            page_order: vec![1, 0],
            delete_pages: vec![],
            rotate_pages: vec![],
            scope: PageScope::All,
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
                paths: vec![source.clone()], rectangle: PdfRect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 },
                scope: PageScope::All, output_location: location(&output_dir),
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
        assert!(outcome.error().is_none(), "selected organize failed: {:?}", outcome.error());
        let output = outcome.output_paths().first().expect("organize should produce an output");
        let output_document = Document::load(output).expect("organized PDF should reopen");
        let order = output_document.get_pages().values().copied().collect::<Vec<_>>();
        let contents = order.iter().map(|page| String::from_utf8_lossy(&output_document.get_page_content(*page)).into_owned()).collect::<Vec<_>>();
        assert_eq!(contents, vec!["BT /F0 12 Tf 36 720 Td (page-0) Tj ET\n", "BT /F0 12 Tf 36 720 Td (page-3) Tj ET\n", "BT /F0 12 Tf 36 720 Td (page-2) Tj ET\n", "BT /F0 12 Tf 36 720 Td (page-1) Tj ET\n"]);

        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(output);
        let _ = std::fs::remove_dir(output_dir);
    }

    #[test]
    fn organize_rejects_nested_page_tree_before_output() {
        let source = temp_path("organize-nested.pdf");
        let output_dir = temp_path("organize-nested-output");
        std::fs::create_dir_all(&output_dir).expect("output directory should be created");
        make_nested_pdf(&source);

        let outcome = organize(&OrganizePdfRequest {
            paths: vec![source.clone()], page_order: vec![], delete_pages: vec![], rotate_pages: vec![],
            scope: PageScope::All, output_location: location(&output_dir),
        }, source.clone());

        assert!(outcome.error().is_some_and(|error| error.message.contains("nested or unsupported")), "{:?}", outcome.error());
        assert!(outcome.output_paths().is_empty());
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);

        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }

    fn add_catalog_marker(path: &Path, key: &[u8], value: Object) {
        let mut document = Document::load(path).expect("fixture should load");
        let catalog = document.trailer.get(b"Root").unwrap().as_reference().unwrap();
        document.get_dictionary_mut(catalog).unwrap().set(key, value);
        document.save(path).expect("fixture marker should save");
    }

    #[test]
    fn all_editor_mutations_reject_signed_pdfs_before_reserving_output() {
        let cases = ["organize", "add"];
        for case in cases {
            let source = temp_path(&format!("signed-{case}.pdf"));
            let output_dir = temp_path(&format!("signed-{case}-output"));
            std::fs::create_dir_all(&output_dir).unwrap();
            make_pdf(&source);
            let signature = {
                let mut document = Document::load(&source).unwrap();
                document.add_object(dictionary! { "Type" => "Sig" })
            };
            add_catalog_marker(&source, b"Perms", dictionary! { "DocMDP" => signature }.into());

            let outcome = match case {
                "organize" => organize(&OrganizePdfRequest {
                    paths: vec![source.clone()], page_order: vec![], delete_pages: vec![], rotate_pages: vec![],
                    scope: PageScope::All, output_location: location(&output_dir),
                }, source.clone()),
                "add" => add_pages(&AddPdfPagesRequest {
                    paths: vec![source.clone()], position: "end".to_string(), page: 0, count: 1,
                    output_location: location(&output_dir),
                }, source.clone()),
                _ => unreachable!(),
            };

            assert!(outcome.error().is_some_and(|error| error.message.contains("Digitally signed")), "{case}: {:?}", outcome.error());
            assert!(outcome.output_paths().is_empty());
            assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
            let _ = std::fs::remove_file(source);
            let _ = std::fs::remove_dir(output_dir);
        }
    }

    #[test]
    fn unsafe_editor_mutations_reject_navigational_pdfs_before_reserving_output() {
        let cases = ["organize-rotate", "add"];
        for case in cases {
            let source = temp_path(&format!("navigation-{case}.pdf"));
            let output_dir = temp_path(&format!("navigation-{case}-output"));
            std::fs::create_dir_all(&output_dir).unwrap();
            make_pdf(&source);
            let outlines = {
                let mut document = Document::load(&source).unwrap();
                document.add_object(dictionary! { "Type" => "Outlines", "Count" => 0 })
            };
            add_catalog_marker(&source, b"Outlines", Object::Reference(outlines));
            let original = std::fs::read(&source).unwrap();

            let outcome = match case {
                "organize-rotate" => organize(&OrganizePdfRequest {
                    paths: vec![source.clone()], page_order: vec![1, 0], delete_pages: vec![],
                    rotate_pages: vec![RotatePage { page: 0, degrees: 90 }],
                    scope: PageScope::All, output_location: location(&output_dir),
                }, source.clone()),
                "add" => add_pages(&AddPdfPagesRequest {
                    paths: vec![source.clone()], position: "end".to_string(), page: 0, count: 1,
                    output_location: location(&output_dir),
                }, source.clone()),
                _ => unreachable!(),
            };

            assert!(outcome.error().is_some_and(|error| error.message.contains("navigation")), "{case}: {:?}", outcome.error());
            assert!(outcome.output_paths().is_empty());
            assert_eq!(std::fs::read(&source).unwrap(), original);
            assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
            let _ = std::fs::remove_file(source);
            let _ = std::fs::remove_dir(output_dir);
        }
    }

    #[test]
    fn all_editor_mutations_reject_tagged_pdfs_before_reserving_output() {
        let cases = ["organize", "add"];
        for case in cases {
            let source = temp_path(&format!("tagged-{case}.pdf"));
            let output_dir = temp_path(&format!("tagged-{case}-output"));
            std::fs::create_dir_all(&output_dir).unwrap();
            make_pdf(&source);
            let structure = {
                let mut document = Document::load(&source).unwrap();
                document.add_object(dictionary! { "Type" => "StructTreeRoot", "K" => vec![] })
            };
            add_catalog_marker(&source, b"StructTreeRoot", Object::Reference(structure));

            let outcome = match case {
                "organize" => organize(&OrganizePdfRequest {
                    paths: vec![source.clone()], page_order: vec![], delete_pages: vec![], rotate_pages: vec![],
                    scope: PageScope::All, output_location: location(&output_dir),
                }, source.clone()),
                "add" => add_pages(&AddPdfPagesRequest {
                    paths: vec![source.clone()], position: "end".to_string(), page: 0, count: 1,
                    output_location: location(&output_dir),
                }, source.clone()),
                _ => unreachable!(),
            };

            assert!(outcome.error().is_some_and(|error| error.message.contains("tagged PDF")), "{case}: {:?}", outcome.error());
            assert!(outcome.output_paths().is_empty());
            assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
            let _ = std::fs::remove_file(source);
            let _ = std::fs::remove_dir(output_dir);
        }
    }

    #[test]
    fn add_rejects_nested_page_trees_before_output() {
        let source = temp_path("nested-add.pdf");
        let output_dir = temp_path("nested-add-output");
        std::fs::create_dir_all(&output_dir).unwrap();
        make_nested_pdf(&source);
        let outcome = add_pages(&AddPdfPagesRequest {
            paths: vec![source.clone()], position: "end".to_string(), page: 0, count: 1,
            output_location: location(&output_dir),
        }, source.clone());
        assert!(outcome.error().is_some_and(|error| error.message.contains("nested or unsupported")), "{:?}", outcome.error());
        assert!(outcome.output_paths().is_empty());
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 0);
        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_dir(output_dir);
    }
}
