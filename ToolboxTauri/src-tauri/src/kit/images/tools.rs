use image::{DynamicImage, ImageFormat};
use image::AnimationDecoder;
use std::fs::File;
use std::io::BufReader;
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
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToneRequest { pub paths: Vec<PathBuf>, pub brightness: i32, pub contrast: f32, pub output_location: OutputLocation }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatermarkRequest { pub paths: Vec<PathBuf>, pub opacity: u8, pub output_location: OutputLocation }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconSetRequest { pub paths: Vec<PathBuf>, pub sizes: Vec<u32>, pub output_location: OutputLocation }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GifCreateRequest { pub paths: Vec<PathBuf>, pub output_location: OutputLocation }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GifExtractRequest { pub paths: Vec<PathBuf>, pub output_location: OutputLocation }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TiffRequest { pub paths: Vec<PathBuf>, pub output_location: OutputLocation }

pub fn resize(request: &ResizeRequest, input: PathBuf) -> JobOutcome { transform(request.paths.as_slice(), input, &request.output_location, "-resized", |image| { if request.width == 0 || request.height == 0 { return Err("Image dimensions must be positive.".to_string()); } Ok(image.resize_exact(request.width, request.height, image::imageops::FilterType::Lanczos3)) }) }
pub fn rotate(request: &RotateRequest, input: PathBuf) -> JobOutcome { transform(request.paths.as_slice(), input, &request.output_location, "-rotated", |image| { Ok(match request.degrees.rem_euclid(360) { 90 => image.rotate90(), 180 => image.rotate180(), 270 => image.rotate270(), 0 => image, _ => return Err("Rotation must be 0, 90, 180, or 270 degrees.".to_string()) }) }) }
pub fn crop(request: &CropRequest, input: PathBuf) -> JobOutcome { transform(request.paths.as_slice(), input, &request.output_location, "-cropped", |image| { if request.width == 0 || request.height == 0 || request.x.saturating_add(request.width) > image.width() || request.y.saturating_add(request.height) > image.height() { return Err("Crop rectangle must fit inside the image.".to_string()); } Ok(image.crop_imm(request.x, request.y, request.width, request.height)) }) }
pub fn tone(request: &ToneRequest, input: PathBuf) -> JobOutcome { transform(&request.paths, input, &request.output_location, "-tone", |image| { let image = image::imageops::brighten(&image, request.brightness); Ok(DynamicImage::ImageRgba8(image).adjust_contrast(request.contrast)) }) }
pub fn watermark(request: &WatermarkRequest, input: PathBuf) -> JobOutcome { transform(&request.paths, input, &request.output_location, "-watermarked", |image| { let mut image = image.to_rgba8(); let opacity = request.opacity.min(100) as u16 * 255 / 100; for pixel in image.pixels_mut() { pixel.0[0] = ((pixel.0[0] as u16 * (255 - opacity) + opacity * 255) / 255) as u8; } Ok(DynamicImage::ImageRgba8(image)) }) }
pub fn icon_set(request: &IconSetRequest, input: PathBuf) -> JobOutcome {
    let image = match image::open(&input) { Ok(image) => image, Err(error) => return failure(input, format!("Could not read image: {error}")) };
    let mut outputs = Vec::new();
    for size in request.sizes.iter().copied().filter(|size| *size > 0) {
        let output = OutputNaming::get_destination(&input, &request.output_location, &format!("-icon-{size}"), "png");
        if let Err(error) = image.resize_exact(size, size, image::imageops::FilterType::Lanczos3).save(&output) { return failure(input, format!("Could not save icon: {error}")); }
        outputs.push(output);
    }
    if outputs.is_empty() { return failure(input, "Select at least one icon size.".to_string()); }
    JobOutcome { input_path: input, output_paths: outputs, detail: "Icons saved".to_string(), failure: None }
}

pub fn gif_create(request: &GifCreateRequest) -> JobOutcome {
    let Some(input) = request.paths.first().cloned() else { return failure(PathBuf::new(), "Select at least one image.".to_string()); };
    let output = OutputNaming::get_destination(&input, &request.output_location, "-animated", "gif");
    let file = match File::create(&output) { Ok(file) => file, Err(error) => return failure(input, error.to_string()) };
    let mut encoder = image::codecs::gif::GifEncoder::new(file);
    let frames = request.paths.iter().map(|path| image::open(path).map(|image| image::Frame::from_parts(image.to_rgba8(), 0, 0, image::Delay::from_numer_denom_ms(100, 1))).map_err(|error| error.to_string())).collect::<Result<Vec<_>, _>>();
    match frames.and_then(|frames| encoder.encode_frames(frames).map_err(|error| error.to_string())) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: "GIF saved".to_string(), failure: None }, Err(error) => failure(input, format!("Could not create GIF: {error}")) }
}

pub fn gif_extract(request: &GifExtractRequest, input: PathBuf) -> JobOutcome {
    let file = match File::open(&input) { Ok(file) => file, Err(error) => return failure(input, error.to_string()) };
    let decoder = match image::codecs::gif::GifDecoder::new(BufReader::new(file)) { Ok(decoder) => decoder, Err(error) => return failure(input, error.to_string()) };
    let frames = match decoder.into_frames().collect_frames() { Ok(frames) => frames, Err(error) => return failure(input, error.to_string()) };
    let mut outputs = Vec::new();
    for (index, frame) in frames.into_iter().enumerate() { let output = OutputNaming::get_destination(&input, &request.output_location, &format!("-frame-{}", index + 1), "png"); if let Err(error) = frame.into_buffer().save(&output) { return failure(input, error.to_string()); } outputs.push(output); }
    JobOutcome { input_path: input, output_paths: outputs, detail: "GIF frames saved".to_string(), failure: None }
}

pub fn tiff(request: &TiffRequest, input: PathBuf) -> JobOutcome { transform(&request.paths, input, &request.output_location, "-tiff", |image| Ok(image)) }

fn transform<F>(_: &[PathBuf], input: PathBuf, location: &OutputLocation, suffix: &str, edit: F) -> JobOutcome where F: FnOnce(DynamicImage) -> Result<DynamicImage, String> {
    let image = match image::open(&input) { Ok(image) => image, Err(error) => return failure(input, format!("Could not read image: {error}")) };
    let image = match edit(image) { Ok(image) => image, Err(error) => return failure(input, error) };
    let extension = input.extension().and_then(|extension| extension.to_str()).unwrap_or("png");
    let output = OutputNaming::get_destination(&input, location, suffix, extension);
    let format = ImageFormat::from_path(&output).unwrap_or(ImageFormat::Png);
    match image.save_with_format(&output, format) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: "Image saved".to_string(), failure: None }, Err(error) => failure(input, format!("Could not save image: {error}")) }
}

fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome { input_path, output_paths: vec![], detail: String::new(), failure: Some(error) } }
