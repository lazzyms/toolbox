use base64::{engine::general_purpose::STANDARD, Engine};
use image::codecs::png::PngEncoder;
use image::{DynamicImage, ImageFormat, ImageEncoder, Rgba};
use image::AnimationDecoder;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufReader, Seek};
use std::path::{Path, PathBuf};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::common::watermark::{WatermarkOptions, WatermarkPosition};
use crate::kit::contracts::ToolError;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePreviewRequest {
    pub path: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePreview {
    pub width: u32,
    pub height: u32,
    pub data_url: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ImageEdit {
    Convert { format: String },
    Compress { quality: u8, #[serde(default)] lossless: bool },
    Resize {
        width: u32,
        height: u32,
        #[serde(default = "default_resize_mode")] mode: String,
        #[serde(default)] percentage: u32,
        #[serde(default)] longest_side: u32,
        #[serde(default = "default_resampling")] resampling: String,
        #[serde(default = "default_keep_aspect_ratio")] keep_aspect_ratio: bool,
    },
    Rotate { degrees: i32, #[serde(default = "default_flip")] flip: String },
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        #[serde(default = "default_crop_mode")] mode: String,
        #[serde(default)] aspect_width: u32,
        #[serde(default)] aspect_height: u32,
        #[serde(default = "default_crop_anchor")] anchor: String,
    },
    Tone { brightness: i32, contrast: f32, #[serde(default)] saturation: f32, #[serde(default)] exposure: f32 },
    Watermark(WatermarkOptions),
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageEditPlan {
    pub edits: Vec<ImageEdit>,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageEditPreviewRequest {
    pub path: PathBuf,
    pub plan: ImageEditPlan,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageEditExportRequest {
    pub paths: Vec<PathBuf>,
    pub plan: ImageEditPlan,
}

impl ImageEditPlan {
    fn canonicalized(&self) -> Self {
        let latest_crop = self.edits.iter().rev().find(|edit| matches!(edit, ImageEdit::Crop { .. })).cloned();
        let mut crop_inserted = false;
        let mut edits = Vec::with_capacity(self.edits.len());
        for edit in &self.edits {
            if matches!(edit, ImageEdit::Crop { .. }) {
                if crop_inserted { continue; }
                crop_inserted = true;
                if let Some(crop) = &latest_crop { edits.push(crop.clone()); }
            } else {
                edits.push(edit.clone());
            }
        }
        Self { edits, ..self.clone() }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.edits.is_empty() { return Err("Add at least one image edit before exporting.".to_string()); }
        for edit in &self.edits {
            match edit {
                ImageEdit::Convert { format } if !matches!(format.as_str(), "jpg" | "jpeg" | "png" | "webp" | "heic") => return Err(format!("Unsupported image format: {format}.")),
                ImageEdit::Compress { quality, .. } if *quality == 0 || *quality > 100 => return Err("Compression quality must be between 1 and 100.".to_string()),
                ImageEdit::Resize { width, height, mode, percentage, longest_side, .. } => {
                    match mode.as_str() {
                        "exact" if *width == 0 || *height == 0 => return Err("Resize dimensions must be positive.".to_string()),
                        "percentage" if *percentage == 0 || *percentage > 1000 => return Err("Resize percentage must be between 1 and 1000.".to_string()),
                        "longestSide" if *longest_side == 0 || *longest_side > 16384 => return Err("Longest side must be between 1 and 16384 pixels.".to_string()),
                        "exact" | "percentage" | "longestSide" => {},
                        _ => return Err(format!("Unsupported resize mode: {mode}.")),
                    }
                }
                ImageEdit::Rotate { degrees, flip } if !matches!(degrees.rem_euclid(360), 0 | 90 | 180 | 270) => return Err("Rotation must be 0, 90, 180, or 270 degrees.".to_string()),
                ImageEdit::Rotate { flip, .. } if !matches!(flip.as_str(), "none" | "horizontal" | "vertical") => return Err("Mirror choice must be none, horizontal, or vertical.".to_string()),
                ImageEdit::Crop { width, height, mode, aspect_width, aspect_height, .. } if mode == "rectangle" && (*width == 0 || *height == 0) => return Err("Crop rectangle must have positive dimensions.".to_string()),
                ImageEdit::Crop { mode, aspect_width, aspect_height, .. } if mode == "aspectRatio" && (*aspect_width == 0 || *aspect_height == 0) => return Err("Aspect ratio dimensions must be positive.".to_string()),
                ImageEdit::Crop { mode, .. } if !matches!(mode.as_str(), "rectangle" | "aspectRatio") => return Err(format!("Unsupported crop mode: {mode}.")),
                ImageEdit::Tone { brightness, contrast, saturation, exposure } if !(-100..=100).contains(brightness) => return Err("Brightness must be between -100 and 100.".to_string()),
                ImageEdit::Tone { contrast, .. } if !(-100.0..=100.0).contains(contrast) => return Err("Contrast must be between -100 and 100.".to_string()),
                ImageEdit::Tone { saturation, .. } if !(-100.0..=100.0).contains(saturation) => return Err("Saturation must be between -100 and 100.".to_string()),
                ImageEdit::Tone { exposure, .. } if !(-100.0..=100.0).contains(exposure) => return Err("Exposure must be between -100 and 100.".to_string()),
                ImageEdit::Watermark(options) => options.validate()?,
                _ => {}
            }
        }
        Ok(())
    }
}

pub fn inspect_edit_preview(request: &ImageEditPreviewRequest) -> Result<ImagePreview, String> {
    let plan = request.plan.canonicalized();
    plan.validate()?;
    let source = crate::kit::images::decode(&request.path)?;
    let format = source.family.output_format();
    let (edited, format, quality) = apply_edit_plan(&plan, source.into_first_page(), format)?;
    let encoded = crate::kit::images::encode(&edited, format, quality)?;
    let preview_source = match format {
        crate::kit::images::OutputFormat::Heic => heif::decode(&encoded)
            .map_err(|error| format!("Could not decode edited HEIC preview: {error}"))?,
        _ => image::load_from_memory(&encoded)
            .map_err(|error| format!("Could not decode edited image preview: {error}"))?,
    };
    encode_preview(preview_source)
}

pub fn export_edit_plan(plan: &ImageEditPlan, input: PathBuf) -> JobOutcome {
    let plan = plan.canonicalized();
    if let Err(error) = plan.validate() { return failure(input, error); }
    let source = match crate::kit::images::decode(&input) {
        Ok(source) => source,
        Err(error) => return failure(input, error),
    };
    let format = source.family.output_format();
    let (edited, format, quality) = match apply_edit_plan(&plan, source.into_first_page(), format) {
        Ok(value) => value,
        Err(error) => return failure(input, error),
    };
    let bytes = match crate::kit::images::encode(&edited, format, quality) {
        Ok(bytes) => bytes,
        Err(error) => return failure(input, error),
    };
    let output = match OutputNaming::reserve_destination(&input, &plan.output_location, "-edited", format.extension()) {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve edited image output: {error}")),
    };
    if let Err(error) = std::fs::write(output.path(), bytes) {
        return failure(input, format!("Could not save edited image: {error}"));
    }
    match output.publish() {
        Ok(path) => JobOutcome::succeeded(input, vec![path], format!("Applied {} edits in one export.", plan.edits.len())),
        Err(error) => failure(input, format!("Could not publish edited image: {error}")),
    }
}

fn encode_preview(image: DynamicImage) -> Result<ImagePreview, String> {
    let width = image.width();
    let height = image.height();
    let preview = if width <= 1200 && height <= 900 {
        image.to_rgba8()
    } else {
        image.thumbnail(1200, 900).to_rgba8()
    };
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(preview.as_raw(), preview.width(), preview.height(), image::ExtendedColorType::Rgba8)
        .map_err(|error| format!("Could not encode image preview: {error}"))?;
    Ok(ImagePreview { width, height, data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)) })
}

fn apply_edit_plan(plan: &ImageEditPlan, mut image: DynamicImage, mut format: crate::kit::images::OutputFormat) -> Result<(DynamicImage, crate::kit::images::OutputFormat, u8), String> {
    let mut quality = 90;
    let source_width = image.width();
    let source_height = image.height();
    let mut geometry = Vec::new();
    for edit in &plan.edits {
        match edit {
            ImageEdit::Convert { format: requested } => format = parse_output_format(requested)?,
            ImageEdit::Compress { quality: requested, lossless } => { quality = if *lossless { 100 } else { *requested }; },
            ImageEdit::Resize { width, height, mode, percentage, longest_side, resampling, keep_aspect_ratio } => {
                let request = ResizeRequest { paths: vec![], width: *width, height: *height, mode: mode.clone(), resampling: resampling.clone(), keep_aspect_ratio: *keep_aspect_ratio, percentage: *percentage, longest_side: *longest_side, output_location: OutputLocation::AlongsideInput };
                let old_width = image.width();
                let old_height = image.height();
                let (width, height) = resize_dimensions(image.width(), image.height(), &request)?;
                let filter = match resampling.as_str() { "nearest" => image::imageops::FilterType::Nearest, "bicubic" => image::imageops::FilterType::CatmullRom, _ => image::imageops::FilterType::Lanczos3 };
                image = image.resize_exact(width, height, filter);
                geometry.push(ImageGeometryStep::Resize { from_width: old_width, from_height: old_height, to_width: width, to_height: height });
            }
            ImageEdit::Rotate { degrees, flip } => {
                let old_width = image.width();
                let old_height = image.height();
                image = match degrees.rem_euclid(360) { 90 => image.rotate90(), 180 => image.rotate180(), 270 => image.rotate270(), 0 => image, _ => return Err("Rotation must be 0, 90, 180, or 270 degrees.".to_string()) };
                geometry.push(ImageGeometryStep::Rotate { width: old_width, height: old_height, degrees: degrees.rem_euclid(360) });
                let current_width = image.width();
                let current_height = image.height();
                image = match flip.as_str() { "none" => image, "horizontal" => DynamicImage::ImageRgba8(image::imageops::flip_horizontal(&image.to_rgba8())), "vertical" => DynamicImage::ImageRgba8(image::imageops::flip_vertical(&image.to_rgba8())), _ => return Err("Mirror choice must be none, horizontal, or vertical.".to_string()) };
                if flip == "horizontal" { geometry.push(ImageGeometryStep::FlipHorizontal { width: current_width }); }
                if flip == "vertical" { geometry.push(ImageGeometryStep::FlipVertical { height: current_height }); }
            }
            ImageEdit::Crop { x, y, width, height, mode, aspect_width, aspect_height, anchor } => {
                let request = CropRequest { paths: vec![], x: *x, y: *y, width: *width, height: *height, mode: mode.clone(), aspect_width: *aspect_width, aspect_height: *aspect_height, anchor: anchor.clone(), output_location: OutputLocation::AlongsideInput };
                let (x, y, width, height) = crop_rect(source_width, source_height, &request)?;
                let crop = map_source_rect((x, y, width, height), &geometry)?;
                image = image.crop_imm(crop.0, crop.1, crop.2, crop.3);
            }
            ImageEdit::Tone { brightness, contrast, saturation, exposure } => {
                let mut rgba = image::imageops::brighten(&image, *brightness);
                let exposure = 2.0_f32.powf(*exposure / 100.0);
                let saturation = (1.0 + *saturation / 100.0).max(0.0);
                for pixel in rgba.pixels_mut() {
                    let [red, green, blue, alpha] = pixel.0;
                    let average = (red as f32 + green as f32 + blue as f32) / 3.0;
                    pixel.0 = [
                        ((average + (red as f32 - average) * saturation) * exposure).clamp(0.0, 255.0) as u8,
                        ((average + (green as f32 - average) * saturation) * exposure).clamp(0.0, 255.0) as u8,
                        ((average + (blue as f32 - average) * saturation) * exposure).clamp(0.0, 255.0) as u8,
                        alpha,
                    ];
                }
                image = DynamicImage::ImageRgba8(rgba).adjust_contrast(*contrast);
            }
            ImageEdit::Watermark(options) => image = apply_watermark(image, options)?,
        }
    }
    Ok((image, format, quality.clamp(1, 100)))
}

#[derive(Debug, Clone, Copy)]
enum ImageGeometryStep {
    Resize { from_width: u32, from_height: u32, to_width: u32, to_height: u32 },
    Rotate { width: u32, height: u32, degrees: i32 },
    FlipHorizontal { width: u32 },
    FlipVertical { height: u32 },
}

fn map_source_rect(mut rect: (u32, u32, u32, u32), geometry: &[ImageGeometryStep]) -> Result<(u32, u32, u32, u32), String> {
    for step in geometry {
        let (x, y, width, height) = rect;
        let right = x.checked_add(width).ok_or_else(|| "Crop rectangle exceeds coordinate bounds.".to_string())?;
        let bottom = y.checked_add(height).ok_or_else(|| "Crop rectangle exceeds coordinate bounds.".to_string())?;
        rect = match step {
            ImageGeometryStep::Resize { from_width, from_height, to_width, to_height } => (
                scale_floor(x, *from_width, *to_width),
                scale_floor(y, *from_height, *to_height),
                scale_ceil(right, *from_width, *to_width) - scale_floor(x, *from_width, *to_width),
                scale_ceil(bottom, *from_height, *to_height) - scale_floor(y, *from_height, *to_height),
            ),
            ImageGeometryStep::Rotate { width: image_width, height: image_height, degrees } => match degrees {
                90 => (*image_height - bottom, x, height, width),
                180 => (*image_width - right, *image_height - bottom, width, height),
                270 => (y, *image_width - right, height, width),
                _ => (x, y, width, height),
            },
            ImageGeometryStep::FlipHorizontal { width: image_width } => (*image_width - right, y, width, height),
            ImageGeometryStep::FlipVertical { height: image_height } => (x, *image_height - bottom, width, height),
        };
    }
    Ok(rect)
}

fn scale_floor(value: u32, from: u32, to: u32) -> u32 { ((value as u64 * to as u64) / from as u64) as u32 }
fn scale_ceil(value: u32, from: u32, to: u32) -> u32 { ((value as u64 * to as u64 + from as u64 - 1) / from as u64) as u32 }

pub(crate) fn parse_output_format(format: &str) -> Result<crate::kit::images::OutputFormat, String> {
    match format.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Ok(crate::kit::images::OutputFormat::Jpeg),
        "png" => Ok(crate::kit::images::OutputFormat::Png),
        "webp" => Ok(crate::kit::images::OutputFormat::WebP),
        "heic" => Ok(crate::kit::images::OutputFormat::Heic),
        _ => Err(format!("Unsupported image format: {format}.")),
    }
}

pub fn inspect_preview(request: &ImagePreviewRequest) -> Result<ImagePreview, String> {
    let image = crate::kit::images::decode(&request.path).map_err(|error| format!("Could not preview image: {error}"))?.into_first_page();
    let width = image.width();
    let height = image.height();
    let preview = image.thumbnail(1200, 900).to_rgba8();
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(
            preview.as_raw(),
            preview.width(),
            preview.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| format!("Could not encode image preview: {error}"))?;
    Ok(ImagePreview {
        width,
        height,
        data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
    })
}

pub fn inspect_tiff_pages(request: &TiffPreviewRequest) -> Result<Vec<ImagePreview>, String> {
    crate::kit::images::decode(&request.path)?
        .pages
        .into_iter()
        .map(encode_preview)
        .collect()
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeRequest {
    pub paths: Vec<PathBuf>, pub width: u32, pub height: u32,
    #[serde(default = "default_resize_mode")] pub mode: String,
    #[serde(default = "default_resampling")] pub resampling: String,
    #[serde(default = "default_keep_aspect_ratio")] pub keep_aspect_ratio: bool,
    #[serde(default)] pub percentage: u32, #[serde(default)] pub longest_side: u32,
    pub output_location: OutputLocation,
}
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotateRequest { pub paths: Vec<PathBuf>, pub degrees: i32, #[serde(default = "default_flip")] pub flip: String, pub output_location: OutputLocation }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropRequest {
    pub paths: Vec<PathBuf>, pub x: u32, pub y: u32, pub width: u32, pub height: u32,
    #[serde(default = "default_crop_mode")] pub mode: String,
    #[serde(default)] pub aspect_width: u32, #[serde(default)] pub aspect_height: u32,
    #[serde(default = "default_crop_anchor")] pub anchor: String,
    pub output_location: OutputLocation,
}
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToneRequest {
    pub paths: Vec<PathBuf>,
    pub brightness: i32,
    pub contrast: f32,
    #[serde(default)] pub saturation: f32,
    #[serde(default)] pub exposure: f32,
    pub output_location: OutputLocation,
}
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatermarkRequest {
    pub paths: Vec<PathBuf>,
    #[serde(flatten)] pub options: WatermarkOptions,
    pub output_location: OutputLocation,
}
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconSetRequest {
    pub paths: Vec<PathBuf>,
    #[serde(default = "default_icon_preset")] pub preset: String,
    #[serde(default)] pub sizes: Vec<u32>,
    pub output_location: OutputLocation,
}
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GifCreateRequest {
    pub paths: Vec<PathBuf>,
    #[serde(default = "default_gif_delay_ms")] pub frame_delay_ms: u32,
    #[serde(default = "default_gif_loop")] pub loop_forever: bool,
    pub output_location: OutputLocation,
    #[serde(default)]
    pub output_path: Option<PathBuf>,
}
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GifExtractRequest { pub paths: Vec<PathBuf>, pub output_location: OutputLocation }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TiffPageRef { pub path: PathBuf, pub page: usize }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TiffRequest {
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub pages: Option<Vec<TiffPageRef>>,
    pub output_location: OutputLocation,
    #[serde(default)]
    pub output_path: Option<PathBuf>,
}
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TiffPreviewRequest { pub path: PathBuf }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataRequest { pub paths: Vec<PathBuf>, pub output_location: OutputLocation }

pub fn resize(request: &ResizeRequest, input: PathBuf) -> JobOutcome {
    transform(request.paths.as_slice(), input, &request.output_location, "-resized", |image| {
        let (width, height) = resize_dimensions(image.width(), image.height(), request)?;
        let filter = match request.resampling.as_str() { "nearest" => image::imageops::FilterType::Nearest, "bicubic" => image::imageops::FilterType::CatmullRom, _ => image::imageops::FilterType::Lanczos3 };
        Ok(image.resize_exact(width, height, filter))
    })
}

fn default_resize_mode() -> String { "exact".to_string() }
fn default_resampling() -> String { "lanczos".to_string() }
fn default_keep_aspect_ratio() -> bool { true }

fn resize_dimensions(source_width: u32, source_height: u32, request: &ResizeRequest) -> Result<(u32, u32), String> {
    let dimensions = match request.mode.as_str() {
        "exact" if request.keep_aspect_ratio => {
            if request.width == 0 || request.height == 0 { return Err("Resize dimensions must be positive.".to_string()); }
            let scale = (request.width as f64 / source_width as f64).min(request.height as f64 / source_height as f64);
            (((source_width as f64 * scale).round() as u32).max(1), ((source_height as f64 * scale).round() as u32).max(1))
        }
        "exact" => (request.width, request.height),
        "percentage" => {
            if request.percentage == 0 || request.percentage > 1000 { return Err("Resize percentage must be between 1 and 1000.".to_string()); }
            (scale_dimension(source_width, request.percentage), scale_dimension(source_height, request.percentage))
        }
        "longestSide" => {
            if request.longest_side == 0 || request.longest_side > 16384 { return Err("Longest side must be between 1 and 16384 pixels.".to_string()); }
            let scale = request.longest_side as f64 / source_width.max(source_height) as f64;
            (((source_width as f64 * scale).round() as u32).max(1), ((source_height as f64 * scale).round() as u32).max(1))
        }
        _ => return Err(format!("Unsupported resize mode: {}.", request.mode)),
    };
    if dimensions.0 == 0 || dimensions.1 == 0 || dimensions.0 > 16384 || dimensions.1 > 16384 { return Err("Resize dimensions must be between 1 and 16384 pixels.".to_string()); }
    Ok(dimensions)
}

fn scale_dimension(value: u32, percentage: u32) -> u32 { ((value as u64 * percentage as u64 + 50) / 100).max(1).min(16384) as u32 }
pub fn rotate(request: &RotateRequest, input: PathBuf) -> JobOutcome {
    transform(request.paths.as_slice(), input, &request.output_location, "-rotated", |image| {
        let image = match request.degrees.rem_euclid(360) { 90 => image.rotate90(), 180 => image.rotate180(), 270 => image.rotate270(), 0 => image, _ => return Err("Rotation must be 0, 90, 180, or 270 degrees.".to_string()) };
        Ok(match request.flip.as_str() { "none" => image, "horizontal" => DynamicImage::ImageRgba8(image::imageops::flip_horizontal(&image.to_rgba8())), "vertical" => DynamicImage::ImageRgba8(image::imageops::flip_vertical(&image.to_rgba8())), _ => return Err("Mirror choice must be none, horizontal, or vertical.".to_string()) })
    })
}

fn default_flip() -> String { "none".to_string() }
pub fn crop(request: &CropRequest, input: PathBuf) -> JobOutcome {
    transform(request.paths.as_slice(), input, &request.output_location, "-cropped", |image| {
        let (x, y, width, height) = crop_rect(image.width(), image.height(), request)?;
        Ok(image.crop_imm(x, y, width, height))
    })
}

fn default_crop_mode() -> String { "rectangle".to_string() }
fn default_crop_anchor() -> String { "center".to_string() }

fn crop_rect(image_width: u32, image_height: u32, request: &CropRequest) -> Result<(u32, u32, u32, u32), String> {
    if request.mode == "aspectRatio" {
        if request.aspect_width == 0 || request.aspect_height == 0 { return Err("Aspect ratio dimensions must be positive.".to_string()); }
        let ratio = request.aspect_width as f64 / request.aspect_height as f64;
        let (width, height) = if image_width as f64 / image_height as f64 > ratio {
            let height = image_height;
            ((height as f64 * ratio).round() as u32, height)
        } else {
            let width = image_width;
            (width, (width as f64 / ratio).round() as u32)
        };
        if width == 0 || height == 0 || width > image_width || height > image_height { return Err("Aspect ratio crop does not fit inside the image.".to_string()); }
        let max_x = image_width - width;
        let max_y = image_height - height;
        let x = match request.anchor.as_str() { "left" => 0, "right" => max_x, "custom" => request.x.min(max_x), _ => max_x / 2 };
        let y = match request.anchor.as_str() { "top" => 0, "bottom" => max_y, "custom" => request.y.min(max_y), _ => max_y / 2 };
        Ok((x, y, width, height))
    } else if request.mode == "rectangle" {
        if request.width == 0 || request.height == 0 || request.x.saturating_add(request.width) > image_width || request.y.saturating_add(request.height) > image_height { return Err("Crop rectangle must fit inside the image.".to_string()); }
        Ok((request.x, request.y, request.width, request.height))
    } else { Err(format!("Unsupported crop mode: {}.", request.mode)) }
}
pub fn tone(request: &ToneRequest, input: PathBuf) -> JobOutcome {
    transform(&request.paths, input, &request.output_location, "-tone", |image| {
        let mut image = image::imageops::brighten(&image, request.brightness);
        let exposure = 2.0_f32.powf(request.exposure.clamp(-100.0, 100.0) / 100.0);
        let saturation = (1.0 + request.saturation.clamp(-100.0, 100.0) / 100.0).max(0.0);
        for pixel in image.pixels_mut() {
            let [red, green, blue, alpha] = pixel.0;
            let average = (red as f32 + green as f32 + blue as f32) / 3.0;
            pixel.0 = [
                ((average + (red as f32 - average) * saturation) * exposure).clamp(0.0, 255.0) as u8,
                ((average + (green as f32 - average) * saturation) * exposure).clamp(0.0, 255.0) as u8,
                ((average + (blue as f32 - average) * saturation) * exposure).clamp(0.0, 255.0) as u8,
                alpha,
            ];
        }
        Ok(DynamicImage::ImageRgba8(image).adjust_contrast(request.contrast))
    })
}
pub fn watermark(request: &WatermarkRequest, input: PathBuf) -> JobOutcome {
    transform(&request.paths, input, &request.output_location, "-watermarked", |image| {
        apply_watermark(image, &request.options)
    })
}

fn apply_watermark(image: DynamicImage, options: &WatermarkOptions) -> Result<DynamicImage, String> {
    options.validate()?;
    let (x, y) = match options.position {
        Some(WatermarkPosition::Offset { x, y }) => (x, y),
        _ => (16, 16),
    };
    let mut image = image.to_rgba8();
    let original_alpha = image.pixels().map(|pixel| pixel.0[3]).collect::<Vec<_>>();
    let alpha = options.opacity as u16 * 255 / 100;
    let mut watermark = image::RgbaImage::new(image.width(), image.height());
    if let Some(text) = options.text() {
        draw_text(&mut watermark, text, x, y, alpha as u8)?;
    }
    if let Some(path) = options.logo_path.as_ref() {
        let logo = crate::kit::images::decode(path)?.into_first_page().to_rgba8();
        image::imageops::overlay(&mut watermark, &logo, x as i64, y as i64);
    }
    if watermark.pixels().all(|pixel| pixel.0[3] == 0) {
        return Err("Provide watermark text or a logo image.".to_string());
    }
    image::imageops::overlay(&mut image, &watermark, 0, 0);
    for (pixel, alpha) in image.pixels_mut().zip(original_alpha) { pixel.0[3] = alpha; }
    Ok(DynamicImage::ImageRgba8(image))
}

const WATERMARK_FONT_RESOURCE: &str = "watermarkFont";
const WATERMARK_FONT_FILE: &str = "DejaVuSans.ttf";
const WATERMARK_FONT_SIZE: f32 = 32.0;
pub(crate) const INCLUDED_WATERMARK_FONT: &[u8] = include_bytes!("../../../resources/fonts/DejaVuSans.ttf");

/// The bundled Unicode font (DejaVu Sans): a development override or the staged
/// bundle copy when present, otherwise the copy built into the app. Watermarks draw
/// with it and searchable OCR PDFs embed a subset of it.
pub(crate) fn bundled_text_font_bytes() -> Result<Vec<u8>, String> {
    let bundled_root = crate::kit::resources::application_resource_root()
        .map(|root| root.join("resources").join("fonts"));
    if let Some(root) = bundled_root.filter(|root| root.join("manifest.json").is_file()) {
        let resource = crate::kit::resources::resolve(
            WATERMARK_FONT_RESOURCE,
            &root,
            "TOOLBOX_WATERMARK_FONT_PATH",
            WATERMARK_FONT_FILE,
        )?;
        return std::fs::read(&resource.path)
            .map_err(|error| format!("Could not read the bundled font: {error}"));
    }
    Ok(INCLUDED_WATERMARK_FONT.to_vec())
}

fn draw_text(canvas: &mut image::RgbaImage, text: &str, x: u32, y: u32, alpha: u8) -> Result<(), String> {
    let bytes = bundled_text_font_bytes()?;
    let font = fontdue::Font::from_bytes(bytes.clone(), fontdue::FontSettings::default())
        .map_err(|error| format!("Could not parse bundled watermark font: {error}"))?;
    let mut face = rustybuzz::Face::from_slice(&bytes, 0)
        .ok_or_else(|| "Could not parse bundled watermark font for shaping.".to_string())?;
    face.set_pixels_per_em(Some((WATERMARK_FONT_SIZE as u16, WATERMARK_FONT_SIZE as u16)));

    let unsupported = text
        .chars()
        .filter(|character| !font.has_glyph(*character))
        .collect::<BTreeSet<_>>();
    if !unsupported.is_empty() {
        return Err(unsupported_watermark_characters(unsupported));
    }

    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(rustybuzz::Direction::LeftToRight);
    buffer.guess_segment_properties();
    let glyph_buffer = rustybuzz::shape(&face, &[], buffer);
    let glyph_count = u32::from(font.glyph_count());
    let mut unsupported_shaped = BTreeSet::new();
    let mut has_invalid_glyph = false;
    for info in glyph_buffer.glyph_infos() {
        if info.glyph_id == 0 || info.glyph_id >= glyph_count || info.glyph_id > u32::from(u16::MAX) {
            has_invalid_glyph = true;
            if let Some(character) = text.get(info.cluster as usize..).and_then(|value| value.chars().next()) {
                unsupported_shaped.insert(character);
            }
        }
    }
    if !unsupported_shaped.is_empty() {
        return Err(unsupported_watermark_characters(unsupported_shaped));
    }
    if has_invalid_glyph {
        return Err("Unsupported watermark text: shaping produced a missing glyph.".to_string());
    }

    let scale = WATERMARK_FONT_SIZE / face.units_per_em() as f32;
    let ascent = font
        .horizontal_line_metrics(WATERMARK_FONT_SIZE)
        .map(|metrics| metrics.ascent)
        .unwrap_or(WATERMARK_FONT_SIZE * 0.8);
    let baseline_y = y as f32 + ascent;
    let mut pen_x = 0.0_f32;
    for (info, position) in glyph_buffer.glyph_infos().iter().zip(glyph_buffer.glyph_positions()) {
        let glyph_id = info.glyph_id as u16;
        let (metrics, bitmap) = font.rasterize_indexed(glyph_id, WATERMARK_FONT_SIZE);
        let glyph_x = (x as f32
            + (pen_x + position.x_offset as f32) * scale
            + metrics.bounds.xmin)
            .floor() as i64;
        let glyph_y = (baseline_y
            - position.y_offset as f32 * scale
            - metrics.bounds.height
            - metrics.bounds.ymin)
            .floor() as i64;
        for row in 0..metrics.height {
            for column in 0..metrics.width {
                let coverage = bitmap[row * metrics.width + column];
                let glyph_alpha = ((u16::from(coverage) * u16::from(alpha) + 127) / 255) as u8;
                if glyph_alpha == 0 { continue; }
                let px = glyph_x + column as i64;
                let py = glyph_y + row as i64;
                if px < 0 || py < 0 || px >= canvas.width() as i64 || py >= canvas.height() as i64 { continue; }
                blend_watermark_pixel(canvas.get_pixel_mut(px as u32, py as u32), glyph_alpha);
            }
        }
        pen_x += position.x_advance as f32;
    }
    Ok(())
}

fn blend_watermark_pixel(pixel: &mut Rgba<u8>, source_alpha: u8) {
    let source_alpha = u16::from(source_alpha);
    let destination_alpha = u16::from(pixel.0[3]);
    let output_alpha = source_alpha + ((destination_alpha * (255 - source_alpha) + 127) / 255);
    pixel.0 = [255, 255, 255, output_alpha.min(255) as u8];
}

fn unsupported_watermark_characters(characters: impl IntoIterator<Item = char>) -> String {
    let details = characters
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|character| {
            let codepoint = format!("U+{:04X}", character as u32);
            if character.is_control() || character.is_whitespace() {
                format!("{codepoint} ({character:?})")
            } else {
                format!("{codepoint} ({character})")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("Unsupported watermark characters: {details}.")
}
/// Makes one master's icon set in its own `{stem}-icons` folder, with a folder for the
/// platform inside. The folder is built in a hidden staging folder and published in one
/// rename only when every size worked, so a failed master leaves nothing behind.
pub fn icon_set(request: &IconSetRequest, input: PathBuf) -> JobOutcome {
    icon_set_with(request, input, |icon, output| icon.save_with_format(output, ImageFormat::Png).map_err(|error| error.to_string()))
}

fn icon_set_with(request: &IconSetRequest, input: PathBuf, mut save: impl FnMut(&DynamicImage, &Path) -> Result<(), String>) -> JobOutcome {
    let plan = match icon_plan(&request.preset, &request.sizes) { Ok(plan) => plan, Err(error) => return failure(input, error) };
    let master = match crate::kit::images::decode(&input) {
        Ok(source) => DynamicImage::ImageRgba8(source.into_first_page().to_rgba8()),
        Err(_) => return failure(input.clone(), format!("Not a readable {} image. The file may be damaged. Nothing was saved.", image_kind(&input))),
    };
    let folder = match OutputNaming::reserve_staged_folder(&input, &request.output_location, ICON_SET_SUFFIX) {
        Ok(folder) => folder,
        Err(error) => return failure(input, format!("Could not make the icon folder: {error}. Nothing was saved for this image.")),
    };
    let platform = folder.path().join(plan.folder);
    if let Err(error) = std::fs::create_dir(&platform) {
        return failure(input, format!("Could not make the {} folder: {error}. Nothing was saved for this image.", plan.folder));
    }
    let mut pngs = Vec::with_capacity(plan.sizes.len());
    for &size in &plan.sizes {
        let output = platform.join(format!("{}-{size}.png", plan.file_prefix));
        let icon = master.resize_exact(size, size, image::imageops::FilterType::Lanczos3);
        if let Err(error) = save(&icon, &output) {
            return failure(input, format!("The {} {size} size failed: {error}. Nothing was saved for this image.", plan.label));
        }
        pngs.push(output);
    }
    if request.preset == "macos" {
        let stem = input.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        if let Err(error) = write_icns(&platform.join(format!("{stem}.icns")), &plan.sizes, &pngs) {
            return failure(input, format!("The macOS ICNS file failed: {error}. Nothing was saved for this image."));
        }
    }
    let files = pngs.len() + usize::from(request.preset == "macos");
    let expected = folder.destination_path().with_file_name(format!("{}{ICON_SET_SUFFIX}", input.file_stem().map(|stem| stem.to_string_lossy()).unwrap_or_default()));
    match folder.publish() {
        Ok(published) => crate::kit::common::note_renamed(JobOutcome::succeeded(input, vec![published], format!("{files} files")), Some(&expected)),
        Err(error) => failure(input, format!("Could not save the icon folder: {error}. Nothing was saved for this image.")),
    }
}

const ICON_SET_SUFFIX: &str = "-icons";

fn image_kind(path: &Path) -> &'static str {
    match crate::kit::images::ImageFamily::of(path) {
        crate::kit::images::ImageFamily::Png => "PNG",
        crate::kit::images::ImageFamily::Jpeg => "JPEG",
        crate::kit::images::ImageFamily::WebP => "WebP",
        crate::kit::images::ImageFamily::Heif => "HEIC",
        crate::kit::images::ImageFamily::Tiff => "TIFF",
        crate::kit::images::ImageFamily::Other => "",
    }
}

fn write_icns(output: &Path, sizes: &[u32], pngs: &[PathBuf]) -> Result<(), String> {
    let mut items = Vec::new();
    for (size, png) in sizes.iter().zip(pngs) {
        let type_code = match size {
            16 => *b"icp4",
            32 => *b"icp5",
            128 => *b"ic07",
            256 => *b"ic08",
            512 => *b"ic09",
            1024 => *b"ic10",
            _ => return Err(format!("Unsupported ICNS image size: {size}")),
        };
        let payload = std::fs::read(png).map_err(|error| format!("Could not read generated PNG: {error}"))?;
        let item_length = payload.len().checked_add(8).ok_or_else(|| "ICNS item is too large.".to_string())?;
        let item_length = u32::try_from(item_length).map_err(|_| "ICNS item is too large.".to_string())?;
        items.push((type_code, item_length, payload));
    }
    let total_length = 8usize.checked_add(items.iter().map(|(_, length, _)| *length as usize).sum::<usize>()).ok_or_else(|| "ICNS container is too large.".to_string())?;
    let total_length = u32::try_from(total_length).map_err(|_| "ICNS container is too large.".to_string())?;
    let mut bytes = Vec::with_capacity(total_length as usize);
    bytes.extend_from_slice(b"icns");
    bytes.extend_from_slice(&total_length.to_be_bytes());
    for (type_code, item_length, payload) in items {
        bytes.extend_from_slice(&type_code);
        bytes.extend_from_slice(&item_length.to_be_bytes());
        bytes.extend_from_slice(&payload);
    }
    std::fs::write(output, bytes).map_err(|error| error.to_string())
}

fn default_icon_preset() -> String { "custom".to_string() }

/// Where a preset's icons go inside the set folder, how they are named, and the name a
/// failure uses for the platform.
#[derive(Debug, PartialEq, Eq)]
struct IconPlan {
    folder: &'static str,
    file_prefix: &'static str,
    label: &'static str,
    sizes: Vec<u32>,
}

fn icon_plan(preset: &str, custom_sizes: &[u32]) -> Result<IconPlan, String> {
    let (folder, file_prefix, label, sizes) = match preset {
        "macos" => ("macos", "macos", "macOS", vec![16, 32, 128, 256, 512, 1024]),
        "favicon" => ("web", "favicon", "Website", vec![16, 32, 48, 64, 128, 256]),
        "ios" => ("ios", "ios", "iOS", vec![20, 29, 40, 60, 76, 83, 1024]),
        "android" => ("android", "android", "Android", vec![48, 72, 96, 144, 192, 512]),
        "custom" => ("custom", "custom", "custom", custom_sizes.iter().copied().filter(|size| *size > 0 && *size <= 4096).collect::<BTreeSet<_>>().into_iter().collect()),
        _ => return Err(format!("Unsupported icon preset: {preset}.")),
    };
    if sizes.is_empty() { return Err("Select at least one custom icon size from 1 to 4096 pixels.".to_string()); }
    Ok(IconPlan { folder, file_prefix, label, sizes })
}

pub fn gif_create(request: &GifCreateRequest) -> JobOutcome {
    let Some(input) = request.paths.first().cloned() else { return failure(PathBuf::new(), "Select at least one image.".to_string()); };
    let (output, file) = match OutputNaming::create_single_output(request.output_path.as_deref(), &input, &request.output_location, "-animated", "gif") {
        Ok(destination) => destination,
        Err(error) => return failure(input, error.to_string()),
    };
    let encode_result = {
        let mut encoder = image::codecs::gif::GifEncoder::new(file);
        if request.loop_forever { if let Err(error) = encoder.set_repeat(image::codecs::gif::Repeat::Infinite) { return failure(input, format!("Could not configure GIF loop: {error}")); } }
        let mut images = Vec::with_capacity(request.paths.len());
        for path in &request.paths {
            match image::open(path) {
                Ok(image) => images.push(image.to_rgba8()),
                Err(error) => {
                    return failure(path.clone(), format!("Could not create GIF from {}: {error}", path.display()));
                }
            }
        }
        let width = images.iter().map(|image| image.width()).max().unwrap_or(0);
        let height = images.iter().map(|image| image.height()).max().unwrap_or(0);
        let delay = image::Delay::from_numer_denom_ms(request.frame_delay_ms.clamp(1, 60_000), 1);
        let frames = images.into_iter().map(|image| { let mut canvas = image::RgbaImage::new(width, height); image::imageops::overlay(&mut canvas, &image, 0, 0); image::Frame::from_parts(canvas, 0, 0, delay) }).collect::<Vec<_>>();
        encoder.encode_frames(frames)
    };
    match encode_result {
        Ok(_) => match output.publish() {
            Ok(path) => JobOutcome::succeeded(input, vec![path], "GIF saved"),
            Err(error) => failure(input, format!("Could not publish GIF output: {error}")),
        },
        Err(error) => failure(input, format!("Could not create GIF: {error}")),
    }
}

fn default_gif_delay_ms() -> u32 { 100 }
fn default_gif_loop() -> bool { true }

pub fn gif_extract(request: &GifExtractRequest, input: PathBuf) -> JobOutcome {
    let file = match File::open(&input) { Ok(file) => file, Err(error) => return failure(input, error.to_string()) };
    let decoder = match image::codecs::gif::GifDecoder::new(BufReader::new(file)) { Ok(decoder) => decoder, Err(error) => return failure(input, error.to_string()) };
    let frames = match decoder.into_frames().collect_frames() { Ok(frames) => frames, Err(error) => return failure(input, error.to_string()) };
    let mut outputs = Vec::new();
    let mut timing = Vec::new();
    for (index, frame) in frames.into_iter().enumerate() {
        let output = match OutputNaming::reserve_destination(&input, &request.output_location, &format!("-frame-{}", index + 1), "png") {
            Ok(output) => output,
            Err(error) => return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not reserve GIF frame output: {error}"))),
        };
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay_ms = (numerator as u64 * 1000 / denominator.max(1) as u64).max(1);
        if let Err(error) = frame.into_buffer().save(output.path()) {
            return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(error.to_string()));
        }
        let output = match output.publish() {
            Ok(output) => output,
            Err(error) => return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not publish GIF frame output: {error}"))),
        };
        timing.push(serde_json::json!({ "file": output, "delayMs": delay_ms }));
        outputs.push(output);
    }
    let timing_path = match OutputNaming::reserve_destination(&input, &request.output_location, "-frame-timing", "json") {
        Ok(output) => output,
        Err(error) => return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not reserve GIF timing output: {error}"))),
    };
    let timing_bytes = match serde_json::to_vec_pretty(&timing) {
        Ok(bytes) => bytes,
        Err(error) => return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not encode GIF timing manifest: {error}"))),
    };
    if let Err(error) = std::fs::write(timing_path.path(), timing_bytes) {
        return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not save GIF timing manifest: {error}")));
    }
    outputs.push(match timing_path.publish() {
        Ok(path) => path,
        Err(error) => return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not publish GIF timing manifest: {error}"))),
    });
    JobOutcome::succeeded(input, outputs, "GIF frames saved")
}

pub fn tiff(request: &TiffRequest) -> JobOutcome {
    let Some(input) = request.paths.first().cloned() else { return failure(PathBuf::new(), "Select at least one TIFF file.".to_string()); };
    let pages = if let Some(page_refs) = &request.pages {
        if page_refs.is_empty() { return failure(input, "Select at least one TIFF page.".to_string()); }
        let mut selected = Vec::with_capacity(page_refs.len());
        for page_ref in page_refs {
            if !request.paths.iter().any(|path| path == &page_ref.path) {
                return failure(page_ref.path.clone(), "The TIFF page selection contains an unselected file.".to_string());
            }
            let decoded = match crate::kit::images::decode(&page_ref.path) {
                Ok(decoded) => decoded.pages,
                Err(error) => return failure(page_ref.path.clone(), format!("Could not process {}: {error}", page_ref.path.display())),
            };
            let Some(page) = decoded.get(page_ref.page) else {
                return failure(page_ref.path.clone(), format!("TIFF page {} is outside the document.", page_ref.page + 1));
            };
            selected.push(page.clone());
        }
        selected
    } else {
        let mut pages = Vec::new();
        for path in &request.paths {
            match crate::kit::images::decode(path) {
                Ok(mut decoded) => pages.append(&mut decoded.pages),
                Err(error) => return failure(path.clone(), format!("Could not process {}: {error}", path.display())),
            }
        }
        pages
    };
    let output = if request.paths.len() == 1 {
        let mut outputs = Vec::new();
        for (index, page) in pages.iter().enumerate() {
            let output = match OutputNaming::reserve_destination(&input, &request.output_location, &format!("-page-{}", index + 1), "tiff") {
                Ok(output) => output,
                Err(error) => return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not reserve TIFF page output: {error}"))),
            };
            if let Err(error) = write_tiff_page(output.path(), page) {
                return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not write TIFF page: {error}")));
            }
            outputs.push(match output.publish() {
                Ok(path) => path,
                Err(error) => return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not publish TIFF page: {error}"))),
            });
        }
        return JobOutcome::succeeded(input, outputs, "TIFF pages saved");
    } else {
        match OutputNaming::reserve_single_output(request.output_path.as_deref(), &input, &request.output_location, "-combined", "tiff") {
            Ok(output) => output,
            Err(error) => return failure(input, format!("Could not reserve combined TIFF output: {error}")),
        }
    };
    if let Err(error) = write_tiff_pages(output.path(), &pages) { return failure(input, format!("Could not combine TIFF pages: {error}")); }
    match output.publish() {
        Ok(path) => JobOutcome::succeeded(input, vec![path], "TIFF pages combined"),
        Err(error) => failure(input, format!("Could not publish combined TIFF: {error}")),
    }
}

fn write_tiff_pages(path: &std::path::Path, pages: &[DynamicImage]) -> Result<(), String> {
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut encoder = tiff::encoder::TiffEncoder::new(file).map_err(|error| error.to_string())?;
    for page in pages { write_encoded_page(&mut encoder, page)?; }
    Ok(())
}

fn write_tiff_page(path: &std::path::Path, page: &DynamicImage) -> Result<(), String> {
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut encoder = tiff::encoder::TiffEncoder::new(file).map_err(|error| error.to_string())?;
    write_encoded_page(&mut encoder, page)
}

fn write_encoded_page<W: std::io::Write + Seek>(encoder: &mut tiff::encoder::TiffEncoder<W>, page: &DynamicImage) -> Result<(), String> {
    match page {
        DynamicImage::ImageLuma8(page) => encoder.write_image::<tiff::encoder::colortype::Gray8>(page.width(), page.height(), page.as_raw()),
        DynamicImage::ImageRgb8(page) => encoder.write_image::<tiff::encoder::colortype::RGB8>(page.width(), page.height(), page.as_raw()),
        DynamicImage::ImageRgba8(page) => encoder.write_image::<tiff::encoder::colortype::RGBA8>(page.width(), page.height(), page.as_raw()),
        page => return Err(format!("Unsupported TIFF page color type: {:?}", page.color())),
    }
    .map_err(|error| error.to_string())
}
pub fn strip_metadata(request: &MetadataRequest, input: PathBuf) -> JobOutcome {
    let image = match image::open(&input) { Ok(image) => image, Err(error) => return failure(input, format!("Could not read image: {error}")) };
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, "-stripped", input.extension().and_then(|extension| extension.to_str()).unwrap_or("png")) {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve metadata-free image output: {error}")),
    };
    let format = ImageFormat::from_path(output.path()).unwrap_or(ImageFormat::Png);
    if let Err(error) = image.save_with_format(output.path(), format) { return failure(input, format!("Could not save metadata-free image: {error}")); }
    match crate::kit::images::metadata::read_file(output.path()) {
        Ok(report) if !report.exif && !report.xmp && !report.icc => match output.publish() {
            Ok(path) => JobOutcome::succeeded(input, vec![path], "Metadata removed and verified"),
            Err(error) => failure(input, format!("Could not publish metadata-free image: {error}")),
        },
        Ok(_) => failure(input, "Image encoder retained metadata that could not be removed safely.".to_string()),
        Err(error) => failure(input, format!("Could not verify metadata removal: {error}")),
    }
}

fn transform<F>(_: &[PathBuf], input: PathBuf, location: &OutputLocation, suffix: &str, edit: F) -> JobOutcome where F: FnOnce(DynamicImage) -> Result<DynamicImage, String> {
    let image = match crate::kit::images::decode(&input) { Ok(decoded) => decoded.into_first_page(), Err(error) => return failure(input, format!("Could not read image: {error}")) };
    let image = match edit(image) { Ok(image) => image, Err(error) => return failure(input, error) };
    let extension = input.extension().and_then(|extension| extension.to_str()).unwrap_or("png");
    let output = match OutputNaming::reserve_destination(&input, location, suffix, extension) {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve image output: {error}")),
    };
    let format = ImageFormat::from_path(output.path()).unwrap_or(ImageFormat::Png);
    match image.save_with_format(output.path(), format) {
        Ok(_) => match output.publish() {
            Ok(path) => JobOutcome::succeeded(input, vec![path], "Image saved"),
            Err(error) => failure(input, format!("Could not publish image: {error}")),
        },
        Err(error) => failure(input, format!("Could not save image: {error}")),
    }
}

fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::processing(error)) }

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    fn path(name: &str) -> PathBuf { std::env::temp_dir().join(format!("toolbox_tiff_{}_{}", std::process::id(), name)) }
    fn gray(width: u32, height: u32, data: Vec<u8>) -> DynamicImage { DynamicImage::ImageLuma8(image::ImageBuffer::from_raw(width, height, data).unwrap()) }
    fn read_pages(path: &Path) -> Result<Vec<DynamicImage>, String> { crate::kit::images::decode(path).map(|decoded| decoded.pages) }

    fn text_watermark(input: &Path, text: &str, opacity: u8, offset: u32) -> WatermarkRequest {
        WatermarkRequest {
            paths: vec![input.to_path_buf()],
            options: WatermarkOptions { text: Some(text.to_string()), logo_path: None, size: None, opacity, color: None, position: Some(WatermarkPosition::Offset { x: offset, y: offset }), rotation: 0.0 },
            output_location: OutputLocation::AlongsideInput,
        }
    }

    #[test]
    fn splits_and_combines_pages_in_order() {
        let input = path("source.tiff");
        let file = File::create(&input).unwrap();
        let mut encoder = tiff::encoder::TiffEncoder::new(file).unwrap();
        encoder.write_image::<tiff::encoder::colortype::Gray8>(2, 1, &[10, 20]).unwrap();
        encoder.write_image::<tiff::encoder::colortype::Gray8>(2, 1, &[30, 40]).unwrap();

        let split = tiff(&TiffRequest { paths: vec![input.clone()], pages: None, output_location: OutputLocation::AlongsideInput, output_path: None });
        assert_eq!(split.output_paths().len(), 2);
        assert_eq!(read_pages(&split.output_paths()[0]).unwrap().len(), 1);
        assert_eq!(read_pages(&split.output_paths()[1]).unwrap().len(), 1);

        let first = path("first.tiff");
        let second = path("second.tiff");
        write_tiff_page(&first, &gray(1, 1, vec![1])).unwrap();
        write_tiff_page(&second, &gray(1, 1, vec![2])).unwrap();
        let combined = tiff(&TiffRequest { paths: vec![first.clone(), second.clone()], pages: None, output_location: OutputLocation::AlongsideInput, output_path: None });
        let pages = read_pages(&combined.output_paths()[0]).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].as_bytes(), &[1]);
        assert_eq!(pages[1].as_bytes(), &[2]);

        for output in split.output_paths() { let _ = std::fs::remove_file(output); }
        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
        let _ = std::fs::remove_file(combined.output_paths()[0].clone());
    }

    #[test]
    fn rejects_unsupported_tiff_bit_depths() {
        let input = path("unsupported.tiff");
        let file = File::create(&input).unwrap();
        let mut encoder = tiff::encoder::TiffEncoder::new(file).unwrap();
        encoder.write_image::<tiff::encoder::colortype::Gray16>(1, 1, &[1]).unwrap();
        let result = tiff(&TiffRequest { paths: vec![input.clone()], pages: None, output_location: OutputLocation::AlongsideInput, output_path: None });
        assert!(result.error().is_some());
        let _ = std::fs::remove_file(input);
    }

    #[test]
    fn selected_tiff_pages_follow_explicit_cross_file_order() {
        let two_page = path("ordered-two-page.tiff");
        let one_page = path("ordered-one-page.tiff");
        let file = File::create(&two_page).unwrap();
        let mut encoder = tiff::encoder::TiffEncoder::new(file).unwrap();
        encoder.write_image::<tiff::encoder::colortype::Gray8>(2, 1, &[10, 20]).unwrap();
        encoder.write_image::<tiff::encoder::colortype::Gray8>(2, 1, &[30, 40]).unwrap();
        write_tiff_page(&one_page, &gray(1, 1, vec![2])).unwrap();

        let outcome = tiff(&TiffRequest {
            paths: vec![two_page.clone(), one_page.clone()],
            pages: Some(vec![
                TiffPageRef { path: two_page.clone(), page: 1 },
                TiffPageRef { path: one_page.clone(), page: 0 },
                TiffPageRef { path: two_page.clone(), page: 0 },
            ]),
            output_location: OutputLocation::AlongsideInput, output_path: None,
        });

        assert!(outcome.error().is_none(), "selected TIFF pages failed: {:?}", outcome.error());
        let output = outcome.output_paths().first().unwrap().clone();
        let pages = read_pages(&output).unwrap();
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0].as_bytes(), &[30, 40]);
        assert_eq!(pages[1].as_bytes(), &[2]);
        assert_eq!(pages[2].as_bytes(), &[10, 20]);

        let _ = std::fs::remove_file(two_page);
        let _ = std::fs::remove_file(one_page);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn text_watermark_composites_without_changing_alpha() {
        let input = path("watermark-input.png");
        let image = image::RgbaImage::from_pixel(64, 32, image::Rgba([10, 20, 30, 77]));
        image.save(&input).unwrap();
        let result = watermark(&text_watermark(&input, "TOOL", 50, 2), input.clone());
        let output = result.output_paths().first().unwrap();
        let output_image = image::open(output).unwrap().to_rgba8();
        assert!(output_image.pixels().any(|pixel| pixel.0[0..3] != [10, 20, 30]));
        assert!(output_image.pixels().all(|pixel| pixel.0[3] == 77));
        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn text_watermark_supports_ascii_latin_combining_and_non_latin_text() {
        let mut canvas = image::RgbaImage::new(640, 96);
        draw_text(&mut canvas, "ASCII Café e\u{301} Ж", 4, 4, 200).unwrap();
        assert!(canvas.pixels().any(|pixel| pixel.0[3] > 0));

        for text in ["Watermark", "Café", "e\u{301}", "Ж"] {
            let mut sample = image::RgbaImage::new(160, 64);
            draw_text(&mut sample, text, 4, 4, 255).unwrap();
            assert!(sample.pixels().any(|pixel| pixel.0[3] > 0), "{text:?} did not rasterize");
        }
    }

    #[test]
    fn unsupported_mixed_text_fails_without_reserving_or_writing_output() {
        let input = path("watermark-unsupported.png");
        let stem = input.file_stem().and_then(|value| value.to_str()).unwrap();
        let output = input.with_file_name(format!("{stem}-watermarked.png"));
        let alternate_output = input.with_file_name(format!("{stem}-watermarked-1.png"));
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&output);
        let _ = std::fs::remove_file(&alternate_output);
        image::RgbaImage::from_pixel(64, 64, image::Rgba([10, 20, 30, 91])).save(&input).unwrap();
        let source_bytes = std::fs::read(&input).unwrap();
        let existing_bytes = b"existing output must remain intact";
        std::fs::write(&output, existing_bytes).unwrap();

        let result = watermark(&text_watermark(&input, "Café Ж \u{10FFFF}", 80, 4), input.clone());

        let error = result.error().expect("unsupported text must fail");
        assert!(error.message.contains("Unsupported watermark characters"));
        assert!(error.message.contains("U+10FFFF"));
        assert!(result.output_paths().is_empty());
        assert_eq!(std::fs::read(&input).unwrap(), source_bytes);
        assert_eq!(std::fs::read(&output).unwrap(), existing_bytes);
        assert!(!alternate_output.exists());

        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
        let _ = std::fs::remove_file(alternate_output);
    }

    #[test]
    fn repeated_text_watermark_rendering_is_deterministic() {
        fn digest(text: &str) -> [u8; 32] {
            let mut canvas = image::RgbaImage::new(320, 96);
            draw_text(&mut canvas, text, 8, 8, 173).unwrap();
            Sha256::digest(canvas.as_raw()).into()
        }

        assert_eq!(digest("Café e\u{301} Ж"), digest("Café e\u{301} Ж"));
    }

    #[test]
    fn watermark_preserves_source_alpha_and_existing_outputs_on_collision() {
        let input = path("watermark-safety.png");
        let stem = input.file_stem().and_then(|value| value.to_str()).unwrap();
        let existing_output = input.with_file_name(format!("{stem}-watermarked.png"));
        let expected_output = input.with_file_name(format!("{stem}-watermarked-1.png"));
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&existing_output);
        image::RgbaImage::from_pixel(64, 64, image::Rgba([10, 20, 30, 91])).save(&input).unwrap();
        let source_bytes = std::fs::read(&input).unwrap();
        let existing_bytes = b"do not overwrite this output";
        std::fs::write(&existing_output, existing_bytes).unwrap();

        let result = watermark(&text_watermark(&input, "Safe", 50, 4), input.clone());

        assert!(result.error().is_none(), "watermark failed: {:?}", result.error());
        let output = result.output_paths().first().expect("watermark output");
        assert_eq!(output, &expected_output);
        assert_eq!(std::fs::read(&input).unwrap(), source_bytes);
        assert_eq!(std::fs::read(&existing_output).unwrap(), existing_bytes);
        let output_image = image::open(output).unwrap().to_rgba8();
        assert!(output_image.pixels().any(|pixel| pixel.0[0..3] != [10, 20, 30]));
        assert!(output_image.pixels().all(|pixel| pixel.0[3] == 91));

        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(existing_output);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn gif_creation_preserves_order_timing_and_canvas() {
        let first = path("gif-first.png");
        let second = path("gif-second.png");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 128])).save(&first).unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 255, 0, 255])).save(&second).unwrap();
        let result = gif_create(&GifCreateRequest { paths: vec![first.clone(), second.clone()], frame_delay_ms: 250, loop_forever: true, output_location: OutputLocation::AlongsideInput, output_path: None });
        let output = result.output_paths().first().unwrap();
        let file = File::open(output).unwrap();
        let decoder = image::codecs::gif::GifDecoder::new(BufReader::new(file)).unwrap();
        let frames = decoder.into_frames().collect_frames().unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].buffer().dimensions(), (2, 2));
        assert_eq!(frames[0].delay().numer_denom_ms(), (250, 1));
        assert_eq!(frames[1].buffer().get_pixel(0, 0).0[1], 255);
        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn concurrent_gif_jobs_with_same_named_inputs_never_share_an_output() {
        use std::collections::HashSet;
        use std::sync::{Arc, Barrier};

        let root = std::env::temp_dir().join(format!(
            "toolbox_gif_collision_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output_directory = root.join("outputs");
        std::fs::create_dir_all(&output_directory).unwrap();
        let existing_output = output_directory.join("frame-animated.gif");
        let existing_bytes = b"existing output must remain intact";
        std::fs::write(&existing_output, existing_bytes).unwrap();

        let worker_count = 8usize;
        let mut inputs = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            let input_directory = root.join(format!("input-{index}"));
            std::fs::create_dir_all(&input_directory).unwrap();
            let input = input_directory.join("frame.png");
            image::RgbaImage::from_pixel(2, 2, image::Rgba([index as u8 * 24, 80, 160, 255]))
                .save(&input)
                .unwrap();
            inputs.push(input);
        }

        let barrier = Arc::new(Barrier::new(worker_count));
        let handles = inputs
            .iter()
            .cloned()
            .map(|input| {
                let barrier = Arc::clone(&barrier);
                let output_directory = output_directory.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    gif_create(&GifCreateRequest {
                        paths: vec![input],
                        frame_delay_ms: 100,
                        loop_forever: true,
                        output_location: OutputLocation::CustomFolder(output_directory), output_path: None,
                    })
                })
            })
            .collect::<Vec<_>>();
        let outcomes = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();

        assert!(outcomes.iter().all(|outcome| outcome.error().is_none()));
        let outputs = outcomes
            .iter()
            .map(|outcome| outcome.output_paths()[0].clone())
            .collect::<Vec<_>>();
        assert_eq!(outputs.iter().collect::<HashSet<_>>().len(), worker_count);
        assert_eq!(std::fs::read(&existing_output).unwrap(), existing_bytes);
        for output in &outputs {
            let decoder = image::codecs::gif::GifDecoder::new(BufReader::new(File::open(output).unwrap())).unwrap();
            let frames = decoder.into_frames().collect_frames().unwrap();
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].buffer().dimensions(), (2, 2));
            assert!(std::fs::metadata(output).unwrap().len() > 0);
        }
        let output_entries = std::fs::read_dir(&output_directory).unwrap().count();
        assert_eq!(output_entries, worker_count + 1);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_gif_job_removes_reserved_output_and_claim() {
        let root = std::env::temp_dir().join(format!(
            "toolbox_gif_cleanup_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let input_directory = root.join("input");
        let output_directory = root.join("outputs");
        std::fs::create_dir_all(&input_directory).unwrap();
        std::fs::create_dir_all(&output_directory).unwrap();
        let input = input_directory.join("frame.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 40, 60, 255]))
            .save(&input)
            .unwrap();
        let existing_output = output_directory.join("frame-animated.gif");
        let existing_bytes = b"existing output must remain intact";
        std::fs::write(&existing_output, existing_bytes).unwrap();

        let result = gif_create(&GifCreateRequest {
            paths: vec![input, input_directory.join("missing.png")],
            frame_delay_ms: 100,
            loop_forever: true,
            output_location: OutputLocation::CustomFolder(output_directory.clone()), output_path: None,
        });

        assert!(result.error().is_some());
        assert_eq!(std::fs::read(&existing_output).unwrap(), existing_bytes);
        assert_eq!(std::fs::read_dir(&output_directory).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn icon_presets_are_named_and_reject_invalid_custom_sizes() {
        let macos = icon_plan("macos", &[]).unwrap();
        assert_eq!((macos.folder, macos.file_prefix, macos.label), ("macos", "macos", "macOS"));
        assert_eq!(macos.sizes, vec![16, 32, 128, 256, 512, 1024]);
        assert_eq!(icon_plan("custom", &[300, 0, 16, 300, 5000]).unwrap().sizes, vec![16, 300]);
        assert!(icon_plan("custom", &[0, 5000]).is_err());
        assert!(icon_plan("unknown", &[]).is_err());
    }

    fn icon_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("toolbox_icon_set_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A 64 px master whose left half is fully transparent and right half opaque.
    fn write_master(path: &Path) -> Vec<u8> {
        let master = image::RgbaImage::from_fn(64, 64, |x, _| if x < 32 { image::Rgba([0, 0, 0, 0]) } else { image::Rgba([200, 40, 90, 255]) });
        master.save(path).unwrap();
        std::fs::read(path).unwrap()
    }

    fn icon_request(paths: &[PathBuf], preset: &str, sizes: Vec<u32>) -> IconSetRequest {
        IconSetRequest { paths: paths.to_vec(), preset: preset.to_string(), sizes, output_location: OutputLocation::AlongsideInput }
    }

    fn entry_names(folder: &Path) -> BTreeSet<String> {
        std::fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn each_preset_saves_exactly_its_sizes_with_alpha_in_one_icons_folder() {
        let root = icon_root("presets");
        let input = root.join("app-icon.png");
        let original = write_master(&input);
        let cases: [(&str, Vec<u32>, &str, Vec<u32>); 5] = [
            ("macos", vec![], "macos", vec![16, 32, 128, 256, 512, 1024]),
            ("favicon", vec![], "web", vec![16, 32, 48, 64, 128, 256]),
            ("ios", vec![], "ios", vec![20, 29, 40, 60, 76, 83, 1024]),
            ("android", vec![], "android", vec![48, 72, 96, 144, 192, 512]),
            ("custom", vec![300, 24], "custom", vec![24, 300]),
        ];
        for (preset, sizes, folder, expected) in cases {
            let outcome = icon_set(&icon_request(std::slice::from_ref(&input), preset, sizes), input.clone());
            assert!(outcome.error().is_none(), "{preset}: {:?}", outcome.error());
            assert!(outcome.warnings.is_empty(), "{preset}");
            let set = root.join("app-icon-icons");
            assert_eq!(outcome.output_paths(), [set.clone()]);
            assert_eq!(entry_names(&set), BTreeSet::from([folder.to_string()]));
            let prefix = if preset == "favicon" { "favicon" } else { folder };
            let mut names: BTreeSet<String> = expected.iter().map(|size| format!("{prefix}-{size}.png")).collect();
            if preset == "macos" { names.insert("app-icon.icns".to_string()); }
            assert_eq!(entry_names(&set.join(folder)), names, "{preset}");
            for size in &expected {
                let icon = image::open(set.join(folder).join(format!("{prefix}-{size}.png"))).unwrap();
                assert_eq!(icon.color(), image::ColorType::Rgba8, "{preset} {size}");
                let icon = icon.to_rgba8();
                assert_eq!(icon.dimensions(), (*size, *size), "{preset} {size}");
                assert_eq!(icon.get_pixel(0, size / 2)[3], 0, "{preset} {size} keeps the transparent side");
                assert_eq!(icon.get_pixel(size - 1, size / 2)[3], 255, "{preset} {size} keeps the opaque side");
            }
            assert_eq!(outcome.detail, format!("{} files", names.len()));
            std::fs::remove_dir_all(&set).unwrap();
        }
        assert_eq!(std::fs::read(&input).unwrap(), original);
        assert_eq!(entry_names(&root), BTreeSet::from(["app-icon.png".to_string()]));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn macos_icon_set_includes_a_valid_icns_container() {
        let root = icon_root("icns");
        let input = root.join("logo.png");
        write_master(&input);
        let outcome = icon_set(&icon_request(std::slice::from_ref(&input), "macos", vec![]), input.clone());
        assert!(outcome.error().is_none());
        let bytes = std::fs::read(root.join("logo-icons/macos/logo.icns")).unwrap();
        assert!(bytes.starts_with(b"icns"));
        assert_eq!(u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize, bytes.len());
        for code in [b"icp4", b"icp5", b"ic07", b"ic08", b"ic09", b"ic10"] {
            assert!(bytes.windows(4).any(|item| item == code));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_existing_icons_folder_is_kept_and_the_set_gets_a_numbered_folder() {
        let root = icon_root("collision");
        let input = root.join("brand-mark.png");
        write_master(&input);
        let existing = root.join("brand-mark-icons");
        std::fs::create_dir(&existing).unwrap();
        std::fs::write(existing.join("keep.txt"), b"mine").unwrap();
        let outcome = icon_set(&icon_request(std::slice::from_ref(&input), "android", vec![]), input.clone());
        assert_eq!(outcome.output_paths(), [root.join("brand-mark-icons-1")]);
        assert_eq!(outcome.warnings.iter().map(|warning| warning.kind.clone()).collect::<Vec<_>>(), [crate::kit::contracts::WarningKind::RenamedToAvoidReplacing { chosen_name: "brand-mark-icons".into(), final_name: "brand-mark-icons-1".into() }]);
        assert_eq!(entry_names(&existing), BTreeSet::from(["keep.txt".to_string()]));
        assert_eq!(std::fs::read(existing.join("keep.txt")).unwrap(), b"mine");
        assert_eq!(entry_names(&root.join("brand-mark-icons-1/android")).len(), 6);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_size_saves_nothing_for_that_master_and_names_the_size_while_others_succeed() {
        let root = icon_root("forced_failure");
        let inputs = ["first", "broken", "last"].map(|name| root.join(format!("{name}.png")));
        let originals = inputs.iter().map(|input| write_master(input)).collect::<Vec<_>>();
        let request = icon_request(&inputs, "macos", vec![]);
        let outcomes = inputs.iter().map(|input| {
            let broken = input.ends_with("broken.png");
            icon_set_with(&request, input.clone(), |icon, output| {
                if broken && icon.width() == 1024 { return Err("the image data is damaged".into()); }
                icon.save_with_format(output, ImageFormat::Png).map_err(|error| error.to_string())
            })
        }).collect::<Vec<_>>();
        assert!(outcomes[0].error().is_none() && outcomes[2].error().is_none());
        assert_eq!(outcomes[1].error().unwrap().message, "The macOS 1024 size failed: the image data is damaged. Nothing was saved for this image.");
        assert!(outcomes[1].output_paths().is_empty());
        assert_eq!(entry_names(&root), BTreeSet::from(["first.png", "broken.png", "last.png", "first-icons", "last-icons"].map(String::from)));
        assert_eq!(entry_names(&root.join("last-icons/macos")).len(), 7);
        for (input, original) in inputs.iter().zip(originals) { assert_eq!(std::fs::read(input).unwrap(), original); }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_corrupt_master_fails_alone_and_saves_nothing() {
        let root = icon_root("corrupt");
        let good = root.join("good.png");
        let original = write_master(&good);
        let corrupt = root.join("damaged.webp");
        std::fs::write(&corrupt, b"RIFF\x10\0\0\0WEBPnot really an image").unwrap();
        let request = icon_request(&[good.clone(), corrupt.clone()], "favicon", vec![]);
        let failed = icon_set(&request, corrupt.clone());
        let saved = icon_set(&request, good.clone());
        assert_eq!(failed.error().unwrap().message, "Not a readable WebP image. The file may be damaged. Nothing was saved.");
        assert!(saved.error().is_none());
        assert_eq!(entry_names(&root), BTreeSet::from(["good.png", "damaged.webp", "good-icons"].map(String::from)));
        assert_eq!(std::fs::read(&corrupt).unwrap(), b"RIFF\x10\0\0\0WEBPnot really an image");
        assert_eq!(std::fs::read(&good).unwrap(), original);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn crop_aspect_ratio_is_anchored_and_bounds_checked() {
        let request = CropRequest { paths: vec![], x: 0, y: 0, width: 1, height: 1, mode: "aspectRatio".to_string(), aspect_width: 1, aspect_height: 1, anchor: "top".to_string(), output_location: OutputLocation::AlongsideInput };
        assert_eq!(crop_rect(8, 4, &request).unwrap(), (2, 0, 4, 4));
        let invalid = CropRequest { mode: "rectangle".to_string(), x: 7, y: 0, width: 2, height: 1, ..request };
        assert!(crop_rect(8, 4, &invalid).is_err());
    }

    #[test]
    fn rotate_mirror_preserves_pixels_without_resampling() {
        let input = path("rotate-input.png");
        let mut image = image::RgbaImage::new(2, 1);
        image.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        image.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        image.save(&input).unwrap();
        let result = rotate(&RotateRequest { paths: vec![input.clone()], degrees: 0, flip: "horizontal".to_string(), output_location: OutputLocation::AlongsideInput }, input.clone());
        let output = result.output_paths().first().unwrap();
        let pixels = image::open(output).unwrap().to_rgba8();
        assert_eq!(pixels.dimensions(), (2, 1));
        assert_eq!(pixels.get_pixel(0, 0).0, [0, 255, 0, 255]);
        let invalid = rotate(&RotateRequest { paths: vec![input.clone()], degrees: 0, flip: "diagonal".to_string(), output_location: OutputLocation::AlongsideInput }, input.clone());
        assert!(invalid.error().is_some());
        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn resize_modes_produce_bounded_dimensions() {
        let percentage = ResizeRequest { paths: vec![], width: 0, height: 0, mode: "percentage".to_string(), resampling: "lanczos".to_string(), keep_aspect_ratio: true, percentage: 50, longest_side: 0, output_location: OutputLocation::AlongsideInput };
        assert_eq!(resize_dimensions(400, 200, &percentage).unwrap(), (200, 100));
        let longest = ResizeRequest { mode: "longestSide".to_string(), longest_side: 100, ..percentage };
        assert_eq!(resize_dimensions(400, 200, &longest).unwrap(), (100, 50));
        let invalid = ResizeRequest { mode: "percentage".to_string(), percentage: 0, ..longest };
        assert!(resize_dimensions(400, 200, &invalid).is_err());
    }

    #[test]
    fn composed_plan_applies_ordered_edits_once_without_mutating_source() {
        let input = path("composed-plan.png");
        image::RgbaImage::from_fn(4, 2, |x, y| image::Rgba([x as u8 * 40, y as u8 * 80, 10, 255]))
            .save(&input)
            .unwrap();
        let source = std::fs::read(&input).unwrap();
        let plan = ImageEditPlan {
            edits: vec![
                ImageEdit::Resize {
                    width: 4,
                    height: 2,
                    mode: "exact".to_string(),
                    percentage: 0,
                    longest_side: 0,
                    resampling: "nearest".to_string(),
                    keep_aspect_ratio: false,
                },
                ImageEdit::Rotate {
                    degrees: 90,
                    flip: "none".to_string(),
                },
                ImageEdit::Crop {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2,
                    mode: "rectangle".to_string(),
                    aspect_width: 0,
                    aspect_height: 0,
                    anchor: "center".to_string(),
                },
            ],
            output_location: OutputLocation::AlongsideInput,
        };

        let preview = inspect_edit_preview(&ImageEditPreviewRequest { path: input.clone(), plan: plan.clone() }).unwrap();
        assert_eq!((preview.width, preview.height), (2, 2));
        let result = export_edit_plan(&plan, input.clone());
        assert!(result.error().is_none(), "{}", result.error().cloned().unwrap_or_default());
        assert_eq!(std::fs::read(&input).unwrap(), source);
        let output = result.output_paths().first().unwrap();
        let edited = image::open(output).unwrap();
        assert_eq!((edited.width(), edited.height()), (2, 2));
        let output_name = output.file_name().and_then(|name| name.to_str()).unwrap();
        let claim = output.with_file_name(format!(".{output_name}.toolbox-reservation"));
        assert!(!claim.exists());
        assert!(!std::fs::read_dir(input.parent().unwrap()).unwrap().filter_map(Result::ok).any(|entry| {
            entry.file_name().to_str().is_some_and(|name| name.starts_with(".composed-plan-edited.png.toolbox-tmp-"))
        }));

        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn source_space_crop_replacement_uses_latest_region_before_later_edits() {
        let input = path("source-space-crops.png");
        let mut source = image::RgbaImage::from_pixel(10, 8, image::Rgba([0, 0, 0, 255]));
        for y in 0..3 {
            for x in 0..2 {
                source.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        for y in 5..7 {
            for x in 7..10 {
                source.put_pixel(x, y, image::Rgba([0, 255, 0, 255]));
            }
        }
        source.save(&input).unwrap();
        let plan = ImageEditPlan {
            edits: vec![
                ImageEdit::Crop {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 3,
                    mode: "rectangle".to_string(),
                    aspect_width: 2,
                    aspect_height: 3,
                    anchor: "center".to_string(),
                },
                ImageEdit::Rotate {
                    degrees: 90,
                    flip: "none".to_string(),
                },
                ImageEdit::Crop {
                    x: 7,
                    y: 5,
                    width: 3,
                    height: 2,
                    mode: "rectangle".to_string(),
                    aspect_width: 3,
                    aspect_height: 2,
                    anchor: "center".to_string(),
                },
            ],
            output_location: OutputLocation::AlongsideInput,
        };

        let preview = inspect_edit_preview(&ImageEditPreviewRequest { path: input.clone(), plan: plan.clone() }).unwrap();
        assert_eq!((preview.width, preview.height), (2, 3));
        let result = export_edit_plan(&plan, input.clone());
        assert!(result.error().is_none(), "{}", result.error().cloned().unwrap_or_default());
        let output = result.output_paths().first().unwrap();
        let edited = image::open(output).unwrap().to_rgba8();
        assert_eq!(edited.dimensions(), (2, 3));
        assert!(edited.pixels().all(|pixel| pixel.0 == [0, 255, 0, 255]));

        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn source_space_crop_maps_through_rotation_before_execution() {
        let input = path("rotate-before-source-crop.png");
        image::RgbaImage::from_pixel(100, 60, image::Rgba([40, 80, 120, 255])).save(&input).unwrap();
        let plan = ImageEditPlan {
            edits: vec![
                ImageEdit::Rotate { degrees: 90, flip: "none".to_string() },
                ImageEdit::Crop {
                    x: 0, y: 0, width: 100, height: 60, mode: "rectangle".to_string(),
                    aspect_width: 100, aspect_height: 60, anchor: "center".to_string(),
                },
            ],
            output_location: OutputLocation::AlongsideInput,
        };

        let preview = inspect_edit_preview(&ImageEditPreviewRequest { path: input.clone(), plan: plan.clone() }).unwrap();
        assert_eq!((preview.width, preview.height), (60, 100));
        let result = export_edit_plan(&plan, input.clone());
        assert!(result.error().is_none(), "{}", result.error().cloned().unwrap_or_default());
        let output = result.output_paths().first().unwrap();
        let edited = image::open(output).unwrap();
        assert_eq!((edited.width(), edited.height()), (60, 100));

        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn aspect_crop_uses_source_geometry_for_preview_and_export() {
        let input = path("aspect-source-crop.png");
        image::RgbaImage::from_fn(100, 60, |x, _| if x < 40 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 255, 0, 255])
        }).save(&input).unwrap();
        let plan = ImageEditPlan {
            edits: vec![
                ImageEdit::Rotate { degrees: 90, flip: "none".to_string() },
                ImageEdit::Crop {
                    x: 0, y: 0, width: 1, height: 1, mode: "aspectRatio".to_string(),
                    aspect_width: 1, aspect_height: 1, anchor: "right".to_string(),
                },
            ],
            output_location: OutputLocation::AlongsideInput,
        };

        let request = CropRequest {
            paths: vec![], x: 0, y: 0, width: 1, height: 1, mode: "aspectRatio".to_string(),
            aspect_width: 1, aspect_height: 1, anchor: "right".to_string(),
            output_location: OutputLocation::AlongsideInput,
        };
        assert_eq!(crop_rect(100, 60, &request).unwrap(), (40, 0, 60, 60));
        let preview = inspect_edit_preview(&ImageEditPreviewRequest { path: input.clone(), plan: plan.clone() }).unwrap();
        assert_eq!((preview.width, preview.height), (60, 60));
        let result = export_edit_plan(&plan, input.clone());
        assert!(result.error().is_none(), "{}", result.error().cloned().unwrap_or_default());
        let output = result.output_paths().first().unwrap();
        let edited = image::open(output).unwrap().to_rgba8();
        assert_eq!((edited.width(), edited.height()), (60, 60));
        assert!(edited.pixels().all(|pixel| pixel.0 == [0, 255, 0, 255]));

        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn source_space_crop_maps_through_resize_before_execution() {
        let input = path("resize-before-source-crop.png");
        image::RgbaImage::from_pixel(100, 60, image::Rgba([40, 80, 120, 255])).save(&input).unwrap();
        let plan = ImageEditPlan {
            edits: vec![
                ImageEdit::Resize {
                    width: 50, height: 30, mode: "exact".to_string(), percentage: 0, longest_side: 0,
                    resampling: "nearest".to_string(), keep_aspect_ratio: false,
                },
                ImageEdit::Crop {
                    x: 0, y: 0, width: 100, height: 60, mode: "rectangle".to_string(),
                    aspect_width: 100, aspect_height: 60, anchor: "center".to_string(),
                },
            ],
            output_location: OutputLocation::AlongsideInput,
        };

        let preview = inspect_edit_preview(&ImageEditPreviewRequest { path: input.clone(), plan: plan.clone() }).unwrap();
        assert_eq!((preview.width, preview.height), (50, 30));
        let result = export_edit_plan(&plan, input.clone());
        assert!(result.error().is_none(), "{}", result.error().cloned().unwrap_or_default());
        let output = result.output_paths().first().unwrap();
        let edited = image::open(output).unwrap();
        assert_eq!((edited.width(), edited.height()), (50, 30));

        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn composed_plan_rejects_invalid_editor_values() {
        let plan = ImageEditPlan {
            edits: vec![ImageEdit::Tone {
                brightness: 101,
                contrast: 0.0,
                saturation: 0.0,
                exposure: 0.0,
            }],
            output_location: OutputLocation::AlongsideInput,
        };

        assert_eq!(plan.validate().unwrap_err(), "Brightness must be between -100 and 100.");
    }
}
