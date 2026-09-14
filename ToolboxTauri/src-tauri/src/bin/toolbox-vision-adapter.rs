use std::env;
use std::path::{Path, PathBuf};

use image::{imageops, DynamicImage, GenericImageView, GrayImage, ImageBuffer, Luma, RgbaImage};
use tract_onnx::prelude::*;

const YUNET_SIZE: usize = 640;
const U2NETP_SIZE: usize = 320;
const FACE_SCORE_THRESHOLD: f32 = 0.7;
const FACE_NMS_THRESHOLD: f32 = 0.3;

#[derive(Debug, Clone, Copy)]
enum Mode {
    FaceBlur,
    BackgroundRemoval,
}

struct Request {
    mode: Mode,
    input: PathBuf,
    output: PathBuf,
    model: PathBuf,
}

fn main() {
    if let Err(error) = run(parse_args(env::args().skip(1))) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn parse_args<I>(mut args: I) -> Result<Request, String>
where
    I: Iterator<Item = String>,
{
    let mut mode = None;
    let mut input = None;
    let mut output = None;
    let mut model = None;

    while let Some(argument) = args.next() {
        let value = args.next().ok_or_else(|| format!("Missing value for {argument}."))?;
        match argument.as_str() {
            "--mode" => mode = Some(parse_mode(&value)?),
            "--input" => input = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--model" => model = Some(PathBuf::from(value)),
            _ => return Err(format!("Unknown argument {argument}.")),
        }
    }

    Ok(Request {
        mode: mode.ok_or_else(|| "Missing --mode.".to_string())?,
        input: input.ok_or_else(|| "Missing --input.".to_string())?,
        output: output.ok_or_else(|| "Missing --output.".to_string())?,
        model: model.ok_or_else(|| "Missing --model.".to_string())?,
    })
}

fn parse_mode(value: &str) -> Result<Mode, String> {
    match value {
        "face-blur" => Ok(Mode::FaceBlur),
        "background-removal" => Ok(Mode::BackgroundRemoval),
        _ => Err(format!("Unsupported vision mode {value}.")),
    }
}

fn run(request: Result<Request, String>) -> Result<(), String> {
    let request = request?;
    if !request.input.is_file() {
        return Err(format!("Input image does not exist: {}", request.input.display()));
    }
    if !request.model.is_file() {
        return Err(format!("Vision model does not exist: {}", request.model.display()));
    }

    let image = image::open(&request.input).map_err(|error| format!("Could not decode input image: {error}"))?;
    let output = match request.mode {
        Mode::FaceBlur => blur_faces(image, &request.model)?,
        Mode::BackgroundRemoval => remove_background(image, &request.model)?,
    };
    output
        .save_with_format(&request.output, image::ImageFormat::Png)
        .map_err(|error| format!("Could not write PNG output: {error}"))
}

fn blur_faces(image: DynamicImage, model_path: &Path) -> Result<RgbaImage, String> {
    let (width, height) = image.dimensions();
    let mut output = image.to_rgba8();
    let resized = image.resize_exact(YUNET_SIZE as u32, YUNET_SIZE as u32, imageops::FilterType::Triangle).to_rgb8();
    let input = image_tensor(&resized, YUNET_SIZE, false, true);
    let outputs = run_model(model_path, input)?;
    let detections = decode_yunet(&outputs, width, height);

    if detections.is_empty() {
        return Err("No face was detected in the image.".to_string());
    }

    for detection in detections {
        let radius = ((detection.width.max(detection.height) * 0.16).round() as i32).max(2);
        let left = detection.left.max(0.0) as u32;
        let top = detection.top.max(0.0) as u32;
        let right = (detection.left + detection.width).min(width as f32).max(0.0) as u32;
        let bottom = (detection.top + detection.height).min(height as f32).max(0.0) as u32;
        if right <= left || bottom <= top {
            continue;
        }

        let crop = imageops::crop_imm(&output, left, top, right - left, bottom - top).to_image();
        let blurred = imageops::blur(&crop, radius as f32);
        imageops::overlay(&mut output, &blurred, left as i64, top as i64);
    }

    Ok(output)
}

fn remove_background(image: DynamicImage, model_path: &Path) -> Result<RgbaImage, String> {
    let (width, height) = image.dimensions();
    let original = image.to_rgba8();
    let resized = image.resize_exact(U2NETP_SIZE as u32, U2NETP_SIZE as u32, imageops::FilterType::Triangle).to_rgb8();
    let input = image_tensor(&resized, U2NETP_SIZE, true, false);
    let outputs = run_model(model_path, input)?;
    let mask = decode_u2net(&outputs)?;
    let mask = imageops::resize(&mask, width, height, imageops::FilterType::CatmullRom);
    let output = ImageBuffer::from_fn(width, height, |x, y| {
        let mut pixel = *original.get_pixel(x, y);
        pixel.0[3] = mask.get_pixel(x, y).0[0];
        pixel
    });

    if output.pixels().all(|pixel| pixel.0[3] == 0) || output.pixels().all(|pixel| pixel.0[3] == 255) {
        return Err("Background model produced an invalid subject mask.".to_string());
    }
    Ok(output)
}

fn image_tensor(image: &image::RgbImage, size: usize, normalize: bool, bgr: bool) -> Tensor {
    let values = (0..3)
        .flat_map(|channel| {
            (0..size).flat_map(move |y| {
                (0..size).map(move |x| {
                    let source_channel = if bgr { 2 - channel } else { channel };
                    let value = image.get_pixel(x as u32, y as u32).0[source_channel] as f32;
                    if normalize { value / 255.0 } else { value }
                })
            })
        })
        .collect::<Vec<_>>();
    Tensor::from_shape(&[1, 3, size, size], &values).expect("image tensor shape is fixed")
}

fn run_model(model_path: &Path, input: Tensor) -> Result<TVec<TValue>, String> {
    let model = tract_onnx::onnx()
        .model_for_path(model_path)
        .map_err(|error| format!("Could not load vision model: {error}"))?
        .into_optimized()
        .map_err(|error| format!("Could not optimize vision model: {error}"))?
        .into_runnable()
        .map_err(|error| format!("Could not prepare vision model: {error}"))?;
    model
        .run(tvec!(input.into_tvalue()))
        .map_err(|error| format!("Vision model inference failed: {error}"))
}

#[derive(Debug, Clone, Copy)]
struct FaceDetection {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    score: f32,
}

fn decode_yunet(outputs: &[TValue], width: u32, height: u32) -> Vec<FaceDetection> {
    let mut candidates = Vec::new();
    let scale_x = width as f32 / YUNET_SIZE as f32;
    let scale_y = height as f32 / YUNET_SIZE as f32;
    for (stride_index, stride) in [8usize, 16, 32].into_iter().enumerate() {
        let count = (YUNET_SIZE / stride) * (YUNET_SIZE / stride);
        let (Some(classification), Some(objectness), Some(location)) = (
            outputs.get(stride_index),
            outputs.get(stride_index + 3),
            outputs.get(stride_index + 6),
        ) else { continue };
        if classification.shape().as_ref() != [1, count, 1]
            || objectness.shape().as_ref() != [1, count, 1]
            || location.shape().as_ref() != [1, count, 4]
        { continue; }
        let Ok(location) = location.to_plain_array_view::<f32>() else { continue };
        let Ok(classification) = classification.to_plain_array_view::<f32>() else { continue };
        let Ok(objectness) = objectness.to_plain_array_view::<f32>() else { continue };
        for index in 0..count {
            let score = (classification[[0, index, 0]].clamp(0.0, 1.0) * objectness[[0, index, 0]].clamp(0.0, 1.0)).sqrt();
            if score < FACE_SCORE_THRESHOLD { continue; }
            let row = index / (YUNET_SIZE / stride);
            let column = index % (YUNET_SIZE / stride);
            let center_x = (column as f32 + location[[0, index, 0]]) * stride as f32;
            let center_y = (row as f32 + location[[0, index, 1]]) * stride as f32;
            let box_width = location[[0, index, 2]].exp() * stride as f32 * scale_x;
            let box_height = location[[0, index, 3]].exp() * stride as f32 * scale_y;
            let left = (center_x - box_width / 2.0).max(0.0);
            let top = (center_y - box_height / 2.0).max(0.0);
            candidates.push(FaceDetection { left, top, width: box_width, height: box_height, score });
        }
    }
    candidates.sort_by(|left, right| right.score.total_cmp(&left.score));
    non_max_suppression(candidates)
}

fn decode_u2net(outputs: &[TValue]) -> Result<GrayImage, String> {
    let output = outputs.first().ok_or_else(|| "Background model returned no mask.".to_string())?;
    let view = output.to_plain_array_view::<f32>().map_err(|error| format!("Background model returned an invalid mask: {error}"))?;
    if view.shape() != [1, 1, U2NETP_SIZE, U2NETP_SIZE] {
        return Err("Background model returned an unexpected mask shape.".to_string());
    }
    let mut mask = GrayImage::new(U2NETP_SIZE as u32, U2NETP_SIZE as u32);
    for y in 0..U2NETP_SIZE {
        for x in 0..U2NETP_SIZE {
            let value = view[[0, 0, y, x]].clamp(0.0, 1.0);
            mask.put_pixel(x as u32, y as u32, Luma([(value * 255.0).round() as u8]));
        }
    }
    Ok(mask)
}

fn non_max_suppression(mut detections: Vec<FaceDetection>) -> Vec<FaceDetection> {
    let mut selected = Vec::new();
    while let Some(candidate) = detections.first().copied() {
        detections.remove(0);
        detections.retain(|other| intersection_over_union(candidate, *other) < FACE_NMS_THRESHOLD);
        selected.push(candidate);
    }
    selected
}

fn intersection_over_union(left: FaceDetection, right: FaceDetection) -> f32 {
    let x1 = left.left.max(right.left);
    let y1 = left.top.max(right.top);
    let x2 = (left.left + left.width).min(right.left + right.width);
    let y2 = (left.top + left.height).min(right.top + right.height);
    let intersection = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let union = left.width * left.height + right.width * right.height - intersection;
    if union <= 0.0 { 0.0 } else { intersection / union }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_explicit_adapter_request() {
        let request = parse_args([
            "--mode", "face-blur", "--input", "input.png", "--output", "output.png", "--model", "model.onnx",
        ].into_iter().map(String::from)).unwrap();
        assert!(matches!(request.mode, Mode::FaceBlur));
        assert_eq!(request.input, PathBuf::from("input.png"));
        assert_eq!(request.output, PathBuf::from("output.png"));
        assert_eq!(request.model, PathBuf::from("model.onnx"));
    }

    #[test]
    fn non_max_suppression_keeps_the_highest_scoring_overlap() {
        let detections = non_max_suppression(vec![
            FaceDetection { left: 0.0, top: 0.0, width: 10.0, height: 10.0, score: 0.9 },
            FaceDetection { left: 1.0, top: 1.0, width: 10.0, height: 10.0, score: 0.8 },
        ]);
        assert_eq!(detections.len(), 1);
        assert_eq!(detections[0].score, 0.9);
    }
}
