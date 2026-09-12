use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::io::Read;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use image::GenericImageView;
use lopdf::Document;

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::ToolError;
use crate::kit::pdf::metadata::page_bounds;
use crate::kit::resources;

const OCR_MAX_INPUT_BYTES: u64 = 100 * 1024 * 1024;
const OCR_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;
const OCR_RENDER_DPI: u32 = 200;
const OCR_MAX_SINGLE_PIXELS: u64 = 40_000_000;
const OCR_MAX_TOTAL_PIXELS: u64 = 120_000_000;
const OCR_MAX_RENDERED_BYTES: u64 = 50 * 1024 * 1024;
const OCR_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy)]
struct OcrPage {
    number: u32,
}

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
            let text = match normalize_ocr_text(&stdout) {
                Ok(text) => text,
                Err(error) => return failure(input, error),
            };
            if text.len() > OCR_MAX_OUTPUT_BYTES { return failure(input, "OCR output exceeds the 10 MiB text limit.".to_string()); }
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

fn selected_ocr_pages(input: &Path, requested: Option<&[usize]>) -> Result<Vec<OcrPage>, String> {
    let document = Document::load(input).map_err(|error| format!("Could not inspect PDF for OCR: {error}"))?;
    let pages = document.get_pages();
    if pages.is_empty() { return Err("PDF has no pages to OCR.".to_string()); }
    let requested = requested.map(|pages| {
        if pages.is_empty() { return Err("Select at least one PDF page for OCR.".to_string()); }
        pages.iter().map(|page| page.checked_add(1).and_then(|page| u32::try_from(page).ok())).collect::<Option<std::collections::HashSet<_>>>()
            .ok_or_else(|| "Selected PDF pages are outside the document.".to_string())
    }).transpose()?;
    if let Some(requested) = &requested {
        if requested.iter().any(|page| !pages.contains_key(page)) {
            return Err("Selected PDF pages are outside the document.".to_string());
        }
    }
    let mut total_pixels = 0u64;
    let mut selected = Vec::new();
    for (number, page_id) in pages {
        if requested.as_ref().is_some_and(|requested| !requested.contains(&number)) { continue; }
        let (left, bottom, right, top) = page_bounds(&document, page_id)?;
        let width = right - left;
        let height = top - bottom;
        let pixels = rendered_pixels(width, height)?;
        if pixels > OCR_MAX_SINGLE_PIXELS {
            return Err("Selected PDF page exceeds the 40 million pixel OCR limit.".to_string());
        }
        total_pixels = total_pixels.checked_add(pixels).ok_or_else(|| "Selected PDF pages exceed the OCR pixel limit.".to_string())?;
        if total_pixels > OCR_MAX_TOTAL_PIXELS {
            return Err("Selected PDF pages exceed the 120 million pixel OCR limit.".to_string());
        }
        selected.push(OcrPage { number });
    }
    if selected.is_empty() { Err("Select at least one PDF page for OCR.".to_string()) } else { Ok(selected) }
}

fn rendered_pixels(width: f32, height: f32) -> Result<u64, String> {
    let width = (width / 72.0 * OCR_RENDER_DPI as f32).ceil();
    let height = (height / 72.0 * OCR_RENDER_DPI as f32).ceil();
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("Selected PDF page has invalid OCR dimensions.".to_string());
    }
    (width as u64).checked_mul(height as u64).ok_or_else(|| "Selected PDF pages exceed the OCR pixel limit.".to_string())
}

struct OcrWorkspace(PathBuf);

impl OcrWorkspace {
    fn new() -> Result<Self, String> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        for attempt in 0..100 {
            let path = std::env::temp_dir().join(format!("toolbox_ocr_{}_{}_{}", std::process::id(), timestamp, attempt));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
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

fn run_ocr(engine: &Path, renderer: &Path, input: &Path, pages: &[OcrPage]) -> Result<Vec<u8>, String> {
    run_ocr_until(engine, renderer, input, pages, Instant::now() + OCR_TIMEOUT)
}

fn run_ocr_until(engine: &Path, renderer: &Path, input: &Path, pages: &[OcrPage], deadline: Instant) -> Result<Vec<u8>, String> {
    let workspace = OcrWorkspace::new()?;
    let result = (|| {
        let mut text = Vec::new();
        for page in pages {
            let image = render_ocr_page(renderer, input, &workspace, *page, deadline)?;
            let result = run_command({
                let mut command = Command::new(engine);
                command.arg(&image).arg("stdout");
                command
            }, deadline)?;
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

fn render_ocr_page(renderer: &Path, input: &Path, workspace: &OcrWorkspace, page: OcrPage, deadline: Instant) -> Result<PathBuf, String> {
    let prefix = workspace.page_prefix(page.number);
    let output = prefix.with_extension("png");
    let result = run_command({
        let mut command = Command::new(renderer);
        command.arg("-png").arg("-r").arg(OCR_RENDER_DPI.to_string()).arg("-f").arg(page.number.to_string()).arg("-l").arg(page.number.to_string()).arg("-singlefile").arg(input).arg(&prefix);
        command
    }, deadline)?;
    if result.status.success() && output.is_file() {
        let size = std::fs::metadata(&output).map_err(|error| format!("Could not inspect rendered OCR page: {error}"))?.len();
        if size > OCR_MAX_RENDERED_BYTES {
            return Err("Rendered OCR page exceeds the 50 MiB file-size limit.".to_string());
        }
        Ok(output)
    } else {
        Err(stderr(result, "PDF renderer failed to render the selected OCR page."))
    }
}

fn read_command_output<R: Read>(mut reader: R, stream: &'static str) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = reader.read(&mut buffer).map_err(|error| format!("Could not read OCR helper {stream}: {error}"))?;
        if count == 0 { return Ok(output); }
        if output.len().saturating_add(count) > OCR_MAX_OUTPUT_BYTES {
            return Err(format!("OCR helper {stream} output exceeds the 10 MiB limit."));
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

fn spawn_output_reader<R: Read + Send + 'static>(reader: R, stream: &'static str, failed: Arc<AtomicBool>) -> JoinHandle<Result<Vec<u8>, String>> {
    thread::spawn(move || {
        let result = read_command_output(reader, stream);
        if result.is_err() { failed.store(true, Ordering::Release); }
        result
    })
}

fn join_output_reader(handle: JoinHandle<Result<Vec<u8>, String>>, stream: &'static str) -> Result<Vec<u8>, String> {
    handle.join().map_err(|_| format!("OCR helper {stream} reader stopped unexpectedly."))?
}

fn run_command(mut command: Command, deadline: Instant) -> Result<std::process::Output, String> {
    if Instant::now() >= deadline { return Err("OCR helper timed out after 120 seconds.".to_string()); }
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|error| format!("Could not run OCR helper: {error}"))?;
    let stdout = child.stdout.take().ok_or("OCR helper stdout was not captured.")?;
    let stderr = child.stderr.take().ok_or("OCR helper stderr was not captured.")?;
    let failed = Arc::new(AtomicBool::new(false));
    let stdout_reader = spawn_output_reader(stdout, "stdout", Arc::clone(&failed));
    let stderr_reader = spawn_output_reader(stderr, "stderr", Arc::clone(&failed));
    let mut timed_out = false;
    let status = loop {
        match child.try_wait().map_err(|error| format!("Could not read OCR helper status: {error}"))? {
            Some(status) => break status,
            None if failed.load(Ordering::Acquire) => {
                let _ = child.kill();
                break child.wait().map_err(|error| format!("Could not stop OCR helper: {error}"))?;
            }
            None if Instant::now() >= deadline => {
                timed_out = true;
                let _ = child.kill();
                break child.wait().map_err(|error| format!("Could not stop OCR helper: {error}"))?;
            }
            None => thread::sleep(Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now()))),
        }
    };
    let stdout = join_output_reader(stdout_reader, "stdout")?;
    let stderr = join_output_reader(stderr_reader, "stderr")?;
    if timed_out { return Err("OCR helper timed out after 120 seconds.".to_string()); }
    Ok(std::process::Output { status, stdout, stderr })
}

fn find_pdf_renderer() -> Result<PathBuf, String> {
    find_pdf_renderer_with_override(std::env::var_os("TOOLBOX_PDFTOPPM_PATH").map(PathBuf::from), || {
        let root = crate::kit::resources::application_resource_root().unwrap_or_default();
        let executable = if cfg!(windows) { "pdftoppm.exe" } else { "pdftoppm" };
        [root.join("pdf-bin").join(executable), root.join("resources").join(executable), root.join(executable)]
            .into_iter()
            .find(|path| path.is_file())
            .or_else(|| Command::new(executable).arg("-h").output().ok().filter(|result| result.status.success()).map(|_| PathBuf::from(executable)))
            .ok_or_else(|| "PDF rasterizer is unavailable. Bundle pdftoppm or set TOOLBOX_PDFTOPPM_PATH.".to_string())
    })
}

fn find_pdf_renderer_with_override<F>(override_path: Option<PathBuf>, fallback: F) -> Result<PathBuf, String>
where
    F: FnOnce() -> Result<PathBuf, String>,
{
    if let Some(path) = override_path.filter(|path| !path.as_os_str().is_empty()) {
        if path.is_file() { return Ok(path); }
        return Err("TOOLBOX_PDFTOPPM_PATH does not point to a file.".to_string());
    }
    fallback()
}

fn normalize_ocr_text(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "OCR helper produced invalid UTF-8 text.".to_string())?
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(text)
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
    use super::{normalize_ocr_text, ocr_pdf, run_command, validate_cutout, validate_face_blur, VisionRequest};
    use image::{Rgba, RgbaImage};
    use lopdf::{dictionary, Document, Object, Stream};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{Duration, Instant};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;


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
        let _guard = crate::kit::PROCESS_ENV_LOCK.lock().unwrap();
        let input = path("selected-pages-scope.pdf");
        make_pdf(&input, 2);

        assert_eq!(super::selected_ocr_pages(&input, Some(&[1, 0])).unwrap().iter().map(|page| page.number).collect::<Vec<_>>(), vec![1, 2]);
        assert!(super::selected_ocr_pages(&input, Some(&[2])).is_err());

        let _ = fs::remove_file(input);
    }

    #[test]
    fn selected_ocr_pages_reject_excessive_render_pixels_before_starting_helpers() {
        let _guard = crate::kit::PROCESS_ENV_LOCK.lock().unwrap();
        let input = path("oversized-page.pdf");
        make_pdf(&input, 1);
        let mut document = Document::load(&input).unwrap();
        let page = document.get_pages().values().next().copied().unwrap();
        let root = document.get_dictionary(page).unwrap().get(b"Parent").unwrap().as_reference().unwrap();
        document.get_dictionary_mut(page).unwrap().remove(b"MediaBox");
        document.get_dictionary_mut(root).unwrap().set("MediaBox", vec![0.into(), 0.into(), 14400.into(), 14400.into()]);
        document.save(&input).unwrap();

        let error = super::selected_ocr_pages(&input, None).unwrap_err();
        assert!(error.contains("pixel"), "{error}");
        let _ = fs::remove_file(input);
    }

    #[cfg(unix)]
    #[test]
    fn oversized_rendered_page_is_rejected_before_ocr_runs() {
        let renderer = path("oversized-renderer.sh");
        let engine = path("oversized-engine.sh");
        let input = path("oversized-render.pdf");
        make_pdf(&input, 1);
        fs::write(&renderer, format!("#!/bin/sh\ndd if=/dev/zero of=\"${{10}}.png\" bs=1 count=0 seek={} 2>/dev/null\n", super::OCR_MAX_RENDERED_BYTES + 1)).unwrap();
        fs::write(&engine, "#!/bin/sh\nprintf 'ocr should not run\\n'\n").unwrap();
        for helper in [&renderer, &engine] {
            let mut permissions = fs::metadata(helper).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(helper, permissions).unwrap();
        }
        let workspace = super::OcrWorkspace::new().unwrap();
        let result = super::render_ocr_page(&renderer, &input, &workspace, super::OcrPage { number: 1 }, Instant::now() + Duration::from_secs(5));
        let error = result.unwrap_err();
        assert!(error.contains("file-size limit"), "{error}");

        drop(workspace);
        let _ = fs::remove_file(renderer);
        let _ = fs::remove_file(engine);
        let _ = fs::remove_file(input);
    }

    #[cfg(unix)]
    #[test]
    fn selected_ocr_page_cannot_process_an_excluded_page() {
        let _guard = crate::kit::PROCESS_ENV_LOCK.lock().unwrap();
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
        let _guard = crate::kit::PROCESS_ENV_LOCK.lock().unwrap();
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
    #[test]
    fn ocr_workspace_is_private_and_removed_on_drop() {
        let _guard = crate::kit::PROCESS_ENV_LOCK.lock().unwrap();
        let workspace = super::OcrWorkspace::new().unwrap();
        let path = workspace.0.clone();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o700);
        fs::write(path.join("secret.png"), b"private OCR input").unwrap();
        drop(workspace);
        assert!(!path.exists());
    }

    #[test]
    fn empty_pdf_renderer_override_uses_path_fallback() {
        let result = super::find_pdf_renderer_with_override(Some(PathBuf::new()), || Ok(PathBuf::from("pdftoppm")));
        assert_eq!(result.unwrap(), PathBuf::from("pdftoppm"));
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
        assert_eq!(normalize_ocr_text(b"  first   line\r\n\n second line  \n").unwrap(), "first line\nsecond line");
    }

    #[test]
    fn rejects_invalid_utf8_ocr_output() {
        assert!(normalize_ocr_text(b"valid\xfftext").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn multi_page_ocr_uses_one_request_deadline() {
        let renderer = path("deadline-renderer.sh");
        let engine = path("deadline-engine.sh");
        let input = path("deadline.pdf");
        make_pdf(&input, 2);
        fs::write(&renderer, "#!/bin/sh\nsleep 0.18\ntouch \"${10}.png\"\n").unwrap();
        fs::write(&engine, "#!/bin/sh\nsleep 0.18\nprintf 'page\\n'\n").unwrap();
        for helper in [&renderer, &engine] {
            let mut permissions = fs::metadata(helper).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(helper, permissions).unwrap();
        }

        let started = Instant::now();
        let result = super::run_ocr_until(&engine, &renderer, &input, &[super::OcrPage { number: 1 }, super::OcrPage { number: 2 }], Instant::now() + Duration::from_millis(300));

        assert_eq!(result.unwrap_err(), "OCR helper timed out after 120 seconds.");
        assert!(started.elapsed() < Duration::from_secs(1), "request deadline was not shared: {:?}", started.elapsed());
        let _ = fs::remove_file(renderer);
        let _ = fs::remove_file(engine);
        let _ = fs::remove_file(input);
    }

    #[cfg(unix)]
    #[test]
    fn run_command_drains_large_helper_output_without_timeout() {
        let started = Instant::now();
        let mut command = Command::new("sh");
        command.args(["-c", "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2"]);
        let result = run_command(command, Instant::now() + Duration::from_secs(5)).expect("large helper output should be drained");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(result.stdout.len(), 131072);
        assert_eq!(result.stderr.len(), 131072);
    }
}
