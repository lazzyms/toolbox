use std::path::PathBuf;
use std::process::Command;

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
}

pub fn ocr_pdf(request: &VisionRequest, input: PathBuf) -> JobOutcome {
    let output = OutputNaming::get_destination(&input, &request.output_location, "-ocr-text", "txt");
    let Some(engine) = find_engine("TOOLBOX_TESSERACT_PATH", "tesseract") else {
        return failure(input, "OCR is unavailable offline. Install tesseract or set TOOLBOX_TESSERACT_PATH to the bundled engine.".to_string());
    };
    let result = Command::new(engine).arg(&input).arg("stdout").output();
    match result {
        Ok(result) if result.status.success() => match std::fs::write(&output, result.stdout) {
            Ok(_) => success(input, output, "OCR text extracted"),
            Err(error) => failure(input, format!("Could not write OCR output: {error}")),
        },
        Ok(result) => failure(input, stderr(result, "OCR engine failed.")),
        Err(error) => failure(input, format!("Could not run OCR engine: {error}")),
    }
}

pub fn blur_faces(request: &VisionRequest, input: PathBuf) -> JobOutcome {
    run_image_adapter(request, input, "TOOLBOX_FACE_BLUR_PATH", "toolbox-face-blur", "-blurred", "Blurred faces")
}

pub fn remove_background(request: &VisionRequest, input: PathBuf) -> JobOutcome {
    run_image_adapter(request, input, "TOOLBOX_BACKGROUND_REMOVAL_PATH", "toolbox-background-removal", "-cutout", "Background removed")
}

fn run_image_adapter(request: &VisionRequest, input: PathBuf, variable: &str, command: &str, suffix: &str, detail: &str) -> JobOutcome {
    let output = OutputNaming::get_destination(&input, &request.output_location, suffix, "png");
    let Some(engine) = find_engine(variable, command) else {
        return failure(input, format!("This offline vision feature is unavailable on this installation. Set {variable} to a compatible bundled adapter."));
    };
    match Command::new(engine).arg(&input).arg(&output).output() {
        Ok(result) if result.status.success() && output.is_file() => success(input, output, detail),
        Ok(result) if result.status.success() => failure(input, "Vision adapter reported success but produced no output.".to_string()),
        Ok(result) => failure(input, stderr(result, "Vision adapter failed.")),
        Err(error) => failure(input, format!("Could not run vision adapter: {error}")),
    }
}

fn find_engine(variable: &str, command: &str) -> Option<PathBuf> {
    std::env::var_os(variable).map(PathBuf::from).filter(|path| path.is_file()).or_else(|| Command::new(command).arg("--help").output().ok().map(|_| PathBuf::from(command)))
}

fn success(input_path: PathBuf, output: PathBuf, detail: &str) -> JobOutcome { JobOutcome { input_path, output_paths: vec![output], detail: detail.to_string(), failure: None } }
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome { input_path, output_paths: vec![], detail: String::new(), failure: Some(error) } }
fn stderr(result: std::process::Output, fallback: &str) -> String { String::from_utf8_lossy(&result.stderr).trim().lines().last().filter(|line| !line.is_empty()).unwrap_or(fallback).to_string() }
