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
use crate::kit::pdf::{editor, metadata, remaining, scene, PDFProcessor};
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

/// Rejects the whole run when a ranged page scope can't apply to any file, before any file is read.
fn check_ranged_scope(command: &str, paths: &[std::path::PathBuf], scope: &editor::RangedPageScope) -> Result<(), Vec<JobOutcome>> {
    match scope {
        editor::RangedPageScope::All => Ok(()),
        editor::RangedPageScope::Selected { pages } => validate_page_selection(command, paths, Some(pages.as_slice())),
        editor::RangedPageScope::Range { ranges } => require_page_ranges(paths, ranges).map(drop),
    }
}

#[tauri::command]
async fn crop_pdf(events: RunEvents, request: editor::CropPdfRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = check_ranged_scope("crop_pdf", &request.paths, &request.scope) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "crop_pdf", request.paths.clone(), &request.output_location, &request, editor::crop)
}

#[tauri::command]
async fn resize_pdf(events: RunEvents, request: editor::ResizePdfRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = check_ranged_scope("resize_pdf", &request.paths, &request.scope) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "resize_pdf", request.paths.clone(), &request.output_location, &request, editor::resize)
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

#[tauri::command]
async fn repair_pdf(events: RunEvents, request: remaining::PdfToTextRequest) -> Vec<JobOutcome> {
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

#[tauri::command]
async fn compress_pdf(events: RunEvents, request: remaining::CompressPdfRequest) -> Vec<JobOutcome> {
    if let Err(outcomes) = validate_command_inputs("compress_pdf", &request.paths) { return outcomes; }
    let request = Arc::new(request);
    BatchRunner::run(&events, "compress_pdf", request.paths.clone(), &request.output_location, &request, |request, path| with_measured_sizes(remaining::compress(request, path)))
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
    if let Err(outcomes) = validate_page_ranges(&request.paths, request.split_page_ranges()) { return outcomes; }
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
async fn ocr_pdf(app: tauri::AppHandle, events: RunEvents, request: vision::OcrRequest) -> Vec<JobOutcome> {
    let packs = vision::OcrPacks::new(packs::app_data(&app), app.package_info().version.to_string(), packs::load_catalog());
    run_ocr_pdf(events, request, packs).await
}

async fn run_ocr_pdf(events: RunEvents, request: vision::OcrRequest, packs: vision::OcrPacks) -> Vec<JobOutcome> {
    if let Err(outcomes) = vision::validate_ocr_request(&request, &packs) { return outcomes; }
    if let Err(outcomes) = validate_command_inputs("ocr_pdf", &request.paths) { return outcomes; }
    let run = Arc::new(vision::OcrRun { request, packs });
    BatchRunner::run(&events, "ocr_pdf", run.request.paths.clone(), &run.request.output_location, &run, vision::ocr_pdf)
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
fn inspect_image_metadata(request: tools::MetadataRequest) -> Vec<crate::kit::images::metadata::MetadataInspection> { request.paths.into_iter().map(crate::kit::images::metadata::inspect).collect() }

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
            OutcomeState::Failed { output_paths, error } if input_path != aggregate.input_path => {
                let message = format!("Aggregate operation stopped because {} failed: {}", aggregate.input_path.display(), error.message);
                OutcomeState::Failed {
                    output_paths: output_paths.clone(),
                    error: if error.kind == crate::kit::contracts::ErrorKind::Unavailable { ToolError::unavailable(message) } else { ToolError::processing(message) },
                }
            }
            state => state.clone(),
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
            resize_pdf,
            edit_pdf,
            edit_pdf_session,
            sign_pdf,
            add_page_numbers,
            watermark_pdf,
            compress_pdf,
            rotate_pdf,
            repair_pdf,
            redact_pdf,
            compare_pdfs,
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
            export_image_edit_plan,
            get_quick_actions,
            set_quick_actions,
            packs::list_packs,
            packs::ocr_languages,
            packs::install::inspect_pack_file,
            packs::install::install_pack_file,
            packs::download::install_pack,
            packs::uninstall::uninstall_pack
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
    use crate::kit::images::tools::{CropRequest, GifCreateRequest, GifExtractRequest, ImageEdit, ImageEditExportRequest, ImageEditPlan, ImageEditPreviewRequest, IconSetRequest, MetadataRequest, ResizeRequest, RotateRequest, TiffRequest, ToneRequest, WatermarkRequest};
    use crate::kit::office::{test_minimal_ooxml_package, OfficeProcessor};
    use crate::kit::pdf::editor::{CropPdfRequest, CropSpec, EditPdfRequest, PageScope, PdfEditMode, PdfEditOperation, PdfEditSessionPlan, PdfEditSessionRequest, PdfOverlay, PdfOverlayPosition, PdfRect, RangedPageScope, RedactPdfRequest, ResizeOrientation, ResizePdfRequest, ResizePlacement, RotatePage, SignPdfRequest, TargetSize};
    use crate::kit::pdf::remaining::{CompressPdfRequest, ImagesToPdfRequest, MergePdfRequest, PageOverlayRequest, PageSelectionRequest, PdfToImagesRequest, PdfWatermarkRequest, PdfToTextRequest, RotatePdfRequest, ComparePdfsRequest};
    use crate::kit::vision::VisionRequest;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use image::Rgba;
    use std::path::PathBuf;
    use std::future::Future;

    /// The OCR command as a fresh install runs it: English only, with no packs or catalog to find.
    async fn ocr_pdf(events: RunEvents, request: vision::OcrRequest) -> Vec<JobOutcome> {
        run_ocr_pdf(events, request, no_ocr_packs()).await
    }

    fn no_ocr_packs() -> vision::OcrPacks {
        vision::OcrPacks::new(Err("No packs in this test.".into()), "0.0.0", Err("No catalog in this test.".into()))
    }

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
        let pages = (1..=page_count).map(|number| format!("BT /F1 24 Tf 72 720 Td (Toolbox page {number}) Tj ET")).collect::<Vec<_>>();
        pdf_of_pages(path, &pages);
    }

    /// Writes a PDF with one page per content stream. Every page can use Helvetica as /F1.
    fn pdf_of_pages(path: &PathBuf, pages: &[String]) {
        use lopdf::{dictionary, Document, Object, Stream};
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let mut kids = Vec::new();
        for content in pages {
            let content_id = document.add_object(Stream::new(dictionary! {}, content.clone().into_bytes()));
            let page_id = document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id,
                "MediaBox" => vec![Object::Real(0.5), Object::Real(0.5), Object::Real(612.5), Object::Real(792.5)],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } }, "Contents" => content_id,
            });
            kids.push(Object::Reference(page_id));
        }
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages.len() as i64 }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    /// A page with a drawing and no text at all, the way a scan looks before OCR.
    fn drawn_page(number: usize) -> String { format!("q 0 0 0 rg 72 720 {} 10 re f Q", 40 * number) }

    /// Writes a PDF whose pages have no text, so OCR recognises every one of them.
    fn drawn_pdf(path: &PathBuf, page_count: usize) {
        pdf_of_pages(path, &(1..=page_count).map(drawn_page).collect::<Vec<_>>());
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
        CropPdfRequest { paths, spec: CropSpec::Margins { top: 392.5, right: 312.5, bottom: 99.5, left: 99.5 }, scope, output_location }
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

    /// Each page's CropBox (if any), MediaBox and content bytes, by page number.
    fn page_boxes_and_contents(path: &std::path::Path) -> Vec<(Option<Vec<f32>>, Vec<f32>, Vec<u8>)> {
        let document = lopdf::Document::load(path).unwrap();
        let floats = |value: &lopdf::Object| value.as_array().unwrap().iter().map(|number| number.as_float().unwrap()).collect::<Vec<_>>();
        document.get_pages().into_values().map(|page_id| {
            let page = document.get_dictionary(page_id).unwrap();
            (page.get(b"CropBox").ok().map(floats), floats(page.get(b"MediaBox").unwrap()), document.get_page_content(page_id))
        }).collect()
    }

    #[test]
    fn crop_command_scopes_a_mixed_batch_to_the_typed_range_with_skipped_signed_and_failed_files_isolated() {
        use crate::kit::contracts::{OutcomeWarning, SkipCode, WarningKind};
        let root = sandbox("crop-scoped-batch");
        let (lease, nda, cover, broken) = (root.join("lease.pdf"), root.join("vendor-nda.pdf"), root.join("cover.pdf"), root.join("broken.pdf"));
        make_pdf(&lease, 5); make_pdf(&nda, 4); make_pdf(&cover, 2);
        let mut document = lopdf::Document::load(&nda).unwrap();
        crate::kit::pdf::tests::sign(&mut document);
        document.save(&nda).unwrap();
        std::fs::write(&broken, b"%PDF-1.7 not really").unwrap();
        let inputs = vec![lease.clone(), nda.clone(), cover.clone(), broken.clone()];
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();
        let before = [&lease, &nda].map(|path| page_boxes_and_contents(path));

        let outcomes = tauri::async_runtime::block_on(crop_pdf(RunEvents::silent(), crop_request(inputs.clone(), RangedPageScope::Range { ranges: "3-".into() }, location(&root, "out"))));
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs);

        let removed = vec![OutcomeWarning { kind: WarningKind::SignatureRemoved, message: crate::kit::pdf::SIGNATURE_REMOVED.into() }];
        for ((outcome, before), (pages, warnings)) in outcomes.iter().zip(&before).zip([(5, vec![]), (4, removed)]) {
            assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
            assert_eq!(outcome.detail, "PDF saved");
            assert_eq!(outcome.warnings, warnings);
            let after = page_boxes_and_contents(&outcome.output_paths()[0]);
            assert_eq!(after.len(), pages);
            for (number, (page_after, page_before)) in after.iter().zip(before).enumerate() {
                if number < 2 {
                    assert_eq!(page_after, page_before, "page {} is outside 3- and must be unchanged", number + 1);
                    assert!(page_after.0.is_none());
                } else {
                    assert_eq!(page_after.0.as_deref(), Some(&[100.0, 100.0, 300.0, 400.0][..]), "page {}", number + 1);
                    assert_eq!((&page_after.1, &page_after.2), (&page_before.1, &page_before.2), "page {} keeps its MediaBox and content", number + 1);
                }
            }
        }
        assert!(!crate::kit::pdf::has_signed_signature_field(&lopdf::Document::load(&outcomes[1].output_paths()[0]).unwrap()));

        match &outcomes[2].state {
            OutcomeState::Skipped { reason } => {
                assert_eq!(reason.code, SkipCode::RangeEmptyAfterClamping);
                assert_eq!(reason.message, "No pages in 3- are in this PDF, which has 2 pages.");
            }
            state => panic!("expected cover.pdf to be skipped, got {state:?}"),
        }
        assert!(outcomes[2].output_paths().is_empty() && outcomes[2].warnings.is_empty());
        assert!(outcomes[3].error().is_some() && outcomes[3].output_paths().is_empty());

        let mut written = std::fs::read_dir(root.join("out")).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect::<Vec<_>>();
        written.sort();
        assert_eq!(written, vec!["lease-cropped.pdf", "vendor-nda-cropped.pdf"]);
        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resize_command_applies_the_view_payload_to_a_mixed_batch_with_skipped_signed_and_failed_files_isolated() {
        use crate::kit::contracts::{OutcomeWarning, SkipCode, WarningKind};
        let root = sandbox("resize-scoped-batch");
        let (lease, nda, already, broken) = (root.join("lease.pdf"), root.join("vendor-nda.pdf"), root.join("already-a4.pdf"), root.join("broken.pdf"));
        make_pdf(&lease, 5); make_pdf(&nda, 4); make_pdf(&already, 3);
        let mut document = lopdf::Document::load(&nda).unwrap();
        crate::kit::pdf::tests::sign(&mut document);
        document.save(&nda).unwrap();
        let mut document = lopdf::Document::load(&already).unwrap();
        for page in document.get_pages().into_values() {
            document.get_dictionary_mut(page).unwrap().set("MediaBox", vec![lopdf::Object::Integer(0), 0.into(), lopdf::Object::Real(595.2756), lopdf::Object::Real(841.8898)]);
        }
        document.save(&already).unwrap();
        std::fs::write(&broken, b"%PDF-1.7 not really").unwrap();
        let inputs = vec![lease.clone(), nda.clone(), already.clone(), broken.clone()];
        let originals = inputs.iter().map(|input| std::fs::read(input).unwrap()).collect::<Vec<_>>();
        let before = [&lease, &nda].map(|path| page_boxes_and_contents(path));

        // The exact request the Crop and Resize view sends in Resize mode.
        let request = serde_json::from_value::<ResizePdfRequest>(serde_json::json!({
            "paths": inputs.clone(),
            "size": { "preset": "a4" },
            "orientation": "match",
            "placement": "fit",
            "scope": { "range": { "ranges": "3-" } },
            "outputLocation": location(&root, "out"),
        })).expect("Crop and Resize view payload for resize_pdf");
        let outcomes = tauri::async_runtime::block_on(resize_pdf(RunEvents::silent(), request));
        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs);

        let removed = vec![OutcomeWarning { kind: WarningKind::SignatureRemoved, message: crate::kit::pdf::SIGNATURE_REMOVED.into() }];
        for ((outcome, before), (pages, detail, warnings)) in outcomes.iter().zip(&before).zip([(5, "Pages 3\u{2013}5 changed to A4.", vec![]), (4, "Pages 3\u{2013}4 changed to A4.", removed)]) {
            assert!(outcome.error().is_none(), "{}", outcome.error().cloned().unwrap_or_default());
            assert_eq!(outcome.detail, detail);
            assert_eq!(outcome.warnings, warnings);
            let after = page_boxes_and_contents(&outcome.output_paths()[0]);
            assert_eq!(after.len(), pages);
            for (number, (page_after, page_before)) in after.iter().zip(before).enumerate() {
                if number < 2 {
                    assert_eq!(page_after, page_before, "page {} is outside 3- and must be unchanged", number + 1);
                } else {
                    let a4 = [0.0, 0.0, 595.2756, 841.8898];
                    assert!(page_after.1.iter().zip(a4).all(|(actual, expected)| (actual - expected).abs() < 1e-3), "page {}: {:?}", number + 1, page_after.1);
                    assert_eq!(page_after.0.as_deref(), Some(&page_after.1[..]), "page {} crop box is the new paper", number + 1);
                }
            }
        }
        assert!(!crate::kit::pdf::has_signed_signature_field(&lopdf::Document::load(&outcomes[1].output_paths()[0]).unwrap()));

        match &outcomes[2].state {
            OutcomeState::Skipped { reason } => {
                assert_eq!(reason.code, SkipCode::NoEffectiveChange);
                assert_eq!(reason.message, "Page 3 is already A4. Nothing was saved.");
            }
            state => panic!("expected already-a4.pdf to be skipped, got {state:?}"),
        }
        assert!(outcomes[2].output_paths().is_empty());
        assert!(outcomes[3].error().is_some() && outcomes[3].output_paths().is_empty());

        let mut written = std::fs::read_dir(root.join("out")).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect::<Vec<_>>();
        written.sort();
        assert_eq!(written, vec!["lease-resized.pdf", "vendor-nda-resized.pdf"]);
        for (input, original) in inputs.iter().zip(originals) {
            assert_eq!(std::fs::read(input).unwrap(), original, "original was modified: {}", input.display());
        }

        // A malformed range is rejected for every file before anything is read or written.
        let outcomes = tauri::async_runtime::block_on(resize_pdf(RunEvents::silent(), ResizePdfRequest {
            paths: vec![lease.clone(), nda.clone()], size: TargetSize::Preset("letter".into()), orientation: ResizeOrientation::Match,
            placement: ResizePlacement::Fit, scope: RangedPageScope::Range { ranges: "4-2".into() }, output_location: location(&root, "bad-range"),
        }));
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|outcome| outcome.error().is_some_and(|error| error.message.contains("\"4-2\" runs backwards"))), "{outcomes:?}");
        assert!(!root.join("bad-range").exists() || std::fs::read_dir(root.join("bad-range")).unwrap().count() == 0);
        let _ = std::fs::remove_dir_all(root);
    }

    /// One batch of good, locked, damaged, malformed and over-limit PDFs for Crop and for Resize: each bad file fails
    /// alone with its reason, the good ones are saved next to their originals or in the chosen folder, a second run
    /// takes the next free name with the same bytes, and no original changes.
    #[test]
    fn crop_and_resize_isolate_locked_damaged_malformed_and_over_limit_pdfs_in_either_output_location() {
        use crate::kit::contracts::OutcomePointer;
        let root = sandbox("crop-resize-safety");
        let folder = root.join("in");
        std::fs::create_dir_all(&folder).unwrap();
        let (lease, scan, broken, no_media_box, orphan, archive, huge) = (
            folder.join("lease.pdf"), folder.join("scan.pdf"), folder.join("broken.pdf"), folder.join("no-media-box.pdf"),
            folder.join("orphan-page.pdf"), folder.join("archive.pdf"), folder.join("huge.pdf"),
        );
        make_pdf(&lease, 3); make_pdf(&scan, 2); make_pdf(&no_media_box, 3); make_pdf(&orphan, 3); make_pdf(&archive, 5_001);
        let locked = locked_pdf(&root, "payroll", crate::kit::TEST_PASSWORD);
        std::fs::write(&broken, b"%PDF-1.7 not really").unwrap();
        std::fs::write(&huge, b"%PDF-1.7\n").unwrap();
        std::fs::OpenOptions::new().write(true).open(&huge).unwrap().set_len(1_000_000_001).unwrap();
        let mut document = lopdf::Document::load(&no_media_box).unwrap();
        let second = document.get_pages()[&2];
        document.get_dictionary_mut(second).unwrap().remove(b"MediaBox");
        document.save(&no_media_box).unwrap();
        let mut document = lopdf::Document::load(&orphan).unwrap();
        let second = document.get_pages()[&2];
        document.get_dictionary_mut(second).unwrap().set("Parent", lopdf::Object::Reference(second));
        document.save(&orphan).unwrap();

        let inputs = vec![lease.clone(), locked.clone(), broken.clone(), no_media_box.clone(), orphan.clone(), archive.clone(), scan.clone(), huge.clone()];
        let small = inputs.iter().filter(|input| **input != huge).map(|input| (input.clone(), std::fs::read(input).unwrap())).collect::<Vec<_>>();
        let huge_original = Original::of(&huge);
        let assert_originals = || {
            for (input, original) in &small { assert_eq!(&std::fs::read(input).unwrap(), original, "original was modified: {}", input.display()); }
            huge_original.assert_untouched();
        };

        let assert_batch = |outcomes: &[JobOutcome], tool: &str| {
            assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), inputs, "{tool}");
            for good in [&outcomes[0], &outcomes[6]] {
                assert!(good.error().is_none(), "{tool}: {}", good.error().cloned().unwrap_or_default());
                assert!(good.output_paths().len() == 1 && good.output_paths()[0].is_file(), "{tool}");
            }
            let error = |index: usize| outcomes[index].error().cloned().unwrap_or_else(|| panic!("{tool}: {} must fail", inputs[index].display()));
            for index in 1..6 { assert!(outcomes[index].output_paths().is_empty(), "{tool}: {}", inputs[index].display()); }
            assert!(outcomes[7].output_paths().is_empty(), "{tool}");

            let locked_error = error(1);
            assert_eq!((locked_error.kind, locked_error.message.as_str()), (ErrorKind::InvalidInput, editor::LOCKED_PDF), "{tool}");
            assert_eq!(locked_error.pointer, Some(OutcomePointer::Tool { tool_id: "lock-unlock".into() }), "{tool}");
            assert_eq!(error(2).kind, ErrorKind::Processing, "{tool}");
            assert_eq!(error(3).message, "Page 2 can't be read: PDF page has no media box.", "{tool}");
            assert!(error(4).message.starts_with("Page 2 has a missing or wrong /Parent, so the PDF page-tree is malformed"), "{tool}: {}", error(4).message);
            let pages = error(5);
            assert_eq!(pages.kind, ErrorKind::LimitExceeded { limit: LimitKind::PageCount }, "{tool}");
            assert_eq!(pages.message, "More than 5,000 pages, over the limit for one PDF. Use fewer pages and try again.", "{tool}");
            let size = error(7);
            assert_eq!(size.kind, ErrorKind::LimitExceeded { limit: LimitKind::FileSize }, "{tool}");
            assert_eq!(size.message, "Larger than the 1 GB limit for one PDF. Reduce its size and try again.", "{tool}");
        };

        type Run = Box<dyn Fn(OutputLocation) -> Vec<JobOutcome>>;
        let crop: Run = Box::new({
            let inputs = inputs.clone();
            move |output_location| tauri::async_runtime::block_on(crop_pdf(RunEvents::silent(), crop_request(inputs.clone(), RangedPageScope::All, output_location)))
        });
        let resize: Run = Box::new({
            let inputs = inputs.clone();
            move |output_location| tauri::async_runtime::block_on(resize_pdf(RunEvents::silent(), ResizePdfRequest {
                paths: inputs.clone(), size: TargetSize::Preset("a4".into()), orientation: ResizeOrientation::Match,
                placement: ResizePlacement::Fit, scope: RangedPageScope::All, output_location,
            }))
        });
        for (tool, suffix, run) in [("crop", "-cropped", crop), ("resize", "-resized", resize)] {
            let chosen = location(&root, &format!("{tool}-out"));
            for (output_location, destination) in [(chosen.clone(), root.join(format!("{tool}-out"))), (OutputLocation::AlongsideInput, folder.clone())] {
                let first = run(output_location.clone());
                assert_batch(&first, tool);
                assert_originals();
                let second = run(output_location);
                assert_batch(&second, tool);
                assert_originals();
                for (index, name) in [(0, "lease"), (6, "scan")] {
                    assert_eq!(first[index].output_paths()[0], destination.join(format!("{name}{suffix}.pdf")), "{tool}");
                    assert_eq!(second[index].output_paths()[0], destination.join(format!("{name}{suffix}-1.pdf")), "{tool}: a taken name gets -1");
                    assert_eq!(std::fs::read(&first[index].output_paths()[0]).unwrap(), std::fs::read(&second[index].output_paths()[0]).unwrap(), "{tool}: {name} runs are byte-identical");
                }
                let written = file_names(&destination).into_iter().filter(|name| name.contains(suffix)).collect::<Vec<_>>();
                assert_eq!(written, [format!("lease{suffix}-1.pdf"), format!("lease{suffix}.pdf"), format!("scan{suffix}-1.pdf"), format!("scan{suffix}.pdf")], "{tool}: only the good files are written");
            }
        }
        assert!(file_names(&root.join("locked")).iter().all(|name| !name.contains("-cropped") && !name.contains("-resized")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resize_command_applies_a_custom_size_payload_and_rejects_one_out_of_bounds_for_every_file() {
        use crate::kit::contracts::ErrorKind;
        let root = sandbox("resize-custom-batch");
        let (lease, broken) = (root.join("lease.pdf"), root.join("broken.pdf"));
        make_pdf(&lease, 2);
        std::fs::write(&broken, b"%PDF-1.7 not really").unwrap();
        let original = std::fs::read(&lease).unwrap();
        let payload = |width: f64, height: f64, out: &str| serde_json::from_value::<ResizePdfRequest>(serde_json::json!({
            "paths": [lease.clone(), broken.clone()],
            "size": { "custom": { "width": width, "height": height } },
            "orientation": "landscape",
            "placement": "actualSize",
            "scope": "all",
            "outputLocation": location(&root, out),
        })).expect("Crop and Resize view payload for a custom resize");

        // 200 x 250 mm in points, turned landscape, at actual size.
        let outcomes = tauri::async_runtime::block_on(resize_pdf(RunEvents::silent(), payload(566.9291, 708.6614, "out")));
        assert!(outcomes[0].error().is_none(), "{:?}", outcomes[0].error());
        assert_eq!(outcomes[0].detail, "Pages 1\u{2013}2 changed to 200 \u{d7} 250 mm.");
        for (_, media, _) in page_boxes_and_contents(&outcomes[0].output_paths()[0]) {
            assert!(media.iter().zip([0.0, 0.0, 708.6614, 566.9291]).all(|(actual, expected)| (actual - expected).abs() < 1e-3), "{media:?}");
        }
        assert!(outcomes[1].error().is_some() && outcomes[1].output_paths().is_empty(), "the broken file fails on its own");

        // Out of bounds fails every file with InvalidInput and writes nothing.
        let outcomes = tauri::async_runtime::block_on(resize_pdf(RunEvents::silent(), payload(71.0, 708.6614, "too-small")));
        assert!(outcomes[0].error().is_some_and(|error| matches!(error.kind, ErrorKind::InvalidInput) && error.message == "Width must be at least 25.4 mm (72 pt)."), "{:?}", outcomes[0]);
        assert!(outcomes.iter().all(|outcome| outcome.error().is_some() && outcome.output_paths().is_empty()));
        assert!(!root.join("too-small").exists() || std::fs::read_dir(root.join("too-small")).unwrap().count() == 0);
        assert_eq!(std::fs::read(&lease).unwrap(), original);
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
    fn crop_command_applies_one_relative_rectangle_from_the_view_payload_to_every_page_size() {
        use lopdf::{Document, Object};
        let root = sandbox("crop-rectangle-batch");
        let (a4, letter, turned) = (root.join("a4.pdf"), root.join("letter.pdf"), root.join("turned.pdf"));
        for (path, media, rotate) in [(&a4, [595.2756, 841.8898], None), (&letter, [612.0, 792.0], None), (&turned, [612.0, 792.0], Some(90))] {
            make_pdf(path, 1);
            let mut document = Document::load(path).unwrap();
            let page = document.get_pages().values().copied().next().unwrap();
            let page = document.get_dictionary_mut(page).unwrap();
            page.set("MediaBox", vec![Object::Real(0.0), Object::Real(0.0), Object::Real(media[0]), Object::Real(media[1])]);
            if let Some(rotate) = rotate { page.set("Rotate", rotate); }
            document.save(path).unwrap();
        }
        let originals = [&a4, &letter, &turned].map(|path| std::fs::read(path).unwrap());
        // The exact request the Crop and Resize view sends: displayed-page fractions with a top-left origin.
        let request = serde_json::from_value::<CropPdfRequest>(serde_json::json!({
            "paths": [a4.clone(), letter.clone(), turned.clone()],
            "spec": { "kind": "rectangle", "left": 0.25, "top": 0.125, "width": 0.5, "height": 0.75 },
            "scope": "all",
            "outputLocation": location(&root, "crop-rectangle"),
        })).expect("Crop and Resize view payload for crop_pdf");
        let outcomes = tauri::async_runtime::block_on(crop_pdf(RunEvents::silent(), request));

        assert_eq!(outcomes.len(), 3);
        for ((outcome, original), path) in outcomes.iter().zip(&originals).zip([&a4, &letter, &turned]) {
            assert!(outcome.error().is_none(), "{path:?}: {:?}", outcome.error());
            assert_eq!(&std::fs::read(path).unwrap(), original, "{path:?}: original must stay");
        }
        let crop_box = |outcome: &JobOutcome| {
            let document = Document::load(outcome.output_paths().first().unwrap()).unwrap();
            let page = document.get_pages().values().copied().next().unwrap();
            let values = document.get_dictionary(page).unwrap().get(b"CropBox").unwrap().as_array().unwrap().iter().map(|value| value.as_float().unwrap()).collect::<Vec<_>>();
            [values[0], values[1], values[2], values[3]]
        };
        let near = |actual: [f32; 4], expected: [f32; 4]| actual.iter().zip(expected).all(|(a, e)| (a - e).abs() < 0.01);
        // The same quarter-in, eighth-down area of each upright page, in its own points.
        let a4_box = crop_box(&outcomes[0]);
        assert!(near(a4_box, [148.8189, 105.2362, 446.4567, 736.6536]), "{a4_box:?}");
        let letter_box = crop_box(&outcomes[1]);
        assert!(near(letter_box, [153.0, 99.0, 459.0, 693.0]), "{letter_box:?}");
        // On screen the turned page is 792 x 612; its displayed top is the user-space left edge.
        let turned_box = crop_box(&outcomes[2]);
        assert!(near(turned_box, [76.5, 198.0, 535.5, 594.0]), "{turned_box:?}");

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
            let mut request = vision::OcrRequest { paths: vec![input.clone()], page_range: Some("1".into()), mode: "text".into(), force: false, languages: vec!["eng".into()], language_overrides: Default::default(), output_location: location(&root, "ocr") };
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

    /// A white letter page at 150 pixels per inch with `lines` inked in black from the
    /// bundled font, the way a scanner sees printed text. Returns the raster and where
    /// each word is inked, as left, top, right and bottom pixels.
    fn scanned_page(lines: &[&str]) -> (image::GrayImage, Vec<(String, [i32; 4])>) {
        const WIDTH: usize = 1275;
        const HEIGHT: usize = 1650;
        const SIZE: f32 = 56.0;
        let font = fontdue::Font::from_bytes(include_bytes!("../resources/fonts/DejaVuSans.ttf") as &[u8], fontdue::FontSettings::default()).unwrap();
        let mut pixels = vec![255u8; WIDTH * HEIGHT];
        let mut words = Vec::new();
        for (row, line) in lines.iter().enumerate() {
            let baseline = 200 + row * 120;
            let mut pen = 120.0f32;
            let mut word: Option<(String, [i32; 4])> = None;
            for character in line.chars() {
                let (metrics, coverage) = font.rasterize(character, SIZE);
                let top = baseline as i32 - metrics.height as i32 - metrics.ymin;
                if character == ' ' {
                    words.extend(word.take());
                } else {
                    let (left, right) = (pen as i32 + metrics.xmin, pen as i32 + metrics.xmin + metrics.width as i32);
                    let (text, bounds) = word.get_or_insert_with(|| (String::new(), [left, top, right, top + metrics.height as i32]));
                    text.push(character);
                    *bounds = [bounds[0].min(left), bounds[1].min(top), bounds[2].max(right), bounds[3].max(top + metrics.height as i32)];
                }
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
            words.extend(word);
        }
        (image::GrayImage::from_raw(WIDTH as u32, HEIGHT as u32, pixels).unwrap(), words)
    }

    /// Writes a PDF whose pages are only pictures of text, the way a scanner makes them.
    /// There is no text layer, so the words can come back only through OCR. A page with
    /// no lines is a blank white scan. Returns where each page's words are inked, as
    /// left, top, right and bottom in points from the page's top left.
    fn scanned_pdf(path: &PathBuf, pages: &[&[&str]]) -> Vec<Vec<(String, [f32; 4])>> {
        use flate2::{write::ZlibEncoder, Compression};
        use lopdf::{dictionary, Document, Object, Stream};
        use std::io::Write;
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let mut kids = Vec::new();
        let mut inked = Vec::new();
        for lines in pages {
            let (raster, words) = scanned_page(lines);
            let points = 612.0 / raster.width() as f32;
            inked.push(words.into_iter().map(|(text, bounds)| (text, bounds.map(|pixel| pixel as f32 * points))).collect::<Vec<_>>());
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(raster.as_raw()).unwrap();
            let mut scan = Stream::new(dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => raster.width() as i64, "Height" => raster.height() as i64,
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
        inked
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
        vision::OcrRequest { paths, page_range: None, mode: "text".into(), force: false, languages: vec!["eng".into()], language_overrides: Default::default(), output_location }
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
    fn ocr_reads_english_and_a_development_override_language_together_through_the_real_engine() {
        let _guard = crate::kit::lock_process_env();
        let Some(_engine) = real_ocr_engine() else { return };
        let root = sandbox("ocr-two-languages");
        let input = root.join("scan.pdf");
        scanned_pdf(&input, &[&["Quarterly invoice summary", "Northwind shipping ledger"]]);
        // The fixture Hindi language is English's data under Hindi's code, so the real engine
        // reads both languages from the folder they are gathered into.
        let languages = root.join("languages");
        std::fs::create_dir_all(&languages).unwrap();
        let english = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/ocr/tessdata/eng.traineddata");
        std::fs::copy(&english, languages.join("hin.traineddata")).unwrap();
        let _override = EnvGuard::set(&[("TOOLBOX_OCR_LANGUAGE_DIR", Some(languages))]);
        let folder = location(&root, "ocr");
        let run = |folder: OutputLocation| {
            let mut request = ocr_text_request(vec![input.clone()], folder);
            request.languages = vec!["eng".into(), "hin".into()];
            let packs = vision::OcrPacks::new(Ok(root.join("app-data")), "1.0.14", Ok(crate::packs::install::tests::Fixture::new().catalog()));
            tauri::async_runtime::block_on(run_ocr_pdf(RunEvents::silent(), request, packs))
        };

        let first = run(folder.clone());
        let second = run(folder);
        let texts = [&first, &second].map(|outcomes| {
            assert!(outcomes[0].error().is_none(), "{:?}", outcomes[0]);
            assert_eq!(outcomes[0].detail, "1 page recognized. English and Hindi.");
            std::fs::read(&outcomes[0].output_paths()[0]).unwrap()
        });
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(texts[0], texts[1], "the same languages give the same text");
        let text = String::from_utf8(texts[0].clone()).unwrap();
        for word in ["Quarterly", "invoice", "Northwind", "ledger"] { assert!(text.contains(word), "{word} is missing from {text:?}"); }
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
    fn pdf_to_text_hands_a_scan_to_ocr_while_a_text_pdf_in_the_same_batch_succeeds() {
        use crate::kit::contracts::{OutcomePointer, SkipCode};
        let root = sandbox("pdf-to-text-needs-ocr");
        let scan = root.join("scan.pdf");
        let text_pdf = root.join("text.pdf");
        scanned_pdf(&scan, &[&["Only pixels here"]]);
        make_pdf(&text_pdf, 2);
        let paths = vec![scan.clone(), text_pdf.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let outcomes = tauri::async_runtime::block_on(pdf_to_text(RunEvents::silent(), PdfToTextRequest {
            paths: paths.clone(),
            pages: None,
            output_location: location(&root, "text"),
        }));
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
        std::fs::write(&renderer, "#!/bin/sh\nfor prefix; do :; done\ntouch \"$prefix.png\"\n").unwrap();
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
        for input in &inputs { drawn_pdf(input, 1); }
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
        drawn_pdf(&first, 1);
        std::fs::write(&locked, crate::kit::password::tests::pdf(Some(("owner", "user")))).unwrap();
        drawn_pdf(&last, 2);
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

    /// Fake OCR helpers that act on markers in the input's name: the renderer tags the
    /// page it renders, and the engine stalls on a STALL page or prints 6 MB for a WORDY
    /// one. A HUGE input renders a page past the rendered-page limit. Every other page
    /// reads "recognized words".
    #[cfg(unix)]
    fn marked_ocr_helpers(root: &PathBuf) -> EnvGuard {
        use std::os::unix::fs::PermissionsExt;
        let renderer = root.join("marked-pdftoppm");
        let engine = root.join("marked-tesseract");
        let tessdata = root.join("eng.traineddata");
        std::fs::write(&renderer, "#!/bin/sh\nfor prefix; do :; done\ncase \"$*\" in\n  *HUGE*) dd if=/dev/zero of=\"$prefix.png\" bs=1 count=0 seek=52428801 2>/dev/null ;;\n  *STALL*) echo STALL > \"$prefix.png\" ;;\n  *WORDY*) echo WORDY > \"$prefix.png\" ;;\n  *) touch \"$prefix.png\" ;;\nesac\n").unwrap();
        std::fs::write(&engine, "#!/bin/sh\nif grep -q STALL \"$1\" 2>/dev/null; then exec sleep 30; fi\nif grep -q WORDY \"$1\" 2>/dev/null; then head -c 6000000 /dev/zero | tr '\\0' a; exit 0; fi\nprintf 'recognized words\\n'\n").unwrap();
        std::fs::write(&tessdata, b"fake tessdata").unwrap();
        for helper in [&renderer, &engine] { std::fs::set_permissions(helper, std::fs::Permissions::from_mode(0o755)).unwrap(); }
        EnvGuard::set(&[("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-bundle"))), ("TOOLBOX_PDFTOPPM_PATH", Some(renderer)), ("TOOLBOX_TESSERACT_PATH", Some(engine)), ("TOOLBOX_TESSDATA_PATH", Some(tessdata))])
    }

    fn names_in(folder: &std::path::Path) -> Vec<String> {
        let mut names = std::fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        names.sort();
        names
    }

    #[cfg(unix)]
    #[test]
    fn ocr_writes_next_to_the_original_or_into_the_chosen_folder_and_numbers_a_second_run() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-destination");
        let _env = marked_ocr_helpers(&root);
        let inputs = root.join("inputs");
        std::fs::create_dir_all(&inputs).unwrap();
        let scan = inputs.join("scan.pdf");
        drawn_pdf(&scan, 1);
        let original = std::fs::read(&scan).unwrap();
        let chosen = root.join("chosen");
        std::fs::create_dir_all(&chosen).unwrap();

        for (location, folder) in [(OutputLocation::AlongsideInput, inputs.clone()), (OutputLocation::CustomFolder(chosen.clone()), chosen.clone())] {
            let first = assert_success("first OCR run", ocr_pdf(RunEvents::silent(), ocr_text_request(vec![scan.clone()], location.clone())));
            let first_text = std::fs::read(&first[0]).unwrap();
            let second = assert_success("second OCR run", ocr_pdf(RunEvents::silent(), ocr_text_request(vec![scan.clone()], location)));
            assert_eq!(first, vec![folder.join("scan-ocr.txt")]);
            assert_eq!(second, vec![folder.join("scan-ocr-1.txt")], "a second run gets the next number");
            assert_eq!(std::fs::read(&first[0]).unwrap(), first_text, "the first output is not overwritten");
        }
        assert_eq!(names_in(&inputs), ["scan-ocr-1.txt", "scan-ocr.txt", "scan.pdf"]);
        assert_eq!(names_in(&chosen), ["scan-ocr-1.txt", "scan-ocr.txt"]);
        assert_eq!(std::fs::read(&scan).unwrap(), original);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Runs OCR on the offender beside a good one-page scan, and checks that only the
    /// offender fails, with its limit and recorded value, and that neither original changed.
    #[cfg(unix)]
    fn assert_ocr_limits_only(root: &PathBuf, offender: &PathBuf, limit: LimitKind, recorded_value: &str) {
        let good = root.join("good.pdf");
        if !good.exists() { drawn_pdf(&good, 1); }
        let originals = [Original::of(offender), Original::of(&good)];
        let name = offender.file_stem().unwrap().to_string_lossy().into_owned();
        let outcomes = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_text_request(vec![offender.clone(), good], location(root, &name))));
        for original in &originals { original.assert_untouched(); }
        assert_only_offender_limited(&outcomes, offender, limit, recorded_value);
        assert_eq!(std::fs::read_dir(root.join(&name)).unwrap().count(), 1, "{name}: only the good file's text is written");
    }

    #[cfg(unix)]
    #[test]
    fn ocr_holds_each_file_to_its_recorded_limits_alone() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-limits");
        let _env = marked_ocr_helpers(&root);

        let oversized = root.join("oversized.pdf");
        drawn_pdf(&oversized, 1);
        std::fs::OpenOptions::new().write(true).open(&oversized).unwrap().set_len(100 * 1024 * 1024 + 1).unwrap();
        assert_ocr_limits_only(&root, &oversized, LimitKind::FileSize, "100 MiB");

        let long = root.join("long.pdf");
        make_pdf(&long, 5_001);
        assert_ocr_limits_only(&root, &long, LimitKind::PageCount, "5,000");

        let bomb = root.join("bomb.pdf");
        std::fs::write(&bomb, crate::kit::common::limits::fixtures::pdf_with_stream(&crate::kit::common::limits::fixtures::zlib_of_zeros(500_000_001))).unwrap();
        assert_ocr_limits_only(&root, &bomb, LimitKind::Decompression, "500 MB");

        let poster = root.join("poster.pdf");
        drawn_pdf(&poster, 1);
        let mut document = lopdf::Document::load(&poster).unwrap();
        let page = document.get_pages().values().next().copied().unwrap();
        document.get_dictionary_mut(page).unwrap().set("MediaBox", vec![0.into(), 0.into(), 14400.into(), 14400.into()]);
        document.save(&poster).unwrap();
        assert_ocr_limits_only(&root, &poster, LimitKind::Decompression, "40 million pixel");

        let huge = root.join("HUGE.pdf");
        drawn_pdf(&huge, 1);
        assert_ocr_limits_only(&root, &huge, LimitKind::Decompression, "50 MiB");

        let wordy = root.join("WORDY.pdf");
        drawn_pdf(&wordy, 2);
        assert_ocr_limits_only(&root, &wordy, LimitKind::FileSize, "10 MiB");
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn ocr_stops_a_stalled_page_or_file_at_its_deadline_and_finishes_the_others() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-deadlines");
        let _env = marked_ocr_helpers(&root);
        let stalled = root.join("STALL.pdf");
        let good = root.join("good.pdf");
        drawn_pdf(&stalled, 2);
        drawn_pdf(&good, 1);
        let originals = [Original::of(&stalled), Original::of(&good)];
        let run = |name: &str, limits: crate::kit::common::limits::ToolLimits, deadline: crate::kit::common::limits::HelperDeadline| {
            let run = Arc::new(vision::OcrRun { request: ocr_text_request(vec![stalled.clone(), good.clone()], location(&root, name)), packs: no_ocr_packs() });
            BatchRunner::run_with_limits(&RunEvents::silent(), "ocr_pdf", run.request.paths.clone(), &run.request.output_location, Some(limits), &run, move |run, path| {
                vision::ocr_pdf_within(run, path, deadline)
            })
        };
        let recorded = crate::kit::common::limits::recorded("ocr_pdf").unwrap();
        let short = crate::kit::common::limits::HelperDeadline { per_file: std::time::Duration::from_secs(1), per_page: std::time::Duration::from_secs(3) };

        let by_page = run("by-page", recorded, short);
        assert_only_offender_limited(&by_page, &stalled, LimitKind::Time, "4 seconds");
        assert_eq!(by_page[0].error().unwrap().message, "Stopped at page 1 of 2: that page took longer than 4 seconds. Nothing was saved.");

        let generous = crate::kit::common::limits::HelperDeadline { per_file: std::time::Duration::from_secs(60), per_page: std::time::Duration::from_secs(60) };
        let by_file = run("by-file", crate::kit::common::limits::ToolLimits { max_duration: std::time::Duration::from_secs(1), ..recorded }, generous);
        assert_only_offender_limited(&by_file, &stalled, LimitKind::Time, "time limit for one file");
        assert_eq!(by_file[0].error().unwrap().message, "Stopped at page 1 of 2: it reached the time limit for one file. Nothing was saved. Split it into smaller PDFs, then try again.");

        for original in &originals { original.assert_untouched(); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn an_ocr_batch_fails_one_file_skips_one_and_finishes_the_rest_without_touching_originals() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-mixed");
        let _env = marked_ocr_helpers(&root);
        let first = root.join("first.pdf");
        let broken = root.join("broken.pdf");
        let has_text = root.join("has-text.pdf");
        let photo = root.join("photo.png");
        let last = root.join("last.pdf");
        drawn_pdf(&first, 2);
        std::fs::write(&broken, b"%PDF-1.7\nthis is not a PDF body\n").unwrap();
        make_pdf(&has_text, 1);
        write_png(&photo, 32, 9);
        drawn_pdf(&last, 1);
        let paths = vec![first, broken.clone(), has_text.clone(), photo, last];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let outcomes = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_text_request(paths.clone(), location(&root, "out"))));

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths);
        assert_eq!(paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>(), originals, "originals stay byte for byte");
        let failed = outcomes.iter().filter(|outcome| outcome.error().is_some()).map(|outcome| &outcome.input_path).collect::<Vec<_>>();
        let skipped = outcomes.iter().filter(|outcome| matches!(outcome.state, crate::kit::contracts::OutcomeState::Skipped { .. })).map(|outcome| &outcome.input_path).collect::<Vec<_>>();
        assert_eq!(failed, [&broken]);
        assert_eq!(skipped, [&has_text]);
        for index in [0, 3, 4] {
            let output = outcomes[index].output_paths().first().unwrap_or_else(|| panic!("{} must succeed: {:?}", paths[index].display(), outcomes[index]));
            assert!(std::fs::read_to_string(output).unwrap().contains("recognized words"));
        }
        assert_eq!(std::fs::read_dir(root.join("out")).unwrap().count(), 3, "only the three good files are written");
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn ocr_streams_page_progress_for_a_multi_page_file() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-progress");
        let _env = marked_ocr_helpers(&root);
        let single = root.join("single.pdf");
        let long = root.join("long.pdf");
        drawn_pdf(&single, 1);
        drawn_pdf(&long, 3);
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let outcomes = tauri::async_runtime::block_on(ocr_pdf(RunEvents::new(move |event| sink.lock().unwrap().push(event)), ocr_text_request(vec![single, long], location(&root, "out"))));
        let _ = std::fs::remove_dir_all(&root);

        assert!(outcomes.iter().all(|outcome| outcome.error().is_none()), "{outcomes:?}");
        let events = events.lock().unwrap();
        let of = |index: usize| events.iter().filter(|event| match *event {
            FileEvent::Started { index: at } | FileEvent::Pages { index: at, .. } | FileEvent::Finished { index: at, .. } => *at == index,
        }).cloned().collect::<Vec<_>>();
        let long_events = of(1);
        assert_eq!(long_events[0], FileEvent::Started { index: 1 });
        assert_eq!(long_events[1..5], (0..=3).map(|done| FileEvent::Pages { index: 1, done, total: 3 }).collect::<Vec<_>>());
        assert!(matches!(long_events[5], FileEvent::Finished { index: 1, .. }));
        assert_eq!(long_events.len(), 6);
        assert_eq!(of(0)[1..3], [FileEvent::Pages { index: 0, done: 0, total: 1 }, FileEvent::Pages { index: 0, done: 1, total: 1 }]);
    }

    fn searchable_pdf_request(paths: Vec<PathBuf>, output_location: OutputLocation) -> vision::OcrRequest {
        vision::OcrRequest { mode: "searchablePdf".into(), ..ocr_text_request(paths, output_location) }
    }

    /// Gives a PDF the same title in its Info dictionary and in its XMP metadata.
    fn give_title(path: &PathBuf, title: &str) {
        use lopdf::{dictionary, Document, Object, Stream};
        let mut document = Document::load(path).unwrap();
        let xmp = format!(r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title><rdf:Alt><rdf:li xml:lang="x-default">{title}</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta>"#);
        let metadata = document.add_object(Stream::new(dictionary! { "Type" => "Metadata", "Subtype" => "XML" }, xmp.into_bytes()));
        let info = document.add_object(dictionary! { "Title" => Object::string_literal(title) });
        document.trailer.set("Info", info);
        let catalog = document.trailer.get(b"Root").and_then(Object::as_reference).unwrap();
        document.get_dictionary_mut(catalog).unwrap().set("Metadata", metadata);
        document.save(path).unwrap();
    }

    /// Each page's MediaBox and raw content streams, to compare a copy against its original.
    fn page_boxes_and_streams(path: &PathBuf) -> Vec<(Vec<lopdf::Object>, Vec<Vec<u8>>)> {
        let document = lopdf::Document::load(path).unwrap();
        document.get_pages().values().map(|id| {
            let media_box = document.get_dictionary(*id).unwrap().get(b"MediaBox").unwrap().as_array().unwrap().clone();
            let streams = document.get_page_contents(*id).into_iter().map(|stream| document.get_object(stream).unwrap().as_stream().unwrap().content.clone()).collect();
            (media_box, streams)
        }).collect()
    }

    fn extracted_text(root: &PathBuf, pdf: &PathBuf, folder: &str) -> String {
        let text = assert_success("extract text from the searchable PDF", pdf_to_text(RunEvents::silent(), PdfToTextRequest { paths: vec![pdf.clone()], pages: None, output_location: location(root, folder) }));
        std::fs::read_to_string(&text[0]).unwrap()
    }

    #[test]
    fn searchable_pdf_keeps_the_scan_and_title_gives_back_its_words_and_repeats_byte_for_byte() {
        let _guard = crate::kit::lock_process_env();
        let Some(_engine) = real_ocr_engine() else { return };
        let root = sandbox("ocr-searchable");
        let input = root.join("scan.pdf");
        scanned_pdf(&input, &[&["Quarterly invoice summary", "Northwind shipping ledger"], &["Second page closing remarks", "Thank you for reading"]]);
        give_title(&input, "Northwind scans");
        let original = std::fs::read(&input).unwrap();
        let folder = location(&root, "ocr");

        let first = assert_success("searchable first run", ocr_pdf(RunEvents::silent(), searchable_pdf_request(vec![input.clone()], folder.clone())));
        let second = assert_success("searchable second run", ocr_pdf(RunEvents::silent(), searchable_pdf_request(vec![input.clone()], folder)));
        let names = [&first[0], &second[0]].map(|path| path.file_name().unwrap().to_string_lossy().into_owned());
        let (bytes, again) = (std::fs::read(&first[0]).unwrap(), std::fs::read(&second[0]).unwrap());
        let (before, after) = (page_boxes_and_streams(&input), page_boxes_and_streams(&first[0]));
        let text = extracted_text(&root, &first[0], "text");
        let info = crate::kit::pdf::document_info::read(&first[0], None).unwrap();
        let input_after = std::fs::read(&input).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(names, ["scan-ocr.pdf".to_string(), "scan-ocr-1.pdf".to_string()], "a second run gets a numbered name instead of overwriting");
        assert!(bytes == again, "the same scan gives the same PDF bytes");
        assert_eq!(input_after, original, "the scan is never changed");
        assert_eq!(after.len(), before.len(), "same page count");
        for (page, ((old_box, old_streams), (new_box, new_streams))) in before.iter().zip(&after).enumerate() {
            assert_eq!(new_box, old_box, "page {} keeps its MediaBox", page + 1);
            assert!(new_streams.windows(old_streams.len()).any(|window| window == old_streams.as_slice()), "page {} keeps its content streams byte for byte", page + 1);
        }
        let found = ["Quarterly", "invoice", "Northwind", "ledger", "Second", "closing", "Thank", "reading"].map(|word| text.find(word));
        assert!(found.iter().all(Option::is_some), "every word comes back: {text:?}");
        assert!(found.windows(2).all(|pair| pair[0] < pair[1]), "words keep page and reading order: {text:?}");
        assert_eq!(info.info.title.as_deref(), Some("Northwind scans"));
        assert_eq!(info.xmp.and_then(|xmp| xmp.values.title).as_deref(), Some("Northwind scans"));
    }

    /// Fake helpers whose engine prints the same TSV word boxes for every page.
    #[cfg(unix)]
    fn word_box_ocr_helpers(root: &PathBuf, words: &[(&str, u32, u32)]) -> EnvGuard {
        word_box_ocr_helpers_on(root, (1700, 2200), words)
    }

    /// Fake helpers whose engine reports a `width` by `height` pixel page with one line of
    /// `words`, each a text, left and width, 300 pixels down and 80 tall.
    #[cfg(unix)]
    fn word_box_ocr_helpers_on(root: &PathBuf, (width, height): (u32, u32), words: &[(&str, u32, u32)]) -> EnvGuard {
        let [renderer, engine, tessdata] = fake_ocr_helpers(root);
        let mut tsv = format!("level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n1\t1\t0\t0\t0\t0\t0\t0\t{width}\t{height}\t-1\t\n4\t1\t1\t1\t1\t0\t200\t300\t1200\t80\t-1\t\n");
        for (index, (word, left, width)) in words.iter().enumerate() {
            tsv.push_str(&format!("5\t1\t1\t1\t1\t{}\t{left}\t300\t{width}\t80\t95.0\t{word}\n", index + 1));
        }
        std::fs::write(&engine, format!("#!/bin/sh\ncat <<'TSV'\n{tsv}TSV\n")).unwrap();
        EnvGuard::set(&[("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-bundle"))), ("TOOLBOX_PDFTOPPM_PATH", Some(renderer)), ("TOOLBOX_TESSERACT_PATH", Some(engine)), ("TOOLBOX_TESSDATA_PATH", Some(tessdata))])
    }

    #[cfg(unix)]
    #[test]
    fn searchable_pdf_fails_only_the_file_whose_output_fails_its_check_while_a_rotated_page_succeeds() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-searchable-batch");
        let _env = word_box_ocr_helpers(&root, &[("Searchable", 200, 500), ("words", 750, 400)]);
        let [first, broken, rotated, last] = ["first", "broken", "rotated", "last"].map(|name| root.join(format!("{name}.pdf")));
        drawn_pdf(&first, 1);
        drawn_pdf(&broken, 1);
        drawn_pdf(&rotated, 1);
        drawn_pdf(&last, 2);
        let mut turned = lopdf::Document::load(&rotated).unwrap();
        let page = *turned.get_pages().get(&1).unwrap();
        turned.get_dictionary_mut(page).unwrap().set("Rotate", 90);
        turned.save(&rotated).unwrap();
        let paths = vec![first.clone(), broken.clone(), rotated.clone(), last.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        *vision::BREAK_SEARCHABLE_OUTPUT_FOR.lock().unwrap() = Some(broken.clone());
        let outcomes = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), searchable_pdf_request(paths.clone(), location(&root, "ocr"))));
        *vision::BREAK_SEARCHABLE_OUTPUT_FOR.lock().unwrap() = None;
        let mut written = std::fs::read_dir(root.join("ocr")).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        written.sort();
        let texts = [&outcomes[0], &outcomes[2], &outcomes[3]].map(|outcome| outcome.output_paths().first().map(|pdf| extracted_text(&root, pdf, "text").split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default());
        let after = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths);
        for index in [0, 2, 3] { assert!(outcomes[index].error().is_none(), "{} still runs: {:?}", paths[index].display(), outcomes[index].error()); }
        let broken_error = outcomes[1].error().expect("an output that fails its check fails its row");
        assert_eq!(broken_error.kind, ErrorKind::Processing);
        assert!(broken_error.message.starts_with("The searchable PDF didn't pass its check, so it wasn't saved: it could not be reopened"), "{}", broken_error.message);
        assert!(outcomes[1].output_paths().is_empty());
        assert_eq!(written, vec!["first-ocr.pdf".to_string(), "last-ocr.pdf".to_string(), "rotated-ocr.pdf".to_string()], "nothing is left behind for the failed file");
        assert_eq!(texts[0], "Searchable words", "the drawn page gains only the new text layer");
        assert_eq!(texts[1], "Searchable words", "a rotated page gets its text layer too");
        assert_eq!(texts[2].matches("Searchable words").count(), 2, "both pages get a text layer: {:?}", texts[2]);
        assert_eq!(after, originals);
    }

    #[cfg(unix)]
    #[test]
    fn searchable_pdf_round_trips_non_latin_words_through_the_embedded_font() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-searchable-unicode");
        let _env = word_box_ocr_helpers(&root, &[("Привет", 200, 420), ("κόσμε", 700, 380), ("café", 1150, 300)]);
        let input = root.join("letter.pdf");
        drawn_pdf(&input, 1);

        let output = assert_success("searchable non-Latin", ocr_pdf(RunEvents::silent(), searchable_pdf_request(vec![input.clone()], location(&root, "ocr"))));
        let text = extracted_text(&root, &output[0], "text");
        let _ = std::fs::remove_dir_all(&root);

        assert!(text.split_whitespace().collect::<Vec<_>>().join(" ").contains("Привет κόσμε café"), "{text:?}");
    }

    /// A baseline JPEG of `image`, with an EXIF orientation tag after its JFIF header
    /// when `orientation` is given.
    fn jpeg_of(image: &image::DynamicImage, orientation: Option<u16>) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95).encode_image(image).unwrap();
        if let Some(orientation) = orientation {
            let payload = [b"Exif\0\0".as_slice(), &crate::kit::images::metadata::fixtures::exif(Some(orientation), false)].concat();
            let mut segment = vec![0xFF, 0xE1];
            segment.extend(((payload.len() + 2) as u16).to_be_bytes());
            segment.extend(payload);
            let after_jfif = 4 + u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
            bytes.splice(after_jfif..after_jfif, segment);
        }
        bytes
    }

    /// The image a one-page PDF draws, with its decoded samples, and the page's MediaBox.
    fn drawn_image(path: &PathBuf) -> (lopdf::Dictionary, Vec<u8>, Vec<u8>, Vec<f32>) { drawn_image_on(path, 1) }

    /// The /Im0 image a page draws: its dictionary, raw and decoded samples, and the page's MediaBox.
    fn drawn_image_on(path: &PathBuf, number: u32) -> (lopdf::Dictionary, Vec<u8>, Vec<u8>, Vec<f32>) {
        let document = lopdf::Document::load(path).unwrap();
        let page = *document.get_pages().get(&number).unwrap();
        let page = document.get_dictionary(page).unwrap();
        let media_box = page.get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|value| match value {
            lopdf::Object::Integer(value) => *value as f32,
            value => value.as_float().unwrap(),
        }).collect();
        let resources = page.get(b"Resources").and_then(lopdf::Object::as_dict).unwrap();
        let image = resources.get(b"XObject").and_then(lopdf::Object::as_dict).unwrap().get(b"Im0").and_then(lopdf::Object::as_reference).unwrap();
        let stream = document.get_object(image).unwrap().as_stream().unwrap();
        let samples = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
        (stream.dict.clone(), stream.content.clone(), samples, media_box)
    }

    #[test]
    fn ocr_reads_png_and_jpeg_scans_and_keeps_their_pixels_in_the_searchable_pdf() {
        let _guard = crate::kit::lock_process_env();
        let Some(_engine) = real_ocr_engine() else { return };
        let root = sandbox("ocr-images");
        let (raster, _) = scanned_page(&["Quarterly invoice summary", "Northwind shipping ledger"]);
        let png = root.join("page.png");
        let jpeg = root.join("photo.jpg");
        raster.save(&png).unwrap();
        std::fs::write(&jpeg, jpeg_of(&image::DynamicImage::ImageLuma8(raster.clone()), None)).unwrap();
        let paths = vec![png.clone(), jpeg.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let texts = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_text_request(paths.clone(), location(&root, "text"))));
        let pdfs = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), searchable_pdf_request(paths.clone(), location(&root, "pdf"))));
        let names = texts.iter().chain(&pdfs).map(|outcome| outcome.output_paths().first().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).unwrap_or_default()).collect::<Vec<_>>();
        let words = texts.iter().map(|outcome| std::fs::read_to_string(&outcome.output_paths()[0]).unwrap()).collect::<Vec<_>>();
        let (png_page, jpeg_page) = (drawn_image(&pdfs[0].output_paths()[0]), drawn_image(&pdfs[1].output_paths()[0]));
        let searchable = pdfs.iter().map(|outcome| extracted_text(&root, &outcome.output_paths()[0], "extracted")).collect::<Vec<_>>();
        let after = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let _ = std::fs::remove_dir_all(&root);

        for outcome in texts.iter().chain(&pdfs) { assert!(outcome.error().is_none(), "{}: {:?}", outcome.input_path.display(), outcome.error()); }
        assert_eq!(names, ["page-ocr.txt", "photo-ocr.txt", "page-ocr.pdf", "photo-ocr.pdf"]);
        assert_eq!(texts[0].detail, "1 page recognized. English.");
        for text in words.iter().chain(&searchable) {
            for word in ["Quarterly", "invoice", "Northwind", "ledger"] { assert!(text.contains(word), "{word} is missing from {text:?}"); }
        }
        let (dictionary, _, samples, media_box) = png_page;
        assert_eq!(dictionary.get(b"Filter").unwrap().as_name().unwrap(), b"FlateDecode", "a PNG is stored losslessly");
        assert_eq!(dictionary.get(b"ColorSpace").unwrap().as_name().unwrap(), b"DeviceGray");
        assert!(samples == raster.as_raw().as_slice(), "the PNG's pixels are kept exactly");
        assert_eq!(media_box, [0., 0., 1275., 1650.]);
        let (dictionary, content, _, media_box) = jpeg_page;
        assert_eq!(dictionary.get(b"Filter").unwrap().as_name().unwrap(), b"DCTDecode", "a JPEG is embedded as it is");
        assert!(content == originals[1], "the JPEG's bytes are kept exactly");
        assert_eq!(media_box, [0., 0., 1275., 1650.]);
        assert_eq!(after, originals, "originals are never changed");
    }

    #[test]
    fn ocr_reads_a_jpeg_the_way_its_orientation_tag_shows_it() {
        let _guard = crate::kit::lock_process_env();
        let Some(_engine) = real_ocr_engine() else { return };
        let root = sandbox("ocr-image-orientation");
        let (raster, _) = scanned_page(&["Quarterly invoice summary", "Northwind shipping ledger"]);
        // Orientation 6 shows the stored pixels turned a quarter clockwise, so store them turned back.
        let stored = image::DynamicImage::ImageLuma8(image::imageops::rotate270(&raster));
        let input = root.join("sideways.jpg");
        std::fs::write(&input, jpeg_of(&stored, Some(6))).unwrap();
        let original = std::fs::read(&input).unwrap();

        let text = assert_success("ocr sideways text", ocr_pdf(RunEvents::silent(), ocr_text_request(vec![input.clone()], location(&root, "text"))));
        let pdf = assert_success("ocr sideways pdf", ocr_pdf(RunEvents::silent(), searchable_pdf_request(vec![input.clone()], location(&root, "pdf"))));
        let text = std::fs::read_to_string(&text[0]).unwrap();
        let (dictionary, content, _, media_box) = drawn_image(&pdf[0]);
        let searchable = extracted_text(&root, &pdf[0], "extracted");
        let after = std::fs::read(&input).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        for words in [&text, &searchable] {
            for word in ["Quarterly", "invoice", "Northwind", "ledger"] { assert!(words.contains(word), "{word} is missing from {words:?}"); }
        }
        assert!(text.find("Quarterly") < text.find("Northwind"), "lines read top to bottom: {text:?}");
        assert_eq!(dictionary.get(b"Filter").unwrap().as_name().unwrap(), b"DCTDecode");
        assert!(content == original, "the JPEG is embedded as it is, tag included");
        assert_eq!(media_box, [0., 0., 1275., 1650.], "the page is the upright size");
        assert_eq!(after, original);
    }

    #[cfg(unix)]
    #[test]
    fn ocr_holds_each_image_to_the_pixel_and_decompression_limits_on_its_own() {
        use crate::kit::common::limits::fixtures::png_header;
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-image-limits");
        let [_, engine, tessdata] = fake_ocr_helpers(&root);
        // An image needs no page renderer, so a missing one fails nothing here.
        let _env = EnvGuard::set(&[("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-bundle"))), ("TOOLBOX_PDFTOPPM_PATH", Some(root.join("missing"))), ("TOOLBOX_TESSERACT_PATH", Some(engine)), ("TOOLBOX_TESSDATA_PATH", Some(tessdata))]);
        let [first, many_pixels, huge, last] = ["first", "many-pixels", "huge", "last"].map(|name| root.join(format!("{name}.png")));
        write_png(&first, 32, 10);
        std::fs::write(&many_pixels, png_header(8000, 6000)).unwrap();
        std::fs::write(&huge, png_header(20000, 20000)).unwrap();
        write_png(&last, 32, 20);
        let paths = vec![first, many_pixels, huge, last];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let outcomes = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_text_request(paths.clone(), location(&root, "ocr"))));
        let mut written = std::fs::read_dir(root.join("ocr")).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        written.sort();
        let after = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths);
        for index in [0, 3] { assert!(outcomes[index].error().is_none(), "{} still runs: {:?}", paths[index].display(), outcomes[index].error()); }
        let pixels = outcomes[1].error().expect("an image over the pixel limit fails");
        assert_eq!(pixels.kind, ErrorKind::LimitExceeded { limit: LimitKind::Decompression });
        assert_eq!(pixels.message, "This image has more than 40 million pixels, which is over the OCR limit. Use a smaller image and try again.");
        let decompressed = outcomes[2].error().expect("an image over the decompression limit fails");
        assert_eq!(decompressed.kind, ErrorKind::LimitExceeded { limit: LimitKind::Decompression });
        assert!(outcomes[1].output_paths().is_empty() && outcomes[2].output_paths().is_empty());
        assert_eq!(written, ["first-ocr.txt", "last-ocr.txt"]);
        assert_eq!(after, originals);
    }

    #[cfg(unix)]
    #[test]
    fn ocr_gives_one_output_per_file_in_a_batch_of_pdfs_and_every_image_type() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-image-batch");
        let scan = root.join("scan.pdf");
        let png = root.join("page.png");
        let jpeg = root.join("photo.jpg");
        let webp = root.join("screen.webp");
        let heic = root.join("phone.heic");
        drawn_pdf(&scan, 1);
        write_png(&png, 64, 30);
        let colour = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([x as u8 * 4, y as u8 * 5, 90])));
        std::fs::write(&jpeg, jpeg_of(&colour, None)).unwrap();
        colour.save(&webp).unwrap();
        std::fs::write(&heic, heif::encode(&colour).unwrap()).unwrap();
        let paths = vec![scan, png, jpeg.clone(), webp, heic];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let env = word_box_ocr_helpers(&root, &[("Searchable", 200, 500), ("words", 750, 400)]);

        let pdfs = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), searchable_pdf_request(paths.clone(), location(&root, "pdf"))));
        drop(env);
        let [renderer, engine, tessdata] = fake_ocr_helpers(&root);
        let env = EnvGuard::set(&[("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-bundle"))), ("TOOLBOX_PDFTOPPM_PATH", Some(renderer)), ("TOOLBOX_TESSERACT_PATH", Some(engine)), ("TOOLBOX_TESSDATA_PATH", Some(tessdata))]);
        let texts = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_text_request(paths.clone(), location(&root, "text"))));
        drop(env);
        let pdf_names = pdfs.iter().map(|outcome| outcome.output_paths().iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>()).collect::<Vec<_>>();
        let text_contents = texts.iter().map(|outcome| outcome.output_paths().iter().map(|path| std::fs::read_to_string(path).unwrap()).collect::<Vec<_>>()).collect::<Vec<_>>();
        let searchable = pdfs.iter().map(|outcome| outcome.output_paths().first().map(|pdf| extracted_text(&root, pdf, "extracted").split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()).collect::<Vec<_>>();
        let images = pdfs[1..].iter().map(|outcome| drawn_image(&outcome.output_paths()[0]).0).collect::<Vec<_>>();
        let jpeg_content = drawn_image(&pdfs[2].output_paths()[0]).1;
        let after = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let _ = std::fs::remove_dir_all(&root);

        for outcome in pdfs.iter().chain(&texts) { assert!(outcome.error().is_none(), "{}: {:?}", outcome.input_path.display(), outcome.error()); }
        assert_eq!(pdf_names, [["scan-ocr.pdf"], ["page-ocr.pdf"], ["photo-ocr.pdf"], ["screen-ocr.pdf"], ["phone-ocr.pdf"]], "one output per input");
        assert_eq!(text_contents, vec![vec!["recognized words\n".to_string()]; 5]);
        assert!(searchable.iter().all(|text| text == "Searchable words"), "{searchable:?}");
        assert!(images[0].get(b"SMask").is_ok(), "the PNG's transparency is kept as a soft mask");
        assert_eq!(images[1].get(b"Filter").unwrap().as_name().unwrap(), b"DCTDecode");
        assert!(jpeg_content == originals[2], "the JPEG is embedded as it is");
        for image in [&images[2], &images[3]] {
            assert_eq!(image.get(b"Filter").unwrap().as_name().unwrap(), b"FlateDecode", "WebP and HEIC are stored losslessly");
            assert_eq!(image.get(b"ColorSpace").unwrap().as_name().unwrap(), b"DeviceRGB");
        }
        assert_eq!(after, originals, "originals are never changed");
    }

    #[cfg(unix)]
    #[test]
    fn ocr_notes_a_high_bit_depth_image_saved_as_8_bit_and_skips_an_image_a_range_leaves_empty() {
        use crate::kit::contracts::WarningKind;
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-image-precision");
        let _env = word_box_ocr_helpers(&root, &[("Searchable", 200, 500), ("words", 750, 400)]);
        let deep = root.join("deep.png");
        let plain = root.join("plain.png");
        image::DynamicImage::ImageRgb16(image::ImageBuffer::from_fn(64, 48, |x, y| image::Rgb([x as u16 * 1000, y as u16 * 1300, 40000]))).save(&deep).unwrap();
        write_png(&plain, 64, 40);
        let paths = vec![deep.clone(), plain.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let outcomes = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), searchable_pdf_request(paths.clone(), location(&root, "pdf"))));
        let ranged = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), vision::OcrRequest { page_range: Some("2-".into()), ..searchable_pdf_request(vec![plain.clone()], location(&root, "ranged")) }));
        let (deep_image, _, samples, _) = drawn_image(&outcomes[0].output_paths()[0]);
        let ranged_written = std::fs::read_dir(root.join("ranged")).unwrap().count();
        let after = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let _ = std::fs::remove_dir_all(&root);

        for outcome in &outcomes { assert!(outcome.error().is_none(), "{}: {:?}", outcome.input_path.display(), outcome.error()); }
        assert_eq!(outcomes[0].warnings.len(), 1, "{:?}", outcomes[0].warnings);
        assert_eq!(outcomes[0].warnings[0].kind, WarningKind::PrecisionReduced);
        assert_eq!(outcomes[0].warnings[0].message, "Saved as 8-bit colour, because the image is 16-bit.");
        assert_eq!(outcomes[0].detail, "1 page recognized. English.");
        assert!(outcomes[1].warnings.is_empty(), "an 8-bit image needs no note");
        assert_eq!(deep_image.get(b"BitsPerComponent").unwrap().as_i64().unwrap(), 8);
        assert_eq!(samples.len(), 64 * 48 * 3);
        let OutcomeState::Skipped { reason } = &ranged[0].state else { panic!("a range past its one page skips the image: {:?}", ranged[0]) };
        assert_eq!(reason.message, "No pages left in 2-. It has 1 page, so nothing was made.");
        assert_eq!(ranged_written, 0);
        assert_eq!(after, originals);
    }

    type RawTiffPage = (u32, u32, Vec<(u16, Vec<u32>)>, Vec<u8>);

    /// A little-endian TIFF with one strip per page, written tag by tag, so a test can
    /// make compressions and headers the TIFF encoder doesn't write.
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

    /// An uncompressed 8-bit gray TIFF page.
    fn gray_tiff_page(width: u32, height: u32, pixels: Vec<u8>) -> RawTiffPage {
        (width, height, vec![(258, vec![8]), (259, vec![1]), (262, vec![1]), (277, vec![1])], pixels)
    }

    /// A Group 4 fax TIFF of `raster`, inked where a pixel is darker than mid gray.
    /// Returns the file's bytes and the black and white pixels it holds.
    fn group_four_tiff(raster: &image::GrayImage) -> (Vec<u8>, Vec<u8>) {
        use fax::{encoder::Encoder, Color, VecWriter};
        let (width, height) = raster.dimensions();
        let mut encoder = Encoder::new(VecWriter::new());
        for y in 0..height {
            encoder.encode_line((0..width).map(|x| if raster.get_pixel(x, y)[0] < 128 { Color::Black } else { Color::White }), width as u16).unwrap();
        }
        let bilevel = raster.pixels().map(|pixel| if pixel[0] < 128 { 0 } else { 255 }).collect();
        (fax::tiff::wrap(&encoder.finish().unwrap().finish(), width, height), bilevel)
    }

    /// A one-page TIFF holding a JPEG of `image` as its one strip (compression 7, YCbCr).
    fn jpeg_tiff(image: &image::RgbImage) -> Vec<u8> {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95).encode_image(image).unwrap();
        raw_tiff(&[(image.width(), image.height(), vec![(258, vec![8, 8, 8]), (259, vec![7]), (262, vec![6]), (277, vec![3]), (530, vec![1, 1])], jpeg)])
    }

    #[test]
    fn ocr_reads_a_multi_page_tiff_frame_by_frame_into_text_and_one_searchable_page_per_frame() {
        let _guard = crate::kit::lock_process_env();
        let Some(_engine) = real_ocr_engine() else { return };
        let root = sandbox("ocr-tiff-pages");
        let frames: [&[&str]; 3] = [&["Quarterly invoice summary"], &["Northwind shipping ledger"], &["Closing remarks attached"]];
        let rasters = frames.map(|lines| scanned_page(lines).0);
        let colour = image::DynamicImage::ImageLuma8(rasters[1].clone()).to_rgb8();
        let input = root.join("flatbed-scan.tiff");
        write_tiff(&input, &[image::DynamicImage::ImageLuma8(rasters[0].clone()), image::DynamicImage::ImageRgb8(colour.clone()), image::DynamicImage::ImageLuma8(rasters[2].clone())]);
        let original = std::fs::read(&input).unwrap();

        let text = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_text_request(vec![input.clone()], location(&root, "text"))));
        let pdf = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), searchable_pdf_request(vec![input.clone()], location(&root, "pdf"))));
        for outcome in text.iter().chain(&pdf) { assert!(outcome.error().is_none(), "{:?}", outcome.error()); }
        let words = std::fs::read_to_string(&text[0].output_paths()[0]).unwrap();
        let output = pdf[0].output_paths()[0].clone();
        let page_count = lopdf::Document::load(&output).unwrap().get_pages().len();
        let pages = (1..=3).map(|number| drawn_image_on(&output, number)).collect::<Vec<_>>();
        let searchable = extracted_text(&root, &output, "extracted");
        let after = std::fs::read(&input).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(text[0].output_paths()[0].file_name().unwrap(), "flatbed-scan-ocr.txt");
        assert_eq!(output.file_name().unwrap(), "flatbed-scan-ocr.pdf");
        assert_eq!(text[0].detail, "3 TIFF pages recognized.");
        assert_eq!(pdf[0].detail, "3 TIFF pages recognized, saved as one 3-page PDF.");
        let parts = words.split('\u{c}').collect::<Vec<_>>();
        assert_eq!(parts.len(), 3, "a form feed separates each frame: {words:?}");
        let expected = [["Quarterly", "invoice"], ["Northwind", "ledger"], ["Closing", "remarks"]];
        for (index, part) in parts.iter().enumerate() {
            for (other, frame_words) in expected.iter().enumerate() {
                for word in frame_words { assert_eq!(part.contains(word), index == other, "frame {} and {word}: {part:?}", index + 1); }
            }
        }
        assert_eq!(page_count, 3, "one PDF page per frame");
        for (index, (dictionary, _, samples, media_box)) in pages.iter().enumerate() {
            assert_eq!(dictionary.get(b"Filter").unwrap().as_name().unwrap(), b"FlateDecode", "frame {} is stored losslessly", index + 1);
            assert_eq!(media_box, &[0., 0., 1275., 1650.]);
            let frame: &[u8] = if index == 1 { colour.as_raw() } else { rasters[index].as_raw() };
            assert!(samples == frame, "frame {}'s pixels are kept exactly", index + 1);
        }
        assert_eq!(pages[1].0.get(b"ColorSpace").unwrap().as_name().unwrap(), b"DeviceRGB");
        let found = ["Quarterly", "Northwind", "Closing"].map(|word| searchable.find(word));
        assert!(found.iter().all(Option::is_some) && found.windows(2).all(|pair| pair[0] < pair[1]), "the searchable PDF keeps frame order: {searchable:?}");
        assert_eq!(after, original, "the TIFF is never changed");
    }

    #[test]
    fn ocr_recognises_group_four_fax_and_jpeg_compressed_tiff_scans() {
        let _guard = crate::kit::lock_process_env();
        let Some(_engine) = real_ocr_engine() else { return };
        let root = sandbox("ocr-tiff-compressions");
        let (raster, _) = scanned_page(&["Quarterly invoice summary", "Northwind shipping ledger"]);
        let (fax_bytes, bilevel) = group_four_tiff(&raster);
        let fax = root.join("fax.tif");
        let photo = root.join("photo.tif");
        std::fs::write(&fax, fax_bytes).unwrap();
        std::fs::write(&photo, jpeg_tiff(&image::DynamicImage::ImageLuma8(raster.clone()).to_rgb8())).unwrap();
        let paths = vec![fax.clone(), photo.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let texts = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_text_request(paths.clone(), location(&root, "text"))));
        let pdfs = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), searchable_pdf_request(paths.clone(), location(&root, "pdf"))));
        for outcome in texts.iter().chain(&pdfs) { assert!(outcome.error().is_none(), "{}: {:?}", outcome.input_path.display(), outcome.error()); }
        let words = texts.iter().map(|outcome| std::fs::read_to_string(&outcome.output_paths()[0]).unwrap()).collect::<Vec<_>>();
        let searchable = pdfs.iter().map(|outcome| extracted_text(&root, &outcome.output_paths()[0], "extracted")).collect::<Vec<_>>();
        let (fax_image, _, fax_samples, _) = drawn_image(&pdfs[0].output_paths()[0]);
        let after = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(texts.iter().map(|outcome| outcome.detail.as_str()).collect::<Vec<_>>(), ["1 TIFF page recognized.", "1 TIFF page recognized."]);
        for text in words.iter().chain(&searchable) {
            for word in ["Quarterly", "invoice", "Northwind", "ledger"] { assert!(text.contains(word), "{word} is missing from {text:?}"); }
        }
        assert_eq!(fax_image.get(b"ColorSpace").unwrap().as_name().unwrap(), b"DeviceGray");
        assert!(fax_samples == bilevel, "the fax page keeps every black and white pixel");
        assert_eq!(after, originals, "originals are never changed");
    }

    #[cfg(unix)]
    #[test]
    fn ocr_keeps_group_three_fax_tiff_pages_pixel_for_pixel_in_the_searchable_pdf() {
        const WHITE: [&str; 9] = ["00110101", "000111", "0111", "1000", "1011", "1100", "1110", "1111", "10011"];
        const BLACK: [&str; 9] = ["0000110111", "010", "11", "10", "011", "0011", "0010", "00011", "000101"];
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-tiff-g3");
        let _env = word_box_ocr_helpers(&root, &[("Searchable", 200, 500), ("words", 750, 400)]);
        let rows: [&[usize]; 5] = [&[8], &[2, 3, 3], &[0, 8], &[1, 1, 1, 1, 1, 1, 1, 1], &[5, 2, 1]];
        let mut bits = String::new();
        let mut white = Vec::new();
        for runs in rows {
            bits.push_str("000000000001");
            for (index, run) in runs.iter().enumerate() {
                bits.push_str(if index % 2 == 0 { WHITE[*run] } else { BLACK[*run] });
                white.extend(std::iter::repeat_n(index % 2 == 0, *run));
            }
        }
        while bits.len() % 8 != 0 { bits.push('0'); }
        let data = bits.as_bytes().chunks(8).map(|byte| u8::from_str_radix(std::str::from_utf8(byte).unwrap(), 2).unwrap()).collect::<Vec<_>>();
        let page = |photometric: u32, fill_order: u32, strip: Vec<u8>| -> RawTiffPage {
            (8, rows.len() as u32, vec![(258, vec![1]), (259, vec![3]), (262, vec![photometric]), (266, vec![fill_order]), (277, vec![1]), (292, vec![0])], strip)
        };
        let input = root.join("fax.tiff");
        std::fs::write(&input, raw_tiff(&[page(0, 1, data.clone()), page(1, 2, data.iter().map(|byte| byte.reverse_bits()).collect())])).unwrap();
        let original = std::fs::read(&input).unwrap();

        let output = assert_success("ocr group 3 fax", ocr_pdf(RunEvents::silent(), searchable_pdf_request(vec![input.clone()], location(&root, "pdf"))));
        let pages = [1, 2].map(|number| drawn_image_on(&output[0], number).2);
        let searchable = extracted_text(&root, &output[0], "extracted").split_whitespace().collect::<Vec<_>>().join(" ");
        let after = std::fs::read(&input).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(pages[0], white.iter().map(|white| if *white { 255 } else { 0 }).collect::<Vec<u8>>(), "WhiteIsZero Group 3 runs keep their pixels");
        assert_eq!(pages[1], white.iter().map(|white| if *white { 0 } else { 255 }).collect::<Vec<u8>>(), "BlackIsZero and reversed bit order are honoured");
        assert_eq!(searchable, "Searchable words Searchable words");
        assert_eq!(after, original);
    }

    #[cfg(unix)]
    #[test]
    fn ocr_applies_the_page_range_to_tiff_frames_in_order_and_skips_a_tiff_it_leaves_empty() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-tiff-range");
        let input = root.join("flatbed-scan.tiff");
        // Each frame is a different width, so the PDF shows which frames it holds.
        std::fs::write(&input, raw_tiff(&[10, 20, 30].map(|width| gray_tiff_page(width, 4, vec![200; width as usize * 4])))).unwrap();
        let original = std::fs::read(&input).unwrap();
        let ranged = |range: &str, request: vision::OcrRequest| tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), vision::OcrRequest { page_range: Some(range.into()), ..request }));

        let env = word_box_ocr_helpers(&root, &[("Searchable", 200, 500), ("words", 750, 400)]);
        let pdf = ranged("2-", searchable_pdf_request(vec![input.clone()], location(&root, "pdf")));
        drop(env);
        let [renderer, engine, tessdata] = fake_ocr_helpers(&root);
        let _env = EnvGuard::set(&[("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-bundle"))), ("TOOLBOX_PDFTOPPM_PATH", Some(renderer)), ("TOOLBOX_TESSERACT_PATH", Some(engine)), ("TOOLBOX_TESSDATA_PATH", Some(tessdata))]);
        let text = ranged("3, 2", ocr_text_request(vec![input.clone()], location(&root, "text")));
        let clamped = ranged("1, 9", ocr_text_request(vec![input.clone()], location(&root, "clamped")));
        let skipped = ranged("4-", ocr_text_request(vec![input.clone()], location(&root, "skipped")));
        for outcome in pdf.iter().chain(&text).chain(&clamped) { assert!(outcome.error().is_none(), "{:?}", outcome.error()); }
        let pdf_output = pdf[0].output_paths()[0].clone();
        let page_widths = lopdf::Document::load(&pdf_output).unwrap().get_pages().len();
        let widths = (1..=2).map(|number| drawn_image_on(&pdf_output, number).3[2]).collect::<Vec<_>>();
        let words = std::fs::read_to_string(&text[0].output_paths()[0]).unwrap();
        let clamped_words = std::fs::read_to_string(&clamped[0].output_paths()[0]).unwrap();
        let skipped_written = std::fs::read_dir(root.join("skipped")).map(|entries| entries.count()).unwrap_or(0);
        let after = std::fs::read(&input).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(page_widths, 2);
        assert_eq!(widths, [20., 30.], "frames 2 and 3, in document order");
        assert_eq!(pdf[0].detail, "TIFF pages 2 and 3 recognized, saved as one 2-page PDF.");
        assert_eq!(words, "recognized words\n\u{c}recognized words\n");
        assert_eq!(text[0].detail, "TIFF pages 2 and 3 recognized.");
        assert_eq!(clamped_words, "recognized words\n", "a range past the TIFF's end is clamped to its pages");
        assert_eq!(clamped[0].detail, "TIFF page 1 recognized.");
        let OutcomeState::Skipped { reason } = &skipped[0].state else { panic!("a range past every frame skips the TIFF: {:?}", skipped[0]) };
        assert_eq!(reason.message, "No pages left in 4-. It has 3 pages, so nothing was made.");
        assert_eq!(skipped_written, 0);
        assert_eq!(after, original);
    }

    #[cfg(unix)]
    #[test]
    fn ocr_holds_each_tiff_page_and_all_its_pages_together_to_the_pixel_limits_on_their_own() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-tiff-limits");
        let [renderer, engine, tessdata] = fake_ocr_helpers(&root);
        let _env = EnvGuard::set(&[("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-bundle"))), ("TOOLBOX_PDFTOPPM_PATH", Some(renderer)), ("TOOLBOX_TESSERACT_PATH", Some(engine)), ("TOOLBOX_TESSDATA_PATH", Some(tessdata))]);
        let [first, big_page, many_pages, last] = ["first.tiff", "big-page.tiff", "many-pages.tif", "last.png"].map(|name| root.join(name));
        let small = || gray_tiff_page(8, 4, vec![200; 32]);
        std::fs::write(&first, raw_tiff(&[small(), small()])).unwrap();
        // Only the headers are read before the limits apply, so the strips can be short.
        std::fs::write(&big_page, raw_tiff(&[small(), gray_tiff_page(8000, 6000, vec![0; 16])])).unwrap();
        std::fs::write(&many_pages, raw_tiff(&[(); 4].map(|_| gray_tiff_page(6000, 6000, vec![0; 16])))).unwrap();
        write_png(&last, 32, 20);
        let paths = vec![first, big_page, many_pages, last];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let outcomes = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_text_request(paths.clone(), location(&root, "ocr"))));
        let mut written = std::fs::read_dir(root.join("ocr")).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        written.sort();
        let after = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths);
        for index in [0, 3] { assert!(outcomes[index].error().is_none(), "{} still runs: {:?}", paths[index].display(), outcomes[index].error()); }
        let page = outcomes[1].error().expect("a TIFF page over the pixel limit fails its file");
        assert_eq!(page.kind, ErrorKind::LimitExceeded { limit: LimitKind::Decompression });
        assert_eq!(page.message, "TIFF page 2 has more than 40 million pixels, which is over the OCR limit. Use a smaller image and try again.");
        let total = outcomes[2].error().expect("TIFF pages over the total pixel limit fail their file");
        assert_eq!(total.kind, ErrorKind::LimitExceeded { limit: LimitKind::Decompression });
        assert_eq!(total.message, "The TIFF pages in scope have more than 120 million pixels together, which is over the OCR limit. Choose a page range or split the TIFF, then try again.");
        assert_eq!(written, ["first-ocr.txt", "last-ocr.txt"]);
        assert_eq!(after, originals);
    }

    /// Turns each page of a `scanned_pdf` so it is shown with /Rotate `rotation` yet still
    /// reads upright, then crops it to the rectangle `[left, top, width, height]`, in
    /// fractions of the displayed page. Returns the crop's displayed top left in points.
    fn turn_and_crop_scan(path: &PathBuf, rotation: i64, [left, top, width, height]: [f32; 4]) -> (f32, f32) {
        let mut document = lopdf::Document::load(path).unwrap();
        // The media box, and the matrix that draws the scan upright once the page is turned.
        let (media, placement) = match rotation {
            0 => ([0., 0., 612., 792.], "612 0 0 792 0 0"),
            90 => ([0., 0., 792., 612.], "0 612 -792 0 792 0"),
            180 => ([0., 0., 612., 792.], "-612 0 0 -792 612 792"),
            270 => ([0., 0., 792., 612.], "0 -612 792 0 0 612"),
            rotation => panic!("no fixture for a rotation of {rotation}"),
        };
        let crop = crate::kit::pdf::editor::crop_box_within(media, rotation, 1., 0, &CropSpec::Rectangle { left, top, width, height }).unwrap();
        for page in document.get_pages().into_values() {
            let content = document.get_page_contents(page)[0];
            document.get_object_mut(content).unwrap().as_stream_mut().unwrap().set_plain_content(format!("q {placement} cm /Scan Do Q").into_bytes());
            let dictionary = document.get_dictionary_mut(page).unwrap();
            dictionary.set("MediaBox", media.iter().map(|value| lopdf::Object::Real(*value)).collect::<Vec<_>>());
            dictionary.set("CropBox", crop.iter().map(|value| lopdf::Object::Real(*value)).collect::<Vec<_>>());
            dictionary.set("Rotate", rotation);
        }
        document.save(path).unwrap();
        (left * 612., top * 792.)
    }

    /// Page 1's words and where a reader selects them on the page as shown, turned and
    /// cropped: left, top, right and bottom in points from the displayed top left.
    fn displayed_word_boxes(pdf: &PathBuf) -> Vec<(String, [f32; 4])> {
        let reader = crate::kit::resources::resolve_helper(&crate::kit::resources::PDF_TEXT).expect("pdftotext");
        let output = std::process::Command::new(reader).args(["-bbox", "-cropbox", "-f", "1", "-l", "1"]).arg(pdf).arg("-").output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap().split("<word ").skip(1).map(|word| {
            let number = |name: &str| word.split(&format!("{name}=\"")).nth(1).and_then(|rest| rest.split('"').next()).unwrap().parse::<f32>().unwrap();
            let text = word.split('>').nth(1).and_then(|rest| rest.split('<').next()).unwrap().to_string();
            (text, [number("xMin"), number("yMin"), number("xMax"), number("yMax")])
        }).collect()
    }

    #[cfg(unix)]
    #[test]
    fn searchable_pdf_places_each_word_where_it_shows_on_rotated_and_cropped_pages() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-searchable-turned");
        // The crop shows 459 by 594 points; the engine reports a picture twice that size.
        let crop = [0.125, 0.125, 0.75, 0.75];
        let words = [("Northwind", 100, 300), ("ledger", 500, 200)];
        let _env = word_box_ocr_helpers_on(&root, (918, 1188), &words);

        for rotation in [0, 90, 180, 270] {
            let input = root.join(format!("turned-{rotation}.pdf"));
            scanned_pdf(&input, &[&[]]);
            turn_and_crop_scan(&input, rotation, crop);
            let original = std::fs::read(&input).unwrap();
            let output = assert_success("searchable turned page", ocr_pdf(RunEvents::silent(), searchable_pdf_request(vec![input.clone()], location(&root, "ocr"))));
            let found = displayed_word_boxes(&output[0]);

            assert_eq!(found.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>(), ["Northwind", "ledger"], "rotation {rotation}");
            // Each word box is 300 to 380 pixels down the picture, so 150 to 190 points.
            for ((_, left, width), (text, actual)) in words.iter().zip(&found) {
                let drawn = [*left as f32 / 2., 150., (left + width) as f32 / 2., 190.];
                assert!(actual.iter().zip(drawn).all(|(actual, drawn)| (actual - drawn).abs() < 1.5), "rotation {rotation}: {text} is selected at {actual:?} but drawn at {drawn:?}");
            }
            assert_eq!(std::fs::read(&input).unwrap(), original, "rotation {rotation}: the original is never changed");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn searchable_pdf_puts_recognized_words_over_their_ink_on_rotated_and_cropped_scans() {
        let _guard = crate::kit::lock_process_env();
        let Some(_engine) = real_ocr_engine() else { return };
        let root = sandbox("ocr-searchable-turned-scan");
        let crop = [0.05, 0.05, 0.9, 0.5];

        for rotation in [0, 90, 180, 270] {
            let input = root.join(format!("scan-{rotation}.pdf"));
            let inked = scanned_pdf(&input, &[&["Quarterly invoice summary", "Northwind shipping ledger"]]).remove(0);
            let (offset_x, offset_y) = turn_and_crop_scan(&input, rotation, crop);
            let original = std::fs::read(&input).unwrap();
            let output = assert_success("searchable turned scan", ocr_pdf(RunEvents::silent(), searchable_pdf_request(vec![input.clone()], location(&root, "ocr"))));
            let found = displayed_word_boxes(&output[0]);

            assert_eq!(std::fs::read(&input).unwrap(), original, "rotation {rotation}: the scan is never changed");
            for (word, [left, top, right, bottom]) in &inked {
                let ink = [left - offset_x - 1., top - offset_y - 1., right - offset_x + 1., bottom - offset_y + 1.];
                let (_, actual) = found.iter().find(|(text, _)| text == word).unwrap_or_else(|| panic!("rotation {rotation}: {word} is missing from {found:?}"));
                let (x, y) = ((actual[0] + actual[2]) / 2., (actual[1] + actual[3]) / 2.);
                assert!(ink[0] < x && x < ink[2] && ink[1] < y && y < ink[3], "rotation {rotation}: {word} is selected at {actual:?} but inked at {ink:?}");
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn ocr_of_a_signed_pdf_removes_the_signature_and_warns_only_in_searchable_mode() {
        use crate::kit::contracts::{OutcomeWarning, WarningKind};
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-signed");
        let signed = root.join("signed-contract.pdf");
        drawn_pdf(&signed, 1);
        let mut document = lopdf::Document::load(&signed).unwrap();
        crate::kit::pdf::tests::sign(&mut document);
        document.save(&signed).unwrap();
        let original = std::fs::read(&signed).unwrap();

        let searchable = {
            let _env = word_box_ocr_helpers(&root, &[("Signed", 200, 400), ("contract", 700, 500)]);
            tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), searchable_pdf_request(vec![signed.clone()], location(&root, "searchable")))).remove(0)
        };
        let text = {
            let [renderer, engine, tessdata] = fake_ocr_helpers(&root);
            let _env = EnvGuard::set(&[("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-bundle"))), ("TOOLBOX_PDFTOPPM_PATH", Some(renderer)), ("TOOLBOX_TESSERACT_PATH", Some(engine)), ("TOOLBOX_TESSDATA_PATH", Some(tessdata))]);
            tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_text_request(vec![signed.clone()], location(&root, "text")))).remove(0)
        };
        let copy = searchable.output_paths().first().map(|path| lopdf::Document::load(path).unwrap());
        let words = searchable.output_paths().first().map(|path| extracted_text(&root, path, "extracted").split_whitespace().collect::<Vec<_>>().join(" "));
        let saved_text = text.output_paths().first().map(|path| std::fs::read_to_string(path).unwrap());
        let after = std::fs::read(&signed).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert!(searchable.error().is_none(), "{:?}", searchable.error());
        assert_eq!(searchable.warnings, [OutcomeWarning { kind: WarningKind::SignatureRemoved, message: crate::kit::pdf::SIGNATURE_REMOVED.into() }]);
        assert!(copy.is_some_and(|copy| !crate::kit::pdf::has_signed_signature_field(&copy)), "the searchable copy is not signed");
        assert_eq!(words.as_deref(), Some("Signed contract"));
        assert!(text.error().is_none(), "{:?}", text.error());
        assert!(text.warnings.is_empty(), "a text file carries no warning: {:?}", text.warnings);
        assert_eq!(saved_text.as_deref(), Some("recognized words\n"));
        assert_eq!(after, original, "the signed original is never changed");
        assert!(crate::kit::pdf::has_signed_signature_field(&lopdf::Document::load_mem(&after).unwrap()));
    }

    /// Four pages, one of each kind and a second scan: 1 and 4 have no text, 2 has
    /// visible text and 3 is a scan with an invisible layer from an earlier OCR run.
    fn mixed_text_pdf(path: &PathBuf) {
        pdf_of_pages(path, &[
            drawn_page(1),
            "BT /F1 24 Tf 72 720 Td (Toolbox page 2) Tj ET".to_string(),
            format!("{} BT 3 Tr /F1 24 Tf 72 720 Td (Earlier layer 3) Tj ET", drawn_page(3)),
            drawn_page(4),
        ]);
    }

    fn ocr_request_with(mode: &str, force: bool, paths: Vec<PathBuf>, output_location: OutputLocation) -> vision::OcrRequest {
        vision::OcrRequest { mode: mode.into(), force, ..ocr_text_request(paths, output_location) }
    }

    fn page_details(outcome: &JobOutcome) -> Vec<(String, String)> {
        outcome.page_details.iter().map(|detail| (detail.label.clone(), detail.value.clone())).collect()
    }

    fn rows(rows: &[(&str, &str)]) -> Vec<(String, String)> {
        rows.iter().map(|(label, value)| (label.to_string(), value.to_string())).collect()
    }

    #[cfg(unix)]
    #[test]
    fn ocr_text_reads_only_pages_without_text_and_adds_the_rest_as_they_are_in_page_order() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-existing-text");
        let [renderer, engine, tessdata] = fake_ocr_helpers(&root);
        let _env = EnvGuard::set(&[("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-bundle"))), ("TOOLBOX_PDFTOPPM_PATH", Some(renderer)), ("TOOLBOX_TESSERACT_PATH", Some(engine)), ("TOOLBOX_TESSDATA_PATH", Some(tessdata))]);
        let input = root.join("mixed.pdf");
        mixed_text_pdf(&input);
        let original = std::fs::read(&input).unwrap();

        let run = |force: bool, folder: &str| {
            let outcomes = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_request_with("text", force, vec![input.clone()], location(&root, folder))));
            let text = outcomes[0].output_paths().first().map(|path| std::fs::read_to_string(path).unwrap()).unwrap_or_default();
            (outcomes.into_iter().next().unwrap(), text)
        };
        let (kept, kept_text) = run(false, "kept");
        let (forced, forced_text) = run(true, "forced");
        let input_after = std::fs::read(&input).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert!(kept.error().is_none(), "{:?}", kept.error());
        assert_eq!(kept.detail, "2 pages recognized, 2 pages of text added as they are");
        assert_eq!(page_details(&kept), rows(&[("Recognized", "Pages 1 and 4"), ("Visible text, added as it is", "Page 2"), ("Earlier OCR text, added as it is", "Page 3"), ("Languages", "English")]));
        assert_eq!(kept_text, "recognized words\n\u{c}Toolbox page 2\n\u{c}Earlier layer 3\n\u{c}recognized words\n", "every page's words once, in page order");

        assert!(forced.error().is_none(), "{:?}", forced.error());
        assert_eq!(forced.detail, "3 pages recognized, 1 page of text added as it is");
        assert_eq!(page_details(&forced), rows(&[("Recognized", "Pages 1 and 4"), ("Earlier OCR text replaced", "Page 3"), ("Visible text, added as it is", "Page 2"), ("Languages", "English")]));
        assert_eq!(forced_text, "recognized words\n\u{c}Toolbox page 2\n\u{c}recognized words\n\u{c}recognized words\n", "a replaced page gives only its new text");
        assert_eq!(input_after, original);
    }

    #[cfg(unix)]
    #[test]
    fn searchable_pdf_skips_pages_with_text_and_force_replaces_only_an_earlier_invisible_layer() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-searchable-existing-text");
        let _env = word_box_ocr_helpers(&root, &[("Searchable", 200, 500), ("words", 750, 400)]);
        let input = root.join("mixed.pdf");
        mixed_text_pdf(&input);
        let original = std::fs::read(&input).unwrap();
        let before = page_boxes_and_streams(&input);

        let run = |force: bool, folder: &str| {
            let outcome = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_request_with("searchablePdf", force, vec![input.clone()], location(&root, folder)))).remove(0);
            let output = outcome.output_paths().first().cloned().unwrap_or_else(|| panic!("{folder} made no PDF: {:?}", outcome.error()));
            let text = extracted_text(&root, &output, &format!("{folder}-text")).split_whitespace().collect::<Vec<_>>().join(" ");
            let document = lopdf::Document::load(&output).unwrap();
            let page_three = document.get_page_contents(document.get_pages()[&3]).into_iter()
                .map(|id| { let stream = document.get_object(id).unwrap().as_stream().unwrap(); String::from_utf8_lossy(&stream.decompressed_content().unwrap_or_else(|_| stream.content.clone())).into_owned() })
                .collect::<Vec<_>>().join("\n");
            (outcome, page_boxes_and_streams(&output), text, page_three)
        };
        let (kept, kept_pages, kept_text, _) = run(false, "kept");
        let (forced, forced_pages, forced_text, page_three) = run(true, "forced");
        let input_after = std::fs::read(&input).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(kept.detail, "2 of 4 pages recognized, 2 skipped for their text.");
        assert_eq!(page_details(&kept), rows(&[("Recognized", "Pages 1 and 4"), ("Skipped, visible text", "Page 2"), ("Skipped, earlier OCR text", "Page 3. Turn on Force to replace it."), ("Languages", "English")]));
        assert_eq!(kept_pages[1..3], before[1..3], "pages that keep their text are not changed");
        assert_eq!(kept_text.matches("Searchable words").count(), 2, "{kept_text:?}");
        assert_eq!(kept_text.matches("Earlier layer 3").count(), 1, "{kept_text:?}");
        assert_eq!(kept_text.matches("Toolbox page 2").count(), 1, "{kept_text:?}");

        assert_eq!(forced.detail, "3 of 4 pages recognized, 1 skipped for its text.");
        assert_eq!(page_details(&forced), rows(&[("Recognized", "Pages 1 and 4"), ("Earlier OCR text replaced", "Page 3"), ("Skipped, visible text", "Page 2"), ("Languages", "English")]));
        assert_eq!(forced_pages[1], before[1], "the visible page's content streams are unchanged");
        assert!(!page_three.contains("Earlier"), "the earlier layer is removed: {page_three:?}");
        assert!(page_three.contains("72 720 120 10 re") && page_three.contains("/ToolboxOcr"), "the scan is kept and the new layer added: {page_three:?}");
        assert_eq!(forced_text.matches("Searchable words").count(), 3, "each word is found once: {forced_text:?}");
        assert!(!forced_text.contains("Earlier"), "{forced_text:?}");
        assert_eq!(forced_text.matches("Toolbox page 2").count(), 1, "{forced_text:?}");
        assert_eq!(input_after, original);
    }

    #[cfg(unix)]
    #[test]
    fn ocr_skips_a_file_whose_pages_all_keep_their_text_and_force_redoes_only_an_earlier_layer() {
        use crate::kit::contracts::SkipCode;
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-whole-file-text");
        let _env = word_box_ocr_helpers(&root, &[("Searchable", 200, 500), ("words", 750, 400)]);
        let invisible = |number| format!("{} BT 3 Tr /F1 24 Tf 72 720 Td (Earlier layer {number}) Tj ET", drawn_page(number));
        let [all_text, all_visible, earlier] = ["all-text", "all-visible", "earlier"].map(|name| root.join(format!("{name}.pdf")));
        pdf_of_pages(&all_text, &["BT /F1 24 Tf 72 720 Td (Toolbox page 1) Tj ET".to_string(), invisible(2)]);
        make_pdf(&all_visible, 2);
        pdf_of_pages(&earlier, &[invisible(1)]);
        let paths = [all_text.clone(), all_visible.clone(), earlier.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let run = |mode: &str, force: bool, input: &PathBuf, folder: &str| tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_request_with(mode, force, vec![input.clone()], location(&root, folder)))).remove(0);
        let skipped = [
            (run("text", false, &all_text, "all-text-file"), "Every page already has text: page 1 visible, page 2 from an earlier OCR run. Nothing was made. Turn on Force to replace the earlier OCR text."),
            (run("searchablePdf", false, &all_text, "all-text-pdf"), "Every page already has text: page 1 visible, page 2 from an earlier OCR run. Nothing was made. Turn on Force to replace the earlier OCR text."),
            (run("text", true, &all_visible, "all-visible-file"), "Every page already has text: pages 1 and 2 visible. Nothing was made."),
            (run("searchablePdf", true, &all_visible, "all-visible-pdf"), "Every page already has text: pages 1 and 2 visible. Nothing was made."),
        ];
        let written = ["all-text-file", "all-text-pdf", "all-visible-file", "all-visible-pdf"].map(|folder| std::fs::read_dir(root.join(folder)).unwrap().count());
        let redone = run("searchablePdf", true, &earlier, "earlier");
        let redone_text = redone.output_paths().first().map(|pdf| extracted_text(&root, pdf, "earlier-text").split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
        let without_force = run("searchablePdf", false, &earlier, "earlier-without-force");
        let after = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let _ = std::fs::remove_dir_all(&root);

        for (outcome, message) in &skipped {
            let OutcomeState::Skipped { reason } = &outcome.state else { panic!("expected Skipped: {outcome:?}") };
            assert_eq!(reason.code, SkipCode::TextAlreadyPresent);
            assert_eq!(reason.message, *message);
            assert!(outcome.output_paths().is_empty());
        }
        assert_eq!(written, [0; 4], "a skipped file writes nothing");
        assert!(redone.error().is_none(), "{:?}", redone.error());
        assert_eq!(redone.detail, "1 page recognized, 1 with earlier OCR text replaced.");
        assert_eq!(page_details(&redone), rows(&[("Earlier OCR text replaced", "Page 1"), ("Languages", "English")]));
        assert_eq!(redone_text, "Searchable words", "only the new layer is left");
        assert!(matches!(&without_force.state, OutcomeState::Skipped { reason } if reason.message == "Every page already has text: page 1 from an earlier OCR run. Nothing was made. Turn on Force to replace the earlier OCR text."), "{without_force:?}");
        assert_eq!(after, originals);
    }

    /// Fake helpers whose engine reads each rendered page back as "page N", so a text
    /// file shows which pages were recognized and in what order.
    #[cfg(unix)]
    fn page_number_ocr_helpers(root: &PathBuf) -> EnvGuard {
        let [renderer, engine, tessdata] = fake_ocr_helpers(root);
        std::fs::write(&engine, "#!/bin/sh\nn=${1##*page-}\nprintf 'page %s\\n' \"${n%.png}\"\n").unwrap();
        EnvGuard::set(&[("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-bundle"))), ("TOOLBOX_PDFTOPPM_PATH", Some(renderer)), ("TOOLBOX_TESSERACT_PATH", Some(engine)), ("TOOLBOX_TESSDATA_PATH", Some(tessdata))])
    }

    fn ranged_ocr_request(mode: &str, range: &str, paths: Vec<PathBuf>, output_location: OutputLocation) -> vision::OcrRequest {
        vision::OcrRequest { page_range: Some(range.into()), ..ocr_request_with(mode, false, paths, output_location) }
    }

    fn written(folder: &PathBuf) -> Vec<String> {
        let mut names = std::fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        names.sort();
        names
    }

    #[cfg(unix)]
    #[test]
    fn ocr_range_runs_in_page_order_copies_pages_outside_it_and_leaves_them_out_of_a_text_file() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-range-scope");
        let input = root.join("scan.pdf");
        drawn_pdf(&input, 4);
        let original = std::fs::read(&input).unwrap();
        let before = page_boxes_and_streams(&input);

        let searchable = {
            let _env = word_box_ocr_helpers(&root, &[("Searchable", 200, 500), ("words", 750, 400)]);
            tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ranged_ocr_request("searchablePdf", "4, 1-2", vec![input.clone()], location(&root, "pdf")))).remove(0)
        };
        let pdf = searchable.output_paths().first().cloned().unwrap_or_else(|| panic!("no searchable PDF: {:?}", searchable.state));
        let after = page_boxes_and_streams(&pdf);
        let reopened = lopdf::Document::load(&pdf).unwrap();
        let layered = (1..=4).map(|page| reopened.extract_text(&[page]).unwrap_or_default().contains("Searchable")).collect::<Vec<_>>();
        let text = {
            let _env = page_number_ocr_helpers(&root);
            tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ranged_ocr_request("text", "4, 1-2", vec![input.clone()], location(&root, "text")))).remove(0)
        };
        let text_file = text.output_paths().first().map(|path| std::fs::read_to_string(path).unwrap()).unwrap_or_default();
        let input_after = std::fs::read(&input).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert!(searchable.error().is_none(), "{:?}", searchable.error());
        assert_eq!(searchable.detail, "3 pages recognized. English.");
        assert_eq!(after.len(), 4, "the searchable PDF is still the whole document");
        assert_eq!(after[2], before[2], "page 3 is outside the range and is copied byte for byte");
        assert_eq!(layered, [true, true, false, true], "only pages in the range gain a text layer");
        assert!(text.error().is_none(), "{:?}", text.error());
        assert_eq!(text.detail, "3 pages recognized. English.");
        assert_eq!(text_file, "page 1\n\u{c}page 2\n\u{c}page 4\n", "a range typed out of order runs in page order and page 3 is left out");
        assert_eq!(input_after, original);
    }

    #[cfg(unix)]
    #[test]
    fn ocr_clamps_one_range_to_each_file_and_skips_the_file_with_no_pages_left() {
        use crate::kit::contracts::SkipCode;
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-range-batch");
        let (long, medium, short) = (root.join("long.pdf"), root.join("medium.pdf"), root.join("short.pdf"));
        drawn_pdf(&long, 10); drawn_pdf(&medium, 3); drawn_pdf(&short, 1);
        let paths = vec![long.clone(), medium.clone(), short.clone()];
        let originals = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let texts = {
            let _env = page_number_ocr_helpers(&root);
            tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ranged_ocr_request("text", "9-, 2-4", paths.clone(), location(&root, "text"))))
        };
        let pdfs = {
            let _env = word_box_ocr_helpers(&root, &[("Searchable", 200, 500), ("words", 750, 400)]);
            tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ranged_ocr_request("searchablePdf", "9-, 2-4", paths.clone(), location(&root, "pdf"))))
        };
        let text_of = |outcome: &JobOutcome| outcome.output_paths().first().map(|path| std::fs::read_to_string(path).unwrap()).unwrap_or_default();
        let (long_text, medium_text) = (text_of(&texts[0]), text_of(&texts[1]));
        let layered = |outcome: &JobOutcome| outcome.output_paths().first().map(|pdf| {
            let document = lopdf::Document::load(pdf).unwrap();
            document.get_pages().keys().filter(|page| document.extract_text(&[**page]).unwrap_or_default().contains("Searchable")).copied().collect::<Vec<_>>()
        }).unwrap_or_default();
        let (long_layers, medium_layers) = (layered(&pdfs[0]), layered(&pdfs[1]));
        let written_files = [written(&root.join("text")), written(&root.join("pdf"))];
        let after = paths.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let _ = std::fs::remove_dir_all(&root);

        for outcomes in [&texts, &pdfs] {
            assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths);
            for outcome in &outcomes[..2] { assert!(outcome.error().is_none(), "{:?}", outcome.error()); }
            let OutcomeState::Skipped { reason } = &outcomes[2].state else { panic!("expected short.pdf to be skipped: {:?}", outcomes[2]) };
            assert_eq!(reason.code, SkipCode::RangeEmptyAfterClamping);
            assert_eq!(reason.message, "No pages left in 9-, 2-4. It has 1 page, so nothing was made.");
            assert!(outcomes[2].output_paths().is_empty());
        }
        assert_eq!(long_text, "page 2\n\u{c}page 3\n\u{c}page 4\n\u{c}page 9\n\u{c}page 10\n");
        assert_eq!(medium_text, "page 2\n\u{c}page 3\n", "the range is shortened to the pages medium.pdf has");
        assert_eq!(long_layers, vec![2, 3, 4, 9, 10]);
        assert_eq!(medium_layers, vec![2, 3]);
        assert_eq!(written_files, [vec!["long-ocr.txt".to_string(), "medium-ocr.txt".to_string()], vec!["long-ocr.pdf".to_string(), "medium-ocr.pdf".to_string()]], "the skipped file writes nothing");
        assert_eq!(after, originals);
    }

    #[cfg(unix)]
    #[test]
    fn ocr_range_over_only_text_pages_is_skipped_and_one_with_a_scanned_page_succeeds() {
        use crate::kit::contracts::SkipCode;
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-range-mixed");
        let _env = word_box_ocr_helpers(&root, &[("Searchable", 200, 500), ("words", 750, 400)]);
        let input = root.join("mixed.pdf");
        mixed_text_pdf(&input);
        let original = std::fs::read(&input).unwrap();

        let run = |mode: &str, range: &str, folder: &str| tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ranged_ocr_request(mode, range, vec![input.clone()], location(&root, folder)))).remove(0);
        let text_only = [run("searchablePdf", "3, 2", "text-only-pdf"), run("text", "3, 2", "text-only-file")];
        let with_scan = run("searchablePdf", "2-4", "with-scan-pdf");
        let with_scan_text = run("text", "2-4", "with-scan-file");
        let written_files = ["text-only-pdf", "text-only-file", "with-scan-pdf", "with-scan-file"].map(|folder| written(&root.join(folder)).len());
        let input_after = std::fs::read(&input).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        for outcome in &text_only {
            let OutcomeState::Skipped { reason } = &outcome.state else { panic!("expected Skipped: {outcome:?}") };
            assert_eq!(reason.code, SkipCode::TextAlreadyPresent);
            assert_eq!(reason.message, "Every page already has text: page 2 visible, page 3 from an earlier OCR run. Nothing was made. Turn on Force to replace the earlier OCR text.");
            assert!(outcome.output_paths().is_empty());
        }
        assert!(with_scan.error().is_none(), "{:?}", with_scan.error());
        assert_eq!(with_scan.detail, "1 of 3 pages recognized, 2 skipped for their text.");
        assert_eq!(page_details(&with_scan), rows(&[("Recognized", "Page 4"), ("Skipped, visible text", "Page 2"), ("Skipped, earlier OCR text", "Page 3. Turn on Force to replace it."), ("Languages", "English")]));
        assert!(with_scan_text.error().is_none(), "{:?}", with_scan_text.error());
        assert_eq!(page_details(&with_scan_text), rows(&[("Recognized", "Page 4"), ("Visible text, added as it is", "Page 2"), ("Earlier OCR text, added as it is", "Page 3"), ("Outside 2-4, left out", "Page 1"), ("Languages", "English")]));
        assert_eq!(written_files, [0, 0, 1, 1]);
        assert_eq!(input_after, original);
    }

    fn split_ranges_request(paths: Vec<PathBuf>, ranges: &str, output_location: OutputLocation) -> PageSelectionRequest {
        PageSelectionRequest { paths, pages: vec![], page_ranges: Some(ranges.into()), split_mode: Some("ranges".into()), chunk_size: None, output_location }
    }

    fn images_range_request(paths: Vec<PathBuf>, range: &str, output_location: OutputLocation) -> PdfToImagesRequest {
        PdfToImagesRequest { paths, dpi: 72, format: "png".into(), page_range: Some(range.into()), pages: None, output_location }
    }

    #[test]
    fn split_and_pdf_to_images_reject_malformed_ranges_before_any_file_runs() {
        let root = sandbox("malformed-page-ranges");
        let inputs = vec![root.join("first.pdf"), root.join("second.pdf")];
        std::fs::write(&inputs[0], b"not a pdf yet").unwrap();
        make_pdf(&inputs[1], 3);
        for (range, named) in [("3-1", "\"3-1\""), ("0-2", "\"0-2\""), ("1,,2", "Entry 2"), ("1, two", "\"two\"")] {
            let split = tauri::async_runtime::block_on(split_pdf(RunEvents::silent(), split_ranges_request(inputs[..1].to_vec(), range, location(&root, "split"))));
            let images = tauri::async_runtime::block_on(pdf_to_images(RunEvents::silent(), images_range_request(inputs.clone(), range, location(&root, "images"))));
            for (tool, outcomes, expected) in [("split", split, &inputs[..1]), ("pdf to images", images, &inputs[..])] {
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
        assert_eq!(names, ["document-split-0.pdf", "document-split-1.pdf", "document-split-2.pdf"]);
        let page_counts = parts.iter().map(|path| lopdf::Document::load(path).unwrap().get_pages().len()).collect::<Vec<_>>();
        assert_eq!(page_counts, [3, 1, 2]);

        let images = assert_success("pdf to images ranges", pdf_to_images(RunEvents::silent(), images_range_request(vec![input.clone()], "1-3, 5, 8-", location(&root, "images"))));
        let names = images.iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(names, ["document-images-1.png", "document-images-2.png", "document-images-3.png", "document-images-5.png", "document-images-8.png", "document-images-9.png"]);

        for (tool, outcomes) in [
            ("split", tauri::async_runtime::block_on(split_pdf(RunEvents::silent(), split_ranges_request(vec![input.clone()], "1, 8-12", location(&root, "split-past-end"))))),
            ("pdf to images", tauri::async_runtime::block_on(pdf_to_images(RunEvents::silent(), images_range_request(vec![input.clone()], "1, 10-", location(&root, "images-past-end"))))),
        ] {
            let error = outcomes[0].error().unwrap_or_else(|| panic!("{tool} accepted pages past the end"));
            assert_eq!(error.kind, ErrorKind::Processing, "{tool}: {}", error.message);
            assert!(outcomes[0].output_paths().is_empty(), "{tool}");
        }
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
        check_ocr_outcome(name, outcome, resources_staged, std::env::var_os("TOOLBOX_REQUIRE_OCR_RESOURCES").is_some());
        outcome.output_paths().to_vec()
    }

    /// Release runs set TOOLBOX_REQUIRE_OCR_RESOURCES, so OCR must recognize the fixture there.
    /// A local run without staged resources may get Unavailable and still pass.
    fn check_ocr_outcome(name: &str, outcome: &JobOutcome, resources_staged: bool, release_strict: bool) {
        if let Some(error) = outcome.error() {
            let unavailable = matches!(error.kind, crate::kit::contracts::ErrorKind::Unavailable);
            assert!(!(unavailable && release_strict), "{name}: release checks require OCR, but it was unavailable: {error}");
            assert!(!(unavailable && resources_staged), "{name} resources were staged but OCR was unavailable: {error}");
            assert!(!release_strict, "{name}: release checks require OCR to recognize the fixture, but it failed: {error}");
        } else {
            assert!(!outcome.output_paths().is_empty(), "{name} reported success without an output");
            for output in outcome.output_paths() { assert!(output.is_file(), "{name} reported missing output {}", output.display()); }
            let output = &outcome.output_paths()[0];
            assert!(output.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.ends_with("-ocr.txt")), "{name} must save a -ocr text file: {}", output.display());
            let text = std::fs::read_to_string(output).unwrap_or_default();
            assert!(text.contains("Toolbox page"), "{name} did not extract the fixture text: {text:?}");
            assert!(!text.contains('\r'), "{name} must end lines in LF: {text:?}");
        }
    }

    #[test]
    fn release_strictness_fails_the_ocr_check_on_unavailable_while_local_runs_without_resources_accept_it() {
        let _guard = crate::kit::lock_process_env();
        let root = sandbox("ocr-strict");
        let _env = EnvGuard::set(&[("TOOLBOX_RESOURCE_ROOT", Some(root.join("no-resources"))), ("TOOLBOX_TESSERACT_PATH", Some(root.join("missing-tesseract")))]);
        let scan = root.join("scan.pdf");
        make_pdf(&scan, 1);
        let outcomes = tauri::async_runtime::block_on(ocr_pdf(RunEvents::silent(), ocr_text_request(vec![scan], location(&root, "ocr"))));
        let _ = std::fs::remove_dir_all(&root);
        let outcome = &outcomes[0];
        assert!(outcome.error().is_some_and(|error| matches!(error.kind, ErrorKind::Unavailable)), "{outcome:?}");

        let fails = |staged: bool, strict: bool| std::panic::catch_unwind(|| check_ocr_outcome("ocr", outcome, staged, strict)).is_err();
        assert!(!fails(false, false), "a local run without staged resources accepts Unavailable");
        assert!(fails(false, true), "release strictness fails on Unavailable even when nothing was staged");
        assert!(fails(true, false), "staged resources must make OCR available");
        assert!(fails(true, true));
    }

    /// Recognizes the English fixture with a Release artifact's own OCR resources, as text and
    /// as a searchable PDF, and saves both with their SHA-256 for the cross-target comparison.
    /// Runs only when TOOLBOX_OCR_RELEASE_ROOT names the artifact's resource folder (a Windows
    /// install folder or Toolbox.app/Contents/Resources) and TOOLBOX_OCR_RELEASE_OUT the folder
    /// to save into. It never accepts Unavailable.
    #[test]
    fn release_artifact_recognizes_the_english_fixture_with_its_own_resources() {
        let (Some(artifact), Some(out)) = (std::env::var_os("TOOLBOX_OCR_RELEASE_ROOT").map(PathBuf::from), std::env::var_os("TOOLBOX_OCR_RELEASE_OUT").map(PathBuf::from)) else {
            eprintln!("skipping: TOOLBOX_OCR_RELEASE_ROOT and TOOLBOX_OCR_RELEASE_OUT are not set");
            return;
        };
        let _guard = crate::kit::lock_process_env();
        let _env = EnvGuard::set(&[
            ("TOOLBOX_RESOURCE_ROOT", Some(artifact.clone())),
            ("TOOLBOX_TESSERACT_PATH", None), ("TOOLBOX_TESSDATA_PATH", None), ("TOOLBOX_PDFTOPPM_PATH", None), ("TOOLBOX_OCR_LANGUAGE_DIR", None),
        ]);
        assert!(["ocr/manifest.json", "resources/ocr/manifest.json"].iter().any(|path| artifact.join(path).is_file()), "the artifact at {} has no OCR resource manifest", artifact.display());
        let rasterizer = if cfg!(windows) { "pdftoppm.exe" } else { "pdftoppm" };
        assert!(["pdf-bin", "resources", ""].iter().any(|folder| artifact.join(folder).join(rasterizer).is_file()), "the artifact at {} has no PDF rasterizer", artifact.display());

        let root = sandbox("ocr-release");
        let scan = root.join("scan.pdf");
        scanned_pdf(&scan, &[&["Toolbox page one", "Quarterly invoice summary"]]);
        let text = assert_success("release OCR text", ocr_pdf(RunEvents::silent(), ocr_text_request(vec![scan.clone()], location(&root, "text"))));
        let searchable = assert_success("release searchable PDF", ocr_pdf(RunEvents::silent(), searchable_pdf_request(vec![scan.clone()], location(&root, "searchable"))));
        let recognized = std::fs::read_to_string(&text[0]).unwrap();
        for word in ["Toolbox", "page", "Quarterly", "invoice"] { assert!(recognized.contains(word), "{word} is missing from {recognized:?}"); }
        assert_eq!(lopdf::Document::load(&searchable[0]).unwrap().get_pages().len(), 1, "the searchable PDF keeps the scan's one page");

        std::fs::create_dir_all(&out).unwrap();
        let mut fingerprints = serde_json::Map::new();
        for (name, path) in [("text.txt", &text[0]), ("searchable.pdf", &searchable[0])] {
            let bytes = std::fs::read(path).unwrap();
            std::fs::write(out.join(name), &bytes).unwrap();
            let digest = <sha2::Sha256 as sha2::Digest>::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
            fingerprints.insert(name.into(), digest.into());
        }
        let summary = serde_json::json!({ "os": std::env::consts::OS, "architecture": std::env::consts::ARCH, "files": fingerprints });
        std::fs::write(out.join("fingerprints.json"), format!("{}\n", serde_json::to_string_pretty(&summary).unwrap())).unwrap();
        let _ = std::fs::remove_dir_all(&root);
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
                assert!(error.message.contains(&format!("stopped because {} failed", offender.display())), "{}", error.message);
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
        let outcomes = [&offender, &good].into_iter().flat_map(|path| tauri::async_runtime::block_on(split_pdf(RunEvents::silent(), PageSelectionRequest {
            paths: vec![path.clone()], pages: vec![], page_ranges: None, split_mode: Some("pages".into()), chunk_size: None, output_location: output.clone(),
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
        // OCR skips pages that already have text, so it reads a scan whose words are only pixels.
        let scan = root.join("scan.pdf"); scanned_pdf(&scan, &[&["Toolbox page one"]]);
        let plain_pdf_bytes = std::fs::read(&pdf).unwrap();
        let plain_image_bytes = std::fs::read(&image).unwrap();
        let plain_lossy_image_bytes = std::fs::read(&lossy_image).unwrap();
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
        outputs.extend(named(&mut naming, "export_pdf_scene", pdf.clone(), assert_success("PDF scene", export_pdf_scene(RunEvents::silent(), scene::ExportRequest { paths: vec![pdf.clone()], scene: visual_scene, output_location: OutputLocation::AlongsideInput, sections: Default::default() }))));
        outputs.extend(named(&mut naming, "remove_password", protected.clone(), assert_success("unlock", remove_password(RunEvents::silent(), PasswordRequest { paths: vec![protected], password: crate::kit::TEST_PASSWORD.into(), password_overrides: Default::default(), output_location: location(&root, "unlock") }))));
        outputs.extend(named(&mut naming, "add_page_numbers", pdf.clone(), assert_success("page numbers", add_page_numbers(RunEvents::silent(), PageOverlayRequest { paths: vec![pdf.clone()], position: Some("bottom-right".into()), pages: None, start_number: Some(1), font_size: Some(12), output_location: location(&root, "page numbers") }))));
        outputs.extend(named(&mut naming, "merge_pdfs", pdf.clone(), assert_aggregate_success("merge", &[pdf.clone(), pdf_two.clone()], run_merge_pdfs(&MergePdfRequest { paths: vec![pdf.clone(), pdf_two.clone()], output_location: location(&root, "merge"), output_path: None }))));
        outputs.extend(named(&mut naming, "watermark_pdf", pdf.clone(), assert_success("watermark pdf", watermark_pdf(RunEvents::silent(), PdfWatermarkRequest { paths: vec![pdf.clone()], options: WatermarkOptions { text: Some("TEST".into()), logo_path: None, size: None, opacity: 70, color: None, position: Some(WatermarkPosition::Placement(WatermarkPlacement::Center)), rotation: 0.0 }, pages: None, output_location: location(&root, "watermark pdf") }))));
        outputs.extend(named(&mut naming, "crop_pdf", pdf.clone(), assert_success("crop pdf", crop_pdf(RunEvents::silent(), CropPdfRequest { paths: vec![pdf.clone()], spec: CropSpec::Margins { top: 36.0, right: 18.0, bottom: 36.0, left: 18.0 }, scope: RangedPageScope::All, output_location: location(&root, "crop pdf") }))));
        outputs.extend(named(&mut naming, "resize_pdf", pdf.clone(), assert_success("resize pdf", resize_pdf(RunEvents::silent(), ResizePdfRequest { paths: vec![pdf.clone()], size: TargetSize::Preset("a4".into()), orientation: ResizeOrientation::Match, placement: ResizePlacement::Fit, scope: RangedPageScope::All, output_location: location(&root, "resize pdf") }))));
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
        outputs.extend(named(&mut naming, "ocr_pdf", scan.clone(), assert_ocr_adapter("ocr", ocr_pdf(RunEvents::silent(), vision::OcrRequest { paths: vec![scan.clone()], page_range: None, mode: "text".into(), force: false, languages: vec!["eng".into()], language_overrides: Default::default(), output_location: location(&root, "ocr") }))));
        outputs.extend(named(&mut naming, "extract_pdf_pages", pdf.clone(), assert_success("extract pages", extract_pdf_pages(RunEvents::silent(), PageSelectionRequest { paths: vec![pdf.clone()], pages: vec![], page_ranges: Some("1".into()), split_mode: None, chunk_size: None, output_location: location(&root, "extract pages") }))));
        outputs.extend(named(&mut naming, "compress_pdf", pdf.clone(), assert_success("compress pdf", compress_pdf(RunEvents::silent(), serde_json::from_value::<CompressPdfRequest>(serde_json::json!({ "paths": [pdf.clone()], "quality": 80, "outputLocation": location(&root, "compress pdf") })).expect("Compress Files view payload for compress_pdf")))));
        outputs.extend(named(&mut naming, "rotate_pdf", pdf.clone(), assert_success("rotate pdf", rotate_pdf(RunEvents::silent(), RotatePdfRequest { paths: vec![pdf.clone()], degrees: 90, pages: Some(vec![1]), output_location: location(&root, "rotate pdf") }))));
        outputs.extend(named(&mut naming, "repair_pdf", pdf.clone(), assert_success("repair pdf", repair_pdf(RunEvents::silent(), PdfToTextRequest { pages: None, paths: vec![pdf.clone()], output_location: location(&root, "repair pdf") }))));
        outputs.extend(named(&mut naming, "redact_pdf", pdf.clone(), assert_success("redact pdf", redact_pdf(RunEvents::silent(), RedactPdfRequest { paths: vec![pdf.clone()], rectangle: PdfRect { x: 40.0, y: 650.0, width: 220.0, height: 60.0 }, scope: PageScope::All, output_location: location(&root, "redact pdf") }))));
        outputs.extend(named(&mut naming, "compare_pdfs", pdf.clone(), assert_aggregate_success("compare pdfs", &[pdf.clone(), pdf_two.clone()], run_compare_pdfs(&ComparePdfsRequest { paths: vec![pdf.clone(), pdf_two.clone()], output_location: location(&root, "compare pdfs"), output_path: None }))));
        outputs.extend(named(&mut naming, "convert_images", image.clone(), assert_success("convert", convert_images(RunEvents::silent(), ConvertImagesRequest { paths: vec![image.clone()], format: "jpg".into(), output_location: location(&root, "convert") }))));
        outputs.extend(named(&mut naming, "compress_images", image.clone(), assert_success("compress images", compress_images(RunEvents::silent(), serde_json::from_value::<CompressImagesRequest>(serde_json::json!({ "paths": [image.clone()], "quality": 80, "lossless": false, "outputLocation": location(&root, "compress images") })).expect("Compress Files view payload for compress_images")))));
        outputs.extend(named(&mut naming, "resize_images", image.clone(), assert_success("resize", resize_images(RunEvents::silent(), ResizeRequest { paths: vec![image.clone()], width: 128, height: 128, mode: "exact".into(), resampling: "lanczos".into(), keep_aspect_ratio: true, percentage: 100, longest_side: 128, output_location: location(&root, "resize") }))));
        outputs.extend(named(&mut naming, "rotate_images", image.clone(), assert_success("rotate", rotate_images(RunEvents::silent(), RotateRequest { paths: vec![image.clone()], degrees: 90, flip: "none".into(), output_location: location(&root, "rotate") }))));
        outputs.extend(named(&mut naming, "crop_images", image.clone(), assert_success("crop image", crop_images(RunEvents::silent(), CropRequest { paths: vec![image.clone()], x: 0, y: 0, width: 128, height: 128, mode: "rectangle".into(), aspect_width: 128, aspect_height: 128, anchor: "center".into(), output_location: location(&root, "crop image") }))));
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
        outputs.extend(named(&mut naming, "process_tiff_pages", tiff.clone(), assert_aggregate_success("tiff", &[tiff.clone()], run_process_tiff_pages(&TiffRequest { paths: vec![tiff.clone()], pages: None, output_location: location(&root, "tiff"), output_path: None }))));
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
            spec: CropSpec::Margins { top: 382.5, right: 302.5, bottom: 9.5, left: 9.5 },
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
            ("compress", tauri::async_runtime::block_on(compress_pdf(RunEvents::silent(), CompressPdfRequest { paths: inputs.clone(), quality: 80, output_location: location(&root, "compress") }))),
            ("merge", tauri::async_runtime::block_on(run_merge_pdfs(&MergePdfRequest { paths: inputs.clone(), output_location: location(&root, "merge"), output_path: None }))),
            ("split", tauri::async_runtime::block_on(split_pdf(RunEvents::silent(), PageSelectionRequest { paths: inputs[..1].to_vec(), pages: vec![], page_ranges: None, split_mode: Some("pages".into()), chunk_size: None, output_location: location(&root, "split") }))),
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

        let pdf_outcomes = tauri::async_runtime::block_on(compress_pdf(RunEvents::silent(), CompressPdfRequest {
            paths: vec![pdf.clone()], quality: 50, output_location: location(&root, "pdf"),
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

    #[test]
    fn scene_export_alongside_input_writes_reordered_output_and_keeps_the_original() {
        let root = sandbox("scene-alongside");
        let input = root.join("sample.pdf");
        make_pdf(&input, 3);
        let original = std::fs::read(&input).unwrap();
        let pages = [2, 0, 1].map(|index| serde_json::json!({ "sourceIndex": index, "width": 612, "height": 792, "rotation": 0, "crop": null, "objects": [] }));
        let scene = serde_json::from_value::<scene::PdfScene>(serde_json::json!({ "pages": pages })).unwrap();
        let outputs = assert_success("scene alongside", export_pdf_scene(RunEvents::silent(), scene::ExportRequest { paths: vec![input.clone()], scene, output_location: OutputLocation::AlongsideInput, sections: Default::default() }));
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
            let png = assert_success(&name, convert_images(RunEvents::silent(), ConvertImagesRequest {
                paths: vec![source.clone()], format: "png".into(), output_location: location(&root, &format!("png-{index}")),
            }));
            let converted = image::open(&png[0]).unwrap();
            assert_eq!(converted.color(), expected.color(), "{name} keeps its color type in PNG");
            assert_eq!(converted.as_bytes(), expected.as_bytes(), "{name} keeps its pixels in PNG");
            for format in ["jpg", "webp", "heic"] {
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

        let split = assert_success("split", run_process_tiff_pages(&TiffRequest { paths: vec![multi.clone()], pages: None, output_location: location(&root, "split"), output_path: None }));
        assert_eq!(split.iter().map(tiff_pages).collect::<Vec<_>>(), vec![vec![page(&gray)], vec![page(&rgb)], vec![page(&rgba)]]);

        let combined = assert_aggregate_success("combine", &[multi.clone(), other.clone()], run_process_tiff_pages(&TiffRequest {
            paths: vec![multi.clone(), other.clone()], pages: None, output_location: location(&root, "combine"), output_path: None,
        }));
        assert_eq!(tiff_pages(&combined[0]), vec![page(&gray), page(&rgb), page(&rgba), page(&single)]);

        let selected = assert_aggregate_success("select", &[multi.clone(), other.clone()], run_process_tiff_pages(&TiffRequest {
            paths: vec![multi.clone(), other.clone()],
            pages: Some(vec![TiffPageRef { path: other.clone(), page: 0 }, TiffPageRef { path: multi.clone(), page: 2 }, TiffPageRef { path: multi.clone(), page: 0 }]),
            output_location: location(&root, "select"), output_path: None,
        }));
        assert_eq!(tiff_pages(&selected[0]), vec![page(&single), page(&rgba), page(&gray)]);

        let outside = tauri::async_runtime::block_on(run_process_tiff_pages(&TiffRequest {
            paths: vec![multi.clone()], pages: Some(vec![TiffPageRef { path: multi.clone(), page: 3 }]), output_location: location(&root, "outside"), output_path: None,
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
        let unsupported = "Unsupported TIFF color type or bit depth: RGB(16)";

        let split = tauri::async_runtime::block_on(run_process_tiff_pages(&TiffRequest { paths: vec![deep.clone()], pages: None, output_location: location(&root, "split"), output_path: None }));
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
}
