#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod file_actions;
mod kit;

use crate::file_actions::{open_output_path, reveal_output_path};
use crate::kit::common::JobOutcome;
use crate::kit::images::{ImageProcessor, Options as ImageOptions};
use crate::kit::images::tools;
use crate::kit::pdf::{editor, metadata, remaining, scene, PDFProcessor};
use crate::kit::vision;
use crate::kit::contracts::{command_supports_preview, validate_command_inputs, validate_page_selection, CompressImagesRequest, ConvertImagesRequest, PasswordRequest, PdfRequest, ToolError};
use crate::kit::common::batch_runner::BatchRunner;
use crate::kit::password::PasswordProcessor;

#[tauri::command]
async fn remove_password(request: PasswordRequest) -> Vec<JobOutcome> {
    BatchRunner::run("remove_password", request.paths, |path| {
        PasswordProcessor::remove_password(path, &request.password, &request.output_location)
    })
}

#[tauri::command]
async fn protect_pdf(request: PdfRequest) -> Vec<JobOutcome> {
    BatchRunner::run("protect_pdf", request.paths, |path| {
        PDFProcessor::protect(path, &request.password, &request.output_location)
    })
}

#[tauri::command]
async fn compress_images(request: CompressImagesRequest) -> Vec<JobOutcome> {
    BatchRunner::run("compress_images", request.paths, |path| {
        ImageProcessor::run(path, ImageOptions {
            target_format: None,
            quality: if request.lossless { 0 } else { request.quality },
            keep_smaller_original: true,
            suffix: "-compressed".to_string(),
            output_location: request.output_location.clone(),
        })
    })
}

#[tauri::command]
async fn convert_images(request: ConvertImagesRequest) -> Vec<JobOutcome> {
    let img_format = match tools::parse_output_format(&request.format) {
        Ok(format) => format,
        Err(error) => {
            if request.paths.is_empty() {
                return vec![JobOutcome::failure(std::path::PathBuf::new(), ToolError::invalid_input(error))];
            }
            return request.paths.into_iter().map(|path| JobOutcome::failure(path, ToolError::invalid_input(error.clone()))).collect();
        }
    };

    BatchRunner::run("convert_images", request.paths, |path| {
        ImageProcessor::run(path, ImageOptions {
            target_format: Some(img_format),
            quality: 80,
            keep_smaller_original: false,
            suffix: "-converted".to_string(),
            output_location: request.output_location.clone(),
        })
    })
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct InspectPdfRequest {
    path: std::path::PathBuf,
}

#[tauri::command]
fn inspect_pdf(request: InspectPdfRequest) -> Result<metadata::PdfDocumentMetadata, String> {
    metadata::inspect(&request.path)
}

#[tauri::command]
async fn inspect_pdf_scene(request: InspectPdfRequest) -> Result<metadata::PdfDocumentMetadata, String> {
    tauri::async_runtime::spawn_blocking(move || scene::inspect(&request.path)).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn preview_pdf_scene(request: scene::PreviewRequest) -> Result<scene::ScenePreview, String> {
    if !command_supports_preview("export_pdf_scene") { return Err("PDF scene preview is unavailable in this build.".to_string()); }
    tauri::async_runtime::spawn_blocking(move || scene::preview(&request)).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn export_pdf_scene(request: scene::ExportRequest) -> Vec<JobOutcome> {
    BatchRunner::run("export_pdf_scene", request.paths.clone(), |path| scene::export(&request, path))
}

#[tauri::command]
async fn crop_pdf(request: editor::CropPdfRequest) -> Vec<JobOutcome> {
    let explicit_pages = match &request.scope {
        editor::PageScope::All => None,
        editor::PageScope::Selected { pages } => Some(pages.as_slice()),
    };
    if let Err(outcomes) = validate_page_selection("crop_pdf", &request.paths, explicit_pages) { return outcomes; }
    BatchRunner::run("crop_pdf", request.paths.clone(), |path| editor::crop(&request, path))
}

#[tauri::command]
async fn sign_pdf(request: editor::SignPdfRequest) -> Vec<JobOutcome> {
    let explicit_pages = match &request.scope {
        editor::PageScope::All => None,
        editor::PageScope::Selected { pages } => Some(pages.as_slice()),
    };
    if let Err(outcomes) = validate_page_selection("sign_pdf", &request.paths, explicit_pages) { return outcomes; }
    BatchRunner::run("sign_pdf", request.paths.clone(), |path| editor::sign(&request, path))
}

#[tauri::command]
async fn edit_pdf(request: editor::EditPdfRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_page_selection("edit_pdf", &request.paths, request.pages.as_deref()) { return outcomes; }
    BatchRunner::run("edit_pdf", request.paths.clone(), |path| editor::edit(&request, path))
}

#[tauri::command]
async fn edit_pdf_session(request: editor::PdfEditSessionRequest) -> Vec<JobOutcome> {
    BatchRunner::run("edit_pdf_session", request.paths.clone(), |path| editor::apply_session(&request, path))
}

#[tauri::command]
async fn organize_pdf(request: editor::OrganizePdfRequest) -> Vec<JobOutcome> {
    let explicit_pages = match &request.scope {
        editor::PageScope::All => None,
        editor::PageScope::Selected { pages } => Some(pages.as_slice()),
    };
    if let Err(outcomes) = validate_page_selection("organize_pdf", &request.paths, explicit_pages) { return outcomes; }
    BatchRunner::run("organize_pdf", request.paths.clone(), |path| editor::organize(&request, path))
}

#[tauri::command]
async fn add_pdf_pages(request: editor::AddPdfPagesRequest) -> Vec<JobOutcome> {
    BatchRunner::run("add_pdf_pages", request.paths.clone(), |path| editor::add_pages(&request, path))
}

#[tauri::command]
async fn add_page_numbers(request: remaining::PageOverlayRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_page_selection("add_page_numbers", &request.paths, request.pages.as_deref()) { return outcomes; }
    BatchRunner::run("add_page_numbers", request.paths.clone(), |path| remaining::add_page_numbers(&request, path))
}

#[tauri::command]
async fn watermark_pdf(request: remaining::PageOverlayRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_page_selection("watermark_pdf", &request.paths, request.pages.as_deref()) { return outcomes; }
    BatchRunner::run("watermark_pdf", request.paths.clone(), |path| remaining::watermark(&request, path))
}

#[tauri::command]
async fn compress_pdf(request: remaining::CompressPdfRequest) -> Vec<JobOutcome> {
    BatchRunner::run("compress_pdf", request.paths.clone(), |path| remaining::compress(&request, path))
}

#[tauri::command]
async fn remove_pdf_pages(request: remaining::PageSelectionRequest) -> Vec<JobOutcome> {
    let explicit_pages = request.page_ranges.as_deref().filter(|range| !range.trim().is_empty()).is_none().then_some(request.pages.as_slice());
    if let Err(outcomes) = validate_page_selection("remove_pdf_pages", &request.paths, explicit_pages) { return outcomes; }
    BatchRunner::run("remove_pdf_pages", request.paths.clone(), |path| remaining::remove_pages(&request, path))
}

#[tauri::command]
async fn extract_pdf_pages(request: remaining::PageSelectionRequest) -> Vec<JobOutcome> {
    let explicit_pages = request.page_ranges.as_deref().filter(|range| !range.trim().is_empty()).is_none().then_some(request.pages.as_slice());
    if let Err(outcomes) = validate_page_selection("extract_pdf_pages", &request.paths, explicit_pages) { return outcomes; }
    BatchRunner::run("extract_pdf_pages", request.paths.clone(), |path| remaining::extract_pages(&request, path))
}

#[tauri::command]
async fn merge_pdfs(request: remaining::MergePdfRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("merge_pdfs", &request.paths) { return outcomes; }
    aggregate_outcomes(&request.paths, || remaining::merge(&request))
}

#[tauri::command]
async fn split_pdf(request: remaining::PageSelectionRequest) -> Vec<JobOutcome> {
    BatchRunner::run("split_pdf", request.paths.clone(), |path| remaining::split(&request, path))
}

#[tauri::command]
async fn pdf_to_images(request: remaining::PdfToImagesRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_page_selection("pdf_to_images", &request.paths, request.pages.as_deref()) { return outcomes; }
    BatchRunner::run("pdf_to_images", request.paths.clone(), |path| remaining::to_images(&request, path))
}

#[tauri::command]
async fn pdf_to_text(request: remaining::PdfToTextRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_page_selection("pdf_to_text", &request.paths, request.pages.as_deref()) { return outcomes; }
    BatchRunner::run("pdf_to_text", request.paths.clone(), |path| remaining::to_text(&request, path))
}

#[tauri::command]
async fn extract_pdf_images(request: remaining::PdfToTextRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_page_selection("extract_pdf_images", &request.paths, request.pages.as_deref()) { return outcomes; }
    BatchRunner::run("extract_pdf_images", request.paths.clone(), |path| remaining::extract_images(&request, path))
}

#[tauri::command]
async fn images_to_pdf(request: remaining::ImagesToPdfRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("images_to_pdf", &request.paths) { return outcomes; }
    aggregate_outcomes(&request.paths, || remaining::images_to_pdf(&request))
}

#[tauri::command]
async fn ocr_pdf(request: vision::VisionRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_page_selection("ocr_pdf", &request.paths, request.pages.as_deref()) { return outcomes; }
    BatchRunner::run("ocr_pdf", request.paths.clone(), |path| vision::ocr_pdf(&request, path))
}
#[tauri::command]
async fn blur_faces(request: vision::VisionRequest) -> Vec<JobOutcome> { BatchRunner::run("blur_faces", request.paths.clone(), |path| vision::blur_faces(&request, path)) }
#[tauri::command]
async fn remove_image_background(request: vision::VisionRequest) -> Vec<JobOutcome> { BatchRunner::run("remove_image_background", request.paths.clone(), |path| vision::remove_background(&request, path)) }
#[tauri::command]
async fn resize_images(request: tools::ResizeRequest) -> Vec<JobOutcome> { BatchRunner::run("resize_images", request.paths.clone(), |path| tools::resize(&request, path)) }
#[tauri::command]
async fn rotate_images(request: tools::RotateRequest) -> Vec<JobOutcome> { BatchRunner::run("rotate_images", request.paths.clone(), |path| tools::rotate(&request, path)) }
#[tauri::command]
async fn crop_images(request: tools::CropRequest) -> Vec<JobOutcome> { BatchRunner::run("crop_images", request.paths.clone(), |path| tools::crop(&request, path)) }
#[tauri::command]
async fn adjust_image_tone(request: tools::ToneRequest) -> Vec<JobOutcome> { BatchRunner::run("adjust_image_tone", request.paths.clone(), |path| tools::tone(&request, path)) }
#[tauri::command]
async fn watermark_images(request: tools::WatermarkRequest) -> Vec<JobOutcome> { BatchRunner::run("watermark_images", request.paths.clone(), |path| tools::watermark(&request, path)) }
#[tauri::command]
async fn generate_icon_set(request: tools::IconSetRequest) -> Vec<JobOutcome> { BatchRunner::run("generate_icon_set", request.paths.clone(), |path| tools::icon_set(&request, path)) }
#[tauri::command]
async fn create_gif(request: tools::GifCreateRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("create_gif", &request.paths) { return outcomes; }
    aggregate_outcomes(&request.paths, || tools::gif_create(&request))
}
#[tauri::command]
async fn extract_gif_frames(request: tools::GifExtractRequest) -> Vec<JobOutcome> { BatchRunner::run("extract_gif_frames", request.paths.clone(), |path| tools::gif_extract(&request, path)) }
#[tauri::command]
async fn process_tiff_pages(request: tools::TiffRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("process_tiff_pages", &request.paths) { return outcomes; }
    aggregate_outcomes(&request.paths, || tools::tiff(&request))
}
#[tauri::command]
async fn image_metadata(request: tools::MetadataRequest) -> Vec<JobOutcome> { BatchRunner::run("image_metadata", request.paths.clone(), |path| tools::strip_metadata(&request, path)) }
#[tauri::command]
fn inspect_image_metadata(request: tools::MetadataRequest) -> Vec<Result<tools::MetadataReport, String>> { request.paths.into_iter().map(tools::inspect_metadata).collect() }

#[tauri::command]
fn inspect_image_preview(request: tools::ImagePreviewRequest) -> Result<tools::ImagePreview, String> { tools::inspect_preview(&request) }

#[tauri::command]
fn inspect_tiff_pages(request: tools::TiffPreviewRequest) -> Result<Vec<tools::ImagePreview>, String> { tools::inspect_tiff_pages(&request) }

#[tauri::command]
fn inspect_image_edit_preview(request: tools::ImageEditPreviewRequest) -> Result<tools::ImagePreview, String> { tools::inspect_edit_preview(&request) }

#[tauri::command]
async fn export_image_edit_plan(request: tools::ImageEditExportRequest) -> Vec<JobOutcome> {
    BatchRunner::run("export_image_edit_plan", request.paths.clone(), |path| tools::export_edit_plan(&request.plan, path))
}

fn aggregate_outcomes<F>(paths: &[std::path::PathBuf], operation: F) -> Vec<JobOutcome>
where
    F: FnOnce() -> JobOutcome,
{
    let aggregate = operation();
    match aggregate.failure {
        None => paths.iter().cloned().map(|input_path| JobOutcome {
            input_path,
            output_paths: aggregate.output_paths.clone(),
            detail: aggregate.detail.clone(),
            failure: None,
        }).collect(),
        Some(error) => paths.iter().cloned().map(|input_path| {
            let error = if input_path == aggregate.input_path {
                error.clone()
            } else {
                ToolError::processing(format!(
                    "Aggregate operation stopped because {} failed: {}",
                    aggregate.input_path.display(), error.message
                ))
            };
            JobOutcome::failure_with_outputs(input_path, aggregate.output_paths.clone(), error)
        }).collect(),
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            open_output_path,
            reveal_output_path,
            remove_password,
            protect_pdf,
            compress_images,
            convert_images,
            inspect_pdf,
            inspect_pdf_scene,
            preview_pdf_scene,
            export_pdf_scene,
            crop_pdf,
            edit_pdf,
            edit_pdf_session,
            sign_pdf,
            organize_pdf,
            add_pdf_pages
            ,add_page_numbers,
            watermark_pdf,
            compress_pdf,
            remove_pdf_pages,
            extract_pdf_pages
            ,merge_pdfs,
            split_pdf,
            pdf_to_images,
            pdf_to_text,
            extract_pdf_images,
            images_to_pdf,
            ocr_pdf,
            blur_faces,
            remove_image_background,
            resize_images,
            rotate_images,
            crop_images
            ,adjust_image_tone,
            watermark_images
            ,generate_icon_set,
            create_gif,
            extract_gif_frames,
            process_tiff_pages
            ,image_metadata
            ,inspect_image_metadata,
            inspect_image_preview,
            inspect_tiff_pages,
            inspect_image_edit_preview,
            export_image_edit_plan
        ])
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod command_tests {
    // Drives the same command functions the frontend `invoke`s, with the same
    // argument shapes (Vec<String> paths, format strings), so the whole
    // IPC-facing layer is exercised without needing a live WebView.

    use super::*;
    use crate::kit::common::OutputLocation;
    use crate::kit::images::tools::{CropRequest, GifCreateRequest, GifExtractRequest, ImageEdit, ImageEditExportRequest, ImageEditPlan, ImageEditPreviewRequest, IconSetRequest, MetadataRequest, ResizeRequest, RotateRequest, TiffRequest, ToneRequest, WatermarkRequest};
    use crate::kit::pdf::editor::{AddPdfPagesRequest, CropPdfRequest, EditPdfRequest, OrganizePdfRequest, PageScope, PdfEditMode, PdfEditOperation, PdfEditSessionPlan, PdfEditSessionRequest, PdfOverlay, PdfOverlayPosition, PdfRect, RotatePage, SignPdfRequest};
    use crate::kit::pdf::remaining::{CompressPdfRequest, ImagesToPdfRequest, MergePdfRequest, PageOverlayRequest, PageSelectionRequest, PdfToImagesRequest, PdfToTextRequest};
    use crate::kit::vision::VisionRequest;
    use image::Rgba;
    use std::path::PathBuf;
    use std::future::Future;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("toolbox_cmd_{}_{}", std::process::id(), name))
    }

    fn cleanup(paths: &[PathBuf]) {
        for p in paths {
            let _ = std::fs::remove_file(p);
        }
    }

    fn write_png(path: &PathBuf, size: u32, byte: u8) {
        image::RgbaImage::from_fn(size, size, |x, y| Rgba([byte, x as u8, y as u8, 255]))
            .save(path)
            .unwrap();
    }

    fn sandbox(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("toolbox_native_e2e_{}_{}", std::process::id(), name));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn location(root: &PathBuf, name: &str) -> OutputLocation {
        let folder = root.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        OutputLocation::CustomFolder(folder)
    }

    fn make_pdf(path: &PathBuf, page_count: usize) {
        use lopdf::{dictionary, Document, Object, Stream};
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let mut kids = Vec::new();
        for number in 0..page_count {
            let content_id = document.add_object(Stream::new(dictionary! {}, format!("BT /F1 24 Tf 72 720 Td (Toolbox page {}) Tj ET", number + 1).into_bytes()));
            let page_id = document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id,
                "MediaBox" => vec![Object::Real(0.5), Object::Real(0.5), Object::Real(612.5), Object::Real(792.5)],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } }, "Contents" => content_id,
            });
            kids.push(Object::Reference(page_id));
        }
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => page_count as i64 }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    fn assert_success<F>(name: &str, future: F) -> Vec<PathBuf>
    where F: Future<Output = Vec<JobOutcome>> {
        let outcomes = tauri::async_runtime::block_on(future);
        assert_eq!(outcomes.len(), 1, "{name} should return one input outcome");
        let outcome = &outcomes[0];
        assert!(outcome.failure.is_none(), "{name} failed: {}", outcome.failure.clone().unwrap_or_default());
        assert!(!outcome.output_paths.is_empty(), "{name} must report an output");
        for output in &outcome.output_paths { assert!(output.is_file(), "{name} reported missing output {}", output.display()); }
        outcome.output_paths.clone()
    }

    fn assert_aggregate_success<F>(name: &str, paths: &[PathBuf], future: F) -> Vec<PathBuf>
    where F: Future<Output = Vec<JobOutcome>> {
        let outcomes = tauri::async_runtime::block_on(future);
        assert_eq!(outcomes.len(), paths.len(), "{name} should return one outcome per input");
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths, "{name} must preserve input order");
        let shared_outputs = outcomes[0].output_paths.clone();
        assert!(!shared_outputs.is_empty(), "{name} must report an aggregate output");
        for outcome in &outcomes {
            assert!(outcome.failure.is_none(), "{name} failed for {}: {}", outcome.input_path.display(), outcome.failure.clone().unwrap_or_default());
            assert_eq!(outcome.output_paths, shared_outputs, "{name} must report the shared aggregate output for every input");
            for output in &outcome.output_paths { assert!(output.is_file(), "{name} reported missing output {}", output.display()); }
        }
        shared_outputs
    }

    fn assert_aggregate_failure<F>(name: &str, paths: &[PathBuf], failed_path: &PathBuf, future: F)
    where F: Future<Output = Vec<JobOutcome>> {
        let outcomes = tauri::async_runtime::block_on(future);
        assert_eq!(outcomes.len(), paths.len(), "{name} should return one outcome per input");
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths, "{name} must preserve input order");
        assert!(outcomes.iter().all(|outcome| outcome.output_paths.is_empty() && outcome.failure.is_some()), "{name} must fail every input when the aggregate cannot run");
        let failed = outcomes.iter().find(|outcome| outcome.input_path == *failed_path).expect("missing input must have an outcome");
        assert!(failed.failure.as_ref().is_some_and(|error| error.message.contains(&failed_path.display().to_string())), "{name} must identify the failed input");
    }

    #[test]
    fn aggregate_commands_report_ordered_successes_and_missing_inputs() {
        let root = sandbox("aggregate-outcomes");
        let image = root.join("first.png"); write_png(&image, 32, 44);
        let image_two = root.join("second.png"); write_png(&image_two, 32, 88);
        let pdf = root.join("first.pdf"); make_pdf(&pdf, 1);
        let pdf_two = root.join("second.pdf"); make_pdf(&pdf_two, 1);
        let tiff = root.join("first.tiff"); image::open(&image).unwrap().save(&tiff).unwrap();
        let tiff_two = root.join("second.tiff"); image::open(&image_two).unwrap().save(&tiff_two).unwrap();
        let missing_pdf = root.join("missing.pdf");
        let missing_image = root.join("missing.png");
        let missing_gif_frame = root.join("missing.gif.png");
        let missing_tiff = root.join("missing.tiff");

        assert_aggregate_success("merge valid inputs", &[pdf.clone(), pdf_two.clone()], merge_pdfs(MergePdfRequest {
            paths: vec![pdf.clone(), pdf_two.clone()], output_location: location(&root, "merge-valid"),
        }));
        assert_aggregate_failure("merge missing later input", &[pdf.clone(), missing_pdf.clone()], &missing_pdf, merge_pdfs(MergePdfRequest {
            paths: vec![pdf.clone(), missing_pdf.clone()], output_location: location(&root, "merge-missing"),
        }));
        assert_aggregate_success("images-to-pdf valid inputs", &[image.clone(), image_two.clone()], images_to_pdf(ImagesToPdfRequest {
            paths: vec![image.clone(), image_two.clone()], output_location: location(&root, "images-valid"),
        }));
        assert_aggregate_failure("images-to-pdf missing later input", &[image.clone(), missing_image.clone()], &missing_image, images_to_pdf(ImagesToPdfRequest {
            paths: vec![image.clone(), missing_image.clone()], output_location: location(&root, "images-missing"),
        }));
        assert_aggregate_success("GIF valid inputs", &[image.clone(), image_two.clone()], create_gif(GifCreateRequest {
            paths: vec![image.clone(), image_two.clone()], frame_delay_ms: 100, loop_forever: true, output_location: location(&root, "gif-valid"),
        }));
        assert_aggregate_failure("GIF missing later input", &[image.clone(), missing_gif_frame.clone()], &missing_gif_frame, create_gif(GifCreateRequest {
            paths: vec![image.clone(), missing_gif_frame.clone()], frame_delay_ms: 100, loop_forever: true, output_location: location(&root, "gif-missing"),
        }));
        assert_aggregate_success("TIFF valid inputs", &[tiff.clone(), tiff_two.clone()], process_tiff_pages(TiffRequest {
            paths: vec![tiff.clone(), tiff_two.clone()], pages: None, output_location: location(&root, "tiff-valid"),
        }));
        assert_aggregate_failure("TIFF missing later input", &[tiff.clone(), missing_tiff.clone()], &missing_tiff, process_tiff_pages(TiffRequest {
            paths: vec![tiff.clone(), missing_tiff.clone()], pages: None, output_location: location(&root, "tiff-missing"),
        }));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn aggregate_failures_preserve_partial_outputs_for_result_list_consumers() {
        let paths = vec![PathBuf::from("first.tiff"), PathBuf::from("second.tiff")];
        let published = PathBuf::from("first-page-1.tiff");
        let outcomes = aggregate_outcomes(&paths, || {
            JobOutcome::failure_with_outputs(
                paths[0].clone(),
                vec![published.clone()],
                ToolError::processing("The next TIFF page could not be published."),
            )
        });

        assert_eq!(outcomes.len(), paths.len());
        assert!(outcomes.iter().all(|outcome| outcome.failure.is_some()));
        assert!(outcomes.iter().all(|outcome| outcome.output_paths == vec![published.clone()]));
    }

    #[test]
    fn crop_command_rejects_empty_selected_scope_before_processing() {
        let root = sandbox("crop-empty-selection");
        let input = root.join("source.pdf"); make_pdf(&input, 1);
        let original = std::fs::read(&input).unwrap();
        let outcomes = tauri::async_runtime::block_on(crop_pdf(CropPdfRequest {
            paths: vec![input.clone()],
            rectangle: PdfRect { x: 1.0, y: 1.0, width: 100.0, height: 100.0 },
            scope: PageScope::Selected { pages: vec![] },
            output_location: location(&root, "crop-empty"),
        }));

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].input_path, input);
        assert!(outcomes[0].output_paths.is_empty());
        assert!(outcomes[0].failure.as_ref().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput)));
        assert!(outcomes[0].failure.as_ref().is_some_and(|error| error.message.contains("at least one page")));
        assert_eq!(std::fs::read(&input).unwrap(), original);
        assert_eq!(std::fs::read_dir(root.join("crop-empty")).unwrap().count(), 0);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sign_command_rejects_empty_selected_scope_at_the_boundary() {
        let root = sandbox("sign-empty-selection");
        let input = root.join("source.pdf");
        make_pdf(&input, 1);
        let outcomes = tauri::async_runtime::block_on(sign_pdf(SignPdfRequest {
            paths: vec![input.clone()],
            page: 0,
            text: "signed".to_string(),
            signature_path: None,
            rectangle: PdfRect { x: 1.0, y: 1.0, width: 100.0, height: 100.0 },
            scope: PageScope::Selected { pages: vec![] },
            output_location: location(&root, "sign-empty"),
        }));

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].input_path, input);
        assert!(outcomes[0].output_paths.is_empty());
        assert!(outcomes[0].failure.as_ref().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput)));
        assert!(outcomes[0].failure.as_ref().is_some_and(|error| error.message.contains("at least one page")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn organize_command_rejects_empty_selected_scope_before_processing() {
        let root = sandbox("organize-empty-selection");
        let input = root.join("source.pdf"); make_pdf(&input, 1);
        let outcomes = tauri::async_runtime::block_on(organize_pdf(OrganizePdfRequest {
            paths: vec![input.clone()], page_order: vec![], delete_pages: vec![], rotate_pages: vec![],
            scope: PageScope::Selected { pages: vec![] }, output_location: location(&root, "organize-empty"),
        }));

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].input_path, input);
        assert!(outcomes[0].output_paths.is_empty());
        assert!(outcomes[0].failure.as_ref().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput)));
        assert!(outcomes[0].failure.as_ref().is_some_and(|error| error.message.contains("at least one page")));
        assert_eq!(std::fs::read_dir(root.join("organize-empty")).unwrap().count(), 0);

        let _ = std::fs::remove_dir_all(root);
    }

    fn assert_optional_adapter<F>(name: &str, future: F)
    where F: Future<Output = Vec<JobOutcome>> {
        let outcomes = tauri::async_runtime::block_on(future);
        assert_eq!(outcomes.len(), 1, "{name} should return one input outcome");
        let outcome = &outcomes[0];
        if let Some(error) = &outcome.failure {
            assert!(matches!(error.kind, crate::kit::contracts::ErrorKind::Unavailable | crate::kit::contracts::ErrorKind::Processing), "{name} failed for an unexpected reason: {error}");
            assert!(outcome.output_paths.is_empty(), "{name} must not report an output after an adapter failure");
        } else {
            assert!(!outcome.output_paths.is_empty(), "{name} adapter reported success without an output");
            for output in &outcome.output_paths { assert!(output.is_file(), "{name} reported missing output {}", output.display()); }
        }
    }

    #[test]
    fn every_registered_command_runs_with_real_fixtures_in_isolated_sandboxes() {
        let _guard = crate::kit::PROCESS_ENV_LOCK.lock().unwrap();
        let root = sandbox("all");
        let image = root.join("image.png"); write_png(&image, 256, 44);
        let image_two = root.join("image-two.png"); write_png(&image_two, 256, 88);
        let animated = root.join("animated.gif");
        {
            use image::codecs::gif::{GifEncoder, Repeat};
            use image::{Frame, RgbaImage};
            let file = std::fs::File::create(&animated).unwrap();
            let mut encoder = GifEncoder::new(file); encoder.set_repeat(Repeat::Infinite).unwrap();
            encoder.encode_frame(Frame::new(RgbaImage::from_pixel(256, 256, Rgba([255, 0, 0, 255])))).unwrap();
            encoder.encode_frame(Frame::new(RgbaImage::from_pixel(256, 256, Rgba([0, 0, 255, 255])))).unwrap();
        }
        let tiff = root.join("image.tiff"); image::open(&image).unwrap().save(&tiff).unwrap();
        let pdf = root.join("document.pdf"); make_pdf(&pdf, 2);
        let pdf_two = root.join("document-two.pdf"); make_pdf(&pdf_two, 1);
        let plain_pdf_bytes = std::fs::read(&pdf).unwrap();
        let plain_image_bytes = std::fs::read(&image).unwrap();
        let animated_bytes = std::fs::read(&animated).unwrap();
        let tiff_bytes = std::fs::read(&tiff).unwrap();

        let protected = root.join("protected.pdf");
        let protected_result = protect_pdf(PdfRequest { paths: vec![pdf.clone()], password: "test-password".into(), output_location: OutputLocation::AlongsideInput });
        let protected_output = assert_success("protect fixture", protected_result)[0].clone();
        std::fs::rename(protected_output, &protected).unwrap();

        let image_pdf = root.join("image-pdf.pdf");
        let image_pdf_result = images_to_pdf(ImagesToPdfRequest { paths: vec![image.clone()], output_location: OutputLocation::AlongsideInput });
        let image_pdf_output = assert_success("images-to-pdf fixture", image_pdf_result)[0].clone();
        std::fs::rename(image_pdf_output, &image_pdf).unwrap();

        let mut outputs = Vec::new();
        let inspected_scene = tauri::async_runtime::block_on(inspect_pdf_scene(InspectPdfRequest { path: pdf.clone() })).unwrap();
        assert_eq!(inspected_scene.pages.len(), 2);
        let visual_scene: scene::PdfScene = serde_json::from_value(serde_json::json!({ "pages": [
            { "sourceIndex": 0, "width": 612, "height": 792, "rotation": 0, "crop": null, "objects": [
                { "kind": "highlight", "rect": { "x": 65, "y": 62, "width": 200, "height": 30 }, "text": "", "fontSize": 18, "color": "#FFD700", "opacity": 0.35, "strokes": [] },
                { "kind": "text", "rect": { "x": 72, "y": 150, "width": 360, "height": 40 }, "text": "Edited entirely on this device", "fontSize": 18, "color": "#1455AA", "opacity": 1, "strokes": [] },
                { "kind": "shape", "rect": { "x": 65, "y": 145, "width": 380, "height": 50 }, "text": "", "fontSize": 18, "color": "#1455AA", "opacity": 1, "strokes": [] },
                { "kind": "watermark", "rect": { "x": 120, "y": 340, "width": 370, "height": 80 }, "text": "PRIVATE DRAFT", "fontSize": 40, "color": "#AAAAAA", "opacity": 0.4, "strokes": [] },
                { "kind": "signature", "rect": { "x": 90, "y": 580, "width": 200, "height": 50 }, "text": "", "fontSize": 18, "color": "#202020", "opacity": 1, "strokes": [[{"x":0,"y":0.8},{"x":0.2,"y":0.1},{"x":0.15,"y":1},{"x":0.6,"y":0.2},{"x":0.4,"y":0.8},{"x":1,"y":0.4}]] }
            ] },
            { "sourceIndex": null, "width": 612, "height": 792, "rotation": 90, "crop": {"x": 20, "y": 20, "width": 550, "height": 700}, "objects": [
                { "kind": "text", "rect": { "x": 72, "y": 100, "width": 350, "height": 40 }, "text": "New page in the same session", "fontSize": 18, "color": "#202020", "opacity": 1, "strokes": [] }
            ] }
        ] })).unwrap();
        let rendered_scene = tauri::async_runtime::block_on(preview_pdf_scene(scene::PreviewRequest { path: pdf.clone(), scene: visual_scene.clone(), page_index: 0 })).unwrap();
        assert!(rendered_scene.data_url.starts_with("data:image/png;base64,"));
        outputs.extend(assert_success("PDF scene", export_pdf_scene(scene::ExportRequest { paths: vec![pdf.clone()], scene: visual_scene, output_location: OutputLocation::AlongsideInput })));
        outputs.extend(assert_success("unlock", remove_password(PasswordRequest { paths: vec![protected], password: "test-password".into(), output_location: location(&root, "unlock") })));
        outputs.extend(assert_success("page numbers", add_page_numbers(PageOverlayRequest { paths: vec![pdf.clone()], text: "1".into(), opacity: 100, position: Some("bottom-right".into()), logo_path: None, pages: None, start_number: Some(1), font_size: Some(12), output_location: location(&root, "page numbers") })));
        outputs.extend(assert_aggregate_success("merge", &[pdf.clone(), pdf_two.clone()], merge_pdfs(MergePdfRequest { paths: vec![pdf.clone(), pdf_two.clone()], output_location: location(&root, "merge") })));
        outputs.extend(assert_success("watermark pdf", watermark_pdf(PageOverlayRequest { paths: vec![pdf.clone()], text: "TEST".into(), opacity: 70, position: Some("center".into()), logo_path: None, pages: None, start_number: None, font_size: None, output_location: location(&root, "watermark pdf") })));
        outputs.extend(assert_success("crop pdf", crop_pdf(CropPdfRequest { paths: vec![pdf.clone()], rectangle: PdfRect { x: 0.5, y: 0.5, width: 500.0, height: 700.0 }, scope: PageScope::All, output_location: location(&root, "crop pdf") })));
        outputs.extend(assert_success("edit pdf", edit_pdf(EditPdfRequest { paths: vec![pdf.clone()], mode: "highlight".into(), text: "TEST NOTE".into(), pages: None, rectangle: PdfRect { x: 40.0, y: 650.0, width: 220.0, height: 60.0 }, output_location: location(&root, "edit pdf") })));
        outputs.extend(assert_success("composed pdf editor", edit_pdf_session(PdfEditSessionRequest {
            paths: vec![pdf.clone()],
            plan: PdfEditSessionPlan {
                page_order: vec![1, 0],
                delete_pages: vec![],
                rotate_pages: vec![RotatePage { page: 0, degrees: 90 }],
                operations: vec![
                    PdfEditOperation::Overlay { overlay: PdfOverlay::Edit { mode: PdfEditMode::Highlight, text: "COMPOSED".into(), pages: None, rectangle: PdfRect { x: 40.0, y: 650.0, width: 220.0, height: 60.0 } } },
                    PdfEditOperation::Overlay { overlay: PdfOverlay::Watermark { text: "WATERMARK".into(), opacity: 65, position: Some(PdfOverlayPosition::Center), logo_path: None, pages: None } },
                ],
            },
            output_location: location(&root, "composed pdf editor"),
        })));
        outputs.extend(assert_success("protect", protect_pdf(PdfRequest { paths: vec![pdf_two.clone()], password: "another-password".into(), output_location: location(&root, "protect") })));
        outputs.extend(assert_aggregate_success("images to pdf", &[image.clone(), image_two.clone()], images_to_pdf(ImagesToPdfRequest { paths: vec![image.clone(), image_two.clone()], output_location: location(&root, "images to pdf") })));
        outputs.extend(assert_success("pdf to images", pdf_to_images(PdfToImagesRequest { paths: vec![pdf.clone()], dpi: 72, format: "png".into(), page_range: None, pages: None, output_location: location(&root, "pdf to images") })));
        let selected_image_outputs = assert_success("pdf selected pages to images", pdf_to_images(PdfToImagesRequest { paths: vec![pdf.clone()], dpi: 72, format: "png".into(), page_range: None, pages: Some(vec![1]), output_location: location(&root, "pdf selected pages to images") }));
        assert_eq!(selected_image_outputs.len(), 1, "selected PDF page rendering must produce only the selected page");
        outputs.extend(selected_image_outputs);
        outputs.extend(assert_success("pdf to text", pdf_to_text(PdfToTextRequest { paths: vec![pdf.clone()], pages: None, output_location: location(&root, "pdf to text") })));
        outputs.extend(assert_success("split", split_pdf(PageSelectionRequest { paths: vec![pdf.clone()], pages: vec![], page_ranges: None, split_mode: Some("pages".into()), chunk_size: None, output_location: location(&root, "split") })));
        outputs.extend(assert_success("extract pdf images", extract_pdf_images(PdfToTextRequest { paths: vec![image_pdf.clone()], pages: None, output_location: location(&root, "extract pdf images") })));
        outputs.extend(assert_success("sign", sign_pdf(SignPdfRequest { paths: vec![pdf.clone()], page: 0, text: "Signed".into(), signature_path: None, rectangle: PdfRect { x: 40.0, y: 40.0, width: 180.0, height: 60.0 }, scope: PageScope::All, output_location: location(&root, "sign") })));
        assert_optional_adapter("ocr", ocr_pdf(VisionRequest { paths: vec![pdf.clone()], pages: None, output_location: location(&root, "ocr") }));
        outputs.extend(assert_success("remove pages", remove_pdf_pages(PageSelectionRequest { paths: vec![pdf.clone()], pages: vec![0], page_ranges: None, split_mode: None, chunk_size: None, output_location: location(&root, "remove pages") })));
        outputs.extend(assert_success("extract pages", extract_pdf_pages(PageSelectionRequest { paths: vec![pdf.clone()], pages: vec![], page_ranges: Some("1".into()), split_mode: None, chunk_size: None, output_location: location(&root, "extract pages") })));
        outputs.extend(assert_success("organize", organize_pdf(OrganizePdfRequest { paths: vec![pdf.clone()], page_order: vec![1, 0], delete_pages: vec![], rotate_pages: vec![RotatePage { page: 0, degrees: 90 }], scope: PageScope::All, output_location: location(&root, "organize") })));
        let added = assert_success(
            "add pages",
            add_pdf_pages(AddPdfPagesRequest {
                paths: vec![pdf.clone()],
                position: "after".into(),
                page: 0,
                count: 2,
                output_location: location(&root, "add pages"),
            }),
        );
        let added_document = lopdf::Document::load(&added[0]).unwrap();
        assert_eq!(added_document.get_pages().len(), 4, "add pages must insert the requested number of pages");
        outputs.extend(added);
        outputs.extend(assert_success("compress pdf", compress_pdf(CompressPdfRequest { paths: vec![pdf.clone()], quality: 80, output_location: location(&root, "compress pdf") })));
        outputs.extend(assert_success("convert", convert_images(ConvertImagesRequest { paths: vec![image.clone()], format: "jpg".into(), output_location: location(&root, "convert") })));
        outputs.extend(assert_success("compress images", compress_images(CompressImagesRequest { paths: vec![image.clone()], quality: 80, lossless: false, output_location: location(&root, "compress images") })));
        outputs.extend(assert_success("resize", resize_images(ResizeRequest { paths: vec![image.clone()], width: 128, height: 128, mode: "exact".into(), resampling: "lanczos".into(), keep_aspect_ratio: true, percentage: 100, longest_side: 128, output_location: location(&root, "resize") })));
        outputs.extend(assert_success("rotate", rotate_images(RotateRequest { paths: vec![image.clone()], degrees: 90, flip: "none".into(), output_location: location(&root, "rotate") })));
        outputs.extend(assert_success("crop image", crop_images(CropRequest { paths: vec![image.clone()], x: 0, y: 0, width: 128, height: 128, mode: "rectangle".into(), aspect_width: 128, aspect_height: 128, anchor: "center".into(), output_location: location(&root, "crop image") })));
        outputs.extend(assert_success("icons", generate_icon_set(IconSetRequest { paths: vec![image.clone()], preset: "favicon".into(), sizes: vec![], output_location: location(&root, "icons") })));
        outputs.extend(assert_aggregate_success("create gif", &[image.clone(), image_two.clone()], create_gif(GifCreateRequest { paths: vec![image.clone(), image_two.clone()], frame_delay_ms: 100, loop_forever: true, output_location: location(&root, "create gif") })));
        outputs.extend(assert_success("extract gif", extract_gif_frames(GifExtractRequest { paths: vec![animated.clone()], output_location: location(&root, "extract gif") })));
        outputs.extend(assert_success("watermark image", watermark_images(WatermarkRequest { paths: vec![image.clone()], opacity: 70, text: Some("TEST".into()), logo_path: None, x: 16, y: 16, output_location: location(&root, "watermark image") })));
        let image_plan = ImageEditPlan {
            edits: vec![
                ImageEdit::Resize { width: 128, height: 128, mode: "exact".into(), percentage: 100, longest_side: 128, resampling: "lanczos".into(), keep_aspect_ratio: false },
                ImageEdit::Rotate { degrees: 90, flip: "horizontal".into() },
            ],
            output_location: location(&root, "composed image editor"),
            suffix: "-session".into(),
        };
        let image_preview = inspect_image_edit_preview(ImageEditPreviewRequest { path: image.clone(), plan: image_plan.clone() }).expect("composed image preview should be available");
        assert_eq!((image_preview.width, image_preview.height), (128, 128), "composed image preview must reflect the complete edit plan");
        outputs.extend(assert_success("composed image editor", export_image_edit_plan(ImageEditExportRequest { paths: vec![image.clone()], plan: image_plan })));
        outputs.extend(assert_success("metadata", image_metadata(MetadataRequest { paths: vec![image.clone()], output_location: location(&root, "metadata") })));
        outputs.extend(assert_success("tone", adjust_image_tone(ToneRequest { paths: vec![image.clone()], brightness: 20, contrast: 0.0, saturation: 0.0, exposure: 0.0, output_location: location(&root, "tone") })));
        outputs.extend(assert_aggregate_success("tiff", &[tiff.clone()], process_tiff_pages(TiffRequest { paths: vec![tiff.clone()], pages: None, output_location: location(&root, "tiff") })));
        assert_optional_adapter("face blur", blur_faces(VisionRequest { paths: vec![image.clone()], pages: None, output_location: location(&root, "face blur") }));
        assert_optional_adapter("background removal", remove_image_background(VisionRequest { paths: vec![image.clone()], pages: None, output_location: location(&root, "background removal") }));

        assert_eq!(std::fs::read(&pdf).unwrap(), plain_pdf_bytes, "native E2E commands must not modify their PDF input");
        assert_eq!(std::fs::read(&image).unwrap(), plain_image_bytes, "native E2E commands must not modify their image input");
        assert_eq!(std::fs::read(&animated).unwrap(), animated_bytes, "native E2E commands must not modify their GIF input");
        assert_eq!(std::fs::read(&tiff).unwrap(), tiff_bytes, "native E2E commands must not modify their TIFF input");
        assert!(!outputs.is_empty(), "native E2E matrix must exercise every producing command");
        if std::env::var_os("TOOLBOX_KEEP_NATIVE_E2E").is_none() {
            let _ = std::fs::remove_dir_all(root);
        } else {
            eprintln!("native e2e artifacts retained at {}", root.display());
        }
    }

    #[test]
    fn convert_command_heic_and_webp() {
        let src = temp_path("cmd_convert.png");
        write_png(&src, 256, 44);

        let heic = tauri::async_runtime::block_on(convert_images(ConvertImagesRequest {
            paths: vec![src.clone()], format: "heic".to_string(), output_location: OutputLocation::AlongsideInput,
        }));
        assert!(heic[0].failure.is_none(), "{}", heic[0].failure.clone().unwrap_or_default());
        assert_eq!(
            heic[0].output_paths[0].extension().and_then(|e| e.to_str()),
            Some("heic")
        );
        let bytes = std::fs::read(&heic[0].output_paths[0]).unwrap();
        assert!(heif::decode(&bytes).is_ok(), "command output must be real HEIC");

        let webp = tauri::async_runtime::block_on(convert_images(ConvertImagesRequest {
            paths: vec![src.clone()], format: "webp".to_string(), output_location: OutputLocation::AlongsideInput,
        }));
        assert!(webp[0].failure.is_none(), "{}", webp[0].failure.clone().unwrap_or_default());
        let bytes = std::fs::read(&webp[0].output_paths[0]).unwrap();
        assert_eq!(&bytes[..4], b"RIFF", "command output must be real WebP");

        cleanup(&heic[0].output_paths);
        cleanup(&webp[0].output_paths);
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn single_input_convert_rejects_multiple_files() {
        let a = temp_path("cmd_a.png");
        let b = temp_path("cmd_b.png");
        write_png(&a, 128, 1);
        write_png(&b, 128, 2);

        let out = tauri::async_runtime::block_on(convert_images(ConvertImagesRequest {
            paths: vec![a.clone(), b.clone()], format: "jpg".to_string(), output_location: OutputLocation::AlongsideInput,
        }));
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|job| job.failure.as_ref().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput))));
        assert!(out.iter().all(|job| job.output_paths.is_empty()));

        cleanup(&out[0].output_paths);
        cleanup(&out[1].output_paths);
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn convert_command_rejects_bogus_and_unsupported_formats_without_writing() {
        for format in ["gif", "bogus"] {
            let root = sandbox(&format!("convert-invalid-{format}"));
            let src = root.join("source.png");
            write_png(&src, 32, 4);
            let original = std::fs::read(&src).unwrap();
            let outcomes = tauri::async_runtime::block_on(convert_images(ConvertImagesRequest {
                paths: vec![src.clone()], format: format.to_string(), output_location: location(&root, "output"),
            }));
            assert_eq!(outcomes.len(), 1);
            assert!(outcomes[0].failure.as_ref().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput)));
            assert!(outcomes[0].failure.as_ref().is_some_and(|error| error.message.contains("Unsupported image format")));
            assert!(outcomes[0].output_paths.is_empty());
            assert_eq!(std::fs::read(&src).unwrap(), original);
            assert_eq!(std::fs::read_dir(root.join("output")).unwrap().count(), 0);
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn compress_command_keeps_original_format() {
        let src = temp_path("cmd_compress.png");
        write_png(&src, 128, 7);
        let before = std::fs::metadata(&src).unwrap().len();

        let out = tauri::async_runtime::block_on(compress_images(CompressImagesRequest {
            paths: vec![src.clone()], quality: 50, lossless: false, output_location: OutputLocation::AlongsideInput,
        }));
        assert!(out[0].failure.is_none(), "{}", out[0].failure.clone().unwrap_or_default());
        assert_eq!(
            out[0].output_paths[0].extension().and_then(|e| e.to_str()),
            Some("png"),
            "compress keeps the source format"
        );
        assert_eq!(std::fs::metadata(&src).unwrap().len(), before, "originals untouched");

        cleanup(&out[0].output_paths);
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn metadata_command_keeps_batch_failures_isolated() {
        let valid = temp_path("cmd_mixed_valid.png");
        let missing = temp_path("cmd_mixed_missing.png");
        write_png(&valid, 64, 9);

        let out = tauri::async_runtime::block_on(image_metadata(MetadataRequest {
            paths: vec![valid.clone(), missing.clone()],
            output_location: OutputLocation::AlongsideInput,
        }));
        assert_eq!(out.len(), 2);
        assert_eq!(out.iter().map(|job| job.input_path.clone()).collect::<Vec<_>>(), vec![valid.clone(), missing.clone()]);
        assert!(out.iter().any(|job| job.failure.is_none()));
        assert!(out.iter().any(|job| job.failure.is_some()));

        cleanup(&out.iter().flat_map(|job| job.output_paths.clone()).collect::<Vec<_>>());
        let _ = std::fs::remove_file(valid);
    }

    #[test]
    fn page_commands_reject_empty_explicit_selections() {
        let root = sandbox("empty-pages");
        let pdf = root.join("document.pdf");
        make_pdf(&pdf, 2);

        let outcomes = tauri::async_runtime::block_on(pdf_to_text(PdfToTextRequest {
            paths: vec![pdf.clone()],
            pages: Some(Vec::new()),
            output_location: location(&root, "text"),
        }));

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].input_path, pdf);
        assert!(outcomes[0].output_paths.is_empty());
        assert!(outcomes[0].failure.as_ref().is_some_and(|error| error.message.contains("at least one page")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn collision_safe_outputs_preserve_existing_files() {
        let root = sandbox("collision");
        let image = root.join("image.png");
        write_png(&image, 32, 12);
        let original = std::fs::read(&image).unwrap();
        let existing = root.join("image-compressed.png");
        std::fs::write(&existing, b"existing output").unwrap();

        let outcomes = tauri::async_runtime::block_on(compress_images(CompressImagesRequest {
            paths: vec![image.clone()],
            quality: 80,
            lossless: false,
            output_location: OutputLocation::AlongsideInput,
        }));

        assert!(outcomes[0].failure.is_none(), "{}", outcomes[0].failure.clone().unwrap_or_default());
        assert_ne!(outcomes[0].output_paths[0], existing);
        assert_eq!(std::fs::read(&existing).unwrap(), b"existing output");
        assert_eq!(std::fs::read(&image).unwrap(), original);
        cleanup(&outcomes[0].output_paths);
        let _ = std::fs::remove_dir_all(root);
    }
}
