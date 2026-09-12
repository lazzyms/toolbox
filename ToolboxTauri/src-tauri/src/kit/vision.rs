use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use image::GenericImageView;
use lopdf::Document;

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::ToolError;
use crate::kit::resources;

const OCR_MAX_INPUT_BYTES: u64 = 100 * 1024 * 1024;
const OCR_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;
const OCR_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionRequest {
    pub paths: Vec<PathBuf>,
    #[serde(default)] pub pages: Option<Vec<usize>>,
    pub output_location: OutputLocation,
}

pub fn ocr_pdf(request: &VisionRequest, input: PathBuf) -> JobOutcome {
    if request.pages.as_ref().is_some_and(Vec::is_empty) {
        return failure(input, "Select at least one PDF page for OCR.".to_string());
    }
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-ocr-text", "txt") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve OCR output: {error}")),
    };
    let input_size = match std::fs::metadata(&input) {
        Ok(metadata) => metadata.len(),
        Err(error) => return failure(input, format!("Could not inspect PDF for OCR: {error}")),
    };
    if input_size > OCR_MAX_INPUT_BYTES { return failure(input, "PDF exceeds the 100 MiB OCR input limit.".to_string()); }
    let pages = match selected_ocr_pages(&input, request.pages.as_deref()) {
        Ok(pages) => pages,
        Err(error) => return failure(input, error),
    };
    let engine = match find_engine("ocr", "TOOLBOX_TESSERACT_PATH", "tesseract") {
        Ok(engine) => engine,
        Err(error) => return unavailable(input, error),
    };
    let renderer = match find_pdf_renderer() {
        Ok(renderer) => renderer,
        Err(error) => return unavailable(input, error),
    };
    let result = run_ocr(&engine, &renderer, &input, &pages);
    match result {
        Ok(stdout) => {
            if stdout.len() > OCR_MAX_OUTPUT_BYTES { return failure(input, "OCR output exceeds the 10 MiB text limit.".to_string()); }
            let text = normalize_ocr_text(&stdout);
            if text.is_empty() { return failure(input, "OCR completed but found no readable text.".to_string()); }
            match std::fs::write(output.path(), text) {
                Ok(_) => match output.publish() {
                    Ok(path) => success(input, path, "OCR text extracted in page order"),
                    Err(error) => failure(input, format!("Could not publish OCR output: {error}")),
                },
                Err(error) => failure(input, format!("Could not write OCR output: {error}")),
            }
        }
        Err(error) => failure(input, error),
    }
}

fn selected_ocr_pages(input: &Path, requested: Option<&[usize]>) -> Result<Vec<u32>, String> {
    let document = Document::load(input).map_err(|error| format!("Could not inspect PDF for OCR: {error}"))?;
    let pages = document.get_pages().keys().copied().collect::<Vec<_>>();
    let Some(requested) = requested else {
        return if pages.is_empty() { Err("PDF has no pages to OCR.".to_string()) } else { Ok(pages) };
    };
    if requested.is_empty() { return Err("Select at least one PDF page for OCR.".to_string()); }
    let requested = requested.iter().map(|page| page.checked_add(1).and_then(|page| u32::try_from(page).ok())).collect::<Option<std::collections::HashSet<_>>>()
        .ok_or_else(|| "Selected PDF pages are outside the document.".to_string())?;
    if requested.iter().any(|page| !pages.contains(page)) {
        return Err("Selected PDF pages are outside the document.".to_string());
    }
    let selected = pages.into_iter().filter(|page| requested.contains(page)).collect::<Vec<_>>();
    if selected.is_empty() { Err("Select at least one PDF page for OCR.".to_string()) } else { Ok(selected) }
}

struct OcrWorkspace(PathBuf);

impl OcrWorkspace {
    fn new() -> Result<Self, String> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        for attempt in 0..100 {
            let path = std::env::temp_dir().join(format!("toolbox_ocr_{}_{}_{}", std::process::id(), timestamp, attempt));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("Could not create private OCR workspace: {error}")),
            }
        }
        Err("Could not allocate a unique private OCR workspace.".to_string())
    }

    fn page_prefix(&self, page: u32) -> PathBuf { self.0.join(format!("page-{page}")) }
}

impl Drop for OcrWorkspace {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn run_ocr(engine: &Path, renderer: &Path, input: &Path, pages: &[u32]) -> Result<Vec<u8>, String> {
    let workspace = OcrWorkspace::new()?;
    let result = (|| {
        let mut text = Vec::new();
        for page in pages {
            let image = render_ocr_page(renderer, input, &workspace, *page)?;
            let result = run_command({
                let mut command = Command::new(engine);
                command.arg(&image).arg("stdout");
                command
            })?;
            if !result.status.success() { return Err(stderr(result, "OCR engine failed.")); }
            if text.len().saturating_add(result.stdout.len()).saturating_add(1) > OCR_MAX_OUTPUT_BYTES { return Err("OCR output exceeds the 10 MiB text limit.".to_string()); }
            text.extend_from_slice(&result.stdout);
            text.push(b'\n');
        }
        Ok(text)
    })();
    drop(workspace);
    result
}

fn render_ocr_page(renderer: &Path, input: &Path, workspace: &OcrWorkspace, page: u32) -> Result<PathBuf, String> {
    let prefix = workspace.page_prefix(page);
    let output = prefix.with_extension("png");
    let result = run_command({
        let mut command = Command::new(renderer);
        command.arg("-png").arg("-r").arg("200").arg("-f").arg(page.to_string()).arg("-l").arg(page.to_string()).arg("-singlefile").arg(input).arg(&prefix);
        command
    })?;
    if result.status.success() && output.is_file() { Ok(output) } else { Err(stderr(result, "PDF renderer failed to render the selected OCR page.")) }
}

fn run_command(mut command: Command) -> Result<std::process::Output, String> {
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|error| format!("Could not run OCR helper: {error}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait().map_err(|error| format!("Could not read OCR helper status: {error}"))? {
            Some(_) => return child.wait_with_output().map_err(|error| format!("Could not collect OCR helper output: {error}")),
            None if started.elapsed() >= OCR_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("OCR helper timed out after 120 seconds.".to_string());
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn find_pdf_renderer() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("TOOLBOX_PDFTOPPM_PATH") {
        let path = PathBuf::from(path);
        if path.is_file() { return Ok(path); }
        return Err("TOOLBOX_PDFTOPPM_PATH does not point to a file.".to_string());
    }
    let root = crate::kit::resources::application_resource_root().unwrap_or_default();
    let executable = if cfg!(windows) { "pdftoppm.exe" } else { "pdftoppm" };
    [root.join("pdf-bin").join(executable), root.join("resources").join(executable), root.join(executable)]
        .into_iter()
        .find(|path| path.is_file())
        .or_else(|| Command::new(executable).arg("-h").output().ok().filter(|result| result.status.success()).map(|_| PathBuf::from(executable)))
        .ok_or_else(|| "PDF rasterizer is unavailable. Bundle pdftoppm or set TOOLBOX_PDFTOPPM_PATH.".to_string())
}

fn normalize_ocr_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn blur_faces(request: &VisionRequest, input: PathBuf) -> JobOutcome {
    run_image_adapter(request, input, "faceBlur", "TOOLBOX_FACE_BLUR_PATH", "toolbox-face-blur", "-blurred", "Blurred faces", ImageAdapterValidation::FaceBlur)
}

pub fn remove_background(request: &VisionRequest, input: PathBuf) -> JobOutcome {
    run_image_adapter(request, input, "backgroundRemoval", "TOOLBOX_BACKGROUND_REMOVAL_PATH", "toolbox-background-removal", "-cutout", "Background removed", ImageAdapterValidation::Cutout)
}

enum ImageAdapterValidation { FaceBlur, Cutout }

fn run_image_adapter(request: &VisionRequest, input: PathBuf, resource_name: &str, variable: &str, command: &str, suffix: &str, detail: &str, validation: ImageAdapterValidation) -> JobOutcome {
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, suffix, "png") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve vision output: {error}")),
    };
    let engine = match find_engine(resource_name, variable, command) {
        Ok(engine) => engine,
        Err(error) => return unavailable(input, error),
    };
    match Command::new(engine).arg(&input).arg(output.path()).output() {
        Ok(result) if result.status.success() && output.path().is_file() => {
            let validation = match validation {
                ImageAdapterValidation::FaceBlur => validate_face_blur(&input, output.path()),
                ImageAdapterValidation::Cutout => validate_cutout(output.path()),
            };
            match validation {
                Ok(()) => match output.publish() {
                    Ok(path) => success(input, path, detail),
                    Err(error) => failure(input, format!("Could not publish vision output: {error}")),
                },
                Err(error) => failure(input, error),
            }
        }
        Ok(result) if result.status.success() => {
            failure(input, "Vision adapter reported success but produced no output.".to_string())
        }
        Ok(result) => failure(input, stderr(result, "Vision adapter failed.")),
        Err(error) => failure(input, format!("Could not run vision adapter: {error}")),
    }
}

fn find_engine(name: &str, variable: &str, command: &str) -> Result<PathBuf, String> {
    let root = resources::application_resource_root().map(|root| root.join("vision")).unwrap_or_default();
    resources::resolve(name, &root, variable, command).map(|resource| resource.path)
}

fn validate_cutout(path: &std::path::Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|error| format!("Background adapter output could not be read: {error}"))?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Background adapter must produce a PNG with a transparent subject mask.".to_string());
    }
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .map_err(|error| format!("Background adapter produced an unreadable PNG: {error}"))?
        .to_rgba8();
    let has_visible = image.pixels().any(|pixel| pixel.0[3] > 0);
    let has_soft_edge = image.pixels().any(|pixel| pixel.0[3] < 255);
    if !has_visible || !has_soft_edge { return Err("Background adapter produced no transparent subject mask.".to_string()); }
    Ok(())
}

fn validate_face_blur(input: &std::path::Path, output: &std::path::Path) -> Result<(), String> {
    let input_image = image::open(input).map_err(|error| format!("Face adapter input could not be read: {error}"))?;
    let output_bytes = std::fs::read(output).map_err(|error| format!("Face adapter output could not be read: {error}"))?;
    if !output_bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Face adapter must produce a PNG output.".to_string());
    }
    let output_image = image::load_from_memory_with_format(&output_bytes, image::ImageFormat::Png)
        .map_err(|error| format!("Face adapter produced an unreadable PNG: {error}"))?;
    if input_image.dimensions() != output_image.dimensions() {
        return Err("Face adapter changed the image dimensions.".to_string());
    }
    if input_image.to_rgba8() == output_image.to_rgba8() {
        return Err("Face adapter reported success without detecting or blurring a face.".to_string());
    }
    Ok(())
}

fn success(input_path: PathBuf, output: PathBuf, detail: &str) -> JobOutcome { JobOutcome { input_path, output_paths: vec![output], detail: detail.to_string(), failure: None } }
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::processing(error)) }
fn unavailable(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::unavailable(error)) }
fn stderr(result: std::process::Output, fallback: &str) -> String { String::from_utf8_lossy(&result.stderr).trim().lines().last().filter(|line| !line.is_empty()).unwrap_or(fallback).to_string() }

#[cfg(test)]
mod tests {
    use super::{normalize_ocr_text, ocr_pdf, validate_cutout, validate_face_blur, VisionRequest};
    use image::{Rgba, RgbaImage};
    use lopdf::{dictionary, Document, Object, Stream};
    use std::fs;
    use std::path::{Path, PathBuf};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    static OCR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn path(name: &str) -> PathBuf { std::env::temp_dir().join(format!("toolbox_vision_{}_{}", std::process::id(), name)) }

    fn make_pdf(path: &PathBuf, page_count: usize) {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let mut kids = Vec::new();
        for number in 0..page_count {
            let content_id = document.add_object(Stream::new(dictionary! {}, format!("BT /F1 24 Tf 72 720 Td (OCR page {}) Tj ET", number + 1).into_bytes()));
            let page_id = document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } }, "Contents" => content_id,
            });
            kids.push(Object::Reference(page_id));
        }
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => page_count as i64 }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    #[cfg(unix)]
    fn fake_ocr_engine(fail_on_page_two: bool) -> PathBuf {
        let engine = path("fake-ocr.sh");
        let page_two = if fail_on_page_two { "  *page-2.png) printf 'OCR failure\\n' >&2; exit 1 ;;" } else { "  *page-2.png) printf 'page two\\n' ;;" };
        fs::write(&engine, format!("#!/bin/sh\ncase \"$1\" in\n  *page-1.png) printf 'selected page one\\n' ;;\n{page_two}\n  *.pdf) printf 'selected page one\\npage two\\n' ;;\n  *) exit 2 ;;\nesac\n")).unwrap();
        let mut permissions = fs::metadata(&engine).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&engine, permissions).unwrap();
        engine
    }

    #[test]
    fn selected_ocr_pages_preserve_document_order_and_reject_out_of_range_pages() {
        let input = path("selected-pages-scope.pdf");
        make_pdf(&input, 2);

        assert_eq!(super::selected_ocr_pages(&input, Some(&[1, 0])).unwrap(), vec![1, 2]);
        assert!(super::selected_ocr_pages(&input, Some(&[2])).is_err());

        let _ = fs::remove_file(input);
    }

    #[cfg(unix)]
    #[test]
    fn selected_ocr_page_cannot_process_an_excluded_page() {
        let _guard = OCR_ENV_LOCK.lock().unwrap();
        let input = path("selected-pages.pdf");
        let output_root = path("selected-pages-output");
        make_pdf(&input, 2);
        let original = fs::read(&input).unwrap();
        fs::create_dir_all(&output_root).unwrap();
        let engine = fake_ocr_engine(false);
        let old_engine = std::env::var_os("TOOLBOX_TESSERACT_PATH");
        std::env::set_var("TOOLBOX_TESSERACT_PATH", &engine);

        let outcome = ocr_pdf(&VisionRequest {
            paths: vec![input.clone()],
            pages: Some(vec![0]),
            output_location: crate::kit::common::OutputLocation::CustomFolder(output_root.clone()),
        }, input.clone());
        let text = outcome.output_paths.first().and_then(|path| fs::read_to_string(path).ok());
        let output_path = outcome.output_paths.first().cloned();
        let input_after = fs::read(&input).unwrap();

        match old_engine {
            Some(value) => std::env::set_var("TOOLBOX_TESSERACT_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSERACT_PATH"),
        }
        let _ = fs::remove_file(&engine);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_dir_all(&output_root);

        assert!(outcome.failure.is_none(), "selected OCR failed: {:?}", outcome.failure);
        assert_eq!(text.as_deref(), Some("selected page one"));
        assert_eq!(output_path.as_deref().and_then(Path::parent), Some(output_root.as_path()));
        assert_eq!(input_after, original);
    }

    #[cfg(unix)]
    #[test]
    fn failed_selected_ocr_cleans_private_workspace_and_reserved_output() {
        let _guard = OCR_ENV_LOCK.lock().unwrap();
        let input = path("failed-selected-pages.pdf");
        let output_root = path("failed-selected-pages-output");
        make_pdf(&input, 2);
        let original = fs::read(&input).unwrap();
        fs::create_dir_all(&output_root).unwrap();
        let engine = fake_ocr_engine(true);
        let before_workspaces = ocr_workspaces();
        let old_engine = std::env::var_os("TOOLBOX_TESSERACT_PATH");
        std::env::set_var("TOOLBOX_TESSERACT_PATH", &engine);

        let outcome = ocr_pdf(&VisionRequest {
            paths: vec![input.clone()],
            pages: Some(vec![1]),
            output_location: crate::kit::common::OutputLocation::CustomFolder(output_root.clone()),
        }, input.clone());
        let after_workspaces = ocr_workspaces();
        let input_after = fs::read(&input).unwrap();
        let output_entries = fs::read_dir(&output_root).unwrap().count();

        match old_engine {
            Some(value) => std::env::set_var("TOOLBOX_TESSERACT_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSERACT_PATH"),
        }
        let _ = fs::remove_file(&engine);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_dir_all(&output_root);

        assert!(outcome.failure.as_ref().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::Processing)));
        assert!(outcome.output_paths.is_empty());
        assert_eq!(before_workspaces, after_workspaces);
        assert_eq!(original, input_after);
        assert_eq!(output_entries, 0);
    }

    #[cfg(unix)]
    fn ocr_workspaces() -> Vec<PathBuf> {
        let prefix = format!("toolbox_ocr_{}", std::process::id());
        let mut paths = fs::read_dir(std::env::temp_dir()).unwrap().filter_map(Result::ok).map(|entry| entry.path()).filter(|path| {
            path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(&prefix))
        }).collect::<Vec<_>>();
        paths.sort();
        paths
    }

    #[test]
    fn accepts_cutouts_with_soft_transparency() {
        let output = path("soft.png");
        let mut image = RgbaImage::from_pixel(2, 1, Rgba([20, 30, 40, 255]));
        image.put_pixel(1, 0, Rgba([20, 30, 40, 120]));
        image.save(&output).unwrap();
        assert!(validate_cutout(&output).is_ok());
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn rejects_opaque_adapter_outputs() {
        let output = path("opaque.png");
        RgbaImage::from_pixel(2, 1, Rgba([20, 30, 40, 255])).save(&output).unwrap();
        assert!(validate_cutout(&output).is_err());
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn rejects_identity_face_blur_outputs() {
        let input = path("face-input.png");
        let output = path("face-output.png");
        let image = RgbaImage::from_pixel(2, 1, Rgba([20, 30, 40, 255]));
        image.save(&input).unwrap();
        image.save(&output).unwrap();
        assert!(validate_face_blur(&input, &output).is_err());
        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn normalizes_ocr_lines_without_reordering_them() {
        assert_eq!(normalize_ocr_text(b"  first   line\r\n\n second line  \n"), "first line\nsecond line");
    }
}
