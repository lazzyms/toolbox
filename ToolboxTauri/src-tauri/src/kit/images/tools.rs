use image::{DynamicImage, ImageFormat};
use std::path::PathBuf;

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeRequest { pub paths: Vec<PathBuf>, pub width: u32, pub height: u32, pub output_location: OutputLocation }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotateRequest { pub paths: Vec<PathBuf>, pub degrees: i32, pub output_location: OutputLocation }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropRequest { pub paths: Vec<PathBuf>, pub x: u32, pub y: u32, pub width: u32, pub height: u32, pub output_location: OutputLocation }

pub fn resize(request: &ResizeRequest, input: PathBuf) -> JobOutcome { transform(request.paths.as_slice(), input, &request.output_location, "-resized", |image| { if request.width == 0 || request.height == 0 { return Err("Image dimensions must be positive.".to_string()); } Ok(image.resize_exact(request.width, request.height, image::imageops::FilterType::Lanczos3)) }) }
pub fn rotate(request: &RotateRequest, input: PathBuf) -> JobOutcome { transform(request.paths.as_slice(), input, &request.output_location, "-rotated", |image| { Ok(match request.degrees.rem_euclid(360) { 90 => image.rotate90(), 180 => image.rotate180(), 270 => image.rotate270(), 0 => image, _ => return Err("Rotation must be 0, 90, 180, or 270 degrees.".to_string()) }) }) }
pub fn crop(request: &CropRequest, input: PathBuf) -> JobOutcome { transform(request.paths.as_slice(), input, &request.output_location, "-cropped", |image| { if request.width == 0 || request.height == 0 || request.x.saturating_add(request.width) > image.width() || request.y.saturating_add(request.height) > image.height() { return Err("Crop rectangle must fit inside the image.".to_string()); } Ok(image.crop_imm(request.x, request.y, request.width, request.height)) }) }

fn transform<F>(_: &[PathBuf], input: PathBuf, location: &OutputLocation, suffix: &str, edit: F) -> JobOutcome where F: FnOnce(DynamicImage) -> Result<DynamicImage, String> {
    let image = match image::open(&input) { Ok(image) => image, Err(error) => return failure(input, format!("Could not read image: {error}")) };
    let image = match edit(image) { Ok(image) => image, Err(error) => return failure(input, error) };
    let extension = input.extension().and_then(|extension| extension.to_str()).unwrap_or("png");
    let output = OutputNaming::get_destination(&input, location, suffix, extension);
    let format = ImageFormat::from_path(&output).unwrap_or(ImageFormat::Png);
    match image.save_with_format(&output, format) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: "Image saved".to_string(), failure: None }, Err(error) => failure(input, format!("Could not save image: {error}")) }
}

fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome { input_path, output_paths: vec![], detail: String::new(), failure: Some(error) } }
