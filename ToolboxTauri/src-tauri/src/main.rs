#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod file_actions;
mod kit;

use crate::file_actions::{open_output_path, reveal_output_path};
use crate::kit::common::JobOutcome;
use crate::kit::images::{ImageProcessor, Options as ImageOptions, OutputFormat};
use crate::kit::images::tools;
use crate::kit::pdf::{editor, metadata, remaining, PDFProcessor};
use crate::kit::vision;
use crate::kit::contracts::{CompressImagesRequest, ConvertImagesRequest, PasswordRequest, PdfRequest};
use crate::kit::common::batch_runner::BatchRunner;
use crate::kit::password::PasswordProcessor;

#[tauri::command]
async fn remove_password(request: PasswordRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths, |path| {
        PasswordProcessor::remove_password(path, &request.password, &request.output_location)
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
            quality: if request.lossless { 0 } else { request.quality },
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
async fn edit_pdf(request: editor::EditPdfRequest) -> Vec<JobOutcome> {
    BatchRunner::run(request.paths.clone(), |path| editor::edit(&request, path))
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
    BatchRunner::run(request.paths.clone(), |path| remaining::split(&request, path))
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
async fn ocr_pdf(request: vision::VisionRequest) -> Vec<JobOutcome> { BatchRunner::run(request.paths.clone(), |path| vision::ocr_pdf(&request, path)) }
#[tauri::command]
async fn blur_faces(request: vision::VisionRequest) -> Vec<JobOutcome> { BatchRunner::run(request.paths.clone(), |path| vision::blur_faces(&request, path)) }
#[tauri::command]
async fn remove_image_background(request: vision::VisionRequest) -> Vec<JobOutcome> { BatchRunner::run(request.paths.clone(), |path| vision::remove_background(&request, path)) }
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
async fn process_tiff_pages(request: tools::TiffRequest) -> Vec<JobOutcome> { vec![tools::tiff(&request)] }
#[tauri::command]
async fn image_metadata(request: tools::MetadataRequest) -> Vec<JobOutcome> { BatchRunner::run(request.paths.clone(), |path| tools::strip_metadata(&request, path)) }
#[tauri::command]
fn inspect_image_metadata(request: tools::MetadataRequest) -> Vec<Result<tools::MetadataReport, String>> { request.paths.into_iter().map(tools::inspect_metadata).collect() }

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
            crop_pdf,
            edit_pdf,
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
            ,inspect_image_metadata
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
    use crate::kit::images::tools::{CropRequest, GifCreateRequest, GifExtractRequest, IconSetRequest, MetadataRequest, ResizeRequest, RotateRequest, TiffRequest, ToneRequest, WatermarkRequest};
    use crate::kit::pdf::editor::{CropPdfRequest, EditPdfRequest, OrganizePdfRequest, PageScope, PdfRect, RotatePage, SignPdfRequest};
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

    fn assert_adapter_unavailable<F>(name: &str, future: F)
    where F: Future<Output = Vec<JobOutcome>> {
        let outcomes = tauri::async_runtime::block_on(future);
        assert_eq!(outcomes.len(), 1, "{name} should return one input outcome");
        let outcome = &outcomes[0];
        if let Some(error) = &outcome.failure {
            assert!(matches!(error.kind, crate::kit::contracts::ErrorKind::Unavailable), "{name} failed for an unexpected reason: {error}");
            assert!(outcome.output_paths.is_empty(), "{name} must not fake an output when the adapter is unavailable");
        } else {
            assert!(!outcome.output_paths.is_empty(), "{name} adapter reported success without an output");
            for output in &outcome.output_paths { assert!(output.is_file(), "{name} reported missing output {}", output.display()); }
        }
    }

    #[test]
    fn every_registered_command_runs_with_real_fixtures_in_isolated_sandboxes() {
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
        outputs.extend(assert_success("unlock", remove_password(PasswordRequest { paths: vec![protected], password: "test-password".into(), output_location: location(&root, "unlock") })));
        outputs.extend(assert_success("page numbers", add_page_numbers(PageOverlayRequest { paths: vec![pdf.clone()], text: "1".into(), opacity: 100, position: Some("bottom-right".into()), logo_path: None, pages: None, start_number: Some(1), font_size: Some(12), output_location: location(&root, "page numbers") })));
        outputs.extend(assert_success("merge", merge_pdfs(MergePdfRequest { paths: vec![pdf.clone(), pdf_two.clone()], output_location: location(&root, "merge") })));
        outputs.extend(assert_success("watermark pdf", watermark_pdf(PageOverlayRequest { paths: vec![pdf.clone()], text: "TEST".into(), opacity: 70, position: Some("center".into()), logo_path: None, pages: None, start_number: None, font_size: None, output_location: location(&root, "watermark pdf") })));
        outputs.extend(assert_success("crop pdf", crop_pdf(CropPdfRequest { paths: vec![pdf.clone()], rectangle: PdfRect { x: 0.5, y: 0.5, width: 500.0, height: 700.0 }, scope: PageScope::All, output_location: location(&root, "crop pdf") })));
        outputs.extend(assert_success("edit pdf", edit_pdf(EditPdfRequest { paths: vec![pdf.clone()], mode: "highlight".into(), text: "TEST NOTE".into(), pages: None, rectangle: PdfRect { x: 40.0, y: 650.0, width: 220.0, height: 60.0 }, output_location: location(&root, "edit pdf") })));
        outputs.extend(assert_success("protect", protect_pdf(PdfRequest { paths: vec![pdf_two.clone()], password: "another-password".into(), output_location: location(&root, "protect") })));
        outputs.extend(assert_success("images to pdf", images_to_pdf(ImagesToPdfRequest { paths: vec![image.clone(), image_two.clone()], output_location: location(&root, "images to pdf") })));
        outputs.extend(assert_success("pdf to images", pdf_to_images(PdfToImagesRequest { paths: vec![pdf.clone()], dpi: 72, format: "png".into(), page_range: None, output_location: location(&root, "pdf to images") })));
        outputs.extend(assert_success("pdf to text", pdf_to_text(PdfToTextRequest { paths: vec![pdf.clone()], output_location: location(&root, "pdf to text") })));
        outputs.extend(assert_success("split", split_pdf(PageSelectionRequest { paths: vec![pdf.clone()], pages: vec![], page_ranges: None, split_mode: Some("pages".into()), chunk_size: None, output_location: location(&root, "split") })));
        outputs.extend(assert_success("extract pdf images", extract_pdf_images(PdfToTextRequest { paths: vec![image_pdf.clone()], output_location: location(&root, "extract pdf images") })));
        outputs.extend(assert_success("sign", sign_pdf(SignPdfRequest { paths: vec![pdf.clone()], page: 0, text: "Signed".into(), signature_path: None, rectangle: PdfRect { x: 40.0, y: 40.0, width: 180.0, height: 60.0 }, scope: PageScope::All, output_location: location(&root, "sign") })));
        assert_adapter_unavailable("ocr", ocr_pdf(VisionRequest { paths: vec![pdf.clone()], output_location: location(&root, "ocr") }));
        outputs.extend(assert_success("remove pages", remove_pdf_pages(PageSelectionRequest { paths: vec![pdf.clone()], pages: vec![0], page_ranges: None, split_mode: None, chunk_size: None, output_location: location(&root, "remove pages") })));
        outputs.extend(assert_success("extract pages", extract_pdf_pages(PageSelectionRequest { paths: vec![pdf.clone()], pages: vec![], page_ranges: Some("1".into()), split_mode: None, chunk_size: None, output_location: location(&root, "extract pages") })));
        outputs.extend(assert_success("organize", organize_pdf(OrganizePdfRequest { paths: vec![pdf.clone()], page_order: vec![1, 0], delete_pages: vec![], rotate_pages: vec![RotatePage { page: 0, degrees: 90 }], scope: PageScope::All, output_location: location(&root, "organize") })));
        outputs.extend(assert_success("compress pdf", compress_pdf(CompressPdfRequest { paths: vec![pdf.clone()], quality: 80, output_location: location(&root, "compress pdf") })));
        outputs.extend(assert_success("convert", convert_images(ConvertImagesRequest { paths: vec![image.clone()], format: "jpg".into(), output_location: location(&root, "convert") })));
        outputs.extend(assert_success("compress images", compress_images(CompressImagesRequest { paths: vec![image.clone()], quality: 80, lossless: false, output_location: location(&root, "compress images") })));
        outputs.extend(assert_success("resize", resize_images(ResizeRequest { paths: vec![image.clone()], width: 128, height: 128, mode: "exact".into(), resampling: "lanczos".into(), keep_aspect_ratio: true, percentage: 100, longest_side: 128, output_location: location(&root, "resize") })));
        outputs.extend(assert_success("rotate", rotate_images(RotateRequest { paths: vec![image.clone()], degrees: 90, flip: "none".into(), output_location: location(&root, "rotate") })));
        outputs.extend(assert_success("crop image", crop_images(CropRequest { paths: vec![image.clone()], x: 0, y: 0, width: 128, height: 128, mode: "rectangle".into(), aspect_width: 128, aspect_height: 128, anchor: "center".into(), output_location: location(&root, "crop image") })));
        outputs.extend(assert_success("icons", generate_icon_set(IconSetRequest { paths: vec![image.clone()], preset: "favicon".into(), sizes: vec![], output_location: location(&root, "icons") })));
        outputs.extend(assert_success("create gif", create_gif(GifCreateRequest { paths: vec![image.clone(), image_two.clone()], frame_delay_ms: 100, loop_forever: true, output_location: location(&root, "create gif") })));
        outputs.extend(assert_success("extract gif", extract_gif_frames(GifExtractRequest { paths: vec![animated.clone()], output_location: location(&root, "extract gif") })));
        outputs.extend(assert_success("watermark image", watermark_images(WatermarkRequest { paths: vec![image.clone()], opacity: 70, text: Some("TEST".into()), logo_path: None, x: 16, y: 16, output_location: location(&root, "watermark image") })));
        outputs.extend(assert_success("metadata", image_metadata(MetadataRequest { paths: vec![image.clone()], output_location: location(&root, "metadata") })));
        outputs.extend(assert_success("tone", adjust_image_tone(ToneRequest { paths: vec![image.clone()], brightness: 20, contrast: 0.0, saturation: 0.0, exposure: 0.0, output_location: location(&root, "tone") })));
        outputs.extend(assert_success("tiff", process_tiff_pages(TiffRequest { paths: vec![tiff.clone()], output_location: location(&root, "tiff") })));
        assert_adapter_unavailable("face blur", blur_faces(VisionRequest { paths: vec![image.clone()], output_location: location(&root, "face blur") }));
        assert_adapter_unavailable("background removal", remove_image_background(VisionRequest { paths: vec![image.clone()], output_location: location(&root, "background removal") }));

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
