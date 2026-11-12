use base64::{engine::general_purpose::STANDARD, Engine};
use image::codecs::png::PngEncoder;
use image::{DynamicImage, ImageEncoder, ImageFormat, Rgba};
use image::AnimationDecoder;
use std::collections::{BTreeMap, BTreeSet};
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
    let source = crate::kit::images::load_upright_image(&request.path)?;
    let format = source.family.output_format();
    let (edited, format, quality) = apply_edit_plan(&plan, source.image, format)?;
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
        Ok(source) => source,
        Err(error) => return failure(input, error),
    };
    let format = source.family.output_format();
    let (edited, format, quality) = match apply_edit_plan(&plan, source.image, format) {
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
    let requested = match request.format.as_deref().map(parse_output_format).transpose() {
        Ok(requested) => requested,
        Err(error) => return JobOutcome::failure(input, ToolError::invalid_input(error)),
    };
    // Editing keeps one picture, so an animated GIF would lose its other frames. It fails before
    // anything is decoded or reserved and points to the tool that saves every frame.
    if crate::kit::images::is_animated_gif(&input) {
        return JobOutcome::failure(input, ToolError::invalid_input(ANIMATED_GIF).with_pointer(OutcomePointer::Tool { tool_id: "gif-extract".into() }));
    }
    let decoded = match crate::kit::images::load_upright_image(&input) {
        Ok(decoded) => decoded,
        Err(error) => return failure(input, error),
    };
    let format = requested.unwrap_or_else(|| decoded.family.output_format());
    let oriented = decoded.oriented;
    let source = decoded.image;
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
    let quality = crate::kit::images::DEFAULT_QUALITY;
    let source_width = image.width();
    let source_height = image.height();
    let mut geometry = Vec::new();
    for edit in &plan.edits {
        match edit {
            ImageEdit::Convert { format: requested } => format = parse_output_format(requested)?,
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

/// Convert Image Format's target: the editor's formats plus TIFF, whose
/// compression is accepted only with a TIFF target and defaults to LZW.
pub(crate) fn parse_convert_target(format: &str, tiff_compression: Option<crate::kit::contracts::TiffCompression>) -> Result<crate::kit::images::OutputFormat, String> {
    match (format.to_ascii_lowercase().as_str(), tiff_compression) {
        ("tif" | "tiff", compression) => Ok(crate::kit::images::OutputFormat::Tiff(compression.unwrap_or_default())),
        (_, Some(_)) => Err("TIFF compression applies only to a TIFF target.".to_string()),
        (_, None) => parse_output_format(format),
    }
}

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
    let image = crate::kit::images::load_upright_image(&request.path).map_err(|error| format!("Could not preview image: {error}"))?.image;
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

pub fn inspect_image_pages(request: &ImagePagesRequest) -> Vec<Option<usize>> {
    request.paths.iter().map(|path| crate::kit::images::source::page_count(path).ok()).collect()
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
    /// The ticked platforms, each with the sizes ticked for it.
    pub platforms: Vec<IconPlatform>,
    #[serde(default)] pub square_rule: SquareRule,
    pub output_location: OutputLocation,
}
/// One ticked platform and the sizes ticked for it. Only these sizes are made.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconPlatform {
    pub preset: String,
    pub sizes: Vec<u32>,
    /// The containers ticked for this platform. When it is missing, every container the
    /// preset offers is made whenever its sizes allow it.
    #[serde(default)]
    pub containers: Option<Vec<IconContainer>>,
}
/// A file that bundles a platform's sizes: an Xcode asset catalog folder, an ICNS or an ICO.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IconContainer {
    /// `AppIcon.appiconset` with its `Contents.json`, for macOS and iOS.
    Appiconset,
    /// The macOS `.icns`.
    Icns,
    /// The website `favicon.ico`.
    Ico,
}
impl IconContainer {
    /// The name failures and validation messages use for the container.
    fn name(self) -> &'static str {
        match self {
            IconContainer::Appiconset => "AppIcon.appiconset",
            IconContainer::Icns => "ICNS",
            IconContainer::Ico => "favicon.ico",
        }
    }
    /// The asset catalog and the ICNS hold every size of their preset, so they need all of them ticked.
    /// favicon.ico holds whichever sizes are ticked.
    fn needs_every_size(self) -> bool { !matches!(self, IconContainer::Ico) }
}
/// How a non-square master becomes square before its sizes are made. A master is never stretched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SquareRule {
    /// Centre the whole image on a transparent square of its longer side.
    #[default]
    Pad,
    /// Keep the largest centred square.
    Crop,
}
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GifCreateRequest {
    pub paths: Vec<PathBuf>,
    #[serde(default = "default_gif_delay_ms")] pub frame_delay_ms: u32,
    #[serde(default = "default_gif_loop")] pub loop_forever: bool,
    /// Sparse per-frame delays by 0-based frame position; every other frame uses `frame_delay_ms`.
    #[serde(default)] pub frame_delay_overrides: Vec<GifFrameDelay>,
    pub output_location: OutputLocation,
    #[serde(default)]
    pub output_path: Option<PathBuf>,
}
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GifFrameDelay { pub index: usize, pub delay_ms: u32 }
/// Frame delays a person can choose, in milliseconds.
pub const GIF_DELAY_RANGE_MS: std::ops::RangeInclusive<u32> = 1..=60_000;
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GifExtractRequest { pub paths: Vec<PathBuf>, pub output_location: OutputLocation }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TiffPageRef { pub path: PathBuf, pub page: usize }
/// The person picks the mode; it is never inferred from how many files were chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TiffMode { Split, Combine }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TiffRequest {
    pub mode: TiffMode,
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
pub struct ImagePagesRequest { pub paths: Vec<PathBuf> }
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
            let mut logo = crate::kit::images::decode(path)?.into_first_page().to_rgba8();
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

/// Draws the text into a layer just big enough for its glyphs. Returns the layer, the offset of its
/// top-left corner from the pen's starting point, and how far the pen advanced.
fn draw_text(text: &str, font_size: f32, color: [u8; 3], alpha: u8) -> Result<(image::RgbaImage, i64, i64, f32), String> {
    let bytes = bundled_text_font_bytes()?;
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
/// Makes one master's icon set in its own `{stem}-icons` folder, with a folder for each
/// ticked platform inside. The folder is built in a hidden staging folder and published in
/// one rename only when every size and container worked, so a failed master leaves nothing behind.
pub fn icon_set(request: &IconSetRequest, input: PathBuf) -> JobOutcome {
    icon_set_with(
        request,
        input,
        |icon, output| icon.save_with_format(output, ImageFormat::Png).map_err(|error| error.to_string()),
        |output, bytes| std::fs::write(output, bytes).map_err(|error| error.to_string()),
    )
}

/// `save` writes each size's PNG and `write` writes every container file, so a test can fail either.
fn icon_set_with(
    request: &IconSetRequest,
    input: PathBuf,
    mut save: impl FnMut(&DynamicImage, &Path) -> Result<(), String>,
    mut write: impl FnMut(&Path, &[u8]) -> Result<(), String>,
) -> JobOutcome {
    let plans = match icon_plans(&request.platforms) { Ok(plans) => plans, Err(error) => return failure(input, error) };
    // The master is read upright and reduced to 8-bit RGBA once, keeping its transparency, then made square by the run's rule.
    let (master, high_bit_depth) = match crate::kit::images::load_upright_image(&input) {
        Ok(source) => (square_master(source.image.to_rgba8(), request.square_rule), source.high_bit_depth),
        Err(_) => return failure(input.clone(), format!("Not a readable {} image. The file may be damaged. Nothing was saved.", image_kind(&input))),
    };
    let folder = match OutputNaming::reserve_staged_folder(&input, &request.output_location, ICON_SET_SUFFIX) {
        Ok(folder) => folder,
        Err(error) => return failure(input, format!("Could not make the icon folder: {error}. Nothing was saved for this image.")),
    };
    let stem = input.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let mut files = 0;
    for plan in &plans {
        let platform = folder.path().join(plan.folder);
        if let Err(error) = std::fs::create_dir(&platform) {
            return failure(input, format!("Could not make the {} folder: {error}. Nothing was saved for this image.", plan.folder));
        }
        let mut pngs = BTreeMap::new();
        for &size in &plan.sizes {
            let output = platform.join(format!("{}-{size}.png", plan.file_prefix));
            let icon = master.resize_exact(size, size, image::imageops::FilterType::Lanczos3);
            let icon = if plan.opaque { DynamicImage::ImageRgb8(flatten_on_white(&icon.to_rgba8())) } else { icon };
            let saved = save(&icon, &output).and_then(|()| std::fs::read(&output).map_err(|error| error.to_string()));
            match saved {
                Ok(bytes) => { pngs.insert(size, bytes); }
                Err(error) => return failure(input, format!("The {} {size} size failed: {error}. Nothing was saved for this image.", plan.label)),
            }
        }
        files += pngs.len();
        for &container in &plan.containers {
            match write_container(container, plan, &platform, &stem, &pngs, &mut write) {
                Ok(written) => files += written,
                Err(error) => return failure(input, format!("The {} {} failed: {error}. Nothing was saved for this image.", plan.label, container.name())),
            }
        }
    }
    let expected = folder.destination_path().with_file_name(format!("{stem}{ICON_SET_SUFFIX}"));
    let largest = plans.iter().flat_map(|plan| plan.sizes.iter().copied()).max().unwrap_or(0);
    let side = master.width();
    match folder.publish() {
        Ok(published) => {
            let outcome = JobOutcome::succeeded(input, vec![published], format!("{files} files"));
            let outcome = if side < largest {
                outcome.with_warning(OutcomeWarning { kind: WarningKind::Upscaled { from: side, to: largest }, message: format!("Upscaled to {largest} from {side} pixels.") })
            } else {
                outcome
            };
            let outcome = crate::kit::common::note_renamed(outcome, Some(&expected));
            match high_bit_depth {
                Some(bits) => outcome.with_warning(precision_reduced(format!("{bits}-bit HEIC saved as 8-bit sRGB."))),
                None => outcome,
            }
        }
        Err(error) => failure(input, format!("Could not save the icon folder: {error}. Nothing was saved for this image.")),
    }
}

/// iPhone and iPad icons can't be transparent, so transparent areas are filled with white and
/// the PNG has no alpha channel. Integer blending keeps the result the same on every machine.
fn flatten_on_white(icon: &image::RgbaImage) -> image::RgbImage {
    image::RgbImage::from_fn(icon.width(), icon.height(), |x, y| {
        let [red, green, blue, alpha] = icon.get_pixel(x, y).0;
        let blend = |channel: u8| ((u32::from(channel) * u32::from(alpha) + 255 * (255 - u32::from(alpha)) + 127) / 255) as u8;
        image::Rgb([blend(red), blend(green), blend(blue)])
    })
}

/// Writes one container into the platform folder from the PNGs already made, and returns how many files it wrote.
fn write_container(
    container: IconContainer,
    plan: &IconPlan,
    platform: &Path,
    stem: &str,
    pngs: &BTreeMap<u32, Vec<u8>>,
    write: &mut impl FnMut(&Path, &[u8]) -> Result<(), String>,
) -> Result<usize, String> {
    let png = |size: u32| pngs.get(&size).map(Vec::as_slice).ok_or_else(|| format!("the {size} size is missing"));
    match container {
        IconContainer::Appiconset => {
            let slots = match plan.folder { "macos" => MACOS_APPICONSET, "ios" => IOS_APPICONSET, _ => return Err("this platform has no asset catalog".into()) };
            let catalog = platform.join("AppIcon.appiconset");
            std::fs::create_dir(&catalog).map_err(|error| error.to_string())?;
            for slot in slots { write(&catalog.join(slot.filename()), png(slot.pixels())?)?; }
            write(&catalog.join("Contents.json"), appiconset_contents(slots).as_bytes())?;
            Ok(slots.len() + 1)
        }
        IconContainer::Icns => {
            let items = ICNS_TYPES.iter().map(|(code, size)| Ok((*code, png(*size)?))).collect::<Result<Vec<_>, String>>()?;
            write(&platform.join(format!("{stem}.icns")), &icns_bytes(&items)?)?;
            Ok(1)
        }
        IconContainer::Ico => {
            let images = plan.sizes.iter().map(|&size| Ok((size, png(size)?))).collect::<Result<Vec<_>, String>>()?;
            write(&platform.join("favicon.ico"), &ico_bytes(&images)?)?;
            Ok(1)
        }
    }
}

const ICON_SET_SUFFIX: &str = "-icons";

/// Makes the master square by the run's rule, so every size keeps the master's pixel aspect.
/// A square master is returned as it is.
fn square_master(master: image::RgbaImage, rule: SquareRule) -> DynamicImage {
    let (width, height) = master.dimensions();
    if width == height { return DynamicImage::ImageRgba8(master); }
    match rule {
        SquareRule::Pad => {
            let side = width.max(height);
            let mut square = image::RgbaImage::from_pixel(side, side, Rgba([0, 0, 0, 0]));
            image::imageops::replace(&mut square, &master, i64::from((side - width) / 2), i64::from((side - height) / 2));
            DynamicImage::ImageRgba8(square)
        }
        SquareRule::Crop => {
            let side = width.min(height);
            DynamicImage::ImageRgba8(image::imageops::crop_imm(&master, (width - side) / 2, (height - side) / 2, side, side).to_image())
        }
    }
}

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

fn precision_reduced(message: String) -> crate::kit::contracts::OutcomeWarning {
    crate::kit::contracts::OutcomeWarning { kind: crate::kit::contracts::WarningKind::PrecisionReduced, message }
}

/// The ICNS types written for each macOS size, in the order of the asset catalog's slots:
/// each point size at 1x, then 2x. The 2x types (`ic11` to `ic14`, and `ic10` for 512@2x) let
/// Retina displays pick the sharper image.
const ICNS_TYPES: [([u8; 4], u32); 10] = [
    (*b"icp4", 16), (*b"ic11", 32),
    (*b"icp5", 32), (*b"ic12", 64),
    (*b"ic07", 128), (*b"ic13", 256),
    (*b"ic08", 256), (*b"ic14", 512),
    (*b"ic09", 512), (*b"ic10", 1024),
];

/// An ICNS file: the `icns` header and its total length, then each type with its length and PNG.
fn icns_bytes(items: &[([u8; 4], &[u8])]) -> Result<Vec<u8>, String> {
    let total = items.iter().try_fold(8usize, |total, (_, png)| total.checked_add(png.len() + 8)).ok_or("the ICNS is too large")?;
    let total = u32::try_from(total).map_err(|_| "the ICNS is too large".to_string())?;
    let mut bytes = Vec::with_capacity(total as usize);
    bytes.extend_from_slice(b"icns");
    bytes.extend_from_slice(&total.to_be_bytes());
    for (code, png) in items {
        bytes.extend_from_slice(code);
        bytes.extend_from_slice(&u32::try_from(png.len() + 8).map_err(|_| "an ICNS image is too large".to_string())?.to_be_bytes());
        bytes.extend_from_slice(png);
    }
    Ok(bytes)
}

/// An ICO file holding each size as a 32-bit PNG image, smallest first. Every current browser
/// and Windows Vista onwards read PNG images in an ICO. A width or height of 256 is written as 0.
fn ico_bytes(images: &[(u32, &[u8])]) -> Result<Vec<u8>, String> {
    if images.is_empty() || images.len() > usize::from(u16::MAX) { return Err("an ICO needs 1 or more sizes".into()); }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (size, png) in images {
        let side = match size { 1..=255 => *size as u8, 256 => 0, _ => return Err(format!("an ICO can't hold {size} pixels")) };
        bytes.extend_from_slice(&[side, side, 0, 0]);
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(png.len()).map_err(|_| "an ICO image is too large".to_string())?.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(offset).map_err(|_| "the ICO is too large".to_string())?.to_le_bytes());
        offset += png.len();
    }
    for (_, png) in images { bytes.extend_from_slice(png); }
    Ok(bytes)
}

/// One image slot of an Xcode asset catalog app icon: who it is for, its size in points and its scale.
struct AppIconSlot {
    idiom: &'static str,
    /// The point size as Xcode writes it, which can be fractional (83.5 for iPad Pro).
    points: &'static str,
    scale: u32,
}

impl AppIconSlot {
    const fn new(idiom: &'static str, points: &'static str, scale: u32) -> Self { Self { idiom, points, scale } }

    /// The image's side in pixels: the point size times the scale.
    fn pixels(&self) -> u32 {
        let tenths: u32 = match self.points.split_once('.') {
            Some((whole, fraction)) => whole.parse::<u32>().unwrap_or(0) * 10 + fraction.parse::<u32>().unwrap_or(0),
            None => self.points.parse::<u32>().unwrap_or(0) * 10,
        };
        tenths * self.scale / 10
    }

    /// One file per slot, so Xcode never sees a file shared between slots.
    fn filename(&self) -> String {
        if self.idiom == "mac" {
            let suffix = if self.scale == 1 { String::new() } else { format!("@{}x", self.scale) };
            format!("icon_{0}x{0}{suffix}.png", self.points)
        } else {
            format!("{}-{}@{}x.png", self.idiom, self.points, self.scale)
        }
    }
}

/// The macOS app icon: 16, 32, 128, 256 and 512 points at 1x and 2x. Their pixels are exactly
/// the seven macOS sizes, which is why the asset catalog needs every one ticked.
const MACOS_APPICONSET: &[AppIconSlot] = &[
    AppIconSlot::new("mac", "16", 1), AppIconSlot::new("mac", "16", 2),
    AppIconSlot::new("mac", "32", 1), AppIconSlot::new("mac", "32", 2),
    AppIconSlot::new("mac", "128", 1), AppIconSlot::new("mac", "128", 2),
    AppIconSlot::new("mac", "256", 1), AppIconSlot::new("mac", "256", 2),
    AppIconSlot::new("mac", "512", 1), AppIconSlot::new("mac", "512", 2),
];

/// Xcode's "All Sizes" iOS app icon: iPhone, iPad and the 1024 App Store icon. Their pixels are
/// exactly the 13 iPhone and iPad sizes.
const IOS_APPICONSET: &[AppIconSlot] = &[
    AppIconSlot::new("iphone", "20", 2), AppIconSlot::new("iphone", "20", 3),
    AppIconSlot::new("iphone", "29", 2), AppIconSlot::new("iphone", "29", 3),
    AppIconSlot::new("iphone", "40", 2), AppIconSlot::new("iphone", "40", 3),
    AppIconSlot::new("iphone", "60", 2), AppIconSlot::new("iphone", "60", 3),
    AppIconSlot::new("ipad", "20", 1), AppIconSlot::new("ipad", "20", 2),
    AppIconSlot::new("ipad", "29", 1), AppIconSlot::new("ipad", "29", 2),
    AppIconSlot::new("ipad", "40", 1), AppIconSlot::new("ipad", "40", 2),
    AppIconSlot::new("ipad", "76", 1), AppIconSlot::new("ipad", "76", 2),
    AppIconSlot::new("ipad", "83.5", 2),
    AppIconSlot::new("ios-marketing", "1024", 1),
];

/// `Contents.json` as Xcode writes it: keys in alphabetical order, two-space indents and
/// `" : "` separators, with no dates, so the same slots always give the same bytes.
fn appiconset_contents(slots: &[AppIconSlot]) -> String {
    let images = slots.iter().map(|slot| format!(
        "    {{\n      \"filename\" : \"{filename}\",\n      \"idiom\" : \"{idiom}\",\n      \"scale\" : \"{scale}x\",\n      \"size\" : \"{points}x{points}\"\n    }}",
        filename = slot.filename(), idiom = slot.idiom, scale = slot.scale, points = slot.points,
    )).collect::<Vec<_>>();
    format!("{{\n  \"images\" : [\n{}\n  ],\n  \"info\" : {{\n    \"author\" : \"xcode\",\n    \"version\" : 1\n  }}\n}}\n", images.join(",\n"))
}

/// The sizes each preset offers, as the design lists them. A request may tick any of them.
const MACOS_ICON_SIZES: [u32; 7] = [16, 32, 64, 128, 256, 512, 1024];
const IOS_ICON_SIZES: [u32; 13] = [20, 29, 40, 58, 60, 76, 80, 87, 120, 152, 167, 180, 1024];
const ANDROID_ICON_SIZES: [u32; 6] = [48, 72, 96, 144, 192, 512];
const WEBSITE_ICON_SIZES: [u32; 6] = [16, 32, 48, 64, 128, 256];
const CUSTOM_ICON_SIZES: std::ops::RangeInclusive<u32> = 1..=4096;

/// Where a platform's icons go inside the set folder, how they are named, the name a
/// failure uses for the platform, the sizes ticked for it, smallest first, and its containers.
#[derive(Debug, PartialEq, Eq)]
struct IconPlan {
    folder: &'static str,
    file_prefix: &'static str,
    label: &'static str,
    sizes: Vec<u32>,
    containers: Vec<IconContainer>,
    /// iPhone and iPad icons are filled with white, as they can't be transparent.
    opaque: bool,
}

/// Checks the ticked platforms and their sizes, and orders them the same way every run.
fn icon_plans(platforms: &[IconPlatform]) -> Result<Vec<IconPlan>, String> {
    if platforms.is_empty() { return Err("Tick at least one platform to make icons for.".to_string()); }
    let mut plans = platforms.iter().map(icon_plan).collect::<Result<Vec<_>, _>>()?;
    plans.sort_by_key(|plan| ["macos", "ios", "android", "web", "custom"].iter().position(|folder| *folder == plan.folder));
    if plans.windows(2).any(|pair| pair[0].folder == pair[1].folder) { return Err("Each platform can be ticked only once.".to_string()); }
    Ok(plans)
}

fn icon_plan(platform: &IconPlatform) -> Result<IconPlan, String> {
    let (folder, file_prefix, label, offered, containers): (_, _, _, &[u32], &[IconContainer]) = match platform.preset.as_str() {
        "macos" => ("macos", "macos", "macOS", &MACOS_ICON_SIZES, &[IconContainer::Appiconset, IconContainer::Icns]),
        "favicon" => ("web", "favicon", "Website", &WEBSITE_ICON_SIZES, &[IconContainer::Ico]),
        "ios" => ("ios", "ios", "iOS", &IOS_ICON_SIZES, &[IconContainer::Appiconset]),
        "android" => ("android", "android", "Android", &ANDROID_ICON_SIZES, &[]),
        "custom" => ("custom", "custom", "custom", &[], &[]),
        preset => return Err(format!("Unsupported icon preset: {preset}.")),
    };
    let sizes = platform.sizes.iter().copied().collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>();
    if sizes.is_empty() { return Err(format!("Tick at least one {label} size.")); }
    if folder == "custom" {
        if let Some(size) = sizes.iter().find(|size| !CUSTOM_ICON_SIZES.contains(size)) {
            return Err(format!("A custom size of {size} pixels is not possible. Use sizes from 1 to 4096 pixels."));
        }
    } else if let Some(size) = sizes.iter().find(|size| !offered.contains(size)) {
        return Err(format!("{label} icons don't come in {size} pixels."));
    }
    let missing = offered.iter().copied().filter(|size| !sizes.contains(size)).collect::<Vec<_>>();
    let available = |container: &IconContainer| !container.needs_every_size() || missing.is_empty();
    let containers = match &platform.containers {
        None => containers.iter().copied().filter(available).collect::<Vec<_>>(),
        Some(ticked) => {
            let ticked = ticked.iter().copied().collect::<BTreeSet<_>>();
            if let Some(container) = ticked.iter().find(|container| !containers.contains(container)) {
                return Err(format!("{label} icons don't come as {}.", container.name()));
            }
            if let Some(container) = ticked.iter().find(|container| !available(container)) {
                let list = missing.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
                return Err(format!("The {} needs every {label} size. Tick {list} too, or untick it.", container.name()));
            }
            ticked.into_iter().collect()
        }
    };
    Ok(IconPlan { folder, file_prefix, label, sizes, containers, opaque: folder == "ios" })
}

/// Names an unreadable frame the way the Create GIF results show it.
fn unreadable_frame(number: usize, path: &Path) -> String {
    if !path.is_file() {
        return format!("Frame {number} is no longer at {}. Replace or remove frame {number}, then make the GIF again. Nothing was saved.", path.display());
    }
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    let kind = path.extension().map(|extension| format!("{} image", extension.to_string_lossy().to_uppercase())).unwrap_or_else(|| "image".to_string());
    format!("Frame {number}, {name}, isn't a readable {kind}. Replace or remove frame {number}, then make the GIF again. Nothing was saved.")
}

/// Writes the frames as a GIF the size of the first frame. Every frame is
/// centred, larger ones scaled down to fit, and quantised to its own palette
/// with a reserved transparent index, so the same frames always give the same bytes.
fn encode_gif<W: std::io::Write>(writer: W, frames: &[image::RgbaImage], request: &GifCreateRequest) -> Result<W, String> {
    use crate::kit::images::gif_frames::{place_on_canvas, quantise, TRANSPARENT_INDEX};
    let canvas = frames[0].dimensions();
    let (Ok(width), Ok(height)) = (u16::try_from(canvas.0), u16::try_from(canvas.1)) else {
        return Err(format!("Frame 1 is {} × {} pixels, larger than a GIF can hold (65535 × 65535).", canvas.0, canvas.1));
    };
    let mut encoder = gif::Encoder::new(writer, width, height, &[]).map_err(|error| format!("Could not create GIF: {error}"))?;
    if request.loop_forever { encoder.set_repeat(gif::Repeat::Infinite).map_err(|error| format!("Could not configure GIF loop: {error}"))?; }
    let delays = gif_frame_delays(request);
    for (frame, delay) in frames.iter().zip(delays) {
        let quantised = quantise(&place_on_canvas(frame, canvas));
        let frame = gif::Frame {
            width,
            height,
            delay,
            dispose: gif::DisposalMethod::Background,
            transparent: Some(TRANSPARENT_INDEX),
            palette: Some(quantised.palette),
            buffer: std::borrow::Cow::Owned(quantised.indices),
            ..gif::Frame::default()
        };
        encoder.write_frame(&frame).map_err(|error| format!("Could not create GIF: {error}"))?;
    }
    encoder.into_inner().map_err(|error| format!("Could not create GIF: {error}"))
}

/// Each frame's delay in GIF hundredths of a second, in frame order: its own
/// delay when it has one, otherwise the delay for every frame. Milliseconds
/// round to the nearest hundredth, and never to zero, which players ignore.
fn gif_frame_delays(request: &GifCreateRequest) -> Vec<u16> {
    let mut delays = vec![request.frame_delay_ms; request.paths.len()];
    for frame in &request.frame_delay_overrides { delays[frame.index] = frame.delay_ms; }
    delays.into_iter().map(|ms| ((ms + 5) / 10).max(1) as u16).collect()
}

/// Rejects delays outside 1 to 60000 ms and overrides that name no frame,
/// before any frame is read, attributing the failure to the frame it concerns.
fn validate_gif_timing(request: &GifCreateRequest) -> Result<(), JobOutcome> {
    let invalid = |path: &PathBuf, message: String| JobOutcome::failure(path.clone(), ToolError::invalid_input(message));
    let first = &request.paths[0];
    if !GIF_DELAY_RANGE_MS.contains(&request.frame_delay_ms) {
        return Err(invalid(first, format!("The delay for every frame is {} ms. Choose 1 to 60000 ms. Nothing was saved.", request.frame_delay_ms)));
    }
    let mut seen = BTreeSet::new();
    for frame in &request.frame_delay_overrides {
        let number = frame.index + 1;
        let Some(path) = request.paths.get(frame.index) else {
            return Err(invalid(first, format!("There's no frame {number} to give its own delay. Nothing was saved.")));
        };
        if !seen.insert(frame.index) {
            return Err(invalid(path, format!("Frame {number} has more than one own delay. Nothing was saved.")));
        }
        if !GIF_DELAY_RANGE_MS.contains(&frame.delay_ms) {
            return Err(invalid(path, format!("Frame {number}'s own delay is {} ms. Choose 1 to 60000 ms. Nothing was saved.", frame.delay_ms)));
        }
    }
    Ok(())
}

pub fn gif_create(request: &GifCreateRequest) -> JobOutcome {
    let Some(input) = request.paths.first().cloned() else { return failure(PathBuf::new(), "Select at least one image.".to_string()); };
    if let Err(outcome) = validate_gif_timing(request) { return outcome; }
    // Read every frame before reserving the output, so an unreadable frame publishes nothing.
    let mut reduced = Vec::new();
    let mut images = Vec::with_capacity(request.paths.len());
    for (index, path) in request.paths.iter().enumerate() {
        match crate::kit::images::load_upright_image(path) {
            Ok(source) => {
                if let Some(bits) = source.high_bit_depth { reduced.push((index + 1, path, bits)); }
                images.push(source.image.to_rgba8());
            }
            Err(_) => return failure(path.clone(), unreadable_frame(index + 1, path)),
        }
    }
    let (output, file) = match OutputNaming::create_single_output(request.output_path.as_deref(), &input, &request.output_location, "-animated", "gif") {
        Ok(destination) => destination,
        Err(error) => return failure(input, error.to_string()),
    };
    let encode_result = encode_gif(std::io::BufWriter::new(file), &images, request)
        .and_then(|writer| writer.into_inner().map_err(|error| format!("Could not create GIF: {}", error.error())).map(drop));
    match encode_result {
        Ok(_) => match output.publish() {
            Ok(path) => {
                let outcome = JobOutcome::succeeded(input, vec![path], "GIF saved");
                match reduced.as_slice() {
                    [] => outcome,
                    [(frame, path, bits)] => {
                        let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
                        outcome.with_warning(precision_reduced(format!("Frame {frame} ({name}) is {bits}-bit HEIC, saved as 8-bit sRGB.")))
                    }
                    frames => outcome.with_warning(precision_reduced(format!("{} frames are high-bit-depth HEIC, saved as 8-bit sRGB.", frames.len()))),
                }
            }
            Err(error) => failure(input, format!("Could not publish GIF output: {error}")),
        },
        Err(error) => failure(input, error),
    }
}

fn default_gif_delay_ms() -> u32 { 100 }
fn default_gif_loop() -> bool { true }

pub fn gif_extract(request: &GifExtractRequest, input: PathBuf) -> JobOutcome {
    super::gif_extract::extract(request, input)
}

pub const TIFF_SINGLE_PAGE_SKIP: &str = "Only 1 page, so there's nothing to split. No file was made.";

/// Split mode, one input at a time: each page becomes its own TIFF named with a padded page
/// number so the names sort. A one-page TIFF is skipped rather than copied.
pub fn tiff_split(request: &TiffRequest, input: PathBuf) -> JobOutcome {
    if request.pages.is_some() {
        return JobOutcome::failure(input, ToolError::invalid_input("Split mode writes every page of each TIFF; choosing pages belongs to Combine mode."));
    }
    let pages = match crate::kit::images::decode(&input) {
        Ok(decoded) => decoded.pages,
        Err(error) => return failure(input.clone(), format!("Could not process {}: {error}", input.display())),
    };
    if pages.len() == 1 {
        return JobOutcome::skipped(input, SkipReason::new(SkipCode::NoEffectiveChange, TIFF_SINGLE_PAGE_SKIP));
    }
    let width = pages.len().to_string().len().max(2);
    let mut outputs = Vec::with_capacity(pages.len());
    for (index, page) in pages.iter().enumerate() {
        let output = match OutputNaming::reserve_destination(&input, &request.output_location, &format!("-page-{:0width$}", index + 1), "tif") {
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
    JobOutcome::succeeded(input, outputs, "TIFF pages saved")
}

/// Combine mode: every chosen page goes into one TIFF, even when only one file was chosen.
pub fn tiff_combine(request: &TiffRequest) -> JobOutcome {
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
    let output = match OutputNaming::reserve_single_output(request.output_path.as_deref(), &input, &request.output_location, "-combined", "tiff") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve combined TIFF output: {error}")),
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
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::processing(error)) }

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    fn path(name: &str) -> PathBuf { std::env::temp_dir().join(format!("toolbox_tiff_{}_{}", std::process::id(), name)) }
    fn gray(width: u32, height: u32, data: Vec<u8>) -> DynamicImage { DynamicImage::ImageLuma8(image::ImageBuffer::from_raw(width, height, data).unwrap()) }
    fn read_pages(path: &Path) -> Result<Vec<DynamicImage>, String> { crate::kit::images::decode(path).map(|decoded| decoded.pages) }

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

    fn write_gray_tiff(path: &Path, pages: &[&[u8]]) {
        let file = File::create(path).unwrap();
        let mut encoder = tiff::encoder::TiffEncoder::new(file).unwrap();
        for data in pages { encoder.write_image::<tiff::encoder::colortype::Gray8>(data.len() as u32, 1, data).unwrap(); }
    }

    fn request(mode: TiffMode, paths: Vec<PathBuf>) -> TiffRequest {
        TiffRequest { mode, paths, pages: None, output_location: OutputLocation::AlongsideInput, output_path: None }
    }

    fn file_name(path: &Path) -> String { path.file_name().unwrap().to_string_lossy().into_owned() }

    #[test]
    fn split_writes_each_page_with_padded_sortable_names() {
        let input = path("split-source.tiff");
        write_gray_tiff(&input, &[&[10, 20], &[30, 40]]);
        let stem = file_name(&input).trim_end_matches(".tiff").to_string();

        let split = tiff_split(&request(TiffMode::Split, vec![input.clone()]), input.clone());
        assert!(split.error().is_none(), "split failed: {:?}", split.error());
        let names = split.output_paths().iter().map(|output| file_name(output)).collect::<Vec<_>>();
        assert_eq!(names, vec![format!("{stem}-page-01.tif"), format!("{stem}-page-02.tif")]);
        let first_pages = read_pages(&split.output_paths()[0]).unwrap();
        assert_eq!(first_pages.len(), 1);
        assert_eq!(first_pages[0].as_bytes(), &[10, 20]);
        let second_pages = read_pages(&split.output_paths()[1]).unwrap();
        assert_eq!(second_pages.len(), 1);
        assert_eq!(second_pages[0].as_bytes(), &[30, 40]);

        let again = tiff_split(&request(TiffMode::Split, vec![input.clone()]), input.clone());
        assert_eq!(file_name(&again.output_paths()[0]), format!("{stem}-page-01-1.tif"), "a second split must not overwrite the first");

        for output in split.output_paths().iter().chain(again.output_paths()) { let _ = std::fs::remove_file(output); }
        let _ = std::fs::remove_file(input);
    }

    #[test]
    fn split_pads_page_numbers_to_the_page_count_width() {
        let input = path("split-hundred.tiff");
        let pages = (0..100u8).map(|value| vec![value]).collect::<Vec<_>>();
        write_gray_tiff(&input, &pages.iter().map(Vec::as_slice).collect::<Vec<_>>());

        let split = tiff_split(&request(TiffMode::Split, vec![input.clone()]), input.clone());
        let names = split.output_paths().iter().map(|output| file_name(output)).collect::<Vec<_>>();
        assert_eq!(names.len(), 100);
        assert!(names[0].ends_with("-page-001.tif") && names[99].ends_with("-page-100.tif"), "{:?}", (&names[0], &names[99]));
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(sorted, names, "padded page names must sort in page order");

        for output in split.output_paths() { let _ = std::fs::remove_file(output); }
        let _ = std::fs::remove_file(input);
    }

    #[test]
    fn split_skips_a_single_page_tiff_without_writing_anything() {
        let input = path("split-single.tiff");
        write_gray_tiff(&input, &[&[7]]);
        let original = std::fs::read(&input).unwrap();

        let outcome = tiff_split(&request(TiffMode::Split, vec![input.clone()]), input.clone());
        let crate::kit::contracts::OutcomeState::Skipped { reason } = &outcome.state else { panic!("expected skipped, got {outcome:?}") };
        assert_eq!(reason.code, SkipCode::NoEffectiveChange);
        assert_eq!(reason.message, TIFF_SINGLE_PAGE_SKIP);
        assert!(outcome.output_paths().is_empty());
        let stem = file_name(&input).trim_end_matches(".tiff").to_string();
        let leftovers = std::fs::read_dir(std::env::temp_dir()).unwrap()
            .filter_map(Result::ok)
            .filter(|entry| file_name(&entry.path()).starts_with(&format!("{stem}-")))
            .count();
        assert_eq!(leftovers, 0, "a skipped TIFF must leave no output behind");
        assert_eq!(std::fs::read(&input).unwrap(), original);
        let _ = std::fs::remove_file(input);
    }

    #[test]
    fn split_rejects_a_page_selection() {
        let input = path("split-selection.tiff");
        write_gray_tiff(&input, &[&[1], &[2]]);
        let mut selection = request(TiffMode::Split, vec![input.clone()]);
        selection.pages = Some(vec![TiffPageRef { path: input.clone(), page: 1 }]);
        let outcome = tiff_split(&selection, input.clone());
        assert_eq!(outcome.error().map(|error| error.kind.clone()), Some(crate::kit::contracts::ErrorKind::InvalidInput));
        assert!(outcome.output_paths().is_empty());
        let _ = std::fs::remove_file(input);
    }

    #[test]
    fn combine_follows_combine_mode_for_one_file_and_for_several() {
        let single = path("combine-single.tiff");
        write_gray_tiff(&single, &[&[10, 20], &[30, 40]]);
        let one = tiff_combine(&request(TiffMode::Combine, vec![single.clone()]));
        assert_eq!(one.output_paths().len(), 1, "one file in Combine mode makes one TIFF, not a split");
        assert!(file_name(&one.output_paths()[0]).ends_with("-combined.tiff"));
        assert_eq!(read_pages(&one.output_paths()[0]).unwrap().len(), 2);

        let first = path("first.tiff");
        let second = path("second.tiff");
        write_tiff_page(&first, &gray(1, 1, vec![1])).unwrap();
        write_tiff_page(&second, &gray(1, 1, vec![2])).unwrap();
        let combined = tiff_combine(&request(TiffMode::Combine, vec![first.clone(), second.clone()]));
        let pages = read_pages(&combined.output_paths()[0]).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].as_bytes(), &[1]);
        assert_eq!(pages[1].as_bytes(), &[2]);

        for output in one.output_paths().iter().chain(combined.output_paths()) { let _ = std::fs::remove_file(output); }
        for input in [single, first, second] { let _ = std::fs::remove_file(input); }
    }

    #[test]
    fn tiff_requests_require_an_explicit_mode() {
        let missing = serde_json::json!({ "paths": ["/a.tiff"], "outputLocation": "alongsideInput" });
        assert!(serde_json::from_value::<TiffRequest>(missing).is_err(), "the mode must never be inferred");
        for (wire, mode) in [("split", TiffMode::Split), ("combine", TiffMode::Combine)] {
            let request = serde_json::from_value::<TiffRequest>(serde_json::json!({ "mode": wire, "paths": ["/a.tiff"], "outputLocation": "alongsideInput" })).unwrap();
            assert_eq!(request.mode, mode);
        }
    }

    #[test]
    fn rejects_unsupported_tiff_bit_depths() {
        let input = path("unsupported.tiff");
        let file = File::create(&input).unwrap();
        let mut encoder = tiff::encoder::TiffEncoder::new(file).unwrap();
        encoder.write_image::<tiff::encoder::colortype::Gray16>(1, 1, &[1]).unwrap();
        let split = tiff_split(&request(TiffMode::Split, vec![input.clone()]), input.clone());
        assert!(split.error().is_some());
        let combined = tiff_combine(&request(TiffMode::Combine, vec![input.clone()]));
        assert!(combined.error().is_some());
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

        let outcome = tiff_combine(&TiffRequest {
            mode: TiffMode::Combine,
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
        let result = gif_create(&GifCreateRequest { paths: vec![first.clone(), second.clone()], frame_delay_ms: 250, loop_forever: true, frame_delay_overrides: Vec::new(), output_location: OutputLocation::AlongsideInput, output_path: None });
        let output = result.output_paths().first().unwrap();
        let file = File::open(output).unwrap();
        let decoder = image::codecs::gif::GifDecoder::new(BufReader::new(file)).unwrap();
        let frames = decoder.into_frames().collect_frames().unwrap();
        assert_eq!(frames.len(), 2);
        // The canvas is the first frame's size; the larger second frame is scaled down into it.
        assert_eq!(frames[0].buffer().dimensions(), (1, 1));
        assert_eq!(frames[0].delay().numer_denom_ms(), (250, 1));
        assert_eq!(frames[1].buffer().get_pixel(0, 0).0, [0, 255, 0, 255]);
        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
        let _ = std::fs::remove_file(output);
    }

    fn gif_sandbox(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "toolbox_gif_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(root.join("outputs")).unwrap();
        root
    }

    /// Frames of different sizes and alpha: a 40×30 first frame with a
    /// half-transparent strip, a smaller 10×6 frame, a wider 80×20 frame and a
    /// taller 20×60 frame with a gradient (more than 255 colours).
    fn mixed_gif_frames(root: &Path) -> Vec<PathBuf> {
        let frames = [
            ("1-first.png", image::RgbaImage::from_fn(40, 30, |x, y| image::Rgba([200, 40, (x * 6) as u8, if y < 5 { 60 } else { 255 }]))),
            ("2-small.png", image::RgbaImage::from_pixel(10, 6, image::Rgba([20, 200, 20, 255]))),
            ("3-wide.png", image::RgbaImage::from_pixel(80, 20, image::Rgba([20, 20, 200, 255]))),
            ("4-tall.png", image::RgbaImage::from_fn(20, 60, |x, y| image::Rgba([(x * 12) as u8, (y * 4) as u8, ((x + y) * 3) as u8, if x == 0 { 0 } else { 255 }]))),
        ];
        frames.into_iter().map(|(name, frame)| { let path = root.join(name); frame.save(&path).unwrap(); path }).collect()
    }

    fn decoded_gif_frames(path: &Path) -> Vec<image::RgbaImage> {
        let decoder = image::codecs::gif::GifDecoder::new(BufReader::new(File::open(path).unwrap())).unwrap();
        decoder.into_frames().collect_frames().unwrap().into_iter().map(|frame| frame.into_buffer()).collect()
    }

    fn gif_request(paths: Vec<PathBuf>, root: &Path) -> GifCreateRequest {
        GifCreateRequest { paths, frame_delay_ms: 120, loop_forever: true, frame_delay_overrides: Vec::new(), output_location: OutputLocation::CustomFolder(root.join("outputs")), output_path: None }
    }

    #[test]
    fn gif_canvas_is_the_first_frame_and_other_frames_are_centred_or_scaled_down() {
        let root = gif_sandbox("canvas");
        let inputs = mixed_gif_frames(&root);
        let originals = inputs.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let result = gif_create(&gif_request(inputs.clone(), &root));
        assert!(result.error().is_none(), "{:?}", result.error());
        let output = result.output_paths().first().unwrap().clone();
        assert_eq!(output, root.join("outputs/1-first-animated.gif"));
        let frames = decoded_gif_frames(&output);
        assert_eq!(frames.len(), 4);
        for frame in &frames { assert_eq!(frame.dimensions(), (40, 30)); }

        // Frame 1: half-transparent pixels become transparent, opaque ones keep their colour.
        assert_eq!(frames[0].get_pixel(3, 2).0[3], 0);
        assert_eq!(frames[0].get_pixel(3, 20).0, [200, 40, 18, 255]);

        // Frame 2 (10×6) keeps its size, centred at (15, 12), padded with transparency.
        let opaque = |frame: &image::RgbaImage| {
            let mut bounds = (u32::MAX, u32::MAX, 0, 0);
            for (x, y, pixel) in frame.enumerate_pixels() {
                if pixel.0[3] != 0 { bounds = (bounds.0.min(x), bounds.1.min(y), bounds.2.max(x), bounds.3.max(y)); }
            }
            bounds
        };
        assert_eq!(opaque(&frames[1]), (15, 12, 24, 17));
        assert_eq!(frames[1].get_pixel(20, 15).0, [20, 200, 20, 255]);
        assert_eq!(frames[1].get_pixel(0, 0).0[3], 0);

        // Frame 3 (80×20) is scaled to 40×10 keeping its aspect, centred vertically.
        assert_eq!(opaque(&frames[2]), (0, 10, 39, 19));
        assert_eq!(frames[2].get_pixel(20, 15).0, [20, 20, 200, 255]);

        // Frame 4 (20×60) is scaled to 10×30 keeping its aspect, centred horizontally.
        let (left, top, right, bottom) = opaque(&frames[3]);
        assert!(left >= 15 && right <= 24 && top == 0 && bottom == 29, "{:?}", (left, top, right, bottom));
        assert_eq!(frames[3].get_pixel(0, 15).0[3], 0);
        assert_eq!(frames[3].get_pixel(39, 15).0[3], 0);

        for (path, bytes) in inputs.iter().zip(&originals) { assert_eq!(&std::fs::read(path).unwrap(), bytes); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn gif_bytes_are_identical_across_runs_and_pinned_for_every_target() {
        let root = gif_sandbox("deterministic");
        let inputs = mixed_gif_frames(&root);
        let first = gif_create(&gif_request(inputs.clone(), &root));
        let second = gif_create(&gif_request(inputs, &root));
        let (first, second) = (first.output_paths()[0].clone(), second.output_paths()[0].clone());
        assert_ne!(first, second);
        let (first_bytes, second_bytes) = (std::fs::read(&first).unwrap(), std::fs::read(&second).unwrap());
        assert_eq!(first_bytes, second_bytes);
        // The same digest on every supported target: palette, scaling and encoding use no platform maths or randomness.
        let digest = Sha256::digest(&first_bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        assert_eq!(digest, PINNED_GIF_SHA256);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Decodes a GIF with the `gif` crate: each frame's top-left RGBA pixel
    /// and delay in milliseconds, and whether it loops forever.
    fn gif_timeline(path: &Path) -> (Vec<([u8; 4], u32)>, gif::Repeat) {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(File::open(path).unwrap()).unwrap();
        let mut frames = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            let pixel = [frame.buffer[0], frame.buffer[1], frame.buffer[2], frame.buffer[3]];
            frames.push((pixel, u32::from(frame.delay) * 10));
        }
        (frames, decoder.repeat())
    }

    fn solid_gif_frames(root: &Path, colours: &[(&str, [u8; 3])]) -> Vec<PathBuf> {
        colours.iter().map(|(name, [r, g, b])| {
            let path = root.join(name);
            image::RgbaImage::from_pixel(4, 4, image::Rgba([*r, *g, *b, 255])).save(&path).unwrap();
            path
        }).collect()
    }

    fn own_delay(index: usize, delay_ms: u32) -> GifFrameDelay { GifFrameDelay { index, delay_ms } }

    #[test]
    fn gif_frames_play_in_request_order_with_their_own_delays_and_loop_flag() {
        let root = gif_sandbox("timeline");
        let [red, green, blue] = [[255, 0, 0], [0, 255, 0], [0, 0, 255]];
        let frames = solid_gif_frames(&root, &[("red.png", red), ("green.png", green), ("blue.png", blue)]);
        // A timing manifest beside the frames is never read.
        std::fs::write(root.join("red-timings.json"), br#"{"delays":[9990,9990,9990],"loop":false}"#).unwrap();
        let originals = frames.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        // Blue, red, green, red again: the repeated red frame keeps the global delay.
        let request = GifCreateRequest {
            paths: vec![frames[2].clone(), frames[0].clone(), frames[1].clone(), frames[0].clone()],
            frame_delay_ms: 70,
            frame_delay_overrides: vec![own_delay(2, 1500), own_delay(0, 40)],
            ..gif_request(Vec::new(), &root)
        };
        let result = gif_create(&request);
        let output = result.output_paths().first().unwrap().clone();
        let (timeline, repeat) = gif_timeline(&output);
        let opaque = |[r, g, b]: [u8; 3]| [r, g, b, 255];
        assert_eq!(timeline, vec![(opaque(blue), 40), (opaque(red), 70), (opaque(green), 1500), (opaque(red), 70)]);
        assert_eq!(repeat, gif::Repeat::Infinite);

        // Play once: no loop extension, so players stop on the last frame.
        let once = gif_create(&GifCreateRequest { loop_forever: false, ..request.clone() });
        let once_output = once.output_paths().first().unwrap().clone();
        assert_ne!(once_output, output, "a second run never overwrites the first GIF");
        let (timeline, repeat) = gif_timeline(&once_output);
        assert_eq!(timeline.len(), 4);
        assert_eq!(repeat, gif::Repeat::Finite(0));
        for (path, bytes) in frames.iter().zip(&originals) { assert_eq!(&std::fs::read(path).unwrap(), bytes); }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn gif_request_reads_the_ui_shape_and_defaults_to_no_overrides() {
        let request: GifCreateRequest = serde_json::from_value(serde_json::json!({
            "paths": ["/a.png", "/b.png"], "frameDelayMs": 80, "loopForever": false,
            "frameDelayOverrides": [{ "index": 1, "delayMs": 1500 }], "outputLocation": "alongsideInput", "outputPath": null,
        })).unwrap();
        assert_eq!((request.frame_delay_ms, request.loop_forever), (80, false));
        assert_eq!(request.frame_delay_overrides.iter().map(|frame| (frame.index, frame.delay_ms)).collect::<Vec<_>>(), vec![(1, 1500)]);
        let bare: GifCreateRequest = serde_json::from_value(serde_json::json!({ "paths": ["/a.png"], "outputLocation": "alongsideInput" })).unwrap();
        assert!(bare.frame_delay_overrides.is_empty());
        assert_eq!((bare.frame_delay_ms, bare.loop_forever), (100, true));
    }

    #[test]
    fn gif_delays_cover_the_whole_range_rounded_to_hundredths() {
        let request = |global, overrides| GifCreateRequest {
            paths: vec![PathBuf::from("a.png"), PathBuf::from("b.png"), PathBuf::from("c.png")],
            frame_delay_ms: global,
            frame_delay_overrides: overrides,
            loop_forever: true,
            output_location: OutputLocation::AlongsideInput,
            output_path: None,
        };
        assert_eq!(gif_frame_delays(&request(100, Vec::new())), vec![10, 10, 10]);
        assert_eq!(gif_frame_delays(&request(1, vec![own_delay(1, 60_000), own_delay(2, 15)])), vec![1, 6000, 2]);
        assert_eq!(gif_frame_delays(&request(44, vec![own_delay(0, 45)])), vec![5, 4, 4]);
    }

    #[test]
    fn gif_rejects_out_of_range_delays_before_reading_frames_and_publishes_nothing() {
        let root = gif_sandbox("delay_range");
        let frames = solid_gif_frames(&root, &[("one.png", [1, 2, 3]), ("two.png", [4, 5, 6])]);
        let cases = [
            (0, Vec::new(), &frames[0], "The delay for every frame is 0 ms. Choose 1 to 60000 ms. Nothing was saved."),
            (60_001, Vec::new(), &frames[0], "The delay for every frame is 60001 ms. Choose 1 to 60000 ms. Nothing was saved."),
            (100, vec![own_delay(1, 0)], &frames[1], "Frame 2's own delay is 0 ms. Choose 1 to 60000 ms. Nothing was saved."),
            (100, vec![own_delay(1, 60_001)], &frames[1], "Frame 2's own delay is 60001 ms. Choose 1 to 60000 ms. Nothing was saved."),
            (100, vec![own_delay(2, 500)], &frames[0], "There's no frame 3 to give its own delay. Nothing was saved."),
            (100, vec![own_delay(0, 500), own_delay(0, 600)], &frames[0], "Frame 1 has more than one own delay. Nothing was saved."),
        ];
        for (global, overrides, attributed, message) in cases {
            let result = gif_create(&GifCreateRequest { frame_delay_ms: global, frame_delay_overrides: overrides, ..gif_request(frames.clone(), &root) });
            assert!(result.output_paths().is_empty(), "{message}");
            assert_eq!(&result.input_path, attributed);
            let error = result.error().unwrap();
            assert_eq!(error.message, message);
            assert_eq!(error.kind, ToolError::invalid_input("").kind);
        }
        // The boundaries themselves are accepted.
        let result = gif_create(&GifCreateRequest { frame_delay_ms: 1, frame_delay_overrides: vec![own_delay(1, 60_000)], ..gif_request(frames.clone(), &root) });
        let (timeline, _) = gif_timeline(result.output_paths().first().unwrap());
        assert_eq!(timeline.iter().map(|(_, delay)| *delay).collect::<Vec<_>>(), vec![10, 60_000]);
        assert_eq!(std::fs::read_dir(root.join("outputs")).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    const PINNED_GIF_SHA256: &str = "2c6430b054702737cbb35f4987af9265736ea8d985277a9736e63fb2351ed3bb";

    #[test]
    fn gif_with_an_unreadable_frame_names_it_and_publishes_nothing() {
        let root = gif_sandbox("unreadable");
        let mut inputs = mixed_gif_frames(&root);
        let broken = root.join("IMG_2212.heic");
        std::fs::write(&broken, b"not an image at all").unwrap();
        inputs.insert(2, broken.clone());
        let originals = inputs.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        let result = gif_create(&gif_request(inputs.clone(), &root));
        assert!(result.output_paths().is_empty());
        assert_eq!(result.input_path, broken);
        assert_eq!(
            result.error().map(|error| error.message.as_str()),
            Some("Frame 3, IMG_2212.heic, isn't a readable HEIC image. Replace or remove frame 3, then make the GIF again. Nothing was saved.")
        );
        assert_eq!(std::fs::read_dir(root.join("outputs")).unwrap().count(), 0);
        for (path, bytes) in inputs.iter().zip(&originals) { assert_eq!(&std::fs::read(path).unwrap(), bytes); }

        // A frame that was moved away is named by its full path.
        let gone = root.join("gone.png");
        let result = gif_create(&gif_request(vec![inputs[0].clone(), gone.clone()], &root));
        assert_eq!(result.input_path, gone);
        assert_eq!(
            result.error().map(|error| error.message.clone()),
            Some(format!("Frame 2 is no longer at {}. Replace or remove frame 2, then make the GIF again. Nothing was saved.", gone.display()))
        );
        assert_eq!(std::fs::read_dir(root.join("outputs")).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(root);
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
                        loop_forever: true, frame_delay_overrides: Vec::new(),
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
            loop_forever: true, frame_delay_overrides: Vec::new(),
            output_location: OutputLocation::CustomFolder(output_directory.clone()), output_path: None,
        });

        assert!(result.error().is_some());
        assert_eq!(std::fs::read(&existing_output).unwrap(), existing_bytes);
        assert_eq!(std::fs::read_dir(&output_directory).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn icon_presets_are_named_and_reject_sizes_they_do_not_offer() {
        let platform = |preset: &str, sizes: &[u32]| IconPlatform { preset: preset.to_string(), sizes: sizes.to_vec(), containers: None };
        let macos = icon_plan(&platform("macos", &MACOS_ICON_SIZES)).unwrap();
        assert_eq!((macos.folder, macos.file_prefix, macos.label), ("macos", "macos", "macOS"));
        assert_eq!(macos.sizes, vec![16, 32, 64, 128, 256, 512, 1024]);
        assert_eq!(macos.containers, [IconContainer::Appiconset, IconContainer::Icns]);
        assert!(icon_plan(&platform("macos", &[1024, 16])).unwrap().containers.is_empty(), "a partial macOS set is PNGs only");
        let ios = icon_plan(&platform("ios", &IOS_ICON_SIZES)).unwrap();
        assert_eq!((ios.sizes.len(), ios.containers.as_slice(), ios.opaque), (13, [IconContainer::Appiconset].as_slice(), true));
        assert!(icon_plan(&platform("ios", &[20])).unwrap().containers.is_empty());
        assert_eq!(icon_plan(&platform("favicon", &[16])).unwrap().containers, [IconContainer::Ico], "the favicon.ico takes any ticked sizes");
        assert!(icon_plan(&platform("android", &ANDROID_ICON_SIZES)).unwrap().containers.is_empty());
        assert!(!icon_plan(&platform("macos", &MACOS_ICON_SIZES)).unwrap().opaque);
        assert_eq!(icon_plan(&platform("custom", &[300, 1, 16, 300, 4096])).unwrap().sizes, vec![1, 16, 300, 4096]);
        assert_eq!(icon_plan(&platform("custom", &[16, 0])).unwrap_err(), "A custom size of 0 pixels is not possible. Use sizes from 1 to 4096 pixels.");
        assert_eq!(icon_plan(&platform("custom", &[5000])).unwrap_err(), "A custom size of 5000 pixels is not possible. Use sizes from 1 to 4096 pixels.");
        assert_eq!(icon_plan(&platform("custom", &[])).unwrap_err(), "Tick at least one custom size.");
        assert_eq!(icon_plan(&platform("android", &[48, 300])).unwrap_err(), "Android icons don't come in 300 pixels.");
        assert_eq!(icon_plan(&platform("unknown", &[16])).unwrap_err(), "Unsupported icon preset: unknown.");
        assert_eq!(icon_plans(&[]).unwrap_err(), "Tick at least one platform to make icons for.");
        assert_eq!(icon_plans(&[platform("ios", &[20]), platform("ios", &[29])]).unwrap_err(), "Each platform can be ticked only once.");
        let ordered = icon_plans(&[platform("custom", &[8]), platform("favicon", &[16]), platform("macos", &[16])]).unwrap();
        assert_eq!(ordered.iter().map(|plan| plan.folder).collect::<Vec<_>>(), ["macos", "web", "custom"]);
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
        icon_request_for(paths, &[(preset, sizes)])
    }

    fn icon_request_for(paths: &[PathBuf], platforms: &[(&str, Vec<u32>)]) -> IconSetRequest {
        let platforms = platforms.iter().map(|(preset, sizes)| IconPlatform { preset: preset.to_string(), sizes: sizes.clone(), containers: None }).collect();
        IconSetRequest { paths: paths.to_vec(), platforms, square_rule: SquareRule::Pad, output_location: OutputLocation::AlongsideInput }
    }

    fn upscaled(from: u32, to: u32) -> crate::kit::contracts::WarningKind { WarningKind::Upscaled { from, to } }

    fn entry_names(folder: &Path) -> BTreeSet<String> {
        std::fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn each_preset_saves_exactly_its_sizes_and_containers_in_one_icons_folder() {
        let root = icon_root("presets");
        let input = root.join("app-icon.png");
        let original = write_master(&input);
        let cases: [(&str, Vec<u32>, &str, Vec<u32>); 5] = [
            ("macos", MACOS_ICON_SIZES.to_vec(), "macos", vec![16, 32, 64, 128, 256, 512, 1024]),
            ("favicon", WEBSITE_ICON_SIZES.to_vec(), "web", vec![16, 32, 48, 64, 128, 256]),
            ("ios", IOS_ICON_SIZES.to_vec(), "ios", vec![20, 29, 40, 58, 60, 76, 80, 87, 120, 152, 167, 180, 1024]),
            ("android", ANDROID_ICON_SIZES.to_vec(), "android", vec![48, 72, 96, 144, 192, 512]),
            ("custom", vec![300, 24], "custom", vec![24, 300]),
        ];
        for (preset, sizes, folder, expected) in cases {
            let outcome = icon_set(&icon_request(std::slice::from_ref(&input), preset, sizes), input.clone());
            assert!(outcome.error().is_none(), "{preset}: {:?}", outcome.error());
            // The 64 px master is smaller than every preset's largest size.
            let largest = *expected.last().unwrap();
            assert_eq!(outcome.warnings.iter().map(|warning| warning.kind.clone()).collect::<Vec<_>>(), [upscaled(64, largest)], "{preset}");
            assert_eq!(outcome.warnings[0].message, format!("Upscaled to {largest} from 64 pixels."));
            let set = root.join("app-icon-icons");
            assert_eq!(outcome.output_paths(), [set.clone()]);
            assert_eq!(entry_names(&set), BTreeSet::from([folder.to_string()]));
            let prefix = if preset == "favicon" { "favicon" } else { folder };
            let mut names: BTreeSet<String> = expected.iter().map(|size| format!("{prefix}-{size}.png")).collect();
            let containers: &[&str] = match preset { "macos" => &["AppIcon.appiconset", "app-icon.icns"], "ios" => &["AppIcon.appiconset"], "favicon" => &["favicon.ico"], _ => &[] };
            names.extend(containers.iter().map(|name| name.to_string()));
            assert_eq!(entry_names(&set.join(folder)), names, "{preset}");
            for size in &expected {
                let icon = image::open(set.join(folder).join(format!("{prefix}-{size}.png"))).unwrap();
                if preset == "ios" {
                    // iPhone and iPad icons can't be transparent, so the transparent side is white.
                    assert_eq!(icon.color(), image::ColorType::Rgb8, "{preset} {size}");
                    let icon = icon.to_rgb8();
                    assert_eq!(icon.dimensions(), (*size, *size), "{preset} {size}");
                    assert_eq!(icon.get_pixel(0, size / 2).0, [255, 255, 255], "{preset} {size} fills the transparent side with white");
                    assert_eq!(icon.get_pixel(size - 1, size / 2).0, [200, 40, 90], "{preset} {size} keeps the opaque side");
                    continue;
                }
                assert_eq!(icon.color(), image::ColorType::Rgba8, "{preset} {size}");
                let icon = icon.to_rgba8();
                assert_eq!(icon.dimensions(), (*size, *size), "{preset} {size}");
                assert_eq!(icon.get_pixel(0, size / 2)[3], 0, "{preset} {size} keeps the transparent side");
                assert_eq!(icon.get_pixel(size - 1, size / 2)[3], 255, "{preset} {size} keeps the opaque side");
            }
            let catalog = match preset { "macos" => MACOS_APPICONSET.len() + 1, "ios" => IOS_APPICONSET.len() + 1, _ => 0 };
            let files = expected.len() + catalog + containers.iter().filter(|name| !name.ends_with("appiconset")).count();
            assert_eq!(outcome.detail, format!("{files} files"));
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
        let outcome = icon_set(&icon_request(std::slice::from_ref(&input), "macos", MACOS_ICON_SIZES.to_vec()), input.clone());
        assert!(outcome.error().is_none());
        let bytes = std::fs::read(root.join("logo-icons/macos/logo.icns")).unwrap();
        assert_eq!(&bytes[..4], b"icns");
        assert_eq!(u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize, bytes.len());
        // Walk every entry: each is a type, its length and a PNG of that type's size.
        let mut entries = Vec::new();
        let mut at = 8;
        while at < bytes.len() {
            let code = String::from_utf8(bytes[at..at + 4].to_vec()).unwrap();
            let length = u32::from_be_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            let icon = image::load_from_memory_with_format(&bytes[at + 8..at + length], ImageFormat::Png).unwrap();
            assert_eq!(icon.width(), icon.height(), "{code}");
            entries.push((code, icon.width()));
            at += length;
        }
        assert_eq!(at, bytes.len());
        let expected = [("icp4", 16), ("ic11", 32), ("icp5", 32), ("ic12", 64), ("ic07", 128), ("ic13", 256), ("ic08", 256), ("ic14", 512), ("ic09", 512), ("ic10", 1024)];
        assert_eq!(entries, expected.map(|(code, size)| (code.to_string(), size)));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Reads `Contents.json` and checks that every slot names a file in the folder with the
    /// slot's pixel size, and that the folder holds nothing else. Returns each slot's
    /// idiom, size and scale.
    fn read_appiconset(catalog: &Path) -> Vec<(String, String, String)> {
        let contents: serde_json::Value = serde_json::from_slice(&std::fs::read(catalog.join("Contents.json")).unwrap()).unwrap();
        assert_eq!(contents["info"], serde_json::json!({ "author": "xcode", "version": 1 }));
        let mut files = BTreeSet::from(["Contents.json".to_string()]);
        let slots = contents["images"].as_array().unwrap().iter().map(|slot| {
            let text = |key: &str| slot[key].as_str().unwrap().to_string();
            let (size, scale) = (text("size"), text("scale"));
            let points: f64 = size.split_once('x').unwrap().0.parse().unwrap();
            let pixels = (points * scale.trim_end_matches('x').parse::<f64>().unwrap()) as u32;
            let icon = image::open(catalog.join(text("filename"))).unwrap();
            assert_eq!((icon.width(), icon.height()), (pixels, pixels), "{}", text("filename"));
            assert!(files.insert(text("filename")), "each slot has its own file");
            (text("idiom"), size, scale)
        }).collect();
        assert_eq!(entry_names(catalog), files, "the folder holds exactly the files Contents.json lists");
        slots
    }

    fn slots(list: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
        list.iter().map(|(idiom, size, scale)| (idiom.to_string(), size.to_string(), scale.to_string())).collect()
    }

    #[test]
    fn macos_and_ios_sets_include_a_complete_asset_catalog() {
        let root = icon_root("appiconset");
        let input = root.join("app-icon.png");
        write_master(&input);
        let request = icon_request_for(std::slice::from_ref(&input), &[("macos", MACOS_ICON_SIZES.to_vec()), ("ios", IOS_ICON_SIZES.to_vec())]);
        let outcome = icon_set(&request, input.clone());
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let set = root.join("app-icon-icons");
        assert_eq!(read_appiconset(&set.join("macos/AppIcon.appiconset")), slots(&[
            ("mac", "16x16", "1x"), ("mac", "16x16", "2x"), ("mac", "32x32", "1x"), ("mac", "32x32", "2x"),
            ("mac", "128x128", "1x"), ("mac", "128x128", "2x"), ("mac", "256x256", "1x"), ("mac", "256x256", "2x"),
            ("mac", "512x512", "1x"), ("mac", "512x512", "2x"),
        ]));
        assert_eq!(read_appiconset(&set.join("ios/AppIcon.appiconset")), slots(&[
            ("iphone", "20x20", "2x"), ("iphone", "20x20", "3x"), ("iphone", "29x29", "2x"), ("iphone", "29x29", "3x"),
            ("iphone", "40x40", "2x"), ("iphone", "40x40", "3x"), ("iphone", "60x60", "2x"), ("iphone", "60x60", "3x"),
            ("ipad", "20x20", "1x"), ("ipad", "20x20", "2x"), ("ipad", "29x29", "1x"), ("ipad", "29x29", "2x"),
            ("ipad", "40x40", "1x"), ("ipad", "40x40", "2x"), ("ipad", "76x76", "1x"), ("ipad", "76x76", "2x"),
            ("ipad", "83.5x83.5", "2x"), ("ios-marketing", "1024x1024", "1x"),
        ]));
        // The catalog images are the platform's own PNGs, so iOS ones have no alpha either.
        let marketing = image::open(set.join("ios/AppIcon.appiconset/ios-marketing-1024@1x.png")).unwrap();
        assert_eq!(marketing.color(), image::ColorType::Rgb8);
        assert_eq!(std::fs::read(set.join("macos/AppIcon.appiconset/icon_512x512@2x.png")).unwrap(), std::fs::read(set.join("macos/macos-1024.png")).unwrap());
        assert_eq!(outcome.detail, format!("{} files", 7 + 11 + 1 + 13 + 19));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_website_set_includes_a_favicon_ico_with_the_ticked_sizes() {
        let root = icon_root("ico");
        let input = root.join("site.png");
        write_master(&input);
        for sizes in [WEBSITE_ICON_SIZES.to_vec(), vec![16, 256], vec![48]] {
            let outcome = icon_set(&icon_request(std::slice::from_ref(&input), "favicon", sizes.clone()), input.clone());
            assert!(outcome.error().is_none(), "{sizes:?}");
            let set = outcome.output_paths()[0].clone();
            let bytes = std::fs::read(set.join("web/favicon.ico")).unwrap();
            assert_eq!(&bytes[..4], &[0, 0, 1, 0], "an ICO header");
            let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
            assert_eq!(count, sizes.len());
            let mut found = Vec::new();
            for index in 0..count {
                let entry = &bytes[6 + 16 * index..6 + 16 * (index + 1)];
                let side = if entry[0] == 0 { 256 } else { u32::from(entry[0]) };
                assert_eq!(entry[0], entry[1]);
                assert_eq!(u16::from_le_bytes([entry[6], entry[7]]), 32, "32 bits per pixel");
                let length = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
                let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
                let icon = image::load_from_memory_with_format(&bytes[offset..offset + length], ImageFormat::Png).unwrap();
                assert_eq!((icon.width(), icon.height()), (side, side));
                assert_eq!(&bytes[offset..offset + length], std::fs::read(set.join(format!("web/favicon-{side}.png"))).unwrap());
                found.push(side);
            }
            assert_eq!(found, sizes);
            std::fs::remove_dir_all(set).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_same_master_gives_byte_identical_sets_on_every_run() {
        let root = icon_root("deterministic");
        let input = root.join("brand.png");
        write_master(&input);
        let request = icon_request_for(std::slice::from_ref(&input), &[
            ("macos", MACOS_ICON_SIZES.to_vec()), ("ios", IOS_ICON_SIZES.to_vec()), ("favicon", WEBSITE_ICON_SIZES.to_vec()), ("android", ANDROID_ICON_SIZES.to_vec()),
        ]);
        fn files(folder: &Path, into: &mut BTreeMap<PathBuf, Vec<u8>>, base: &Path) {
            for entry in std::fs::read_dir(folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() { files(&path, into, base) } else { into.insert(path.strip_prefix(base).unwrap().to_path_buf(), std::fs::read(&path).unwrap()); }
            }
        }
        let mut runs = Vec::new();
        for _ in 0..2 {
            let outcome = icon_set(&request, input.clone());
            let set = outcome.output_paths()[0].clone();
            let mut found = BTreeMap::new();
            files(&set, &mut found, &set);
            std::fs::remove_dir_all(set).unwrap();
            runs.push(found);
        }
        assert_eq!(runs[0].len(), 7 + 11 + 1 + 13 + 19 + 6 + 1 + 6);
        assert!(runs[0] == runs[1], "two runs give the same files and bytes");
        let contents = String::from_utf8(runs[0][Path::new("macos/AppIcon.appiconset/Contents.json")].clone()).unwrap();
        assert!(contents.starts_with("{\n  \"images\" : [\n    {\n      \"filename\" : \"icon_16x16.png\",\n      \"idiom\" : \"mac\",\n      \"scale\" : \"1x\",\n      \"size\" : \"16x16\"\n    },"), "{contents}");
        assert!(contents.ends_with("  \"info\" : {\n    \"author\" : \"xcode\",\n    \"version\" : 1\n  }\n}\n"), "{contents}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn containers_follow_the_ticks_and_need_every_size_where_the_design_says() {
        let root = icon_root("container_choice");
        let input = root.join("logo.png");
        write_master(&input);
        let request = |preset: &str, sizes: &[u32], containers: &[IconContainer]| IconSetRequest {
            platforms: vec![IconPlatform { preset: preset.into(), sizes: sizes.to_vec(), containers: Some(containers.to_vec()) }],
            ..icon_request(std::slice::from_ref(&input), preset, vec![])
        };
        // Unticking a container leaves just the PNGs, or the other container.
        for (preset, sizes, containers, expected) in [
            ("macos", MACOS_ICON_SIZES.as_slice(), vec![IconContainer::Icns], vec!["logo.icns"]),
            ("macos", MACOS_ICON_SIZES.as_slice(), vec![IconContainer::Appiconset], vec!["AppIcon.appiconset"]),
            ("ios", IOS_ICON_SIZES.as_slice(), vec![], vec![]),
            ("favicon", [16, 32].as_slice(), vec![], vec![]),
            ("favicon", [32].as_slice(), vec![IconContainer::Ico], vec!["favicon.ico"]),
        ] {
            let outcome = icon_set(&request(preset, sizes, &containers), input.clone());
            assert!(outcome.error().is_none(), "{preset} {containers:?}: {:?}", outcome.error());
            let set = outcome.output_paths()[0].clone();
            let platform = std::fs::read_dir(&set).unwrap().next().unwrap().unwrap().path();
            let extra = entry_names(&platform).into_iter().filter(|name| !name.ends_with(".png")).collect::<Vec<_>>();
            assert_eq!(extra, expected, "{preset} {containers:?}");
            std::fs::remove_dir_all(set).unwrap();
        }
        for (preset, sizes, containers, message) in [
            ("macos", [16, 32, 128, 256, 512, 1024].as_slice(), vec![IconContainer::Icns], "The ICNS needs every macOS size. Tick 64 too, or untick it."),
            ("ios", [20, 1024].as_slice(), vec![IconContainer::Appiconset], "The AppIcon.appiconset needs every iOS size. Tick 29, 40, 58, 60, 76, 80, 87, 120, 152, 167, 180 too, or untick it."),
            ("android", ANDROID_ICON_SIZES.as_slice(), vec![IconContainer::Ico], "Android icons don't come as favicon.ico."),
            ("favicon", WEBSITE_ICON_SIZES.as_slice(), vec![IconContainer::Icns], "Website icons don't come as ICNS."),
        ] {
            let outcome = icon_set(&request(preset, sizes, &containers), input.clone());
            assert_eq!(outcome.error().map(|error| error.message.as_str()), Some(message));
        }
        assert_eq!(entry_names(&root), BTreeSet::from(["logo.png".to_string()]));
        let parsed: IconPlatform = serde_json::from_value(serde_json::json!({ "preset": "macos", "sizes": [16], "containers": ["appiconset", "icns"] })).unwrap();
        assert_eq!(parsed.containers, Some(vec![IconContainer::Appiconset, IconContainer::Icns]));
        let parsed: IconPlatform = serde_json::from_value(serde_json::json!({ "preset": "favicon", "sizes": [16], "containers": ["ico"] })).unwrap();
        assert_eq!(parsed.containers, Some(vec![IconContainer::Ico]));
        assert!(serde_json::from_value::<IconPlatform>(serde_json::json!({ "preset": "macos", "sizes": [16], "containers": ["zip"] })).is_err());
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
        let outcome = icon_set(&icon_request(std::slice::from_ref(&input), "android", ANDROID_ICON_SIZES.to_vec()), input.clone());
        assert_eq!(outcome.output_paths(), [root.join("brand-mark-icons-1")]);
        assert_eq!(outcome.warnings.iter().map(|warning| warning.kind.clone()).collect::<Vec<_>>(), [upscaled(64, 512), WarningKind::RenamedToAvoidReplacing { chosen_name: "brand-mark-icons".into(), final_name: "brand-mark-icons-1".into() }]);
        assert_eq!(entry_names(&existing), BTreeSet::from(["keep.txt".to_string()]));
        assert_eq!(std::fs::read(existing.join("keep.txt")).unwrap(), b"mine");
        assert_eq!(entry_names(&root.join("brand-mark-icons-1/android")).len(), 6);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_the_ticked_sizes_of_each_ticked_platform_are_made_in_one_set() {
        let root = icon_root("selection");
        let input = root.join("brand.png");
        let original = write_master(&input);
        let all = icon_request_for(std::slice::from_ref(&input), &[("ios", vec![20, 40, 60]), ("macos", vec![16, 32, 64]), ("custom", vec![8])]);
        let outcome = icon_set(&all, input.clone());
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        assert!(outcome.warnings.is_empty(), "no size is above the 64 px master");
        let set = root.join("brand-icons");
        assert_eq!(entry_names(&set), BTreeSet::from(["custom", "ios", "macos"].map(String::from)));
        assert_eq!(entry_names(&set.join("ios")), BTreeSet::from(["ios-20.png", "ios-40.png", "ios-60.png"].map(String::from)));
        assert_eq!(entry_names(&set.join("macos")), BTreeSet::from(["macos-16.png", "macos-32.png", "macos-64.png"].map(String::from)), "a partial macOS set is PNGs only");
        assert_eq!(entry_names(&set.join("custom")), BTreeSet::from(["custom-8.png".to_string()]));
        assert_eq!(outcome.detail, "7 files");
        std::fs::remove_dir_all(&set).unwrap();

        // Unticking one size removes exactly that output.
        let fewer = icon_request_for(std::slice::from_ref(&input), &[("ios", vec![20, 60]), ("macos", vec![16, 32, 64]), ("custom", vec![8])]);
        assert!(icon_set(&fewer, input.clone()).error().is_none());
        assert_eq!(entry_names(&set.join("ios")), BTreeSet::from(["ios-20.png", "ios-60.png"].map(String::from)));
        assert_eq!(entry_names(&set.join("macos")).len(), 3);
        assert_eq!(entry_names(&set.join("custom")).len(), 1);
        assert_eq!(std::fs::read(&input).unwrap(), original);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_invalid_selection_fails_the_master_before_anything_is_saved() {
        let root = icon_root("invalid_selection");
        let input = root.join("logo.png");
        write_master(&input);
        for (platforms, message) in [
            (vec![("custom", vec![16, 5000])], "A custom size of 5000 pixels is not possible. Use sizes from 1 to 4096 pixels."),
            (vec![("custom", vec![0])], "A custom size of 0 pixels is not possible. Use sizes from 1 to 4096 pixels."),
            (vec![("ios", vec![83])], "iOS icons don't come in 83 pixels."),
            (vec![("macos", vec![])], "Tick at least one macOS size."),
            (vec![], "Tick at least one platform to make icons for."),
        ] {
            let outcome = icon_set(&icon_request_for(std::slice::from_ref(&input), &platforms), input.clone());
            assert_eq!(outcome.error().map(|error| error.message.as_str()), Some(message));
        }
        assert_eq!(entry_names(&root), BTreeSet::from(["logo.png".to_string()]));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_master_smaller_than_the_largest_size_succeeds_with_an_upscale_warning_for_pad_and_crop() {
        let root = icon_root("upscale");
        // The 128 by 64 master squares to 128 when padded and to 64 when cropped.
        let input = root.join("wide.png");
        write_non_square_master(&input, Orientation::Wide);
        for (rule, sizes, warning) in [
            (SquareRule::Pad, vec![32, 256], Some(upscaled(128, 256))),
            (SquareRule::Pad, vec![32, 128], None),
            (SquareRule::Crop, vec![32, 128], Some(upscaled(64, 128))),
            (SquareRule::Crop, vec![32, 64], None),
        ] {
            let request = IconSetRequest { square_rule: rule, ..icon_request(std::slice::from_ref(&input), "custom", sizes.clone()) };
            let outcome = icon_set(&request, input.clone());
            assert!(outcome.error().is_none(), "{rule:?} {sizes:?}");
            assert_eq!(outcome.warnings.iter().map(|warning| warning.kind.clone()).collect::<Vec<_>>(), warning.into_iter().collect::<Vec<_>>(), "{rule:?} {sizes:?}");
            let set = outcome.output_paths()[0].clone();
            let largest = *sizes.last().unwrap();
            assert_eq!(image::open(set.join(format!("custom/custom-{largest}.png"))).unwrap().width(), largest);
            std::fs::remove_dir_all(set).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_size_saves_nothing_for_that_master_and_names_the_size_while_others_succeed() {
        let root = icon_root("forced_failure");
        let inputs = ["first", "broken", "last"].map(|name| root.join(format!("{name}.png")));
        let originals = inputs.iter().map(|input| write_master(input)).collect::<Vec<_>>();
        let request = icon_request(&inputs, "macos", MACOS_ICON_SIZES.to_vec());
        let outcomes = inputs.iter().map(|input| {
            let broken = input.ends_with("broken.png");
            icon_set_with(&request, input.clone(), |icon, output| {
                if broken && icon.width() == 1024 { return Err("the image data is damaged".into()); }
                icon.save_with_format(output, ImageFormat::Png).map_err(|error| error.to_string())
            }, |output, bytes| std::fs::write(output, bytes).map_err(|error| error.to_string()))
        }).collect::<Vec<_>>();
        assert!(outcomes[0].error().is_none() && outcomes[2].error().is_none());
        assert_eq!(outcomes[1].error().unwrap().message, "The macOS 1024 size failed: the image data is damaged. Nothing was saved for this image.");
        assert!(outcomes[1].output_paths().is_empty());
        assert_eq!(entry_names(&root), BTreeSet::from(["first.png", "broken.png", "last.png", "first-icons", "last-icons"].map(String::from)));
        assert_eq!(entry_names(&root.join("last-icons/macos")).len(), 9, "seven macOS sizes, the asset catalog and the ICNS");
        for (input, original) in inputs.iter().zip(originals) { assert_eq!(std::fs::read(input).unwrap(), original); }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_container_saves_nothing_for_that_master_and_names_the_container() {
        let root = icon_root("container_failure");
        let inputs = ["first", "broken", "last"].map(|name| root.join(format!("{name}.png")));
        let originals = inputs.iter().map(|input| write_master(input)).collect::<Vec<_>>();
        let save = |icon: &DynamicImage, output: &Path| icon.save_with_format(output, ImageFormat::Png).map_err(|error| error.to_string());
        for (platforms, failing, message) in [
            (vec![("macos", MACOS_ICON_SIZES.to_vec())], "Contents.json", "The macOS AppIcon.appiconset failed: the disk is full. Nothing was saved for this image."),
            (vec![("macos", MACOS_ICON_SIZES.to_vec())], "broken.icns", "The macOS ICNS failed: the disk is full. Nothing was saved for this image."),
            (vec![("android", ANDROID_ICON_SIZES.to_vec()), ("ios", IOS_ICON_SIZES.to_vec())], "ipad-83.5@2x.png", "The iOS AppIcon.appiconset failed: the disk is full. Nothing was saved for this image."),
            (vec![("favicon", WEBSITE_ICON_SIZES.to_vec())], "favicon.ico", "The Website favicon.ico failed: the disk is full. Nothing was saved for this image."),
        ] {
            let request = icon_request_for(&inputs, &platforms);
            let outcomes = inputs.iter().map(|input| {
                let broken = input.ends_with("broken.png");
                icon_set_with(&request, input.clone(), save, |output, bytes| {
                    if broken && output.ends_with(failing) { return Err("the disk is full".into()); }
                    std::fs::write(output, bytes).map_err(|error| error.to_string())
                })
            }).collect::<Vec<_>>();
            assert!(outcomes[0].error().is_none() && outcomes[2].error().is_none(), "{failing}");
            assert_eq!(outcomes[1].error().unwrap().message, message);
            assert!(outcomes[1].output_paths().is_empty());
            assert_eq!(entry_names(&root), BTreeSet::from(["first.png", "broken.png", "last.png", "first-icons", "last-icons"].map(String::from)), "{failing}");
            for name in ["first-icons", "last-icons"] { std::fs::remove_dir_all(root.join(name)).unwrap(); }
        }
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
        let request = icon_request(&[good.clone(), corrupt.clone()], "favicon", WEBSITE_ICON_SIZES.to_vec());
        let failed = icon_set(&request, corrupt.clone());
        let saved = icon_set(&request, good.clone());
        assert_eq!(failed.error().unwrap().message, "Not a readable WebP image. The file may be damaged. Nothing was saved.");
        assert!(saved.error().is_none());
        assert_eq!(entry_names(&root), BTreeSet::from(["good.png", "damaged.webp", "good-icons"].map(String::from)));
        assert_eq!(std::fs::read(&corrupt).unwrap(), b"RIFF\x10\0\0\0WEBPnot really an image");
        assert_eq!(std::fs::read(&good).unwrap(), original);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[derive(Clone, Copy, Debug)]
    enum Orientation { Wide, Tall }

    const STRIP_RED: [u8; 3] = [220, 40, 40];
    const STRIP_BLUE: [u8; 3] = [40, 60, 220];
    const STRIP_GREEN: [u8; 3] = [40, 200, 80];

    /// A 2:1 master, 128 by 64 (wide) or 64 by 128 (tall). Along its long side it has a
    /// transparent strip, then red, blue and green strips, 32 pixels each.
    fn write_non_square_master(path: &Path, orientation: Orientation) -> Vec<u8> {
        let strip = |along: u32| match along / 32 {
            0 => image::Rgba([0, 0, 0, 0]),
            1 => image::Rgba([STRIP_RED[0], STRIP_RED[1], STRIP_RED[2], 255]),
            2 => image::Rgba([STRIP_BLUE[0], STRIP_BLUE[1], STRIP_BLUE[2], 255]),
            _ => image::Rgba([STRIP_GREEN[0], STRIP_GREEN[1], STRIP_GREEN[2], 255]),
        };
        let master = match orientation {
            Orientation::Wide => image::RgbaImage::from_fn(128, 64, |x, _| strip(x)),
            Orientation::Tall => image::RgbaImage::from_fn(64, 128, |_, y| strip(y)),
        };
        master.save(path).unwrap();
        std::fs::read(path).unwrap()
    }

    /// The pixel `along` the master's long axis and `across` it, so one check fits both orientations.
    fn pixel_at(icon: &image::RgbaImage, orientation: Orientation, along: u32, across: u32) -> image::Rgba<u8> {
        match orientation { Orientation::Wide => *icon.get_pixel(along, across), Orientation::Tall => *icon.get_pixel(across, along) }
    }

    fn is_colour(pixel: image::Rgba<u8>, colour: [u8; 3]) -> bool {
        pixel[3] == 255 && pixel.0[..3].iter().zip(colour).all(|(channel, expected)| channel.abs_diff(expected) <= 12)
    }

    fn square_icon(root: &Path, orientation: Orientation, rule: SquareRule, size: u32) -> (image::RgbaImage, Vec<u8>, PathBuf) {
        let name = format!("{orientation:?}-{rule:?}").to_lowercase();
        let input = root.join(format!("{name}.png"));
        let original = write_non_square_master(&input, orientation);
        let request = IconSetRequest { square_rule: rule, ..icon_request(std::slice::from_ref(&input), "custom", vec![size]) };
        let outcome = icon_set(&request, input.clone());
        assert!(outcome.error().is_none(), "{name}: {:?}", outcome.error());
        let icon = image::open(root.join(format!("{name}-icons/custom/custom-{size}.png"))).unwrap();
        assert_eq!(icon.color(), image::ColorType::Rgba8, "{name}");
        (icon.to_rgba8(), original, input)
    }

    #[test]
    fn the_square_rule_defaults_to_pad_when_the_request_leaves_it_out() {
        let request: IconSetRequest = serde_json::from_value(serde_json::json!({ "paths": ["/m.png"], "platforms": [{ "preset": "macos", "sizes": [16] }], "outputLocation": "alongsideInput" })).unwrap();
        assert_eq!(request.square_rule, SquareRule::Pad);
        let request: IconSetRequest = serde_json::from_value(serde_json::json!({ "paths": ["/m.png"], "platforms": [], "squareRule": "crop", "outputLocation": "alongsideInput" })).unwrap();
        assert_eq!(request.square_rule, SquareRule::Crop);
        assert!(serde_json::from_value::<IconSetRequest>(serde_json::json!({ "paths": ["/m.png"], "platforms": [], "squareRule": "stretch", "outputLocation": "alongsideInput" })).is_err());
    }

    #[test]
    fn pad_centres_the_whole_non_square_master_on_a_transparent_square_without_stretching() {
        let root = icon_root("square_pad");
        for orientation in [Orientation::Wide, Orientation::Tall] {
            // The 128 px square is halved to 64, so the 64 px tall band becomes rows 16 to 47.
            let (icon, original, input) = square_icon(&root, orientation, SquareRule::Pad, 64);
            assert_eq!(icon.dimensions(), (64, 64), "{orientation:?}");
            for along in [4, 24, 40, 60] {
                assert_eq!(pixel_at(&icon, orientation, along, 2)[3], 0, "{orientation:?}: padding before the image at {along}");
                assert_eq!(pixel_at(&icon, orientation, along, 61)[3], 0, "{orientation:?}: padding after the image at {along}");
            }
            assert_eq!(pixel_at(&icon, orientation, 4, 32)[3], 0, "{orientation:?}: the master's own transparent strip is kept");
            assert!(is_colour(pixel_at(&icon, orientation, 24, 32), STRIP_RED), "{orientation:?}: red strip");
            assert!(is_colour(pixel_at(&icon, orientation, 40, 32), STRIP_BLUE), "{orientation:?}: blue strip");
            assert!(is_colour(pixel_at(&icon, orientation, 60, 32), STRIP_GREEN), "{orientation:?}: the far edge is kept");
            // Aspect: the image is twice as long as it is deep. A stretched master would fill all 64 pixels.
            let depth = (0..64).filter(|across| pixel_at(&icon, orientation, 40, *across)[3] > 127).count();
            assert!((31..=33).contains(&depth), "{orientation:?}: the image is {depth} pixels deep, not 32");
            let length = (0..64).filter(|along| pixel_at(&icon, orientation, *along, 32)[3] > 127).count();
            assert!((47..=49).contains(&length), "{orientation:?}: the opaque strips are {length} pixels long, not 48");
            assert_eq!(std::fs::read(&input).unwrap(), original, "{orientation:?}: the original is unchanged");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn centre_crop_keeps_the_largest_centred_square_without_stretching() {
        let root = icon_root("square_crop");
        for orientation in [Orientation::Wide, Orientation::Tall] {
            // The centred 64 px square holds the red and blue strips only.
            let (icon, original, input) = square_icon(&root, orientation, SquareRule::Crop, 64);
            assert_eq!(icon.dimensions(), (64, 64), "{orientation:?}");
            for across in [0, 32, 63] {
                assert!(is_colour(pixel_at(&icon, orientation, 1, across), STRIP_RED), "{orientation:?}: red reaches the near edge at {across}");
                assert!(is_colour(pixel_at(&icon, orientation, 62, across), STRIP_BLUE), "{orientation:?}: blue reaches the far edge at {across}");
            }
            // Aspect: each strip keeps its 32 px length. A stretched master would squeeze them to 16 and show green.
            let red = (0..64).filter(|along| is_colour(pixel_at(&icon, orientation, *along, 32), STRIP_RED)).count();
            assert!((31..=33).contains(&red), "{orientation:?}: the red strip is {red} pixels long, not 32");
            assert!(icon.pixels().all(|pixel| pixel[3] == 255), "{orientation:?}: the transparent strip is cut off");
            assert!(!icon.pixels().any(|pixel| is_colour(*pixel, STRIP_GREEN)), "{orientation:?}: the green strip is cut off");
            assert_eq!(std::fs::read(&input).unwrap(), original, "{orientation:?}: the original is unchanged");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_square_master_is_unaffected_by_the_square_rule() {
        let root = icon_root("square_master");
        let input = root.join("square.png");
        let original = write_master(&input);
        let mut saved = Vec::new();
        for rule in [SquareRule::Pad, SquareRule::Crop] {
            let request = IconSetRequest { square_rule: rule, ..icon_request(std::slice::from_ref(&input), "custom", vec![32, 64]) };
            let outcome = icon_set(&request, input.clone());
            assert!(outcome.error().is_none(), "{rule:?}");
            let set = outcome.output_paths()[0].clone();
            saved.push([32, 64].map(|size| std::fs::read(set.join(format!("custom/custom-{size}.png"))).unwrap()));
            std::fs::remove_dir_all(set).unwrap();
        }
        assert_eq!(saved[0], saved[1], "pad and centre-crop make the same icons from a square master");
        let full = image::load_from_memory(&saved[0][1]).unwrap().to_rgba8();
        assert_eq!(full, image::open(&input).unwrap().to_rgba8(), "the 64 px icon of a 64 px square master is the master itself");
        assert_eq!(std::fs::read(&input).unwrap(), original);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn write_fixture(name: &str, bytes: &[u8]) -> PathBuf {
        let input = path(name);
        std::fs::write(&input, bytes).unwrap();
        input
    }

    fn master_request(input: &Path, preset: &str) -> IconSetRequest {
        icon_request(&[input.to_path_buf()], preset, vec![32, 64])
    }

    /// The PNG icons a set saved in its platform folder, in size order.
    fn set_pngs(set: &Path, platform: &str) -> Vec<PathBuf> {
        let mut pngs = std::fs::read_dir(set.join(platform)).unwrap().map(|entry| entry.unwrap().path()).filter(|path| path.extension().is_some_and(|extension| extension == "png")).collect::<Vec<_>>();
        pngs.sort_by_key(|path| image::image_dimensions(path).unwrap().0);
        pngs
    }

    fn assert_8bit_png(output: &Path) {
        let color = image::open(output).unwrap().color();
        assert!(matches!(color, image::ColorType::Rgb8 | image::ColorType::Rgba8), "{} is {color:?}", output.display());
    }

    #[test]
    fn heic_masters_make_icon_sets_and_high_bit_depth_is_reduced_with_a_warning() {
        use crate::kit::images::fixtures;
        let source = fixtures::red_left_blue_right(64, 32);
        for (name, bytes, warning) in [
            ("icon-master-8bit.heic", fixtures::heic(&source), None),
            ("icon-master-10bit.heif", fixtures::heic_10bit(&source), Some("10-bit HEIC saved as 8-bit sRGB.")),
        ] {
            let input = write_fixture(name, &bytes);
            for preset in ["custom", "macos"] {
                let result = icon_set(&master_request(&input, preset), input.clone());
                assert!(result.error().is_none(), "{name}: {:?}", result.error());
                let pngs = set_pngs(&result.output_paths()[0], preset);
                assert!(!pngs.is_empty());
                for png in &pngs { assert_8bit_png(png); }
                assert_eq!(fixtures::red_edge(&image::open(&pngs[pngs.len() - 1]).unwrap()), "left");
                match warning {
                    Some(message) => assert_eq!(result.warnings, vec![crate::kit::contracts::OutcomeWarning { kind: crate::kit::contracts::WarningKind::PrecisionReduced, message: message.to_string() }]),
                    None => assert!(result.warnings.is_empty(), "{name}: {:?}", result.warnings),
                }
                for output in result.output_paths() { let _ = std::fs::remove_dir_all(output); }
            }
            assert_eq!(std::fs::read(&input).unwrap(), bytes, "{name} must stay byte-identical");
            let _ = std::fs::remove_file(input);
        }
    }

    #[test]
    fn oriented_icon_masters_are_used_upright() {
        use crate::kit::images::fixtures;
        let source = fixtures::red_left_blue_right(64, 32);
        for (name, bytes) in [("icon-exif-6.jpg", fixtures::jpeg_with_orientation(&source, 6)), ("icon-irot-3.heic", fixtures::heic_rotated(&source, 3))] {
            let input = write_fixture(name, &bytes);
            let result = icon_set(&master_request(&input, "custom"), input.clone());
            assert!(result.error().is_none(), "{name}: {:?}", result.error());
            // Both sources display with the red half on top; a second rotation would move it.
            let set = result.output_paths()[0].clone();
            for output in set_pngs(&set, "custom") {
                assert_eq!(fixtures::red_edge(&image::open(&output).unwrap()), "top", "{name} -> {}", output.display());
            }
            let _ = std::fs::remove_dir_all(set);
            assert_eq!(std::fs::read(&input).unwrap(), bytes, "{name} must stay byte-identical");
            let _ = std::fs::remove_file(input);
        }
    }

    #[test]
    fn gif_accepts_heic_and_oriented_frames_upright_and_names_the_reduced_frame() {
        use crate::kit::images::fixtures;
        let source = fixtures::red_left_blue_right(64, 32);
        let frames = [
            write_fixture("gif-frame-exif-6.jpg", &fixtures::jpeg_with_orientation(&source, 6)),
            write_fixture("gif-frame-10bit.heic", &fixtures::heic_10bit(&fixtures::red_left_blue_right(32, 64))),
            write_fixture("gif-frame-irot-3.heif", &fixtures::heic_rotated(&source, 3)),
        ];
        let originals = frames.iter().map(|frame| std::fs::read(frame).unwrap()).collect::<Vec<_>>();
        let result = gif_create(&GifCreateRequest { paths: frames.to_vec(), frame_delay_ms: 100, loop_forever: true, frame_delay_overrides: Vec::new(), output_location: OutputLocation::AlongsideInput, output_path: None });
        assert!(result.error().is_none(), "{:?}", result.error());
        assert_eq!(result.warnings, vec![crate::kit::contracts::OutcomeWarning {
            kind: crate::kit::contracts::WarningKind::PrecisionReduced,
            message: format!("Frame 2 ({}) is 10-bit HEIC, saved as 8-bit sRGB.", frames[1].file_name().unwrap().to_string_lossy()),
        }]);
        let output = result.output_paths()[0].clone();
        let decoded = image::codecs::gif::GifDecoder::new(BufReader::new(File::open(&output).unwrap())).unwrap().into_frames().collect_frames().unwrap();
        assert_eq!(decoded.len(), 3);
        let edges = decoded.into_iter().map(|frame| {
            assert_eq!(frame.buffer().dimensions(), (32, 64), "every frame is upright portrait, so the canvas is too");
            fixtures::red_edge(&image::DynamicImage::ImageRgba8(frame.into_buffer()))
        }).collect::<Vec<_>>();
        assert_eq!(edges, ["top", "left", "top"]);
        for (frame, original) in frames.iter().zip(&originals) { assert_eq!(&std::fs::read(frame).unwrap(), original, "{} must stay byte-identical", frame.display()); }
        let several = gif_create(&GifCreateRequest { paths: vec![frames[1].clone(), frames[1].clone()], frame_delay_ms: 100, loop_forever: true, frame_delay_overrides: Vec::new(), output_location: OutputLocation::AlongsideInput, output_path: None });
        assert_eq!(several.warnings[0].message, "2 frames are high-bit-depth HEIC, saved as 8-bit sRGB.");
        for path in frames.iter().chain(result.output_paths()).chain(several.output_paths()) { let _ = std::fs::remove_file(path); }
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
