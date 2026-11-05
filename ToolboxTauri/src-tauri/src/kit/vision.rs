use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::io::Read;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use image::GenericImageView;
use lopdf::Document;

use crate::kit::common::limits::{self, StoppableHelper};
use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::{OutcomePointer, PackNeed, ToolError};
use crate::kit::pdf::metadata::page_bounds;
use crate::kit::resources;
use crate::packs::uninstall::{resolve_pack_resource, PackHolds, RequiredPack};
use crate::packs::Catalog;

const OCR_MAX_INPUT_BYTES: u64 = 100 * 1024 * 1024;
const OCR_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;
const OCR_RENDER_DPI: u32 = 200;
const OCR_MAX_SINGLE_PIXELS: u64 = 40_000_000;
const OCR_MAX_TOTAL_PIXELS: u64 = 120_000_000;
const OCR_MAX_RENDERED_BYTES: u64 = 50 * 1024 * 1024;
const OCR_TIMEOUT: Duration = Duration::from_secs(120);
/// The language OCR reads with the data bundled in the app. Every other language needs a pack.
const ENGLISH: &str = "eng";
/// Development builds read `<code>.traineddata` from this folder when the language's pack is not installed.
const OCR_LANGUAGE_DIR_ENV: &str = "TOOLBOX_OCR_LANGUAGE_DIR";

#[derive(Debug, Clone, Copy)]
struct OcrPage {
    number: u32,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrRequest {
    pub paths: Vec<PathBuf>,
    #[serde(default)] pub pages: Option<Vec<usize>>,
    pub output_location: OutputLocation,
    /// The Tesseract code of the language to read, as in `hin`. English is built in.
    #[serde(default = "english")] pub language: String,
    /// Files read in another language than `language`.
    #[serde(default)] pub language_overrides: HashMap<PathBuf, String>,
}

fn english() -> String { ENGLISH.to_string() }

impl OcrRequest {
    pub fn language_for(&self, path: &Path) -> &str {
        self.language_overrides.get(path).unwrap_or(&self.language)
    }
}

/// Where an OCR run finds language packs. The packs a run reads stay held until the run ends, so
/// they cannot be uninstalled under it.
pub struct OcrPacks {
    source: Result<PackSource, String>,
    holds: Mutex<PackHolds>,
}

struct PackSource {
    app_data: PathBuf,
    app_version: String,
    catalog: Catalog,
}

impl OcrPacks {
    pub fn new(app_data: Result<PathBuf, String>, app_version: impl Into<String>, catalog: Result<Catalog, String>) -> Self {
        let app_version = app_version.into();
        let source = app_data.and_then(|app_data| catalog.map(|catalog| PackSource { app_data, app_version, catalog }));
        Self { source, holds: Mutex::default() }
    }
}

pub struct OcrRun {
    pub request: OcrRequest,
    pub packs: OcrPacks,
}

/// The language data for `code`: English from the app, any other language from its pack. A pack
/// that is not installed, is damaged or does not fit this app fails with a pointer to it.
fn ocr_language(packs: &OcrPacks, code: &str) -> Result<PathBuf, ToolError> {
    if code == ENGLISH {
        return find_resource("engTraineddata", "TOOLBOX_TESSDATA_PATH", &ocr_resource_root()).map_err(ToolError::unavailable);
    }
    let source = packs.source.as_ref().map_err(|error| ToolError::unavailable(error.clone()))?;
    let (pack, language) = source.catalog.packs.iter()
        .find_map(|pack| pack.ocr_language.as_ref().filter(|language| language.code == code).map(|language| (pack, language)))
        .ok_or_else(|| ToolError::invalid_input(format!("Toolbox can't read text in language {code:?}.")))?;
    let file_name = format!("{code}.traineddata");
    let development_override = cfg!(debug_assertions).then(|| std::env::var_os(OCR_LANGUAGE_DIR_ENV)).flatten()
        .map(|folder| PathBuf::from(folder).join(&file_name));
    let mut holds = packs.holds.lock().unwrap_or_else(PoisonError::into_inner);
    let required = RequiredPack { id: &pack.id, name: &language.name };
    let resolved = resolve_pack_resource(&format!("tessdata/{file_name}"), &source.app_data, required, &source.app_version, Some(ocr_resource_root()), development_override, &mut holds)
        .map_err(|mut error| {
            if matches!(error.pointer, Some(OutcomePointer::Pack { reason: PackNeed::NotInstalled, .. })) {
                error.message = format!("Needs the {} text recognition pack, which isn't installed. Nothing was saved.", language.name);
            }
            error
        })?;
    // Tesseract reads `<code>.traineddata` from the folder it is given.
    if resolved.path.file_name() != Some(OsStr::new(&file_name)) {
        return Err(ToolError::unavailable(format!("The {} pack has no {file_name}.", language.name)));
    }
    Ok(resolved.path)
}

pub fn ocr_pdf(run: &OcrRun, input: PathBuf) -> JobOutcome {
    let request = &run.request;
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
    let language = request.language_for(&input);
    let tessdata = match ocr_language(&run.packs, language) {
        Ok(tessdata) => tessdata,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let engine = match find_engine("tesseract", "TOOLBOX_TESSERACT_PATH", "tesseract") {
        Ok(engine) => engine,
        Err(error) => return unavailable(input, error),
    };
    let renderer = match find_pdf_renderer() {
        Ok(renderer) => renderer,
        Err(error) => return unavailable(input, error),
    };
    let result = run_ocr(&engine, &tessdata, language, &renderer, &input, &pages);
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

fn run_ocr(engine: &Path, tessdata: &Path, language: &str, renderer: &Path, input: &Path, pages: &[OcrPage]) -> Result<Vec<u8>, String> {
    run_ocr_until_with_tessdata(engine, Some((tessdata, language)), renderer, input, pages, Instant::now() + OCR_TIMEOUT)
}

fn run_ocr_until(engine: &Path, renderer: &Path, input: &Path, pages: &[OcrPage], deadline: Instant) -> Result<Vec<u8>, String> {
    run_ocr_until_with_tessdata(engine, None, renderer, input, pages, deadline)
}

fn run_ocr_until_with_tessdata(engine: &Path, tessdata: Option<(&Path, &str)>, renderer: &Path, input: &Path, pages: &[OcrPage], deadline: Instant) -> Result<Vec<u8>, String> {
    let workspace = OcrWorkspace::new()?;
    let tessdata = tessdata.map(|(path, language)| path.parent().map(|dir| (dir, language)).ok_or_else(|| "OCR language data has no parent directory.".to_string())).transpose()?;
    let result = (|| {
        let mut text = Vec::new();
        for page in pages {
            let image = render_ocr_page(renderer, input, &workspace, *page, deadline)?;
            let result = run_command({
                let mut command = Command::new(engine);
                command.arg(&image).arg("stdout");
                if let Some((tessdata_dir, language)) = tessdata {
                    command.arg("--tessdata-dir").arg(tessdata_dir).arg("-l").arg(language).arg("--psm").arg("3");
                }
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
            None if Instant::now() >= deadline || limits::job_stopped() => {
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
    run_image_adapter(request, input, "face-blur", "faceBlur", "faceBlurModel", "TOOLBOX_FACE_BLUR_PATH", "TOOLBOX_FACE_BLUR_MODEL_PATH", "toolbox-face-blur", "-blurred", "Blurred faces", ImageAdapterValidation::FaceBlur)
}

pub fn remove_background(request: &VisionRequest, input: PathBuf) -> JobOutcome {
    run_image_adapter(request, input, "background-removal", "backgroundRemoval", "backgroundRemovalModel", "TOOLBOX_BACKGROUND_REMOVAL_PATH", "TOOLBOX_BACKGROUND_REMOVAL_MODEL_PATH", "toolbox-background-removal", "-cutout", "Background removed", ImageAdapterValidation::Cutout)
}

enum ImageAdapterValidation { FaceBlur, Cutout }

fn run_image_adapter(request: &VisionRequest, input: PathBuf, mode: &str, resource_name: &str, model_name: &str, variable: &str, model_variable: &str, command: &str, suffix: &str, detail: &str, validation: ImageAdapterValidation) -> JobOutcome {
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, suffix, "png") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve vision output: {error}")),
    };
    let engine = match find_engine(resource_name, variable, command) {
        Ok(engine) => engine,
        Err(error) => return unavailable(input, error),
    };
    let model = match find_model(model_name, model_variable) {
        Ok(model) => model,
        Err(error) => return unavailable(input, error),
    };
    match Command::new(engine).arg("--mode").arg(mode).arg("--input").arg(&input).arg("--output").arg(output.path()).arg("--model").arg(model).output_until_stopped() {
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
    let root = if name == "tesseract" { ocr_resource_root() } else { vision_resource_root() };
    resources::resolve(name, &root, variable, command).map(|resource| resource.path)
}

fn find_model(name: &str, variable: &str) -> Result<PathBuf, String> {
    let root = vision_resource_root();
    resources::resolve_bundled_or_override(name, &root, variable)
        .and_then(|resource| resource.map(|resource| resource.path).ok_or_else(|| format!("Resource {name} is unavailable.")))
}

fn find_resource(name: &str, variable: &str, root: &std::path::Path) -> Result<PathBuf, String> {
    resources::resolve_bundled_or_override(name, root, variable)
        .and_then(|resource| resource.map(|resource| resource.path).ok_or_else(|| format!("Resource {name} is unavailable.")))
}

fn vision_resource_root() -> std::path::PathBuf {
    let root = resources::application_resource_root().unwrap_or_default();
    let direct = root.join("vision");
    if direct.is_dir() || !root.join("resources").join("vision").is_dir() { direct } else { root.join("resources").join("vision") }
}

fn ocr_resource_root() -> std::path::PathBuf {
    let root = resources::application_resource_root().unwrap_or_default();
    let direct = root.join("ocr");
    if direct.is_dir() || !root.join("resources").join("ocr").is_dir() { direct } else { root.join("resources").join("ocr") }
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

fn success(input_path: PathBuf, output: PathBuf, detail: &str) -> JobOutcome { JobOutcome::succeeded(input_path, vec![output], detail.to_string()) }
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::processing(error)) }
fn unavailable(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::unavailable(error)) }
fn stderr(result: std::process::Output, fallback: &str) -> String { String::from_utf8_lossy(&result.stderr).trim().lines().last().filter(|line| !line.is_empty()).unwrap_or(fallback).to_string() }

#[cfg(test)]
mod tests {
    use super::{normalize_ocr_text, ocr_pdf, run_command, validate_cutout, validate_face_blur, OcrPacks, OcrRequest, OcrRun};
    use crate::kit::contracts::{ErrorKind, OutcomePointer, PackNeed};
    use image::{Rgba, RgbaImage};
    use lopdf::{dictionary, Document, Object, Stream};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{Duration, Instant};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;


    fn path(name: &str) -> PathBuf { std::env::temp_dir().join(format!("toolbox_vision_{}_{}", std::process::id(), name)) }

    #[cfg(unix)]
    /// A run that reads English only, with no packs to find.
    fn english_run(input: &Path, pages: Option<Vec<usize>>, output_root: &Path) -> OcrRun {
        OcrRun {
            request: OcrRequest { paths: vec![input.to_path_buf()], pages, output_location: crate::kit::common::OutputLocation::CustomFolder(output_root.to_path_buf()), language: "eng".into(), language_overrides: Default::default() },
            packs: OcrPacks::new(Err("No packs in this test.".into()), "1.0.14", Err("No catalog in this test.".into())),
        }
    }

    #[cfg(unix)]
    fn executable(path: &Path, script: &str) {
        fs::write(path, script).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(unix)]
    /// Sets environment variables for one test and puts the old values back when dropped.
    struct Env(Vec<(&'static str, Option<std::ffi::OsString>)>);

    #[cfg(unix)]
    impl Env {
        fn set(values: &[(&'static str, Option<&Path>)]) -> Self {
            let old = values.iter().map(|(name, _)| (*name, std::env::var_os(name))).collect();
            for (name, value) in values {
                match value { Some(value) => std::env::set_var(name, value), None => std::env::remove_var(name) }
            }
            Self(old)
        }
    }

    #[cfg(unix)]
    impl Drop for Env {
        fn drop(&mut self) {
            for (name, value) in &self.0 {
                match value { Some(value) => std::env::set_var(name, value), None => std::env::remove_var(name) }
            }
        }
    }

    #[cfg(unix)]
    struct MixedBatch {
        root: PathBuf,
        english: PathBuf,
        hindi: PathBuf,
        output: PathBuf,
    }

    #[cfg(unix)]
    impl MixedBatch {
        /// One English and one Hindi PDF, an engine that prints the language it was asked for and
        /// the language file it read, and a renderer that needs no PDF tools.
        fn new(name: &str) -> Self {
            let root = crate::packs::install::tests::sandbox(&format!("ocr-{name}"));
            let batch = Self { english: root.join("english.pdf"), hindi: root.join("hindi.pdf"), output: root.join("output"), root };
            make_pdf(&batch.english, 1);
            make_pdf(&batch.hindi, 1);
            fs::create_dir_all(&batch.output).unwrap();
            fs::create_dir_all(batch.root.join("eng")).unwrap();
            fs::write(batch.root.join("eng/eng.traineddata"), "english data").unwrap();
            executable(&batch.root.join("engine.sh"), "#!/bin/sh\ndata=''\nlanguage=''\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    --tessdata-dir) data=\"$2\"; shift ;;\n    -l) language=\"$2\"; shift ;;\n  esac\n  shift\ndone\nprintf '%s: ' \"$language\"\ncat \"$data/$language.traineddata\"\nprintf '\\n'\n");
            executable(&batch.root.join("renderer.sh"), "#!/bin/sh\n: > \"${10}.png\"\n");
            batch
        }

        /// Runs the English file in English and the Hindi file in Hindi as one batch, and checks
        /// the originals are untouched. The outcomes come back in input order.
        fn run(&self, app_data: &Path, app_version: &str, language_dir: Option<&Path>) -> Vec<crate::kit::common::JobOutcome> {
            let originals = [fs::read(&self.english).unwrap(), fs::read(&self.hindi).unwrap()];
            let outcomes = {
                let _guard = crate::kit::lock_process_env();
                let _env = Env::set(&[
                    ("TOOLBOX_TESSERACT_PATH", Some(&self.root.join("engine.sh"))),
                    ("TOOLBOX_PDFTOPPM_PATH", Some(&self.root.join("renderer.sh"))),
                    ("TOOLBOX_TESSDATA_PATH", Some(&self.root.join("eng/eng.traineddata"))),
                    (super::OCR_LANGUAGE_DIR_ENV, language_dir),
                ]);
                let request = OcrRequest {
                    paths: vec![self.english.clone(), self.hindi.clone()],
                    pages: None,
                    output_location: crate::kit::common::OutputLocation::CustomFolder(self.output.clone()),
                    language: "eng".into(),
                    language_overrides: [(self.hindi.clone(), "hin".to_string())].into_iter().collect(),
                };
                let packs = OcrPacks::new(Ok(app_data.to_path_buf()), app_version, Ok(crate::packs::install::tests::Fixture::new().catalog()));
                // One run for the whole batch, each file through the job the batch runner calls. The
                // runner itself refuses ocr_pdf while the tool is unavailable in this build.
                let run = OcrRun { request, packs };
                run.request.paths.iter().map(|input| ocr_pdf(&run, input.clone())).collect::<Vec<_>>()
            };
            assert_eq!([fs::read(&self.english).unwrap(), fs::read(&self.hindi).unwrap()], originals, "originals are never modified");
            assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), [self.english.clone(), self.hindi.clone()]);
            outcomes
        }

        fn assert_english_read(&self, outcome: &crate::kit::common::JobOutcome) {
            assert!(outcome.error().is_none(), "{outcome:?}");
            let text = fs::read_to_string(&outcome.output_paths()[0]).unwrap();
            assert_eq!(text, "eng: english data");
        }

        /// The Hindi file failed with the pointer and saved nothing.
        fn assert_needs_pack(&self, outcome: &crate::kit::common::JobOutcome, reason: PackNeed, message: &str) {
            let error = outcome.error().unwrap_or_else(|| panic!("{outcome:?}"));
            assert_eq!(error.kind, ErrorKind::Unavailable);
            assert_eq!(error.pointer, Some(OutcomePointer::Pack { pack_id: "ocr-hindi".into(), name: "Hindi".into(), reason }));
            assert_eq!(error.message, message);
            assert!(outcome.output_paths().is_empty());
            let saved: Vec<_> = fs::read_dir(&self.output).unwrap().flatten().map(|entry| entry.file_name()).collect();
            assert!(saved.iter().all(|name| name.to_string_lossy().starts_with("english")), "nothing was saved for the Hindi file: {saved:?}");
        }
    }

    #[cfg(unix)]
    /// Installs the fixture Hindi pack into a fresh app-data folder.
    fn install_hindi(root: &Path) -> PathBuf {
        use crate::packs::install::{install, tests::{Fixture, TestKey}, InstallContext, PackLimits};
        let (fixture, key) = (Fixture::new(), TestKey::new(1));
        let file = root.join("hindi.toolboxpack");
        fs::write(&file, fixture.file(&key)).unwrap();
        let app_data = root.join("app-data");
        let target = crate::packs::current_target();
        install(&file, &InstallContext { catalog: &fixture.catalog(), app_data: &app_data, target: &target, app_version: "1.0.14", public_key: &key.public_key(), limits: PackLimits::DEFAULT, cancel: None, changed: None }).unwrap();
        app_data
    }

    #[cfg(unix)]
    #[test]
    fn a_language_whose_pack_is_not_installed_fails_with_a_pointer_while_english_in_the_same_batch_succeeds() {
        let batch = MixedBatch::new("not-installed");
        let outcomes = batch.run(&batch.root.join("app-data"), "1.0.14", None);
        batch.assert_english_read(&outcomes[0]);
        batch.assert_needs_pack(&outcomes[1], PackNeed::NotInstalled, "Needs the Hindi text recognition pack, which isn't installed. Nothing was saved.");
    }

    #[cfg(unix)]
    #[test]
    fn a_development_override_stands_in_for_the_language_pack_and_its_language_is_read() {
        let batch = MixedBatch::new("override");
        let language_dir = batch.root.join("languages");
        fs::create_dir_all(&language_dir).unwrap();
        fs::write(language_dir.join("hin.traineddata"), "fixture hindi").unwrap();
        let outcomes = batch.run(&batch.root.join("app-data"), "1.0.14", Some(&language_dir));
        batch.assert_english_read(&outcomes[0]);
        assert!(outcomes[1].error().is_none(), "{:?}", outcomes[1]);
        assert_eq!(fs::read_to_string(&outcomes[1].output_paths()[0]).unwrap(), "hin: fixture hindi");
    }

    #[cfg(unix)]
    #[test]
    fn an_installed_language_pack_is_read_and_a_damaged_or_incompatible_one_names_its_own_reason() {
        let batch = MixedBatch::new("installed");
        let app_data = install_hindi(&batch.root);
        let outcomes = batch.run(&app_data, "1.0.14", None);
        assert_eq!(fs::read_to_string(&outcomes[1].output_paths()[0]).unwrap(), format!("hin: {}", "hindi model".repeat(100)));

        // The fixture pack fits apps from 1.0.0. A development override does not stand in for it.
        let language_dir = batch.root.join("languages");
        fs::create_dir_all(&language_dir).unwrap();
        fs::write(language_dir.join("hin.traineddata"), "fixture hindi").unwrap();
        let incompatible = MixedBatch::new("incompatible");
        let outcomes = incompatible.run(&install_hindi(&incompatible.root), "0.9.0", Some(&language_dir));
        incompatible.assert_english_read(&outcomes[0]);
        incompatible.assert_needs_pack(&outcomes[1], PackNeed::Incompatible, "Needs a newer Hindi pack. The installed one doesn't work with this version of Toolbox. Nothing was saved.");

        let damaged = MixedBatch::new("damaged");
        let app_data = install_hindi(&damaged.root);
        let model = crate::packs::installed_pack_root(&app_data, "ocr-hindi").unwrap().join("tessdata/hin.traineddata");
        fs::write(&model, "changed after install").unwrap();
        let outcomes = damaged.run(&app_data, "1.0.14", Some(&language_dir));
        damaged.assert_english_read(&outcomes[0]);
        damaged.assert_needs_pack(&outcomes[1], PackNeed::Damaged, "The Hindi pack is damaged, so it was not used. Nothing was saved.");
    }

    #[test]
    fn a_pack_pointer_names_the_catalog_pack_for_its_language_and_a_language_no_pack_reads_has_none() {
        // The development override is read from the environment.
        let _guard = crate::kit::lock_process_env();
        let root = std::env::temp_dir().join(format!("toolbox-ocr-pointer-{}", std::process::id()));
        let catalogs = [
            ("built-in", include_str!("../../../shared/pack-catalog.json").to_string()),
            ("fixture", fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/pack-catalog.json")).unwrap()),
        ];
        for (name, text) in catalogs {
            let catalog = crate::packs::parse_catalog(&text).unwrap();
            let packs = OcrPacks::new(Ok(root.join(name)), "1.0.14", Ok(catalog.clone()));
            for pack in &catalog.packs {
                let Some(language) = &pack.ocr_language else { continue };
                let error = super::ocr_language(&packs, &language.code).unwrap_err();
                assert_eq!(error.kind, ErrorKind::Unavailable, "{name} {}", pack.id);
                assert_eq!(error.pointer, Some(OutcomePointer::Pack { pack_id: pack.id.clone(), name: language.name.clone(), reason: PackNeed::NotInstalled }), "{name} {}", pack.id);
            }
            let error = super::ocr_language(&packs, "xyz").unwrap_err();
            assert_eq!(error.kind, ErrorKind::InvalidInput);
            assert_eq!(error.pointer, None, "a language no {name} pack reads points nowhere");
        }
        assert!(!root.exists(), "looking for a pack creates nothing");
    }

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
        let _guard = crate::kit::lock_process_env();
        let input = path("selected-pages-scope.pdf");
        make_pdf(&input, 2);

        assert_eq!(super::selected_ocr_pages(&input, Some(&[1, 0])).unwrap().iter().map(|page| page.number).collect::<Vec<_>>(), vec![1, 2]);
        assert!(super::selected_ocr_pages(&input, Some(&[2])).is_err());

        let _ = fs::remove_file(input);
    }

    #[test]
    fn selected_ocr_pages_reject_excessive_render_pixels_before_starting_helpers() {
        let _guard = crate::kit::lock_process_env();
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
        // Holds the lock the workspace-cleanup tests count OCR workspaces under.
        let _guard = crate::kit::lock_process_env();
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
        let _guard = crate::kit::lock_process_env();
        let input = path("selected-pages.pdf");
        let output_root = path("selected-pages-output");
        make_pdf(&input, 2);
        let original = fs::read(&input).unwrap();
        fs::create_dir_all(&output_root).unwrap();
        let engine = fake_ocr_engine(false);
        let tessdata = path("selected-eng.traineddata");
        fs::write(&tessdata, b"fake tessdata").unwrap();
        let old_engine = std::env::var_os("TOOLBOX_TESSERACT_PATH");
        let old_tessdata = std::env::var_os("TOOLBOX_TESSDATA_PATH");
        std::env::set_var("TOOLBOX_TESSERACT_PATH", &engine);
        std::env::set_var("TOOLBOX_TESSDATA_PATH", &tessdata);

        let outcome = ocr_pdf(&english_run(&input, Some(vec![0]), &output_root), input.clone());
        let text = outcome.output_paths().first().and_then(|path| fs::read_to_string(path).ok());
        let output_path = outcome.output_paths().first().cloned();
        let input_after = fs::read(&input).unwrap();

        match old_engine {
            Some(value) => std::env::set_var("TOOLBOX_TESSERACT_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSERACT_PATH"),
        }
        match old_tessdata {
            Some(value) => std::env::set_var("TOOLBOX_TESSDATA_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSDATA_PATH"),
        }
        let _ = fs::remove_file(&engine);
        let _ = fs::remove_file(&tessdata);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_dir_all(&output_root);

        assert!(outcome.error().is_none(), "selected OCR failed: {:?}", outcome.error());
        assert_eq!(text.as_deref(), Some("selected page one"));
        assert_eq!(output_path.as_deref().and_then(Path::parent), Some(output_root.as_path()));
        assert_eq!(input_after, original);
    }

    #[cfg(unix)]
    #[test]
    fn failed_selected_ocr_cleans_private_workspace_and_reserved_output() {
        let _guard = crate::kit::lock_process_env();
        let input = path("failed-selected-pages.pdf");
        let output_root = path("failed-selected-pages-output");
        make_pdf(&input, 2);
        let original = fs::read(&input).unwrap();
        fs::create_dir_all(&output_root).unwrap();
        let engine = fake_ocr_engine(true);
        let tessdata = path("failed-eng.traineddata");
        fs::write(&tessdata, b"fake tessdata").unwrap();
        let before_workspaces = ocr_workspaces();
        let old_engine = std::env::var_os("TOOLBOX_TESSERACT_PATH");
        let old_tessdata = std::env::var_os("TOOLBOX_TESSDATA_PATH");
        std::env::set_var("TOOLBOX_TESSERACT_PATH", &engine);
        std::env::set_var("TOOLBOX_TESSDATA_PATH", &tessdata);

        let outcome = ocr_pdf(&english_run(&input, Some(vec![1]), &output_root), input.clone());
        let after_workspaces = ocr_workspaces();
        let input_after = fs::read(&input).unwrap();
        let output_entries = fs::read_dir(&output_root).unwrap().count();

        match old_engine {
            Some(value) => std::env::set_var("TOOLBOX_TESSERACT_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSERACT_PATH"),
        }
        match old_tessdata {
            Some(value) => std::env::set_var("TOOLBOX_TESSDATA_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSDATA_PATH"),
        }
        let _ = fs::remove_file(&engine);
        let _ = fs::remove_file(&tessdata);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_dir_all(&output_root);

        assert!(outcome.error().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::Processing)));
        assert!(outcome.output_paths().is_empty());
        assert_eq!(before_workspaces, after_workspaces);
        assert_eq!(original, input_after);
        assert_eq!(output_entries, 0);
    }

    #[cfg(unix)]
    #[test]
    fn ocr_workspace_is_private_and_removed_on_drop() {
        let _guard = crate::kit::lock_process_env();
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
        // Holds the lock the workspace-cleanup tests count OCR workspaces under.
        let _guard = crate::kit::lock_process_env();
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
        // Other tests point PATH elsewhere while they hold this lock, and `sh` is found on PATH.
        let _guard = crate::kit::lock_process_env();
        let started = Instant::now();
        let mut command = Command::new("sh");
        command.args(["-c", "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2"]);
        let result = run_command(command, Instant::now() + Duration::from_secs(5)).expect("large helper output should be drained");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(result.stdout.len(), 131072);
        assert_eq!(result.stderr.len(), 131072);
    }
}
