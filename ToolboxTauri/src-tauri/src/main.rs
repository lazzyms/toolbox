// Prev: ToolboxTauri/src-tauri/src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod kit;

use crate::kit::common::{JobOutcome, OutputLocation};
use crate::kit::images::{ImageProcessor, Options as ImageOptions, OutputFormat};
use crate::kit::images::tools;
use crate::kit::pdf::{editor, metadata, remaining, PDFProcessor};
use crate::kit::contracts::{CompressImagesRequest, ConvertImagesRequest, PdfRequest};
use crate::kit::common::batch_runner::BatchRunner;

#[tauri::command]
async fn unlock_pdf(request: PdfRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths, |path| {
        PDFProcessor::remove_password(path, &request.password, &request.output_location)
    })
}

#[tauri::command]
async fn protect_pdf(request: PdfRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths, |path| {
        PDFProcessor::protect(path, &request.password, &request.output_location)
    })
}

#[tauri::command]
async fn compress_images(request: CompressImagesRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths, |path| {
        ImageProcessor::run(path, ImageOptions {
            target_format: None,
            quality: request.quality,
            keep_smaller_original: true,
            suffix: "-compressed".to_string(),
            output_location: request.output_location.clone(),
        })
    })
}

#[tauri::command]
async fn convert_images(request: ConvertImagesRequest) -> Vec<JobOutcome> {
    let img_format = match request.format.as_str() {
        "jpg" => OutputFormat::Jpeg,
        "png" => OutputFormat::Png,
        "webp" => OutputFormat::WebP,
        "heic" => OutputFormat::Heic,
        _ => OutputFormat::Png,
    };

    BatchRunner::run(request.paths, |path| {
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
async fn crop_pdf(request: editor::CropPdfRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths.clone(), |path| editor::crop(&request, path))
}

#[tauri::command]
async fn sign_pdf(request: editor::SignPdfRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths.clone(), |path| editor::sign(&request, path))
}

#[tauri::command]
async fn organize_pdf(request: editor::OrganizePdfRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths.clone(), |path| editor::organize(&request, path))
}

#[tauri::command]
async fn add_page_numbers(request: remaining::PageOverlayRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths.clone(), |path| remaining::add_page_numbers(&request, path))
}

#[tauri::command]
async fn watermark_pdf(request: remaining::PageOverlayRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths.clone(), |path| remaining::watermark(&request, path))
}

#[tauri::command]
async fn compress_pdf(request: remaining::CompressPdfRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths.clone(), |path| remaining::compress(&request, path))
}

#[tauri::command]
async fn remove_pdf_pages(request: remaining::PageSelectionRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths.clone(), |path| remaining::remove_pages(&request, path))
}

#[tauri::command]
async fn extract_pdf_pages(request: remaining::PageSelectionRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths.clone(), |path| remaining::extract_pages(&request, path))
}

#[tauri::command]
async fn merge_pdfs(request: remaining::MergePdfRequest) -> Vec<JobOutcome> {
    vec![remaining::merge(&request)]
}

#[tauri::command]
async fn split_pdf(request: remaining::PageSelectionRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths.clone(), |path| remaining::split(path, &request.output_location))
}

#[tauri::command]
async fn pdf_to_images(request: remaining::PdfToImagesRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths.clone(), |path| remaining::to_images(&request, path))
}

#[tauri::command]
async fn pdf_to_text(request: remaining::PdfToTextRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths.clone(), |path| remaining::to_text(&request, path))
}

#[tauri::command]
async fn extract_pdf_images(request: remaining::PdfToTextRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths.clone(), |path| remaining::extract_images(&request, path))
}

#[tauri::command]
async fn images_to_pdf(request: remaining::ImagesToPdfRequest) -> Vec<JobOutcome> {
    vec![remaining::images_to_pdf(&request)]
}
#[tauri::command]
async fn resize_images(request: tools::ResizeRequest) -> Vec<JobOutcome> { BatchRunner::run(request.paths.clone(), |path| tools::resize(&request, path)) }
#[tauri::command]
async fn rotate_images(request: tools::RotateRequest) -> Vec<JobOutcome> { BatchRunner::run(request.paths.clone(), |path| tools::rotate(&request, path)) }
#[tauri::command]
async fn crop_images(request: tools::CropRequest) -> Vec<JobOutcome> { BatchRunner::run(request.paths.clone(), |path| tools::crop(&request, path)) }
#[tauri::command]
async fn adjust_image_tone(request: tools::ToneRequest) -> Vec<JobOutcome> { BatchRunner::run(request.paths.clone(), |path| tools::tone(&request, path)) }
#[tauri::command]
async fn watermark_images(request: tools::WatermarkRequest) -> Vec<JobOutcome> { BatchRunner::run(request.paths.clone(), |path| tools::watermark(&request, path)) }
#[tauri::command]
async fn generate_icon_set(request: tools::IconSetRequest) -> Vec<JobOutcome> { BatchRunner::run(request.paths.clone(), |path| tools::icon_set(&request, path)) }
#[tauri::command]
async fn create_gif(request: tools::GifCreateRequest) -> Vec<JobOutcome> { vec![tools::gif_create(&request)] }
#[tauri::command]
async fn extract_gif_frames(request: tools::GifExtractRequest) -> Vec<JobOutcome> { BatchRunner::run(request.paths.clone(), |path| tools::gif_extract(&request, path)) }
#[tauri::command]
async fn process_tiff_pages(request: tools::TiffRequest) -> Vec<JobOutcome> { BatchRunner::run(request.paths.clone(), |path| tools::tiff(&request, path)) }
#[tauri::command]
async fn image_metadata(request: tools::MetadataRequest) -> Vec<JobOutcome> { BatchRunner::run(request.paths.clone(), |path| tools::strip_metadata(&request, path)) }

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            unlock_pdf,
            protect_pdf,
            compress_images,
            convert_images,
            inspect_pdf,
            crop_pdf,
            sign_pdf,
            organize_pdf
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
    use std::path::PathBuf;

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
    fn convert_command_batches_multiple_files() {
        let a = temp_path("cmd_a.png");
        let b = temp_path("cmd_b.png");
        write_png(&a, 128, 1);
        write_png(&b, 128, 2);

        let out = tauri::async_runtime::block_on(convert_images(ConvertImagesRequest {
            paths: vec![a.clone(), b.clone()], format: "jpg".to_string(), output_location: OutputLocation::AlongsideInput,
        }));
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

        let out = tauri::async_runtime::block_on(compress_images(CompressImagesRequest {
            paths: vec![src.clone()], quality: 50, output_location: OutputLocation::AlongsideInput,
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
    fn convert_command_keeps_batch_failures_isolated() {
        let valid = temp_path("cmd_mixed_valid.png");
        let missing = temp_path("cmd_mixed_missing.png");
        write_png(&valid, 64, 9);

        let out = tauri::async_runtime::block_on(convert_images(ConvertImagesRequest {
            paths: vec![valid.clone(), missing.clone()],
            format: "jpg".to_string(),
            output_location: OutputLocation::AlongsideInput,
        }));
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|job| job.failure.is_none()));
        assert!(out.iter().any(|job| job.failure.is_some()));

        cleanup(&out.iter().flat_map(|job| job.output_paths.clone()).collect::<Vec<_>>());
        let _ = std::fs::remove_file(valid);
    }
}
