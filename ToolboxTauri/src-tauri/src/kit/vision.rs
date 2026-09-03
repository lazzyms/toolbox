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
    let engine = match find_engine("ocr", "TOOLBOX_TESSERACT_PATH", "tesseract") {
        Ok(engine) => engine,
        Err(error) => return unavailable(input, error),
    };
    let result = run_ocr(&engine, &input);
    match result {
        Ok(result) if result.status.success() => {
            if result.stdout.len() > OCR_MAX_OUTPUT_BYTES { return failure(input, "OCR output exceeds the 10 MiB text limit.".to_string()); }
            let text = normalize_ocr_text(&result.stdout);
            if text.is_empty() { return failure(input, "OCR completed but found no readable text.".to_string()); }
            match std::fs::write(&output, text) {
                Ok(_) => success(input, output, "OCR text extracted in page order"),
                Err(error) => failure(input, format!("Could not write OCR output: {error}")),
            }
        }
        Ok(result) => failure(input, stderr(result, "OCR engine failed.")),
        Err(error) => failure(input, error),
    }
}

fn run_ocr(engine: &std::path::Path, input: &std::path::Path) -> Result<std::process::Output, String> {
    let mut child = Command::new(engine).arg(input).arg("stdout").stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|error| format!("Could not run OCR engine: {error}"))?;
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
    let output = OutputNaming::get_destination(&input, &request.output_location, suffix, "png");
    let engine = match find_engine(resource_name, variable, command) {
        Ok(engine) => engine,
        Err(error) => return unavailable(input, error),
    };
    match Command::new(engine).arg(&input).arg(&output).output() {
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
