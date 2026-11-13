#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod file_actions;
mod intake;
mod kit;
mod packs;
mod quick_action;
mod settings;
mod shell;

use crate::file_actions::{open_output_path, open_sponsor_page, open_third_party_notices, reveal_output_path};
use crate::kit::common::{note_renamed, with_measured_sizes, JobOutcome};
use crate::kit::images::{ImageProcessor, Options as ImageOptions};
use crate::kit::images::tools;
use crate::kit::compress::{compress_file, CompressFilesRequest};
use crate::kit::pdf::{convert, editor, metadata, privacy, properties, remaining, scene, PDFProcessor};
use crate::kit::pdf::page_ranges::{require_page_ranges, validate_page_ranges};
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
use tauri::Manager;

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
        with_measured_sizes(ImageProcessor::run(path, ImageOptions {
            target_format: None,
            quality: if request.lossless { 0 } else { request.quality },
            keep_smaller_original: true,
            suffix: "-compressed".to_string(),
            output_location: request.output_location.clone(),
        }))
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
        ImageProcessor::convert(path, img_format, request.output_location.clone())
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
async fn inspect_pdf_scene(request: InspectPdfRequest) -> Result<metadata::PdfDocumentMetadata, ToolError> {
    tauri::async_runtime::spawn_blocking(move || scene::inspect(&request.path)).await.map_err(|error| ToolError::processing(error.to_string()))?
}

#[tauri::command]
async fn inspect_pdf_form(request: InspectPdfRequest) -> Result<scene::FormInspection, String> {
    tauri::async_runtime::spawn_blocking(move || scene::inspect_form(&request.path)).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn inspect_pdf_scene_page(request: InspectPdfScenePageRequest) -> Result<metadata::PdfPageMetadata, String> {
    tauri::async_runtime::spawn_blocking(move || scene::inspect_page(&request.path, request.page_index)).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn inspect_pdf_watermark_image(request: InspectPdfRequest) -> Result<scene::WatermarkImageInfo, String> {
    tauri::async_runtime::spawn_blocking(move || scene::inspect_watermark_image(&request.path)).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn inspect_signature_image(request: InspectPdfRequest) -> Result<scene::SignatureSize, String> {
    tauri::async_runtime::spawn_blocking(move || scene::inspect_signature_image(&request.path)).await.map_err(|error| error.to_string())?
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
async fn export_pdf_scene(app: tauri::AppHandle, events: RunEvents, request: scene::ExportRequest) -> Vec<JobOutcome> {
    let chosen = request.output_path.clone();
    let outcomes = run_export_pdf_scene(events, request).await;
    settings::remember_chosen_save(&app, chosen.as_deref(), outcomes)
}

async fn run_export_pdf_scene(events: RunEvents, request: scene::ExportRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "export_pdf_scene", request.paths.clone(), &request.output_location, &request, scene::export)
}

#[tauri::command]
async fn crop_pdf(events: RunEvents, request: editor::CropPdfRequest) -> Vec<JobOutcome> {
    let checked = match &request.scope {
        editor::RangedPageScope::All => Ok(()),
        editor::RangedPageScope::Selected { pages } => validate_page_selection("crop_pdf", &request.paths, Some(pages.as_slice())),
        editor::RangedPageScope::Range { ranges } => require_page_ranges(&request.paths, ranges).map(drop),
    };
    if let Err(outcomes) = checked { return outcomes; }
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
async fn add_page_numbers(events: RunEvents, request: remaining::PageOverlayRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_page_selection("add_page_numbers", &request.paths, request.pages.as_deref()) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "add_page_numbers", request.paths.clone(), &request.output_location, &request, remaining::add_page_numbers)
}

#[tauri::command]
async fn watermark_pdf(events: RunEvents, request: remaining::PdfWatermarkRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_page_selection("watermark_pdf", &request.paths, request.pages.as_deref()) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "watermark_pdf", request.paths.clone(), &request.output_location, &request, remaining::watermark)
}

#[tauri::command]
async fn rotate_pdf(events: RunEvents, request: remaining::RotatePdfRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_page_selection("rotate_pdf", &request.paths, request.pages.as_deref()) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "rotate_pdf", request.paths.clone(), &request.output_location, &request, remaining::rotate)
}

/// Reads each PDF and returns one report per file. It saves nothing, so it never touches an output folder.
#[tauri::command]
async fn inspect_pdf_privacy(events: RunEvents, request: privacy::PrivacyRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::inspect(&events, "inspect_pdf_privacy", request.paths.clone(), &request, privacy::inspect_job)
}

#[tauri::command]
async fn read_pdf_properties(events: RunEvents, request: properties::ReadPropertiesRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::inspect(&events, "edit_pdf_properties", request.paths.clone(), &request, properties::read_job)
}

#[tauri::command]
async fn edit_pdf_properties(events: RunEvents, request: properties::EditPropertiesRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = properties::validate(&request) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "edit_pdf_properties", request.paths.clone(), &request.output_location, &request, properties::edit_job)
}

/// Saves the reports the view already holds, one "-details" file per PDF. It never opens a PDF.
#[tauri::command]
async fn save_privacy_reports(request: privacy::SaveReportsRequest) -> Vec<JobOutcome> {
    privacy::save_reports(&request)
}

#[tauri::command]
async fn repair_pdf(events: RunEvents, request: remaining::RepairPdfRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "repair_pdf", request.paths.clone(), &request.output_location, &request, remaining::repair)
}

#[tauri::command]
async fn redact_pdf(events: RunEvents, request: editor::RedactPdfRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "redact_pdf", request.paths.clone(), &request.output_location, &request, editor::redact)
}

#[tauri::command]
async fn compare_pdfs(app: tauri::AppHandle, request: remaining::ComparePdfsRequest) -> Vec<JobOutcome> {
    let outcomes = run_compare_pdfs(&request).await;
    settings::remember_chosen_save(&app, request.output_path.as_deref(), outcomes)
}

async fn run_compare_pdfs(request: &remaining::ComparePdfsRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("compare_pdfs", &request.paths) { return outcomes; }
    let request = request.clone();
    aggregate_outcomes("compare_pdfs", &request.paths.clone(), move || note_renamed(remaining::compare(&request), request.output_path.as_deref()))
}

/// Each file is checked and compressed on its own, so an unsupported or locked file fails alone.
#[tauri::command]
async fn compress_files(events: RunEvents, request: CompressFilesRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "compress_files", request.paths.clone(), &request.output_location, &request, compress_file)
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
    let paths = request.paths.clone();
    let chosen = request.output_path.clone();
    let entries = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&entries);
    let outcomes = aggregate_outcomes("merge_pdfs", &paths, move || {
        let merged = remaining::merge(&request);
        *sink.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = merged.entries;
        merged.combined
    });
    let mut entries = std::mem::take(&mut *entries.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    // The combined file is one result, but each entry gets its own row with its page span and its own notes.
    if !matches!(outcomes.as_slice(), [combined] if matches!(combined.state, OutcomeState::Succeeded { .. })) || entries.len() != paths.len() {
        return outcomes;
    }
    entries[0] = note_renamed(entries[0].clone(), chosen.as_deref());
    entries
}

#[tauri::command]
async fn split_pdf(events: RunEvents, request: remaining::SplitPdfRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("split_pdf", &request.paths) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "split_pdf", request.paths.clone(), &request.output_location, &request, remaining::split)
}

#[tauri::command]
async fn pdf_to_images(events: RunEvents, request: remaining::PdfToImagesRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("pdf_to_images", &request.paths) { return outcomes; }
    if let Err(outcomes) = validate_page_selection("pdf_to_images", &request.paths, request.pages.as_deref()) { return outcomes; }
    if let Err(outcomes) = validate_page_ranges(&request.paths, request.page_range()) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "pdf_to_images", request.paths.clone(), &request.output_location, &request, remaining::to_images)
}

#[tauri::command]
async fn convert_to_pdf(events: RunEvents, request: serde_json::Value) -> Vec<JobOutcome> {
    convert::run(&events, request)
}

#[tauri::command]
async fn extract_from_pdf(events: RunEvents, request: remaining::ExtractFromPdfRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("extract_from_pdf", &request.paths) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "extract_from_pdf", request.paths.clone(), &request.output_location, &request, remaining::extract)
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
async fn ocr_pdf(events: RunEvents, request: vision::OcrRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = vision::validate_ocr_request(&request) { return outcomes; }
    if let Err(outcomes) = validate_command_inputs("ocr_pdf", &request.paths) { return outcomes; }
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
async fn process_tiff_pages(app: tauri::AppHandle, events: RunEvents, request: tools::TiffRequest) -> Vec<JobOutcome> {
    match request.mode {
        tools::TiffMode::Split => run_split_tiff_pages(&events, request),
        tools::TiffMode::Combine => {
            let outcomes = run_combine_tiff_pages(&request).await;
            settings::remember_chosen_save(&app, request.output_path.as_deref(), outcomes)
        }
    }
}

/// Split is a per-file batch: a corrupt or one-page TIFF settles alone and the rest still run.
fn run_split_tiff_pages(events: &RunEvents, request: tools::TiffRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(events, "process_tiff_pages", request.paths.clone(), &request.output_location, &request, tools::tiff_split)
}

async fn run_combine_tiff_pages(request: &tools::TiffRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("process_tiff_pages", &request.paths) { return outcomes; }
    let request = request.clone();
    aggregate_outcomes("process_tiff_pages", &request.paths.clone(), move || note_renamed(tools::tiff_combine(&request), request.output_path.as_deref()))
}
#[tauri::command]
async fn image_metadata(events: RunEvents, request: tools::MetadataRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "image_metadata", request.paths.clone(), &request.output_location, &request, tools::strip_metadata)
}
#[tauri::command]
fn inspect_image_metadata(request: tools::MetadataRequest) -> Vec<crate::kit::images::metadata::MetadataInspection> { request.paths.into_iter().map(crate::kit::images::metadata::inspect).collect() }

#[tauri::command]
fn inspect_image_preview(request: tools::ImagePreviewRequest) -> Result<tools::ImagePreview, String> { tools::inspect_preview(&request) }

#[tauri::command]
fn inspect_tiff_pages(request: tools::TiffPreviewRequest) -> Result<Vec<tools::ImagePreview>, String> { tools::inspect_tiff_pages(&request) }

#[tauri::command]
fn inspect_image_pages(request: tools::ImagePagesRequest) -> Vec<Option<usize>> { tools::inspect_image_pages(&request) }

#[tauri::command]
fn inspect_image_edit_preview(request: tools::ImageEditPreviewRequest) -> Result<tools::ImagePreview, String> { tools::inspect_edit_preview(&request) }

#[tauri::command]
async fn export_image_edit_plan(events: RunEvents, request: tools::ImageEditExportRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "export_image_edit_plan", request.paths.clone(), &request.plan.output_location, &request, |request, path| tools::export_edit_plan(&request.plan, path))
}

#[tauri::command]
async fn export_image_edit_stack(events: RunEvents, request: tools::ImageEditStackRequest) -> Vec<JobOutcome> {
    let request = Arc::new(request);
    BatchRunner::run(&events, "export_image_edit_stack", request.paths.clone(), &request.output_location, &request, tools::export_edit_stack)
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

/// One output from many inputs: success is one outcome, failure is one outcome per entry so every row explains itself.
/// Inputs are checked in list order, so the first entry with the failing path is the one that stopped the run.
fn combine_aggregate(paths: &[std::path::PathBuf], aggregate: JobOutcome) -> Vec<JobOutcome> {
    let (output_paths, unavailable) = match &aggregate.state {
        OutcomeState::Succeeded { .. } => return vec![aggregate],
        OutcomeState::Failed { output_paths, error } => (output_paths.clone(), error.kind == crate::kit::contracts::ErrorKind::Unavailable),
        OutcomeState::Skipped { .. } => return paths.iter().cloned().map(|input_path| JobOutcome { input_path, ..aggregate.clone() }).collect(),
    };
    let failed_index = paths.iter().position(|path| *path == aggregate.input_path).unwrap_or(0);
    let failed_name = aggregate.input_path.file_name().map_or_else(|| aggregate.input_path.display().to_string(), |name| name.to_string_lossy().into_owned());
    let repeated = paths.iter().filter(|path| **path == aggregate.input_path).count() > 1;
    let cause = if repeated { format!("{failed_name} (entry {})", failed_index + 1) } else { failed_name };
    paths.iter().cloned().enumerate().map(|(index, input_path)| {
        let state = if index == failed_index { aggregate.state.clone() } else {
            OutcomeState::Failed {
                output_paths: output_paths.clone(),
                error: {
                    let message = format!("Stopped because {cause} failed. Nothing was saved.");
                    if unavailable { ToolError::unavailable(message) } else { ToolError::processing(message) }
                },
            }
        };
        JobOutcome { input_path, state, ..aggregate.clone() }
    }).collect()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct QuickActionSetting { id: &'static str, title: &'static str, enabled: bool }

fn quick_action_settings_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    app.path().app_config_dir().map(|dir| dir.join("quick-actions.json")).map_err(|e| e.to_string())
}

/// Debug builds skip registration so the menus never point at a throwaway dev binary.
fn register_quick_actions(enabled: &[quick_action::QuickAction]) -> Result<(), String> {
    if cfg!(debug_assertions) { return Ok(()); }
    quick_action::register(&std::env::current_exe().map_err(|e| e.to_string())?, enabled)
}

#[tauri::command]
fn get_quick_actions(app: tauri::AppHandle) -> Result<Vec<QuickActionSetting>, String> {
    let enabled = quick_action::load_enabled(&quick_action_settings_path(&app)?);
    Ok(quick_action::QuickAction::ALL.into_iter()
        .map(|action| QuickActionSetting { id: action.id(), title: action.title(), enabled: enabled.contains(&action) })
        .collect())
}

#[tauri::command]
async fn set_quick_actions(app: tauri::AppHandle, ids: Vec<String>) -> Result<(), String> {
    let enabled: Vec<_> = quick_action::QuickAction::ALL.into_iter().filter(|action| ids.iter().any(|id| id == action.id())).collect();
    quick_action::save_enabled(&quick_action_settings_path(&app)?, &enabled)?;
    register_quick_actions(&enabled)
}

fn run_quick_action(invocation: Result<quick_action::Invocation, String>) -> bool {
    match invocation {
        Ok(quick_action::Invocation { action, paths }) => {
            let outcomes = action.run(paths);
            quick_action::notify("Toolbox", &quick_action::summary(action, &outcomes));
            outcomes.iter().all(|outcome| outcome.error().is_none())
        }
        Err(error) => {
            quick_action::notify("Toolbox", &error);
            false
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(invocation) = quick_action::parse_args(&args) {
        std::process::exit(if run_quick_action(invocation) { 0 } else { 1 });
    }

    let builder = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            shell::shell_ready,
            shell::acknowledge_activation,
            shell::open_paths,
            shell::set_document_title,
            open_output_path,
            reveal_output_path,
            open_third_party_notices,
            open_sponsor_page,
            intake::inspect_intake_paths,
            intake::describe_intake_files,
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
            inspect_pdf_watermark_image,
            inspect_pdf_form,
            inspect_signature_image,
            preview_pdf_scene,
            preview_pdf_scene_pages,
            export_pdf_scene,
            crop_pdf,
            edit_pdf,
            edit_pdf_session,
            sign_pdf,
            add_page_numbers,
            watermark_pdf,
            compress_files,
            rotate_pdf,
            repair_pdf,
            inspect_pdf_privacy,
            read_pdf_properties,
            edit_pdf_properties,
            save_privacy_reports,
            redact_pdf,
            compare_pdfs,
            extract_pdf_pages
            ,merge_pdfs,
            split_pdf,
            pdf_to_images,
            extract_from_pdf,
            convert_to_pdf,
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
            inspect_image_pages,
            inspect_image_edit_preview,
            export_image_edit_plan,
            export_image_edit_stack,
            get_quick_actions,
            set_quick_actions,
            packs::list_packs
        ])
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            shell::focus(app);
            let paths = argv.into_iter().skip(1).collect::<Vec<_>>();
            if !paths.is_empty() {
                shell::dispatch(app, shell::parse_paths(paths));
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init());
    #[cfg(feature = "updater")]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    builder
        .setup(|app| {
            app.manage(shell::ShellState::default());
            shell::install_geometry(app)?;
            shell::menu(app)?;
            let quick_action_settings = quick_action_settings_path(app.handle())?;
            std::thread::spawn(move || {
                if let Err(error) = register_quick_actions(&quick_action::load_enabled(&quick_action_settings)) {
                    eprintln!("Quick actions could not be registered: {error}");
                }
            });
            let startup_paths = std::env::args().skip(1).collect::<Vec<_>>();
            if !startup_paths.is_empty() {
                shell::dispatch(app.handle(), shell::parse_paths(startup_paths));
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            shell::handle_window_event(window, event);
            if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                shell::handle_drop(window.app_handle(), paths.clone());
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| shell::handle_run_event(app, &event));
}

#[cfg(test)]
mod command_tests {
    // Drives the same command functions the frontend `invoke`s, with the same
    // argument shapes (Vec<String> paths, format strings), so the whole
    // IPC-facing layer is exercised without needing a live WebView.

    use super::*;
    use crate::kit::common::OutputLocation;
    use crate::kit::common::watermark::{WatermarkOptions, WatermarkPlacement, WatermarkPosition};
    use crate::kit::contracts::{ErrorKind, LimitKind};
    use crate::kit::images::tools::{CropRequest, CropStep, ImageEditStackRequest, ImageEditSteps, GifCreateRequest, GifExtractRequest, ImageEdit, ImageEditExportRequest, ImageEditPlan, ImageEditPreviewRequest, IconSetRequest, MetadataRequest, ResizeRequest, RotateRequest, TiffMode, TiffRequest, ToneRequest, WatermarkRequest};
    use crate::kit::office::{test_minimal_ooxml_package, OfficeProcessor};
    use crate::kit::pdf::editor::{CropPdfRequest, CropSpec, EditPdfRequest, PageScope, PdfEditMode, PdfEditOperation, PdfEditSessionPlan, PdfEditSessionRequest, PdfOverlay, PdfOverlayPosition, PdfRect, RangedPageScope, RedactPdfRequest, RotatePage, SignPdfRequest};
    use crate::kit::compress::CompressLevel;
    use crate::kit::pdf::remaining::{ImagesToPdfRequest, MergePdfRequest, PageOverlayRequest, PageSelectionRequest, PdfToImagesRequest, PdfWatermarkRequest, RepairPdfRequest, RotatePdfRequest, ComparePdfsRequest, ExtractFromPdfRequest, ExtractMode, SplitMode, SplitPdfRequest};
    use crate::kit::vision::VisionRequest;
    use base64::{engine::general_purpose::STANDARD, Engine};
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
        for output in outcome.output_paths() { assert!(output.is_file() || output.is_dir(), "{name} reported missing output {}", output.display()); }
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

    /// Merge makes one file but reports it once per entry, in list order, so each entry's row can carry its own notes.
    fn assert_merge_success<F>(name: &str, paths: &[PathBuf], future: F) -> Vec<PathBuf>
    where F: Future<Output = Vec<JobOutcome>> {
        let outcomes = tauri::async_runtime::block_on(future);
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths, "{name} reports one outcome per entry in list order");
        assert!(outcomes.iter().all(|outcome| outcome.error().is_none()), "{name} failed: {:?}", outcomes.iter().find_map(JobOutcome::error));
        let output = outcomes[0].output_paths().to_vec();
        assert_eq!(output.len(), 1, "{name} makes one file");
        assert!(output[0].is_file(), "{name} reported missing output {}", output[0].display());
        assert!(outcomes.iter().all(|outcome| outcome.output_paths() == output), "{name} reports the same merged file on every entry");
        output
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
            (merge.clone(), "statement-1.pdf", 2, tauri::async_runtime::block_on(run_merge_pdfs(&MergePdfRequest { paths: vec![pdf.clone(), pdf_two.clone()], output_location: OutputLocation::AlongsideInput, output_path: Some(merge.clone()) }))),
            (combined.clone(), "scans-1.pdf", 1, tauri::async_runtime::block_on(run_images_to_pdf(&ImagesToPdfRequest { layout: Default::default(), paths: vec![image.clone(), image_two.clone()], output_location: OutputLocation::AlongsideInput, output_path: Some(combined.clone()) }))),
            (gif.clone(), "loop-1.gif", 1, tauri::async_runtime::block_on(run_create_gif(&GifCreateRequest { paths: vec![image.clone(), image_two.clone()], frame_delay_ms: 100, loop_forever: true, output_location: OutputLocation::AlongsideInput, output_path: Some(gif.clone()) }))),
            (tiff_out.clone(), "pages-1.tiff", 1, tauri::async_runtime::block_on(run_combine_tiff_pages(&TiffRequest { mode: TiffMode::Combine, paths: vec![tiff.clone(), tiff_two.clone()], pages: None, output_location: OutputLocation::AlongsideInput, output_path: Some(tiff_out.clone()) }))),
        ];
        for (existing, expected_name, entries, outcomes) in runs {
            let chosen_name = existing.file_name().unwrap().to_string_lossy().into_owned();
            assert_eq!(outcomes.len(), entries, "{chosen_name} makes one file; only merge reports it once per entry");
            assert!(outcomes[1..].iter().all(|entry| entry.warnings.is_empty() && entry.output_paths() == [chosen_folder.join(expected_name)]), "the rename is noted once, on the first entry");
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
    fn images_to_pdf_writes_the_chosen_path_in_the_requested_order_and_numbers_around_existing_files() {
        use crate::kit::contracts::WarningKind;
        let root = sandbox("images-to-pdf-chosen-path");
        let narrow = root.join("a-narrow.png"); write_png(&narrow, 16, 10);
        let wide = root.join("b-wide.png"); write_png(&wide, 48, 20);
        let middle = root.join("c-middle.png"); write_png(&middle, 32, 30);
        let paths = vec![wide.clone(), narrow.clone(), middle.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let chosen_folder = root.join("elsewhere");
        std::fs::create_dir_all(&chosen_folder).unwrap();
        let run = |output_path: &PathBuf| tauri::async_runtime::block_on(run_images_to_pdf(&ImagesToPdfRequest {
            layout: Default::default(), paths: paths.clone(), output_location: OutputLocation::AlongsideInput, output_path: Some(output_path.clone()),
        }));

        let fresh = chosen_folder.join("b-wide-combined.pdf");
        let outcomes = run(&fresh);
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].error().is_none(), "{:?}", outcomes[0].error());
        assert_eq!(outcomes[0].output_paths(), [fresh.clone()]);
        assert!(outcomes[0].warnings.is_empty());
        let widths = pdf_images(&fresh).iter().map(|image| image.media_width).collect::<Vec<_>>();
        assert_eq!(widths.len(), 3);
        assert!(widths[0] > widths[2] && widths[2] > widths[1], "pages follow the request order: {widths:?}");
        assert!(std::fs::read_dir(&root).unwrap().all(|entry| entry.unwrap().path().extension().map_or(true, |extension| extension != "pdf")), "nothing is written next to the inputs");

        let taken = chosen_folder.join("scans.pdf");
        std::fs::write(&taken, b"existing scans.pdf").unwrap();
        let taken_once = chosen_folder.join("scans-1.pdf");
        std::fs::write(&taken_once, b"existing scans-1.pdf").unwrap();
        let first_bytes = std::fs::read(&fresh).unwrap();
        let outcomes = run(&taken);
        assert_eq!(outcomes[0].output_paths(), [chosen_folder.join("scans-2.pdf")]);
        assert_eq!(outcomes[0].warnings.iter().map(|warning| warning.kind.clone()).collect::<Vec<_>>(), [WarningKind::RenamedToAvoidReplacing { chosen_name: "scans.pdf".into(), final_name: "scans-2.pdf".into() }]);
        assert_eq!(pdf_images(&chosen_folder.join("scans-2.pdf")).len(), 3);
        assert_eq!(std::fs::read(&taken).unwrap(), b"existing scans.pdf");
        assert_eq!(std::fs::read(&taken_once).unwrap(), b"existing scans-1.pdf");
        assert_eq!(std::fs::read(&fresh).unwrap(), first_bytes);
        for (path, bytes) in paths.iter().zip(originals) { assert_eq!(std::fs::read(path).unwrap(), bytes, "{} must not change", path.display()); }
        let _ = std::fs::remove_dir_all(root);
    }

    // The bundled x265 only encodes 8-bit HEVC, so a 10-bit HEIC is made by
    // rewriting the encoded parameter sets: the hvcC record and the SPS both
    // declare 10-bit samples. The SPS keeps its length by also shortening the
    // picture-order-count field, which a single intra frame never reads.
    fn ten_bit_heic(image: &image::DynamicImage) -> Vec<u8> {
        struct Reader<'a> { data: &'a [u8], at: usize }
        impl Reader<'_> {
            fn bit(&mut self) -> u32 { let bit = (self.data[self.at / 8] >> (7 - self.at % 8)) & 1; self.at += 1; u32::from(bit) }
            fn bits(&mut self, count: usize) -> u32 { (0..count).fold(0, |value, _| (value << 1) | self.bit()) }
            fn ue(&mut self) -> u32 { let mut zeros = 0; while self.bit() == 0 { zeros += 1; } ((1 << zeros) - 1) + self.bits(zeros) }
        }
        #[derive(Default)]
        struct Writer { out: Vec<u8>, written: usize }
        impl Writer {
            fn bit(&mut self, bit: u32) {
                if self.written % 8 == 0 { self.out.push(0); }
                if bit & 1 == 1 { *self.out.last_mut().unwrap() |= 1 << (7 - self.written % 8); }
                self.written += 1;
            }
            fn bits(&mut self, value: u32, count: usize) { for shift in (0..count).rev() { self.bit(value >> shift); } }
            fn ue(&mut self, value: u32) { let coded = value + 1; let len = 32 - coded.leading_zeros() as usize; self.bits(0, len - 1); self.bits(coded, len); }
        }
        fn unescape(data: &[u8]) -> Vec<u8> {
            let (mut out, mut zeros) = (Vec::new(), 0);
            for &byte in data { if zeros >= 2 && byte == 3 { zeros = 0; continue; } zeros = if byte == 0 { zeros + 1 } else { 0 }; out.push(byte); }
            out
        }
        fn escape(data: &[u8]) -> Vec<u8> {
            let (mut out, mut zeros) = (Vec::new(), 0);
            for &byte in data { if zeros >= 2 && byte <= 3 { out.push(3); zeros = 0; } zeros = if byte == 0 { zeros + 1 } else { 0 }; out.push(byte); }
            out
        }
        fn ten_bit_sps(nal: &[u8]) -> Vec<u8> {
            let rbsp = unescape(&nal[2..]);
            let last = rbsp.iter().rposition(|&byte| byte != 0).unwrap();
            let stop_bit = last * 8 + 7 - rbsp[last].trailing_zeros() as usize;
            let (mut reader, mut writer) = (Reader { data: &rbsp, at: 0 }, Writer::default());
            let head = reader.bits(8);
            assert_eq!((head >> 1) & 7, 0, "fixture expects a single temporal layer");
            writer.bits(head, 8);
            for _ in 0..96 { let bit = reader.bit(); writer.bit(bit); }
            let id = reader.ue(); writer.ue(id);
            let chroma = reader.ue(); writer.ue(chroma);
            if chroma == 3 { let bit = reader.bit(); writer.bit(bit); }
            for _ in 0..2 { let size = reader.ue(); writer.ue(size); }
            let window = reader.bit(); writer.bit(window);
            if window == 1 { for _ in 0..4 { let offset = reader.ue(); writer.ue(offset); } }
            let (luma, chroma_depth, poc) = (reader.ue(), reader.ue(), reader.ue());
            assert_eq!((luma, chroma_depth, poc), (0, 0, 4), "fixture expects 8-bit x265 parameters");
            writer.ue(2); writer.ue(2); writer.ue(0);
            while reader.at < stop_bit { let bit = reader.bit(); writer.bit(bit); }
            writer.bit(1);
            while writer.written % 8 != 0 { writer.bit(0); }
            [&nal[..2], escape(&writer.out).as_slice()].concat()
        }
        let mut bytes = heif::encode(image).unwrap();
        let name = bytes.windows(4).position(|window| window == b"hvcC").unwrap();
        let size = u32::from_be_bytes(bytes[name - 4..name].try_into().unwrap()) as usize;
        let body = bytes[name + 4..name - 4 + size].to_vec();
        let mut patched = body[..23].to_vec();
        patched[17] = (patched[17] & 0xF8) | 2;
        patched[18] = (patched[18] & 0xF8) | 2;
        let mut at = 23;
        for _ in 0..body[22] {
            let kind = body[at] & 0x3F;
            let count = u16::from_be_bytes([body[at + 1], body[at + 2]]);
            patched.extend_from_slice(&body[at..at + 3]);
            at += 3;
            for _ in 0..count {
                let len = usize::from(u16::from_be_bytes([body[at], body[at + 1]]));
                let nal = &body[at + 2..at + 2 + len];
                at += 2 + len;
                let nal = if kind == 33 { ten_bit_sps(nal) } else { nal.to_vec() };
                patched.extend_from_slice(&(nal.len() as u16).to_be_bytes());
                patched.extend_from_slice(&nal);
            }
        }
        assert_eq!(patched.len(), body.len(), "the patched hvcC record must keep its size");
        bytes[name + 4..name - 4 + size].copy_from_slice(&patched);
        assert_eq!(heif::probe(&bytes).unwrap().bit_depth, heif::BitDepth::Ten);
        bytes
    }

    struct PdfImage { dict: lopdf::Dictionary, content: Vec<u8>, mask: Option<Vec<u8>>, media_width: f32 }

    fn pdf_images(path: &PathBuf) -> Vec<PdfImage> {
        let document = lopdf::Document::load(path).unwrap();
        let inflate = |stream: &lopdf::Stream| {
            assert_eq!(stream.dict.get(b"Filter").unwrap().as_name().unwrap(), b"FlateDecode");
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(stream.content.as_slice()), &mut out).unwrap();
            out
        };
        document.get_pages().values().map(|page_id| {
            let page = document.get_dictionary(*page_id).unwrap();
            let media_width = page.get(b"MediaBox").unwrap().as_array().unwrap()[2].as_float().unwrap();
            let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
            let image_id = resources.get(b"XObject").unwrap().as_dict().unwrap().get(b"Im0").unwrap().as_reference().unwrap();
            let stream = document.get_object(image_id).unwrap().as_stream().unwrap();
            let content = if stream.dict.get(b"Filter").unwrap().as_name().unwrap() == b"DCTDecode" { stream.content.clone() } else { inflate(stream) };
            let mask = stream.dict.get(b"SMask").ok().map(|mask| inflate(document.get_object(mask.as_reference().unwrap()).unwrap().as_stream().unwrap()));
            PdfImage { dict: stream.dict.clone(), content, mask, media_width }
        }).collect()
    }

    fn filter_of(image: &PdfImage) -> &[u8] { image.dict.get(b"Filter").unwrap().as_name().unwrap() }

    #[test]
    fn images_to_pdf_embeds_jpeg_as_is_and_stores_every_other_image_without_loss() {
        use image::{DynamicImage, Rgb, RgbImage, RgbaImage};
        let root = sandbox("images-to-pdf-embed");
        let rgb = |width: u32, seed: u8| RgbImage::from_fn(width, 12, |x, y| Rgb([seed, (x * 9) as u8, (y * 17) as u8]));
        let jpeg = root.join("photo.jpg");
        let mut jpeg_bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, 80).encode_image(&rgb(20, 10)).unwrap();
        std::fs::write(&jpeg, &jpeg_bytes).unwrap();
        let png = root.join("chart.png"); rgb(21, 40).save(&png).unwrap();
        let clear = root.join("logo.png");
        let clear_pixels = RgbaImage::from_fn(22, 12, |x, y| Rgba([200, x as u8, y as u8, (x * 11) as u8]));
        clear_pixels.save(&clear).unwrap();
        let webp = root.join("still.webp"); rgb(23, 70).save(&webp).unwrap();
        let heic = root.join("camera.heic"); std::fs::write(&heic, heif::encode(&DynamicImage::ImageRgb8(rgb(24, 90))).unwrap()).unwrap();
        let heif_file = root.join("camera.heif"); std::fs::write(&heif_file, heif::encode(&DynamicImage::ImageRgb8(rgb(25, 120))).unwrap()).unwrap();
        let paths = vec![jpeg.clone(), png.clone(), clear.clone(), webp.clone(), heic.clone(), heif_file.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let outcomes = tauri::async_runtime::block_on(run_images_to_pdf(&ImagesToPdfRequest { layout: Default::default(),
            paths: paths.clone(), output_location: location(&root, "out"), output_path: None,
        }));

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].error().is_none(), "{:?}", outcomes[0].error());
        assert!(outcomes[0].warnings.is_empty(), "8-bit sources need no precision note: {:?}", outcomes[0].warnings);
        let images = pdf_images(&outcomes[0].output_paths()[0]);
        assert_eq!(images.iter().map(|image| image.media_width).collect::<Vec<_>>(), [20.0, 21.0, 22.0, 23.0, 24.0, 25.0], "one page per image, in the requested order");
        assert_eq!(filter_of(&images[0]), b"DCTDecode");
        assert_eq!(images[0].content, jpeg_bytes, "the JPEG is embedded byte for byte");
        assert_eq!(images[1].content, image::open(&png).unwrap().to_rgb8().into_raw(), "the PNG decodes back to the same pixels");
        assert!(images[1].mask.is_none(), "an opaque PNG needs no soft mask");
        assert_eq!(images[2].content, image::open(&clear).unwrap().to_rgb8().into_raw());
        assert_eq!(images[2].mask.as_deref(), Some(clear_pixels.pixels().map(|pixel| pixel[3]).collect::<Vec<_>>().as_slice()), "a transparent PNG keeps its alpha as a soft mask");
        assert_eq!(images[3].content, image::open(&webp).unwrap().to_rgb8().into_raw(), "the WebP is stored without loss");
        for (image, source) in images[4..].iter().zip([&heic, &heif_file]) {
            assert_eq!(image.content, heif::decode(&std::fs::read(source).unwrap()).unwrap().to_rgb8().into_raw(), "{} is decoded in-process and stored without loss", source.display());
        }
        for image in &images[1..] {
            assert_eq!(filter_of(image), b"FlateDecode");
            assert_eq!(image.dict.get(b"BitsPerComponent").unwrap().as_i64().unwrap(), 8);
        }
        for (path, original) in paths.iter().zip(&originals) { assert_eq!(&std::fs::read(path).unwrap(), original, "{} must stay unchanged", path.display()); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn images_to_pdf_reduces_high_bit_depth_heic_to_eight_bit_with_a_note() {
        use crate::kit::contracts::{OutcomeWarning, WarningKind};
        use image::{DynamicImage, Rgb, RgbImage};
        let root = sandbox("images-to-pdf-ten-bit");
        let png = root.join("cover.png"); write_png(&png, 16, 30);
        let sample = DynamicImage::ImageRgb8(RgbImage::from_fn(32, 24, |x, y| Rgb([(x * 8) as u8, (y * 10) as u8, 128])));
        let deep = root.join("deep.heic"); std::fs::write(&deep, ten_bit_heic(&sample)).unwrap();
        let deeper = root.join("deep-two.heif"); std::fs::write(&deeper, ten_bit_heic(&sample)).unwrap();
        let paths = vec![png.clone(), deep.clone(), deeper.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let outcomes = tauri::async_runtime::block_on(run_images_to_pdf(&ImagesToPdfRequest { layout: Default::default(),
            paths: paths.clone(), output_location: location(&root, "out"), output_path: None,
        }));

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].error().is_none(), "{:?}", outcomes[0].error());
        assert_eq!(outcomes[0].warnings, [OutcomeWarning {
            kind: WarningKind::PrecisionReduced,
            message: "Reduced to 8-bit sRGB to fit in the PDF: page 2 (10-bit), page 3 (10-bit).".into(),
        }], "the combined row carries one note naming every reduced page");
        let images = pdf_images(&outcomes[0].output_paths()[0]);
        assert_eq!(images.len(), 3);
        for image in &images[1..] {
            assert_eq!(image.dict.get(b"BitsPerComponent").unwrap().as_i64().unwrap(), 8);
            assert_eq!(image.content.len(), 32 * 24 * 3, "the 10-bit page is stored as 8-bit RGB");
        }
        for (path, original) in paths.iter().zip(&originals) { assert_eq!(&std::fs::read(path).unwrap(), original); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn images_to_pdf_fails_the_combine_and_publishes_nothing_when_one_image_is_unreadable() {
        let root = sandbox("images-to-pdf-unreadable");
        let first = root.join("first.png"); write_png(&first, 16, 30);
        let broken = root.join("broken.heic"); std::fs::write(&broken, b"not really a HEIC file").unwrap();
        let last = root.join("last.png"); write_png(&last, 16, 90);
        let paths = vec![first.clone(), broken.clone(), last.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let out = location(&root, "out");
        let OutputLocation::CustomFolder(folder) = &out else { unreachable!() };
        let folder = folder.clone();

        assert_aggregate_failure("images to pdf unreadable", &paths, &broken, run_images_to_pdf(&ImagesToPdfRequest { layout: Default::default(),
            paths: paths.clone(), output_location: out, output_path: None,
        }));

        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0, "no PDF and no partial file is left behind");
        for (path, original) in paths.iter().zip(&originals) { assert_eq!(&std::fs::read(path).unwrap(), original); }
        let _ = std::fs::remove_dir_all(root);
    }

    type RawTiffPage = (u32, u32, Vec<(u16, Vec<u32>)>, Vec<u8>);

    fn raw_tiff(pages: &[RawTiffPage]) -> Vec<u8> {
        const SHORT_TAGS: [u16; 7] = [258, 259, 262, 266, 277, 284, 530];
        let mut out = vec![b'I', b'I', 42, 0, 0, 0, 0, 0];
        let mut link = 4;
        for (width, height, tags, strip) in pages {
            let strip_offset = out.len() as u32;
            out.extend_from_slice(strip);
            if out.len() % 2 == 1 { out.push(0); }
            let mut entries = tags.clone();
            entries.extend([(256, vec![*width]), (257, vec![*height]), (273, vec![strip_offset]), (278, vec![*height]), (279, vec![strip.len() as u32])]);
            entries.sort_by_key(|(tag, _)| *tag);
            let ifd = out.len();
            out[link..link + 4].copy_from_slice(&(ifd as u32).to_le_bytes());
            let tail_start = ifd + 2 + entries.len() * 12 + 4;
            let mut tail = Vec::new();
            out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
            for (tag, values) in &entries {
                let short = SHORT_TAGS.contains(tag);
                let mut bytes = values.iter().flat_map(|value| if short { (*value as u16).to_le_bytes().to_vec() } else { value.to_le_bytes().to_vec() }).collect::<Vec<_>>();
                out.extend_from_slice(&tag.to_le_bytes());
                out.extend_from_slice(&(if short { 3u16 } else { 4 }).to_le_bytes());
                out.extend_from_slice(&(values.len() as u32).to_le_bytes());
                if bytes.len() <= 4 {
                    bytes.resize(4, 0);
                    out.extend_from_slice(&bytes);
                } else {
                    out.extend_from_slice(&((tail_start + tail.len()) as u32).to_le_bytes());
                    tail.extend_from_slice(&bytes);
                }
            }
            link = out.len();
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&tail);
        }
        out
    }

    fn gray_tiff_page(width: u32, height: u32, compression: u32, pixels: Vec<u8>) -> RawTiffPage {
        (width, height, vec![(258, vec![8]), (259, vec![compression]), (262, vec![1]), (277, vec![1])], pixels)
    }

    #[test]
    fn images_to_pdf_adds_every_tiff_page_in_order_at_the_tiff_position() {
        use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage, RgbaImage};
        let root = sandbox("images-to-pdf-tiff-pages");
        let first = root.join("first.png"); write_png(&first, 16, 30);
        let gray = GrayImage::from_fn(5, 2, |x, y| Luma([(x * 40 + y * 7) as u8]));
        let rgb = RgbImage::from_fn(4, 3, |x, y| Rgb([(x * 50) as u8, (y * 60) as u8, 77]));
        let rgba = RgbaImage::from_fn(3, 2, |x, y| Rgba([10, (x * 70) as u8, (y * 90) as u8, (x * 80 + 20) as u8]));
        let scan = root.join("scan.tiff");
        write_tiff(&scan, &[DynamicImage::ImageLuma8(gray.clone()), DynamicImage::ImageRgb8(rgb.clone()), DynamicImage::ImageRgba8(rgba.clone())]);
        let last = root.join("last.png"); write_png(&last, 18, 90);
        let paths = vec![first.clone(), scan.clone(), last.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let output = assert_aggregate_success("images to pdf tiff pages", &paths, run_images_to_pdf(&ImagesToPdfRequest {
            paths: paths.clone(), output_location: location(&root, "out"), output_path: None, layout: Default::default(),
        }));

        let images = pdf_images(&output[0]);
        assert_eq!(images.iter().map(|image| image.media_width).collect::<Vec<_>>(), [16.0, 5.0, 4.0, 3.0, 18.0], "each TIFF page becomes one PDF page at the TIFF's place in the list");
        assert_eq!(images[1].content, gray.into_raw());
        assert_eq!(images[1].dict.get(b"ColorSpace").unwrap().as_name().unwrap(), b"DeviceGray");
        assert_eq!(images[2].content, rgb.into_raw());
        assert!(images[2].mask.is_none());
        assert_eq!(images[3].content, DynamicImage::ImageRgba8(rgba.clone()).to_rgb8().into_raw());
        assert_eq!(images[3].mask.as_deref(), Some(rgba.pixels().map(|pixel| pixel[3]).collect::<Vec<_>>().as_slice()), "an alpha TIFF page keeps its soft mask");
        for image in &images[1..4] { assert_eq!(filter_of(image), b"FlateDecode"); }
        for (path, original) in paths.iter().zip(&originals) { assert_eq!(&std::fs::read(path).unwrap(), original); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn images_to_pdf_decodes_group_four_fax_tiff_pixel_for_pixel() {
        use fax::{encoder::Encoder, Color, VecWriter};
        let root = sandbox("images-to-pdf-g4");
        let (width, height) = (37u32, 9u32);
        let ink = |x: u32, y: u32| (x * 3 + y * 5) % 7 < 3 || x == y;
        let mut encoder = Encoder::new(VecWriter::new());
        for y in 0..height {
            encoder.encode_line((0..width).map(|x| if ink(x, y) { Color::Black } else { Color::White }), width as u16).unwrap();
        }
        let fax = root.join("fax.tif");
        std::fs::write(&fax, fax::tiff::wrap(&encoder.finish().unwrap().finish(), width, height)).unwrap();

        let output = assert_aggregate_success("images to pdf g4", &[fax.clone()], run_images_to_pdf(&ImagesToPdfRequest {
            paths: vec![fax.clone()], output_location: location(&root, "out"), output_path: None, layout: Default::default(),
        }));

        let images = pdf_images(&output[0]);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].dict.get(b"ColorSpace").unwrap().as_name().unwrap(), b"DeviceGray");
        let expected = (0..height).flat_map(|y| (0..width).map(move |x| if ink(x, y) { 0 } else { 255 })).collect::<Vec<u8>>();
        assert_eq!(images[0].content, expected, "black fax pixels stay black and white stay white");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn images_to_pdf_decodes_one_dimensional_group_three_fax_tiff_pixel_for_pixel() {
        const WHITE: [&str; 9] = ["00110101", "000111", "0111", "1000", "1011", "1100", "1110", "1111", "10011"];
        const BLACK: [&str; 9] = ["0000110111", "010", "11", "10", "011", "0011", "0010", "00011", "000101"];
        let root = sandbox("images-to-pdf-g3");
        let rows: [&[usize]; 5] = [&[8], &[2, 3, 3], &[0, 8], &[1, 1, 1, 1, 1, 1, 1, 1], &[5, 2, 1]];
        let mut bits = String::new();
        let mut white_is_one = Vec::new();
        for runs in rows {
            bits.push_str("000000000001");
            for (index, run) in runs.iter().enumerate() {
                bits.push_str(if index % 2 == 0 { WHITE[*run] } else { BLACK[*run] });
                white_is_one.extend(std::iter::repeat_n(index % 2 == 0, *run));
            }
        }
        while bits.len() % 8 != 0 { bits.push('0'); }
        let data = bits.as_bytes().chunks(8).map(|byte| u8::from_str_radix(std::str::from_utf8(byte).unwrap(), 2).unwrap()).collect::<Vec<_>>();
        let page = |photometric: u32, fill_order: u32, strip: Vec<u8>| -> RawTiffPage {
            (8, rows.len() as u32, vec![(258, vec![1]), (259, vec![3]), (262, vec![photometric]), (266, vec![fill_order]), (277, vec![1]), (292, vec![0])], strip)
        };
        let fax = root.join("fax.tiff");
        std::fs::write(&fax, raw_tiff(&[page(0, 1, data.clone()), page(1, 2, data.iter().map(|byte| byte.reverse_bits()).collect())])).unwrap();

        let output = assert_aggregate_success("images to pdf g3", &[fax.clone()], run_images_to_pdf(&ImagesToPdfRequest {
            paths: vec![fax.clone()], output_location: location(&root, "out"), output_path: None, layout: Default::default(),
        }));

        let images = pdf_images(&output[0]);
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].content, white_is_one.iter().map(|white| if *white { 255 } else { 0 }).collect::<Vec<u8>>(), "WhiteIsZero Group 3 runs decode to their pixels");
        assert_eq!(images[1].content, white_is_one.iter().map(|white| if *white { 0 } else { 255 }).collect::<Vec<u8>>(), "BlackIsZero and reversed bit order are honoured");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn images_to_pdf_decodes_jpeg_compressed_tiff() {
        use image::{Rgb, RgbImage};
        let root = sandbox("images-to-pdf-jpeg-tiff");
        let source = RgbImage::from_fn(16, 8, |x, y| Rgb([(x * 15) as u8, (y * 30) as u8, 160]));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95).encode_image(&source).unwrap();
        let expected = image::load_from_memory(&jpeg).unwrap().to_rgb8().into_raw();
        let tiff = root.join("photo.tif");
        std::fs::write(&tiff, raw_tiff(&[(16, 8, vec![(258, vec![8, 8, 8]), (259, vec![7]), (262, vec![6]), (277, vec![3]), (530, vec![1, 1])], jpeg)])).unwrap();

        let output = assert_aggregate_success("images to pdf jpeg tiff", &[tiff.clone()], run_images_to_pdf(&ImagesToPdfRequest {
            paths: vec![tiff.clone()], output_location: location(&root, "out"), output_path: None, layout: Default::default(),
        }));

        let images = pdf_images(&output[0]);
        assert_eq!(images.len(), 1);
        assert_eq!(filter_of(&images[0]), b"FlateDecode");
        assert_eq!(images[0].content.len(), expected.len());
        let worst = images[0].content.iter().zip(&expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 3, "the JPEG-in-TIFF page matches the decoded JPEG, worst channel difference {worst}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn images_to_pdf_names_the_tiff_page_it_cannot_read_and_publishes_nothing() {
        let root = sandbox("images-to-pdf-tiff-bad-page");
        let first = root.join("first.png"); write_png(&first, 16, 30);
        let scan = root.join("scan.tiff");
        std::fs::write(&scan, raw_tiff(&[gray_tiff_page(2, 2, 1, vec![1, 2, 3, 4]), gray_tiff_page(2, 2, 32809, vec![5, 6, 7, 8]), gray_tiff_page(2, 2, 1, vec![9, 10, 11, 12])])).unwrap();
        let paths = vec![first.clone(), scan.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let out = location(&root, "out");
        let OutputLocation::CustomFolder(folder) = &out else { unreachable!() };
        let folder = folder.clone();

        let outcomes = tauri::async_runtime::block_on(run_images_to_pdf(&ImagesToPdfRequest { paths: paths.clone(), output_location: out.clone(), output_path: None, layout: Default::default() }));
        let failed = outcomes.iter().find(|outcome| outcome.input_path == scan).unwrap();
        let message = &failed.error().unwrap().message;
        assert!(message.contains(&scan.display().to_string()) && message.contains("TIFF page 2 of 3 can't be read"), "{message}");
        assert_aggregate_failure("images to pdf tiff bad page", &paths, &scan, run_images_to_pdf(&ImagesToPdfRequest { paths: paths.clone(), output_location: out, output_path: None, layout: Default::default() }));

        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0, "no PDF and no partial file is left behind");
        for (path, original) in paths.iter().zip(&originals) { assert_eq!(&std::fs::read(path).unwrap(), original); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn inspect_image_pages_counts_tiff_pages_and_one_page_for_other_images() {
        use crate::kit::images::tools::ImagePagesRequest;
        let root = sandbox("inspect-image-pages");
        let png = root.join("photo.png"); write_png(&png, 8, 30);
        let scan = root.join("scan.tiff");
        std::fs::write(&scan, raw_tiff(&[gray_tiff_page(1, 1, 1, vec![0]), gray_tiff_page(1, 1, 1, vec![1]), gray_tiff_page(1, 1, 1, vec![2])])).unwrap();
        let missing = root.join("missing.tif");

        assert_eq!(inspect_image_pages(ImagePagesRequest { paths: vec![png, scan, missing] }), [Some(1), Some(3), None]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn images_to_pdf_stops_at_the_recorded_tiff_page_limit() {
        use crate::kit::common::limits::MAX_IMAGE_PAGES;
        let root = sandbox("images-to-pdf-tiff-limit");
        let first = root.join("first.png"); write_png(&first, 16, 30);
        let long = root.join("long.tiff");
        std::fs::write(&long, raw_tiff(&(0..=MAX_IMAGE_PAGES).map(|page| gray_tiff_page(1, 1, 1, vec![page as u8])).collect::<Vec<_>>())).unwrap();
        let paths = vec![first.clone(), long.clone()];
        let out = location(&root, "out");
        let OutputLocation::CustomFolder(folder) = &out else { unreachable!() };
        let folder = folder.clone();

        let outcomes = tauri::async_runtime::block_on(run_images_to_pdf(&ImagesToPdfRequest { paths: paths.clone(), output_location: out, output_path: None, layout: Default::default() }));

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths);
        let error = outcomes[1].error().expect("the TIFF over the page limit must fail");
        assert_eq!(error.kind, ErrorKind::LimitExceeded { limit: LimitKind::PageCount }, "{}", error.message);
        assert!(error.message.contains("1,000"), "{}", error.message);
        assert!(outcomes[0].error().is_some_and(|error| error.message.contains("Stopped because long.tiff failed")), "the other image names the TIFF that stopped the combine");
        assert!(outcomes.iter().all(|outcome| outcome.output_paths().is_empty()));
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    // What one Images to PDF page shows: its media box, the clip rectangle a
    // fill draws through (if any), the `cm` matrix and the rectangle it draws
    // the image into, and the embedded image stream as stored.
    #[derive(Debug)]
    struct LaidOutPage { media: Vec<f32>, clip: Option<[f32; 4]>, matrix: [f32; 6], drawn: [f32; 4], image_dict: lopdf::Dictionary, image_bytes: Vec<u8> }

    fn laid_out_pages(path: &PathBuf) -> Vec<LaidOutPage> {
        use lopdf::content::Content;
        let document = lopdf::Document::load(path).unwrap();
        let number = |object: &lopdf::Object| object.as_float().or_else(|_| object.as_i64().map(|value| value as f32)).unwrap();
        document.get_pages().values().map(|page_id| {
            let page = document.get_dictionary(*page_id).unwrap();
            let media = page.get(b"MediaBox").unwrap().as_array().unwrap().iter().map(number).collect::<Vec<_>>();
            let operations = Content::decode(&document.get_page_content(*page_id)).unwrap().operations;
            let operands = |operator: &str| operations.iter().find(|operation| operation.operator == operator).map(|operation| operation.operands.iter().map(number).collect::<Vec<_>>());
            let clip = operands("re").map(|values| [values[0], values[1], values[2], values[3]]);
            if clip.is_some() { assert!(operations.iter().any(|operation| operation.operator == "W"), "a fill clips to its rectangle"); }
            let cm = operands("cm").expect("every page places its image with cm");
            let matrix = [cm[0], cm[1], cm[2], cm[3], cm[4], cm[5]];
            let corners = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].map(|(u, v)| (cm[0] * u + cm[2] * v + cm[4], cm[1] * u + cm[3] * v + cm[5]));
            let (left, bottom) = corners.iter().fold((f32::MAX, f32::MAX), |(x, y), corner| (x.min(corner.0), y.min(corner.1)));
            let (right, top) = corners.iter().fold((f32::MIN, f32::MIN), |(x, y), corner| (x.max(corner.0), y.max(corner.1)));
            let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
            let image_id = resources.get(b"XObject").unwrap().as_dict().unwrap().get(b"Im0").unwrap().as_reference().unwrap();
            let stream = document.get_object(image_id).unwrap().as_stream().unwrap();
            LaidOutPage { media, clip, matrix, drawn: [left, bottom, right - left, top - bottom], image_dict: stream.dict.clone(), image_bytes: stream.content.clone() }
        }).collect()
    }

    fn assert_close(actual: &[f32], expected: &[f32], context: &str) {
        assert_eq!(actual.len(), expected.len(), "{context}: {actual:?} vs {expected:?}");
        for (actual_value, expected_value) in actual.iter().zip(expected) {
            assert!((actual_value - expected_value).abs() < 0.01, "{context}: {actual:?} should be {expected:?}");
        }
    }

    // A wide JPEG (40 × 20), a tall PNG (20 × 30) and a square PNG (24 × 24),
    // generated here so every orientation rule has a page to act on.
    fn layout_fixtures(root: &PathBuf) -> (Vec<PathBuf>, Vec<u8>) {
        use image::{Rgb, RgbImage};
        let wide = root.join("wide.jpg");
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85).encode_image(&RgbImage::from_fn(40, 20, |x, y| Rgb([(x * 6) as u8, (y * 12) as u8, 90]))).unwrap();
        std::fs::write(&wide, &jpeg).unwrap();
        let tall = root.join("tall.png");
        RgbImage::from_fn(20, 30, |x, y| Rgb([30, (x * 12) as u8, (y * 8) as u8])).save(&tall).unwrap();
        let square = root.join("square.png"); write_png(&square, 24, 140);
        (vec![wide, tall, square], jpeg)
    }

    fn combine_with(root: &PathBuf, name: &str, paths: &[PathBuf], layout: serde_json::Value) -> Vec<LaidOutPage> {
        let mut request = serde_json::json!({ "paths": paths, "outputLocation": location(root, name) });
        request.as_object_mut().unwrap().extend(layout.as_object().unwrap().clone());
        let request = serde_json::from_value::<ImagesToPdfRequest>(request).expect("PDF Conversion view payload for images_to_pdf");
        let outcomes = tauri::async_runtime::block_on(run_images_to_pdf(&request));
        assert_eq!(outcomes.len(), 1, "{name}");
        assert!(outcomes[0].error().is_none(), "{name}: {:?}", outcomes[0].error());
        laid_out_pages(&outcomes[0].output_paths()[0])
    }

    #[test]
    fn images_to_pdf_match_image_makes_each_page_the_size_of_its_image() {
        let root = sandbox("images-to-pdf-layout-match");
        let (paths, _) = layout_fixtures(&root);
        for (name, layout) in [("default", serde_json::json!({})), ("explicit", serde_json::json!({ "pageSize": "matchImage" }))] {
            let pages = combine_with(&root, name, &paths, layout);
            for (page, (width, height)) in pages.iter().zip([(40.0, 20.0), (20.0, 30.0), (24.0, 24.0)]) {
                assert_close(&page.media, &[0.0, 0.0, width, height], name);
                assert_close(&page.drawn, &[0.0, 0.0, width, height], name);
                assert!(page.clip.is_none(), "{name}: a matched page draws the whole image with no clip");
            }
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn images_to_pdf_lays_out_a4_and_letter_for_every_margin_fit_and_orientation() {
        let root = sandbox("images-to-pdf-layout-paper");
        let (paths, jpeg) = layout_fixtures(&root);
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let matched = combine_with(&root, "matched", &paths, serde_json::json!({}));
        let sources = [(40.0_f32, 20.0_f32), (20.0, 30.0), (24.0, 24.0)];
        // Recorded paper sizes (portrait, points) and margins (points on every side).
        let sizes = [("a4", 595.2756_f32, 841.8898_f32), ("letter", 612.0, 792.0)];
        let margins = [("none", 0.0_f32), ("small", 36.0), ("large", 72.0)];
        let mut combinations = 0;
        for (size, short, long) in sizes {
            for (margin, inset) in margins {
                for fit in ["fit", "fill"] {
                    for orientation in ["automatic", "portrait", "landscape"] {
                        let name = format!("{size}-{margin}-{fit}-{orientation}");
                        let pages = combine_with(&root, &name, &paths, serde_json::json!({ "pageSize": size, "margin": margin, "fit": fit, "orientation": orientation }));
                        assert_eq!(pages.len(), 3, "{name}: one page per image");
                        for ((page, (width, height)), plain) in pages.iter().zip(sources).zip(&matched) {
                            let landscape = match orientation { "automatic" => width > height, "landscape" => true, _ => false };
                            let (page_width, page_height) = if landscape { (long, short) } else { (short, long) };
                            assert_close(&page.media, &[0.0, 0.0, page_width, page_height], &name);
                            let content = [inset, inset, page_width - 2.0 * inset, page_height - 2.0 * inset];
                            let [x, y, drawn_width, drawn_height] = page.drawn;
                            assert_eq!((page.matrix[1], page.matrix[2]), (0.0, 0.0), "{name}: untagged images are placed without rotation or skew");
                            assert!(((drawn_width / drawn_height) - (width / height)).abs() < 0.001, "{name}: the image keeps its aspect ratio");
                            assert_close(&[x + drawn_width / 2.0, y + drawn_height / 2.0], &[content[0] + content[2] / 2.0, content[1] + content[3] / 2.0], &format!("{name}: centred in the margin box"));
                            if fit == "fit" {
                                assert!(page.clip.is_none(), "{name}: fit needs no clip");
                                assert!(x >= inset - 0.01 && y >= inset - 0.01 && drawn_width <= content[2] + 0.01 && drawn_height <= content[3] + 0.01, "{name}: fit stays inside the margins");
                                assert!((drawn_width - content[2]).abs() < 0.01 || (drawn_height - content[3]).abs() < 0.01, "{name}: fit touches two sides of the margin box");
                            } else {
                                assert_close(&page.clip.expect("fill clips to the margin box"), &content, &name);
                                assert!(drawn_width >= content[2] - 0.01 && drawn_height >= content[3] - 0.01, "{name}: fill covers the margin box");
                                assert!((drawn_width - content[2]).abs() < 0.01 || (drawn_height - content[3]).abs() < 0.01, "{name}: fill trims only one direction");
                            }
                            assert_eq!(page.image_bytes, plain.image_bytes, "{name}: the embedded image data is unchanged by layout");
                            assert_eq!(page.image_dict, plain.image_dict, "{name}: the image keeps its own size and encoding");
                        }
                        assert_eq!(pages[0].image_bytes, jpeg, "{name}: the JPEG is still embedded byte for byte");
                        combinations += 1;
                    }
                }
            }
        }
        assert_eq!(combinations, 36);
        for (path, original) in paths.iter().zip(&originals) { assert_eq!(&std::fs::read(path).unwrap(), original, "{} must stay unchanged", path.display()); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn images_to_pdf_records_the_exact_placement_for_paper_pages() {
        let root = sandbox("images-to-pdf-layout-recorded");
        let (paths, _) = layout_fixtures(&root);
        let pages = combine_with(&root, "a4-small-fill", &paths[..1], serde_json::json!({ "pageSize": "a4", "margin": "small", "fit": "fill", "orientation": "automatic" }));
        assert_close(&pages[0].media, &[0.0, 0.0, 841.8898, 595.2756], "A4 landscape for the wide image");
        assert_close(&pages[0].clip.unwrap(), &[36.0, 36.0, 769.8898, 523.2756], "a small margin is 36 pt");
        assert_close(&pages[0].drawn, &[-102.3306, 36.0, 1046.5512, 523.2756], "fill trims the left and right edges equally");
        let pages = combine_with(&root, "letter-large-fit", &paths[1..2], serde_json::json!({ "pageSize": "letter", "margin": "large", "fit": "fit", "orientation": "automatic" }));
        assert_close(&pages[0].media, &[0.0, 0.0, 612.0, 792.0], "Letter portrait for the tall image");
        assert!(pages[0].clip.is_none());
        assert_close(&pages[0].drawn, &[90.0, 72.0, 432.0, 648.0], "a large margin is 72 pt and fit letterboxes the sides");
        let pages = combine_with(&root, "letter-none-fit", &paths[..1], serde_json::json!({ "pageSize": "letter", "margin": "none", "fit": "fit", "orientation": "automatic" }));
        assert_close(&pages[0].media, &[0.0, 0.0, 792.0, 612.0], "Letter landscape");
        assert_close(&pages[0].drawn, &[0.0, 108.0, 792.0, 396.0], "no margin reaches the page edge");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn images_to_pdf_automatic_orientation_turns_wide_images_landscape_and_others_portrait() {
        let root = sandbox("images-to-pdf-layout-orientation");
        let (paths, _) = layout_fixtures(&root);
        for size in ["a4", "letter"] {
            let pages = combine_with(&root, size, &paths, serde_json::json!({ "pageSize": size, "margin": "small", "fit": "fit", "orientation": "automatic" }));
            let shapes = pages.iter().map(|page| if page.media[2] > page.media[3] { "landscape" } else { "portrait" }).collect::<Vec<_>>();
            assert_eq!(shapes, ["landscape", "portrait", "portrait"], "{size}: wide, tall and square images");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    // An upright 48 × 80 portrait with a different colour in each quadrant,
    // so every rotation or mirror of it is told apart.
    fn quadrant_portrait() -> image::RgbImage {
        let colours = [[220, 30, 30], [30, 200, 40], [30, 40, 220], [230, 220, 30]];
        image::RgbImage::from_fn(48, 80, |x, y| image::Rgb(colours[usize::from(x >= 24) + 2 * usize::from(y >= 40)]))
    }

    // The pixels a camera stores for an upright image under EXIF orientation
    // `tag`, written out per value from the EXIF definition.
    fn stored_for_tag(upright: &image::RgbImage, tag: u16) -> image::RgbImage {
        let (width, height) = upright.dimensions();
        let (stored_width, stored_height) = if tag >= 5 { (height, width) } else { (width, height) };
        image::RgbImage::from_fn(stored_width, stored_height, |x, y| *upright.get_pixel(match tag {
            1 => x, 2 => width - 1 - x, 3 => width - 1 - x, 4 => x, 5 => y, 6 => width - 1 - y, 7 => width - 1 - y, _ => y,
        }, match tag {
            1 | 2 => y, 3 | 4 => height - 1 - y, 5 | 6 => x, _ => height - 1 - x,
        }))
    }

    fn exif_orientation(tag: u16) -> Vec<u8> {
        let mut tiff = b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01".to_vec();
        tiff.extend_from_slice(&tag.to_be_bytes());
        tiff.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        tiff
    }

    fn tagged_jpeg(image: &image::RgbImage, tag: u16) -> Vec<u8> {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95).encode_image(image).unwrap();
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend(exif_orientation(tag));
        let mut segment = vec![0xFF, 0xE1];
        segment.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        segment.extend(exif);
        jpeg.splice(2..2, segment);
        jpeg
    }

    fn tagged_png(image: &image::RgbImage, tag: u16) -> Vec<u8> {
        let mut png = Vec::new();
        image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let mut body = b"eXIf".to_vec();
        body.extend(exif_orientation(tag));
        let mut crc = flate2::Crc::new();
        crc.update(&body);
        let mut chunk = ((body.len() - 4) as u32).to_be_bytes().to_vec();
        chunk.extend(body);
        chunk.extend_from_slice(&crc.sum().to_be_bytes());
        png.splice(33..33, chunk);
        png
    }

    // Samples the centre of each quadrant of the upright rectangle the page
    // draws into, follows the `cm` matrix back into the embedded pixels, and
    // checks each one shows that quadrant's colour.
    fn assert_upright_on_page(page: &LaidOutPage, stored: &image::RgbImage, upright: &image::RgbImage, context: &str) {
        let [a, b, c, d, e, f] = page.matrix;
        let [x, y, width, height] = page.drawn;
        let determinant = a * d - b * c;
        for (across, down) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
            let (page_x, page_y) = (x + width * across - e, y + height * (1.0 - down) - f);
            let (u, v) = ((d * page_x - c * page_y) / determinant, (a * page_y - b * page_x) / determinant);
            let actual = stored.get_pixel((u * stored.width() as f32) as u32, ((1.0 - v) * stored.height() as f32) as u32);
            let expected = upright.get_pixel((across * upright.width() as f32) as u32, (down * upright.height() as f32) as u32);
            assert!(actual.0.iter().zip(expected.0).all(|(actual, expected)| actual.abs_diff(expected) < 40), "{context}: quadrant ({across}, {down}) shows {actual:?}, expected {expected:?}");
        }
    }

    #[test]
    fn images_to_pdf_places_every_orientation_tag_upright_and_keeps_the_jpeg_byte_identical() {
        let root = sandbox("images-to-pdf-orientation-jpeg");
        let upright = quadrant_portrait();
        for tag in 1..=8 {
            let stored = stored_for_tag(&upright, tag);
            let path = root.join(format!("exif-{tag}.jpg"));
            let jpeg = tagged_jpeg(&stored, tag);
            std::fs::write(&path, &jpeg).unwrap();
            for (name, layout) in [("match", serde_json::json!({})), ("a4", serde_json::json!({ "pageSize": "a4", "margin": "small", "fit": "fit", "orientation": "automatic" }))] {
                let context = format!("tag {tag} on {name}");
                let pages = combine_with(&root, &format!("{tag}-{name}"), std::slice::from_ref(&path), layout);
                let page = &pages[0];
                assert_eq!(page.image_bytes, jpeg, "{context}: the JPEG is embedded byte for byte");
                assert_eq!(page.image_dict.get(b"Filter").unwrap().as_name().unwrap(), b"DCTDecode");
                assert_eq!((page.image_dict.get(b"Width").unwrap().as_i64().unwrap(), page.image_dict.get(b"Height").unwrap().as_i64().unwrap()), (i64::from(stored.width()), i64::from(stored.height())), "{context}: the embedded data keeps its stored size");
                if name == "match" {
                    assert_close(&page.media, &[0.0, 0.0, 48.0, 80.0], &format!("{context}: the page is the upright size"));
                    assert_close(&page.drawn, &[0.0, 0.0, 48.0, 80.0], &context);
                } else {
                    assert!(page.media[3] > page.media[2], "{context}: an upright portrait gets a portrait page");
                }
                assert_upright_on_page(page, &image::load_from_memory(&page.image_bytes).unwrap().to_rgb8(), &upright, &context);
            }
            assert_eq!(std::fs::read(&path).unwrap(), jpeg, "the source stays unchanged");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn images_to_pdf_orients_tagged_png_pixels_before_embedding_them() {
        use std::io::Read;
        let root = sandbox("images-to-pdf-orientation-png");
        let upright = quadrant_portrait();
        for tag in 1..=8 {
            let path = root.join(format!("exif-{tag}.png"));
            let png = tagged_png(&stored_for_tag(&upright, tag), tag);
            std::fs::write(&path, &png).unwrap();
            let pages = combine_with(&root, &format!("png-{tag}"), std::slice::from_ref(&path), serde_json::json!({}));
            let page = &pages[0];
            assert_close(&page.media, &[0.0, 0.0, 48.0, 80.0], &format!("tag {tag}: the page is the upright size"));
            assert_eq!((page.matrix[1], page.matrix[2]), (0.0, 0.0), "tag {tag}: oriented pixels need no turn on the page");
            assert_eq!(page.image_dict.get(b"Filter").unwrap().as_name().unwrap(), b"FlateDecode");
            let mut samples = Vec::new();
            flate2::read::ZlibDecoder::new(page.image_bytes.as_slice()).read_to_end(&mut samples).unwrap();
            let decoded = image::RgbImage::from_raw(page.image_dict.get(b"Width").unwrap().as_i64().unwrap() as u32, page.image_dict.get(b"Height").unwrap().as_i64().unwrap() as u32, samples).unwrap();
            assert_eq!(decoded, upright, "tag {tag}: the PDF holds the upright pixels exactly");
            assert_eq!(std::fs::read(&path).unwrap(), png, "the source stays unchanged");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn images_to_pdf_gives_a_tagged_portrait_photo_a_portrait_a4_page() {
        let root = sandbox("images-to-pdf-orientation-a4");
        let upright = quadrant_portrait();
        let stored = stored_for_tag(&upright, 6);
        assert!(stored.width() > stored.height(), "the camera stored the portrait sideways");
        let path = root.join("portrait.jpg");
        std::fs::write(&path, tagged_jpeg(&stored, 6)).unwrap();
        let pages = combine_with(&root, "a4", std::slice::from_ref(&path), serde_json::json!({ "pageSize": "a4", "margin": "none", "fit": "fit", "orientation": "automatic" }));
        assert_close(&pages[0].media, &[0.0, 0.0, 595.2756, 841.8898], "portrait A4");
        assert_close(&pages[0].drawn, &[(595.2756 - 841.8898 * 48.0 / 80.0) / 2.0, 0.0, 841.8898 * 48.0 / 80.0, 841.8898], "the upright photo fits the page height");
        assert_upright_on_page(&pages[0], &image::load_from_memory(&pages[0].image_bytes).unwrap().to_rgb8(), &upright, "portrait A4");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A PDF whose pages each draw one text label, so a merged file's page order can be read back.
    fn make_labeled_pdf(path: &PathBuf, labels: &[&str]) {
        use lopdf::{dictionary, Document, Object, Stream};
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let kids = labels.iter().map(|label| {
            let content_id = document.add_object(Stream::new(dictionary! {}, format!("BT /F1 24 Tf 72 720 Td (label:{label}) Tj ET").into_bytes()));
            Object::Reference(document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } }, "Contents" => content_id,
            }))
        }).collect::<Vec<_>>();
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => labels.len() as i64 }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    fn page_labels(path: &PathBuf) -> Vec<String> {
        let document = lopdf::Document::load(path).unwrap();
        document.get_pages().values().map(|page_id| {
            let content = String::from_utf8_lossy(&document.get_page_content(*page_id)).into_owned();
            let start = content.find("(label:").expect("every merged page keeps its label") + "(label:".len();
            content[start..start + content[start..].find(')').unwrap()].to_string()
        }).collect()
    }

    fn merged(root: &PathBuf, folder: &str, paths: &[&PathBuf]) -> Vec<JobOutcome> {
        tauri::async_runtime::block_on(run_merge_pdfs(&MergePdfRequest {
            paths: paths.iter().map(|path| (*path).clone()).collect(), output_location: location(root, folder), output_path: None,
        }))
    }

    fn folder_files(root: &PathBuf, folder: &str) -> Vec<PathBuf> {
        std::fs::read_dir(root.join(folder)).unwrap().map(|entry| entry.unwrap().path()).collect()
    }

    #[test]
    fn merge_joins_every_page_of_each_entry_in_list_order() {
        let root = sandbox("merge-order");
        let a = root.join("a.pdf"); make_labeled_pdf(&a, &["A1", "A2"]);
        let b = root.join("b.pdf"); make_labeled_pdf(&b, &["B1", "B2"]);
        let originals = [&a, &b].map(|path| (path.clone(), std::fs::read(path).unwrap()));

        let forward = merged(&root, "forward", &[&a, &b]);
        assert_eq!(forward.len(), 2, "each entry gets its own row");
        assert!(forward.iter().all(|outcome| outcome.error().is_none()), "{:?}", forward[0].error());
        let name = forward[0].output_paths()[0].file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(forward.iter().map(|outcome| outcome.detail.clone()).collect::<Vec<_>>(), [format!("Pages 1–2 of {name}."), format!("Pages 3–4 of {name}.")]);
        assert_eq!(page_labels(&forward[0].output_paths()[0]), ["A1", "A2", "B1", "B2"]);

        let reversed = merged(&root, "reversed", &[&b, &a]);
        assert_eq!(page_labels(&reversed[0].output_paths()[0]), ["B1", "B2", "A1", "A2"]);
        assert_eq!(reversed.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), [b.clone(), a.clone()], "rows follow the list order");

        // The page-count check counts a repeated file once per entry, so a repeat passes it.
        let repeated = merged(&root, "repeated", &[&a, &b, &a]);
        assert!(repeated[0].error().is_none(), "{:?}", repeated[0].error());
        assert_eq!(page_labels(&repeated[0].output_paths()[0]), ["A1", "A2", "B1", "B2", "A1", "A2"]);

        for (path, bytes) in originals { assert_eq!(std::fs::read(&path).unwrap(), bytes, "{} must stay byte-identical", path.display()); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn merge_fails_every_entry_and_writes_nothing_when_one_input_is_missing_or_damaged() {
        let root = sandbox("merge-failure");
        let a = root.join("a.pdf"); make_labeled_pdf(&a, &["A1", "A2"]);
        let b = root.join("b.pdf"); make_labeled_pdf(&b, &["B1"]);
        let damaged = root.join("damaged.pdf"); std::fs::write(&damaged, b"%PDF-1.7\nthis is not a PDF body").unwrap();
        let missing = root.join("missing.pdf");
        let originals = [&a, &b, &damaged].map(|path| (path.clone(), std::fs::read(path).unwrap()));

        for (folder, bad) in [("missing", &missing), ("damaged", &damaged)] {
            let paths = [&a, bad, &b, bad];
            let outcomes = merged(&root, folder, &paths);
            assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths.map(|path| path.clone()), "{folder}: one outcome per entry, in order");
            assert!(outcomes.iter().all(|outcome| outcome.error().is_some() && outcome.output_paths().is_empty()), "{folder}: every entry fails");
            let messages = outcomes.iter().map(|outcome| outcome.error().unwrap().message.clone()).collect::<Vec<_>>();
            assert!(messages[1].contains(&bad.display().to_string()), "{folder}: the failing entry carries its own error: {}", messages[1]);
            let bad_name = bad.file_name().unwrap().to_string_lossy();
            let stopped = format!("Stopped because {bad_name} (entry 2) failed. Nothing was saved.");
            for index in [0, 2, 3] { assert_eq!(messages[index], stopped, "{folder}: entry {} names the entry that stopped the merge", index + 1); }
            assert!(folder_files(&root, folder).is_empty(), "{folder}: no output, partial or final, is left behind");
        }

        let single = merged(&root, "single-missing", &[&a, &missing]);
        assert_eq!(single[0].error().unwrap().message, "Stopped because missing.pdf failed. Nothing was saved.");

        for (path, bytes) in originals { assert_eq!(std::fs::read(&path).unwrap(), bytes, "{} must stay byte-identical", path.display()); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn merge_accepts_structured_and_signed_pdfs_and_notes_each_entry_on_its_own_row() {
        use crate::kit::contracts::WarningKind;
        use crate::kit::pdf::remaining::tests::{assert_nothing_carried, media_boxes, structured_pdf, warning_kinds, Structure};
        let root = sandbox("merge-structured");
        let signed = root.join("signed.pdf");
        structured_pdf(&signed, Structure { sizes: &[(842, 595)], form: true, signed: true, ..Structure::default() });
        let plain = root.join("plain.pdf");
        structured_pdf(&plain, Structure { sizes: &[(595, 842), (612, 792)], ..Structure::default() });
        let marked = root.join("marked.pdf");
        structured_pdf(&marked, Structure { sizes: &[(420, 595)], bookmarks: true, tags: true, form: true, ..Structure::default() });
        let originals = [&signed, &plain, &marked].map(|path| (path.clone(), std::fs::read(path).unwrap()));

        let outcomes = merged(&root, "out", &[&signed, &plain, &marked]);

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), [signed.clone(), plain.clone(), marked.clone()]);
        assert!(outcomes.iter().all(|outcome| outcome.error().is_none()), "{:?}", outcomes.iter().find_map(JobOutcome::error));
        let output = outcomes[0].output_paths()[0].clone();
        assert!(outcomes.iter().all(|outcome| outcome.output_paths() == [output.clone()]));
        assert_eq!(warning_kinds(&outcomes[0]), [WarningKind::SignatureRemoved], "only the signed entry says its signature was removed");
        assert!(outcomes[1].warnings.is_empty(), "an entry with nothing lost has no notes");
        assert_eq!(warning_kinds(&outcomes[2]), [
            WarningKind::BookmarksNotCarried { into: "the merged file".into() },
            WarningKind::TagsNotCarried { into: "the merged file".into() },
        ]);
        let name = output.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(outcomes.iter().map(|outcome| outcome.detail.clone()).collect::<Vec<_>>(), [format!("Page 1 of {name}."), format!("Pages 2–3 of {name}."), format!("Page 4 of {name}.")]);
        assert_eq!(media_boxes(&output), [(842, 595), (595, 842), (612, 792), (420, 595)], "each page keeps its own size and orientation");
        assert_nothing_carried(&output);
        for (path, bytes) in originals { assert_eq!(std::fs::read(&path).unwrap(), bytes, "{} must stay byte-identical", path.display()); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn merge_refuses_a_password_protected_or_empty_entry_and_writes_nothing() {
        use crate::kit::contracts::{ErrorKind, OutcomePointer};
        use crate::kit::pdf::remaining::tests::{password_protected_pdf, zero_page_pdf};
        let root = sandbox("merge-refused");
        let a = root.join("a.pdf"); make_labeled_pdf(&a, &["A1"]);
        let b = root.join("b.pdf"); make_labeled_pdf(&b, &["B1"]);
        let locked = root.join("locked.pdf"); password_protected_pdf(&locked);
        let empty = root.join("empty.pdf"); zero_page_pdf(&empty);
        let originals = [&a, &b, &locked, &empty].map(|path| (path.clone(), std::fs::read(path).unwrap()));

        for (folder, bad, pointer) in [("locked", &locked, Some(OutcomePointer::Tool { tool_id: "lock-unlock".into() })), ("empty", &empty, None)] {
            let outcomes = merged(&root, folder, &[&a, bad, &b]);
            assert!(outcomes.iter().all(|outcome| outcome.output_paths().is_empty() && outcome.warnings.is_empty()), "{folder}");
            let failed = outcomes[1].error().expect("the bad entry fails");
            assert_eq!(failed.kind, ErrorKind::InvalidInput, "{folder}");
            assert_eq!(failed.pointer, pointer, "{folder}");
            if pointer.is_some() { assert_eq!(failed.message, remaining::PASSWORD_PROTECTED); }
            let stopped = format!("Stopped because {}.pdf failed. Nothing was saved.", folder);
            for index in [0, 2] { assert_eq!(outcomes[index].error().unwrap().message, stopped, "{folder}"); }
            assert!(folder_files(&root, folder).is_empty(), "{folder}: nothing is written");
        }
        for (path, bytes) in originals { assert_eq!(std::fs::read(&path).unwrap(), bytes, "{} must stay byte-identical", path.display()); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn create_gif_with_an_unreadable_frame_names_that_frame_and_saves_nothing() {
        let root = sandbox("gif-unreadable-frame");
        let first = root.join("first.png"); write_png(&first, 24, 10);
        let broken = root.join("IMG_2212.heic"); std::fs::write(&broken, b"not a heic image").unwrap();
        let last = root.join("last.png");
        image::RgbaImage::from_pixel(48, 12, Rgba([1, 2, 3, 40])).save(&last).unwrap();
        let chosen_folder = root.join("chosen");
        std::fs::create_dir_all(&chosen_folder).unwrap();
        let paths = vec![first.clone(), broken.clone(), last.clone()];
        let inputs = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let outcomes = tauri::async_runtime::block_on(run_create_gif(&GifCreateRequest {
            paths: paths.clone(), frame_delay_ms: 100, loop_forever: true, output_location: OutputLocation::AlongsideInput, output_path: Some(chosen_folder.join("first-animated.gif")),
        }));
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths);
        assert!(outcomes.iter().all(|outcome| outcome.output_paths().is_empty() && outcome.error().is_some()));
        assert_eq!(
            outcomes[1].error().unwrap().message,
            "Frame 2, IMG_2212.heic, isn't a readable HEIC image. Replace or remove frame 2, then make the GIF again. Nothing was saved."
        );
        assert_eq!(std::fs::read_dir(&chosen_folder).unwrap().count(), 0, "nothing is published");
        for (path, bytes) in paths.iter().zip(inputs) { assert_eq!(std::fs::read(path).unwrap(), bytes); }
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

        assert_merge_success("merge valid inputs", &[pdf.clone(), pdf_two.clone()], run_merge_pdfs(&MergePdfRequest {
            paths: vec![pdf.clone(), pdf_two.clone()], output_location: location(&root, "merge-valid"), output_path: None,
        }));
        assert_aggregate_failure("merge missing later input", &[pdf.clone(), missing_pdf.clone()], &missing_pdf, run_merge_pdfs(&MergePdfRequest {
            paths: vec![pdf.clone(), missing_pdf.clone()], output_location: location(&root, "merge-missing"), output_path: None,
        }));
        assert_aggregate_success("images-to-pdf valid inputs", &[image.clone(), image_two.clone()], run_images_to_pdf(&ImagesToPdfRequest { layout: Default::default(),
            paths: vec![image.clone(), image_two.clone()], output_location: location(&root, "images-valid"), output_path: None,
        }));
        assert_aggregate_failure("images-to-pdf missing later input", &[image.clone(), missing_image.clone()], &missing_image, run_images_to_pdf(&ImagesToPdfRequest { layout: Default::default(),
            paths: vec![image.clone(), missing_image.clone()], output_location: location(&root, "images-missing"), output_path: None,
        }));
        assert_aggregate_success("GIF valid inputs", &[image.clone(), image_two.clone()], run_create_gif(&GifCreateRequest {
            paths: vec![image.clone(), image_two.clone()], frame_delay_ms: 100, loop_forever: true, output_location: location(&root, "gif-valid"), output_path: None,
        }));
        assert_aggregate_failure("GIF missing later input", &[image.clone(), missing_gif_frame.clone()], &missing_gif_frame, run_create_gif(&GifCreateRequest {
            paths: vec![image.clone(), missing_gif_frame.clone()], frame_delay_ms: 100, loop_forever: true, output_location: location(&root, "gif-missing"), output_path: None,
        }));
        assert_aggregate_success("TIFF valid inputs", &[tiff.clone(), tiff_two.clone()], run_combine_tiff_pages(&TiffRequest {
            mode: TiffMode::Combine, paths: vec![tiff.clone(), tiff_two.clone()], pages: None, output_location: location(&root, "tiff-valid"), output_path: None,
        }));
        assert_aggregate_failure("TIFF missing later input", &[tiff.clone(), missing_tiff.clone()], &missing_tiff, run_combine_tiff_pages(&TiffRequest {
            mode: TiffMode::Combine, paths: vec![tiff.clone(), missing_tiff.clone()], pages: None, output_location: location(&root, "tiff-missing"), output_path: None,
        }));

        let _ = std::fs::remove_dir_all(root);
    }

    fn write_gray_tiff(path: &std::path::Path, pages: &[&[u8]]) {
        let mut encoder = tiff::encoder::TiffEncoder::new(std::fs::File::create(path).unwrap()).unwrap();
        for data in pages { encoder.write_image::<tiff::encoder::colortype::Gray8>(data.len() as u32, 1, data).unwrap(); }
    }

    #[test]
    fn tiff_split_batch_settles_each_file_on_its_own() {
        let root = sandbox("tiff-split-batch");
        let two = root.join("scan-0042.tiff"); write_gray_tiff(&two, &[&[1, 2], &[3, 4]]);
        let three = root.join("scan-0043.tiff"); write_gray_tiff(&three, &[&[5], &[6], &[7]]);
        let single = root.join("cover.tiff"); write_gray_tiff(&single, &[&[8]]);
        let corrupt = root.join("broken.tiff"); std::fs::write(&corrupt, b"II*\0not a real tiff").unwrap();
        let paths = vec![two.clone(), single.clone(), corrupt.clone(), three.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let out = location(&root, "split");
        let OutputLocation::CustomFolder(folder) = &out else { unreachable!() };
        let folder = folder.clone();

        let outcomes = run_split_tiff_pages(&RunEvents::silent(), TiffRequest { mode: TiffMode::Split, paths: paths.clone(), pages: None, output_location: out, output_path: None });

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths, "each file settles in its own row, in order");
        let names = |outcome: &JobOutcome| outcome.output_paths().iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert!(matches!(outcomes[0].state, OutcomeState::Succeeded { .. }), "{:?}", outcomes[0]);
        assert_eq!(names(&outcomes[0]), ["scan-0042-page-01.tif", "scan-0042-page-02.tif"]);
        let OutcomeState::Skipped { reason } = &outcomes[1].state else { panic!("one-page TIFF must be skipped: {:?}", outcomes[1]) };
        assert_eq!(reason.message, tools::TIFF_SINGLE_PAGE_SKIP);
        assert!(outcomes[1].output_paths().is_empty());
        assert!(outcomes[2].error().is_some(), "the corrupt TIFF fails alone: {:?}", outcomes[2]);
        assert!(outcomes[2].output_paths().is_empty());
        assert!(matches!(outcomes[3].state, OutcomeState::Succeeded { .. }), "{:?}", outcomes[3]);
        assert_eq!(names(&outcomes[3]), ["scan-0043-page-01.tif", "scan-0043-page-02.tif", "scan-0043-page-03.tif"]);

        let mut written = std::fs::read_dir(&folder).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        written.sort();
        assert_eq!(written, ["scan-0042-page-01.tif", "scan-0042-page-02.tif", "scan-0043-page-01.tif", "scan-0043-page-02.tif", "scan-0043-page-03.tif"], "several files in Split mode are never combined, and skipped or failed files write nothing");
        for (path, original) in paths.iter().zip(originals) { assert_eq!(std::fs::read(path).unwrap(), original, "split must not modify {}", path.display()); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn tiff_combine_with_one_file_still_combines() {
        let root = sandbox("tiff-combine-one");
        let two = root.join("scan.tiff"); write_gray_tiff(&two, &[&[1, 2], &[3, 4]]);
        let outcomes = tauri::async_runtime::block_on(run_combine_tiff_pages(&TiffRequest { mode: TiffMode::Combine, paths: vec![two.clone()], pages: None, output_location: location(&root, "combine"), output_path: None }));
        assert_eq!(outcomes.len(), 1);
        let outputs = outcomes[0].output_paths();
        assert_eq!(outputs.len(), 1, "one file in Combine mode makes one TIFF: {:?}", outcomes[0]);
        assert_eq!(outputs[0].file_name().unwrap(), "scan-combined.tiff");
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
            spec: CropSpec::Margins { top: 10.0, right: 10.0, bottom: 10.0, left: 10.0 },
            scope: RangedPageScope::Selected { pages: vec![] },
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

    fn crop_request(paths: Vec<PathBuf>, scope: RangedPageScope, output_location: OutputLocation) -> CropPdfRequest {
        CropPdfRequest { paths, spec: CropSpec::Rectangle { rectangle: PdfRect { x: 100.0, y: 100.0, width: 200.0, height: 300.0 } }, scope, output_location }
    }

    fn cropped_pages(path: &std::path::Path) -> Vec<usize> {
        let document = lopdf::Document::load(path).unwrap();
        document.get_pages().into_iter().filter(|(_, page)| {
            // Crop sets only the CropBox; an uncropped page keeps its MediaBox alone.
            let crop_box = document.get_dictionary(*page).unwrap().get(b"CropBox").ok().and_then(|value| value.as_array().ok()).cloned().unwrap_or_default();
            crop_box.iter().map(|value| value.as_float().unwrap()).collect::<Vec<_>>() == vec![100.0, 100.0, 300.0, 400.0]
        }).map(|(number, _)| number as usize).collect()
    }

    #[test]
    fn crop_command_clamps_a_typed_range_per_file_and_skips_files_with_no_pages_left() {
        assert!(matches!(serde_json::from_value::<RangedPageScope>(serde_json::json!({ "range": { "ranges": "2-4, 9-" } })).unwrap(), RangedPageScope::Range { ranges } if ranges == "2-4, 9-"));
        let root = sandbox("crop-clamped-range");
        let (long, medium, short) = (root.join("long.pdf"), root.join("medium.pdf"), root.join("short.pdf"));
        make_pdf(&long, 10); make_pdf(&medium, 3); make_pdf(&short, 1);
        let originals = [&long, &medium, &short].map(|path| std::fs::read(path).unwrap());
        let request = crop_request(vec![long.clone(), medium.clone(), short.clone()], RangedPageScope::Range { ranges: "2-4, 9-".into() }, location(&root, "out"));
        let outcomes = request.paths.iter().map(|path| editor::crop(&request, path.clone())).collect::<Vec<_>>();

        assert_eq!(cropped_pages(&outcomes[0].output_paths()[0]), vec![2, 3, 4, 9, 10]);
        assert_eq!(outcomes[0].detail, "PDF saved");
        assert_eq!(cropped_pages(&outcomes[1].output_paths()[0]), vec![2, 3]);
        assert_eq!(outcomes[1].detail, "PDF saved. Shortened pages 2-4 to 2-3 because the PDF has 3 pages. Skipped pages 9- because the PDF has 3 pages.");
        match &outcomes[2].state {
            OutcomeState::Skipped { reason } => {
                assert_eq!(reason.code, crate::kit::contracts::SkipCode::RangeEmptyAfterClamping);
                assert_eq!(reason.message, "No pages in 2-4, 9- are in this PDF, which has 1 page.");
            }
            state => panic!("expected the short file to be skipped, got {state:?}"),
        }
        let mut written = std::fs::read_dir(root.join("out")).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect::<Vec<_>>();
        written.sort();
        assert_eq!(written, vec!["long-cropped.pdf", "medium-cropped.pdf"]);

        let through_command = tauri::async_runtime::block_on(crop_pdf(RunEvents::silent(), crop_request(vec![short.clone()], RangedPageScope::Range { ranges: "2-4, 9-".into() }, location(&root, "command"))));
        assert!(matches!(&through_command[..], [outcome] if outcome.input_path == short && matches!(&outcome.state, OutcomeState::Skipped { reason } if reason.code == crate::kit::contracts::SkipCode::RangeEmptyAfterClamping)));
        assert_eq!(std::fs::read_dir(root.join("command")).unwrap().count(), 0);
        assert_eq!([&long, &medium, &short].map(|path| std::fs::read(path).unwrap()), originals);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn crop_command_rejects_malformed_ranges_for_every_file_before_writing() {
        let root = sandbox("crop-malformed-range");
        let (first, second) = (root.join("first.pdf"), root.join("second.pdf"));
        make_pdf(&first, 3); make_pdf(&second, 3);
        for (ranges, reason) in [("4-2", "\"4-2\" runs backwards"), ("0", "\"0\" uses page 0"), ("2, x", "\"x\" isn't a page or range"), (" ", "Enter at least one page or range")] {
            let outcomes = tauri::async_runtime::block_on(crop_pdf(RunEvents::silent(), crop_request(
                vec![first.clone(), second.clone()],
                RangedPageScope::Range { ranges: ranges.into() },
                location(&root, "out"),
            )));
            assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), vec![first.clone(), second.clone()], "{ranges}");
            assert!(outcomes.iter().all(|outcome| outcome.error().is_some_and(|error| error.kind == ErrorKind::InvalidInput && error.message.contains(reason))), "{ranges}: {outcomes:?}");
        }
        assert_eq!(std::fs::read_dir(root.join("out")).unwrap().count(), 0);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn crop_command_fails_only_the_pdf_whose_margins_leave_no_area() {
        use lopdf::{Document, Object};
        let root = sandbox("crop-margins-batch");
        let (letter, narrow) = (root.join("letter.pdf"), root.join("narrow.pdf"));
        make_pdf(&letter, 2); make_pdf(&narrow, 2);
        let mut document = Document::load(&narrow).unwrap();
        let second = document.get_pages().values().copied().nth(1).unwrap();
        document.get_dictionary_mut(second).unwrap().set("MediaBox", vec![Object::Real(0.0), Object::Real(0.0), Object::Real(300.0), Object::Real(792.0)]);
        document.save(&narrow).unwrap();
        let narrow_original = std::fs::read(&narrow).unwrap();
        let outcomes = tauri::async_runtime::block_on(crop_pdf(RunEvents::silent(), CropPdfRequest {
            paths: vec![narrow.clone(), letter.clone()],
            spec: CropSpec::Margins { top: 0.0, right: 160.0, bottom: 0.0, left: 160.0 },
            scope: RangedPageScope::All,
            output_location: location(&root, "crop-batch"),
        }));

        assert_eq!(outcomes.len(), 2);
        let error = outcomes[0].error().expect("the narrow PDF must fail");
        assert!(matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput), "{error:?}");
        assert!(error.message.contains("Page 2"), "{}", error.message);
        assert!(outcomes[0].output_paths().is_empty());
        assert_eq!(std::fs::read(&narrow).unwrap(), narrow_original);
        assert!(outcomes[1].error().is_none(), "{:?}", outcomes[1].error());
        assert_eq!(outcomes[1].output_paths(), vec![root.join("crop-batch").join("letter-cropped.pdf")]);
        assert_eq!(std::fs::read_dir(root.join("crop-batch")).unwrap().count(), 1);

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
    fn ocr_command_rejects_bad_languages_modes_and_ranges_before_any_helper_runs() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-boundary");
        // A valid request must still run nothing here, so the engine is made missing on purpose.
        let _env = EnvGuard::set(&[("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-resources"))), ("TOOLBOX_TESSERACT_PATH", Some(root.join("missing-tesseract")))]);
        let input = root.join("scan.pdf");
        make_pdf(&input, 1);
        let request = |change: fn(&mut vision::OcrRequest)| {
            let mut request = vision::OcrRequest { paths: vec![input.clone()], page_range: Some("1".into()), mode: "text".into(), force: false, languages: vec!["eng".into()], output_location: location(&root, "ocr") };
            change(&mut request);
            tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), request))
        };
        let cases: [(fn(&mut vision::OcrRequest), &str); 4] = [
            (|request| request.languages.clear(), "Choose at least one recognition language."),
            (|request| request.languages = vec!["eng".into(), "xx".into()], "\"xx\" is not a recognition language Toolbox knows."),
            (|request| request.mode = "pdfa".into(), "\"pdfa\" is not an OCR output. Choose a searchable PDF or a text file."),
            (|request| request.page_range = Some("3-1".into()), "runs backwards"),
        ];
        for (change, message) in cases {
            let outcomes = request(change);
            assert_eq!(outcomes.len(), 1);
            assert_eq!(outcomes[0].input_path, input);
            let error = outcomes[0].error().expect(message);
            assert!(matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput), "{message}: {:?}", error.kind);
            assert!(error.message.contains(message), "{message}: {}", error.message);
        }
        let valid = request(|_| {});
        assert!(valid[0].error().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::Unavailable)));
        assert!(!root.join("ocr").read_dir().is_ok_and(|mut entries| entries.next().is_some()));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Sets process variables for one test and puts the old values back when dropped.
    /// Hold `lock_process_env` for as long as this lives.
    struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);

    impl EnvGuard {
        fn set(values: &[(&'static str, Option<PathBuf>)]) -> Self {
            let saved = values.iter().map(|(key, _)| (*key, std::env::var_os(key))).collect();
            for (key, value) in values {
                match value { Some(value) => std::env::set_var(key, value), None => std::env::remove_var(key) }
            }
            Self(saved)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.0.drain(..) {
                match value { Some(value) => std::env::set_var(key, value), None => std::env::remove_var(key) }
            }
        }
    }

    /// Writes a PDF whose pages are only pictures of text, the way a scanner makes them.
    /// There is no text layer, so the words can come back only through OCR. A page with
    /// no lines is a blank white scan.
    fn scanned_pdf(path: &PathBuf, pages: &[&[&str]]) {
        use flate2::{write::ZlibEncoder, Compression};
        use lopdf::{dictionary, Document, Object, Stream};
        use std::io::Write;
        const WIDTH: usize = 1275;
        const HEIGHT: usize = 1650;
        const SIZE: f32 = 56.0;
        let font = fontdue::Font::from_bytes(include_bytes!("../resources/fonts/DejaVuSans.ttf") as &[u8], fontdue::FontSettings::default()).unwrap();
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let mut kids = Vec::new();
        for lines in pages {
            let mut pixels = vec![255u8; WIDTH * HEIGHT];
            for (row, line) in lines.iter().enumerate() {
                let baseline = 200 + row * 120;
                let mut pen = 120.0f32;
                for character in line.chars() {
                    let (metrics, coverage) = font.rasterize(character, SIZE);
                    let top = baseline as i32 - metrics.height as i32 - metrics.ymin;
                    for y in 0..metrics.height {
                        for x in 0..metrics.width {
                            let (px, py) = (pen as i32 + metrics.xmin + x as i32, top + y as i32);
                            if px < 0 || py < 0 || px as usize >= WIDTH || py as usize >= HEIGHT { continue; }
                            let index = py as usize * WIDTH + px as usize;
                            pixels[index] = pixels[index].min(255 - coverage[y * metrics.width + x]);
                        }
                    }
                    pen += metrics.advance_width;
                }
            }
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&pixels).unwrap();
            let mut scan = Stream::new(dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => WIDTH as i64, "Height" => HEIGHT as i64,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8, "Filter" => "FlateDecode",
            }, encoder.finish().unwrap());
            scan.allows_compression = false;
            let scan_id = document.add_object(scan);
            let content_id = document.add_object(Stream::new(dictionary! {}, b"q 612 0 0 792 0 0 cm /Scan Do Q".to_vec()));
            let page_id = document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! { "XObject" => dictionary! { "Scan" => scan_id } }, "Contents" => content_id,
            });
            kids.push(Object::Reference(page_id));
        }
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages.len() as i64 }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    /// Points OCR at a real engine. Staged bundle resources win; otherwise the engine and
    /// renderer come from PATH and the English data is the copy committed with the app.
    /// Returns None, so the caller can skip, only when no engine exists and
    /// TOOLBOX_REQUIRE_OCR_RESOURCES is unset.
    fn real_ocr_engine() -> Option<EnvGuard> {
        let resources = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
        if resources.join("ocr/manifest.json").is_file() { return Some(EnvGuard::set(&[])); }
        let runs = |program: &str, flag: &str| std::process::Command::new(program).arg(flag).output().is_ok_and(|result| result.status.success());
        if runs("tesseract", "--version") && runs("pdftoppm", "-v") {
            return Some(EnvGuard::set(&[("TOOLBOX_TESSDATA_PATH", Some(resources.join("ocr/tessdata/eng.traineddata")))]));
        }
        assert!(std::env::var_os("TOOLBOX_REQUIRE_OCR_RESOURCES").is_none(), "OCR resources are required but neither a staged bundle nor tesseract and pdftoppm on PATH were found");
        eprintln!("skipping: no OCR engine is staged or on PATH");
        None
    }

    fn ocr_text_request(paths: Vec<PathBuf>, output_location: OutputLocation) -> vision::OcrRequest {
        vision::OcrRequest { paths, page_range: None, mode: "text".into(), force: false, languages: vec!["eng".into()], output_location }
    }

    #[test]
    fn ocr_reads_an_image_only_scan_into_page_ordered_text_and_repeats_it_byte_for_byte() {
        let _guard = crate::kit::lock_process_env();
        let Some(_engine) = real_ocr_engine() else { return };
        let root = sandbox("ocr-scan");
        let input = root.join("scan.pdf");
        scanned_pdf(&input, &[&["Quarterly invoice summary", "Northwind shipping ledger"], &["Second page closing remarks", "Thank you for reading"]]);
        let original = std::fs::read(&input).unwrap();
        let folder = location(&root, "ocr");

        let first = assert_success("ocr first run", ocr_pdf(RunEvents::silent(), ocr_text_request(vec![input.clone()], folder.clone())));
        let second = assert_success("ocr second run", ocr_pdf(RunEvents::silent(), ocr_text_request(vec![input.clone()], folder)));
        let text = std::fs::read_to_string(&first[0]).unwrap();
        let again = std::fs::read(&second[0]).unwrap();
        let names = [&first[0], &second[0]].map(|path| path.file_name().unwrap().to_string_lossy().into_owned());
        let input_after = std::fs::read(&input).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(names, ["scan-ocr.txt".to_string(), "scan-ocr-1.txt".to_string()], "a second run gets a numbered name instead of overwriting");
        assert_eq!(text.as_bytes(), again.as_slice(), "the same scan gives the same text");
        assert_eq!(input_after, original, "the scan is never changed");
        assert!(!text.contains('\r'), "lines end in LF: {text:?}");
        assert!(text.ends_with('\n'), "{text:?}");
        let pages = text.split('\u{c}').collect::<Vec<_>>();
        assert_eq!(pages.len(), 2, "one form feed between two pages: {text:?}");
        for (page, words) in pages.iter().zip([["Quarterly", "invoice", "Northwind", "ledger"], ["Second", "closing", "Thank", "reading"]]) {
            for word in words { assert!(page.contains(word), "{word} is missing from {page:?}"); }
        }
        assert!(pages[0].find("Quarterly") < pages[0].find("Northwind"), "lines keep reading order: {:?}", pages[0]);
    }

    #[test]
    fn ocr_reports_locked_and_blank_scans_beside_a_readable_one() {
        let _guard = crate::kit::lock_process_env();
        let Some(_engine) = real_ocr_engine() else { return };
        let root = sandbox("ocr-batch");
        let locked = root.join("locked.pdf");
        let blank = root.join("blank.pdf");
        let readable = root.join("readable.pdf");
        std::fs::write(&locked, crate::kit::password::tests::pdf(Some(("owner", "user")))).unwrap();
        scanned_pdf(&blank, &[&[]]);
        scanned_pdf(&readable, &[&["Toolbox reads this scan"]]);
        let paths = vec![locked.clone(), blank.clone(), readable.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let outcomes = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_text_request(paths.clone(), location(&root, "ocr"))));
        let written = std::fs::read_dir(root.join("ocr")).unwrap().count();
        let text = outcomes.get(2).and_then(|outcome| outcome.output_paths().first()).and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
        let after = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths);
        let locked_error = outcomes[0].error().expect("a locked scan fails");
        assert_eq!(locked_error.kind, ErrorKind::InvalidInput);
        assert_eq!(locked_error.message, "It's password-protected. Remove its password in File Security, then run OCR again.");
        assert_eq!(locked_error.pointer, Some(crate::kit::contracts::OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
        let blank_error = outcomes[1].error().expect("a blank scan fails");
        assert_eq!(blank_error.kind, ErrorKind::Processing);
        assert_eq!(blank_error.message, "OCR completed but found no readable text.");
        assert!(outcomes[0].output_paths().is_empty() && outcomes[1].output_paths().is_empty());
        assert!(outcomes[2].error().is_none(), "the readable scan still runs: {:?}", outcomes[2].error());
        assert!(text.contains("Toolbox reads this scan"), "{text:?}");
        assert_eq!(written, 1, "only the readable scan writes a file");
        assert_eq!(after, originals);
    }

    #[test]
    fn extract_from_pdf_hands_a_scan_to_ocr_while_a_text_pdf_in_the_same_batch_succeeds() {
        if !pdftotext_available() { return; }
        use crate::kit::contracts::{OutcomePointer, SkipCode};
        let root = sandbox("pdf-to-text-needs-ocr");
        let scan = root.join("scan.pdf");
        let text_pdf = root.join("text.pdf");
        scanned_pdf(&scan, &[&["Only pixels here"]]);
        make_pdf(&text_pdf, 2);
        let paths = vec![scan.clone(), text_pdf.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let outcomes = tauri::async_runtime::block_on(extract_from_pdf(RunEvents::silent(), extract_request(paths.clone(), ExtractMode::Text, None, location(&root, "text"))));
        let written = std::fs::read_dir(root.join("text")).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        let text = outcomes.get(1).and_then(|outcome| outcome.output_paths().first()).and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
        let after = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths);
        let OutcomeState::Skipped { reason } = &outcomes[0].state else { panic!("the scan needs OCR: {:?}", outcomes[0]) };
        assert_eq!(reason.code, SkipCode::NeedsOcr);
        assert_eq!(reason.message, remaining::NEEDS_OCR_MESSAGE);
        assert_eq!(reason.pointer, Some(OutcomePointer::Tool { tool_id: "pdf-ocr".into() }));
        assert!(outcomes[0].output_paths().is_empty());
        assert!(outcomes[1].error().is_none(), "the text PDF still runs: {:?}", outcomes[1].error());
        assert!(text.contains("Toolbox page 1") && text.contains("Toolbox page 2"), "{text:?}");
        assert_eq!(written, vec!["text-text.txt".to_string()], "only the text PDF writes a file");
        assert_eq!(after, originals);
    }

    #[cfg(unix)]
    fn fake_ocr_helpers(root: &PathBuf) -> [PathBuf; 3] {
        use std::os::unix::fs::PermissionsExt;
        let renderer = root.join("fake-pdftoppm");
        let engine = root.join("fake-tesseract");
        let tessdata = root.join("eng.traineddata");
        std::fs::write(&renderer, "#!/bin/sh\ntouch \"${10}.png\"\n").unwrap();
        std::fs::write(&engine, "#!/bin/sh\nprintf 'recognized words\\n'\n").unwrap();
        std::fs::write(&tessdata, b"fake tessdata").unwrap();
        for helper in [&renderer, &engine] { std::fs::set_permissions(helper, std::fs::Permissions::from_mode(0o755)).unwrap(); }
        [renderer, engine, tessdata]
    }

    #[cfg(unix)]
    #[test]
    fn ocr_names_the_missing_engine_renderer_or_english_data_for_every_file() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-missing");
        let [renderer, engine, tessdata] = fake_ocr_helpers(&root);
        let missing = root.join("missing");
        let inputs = vec![root.join("first.pdf"), root.join("second.pdf")];
        for input in &inputs { make_pdf(input, 1); }
        let cases = [
            ("TOOLBOX_TESSERACT_PATH", "The text recognition engine is missing. Reinstall Toolbox."),
            ("TOOLBOX_PDFTOPPM_PATH", "The PDF page renderer is missing. Reinstall Toolbox."),
            ("TOOLBOX_TESSDATA_PATH", "The English recognition data is missing. Reinstall Toolbox."),
        ];
        let run = |name: &str| tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_text_request(inputs.clone(), location(&root, name))));
        for (variable, message) in cases {
            let values = [("TOOLBOX_PDFTOPPM_PATH", &renderer), ("TOOLBOX_TESSERACT_PATH", &engine), ("TOOLBOX_TESSDATA_PATH", &tessdata)]
                .map(|(key, value)| (key, Some(if key == variable { missing.clone() } else { value.clone() })));
            let mut values = values.to_vec();
            values.push(("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-bundle"))));
            let env = EnvGuard::set(&values);
            let outcomes = run(variable);
            drop(env);
            assert_eq!(outcomes.len(), 2, "{variable}");
            for outcome in &outcomes {
                let error = outcome.error().unwrap_or_else(|| panic!("{variable}: {} ran without its resource", outcome.input_path.display()));
                assert_eq!(error.kind, ErrorKind::Unavailable, "{variable}");
                assert_eq!(error.message, message, "{variable}");
                assert!(outcome.output_paths().is_empty());
            }
            assert_eq!(std::fs::read_dir(root.join(variable)).unwrap().count(), 0, "{variable} left an output behind");
        }
        let env = EnvGuard::set(&[("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-bundle"))), ("TOOLBOX_PDFTOPPM_PATH", Some(renderer)), ("TOOLBOX_TESSERACT_PATH", Some(engine)), ("TOOLBOX_TESSDATA_PATH", Some(tessdata))]);
        let restored = run("restored");
        drop(env);
        let _ = std::fs::remove_dir_all(&root);
        assert!(restored.iter().all(|outcome| outcome.error().is_none()), "the same files run once every resource is back: {restored:?}");
    }

    #[cfg(unix)]
    #[test]
    fn ocr_fails_only_the_locked_file_in_a_batch() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-locked");
        let [renderer, engine, tessdata] = fake_ocr_helpers(&root);
        let _env = EnvGuard::set(&[("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-bundle"))), ("TOOLBOX_PDFTOPPM_PATH", Some(renderer)), ("TOOLBOX_TESSERACT_PATH", Some(engine)), ("TOOLBOX_TESSDATA_PATH", Some(tessdata))]);
        let first = root.join("first.pdf");
        let locked = root.join("locked.pdf");
        let last = root.join("last.pdf");
        make_pdf(&first, 1);
        std::fs::write(&locked, crate::kit::password::tests::pdf(Some(("owner", "user")))).unwrap();
        make_pdf(&last, 2);
        let locked_original = std::fs::read(&locked).unwrap();

        let outcomes = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_text_request(vec![first, locked.clone(), last], location(&root, "ocr"))));
        let locked_after = std::fs::read(&locked).unwrap();
        let texts = [&outcomes[0], &outcomes[2]].map(|outcome| outcome.output_paths().first().and_then(|path| std::fs::read_to_string(path).ok()));
        let _ = std::fs::remove_dir_all(&root);

        let error = outcomes[1].error().expect("the locked file fails");
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        assert_eq!(error.message, "It's password-protected. Remove its password in File Security, then run OCR again.");
        assert_eq!(error.pointer, Some(crate::kit::contracts::OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
        assert!(outcomes[1].output_paths().is_empty());
        assert_eq!(texts, [Some("recognized words\n".to_string()), Some("recognized words\n\u{c}recognized words\n".to_string())]);
        assert_eq!(locked_after, locked_original);
    }

    fn split_ranges_request(paths: Vec<PathBuf>, ranges: &str, output_location: OutputLocation) -> SplitPdfRequest {
        SplitPdfRequest { paths, mode: SplitMode::Ranges { ranges: ranges.into() }.into(), output_location }
    }

    fn images_range_request(paths: Vec<PathBuf>, range: &str, output_location: OutputLocation) -> PdfToImagesRequest {
        PdfToImagesRequest { paths, dpi: 72, format: "png".into(), page_range: Some(range.into()), pages: None, picks: Vec::new(), output_location }
    }

    #[test]
    fn split_and_pdf_to_images_reject_malformed_ranges_before_any_file_runs() {
        let root = sandbox("malformed-page-ranges");
        let inputs = vec![root.join("first.pdf"), root.join("second.pdf")];
        std::fs::write(&inputs[0], b"not a pdf yet").unwrap();
        make_pdf(&inputs[1], 3);
        // Split names the offending entry in its own words; the shared grammar quotes it.
        for (range, split_named, named) in [("3-1", "3-1 runs backwards (write 1-3).", "\"3-1\""), ("0-2", "0-2 isn't a range. Pages start at 1.", "\"0-2\""), ("1,,2", "Entry 2 is empty.", "Entry 2"), ("1, two", "two isn't a page.", "\"two\"")] {
            let split = tauri::async_runtime::block_on(split_pdf(RunEvents::silent(), split_ranges_request(inputs[..1].to_vec(), range, location(&root, "split"))));
            let images = tauri::async_runtime::block_on(pdf_to_images(RunEvents::silent(), images_range_request(inputs.clone(), range, location(&root, "images"))));
            for (tool, outcomes, expected, named) in [("split", split, &inputs[..1], split_named), ("pdf to images", images, &inputs[..], named)] {
                assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), expected, "{tool} {range}");
                for outcome in &outcomes {
                    let error = outcome.error().unwrap_or_else(|| panic!("{tool} accepted {range}"));
                    assert_eq!(error.kind, ErrorKind::InvalidInput, "{tool} {range}");
                    assert!(error.message.contains(named), "{tool} {range}: {}", error.message);
                    assert!(outcome.output_paths().is_empty(), "{tool} {range}");
                }
            }
        }
        assert_eq!(std::fs::read_dir(root.join("split")).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(root.join("images")).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn split_and_pdf_to_images_accept_pages_spans_and_open_ends() {
        let _split = remaining::tests::SPLIT_TEST_LOCK.lock().unwrap();
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("page-range-grammar");
        let input = root.join("document.pdf");
        make_pdf(&input, 9);

        let parts = assert_success("split ranges", split_pdf(RunEvents::silent(), split_ranges_request(vec![input.clone()], "1-3, 5, 8-", location(&root, "split"))));
        let names = parts.iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(names, ["document-pages-1-3.pdf", "document-page-5.pdf", "document-pages-8-9.pdf"]);
        let page_counts = parts.iter().map(|path| lopdf::Document::load(path).unwrap().get_pages().len()).collect::<Vec<_>>();
        assert_eq!(page_counts, [3, 1, 2]);

        let images = assert_success("pdf to images ranges", pdf_to_images(RunEvents::silent(), images_range_request(vec![input.clone()], "1-3, 5, 8-", location(&root, "images"))));
        let names = images.iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(names, ["document-page-01.png", "document-page-02.png", "document-page-03.png", "document-page-05.png", "document-page-08.png", "document-page-09.png"]);

        // Split checks its ranges against the page count before writing, so going past the end is invalid input there.
        let outcomes = tauri::async_runtime::block_on(split_pdf(RunEvents::silent(), split_ranges_request(vec![input.clone()], "1, 8-12", location(&root, "split-past-end"))));
        let error = outcomes[0].error().expect("split accepted pages past the end");
        assert_eq!(error.kind, ErrorKind::InvalidInput, "{}", error.message);
        assert!(outcomes[0].output_paths().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pdf_to_images_clamps_one_range_to_each_pdf_and_skips_empty_ones() {
        let _split = remaining::tests::SPLIT_TEST_LOCK.lock().unwrap();
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("pdf-to-images-clamp");
        let (long, memo, letter) = (root.join("long.pdf"), root.join("memo.pdf"), root.join("letter.pdf"));
        make_pdf(&long, 9);
        make_pdf(&memo, 4);
        make_pdf(&letter, 2);
        let originals = [&long, &memo, &letter].map(|path| std::fs::read(path).unwrap());

        let outcomes = tauri::async_runtime::block_on(pdf_to_images(RunEvents::silent(), images_range_request(vec![long.clone(), memo.clone(), letter.clone()], "3-5, 8-", location(&root, "images"))));

        let names = |outcome: &crate::kit::contracts::JobOutcome| outcome.output_paths().iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert!(outcomes[0].error().is_none(), "{:?}", outcomes[0]);
        assert_eq!(names(&outcomes[0]), ["long-page-03.png", "long-page-04.png", "long-page-05.png", "long-page-08.png", "long-page-09.png"]);
        assert_eq!(outcomes[0].detail, "5 images, pages 3-5, 8 and 9");
        assert!(outcomes[1].error().is_none(), "{:?}", outcomes[1]);
        assert_eq!(names(&outcomes[1]), ["memo-page-03.png", "memo-page-04.png"]);
        assert_eq!(outcomes[1].detail, "2 images. Range shortened to pages 3 and 4, because memo.pdf has 4 pages.");
        match &outcomes[2].state {
            crate::kit::contracts::OutcomeState::Skipped { reason } => {
                assert_eq!(reason.code, crate::kit::contracts::SkipCode::RangeEmptyAfterClamping);
                assert_eq!(reason.message, "No pages left in 3-5, 8-. It has 2 pages, so nothing was made.");
            }
            state => panic!("letter.pdf must be skipped, got {state:?}"),
        }
        let made = std::fs::read_dir(root.join("images")).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert!(made.iter().all(|name| !name.starts_with("letter")), "{made:?}");
        assert_eq!(made.len(), 7, "{made:?}");
        assert_eq!([&long, &memo, &letter].map(|path| std::fs::read(path).unwrap()), originals);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pdf_to_images_uses_one_pdfs_picks_instead_of_the_shared_range() {
        let _split = remaining::tests::SPLIT_TEST_LOCK.lock().unwrap();
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("pdf-to-images-picks");
        let (picked, ranged) = (root.join("picked.pdf"), root.join("ranged.pdf"));
        make_pdf(&picked, 2);
        make_pdf(&ranged, 6);
        let mut request = images_range_request(vec![picked.clone(), ranged.clone()], "5-", location(&root, "images"));
        request.picks = vec![remaining::PagePicks { path: picked.clone(), pages: vec![0] }];

        let outcomes = tauri::async_runtime::block_on(pdf_to_images(RunEvents::silent(), request));

        let names = outcomes.iter().map(|outcome| outcome.output_paths().iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>()).collect::<Vec<_>>();
        assert_eq!(names, [vec!["picked-page-01.png"], vec!["ranged-page-05.png", "ranged-page-06.png"]]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pdf_to_images_rejects_a_malformed_range_for_every_pdf_before_rendering() {
        let root = sandbox("pdf-to-images-malformed");
        let (first, second) = (root.join("first.pdf"), root.join("second.pdf"));
        make_pdf(&first, 3);
        make_pdf(&second, 3);
        let originals = [&first, &second].map(|path| std::fs::read(path).unwrap());

        let outcomes = tauri::async_runtime::block_on(pdf_to_images(RunEvents::silent(), images_range_request(vec![first.clone(), second.clone()], "5-3", location(&root, "images"))));

        assert_eq!(outcomes.len(), 2);
        for outcome in &outcomes {
            let error = outcome.error().expect("a backwards range must be rejected");
            assert_eq!(error.kind, ErrorKind::InvalidInput, "{}", error.message);
            assert!(error.message.contains("runs backwards"), "{}", error.message);
            assert!(outcome.output_paths().is_empty());
        }
        assert!(std::fs::read_dir(root.join("images")).map_or(true, |mut entries| entries.next().is_none()));
        assert_eq!([&first, &second].map(|path| std::fs::read(path).unwrap()), originals);
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
            let output = &outcome.output_paths()[0];
            assert!(output.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.ends_with("-ocr.txt")), "{name} must save a -ocr text file: {}", output.display());
            let text = std::fs::read_to_string(output).unwrap_or_default();
            assert!(text.contains("Toolbox page"), "{name} did not extract the fixture text: {text:?}");
            assert!(!text.contains('\r'), "{name} must end lines in LF: {text:?}");
        }
        outcome.output_paths().to_vec()
    }

    fn named(naming: &mut Vec<(&'static str, PathBuf, Vec<PathBuf>)>, command: &'static str, input: PathBuf, outputs: Vec<PathBuf>) -> Vec<PathBuf> {
        naming.push((command, input, outputs.clone()));
        outputs
    }

    // Icon presets and TIFF Split mode predate declared suffixes and keep their
    // shipped names. Extract from PDF declares its text suffix and names its images folder -images.
    // Extract GIF Frames writes one timings file beside its frames. These are the only
    // outputs allowed a different suffix family.
    const LEGACY_SUFFIX_FAMILIES: &[(&str, &str)] = &[("generate_icon_set", "-favicon"), ("process_tiff_pages", "-page"), ("extract_from_pdf", "-images"), ("extract_gif_frames", "-timings")];

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
        assert!(other.output_paths().iter().all(|output| output.is_file() || output.is_dir()) && !other.output_paths().is_empty());
    }

    fn extract_request(paths: Vec<PathBuf>, mode: ExtractMode, page_range: Option<&str>, output_location: OutputLocation) -> ExtractFromPdfRequest {
        ExtractFromPdfRequest { paths, mode, page_range: page_range.map(str::to_string), output_location, preserve_layout: false, min_image_size: None }
    }

    fn pdftotext_available() -> bool {
        crate::kit::resources::resolve_helper(&crate::kit::resources::PDF_TEXT_EXTRACTOR).is_ok()
    }

    fn extract_pair(root: &PathBuf, offender: &PathBuf, name: &str) -> (PathBuf, Vec<JobOutcome>) {
        let good = root.join("good.pdf");
        make_extract_pdf(&good, &[(false, true)]);
        let originals = [Original::of(offender), Original::of(&good)];
        let outcomes = tauri::async_runtime::block_on(extract_from_pdf(RunEvents::silent(), extract_request(vec![offender.clone(), good.clone()], ExtractMode::Images, None, location(root, name))));
        for original in &originals { original.assert_untouched(); }
        (good, outcomes)
    }

    #[test]
    fn heic_and_oriented_sources_run_through_the_icon_and_gif_commands() {
        use crate::kit::images::fixtures;
        let root = sandbox("heic-media-sources");
        let source = fixtures::red_left_blue_right(64, 32);
        let heic = root.join("master.heic");
        let deep = root.join("deep.heif");
        let oriented = root.join("oriented.jpg");
        std::fs::write(&heic, fixtures::heic(&source)).unwrap();
        std::fs::write(&deep, fixtures::heic_10bit(&source)).unwrap();
        std::fs::write(&oriented, fixtures::jpeg_with_orientation(&source, 6)).unwrap();
        let inputs = [&heic, &deep, &oriented].map(|path| (path.clone(), std::fs::read(path).unwrap()));

        for (input, warned) in [(&heic, false), (&deep, true), (&oriented, false)] {
            let outcomes = tauri::async_runtime::block_on(generate_icon_set(RunEvents::silent(), IconSetRequest {
                paths: vec![input.clone()], preset: "favicon".into(), sizes: vec![], output_location: location(&root, "icons"),
            }));
            assert!(outcomes[0].error().is_none(), "{}: {:?}", input.display(), outcomes[0].error());
            assert_eq!(outcomes[0].warnings.len(), usize::from(warned), "{}", input.display());
            let first = image::open(&outcomes[0].output_paths()[0]).unwrap();
            assert!(matches!(first.color(), image::ColorType::Rgb8 | image::ColorType::Rgba8));
            assert_eq!(fixtures::red_edge(&first), if input == &oriented { "top" } else { "left" });
        }
        let gif = assert_aggregate_success("GIF from HEIC frames", &[heic.clone(), deep.clone(), oriented.clone()], run_create_gif(&GifCreateRequest {
            paths: vec![heic.clone(), deep.clone(), oriented.clone()], frame_delay_ms: 100, loop_forever: true, output_location: location(&root, "gif"), output_path: None,
        }));
        assert!(gif[0].extension().is_some_and(|extension| extension == "gif"));
        for (path, bytes) in inputs { assert_eq!(std::fs::read(&path).unwrap(), bytes, "{} must stay byte-identical", path.display()); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_pdf_over_the_recorded_size_fails_alone() {
        let root = sandbox("limit-size");
        let offender = root.join("oversized.pdf");
        std::fs::write(&offender, b"%PDF-1.7\n").unwrap();
        std::fs::OpenOptions::new().write(true).open(&offender).unwrap().set_len(1_000_000_001).unwrap();
        let (_, outcomes) = extract_pair(&root, &offender, "out");
        assert_only_offender_limited(&outcomes, &offender, LimitKind::FileSize, "1 GB");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_pdf_over_the_recorded_page_count_fails_alone() {
        let root = sandbox("limit-pages");
        let offender = root.join("long.pdf");
        make_pdf(&offender, 5_001);
        let (_, outcomes) = extract_pair(&root, &offender, "out");
        assert_only_offender_limited(&outcomes, &offender, LimitKind::PageCount, "5,000");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_pdf_stream_expanding_past_the_recorded_size_fails_alone() {
        let root = sandbox("limit-pdf-expansion");
        let offender = root.join("bomb.pdf");
        std::fs::write(&offender, crate::kit::common::limits::fixtures::pdf_with_stream(&crate::kit::common::limits::fixtures::zlib_of_zeros(500_000_001))).unwrap();
        let (_, outcomes) = extract_pair(&root, &offender, "out");
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
        make_extract_pdf(&slow, &[(false, true)]);
        make_extract_pdf(&good, &[(false, true)]);
        let originals = [std::fs::read(&slow).unwrap(), std::fs::read(&good).unwrap()];
        let request = Arc::new(extract_request(vec![slow.clone(), good.clone()], ExtractMode::Images, None, location(&root, "out")));
        let released = Arc::new(AtomicBool::new(false));
        let limits = crate::kit::common::limits::ToolLimits { max_duration: std::time::Duration::from_secs(1), ..crate::kit::common::limits::recorded("extract_from_pdf").unwrap() };
        let job_released = Arc::clone(&released);
        let job_slow = slow.clone();
        let outcomes = BatchRunner::run_with_limits(&RunEvents::silent(), "extract_from_pdf", request.paths.clone(), &request.output_location, Some(limits), &request, move |request, path| {
            while path == job_slow && !job_released.load(Ordering::SeqCst) { std::thread::sleep(std::time::Duration::from_millis(10)); }
            remaining::extract(request, path)
        });
        released.store(true, Ordering::SeqCst);
        assert_eq!([std::fs::read(&slow).unwrap(), std::fs::read(&good).unwrap()], originals);
        assert_only_offender_limited(&outcomes, &slow, LimitKind::Time, "1 second");
        let _ = std::fs::remove_dir_all(root);
    }

    fn assert_merge_stopped_by(outcomes: &[JobOutcome], paths: &[PathBuf], offender: &PathBuf, output: &PathBuf, message: &str) {
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths);
        for outcome in outcomes {
            let error = outcome.error().expect("every merge input fails");
            assert!(outcome.output_paths().is_empty());
            if outcome.input_path == *offender {
                assert_eq!(error.kind, ErrorKind::LimitExceeded { limit: LimitKind::PageCount }, "{}", error.message);
                assert!(error.message.contains(message), "{} should contain {message}", error.message);
            } else {
                assert_eq!(error.kind, ErrorKind::Processing);
                let name = offender.file_name().unwrap().to_string_lossy();
                assert!(error.message.contains(&format!("Stopped because {name} failed")), "{}", error.message);
            }
        }
        assert_eq!(std::fs::read_dir(output).unwrap().count(), 0, "no merged output is published");
    }

    #[test]
    fn a_merge_crossing_the_recorded_page_total_fails_on_the_input_that_crossed_it() {
        let root = sandbox("limit-merge-total");
        let paths = ["first", "crosses", "last"].map(|stem| root.join(format!("{stem}.pdf"))).to_vec();
        for (path, pages) in paths.iter().zip([2_500, 2_501, 1]) { make_pdf(path, pages); }
        let originals = paths.iter().map(Original::of).collect::<Vec<_>>();
        let output = root.join("out");
        let outcomes = tauri::async_runtime::block_on(run_merge_pdfs(&MergePdfRequest { paths: paths.clone(), output_location: location(&root, "out"), output_path: None }));
        for original in &originals { original.assert_untouched(); }
        assert_eq!(crate::kit::common::limits::recorded("merge_pdfs").unwrap().max_pages, Some(5_000));
        assert_merge_stopped_by(&outcomes, &paths, &paths[1], &output, "Adding crosses.pdf takes the merged PDF past 5,000 pages, over the limit for one PDF.");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_merge_input_over_the_recorded_page_count_fails_the_merge_on_that_input() {
        let root = sandbox("limit-merge-input");
        let paths = vec![root.join("short.pdf"), root.join("long.pdf")];
        make_pdf(&paths[0], 1);
        make_pdf(&paths[1], 5_001);
        let output = root.join("out");
        let outcomes = tauri::async_runtime::block_on(run_merge_pdfs(&MergePdfRequest { paths: paths.clone(), output_location: location(&root, "out"), output_path: None }));
        assert_merge_stopped_by(&outcomes, &paths, &paths[1], &output, "More than 5,000 pages, over the limit for one PDF.");
        let _ = std::fs::remove_dir_all(root);
    }

    // Split takes one input per run, so a batch is a run per file into the same folder.
    #[test]
    fn a_split_batch_fails_only_the_pdf_over_the_recorded_page_count() {
        let _split = remaining::tests::SPLIT_TEST_LOCK.lock().unwrap();
        let root = sandbox("limit-split");
        let offender = root.join("long.pdf");
        let good = root.join("good.pdf");
        make_pdf(&offender, 5_001);
        make_pdf(&good, 2);
        let originals = [Original::of(&offender), Original::of(&good)];
        let output = location(&root, "out");
        let outcomes = [&offender, &good].into_iter().flat_map(|path| tauri::async_runtime::block_on(split_pdf(RunEvents::silent(), SplitPdfRequest {
            paths: vec![path.clone()], mode: SplitMode::EveryN { pages: 1 }.into(), output_location: output.clone(),
        }))).collect::<Vec<_>>();
        for original in &originals { original.assert_untouched(); }
        assert_only_offender_limited(&outcomes, &offender, LimitKind::PageCount, "5,000");
        assert_eq!(outcomes[1].output_paths().len(), 2);
        assert_eq!(std::fs::read_dir(root.join("out")).unwrap().count(), 2, "only the good PDF's parts are written");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn every_registered_command_runs_with_real_fixtures_in_isolated_sandboxes() {
        let _split = remaining::tests::SPLIT_TEST_LOCK.lock().unwrap();
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("all");
        let image = root.join("image.png"); write_png(&image, 256, 44);
        let image_two = root.join("image-two.png"); write_png(&image_two, 256, 88);
        let lossy_image = root.join("lossy-image.jpg");
        image::RgbImage::from_fn(64, 64, |x, y| {
            if (x / 2 + y / 2) % 2 == 0 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 255, 255]) }
        }).save_with_format(&lossy_image, image::ImageFormat::Jpeg).unwrap();
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
        let compressible_pdf = root.join("compressible.pdf"); make_compressible_pdf(&compressible_pdf);
        let plain_pdf_bytes = std::fs::read(&pdf).unwrap();
        let plain_image_bytes = std::fs::read(&image).unwrap();
        let plain_lossy_image_bytes = std::fs::read(&lossy_image).unwrap();
        let animated_bytes = std::fs::read(&animated).unwrap();
        let tiff_bytes = std::fs::read(&tiff).unwrap();
        let text = root.join("notes.txt"); std::fs::write(&text, b"Caf\xe9 notes\n\tindented line\n").unwrap();
        let text_bytes = std::fs::read(&text).unwrap();
        let markdown = root.join("release-notes.md"); std::fs::write(&markdown, "# Release notes\n\n| Step | Result |\n|---|---|\n| build | ok |\n\n```\ncargo test\n```\n\n![chart](image.png)\n\n![logo](https://example.com/logo.png)\n\nSee [the site](https://example.com).\n").unwrap();
        let markdown_bytes = std::fs::read(&markdown).unwrap();
        let rich_text = root.join("memo.rtf"); std::fs::write(&rich_text, r"{\rtf1\ansi\deff0{\fonttbl{\f0 Times New Roman;}{\f1 Arial;}}{\info{\title Team memo}}{\f1\b\fs32 Team memo\par}{\i Ship} the {\ul release}.\par\trowd\cellx2000\cellx4000\pard\intbl Owner\cell Date\cell\row\pard Done.\par}").unwrap();
        let rich_text_bytes = std::fs::read(&rich_text).unwrap();
        let word = root.join("brief.docx"); std::fs::write(&word, crate::kit::pdf::docx::fixtures::docx(r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Project brief</w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">Scope and </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>timeline</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Phase</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Week</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#, &[("rIdStyles", "styles", "styles.xml", false)], &[("word/styles.xml", br#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style></w:styles>"#)])).unwrap();
        let open_document = root.join("minutes.odt"); std::fs::write(&open_document, crate::kit::pdf::odt::fixtures::odt("", r#"<text:h text:outline-level="1">Meeting minutes</text:h><text:p>Agreed <text:span>actions</text:span></text:p><table:table><table:table-row><table:table-cell><text:p>Owner</text:p></table:table-cell><table:table-cell><text:p>Due</text:p></table:table-cell></table:table-row></table:table>"#, &[], &[])).unwrap();
        let workbook = root.join("budget.xlsx"); std::fs::write(&workbook, crate::kit::pdf::xlsx::fixtures::xlsx(&[("Budget", "visible", &crate::kit::pdf::xlsx::fixtures::sheet("", r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Quarter</t></is></c><c r="B1" t="inlineStr"><is><t>Spend</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>Q1</t></is></c><c r="B2"><f>SUM(1,2)</f><v>3</v></c></row>"#, "")), ("Draft", "hidden", &crate::kit::pdf::xlsx::fixtures::sheet("", r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Scratch</t></is></c></row>"#, ""))], "", "", &[])).unwrap();
        let table = root.join("orders.csv"); std::fs::write(&table, "Order;Customer;Note\n1001;\"Smith; Jo\";\"said \"\"hi\"\"\"\n1002;Lee;plain\n").unwrap();
        let table_bytes = std::fs::read(&table).unwrap();

        let protected = root.join("protected.pdf");
        let protected_result = protect_files(RunEvents::silent(), PasswordRequest { paths: vec![pdf.clone()], password: crate::kit::TEST_PASSWORD.into(), password_overrides: Default::default(), output_location: OutputLocation::AlongsideInput });
        let protected_output = assert_success("protect fixture", protected_result)[0].clone();
        std::fs::rename(protected_output, &protected).unwrap();

        let image_pdf = root.join("image-pdf.pdf");
        // Images to PDF embeds a JPEG as-is, which is the one kind of image PDF extraction returns without recompression.
        let photo = root.join("photo.jpg"); image::open(&image).unwrap().to_rgb8().save(&photo).unwrap();
        let image_pdf_request = ImagesToPdfRequest { layout: Default::default(), paths: vec![photo.clone()], output_location: OutputLocation::AlongsideInput, output_path: None };
        let image_pdf_result = run_images_to_pdf(&image_pdf_request);
        let image_pdf_output = assert_success("images-to-pdf fixture", image_pdf_result)[0].clone();
        std::fs::rename(image_pdf_output, &image_pdf).unwrap();

        let signature_size = tauri::async_runtime::block_on(inspect_signature_image(InspectPdfRequest { path: image.clone() })).unwrap();
        assert_eq!((signature_size.width, signature_size.height), (256, 256));
        assert!(tauri::async_runtime::block_on(inspect_signature_image(InspectPdfRequest { path: lossy_image.clone() })).is_ok());
        let refused = tauri::async_runtime::block_on(inspect_signature_image(InspectPdfRequest { path: tiff.clone() })).unwrap_err();
        assert!(refused.contains("Choose a PNG or JPG"), "{refused}");
        let mut outputs = Vec::new();
        let mut naming = Vec::new();
        let inspected_scene = tauri::async_runtime::block_on(inspect_pdf_scene(InspectPdfRequest { path: pdf.clone() })).unwrap();
        assert_eq!(inspected_scene.pages.len(), 2);
        let visual_scene: scene::PdfScene = serde_json::from_value(serde_json::json!({ "pages": [
            { "sourceIndex": 0, "width": 612, "height": 792, "rotation": 0, "crop": null, "objects": [
                { "kind": "highlight", "rect": { "x": 65, "y": 62, "width": 200, "height": 30 }, "text": "", "fontSize": 18, "color": "#FFD700", "opacity": 0.35 },
                { "kind": "text", "rect": { "x": 72, "y": 150, "width": 360, "height": 40 }, "text": "Edited entirely on this device", "fontSize": 18, "color": "#1455AA", "opacity": 1 },
                { "kind": "shape", "rect": { "x": 65, "y": 145, "width": 380, "height": 50 }, "text": "", "fontSize": 18, "color": "#1455AA", "opacity": 1 },
                { "kind": "watermark", "rect": { "x": 0, "y": 0, "width": 612, "height": 792 }, "text": "", "fontSize": 18, "color": "#000000", "opacity": 1, "watermark": { "text": "PRIVATE DRAFT", "opacity": 40, "color": "#AAAAAA", "rotation": -45 }, "layer": "behind", "pageRange": "all" },
                { "kind": "signature", "rect": { "x": 90, "y": 580, "width": 200, "height": 50 }, "text": "", "fontSize": 18, "color": "#202020", "opacity": 1, "signatureMode": "image", "signaturePath": image, "pageRange": "1-2" },
                { "kind": "signature", "rect": { "x": 320, "y": 580, "width": 200, "height": 50 }, "text": "A. Signer", "fontSize": 18, "color": "#1A3A8F", "opacity": 1, "signatureMode": "text" }
            ] },
            { "sourceIndex": null, "width": 612, "height": 792, "rotation": 90, "crop": {"x": 20, "y": 20, "width": 550, "height": 700}, "objects": [
                { "kind": "text", "rect": { "x": 72, "y": 100, "width": 350, "height": 40 }, "text": "New page in the same session", "fontSize": 18, "color": "#202020", "opacity": 1 }
            ] }
        ] })).unwrap();
        let rendered_scene = tauri::async_runtime::block_on(preview_pdf_scene(scene::PreviewRequest { path: pdf.clone(), scene: visual_scene.clone(), page_index: 0, sections: Default::default() })).unwrap();
        assert!(rendered_scene.data_url.starts_with("data:image/png;base64,"));
        let rendered_scene_pages = tauri::async_runtime::block_on(preview_pdf_scene_pages(scene::PreviewPagesRequest { path: pdf.clone(), scene: visual_scene.clone(), page_indices: vec![1, 0], sections: Default::default() })).unwrap();
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
        outputs.extend(named(&mut naming, "export_pdf_scene", pdf.clone(), assert_success("PDF scene", run_export_pdf_scene(RunEvents::silent(), scene::ExportRequest { paths: vec![pdf.clone()], scene: visual_scene, output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default() }))));
        outputs.extend(named(&mut naming, "remove_password", protected.clone(), assert_success("unlock", remove_password(RunEvents::silent(), PasswordRequest { paths: vec![protected], password: crate::kit::TEST_PASSWORD.into(), password_overrides: Default::default(), output_location: location(&root, "unlock") }))));
        outputs.extend(named(&mut naming, "add_page_numbers", pdf.clone(), assert_success("page numbers", add_page_numbers(RunEvents::silent(), PageOverlayRequest { paths: vec![pdf.clone()], position: Some("bottom-right".into()), pages: None, start_number: Some(1), font_size: Some(12), output_location: location(&root, "page numbers") }))));
        outputs.extend(named(&mut naming, "merge_pdfs", pdf.clone(), assert_merge_success("merge", &[pdf.clone(), pdf_two.clone()], run_merge_pdfs(&MergePdfRequest { paths: vec![pdf.clone(), pdf_two.clone()], output_location: location(&root, "merge"), output_path: None }))));
        outputs.extend(named(&mut naming, "watermark_pdf", pdf.clone(), assert_success("watermark pdf", watermark_pdf(RunEvents::silent(), PdfWatermarkRequest { paths: vec![pdf.clone()], options: WatermarkOptions { text: Some("TEST".into()), logo_path: None, size: None, opacity: 70, color: None, position: Some(WatermarkPosition::Placement(WatermarkPlacement::Center)), rotation: 0.0 }, pages: None, output_location: location(&root, "watermark pdf") }))));
        outputs.extend(named(&mut naming, "crop_pdf", pdf.clone(), assert_success("crop pdf", crop_pdf(RunEvents::silent(), CropPdfRequest { paths: vec![pdf.clone()], spec: CropSpec::Margins { top: 36.0, right: 18.0, bottom: 36.0, left: 18.0 }, scope: RangedPageScope::All, output_location: location(&root, "crop pdf") }))));
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
        outputs.extend(named(&mut naming, "images_to_pdf", image.clone(), assert_aggregate_success("images to pdf", &[image.clone(), image_two.clone()], run_images_to_pdf(&ImagesToPdfRequest { layout: Default::default(), paths: vec![image.clone(), image_two.clone()], output_location: location(&root, "images to pdf"), output_path: None }))));
        outputs.extend(named(&mut naming, "pdf_to_images", pdf.clone(), assert_success("pdf to images", pdf_to_images(RunEvents::silent(), PdfToImagesRequest { paths: vec![pdf.clone()], dpi: 72, format: "png".into(), page_range: None, pages: None, picks: Vec::new(), output_location: location(&root, "pdf to images") }))));
        let selected_image_outputs = assert_success("pdf selected pages to images", pdf_to_images(RunEvents::silent(), PdfToImagesRequest { paths: vec![pdf.clone()], dpi: 72, format: "png".into(), page_range: None, pages: Some(vec![1]), picks: Vec::new(), output_location: location(&root, "pdf selected pages to images") }));
        assert_eq!(selected_image_outputs.len(), 1, "selected PDF page rendering must produce only the selected page");
        outputs.extend(named(&mut naming, "pdf_to_images", pdf.clone(), selected_image_outputs));
        if pdftotext_available() {
            outputs.extend(named(&mut naming, "extract_from_pdf", pdf.clone(), assert_success("extract text from pdf", extract_from_pdf(RunEvents::silent(), extract_request(vec![pdf.clone()], ExtractMode::Text, None, location(&root, "extract text"))))));
        }
        let converted = assert_success("convert to pdf", convert_to_pdf(RunEvents::silent(), serde_json::json!({ "paths": [text.clone()], "outputLocation": location(&root, "convert to pdf"), "plainText": { "font": "monospace", "encoding": "detect", "wrapLines": true } })));
        assert!(lopdf::Document::load(&converted[0]).unwrap().extract_text(&[1]).unwrap().contains("Caf\u{e9} notes"), "converted text must stay searchable");
        outputs.extend(named(&mut naming, "convert_to_pdf", text.clone(), converted));
        let markdown_outcomes = tauri::async_runtime::block_on(convert_to_pdf(RunEvents::silent(), serde_json::json!({ "paths": [markdown.clone()], "outputLocation": location(&root, "convert markdown to pdf"), "markdown": { "bodyFont": "sans", "codeStyle": "shaded", "tableOfContents": true } })));
        assert_eq!(markdown_outcomes[0].detail, "2 pages. Remote content left out: 1 image from the internet wasn't loaded.", "{markdown_outcomes:?}");
        let converted_markdown = markdown_outcomes[0].output_paths().to_vec();
        let markdown_pdf = lopdf::Document::load(&converted_markdown[0]).unwrap();
        assert!(markdown_pdf.extract_text(&[1]).unwrap().contains("Contents"), "the table of contents comes first");
        let markdown_text = markdown_pdf.extract_text(&[2]).unwrap();
        assert!(["Release notes", "build", "cargo test", "the site"].iter().all(|expected| markdown_text.contains(expected)), "converted Markdown must stay searchable: {markdown_text}");
        assert!(markdown_pdf.catalog().unwrap().has(b"Outlines"), "headings become bookmarks");
        outputs.extend(named(&mut naming, "convert_to_pdf", markdown.clone(), converted_markdown));
        let rich_text_outcomes = tauri::async_runtime::block_on(convert_to_pdf(RunEvents::silent(), serde_json::json!({ "paths": [rich_text.clone()], "outputLocation": location(&root, "convert rich text to pdf") })));
        assert_eq!(rich_text_outcomes[0].detail, "1 page.", "{rich_text_outcomes:?}");
        assert!(["Times New Roman", "Arial"].iter().all(|family| rich_text_outcomes[0].fonts.iter().any(|font| font.requested_family == *family)), "requested RTF fonts are reported: {rich_text_outcomes:?}");
        let converted_rich_text = rich_text_outcomes[0].output_paths().to_vec();
        let rich_text_text = lopdf::Document::load(&converted_rich_text[0]).unwrap().extract_text(&[1]).unwrap();
        assert!(["Team memo", "Ship", "release", "Owner", "Date", "Done."].iter().all(|expected| rich_text_text.contains(expected)), "converted RTF must stay searchable: {rich_text_text}");
        outputs.extend(named(&mut naming, "convert_to_pdf", rich_text.clone(), converted_rich_text));
        let word_outcomes = tauri::async_runtime::block_on(convert_to_pdf(RunEvents::silent(), serde_json::json!({ "paths": [word.clone()], "outputLocation": location(&root, "convert word to pdf") })));
        assert_eq!(word_outcomes[0].detail, "1 page.", "{word_outcomes:?}");
        let converted_word = word_outcomes[0].output_paths().to_vec();
        let word_pdf = lopdf::Document::load(&converted_word[0]).unwrap();
        let word_text = word_pdf.extract_text(&[1]).unwrap();
        assert!(["Project brief", "Scope and", "timeline", "Phase", "Week"].iter().all(|expected| word_text.contains(expected)), "converted DOCX must stay searchable: {word_text}");
        assert!(word_pdf.catalog().unwrap().has(b"Outlines"), "Word headings become bookmarks");
        outputs.extend(named(&mut naming, "convert_to_pdf", word.clone(), converted_word));
        let odt_outcomes = tauri::async_runtime::block_on(convert_to_pdf(RunEvents::silent(), serde_json::json!({ "paths": [open_document.clone()], "outputLocation": location(&root, "convert odt to pdf") })));
        assert_eq!(odt_outcomes[0].detail, "1 page.", "{odt_outcomes:?}");
        let converted_odt = odt_outcomes[0].output_paths().to_vec();
        let odt_pdf = lopdf::Document::load(&converted_odt[0]).unwrap();
        let odt_text = odt_pdf.extract_text(&[1]).unwrap();
        assert!(["Meeting minutes", "Agreed", "actions", "Owner", "Due"].iter().all(|expected| odt_text.contains(expected)), "converted ODT must stay searchable: {odt_text}");
        assert!(odt_pdf.catalog().unwrap().has(b"Outlines"), "ODT headings become bookmarks");
        outputs.extend(named(&mut naming, "convert_to_pdf", open_document.clone(), converted_odt));
        let sheet_outcomes = tauri::async_runtime::block_on(convert_to_pdf(RunEvents::silent(), serde_json::json!({ "paths": [workbook.clone()], "outputLocation": location(&root, "convert xlsx to pdf"), "spreadsheets": { "sheets": "visible", "printAreas": true, "fitToWidth": true } })));
        assert_eq!(sheet_outcomes[0].detail, "1 page. 1 sheet. 1 hidden sheet left out.", "{sheet_outcomes:?}");
        let converted_sheet = sheet_outcomes[0].output_paths().to_vec();
        let sheet_text = lopdf::Document::load(&converted_sheet[0]).unwrap().extract_text(&[1]).unwrap();
        assert!(["Quarter", "Spend", "Q1", "3"].iter().all(|expected| sheet_text.contains(expected)) && !sheet_text.contains("Scratch") && !sheet_text.contains("SUM"), "converted XLSX shows stored values of visible sheets: {sheet_text}");
        outputs.extend(named(&mut naming, "convert_to_pdf", workbook.clone(), converted_sheet));
        let table_outcomes = tauri::async_runtime::block_on(convert_to_pdf(RunEvents::silent(), serde_json::json!({ "paths": [table.clone()], "outputLocation": location(&root, "convert table to pdf"), "tables": { "delimiter": "semicolon", "headerRow": true, "encoding": "detect", "orientation": "landscape", "fit": "width" } })));
        assert_eq!(table_outcomes[0].detail, "1 page. 2 rows, semicolon. Read as UTF-8.", "{table_outcomes:?}");
        let converted_table = table_outcomes[0].output_paths().to_vec();
        let table_pdf = lopdf::Document::load(&converted_table[0]).unwrap();
        let table_text = table_pdf.extract_text(&[1]).unwrap();
        let mut rest = table_text.as_str();
        assert!(["Order", "Customer", "Note", "1001", "Smith; Jo", "said \"hi\"", "1002", "Lee", "plain"].iter().all(|expected| rest.find(expected).map(|at| rest = &rest[at + expected.len()..]).is_some()), "converted CSV cells stay in order: {table_text}");
        outputs.extend(named(&mut naming, "convert_to_pdf", table.clone(), converted_table));
        outputs.extend(named(&mut naming, "split_pdf", pdf.clone(), assert_success("split", split_pdf(RunEvents::silent(), SplitPdfRequest { paths: vec![pdf.clone()], mode: SplitMode::EveryN { pages: 1 }.into(), output_location: location(&root, "split") }))));
        outputs.extend(named(&mut naming, "extract_from_pdf", image_pdf.clone(), assert_success("extract images from pdf", extract_from_pdf(RunEvents::silent(), extract_request(vec![image_pdf.clone()], ExtractMode::Images, None, location(&root, "extract images"))))));
        outputs.extend(named(&mut naming, "sign_pdf", pdf.clone(), assert_success("sign", sign_pdf(RunEvents::silent(), SignPdfRequest { paths: vec![pdf.clone()], page: 0, text: "Signed".into(), signature_path: None, rectangle: PdfRect { x: 40.0, y: 40.0, width: 180.0, height: 60.0 }, scope: PageScope::All, output_location: location(&root, "sign") }))));
        outputs.extend(named(&mut naming, "ocr_pdf", pdf.clone(), assert_ocr_adapter("ocr", ocr_pdf(RunEvents::silent(), vision::OcrRequest { paths: vec![pdf.clone()], page_range: None, mode: "text".into(), force: false, languages: vec!["eng".into()], output_location: location(&root, "ocr") }))));
        outputs.extend(named(&mut naming, "extract_pdf_pages", pdf.clone(), assert_success("extract pages", extract_pdf_pages(RunEvents::silent(), PageSelectionRequest { paths: vec![pdf.clone()], pages: vec![], page_ranges: Some("1".into()), output_location: location(&root, "extract pages") }))));
        outputs.extend(named(&mut naming, "compress_files", compressible_pdf.clone(), assert_success("compress files", compress_files(RunEvents::silent(), serde_json::from_value::<CompressFilesRequest>(serde_json::json!({ "paths": [compressible_pdf.clone()], "level": "medium", "stripExif": false, "outputLocation": location(&root, "compress files") })).expect("Compress Files view payload for compress_files")))));
        outputs.extend(named(&mut naming, "rotate_pdf", pdf.clone(), assert_success("rotate pdf", rotate_pdf(RunEvents::silent(), RotatePdfRequest { paths: vec![pdf.clone()], degrees: 90, pages: Some(vec![1]), output_location: location(&root, "rotate pdf") }))));
        outputs.extend(named(&mut naming, "repair_pdf", pdf.clone(), assert_success("repair pdf", repair_pdf(RunEvents::silent(), RepairPdfRequest { paths: vec![pdf.clone()], output_location: location(&root, "repair pdf") }))));
        outputs.extend(named(&mut naming, "redact_pdf", pdf.clone(), assert_success("redact pdf", redact_pdf(RunEvents::silent(), RedactPdfRequest { paths: vec![pdf.clone()], rectangle: PdfRect { x: 40.0, y: 650.0, width: 220.0, height: 60.0 }, scope: PageScope::All, output_location: location(&root, "redact pdf") }))));
        outputs.extend(named(&mut naming, "compare_pdfs", pdf.clone(), assert_aggregate_success("compare pdfs", &[pdf.clone(), pdf_two.clone()], run_compare_pdfs(&ComparePdfsRequest { paths: vec![pdf.clone(), pdf_two.clone()], output_location: location(&root, "compare pdfs"), output_path: None }))));
        let entries_before_inspection = std::fs::read_dir(&root).unwrap().count();
        let privacy_reports = tauri::async_runtime::block_on(inspect_pdf_privacy(RunEvents::silent(), privacy::PrivacyRequest { paths: vec![pdf.clone(), pdf_two.clone()], passwords: Default::default() }));
        assert_eq!(privacy_reports.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), [pdf.clone(), pdf_two.clone()]);
        for (outcome, pages) in privacy_reports.iter().zip([2, 1]) {
            assert!(outcome.error().is_none(), "privacy check failed: {:?}", outcome.error());
            assert!(outcome.output_paths().is_empty(), "privacy check reports name no output");
            let report: serde_json::Value = serde_json::from_str(&outcome.detail).expect("privacy check detail is its JSON report");
            assert_eq!(report["pageCount"], pages);
        }
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), entries_before_inspection, "privacy check must create nothing");
        named(&mut naming, "inspect_pdf_privacy", pdf.clone(), Vec::new());
        let current = tauri::async_runtime::block_on(read_pdf_properties(RunEvents::silent(), properties::ReadPropertiesRequest { paths: vec![pdf.clone()], passwords: Default::default() }));
        assert!(current[0].error().is_none() && current[0].output_paths().is_empty(), "reading properties saves nothing: {:?}", current[0]);
        let edit_request = serde_json::from_value::<properties::EditPropertiesRequest>(serde_json::json!({ "paths": [pdf.clone()], "patch": { "title": { "action": "set", "value": "Sweep title" }, "author": { "action": "clear" }, "creationDate": { "action": "set", "value": "2026-02-28T09:00:00+05:30" }, "modificationDate": { "action": "clear" } }, "outputLocation": location(&root, "edit pdf properties") })).expect("Edit Properties view payload");
        outputs.extend(named(&mut naming, "edit_pdf_properties", pdf.clone(), assert_success("edit pdf properties", edit_pdf_properties(RunEvents::silent(), edit_request))));
        let pdf_bytes = std::fs::read(&pdf).unwrap();
        let reports = privacy_reports.iter().map(|outcome| serde_json::json!({ "inputPath": outcome.input_path, "report": serde_json::from_str::<serde_json::Value>(&outcome.detail).unwrap() })).collect::<Vec<_>>();
        let save_request = serde_json::from_value::<privacy::SaveReportsRequest>(serde_json::json!({ "reports": reports, "format": "json", "outputLocation": location(&root, "privacy reports") })).expect("Privacy Check save payload");
        let saved_reports = tauri::async_runtime::block_on(save_privacy_reports(save_request));
        let saved_names = saved_reports.iter().map(|outcome| outcome.output_paths().first().and_then(|path| path.file_name()).map(|name| name.to_string_lossy().into_owned())).collect::<Vec<_>>();
        let stems = [&pdf, &pdf_two].map(|path| Some(format!("{}-details.json", path.file_stem().unwrap().to_string_lossy())));
        assert_eq!(saved_names, stems, "privacy reports save as one -details file per PDF");
        assert_eq!(std::fs::read(&pdf).unwrap(), pdf_bytes, "saving a report must not touch its PDF");
        outputs.extend(named(&mut naming, "convert_images", image.clone(), assert_success("convert", convert_images(RunEvents::silent(), ConvertImagesRequest { paths: vec![image.clone()], format: "jpg".into(), output_location: location(&root, "convert") }))));
        outputs.extend(named(&mut naming, "compress_images", image.clone(), assert_success("compress images", compress_images(RunEvents::silent(), serde_json::from_value::<CompressImagesRequest>(serde_json::json!({ "paths": [image.clone()], "quality": 80, "lossless": false, "outputLocation": location(&root, "compress images") })).expect("Compress Files view payload for compress_images")))));
        outputs.extend(named(&mut naming, "resize_images", image.clone(), assert_success("resize", resize_images(RunEvents::silent(), ResizeRequest { paths: vec![image.clone()], width: 128, height: 128, mode: "exact".into(), resampling: "lanczos".into(), keep_aspect_ratio: true, percentage: 100, longest_side: 128, output_location: location(&root, "resize") }))));
        outputs.extend(named(&mut naming, "rotate_images", image.clone(), assert_success("rotate", rotate_images(RunEvents::silent(), RotateRequest { paths: vec![image.clone()], degrees: 90, flip: "none".into(), output_location: location(&root, "rotate") }))));
        outputs.extend(named(&mut naming, "crop_images", image.clone(), assert_success("crop image", crop_images(RunEvents::silent(), CropRequest { paths: vec![image.clone()], crop: CropStep { x: 0, y: 0, width: 128, height: 128, mode: "rectangle".into(), aspect_width: 128, aspect_height: 128, anchor: "center".into() }, output_location: location(&root, "crop image") }))));
        outputs.extend(named(&mut naming, "export_image_edit_stack", image.clone(), assert_success("edit stack crop", export_image_edit_stack(RunEvents::silent(), ImageEditStackRequest { paths: vec![image.clone()], steps: ImageEditSteps { crop: Some(CropStep { x: 0, y: 0, width: 0, height: 0, mode: "aspectRatio".into(), aspect_width: 4, aspect_height: 5, anchor: "center".into() }) }, output_location: location(&root, "edit stack crop") }))));
        outputs.extend(named(&mut naming, "generate_icon_set", image.clone(), assert_success("icons", generate_icon_set(RunEvents::silent(), IconSetRequest { paths: vec![image.clone()], preset: "favicon".into(), sizes: vec![], output_location: location(&root, "icons") }))));
        outputs.extend(named(&mut naming, "create_gif", image.clone(), assert_aggregate_success("create gif", &[image.clone(), image_two.clone()], run_create_gif(&GifCreateRequest { paths: vec![image.clone(), image_two.clone()], frame_delay_ms: 100, loop_forever: true, output_location: location(&root, "create gif"), output_path: None }))));
        outputs.extend(named(&mut naming, "extract_gif_frames", animated.clone(), assert_success("extract gif", extract_gif_frames(RunEvents::silent(), GifExtractRequest { paths: vec![animated.clone()], output_location: location(&root, "extract gif") }))));
        outputs.extend(named(&mut naming, "watermark_images", image.clone(), assert_success("watermark image", watermark_images(RunEvents::silent(), WatermarkRequest { paths: vec![image.clone()], options: WatermarkOptions { text: Some("TEST".into()), logo_path: None, size: None, opacity: 70, color: None, position: Some(WatermarkPosition::Offset { x: 16, y: 16 }), rotation: 0.0 }, output_location: location(&root, "watermark image") }))));
        let image_plan = ImageEditPlan {
            edits: vec![
                ImageEdit::Resize { width: 128, height: 128, mode: "exact".into(), percentage: 100, longest_side: 128, resampling: "lanczos".into(), keep_aspect_ratio: false },
                ImageEdit::Crop { x: 0, y: 10, width: 16, height: 9, mode: "aspectRatio".into(), aspect_width: 16, aspect_height: 9, anchor: "custom".into() },
                ImageEdit::Rotate { degrees: 90, flip: "horizontal".into() },
                ImageEdit::Tone { brightness: 12, contrast: 6.0, saturation: 8.0, exposure: 4.0 },
            ],
            output_location: location(&root, "composed image editor"),
        };
        let image_preview = inspect_image_edit_preview(ImageEditPreviewRequest { path: image.clone(), plan: image_plan.clone() }).expect("composed image preview should be available");
        assert_eq!((image_preview.width, image_preview.height), (72, 128), "composed image preview must reflect the complete edit plan");
        let preview_bytes = STANDARD.decode(image_preview.data_url.split_once(',').expect("preview must contain a data URL payload").1).unwrap();
        let preview_pixels = image::load_from_memory(&preview_bytes).unwrap().to_rgba8();
        let composed_image_outputs = assert_success("composed image editor", export_image_edit_plan(RunEvents::silent(), ImageEditExportRequest { paths: vec![image.clone()], plan: image_plan }));
        let exported_pixels = image::open(&composed_image_outputs[0]).unwrap().to_rgba8();
        assert_eq!(exported_pixels.dimensions(), preview_pixels.dimensions(), "exported image dimensions must match the verified edit preview");
        let differing_pixels = exported_pixels.pixels().zip(preview_pixels.pixels()).enumerate()
            .filter_map(|(index, (exported, previewed))| (exported.0 != previewed.0).then_some((index, exported.0, previewed.0)))
            .take(5)
            .collect::<Vec<_>>();
        assert!(differing_pixels.is_empty(), "native image export pixels must match the verified edit preview; first differences: {differing_pixels:?}");
        outputs.extend(named(&mut naming, "export_image_edit_plan", image.clone(), composed_image_outputs));
        let lossy_image_plan = ImageEditPlan {
            edits: vec![ImageEdit::Compress { quality: 40, lossless: false }],
            output_location: location(&root, "lossy image editor"),
        };
        let lossy_image_preview = inspect_image_edit_preview(ImageEditPreviewRequest { path: lossy_image.clone(), plan: lossy_image_plan.clone() }).expect("lossy image preview should be available");
        let lossy_preview_bytes = STANDARD.decode(lossy_image_preview.data_url.split_once(',').expect("lossy preview must contain a data URL payload").1).unwrap();
        let lossy_preview_pixels = image::load_from_memory(&lossy_preview_bytes).unwrap().to_rgba8();
        let lossy_image_outputs = assert_success("lossy composed image editor", export_image_edit_plan(RunEvents::silent(), ImageEditExportRequest { paths: vec![lossy_image.clone()], plan: lossy_image_plan }));
        let lossy_export_pixels = image::open(&lossy_image_outputs[0]).unwrap().to_rgba8();
        assert_eq!(lossy_export_pixels.dimensions(), lossy_preview_pixels.dimensions(), "lossy export dimensions must match the verified edit preview");
        let lossy_differing_pixels = lossy_export_pixels.pixels().zip(lossy_preview_pixels.pixels()).enumerate()
            .filter_map(|(index, (exported, previewed))| (exported.0 != previewed.0).then_some((index, exported.0, previewed.0)))
            .take(5)
            .collect::<Vec<_>>();
        assert!(lossy_differing_pixels.is_empty(), "lossy image export pixels must match the verified edit preview; first differences: {lossy_differing_pixels:?}");
        outputs.extend(named(&mut naming, "export_image_edit_plan", lossy_image.clone(), lossy_image_outputs));
        outputs.extend(named(&mut naming, "image_metadata", image.clone(), assert_success("metadata", image_metadata(RunEvents::silent(), MetadataRequest { paths: vec![image.clone()], output_location: location(&root, "metadata") }))));
        outputs.extend(named(&mut naming, "adjust_image_tone", image.clone(), assert_success("tone", adjust_image_tone(RunEvents::silent(), ToneRequest { paths: vec![image.clone()], brightness: 20, contrast: 0.0, saturation: 0.0, exposure: 0.0, output_location: location(&root, "tone") }))));
        outputs.extend(named(&mut naming, "process_tiff_pages", tiff.clone(), assert_aggregate_success("tiff", &[tiff.clone()], run_combine_tiff_pages(&TiffRequest { mode: TiffMode::Combine, paths: vec![tiff.clone()], pages: None, output_location: location(&root, "tiff"), output_path: None }))));
        outputs.extend(named(&mut naming, "blur_faces", image.clone(), assert_vision_adapter("face blur", blur_faces(RunEvents::silent(), VisionRequest { paths: vec![image.clone()], output_location: location(&root, "face blur") }))));
        outputs.extend(named(&mut naming, "remove_image_background", image.clone(), assert_vision_adapter("background removal", remove_image_background(RunEvents::silent(), VisionRequest { paths: vec![image.clone()], output_location: location(&root, "background removal") }))));
        let docx = root.join("document.docx");
        std::fs::write(&docx, test_minimal_ooxml_package("docx")).unwrap();
        let docx_bytes = std::fs::read(&docx).unwrap();
        let locked_docx = assert_success("lock word", protect_files(RunEvents::silent(), PasswordRequest { paths: vec![docx.clone()], password: crate::kit::TEST_PASSWORD.into(), password_overrides: Default::default(), output_location: location(&root, "lock word") }));
        assert_eq!(locked_docx[0].file_name().and_then(|name| name.to_str()), Some("document-protected.docx"));
        assert_eq!(std::fs::read(&docx).unwrap(), docx_bytes, "native E2E commands must not modify their Word input");
        assert_outputs_use_declared_suffixes(&naming);

        assert_eq!(std::fs::read(&pdf).unwrap(), plain_pdf_bytes, "native E2E commands must not modify their PDF input");
        assert_eq!(std::fs::read(&image).unwrap(), plain_image_bytes, "native E2E commands must not modify their image input");
        assert_eq!(std::fs::read(&lossy_image).unwrap(), plain_lossy_image_bytes, "native E2E commands must not modify their lossy image input");
        assert_eq!(std::fs::read(&animated).unwrap(), animated_bytes, "native E2E commands must not modify their GIF input");
        assert_eq!(std::fs::read(&tiff).unwrap(), tiff_bytes, "native E2E commands must not modify their TIFF input");
        assert_eq!(std::fs::read(&text).unwrap(), text_bytes, "native E2E commands must not modify their text input");
        assert_eq!(std::fs::read(&markdown).unwrap(), markdown_bytes, "native E2E commands must not modify their Markdown input");
        assert_eq!(std::fs::read(&rich_text).unwrap(), rich_text_bytes, "native E2E commands must not modify their RTF input");
        assert_eq!(std::fs::read(&table).unwrap(), table_bytes, "native E2E commands must not modify their CSV input");
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

    fn convert_batch(paths: &[PathBuf], format: &str, output_location: OutputLocation) -> Vec<JobOutcome> {
        tauri::async_runtime::block_on(convert_images(RunEvents::silent(), ConvertImagesRequest {
            paths: paths.to_vec(), format: format.to_string(), output_location,
        }))
    }

    fn skip_message(outcome: &JobOutcome) -> Option<&str> {
        match &outcome.state {
            OutcomeState::Skipped { reason } if reason.code == crate::kit::contracts::SkipCode::AlreadyInTargetFormat => Some(reason.message.as_str()),
            _ => None,
        }
    }

    fn decoded_as(bytes: &[u8], format: &str) -> (u32, u32) {
        if format == "heic" {
            let image = heif::decode(bytes).expect("output decodes as HEIC");
            return (image.width(), image.height());
        }
        let expected = match format { "jpg" => image::ImageFormat::Jpeg, "png" => image::ImageFormat::Png, _ => image::ImageFormat::WebP };
        assert_eq!(image::guess_format(bytes).unwrap(), expected, "output is real {format}");
        let image = image::load_from_memory_with_format(bytes, expected).unwrap();
        (image.width(), image.height())
    }

    fn convert_sources(root: &PathBuf) -> Vec<PathBuf> {
        let picture = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([x as u8 * 4, y as u8 * 5, (x ^ y) as u8])));
        let heic = root.join("d.heic");
        std::fs::write(&heic, crate::kit::images::encode(&picture, crate::kit::images::OutputFormat::Heic, 80).unwrap()).unwrap();
        let mut sources = Vec::new();
        for name in ["a.png", "b.jpg", "c.webp"] {
            let path = root.join(name);
            picture.save(&path).unwrap();
            sources.push(path);
        }
        sources.push(heic);
        sources
    }

    #[test]
    fn edit_stack_crops_a_mixed_batch_through_one_encode_per_file() {
        let root = sandbox("edit_stack_batch");
        let png = root.join("wide.png");
        let jpeg = root.join("tall.jpg");
        let webp = root.join("square.webp");
        let heic = root.join("photo.heic");
        let small = root.join("small.png");
        let corrupt = root.join("corrupt.jpg");
        let pattern = |width: u32, height: u32, seed: u8| image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(width, height, |x, y| Rgba([seed, (x * 3) as u8, (y * 5) as u8, 255])));
        pattern(240, 120, 1).save(&png).unwrap();
        pattern(90, 200, 2).to_rgb8().save(&jpeg).unwrap();
        std::fs::write(&webp, crate::kit::images::encode(&pattern(160, 160, 3), crate::kit::images::OutputFormat::WebP, 90).unwrap()).unwrap();
        std::fs::write(&heic, crate::kit::images::encode(&pattern(128, 96, 4), crate::kit::images::OutputFormat::Heic, 90).unwrap()).unwrap();
        pattern(40, 40, 5).save(&small).unwrap();
        std::fs::write(&corrupt, b"not a jpeg").unwrap();
        let inputs = vec![png.clone(), jpeg.clone(), small.clone(), webp.clone(), corrupt.clone(), heic.clone()];
        let originals = inputs.iter().map(|path| (Original::of(path), std::fs::read(path).unwrap())).collect::<Vec<_>>();
        let rectangle = CropStep { x: 10, y: 20, width: 64, height: 48, mode: "rectangle".into(), aspect_width: 0, aspect_height: 0, anchor: "custom".into() };
        let run = |folder: &str| tauri::async_runtime::block_on(export_image_edit_stack(RunEvents::silent(), ImageEditStackRequest {
            paths: inputs.clone(), steps: ImageEditSteps { crop: Some(rectangle.clone()) }, output_location: OutputLocation::CustomFolder(root.join(folder)),
        }));
        std::fs::create_dir_all(root.join("out")).unwrap();
        std::fs::create_dir_all(root.join("again")).unwrap();

        let first = run("out");
        assert_eq!(first.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs, "one outcome per file, in input order");
        let small_error = first[2].error().expect("a rectangle outside the small image fails only that file");
        assert!(small_error.message.contains("fit inside"), "{}", small_error.message);
        assert!(first[4].error().is_some(), "the unreadable file fails on its own");
        assert!(first[2].output_paths().is_empty() && first[4].output_paths().is_empty());
        let expected_names = [("wide.png", "wide-cropped.png"), ("tall.jpg", "tall-cropped.jpg"), ("square.webp", "square-cropped.webp"), ("photo.heic", "photo-cropped.heic")];
        let successes = [&first[0], &first[1], &first[3], &first[5]];
        for (outcome, (_, expected)) in successes.iter().zip(expected_names) {
            assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
            assert_eq!(outcome.output_paths(), [root.join("out").join(expected)]);
        }
        for path in [&first[0].output_paths()[0], &first[1].output_paths()[0], &first[3].output_paths()[0]] {
            let cropped = image::open(path).unwrap();
            assert_eq!((cropped.width(), cropped.height()), (64, 48), "{} keeps the source-space crop", path.display());
        }
        assert_eq!(image::guess_format(&std::fs::read(&first[0].output_paths()[0]).unwrap()).unwrap(), image::ImageFormat::Png);
        assert_eq!(image::guess_format(&std::fs::read(&first[1].output_paths()[0]).unwrap()).unwrap(), image::ImageFormat::Jpeg);
        assert_eq!(&std::fs::read(&first[3].output_paths()[0]).unwrap()[..4], b"RIFF");
        let heic_output = heif::decode(&std::fs::read(&first[5].output_paths()[0]).unwrap()).expect("HEIC input stays HEIC");
        assert_eq!((heic_output.width(), heic_output.height()), (64, 48));

        let jpeg_source = crate::kit::images::decode(&jpeg).unwrap().into_first_page().crop_imm(10, 20, 64, 48);
        assert_eq!(std::fs::read(&first[1].output_paths()[0]).unwrap(), crate::kit::images::encode(&jpeg_source, crate::kit::images::OutputFormat::Jpeg, 90).unwrap(), "JPEG is decoded and encoded once at quality 90");

        let repeated = run("out");
        for (index, expected) in [(0, "wide-cropped-1.png"), (1, "tall-cropped-1.jpg"), (3, "square-cropped-1.webp"), (5, "photo-cropped-1.heic")] {
            assert_eq!(repeated[index].output_paths(), [root.join("out").join(expected)], "a second run never overwrites");
        }
        let elsewhere = run("again");
        for index in [0, 1, 3, 5] {
            assert_eq!(std::fs::read(&first[index].output_paths()[0]).unwrap(), std::fs::read(&elsewhere[index].output_paths()[0]).unwrap(), "the same stack gives the same bytes");
        }
        for (original, bytes) in &originals {
            original.assert_untouched();
            assert_eq!(&std::fs::read(&original.path).unwrap(), bytes);
        }
        let empty = tauri::async_runtime::block_on(export_image_edit_stack(RunEvents::silent(), ImageEditStackRequest {
            paths: vec![png.clone()], steps: ImageEditSteps::default(), output_location: OutputLocation::CustomFolder(root.join("out")),
        }));
        assert!(empty[0].error().is_some_and(|error| matches!(error.kind, ErrorKind::InvalidInput)), "an empty stack never writes");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn convert_batch_converts_every_cross_format_pair_into_a_custom_folder() {
        let root = sandbox("convert-batch-pairs");
        let sources = convert_sources(&root);
        let originals: Vec<Vec<u8>> = sources.iter().map(|path| std::fs::read(path).unwrap()).collect();
        let mut converted = 0;
        for format in ["jpg", "png", "webp", "heic"] {
            let folder = root.join(format!("out-{format}"));
            let outcomes = convert_batch(&sources, format, location(&root, &format!("out-{format}")));
            assert_eq!(outcomes.len(), sources.len());
            for (source, outcome) in sources.iter().zip(&outcomes) {
                let name = source.file_name().unwrap().to_string_lossy();
                assert_eq!(outcome.input_path, *source);
                if source.extension().unwrap() == format {
                    assert!(skip_message(outcome).is_some(), "{name} to {format} is skipped");
                    assert!(outcome.output_paths().is_empty());
                    continue;
                }
                assert!(outcome.error().is_none(), "{name} to {format}: {}", outcome.error().cloned().unwrap_or_default());
                let stem = source.file_stem().unwrap().to_string_lossy();
                assert_eq!(outcome.output_paths(), [folder.join(format!("{stem}-converted.{format}"))]);
                assert_eq!(decoded_as(&std::fs::read(&outcome.output_paths()[0]).unwrap(), format), (64, 48), "{name} to {format} keeps its size");
                converted += 1;
            }
            assert_eq!(std::fs::read_dir(&folder).unwrap().count(), sources.len() - 1, "only converted files land in {format}");
        }
        assert_eq!(converted, 12, "every cross-format pair among HEIC, JPEG, PNG and WebP converts");
        for (source, original) in sources.iter().zip(&originals) {
            assert_eq!(&std::fs::read(source).unwrap(), original, "{} stays untouched", source.display());
        }
        assert_eq!(std::fs::read_dir(&root).unwrap().filter(|entry| entry.as_ref().unwrap().path().is_file()).count(), sources.len(), "nothing is written next to the originals");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn convert_skips_files_already_in_the_target_family_without_writing() {
        let root = sandbox("convert-skip-family");
        let sources = convert_sources(&root);
        let jpeg = root.join("e.jpeg");
        let heif = root.join("f.heif");
        std::fs::copy(&sources[1], &jpeg).unwrap();
        std::fs::copy(&sources[3], &heif).unwrap();
        let cases = [
            (&sources[0], "png", "Already PNG, so no file was made."),
            (&sources[1], "jpg", "Already JPEG, so no file was made."),
            (&jpeg, "jpg", "Already JPEG. JPG and JPEG count as one format."),
            (&jpeg, "jpeg", "Already JPEG. JPG and JPEG count as one format."),
            (&sources[2], "webp", "Already WebP, so no file was made."),
            (&sources[3], "heic", "Already HEIC, so no file was made."),
            (&heif, "heic", "Already HEIC. HEIC and HEIF count as one format."),
        ];
        for (index, (source, format, message)) in cases.into_iter().enumerate() {
            let original = std::fs::read(source).unwrap();
            let folder = format!("out-{index}");
            let outcomes = convert_batch(&[source.clone()], format, location(&root, &folder));
            assert_eq!(skip_message(&outcomes[0]), Some(message), "{} to {format}", source.display());
            assert!(outcomes[0].output_paths().is_empty());
            assert_eq!(std::fs::read_dir(root.join(&folder)).unwrap().count(), 0, "a skipped file makes no output");
            assert_eq!(std::fs::read(source).unwrap(), original);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn convert_encodes_lossy_targets_at_the_shared_quality_of_90() {
        let root = sandbox("convert-quality");
        let source = convert_sources(&root).remove(0);
        let picture = image::open(&source).unwrap();
        for (format, target) in [("jpg", crate::kit::images::OutputFormat::Jpeg), ("webp", crate::kit::images::OutputFormat::WebP)] {
            let outcomes = convert_batch(&[source.clone()], format, location(&root, format));
            let bytes = std::fs::read(&outcomes[0].output_paths()[0]).unwrap();
            assert_eq!(bytes, crate::kit::images::encode(&picture, target, crate::kit::images::DEFAULT_QUALITY).unwrap(), "{format} uses quality 90");
            assert_ne!(bytes, crate::kit::images::encode(&picture, target, 80).unwrap(), "{format} no longer uses quality 80");
        }
        assert_eq!(crate::kit::images::DEFAULT_QUALITY, 90);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn convert_fails_an_unreadable_image_alone() {
        let root = sandbox("convert-unreadable");
        let sources = convert_sources(&root);
        let broken = root.join("broken.jpg");
        std::fs::write(&broken, b"not an image").unwrap();
        let batch = vec![sources[0].clone(), broken.clone(), sources[2].clone()];
        let outcomes = convert_batch(&batch, "heic", location(&root, "out"));
        assert_eq!(outcomes.len(), 3);
        assert!(outcomes[0].error().is_none() && outcomes[2].error().is_none(), "the readable images still convert");
        assert_eq!(outcomes[1].input_path, broken);
        assert!(outcomes[1].error().is_some(), "the unreadable image fails");
        assert!(outcomes[1].output_paths().is_empty());
        let mut written: Vec<String> = std::fs::read_dir(root.join("out")).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        written.sort();
        assert_eq!(written, ["a-converted.heic", "c-converted.heic"]);
        assert_eq!(std::fs::read(&broken).unwrap(), b"not an image");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn convert_numbers_outputs_instead_of_replacing_existing_files() {
        let root = sandbox("convert-collision");
        let source = convert_sources(&root).remove(0);
        let folder = root.join("out");
        let existing = folder.join("a-converted.jpg");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(&existing, b"keep me").unwrap();
        for expected in ["a-converted-1.jpg", "a-converted-2.jpg"] {
            let outcomes = convert_batch(&[source.clone()], "jpg", OutputLocation::CustomFolder(folder.clone()));
            assert_eq!(outcomes[0].output_paths(), [folder.join(expected)]);
        }
        assert_eq!(std::fs::read(&existing).unwrap(), b"keep me");
        let _ = std::fs::remove_dir_all(root);
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

        let (outcomes, events) = recorded(|events| protect_files(events, PasswordRequest {
            paths: inputs.clone(),
            password: "secret".into(),
            password_overrides: Default::default(),
            output_location: location(&root, "protected"),
        }));

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs);
        assert!(outcomes[0].error().is_none() && outcomes[3].error().is_none());
        assert!(outcomes[1].error().is_some());
        assert!(outcomes[2].error().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput)));
        assert_every_file_streams(&events, &outcomes);
        for (path, original) in inputs.iter().zip(originals) { assert_eq!(std::fs::read(path).unwrap(), original); }
        assert_eq!(std::fs::read_dir(root.join("protected")).unwrap().count(), 2);
    }

    // Runs a command with a sink that records every per-file event it emits.
    fn recorded<F>(command: impl FnOnce(RunEvents) -> F) -> (Vec<JobOutcome>, Vec<FileEvent>)
    where F: Future<Output = Vec<JobOutcome>> {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let outcomes = tauri::async_runtime::block_on(command(RunEvents::new(move |event| sink.lock().unwrap().push(event))));
        let events = seen.lock().unwrap().clone();
        (outcomes, events)
    }

    // Each file starts once, then finishes once with the same outcome the ordered response returns.
    fn assert_every_file_streams(events: &[FileEvent], outcomes: &[JobOutcome]) {
        assert_eq!(events.len(), outcomes.len() * 2, "two events per file and nothing else");
        for index in 0..outcomes.len() {
            let started = events.iter().filter(|event| **event == FileEvent::Started { index }).count();
            assert_eq!(started, 1, "file {index} starts exactly once");
            let started_at = events.iter().position(|event| *event == FileEvent::Started { index }).unwrap();
            let finished = events.iter().enumerate().filter_map(|(position, event)| match event {
                FileEvent::Finished { index: finished, outcome } if *finished == index => Some((position, outcome)),
                _ => None,
            }).collect::<Vec<_>>();
            assert_eq!(finished.len(), 1, "file {index} finishes exactly once");
            assert!(started_at < finished[0].0, "file {index} starts before it finishes");
            assert_eq!(finished[0].1, &outcomes[index], "the streamed outcome for file {index} is the final outcome");
        }
    }

    #[test]
    fn scene_export_streams_page_progress_between_start_and_finish() {
        let root = sandbox("scene-export-progress");
        let pdf = root.join("document.pdf"); make_pdf(&pdf, 2);
        let scene: scene::PdfScene = serde_json::from_value(serde_json::json!({ "pages": [
            { "sourceIndex": 1, "width": 612, "height": 792, "rotation": 90, "crop": null, "objects": [] },
            { "sourceIndex": null, "width": 612, "height": 792, "rotation": 0, "crop": null, "objects": [] },
            { "sourceIndex": 0, "width": 612, "height": 792, "rotation": 0, "crop": null, "objects": [] }
        ] })).unwrap();
        let (outcomes, events) = recorded(|events| run_export_pdf_scene(events, scene::ExportRequest { paths: vec![pdf.clone()], scene, output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default() }));
        assert!(outcomes[0].error().is_none(), "{:?}", outcomes[0].error());
        let progress = events.iter().filter_map(|event| match event {
            FileEvent::Progress { index, completed, total } => Some((*index, *completed, *total)),
            _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(progress, [(0, 0, 3), (0, 1, 3), (0, 2, 3), (0, 3, 3)]);
        assert_eq!(events.first(), Some(&FileEvent::Started { index: 0 }));
        assert!(matches!(events.last(), Some(FileEvent::Finished { index: 0, .. })));
    }

    #[test]
    fn lock_and_unlock_stream_a_progress_event_for_every_file_in_a_mixed_batch() {
        use crate::kit::{TEST_OVERRIDE_PASSWORD, TEST_PASSWORD};
        let root = sandbox("lock-unlock-events");
        let mut lock_inputs = mixed_lock_batch(&root);
        let broken = root.join("broken.pdf");
        let legacy = root.join("minutes.doc");
        std::fs::write(&broken, b"not a pdf").unwrap();
        std::fs::write(&legacy, b"legacy word file").unwrap();
        lock_inputs.extend([broken.clone(), legacy.clone()]);
        let lock_originals = lock_inputs.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let (locked, lock_events) = recorded(|events| protect_files(events, PasswordRequest {
            paths: lock_inputs.clone(),
            password: TEST_PASSWORD.into(),
            password_overrides: Default::default(),
            output_location: location(&root, "protected"),
        }));

        assert_eq!(locked.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), lock_inputs);
        assert_every_file_streams(&lock_events, &locked);
        for outcome in &locked[..4] { assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default()); }
        assert!(locked[4].error().is_some() && locked[5].error().is_some(), "the broken and legacy files fail on their own");
        for (path, original) in lock_inputs.iter().zip(&lock_originals) { assert_eq!(&std::fs::read(path).unwrap(), original, "original was modified: {}", path.display()); }

        let own = locked_pdf(&root, "own", TEST_OVERRIDE_PASSWORD);
        let plain = root.join("plain.pdf");
        make_pdf(&plain, 1);
        let mut unlock_inputs = locked[..4].iter().map(|outcome| outcome.output_paths()[0].clone()).collect::<Vec<_>>();
        unlock_inputs.extend([own.clone(), plain.clone(), broken.clone()]);
        let unlock_originals = unlock_inputs.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let (unlocked, unlock_events) = recorded(|events| remove_password(events, PasswordRequest {
            paths: unlock_inputs.clone(),
            password: TEST_PASSWORD.into(),
            password_overrides: overrides(&[(&own, "not-the-password")]),
            output_location: location(&root, "unlocked"),
        }));

        assert_eq!(unlocked.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), unlock_inputs);
        assert_every_file_streams(&unlock_events, &unlocked);
        for outcome in &unlocked[..4] {
            assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
            assert!(opens_without_password(&outcome.output_paths()[0]));
        }
        assert!(unlocked[4].error().is_some_and(|error| error.kind == ErrorKind::WrongPassword), "a wrong override fails only its own file");
        assert!(matches!(unlocked[5].state, crate::kit::contracts::OutcomeState::Skipped { .. }), "an unprotected file is skipped");
        assert!(unlocked[6].error().is_some(), "a broken file fails on its own");
        for (path, original) in unlock_inputs.iter().zip(&unlock_originals) { assert_eq!(&std::fs::read(path).unwrap(), original, "input was modified: {}", path.display()); }
        assert_eq!(file_names(&root.join("unlocked")).len(), 4, "only the unlocked files are written");
        let _ = std::fs::remove_dir_all(root);
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
    fn cropping_a_signed_pdf_removes_the_signature_and_warns_on_that_file_only() {
        use crate::kit::contracts::{OutcomeWarning, WarningKind};
        let root = sandbox("crop-signed");
        let signed = root.join("vendor-nda.pdf");
        let certified = root.join("certified.pdf");
        let plain = root.join("lease.pdf");
        for path in [&signed, &certified, &plain] {
            make_pdf(path, 1);
        }
        let mut document = lopdf::Document::load(&signed).unwrap();
        crate::kit::pdf::tests::sign(&mut document);
        document.save(&signed).unwrap();
        let mut document = lopdf::Document::load(&certified).unwrap();
        let signature = document.add_object(lopdf::dictionary! { "Type" => "Sig" });
        document.catalog_mut().unwrap().set("Perms", lopdf::dictionary! { "DocMDP" => signature });
        document.save(&certified).unwrap();
        let inputs = vec![signed.clone(), certified.clone(), plain.clone()];
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();

        let outcomes = inputs.iter().flat_map(|input| tauri::async_runtime::block_on(crop_pdf(RunEvents::silent(), CropPdfRequest {
            paths: vec![input.clone()],
            spec: CropSpec::Rectangle { rectangle: PdfRect { x: 10.0, y: 10.0, width: 300.0, height: 400.0 } },
            scope: RangedPageScope::All,
            output_location: location(&root, "cropped"),
        }))).collect::<Vec<_>>();

        let removed = vec![OutcomeWarning { kind: WarningKind::SignatureRemoved, message: crate::kit::pdf::SIGNATURE_REMOVED.into() }];
        for outcome in &outcomes {
            assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
            assert_eq!(outcome.output_paths().len(), 1);
            let output = lopdf::Document::load(&outcome.output_paths()[0]).unwrap();
            assert!(!crate::kit::pdf::has_signed_signature_field(&output));
            let catalog = output.catalog().unwrap();
            assert!(catalog.get(b"Perms").is_err() && catalog.get(b"AcroForm").is_err());
            assert!(!output.objects.values().any(|object| object.as_dict().or_else(|_| object.as_stream().map(|stream| &stream.dict))
                .is_ok_and(|dictionary| dictionary.has(b"ByteRange") || dictionary.get(b"Type").and_then(|value| value.as_name()).is_ok_and(|name| name == b"Sig") || dictionary.has(b"FT"))));
            let page = output.get_dictionary(*output.get_pages().values().next().unwrap()).unwrap();
            assert!(page.get(b"Annots").is_err());
            assert_eq!(page.get(b"CropBox").unwrap().as_array().unwrap().iter().map(|value| value.as_float().unwrap()).collect::<Vec<_>>(), [10.0, 10.0, 310.0, 410.0]);
        }
        assert_eq!(outcomes.iter().map(|outcome| outcome.warnings.clone()).collect::<Vec<_>>(), [removed.clone(), removed, vec![]]);
        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn tools_that_do_not_opt_in_still_reject_signed_pdfs_before_output() {
        let root = sandbox("signed-not-opted-in");
        let signed = root.join("vendor-nda.pdf");
        make_pdf(&signed, 1);
        let mut document = lopdf::Document::load(&signed).unwrap();
        crate::kit::pdf::tests::sign(&mut document);
        document.save(&signed).unwrap();
        let original = std::fs::read(&signed).unwrap();
        let rectangle = PdfRect { x: 10.0, y: 10.0, width: 100.0, height: 100.0 };
        let paths = vec![signed.clone()];
        let runs = [
            ("sign", tauri::async_runtime::block_on(sign_pdf(RunEvents::silent(), SignPdfRequest { paths: paths.clone(), page: 0, text: "Signed".into(), signature_path: None, rectangle: rectangle.clone(), scope: PageScope::All, output_location: location(&root, "sign") }))),
            ("edit", tauri::async_runtime::block_on(edit_pdf(RunEvents::silent(), EditPdfRequest { paths: paths.clone(), mode: "highlight".into(), text: String::new(), pages: None, rectangle, output_location: location(&root, "edit") }))),
        ];
        for (name, outcomes) in runs {
            assert_eq!(outcomes.len(), 1, "{name}");
            assert_eq!(outcomes[0].error().map(|error| error.message.as_str()), Some("Digitally signed PDFs cannot be edited because this mutation would invalidate the signature; remove the signature or use an unsigned copy"), "{name}");
            assert!(outcomes[0].output_paths().is_empty() && outcomes[0].warnings.is_empty(), "{name}");
            assert_eq!(std::fs::read_dir(root.join(name)).unwrap().count(), 0, "{name}");
        }
        assert_eq!(std::fs::read(&signed).unwrap(), original);
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
        for outcome in &unlocked[..2] {
            assert!(!crate::kit::pdf::tests::holds_a_signature(&lopdf::Document::load(&outcome.output_paths()[0]).unwrap()), "{} still holds a signature", outcome.input_path.display());
        }
        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn removing_restrictions_from_a_signed_pdf_strips_the_signature_and_warns_on_that_file_only() {
        use crate::kit::contracts::{OutcomeState, OutcomeWarning, WarningKind};
        let qpdf = crate::kit::resources::resolve_helper(&crate::kit::resources::QPDF).expect("qpdf is required for the native suite");
        let root = sandbox("unrestrict-signed");
        let inputs_dir = root.join("inputs");
        std::fs::create_dir_all(&inputs_dir).unwrap();
        let restrict = |source: &std::path::Path, name: &str| {
            let restricted = inputs_dir.join(name);
            let status = std::process::Command::new(&qpdf)
                .arg(source).arg(&restricted)
                .args(["--encrypt", "--user-password=", "--owner-password=owner-secret", "--bits=256", "--print=none", "--extract=n", "--"])
                .status().unwrap();
            assert!(status.success(), "qpdf made the owner-only fixture {name}");
            restricted
        };
        let signed_plain = root.join("vendor-nda-plain.pdf");
        make_pdf(&signed_plain, 2);
        let mut document = lopdf::Document::load(&signed_plain).unwrap();
        crate::kit::pdf::tests::sign(&mut document);
        document.save(&signed_plain).unwrap();
        let unsigned_plain = root.join("lease-plain.pdf");
        make_pdf(&unsigned_plain, 1);
        let signed = restrict(&signed_plain, "vendor-nda.pdf");
        let unsigned = restrict(&unsigned_plain, "lease.pdf");
        assert!(crate::kit::pdf::tests::holds_a_signature(&lopdf::Document::load(&signed).unwrap()), "the fixture is signed");
        let inputs = vec![signed.clone(), unsigned.clone()];
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();

        let output = location(&root, "unlocked");
        let OutputLocation::CustomFolder(output_dir) = &output else { unreachable!() };
        let outcomes = tauri::async_runtime::block_on(remove_password(RunEvents::silent(), PasswordRequest {
            paths: inputs.clone(),
            password: String::new(),
            password_overrides: Default::default(),
            output_location: output.clone(),
        }));

        let removed = vec![OutcomeWarning { kind: WarningKind::SignatureRemoved, message: crate::kit::pdf::SIGNATURE_REMOVED.into() }];
        assert_eq!(outcomes.iter().map(|outcome| outcome.warnings.clone()).collect::<Vec<_>>(), [removed, vec![]]);
        for (outcome, name, pages) in [(&outcomes[0], "vendor-nda-unlocked.pdf", 2), (&outcomes[1], "lease-unlocked.pdf", 1)] {
            assert!(matches!(outcome.state, OutcomeState::Succeeded { .. }), "{}: {:?}", outcome.input_path.display(), outcome.state);
            assert_eq!(outcome.detail, "Unlocked. Restrictions removed.");
            let path = &outcome.output_paths()[0];
            assert_eq!(path, &output_dir.join(name));
            assert!(opens_without_password(path), "{} still has encryption", path.display());
            let unlocked = lopdf::Document::load(path).unwrap();
            assert!(unlocked.trailer.get(b"Encrypt").is_err(), "{} keeps an /Encrypt dictionary", path.display());
            assert!(!crate::kit::pdf::tests::holds_a_signature(&unlocked), "{} still holds a signature", path.display());
            assert_eq!(unlocked.get_pages().len(), pages);
        }
        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        assert!(crate::kit::pdf::tests::holds_a_signature(&lopdf::Document::load(&signed).unwrap()), "the original keeps its signature");
        let _ = std::fs::remove_dir_all(root);
    }

    const MISSING_QPDF_CHILD: &str = "TOOLBOX_TEST_MISSING_QPDF_CHILD";

    // Other tests resolve qpdf without the env lock, so the missing override is set only in a child test process.
    #[test]
    fn without_qpdf_a_mixed_lock_fails_only_the_pdfs_as_unavailable() {
        if std::env::var_os(MISSING_QPDF_CHILD).is_none() {
            let _guard = crate::kit::lock_process_env();
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
    fn without_qpdf_compress_merge_and_split_fail_each_pdf_as_unavailable() {
        if std::env::var_os(MISSING_QPDF_CHILD).is_none() {
            let _guard = crate::kit::lock_process_env();
            let missing = sandbox("missing-qpdf-pdf-commands").join("qpdf-not-installed");
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["command_tests::without_qpdf_compress_merge_and_split_fail_each_pdf_as_unavailable", "--exact", "--nocapture", "--test-threads=1"])
                .env(MISSING_QPDF_CHILD, "1")
                .env("TOOLBOX_QPDF_PATH", &missing)
                .output()
                .unwrap();
            let report = format!("{}{}", String::from_utf8_lossy(&child.stdout), String::from_utf8_lossy(&child.stderr));
            assert!(child.status.success() && report.contains("1 passed"), "{report}");
            return;
        }
        assert!(crate::kit::resources::resolve_helper(&crate::kit::resources::QPDF).is_err(), "the Development override must hide qpdf");
        let root = sandbox("pdf-commands-missing-qpdf");
        let inputs = vec![root.join("report.pdf"), root.join("appendix.pdf")];
        make_pdf(&inputs[0], 2);
        make_pdf(&inputs[1], 3);
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();

        let runs = [
            ("compress", tauri::async_runtime::block_on(compress_files(RunEvents::silent(), CompressFilesRequest { paths: inputs.clone(), level: CompressLevel::Medium, strip_exif: false, output_location: location(&root, "compress") }))),
            ("merge", tauri::async_runtime::block_on(run_merge_pdfs(&MergePdfRequest { paths: inputs.clone(), output_location: location(&root, "merge"), output_path: None }))),
            ("split", tauri::async_runtime::block_on(split_pdf(RunEvents::silent(), SplitPdfRequest { paths: inputs[..1].to_vec(), mode: SplitMode::EveryN { pages: 1 }.into(), output_location: location(&root, "split") }))),
        ];

        for (name, outcomes) in &runs {
            let expected = if *name == "split" { &inputs[..1] } else { &inputs[..] };
            assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), expected, "{name} reports each PDF");
            for outcome in outcomes {
                assert_eq!(outcome.error().map(|error| error.kind.clone()), Some(ErrorKind::Unavailable), "{name}: {}", outcome.input_path.display());
                assert!(outcome.output_paths().is_empty(), "{name}");
            }
            assert_eq!(std::fs::read_dir(root.join(name)).unwrap().count(), 0, "{name} left outputs");
        }
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
        let _guard = crate::kit::lock_process_env();
        let real = crate::kit::resources::resolve_helper(&crate::kit::resources::QPDF).expect("qpdf is required for the native suite");
        let root = sandbox("protect-files-fake-qpdf");
        let inputs = ["first", "own", "last"].map(|stem| root.join(format!("{stem}.pdf"))).to_vec();
        for input in &inputs { make_pdf(input, 1); }
        let helper = FakeHelper::install(&root, "qpdf", Some(&real));

        let previous = std::env::var_os("TOOLBOX_QPDF_PATH");
        std::env::set_var("TOOLBOX_QPDF_PATH", &helper.script);
        // One worker, as in the cancel test: on hosted macOS runners, parallel first runs of the
        // freshly written fake script hung before it started. This test checks where passwords go,
        // and the parallel batch is covered with the real helper elsewhere.
        let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let outcomes = pool.install(|| tauri::async_runtime::block_on(protect_files(RunEvents::silent(), PasswordRequest {
            paths: inputs.clone(),
            password: TEST_PASSWORD.into(),
            password_overrides: overrides(&[(&inputs[1], TEST_OVERRIDE_PASSWORD)]),
            output_location: location(&root, "locked"),
        })));
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

        assert_eq!(
            calls.len(),
            3,
            "the helper runs once per file; calls: {:?}; errors: {:?}",
            helper.invocations().iter().map(|call| &call.arguments).collect::<Vec<_>>(),
            outcomes.iter().map(|outcome| outcome.error()).collect::<Vec<_>>()
        );
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
        let _guard = crate::kit::lock_process_env();
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
    fn compress_commands_report_sizes_matching_the_files_on_disk() {
        let root = sandbox("compress-sizes");
        let pdf = root.join("document.pdf");
        let image = root.join("image.png");
        make_pdf(&pdf, 3);
        write_png(&image, 128, 7);

        let pdf_outcomes = tauri::async_runtime::block_on(compress_files(RunEvents::silent(), CompressFilesRequest {
            paths: vec![pdf.clone()], level: CompressLevel::High, strip_exif: false, output_location: location(&root, "pdf"),
        }));
        let lossy = tauri::async_runtime::block_on(compress_images(RunEvents::silent(), CompressImagesRequest {
            paths: vec![image.clone()], quality: 50, lossless: false, output_location: location(&root, "lossy"),
        }));
        let lossless = tauri::async_runtime::block_on(compress_images(RunEvents::silent(), CompressImagesRequest {
            paths: vec![image.clone()], quality: 80, lossless: true, output_location: location(&root, "lossless"),
        }));

        for outcome in pdf_outcomes.iter().chain(&lossy).chain(&lossless) {
            assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
            let [output] = outcome.output_paths() else { panic!("one output expected") };
            assert_eq!(outcome.original_bytes, Some(std::fs::metadata(&outcome.input_path).unwrap().len()));
            assert_eq!(outcome.output_bytes, Some(std::fs::metadata(output).unwrap().len()));
            let value = serde_json::to_value(outcome).unwrap();
            assert_eq!(value["originalBytes"], outcome.original_bytes.unwrap());
            assert_eq!(value["outputBytes"], outcome.output_bytes.unwrap());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    /// Three pages with inherited, page-level and every optional page box plus a rotation, and
    /// uncompressed content so a lossless repack has something to save.
    fn make_compressible_pdf(path: &PathBuf) {
        use lopdf::{dictionary, Document, Object, Stream};
        let numbers = |values: [f32; 4]| values.into_iter().map(Object::Real).collect::<Vec<_>>();
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let pages = [
            dictionary! { "CropBox" => numbers([10.0, 12.5, 600.0, 780.0]), "Rotate" => 90 },
            dictionary! { "MediaBox" => numbers([0.0, 0.0, 842.0, 595.0]), "BleedBox" => numbers([5.0, 5.0, 837.0, 590.0]), "TrimBox" => numbers([9.0, 9.0, 833.0, 586.0]), "ArtBox" => numbers([20.0, 20.0, 822.0, 575.0]) },
            dictionary! {},
        ];
        let mut kids = Vec::new();
        for (number, mut page) in pages.into_iter().enumerate() {
            let text = (0..80).map(|line| format!("BT /F1 9 Tf 36 {} Td (Toolbox page {} line {line} repeats to be repacked) Tj ET\n", 760 - line * 9, number + 1)).collect::<String>();
            let content_id = document.add_object(Stream::new(dictionary! {}, text.into_bytes()));
            page.set("Type", "Page");
            page.set("Parent", pages_id);
            page.set("Resources", dictionary! { "Font" => dictionary! { "F1" => font_id } });
            page.set("Contents", content_id);
            kids.push(Object::Reference(document.add_object(page)));
        }
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 3, "MediaBox" => numbers([0.0, 0.0, 612.0, 792.0]) }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    /// Every page's boxes (inherited ones resolved) and rotation, in page order.
    fn page_geometry(path: &PathBuf) -> Vec<(Vec<Option<Vec<f32>>>, i64)> {
        let document = lopdf::Document::load(path).unwrap();
        document.get_pages().into_values().map(|page| {
            let boxes = [b"MediaBox".as_slice(), b"CropBox", b"BleedBox", b"TrimBox", b"ArtBox"].iter().map(|key| {
                crate::kit::pdf::inherited(&document, page, key).unwrap().map(|value| value.as_array().unwrap().iter().map(|number| metadata::number(number).unwrap()).collect())
            }).collect();
            (boxes, metadata::page_rotation(&document, page))
        }).collect()
    }

    fn compress(paths: &[PathBuf], level: CompressLevel, output_location: OutputLocation) -> Vec<JobOutcome> {
        tauri::async_runtime::block_on(compress_files(RunEvents::silent(), CompressFilesRequest { paths: paths.to_vec(), level, strip_exif: false, output_location }))
    }

    #[test]
    fn compress_files_repacks_pdfs_at_every_level_and_keeps_pages_boxes_and_rotation() {
        let root = sandbox("compress-files-levels");
        let pdf = root.join("report.pdf");
        make_compressible_pdf(&pdf);
        let original = std::fs::read(&pdf).unwrap();
        let geometry = page_geometry(&pdf);
        assert_eq!(geometry.len(), 3);
        assert_eq!(geometry[0].1, 90, "the fixture rotates its first page");

        for level in [CompressLevel::Low, CompressLevel::Medium, CompressLevel::High] {
            let name = format!("{level:?}");
            let outcomes = compress(&[pdf.clone()], level, location(&root, &name));
            let [outcome] = outcomes.as_slice() else { panic!("{name}: one outcome per input") };
            assert!(outcome.error().is_none(), "{name}: {}", outcome.error().cloned().unwrap_or_default());
            let [output] = outcome.output_paths() else { panic!("{name}: one output") };
            assert_eq!(output, &root.join(&name).join("report-compressed.pdf"));
            assert_eq!(page_geometry(output), geometry, "{name} changed the page geometry");
            assert_eq!(outcome.original_bytes, Some(original.len() as u64));
            assert_eq!(outcome.output_bytes, Some(std::fs::metadata(output).unwrap().len()));
            assert!(outcome.output_bytes < outcome.original_bytes, "{name} must save space");
        }
        assert_eq!(std::fs::read(&pdf).unwrap(), original, "the original was modified");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn compress_files_skips_an_already_repacked_pdf_with_both_sizes_and_writes_nothing() {
        let root = sandbox("compress-files-no-savings");
        let pdf = root.join("report.pdf");
        make_compressible_pdf(&pdf);
        let first = compress(&[pdf.clone()], CompressLevel::Medium, location(&root, "first"));
        let repacked = first[0].output_paths()[0].clone();
        let before = std::fs::read(&repacked).unwrap();

        let outcomes = compress(&[repacked.clone()], CompressLevel::High, location(&root, "again"));

        let [outcome] = outcomes.as_slice() else { panic!("one outcome per input") };
        let OutcomeState::Skipped { reason } = &outcome.state else { panic!("expected a skip, got {:?}", outcome.state) };
        assert_eq!(reason.code, crate::kit::contracts::SkipCode::NoSavings);
        assert_eq!(reason.message, "No savings: already well compressed");
        assert_eq!(outcome.original_bytes, Some(before.len() as u64));
        assert!(outcome.output_bytes.is_some_and(|bytes| bytes >= before.len() as u64));
        assert!(outcome.output_paths().is_empty());
        assert_eq!(std::fs::read_dir(root.join("again")).unwrap().count(), 0, "a skip leaves no file, temp or claim");
        assert_eq!(std::fs::read(&repacked).unwrap(), before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn compress_files_fails_a_locked_pdf_and_a_docx_alone_and_reports_in_input_order() {
        let root = sandbox("compress-files-mixed");
        let inputs = vec![root.join("locked.pdf"), root.join("report.pdf"), root.join("lease.docx"), root.join("appendix.pdf")];
        std::fs::write(&inputs[0], crate::kit::password::tests::pdf(Some(("owner", "user")))).unwrap();
        make_compressible_pdf(&inputs[1]);
        std::fs::write(&inputs[2], test_minimal_ooxml_package("docx")).unwrap();
        make_compressible_pdf(&inputs[3]);
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();

        let outcomes = compress(&inputs, CompressLevel::Medium, location(&root, "out"));

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs);
        let locked = outcomes[0].error().expect("a locked PDF fails");
        assert_eq!(locked.kind, ErrorKind::InvalidInput);
        assert_eq!(locked.message, "This PDF is password-protected. Unlock it first, then compress the unlocked copy.");
        assert_eq!(locked.pointer, Some(crate::kit::contracts::OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
        let docx = outcomes[2].error().expect("a DOCX fails");
        assert_eq!(docx.kind, ErrorKind::InvalidInput);
        for outcome in [&outcomes[1], &outcomes[3]] {
            assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
            assert_eq!(outcome.output_paths().len(), 1);
        }
        let mut written = std::fs::read_dir(root.join("out")).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect::<Vec<_>>();
        written.sort();
        assert_eq!(written, ["appendix-compressed.pdf", "report-compressed.pdf"]);
        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn compress_files_numbers_copies_beside_an_existing_compressed_file() {
        let root = sandbox("compress-files-collisions");
        let pdf = root.join("report.pdf");
        make_compressible_pdf(&pdf);
        let output = location(&root, "out");
        let OutputLocation::CustomFolder(folder) = &output else { unreachable!() };
        std::fs::write(folder.join("report-compressed.pdf"), b"an earlier copy").unwrap();

        let first = compress(&[pdf.clone()], CompressLevel::Medium, output.clone());
        let second = compress(&[pdf.clone()], CompressLevel::Medium, output.clone());

        assert_eq!(first[0].output_paths(), [folder.join("report-compressed-1.pdf")]);
        assert_eq!(second[0].output_paths(), [folder.join("report-compressed-2.pdf")]);
        assert_eq!(std::fs::read(folder.join("report-compressed.pdf")).unwrap(), b"an earlier copy");
        let _ = std::fs::remove_dir_all(root);
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

        let outcomes = tauri::async_runtime::block_on(pdf_to_images(RunEvents::silent(), PdfToImagesRequest {
            paths: vec![pdf.clone()],
            dpi: 72,
            format: "png".into(),
            page_range: None,
            pages: Some(Vec::new()),
            picks: Vec::new(),
            output_location: location(&root, "images"),
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

        let wrong_format = tauri::async_runtime::block_on(extract_from_pdf(RunEvents::silent(), extract_request(vec![image.clone()], ExtractMode::Text, Some("1"), location(&root, "wrong-format"))));
        assert_eq!(wrong_format.len(), 1);
        assert!(wrong_format[0].output_paths().is_empty());
        assert!(wrong_format[0].error().is_some_and(|error| error.message.contains("Accepted extensions")));

        let wrong_cardinality = tauri::async_runtime::block_on(split_pdf(RunEvents::silent(), SplitPdfRequest {
            paths: vec![pdf.clone(), second_pdf.clone()],
            mode: SplitMode::EveryN { pages: 1 }.into(),
            output_location: location(&root, "wrong-cardinality"),
        }));
        assert_eq!(wrong_cardinality.len(), 2);
        assert!(wrong_cardinality.iter().all(|outcome| outcome.output_paths().is_empty() && outcome.error().is_some()));

        if pdftotext_available() {
            let selected = tauri::async_runtime::block_on(extract_from_pdf(RunEvents::silent(), extract_request(vec![pdf.clone()], ExtractMode::Text, Some("2"), location(&root, "selected-text"))));
            let output = selected[0].output_paths().first().expect("selected text should produce an output");
            let text = std::fs::read_to_string(output).unwrap();
            assert!(text.contains("Toolbox page 2"), "selected text: {text}");
            assert!(!text.contains("Toolbox page 1"), "unselected text: {text}");
            cleanup(&selected.iter().flat_map(|outcome| outcome.output_paths().to_vec()).collect::<Vec<_>>());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    /// Builds a PDF whose pages each may carry a line of selectable text and a 4x3 embedded JPEG.
    fn make_extract_pdf(path: &PathBuf, pages: &[(bool, bool)]) {
        use lopdf::{dictionary, Dictionary, Document, Object, Stream};
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg).encode_image(&image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 3, image::Rgb([200, 30, 30])))).unwrap();
        let jpeg = jpeg.into_inner();
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let mut kids = Vec::new();
        for (number, (text, picture)) in pages.iter().enumerate() {
            let mut content = String::new();
            let mut xobjects = Dictionary::new();
            if *text { content.push_str(&format!("BT /F1 24 Tf 72 720 Td (Toolbox page {}) Tj ET\n", number + 1)); }
            if *picture {
                let image_id = document.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => 4, "Height" => 3, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode" }, jpeg.clone()));
                xobjects.set("Im0", image_id);
                content.push_str("q 40 0 0 30 72 600 cm /Im0 Do Q\n");
            }
            let content_id = document.add_object(Stream::new(dictionary! {}, content.into_bytes()));
            let page_id = document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id }, "XObject" => xobjects }, "Contents" => content_id,
            });
            kids.push(Object::Reference(page_id));
        }
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages.len() as i64 }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    fn files_in(folder: &std::path::Path) -> Vec<String> {
        let mut names = std::fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn extract_from_pdf_runs_each_mode_with_one_outcome_per_pdf() {
        if !pdftotext_available() { return; }
        use crate::kit::contracts::{OutcomePointer, OutcomeState, SkipCode};
        use crate::kit::pdf::remaining::{BOTH_WITHOUT_IMAGES, BOTH_WITHOUT_TEXT, NEEDS_OCR_MESSAGE, NOTHING_TO_EXTRACT, NO_EMBEDDED_IMAGES};
        let root = sandbox("extract-from-pdf");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mixed = root.join("mixed.pdf");
        let scanned = root.join("scanned.pdf");
        let plain = root.join("plain.pdf");
        let blank = root.join("blank.pdf");
        make_extract_pdf(&mixed, &[(true, true), (true, false)]);
        make_extract_pdf(&scanned, &[(false, true)]);
        make_extract_pdf(&plain, &[(true, false), (true, false)]);
        make_extract_pdf(&blank, &[(false, false)]);
        let originals = [&mixed, &scanned, &plain, &blank].map(Original::of);
        let run = |paths: Vec<PathBuf>, mode, range: Option<&str>, folder: &str| {
            let request = extract_request(paths, mode, range, location(&root, folder));
            tauri::async_runtime::block_on(extract_from_pdf(RunEvents::silent(), request))
        };
        let read = |path: &PathBuf| std::fs::read_to_string(path).unwrap();

        // Text: one text file per PDF in reading order; a PDF without a text layer is skipped alone and pointed at OCR.
        let text = run(vec![mixed.clone(), plain.clone(), scanned.clone()], ExtractMode::Text, None, "text");
        assert_eq!(text.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), vec![mixed.clone(), plain.clone(), scanned.clone()]);
        for outcome in &text[..2] {
            assert_eq!(outcome.output_paths().len(), 1, "{outcome:?}");
            let body = read(&outcome.output_paths()[0]);
            assert!(body.contains("Toolbox page 1") && body.contains("Toolbox page 2"), "{body}");
        }
        let OutcomeState::Skipped { reason } = &text[2].state else { panic!("a PDF without a text layer is handed to OCR: {:?}", text[2]) };
        assert_eq!(reason.code, SkipCode::NeedsOcr);
        assert_eq!(reason.message, NEEDS_OCR_MESSAGE);
        assert_eq!(reason.pointer, Some(OutcomePointer::Tool { tool_id: "pdf-ocr".into() }));
        assert!(text[2].output_paths().is_empty());
        assert_eq!(files_in(&root.join("text")), vec!["mixed-text.txt", "plain-text.txt"]);

        // Images: the embedded JPEGs are copied byte for byte; a PDF without images is skipped alone.
        let images = run(vec![mixed.clone(), scanned.clone(), plain.clone()], ExtractMode::Images, None, "images");
        assert_eq!(images.len(), 3);
        assert_eq!(images[0].output_paths(), [root.join("images").join("mixed-images")]);
        assert_eq!(images[0].detail, "Saved 1 image.");
        assert_eq!(images[1].output_paths(), [root.join("images").join("scanned-images")]);
        assert_eq!(std::fs::read(images[0].output_paths()[0].join("page-1-image-1.jpg")).unwrap()[..2], [0xFF, 0xD8]);
        assert!(matches!(&images[2].state, OutcomeState::Skipped { reason } if reason.code == SkipCode::NoEffectiveChange && reason.message == NO_EMBEDDED_IMAGES), "{:?}", images[2]);
        assert_eq!(files_in(&root.join("images")), vec!["mixed-images", "scanned-images"]);
        assert_eq!(files_in(&root.join("images").join("mixed-images")), vec!["page-1-image-1.jpg"]);

        // Both: one outcome per PDF holds the text file first, then the images, with a note when a part is missing.
        let both = run(vec![mixed.clone(), scanned.clone(), plain.clone(), blank.clone()], ExtractMode::Both, None, "both");
        assert_eq!(both.len(), 4);
        let folder = root.join("both");
        assert!(matches!(both[0].state, OutcomeState::Succeeded { .. }));
        assert_eq!(both[0].output_paths(), [folder.join("mixed-text.txt"), folder.join("mixed-images")]);
        assert_eq!(both[0].detail, "Saved the text and 1 image.");
        assert!(matches!(both[1].state, OutcomeState::Succeeded { .. }), "an image-only PDF succeeds in Both: {:?}", both[1]);
        assert_eq!(both[1].output_paths(), [folder.join("scanned-images")]);
        assert_eq!(both[1].detail, BOTH_WITHOUT_TEXT);
        assert!(both[1].detail.contains("No selectable text"));
        assert!(matches!(both[2].state, OutcomeState::Succeeded { .. }));
        assert_eq!(both[2].output_paths(), [folder.join("plain-text.txt")]);
        assert_eq!(both[2].detail, BOTH_WITHOUT_IMAGES);
        match &both[3].state {
            OutcomeState::Skipped { reason } => assert_eq!((reason.code, reason.message.as_str()), (SkipCode::NoEffectiveChange, NOTHING_TO_EXTRACT)),
            state => panic!("a PDF with neither part is skipped, got {state:?}"),
        }
        assert_eq!(files_in(&folder), vec!["mixed-images", "mixed-text.txt", "plain-text.txt", "scanned-images"]);

        // A second run never replaces the first run's outputs.
        let again = run(vec![mixed.clone()], ExtractMode::Both, None, "both");
        assert_eq!(again[0].output_paths(), [folder.join("mixed-text-1.txt"), folder.join("mixed-images-1")]);
        assert_eq!(files_in(&folder.join("mixed-images")), vec!["page-1-image-1.jpg"]);
        assert_eq!(files_in(&folder.join("mixed-images-1")), vec!["page-1-image-1.jpg"]);
        assert_eq!(read(&folder.join("mixed-text.txt")), read(&folder.join("mixed-text-1.txt")));

        // One range applies to every PDF; "2-" runs to the end of the document.
        let ranged = run(vec![mixed.clone(), plain.clone()], ExtractMode::Both, Some("2-"), "ranged");
        assert_eq!(ranged[0].output_paths(), [root.join("ranged").join("mixed-text.txt")]);
        assert_eq!(ranged[0].detail, BOTH_WITHOUT_IMAGES);
        let body = read(&ranged[0].output_paths()[0]);
        assert!(body.contains("Toolbox page 2") && !body.contains("Toolbox page 1"), "{body}");
        assert_eq!(ranged[1].output_paths().len(), 1);
        let backwards = run(vec![mixed.clone()], ExtractMode::Text, Some("2-1"), "backwards");
        assert_eq!(backwards[0].error().map(|error| error.message.as_str()), Some("\"2-1\" runs backwards. Put the lower page first, for example 1-2."));
        assert!(files_in(&root.join("backwards")).is_empty());

        // Skip small images leaves out images under the given size.
        let mut small = extract_request(vec![mixed.clone()], ExtractMode::Both, None, location(&root, "small"));
        small.min_image_size = Some(5);
        let small = tauri::async_runtime::block_on(extract_from_pdf(RunEvents::silent(), small));
        assert_eq!(small[0].output_paths(), [root.join("small").join("mixed-text.txt")]);

        for original in &originals { original.assert_untouched(); }
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

    #[test]
    fn scene_export_alongside_input_writes_reordered_output_and_keeps_the_original() {
        let root = sandbox("scene-alongside");
        let input = root.join("sample.pdf");
        make_pdf(&input, 3);
        let original = std::fs::read(&input).unwrap();
        let pages = [2, 0, 1].map(|index| serde_json::json!({ "sourceIndex": index, "width": 612, "height": 792, "rotation": 0, "crop": null, "objects": [] }));
        let scene = serde_json::from_value::<scene::PdfScene>(serde_json::json!({ "pages": pages })).unwrap();
        let outputs = assert_success("scene alongside", run_export_pdf_scene(RunEvents::silent(), scene::ExportRequest { paths: vec![input.clone()], scene, output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default() }));
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].parent(), Some(root.as_path()));
        assert!(outputs[0].exists(), "scene output must exist on disk");
        let document = lopdf::Document::load(&outputs[0]).unwrap();
        let labels = document.get_pages().values().map(|page| String::from_utf8_lossy(&document.get_page_content(*page)).into_owned()).collect::<Vec<_>>();
        for (content, number) in labels.iter().zip([3, 1, 2]) { assert!(content.contains(&format!("(Toolbox page {number})")), "{content}"); }
        assert_eq!(labels.len(), 3);
        assert_eq!(std::fs::read(&input).unwrap(), original);
        let _ = std::fs::remove_dir_all(root);
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

    fn write_tiff(path: &PathBuf, pages: &[image::DynamicImage]) {
        use tiff::encoder::colortype::{Gray8, RGB16, RGB8, RGBA8};
        let mut encoder = tiff::encoder::TiffEncoder::new(std::fs::File::create(path).unwrap()).unwrap();
        for page in pages {
            let (width, height) = (page.width(), page.height());
            match page {
                image::DynamicImage::ImageLuma8(page) => encoder.write_image::<Gray8>(width, height, page.as_raw()),
                image::DynamicImage::ImageRgb8(page) => encoder.write_image::<RGB8>(width, height, page.as_raw()),
                image::DynamicImage::ImageRgba8(page) => encoder.write_image::<RGBA8>(width, height, page.as_raw()),
                image::DynamicImage::ImageRgb16(page) => encoder.write_image::<RGB16>(width, height, page.as_raw()),
                page => panic!("no TIFF fixture writer for {:?}", page.color()),
            }
            .unwrap();
        }
    }

    fn tiff_pages(path: &PathBuf) -> Vec<(tiff::ColorType, (u32, u32), Vec<u8>)> {
        let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut pages = Vec::new();
        loop {
            let tiff::decoder::DecodingResult::U8(data) = decoder.read_image().unwrap() else { panic!("expected 8-bit TIFF page") };
            pages.push((decoder.colortype().unwrap(), decoder.dimensions().unwrap(), data));
            if !decoder.more_images() { return pages; }
            decoder.next_image().unwrap();
        }
    }

    #[test]
    fn convert_keeps_png_pixels_exact_and_lossy_dimensions_for_every_source_family() {
        let root = sandbox("convert-source-families");
        let rgba = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(30, 20, |x, y| Rgba([x as u8 * 8, y as u8 * 12, (x ^ y) as u8, (x + y) as u8 * 4])));
        let rgb16 = image::DynamicImage::ImageRgb16(image::ImageBuffer::from_fn(24, 16, |x, y| image::Rgb([(x * 2731) as u16, (y * 4093) as u16, ((x * y) * 97) as u16])));
        let gray = image::DynamicImage::ImageLuma8(image::GrayImage::from_fn(17, 9, |x, y| image::Luma([(x * 15 ^ y * 7) as u8])));
        let photo = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(32, 24, |x, y| image::Rgb([x as u8 * 8, y as u8 * 10, 90])));
        let heic_bytes = crate::kit::images::encode(&photo, crate::kit::images::OutputFormat::Heic, 80).unwrap();
        let mut sources: Vec<(PathBuf, image::DynamicImage)> = Vec::new();
        for (name, image) in [("rgba.png", &rgba), ("deep.png", &rgb16), ("gray.png", &gray), ("photo.jpg", &photo), ("photo.webp", &rgba)] {
            let path = root.join(name);
            image.save(&path).unwrap();
            sources.push((path.clone(), image::open(&path).unwrap()));
        }
        for name in ["photo.heic", "photo.heif"] {
            let path = root.join(name);
            std::fs::write(&path, &heic_bytes).unwrap();
            sources.push((path, heif::decode(&heic_bytes).unwrap()));
        }
        let originals: Vec<Vec<u8>> = sources.iter().map(|(path, _)| std::fs::read(path).unwrap()).collect();

        for (index, (source, expected)) in sources.iter().enumerate() {
            let name = source.file_name().unwrap().to_string_lossy().to_string();
            let family = match source.extension().unwrap().to_str().unwrap() { "jpeg" => "jpg", "heif" => "heic", other => other };
            if family != "png" {
                let png = assert_success(&name, convert_images(RunEvents::silent(), ConvertImagesRequest {
                    paths: vec![source.clone()], format: "png".into(), output_location: location(&root, &format!("png-{index}")),
                }));
                let converted = image::open(&png[0]).unwrap();
                assert_eq!(converted.color(), expected.color(), "{name} keeps its color type in PNG");
                assert_eq!(converted.as_bytes(), expected.as_bytes(), "{name} keeps its pixels in PNG");
            }
            for format in ["jpg", "webp", "heic"].into_iter().filter(|format| *format != family) {
                let output = assert_success(&name, convert_images(RunEvents::silent(), ConvertImagesRequest {
                    paths: vec![source.clone()], format: format.into(), output_location: location(&root, &format!("{format}-{index}")),
                }));
                let bytes = std::fs::read(&output[0]).unwrap();
                let decoded = if format == "heic" { heif::decode(&bytes).unwrap() } else { image::load_from_memory(&bytes).unwrap() };
                assert_eq!((decoded.width(), decoded.height()), (expected.width(), expected.height()), "{name} as {format} keeps its size");
            }
        }
        for ((source, _), original) in sources.iter().zip(&originals) {
            assert_eq!(&std::fs::read(source).unwrap(), original, "{} stays untouched", source.display());
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tiff_split_combine_and_selection_keep_every_page_byte_for_byte() {
        use crate::kit::images::tools::{TiffPageRef, TiffPreviewRequest};
        let root = sandbox("tiff-page-bytes");
        let gray = image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(3, 2, vec![1, 2, 3, 4, 5, 6]).unwrap());
        let rgb = image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(2, 2, (10..22).collect()).unwrap());
        let rgba = image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(2, 1, vec![30, 31, 32, 0, 33, 34, 35, 128]).unwrap());
        let single = image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(1, 2, vec![40, 41, 42, 43, 44, 45]).unwrap());
        let multi = root.join("multi.tiff");
        write_tiff(&multi, &[gray.clone(), rgb.clone(), rgba.clone()]);
        let other = root.join("single.tif");
        write_tiff(&other, &[single.clone()]);
        let originals = [std::fs::read(&multi).unwrap(), std::fs::read(&other).unwrap()];
        let page = |image: &image::DynamicImage| {
            let color = match image.color() { image::ColorType::L8 => tiff::ColorType::Gray(8), image::ColorType::Rgb8 => tiff::ColorType::RGB(8), _ => tiff::ColorType::RGBA(8) };
            (color, (image.width(), image.height()), image.as_bytes().to_vec())
        };

        let split = assert_success("split", std::future::ready(run_split_tiff_pages(&RunEvents::silent(), TiffRequest { mode: TiffMode::Split, paths: vec![multi.clone()], pages: None, output_location: location(&root, "split"), output_path: None })));
        assert_eq!(split.iter().map(tiff_pages).collect::<Vec<_>>(), vec![vec![page(&gray)], vec![page(&rgb)], vec![page(&rgba)]]);

        let combined = assert_aggregate_success("combine", &[multi.clone(), other.clone()], run_combine_tiff_pages(&TiffRequest {
            mode: TiffMode::Combine, paths: vec![multi.clone(), other.clone()], pages: None, output_location: location(&root, "combine"), output_path: None,
        }));
        assert_eq!(tiff_pages(&combined[0]), vec![page(&gray), page(&rgb), page(&rgba), page(&single)]);

        let selected = assert_aggregate_success("select", &[multi.clone(), other.clone()], run_combine_tiff_pages(&TiffRequest {
            mode: TiffMode::Combine, paths: vec![multi.clone(), other.clone()],
            pages: Some(vec![TiffPageRef { path: other.clone(), page: 0 }, TiffPageRef { path: multi.clone(), page: 2 }, TiffPageRef { path: multi.clone(), page: 0 }]),
            output_location: location(&root, "select"), output_path: None,
        }));
        assert_eq!(tiff_pages(&selected[0]), vec![page(&single), page(&rgba), page(&gray)]);

        let outside = tauri::async_runtime::block_on(run_combine_tiff_pages(&TiffRequest {
            mode: TiffMode::Combine, paths: vec![multi.clone()], pages: Some(vec![TiffPageRef { path: multi.clone(), page: 3 }]), output_location: location(&root, "outside"), output_path: None,
        }));
        assert_eq!(outside[0].error().map(|error| error.message.as_str()), Some("TIFF page 4 is outside the document."));

        let previews = inspect_tiff_pages(TiffPreviewRequest { path: multi.clone() }).unwrap();
        assert_eq!(previews.iter().map(|preview| (preview.width, preview.height)).collect::<Vec<_>>(), vec![(3, 2), (2, 2), (2, 1)]);

        assert_eq!([std::fs::read(&multi).unwrap(), std::fs::read(&other).unwrap()], originals, "originals stay untouched");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tiff_tools_reject_sixteen_bit_pages_with_the_same_message() {
        use crate::kit::images::tools::TiffPreviewRequest;
        let root = sandbox("tiff-deep-pages");
        let deep = root.join("deep.tiff");
        write_tiff(&deep, &[image::DynamicImage::ImageRgb16(image::ImageBuffer::from_raw(2, 1, vec![1, 2, 3, 4, 5, 6]).unwrap())]);
        let unsupported = "TIFF page 1 of 1 can't be read. Unsupported TIFF color type or bit depth: RGB(16)";

        let split = run_split_tiff_pages(&RunEvents::silent(), TiffRequest { mode: TiffMode::Split, paths: vec![deep.clone()], pages: None, output_location: location(&root, "split"), output_path: None });
        assert_eq!(split[0].error().map(|error| error.message.clone()), Some(format!("Could not process {}: {unsupported}", deep.display())));
        assert!(split[0].output_paths().is_empty());
        assert_eq!(inspect_tiff_pages(TiffPreviewRequest { path: deep.clone() }).err().as_deref(), Some(unsupported));
        let _ = std::fs::remove_dir_all(&root);
    }

    mod image_metadata_inspection {
        use super::*;
        use crate::kit::images::metadata::fixtures::{self, Wanted};
        use crate::kit::images::metadata::{ImageMetadata, MetadataErrorCode, MetadataFormat, MetadataInspection};

        const FORMATS: [(MetadataFormat, &str); 5] = [(MetadataFormat::Jpeg, "jpg"), (MetadataFormat::Png, "png"), (MetadataFormat::Webp, "webp"), (MetadataFormat::Tiff, "tiff"), (MetadataFormat::Heic, "heic")];

        fn fixture(format: MetadataFormat, wanted: Wanted) -> Vec<u8> {
            match format {
                MetadataFormat::Jpeg => fixtures::jpeg(wanted),
                MetadataFormat::Png => fixtures::png(wanted),
                MetadataFormat::Webp => fixtures::webp(wanted),
                MetadataFormat::Tiff => fixtures::tiff(wanted),
                MetadataFormat::Heic => fixtures::heic(wanted),
            }
        }

        fn expected(format: MetadataFormat, wanted: Wanted) -> ImageMetadata {
            let textual = matches!(format, MetadataFormat::Jpeg | MetadataFormat::Png | MetadataFormat::Tiff);
            let exif = wanted.exif || wanted.gps || (wanted.orientation.is_some() && format != MetadataFormat::Tiff);
            ImageMetadata {
                format, exif, gps: wanted.gps, xmp: wanted.xmp, iptc: wanted.iptc && textual, comments: wanted.comments && textual, icc: wanted.icc,
                orientation: wanted.orientation.filter(|value| (1..=8).contains(value)).map(|value| value as u8),
            }
        }

        fn assert_decodes(format: MetadataFormat, bytes: &[u8]) {
            match format {
                MetadataFormat::Heic => { heif::decode(bytes).expect("HEIC fixture must decode"); }
                _ => { image::load_from_memory(bytes).expect("fixture must decode"); }
            }
        }

        fn inspect_files(root: &PathBuf, files: Vec<(String, Vec<u8>)>) -> Vec<MetadataInspection> {
            let paths = files.iter().map(|(name, bytes)| { let path = root.join(name); std::fs::write(&path, bytes).unwrap(); path }).collect::<Vec<_>>();
            let reports = inspect_image_metadata(MetadataRequest { paths: paths.clone(), output_location: OutputLocation::AlongsideInput });
            for (path, (_, bytes)) in paths.iter().zip(&files) { assert_eq!(&std::fs::read(path).unwrap(), bytes, "inspection must not modify {}", path.display()); }
            assert_eq!(reports.len(), paths.len());
            for (report, path) in reports.iter().zip(&paths) {
                let reported = match report { MetadataInspection::Report(report) => &report.path, MetadataInspection::Error(error) => &error.path };
                assert_eq!(reported, path, "one result per file, in request order");
            }
            reports
        }

        fn report(inspection: &MetadataInspection) -> &ImageMetadata {
            match inspection { MetadataInspection::Report(report) => &report.metadata, MetadataInspection::Error(error) => panic!("{}: {}", error.path.display(), error.error.message) }
        }

        fn error_code(inspection: &MetadataInspection) -> MetadataErrorCode {
            match inspection { MetadataInspection::Error(error) => error.error.code, MetadataInspection::Report(report) => panic!("{} unexpectedly read as {:?}", report.path.display(), report.metadata) }
        }

        #[test]
        fn inspect_reports_each_category_for_every_format() {
            let root = sandbox("metadata-categories");
            let cases = [
                ("none", Wanted::default()),
                ("exif", Wanted { exif: true, ..Wanted::default() }),
                ("gps", Wanted { gps: true, ..Wanted::default() }),
                ("xmp", Wanted { xmp: true, ..Wanted::default() }),
                ("iptc", Wanted { iptc: true, ..Wanted::default() }),
                ("comments", Wanted { comments: true, ..Wanted::default() }),
                ("icc", Wanted { icc: true, ..Wanted::default() }),
                ("all", Wanted { exif: true, gps: true, orientation: Some(6), xmp: true, iptc: true, comments: true, icc: true }),
            ];
            for (format, extension) in FORMATS {
                let files = cases.iter().map(|(name, wanted)| {
                    let bytes = fixture(format, *wanted);
                    assert_decodes(format, &bytes);
                    (format!("{name}.{extension}"), bytes)
                }).collect::<Vec<_>>();
                let reports = inspect_files(&root, files);
                for (inspection, (name, wanted)) in reports.iter().zip(cases) {
                    assert_eq!(report(inspection), &expected(format, wanted), "{format:?} {name}");
                }
            }
            let wire = serde_json::to_value(&inspect_image_metadata(MetadataRequest { paths: vec![root.join("all.jpg")], output_location: OutputLocation::AlongsideInput })[0]).unwrap();
            assert_eq!(wire, serde_json::json!({
                "status": "report", "path": root.join("all.jpg"), "format": "jpeg",
                "exif": true, "gps": true, "xmp": true, "iptc": true, "comments": true, "icc": true, "orientation": 6,
            }));
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn inspect_reports_orientation_values_or_absent() {
            let root = sandbox("metadata-orientation");
            for (format, extension) in FORMATS {
                let values = [None, Some(1), Some(2), Some(3), Some(4), Some(5), Some(6), Some(7), Some(8), Some(9)];
                let files = values.iter().map(|value| (format!("{}.{extension}", value.map_or("absent".to_string(), |value: u16| value.to_string())), fixture(format, Wanted { orientation: *value, ..Wanted::default() }))).collect();
                let reports = inspect_files(&root, files);
                for (inspection, value) in reports.iter().zip(values) {
                    assert_eq!(report(inspection).orientation, value.filter(|value| *value <= 8).map(|value| value as u8), "{format:?} {value:?}");
                }
            }
            let absent = serde_json::to_value(&inspect_image_metadata(MetadataRequest { paths: vec![root.join("absent.png")], output_location: OutputLocation::AlongsideInput })[0]).unwrap();
            assert_eq!(absent["orientation"], serde_json::Value::Null);
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn inspect_returns_typed_errors_per_file_for_damaged_input() {
            let root = sandbox("metadata-damaged");
            let all = Wanted { exif: true, gps: true, orientation: Some(3), xmp: true, iptc: true, comments: true, icc: true };
            let mut files = Vec::new();
            let mut codes = Vec::new();
            for (format, extension) in FORMATS {
                let bytes = fixture(format, all);
                files.push((format!("truncated.{extension}"), bytes[..bytes.len() * 2 / 3].to_vec()));
                codes.push(MetadataErrorCode::Truncated);
                files.push((format!("valid.{extension}"), bytes));
                codes.push(MetadataErrorCode::Truncated);
            }
            let mut bad_crc = fixtures::png(all);
            bad_crc[40] ^= 0xFF;
            files.push(("bad-crc.png".into(), bad_crc));
            codes.push(MetadataErrorCode::Corrupt);
            let mut bad_exif = fixtures::jpeg(Wanted { exif: true, ..Wanted::default() });
            bad_exif[12] = b'X';
            files.push(("bad-exif.jpg".into(), bad_exif));
            codes.push(MetadataErrorCode::Corrupt);
            let mut gif = std::io::Cursor::new(Vec::new());
            image::RgbImage::new(4, 4).write_to(&mut gif, image::ImageFormat::Gif).unwrap();
            files.push(("animation.gif".into(), gif.into_inner()));
            codes.push(MetadataErrorCode::UnsupportedFormat);

            let reports = inspect_files(&root, files);
            for (index, (inspection, code)) in reports.iter().zip(&codes).enumerate() {
                if index < 10 && index % 2 == 1 { report(inspection); } else { assert_eq!(error_code(inspection), *code, "{inspection:?}"); }
            }
            let missing = inspect_image_metadata(MetadataRequest { paths: vec![root.join("missing.png")], output_location: OutputLocation::AlongsideInput });
            assert_eq!(error_code(&missing[0]), MetadataErrorCode::Unreadable);
            let wire = serde_json::to_value(&missing[0]).unwrap();
            assert_eq!((wire["status"].as_str(), wire["code"].as_str(), wire["message"].is_string()), (Some("error"), Some("unreadable"), true));
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn inspect_returns_one_report_per_readable_file_and_one_error_per_unreadable_file() {
            let root = sandbox("metadata-mixed-batch");
            let all = Wanted { exif: true, gps: true, orientation: Some(6), xmp: true, iptc: true, comments: true, icc: true };
            let folder = root.join("folder.jpg");
            std::fs::create_dir_all(&folder).unwrap();
            let truncated = fixtures::jpeg(all);
            // (path, original bytes when the file exists, expected readable)
            let mut entries: Vec<(PathBuf, Option<Vec<u8>>, bool)> = vec![
                (root.join("missing.png"), None, false),
                (folder.clone(), None, false),
            ];
            for (format, extension) in FORMATS {
                entries.push((root.join(format!("camera.{extension}")), Some(fixture(format, all)), true));
                entries.push((root.join(format!("plain.{extension}")), Some(fixture(format, Wanted::default())), true));
            }
            entries.insert(3, (root.join("notes.txt"), Some(b"not an image".to_vec()), false));
            entries.insert(6, (root.join("cut.jpg"), Some(truncated[..truncated.len() / 2].to_vec()), false));
            for (path, bytes, _) in &entries { if let Some(bytes) = bytes { std::fs::write(path, bytes).unwrap(); } }

            let paths = entries.iter().map(|(path, _, _)| path.clone()).collect::<Vec<_>>();
            let results = inspect_image_metadata(MetadataRequest { paths: paths.clone(), output_location: OutputLocation::AlongsideInput });

            assert_eq!(results.len(), entries.len(), "exactly one result per requested file");
            let readable = entries.iter().filter(|(_, _, readable)| *readable).count();
            assert_eq!(results.iter().filter(|result| matches!(result, MetadataInspection::Report(_))).count(), readable, "one report per readable file");
            assert_eq!(results.iter().filter(|result| matches!(result, MetadataInspection::Error(_))).count(), entries.len() - readable, "one error per unreadable file");
            for (result, (path, bytes, readable)) in results.iter().zip(&entries) {
                match result {
                    MetadataInspection::Report(report) => {
                        assert!(*readable, "{} should have failed", path.display());
                        assert_eq!(&report.path, path, "reports keep request order");
                        let carries = path.file_stem().unwrap() == "camera";
                        assert_eq!((report.metadata.exif, report.metadata.gps, report.metadata.xmp, report.metadata.icc), (carries, carries, carries, carries), "{}", path.display());
                        assert_eq!(report.metadata.orientation, carries.then_some(6), "{}", path.display());
                    }
                    MetadataInspection::Error(error) => {
                        assert!(!readable, "{} should have been read: {}", path.display(), error.error.message);
                        assert_eq!(&error.path, path, "errors keep request order");
                        assert!(!error.error.message.is_empty());
                    }
                }
                if let Some(bytes) = bytes { assert_eq!(&std::fs::read(path).unwrap(), bytes, "inspection never modifies {}", path.display()); }
            }
            let codes = results.iter().filter_map(|result| match result { MetadataInspection::Error(error) => Some(error.error.code), _ => None }).collect::<Vec<_>>();
            assert_eq!(codes, vec![MetadataErrorCode::Unreadable, MetadataErrorCode::Unreadable, MetadataErrorCode::UnsupportedFormat, MetadataErrorCode::Truncated]);

            let wire = serde_json::to_value(&results).unwrap();
            let statuses = wire.as_array().unwrap().iter().map(|item| item["status"].as_str().unwrap().to_string()).collect::<Vec<_>>();
            let expected = entries.iter().map(|(_, _, readable)| if *readable { "report" } else { "error" }.to_string()).collect::<Vec<_>>();
            assert_eq!(statuses, expected, "every wire item is tagged report or error for the UI");
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn every_truncation_of_every_format_is_an_error_and_never_panics() {
            let all = Wanted { exif: true, gps: true, orientation: Some(8), xmp: true, iptc: true, comments: true, icc: true };
            for (format, _) in FORMATS {
                for wanted in [Wanted::default(), all] {
                    let bytes = fixture(format, wanted);
                    for length in 0..bytes.len() {
                        assert!(crate::kit::images::metadata::read(&bytes[..length]).is_err(), "{format:?} cut at {length} of {} read as valid", bytes.len());
                    }
                }
            }
        }
    }

    /// A stand-in for pdftoppm: it writes one image where the real renderer would, and never
    /// finishes for an input named `slow.pdf`.
    #[cfg(unix)]
    fn fake_pdf_renderer(root: &std::path::Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = root.join("pdftoppm");
        std::fs::write(&script, "#!/bin/sh\nsingle=\nfor arg in \"$@\"; do prefix=\"$arg\"; case \"$arg\" in *slow.pdf) exec sleep 30;; -singlefile) single=1;; esac; done\nif [ -n \"$single\" ]; then printf page > \"$prefix.png\"; else printf page > \"$prefix-1.png\"; fi\n").unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();
        script
    }

    fn run_pdf_to_images_with_renderer(renderer: &std::path::Path, paths: Vec<PathBuf>, output_location: OutputLocation) -> Vec<JobOutcome> {
        let previous = std::env::var_os("TOOLBOX_PDFTOPPM_PATH");
        std::env::set_var("TOOLBOX_PDFTOPPM_PATH", renderer);
        let outcomes = tauri::async_runtime::block_on(pdf_to_images(RunEvents::silent(), PdfToImagesRequest {
            paths, dpi: 72, format: "png".into(), page_range: None, pages: None, picks: Vec::new(), output_location,
        }));
        match previous {
            Some(value) => std::env::set_var("TOOLBOX_PDFTOPPM_PATH", value),
            None => std::env::remove_var("TOOLBOX_PDFTOPPM_PATH"),
        }
        outcomes
    }

    #[test]
    fn pdf_to_images_fails_each_file_as_unavailable_when_the_renderer_override_points_nowhere() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("pdf-to-images-no-renderer");
        let inputs = ["first", "second"].map(|stem| root.join(format!("{stem}.pdf"))).to_vec();
        for input in &inputs { make_pdf(input, 2); }
        let output = location(&root, "images");

        let outcomes = run_pdf_to_images_with_renderer(&root.join("missing").join("pdftoppm"), inputs.clone(), output.clone());

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs);
        for outcome in &outcomes {
            match &outcome.state {
                OutcomeState::Failed { output_paths, error } => {
                    assert_eq!(error.kind, ErrorKind::Unavailable);
                    assert_eq!(error.message, crate::kit::pdf::remaining::PDF_RENDERER_UNAVAILABLE);
                    assert!(error.message.contains("bundled helper") && error.message.contains("not with your PDF"), "{}", error.message);
                    assert!(output_paths.is_empty());
                }
                other => panic!("a missing renderer must fail the file, got {other:?}"),
            }
        }
        let OutputLocation::CustomFolder(folder) = output else { unreachable!() };
        assert_eq!(std::fs::read_dir(folder).unwrap().count(), 0, "nothing is saved without a renderer");
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn pdf_to_images_fails_protected_and_damaged_pdfs_alone_while_the_rest_render() {
        use crate::kit::contracts::OutcomePointer;
        let _split = crate::kit::pdf::remaining::tests::SPLIT_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("pdf-to-images-mixed-batch");
        let renderer = fake_pdf_renderer(&root);
        let inputs = ["first", "locked", "damaged", "last"].map(|stem| root.join(format!("{stem}.pdf"))).to_vec();
        make_pdf(&inputs[0], 1);
        std::fs::write(&inputs[1], crate::kit::password::tests::pdf(Some(("owner", "user")))).unwrap();
        std::fs::write(&inputs[2], b"%PDF-1.7 damaged").unwrap();
        make_pdf(&inputs[3], 1);
        let originals = inputs.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let outcomes = run_pdf_to_images_with_renderer(&renderer, inputs.clone(), location(&root, "images"));

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs, "the response keeps input order");
        for rendered in [&outcomes[0], &outcomes[3]] {
            assert!(rendered.error().is_none(), "{}", rendered.error().cloned().unwrap_or_default());
            assert_eq!(rendered.output_paths().len(), 1);
            assert!(rendered.output_paths()[0].is_file());
        }
        let protected = outcomes[1].error().expect("a password-protected PDF fails");
        assert_eq!(protected.kind, ErrorKind::InvalidInput);
        assert_eq!(protected.message, crate::kit::pdf::remaining::PDF_TO_IMAGES_PASSWORD_PROTECTED);
        assert_eq!(protected.pointer, Some(OutcomePointer::Tool { tool_id: "lock-unlock".into() }), "the row points to Remove Password");
        assert!(outcomes[1].output_paths().is_empty());
        let damaged = outcomes[2].error().expect("a damaged PDF fails");
        assert_eq!(damaged, &crate::kit::password::unreadable(crate::kit::password::ProtectedFormat::Pdf));
        assert!(outcomes[2].output_paths().is_empty());
        for (path, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(path).unwrap(), original, "originals stay untouched");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
