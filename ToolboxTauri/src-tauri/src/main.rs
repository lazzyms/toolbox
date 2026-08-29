// Prev: ToolboxTauri/src-tauri/src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod kit;

use crate::kit::common::JobOutcome;
use crate::kit::images::{ImageProcessor, Options as ImageOptions, OutputFormat};
use crate::kit::pdf::PDFProcessor;
use crate::kit::common::batch_runner::BatchRunner;
use std::path::PathBuf;

#[tauri::command]
async fn unlock_pdf(paths: Vec<String>, password: String) -> Vec<JobOutcome> {
    let inputs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    BatchRunner::run(inputs, |path| {
        PDFProcessor::remove_password(path, &password)
    })
}

#[tauri::command]
async fn protect_pdf(paths: Vec<String>, password: String) -> Vec<JobOutcome> {
    let inputs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    BatchRunner::run(inputs, |path| {
        PDFProcessor::protect(path, &password)
    })
}

#[tauri::command]
async fn compress_images(paths: Vec<String>, quality: u8) -> Vec<JobOutcome> {
    let inputs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    BatchRunner::run(inputs, |path| {
        ImageProcessor::run(path, ImageOptions {
            target_format: None,
            quality,
            keep_smaller_original: true,
            suffix: "-compressed".to_string(),
        })
    })
}

#[tauri::command]
async fn convert_images(paths: Vec<String>, format: String) -> Vec<JobOutcome> {
    let inputs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();

    let img_format = match format.as_str() {
        "jpg" => OutputFormat::Jpeg,
        "png" => OutputFormat::Png,
        "webp" => OutputFormat::WebP,
        "heic" => OutputFormat::Heic,
        _ => OutputFormat::Png,
    };

    BatchRunner::run(inputs, |path| {
        ImageProcessor::run(path, ImageOptions {
            target_format: Some(img_format),
            quality: 80,
            keep_smaller_original: false,
            suffix: "-converted".to_string(),
        })
    })
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            unlock_pdf,
            protect_pdf,
            compress_images,
            convert_images
        ])
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
