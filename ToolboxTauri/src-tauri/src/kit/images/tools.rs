use base64::{engine::general_purpose::STANDARD, Engine};
use image::codecs::png::PngEncoder;
use image::{DynamicImage, ImageBuffer, ImageEncoder, Luma, Rgb, Rgba};
use image::AnimationDecoder;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufReader, Seek};
use std::path::{Path, PathBuf};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::common::watermark::{WatermarkOptions, WatermarkPlacement, WatermarkPosition};
use crate::kit::contracts::{OutcomePointer, OutcomeWarning, SkipCode, SkipReason, ToolError, WarningKind};
use crate::kit::images::{metadata, strip, OutputFormat};

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
                ImageEdit::Tone { brightness, contrast, saturation, exposure } => ToneStep { brightness: *brightness, contrast: *contrast, saturation: *saturation, exposure: *exposure }.validate()?,
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
    let (source, _) = crate::kit::images::load_upright_image(&request.path)?;
    let (edited, format, quality) = apply_edit_plan(&plan, source, crate::kit::images::detect_format(&request.path))?;
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
    let source = match crate::kit::images::load_upright_image(&input) {
        Ok((image, _)) => image,
        Err(error) => return failure(input, error),
    };
    let (edited, format, quality) = match apply_edit_plan(&plan, source, crate::kit::images::detect_format(&input)) {
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

/// One run of the shared edit stack: every path gets the same steps and its own result.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageEditStackRequest {
    pub paths: Vec<PathBuf>,
    pub steps: ImageEditSteps,
    /// The format of the final encode: jpg, png, webp or heic. None keeps each input's format.
    #[serde(default)]
    pub format: Option<String>,
    pub output_location: OutputLocation,
}

/// The stack holds at most one of each step, and they always run in this order.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageEditSteps {
    #[serde(default)] pub crop: Option<CropStep>,
    #[serde(default)] pub resize: Option<ResizeStep>,
    #[serde(default)] pub rotate: Option<RotateStep>,
    #[serde(default)] pub tone: Option<ToneStep>,
    #[serde(default)] pub watermark: Option<WatermarkOptions>,
    #[serde(default)] pub metadata: Option<RemoveMetadataStep>,
}

/// Removing metadata has no options: it always keeps the orientation and colour profile.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveMetadataStep {}

const STACK_QUALITY: u8 = 90;

/// The per-file note for a run that turned or mirrored the pixels, which always means a new encode.
pub const REENCODED_AFTER_ROTATING: &str = "Re-encoded at quality 90 after rotating.";
pub const ORIENTATION_APPLIED_FIRST: &str = "Camera orientation was applied first.";

impl ImageEditSteps {
    /// One step keeps its tool's suffix; any combination is named -edited.
    fn suffix(&self) -> Result<&'static str, String> {
        let present = [self.crop.as_ref().map(|_| "-cropped"), self.resize.as_ref().map(|_| "-resized"), self.rotate.as_ref().map(|_| "-rotated"), self.tone.as_ref().map(|_| "-tone"), self.watermark.as_ref().map(|_| "-watermarked"), self.metadata.as_ref().map(|_| "-stripped")];
        let mut suffixes = present.into_iter().flatten();
        match (suffixes.next(), suffixes.next()) {
            (None, _) => Err("Add a step to the stack before running.".to_string()),
            (Some(suffix), None) => Ok(suffix),
            (Some(_), Some(_)) => Ok("-edited"),
        }
    }

    // The order is fixed here, never by the order the steps were added: crop runs first, so its
    // rectangle is always measured in source pixels, then resize scales what is left, then rotate and
    // flip turn the result, tone adjusts the colour of the pixels that are left, and the watermark
    // goes on last so it is never cropped, scaled, turned or tinted with the photo. Removing
    // metadata changes no pixels, so it has nothing to do here: it decides how the copy is saved.
    fn apply(&self, mut image: DynamicImage) -> Result<DynamicImage, String> {
        if let Some(crop) = &self.crop {
            let (x, y, width, height) = crop_rect(image.width(), image.height(), crop)?;
            image = image.crop_imm(x, y, width, height);
        }
        if let Some(resize) = &self.resize {
            image = resize.apply(image)?;
        }
        if let Some(rotate) = &self.rotate {
            image = rotate.apply(image)?;
        }
        if let Some(tone) = &self.tone {
            image = tone.apply(image)?;
        }
        if let Some(watermark) = &self.watermark {
            image = apply_watermark(image, watermark)?;
        }
        Ok(image)
    }

    /// Whether no step can change this image, measured at its upright size. A crop is neutral when
    /// it covers the whole image, a resize when its result is the source size, a rotate when it
    /// neither turns nor mirrors, and tone when all four values are 0. A step that cannot apply to
    /// the image is never neutral, so it still fails as it would otherwise. A watermark with text or
    /// a logo always marks the image, and one with neither fails, so a watermark is never neutral.
    /// Removing metadata leaves the pixels as they are, so it is neutral here, and whether the file
    /// has any metadata to remove is read from the file itself.
    /// Every step is named here, so a new step must say whether it can be.
    fn is_neutral_for(&self, width: u32, height: u32) -> bool {
        let Self { crop, resize, rotate, tone, watermark, metadata: _ } = self;
        crop.as_ref().is_none_or(|crop| crop_rect(width, height, crop).is_ok_and(|rect| rect == (0, 0, width, height)))
            && resize.as_ref().is_none_or(|resize| resize_dimensions(width, height, resize).is_ok_and(|size| size == (width, height)))
            && rotate.as_ref().is_none_or(RotateStep::is_neutral)
            && tone.as_ref().is_none_or(ToneStep::is_neutral)
            && watermark.is_none()
    }

    fn detail(&self, oriented: bool) -> String {
        let rotated = self.rotate.as_ref().is_some_and(|rotate| !rotate.is_neutral());
        match (oriented, rotated) {
            (true, true) => format!("{ORIENTATION_APPLIED_FIRST} {REENCODED_AFTER_ROTATING}"),
            (true, false) => ORIENTATION_APPLIED_FIRST.to_string(),
            (false, true) => REENCODED_AFTER_ROTATING.to_string(),
            (false, false) => "Image saved".to_string(),
        }
    }
}

/// The reason a file is Skipped when the stack cannot change it and the format stays the same.
pub const NO_CHANGES: &str = "No changes. No copy was made.";

/// The output format an input already is, if it is one the stack can write.
fn input_format(path: &Path) -> Option<OutputFormat> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some(OutputFormat::Jpeg),
        "png" => Some(OutputFormat::Png),
        "webp" => Some(OutputFormat::WebP),
        "heic" | "heif" => Some(OutputFormat::Heic),
        _ => None,
    }
}

pub const TRANSPARENCY_FLATTENED: &str = "Transparent areas were flattened onto white, because JPEG has none.";
pub const ANIMATED_GIF: &str = "Animated GIF. Edit Image works on still images. Extract GIF Frames saves each frame.";
pub const REDUCED_TO_8_BIT: &str = "Saved as 8-bit sRGB. The original had more colour depth.";

/// Decodes the input once, runs the stack and encodes once, in the chosen format or the input's.
pub fn export_edit_stack(request: &ImageEditStackRequest, input: PathBuf) -> JobOutcome {
    let suffix = match request.steps.suffix() {
        Ok(suffix) => suffix,
        Err(error) => return JobOutcome::failure(input, ToolError::invalid_input(error)),
    };
    let format = match request.format.as_deref() {
        Some(requested) => match parse_output_format(requested) {
            Ok(format) => format,
            Err(error) => return JobOutcome::failure(input, ToolError::invalid_input(error)),
        },
        None => crate::kit::images::detect_format(&input),
    };
    // Editing keeps one picture, so an animated GIF would lose its other frames. It fails before
    // anything is decoded or reserved and points to the tool that saves every frame.
    if crate::kit::images::is_animated_gif(&input) {
        return JobOutcome::failure(input, ToolError::invalid_input(ANIMATED_GIF).with_pointer(OutcomePointer::Tool { tool_id: "gif-extract".into() }));
    }
    let (source, oriented) = match crate::kit::images::load_upright_image(&input) {
        Ok(decoded) => decoded,
        Err(error) => return failure(input, error),
    };
    // Removing metadata starts from what the original holds, so a file whose metadata can't be
    // read fails rather than being saved with something left in it.
    let found = match request.steps.metadata {
        Some(_) => match metadata::read_file(&input) {
            Ok(found) => Some(found),
            Err(error) => return failure(input, format!("Could not read this image's metadata: {error}")),
        },
        None => None,
    };
    // A stack that leaves this image as it is, saved in the format it already has, would only
    // make a copy, so the file is Skipped before anything is encoded or reserved. When the stack
    // removes metadata, that copy is only worth making if there is metadata to remove.
    let unchanged = input_format(&input) == Some(format) && request.steps.is_neutral_for(source.width(), source.height());
    if unchanged {
        match &found {
            None => return JobOutcome::skipped(input, SkipReason::new(SkipCode::NoEffectiveChange, NO_CHANGES)),
            Some(found) if strip::removable(found).is_empty() => return JobOutcome::skipped(input, SkipReason::new(SkipCode::NoEffectiveChange, strip::NO_METADATA)),
            Some(_) => {}
        }
    }
    let mut warnings = Vec::new();
    let (bytes, detail) = match (&found, unchanged, format) {
        // Only metadata is removed, and JPEG and PNG can lose it without touching the pixels.
        (Some(found), true, OutputFormat::Jpeg | OutputFormat::Png) => {
            let cleaned = std::fs::read(&input).map_err(|error| format!("Could not read image: {error}")).and_then(|original| match format {
                OutputFormat::Jpeg => strip::clean_jpeg(&original, found.orientation),
                _ => strip::clean_png(&original, found.orientation),
            });
            match cleaned {
                Ok(bytes) => (bytes, strip::CLEANED_LOSSLESSLY.to_string()),
                Err(error) => return failure(input, error),
            }
        }
        _ => {
            let high_bit_source = crate::kit::images::has_high_bit_depth(source.color());
            // A step that cannot apply to this image (a crop outside it, a size past the limits) is a
            // problem with the request, not with the file.
            let edited = match request.steps.apply(source) {
                Ok(image) => image,
                Err(error) => return JobOutcome::failure(input, ToolError::invalid_input(error)),
            };
            // JPEG has no alpha, and only PNG stores more than 8 bits per channel.
            if format == OutputFormat::Jpeg && crate::kit::images::has_transparency(&edited) {
                warnings.push(OutcomeWarning { kind: WarningKind::TransparencyFlattened, message: TRANSPARENCY_FLATTENED.to_string() });
            }
            if high_bit_source && (format != OutputFormat::Png || !crate::kit::images::has_high_bit_depth(edited.color())) {
                warnings.push(OutcomeWarning { kind: WarningKind::PrecisionReduced, message: REDUCED_TO_8_BIT.to_string() });
            }
            // A new encode writes no metadata at all, and the orientation is already in the pixels.
            // When metadata is removed, the colour profile is carried over so colours look the same.
            let profile = found.as_ref().filter(|found| found.icc).and_then(|_| crate::kit::images::icc_profile(&input));
            match crate::kit::images::encode_with_profile(&edited, format, STACK_QUALITY, profile.as_deref()) {
                Ok(bytes) => (bytes, if unchanged { strip::REENCODED_TO_CLEAN.to_string() } else { request.steps.detail(oriented) }),
                Err(error) => return failure(input, error),
            }
        }
    };
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, suffix, format.extension()) {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve image output: {error}")),
    };
    if let Err(error) = std::fs::write(output.path(), bytes) {
        return failure(input, format!("Could not save image: {error}"));
    }
    // Each cleaned copy is read back once saved. If anything removed is still there, the file
    // fails and the reservation is dropped, so no copy is kept.
    let detail = match &found {
        Some(found) => match metadata::read_file(output.path()) {
            Ok(saved) => match strip::verify(&saved) {
                Ok(()) => [detail, strip::summary(found, &saved)].join(" ").trim().to_string(),
                Err(reason) => return failure(input, reason.to_string()),
            },
            Err(error) => return failure(input, format!("Could not check the cleaned copy: {error}")),
        },
        None => detail,
    };
    match output.publish() {
        Ok(path) => warnings.into_iter().fold(JobOutcome::succeeded(input, vec![path], detail), JobOutcome::with_warning),
        Err(error) => failure(input, format!("Could not publish image: {error}")),
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
                let step = ResizeStep { width: *width, height: *height, mode: mode.clone(), resampling: resampling.clone(), keep_aspect_ratio: *keep_aspect_ratio, percentage: *percentage, longest_side: *longest_side };
                let old_width = image.width();
                let old_height = image.height();
                image = step.apply(image)?;
                let (width, height) = (image.width(), image.height());
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
                let step = CropStep { x: *x, y: *y, width: *width, height: *height, mode: mode.clone(), aspect_width: *aspect_width, aspect_height: *aspect_height, anchor: anchor.clone() };
                let (x, y, width, height) = crop_rect(source_width, source_height, &step)?;
                let crop = map_source_rect((x, y, width, height), &geometry)?;
                image = image.crop_imm(crop.0, crop.1, crop.2, crop.3);
            }
            ImageEdit::Tone { brightness, contrast, saturation, exposure } => {
                image = ToneStep { brightness: *brightness, contrast: *contrast, saturation: *saturation, exposure: *exposure }.apply(image)?;
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
    let (image, _) = crate::kit::images::load_upright_image(&request.path).map_err(|error| format!("Could not preview image: {error}"))?;
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
    read_tiff_pages(&request.path)?
        .iter()
        .map(|page| tiff_page_to_dynamic(page).and_then(encode_preview))
        .collect()
}

/// How to size each image: a width-and-height box (fitted inside unless the aspect lock is off),
/// a percentage, or the length of the longer edge.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeStep {
    #[serde(default)] pub width: u32, #[serde(default)] pub height: u32,
    #[serde(default = "default_resize_mode")] pub mode: String,
    #[serde(default = "default_resampling")] pub resampling: String,
    #[serde(default = "default_keep_aspect_ratio")] pub keep_aspect_ratio: bool,
    #[serde(default)] pub percentage: u32, #[serde(default)] pub longest_side: u32,
}
/// A clockwise turn of 0, 90, 180 or 270 degrees, then an optional mirror.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotateStep {
    pub degrees: i32,
    #[serde(default = "default_flip")] pub flip: String,
}
/// The crop geometry, measured in source pixels. Aspect-ratio mode keeps the largest area of that
/// ratio each image has, so one setting fits every size in a batch.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropStep {
    pub x: u32, pub y: u32, pub width: u32, pub height: u32,
    #[serde(default = "default_crop_mode")] pub mode: String,
    #[serde(default)] pub aspect_width: u32, #[serde(default)] pub aspect_height: u32,
    #[serde(default = "default_crop_anchor")] pub anchor: String,
}
/// Brightness, contrast, saturation and exposure, each from -100 to 100, where 0 leaves it as it is.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToneStep {
    pub brightness: i32,
    pub contrast: f32,
    #[serde(default)] pub saturation: f32,
    #[serde(default)] pub exposure: f32,
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
pub struct MetadataRequest { pub paths: Vec<PathBuf> }

impl ResizeStep {
    fn apply(&self, image: DynamicImage) -> Result<DynamicImage, String> {
        let (width, height) = resize_dimensions(image.width(), image.height(), self)?;
        let filter = match self.resampling.as_str() { "nearest" => image::imageops::FilterType::Nearest, "bicubic" => image::imageops::FilterType::CatmullRom, _ => image::imageops::FilterType::Lanczos3 };
        Ok(image.resize_exact(width, height, filter))
    }
}

fn default_resize_mode() -> String { "exact".to_string() }
fn default_resampling() -> String { "lanczos".to_string() }
fn default_keep_aspect_ratio() -> bool { true }

fn resize_dimensions(source_width: u32, source_height: u32, request: &ResizeStep) -> Result<(u32, u32), String> {
    if request.mode == "exact" && (request.width == 0 || request.height == 0 || request.width > 16384 || request.height > 16384) {
        return Err("Resize dimensions must be between 1 and 16384 pixels.".to_string());
    }
    let dimensions = match request.mode.as_str() {
        "exact" if request.keep_aspect_ratio => {
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

// Not clamped: a result past 16384 px is refused by resize_dimensions rather than silently distorted.
fn scale_dimension(value: u32, percentage: u32) -> u32 { ((value as u64 * percentage as u64 + 50) / 100).clamp(1, u32::MAX as u64) as u32 }
impl RotateStep {
    fn apply(&self, image: DynamicImage) -> Result<DynamicImage, String> {
        let image = match self.degrees.rem_euclid(360) { 90 => image.rotate90(), 180 => image.rotate180(), 270 => image.rotate270(), 0 => image, _ => return Err("Rotation must be 0, 90, 180, or 270 degrees.".to_string()) };
        Ok(match self.flip.as_str() { "none" => image, "horizontal" => DynamicImage::ImageRgba8(image::imageops::flip_horizontal(&image.to_rgba8())), "vertical" => DynamicImage::ImageRgba8(image::imageops::flip_vertical(&image.to_rgba8())), _ => return Err("Mirror choice must be none, horizontal, or vertical.".to_string()) })
    }

    fn is_neutral(&self) -> bool { self.degrees.rem_euclid(360) == 0 && self.flip == "none" }
}

fn default_flip() -> String { "none".to_string() }
fn default_crop_mode() -> String { "rectangle".to_string() }
fn default_crop_anchor() -> String { "center".to_string() }

fn crop_rect(image_width: u32, image_height: u32, request: &CropStep) -> Result<(u32, u32, u32, u32), String> {
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
        // The largest fit leaves room on one axis only, so start and end keep the top or left
        // part and the bottom or right part of every image, whatever its orientation.
        let x = match request.anchor.as_str() { "left" | "start" => 0, "right" | "end" => max_x, "custom" => request.x.min(max_x), _ => max_x / 2 };
        let y = match request.anchor.as_str() { "top" | "start" => 0, "bottom" | "end" => max_y, "custom" => request.y.min(max_y), _ => max_y / 2 };
        Ok((x, y, width, height))
    } else if request.mode == "rectangle" {
        if request.width == 0 || request.height == 0 || request.x.saturating_add(request.width) > image_width || request.y.saturating_add(request.height) > image_height { return Err("Crop rectangle must fit inside the image.".to_string()); }
        Ok((request.x, request.y, request.width, request.height))
    } else { Err(format!("Unsupported crop mode: {}.", request.mode)) }
}
impl ToneStep {
    fn is_neutral(&self) -> bool { self.brightness == 0 && self.contrast == 0.0 && self.saturation == 0.0 && self.exposure == 0.0 }

    fn validate(&self) -> Result<(), String> {
        if !(-100..=100).contains(&self.brightness) { return Err("Brightness must be between -100 and 100.".to_string()); }
        for (name, value) in [("Contrast", self.contrast), ("Saturation", self.saturation), ("Exposure", self.exposure)] {
            if !(-100.0..=100.0).contains(&value) { return Err(format!("{name} must be between -100 and 100.")); }
        }
        Ok(())
    }

    // A value of 0 leaves the pixels exactly as they were: the colour maths rounds rather than
    // truncates, and contrast is skipped at 0. Brightness, saturation and exposure leave alpha
    // alone, but the image crate's contrast also stretches alpha, so each pixel gets its alpha
    // back afterwards and transparent areas stay transparent.
    fn apply(&self, image: DynamicImage) -> Result<DynamicImage, String> {
        self.validate()?;
        let mut rgba = image::imageops::brighten(&image, self.brightness);
        let exposure = 2.0_f32.powf(self.exposure / 100.0);
        let saturation = 1.0 + self.saturation / 100.0;
        for pixel in rgba.pixels_mut() {
            let [red, green, blue, alpha] = pixel.0;
            let average = (red as f32 + green as f32 + blue as f32) / 3.0;
            pixel.0 = [
                ((average + (red as f32 - average) * saturation) * exposure).round().clamp(0.0, 255.0) as u8,
                ((average + (green as f32 - average) * saturation) * exposure).round().clamp(0.0, 255.0) as u8,
                ((average + (blue as f32 - average) * saturation) * exposure).round().clamp(0.0, 255.0) as u8,
                alpha,
            ];
        }
        if self.contrast == 0.0 { return Ok(DynamicImage::ImageRgba8(rgba)); }
        let mut contrasted = image::imageops::contrast(&rgba, self.contrast);
        for (pixel, source) in contrasted.pixels_mut().zip(rgba.pixels()) {
            pixel.0[3] = source.0[3];
        }
        Ok(DynamicImage::ImageRgba8(contrasted))
    }
}
fn apply_watermark(image: DynamicImage, options: &WatermarkOptions) -> Result<DynamicImage, String> {
    options.validate()?;
    let mut image = image.to_rgba8();
    let (width, height) = image.dimensions();
    let original_alpha = image.pixels().map(|pixel| pixel.0[3]).collect::<Vec<_>>();
    let (mark, offset_x, offset_y) = watermark_mark(options, width)?;
    let (mark, offset_x, offset_y) = rotate_mark(mark, offset_x, offset_y, options.rotation);
    let mut watermark = image::RgbaImage::new(width, height);
    match options.position {
        Some(WatermarkPosition::Placement(WatermarkPlacement::Tile)) => tile_mark(&mut watermark, &mark),
        position => {
            let (x, y) = mark_origin(position, width, height, &mark, offset_x, offset_y);
            image::imageops::replace(&mut watermark, &mark, x, y);
        }
    }
    if watermark.pixels().all(|pixel| pixel.0[3] == 0) {
        return Err("Provide watermark text or a logo image.".to_string());
    }
    image::imageops::overlay(&mut image, &watermark, 0, 0);
    for (pixel, alpha) in image.pixels_mut().zip(original_alpha) { pixel.0[3] = alpha; }
    Ok(DynamicImage::ImageRgba8(image))
}

/// The mark itself, at its final size and opacity but not yet turned, with the offset of its
/// top-left corner from the point the text pen and the logo start at.
fn watermark_mark(options: &WatermarkOptions, target_width: u32) -> Result<(image::RgbaImage, i64, i64), String> {
    let alpha = (options.opacity as u16 * 255 / 100) as u8;
    let target = options.size.map(|size| (size * target_width as f32).max(1.0));
    let text = match options.text() {
        Some(text) => {
            let color = watermark_color(options.color.as_deref());
            let font_size = match target {
                // Advance width grows in step with the font size, so one measurement finds the size.
                Some(target) => {
                    let (_, _, _, advance) = draw_text(text, 100.0, color, alpha)?;
                    if advance > 0.0 { 100.0 * target / advance } else { WATERMARK_FONT_SIZE }
                }
                None => WATERMARK_FONT_SIZE,
            };
            Some(draw_text(text, font_size, color, alpha)?)
        }
        None => None,
    };
    let logo = match options.logo_path.as_ref() {
        Some(path) => {
            let mut logo = crate::kit::images::load_image(path)?.to_rgba8();
            if let Some(target) = target {
                let width = target.round().max(1.0) as u32;
                let height = ((logo.height() as f32 * width as f32 / logo.width().max(1) as f32).round() as u32).max(1);
                logo = image::imageops::resize(&logo, width, height, image::imageops::FilterType::Triangle);
            }
            for pixel in logo.pixels_mut() { pixel.0[3] = ((u16::from(pixel.0[3]) * u16::from(alpha) + 127) / 255) as u8; }
            Some(logo)
        }
        None => None,
    };
    Ok(match (text, logo) {
        (Some((layer, x, y, _)), None) => (layer, x, y),
        (None, Some(logo)) => (logo, 0, 0),
        // Both start at the same point, with the logo over the text.
        (Some((layer, x, y, _)), Some(logo)) => {
            let (left, top) = (x.min(0), y.min(0));
            let right = (x + layer.width() as i64).max(logo.width() as i64);
            let bottom = (y + layer.height() as i64).max(logo.height() as i64);
            let mut mark = image::RgbaImage::new((right - left) as u32, (bottom - top) as u32);
            image::imageops::replace(&mut mark, &layer, x - left, y - top);
            image::imageops::overlay(&mut mark, &logo, -left, -top);
            (mark, left, top)
        }
        (None, None) => return Err("Provide watermark text or a logo image.".to_string()),
    })
}

fn watermark_color(color: Option<&str>) -> [u8; 3] {
    let channel = |hex: &str, index: usize| u8::from_str_radix(&hex[1 + index * 2..3 + index * 2], 16).unwrap_or(255);
    color.map(|hex| [channel(hex, 0), channel(hex, 1), channel(hex, 2)]).unwrap_or([255, 255, 255])
}

/// Turns the mark about its centre. Positive degrees turn it clockwise as the image is seen, and
/// negative degrees counter-clockwise, as CSS rotate does. Pixels are sampled with premultiplied
/// alpha so the edges do not pick up a dark fringe.
fn rotate_mark(mark: image::RgbaImage, offset_x: i64, offset_y: i64, degrees: f32) -> (image::RgbaImage, i64, i64) {
    if degrees == 0.0 || mark.width() == 0 || mark.height() == 0 { return (mark, offset_x, offset_y); }
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (width, height) = (mark.width() as f32, mark.height() as f32);
    let rotated_width = (width * cos.abs() + height * sin.abs()).ceil().max(1.0) as u32;
    let rotated_height = (width * sin.abs() + height * cos.abs()).ceil().max(1.0) as u32;
    let mut rotated = image::RgbaImage::new(rotated_width, rotated_height);
    let (center_x, center_y) = (width / 2.0, height / 2.0);
    let (rotated_center_x, rotated_center_y) = (rotated_width as f32 / 2.0, rotated_height as f32 / 2.0);
    let sample = |x: i64, y: i64| -> [f32; 4] {
        if x < 0 || y < 0 || x >= mark.width() as i64 || y >= mark.height() as i64 { return [0.0; 4]; }
        let [red, green, blue, alpha] = mark.get_pixel(x as u32, y as u32).0;
        let alpha = alpha as f32 / 255.0;
        [red as f32 * alpha, green as f32 * alpha, blue as f32 * alpha, alpha]
    };
    for (column, row, pixel) in rotated.enumerate_pixels_mut() {
        let dx = column as f32 + 0.5 - rotated_center_x;
        let dy = row as f32 + 0.5 - rotated_center_y;
        let source_x = dx * cos + dy * sin + center_x - 0.5;
        let source_y = -dx * sin + dy * cos + center_y - 0.5;
        let (left, top) = (source_x.floor(), source_y.floor());
        let (fraction_x, fraction_y) = (source_x - left, source_y - top);
        let (left, top) = (left as i64, top as i64);
        let mut value = [0.0_f32; 4];
        for (x, y, weight) in [
            (left, top, (1.0 - fraction_x) * (1.0 - fraction_y)),
            (left + 1, top, fraction_x * (1.0 - fraction_y)),
            (left, top + 1, (1.0 - fraction_x) * fraction_y),
            (left + 1, top + 1, fraction_x * fraction_y),
        ] {
            let source = sample(x, y);
            for channel in 0..4 { value[channel] += source[channel] * weight; }
        }
        if value[3] <= 0.0 { continue; }
        pixel.0 = [
            (value[0] / value[3]).round().clamp(0.0, 255.0) as u8,
            (value[1] / value[3]).round().clamp(0.0, 255.0) as u8,
            (value[2] / value[3]).round().clamp(0.0, 255.0) as u8,
            (value[3] * 255.0).round().clamp(0.0, 255.0) as u8,
        ];
    }
    let shift_x = ((width - rotated_width as f32) / 2.0).round() as i64;
    let shift_y = ((height - rotated_height as f32) / 2.0).round() as i64;
    (rotated, offset_x + shift_x, offset_y + shift_y)
}

/// The share of the shorter side kept clear between an anchored mark and the edge.
const WATERMARK_MARGIN: f32 = 0.03;

/// Where the top-left corner of the (turned) mark goes on the target.
fn mark_origin(position: Option<WatermarkPosition>, width: u32, height: u32, mark: &image::RgbaImage, offset_x: i64, offset_y: i64) -> (i64, i64) {
    let anchor = match position {
        // A pixel offset places the text pen and the logo, as the watermark tool always has.
        None => return (16 + offset_x, 16 + offset_y),
        Some(WatermarkPosition::Offset { x, y }) => return (x as i64 + offset_x, y as i64 + offset_y),
        Some(WatermarkPosition::Anchor { anchor_x, anchor_y }) => (anchor_x, anchor_y),
        Some(WatermarkPosition::Placement(placement)) => match placement {
            WatermarkPlacement::TopLeft => (0.0, 0.0),
            WatermarkPlacement::TopRight => (1.0, 0.0),
            WatermarkPlacement::BottomLeft => (0.0, 1.0),
            WatermarkPlacement::BottomRight => (1.0, 1.0),
            WatermarkPlacement::Center | WatermarkPlacement::Tile => (0.5, 0.5),
        },
    };
    let margin = (width.min(height) as f32 * WATERMARK_MARGIN).round();
    let along = |anchor: f32, space: u32, size: u32| (margin + anchor * (space as f32 - size as f32 - 2.0 * margin)).round() as i64;
    (along(anchor.0, width, mark.width()), along(anchor.1, height, mark.height()))
}

/// Repeats the mark across the whole target in staggered rows, starting half a mark past the
/// top-left corner so the edges are covered too.
fn tile_mark(canvas: &mut image::RgbaImage, mark: &image::RgbaImage) {
    let (mark_width, mark_height) = (mark.width() as i64, mark.height() as i64);
    if mark_width == 0 || mark_height == 0 { return; }
    let gap = (mark_width.max(mark_height) / 2).max(1);
    let (step_x, step_y) = (mark_width + gap, mark_height + gap);
    let mut row = 0;
    let mut y = -mark_height / 2;
    while y < canvas.height() as i64 {
        let mut x = -mark_width / 2 - if row % 2 == 1 { step_x / 2 } else { 0 };
        while x < canvas.width() as i64 {
            image::imageops::overlay(canvas, mark, x, y);
            x += step_x;
        }
        y += step_y;
        row += 1;
    }
}

const WATERMARK_FONT_RESOURCE: &str = "watermarkFont";
const WATERMARK_FONT_FILE: &str = "DejaVuSans.ttf";
const WATERMARK_FONT_SIZE: f32 = 32.0;
const INCLUDED_WATERMARK_FONT: &[u8] = include_bytes!("../../../resources/fonts/DejaVuSans.ttf");

fn watermark_font_bytes() -> Result<Vec<u8>, String> {
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
            .map_err(|error| format!("Could not read bundled watermark font: {error}"));
    }
    Ok(INCLUDED_WATERMARK_FONT.to_vec())
}

/// Draws the text into a layer just big enough for its glyphs. Returns the layer, the offset of its
/// top-left corner from the pen's starting point, and how far the pen advanced.
fn draw_text(text: &str, font_size: f32, color: [u8; 3], alpha: u8) -> Result<(image::RgbaImage, i64, i64, f32), String> {
    let bytes = watermark_font_bytes()?;
    let font = fontdue::Font::from_bytes(bytes.clone(), fontdue::FontSettings::default())
        .map_err(|error| format!("Could not parse bundled watermark font: {error}"))?;
    let mut face = rustybuzz::Face::from_slice(&bytes, 0)
        .ok_or_else(|| "Could not parse bundled watermark font for shaping.".to_string())?;
    face.set_pixels_per_em(Some((font_size.round().clamp(1.0, f32::from(u16::MAX)) as u16, font_size.round().clamp(1.0, f32::from(u16::MAX)) as u16)));

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

    let scale = font_size / face.units_per_em() as f32;
    let ascent = font
        .horizontal_line_metrics(font_size)
        .map(|metrics| metrics.ascent)
        .unwrap_or(font_size * 0.8);
    let mut glyphs = Vec::new();
    let mut pen_x = 0.0_f32;
    for (info, position) in glyph_buffer.glyph_infos().iter().zip(glyph_buffer.glyph_positions()) {
        let (metrics, bitmap) = font.rasterize_indexed(info.glyph_id as u16, font_size);
        let glyph_x = ((pen_x + position.x_offset as f32) * scale + metrics.bounds.xmin).floor() as i64;
        let glyph_y = (ascent - position.y_offset as f32 * scale - metrics.bounds.height - metrics.bounds.ymin).floor() as i64;
        if metrics.width > 0 && metrics.height > 0 { glyphs.push((glyph_x, glyph_y, metrics, bitmap)); }
        pen_x += position.x_advance as f32;
    }
    let advance = pen_x * scale;
    let left = glyphs.iter().map(|glyph| glyph.0).min().unwrap_or(0);
    let top = glyphs.iter().map(|glyph| glyph.1).min().unwrap_or(0);
    let right = glyphs.iter().map(|glyph| glyph.0 + glyph.2.width as i64).max().unwrap_or(0);
    let bottom = glyphs.iter().map(|glyph| glyph.1 + glyph.2.height as i64).max().unwrap_or(0);
    let mut layer = image::RgbaImage::new((right - left).max(0) as u32, (bottom - top).max(0) as u32);
    for (glyph_x, glyph_y, metrics, bitmap) in glyphs {
        for row in 0..metrics.height {
            for column in 0..metrics.width {
                let coverage = bitmap[row * metrics.width + column];
                let glyph_alpha = ((u16::from(coverage) * u16::from(alpha) + 127) / 255) as u8;
                if glyph_alpha == 0 { continue; }
                let pixel = layer.get_pixel_mut((glyph_x - left) as u32 + column as u32, (glyph_y - top) as u32 + row as u32);
                blend_watermark_pixel(pixel, color, glyph_alpha);
            }
        }
    }
    Ok((layer, left, top, advance))
}

fn blend_watermark_pixel(pixel: &mut Rgba<u8>, color: [u8; 3], source_alpha: u8) {
    let source_alpha = u16::from(source_alpha);
    let destination_alpha = u16::from(pixel.0[3]);
    let output_alpha = source_alpha + ((destination_alpha * (255 - source_alpha) + 127) / 255);
    pixel.0 = [color[0], color[1], color[2], output_alpha.min(255) as u8];
}

/// Controls, spaces, private-use characters and noncharacters, which have no glyph to quote.
fn shows_nothing(character: char) -> bool {
    let code = character as u32;
    character.is_control()
        || character.is_whitespace()
        || (0xE000..=0xF8FF).contains(&code)
        || code >= 0xF0000
        || (0xFDD0..=0xFDEF).contains(&code)
        || code & 0xFFFE == 0xFFFE
}

/// The board's wording: each character the bundled font has no glyph for, quoted, or by its code
/// point when it would not show.
fn unsupported_watermark_characters(characters: impl IntoIterator<Item = char>) -> String {
    let details = characters
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|character| if shows_nothing(character) {
            format!("U+{:04X}", character as u32)
        } else {
            format!("\u{201c}{character}\u{201d}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("The watermark font can't draw {details}.")
}
pub fn icon_set(request: &IconSetRequest, input: PathBuf) -> JobOutcome {
    let image = match image::open(&input) { Ok(image) => image, Err(error) => return failure(input, format!("Could not read image: {error}")) };
    let (prefix, sizes) = match icon_plan(&request.preset, &request.sizes) { Ok(plan) => plan, Err(error) => return failure(input, error) };
    let mut outputs = Vec::new();
    for &size in &sizes {
        let output = match OutputNaming::reserve_destination(&input, &request.output_location, &format!("-{prefix}-{size}"), "png") {
            Ok(output) => output,
            Err(error) => return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not reserve icon output: {error}"))),
        };
        if let Err(error) = image.resize_exact(size, size, image::imageops::FilterType::Lanczos3).save(output.path()) {
            return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not save icon: {error}")));
        }
        match output.publish() {
            Ok(path) => outputs.push(path),
            Err(error) => return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not publish icon output: {error}"))),
        }
    }
    if request.preset == "macos" {
        let icns = match OutputNaming::reserve_destination(&input, &request.output_location, "-macos-icon", "icns") {
            Ok(output) => output,
            Err(error) => return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not reserve macOS icon output: {error}"))),
        };
        if let Err(error) = write_icns(icns.path(), &sizes, &outputs) {
            return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not save macOS ICNS container: {error}")));
        }
        match icns.publish() {
            Ok(path) => outputs.push(path),
            Err(error) => return JobOutcome::failure_with_outputs(input.clone(), outputs, ToolError::processing(format!("Could not publish macOS ICNS output: {error}"))),
        }
    }
    let detail = match request.preset.as_str() {
        "macos" => "macOS PNG and ICNS icon set saved.",
        "favicon" => "Favicon PNG icon set saved; ICO container is unavailable on this target.",
        "ios" => "iOS PNG icon set saved.",
        "android" => "Android PNG icon set saved.",
        _ => "Custom PNG icon set saved.",
    };
    JobOutcome::succeeded(input, outputs, detail.to_string())
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

fn icon_plan(preset: &str, custom_sizes: &[u32]) -> Result<(&'static str, Vec<u32>), String> {
    let sizes = match preset {
        "macos" => ("macos-icon", vec![16, 32, 128, 256, 512, 1024]),
        "favicon" => ("favicon", vec![16, 32, 48, 64, 128, 256]),
        "ios" => ("ios-icon", vec![20, 29, 40, 60, 76, 83, 1024]),
        "android" => ("android-icon", vec![48, 72, 96, 144, 192, 512]),
        "custom" => ("icon", custom_sizes.iter().copied().filter(|size| *size > 0 && *size <= 4096).collect()),
        _ => return Err(format!("Unsupported icon preset: {preset}.")),
    };
    if sizes.1.is_empty() { return Err("Select at least one custom icon size from 1 to 4096 pixels.".to_string()); }
    Ok(sizes)
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
            let decoded = match read_tiff_pages(&page_ref.path) {
                Ok(decoded) => decoded,
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
            match read_tiff_pages(path) {
                Ok(mut decoded) => pages.append(&mut decoded),
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

#[derive(Clone)]
enum TiffPage { Gray { width: u32, height: u32, data: Vec<u8> }, Rgb { width: u32, height: u32, data: Vec<u8> }, Rgba { width: u32, height: u32, data: Vec<u8> } }

fn tiff_page_to_dynamic(page: &TiffPage) -> Result<DynamicImage, String> {
    match page {
        TiffPage::Gray { width, height, data } => ImageBuffer::<Luma<u8>, _>::from_raw(*width, *height, data.clone()).map(DynamicImage::ImageLuma8),
        TiffPage::Rgb { width, height, data } => ImageBuffer::<Rgb<u8>, _>::from_raw(*width, *height, data.clone()).map(DynamicImage::ImageRgb8),
        TiffPage::Rgba { width, height, data } => ImageBuffer::<Rgba<u8>, _>::from_raw(*width, *height, data.clone()).map(DynamicImage::ImageRgba8),
    }
    .ok_or_else(|| "TIFF page pixel data has an invalid length.".to_string())
}

fn read_tiff_pages(path: &std::path::Path) -> Result<Vec<TiffPage>, String> {
    let file = File::open(path).map_err(|error| format!("Could not open TIFF: {error}"))?;
    let mut decoder = tiff::decoder::Decoder::new(file).map_err(|error| format!("Could not read TIFF: {error}"))?;
    let mut pages = Vec::new();
    loop {
        let (width, height) = decoder.dimensions().map_err(|error| format!("Could not read TIFF dimensions: {error}"))?;
        let color = decoder.colortype().map_err(|error| format!("Could not read TIFF color type: {error}"))?;
        let data = decoder.read_image().map_err(|error| format!("Unsupported TIFF page: {error}"))?;
        let page = match (color, data) {
            (tiff::ColorType::Gray(8), tiff::decoder::DecodingResult::U8(data)) => TiffPage::Gray { width, height, data },
            (tiff::ColorType::RGB(8), tiff::decoder::DecodingResult::U8(data)) => TiffPage::Rgb { width, height, data },
            (tiff::ColorType::RGBA(8), tiff::decoder::DecodingResult::U8(data)) => TiffPage::Rgba { width, height, data },
            (color, _) => return Err(format!("Unsupported TIFF color type or bit depth: {color:?}")),
        };
        pages.push(page);
        if !decoder.more_images() { break; }
        decoder.next_image().map_err(|error| format!("Could not advance to TIFF page: {error}"))?;
    }
    Ok(pages)
}

fn write_tiff_pages(path: &std::path::Path, pages: &[TiffPage]) -> Result<(), String> {
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut encoder = tiff::encoder::TiffEncoder::new(file).map_err(|error| error.to_string())?;
    for page in pages { write_encoded_page(&mut encoder, page)?; }
    Ok(())
}

fn write_tiff_page(path: &std::path::Path, page: &TiffPage) -> Result<(), String> {
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut encoder = tiff::encoder::TiffEncoder::new(file).map_err(|error| error.to_string())?;
    write_encoded_page(&mut encoder, page)
}

fn write_encoded_page<W: std::io::Write + Seek>(encoder: &mut tiff::encoder::TiffEncoder<W>, page: &TiffPage) -> Result<(), String> {
    match page {
        TiffPage::Gray { width, height, data } => encoder.write_image::<tiff::encoder::colortype::Gray8>(*width, *height, data).map_err(|error| error.to_string()),
        TiffPage::Rgb { width, height, data } => encoder.write_image::<tiff::encoder::colortype::RGB8>(*width, *height, data).map_err(|error| error.to_string()),
        TiffPage::Rgba { width, height, data } => encoder.write_image::<tiff::encoder::colortype::RGBA8>(*width, *height, data).map_err(|error| error.to_string()),
    }
}
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::processing(error)) }

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    fn path(name: &str) -> PathBuf { std::env::temp_dir().join(format!("toolbox_tiff_{}_{}", std::process::id(), name)) }

    fn text_watermark(input: &Path, text: &str, opacity: u8, offset: u32) -> ImageEditStackRequest {
        let options = WatermarkOptions { text: Some(text.to_string()), logo_path: None, size: None, opacity, color: None, position: Some(WatermarkPosition::Offset { x: offset, y: offset }), rotation: 0.0 };
        ImageEditStackRequest {
            paths: vec![input.to_path_buf()],
            steps: ImageEditSteps { watermark: Some(options), ..ImageEditSteps::default() },
            format: None,
            output_location: OutputLocation::AlongsideInput,
        }
    }

    fn mirror(input: &Path, flip: &str) -> ImageEditStackRequest {
        ImageEditStackRequest {
            paths: vec![input.to_path_buf()],
            steps: ImageEditSteps { rotate: Some(RotateStep { degrees: 0, flip: flip.to_string() }), ..ImageEditSteps::default() },
            format: None,
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
        assert_eq!(read_tiff_pages(&split.output_paths()[0]).unwrap().len(), 1);
        assert_eq!(read_tiff_pages(&split.output_paths()[1]).unwrap().len(), 1);

        let first = path("first.tiff");
        let second = path("second.tiff");
        write_tiff_page(&first, &TiffPage::Gray { width: 1, height: 1, data: vec![1] }).unwrap();
        write_tiff_page(&second, &TiffPage::Gray { width: 1, height: 1, data: vec![2] }).unwrap();
        let combined = tiff(&TiffRequest { paths: vec![first.clone(), second.clone()], pages: None, output_location: OutputLocation::AlongsideInput, output_path: None });
        let pages = read_tiff_pages(&combined.output_paths()[0]).unwrap();
        assert_eq!(pages.len(), 2);
        assert!(matches!(&pages[0], TiffPage::Gray { data, .. } if data == &vec![1]));
        assert!(matches!(&pages[1], TiffPage::Gray { data, .. } if data == &vec![2]));

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
        write_tiff_page(&one_page, &TiffPage::Gray { width: 1, height: 1, data: vec![2] }).unwrap();

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
        let pages = read_tiff_pages(&output).unwrap();
        assert_eq!(pages.len(), 3);
        assert!(matches!(&pages[0], TiffPage::Gray { data, .. } if data == &vec![30, 40]));
        assert!(matches!(&pages[1], TiffPage::Gray { data, .. } if data == &vec![2]));
        assert!(matches!(&pages[2], TiffPage::Gray { data, .. } if data == &vec![10, 20]));

        let _ = std::fs::remove_file(two_page);
        let _ = std::fs::remove_file(one_page);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn text_watermark_composites_without_changing_alpha() {
        let input = path("watermark-input.png");
        let image = image::RgbaImage::from_pixel(64, 32, image::Rgba([10, 20, 30, 77]));
        image.save(&input).unwrap();
        let result = export_edit_stack(&text_watermark(&input, "TOOL", 50, 2), input.clone());
        let output = result.output_paths().first().unwrap();
        let output_image = image::open(output).unwrap().to_rgba8();
        assert!(output_image.pixels().any(|pixel| pixel.0[0..3] != [10, 20, 30]));
        assert!(output_image.pixels().all(|pixel| pixel.0[3] == 77));
        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn text_watermark_supports_ascii_latin_combining_and_non_latin_text() {
        let (canvas, _, _, _) = draw_text("ASCII Café e\u{301} Ж", WATERMARK_FONT_SIZE, [255, 255, 255], 200).unwrap();
        assert!(canvas.pixels().any(|pixel| pixel.0[3] > 0));

        for text in ["Watermark", "Café", "e\u{301}", "Ж"] {
            let (sample, _, _, _) = draw_text(text, WATERMARK_FONT_SIZE, [255, 255, 255], 255).unwrap();
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

        let result = export_edit_stack(&text_watermark(&input, "Café Ж \u{10FFFF}", 80, 4), input.clone());

        let error = result.error().expect("unsupported text must fail");
        assert_eq!(error.message, "The watermark font can't draw U+10FFFF.");
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
            let (canvas, _, _, _) = draw_text(text, WATERMARK_FONT_SIZE, [255, 255, 255], 173).unwrap();
            Sha256::digest(canvas.as_raw()).into()
        }

        assert_eq!(digest("Café e\u{301} Ж"), digest("Café e\u{301} Ж"));
    }

    #[test]
    fn tiled_turned_text_and_picture_marks_render_the_same_every_time() {
        let logo = path("watermark-digest-logo.png");
        image::RgbaImage::from_fn(24, 12, |x, y| Rgba([(x * 10) as u8, (y * 20) as u8, 200, 255])).save(&logo).unwrap();
        let source = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(160, 90, Rgba([90, 90, 90, 255])));
        let text = WatermarkOptions { text: Some("Café Ж".to_string()), logo_path: None, size: Some(0.2), opacity: 60, color: Some("#336699".to_string()), position: Some(WatermarkPosition::Placement(WatermarkPlacement::Tile)), rotation: -30.0 };
        let picture = WatermarkOptions { text: None, logo_path: Some(logo.clone()), color: None, rotation: 45.0, ..text.clone() };
        let anchored = WatermarkOptions { position: Some(WatermarkPosition::Anchor { anchor_x: 0.25, anchor_y: 0.75 }), ..picture.clone() };
        let digest = |options: &WatermarkOptions| -> [u8; 32] { Sha256::digest(apply_watermark(source.clone(), options).unwrap().to_rgba8().as_raw()).into() };
        for options in [&text, &picture, &anchored] {
            assert_eq!(digest(options), digest(options));
        }
        assert_ne!(digest(&picture), digest(&anchored));
        let _ = std::fs::remove_file(logo);
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

        let result = export_edit_stack(&text_watermark(&input, "Safe", 50, 4), input.clone());

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
        let (prefix, macos) = icon_plan("macos", &[]).unwrap();
        assert_eq!(prefix, "macos-icon");
        assert_eq!(macos, vec![16, 32, 128, 256, 512, 1024]);
        assert!(icon_plan("custom", &[0, 5000]).is_err());
        assert!(icon_plan("unknown", &[]).is_err());
    }

    #[test]
    fn macos_icon_preset_includes_a_valid_icns_container() {
        let input = path("macos-icon-source.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255])).save(&input).unwrap();
        let result = icon_set(&IconSetRequest { paths: vec![input.clone()], preset: "macos".to_string(), sizes: vec![], output_location: OutputLocation::AlongsideInput }, input.clone());
        assert!(result.error().is_none());
        let icns = result.output_paths().iter().find(|output| output.extension().is_some_and(|extension| extension == "icns")).unwrap();
        let bytes = std::fs::read(icns).unwrap();
        assert!(bytes.starts_with(b"icns"));
        assert_eq!(u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize, bytes.len());
        assert!(bytes.windows(4).any(|item| item == b"icp4"));
        for output in result.output_paths() { let _ = std::fs::remove_file(output); }
        let _ = std::fs::remove_file(input);
    }

    #[test]
    fn crop_aspect_ratio_is_anchored_and_bounds_checked() {
        let request = CropStep { x: 0, y: 0, width: 1, height: 1, mode: "aspectRatio".to_string(), aspect_width: 1, aspect_height: 1, anchor: "top".to_string() };
        assert_eq!(crop_rect(8, 4, &request).unwrap(), (2, 0, 4, 4));
        let start = CropStep { anchor: "start".to_string(), ..request.clone() };
        assert_eq!((crop_rect(8, 4, &start).unwrap(), crop_rect(4, 8, &start).unwrap()), ((0, 0, 4, 4), (0, 0, 4, 4)));
        let end = CropStep { anchor: "end".to_string(), ..request.clone() };
        assert_eq!((crop_rect(8, 4, &end).unwrap(), crop_rect(4, 8, &end).unwrap()), ((4, 0, 4, 4), (0, 4, 4, 4)));
        let invalid = CropStep { mode: "rectangle".to_string(), x: 7, y: 0, width: 2, height: 1, ..request };
        assert!(crop_rect(8, 4, &invalid).is_err());
    }

    #[test]
    fn rotate_mirror_preserves_pixels_without_resampling() {
        let input = path("rotate-input.png");
        let mut image = image::RgbaImage::new(2, 1);
        image.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        image.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        image.save(&input).unwrap();
        let result = export_edit_stack(&mirror(&input, "horizontal"), input.clone());
        let output = result.output_paths().first().unwrap();
        let pixels = image::open(output).unwrap().to_rgba8();
        assert_eq!(pixels.dimensions(), (2, 1));
        assert_eq!(pixels.get_pixel(0, 0).0, [0, 255, 0, 255]);
        let invalid = export_edit_stack(&mirror(&input, "diagonal"), input.clone());
        assert!(invalid.error().is_some());
        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn resize_modes_produce_bounded_dimensions() {
        let percentage = ResizeStep { width: 0, height: 0, mode: "percentage".to_string(), resampling: "lanczos".to_string(), keep_aspect_ratio: true, percentage: 50, longest_side: 0 };
        assert_eq!(resize_dimensions(400, 200, &percentage).unwrap(), (200, 100));
        let longest = ResizeStep { mode: "longestSide".to_string(), longest_side: 100, ..percentage.clone() };
        assert_eq!(resize_dimensions(400, 200, &longest).unwrap(), (100, 50));
        assert_eq!(resize_dimensions(200, 400, &longest).unwrap(), (50, 100));
        let invalid = ResizeStep { mode: "percentage".to_string(), percentage: 0, ..longest.clone() };
        assert!(resize_dimensions(400, 200, &invalid).is_err());
        let boxed = ResizeStep { mode: "exact".to_string(), width: 100, height: 100, ..percentage.clone() };
        assert_eq!(resize_dimensions(400, 200, &boxed).unwrap(), (100, 50));
        assert_eq!(resize_dimensions(400, 200, &ResizeStep { keep_aspect_ratio: false, ..boxed.clone() }).unwrap(), (100, 100));
        // A box past the limit is refused even when the lock would fit the image inside it.
        assert!(resize_dimensions(400, 200, &ResizeStep { width: 16385, ..boxed }).is_err());
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

        let request = CropStep {
            x: 0, y: 0, width: 1, height: 1, mode: "aspectRatio".to_string(),
            aspect_width: 1, aspect_height: 1, anchor: "right".to_string(),
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
