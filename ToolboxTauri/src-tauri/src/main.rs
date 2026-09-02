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
    use image::Rgba;

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

    #[test]
    fn convert_command_heic_and_webp() {
        let src = temp_path("cmd_convert.png");
        write_png(&src, 256, 44);

        let heic = tauri::async_runtime::block_on(convert_images(
            vec![src.display().to_string()],
            "heic".to_string(),
        ));
        assert!(heic[0].failure.is_none(), "{}", heic[0].failure.clone().unwrap_or_default());
        assert_eq!(
            heic[0].output_paths[0].extension().and_then(|e| e.to_str()),
            Some("heic")
        );
        let bytes = std::fs::read(&heic[0].output_paths[0]).unwrap();
        assert!(heif::decode(&bytes).is_ok(), "command output must be real HEIC");

        let webp = tauri::async_runtime::block_on(convert_images(
            vec![src.display().to_string()],
            "webp".to_string(),
        ));
        assert!(webp[0].failure.is_none(), "{}", webp[0].failure.clone().unwrap_or_default());
        let bytes = std::fs::read(&webp[0].output_paths[0]).unwrap();
        assert_eq!(&bytes[..4], b"RIFF", "command output must be real WebP");

        cleanup(&heic[0].output_paths);
        cleanup(&webp[0].output_paths);
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn convert_command_batches_multiple_files() {
        let a = temp_path("cmd_a.png");
        let b = temp_path("cmd_b.png");
        write_png(&a, 128, 1);
        write_png(&b, 128, 2);

        let out = tauri::async_runtime::block_on(convert_images(
            vec![a.display().to_string(), b.display().to_string()],
            "jpg".to_string(),
        ));
        assert_eq!(out.len(), 2);
        for job in &out {
            assert!(job.failure.is_none(), "{}", job.failure.clone().unwrap_or_default());
            assert_eq!(job.output_paths.len(), 1);
        }

        cleanup(&out[0].output_paths);
        cleanup(&out[1].output_paths);
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn compress_command_keeps_original_format() {
        let src = temp_path("cmd_compress.png");
        write_png(&src, 128, 7);
        let before = std::fs::metadata(&src).unwrap().len();

        let out = tauri::async_runtime::block_on(compress_images(vec![src.display().to_string()], 50));
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
}
