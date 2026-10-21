#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod file_actions;
mod intake;
mod kit;
mod settings;

use crate::file_actions::{open_output_path, reveal_output_path};
use crate::kit::common::{note_renamed, JobOutcome};
use crate::kit::images::{ImageProcessor, Options as ImageOptions};
use crate::kit::images::tools;
use crate::kit::pdf::{editor, metadata, remaining, scene, PDFProcessor};
use crate::kit::vision;
use crate::kit::contracts::{command_supports_preview, validate_command_inputs, validate_page_selection, CompressImagesRequest, ConvertImagesRequest, OutcomeState, PasswordRequest, ToolError};
use crate::kit::common::batch_runner::{BatchRunner, FileEvent, FolderCheck, RunEvents};
use crate::kit::password::PasswordProcessor;
use std::sync::Arc;

const BATCH_PROGRESS_EVENT: &str = "batch-progress";

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchProgress<'a> {
    run_id: &'a str,
    #[serde(flatten)]
    event: &'a FileEvent,
}

// Callers that send no runId keep the old silent behavior.
impl<'de> tauri::ipc::CommandArg<'de, tauri::Wry> for RunEvents {
    fn from_command(command: tauri::ipc::CommandItem<'de, tauri::Wry>) -> Result<Self, tauri::ipc::InvokeError> {
        let webview = command.message.webview();
        let run_id = <Option<String> as tauri::ipc::CommandArg<'de, tauri::Wry>>::from_command(tauri::ipc::CommandItem { key: "runId", ..command })?;
        Ok(match run_id {
            None => RunEvents::silent(),
            Some(run_id) => RunEvents::for_run(run_id.clone(), move |event| {
                use tauri::Emitter;
                let _ = webview.emit(BATCH_PROGRESS_EVENT, BatchProgress { run_id: &run_id, event: &event });
            }),
        })
    }
}

#[tauri::command]
fn cancel_run(run_id: String) -> bool {
    crate::kit::common::batch_runner::cancel_run(&run_id)
}

#[tauri::command]
fn check_output_folder(folder: std::path::PathBuf) -> FolderCheck {
    crate::kit::common::batch_runner::check_output_folder(&folder)
}

#[tauri::command]
async fn remove_password(events: RunEvents, request: PasswordRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "remove_password", request.paths.clone(), &request.output_location, &request, |request, path| {
        let password = request.password_for(&path);
        PasswordProcessor::remove_password(path, password, &request.output_location)
    })
}

#[tauri::command]
async fn protect_files(events: RunEvents, request: PasswordRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "protect_files", request.paths.clone(), &request.output_location, &request, |request, path| {
        let password = request.password_for(&path);
        PasswordProcessor::protect(path, password, &request.output_location)
    })
}

#[tauri::command]
async fn compress_images(events: RunEvents, request: CompressImagesRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "compress_images", request.paths.clone(), &request.output_location, &request, |request, path| {
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
async fn convert_images(events: RunEvents, request: ConvertImagesRequest) -> Vec<JobOutcome> {
    let img_format = match tools::parse_output_format(&request.format) {
        Ok(format) => format,
        Err(error) => {
            if request.paths.is_empty() {
                return vec![JobOutcome::failure(std::path::PathBuf::new(), ToolError::invalid_input(error))];
            }
            return request.paths.into_iter().map(|path| JobOutcome::failure(path, ToolError::invalid_input(error.clone()))).collect();
        }
    };

    let request = Arc::new(request);
    BatchRunner::run(&events, "convert_images", request.paths.clone(), &request.output_location, &request, move |request, path| {
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

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct InspectPdfScenePageRequest {
    path: std::path::PathBuf,
    page_index: usize,
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
async fn inspect_pdf_scene_page(request: InspectPdfScenePageRequest) -> Result<metadata::PdfPageMetadata, String> {
    tauri::async_runtime::spawn_blocking(move || scene::inspect_page(&request.path, request.page_index)).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn preview_pdf_scene(request: scene::PreviewRequest) -> Result<scene::ScenePreview, String> {
    if !command_supports_preview("export_pdf_scene") { return Err("PDF scene preview is unavailable in this build.".to_string()); }
    tauri::async_runtime::spawn_blocking(move || scene::preview(&request)).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn preview_pdf_scene_pages(request: scene::PreviewPagesRequest) -> Result<Vec<scene::IndexedScenePreview>, String> {
    if !command_supports_preview("export_pdf_scene") { return Err("PDF scene preview is unavailable in this build.".to_string()); }
    tauri::async_runtime::spawn_blocking(move || scene::preview_pages(&request)).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn export_pdf_scene(events: RunEvents, request: scene::ExportRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "export_pdf_scene", request.paths.clone(), &request.output_location, &request, scene::export)
}

#[tauri::command]
async fn crop_pdf(events: RunEvents, request: editor::CropPdfRequest) -> Vec<JobOutcome> {
    let explicit_pages = match &request.scope {
        editor::PageScope::All => None,
        editor::PageScope::Selected { pages } => Some(pages.as_slice()),
    };
    if let Err(outcomes) = validate_page_selection("crop_pdf", &request.paths, explicit_pages) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "crop_pdf", request.paths.clone(), &request.output_location, &request, editor::crop)
}

#[tauri::command]
async fn sign_pdf(events: RunEvents, request: editor::SignPdfRequest) -> Vec<JobOutcome> {
    let explicit_pages = match &request.scope {
        editor::PageScope::All => None,
        editor::PageScope::Selected { pages } => Some(pages.as_slice()),
    };
    if let Err(outcomes) = validate_page_selection("sign_pdf", &request.paths, explicit_pages) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "sign_pdf", request.paths.clone(), &request.output_location, &request, editor::sign)
}

#[tauri::command]
async fn edit_pdf(events: RunEvents, request: editor::EditPdfRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_page_selection("edit_pdf", &request.paths, request.pages.as_deref()) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "edit_pdf", request.paths.clone(), &request.output_location, &request, editor::edit)
}

#[tauri::command]
async fn edit_pdf_session(events: RunEvents, request: editor::PdfEditSessionRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "edit_pdf_session", request.paths.clone(), &request.output_location, &request, editor::apply_session)
}

#[tauri::command]
async fn organize_pdf(events: RunEvents, request: editor::OrganizePdfRequest) -> Vec<JobOutcome> {
    let explicit_pages = match &request.scope {
        editor::PageScope::All => None,
        editor::PageScope::Selected { pages } => Some(pages.as_slice()),
    };
    if let Err(outcomes) = validate_page_selection("organize_pdf", &request.paths, explicit_pages) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "organize_pdf", request.paths.clone(), &request.output_location, &request, editor::organize)
}

#[tauri::command]
async fn add_pdf_pages(events: RunEvents, request: editor::AddPdfPagesRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "add_pdf_pages", request.paths.clone(), &request.output_location, &request, editor::add_pages)
}

#[tauri::command]
async fn add_page_numbers(events: RunEvents, request: remaining::PageOverlayRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_page_selection("add_page_numbers", &request.paths, request.pages.as_deref()) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "add_page_numbers", request.paths.clone(), &request.output_location, &request, remaining::add_page_numbers)
}

#[tauri::command]
async fn watermark_pdf(events: RunEvents, request: remaining::PageOverlayRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_page_selection("watermark_pdf", &request.paths, request.pages.as_deref()) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "watermark_pdf", request.paths.clone(), &request.output_location, &request, remaining::watermark)
}

#[tauri::command]
async fn compress_pdf(events: RunEvents, request: remaining::CompressPdfRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("compress_pdf", &request.paths) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "compress_pdf", request.paths.clone(), &request.output_location, &request, remaining::compress)
}

#[tauri::command]
async fn remove_pdf_pages(events: RunEvents, request: remaining::PageSelectionRequest) -> Vec<JobOutcome> {
    let explicit_pages = request.page_ranges.as_deref().filter(|range| !range.trim().is_empty()).is_none().then_some(request.pages.as_slice());
    if let Err(outcomes) = validate_page_selection("remove_pdf_pages", &request.paths, explicit_pages) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "remove_pdf_pages", request.paths.clone(), &request.output_location, &request, remaining::remove_pages)
}

#[tauri::command]
async fn extract_pdf_pages(events: RunEvents, request: remaining::PageSelectionRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("extract_pdf_pages", &request.paths) { return outcomes; }
    let explicit_pages = request.page_ranges.as_deref().filter(|range| !range.trim().is_empty()).is_none().then_some(request.pages.as_slice());
    if let Err(outcomes) = validate_page_selection("extract_pdf_pages", &request.paths, explicit_pages) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "extract_pdf_pages", request.paths.clone(), &request.output_location, &request, remaining::extract_pages)
}

#[tauri::command]
async fn merge_pdfs(app: tauri::AppHandle, request: remaining::MergePdfRequest) -> Vec<JobOutcome> {
    let outcomes = run_merge_pdfs(&request).await;
    settings::remember_chosen_save(&app, request.output_path.as_deref(), outcomes)
}

async fn run_merge_pdfs(request: &remaining::MergePdfRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("merge_pdfs", &request.paths) { return outcomes; }
    let request = request.clone();
    aggregate_outcomes("merge_pdfs", &request.paths.clone(), move || note_renamed(remaining::merge(&request), request.output_path.as_deref()))
}

#[tauri::command]
async fn split_pdf(events: RunEvents, request: remaining::PageSelectionRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("split_pdf", &request.paths) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "split_pdf", request.paths.clone(), &request.output_location, &request, remaining::split)
}

#[tauri::command]
async fn pdf_to_images(events: RunEvents, request: remaining::PdfToImagesRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("pdf_to_images", &request.paths) { return outcomes; }
    if let Err(outcomes) = validate_page_selection("pdf_to_images", &request.paths, request.pages.as_deref()) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "pdf_to_images", request.paths.clone(), &request.output_location, &request, remaining::to_images)
}

#[tauri::command]
async fn pdf_to_text(events: RunEvents, request: remaining::PdfToTextRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("pdf_to_text", &request.paths) { return outcomes; }
    if let Err(outcomes) = validate_page_selection("pdf_to_text", &request.paths, request.pages.as_deref()) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "pdf_to_text", request.paths.clone(), &request.output_location, &request, remaining::to_text)
}

#[tauri::command]
async fn extract_pdf_images(events: RunEvents, request: remaining::PdfToTextRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("extract_pdf_images", &request.paths) { return outcomes; }
    if let Err(outcomes) = validate_page_selection("extract_pdf_images", &request.paths, request.pages.as_deref()) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "extract_pdf_images", request.paths.clone(), &request.output_location, &request, remaining::extract_images)
}

#[tauri::command]
async fn images_to_pdf(app: tauri::AppHandle, request: remaining::ImagesToPdfRequest) -> Vec<JobOutcome> {
    let outcomes = run_images_to_pdf(&request).await;
    settings::remember_chosen_save(&app, request.output_path.as_deref(), outcomes)
}

async fn run_images_to_pdf(request: &remaining::ImagesToPdfRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("images_to_pdf", &request.paths) { return outcomes; }
    let request = request.clone();
    aggregate_outcomes("images_to_pdf", &request.paths.clone(), move || note_renamed(remaining::images_to_pdf(&request), request.output_path.as_deref()))
}

#[tauri::command]
async fn ocr_pdf(events: RunEvents, request: vision::VisionRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("ocr_pdf", &request.paths) { return outcomes; }
    if let Err(outcomes) = validate_page_selection("ocr_pdf", &request.paths, request.pages.as_deref()) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "ocr_pdf", request.paths.clone(), &request.output_location, &request, vision::ocr_pdf)
}
#[tauri::command]
async fn blur_faces(events: RunEvents, request: vision::VisionRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "blur_faces", request.paths.clone(), &request.output_location, &request, vision::blur_faces)
}
#[tauri::command]
async fn remove_image_background(events: RunEvents, request: vision::VisionRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "remove_image_background", request.paths.clone(), &request.output_location, &request, vision::remove_background)
}
#[tauri::command]
async fn resize_images(events: RunEvents, request: tools::ResizeRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "resize_images", request.paths.clone(), &request.output_location, &request, tools::resize)
}
#[tauri::command]
async fn rotate_images(events: RunEvents, request: tools::RotateRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "rotate_images", request.paths.clone(), &request.output_location, &request, tools::rotate)
}
#[tauri::command]
async fn crop_images(events: RunEvents, request: tools::CropRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "crop_images", request.paths.clone(), &request.output_location, &request, tools::crop)
}
#[tauri::command]
async fn adjust_image_tone(events: RunEvents, request: tools::ToneRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "adjust_image_tone", request.paths.clone(), &request.output_location, &request, tools::tone)
}
#[tauri::command]
async fn watermark_images(events: RunEvents, request: tools::WatermarkRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "watermark_images", request.paths.clone(), &request.output_location, &request, tools::watermark)
}
#[tauri::command]
async fn generate_icon_set(events: RunEvents, request: tools::IconSetRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "generate_icon_set", request.paths.clone(), &request.output_location, &request, tools::icon_set)
}
#[tauri::command]
async fn create_gif(app: tauri::AppHandle, request: tools::GifCreateRequest) -> Vec<JobOutcome> {
    let outcomes = run_create_gif(&request).await;
    settings::remember_chosen_save(&app, request.output_path.as_deref(), outcomes)
}

async fn run_create_gif(request: &tools::GifCreateRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("create_gif", &request.paths) { return outcomes; }
    let request = request.clone();
    aggregate_outcomes("create_gif", &request.paths.clone(), move || note_renamed(tools::gif_create(&request), request.output_path.as_deref()))
}
#[tauri::command]
async fn extract_gif_frames(events: RunEvents, request: tools::GifExtractRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "extract_gif_frames", request.paths.clone(), &request.output_location, &request, tools::gif_extract)
}
#[tauri::command]
async fn process_tiff_pages(app: tauri::AppHandle, request: tools::TiffRequest) -> Vec<JobOutcome> {
    let outcomes = run_process_tiff_pages(&request).await;
    settings::remember_chosen_save(&app, request.output_path.as_deref(), outcomes)
}

async fn run_process_tiff_pages(request: &tools::TiffRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("process_tiff_pages", &request.paths) { return outcomes; }
    let request = request.clone();
    aggregate_outcomes("process_tiff_pages", &request.paths.clone(), move || note_renamed(tools::tiff(&request), request.output_path.as_deref()))
}
#[tauri::command]
async fn image_metadata(events: RunEvents, request: tools::MetadataRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "image_metadata", request.paths.clone(), &request.output_location, &request, tools::strip_metadata)
}
#[tauri::command]
fn inspect_image_metadata(request: tools::MetadataRequest) -> Vec<Result<tools::MetadataReport, String>> { request.paths.into_iter().map(tools::inspect_metadata).collect() }

#[tauri::command]
fn inspect_image_preview(request: tools::ImagePreviewRequest) -> Result<tools::ImagePreview, String> { tools::inspect_preview(&request) }

#[tauri::command]
fn inspect_tiff_pages(request: tools::TiffPreviewRequest) -> Result<Vec<tools::ImagePreview>, String> { tools::inspect_tiff_pages(&request) }

#[tauri::command]
fn inspect_image_edit_preview(request: tools::ImageEditPreviewRequest) -> Result<tools::ImagePreview, String> { tools::inspect_edit_preview(&request) }

#[tauri::command]
async fn export_image_edit_plan(events: RunEvents, request: tools::ImageEditExportRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "export_image_edit_plan", request.paths.clone(), &request.plan.output_location, &request, |request, path| tools::export_edit_plan(&request.plan, path))
}

fn aggregate_outcomes<F>(command: &str, paths: &[std::path::PathBuf], job: F) -> Vec<JobOutcome>
where
    F: FnOnce() -> JobOutcome + Send + 'static,
{
    let aggregate = match crate::kit::common::limits::recorded(command) {
        Some(limits) => limits.run_all(paths, job),
        None => JobOutcome::failure(paths.first().cloned().unwrap_or_default(), ToolError::processing(format!("No processing limits are recorded for {command}."))),
    };
    combine_aggregate(paths, aggregate)
}

fn combine_aggregate(paths: &[std::path::PathBuf], aggregate: JobOutcome) -> Vec<JobOutcome> {
    if matches!(aggregate.state, OutcomeState::Succeeded { .. }) { return vec![aggregate]; }
    paths.iter().cloned().map(|input_path| {
        let state = match &aggregate.state {
            OutcomeState::Failed { output_paths, error } if input_path != aggregate.input_path => OutcomeState::Failed {
                output_paths: output_paths.clone(),
                error: ToolError::processing(format!(
                    "Aggregate operation stopped because {} failed: {}",
                    aggregate.input_path.display(), error.message
                )),
            },
            state => state.clone(),
        };
        JobOutcome { input_path, state, ..aggregate.clone() }
    }).collect()
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            open_output_path,
            reveal_output_path,
            intake::inspect_intake_paths,
            settings::get_output_settings,
            settings::set_output_settings,
            settings::get_last_save_folder,
            check_output_folder,
            cancel_run,
            remove_password,
            protect_files,
            compress_images,
            convert_images,
            inspect_pdf,
            inspect_pdf_scene,
            inspect_pdf_scene_page,
            preview_pdf_scene,
            preview_pdf_scene_pages,
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
    use crate::kit::contracts::{ErrorKind, LimitKind};
    use crate::kit::images::tools::{CropRequest, GifCreateRequest, GifExtractRequest, ImageEdit, ImageEditExportRequest, ImageEditPlan, ImageEditPreviewRequest, IconSetRequest, MetadataRequest, ResizeRequest, RotateRequest, TiffRequest, ToneRequest, WatermarkRequest};
    use crate::kit::office::{test_minimal_ooxml_package, OfficeProcessor};
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
        assert!(outcome.error().is_none(), "{name} failed: {}", outcome.error().cloned().unwrap_or_default());
        assert!(!outcome.output_paths().is_empty(), "{name} must report an output");
        for output in outcome.output_paths() { assert!(output.is_file(), "{name} reported missing output {}", output.display()); }
        outcome.output_paths().to_vec()
    }

    fn assert_aggregate_success<F>(name: &str, paths: &[PathBuf], future: F) -> Vec<PathBuf>
    where F: Future<Output = Vec<JobOutcome>> {
        let outcomes = tauri::async_runtime::block_on(future);
        assert_eq!(outcomes.len(), 1, "{name} makes one file, so it reports one outcome");
        let outcome = &outcomes[0];
        assert_eq!(outcome.input_path, paths[0], "{name} reports the combined file against the first input");
        assert!(outcome.error().is_none(), "{name} failed: {}", outcome.error().cloned().unwrap_or_default());
        assert!(!outcome.output_paths().is_empty(), "{name} must report an aggregate output");
        for output in outcome.output_paths() { assert!(output.is_file(), "{name} reported missing output {}", output.display()); }
        outcome.output_paths().to_vec()
    }

    fn assert_aggregate_failure<F>(name: &str, paths: &[PathBuf], failed_path: &PathBuf, future: F)
    where F: Future<Output = Vec<JobOutcome>> {
        let outcomes = tauri::async_runtime::block_on(future);
        assert_eq!(outcomes.len(), paths.len(), "{name} should return one outcome per input");
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths, "{name} must preserve input order");
        assert!(outcomes.iter().all(|outcome| outcome.output_paths().is_empty() && outcome.error().is_some()), "{name} must fail every input when the aggregate cannot run");
        let failed = outcomes.iter().find(|outcome| outcome.input_path == *failed_path).expect("missing input must have an outcome");
        assert!(failed.error().is_some_and(|error| error.message.contains(&failed_path.display().to_string())), "{name} must identify the failed input");
    }

    #[test]
    fn many_to_one_commands_never_replace_an_explicitly_chosen_existing_path() {
        use crate::kit::contracts::WarningKind;
        let root = sandbox("chosen-path");
        let image = root.join("first.png"); write_png(&image, 32, 44);
        let image_two = root.join("second.png"); write_png(&image_two, 32, 88);
        let pdf = root.join("first.pdf"); make_pdf(&pdf, 1);
        let pdf_two = root.join("second.pdf"); make_pdf(&pdf_two, 2);
        let tiff = root.join("first.tiff"); image::open(&image).unwrap().save(&tiff).unwrap();
        let tiff_two = root.join("second.tiff"); image::open(&image_two).unwrap().save(&tiff_two).unwrap();
        let chosen_folder = root.join("chosen");
        std::fs::create_dir_all(&chosen_folder).unwrap();
        let inputs = [&image, &image_two, &pdf, &pdf_two, &tiff, &tiff_two].map(|path| (path.clone(), std::fs::read(path).unwrap()));

        let chosen = |name: &str| {
            let path = chosen_folder.join(name);
            std::fs::write(&path, format!("existing {name}")).unwrap();
            path
        };
        let merge = chosen("statement.pdf");
        let combined = chosen("scans.pdf");
        let gif = chosen("loop.gif");
        let tiff_out = chosen("pages.tiff");
        let runs = [
            (merge.clone(), "statement-1.pdf", tauri::async_runtime::block_on(run_merge_pdfs(&MergePdfRequest { paths: vec![pdf.clone(), pdf_two.clone()], output_location: OutputLocation::AlongsideInput, output_path: Some(merge.clone()) }))),
            (combined.clone(), "scans-1.pdf", tauri::async_runtime::block_on(run_images_to_pdf(&ImagesToPdfRequest { paths: vec![image.clone(), image_two.clone()], output_location: OutputLocation::AlongsideInput, output_path: Some(combined.clone()) }))),
            (gif.clone(), "loop-1.gif", tauri::async_runtime::block_on(run_create_gif(&GifCreateRequest { paths: vec![image.clone(), image_two.clone()], frame_delay_ms: 100, loop_forever: true, output_location: OutputLocation::AlongsideInput, output_path: Some(gif.clone()) }))),
            (tiff_out.clone(), "pages-1.tiff", tauri::async_runtime::block_on(run_process_tiff_pages(&TiffRequest { paths: vec![tiff.clone(), tiff_two.clone()], pages: None, output_location: OutputLocation::AlongsideInput, output_path: Some(tiff_out.clone()) }))),
        ];
        for (existing, expected_name, outcomes) in runs {
            let chosen_name = existing.file_name().unwrap().to_string_lossy().into_owned();
            assert_eq!(outcomes.len(), 1, "{chosen_name} makes one file, so it reports one outcome");
            let outcome = &outcomes[0];
            assert!(outcome.error().is_none(), "{chosen_name} failed: {:?}", outcome.error());
            assert_eq!(outcome.output_paths(), [chosen_folder.join(expected_name)]);
            assert_eq!(outcome.warnings.iter().map(|warning| warning.kind.clone()).collect::<Vec<_>>(), [WarningKind::RenamedToAvoidReplacing { chosen_name: chosen_name.clone(), final_name: expected_name.into() }]);
            assert_eq!(std::fs::read(&existing).unwrap(), format!("existing {chosen_name}").into_bytes(), "{chosen_name} must stay byte-identical");
            assert!(chosen_folder.join(expected_name).metadata().unwrap().len() > 0);
        }
        for (path, bytes) in inputs { assert_eq!(std::fs::read(&path).unwrap(), bytes, "{} must not change", path.display()); }

        let fresh = chosen_folder.join("fresh.pdf");
        let outcomes = tauri::async_runtime::block_on(run_merge_pdfs(&MergePdfRequest { paths: vec![pdf.clone(), pdf_two.clone()], output_location: OutputLocation::AlongsideInput, output_path: Some(fresh.clone()) }));
        assert_eq!(outcomes[0].output_paths(), [fresh]);
        assert!(outcomes[0].warnings.is_empty());

        let missing_folder = root.join("gone/out.pdf");
        let outcomes = tauri::async_runtime::block_on(run_merge_pdfs(&MergePdfRequest { paths: vec![pdf.clone(), pdf_two.clone()], output_location: OutputLocation::AlongsideInput, output_path: Some(missing_folder.clone()) }));
        assert!(outcomes.iter().all(|outcome| outcome.error().is_some() && outcome.output_paths().is_empty()));
        assert!(!missing_folder.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn aggregate_commands_report_one_combined_success_and_each_missing_input() {
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

        assert_aggregate_success("merge valid inputs", &[pdf.clone(), pdf_two.clone()], run_merge_pdfs(&MergePdfRequest {
            paths: vec![pdf.clone(), pdf_two.clone()], output_location: location(&root, "merge-valid"), output_path: None,
        }));
        assert_aggregate_failure("merge missing later input", &[pdf.clone(), missing_pdf.clone()], &missing_pdf, run_merge_pdfs(&MergePdfRequest {
            paths: vec![pdf.clone(), missing_pdf.clone()], output_location: location(&root, "merge-missing"), output_path: None,
        }));
        assert_aggregate_success("images-to-pdf valid inputs", &[image.clone(), image_two.clone()], run_images_to_pdf(&ImagesToPdfRequest {
            paths: vec![image.clone(), image_two.clone()], output_location: location(&root, "images-valid"), output_path: None,
        }));
        assert_aggregate_failure("images-to-pdf missing later input", &[image.clone(), missing_image.clone()], &missing_image, run_images_to_pdf(&ImagesToPdfRequest {
            paths: vec![image.clone(), missing_image.clone()], output_location: location(&root, "images-missing"), output_path: None,
        }));
        assert_aggregate_success("GIF valid inputs", &[image.clone(), image_two.clone()], run_create_gif(&GifCreateRequest {
            paths: vec![image.clone(), image_two.clone()], frame_delay_ms: 100, loop_forever: true, output_location: location(&root, "gif-valid"), output_path: None,
        }));
        assert_aggregate_failure("GIF missing later input", &[image.clone(), missing_gif_frame.clone()], &missing_gif_frame, run_create_gif(&GifCreateRequest {
            paths: vec![image.clone(), missing_gif_frame.clone()], frame_delay_ms: 100, loop_forever: true, output_location: location(&root, "gif-missing"), output_path: None,
        }));
        assert_aggregate_success("TIFF valid inputs", &[tiff.clone(), tiff_two.clone()], run_process_tiff_pages(&TiffRequest {
            paths: vec![tiff.clone(), tiff_two.clone()], pages: None, output_location: location(&root, "tiff-valid"), output_path: None,
        }));
        assert_aggregate_failure("TIFF missing later input", &[tiff.clone(), missing_tiff.clone()], &missing_tiff, run_process_tiff_pages(&TiffRequest {
            paths: vec![tiff.clone(), missing_tiff.clone()], pages: None, output_location: location(&root, "tiff-missing"), output_path: None,
        }));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn aggregate_failures_preserve_partial_outputs_for_result_list_consumers() {
        let paths = vec![PathBuf::from("first.tiff"), PathBuf::from("second.tiff")];
        let published = PathBuf::from("first-page-1.tiff");
        let outcomes = combine_aggregate(&paths, JobOutcome::failure_with_outputs(
            paths[0].clone(),
            vec![published.clone()],
            ToolError::processing("The next TIFF page could not be published."),
        ));

        assert_eq!(outcomes.len(), paths.len());
        assert!(outcomes.iter().all(|outcome| outcome.error().is_some()));
        assert!(outcomes.iter().all(|outcome| outcome.output_paths() == vec![published.clone()]));
    }

    #[test]
    fn crop_command_rejects_empty_selected_scope_before_processing() {
        let root = sandbox("crop-empty-selection");
        let input = root.join("source.pdf"); make_pdf(&input, 1);
        let original = std::fs::read(&input).unwrap();
        let outcomes = tauri::async_runtime::block_on(crop_pdf(RunEvents::silent(), CropPdfRequest {
            paths: vec![input.clone()],
            rectangle: PdfRect { x: 1.0, y: 1.0, width: 100.0, height: 100.0 },
            scope: PageScope::Selected { pages: vec![] },
            output_location: location(&root, "crop-empty"),
        }));

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].input_path, input);
        assert!(outcomes[0].output_paths().is_empty());
        assert!(outcomes[0].error().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput)));
        assert!(outcomes[0].error().is_some_and(|error| error.message.contains("at least one page")));
        assert_eq!(std::fs::read(&input).unwrap(), original);
        assert_eq!(std::fs::read_dir(root.join("crop-empty")).unwrap().count(), 0);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sign_command_rejects_empty_selected_scope_at_the_boundary() {
        let root = sandbox("sign-empty-selection");
        let input = root.join("source.pdf");
        make_pdf(&input, 1);
        let outcomes = tauri::async_runtime::block_on(sign_pdf(RunEvents::silent(), SignPdfRequest {
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
        assert!(outcomes[0].output_paths().is_empty());
        assert!(outcomes[0].error().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput)));
        assert!(outcomes[0].error().is_some_and(|error| error.message.contains("at least one page")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn organize_command_rejects_empty_selected_scope_before_processing() {
        let root = sandbox("organize-empty-selection");
        let input = root.join("source.pdf"); make_pdf(&input, 1);
        let outcomes = tauri::async_runtime::block_on(organize_pdf(RunEvents::silent(), OrganizePdfRequest {
            paths: vec![input.clone()], page_order: vec![], delete_pages: vec![], rotate_pages: vec![],
            scope: PageScope::Selected { pages: vec![] }, output_location: location(&root, "organize-empty"),
        }));

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].input_path, input);
        assert!(outcomes[0].output_paths().is_empty());
        assert!(outcomes[0].error().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput)));
        assert!(outcomes[0].error().is_some_and(|error| error.message.contains("at least one page")));
        assert_eq!(std::fs::read_dir(root.join("organize-empty")).unwrap().count(), 0);

        let _ = std::fs::remove_dir_all(root);
    }

    fn assert_vision_adapter<F>(name: &str, future: F) -> Vec<PathBuf>
    where F: Future<Output = Vec<JobOutcome>> {
        let outcomes = tauri::async_runtime::block_on(future);
        assert_eq!(outcomes.len(), 1, "{name} should return one input outcome");
        let outcome = &outcomes[0];
        let resources_staged = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/vision/manifest.json").is_file();
        if let Some(error) = outcome.error() {
            if matches!(error.kind, crate::kit::contracts::ErrorKind::Unavailable) {
                assert!(!resources_staged && std::env::var_os("TOOLBOX_REQUIRE_VISION_RESOURCES").is_none(), "{name} resources were staged but the adapter was unavailable: {error}");
            }
            assert!(outcome.output_paths().is_empty(), "{name} must not report an output after an adapter failure");
        } else {
            assert!(!outcome.output_paths().is_empty(), "{name} adapter reported success without an output");
            for output in outcome.output_paths() { assert!(output.is_file(), "{name} reported missing output {}", output.display()); }
        }
        outcome.output_paths().to_vec()
    }

    fn assert_ocr_adapter<F>(name: &str, future: F) -> Vec<PathBuf>
    where F: Future<Output = Vec<JobOutcome>> {
        let outcomes = tauri::async_runtime::block_on(future);
        assert_eq!(outcomes.len(), 1, "{name} should return one input outcome");
        let outcome = &outcomes[0];
        let resources_staged = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/ocr/manifest.json").is_file();
        if let Some(error) = outcome.error() {
            if matches!(error.kind, crate::kit::contracts::ErrorKind::Unavailable) {
                assert!(!resources_staged && std::env::var_os("TOOLBOX_REQUIRE_OCR_RESOURCES").is_none(), "{name} resources were staged but OCR was unavailable: {error}");
            }
        } else {
            assert!(!outcome.output_paths().is_empty(), "{name} reported success without an output");
            for output in outcome.output_paths() { assert!(output.is_file(), "{name} reported missing output {}", output.display()); }
            let text = std::fs::read_to_string(&outcome.output_paths()[0]).unwrap_or_default();
            assert!(text.contains("Toolbox page"), "{name} did not extract the fixture text: {text:?}");
        }
        outcome.output_paths().to_vec()
    }

    fn named(naming: &mut Vec<(&'static str, PathBuf, Vec<PathBuf>)>, command: &'static str, input: PathBuf, outputs: Vec<PathBuf>) -> Vec<PathBuf> {
        naming.push((command, input, outputs.clone()));
        outputs
    }

    // Icon presets and single-file TIFF splitting predate declared suffixes and keep their
    // shipped names, so these are the only outputs allowed a different suffix family.
    const LEGACY_SUFFIX_FAMILIES: &[(&str, &str)] = &[("generate_icon_set", "-favicon"), ("process_tiff_pages", "-page")];

    fn assert_outputs_use_declared_suffixes(naming: &[(&'static str, PathBuf, Vec<PathBuf>)]) {
        let covered = naming.iter().map(|(command, _, _)| *command).collect::<std::collections::BTreeSet<_>>();
        let registered = crate::kit::contracts::native_capabilities().iter().map(|capability| capability.command.as_str()).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(covered, registered, "the sweep must exercise every registered command");
        for (command, input, outputs) in naming {
            let capability = crate::kit::contracts::native_capability(command).unwrap();
            let stem = input.file_stem().and_then(|value| value.to_str()).unwrap();
            let declared = format!("{stem}{}", capability.output_suffix);
            let legacy = LEGACY_SUFFIX_FAMILIES.iter().find(|(legacy_command, _)| legacy_command == command).map(|(_, suffix)| format!("{stem}{suffix}"));
            for output in outputs {
                let name = output.file_name().and_then(|value| value.to_str()).unwrap();
                let follows = |prefix: &str| name.strip_prefix(prefix).is_some_and(|rest| rest.starts_with(['.', '-']) || rest.is_empty());
                assert!(
                    follows(&declared) || legacy.as_deref().is_some_and(follows),
                    "{command} wrote {name}, which does not use its declared suffix {}", capability.output_suffix
                );
            }
        }
    }

    struct Original { path: PathBuf, len: u64, modified: std::time::SystemTime }

    impl Original {
        fn of(path: &PathBuf) -> Self {
            let metadata = std::fs::metadata(path).unwrap();
            Self { path: path.clone(), len: metadata.len(), modified: metadata.modified().unwrap() }
        }

        fn assert_untouched(&self) {
            let metadata = std::fs::metadata(&self.path).unwrap();
            assert_eq!((metadata.len(), metadata.modified().unwrap()), (self.len, self.modified), "{} was modified", self.path.display());
        }
    }

    fn assert_only_offender_limited(outcomes: &[JobOutcome], offender: &PathBuf, limit: LimitKind, recorded_value: &str) {
        assert_eq!(outcomes.len(), 2);
        let (limited, other) = if outcomes[0].input_path == *offender { (&outcomes[0], &outcomes[1]) } else { (&outcomes[1], &outcomes[0]) };
        assert_eq!(limited.input_path, *offender);
        let error = limited.error().expect("the over-limit file must fail");
        assert_eq!(error.kind, ErrorKind::LimitExceeded { limit }, "{}", error.message);
        assert!(error.message.contains(recorded_value), "{} should state {recorded_value}", error.message);
        assert!(limited.output_paths().is_empty());
        assert!(other.error().is_none(), "the other file must still succeed: {}", other.error().cloned().unwrap_or_default());
        assert!(other.output_paths().iter().all(|output| output.is_file()) && !other.output_paths().is_empty());
    }

    fn pdf_to_text_pair(root: &PathBuf, offender: &PathBuf, name: &str) -> (PathBuf, Vec<JobOutcome>) {
        let good = root.join("good.pdf");
        make_pdf(&good, 1);
        let originals = [Original::of(offender), Original::of(&good)];
        let outcomes = tauri::async_runtime::block_on(pdf_to_text(RunEvents::silent(), PdfToTextRequest {
            paths: vec![offender.clone(), good.clone()], pages: None, output_location: location(root, name),
        }));
        for original in &originals { original.assert_untouched(); }
        (good, outcomes)
    }

    #[test]
    fn a_pdf_over_the_recorded_size_fails_alone() {
        let root = sandbox("limit-size");
        let offender = root.join("oversized.pdf");
        std::fs::write(&offender, b"%PDF-1.7\n").unwrap();
        std::fs::OpenOptions::new().write(true).open(&offender).unwrap().set_len(1_000_000_001).unwrap();
        let (_, outcomes) = pdf_to_text_pair(&root, &offender, "out");
        assert_only_offender_limited(&outcomes, &offender, LimitKind::FileSize, "1 GB");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_pdf_over_the_recorded_page_count_fails_alone() {
        let root = sandbox("limit-pages");
        let offender = root.join("long.pdf");
        make_pdf(&offender, 5_001);
        let (_, outcomes) = pdf_to_text_pair(&root, &offender, "out");
        assert_only_offender_limited(&outcomes, &offender, LimitKind::PageCount, "5,000");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_pdf_stream_expanding_past_the_recorded_size_fails_alone() {
        let root = sandbox("limit-pdf-expansion");
        let offender = root.join("bomb.pdf");
        std::fs::write(&offender, crate::kit::common::limits::fixtures::pdf_with_stream(&crate::kit::common::limits::fixtures::zlib_of_zeros(500_000_001))).unwrap();
        let (_, outcomes) = pdf_to_text_pair(&root, &offender, "out");
        assert_only_offender_limited(&outcomes, &offender, LimitKind::Decompression, "500 MB");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_image_decoding_past_the_recorded_size_fails_alone() {
        let root = sandbox("limit-image-expansion");
        let offender = root.join("huge.png");
        std::fs::write(&offender, crate::kit::common::limits::fixtures::png_header(11_200, 11_200)).unwrap();
        let good = root.join("good.png");
        write_png(&good, 32, 9);
        let originals = [std::fs::read(&offender).unwrap(), std::fs::read(&good).unwrap()];
        let outcomes = tauri::async_runtime::block_on(image_metadata(RunEvents::silent(), MetadataRequest {
            paths: vec![offender.clone(), good.clone()], output_location: location(&root, "out"),
        }));
        assert_eq!([std::fs::read(&offender).unwrap(), std::fs::read(&good).unwrap()], originals);
        assert_only_offender_limited(&outcomes, &offender, LimitKind::Decompression, "500 MB");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_file_past_its_time_limit_fails_alone_and_others_finish() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let root = sandbox("limit-time");
        let slow = root.join("slow.pdf");
        let good = root.join("good.pdf");
        make_pdf(&slow, 1);
        make_pdf(&good, 1);
        let originals = [std::fs::read(&slow).unwrap(), std::fs::read(&good).unwrap()];
        let request = Arc::new(PdfToTextRequest { paths: vec![slow.clone(), good.clone()], pages: None, output_location: location(&root, "out") });
        let released = Arc::new(AtomicBool::new(false));
        let limits = crate::kit::common::limits::ToolLimits { max_duration: std::time::Duration::from_secs(1), ..crate::kit::common::limits::recorded("pdf_to_text").unwrap() };
        let job_released = Arc::clone(&released);
        let job_slow = slow.clone();
        let outcomes = BatchRunner::run_with_limits(&RunEvents::silent(), "pdf_to_text", request.paths.clone(), &request.output_location, Some(limits), &request, move |request, path| {
            while path == job_slow && !job_released.load(Ordering::SeqCst) { std::thread::sleep(std::time::Duration::from_millis(10)); }
            remaining::to_text(request, path)
        });
        released.store(true, Ordering::SeqCst);
        assert_eq!([std::fs::read(&slow).unwrap(), std::fs::read(&good).unwrap()], originals);
        assert_only_offender_limited(&outcomes, &slow, LimitKind::Time, "1 second");
        let _ = std::fs::remove_dir_all(root);
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
        let protected_result = protect_files(RunEvents::silent(), PasswordRequest { paths: vec![pdf.clone()], password: crate::kit::TEST_PASSWORD.into(), password_overrides: Default::default(), output_location: OutputLocation::AlongsideInput });
        let protected_output = assert_success("protect fixture", protected_result)[0].clone();
        std::fs::rename(protected_output, &protected).unwrap();

        let image_pdf = root.join("image-pdf.pdf");
        let image_pdf_request = ImagesToPdfRequest { paths: vec![image.clone()], output_location: OutputLocation::AlongsideInput, output_path: None };
        let image_pdf_result = run_images_to_pdf(&image_pdf_request);
        let image_pdf_output = assert_success("images-to-pdf fixture", image_pdf_result)[0].clone();
        std::fs::rename(image_pdf_output, &image_pdf).unwrap();

        let mut outputs = Vec::new();
        let mut naming = Vec::new();
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
        let rendered_scene_pages = tauri::async_runtime::block_on(preview_pdf_scene_pages(scene::PreviewPagesRequest { path: pdf.clone(), scene: visual_scene.clone(), page_indices: vec![1, 0] })).unwrap();
        assert_eq!(rendered_scene_pages.iter().map(|preview| preview.page_index).collect::<Vec<_>>(), vec![1, 0]);
        assert!(rendered_scene_pages.iter().all(|preview| preview.preview.data_url.starts_with("data:image/png;base64,")));
        let wire = serde_json::to_value(&rendered_scene_pages).unwrap();
        assert_eq!(wire[0]["pageIndex"], 1);
        let preview = &wire[0]["preview"];
        let preview_width = preview["width"].as_u64().expect("preview width must be numeric");
        let preview_height = preview["height"].as_u64().expect("preview height must be numeric");
        assert!(preview_width > 0 && preview_height > 0, "preview dimensions must be positive");
        assert!(preview_width > preview_height, "a 90-degree page should preview in landscape orientation");
        assert!(preview["dataUrl"].as_str().is_some_and(|value| value.starts_with("data:image/png;base64,")));
        outputs.extend(named(&mut naming, "export_pdf_scene", pdf.clone(), assert_success("PDF scene", export_pdf_scene(RunEvents::silent(), scene::ExportRequest { paths: vec![pdf.clone()], scene: visual_scene, output_location: OutputLocation::AlongsideInput }))));
        outputs.extend(named(&mut naming, "remove_password", protected.clone(), assert_success("unlock", remove_password(RunEvents::silent(), PasswordRequest { paths: vec![protected], password: crate::kit::TEST_PASSWORD.into(), password_overrides: Default::default(), output_location: location(&root, "unlock") }))));
        outputs.extend(named(&mut naming, "add_page_numbers", pdf.clone(), assert_success("page numbers", add_page_numbers(RunEvents::silent(), PageOverlayRequest { paths: vec![pdf.clone()], text: "1".into(), opacity: 100, position: Some("bottom-right".into()), logo_path: None, pages: None, start_number: Some(1), font_size: Some(12), output_location: location(&root, "page numbers") }))));
        outputs.extend(named(&mut naming, "merge_pdfs", pdf.clone(), assert_aggregate_success("merge", &[pdf.clone(), pdf_two.clone()], run_merge_pdfs(&MergePdfRequest { paths: vec![pdf.clone(), pdf_two.clone()], output_location: location(&root, "merge"), output_path: None }))));
        outputs.extend(named(&mut naming, "watermark_pdf", pdf.clone(), assert_success("watermark pdf", watermark_pdf(RunEvents::silent(), PageOverlayRequest { paths: vec![pdf.clone()], text: "TEST".into(), opacity: 70, position: Some("center".into()), logo_path: None, pages: None, start_number: None, font_size: None, output_location: location(&root, "watermark pdf") }))));
        outputs.extend(named(&mut naming, "crop_pdf", pdf.clone(), assert_success("crop pdf", crop_pdf(RunEvents::silent(), CropPdfRequest { paths: vec![pdf.clone()], rectangle: PdfRect { x: 0.5, y: 0.5, width: 500.0, height: 700.0 }, scope: PageScope::All, output_location: location(&root, "crop pdf") }))));
        outputs.extend(named(&mut naming, "edit_pdf", pdf.clone(), assert_success("edit pdf", edit_pdf(RunEvents::silent(), EditPdfRequest { paths: vec![pdf.clone()], mode: "highlight".into(), text: "TEST NOTE".into(), pages: None, rectangle: PdfRect { x: 40.0, y: 650.0, width: 220.0, height: 60.0 }, output_location: location(&root, "edit pdf") }))));
        outputs.extend(named(&mut naming, "edit_pdf_session", pdf.clone(), assert_success("composed pdf editor", edit_pdf_session(RunEvents::silent(), PdfEditSessionRequest {
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
        }))));
        outputs.extend(named(&mut naming, "protect_files", pdf_two.clone(), assert_success("lock", protect_files(RunEvents::silent(), PasswordRequest { paths: vec![pdf_two.clone()], password: crate::kit::TEST_OVERRIDE_PASSWORD.into(), password_overrides: Default::default(), output_location: location(&root, "protect") }))));
        outputs.extend(named(&mut naming, "images_to_pdf", image.clone(), assert_aggregate_success("images to pdf", &[image.clone(), image_two.clone()], run_images_to_pdf(&ImagesToPdfRequest { paths: vec![image.clone(), image_two.clone()], output_location: location(&root, "images to pdf"), output_path: None }))));
        outputs.extend(named(&mut naming, "pdf_to_images", pdf.clone(), assert_success("pdf to images", pdf_to_images(RunEvents::silent(), PdfToImagesRequest { paths: vec![pdf.clone()], dpi: 72, format: "png".into(), page_range: None, pages: None, output_location: location(&root, "pdf to images") }))));
        let selected_image_outputs = assert_success("pdf selected pages to images", pdf_to_images(RunEvents::silent(), PdfToImagesRequest { paths: vec![pdf.clone()], dpi: 72, format: "png".into(), page_range: None, pages: Some(vec![1]), output_location: location(&root, "pdf selected pages to images") }));
        assert_eq!(selected_image_outputs.len(), 1, "selected PDF page rendering must produce only the selected page");
        outputs.extend(named(&mut naming, "pdf_to_images", pdf.clone(), selected_image_outputs));
        outputs.extend(named(&mut naming, "pdf_to_text", pdf.clone(), assert_success("pdf to text", pdf_to_text(RunEvents::silent(), PdfToTextRequest { paths: vec![pdf.clone()], pages: None, output_location: location(&root, "pdf to text") }))));
        outputs.extend(named(&mut naming, "split_pdf", pdf.clone(), assert_success("split", split_pdf(RunEvents::silent(), PageSelectionRequest { paths: vec![pdf.clone()], pages: vec![], page_ranges: None, split_mode: Some("pages".into()), chunk_size: None, output_location: location(&root, "split") }))));
        outputs.extend(named(&mut naming, "extract_pdf_images", image_pdf.clone(), assert_success("extract pdf images", extract_pdf_images(RunEvents::silent(), PdfToTextRequest { paths: vec![image_pdf.clone()], pages: None, output_location: location(&root, "extract pdf images") }))));
        outputs.extend(named(&mut naming, "sign_pdf", pdf.clone(), assert_success("sign", sign_pdf(RunEvents::silent(), SignPdfRequest { paths: vec![pdf.clone()], page: 0, text: "Signed".into(), signature_path: None, rectangle: PdfRect { x: 40.0, y: 40.0, width: 180.0, height: 60.0 }, scope: PageScope::All, output_location: location(&root, "sign") }))));
        outputs.extend(named(&mut naming, "ocr_pdf", pdf.clone(), assert_ocr_adapter("ocr", ocr_pdf(RunEvents::silent(), VisionRequest { paths: vec![pdf.clone()], pages: None, output_location: location(&root, "ocr") }))));
        outputs.extend(named(&mut naming, "remove_pdf_pages", pdf.clone(), assert_success("remove pages", remove_pdf_pages(RunEvents::silent(), PageSelectionRequest { paths: vec![pdf.clone()], pages: vec![0], page_ranges: None, split_mode: None, chunk_size: None, output_location: location(&root, "remove pages") }))));
        outputs.extend(named(&mut naming, "extract_pdf_pages", pdf.clone(), assert_success("extract pages", extract_pdf_pages(RunEvents::silent(), PageSelectionRequest { paths: vec![pdf.clone()], pages: vec![], page_ranges: Some("1".into()), split_mode: None, chunk_size: None, output_location: location(&root, "extract pages") }))));
        outputs.extend(named(&mut naming, "organize_pdf", pdf.clone(), assert_success("organize", organize_pdf(RunEvents::silent(), OrganizePdfRequest { paths: vec![pdf.clone()], page_order: vec![1, 0], delete_pages: vec![], rotate_pages: vec![RotatePage { page: 0, degrees: 90 }], scope: PageScope::All, output_location: location(&root, "organize") }))));
        let added = assert_success(
            "add pages",
            add_pdf_pages(RunEvents::silent(), AddPdfPagesRequest {
                paths: vec![pdf.clone()],
                position: "after".into(),
                page: 0,
                count: 2,
                output_location: location(&root, "add pages"),
            }),
        );
        let added_document = lopdf::Document::load(&added[0]).unwrap();
        assert_eq!(added_document.get_pages().len(), 4, "add pages must insert the requested number of pages");
        outputs.extend(named(&mut naming, "add_pdf_pages", pdf.clone(), added));
        outputs.extend(named(&mut naming, "compress_pdf", pdf.clone(), assert_success("compress pdf", compress_pdf(RunEvents::silent(), serde_json::from_value::<CompressPdfRequest>(serde_json::json!({ "paths": [pdf.clone()], "quality": 80, "outputLocation": location(&root, "compress pdf") })).expect("Compress Files view payload for compress_pdf")))));
        outputs.extend(named(&mut naming, "convert_images", image.clone(), assert_success("convert", convert_images(RunEvents::silent(), ConvertImagesRequest { paths: vec![image.clone()], format: "jpg".into(), output_location: location(&root, "convert") }))));
        outputs.extend(named(&mut naming, "compress_images", image.clone(), assert_success("compress images", compress_images(RunEvents::silent(), serde_json::from_value::<CompressImagesRequest>(serde_json::json!({ "paths": [image.clone()], "quality": 80, "lossless": false, "outputLocation": location(&root, "compress images") })).expect("Compress Files view payload for compress_images")))));
        outputs.extend(named(&mut naming, "resize_images", image.clone(), assert_success("resize", resize_images(RunEvents::silent(), ResizeRequest { paths: vec![image.clone()], width: 128, height: 128, mode: "exact".into(), resampling: "lanczos".into(), keep_aspect_ratio: true, percentage: 100, longest_side: 128, output_location: location(&root, "resize") }))));
        outputs.extend(named(&mut naming, "rotate_images", image.clone(), assert_success("rotate", rotate_images(RunEvents::silent(), RotateRequest { paths: vec![image.clone()], degrees: 90, flip: "none".into(), output_location: location(&root, "rotate") }))));
        outputs.extend(named(&mut naming, "crop_images", image.clone(), assert_success("crop image", crop_images(RunEvents::silent(), CropRequest { paths: vec![image.clone()], x: 0, y: 0, width: 128, height: 128, mode: "rectangle".into(), aspect_width: 128, aspect_height: 128, anchor: "center".into(), output_location: location(&root, "crop image") }))));
        outputs.extend(named(&mut naming, "generate_icon_set", image.clone(), assert_success("icons", generate_icon_set(RunEvents::silent(), IconSetRequest { paths: vec![image.clone()], preset: "favicon".into(), sizes: vec![], output_location: location(&root, "icons") }))));
        outputs.extend(named(&mut naming, "create_gif", image.clone(), assert_aggregate_success("create gif", &[image.clone(), image_two.clone()], run_create_gif(&GifCreateRequest { paths: vec![image.clone(), image_two.clone()], frame_delay_ms: 100, loop_forever: true, output_location: location(&root, "create gif"), output_path: None }))));
        outputs.extend(named(&mut naming, "extract_gif_frames", animated.clone(), assert_success("extract gif", extract_gif_frames(RunEvents::silent(), GifExtractRequest { paths: vec![animated.clone()], output_location: location(&root, "extract gif") }))));
        outputs.extend(named(&mut naming, "watermark_images", image.clone(), assert_success("watermark image", watermark_images(RunEvents::silent(), WatermarkRequest { paths: vec![image.clone()], opacity: 70, text: Some("TEST".into()), logo_path: None, x: 16, y: 16, output_location: location(&root, "watermark image") }))));
        let image_plan = ImageEditPlan {
            edits: vec![
                ImageEdit::Resize { width: 128, height: 128, mode: "exact".into(), percentage: 100, longest_side: 128, resampling: "lanczos".into(), keep_aspect_ratio: false },
                ImageEdit::Rotate { degrees: 90, flip: "horizontal".into() },
            ],
            output_location: location(&root, "composed image editor"),
        };
        let image_preview = inspect_image_edit_preview(ImageEditPreviewRequest { path: image.clone(), plan: image_plan.clone() }).expect("composed image preview should be available");
        assert_eq!((image_preview.width, image_preview.height), (128, 128), "composed image preview must reflect the complete edit plan");
        outputs.extend(named(&mut naming, "export_image_edit_plan", image.clone(), assert_success("composed image editor", export_image_edit_plan(RunEvents::silent(), ImageEditExportRequest { paths: vec![image.clone()], plan: image_plan }))));
        outputs.extend(named(&mut naming, "image_metadata", image.clone(), assert_success("metadata", image_metadata(RunEvents::silent(), MetadataRequest { paths: vec![image.clone()], output_location: location(&root, "metadata") }))));
        outputs.extend(named(&mut naming, "adjust_image_tone", image.clone(), assert_success("tone", adjust_image_tone(RunEvents::silent(), ToneRequest { paths: vec![image.clone()], brightness: 20, contrast: 0.0, saturation: 0.0, exposure: 0.0, output_location: location(&root, "tone") }))));
        outputs.extend(named(&mut naming, "process_tiff_pages", tiff.clone(), assert_aggregate_success("tiff", &[tiff.clone()], run_process_tiff_pages(&TiffRequest { paths: vec![tiff.clone()], pages: None, output_location: location(&root, "tiff"), output_path: None }))));
        outputs.extend(named(&mut naming, "blur_faces", image.clone(), assert_vision_adapter("face blur", blur_faces(RunEvents::silent(), VisionRequest { paths: vec![image.clone()], pages: None, output_location: location(&root, "face blur") }))));
        outputs.extend(named(&mut naming, "remove_image_background", image.clone(), assert_vision_adapter("background removal", remove_image_background(RunEvents::silent(), VisionRequest { paths: vec![image.clone()], pages: None, output_location: location(&root, "background removal") }))));
        let docx = root.join("document.docx");
        std::fs::write(&docx, test_minimal_ooxml_package("docx")).unwrap();
        let docx_bytes = std::fs::read(&docx).unwrap();
        let locked_docx = assert_success("lock word", protect_files(RunEvents::silent(), PasswordRequest { paths: vec![docx.clone()], password: crate::kit::TEST_PASSWORD.into(), password_overrides: Default::default(), output_location: location(&root, "lock word") }));
        assert_eq!(locked_docx[0].file_name().and_then(|name| name.to_str()), Some("document-protected.docx"));
        assert_eq!(std::fs::read(&docx).unwrap(), docx_bytes, "native E2E commands must not modify their Word input");
        assert_outputs_use_declared_suffixes(&naming);

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

        let heic = tauri::async_runtime::block_on(convert_images(RunEvents::silent(), ConvertImagesRequest {
            paths: vec![src.clone()], format: "heic".to_string(), output_location: OutputLocation::AlongsideInput,
        }));
        assert!(heic[0].error().is_none(), "{}", heic[0].error().cloned().unwrap_or_default());
        assert_eq!(
            heic[0].output_paths()[0].extension().and_then(|e| e.to_str()),
            Some("heic")
        );
        let bytes = std::fs::read(&heic[0].output_paths()[0]).unwrap();
        assert!(heif::decode(&bytes).is_ok(), "command output must be real HEIC");

        let webp = tauri::async_runtime::block_on(convert_images(RunEvents::silent(), ConvertImagesRequest {
            paths: vec![src.clone()], format: "webp".to_string(), output_location: OutputLocation::AlongsideInput,
        }));
        assert!(webp[0].error().is_none(), "{}", webp[0].error().cloned().unwrap_or_default());
        let bytes = std::fs::read(&webp[0].output_paths()[0]).unwrap();
        assert_eq!(&bytes[..4], b"RIFF", "command output must be real WebP");

        cleanup(&heic[0].output_paths());
        cleanup(&webp[0].output_paths());
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn single_input_convert_rejects_multiple_files() {
        let a = temp_path("cmd_a.png");
        let b = temp_path("cmd_b.png");
        write_png(&a, 128, 1);
        write_png(&b, 128, 2);

        let out = tauri::async_runtime::block_on(convert_images(RunEvents::silent(), ConvertImagesRequest {
            paths: vec![a.clone(), b.clone()], format: "jpg".to_string(), output_location: OutputLocation::AlongsideInput,
        }));
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|job| job.error().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput))));
        assert!(out.iter().all(|job| job.output_paths().is_empty()));

        cleanup(&out[0].output_paths());
        cleanup(&out[1].output_paths());
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn lock_skips_already_protected_files_and_fails_each_legacy_file_with_its_save_as_message_while_modern_files_lock() {
        use crate::kit::contracts::{OutcomeState, SkipCode};
        use crate::kit::office::tests::ppt_plain_fixture;
        use crate::kit::password::tests::{pdf, word_document, workbook};
        enum Expect { Locked, AlreadyProtected, Legacy(&'static str) }
        let already = "Already password-protected. Unlock it first, then lock it again.";
        let fixtures = sandbox("lock-skip-fixtures");
        let root = sandbox("lock-skip-and-legacy");
        let cases: Vec<(&str, Vec<u8>, Expect)> = vec![
            ("signed.pdf", pdf(Some(("owner", "user"))), Expect::AlreadyProtected),
            ("invoice.pdf", std::fs::read(locked_pdf(&fixtures, "invoice", crate::kit::TEST_OVERRIDE_PASSWORD)).unwrap(), Expect::AlreadyProtected),
            ("offer.docx", test_minimal_ooxml_package("docx"), Expect::Locked),
            ("payroll.docx", std::fs::read(locked_docx(&fixtures, "payroll", crate::kit::TEST_OVERRIDE_PASSWORD)).unwrap(), Expect::AlreadyProtected),
            ("minutes.doc", word_document(0), Expect::Legacy("Legacy .doc files can't be locked. Save it as .docx in Word, then add it again.")),
            ("budget.xlsx", test_minimal_ooxml_package("xlsx"), Expect::Locked),
            ("ledger.xls", workbook(false), Expect::Legacy("Legacy .xls files can't be locked. Save it as .xlsx in Excel, then add it again.")),
            ("old-deck.ppt", ppt_plain_fixture(), Expect::Legacy("Legacy .ppt files can't be locked. Save it as .pptx in PowerPoint, then add it again.")),
            ("deck.pptx", test_minimal_ooxml_package("pptx"), Expect::Locked),
        ];
        let inputs = cases.iter().map(|(name, bytes, _)| {
            let input = root.join(name);
            std::fs::write(&input, bytes).unwrap();
            input
        }).collect::<Vec<_>>();

        let outcomes = lock(&inputs, location(&root, "output"));

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs, "BatchRunner must preserve the requested input order");
        for ((name, original, expect), outcome) in cases.iter().zip(&outcomes) {
            match (expect, &outcome.state) {
                (Expect::Locked, OutcomeState::Succeeded { output_paths }) => {
                    let stem = name.rsplit_once('.').unwrap();
                    assert_eq!(output_paths, &[root.join("output").join(format!("{}-protected.{}", stem.0, stem.1))], "{name}");
                }
                (Expect::AlreadyProtected, OutcomeState::Skipped { reason }) => {
                    assert_eq!((&reason.code, reason.message.as_str()), (&SkipCode::AlreadyProtected, already), "{name}");
                }
                (Expect::Legacy(message), OutcomeState::Failed { output_paths, error }) => {
                    assert!(output_paths.is_empty(), "{name}");
                    assert_eq!((&error.kind, error.message.as_str()), (&ErrorKind::InvalidInput, *message), "{name}");
                }
                (_, state) => panic!("{name} ended as {state:?}"),
            }
            assert_eq!(&std::fs::read(root.join(name)).unwrap(), original, "original was modified: {name}");
        }
        let mut written = std::fs::read_dir(root.join("output")).unwrap().map(|entry| entry.unwrap().file_name()).collect::<Vec<_>>();
        written.sort();
        assert_eq!(written, ["budget-protected.xlsx", "deck-protected.pptx", "offer-protected.docx"], "skipped and refused files write and reserve nothing");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), cases.len() + 1, "nothing is written beside the inputs");

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(fixtures);
    }

    #[test]
    fn batch_events_start_and_finish_every_file_once_and_match_the_ordered_response() {
        let root = sandbox("batch-events-mixed");
        let first = root.join("first.pdf");
        let broken = root.join("broken.pdf");
        let office = root.join("notes.docx");
        let last = root.join("last.pdf");
        make_pdf(&first, 2);
        std::fs::write(&broken, b"not a pdf").unwrap();
        std::fs::write(&office, b"Office fixture").unwrap();
        make_pdf(&last, 1);
        let inputs = vec![first.clone(), broken.clone(), office.clone(), last.clone()];
        let originals = inputs.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let outcomes = tauri::async_runtime::block_on(protect_files(RunEvents::new(move |event| sink.lock().unwrap().push(event)), PasswordRequest {
            paths: inputs.clone(),
            password: "secret".into(),
            password_overrides: Default::default(),
            output_location: location(&root, "protected"),
        }));
        let events = seen.lock().unwrap().clone();

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs);
        assert!(outcomes[0].error().is_none() && outcomes[3].error().is_none());
        assert!(outcomes[1].error().is_some());
        assert!(outcomes[2].error().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput)));
        assert_eq!(events.len(), inputs.len() * 2);
        for index in 0..inputs.len() {
            let started = events.iter().position(|event| *event == FileEvent::Started { index }).expect("every file starts once");
            let finished = events.iter().enumerate().filter_map(|(position, event)| match event {
                FileEvent::Finished { index: finished, outcome } if *finished == index => Some((position, outcome)),
                _ => None,
            }).collect::<Vec<_>>();
            assert_eq!(finished.len(), 1, "file {index} finishes exactly once");
            assert!(started < finished[0].0, "file {index} starts before it finishes");
            assert_eq!(finished[0].1, &outcomes[index], "the streamed outcome is the final outcome");
        }
        for (path, original) in inputs.iter().zip(originals) { assert_eq!(std::fs::read(path).unwrap(), original); }
        assert_eq!(std::fs::read_dir(root.join("protected")).unwrap().count(), 2);
    }

    #[test]
    fn refused_password_inputs_never_reserve_an_output_in_any_password_command() {
        let root = sandbox("password-refusals");
        let pdf = root.join("source.pdf");
        let locked = root.join("locked.pdf");
        let docx = root.join("source.docx");
        let text = root.join("notes.txt");
        let damaged_pdf = root.join("damaged.pdf");
        let damaged_docx = root.join("damaged.docx");
        let damaged_xls = root.join("damaged.xls");
        make_pdf(&pdf, 1);
        std::fs::write(&locked, crate::kit::password::tests::pdf(Some(("owner", "user")))).unwrap();
        std::fs::write(&docx, test_minimal_ooxml_package("docx")).unwrap();
        std::fs::write(&text, b"plain text").unwrap();
        std::fs::write(&damaged_pdf, b"%PDF-1.7 damaged").unwrap();
        std::fs::write(&damaged_docx, b"not an office package").unwrap();
        std::fs::write(&damaged_xls, b"not a workbook").unwrap();
        let unreservable = root.join("missing-output-folder");
        let request = |paths: Vec<PathBuf>, password: &str| PasswordRequest {
            paths,
            password: password.into(),
            password_overrides: Default::default(),
            output_location: OutputLocation::CustomFolder(unreservable.clone()),
        };

        let cases = [
            ("remove_password wrong type", ErrorKind::InvalidInput, tauri::async_runtime::block_on(remove_password(RunEvents::silent(), request(vec![text.clone()], "secret")))),
            ("remove_password empty pdf password", ErrorKind::InvalidInput, tauri::async_runtime::block_on(remove_password(RunEvents::silent(), request(vec![locked.clone()], "")))),
            ("remove_password empty office password", ErrorKind::InvalidInput, tauri::async_runtime::block_on(remove_password(RunEvents::silent(), request(vec![docx.clone()], "")))),
            ("protect_files wrong type", ErrorKind::InvalidInput, tauri::async_runtime::block_on(protect_files(RunEvents::silent(), request(vec![text.clone()], "secret")))),
            ("protect_files empty password", ErrorKind::InvalidInput, tauri::async_runtime::block_on(protect_files(RunEvents::silent(), request(vec![pdf.clone()], "")))),
            ("protect_files password with a line break", ErrorKind::InvalidInput, tauri::async_runtime::block_on(protect_files(RunEvents::silent(), request(vec![pdf.clone()], "two\nlines")))),
            ("protect_files empty office password", ErrorKind::InvalidInput, tauri::async_runtime::block_on(protect_files(RunEvents::silent(), request(vec![docx.clone()], "")))),
            ("remove_password damaged pdf", ErrorKind::Processing, tauri::async_runtime::block_on(remove_password(RunEvents::silent(), request(vec![damaged_pdf.clone()], "secret")))),
            ("remove_password damaged docx", ErrorKind::Processing, tauri::async_runtime::block_on(remove_password(RunEvents::silent(), request(vec![damaged_docx.clone()], "secret")))),
            ("remove_password damaged xls", ErrorKind::Processing, tauri::async_runtime::block_on(remove_password(RunEvents::silent(), request(vec![damaged_xls.clone()], "secret")))),
            ("protect_files damaged pdf", ErrorKind::InvalidInput, tauri::async_runtime::block_on(protect_files(RunEvents::silent(), request(vec![damaged_pdf.clone()], "secret")))),
            ("protect_files damaged docx", ErrorKind::InvalidInput, tauri::async_runtime::block_on(protect_files(RunEvents::silent(), request(vec![damaged_docx.clone()], "secret")))),
        ];

        for (name, kind, outcomes) in cases {
            let error = outcomes[0].error().unwrap_or_else(|| panic!("{name} must be refused"));
            assert_eq!(error.kind, kind, "{name} is refused before any output is reserved: {}", error.message);
            assert!(outcomes[0].output_paths().is_empty(), "{name} has no output");
        }
        assert!(!unreservable.exists(), "a refused input never touches the output location");
        let mut left = std::fs::read_dir(&root).unwrap().map(|entry| entry.unwrap().file_name()).collect::<Vec<_>>();
        left.sort();
        assert_eq!(left, ["damaged.docx", "damaged.pdf", "damaged.xls", "locked.pdf", "notes.txt", "source.docx", "source.pdf"], "nothing is written beside the inputs");
        let _ = std::fs::remove_dir_all(root);
    }

    fn mixed_lock_batch(root: &PathBuf) -> Vec<PathBuf> {
        let inputs = vec![root.join("report.pdf"), root.join("lease.docx"), root.join("budget.xlsx"), root.join("deck.pptx")];
        make_pdf(&inputs[0], 1);
        std::fs::write(&inputs[1], test_minimal_ooxml_package("docx")).unwrap();
        std::fs::write(&inputs[2], test_minimal_ooxml_package("xlsx")).unwrap();
        std::fs::write(&inputs[3], test_minimal_ooxml_package("pptx")).unwrap();
        inputs
    }

    fn lock(inputs: &[PathBuf], output_location: OutputLocation) -> Vec<JobOutcome> {
        tauri::async_runtime::block_on(protect_files(RunEvents::silent(), PasswordRequest {
            paths: inputs.to_vec(),
            password: crate::kit::TEST_PASSWORD.into(),
            password_overrides: Default::default(),
            output_location,
        }))
    }

    #[test]
    fn protect_files_locks_a_mixed_pdf_word_excel_and_powerpoint_batch_that_each_reopen_with_the_password() {
        let root = sandbox("protect-files-mixed-office");
        let inputs = mixed_lock_batch(&root);
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();

        let outcomes = lock(&inputs, location(&root, "protected"));

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs);
        let locked = outcomes.iter().map(|outcome| {
            assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
            outcome.output_paths()[0].clone()
        }).collect::<Vec<_>>();
        assert_eq!(locked, ["report-protected.pdf", "lease-protected.docx", "budget-protected.xlsx", "deck-protected.pptx"].map(|name| root.join("protected").join(name)));
        for output in &locked {
            let format = crate::kit::password::ProtectedFormat::from_path(output).unwrap();
            assert_eq!(crate::kit::password::probe(&std::fs::read(output).unwrap(), format).unwrap(), crate::kit::password::ProtectionState::OpenPassword, "{} needs a password", output.display());
        }
        for (input, original) in inputs.iter().zip(&originals) {
            assert_eq!(&std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }

        let unlock = |password: &str, folder: &str| tauri::async_runtime::block_on(remove_password(RunEvents::silent(), PasswordRequest {
            paths: locked.clone(),
            password: password.into(),
            password_overrides: Default::default(),
            output_location: location(&root, folder),
        }));
        for wrong in unlock(crate::kit::TEST_OVERRIDE_PASSWORD, "wrong") {
            assert_eq!(wrong.error().map(|error| error.kind.clone()), Some(ErrorKind::WrongPassword), "{}", wrong.input_path.display());
        }
        for (unlocked, original) in unlock(crate::kit::TEST_PASSWORD, "unlocked").iter().zip(&originals) {
            assert!(unlocked.error().is_none(), "{}", unlocked.error().cloned().unwrap_or_default());
            let output = &unlocked.output_paths()[0];
            assert!(opens_without_password(output), "{} still needs a password", output.display());
            if output.extension().is_some_and(|extension| extension != "pdf") {
                assert_eq!(&std::fs::read(output).unwrap(), original, "{} unlocks to the original package", output.display());
            }
        }
        let reopened = lopdf::Document::load(root.join("unlocked").join("report-protected-unlocked.pdf")).unwrap();
        assert_eq!(reopened.get_pages().len(), 1);
        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn locking_and_unlocking_a_signed_pdf_warns_that_the_signature_was_removed_on_that_file_only() {
        use crate::kit::contracts::{OutcomeWarning, WarningKind};
        let root = sandbox("signature-removed");
        let signed = root.join("vendor-nda.pdf");
        let plain = root.join("lease.pdf");
        let deck = root.join("deck.pptx");
        make_pdf(&signed, 1);
        let mut document = lopdf::Document::load(&signed).unwrap();
        crate::kit::pdf::tests::sign(&mut document);
        document.save(&signed).unwrap();
        make_pdf(&plain, 1);
        std::fs::write(&deck, test_minimal_ooxml_package("pptx")).unwrap();
        let inputs = vec![signed.clone(), plain.clone(), deck.clone()];
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();
        let removed = vec![OutcomeWarning { kind: WarningKind::SignatureRemoved, message: crate::kit::pdf::SIGNATURE_REMOVED.into() }];
        let assert_warned_only_on_signed = |outcomes: &[JobOutcome]| {
            assert_eq!(outcomes.len(), 3);
            for outcome in outcomes {
                assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
                assert_eq!(outcome.output_paths().len(), 1);
            }
            assert_eq!(outcomes.iter().map(|outcome| outcome.warnings.clone()).collect::<Vec<_>>(), [removed.clone(), vec![], vec![]]);
        };

        let locked = lock(&inputs, location(&root, "locked"));
        assert_warned_only_on_signed(&locked);
        let unlocked = tauri::async_runtime::block_on(remove_password(RunEvents::silent(), PasswordRequest {
            paths: locked.iter().map(|outcome| outcome.output_paths()[0].clone()).collect(),
            password: crate::kit::TEST_PASSWORD.into(),
            password_overrides: Default::default(),
            output_location: location(&root, "unlocked"),
        }));
        assert_warned_only_on_signed(&unlocked);
        assert!(opens_without_password(&unlocked[0].output_paths()[0]));
        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    const MISSING_QPDF_CHILD: &str = "TOOLBOX_TEST_MISSING_QPDF_CHILD";

    // Other tests resolve qpdf without the env lock, so the missing override is set only in a child test process.
    #[test]
    fn without_qpdf_a_mixed_lock_fails_only_the_pdfs_as_unavailable() {
        if std::env::var_os(MISSING_QPDF_CHILD).is_none() {
            let _guard = crate::kit::PROCESS_ENV_LOCK.lock().unwrap();
            let missing = sandbox("missing-qpdf-helper").join("qpdf-not-installed");
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["command_tests::without_qpdf_a_mixed_lock_fails_only_the_pdfs_as_unavailable", "--exact", "--nocapture", "--test-threads=1"])
                .env(MISSING_QPDF_CHILD, "1")
                .env("TOOLBOX_QPDF_PATH", &missing)
                .output()
                .unwrap();
            let report = format!("{}{}", String::from_utf8_lossy(&child.stdout), String::from_utf8_lossy(&child.stderr));
            assert!(child.status.success() && report.contains("1 passed"), "{report}");
            return;
        }
        assert!(crate::kit::resources::resolve_helper(&crate::kit::resources::QPDF).is_err(), "the Development override must hide qpdf");
        let root = sandbox("protect-files-missing-qpdf");
        let inputs = mixed_lock_batch(&root);
        let second_pdf = root.join("appendix.pdf");
        make_pdf(&second_pdf, 1);
        let inputs = [inputs, vec![second_pdf]].concat();
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();

        let outcomes = lock(&inputs, location(&root, "protected"));

        for outcome in &outcomes {
            if outcome.input_path.extension().is_some_and(|extension| extension == "pdf") {
                assert_eq!(outcome.error().map(|error| error.kind.clone()), Some(ErrorKind::Unavailable), "{}", outcome.input_path.display());
                assert!(outcome.output_paths().is_empty());
            } else {
                assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
                assert_eq!(outcome.output_paths().len(), 1);
            }
        }
        let mut written = std::fs::read_dir(root.join("protected")).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect::<Vec<_>>();
        written.sort();
        assert_eq!(written, ["budget-protected.xlsx", "deck-protected.pptx", "lease-protected.docx"]);
        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn locking_beside_existing_protected_copies_numbers_the_new_ones() {
        let root = sandbox("protect-files-collisions");
        let pdf = root.join("report.pdf");
        make_pdf(&pdf, 1);
        let output = location(&root, "locked");
        let OutputLocation::CustomFolder(folder) = &output else { unreachable!() };
        std::fs::write(folder.join("report-protected.pdf"), b"an earlier copy").unwrap();

        let lock = || tauri::async_runtime::block_on(protect_files(RunEvents::silent(), PasswordRequest {
            paths: vec![pdf.clone()],
            password: crate::kit::TEST_PASSWORD.into(),
            password_overrides: Default::default(),
            output_location: output.clone(),
        })).remove(0);
        let first = lock();
        let second = lock();

        assert_eq!(first.output_paths(), [folder.join("report-protected-1.pdf")]);
        assert_eq!(second.output_paths(), [folder.join("report-protected-2.pdf")]);
        assert_eq!(std::fs::read(folder.join("report-protected.pdf")).unwrap(), b"an earlier copy");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn locking_a_mixed_batch_into_a_per_run_folder_numbers_collisions_and_leaves_nothing_beside_the_inputs() {
        let root = sandbox("protect-files-per-run-folder");
        let inputs_dir = root.join("inputs");
        std::fs::create_dir_all(&inputs_dir).unwrap();
        let inputs = mixed_lock_batch(&inputs_dir);
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();
        let beside_inputs = file_names(&inputs_dir);
        let output = location(&root, "this-run");
        let OutputLocation::CustomFolder(folder) = &output else { unreachable!() };
        std::fs::write(folder.join("report-protected.pdf"), b"an earlier copy").unwrap();

        let outputs = |outcomes: Vec<JobOutcome>| outcomes.iter().map(|outcome| {
            assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
            outcome.output_paths()[0].clone()
        }).collect::<Vec<_>>();
        let first = outputs(lock(&inputs, output.clone()));
        let second = outputs(lock(&inputs, output.clone()));

        assert_eq!(first, ["report-protected-1.pdf", "lease-protected.docx", "budget-protected.xlsx", "deck-protected.pptx"].map(|name| folder.join(name)));
        assert_eq!(second, ["report-protected-2.pdf", "lease-protected-1.docx", "budget-protected-1.xlsx", "deck-protected-1.pptx"].map(|name| folder.join(name)));
        assert_eq!(std::fs::read(folder.join("report-protected.pdf")).unwrap(), b"an earlier copy");
        assert_eq!(file_names(&inputs_dir), beside_inputs, "nothing is written beside the inputs");
        for (input, original) in inputs.iter().zip(&originals) {
            assert_eq!(&std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    fn locked_docx(root: &PathBuf, name: &str, password: &str) -> PathBuf {
        locked_office(root, name, "docx", password)
    }

    fn locked_office(root: &PathBuf, name: &str, extension: &str, password: &str) -> PathBuf {
        let source = root.join(format!("{name}.{extension}"));
        std::fs::write(&source, test_minimal_ooxml_package(extension)).unwrap();
        let locked = OfficeProcessor::protect(source, password, &location(root, "locked"));
        assert!(locked.error().is_none(), "{}", locked.error().cloned().unwrap_or_default());
        locked.output_paths()[0].clone()
    }

    fn overrides(entries: &[(&PathBuf, &str)]) -> std::collections::HashMap<PathBuf, String> {
        entries.iter().map(|(path, password)| ((*path).clone(), password.to_string())).collect()
    }

    #[test]
    fn unlock_uses_each_override_first_and_the_batch_password_for_every_other_file() {
        use crate::kit::{TEST_OVERRIDE_PASSWORD, TEST_PASSWORD};
        let root = sandbox("unlock-overrides");
        let first = locked_docx(&root, "first", TEST_PASSWORD);
        let own = locked_docx(&root, "own", TEST_OVERRIDE_PASSWORD);
        let own_pdf = locked_pdf(&root, "own-pdf", TEST_OVERRIDE_PASSWORD);
        let last = locked_pdf(&root, "last", TEST_PASSWORD);
        let inputs = vec![first.clone(), own.clone(), own_pdf.clone(), last.clone()];
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();

        let outcomes = tauri::async_runtime::block_on(remove_password(RunEvents::silent(), PasswordRequest {
            paths: inputs.clone(),
            password: TEST_PASSWORD.into(),
            password_overrides: overrides(&[(&own, TEST_OVERRIDE_PASSWORD), (&own_pdf, TEST_OVERRIDE_PASSWORD)]),
            output_location: location(&root, "with-override"),
        }));
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs);
        for outcome in &outcomes {
            assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
            assert!(opens_without_password(&outcome.output_paths()[0]));
        }

        let without = tauri::async_runtime::block_on(remove_password(RunEvents::silent(), PasswordRequest {
            paths: inputs.clone(),
            password: TEST_PASSWORD.into(),
            password_overrides: Default::default(),
            output_location: location(&root, "batch-only"),
        }));
        assert!(without[0].error().is_none() && without[3].error().is_none(), "one wrong password must not stop the other files");
        for failed in [&without[1], &without[2]] {
            let wrong = failed.error().expect("the batch password must not open a file with its own password");
            assert_eq!(wrong.kind, crate::kit::contracts::ErrorKind::WrongPassword);
            assert!(!wrong.message.contains(TEST_PASSWORD) && !wrong.message.contains(TEST_OVERRIDE_PASSWORD));
            assert!(failed.output_paths().is_empty());
        }
        let finished = [&without[0], &without[3]].map(|outcome| {
            let path = outcome.output_paths()[0].clone();
            let bytes = std::fs::read(&path).unwrap();
            let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
            (path, bytes, modified)
        });
        assert_eq!(std::fs::read_dir(root.join("batch-only")).unwrap().count(), 2);

        let retried = tauri::async_runtime::block_on(remove_password(RunEvents::silent(), PasswordRequest {
            paths: vec![own_pdf.clone()],
            password: String::new(),
            password_overrides: overrides(&[(&own_pdf, TEST_OVERRIDE_PASSWORD)]),
            output_location: location(&root, "batch-only"),
        }));
        assert_eq!(retried.len(), 1, "a retry runs only the one file");
        assert!(retried[0].error().is_none(), "{}", retried[0].error().cloned().unwrap_or_default());
        assert!(opens_without_password(&retried[0].output_paths()[0]));
        assert_eq!(std::fs::read_dir(root.join("batch-only")).unwrap().count(), 3, "the retry adds its output beside the earlier ones");
        for (path, bytes, modified) in finished {
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "a retry rewrote an earlier output: {}", path.display());
            assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), modified, "a retry touched an earlier output: {}", path.display());
        }

        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "locked original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    fn locked_pdf(root: &PathBuf, name: &str, password: &str) -> PathBuf {
        let source = root.join(format!("{name}.pdf"));
        make_pdf(&source, 2);
        let locked = PDFProcessor::protect(source, password, &location(root, "locked"));
        assert!(locked.error().is_none(), "{}", locked.error().cloned().unwrap_or_default());
        locked.output_paths()[0].clone()
    }

    fn file_names(folder: &PathBuf) -> Vec<String> {
        let mut names = std::fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        names.sort();
        names
    }

    fn opens_without_password(path: &PathBuf) -> bool {
        let format = crate::kit::password::ProtectedFormat::from_path(path).unwrap();
        crate::kit::password::probe(&std::fs::read(path).unwrap(), format).unwrap() == crate::kit::password::ProtectionState::NotProtected
    }

    #[test]
    fn unlock_runs_a_mixed_pdf_and_xlsx_batch_per_file_without_touching_the_originals() {
        let root = sandbox("unlock-mixed-batch");
        let pdfs = [locked_pdf(&root, "lease", "batch-secret"), locked_pdf(&root, "vendor-nda", "batch-secret")];
        let sheets = ["budget", "payroll", "inventory", "forecast"].map(|name| locked_office(&root, name, "xlsx", "batch-secret"));
        let other = locked_office(&root, "other-password", "xlsx", "other-secret");
        let inputs_dir = root.join("locked");
        let photo = inputs_dir.join("site-photo.png");
        let notes = inputs_dir.join("notes.txt");
        write_png(&photo, 8, 40);
        std::fs::write(&notes, b"meeting notes").unwrap();
        let inputs = vec![
            pdfs[0].clone(), sheets[0].clone(), photo.clone(), sheets[1].clone(), other.clone(),
            pdfs[1].clone(), notes.clone(), sheets[2].clone(), sheets[3].clone(),
        ];
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();
        let beside_inputs = file_names(&inputs_dir);
        let unlock = |output_location: OutputLocation, paths: Vec<PathBuf>| tauri::async_runtime::block_on(remove_password(RunEvents::silent(), PasswordRequest {
            paths,
            password: "batch-secret".into(),
            password_overrides: Default::default(),
            output_location,
        }));

        let custom = location(&root, "unlocked");
        let outcomes = unlock(custom.clone(), inputs.clone());
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs, "one outcome per input, in input order");
        for (input, outcome) in inputs.iter().zip(&outcomes) {
            if [&photo, &notes, &other].contains(&input) {
                assert!(outcome.output_paths().is_empty(), "{} must not produce an output", input.display());
                continue;
            }
            assert!(outcome.error().is_none(), "{}: {}", input.display(), outcome.error().cloned().unwrap_or_default());
            let output = &outcome.output_paths()[0];
            assert_eq!(output.parent().unwrap(), root.join("unlocked"));
            let stem = input.file_stem().unwrap().to_string_lossy();
            let extension = input.extension().unwrap().to_string_lossy();
            assert_eq!(output.file_name().unwrap().to_string_lossy(), format!("{stem}-unlocked.{extension}"));
            assert!(opens_without_password(output), "{} still needs a password", output.display());
            if extension == "xlsx" {
                assert_eq!(std::fs::read(output).unwrap(), test_minimal_ooxml_package("xlsx"));
            }
        }
        let wrong = outcomes[4].error().expect("a file locked with another password fails on its own");
        assert_eq!(wrong.kind, ErrorKind::WrongPassword);
        assert!(!wrong.message.contains("batch-secret") && !wrong.message.contains("other-secret"));
        for unsupported in [&outcomes[2], &outcomes[6]] {
            assert_eq!(unsupported.error().expect("an image or text file fails on its own").kind, ErrorKind::InvalidInput);
        }
        assert_eq!(std::fs::read_dir(root.join("unlocked")).unwrap().count(), 6, "every output lands in the custom folder");
        assert_eq!(file_names(&inputs_dir), beside_inputs, "nothing is written beside the inputs");

        let again = vec![pdfs[0].clone(), sheets[0].clone()];
        for _ in 0..2 {
            assert!(unlock(custom.clone(), again.clone()).iter().all(|outcome| outcome.error().is_none()));
        }
        let unlocked_names = file_names(&root.join("unlocked"));
        for name in ["lease-protected-unlocked.pdf", "lease-protected-unlocked-1.pdf", "lease-protected-unlocked-2.pdf", "budget-protected-unlocked.xlsx", "budget-protected-unlocked-1.xlsx", "budget-protected-unlocked-2.xlsx"] {
            assert!(unlocked_names.iter().any(|existing| existing == name), "{name} missing from {unlocked_names:?}");
        }

        let alongside = unlock(OutputLocation::AlongsideInput, again.clone());
        assert!(alongside.iter().all(|outcome| outcome.error().is_none() && outcome.output_paths()[0].parent() == Some(inputs_dir.as_path())));

        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unlock_skips_files_that_need_no_work_without_writing_or_reserving_a_name() {
        use crate::kit::contracts::{OutcomeState, SkipCode};
        use crate::kit::office::tests::ppt_plain_fixture;
        use crate::kit::password::tests::{pdf, word_document, workbook};
        use crate::kit::password::NOT_PASSWORD_PROTECTED;

        let root = sandbox("unlock-skips");
        let plain_dir = root.join("plain");
        std::fs::create_dir_all(&plain_dir).unwrap();
        let plain = |name: &str, bytes: Vec<u8>| {
            let path = plain_dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };
        let skipped_plain = vec![
            plain("lease.pdf", pdf(None)),
            plain("budget.xlsx", test_minimal_ooxml_package("xlsx")),
            plain("offer.docx", test_minimal_ooxml_package("docx")),
            plain("deck.pptx", test_minimal_ooxml_package("pptx")),
            plain("minutes.doc", word_document(0)),
            plain("ledger.xls", workbook(false)),
            plain("kickoff.ppt", ppt_plain_fixture()),
        ];
        let locked_lease = locked_pdf(&root, "lease", "batch-secret");
        let locked_lease = {
            let renamed = locked_lease.with_file_name("lease.pdf");
            std::fs::rename(&locked_lease, &renamed).unwrap();
            renamed
        };
        let locked_budget = locked_office(&root, "budget", "xlsx", "batch-secret");
        let locked_budget = {
            let renamed = locked_budget.with_file_name("budget.xlsx");
            std::fs::rename(&locked_budget, &renamed).unwrap();
            renamed
        };

        let mut inputs = skipped_plain.clone();
        inputs.push(locked_lease.clone());
        inputs.push(locked_budget.clone());
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();
        let beside_plain = file_names(&plain_dir);

        let outcomes = tauri::async_runtime::block_on(remove_password(RunEvents::silent(), PasswordRequest {
            paths: inputs.clone(),
            password: "batch-secret".into(),
            password_overrides: Default::default(),
            output_location: location(&root, "unlocked"),
        }));
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs, "one outcome per input, in input order");

        for (input, outcome) in skipped_plain.iter().zip(&outcomes) {
            let OutcomeState::Skipped { reason } = &outcome.state else { panic!("{} should be skipped, got {:?}", input.display(), outcome.state) };
            assert_eq!(reason.code, SkipCode::NotProtected, "{}", input.display());
            assert_eq!(reason.message, NOT_PASSWORD_PROTECTED);
            assert!(reason.pointer.is_none());
        }
        for outcome in &outcomes[skipped_plain.len()..] {
            assert!(matches!(outcome.state, OutcomeState::Succeeded { .. }), "{}: {:?}", outcome.input_path.display(), outcome.state);
        }
        assert_eq!(
            file_names(&root.join("unlocked")),
            vec!["budget-unlocked.xlsx".to_string(), "lease-unlocked.pdf".to_string()],
            "skipped files with the same names wrote nothing and reserved no name ahead of the locked ones",
        );
        assert_eq!(file_names(&plain_dir), beside_plain, "nothing is written beside the skipped inputs");
        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unlock_clears_print_and_copy_restrictions_without_a_password_while_open_passwords_still_need_theirs() {
        use crate::kit::contracts::OutcomeState;
        use crate::kit::password::tests::pdf;
        use crate::kit::password::WRONG_PASSWORD;

        let qpdf = crate::kit::resources::resolve_helper(&crate::kit::resources::QPDF).expect("qpdf is required for the native suite");
        let root = sandbox("unlock-restrictions");
        let inputs_dir = root.join("inputs");
        std::fs::create_dir_all(&inputs_dir).unwrap();
        let brochure = inputs_dir.join("brochure.pdf");
        std::fs::write(&brochure, pdf(Some(("owner-secret", "")))).unwrap();
        let catalog_plain = root.join("catalog-plain.pdf");
        make_pdf(&catalog_plain, 3);
        let catalog = inputs_dir.join("catalog.pdf");
        let encrypted = std::process::Command::new(&qpdf)
            .arg(&catalog_plain).arg(&catalog)
            .args(["--encrypt", "--user-password=", "--owner-password=owner-secret", "--bits=256", "--print=none", "--extract=n", "--"])
            .status().unwrap();
        assert!(encrypted.success(), "qpdf made the AES-256 owner-only fixture");
        let lease = locked_pdf(&root, "lease", "open-secret");
        let vendor = locked_pdf(&root, "vendor", "open-secret");
        for restricted in [&brochure, &catalog] {
            assert_eq!(crate::kit::password::probe(&std::fs::read(restricted).unwrap(), crate::kit::password::ProtectedFormat::Pdf).unwrap(), crate::kit::password::ProtectionState::PermissionsOnly, "{}", restricted.display());
        }

        let output = location(&root, "unlocked");
        let OutputLocation::CustomFolder(output_dir) = &output else { unreachable!() };
        std::fs::write(output_dir.join("brochure-unlocked.pdf"), b"keep me").unwrap();
        let inputs = vec![brochure.clone(), lease.clone(), catalog.clone(), vendor.clone()];
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();

        let outcomes = tauri::async_runtime::block_on(remove_password(RunEvents::silent(), PasswordRequest {
            paths: inputs.clone(),
            password: String::new(),
            password_overrides: [(vendor.clone(), "not-it".to_string())].into_iter().collect(),
            output_location: output.clone(),
        }));
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs, "one outcome per input, in input order");

        for (outcome, name, pages) in [(&outcomes[0], "brochure-unlocked-1.pdf", 1), (&outcomes[2], "catalog-unlocked.pdf", 3)] {
            assert!(matches!(outcome.state, OutcomeState::Succeeded { .. }), "{}: {:?}", outcome.input_path.display(), outcome.state);
            assert_eq!(outcome.detail, "Unlocked. Restrictions removed.");
            assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
            let path = &outcome.output_paths()[0];
            assert_eq!(path, &output_dir.join(name));
            assert!(opens_without_password(path), "{} still has encryption", path.display());
            let unlocked = lopdf::Document::load(path).unwrap();
            assert!(unlocked.trailer.get(b"Encrypt").is_err(), "{} keeps an /Encrypt dictionary", path.display());
            assert_eq!(unlocked.get_pages().len(), pages);
            let check = std::process::Command::new(&qpdf).arg("--show-encryption").arg(path).output().unwrap();
            assert!(String::from_utf8_lossy(&check.stdout).contains("File is not encrypted"), "{}", String::from_utf8_lossy(&check.stdout));
        }
        assert_eq!(std::fs::read(output_dir.join("brochure-unlocked.pdf")).unwrap(), b"keep me", "an existing output is never replaced");

        let missing = outcomes[1].error().expect("an open-password PDF with no password fails on its own");
        assert_eq!(missing.kind, ErrorKind::InvalidInput);
        assert_eq!(missing.message, "Enter the PDF password.");
        let wrong = outcomes[3].error().expect("an open-password PDF with the wrong password fails on its own");
        assert_eq!(wrong.kind, ErrorKind::WrongPassword);
        assert_eq!(wrong.message, WRONG_PASSWORD);
        assert!(outcomes[1].output_paths().is_empty() && outcomes[3].output_paths().is_empty());
        assert_eq!(
            file_names(output_dir),
            vec!["brochure-unlocked-1.pdf".to_string(), "brochure-unlocked.pdf".to_string(), "catalog-unlocked.pdf".to_string()],
            "failed files leave nothing behind",
        );
        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unlock_verifies_legacy_outputs_and_reports_each_failure_on_its_own_file() {
        use crate::kit::office::tests::{locked_doc, locked_xls, ppt_failing_verification_fixture, ppt_fixture, LegacyEncryption, TEST_PASSWORD};
        use crate::kit::password::{unreadable, ProtectedFormat, LEGACY_PASSWORD_TOO_LONG, LEGACY_PPT_PICTURES, UNLOCK_NOT_VERIFIED, WRONG_PASSWORD};

        let root = sandbox("unlock-legacy-verification");
        let legacy_dir = root.join("legacy");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let write = |name: &str, bytes: Vec<u8>| {
            let path = legacy_dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };
        let pdf = locked_pdf(&root, "lease", TEST_PASSWORD);
        let succeeding = vec![
            write("memo-xor.doc", locked_doc(LegacyEncryption::Xor)),
            write("memo-rc4.doc", locked_doc(LegacyEncryption::Rc4)),
            write("memo-cryptoapi.doc", locked_doc(LegacyEncryption::CryptoApi)),
            write("ledger-xor.xls", locked_xls(LegacyEncryption::Xor)),
            write("ledger-rc4.xls", locked_xls(LegacyEncryption::Rc4)),
            write("ledger-cryptoapi.xls", locked_xls(LegacyEncryption::CryptoApi)),
            write("kickoff.ppt", ppt_fixture(true, false)),
            locked_office(&root, "offer", "docx", TEST_PASSWORD),
            pdf.clone(),
        ];
        let locked_sheet = locked_xls(LegacyEncryption::Rc4);
        let locked_lease = std::fs::read(&pdf).unwrap();
        let wrong = write("wrong.doc", locked_doc(LegacyEncryption::CryptoApi));
        let long = write("long.xls", locked_xls(LegacyEncryption::Xor));
        let failing = vec![
            (write("photos.ppt", ppt_fixture(true, true)), ErrorKind::Unavailable, LEGACY_PPT_PICTURES.to_string()),
            (long.clone(), ErrorKind::Unavailable, LEGACY_PASSWORD_TOO_LONG.to_string()),
            (wrong.clone(), ErrorKind::WrongPassword, WRONG_PASSWORD.to_string()),
            (write("truncated.xls", locked_sheet[..locked_sheet.len() / 2].to_vec()), ErrorKind::Processing, unreadable(ProtectedFormat::Xls).message),
            (write("truncated.pdf", locked_lease[..locked_lease.len() / 2].to_vec()), ErrorKind::Processing, unreadable(ProtectedFormat::Pdf).message),
            (write("unverified.ppt", ppt_failing_verification_fixture()), ErrorKind::Processing, UNLOCK_NOT_VERIFIED.to_string()),
        ];
        let mut inputs = Vec::new();
        for (index, input) in succeeding.iter().enumerate() {
            inputs.push(input.clone());
            if let Some((failure, _, _)) = failing.get(index) {
                inputs.push(failure.clone());
            }
        }
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();
        let beside_inputs = file_names(&legacy_dir);

        let outcomes = tauri::async_runtime::block_on(remove_password(RunEvents::silent(), PasswordRequest {
            paths: inputs.clone(),
            password: TEST_PASSWORD.into(),
            password_overrides: overrides(&[(&long, TEST_PASSWORD.repeat(2).as_str()), (&wrong, "not-the-password")]),
            output_location: location(&root, "unlocked"),
        }));
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs, "one outcome per input, in input order");

        for outcome in &outcomes {
            if let Some((_, kind, message)) = failing.iter().find(|(input, _, _)| *input == outcome.input_path) {
                let error = outcome.error().unwrap_or_else(|| panic!("{} must fail", outcome.input_path.display()));
                assert_eq!((&error.kind, error.message.as_str()), (kind, message.as_str()), "{}", outcome.input_path.display());
                assert!(outcome.output_paths().is_empty(), "{} has no output", outcome.input_path.display());
                continue;
            }
            assert!(outcome.error().is_none(), "{}: {}", outcome.input_path.display(), outcome.error().cloned().unwrap_or_default());
            assert!(opens_without_password(&outcome.output_paths()[0]), "{} still needs a password", outcome.input_path.display());
        }
        assert_eq!(std::fs::read_dir(root.join("unlocked")).unwrap().count(), succeeding.len(), "a failed file leaves no output or temporary file");
        assert_eq!(file_names(&legacy_dir), beside_inputs, "nothing is written beside the inputs");
        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn lock_gives_an_overridden_file_its_own_password() {
        let root = sandbox("lock-overrides");
        let shared = root.join("shared.docx");
        let own = root.join("own.docx");
        let own_pdf = root.join("own.pdf");
        for input in [&shared, &own] { std::fs::write(input, test_minimal_ooxml_package("docx")).unwrap(); }
        make_pdf(&own_pdf, 1);

        let locked = tauri::async_runtime::block_on(protect_files(RunEvents::silent(), PasswordRequest {
            paths: vec![shared.clone(), own.clone(), own_pdf.clone()],
            password: crate::kit::TEST_PASSWORD.into(),
            password_overrides: overrides(&[(&own, crate::kit::TEST_OVERRIDE_PASSWORD), (&own_pdf, crate::kit::TEST_OVERRIDE_PASSWORD)]),
            output_location: location(&root, "locked"),
        }));
        assert!(locked.iter().all(|outcome| outcome.error().is_none()));
        let opens = |path: &PathBuf, password: &str| {
            crate::kit::password::PasswordProcessor::remove_password(path.clone(), password, &location(&root, "check")).error().is_none()
        };
        assert!(opens(&locked[0].output_paths()[0], crate::kit::TEST_PASSWORD));
        for overridden in [&locked[1], &locked[2]] {
            assert!(opens(&overridden.output_paths()[0], crate::kit::TEST_OVERRIDE_PASSWORD));
            assert!(!opens(&overridden.output_paths()[0], crate::kit::TEST_PASSWORD));
        }

        let _ = std::fs::remove_dir_all(root);
    }

    // The fake helper is a /bin/sh script, so this check runs on Unix-like targets only.
    #[cfg(unix)]
    #[test]
    fn locking_keeps_every_password_out_of_qpdf_arguments_and_environment() {
        use crate::kit::common::fake_helper::{secret_exposure, Exposure, FakeHelper};
        use crate::kit::{TEST_OVERRIDE_PASSWORD, TEST_PASSWORD};
        let _guard = crate::kit::PROCESS_ENV_LOCK.lock().unwrap();
        let real = crate::kit::resources::resolve_helper(&crate::kit::resources::QPDF).expect("qpdf is required for the native suite");
        let root = sandbox("protect-files-fake-qpdf");
        let inputs = ["first", "own", "last"].map(|stem| root.join(format!("{stem}.pdf"))).to_vec();
        for input in &inputs { make_pdf(input, 1); }
        let helper = FakeHelper::install(&root, "qpdf", Some(&real));

        let previous = std::env::var_os("TOOLBOX_QPDF_PATH");
        std::env::set_var("TOOLBOX_QPDF_PATH", &helper.script);
        let outcomes = tauri::async_runtime::block_on(protect_files(RunEvents::silent(), PasswordRequest {
            paths: inputs.clone(),
            password: TEST_PASSWORD.into(),
            password_overrides: overrides(&[(&inputs[1], TEST_OVERRIDE_PASSWORD)]),
            output_location: location(&root, "locked"),
        }));
        match previous {
            Some(value) => std::env::set_var("TOOLBOX_QPDF_PATH", value),
            None => std::env::remove_var("TOOLBOX_QPDF_PATH"),
        }
        // Tests that do not take the env lock may reach the fake while it is installed, so keep only this run's calls.
        let calls = helper
            .invocations()
            .into_iter()
            .filter(|call| inputs.iter().any(|input| call.arguments.contains(&input.display().to_string())))
            .collect::<Vec<_>>();

        assert_eq!(calls.len(), 3, "the helper runs once per file");
        for call in &calls {
            assert_eq!(call.arguments.len(), 6, "{:?}", call.arguments);
            assert_eq!(call.arguments[2..], ["--encrypt", "@-", "--bits=256", "--"], "only non-secret options reach the command line");
        }
        for secret in [TEST_PASSWORD, TEST_OVERRIDE_PASSWORD] {
            assert_eq!(secret_exposure(&calls, secret), Exposure::default(), "a password reached the helper's arguments or environment");
        }
        for (outcome, (password, other)) in outcomes.iter().zip([(TEST_PASSWORD, TEST_OVERRIDE_PASSWORD), (TEST_OVERRIDE_PASSWORD, TEST_PASSWORD), (TEST_PASSWORD, TEST_OVERRIDE_PASSWORD)]) {
            assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
            let output = &outcome.output_paths()[0];
            assert!(output.to_string_lossy().ends_with("-protected.pdf"));
            let opens = |password: &str| lopdf::Document::load_with_options(output, lopdf::LoadOptions::with_password(password)).is_ok_and(|document| !document.is_encrypted());
            assert!(opens(password) && !opens(other), "{} must open with its own password only", output.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    // One worker makes the order deterministic: two files finish, the third stalls in its helper, the rest wait.
    #[cfg(unix)]
    #[test]
    fn cancelling_mid_batch_keeps_finished_files_ends_the_helper_and_skips_the_rest() {
        use crate::kit::common::batch_runner::{cancel_run, CANCELLED_BEFORE_START};
        use crate::kit::common::fake_helper::FakeHelper;
        use crate::kit::contracts::{ErrorKind, SkipCode};
        let _guard = crate::kit::PROCESS_ENV_LOCK.lock().unwrap();
        let real = crate::kit::resources::resolve_helper(&crate::kit::resources::QPDF).expect("qpdf is required for the native suite");
        let root = sandbox("cancel-mid-batch");
        let inputs = ["first", "second", "stalls", "fourth", "fifth"].map(|stem| root.join(format!("{stem}.pdf"))).to_vec();
        for input in &inputs { make_pdf(input, 1); }
        let originals = inputs.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let helper = std::sync::Arc::new(FakeHelper::install_stalling_on(&root, "qpdf", Some(&real), "stalls.pdf"));
        let output = location(&root, "locked");

        let previous = std::env::var_os("TOOLBOX_QPDF_PATH");
        std::env::set_var("TOOLBOX_QPDF_PATH", &helper.script);
        let run_id = format!("cancel-test-{}", std::process::id());
        let canceller = {
            let (helper, run_id) = (helper.clone(), run_id.clone());
            std::thread::spawn(move || {
                let started = std::time::Instant::now();
                while helper.stalled_pids().is_empty() {
                    assert!(started.elapsed() < std::time::Duration::from_secs(30), "the slow helper never started");
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                assert!(cancel_run(&run_id), "the running batch is found by its run id");
                std::time::Instant::now()
            })
        };
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let outcomes = pool.install(|| tauri::async_runtime::block_on(protect_files(
            RunEvents::for_run(run_id.clone(), move |event| sink.lock().unwrap().push(event)),
            PasswordRequest { paths: inputs.clone(), password: crate::kit::TEST_PASSWORD.into(), password_overrides: Default::default(), output_location: output.clone() },
        )));
        match previous {
            Some(value) => std::env::set_var("TOOLBOX_QPDF_PATH", value),
            None => std::env::remove_var("TOOLBOX_QPDF_PATH"),
        }
        let cancelled_at = canceller.join().unwrap();

        assert!(cancelled_at.elapsed() < std::time::Duration::from_millis(1500), "the stalled helper is ended, not waited out");
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs, "the response keeps input order");
        for finished in &outcomes[..2] {
            assert!(finished.error().is_none(), "{}", finished.error().cloned().unwrap_or_default());
            assert!(finished.output_paths()[0].is_file(), "a finished file keeps its output");
        }
        match &outcomes[2].state {
            OutcomeState::Failed { output_paths, error } => {
                assert_eq!(error.kind, ErrorKind::Cancelled);
                assert_eq!(error.message, "Cancelled while it was running. Nothing was saved.");
                assert!(output_paths.is_empty());
            }
            state => panic!("the file in progress fails as cancelled, got {state:?}"),
        }
        for waiting in &outcomes[3..] {
            match &waiting.state {
                OutcomeState::Skipped { reason } => {
                    assert_eq!(reason.code, SkipCode::Cancelled);
                    assert_eq!(reason.message, CANCELLED_BEFORE_START);
                }
                state => panic!("a file that had not started is skipped as cancelled, got {state:?}"),
            }
        }
        let events = seen.lock().unwrap().clone();
        for (index, outcome) in outcomes.iter().enumerate() {
            assert!(events.contains(&FileEvent::Finished { index, outcome: outcome.clone() }), "file {index} streams its final outcome");
        }

        for pid in helper.stalled_pids() {
            let alive = std::process::Command::new("kill").arg("-0").arg(pid.to_string()).stderr(std::process::Stdio::null()).status().unwrap().success();
            assert!(!alive, "the stalled helper {pid} was terminated");
        }
        assert!(!cancel_run(&run_id), "a finished run is no longer registered");
        let mut left = std::fs::read_dir(root.join("locked")).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        left.sort();
        assert_eq!(left, vec!["first-protected.pdf", "second-protected.pdf"], "no temporary or claim file is left behind");
        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn convert_command_rejects_bogus_and_unsupported_formats_without_writing() {
        for format in ["gif", "bogus"] {
            let root = sandbox(&format!("convert-invalid-{format}"));
            let src = root.join("source.png");
            write_png(&src, 32, 4);
            let original = std::fs::read(&src).unwrap();
            let outcomes = tauri::async_runtime::block_on(convert_images(RunEvents::silent(), ConvertImagesRequest {
                paths: vec![src.clone()], format: format.to_string(), output_location: location(&root, "output"),
            }));
            assert_eq!(outcomes.len(), 1);
            assert!(outcomes[0].error().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput)));
            assert!(outcomes[0].error().is_some_and(|error| error.message.contains("Unsupported image format")));
            assert!(outcomes[0].output_paths().is_empty());
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

        let out = tauri::async_runtime::block_on(compress_images(RunEvents::silent(), CompressImagesRequest {
            paths: vec![src.clone()], quality: 50, lossless: false, output_location: OutputLocation::AlongsideInput,
        }));
        assert!(out[0].error().is_none(), "{}", out[0].error().cloned().unwrap_or_default());
        assert_eq!(
            out[0].output_paths()[0].extension().and_then(|e| e.to_str()),
            Some("png"),
            "compress keeps the source format"
        );
        assert_eq!(std::fs::metadata(&src).unwrap().len(), before, "originals untouched");

        cleanup(&out[0].output_paths());
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn metadata_command_keeps_batch_failures_isolated() {
        let valid = temp_path("cmd_mixed_valid.png");
        let missing = temp_path("cmd_mixed_missing.png");
        write_png(&valid, 64, 9);

        let out = tauri::async_runtime::block_on(image_metadata(RunEvents::silent(), MetadataRequest {
            paths: vec![valid.clone(), missing.clone()],
            output_location: OutputLocation::AlongsideInput,
        }));
        assert_eq!(out.len(), 2);
        assert_eq!(out.iter().map(|job| job.input_path.clone()).collect::<Vec<_>>(), vec![valid.clone(), missing.clone()]);
        assert!(out.iter().any(|job| job.error().is_none()));
        assert!(out.iter().any(|job| job.error().is_some()));

        cleanup(&out.iter().flat_map(|job| job.output_paths().to_vec()).collect::<Vec<_>>());
        let _ = std::fs::remove_file(valid);
    }

    #[test]
    fn page_commands_reject_empty_explicit_selections() {
        let root = sandbox("empty-pages");
        let pdf = root.join("document.pdf");
        make_pdf(&pdf, 2);

        let outcomes = tauri::async_runtime::block_on(pdf_to_text(RunEvents::silent(), PdfToTextRequest {
            paths: vec![pdf.clone()],
            pages: Some(Vec::new()),
            output_location: location(&root, "text"),
        }));

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].input_path, pdf);
        assert!(outcomes[0].output_paths().is_empty());
        assert!(outcomes[0].error().is_some_and(|error| error.message.contains("at least one page")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn conversion_commands_validate_formats_cardinality_and_page_scope_at_the_boundary() {
        let root = sandbox("conversion-boundary");
        let pdf = root.join("document.pdf");
        let second_pdf = root.join("second.pdf");
        let image = root.join("wrong-input.png");
        make_pdf(&pdf, 2);
        make_pdf(&second_pdf, 2);
        write_png(&image, 8, 4);

        let wrong_format = tauri::async_runtime::block_on(pdf_to_text(RunEvents::silent(), PdfToTextRequest {
            paths: vec![image.clone()],
            pages: Some(vec![0]),
            output_location: location(&root, "wrong-format"),
        }));
        assert_eq!(wrong_format.len(), 1);
        assert!(wrong_format[0].output_paths().is_empty());
        assert!(wrong_format[0].error().is_some_and(|error| error.message.contains("Accepted extensions")));

        let wrong_cardinality = tauri::async_runtime::block_on(split_pdf(RunEvents::silent(), PageSelectionRequest {
            paths: vec![pdf.clone(), second_pdf.clone()],
            pages: vec![],
            page_ranges: None,
            split_mode: Some("pages".to_string()),
            chunk_size: None,
            output_location: location(&root, "wrong-cardinality"),
        }));
        assert_eq!(wrong_cardinality.len(), 2);
        assert!(wrong_cardinality.iter().all(|outcome| outcome.output_paths().is_empty() && outcome.error().is_some()));

        let selected = tauri::async_runtime::block_on(pdf_to_text(RunEvents::silent(), PdfToTextRequest {
            paths: vec![pdf.clone()],
            pages: Some(vec![1]),
            output_location: location(&root, "selected-text"),
        }));
        let output = selected[0].output_paths().first().expect("selected text should produce an output");
        let text = std::fs::read_to_string(output).unwrap();
        assert!(text.contains("Toolbox page 2"), "selected text: {text}");
        assert!(!text.contains("Toolbox page 1"), "unselected text: {text}");

        cleanup(&selected.iter().flat_map(|outcome| outcome.output_paths().to_vec()).collect::<Vec<_>>());
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

        let outcomes = tauri::async_runtime::block_on(compress_images(RunEvents::silent(), CompressImagesRequest {
            paths: vec![image.clone()],
            quality: 80,
            lossless: false,
            output_location: OutputLocation::AlongsideInput,
        }));

        assert!(outcomes[0].error().is_none(), "{}", outcomes[0].error().cloned().unwrap_or_default());
        assert_ne!(outcomes[0].output_paths()[0], existing);
        assert_eq!(std::fs::read(&existing).unwrap(), b"existing output");
        assert_eq!(std::fs::read(&image).unwrap(), original);
        cleanup(&outcomes[0].output_paths());
        let _ = std::fs::remove_dir_all(root);
    }

    // Regression test: organize_pdf with AlongsideInput must save the output
    // next to the input and report it (guards against false-success).
    #[test]
    fn organize_alongside_input_writes_output() {
        let root = sandbox("organize-repro");
        let input = root.join("sample.pdf");
        make_pdf(&input, 3);
        let outputs = assert_success(
            "organize alongside",
            organize_pdf(RunEvents::silent(), OrganizePdfRequest {
                paths: vec![input.clone()],
                page_order: vec![2, 0, 1],
                delete_pages: vec![],
                rotate_pages: vec![],
                scope: PageScope::All,
                output_location: OutputLocation::AlongsideInput,
            }),
        );
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].parent(), Some(root.as_path()));
        assert!(outputs[0].exists(), "organize output must exist on disk");
    }

    #[test]
    fn custom_folder_numbers_same_stem_inputs_from_different_folders() {
        let root = sandbox("custom-folder-numbering");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        let first = root.join("a/photo.png");
        let second = root.join("b/photo.png");
        write_png(&first, 32, 1);
        write_png(&second, 32, 2);
        let output_location = location(&root, "results");

        let outcomes = tauri::async_runtime::block_on(image_metadata(RunEvents::silent(), MetadataRequest {
            paths: vec![first.clone(), second.clone()], output_location,
        }));

        let mut outputs = outcomes.iter().flat_map(|outcome| {
            assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
            outcome.output_paths().to_vec()
        }).collect::<Vec<_>>();
        outputs.sort();
        assert_eq!(outputs, vec![root.join("results/photo-stripped-1.png"), root.join("results/photo-stripped.png")]);
        assert!(outputs.iter().all(|output| output.is_file()));
        assert!(!root.join("a/photo-stripped.png").exists() && !root.join("b/photo-stripped.png").exists());
    }

    #[cfg(unix)]
    #[test]
    fn read_only_alongside_folder_fails_only_that_file() {
        use crate::kit::common::batch_runner::READ_ONLY_OUTPUT_MESSAGE;
        use crate::kit::contracts::{ErrorKind, OutcomePointer};
        use std::os::unix::fs::PermissionsExt;
        let root = sandbox("read-only-alongside");
        let locked = root.join("locked");
        let open = root.join("open");
        for folder in [&locked, &open] {
            if folder.exists() { std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o755)).unwrap(); }
            let _ = std::fs::remove_dir_all(folder);
            std::fs::create_dir_all(folder).unwrap();
        }
        let blocked = locked.join("scan.png");
        let writable = open.join("scan.png");
        write_png(&blocked, 32, 3);
        write_png(&writable, 32, 4);
        let original = std::fs::read(&blocked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();

        let outcomes = tauri::async_runtime::block_on(image_metadata(RunEvents::silent(), MetadataRequest {
            paths: vec![blocked.clone(), writable.clone()], output_location: OutputLocation::AlongsideInput,
        }));
        let listing = std::fs::read_dir(&locked).unwrap().map(|entry| entry.unwrap().file_name()).collect::<Vec<_>>();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

        let error = outcomes[0].error().expect("the read-only folder must fail");
        assert_eq!(error.kind, ErrorKind::Processing);
        assert_eq!(error.message, READ_ONLY_OUTPUT_MESSAGE);
        assert_eq!(error.pointer, Some(OutcomePointer::OutputFolder));
        assert!(outcomes[0].output_paths().is_empty());
        assert!(outcomes[1].error().is_none(), "{}", outcomes[1].error().cloned().unwrap_or_default());
        assert_eq!(outcomes[1].output_paths(), [open.join("scan-stripped.png")]);
        assert_eq!(std::fs::read(&blocked).unwrap(), original, "originals stay untouched");
        assert_eq!(listing, vec![std::ffi::OsString::from("scan.png")], "nothing is left in the read-only folder");
    }
}
