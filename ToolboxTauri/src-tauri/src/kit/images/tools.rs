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
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataRequest { pub paths: Vec<PathBuf>, pub output_location: OutputLocation }

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
pub fn strip_metadata(request: &MetadataRequest, input: PathBuf) -> JobOutcome { transform(&request.paths, input, &request.output_location, "-stripped", |image| Ok(image)) }

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
}
