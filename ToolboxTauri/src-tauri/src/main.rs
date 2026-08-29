// Prev: ToolboxTauri/src-tauri/src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod kit;

use crate::kit::common::{JobOutcome, OutputLocation};
use crate::kit::images::{ImageProcessor, ImageProcessor::Options as ImageOptions};
use crate::kit::pdf::PDFProcessor;
use crate::kit::common::batch_runner::BatchRunner;
use std::path::PathBuf;
use image::ImageFormat;

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
        "jpg" => ImageFormat::Jpeg,
        "png" => ImageFormat::Png,
        "webp" => ImageFormat::WebP,
        _ => ImageFormat::Png,
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
