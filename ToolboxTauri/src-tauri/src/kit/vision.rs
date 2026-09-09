use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use image::GenericImageView;

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
    pub output_location: OutputLocation,
}

pub fn ocr_pdf(request: &VisionRequest, input: PathBuf) -> JobOutcome {
    let output = OutputNaming::get_destination(&input, &request.output_location, "-ocr-text", "txt");
    let input_size = match std::fs::metadata(&input) {
        Ok(metadata) => metadata.len(),
        Err(error) => return failure(input, format!("Could not inspect PDF for OCR: {error}")),
    };
    if input_size > OCR_MAX_INPUT_BYTES { return failure(input, "PDF exceeds the 100 MiB OCR input limit.".to_string()); }
    let engine = match find_engine("tesseract", "TOOLBOX_TESSERACT_PATH", "tesseract") {
        Ok(engine) => engine,
        Err(error) => return unavailable(input, error),
    };
    let tessdata = match find_resource("engTraineddata", "TOOLBOX_TESSDATA_PATH", &ocr_resource_root()) {
        Ok(resource) => resource,
        Err(error) => return unavailable(input, error),
    };
    let renderer = match find_pdf_renderer() {
        Ok(renderer) => renderer,
        Err(error) => return unavailable(input, error),
    };
    let result = run_ocr(&engine, &tessdata, &renderer, &input);
    match result {
        Ok(stdout) => {
            if stdout.len() > OCR_MAX_OUTPUT_BYTES { return failure(input, "OCR output exceeds the 10 MiB text limit.".to_string()); }
            let text = normalize_ocr_text(&stdout);
            if text.is_empty() { return failure(input, "OCR completed but found no readable text.".to_string()); }
            match std::fs::write(&output, text) {
                Ok(_) => success(input, output, "OCR text extracted in page order"),
                Err(error) => failure(input, format!("Could not write OCR output: {error}")),
            }
        }
        Err(error) => failure(input, error),
    }
}

fn run_ocr(engine: &std::path::Path, tessdata: &std::path::Path, renderer: &std::path::Path, input: &std::path::Path) -> Result<Vec<u8>, String> {
    let temporary_root = std::env::temp_dir().join(format!("toolbox_ocr_{}_{}", std::process::id(), unique_suffix()));
    std::fs::create_dir_all(&temporary_root).map_err(|error| format!("Could not create OCR workspace: {error}"))?;
    let prefix = temporary_root.join("page");
    let render_result = run_command({
        let mut command = Command::new(renderer);
        command.arg("-png").arg("-r").arg("200").arg(input).arg(&prefix);
        command
    });
    let result = match render_result {
        Ok(result) if result.status.success() => (|| -> Result<Vec<u8>, String> {
            let mut pages = std::fs::read_dir(&temporary_root)
                .map_err(|error| format!("Could not inspect rendered OCR pages: {error}"))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("png"))
                .collect::<Vec<_>>();
            pages.sort_by_key(|path| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .and_then(|stem| stem.strip_prefix("page-"))
                    .and_then(|page| page.parse::<usize>().ok())
                    .unwrap_or(usize::MAX)
            });
            if pages.is_empty() { Err("PDF renderer produced no pages for OCR.".to_string()) }
            else {
                let tessdata_dir = tessdata.parent().ok_or_else(|| "OCR language data has no parent directory.".to_string())?;
                let mut text = Vec::new();
                for page in pages {
                    let result = run_command({
                        let mut command = Command::new(engine);
                        command.arg(&page).arg("stdout").arg("--tessdata-dir").arg(tessdata_dir).arg("-l").arg("eng").arg("--psm").arg("3");
                        command
                    })?;
                    if !result.status.success() { return Err(stderr(result, "OCR engine failed.")); }
                    if text.len() + result.stdout.len() > OCR_MAX_OUTPUT_BYTES { return Err("OCR output exceeds the 10 MiB text limit.".to_string()); }
                    text.extend_from_slice(&result.stdout);
                    text.push(b'\n');
                }
                Ok(text)
            }
        })(),
        Ok(result) => Err(stderr(result, "PDF renderer failed.")),
        Err(error) => Err(error),
    };
    cleanup_ocr_workspace(temporary_root, result)
}

fn run_command(mut command: Command) -> Result<std::process::Output, String> {
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|error| format!("Could not run OCR helper: {error}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait().map_err(|error| format!("Could not read OCR engine status: {error}"))? {
            Some(_) => return child.wait_with_output().map_err(|error| format!("Could not collect OCR output: {error}")),
            None if started.elapsed() >= OCR_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("OCR timed out after 120 seconds.".to_string());
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn cleanup_ocr_workspace(root: PathBuf, result: Result<Vec<u8>, String>) -> Result<Vec<u8>, String> {
    let _ = std::fs::remove_dir_all(root);
    result
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|duration| duration.as_nanos()).unwrap_or_default()
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
    run_image_adapter(request, input, "face-blur", "faceBlur", "faceBlurModel", "TOOLBOX_FACE_BLUR_PATH", "TOOLBOX_FACE_BLUR_MODEL_PATH", "toolbox-face-blur", "-blurred", "Blurred faces", ImageAdapterValidation::FaceBlur)
}

pub fn remove_background(request: &VisionRequest, input: PathBuf) -> JobOutcome {
    run_image_adapter(request, input, "background-removal", "backgroundRemoval", "backgroundRemovalModel", "TOOLBOX_BACKGROUND_REMOVAL_PATH", "TOOLBOX_BACKGROUND_REMOVAL_MODEL_PATH", "toolbox-background-removal", "-cutout", "Background removed", ImageAdapterValidation::Cutout)
}

enum ImageAdapterValidation { FaceBlur, Cutout }

fn run_image_adapter(request: &VisionRequest, input: PathBuf, mode: &str, resource_name: &str, model_name: &str, variable: &str, model_variable: &str, command: &str, suffix: &str, detail: &str, validation: ImageAdapterValidation) -> JobOutcome {
    let output = OutputNaming::get_destination(&input, &request.output_location, suffix, "png");
    let engine = match find_engine(resource_name, variable, command) {
        Ok(engine) => engine,
        Err(error) => return unavailable(input, error),
    };
    let model = match find_model(model_name, model_variable) {
        Ok(model) => model,
        Err(error) => return unavailable(input, error),
    };
    match Command::new(engine).arg("--mode").arg(mode).arg("--input").arg(&input).arg("--output").arg(&output).arg("--model").arg(model).output() {
        Ok(result) if result.status.success() && output.is_file() => {
            let validation = match validation {
                ImageAdapterValidation::FaceBlur => validate_face_blur(&input, &output),
                ImageAdapterValidation::Cutout => validate_cutout(&output),
            };
            match validation {
                Ok(()) => success(input, output, detail),
                Err(error) => { let _ = std::fs::remove_file(&output); failure(input, error) },
            }
        }
        Ok(result) if result.status.success() => {
            let _ = std::fs::remove_file(&output);
            failure(input, "Vision adapter reported success but produced no output.".to_string())
        }
        Ok(result) => { let _ = std::fs::remove_file(&output); failure(input, stderr(result, "Vision adapter failed.")) },
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

fn find_pdf_renderer() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("TOOLBOX_PDFTOPPM_PATH") {
        let path = PathBuf::from(path);
        if path.is_file() { return Ok(path); }
        return Err("TOOLBOX_PDFTOPPM_PATH does not point to a file.".to_string());
    }
    let root = resources::application_resource_root().unwrap_or_default();
    let executable = if cfg!(windows) { "pdftoppm.exe" } else { "pdftoppm" };
    [
        root.join("pdf-bin").join(executable),
        root.join("resources").join(executable),
        root.join(executable),
        root.join("resources").join("pdf-bin").join(executable),
    ].into_iter().find(|path| path.is_file())
        .or_else(|| {
            Command::new("pdftoppm").arg("-h").output().ok().filter(|result| result.status.success()).map(|_| PathBuf::from("pdftoppm"))
        })
        .ok_or_else(|| "PDF rasterizer is unavailable. Bundle pdftoppm or set TOOLBOX_PDFTOPPM_PATH.".to_string())
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

fn success(input_path: PathBuf, output: PathBuf, detail: &str) -> JobOutcome { JobOutcome { input_path, output_paths: vec![output], detail: detail.to_string(), failure: None } }
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::processing(error)) }
fn unavailable(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::unavailable(error)) }
fn stderr(result: std::process::Output, fallback: &str) -> String { String::from_utf8_lossy(&result.stderr).trim().lines().last().filter(|line| !line.is_empty()).unwrap_or(fallback).to_string() }

#[cfg(test)]
mod tests {
    use super::{normalize_ocr_text, validate_cutout, validate_face_blur};
    use image::{Rgba, RgbaImage};
    use std::path::PathBuf;

    fn path(name: &str) -> PathBuf { std::env::temp_dir().join(format!("toolbox_vision_{}_{}", std::process::id(), name)) }

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
