use image::{DynamicImage, ImageFormat};
use image::AnimationDecoder;
use std::fs::File;
use std::io::{BufReader, Seek};
use std::path::PathBuf;

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::ToolError;

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
    pub opacity: u8,
    #[serde(default)] pub text: Option<String>,
    #[serde(default)] pub logo_path: Option<PathBuf>,
    #[serde(default = "default_watermark_position")] pub x: u32,
    #[serde(default = "default_watermark_position")] pub y: u32,
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
}
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GifExtractRequest { pub paths: Vec<PathBuf>, pub output_location: OutputLocation }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TiffRequest { pub paths: Vec<PathBuf>, pub output_location: OutputLocation }
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataRequest { pub paths: Vec<PathBuf>, pub output_location: OutputLocation }
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataReport { pub path: PathBuf, pub exif: bool, pub gps: bool, pub xmp: bool, pub icc: bool, pub orientation: bool, pub unsupported: Vec<String> }

pub fn resize(request: &ResizeRequest, input: PathBuf) -> JobOutcome { transform(request.paths.as_slice(), input, &request.output_location, "-resized", |image| { if request.width == 0 || request.height == 0 { return Err("Image dimensions must be positive.".to_string()); } Ok(image.resize_exact(request.width, request.height, image::imageops::FilterType::Lanczos3)) }) }
pub fn rotate(request: &RotateRequest, input: PathBuf) -> JobOutcome { transform(request.paths.as_slice(), input, &request.output_location, "-rotated", |image| { Ok(match request.degrees.rem_euclid(360) { 90 => image.rotate90(), 180 => image.rotate180(), 270 => image.rotate270(), 0 => image, _ => return Err("Rotation must be 0, 90, 180, or 270 degrees.".to_string()) }) }) }
pub fn crop(request: &CropRequest, input: PathBuf) -> JobOutcome { transform(request.paths.as_slice(), input, &request.output_location, "-cropped", |image| { if request.width == 0 || request.height == 0 || request.x.saturating_add(request.width) > image.width() || request.y.saturating_add(request.height) > image.height() { return Err("Crop rectangle must fit inside the image.".to_string()); } Ok(image.crop_imm(request.x, request.y, request.width, request.height)) }) }
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
        let mut image = image.to_rgba8();
        let original_alpha = image.pixels().map(|pixel| pixel.0[3]).collect::<Vec<_>>();
        let alpha = request.opacity.min(100) as u16 * 255 / 100;
        let mut watermark = image::RgbaImage::new(image.width(), image.height());
        if let Some(text) = request.text.as_deref().filter(|text| !text.trim().is_empty()) {
            draw_text(&mut watermark, text, request.x, request.y, alpha as u8);
        }
        if let Some(path) = request.logo_path.as_ref() {
            let logo = image::open(path).map_err(|error| format!("Could not read watermark logo: {error}"))?.to_rgba8();
            image::imageops::overlay(&mut watermark, &logo, request.x as i64, request.y as i64);
        }
        if watermark.pixels().all(|pixel| pixel.0[3] == 0) {
            return Err("Provide watermark text or a logo image.".to_string());
        }
        image::imageops::overlay(&mut image, &watermark, 0, 0);
        for (pixel, alpha) in image.pixels_mut().zip(original_alpha) { pixel.0[3] = alpha; }
        Ok(DynamicImage::ImageRgba8(image))
    })
}

fn default_watermark_position() -> u32 { 16 }

fn draw_text(canvas: &mut image::RgbaImage, text: &str, x: u32, y: u32, alpha: u8) {
    let scale = 3;
    for (index, character) in text.chars().enumerate() {
        let glyph = glyph(character);
        let origin_x = x.saturating_add(index as u32 * 6 * scale);
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    for dy in 0..scale { for dx in 0..scale {
                        let px = origin_x + column * scale + dx;
                        let py = y + row as u32 * scale + dy;
                        if px < canvas.width() && py < canvas.height() { canvas.put_pixel(px, py, image::Rgba([255, 255, 255, alpha])); }
                    }}
                }
            }
        }
    }
}

fn glyph(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        ' ' => [0; 7],
        _ => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b00000, 0b00100],
    }
}
pub fn icon_set(request: &IconSetRequest, input: PathBuf) -> JobOutcome {
    let image = match image::open(&input) { Ok(image) => image, Err(error) => return failure(input, format!("Could not read image: {error}")) };
    let (prefix, sizes) = match icon_plan(&request.preset, &request.sizes) { Ok(plan) => plan, Err(error) => return failure(input, error) };
    let mut outputs = Vec::new();
    for size in sizes {
        let output = OutputNaming::get_destination(&input, &request.output_location, &format!("-{prefix}-{size}"), "png");
        if let Err(error) = image.resize_exact(size, size, image::imageops::FilterType::Lanczos3).save(&output) {
            for created in &outputs { let _ = std::fs::remove_file(created); }
            return failure(input, format!("Could not save icon: {error}"));
        }
        outputs.push(output);
    }
    let detail = match request.preset.as_str() {
        "macos" => "macOS PNG icon set saved; ICNS container is unavailable on this target.",
        "favicon" => "Favicon PNG icon set saved; ICO container is unavailable on this target.",
        "ios" => "iOS PNG icon set saved.",
        "android" => "Android PNG icon set saved.",
        _ => "Custom PNG icon set saved.",
    };
    JobOutcome { input_path: input, output_paths: outputs, detail: detail.to_string(), failure: None }
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
    let output = OutputNaming::get_destination(&input, &request.output_location, "-animated", "gif");
    let file = match File::create(&output) { Ok(file) => file, Err(error) => return failure(input, error.to_string()) };
    let mut encoder = image::codecs::gif::GifEncoder::new(file);
    if request.loop_forever { if let Err(error) = encoder.set_repeat(image::codecs::gif::Repeat::Infinite) { let _ = std::fs::remove_file(&output); return failure(input, format!("Could not configure GIF loop: {error}")); } }
    let images = request.paths.iter().map(|path| image::open(path).map(|image| image.to_rgba8()).map_err(|error| error.to_string())).collect::<Result<Vec<_>, _>>();
    let images = match images { Ok(images) => images, Err(error) => { let _ = std::fs::remove_file(&output); return failure(input, format!("Could not create GIF: {error}")); } };
    let width = images.iter().map(|image| image.width()).max().unwrap_or(0);
    let height = images.iter().map(|image| image.height()).max().unwrap_or(0);
    let delay = image::Delay::from_numer_denom_ms(request.frame_delay_ms.clamp(1, 60_000), 1);
    let frames = images.into_iter().map(|image| { let mut canvas = image::RgbaImage::new(width, height); image::imageops::overlay(&mut canvas, &image, 0, 0); image::Frame::from_parts(canvas, 0, 0, delay) }).collect::<Vec<_>>();
    match encoder.encode_frames(frames) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: "GIF saved".to_string(), failure: None }, Err(error) => { let _ = std::fs::remove_file(&output); failure(input, format!("Could not create GIF: {error}")) } }
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
        let output = OutputNaming::get_destination(&input, &request.output_location, &format!("-frame-{}", index + 1), "png");
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay_ms = (numerator as u64 * 1000 / denominator.max(1) as u64).max(1);
        if let Err(error) = frame.into_buffer().save(&output) {
            for created in &outputs { let _ = std::fs::remove_file(created); }
            return failure(input, error.to_string());
        }
        timing.push(serde_json::json!({ "file": output, "delayMs": delay_ms }));
        outputs.push(output);
    }
    let timing_path = OutputNaming::get_destination(&input, &request.output_location, "-frame-timing", "json");
    if let Err(error) = std::fs::write(&timing_path, serde_json::to_vec_pretty(&timing).unwrap_or_default()) {
        for created in &outputs { let _ = std::fs::remove_file(created); }
        return failure(input, format!("Could not save GIF timing manifest: {error}"));
    }
    outputs.push(timing_path);
    JobOutcome { input_path: input, output_paths: outputs, detail: "GIF frames saved".to_string(), failure: None }
}

pub fn tiff(request: &TiffRequest) -> JobOutcome {
    let Some(input) = request.paths.first().cloned() else { return failure(PathBuf::new(), "Select at least one TIFF file.".to_string()); };
    let mut pages = Vec::new();
    for path in &request.paths {
        match read_tiff_pages(path) {
            Ok(mut decoded) => pages.append(&mut decoded),
            Err(error) => return failure(input, error),
        }
    }
    let output = if request.paths.len() == 1 {
        let mut outputs = Vec::new();
        for (index, page) in pages.iter().enumerate() {
            let output = OutputNaming::get_destination(&input, &request.output_location, &format!("-page-{}", index + 1), "tiff");
            if let Err(error) = write_tiff_page(&output, page) { return failure(input, format!("Could not write TIFF page: {error}")); }
            outputs.push(output);
        }
        return JobOutcome { input_path: input, output_paths: outputs, detail: "TIFF pages saved".to_string(), failure: None };
    } else {
        OutputNaming::get_destination(&input, &request.output_location, "-combined", "tiff")
    };
    if let Err(error) = write_tiff_pages(&output, &pages) { return failure(input, format!("Could not combine TIFF pages: {error}")); }
    JobOutcome { input_path: input, output_paths: vec![output], detail: "TIFF pages combined".to_string(), failure: None }
}

#[derive(Clone)]
enum TiffPage { Gray { width: u32, height: u32, data: Vec<u8> }, Rgb { width: u32, height: u32, data: Vec<u8> }, Rgba { width: u32, height: u32, data: Vec<u8> } }

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
pub fn inspect_metadata(input: PathBuf) -> Result<MetadataReport, String> {
    let bytes = std::fs::read(&input).map_err(|error| format!("Could not read image metadata: {error}"))?;
    let exif = contains(&bytes, b"Exif\0\0");
    let xmp = contains(&bytes, b"http://ns.adobe.com/xap/1.0/") || contains(&bytes, b"<x:xmpmeta");
    let icc = contains(&bytes, b"ICC_PROFILE") || contains(&bytes, b"iCCP");
    let gps = exif && (contains(&bytes, b"GPS") || contains(&bytes, b"gps"));
    let orientation = exif && (contains(&bytes, b"Orientation") || contains(&bytes, b"orientation"));
    let supported = exif || xmp || icc;
    Ok(MetadataReport { path: input, exif, gps, xmp, icc, orientation, unsupported: if supported { Vec::new() } else { vec!["No supported EXIF, GPS, XMP, or ICC metadata marker was recognized.".to_string()] } })
}

pub fn strip_metadata(request: &MetadataRequest, input: PathBuf) -> JobOutcome {
    let image = match image::open(&input) { Ok(image) => image, Err(error) => return failure(input, format!("Could not read image: {error}")) };
    let output = OutputNaming::get_destination(&input, &request.output_location, "-stripped", input.extension().and_then(|extension| extension.to_str()).unwrap_or("png"));
    let format = ImageFormat::from_path(&output).unwrap_or(ImageFormat::Png);
    if let Err(error) = image.save_with_format(&output, format) { return failure(input, format!("Could not save metadata-free image: {error}")); }
    match inspect_metadata(output.clone()) {
        Ok(report) if !report.exif && !report.xmp && !report.icc => JobOutcome { input_path: input, output_paths: vec![output], detail: "Metadata removed and verified".to_string(), failure: None },
        Ok(_) => { let _ = std::fs::remove_file(&output); failure(input, "Image encoder retained metadata that could not be removed safely.".to_string()) },
        Err(error) => { let _ = std::fs::remove_file(&output); failure(input, format!("Could not verify metadata removal: {error}")) },
    }
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool { bytes.windows(needle.len()).any(|window| window == needle) }

fn transform<F>(_: &[PathBuf], input: PathBuf, location: &OutputLocation, suffix: &str, edit: F) -> JobOutcome where F: FnOnce(DynamicImage) -> Result<DynamicImage, String> {
    let image = match image::open(&input) { Ok(image) => image, Err(error) => return failure(input, format!("Could not read image: {error}")) };
    let image = match edit(image) { Ok(image) => image, Err(error) => return failure(input, error) };
    let extension = input.extension().and_then(|extension| extension.to_str()).unwrap_or("png");
    let output = OutputNaming::get_destination(&input, location, suffix, extension);
    let format = ImageFormat::from_path(&output).unwrap_or(ImageFormat::Png);
    match image.save_with_format(&output, format) { Ok(_) => JobOutcome { input_path: input, output_paths: vec![output], detail: "Image saved".to_string(), failure: None }, Err(error) => failure(input, format!("Could not save image: {error}")) }
}

fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::processing(error)) }

#[cfg(test)]
mod tests {
    use super::*;
    fn path(name: &str) -> PathBuf { std::env::temp_dir().join(format!("toolbox_tiff_{}_{}", std::process::id(), name)) }

    #[test]
    fn splits_and_combines_pages_in_order() {
        let input = path("source.tiff");
        let file = File::create(&input).unwrap();
        let mut encoder = tiff::encoder::TiffEncoder::new(file).unwrap();
        encoder.write_image::<tiff::encoder::colortype::Gray8>(2, 1, &[10, 20]).unwrap();
        encoder.write_image::<tiff::encoder::colortype::Gray8>(2, 1, &[30, 40]).unwrap();

        let split = tiff(&TiffRequest { paths: vec![input.clone()], output_location: OutputLocation::AlongsideInput });
        assert_eq!(split.output_paths.len(), 2);
        assert_eq!(read_tiff_pages(&split.output_paths[0]).unwrap().len(), 1);
        assert_eq!(read_tiff_pages(&split.output_paths[1]).unwrap().len(), 1);

        let first = path("first.tiff");
        let second = path("second.tiff");
        write_tiff_page(&first, &TiffPage::Gray { width: 1, height: 1, data: vec![1] }).unwrap();
        write_tiff_page(&second, &TiffPage::Gray { width: 1, height: 1, data: vec![2] }).unwrap();
        let combined = tiff(&TiffRequest { paths: vec![first.clone(), second.clone()], output_location: OutputLocation::AlongsideInput });
        let pages = read_tiff_pages(&combined.output_paths[0]).unwrap();
        assert_eq!(pages.len(), 2);
        assert!(matches!(&pages[0], TiffPage::Gray { data, .. } if data == &vec![1]));
        assert!(matches!(&pages[1], TiffPage::Gray { data, .. } if data == &vec![2]));

        for output in split.output_paths { let _ = std::fs::remove_file(output); }
        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
        let _ = std::fs::remove_file(combined.output_paths[0].clone());
    }

    #[test]
    fn rejects_unsupported_tiff_bit_depths() {
        let input = path("unsupported.tiff");
        let file = File::create(&input).unwrap();
        let mut encoder = tiff::encoder::TiffEncoder::new(file).unwrap();
        encoder.write_image::<tiff::encoder::colortype::Gray16>(1, 1, &[1]).unwrap();
        let result = tiff(&TiffRequest { paths: vec![input.clone()], output_location: OutputLocation::AlongsideInput });
        assert!(result.failure.is_some());
        let _ = std::fs::remove_file(input);
    }

    #[test]
    fn reports_unrecognized_metadata_explicitly() {
        let input = path("metadata.png");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255])).save(&input).unwrap();
        let report = inspect_metadata(input.clone()).unwrap();
        assert!(!report.exif && !report.gps && !report.xmp && !report.icc);
        assert!(!report.unsupported.is_empty());
        let _ = std::fs::remove_file(input);
    }

    #[test]
    fn text_watermark_composites_without_changing_alpha() {
        let input = path("watermark-input.png");
        let image = image::RgbaImage::from_pixel(64, 32, image::Rgba([10, 20, 30, 77]));
        image.save(&input).unwrap();
        let result = watermark(&WatermarkRequest { paths: vec![input.clone()], opacity: 50, text: Some("TOOL".to_string()), logo_path: None, x: 2, y: 2, output_location: OutputLocation::AlongsideInput }, input.clone());
        let output = result.output_paths.first().unwrap();
        let output_image = image::open(output).unwrap().to_rgba8();
        assert!(output_image.pixels().any(|pixel| pixel.0[0..3] != [10, 20, 30]));
        assert!(output_image.pixels().all(|pixel| pixel.0[3] == 77));
        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn gif_creation_preserves_order_timing_and_canvas() {
        let first = path("gif-first.png");
        let second = path("gif-second.png");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 128])).save(&first).unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 255, 0, 255])).save(&second).unwrap();
        let result = gif_create(&GifCreateRequest { paths: vec![first.clone(), second.clone()], frame_delay_ms: 250, loop_forever: true, output_location: OutputLocation::AlongsideInput });
        let output = result.output_paths.first().unwrap();
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
    fn icon_presets_are_named_and_reject_invalid_custom_sizes() {
        let (prefix, macos) = icon_plan("macos", &[]).unwrap();
        assert_eq!(prefix, "macos-icon");
        assert_eq!(macos, vec![16, 32, 128, 256, 512, 1024]);
        assert!(icon_plan("custom", &[0, 5000]).is_err());
        assert!(icon_plan("unknown", &[]).is_err());
    }
}
