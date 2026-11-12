use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use image::metadata::Orientation;
use image::{DynamicImage, GenericImageView};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};

use crate::kit::common::limits::{self, HelperDeadline, StoppableHelper};
use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::{LimitKind, OutcomeDetail, OutcomePointer, OutcomeWarning, PackNeed, SkipCode, SkipReason, ToolError, WarningKind};
use crate::kit::images::{self, ImageFamily};
use crate::kit::pdf::metadata::{effective_crop_bounds, page_bounds, page_rotation};
use crate::kit::pdf::text_layer::{self, PageText};
use crate::kit::pdf::unicode_font::UnicodeFont;
use crate::kit::pdf::page_ranges::{validate_page_ranges, PageRanges};
use crate::kit::resources;
use crate::packs::uninstall::{RequiredPack, RunPacks};

const OCR_MAX_INPUT_BYTES: u64 = 100 * 1024 * 1024;
const OCR_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;
const OCR_RENDER_DPI: u32 = 200;
const OCR_MAX_SINGLE_PIXELS: u64 = 40_000_000;
const OCR_MAX_TOTAL_PIXELS: u64 = 120_000_000;
const OCR_MAX_RENDERED_BYTES: u64 = 50 * 1024 * 1024;
const OCR_DEADLINE: HelperDeadline = HelperDeadline { per_file: Duration::from_secs(30), per_page: Duration::from_secs(30) };
/// The language OCR reads with the data bundled in the app. Every other language needs a pack.
const ENGLISH: &str = crate::packs::BUILT_IN_OCR_LANGUAGE.0;
/// Development builds read `<code>.traineddata` from this folder when the language's pack is not installed.
const OCR_LANGUAGE_DIR_ENV: &str = "TOOLBOX_OCR_LANGUAGE_DIR";
const OCR_TEXT_SUFFIX: &str = "-ocr";
const OCR_PAGE_SEGMENTATION: &str = "3";
const OCR_NO_TEXT: &str = "OCR completed but found no readable text.";
/// The resource name the invisible text layer's font is registered under on each page.
const OCR_LAYER_FONT: &str = "ToolboxOcr";
const OCR_PASSWORD_PROTECTED: &str = "It's password-protected. Remove its password in File Security, then run OCR again.";

#[derive(Debug, Clone, Copy)]
struct OcrPage {
    number: u32,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrMode { SearchablePdf, Text }

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrRequest {
    pub paths: Vec<PathBuf>,
    #[serde(default)] pub page_range: Option<String>,
    pub mode: String,
    #[serde(default)] pub force: bool,
    pub languages: Vec<String>,
    pub output_location: OutputLocation,
    /// Files read in another language than `languages`, by Tesseract code, as in `hin`.
    #[serde(default)] pub language_overrides: HashMap<PathBuf, Vec<String>>,
}

impl OcrRequest {
    fn page_range(&self) -> Option<&str> { self.page_range.as_deref().filter(|range| !range.trim().is_empty()) }

    pub fn mode(&self) -> Result<OcrMode, String> {
        match self.mode.as_str() {
            "searchablePdf" => Ok(OcrMode::SearchablePdf),
            "text" => Ok(OcrMode::Text),
            other => Err(format!("\"{other}\" is not an OCR output. Choose a searchable PDF or a text file.")),
        }
    }

    /// The languages `path` is read in, in the order they go to the engine: its override when it
    /// has one, else the request's languages.
    pub fn languages_for(&self, path: &Path) -> Vec<String> {
        self.language_overrides.get(path).unwrap_or(&self.languages).clone()
    }

    /// Every language list must name at least one language, and only English or a language a
    /// catalog pack reads. Whether that pack is installed is checked per file when it runs.
    fn check_languages(&self, packs: &RunPacks) -> Result<(), String> {
        for languages in std::iter::once(&self.languages).chain(self.language_overrides.values()) {
            if languages.is_empty() { return Err("Choose at least one recognition language.".to_string()); }
            if let Some(code) = languages.iter().find(|code| !knows_language(packs, code)) {
                return Err(format!("\"{code}\" is not a recognition language Toolbox knows."));
            }
        }
        Ok(())
    }
}

/// English, or a language a catalog pack reads.
fn knows_language(packs: &RunPacks, code: &str) -> bool {
    code == ENGLISH || packs.catalog().is_ok_and(|catalog| catalog.packs.iter().any(|pack| pack.ocr_language.as_ref().is_some_and(|language| language.code == code)))
}

pub fn validate_ocr_request(request: &OcrRequest, packs: &RunPacks) -> Result<(), Vec<JobOutcome>> {
    validate_page_ranges(&request.paths, request.page_range())?;
    let Err(error) = request.mode().and_then(|_| request.check_languages(packs)) else { return Ok(()) };
    let error = ToolError::invalid_input(error);
    if request.paths.is_empty() { return Err(vec![JobOutcome::failure(PathBuf::new(), error)]); }
    Err(request.paths.iter().cloned().map(|path| JobOutcome::failure(path, error.clone())).collect())
}

pub struct OcrRun {
    pub request: OcrRequest,
    pub packs: RunPacks,
}

/// One language's recognition data and the name the result names it by.
#[derive(Debug)]
struct OcrLanguage {
    data: PathBuf,
    name: String,
}

/// The language data for `code`: English from the app, any other language from its pack. A pack
/// that is not installed, is damaged or does not fit this app fails with a pointer to it.
fn ocr_language(packs: &RunPacks, code: &str) -> Result<OcrLanguage, ToolError> {
    if code == ENGLISH {
        let data = find_resource("engTraineddata", "TOOLBOX_TESSDATA_PATH", &ocr_resource_root()).map_err(|_| ToolError::unavailable(OcrComponent::EnglishData.to_string()))?;
        return Ok(OcrLanguage { data, name: "English".into() });
    }
    let (pack, language) = packs.catalog()?.packs.iter()
        .find_map(|pack| pack.ocr_language.as_ref().filter(|language| language.code == code).map(|language| (pack, language)))
        .ok_or_else(|| ToolError::invalid_input(format!("Toolbox can't read text in language {code:?}.")))?;
    let file_name = format!("{code}.traineddata");
    let development_override = cfg!(debug_assertions).then(|| std::env::var_os(OCR_LANGUAGE_DIR_ENV)).flatten()
        .map(|folder| PathBuf::from(folder).join(&file_name));
    let required = RequiredPack { id: &pack.id, name: &language.name };
    let resolved = packs.resolve(&format!("tessdata/{file_name}"), required, Some(ocr_resource_root()), development_override)
        .map_err(|mut error| {
            if matches!(error.pointer, Some(OutcomePointer::Pack { reason: PackNeed::NotInstalled, .. })) {
                error.message = format!("Needs the {} text recognition pack, which isn't installed. Nothing was saved.", language.name);
            }
            error
        })?;
    // Tesseract reads `<code>.traineddata` from the folder it is given.
    if resolved.path.file_name() != Some(OsStr::new(&file_name)) {
        return Err(ToolError::unavailable(format!("The {} pack has no {file_name}.", language.name)));
    }
    Ok(OcrLanguage { data: resolved.path, name: language.name.clone() })
}

/// The languages one file is read in.
struct OcrLanguages {
    /// The folder the engine reads every language's data from.
    tessdata_dir: PathBuf,
    /// The languages' names, in the order they go to the engine.
    names: Vec<String>,
    /// Holds the folder the languages were gathered into when their data lives in different
    /// folders, such as English in the app and Hindi in its pack. It is removed with this.
    _gathered: Option<OcrWorkspace>,
}

/// The engine reads every language from one folder, so data from different folders is gathered
/// into a private one first.
fn ocr_languages(packs: &RunPacks, codes: &[String]) -> Result<OcrLanguages, ToolError> {
    let languages = codes.iter().map(|code| ocr_language(packs, code)).collect::<Result<Vec<_>, _>>()?;
    let folders = languages.iter().map(|language| language.data.parent()).collect::<Option<Vec<_>>>()
        .filter(|folders| !folders.is_empty())
        .ok_or_else(|| ToolError::processing("OCR language data has no parent directory."))?;
    let (tessdata_dir, gathered) = if folders.iter().all(|folder| *folder == folders[0]) {
        (folders[0].to_path_buf(), None)
    } else {
        let workspace = OcrWorkspace::new().map_err(ToolError::processing)?;
        for (code, language) in codes.iter().zip(&languages) {
            link_language(&language.data, &workspace.0.join(format!("{code}.traineddata")))
                .map_err(|error| ToolError::processing(format!("Could not prepare the recognition languages: {error}")))?;
        }
        (workspace.0.clone(), Some(workspace))
    };
    Ok(OcrLanguages { tessdata_dir, names: languages.into_iter().map(|language| language.name).collect(), _gathered: gathered })
}

/// Puts a language's data where the engine looks for it without copying it where the system
/// allows: a symbolic link, else a hard link, else a copy.
fn link_language(data: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    if std::os::unix::fs::symlink(data, link).is_ok() { return Ok(()); }
    if std::fs::hard_link(data, link).is_ok() { return Ok(()); }
    std::fs::copy(data, link).map(|_| ())
}

/// "English", "English and Hindi", "English, Hindi and Tamil".
fn spoken_names(names: &[String]) -> String {
    match names.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

pub fn ocr_pdf(run: &OcrRun, input: PathBuf) -> JobOutcome {
    ocr_pdf_within(run, input, OCR_DEADLINE)
}

/// Images go to the engine as they are; every other input is a PDF whose pages are rendered first.
pub(crate) fn ocr_pdf_within(run: &OcrRun, input: PathBuf, deadline: HelperDeadline) -> JobOutcome {
    let mode = match run.request.mode() {
        Ok(mode) => mode,
        Err(error) => return JobOutcome::failure(input, ToolError::invalid_input(error)),
    };
    if matches!(ImageFamily::of(&input), ImageFamily::Jpeg | ImageFamily::Png | ImageFamily::WebP | ImageFamily::Heif | ImageFamily::Tiff) {
        return image_within(run, input, mode, deadline);
    }
    match mode {
        OcrMode::SearchablePdf => searchable_pdf_within(run, input, deadline),
        OcrMode::Text => text_file_within(run, input, deadline),
    }
}

/// Recognises a photo or scan given as an image. It is already a raster, so nothing
/// is rendered: the engine reads each page the way it is shown, with its orientation
/// tag applied. A multi-page TIFF is read frame by frame in document order, and the
/// page range picks its frames as it picks a PDF's pages. A text file holds the
/// recognised text, a form feed between pages. A searchable PDF is a new PDF with one
/// page per frame, each holding its image by Images to PDF's rule, with the invisible
/// text layer over it.
fn image_within(run: &OcrRun, input: PathBuf, mode: OcrMode, deadline: HelperDeadline) -> JobOutcome {
    let request = &run.request;
    let extension = match mode { OcrMode::Text => "txt", OcrMode::SearchablePdf => "pdf" };
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, OCR_TEXT_SUFFIX, extension) {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve OCR output: {error}")),
    };
    let engine = match OcrEngine::resolve() {
        Ok(engine) => engine,
        Err(error) => return unavailable(input, error.to_string()),
    };
    let input_size = match std::fs::metadata(&input) {
        Ok(metadata) => metadata.len(),
        Err(error) => return failure(input, format!("Could not inspect image for OCR: {error}")),
    };
    if input_size > OCR_MAX_INPUT_BYTES { return JobOutcome::failure(input, too_large("Image")); }
    let languages = request.languages_for(&input);
    let OcrLanguages { tessdata_dir, names: language_names, _gathered } = match ocr_languages(&run.packs, &languages) {
        Ok(found) => found,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let mut pages = match ImagePages::open(&input) {
        Ok(pages) => pages,
        Err(error) => return failure(input, format!("Could not read the image for OCR: {error}")),
    };
    // One range applies to every file, so a range that leaves an image no pages skips it as it would a PDF.
    let total = pages.total();
    let selected = match request.page_range() {
        Some(range) => match pages_in_range(range, total) {
            Ok(selected) => selected,
            Err(NoPlan::Failed(error)) => return failure(input, error),
            Err(NoPlan::Skipped(reason)) => return JobOutcome::skipped(input, reason),
        },
        None => (0..total).collect(),
    };
    if let Err(error) = pages.check_pixels(&input, &selected) { return JobOutcome::failure(input, error); }
    let detail = image_detail(&pages, &selected, request.page_range().is_some(), mode, &language_names);
    let settings = match mode { OcrMode::Text => &[][..], OcrMode::SearchablePdf => WORD_BOX_OUTPUT };
    let clock = OcrClock::start(&input, deadline, selected.len());
    let mut document = ImagePdf::new();
    let mut texts = Vec::with_capacity(selected.len());
    let mut layers = Vec::with_capacity(selected.len());
    let mut reduced_from = None;
    let mut total_text = 0usize;
    // One page is decoded at a time, so a long TIFF never holds more than one frame's pixels.
    for (done, &index) in selected.iter().enumerate() {
        let (stop_at, timed_out) = clock.page(done);
        let page = match pages.read(index) {
            Ok(page) => page,
            Err(error) => return failure(input, format!("Could not read the image for OCR: {error}")),
        };
        reduced_from = reduced_from.or(page.reduced_from);
        let recognized = match recognize_image(&engine, &tessdata_dir, &page, &languages, settings, stop_at, &timed_out) {
            Ok(recognized) => recognized,
            Err(error) => return JobOutcome::failure(input, error),
        };
        clock.finished(done + 1);
        total_text = total_text.saturating_add(recognized.len()).saturating_add(1);
        if total_text > OCR_MAX_OUTPUT_BYTES { return JobOutcome::failure(input, text_too_long()); }
        if mode == OcrMode::Text {
            texts.push(recognized);
            continue;
        }
        let layout = match word_boxes(&recognized) {
            Ok(layout) => layout,
            Err(error) => return failure(input, error),
        };
        let (page_id, (width, height)) = match document.add_page(page) {
            Ok(added) => added,
            Err(error) => return failure(input, error),
        };
        let frame = DisplayedPage { index: layers.len(), bounds: [0., 0., width, height], rotation: 0 };
        layers.push((layers.len() as u32 + 1, page_id, frame, layout));
    }
    if mode == OcrMode::Text {
        let text = match ocr_text(&texts) {
            Ok(text) => text,
            Err(error) => return failure(input, error),
        };
        if text.is_empty() { return failure(input, OCR_NO_TEXT.to_string()); }
        if let Err(error) = std::fs::write(output.path(), text) { return failure(input, format!("Could not write OCR output: {error}")); }
        return match output.publish() {
            Ok(path) => success(input, path, &detail),
            Err(error) => failure(input, format!("Could not publish OCR output: {error}")),
        };
    }
    if layers.iter().all(|(_, _, _, layout)| layout.lines.is_empty()) { return failure(input, OCR_NO_TEXT.to_string()); }
    let mut document = document.finish();
    let original = document.clone();
    let expected = match crate::kit::images::tools::bundled_text_font_bytes().map_err(ToolError::processing).and_then(|font| add_text_layer(&mut document, &layers, font)) {
        Ok(expected) => expected,
        Err(error) => return JobOutcome::failure(input, error),
    };
    if let Err(error) = document.save(output.path()) { return failure(input, format!("Could not write OCR output: {error}")); }
    #[cfg(test)]
    if BREAK_SEARCHABLE_OUTPUT_FOR.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_deref() == Some(input.as_path()) {
        let _ = std::fs::write(output.path(), b"%PDF-1.7\nnot a PDF any more\n");
    }
    if let Err(error) = validate_searchable_pdf(&original, output.path(), &expected) {
        return failure(input, format!("The searchable PDF didn't pass its check, so it wasn't saved: {error}"));
    }
    let outcome = match output.publish() {
        Ok(path) => success(input, path, &detail),
        Err(error) => return failure(input, format!("Could not publish OCR output: {error}")),
    };
    match reduced_from {
        Some((bits, family)) => outcome.with_warning(OutcomeWarning {
            kind: WarningKind::PrecisionReduced,
            message: format!("Saved as 8-bit colour, because the {} is {bits}-bit.", if family == ImageFamily::Heif { "photo" } else { "image" }),
        }),
        None => outcome,
    }
}

/// The pages of an image input: one for a photo or scan, every frame of a TIFF.
enum ImagePages {
    Single(PathBuf),
    Tiff(images::TiffPages),
}

/// One page of an image input, ready to recognise and place.
struct ImagePage {
    image: DynamicImage,
    orientation: Option<Orientation>,
    has_alpha: bool,
    /// A baseline JPEG's own bytes, which a searchable PDF keeps as they are.
    jpeg: Option<Vec<u8>>,
    /// The bit depth and family of an image over 8 bits per channel, which is saved at 8.
    reduced_from: Option<(u8, ImageFamily)>,
}

impl ImagePages {
    fn open(input: &Path) -> Result<Self, String> {
        match ImageFamily::of(input) {
            ImageFamily::Tiff => images::TiffPages::open(input).map(Self::Tiff),
            _ => Ok(Self::Single(input.to_path_buf())),
        }
    }

    fn total(&self) -> usize {
        match self { Self::Single(_) => 1, Self::Tiff(tiff) => tiff.total() }
    }

    /// Holds each selected page to the pixel limit a rendered PDF page is, and all of
    /// them together to the total, read from headers before any pixels are decoded.
    fn check_pixels(&mut self, input: &Path, selected: &[usize]) -> Result<(), ToolError> {
        let tiff = match self {
            Self::Single(_) => return check_image_pixels(input),
            Self::Tiff(tiff) => tiff,
        };
        let mut total_pixels = 0u64;
        for &index in selected {
            let (width, height) = tiff.dimensions(index).map_err(|error| ToolError::processing(format!("Could not read the image for OCR: {error}")))?;
            let pixels = u64::from(width) * u64::from(height);
            if pixels > OCR_MAX_SINGLE_PIXELS {
                return Err(ToolError::limit_exceeded(
                    LimitKind::Decompression,
                    format!("TIFF page {} has more than {} million pixels, which is over the OCR limit. Use a smaller image and try again.", index + 1, OCR_MAX_SINGLE_PIXELS / 1_000_000),
                ));
            }
            total_pixels += pixels;
            if total_pixels > OCR_MAX_TOTAL_PIXELS {
                return Err(ToolError::limit_exceeded(
                    LimitKind::Decompression,
                    format!("The TIFF pages in scope have more than {} million pixels together, which is over the OCR limit. Choose a page range or split the TIFF, then try again.", OCR_MAX_TOTAL_PIXELS / 1_000_000),
                ));
            }
        }
        Ok(())
    }

    fn read(&mut self, index: usize) -> Result<ImagePage, String> {
        match self {
            Self::Single(path) => {
                let decoded = images::decode(path)?;
                let reduced_from = (decoded.bits_per_channel > 8).then_some((decoded.bits_per_channel, decoded.family));
                let (orientation, has_alpha, jpeg) = (decoded.orientation, decoded.pages.iter().any(|page| page.color().has_alpha()), decoded.baseline_jpeg.clone());
                Ok(ImagePage { image: decoded.into_first_page(), orientation, has_alpha, jpeg, reduced_from })
            }
            Self::Tiff(tiff) => {
                let (image, orientation) = tiff.read(index)?;
                let has_alpha = image.color().has_alpha();
                Ok(ImagePage { image, orientation, has_alpha, jpeg: None, reduced_from: None })
            }
        }
    }
}

/// "1 page recognized. English." for a photo. A TIFF names its pages as the OCR board
/// does: "3 TIFF pages recognized, saved as one 3-page PDF." or, with a range, "TIFF
/// pages 2 and 3 recognized."
fn image_detail(pages: &ImagePages, selected: &[usize], ranged: bool, mode: OcrMode, languages: &[String]) -> String {
    if matches!(pages, ImagePages::Single(_)) { return recognized_detail(1, languages); }
    let label = if ranged {
        let numbers = selected.iter().map(|&index| index as u32 + 1).collect::<Vec<_>>();
        format!("TIFF p{}", &page_list(&numbers)[1..])
    } else {
        count(selected.len(), "TIFF page", "TIFF pages")
    };
    match mode {
        OcrMode::SearchablePdf if selected.len() > 1 => format!("{label} recognized, saved as one {}-page PDF.", selected.len()),
        _ => format!("{label} recognized."),
    }
}

/// An image the engine would read is held to the pixel limit a rendered PDF page is,
/// read from its header before any pixels are decoded.
fn check_image_pixels(input: &Path) -> Result<(), ToolError> {
    let unreadable = |error: String| ToolError::processing(format!("Could not read the image for OCR: {error}"));
    let (width, height) = if ImageFamily::of(input) == ImageFamily::Heif {
        let bytes = std::fs::read(input).map_err(|error| unreadable(error.to_string()))?;
        heif::probe(&bytes).map(|info| (info.width, info.height)).map_err(|error| unreadable(error.to_string()))?
    } else {
        image::ImageReader::open(input).and_then(|reader| reader.with_guessed_format()).map_err(|error| unreadable(error.to_string()))?
            .into_dimensions().map_err(|error| unreadable(error.to_string()))?
    };
    if u64::from(width) * u64::from(height) > OCR_MAX_SINGLE_PIXELS {
        return Err(ToolError::limit_exceeded(
            LimitKind::Decompression,
            format!("This image has more than {} million pixels, which is over the OCR limit. Use a smaller image and try again.", OCR_MAX_SINGLE_PIXELS / 1_000_000),
        ));
    }
    Ok(())
}

/// Gives the engine the page as it is shown: the orientation tag applied, 8 bits
/// per channel, and any transparency laid over white, as a viewer shows it.
fn recognize_image(engine: &OcrEngine, tessdata_dir: &Path, page: &ImagePage, languages: &[String], settings: &[&str], stop_at: Instant, timed_out: &str) -> Result<Vec<u8>, ToolError> {
    let workspace = OcrWorkspace::new().map_err(ToolError::processing)?;
    let image = &page.image;
    let mut shown = if page.has_alpha {
        let mut flat = image::RgbImage::from_pixel(image.width(), image.height(), image::Rgb([255, 255, 255]));
        for (x, y, pixel) in image.to_rgba8().enumerate_pixels() {
            let alpha = u16::from(pixel[3]);
            let over = |channel: u8| ((u16::from(channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
            flat.put_pixel(x, y, image::Rgb([over(pixel[0]), over(pixel[1]), over(pixel[2])]));
        }
        DynamicImage::ImageRgb8(flat)
    } else if image.color().has_color() {
        DynamicImage::ImageRgb8(image.to_rgb8())
    } else {
        DynamicImage::ImageLuma8(image.to_luma8())
    };
    if let Some(orientation) = page.orientation { shown.apply_orientation(orientation); }
    let path = workspace.page_prefix(1).with_extension("png");
    shown.save_with_format(&path, image::ImageFormat::Png).map_err(|error| ToolError::processing(format!("Could not prepare the image for OCR: {error}")))?;
    engine.recognize(&path, tessdata_dir, languages, settings, stop_at, timed_out)
}

/// A new PDF built one image page at a time.
struct ImagePdf {
    document: Document,
    pages_id: ObjectId,
    kids: Vec<Object>,
}

impl ImagePdf {
    fn new() -> Self {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        Self { document, pages_id, kids: Vec::new() }
    }

    /// Adds a page holding the image by Images to PDF's rule: a baseline JPEG goes in
    /// as its own bytes, and everything else is stored without loss as 8-bit Flate
    /// samples, with any alpha kept as a soft mask. One pixel is one point. The
    /// orientation tag is applied by how the image is drawn, so its data stays as it
    /// was, and the page is the size the image is shown at. Returns the page and its
    /// width and height.
    fn add_page(&mut self, page: ImagePage) -> Result<(ObjectId, (f32, f32)), String> {
        let ImagePage { image, orientation, has_alpha, jpeg, .. } = page;
        let document = &mut self.document;
        let (width, height) = (image.width(), image.height());
        let sampled = dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => width as i64, "Height" => height as i64, "BitsPerComponent" => 8 };
        let image_id = match jpeg.and_then(|bytes| jpeg_color_space(&bytes).map(|space| (bytes, space))) {
            Some((bytes, color_space)) => {
                let mut dict = sampled;
                dict.set("ColorSpace", color_space);
                dict.set("Filter", "DCTDecode");
                document.add_object(Stream::new(dict, bytes).with_compression(false))
            }
            None => {
                let (samples, color_space) = if image.color().has_color() { (image.to_rgb8().into_raw(), "DeviceRGB") } else { (image.to_luma8().into_raw(), "DeviceGray") };
                let mut dict = sampled.clone();
                dict.set("ColorSpace", color_space);
                if has_alpha {
                    let alpha = image.to_rgba8().pixels().map(|pixel| pixel[3]).collect::<Vec<_>>();
                    let mut mask = sampled;
                    mask.set("ColorSpace", "DeviceGray");
                    dict.set("SMask", document.add_object(flate_stream(mask, &alpha)?));
                }
                document.add_object(flate_stream(dict, &samples)?)
            }
        };
        // Where the image's top-left-origin unit square lands on the shown page, as
        // u' = m00 u + m01 v + o0 and v' = m10 u + m11 v + o1, all in fractions of each side.
        let (m00, m01, m10, m11, o0, o1): (f32, f32, f32, f32, f32, f32) = match orientation.unwrap_or(Orientation::NoTransforms) {
            Orientation::NoTransforms => (1., 0., 0., 1., 0., 0.),
            Orientation::Rotate90 => (0., -1., 1., 0., 1., 0.),
            Orientation::Rotate180 => (-1., 0., 0., -1., 1., 1.),
            Orientation::Rotate270 => (0., 1., -1., 0., 0., 1.),
            Orientation::FlipHorizontal => (-1., 0., 0., 1., 1., 0.),
            Orientation::FlipVertical => (1., 0., 0., -1., 0., 1.),
            Orientation::Rotate90FlipH => (0., 1., 1., 0., 0., 0.),
            Orientation::Rotate270FlipH => (0., -1., -1., 0., 1., 1.),
        };
        let (shown_width, shown_height) = if m00 == 0. { (height as f32, width as f32) } else { (width as f32, height as f32) };
        // PDF image space has its origin at the bottom left, so v = 1 - t and the page's y runs up.
        let (a, b, c, d) = (shown_width * m00, -shown_height * m10, -shown_width * m01, shown_height * m11);
        let (e, f) = (shown_width * (m01 + o0), shown_height * (1. - m11 - o1));
        let content = document.add_object(Stream::new(dictionary! {}, format!("q {a} {b} {c} {d} {e} {f} cm /Im0 Do Q").into_bytes()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => self.pages_id,
            "MediaBox" => vec![0.into(), 0.into(), shown_width.into(), shown_height.into()],
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } },
            "Contents" => content,
        });
        self.kids.push(page_id.into());
        Ok((page_id, (shown_width, shown_height)))
    }

    /// Closes the page tree in the order the pages were added.
    fn finish(self) -> Document {
        let Self { mut document, pages_id, kids } = self;
        let count = kids.len() as i64;
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document
    }
}

fn flate_stream(mut dict: lopdf::Dictionary, samples: &[u8]) -> Result<Stream, String> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(samples).map_err(|error| format!("Could not compress image: {error}"))?;
    dict.set("Filter", "FlateDecode");
    Ok(Stream::new(dict, encoder.finish().map_err(|error| format!("Could not compress image: {error}"))?).with_compression(false))
}

/// The component count in the JPEG's SOF0 header picks the PDF color space; a
/// four-component (CMYK) JPEG is stored without loss instead.
fn jpeg_color_space(bytes: &[u8]) -> Option<&'static str> {
    let mut at = 2;
    while let Some(&[0xFF, marker, high, low]) = bytes.get(at..at + 4) {
        match marker {
            0xFF => at += 1,
            0xC0 => return match bytes.get(at + 9)? { 1 => Some("DeviceGray"), 3 => Some("DeviceRGB"), _ => None },
            _ => at += 2 + usize::from(u16::from_be_bytes([high, low])),
        }
    }
    None
}

fn text_file_within(run: &OcrRun, input: PathBuf, deadline: HelperDeadline) -> JobOutcome {
    let request = &run.request;
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, OCR_TEXT_SUFFIX, "txt") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve OCR output: {error}")),
    };
    // A missing engine affects every file the same way, so it is reported before anything is read from the file.
    let resources = match OcrResources::resolve() {
        Ok(resources) => resources,
        Err(error) => return unavailable(input, error.to_string()),
    };
    // A language whose pack is missing, damaged or does not fit points at that pack. Nothing is saved.
    let languages = request.languages_for(&input);
    let OcrLanguages { tessdata_dir, names: language_names, _gathered } = match ocr_languages(&run.packs, &languages) {
        Ok(found) => found,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let input_size = match std::fs::metadata(&input) {
        Ok(metadata) => metadata.len(),
        Err(error) => return failure(input, format!("Could not inspect PDF for OCR: {error}")),
    };
    if input_size > OCR_MAX_INPUT_BYTES { return JobOutcome::failure(input, too_large("PDF")); }
    let document = match load_ocr_document(&input) {
        Ok(document) => document,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let plan = match PagePlan::of(&document, request.page_range(), request.force) {
        Ok(plan) => plan,
        Err(NoPlan::Failed(error)) => return failure(input, error),
        Err(NoPlan::Skipped(reason)) => return JobOutcome::skipped(input, reason),
    };
    let recognized = plan.recognized();
    if recognized.is_empty() { return JobOutcome::skipped(input, plan.text_present()); }
    if let Err(error) = check_render_limits(&document, &recognized) { return JobOutcome::failure(input, error); }
    let mut ocr = match run_ocr(&resources, &tessdata_dir, &input, &recognized, &languages, &[], &[], deadline) {
        Ok(pages) => pages.into_iter(),
        Err(error) => return JobOutcome::failure(input, error),
    };
    let kept = plan.kept();
    let mut existing = match existing_text(&input, &kept, deadline) {
        Ok(pages) => pages.into_iter(),
        Err(error) => return JobOutcome::failure(input, error),
    };
    // Recognised and kept pages are each in page order, so taking from whichever list
    // the page belongs to puts every page's text back in page order.
    let pages = plan.scope.iter().filter_map(|(_, text)| if plan.recognizes(*text) { ocr.next() } else { existing.next() }).collect::<Vec<_>>();
    let text = match ocr_text(&pages) {
        Ok(text) => text,
        Err(error) => return failure(input, error),
    };
    if text.len() > OCR_MAX_OUTPUT_BYTES { return JobOutcome::failure(input, text_too_long()); }
    if text.is_empty() { return failure(input, OCR_NO_TEXT.to_string()); }
    match std::fs::write(output.path(), text) {
        Ok(_) => match output.publish() {
            Ok(path) => plan.text_file_details(success(input, path, &plan.text_file_detail(&language_names)), &language_names, request.page_range()),
            Err(error) => failure(input, format!("Could not publish OCR output: {error}")),
        },
        Err(error) => failure(input, format!("Could not write OCR output: {error}")),
    }
}

fn load_ocr_document(input: &Path) -> Result<Document, ToolError> {
    let document = Document::load(input).map_err(|error| ToolError::processing(format!("Could not inspect PDF for OCR: {error}")))?;
    if document.is_encrypted() {
        return Err(ToolError::invalid_input(OCR_PASSWORD_PROTECTED).with_pointer(OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
    }
    Ok(document)
}

/// "31 pages recognized. English and Hindi."
fn recognized_detail(pages: usize, languages: &[String]) -> String {
    format!("{} recognized. {}.", count(pages, "page", "pages"), spoken_names(languages))
}

fn count(number: usize, one: &str, many: &str) -> String { format!("{number} {}", if number == 1 { one } else { many }) }

/// Why a file has no pages to work on: it can't be read, or the range leaves none of its pages.
#[derive(Debug)]
enum NoPlan { Failed(String), Skipped(SkipReason) }

/// The pages in scope, in page order, and the text each one already has.
struct PagePlan {
    scope: Vec<(OcrPage, PageText)>,
    /// Pages of the file outside the range, which a text file leaves out.
    outside: Vec<u32>,
    force: bool,
}

impl PagePlan {
    /// Checks every page in scope for text before anything is recognized. Without
    /// Force any text keeps a page as it is; with Force a page whose only text is an
    /// invisible layer is read again. Visible text is never read again.
    ///
    /// One range applies to every file in a batch, so it is clamped to this file's
    /// length; a file with no pages left is skipped. Pages are always taken in page
    /// order, whatever order the range was typed in.
    fn of(document: &Document, range: Option<&str>, force: bool) -> Result<Self, NoPlan> {
        let pages = document.get_pages();
        if pages.is_empty() { return Err(NoPlan::Failed("PDF has no pages to OCR.".to_string())); }
        let requested = range.map(|range| pages_in_range(range, pages.len())).transpose()?;
        let mut scope = Vec::new();
        let mut outside = Vec::new();
        for (number, page_id) in pages {
            if requested.as_ref().is_some_and(|indices| indices.binary_search(&(number as usize - 1)).is_err()) {
                outside.push(number);
                continue;
            }
            let text = text_layer::page_text(document, page_id).map_err(|error| NoPlan::Failed(format!("Page {number}'s text could not be checked: {error}")))?;
            scope.push((OcrPage { number }, text));
        }
        if scope.is_empty() { return Err(NoPlan::Failed("Select at least one PDF page for OCR.".to_string())); }
        Ok(Self { scope, outside, force })
    }

    fn recognizes(&self, text: PageText) -> bool { text == PageText::None || (self.force && text == PageText::Invisible) }

    fn recognized(&self) -> Vec<OcrPage> { self.scope.iter().filter(|(_, text)| self.recognizes(*text)).map(|(page, _)| *page).collect() }

    fn kept(&self) -> Vec<OcrPage> { self.scope.iter().filter(|(_, text)| !self.recognizes(*text)).map(|(page, _)| *page).collect() }

    fn numbers(&self, class: PageText) -> Vec<u32> { self.scope.iter().filter(|(_, text)| *text == class).map(|(page, _)| page.number).collect() }

    /// Pages read again that had an earlier invisible layer, which the new one replaces.
    fn replaced(&self) -> Vec<u32> { if self.force { self.numbers(PageText::Invisible) } else { Vec::new() } }

    /// Pages kept as they are because of an earlier invisible layer.
    fn kept_invisible(&self) -> Vec<u32> { if self.force { Vec::new() } else { self.numbers(PageText::Invisible) } }

    /// Why nothing was made when every page in scope keeps its text.
    fn text_present(&self) -> SkipReason {
        let (visible, invisible) = (self.numbers(PageText::Visible), self.kept_invisible());
        let classes = [(visible, "visible"), (invisible.clone(), "from an earlier OCR run")].into_iter()
            .filter(|(pages, _)| !pages.is_empty())
            .map(|(pages, class)| format!("{} {class}", page_list(&pages).to_lowercase()))
            .collect::<Vec<_>>();
        let mut message = format!("Every page already has text: {}. Nothing was made.", classes.join(", "));
        if !invisible.is_empty() { message.push_str(" Turn on Force to replace the earlier OCR text."); }
        SkipReason::new(SkipCode::TextAlreadyPresent, message)
    }

    /// "5 of 9 pages recognized, 4 skipped for their text." when pages kept their text.
    fn searchable_detail(&self, languages: &[String]) -> String {
        let (recognized, kept, replaced) = (self.recognized().len(), self.kept().len(), self.replaced().len());
        if kept > 0 {
            format!("{recognized} of {} recognized, {kept} skipped for {} text.", count(self.scope.len(), "page", "pages"), if kept == 1 { "its" } else { "their" })
        } else if replaced > 0 {
            format!("{} recognized, {replaced} with earlier OCR text replaced.", count(recognized, "page", "pages"))
        } else {
            recognized_detail(recognized, languages)
        }
    }

    /// "5 pages recognized, 3 pages of text added as they are" when pages kept their text.
    fn text_file_detail(&self, languages: &[String]) -> String {
        let (recognized, kept, replaced) = (self.recognized().len(), self.kept().len(), self.replaced().len());
        if kept > 0 {
            format!("{} recognized, {}", count(recognized, "page", "pages"), count(kept, "page of text added as it is", "pages of text added as they are"))
        } else if replaced > 0 {
            format!("{} recognized, {replaced} with earlier OCR text replaced.", count(recognized, "page", "pages"))
        } else {
            recognized_detail(recognized, languages)
        }
    }

    /// The page breakdown, only when some page kept its text or had it replaced.
    fn details(&self, kept_rows: [(&str, Vec<u32>, &str); 2], outside: Option<(String, Vec<u32>)>, languages: &[String]) -> Vec<OutcomeDetail> {
        if self.kept().is_empty() && self.replaced().is_empty() { return Vec::new(); }
        let row = |label: &str, pages: &[u32], after: &str| (!pages.is_empty()).then(|| OutcomeDetail { label: label.to_string(), value: format!("{}{after}", page_list(pages)) });
        let [first, second] = kept_rows;
        [
            row("Recognized", &self.numbers(PageText::None), ""),
            row("Earlier OCR text replaced", &self.replaced(), ""),
            row(first.0, &first.1, first.2),
            row(second.0, &second.1, second.2),
            outside.and_then(|(label, pages)| row(&label, &pages, "")),
            Some(OutcomeDetail { label: "Languages".to_string(), value: languages.join(", ") }),
        ].into_iter().flatten().collect()
    }

    fn searchable_details(&self, outcome: JobOutcome, languages: &[String]) -> JobOutcome {
        let details = self.details([
            ("Skipped, visible text", self.numbers(PageText::Visible), ""),
            ("Skipped, earlier OCR text", self.kept_invisible(), ". Turn on Force to replace it."),
        ], None, languages);
        outcome.with_page_details(details)
    }

    fn text_file_details(&self, outcome: JobOutcome, languages: &[String], range: Option<&str>) -> JobOutcome {
        let outside = range.map(|range| (format!("Outside {}, left out", range.trim()), self.outside.clone()));
        let details = self.details([
            ("Visible text, added as it is", self.numbers(PageText::Visible), ""),
            ("Earlier OCR text, added as it is", self.kept_invisible(), ""),
        ], outside, languages);
        outcome.with_page_details(details)
    }
}

/// The zero-based page indices `range` leaves in a file of `page_count` pages. One
/// range applies to every file in a batch, so a file it leaves no pages in is skipped.
fn pages_in_range(range: &str, page_count: usize) -> Result<Vec<usize>, NoPlan> {
    let ranges = PageRanges::parse(range).map_err(NoPlan::Failed)?;
    let clamped = ranges.clamp_or_skip(page_count).map_err(|skip| NoPlan::Skipped(SkipReason::new(
        skip.code,
        format!("No pages left in {ranges}. It has {}, so nothing was made.", count(page_count, "page", "pages")),
    )))?;
    Ok(clamped.page_indices())
}

/// "Page 8", "Pages 1-4 and 9", "Pages 1, 3-5 and 9". Pages are in page order.
fn page_list(pages: &[u32]) -> String {
    let mut spans: Vec<(u32, u32)> = Vec::new();
    for &page in pages {
        match spans.last_mut() {
            Some((_, last)) if *last + 1 == page => *last = page,
            _ => spans.push((page, page)),
        }
    }
    // Two pages in a row read as "2 and 3"; three or more as "5-7".
    let spans = spans.into_iter().flat_map(|(first, last)| match last - first {
        0 => vec![first.to_string()],
        1 => vec![first.to_string(), last.to_string()],
        _ => vec![format!("{first}-{last}")],
    }).collect::<Vec<_>>();
    let joined = match spans.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    };
    format!("{} {joined}", if pages.len() == 1 { "Page" } else { "Pages" })
}

/// The text a kept page already has, read with the bundled text reader one page at a
/// time, in page order. It is normalized the same way as recognised text.
fn existing_text(input: &Path, pages: &[OcrPage], deadline: HelperDeadline) -> Result<Vec<Vec<u8>>, ToolError> {
    if pages.is_empty() { return Ok(Vec::new()); }
    let reader = resources::resolve_helper(&resources::PDF_TEXT_EXTRACTOR).map_err(|_| ToolError::unavailable(OcrComponent::TextReader.to_string()))?;
    let limit = deadline.limit(1, pages.len());
    let stop_at = Instant::now() + limit;
    let timed_out = format!("Reading the existing text took longer than {} and was stopped. Nothing was saved.", limits::duration_label(limit));
    let mut texts = Vec::with_capacity(pages.len());
    let mut total = 0usize;
    for page in pages {
        let mut command = Command::new(&reader);
        command.args(["-enc", "UTF-8", "-f"]).arg(page.number.to_string()).arg("-l").arg(page.number.to_string()).arg(input).arg("-");
        let result = run_ocr_helper(command, stop_at, &timed_out)?;
        if !result.status.success() { return Err(ToolError::processing(stderr(result, "The PDF text reader could not read the existing text."))); }
        total = total.saturating_add(result.stdout.len()).saturating_add(1);
        if total > OCR_MAX_OUTPUT_BYTES { return Err(text_too_long()); }
        texts.push(result.stdout);
    }
    Ok(texts)
}

/// Only the pages that are rendered count toward the render limits.
fn check_render_limits(document: &Document, pages: &[OcrPage]) -> Result<(), ToolError> {
    let page_ids = document.get_pages();
    let mut total_pixels = 0u64;
    let over = |message: &str| ToolError::limit_exceeded(LimitKind::Decompression, message);
    for page in pages {
        let id = *page_ids.get(&page.number).ok_or_else(|| ToolError::processing("PDF page is missing."))?;
        let (left, bottom, right, top) = page_bounds(document, id).map_err(ToolError::processing)?;
        let pixels = rendered_pixels(right - left, top - bottom).map_err(ToolError::processing)?;
        if pixels > OCR_MAX_SINGLE_PIXELS {
            return Err(over("Selected PDF page exceeds the 40 million pixel OCR limit."));
        }
        total_pixels = total_pixels.checked_add(pixels).ok_or_else(|| over("Selected PDF pages exceed the OCR pixel limit."))?;
        if total_pixels > OCR_MAX_TOTAL_PIXELS {
            return Err(over("Selected PDF pages exceed the 120 million pixel OCR limit."));
        }
    }
    Ok(())
}

fn rendered_pixels(width: f32, height: f32) -> Result<u64, String> {
    let width = (width / 72.0 * OCR_RENDER_DPI as f32).ceil();
    let height = (height / 72.0 * OCR_RENDER_DPI as f32).ceil();
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("Selected PDF page has invalid OCR dimensions.".to_string());
    }
    (width as u64).checked_mul(height as u64).ok_or_else(|| "Selected PDF pages exceed the OCR pixel limit.".to_string())
}

struct OcrWorkspace(PathBuf);

impl OcrWorkspace {
    fn new() -> Result<Self, String> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        for attempt in 0..100 {
            let path = std::env::temp_dir().join(format!("toolbox_ocr_{}_{}_{}", std::process::id(), timestamp, attempt));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("Could not create private OCR workspace: {error}")),
            }
        }
        Err("Could not allocate a unique private OCR workspace.".to_string())
    }

    fn page_prefix(&self, page: u32) -> PathBuf { self.0.join(format!("page-{page}")) }
}

impl Drop for OcrWorkspace {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

/// The recognition engine, which every input needs. Language data comes per file.
struct OcrEngine {
    engine: PathBuf,
}

/// The engine plus the renderer that turns PDF pages into images for it.
struct OcrResources {
    engine: OcrEngine,
    renderer: PathBuf,
}

/// The OCR pieces a file needs. Each one names itself when it is missing, so the
/// person knows reinstalling is the fix rather than trying another file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OcrComponent { Engine, Renderer, EnglishData, TextReader }

impl std::fmt::Display for OcrComponent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Engine => "The text recognition engine is missing. Reinstall Toolbox.",
            Self::Renderer => "The PDF page renderer is missing. Reinstall Toolbox.",
            Self::EnglishData => "The English recognition data is missing. Reinstall Toolbox.",
            Self::TextReader => "The PDF text reader is missing. Reinstall Toolbox.",
        })
    }
}

impl OcrEngine {
    fn resolve() -> Result<Self, OcrComponent> {
        let root = ocr_resource_root();
        Ok(Self {
            engine: resources::resolve("tesseract", &root, "TOOLBOX_TESSERACT_PATH", "tesseract").map_err(|_| OcrComponent::Engine)?.path,
        })
    }

    /// Recognises one image. The engine runs on one thread with a fixed segmentation
    /// mode, resolution and language data, so the same image gives the same text every time.
    /// `settings` are extra engine arguments, such as `WORD_BOX_OUTPUT`; none gives plain text.
    fn recognize(&self, image: &Path, tessdata_dir: &Path, languages: &[String], settings: &[&str], stop_at: Instant, timed_out: &str) -> Result<Vec<u8>, ToolError> {
        let mut command = Command::new(&self.engine);
        command.env("OMP_THREAD_LIMIT", "1")
            .arg(image).arg("stdout")
            .arg("--tessdata-dir").arg(tessdata_dir)
            .arg("-l").arg(languages.join("+"))
            .arg("--psm").arg(OCR_PAGE_SEGMENTATION)
            .arg("--dpi").arg(OCR_RENDER_DPI.to_string())
            .args(settings);
        let result = run_ocr_helper(command, stop_at, timed_out)?;
        if !result.status.success() { return Err(ToolError::processing(stderr(result, "OCR engine failed."))); }
        Ok(result.stdout)
    }
}

impl OcrResources {
    fn resolve() -> Result<Self, OcrComponent> {
        let engine = OcrEngine::resolve()?;
        Ok(Self { engine, renderer: find_pdf_renderer().map_err(|_| OcrComponent::Renderer)? })
    }
}

/// One file's time limits while its pages are recognised. The pages share the file's
/// budget, so a hard page can use time an easy one left, but no page may take longer
/// than a one-page file is given, so one stuck page can't hold the file to the end of
/// its budget. Each finished page is reported, so the view shows the page count and a
/// stop says which page it was on.
struct OcrClock { input: PathBuf, file_stop: Instant, page_limit: Duration, total: usize }

impl OcrClock {
    fn start(input: &Path, deadline: HelperDeadline, total: usize) -> Self {
        limits::report_pages(0, total);
        Self { input: input.to_path_buf(), file_stop: Instant::now() + deadline.limit(1, total), page_limit: deadline.limit(1, 1), total }
    }

    /// When the page after `done` finished pages must stop, and what the file says if it does.
    fn page(&self, done: usize) -> (Instant, String) {
        let page_stop = Instant::now() + self.page_limit;
        if page_stop < self.file_stop {
            let message = format!("{}: that page took longer than {}. Nothing was saved.", limits::stopped_at(done, self.total), limits::duration_label(self.page_limit));
            (page_stop, message)
        } else {
            (self.file_stop, limits::file_time_limit_message(&self.input, done, self.total, 0))
        }
    }

    fn finished(&self, done: usize) { limits::report_pages(done, self.total) }
}

/// Renders and recognises each page on its own, so the text keeps page order and page breaks.
/// `area` is extra renderer arguments, such as `DISPLAYED_PAGE`; none renders the media box.
/// `settings` are extra engine arguments, such as `WORD_BOX_OUTPUT`; none gives plain text.
fn run_ocr(resources: &OcrResources, tessdata_dir: &Path, input: &Path, pages: &[OcrPage], languages: &[String], area: &[&str], settings: &[&str], deadline: HelperDeadline) -> Result<Vec<Vec<u8>>, ToolError> {
    let clock = OcrClock::start(input, deadline, pages.len());
    let workspace = OcrWorkspace::new().map_err(ToolError::processing)?;
    let mut recognized = Vec::with_capacity(pages.len());
    let mut total = 0usize;
    for (done, page) in pages.iter().enumerate() {
        let (stop_at, timed_out) = clock.page(done);
        let image = render_ocr_page(&resources.renderer, input, &workspace, *page, area, stop_at, &timed_out)?;
        let text = resources.engine.recognize(&image, tessdata_dir, languages, settings, stop_at, &timed_out)?;
        clock.finished(done + 1);
        total = total.saturating_add(text.len()).saturating_add(1);
        if total > OCR_MAX_OUTPUT_BYTES { return Err(text_too_long()); }
        recognized.push(text);
    }
    Ok(recognized)
}

/// Joins recognised pages with a form feed between them. Lines end in LF. A page
/// with no text stays an empty page, so page numbers still line up. When no page
/// has text the result is empty.
fn ocr_text(pages: &[Vec<u8>]) -> Result<String, String> {
    let pages = pages.iter().map(|page| normalize_ocr_text(page)).collect::<Result<Vec<_>, _>>()?;
    if pages.iter().all(String::is_empty) { return Ok(String::new()); }
    Ok(pages.iter().map(|page| if page.is_empty() { String::new() } else { format!("{page}\n") }).collect::<Vec<_>>().join("\u{c}"))
}

/// Makes a copy of the scan whose pages look the same but carry an invisible text
/// layer placed from the engine's word boxes, so the text can be selected, copied and
/// searched. The original content streams, resources and page boxes are kept as they
/// are; each recognised page only gains a font and one more content stream.
fn searchable_pdf_within(run: &OcrRun, input: PathBuf, deadline: HelperDeadline) -> JobOutcome {
    let request = &run.request;
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, OCR_TEXT_SUFFIX, "pdf") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve OCR output: {error}")),
    };
    let resources = match OcrResources::resolve() {
        Ok(resources) => resources,
        Err(error) => return unavailable(input, error.to_string()),
    };
    let languages = request.languages_for(&input);
    let OcrLanguages { tessdata_dir, names: language_names, _gathered } = match ocr_languages(&run.packs, &languages) {
        Ok(found) => found,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let input_size = match std::fs::metadata(&input) {
        Ok(metadata) => metadata.len(),
        Err(error) => return failure(input, format!("Could not inspect PDF for OCR: {error}")),
    };
    if input_size > OCR_MAX_INPUT_BYTES { return JobOutcome::failure(input, too_large("PDF")); }
    let mut document = match load_ocr_document(&input) {
        Ok(document) => document,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let plan = match PagePlan::of(&document, request.page_range(), request.force) {
        Ok(plan) => plan,
        Err(NoPlan::Failed(error)) => return failure(input, error),
        Err(NoPlan::Skipped(reason)) => return JobOutcome::skipped(input, reason),
    };
    let pages = plan.recognized();
    if pages.is_empty() { return JobOutcome::skipped(input, plan.text_present()); }
    let page_ids = document.get_pages();
    if let Err(error) = check_render_limits(&document, &pages) { return JobOutcome::failure(input, error); }
    let frames = match displayed_pages(&document, &page_ids, &pages) {
        Ok(frames) => frames,
        Err(error) => return failure(input, error),
    };
    let layouts = match run_ocr(&resources, &tessdata_dir, &input, &pages, &languages, DISPLAYED_PAGE, WORD_BOX_OUTPUT, deadline)
        .and_then(|recognized| recognized.iter().map(|tsv| word_boxes(tsv)).collect::<Result<Vec<_>, _>>().map_err(ToolError::processing))
    {
        Ok(layouts) => layouts,
        Err(error) => return JobOutcome::failure(input, error),
    };
    if layouts.iter().all(|layout| layout.lines.is_empty()) { return failure(input, OCR_NO_TEXT.to_string()); }
    // An earlier invisible layer goes before the new one is added, so each word is
    // found once. Visible content and every other page stay byte for byte as they were,
    // which the check below holds the output to.
    for number in plan.replaced() {
        let Some(id) = page_ids.get(&number) else { continue };
        if let Err(error) = text_layer::remove_invisible_text(&mut document, *id) {
            return failure(input, format!("Page {number}'s earlier OCR text couldn't be replaced: {error}"));
        }
    }
    let original = document.clone();
    let signature_removed = match crate::kit::pdf::remove_signatures(&mut document) {
        Ok(removed) => removed,
        Err(error) => return failure(input, error),
    };
    let layered = pages.iter().zip(frames).zip(layouts).filter_map(|((page, frame), layout)| page_ids.get(&page.number).map(|id| (page.number, *id, frame, layout))).collect::<Vec<_>>();
    let expected = match crate::kit::images::tools::bundled_text_font_bytes().map_err(ToolError::processing).and_then(|font| add_text_layer(&mut document, &layered, font)) {
        Ok(expected) => expected,
        Err(error) => return JobOutcome::failure(input, error),
    };
    if let Err(error) = document.save(output.path()) { return failure(input, format!("Could not write OCR output: {error}")); }
    #[cfg(test)]
    if BREAK_SEARCHABLE_OUTPUT_FOR.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_deref() == Some(input.as_path()) {
        let _ = std::fs::write(output.path(), b"%PDF-1.7\nnot a PDF any more\n");
    }
    if let Err(error) = validate_searchable_pdf(&original, output.path(), &expected) {
        return failure(input, format!("The searchable PDF didn't pass its check, so it wasn't saved: {error}"));
    }
    match output.publish() {
        Ok(path) => crate::kit::pdf::note_signature_removed(plan.searchable_details(success(input, path, &plan.searchable_detail(&language_names)), &language_names), signature_removed),
        Err(error) => failure(input, format!("Could not publish OCR output: {error}")),
    }
}

/// Asks the engine for word boxes as TSV through settings rather than its `tsv` config
/// file, because only the language data ships with the app, not the engine's configs.
const WORD_BOX_OUTPUT: &[&str] = &["-c", "tessedit_create_tsv=1", "-c", "tessedit_create_txt=0"];

/// Test seam: the searchable PDF written for this input is damaged after it is saved,
/// so tests can see reopen validation keep it from being published.
#[cfg(test)]
pub(crate) static BREAK_SEARCHABLE_OUTPUT_FOR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Renders the page as it is shown, its crop box turned by its /Rotate, so the word
/// boxes are in the same frame as the page a reader selects text on.
const DISPLAYED_PAGE: &[&str] = &["-cropbox"];

/// How a page is shown: its visible area in user space (the crop box, kept inside the
/// media box) and the clockwise quarter turns of its /Rotate.
#[derive(Debug, Clone, Copy, PartialEq)]
struct DisplayedPage { index: usize, bounds: [f32; 4], rotation: i64 }

impl DisplayedPage {
    /// The displayed width and height in user space: a quarter turn swaps the page's sides.
    fn size(&self) -> (f32, f32) {
        let [left, bottom, right, top] = self.bounds;
        if self.rotation % 180 == 90 { (top - bottom, right - left) } else { (right - left, top - bottom) }
    }

    /// The user-space box drawn at a rectangle given in fractions of the displayed page,
    /// measured from its displayed top left. Crop's rule maps it, so OCR and Crop agree
    /// on every rotation.
    fn user_box(&self, left: f32, top: f32, width: f32, height: f32) -> Result<[f32; 4], ToolError> {
        crate::kit::pdf::editor::crop_box_within(self.bounds, self.rotation, 1., self.index, &crate::kit::pdf::editor::CropSpec::Rectangle { left, top, width, height })
    }

    /// The text matrix's `a b c d` and the user-space point of a box's displayed bottom
    /// left corner, so text starts there, runs to the displayed right and stands upright.
    fn text_origin(&self, [left, bottom, right, top]: [f32; 4]) -> ([i8; 4], (f32, f32)) {
        match self.rotation {
            90 => ([0, 1, -1, 0], (right, bottom)),
            180 => ([-1, 0, 0, -1], (right, top)),
            270 => ([0, -1, 1, 0], (left, top)),
            _ => ([1, 0, 0, 1], (left, bottom)),
        }
    }
}

/// Each recognized page's displayed frame. A /Rotate that isn't a quarter turn fails
/// the file before the engine runs, with Crop's wording.
fn displayed_pages(document: &Document, page_ids: &std::collections::BTreeMap<u32, ObjectId>, pages: &[OcrPage]) -> Result<Vec<DisplayedPage>, String> {
    pages.iter().map(|page| {
        let id = *page_ids.get(&page.number).ok_or_else(|| "PDF page is missing.".to_string())?;
        let frame = DisplayedPage { index: page.number as usize - 1, bounds: effective_crop_bounds(document, id)?, rotation: page_rotation(document, id) };
        frame.user_box(0., 0., 1., 1.).map_err(|error| error.message)?;
        Ok(frame)
    }).collect()
}

#[derive(Debug, Clone, PartialEq)]
struct OcrWord { text: String, left: f32, width: f32 }

#[derive(Debug, Clone, PartialEq)]
struct OcrLine { top: f32, bottom: f32, words: Vec<OcrWord> }

/// Recognised words on one rendered page, in reading order, in raster pixels.
#[derive(Debug, Clone, PartialEq)]
struct OcrLayout { width: f32, height: f32, lines: Vec<OcrLine> }

/// Reads the engine's TSV: the page row gives the raster size, line rows give each
/// line's height, and word rows give each word and its box.
fn word_boxes(tsv: &[u8]) -> Result<OcrLayout, String> {
    let tsv = std::str::from_utf8(tsv).map_err(|_| "OCR helper produced invalid UTF-8 text.".to_string())?;
    let mut size = None;
    let mut lines: Vec<((u32, u32, u32), OcrLine)> = Vec::new();
    for row in tsv.lines().map(|row| row.trim_end_matches('\r')).filter(|row| !row.is_empty() && !row.starts_with("level")) {
        let fields = row.splitn(12, '\t').collect::<Vec<_>>();
        if fields.len() < 11 { return Err("OCR helper produced unreadable word boxes.".to_string()); }
        let number = |index: usize| fields[index].trim().parse::<f32>().map_err(|_| "OCR helper produced unreadable word boxes.".to_string());
        let key = |index: usize| fields[index].trim().parse::<u32>().unwrap_or(0);
        let (level, left, top, width, height) = (number(0)? as u32, number(6)?, number(7)?, number(8)?, number(9)?);
        let line_key = (key(2), key(3), key(4));
        match level {
            1 => size = Some((width, height)),
            4 => lines.push((line_key, OcrLine { top, bottom: top + height, words: Vec::new() })),
            5 => {
                let text = fields.get(11).map(|text| text.trim()).unwrap_or_default();
                if text.is_empty() || width <= 0. || height <= 0. { continue; }
                let word = OcrWord { text: text.to_string(), left, width };
                match lines.iter_mut().rev().find(|(key, _)| *key == line_key) {
                    Some((_, line)) => {
                        line.top = line.top.min(top);
                        line.bottom = line.bottom.max(top + height);
                        line.words.push(word);
                    }
                    None => lines.push((line_key, OcrLine { top, bottom: top + height, words: vec![word] })),
                }
            }
            _ => {}
        }
    }
    let lines = lines.into_iter().map(|(_, line)| line).filter(|line| !line.words.is_empty() && line.bottom > line.top).collect::<Vec<_>>();
    match size {
        Some((width, height)) if width > 0. && height > 0. => Ok(OcrLayout { width, height, lines }),
        _ if lines.is_empty() => Ok(OcrLayout { width: 1., height: 1., lines }),
        _ => Err("OCR helper reported words without a page size.".to_string()),
    }
}

/// What the text layer on one page must give back when the output is reopened.
struct ExpectedLayer { page: u32, words: Vec<String> }

/// Writes each page's words in invisible text (rendering mode 3) with an embedded,
/// subset Unicode font. A word starts at its box's left edge on its line's baseline and
/// is stretched to its box's width, so selecting it highlights the word in the scan.
/// The page's own content runs first inside q/Q, so its graphics state can't move the
/// text. Numbers have fixed precision, so the same words always give the same bytes.
fn add_text_layer(document: &mut Document, pages: &[(u32, ObjectId, DisplayedPage, OcrLayout)], font: Vec<u8>) -> Result<Vec<ExpectedLayer>, ToolError> {
    let mut font = UnicodeFont::parse(font)?;
    let (ascent, descent) = font.vertical_metrics();
    let mut layers = Vec::new();
    for (number, page_id, frame, layout) in pages {
        if layout.lines.is_empty() { continue; }
        // The raster is the displayed page, so a pixel scales to the displayed size.
        let (width, height) = frame.size();
        let (scale_x, scale_y) = (width / layout.width, height / layout.height);
        let name = layer_font_name(document, *page_id).map_err(ToolError::processing)?;
        let mut content = String::new();
        let mut words = Vec::new();
        for line in &layout.lines {
            let size = (line.bottom - line.top) * scale_y / (ascent - descent);
            // From the line's top down to its baseline, which sits above the box's bottom by the font's descent.
            let (top, baseline) = (line.top / layout.height, (line.bottom * scale_y + descent * size) / height);
            content.push_str(&format!("BT\n3 Tr\n/{name} {size:.2} Tf\n"));
            for word in &line.words {
                let natural = font.show_unshaped(&word.text, size)?.width;
                let run = font.show_unshaped(&format!("{} ", word.text), size)?;
                let stretch = if natural > 0. { 100. * word.width * scale_x / natural } else { 100. };
                let drawn = frame.user_box(word.left / layout.width, top, word.width.max(1.) / layout.width, baseline - top)?;
                let ([a, b, c, d], (x, y)) = frame.text_origin(drawn);
                content.push_str(&format!("{stretch:.2} Tz {a} {b} {c} {d} {x:.2} {y:.2} Tm {}\n", run.operators));
                words.push(word.text.clone());
            }
            content.push_str("ET\n");
        }
        layers.push((*page_id, name, content, ExpectedLayer { page: *number, words }));
    }
    let font_id = font.embed(document)?;
    for (page_id, name, content, _) in &layers {
        attach_text_layer(document, *page_id, name, font_id, content).map_err(ToolError::processing)?;
    }
    Ok(layers.into_iter().map(|(_, _, _, expected)| expected).collect())
}

/// The page's resources as the page sees them, inherited or its own.
fn page_resources(document: &Document, page_id: ObjectId) -> Result<(lopdf::Dictionary, lopdf::Dictionary), String> {
    let resources = text_layer::page_resources(document, page_id)?;
    let fonts = match resources.get(b"Font") {
        Ok(fonts) => crate::kit::pdf::resolve(document, fonts)?.as_dict().map_err(|_| "PDF page fonts are invalid.".to_string())?.clone(),
        Err(_) => lopdf::Dictionary::new(),
    };
    Ok((resources, fonts))
}

fn layer_font_name(document: &Document, page_id: ObjectId) -> Result<String, String> {
    let (_, fonts) = page_resources(document, page_id)?;
    (0..).map(|index| if index == 0 { OCR_LAYER_FONT.to_string() } else { format!("{OCR_LAYER_FONT}{index}") })
        .find(|name| !fonts.has(name.as_bytes()))
        .ok_or_else(|| "PDF page has no free font name.".to_string())
}

/// Gives the page its own copy of its resources with the layer font added, so
/// resources shared with other pages stay as they were, then wraps the original
/// content streams, untouched, between a q stream and the text layer.
fn attach_text_layer(document: &mut Document, page_id: ObjectId, name: &str, font_id: ObjectId, content: &str) -> Result<(), String> {
    let (mut resources, mut fonts) = page_resources(document, page_id)?;
    fonts.set(name.as_bytes().to_vec(), Object::Reference(font_id));
    resources.set("Font", fonts);
    let original = document.get_page_contents(page_id);
    let open = document.add_object(Stream::new(dictionary! {}, b"q\n".to_vec()));
    let mut layer = Stream::new(dictionary! {}, format!("\nQ\n{content}").into_bytes());
    layer.compress().map_err(|error| format!("Could not compress the OCR text layer: {error}"))?;
    let layer = document.add_object(layer);
    let contents = std::iter::once(open).chain(original).chain(std::iter::once(layer)).map(Object::Reference).collect::<Vec<_>>();
    let page = document.get_dictionary_mut(page_id).map_err(|error| error.to_string())?;
    page.set("Resources", resources);
    page.set("Contents", contents);
    Ok(())
}

/// Reopens the written PDF and checks it before anyone can see it: the same pages,
/// the same boxes, every original content stream byte for byte, and a text layer that
/// gives back the recognized words through text extraction.
fn validate_searchable_pdf(original: &Document, output: &Path, expected: &[ExpectedLayer]) -> Result<(), String> {
    let reopened = Document::load(output).map_err(|error| format!("it could not be reopened ({error})."))?;
    let (before, after) = (original.get_pages(), reopened.get_pages());
    if before.len() != after.len() { return Err("its page count changed.".to_string()); }
    let streams = |document: &Document, page: ObjectId| document.get_page_contents(page).into_iter()
        .map(|id| document.get_object(id).and_then(Object::as_stream).map(|stream| stream.content.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("its content could not be read ({error})."));
    for ((number, old), new) in before.iter().zip(after.values()) {
        if page_bounds(original, *old)? != page_bounds(&reopened, *new)? || effective_crop_bounds(original, *old)? != effective_crop_bounds(&reopened, *new)? {
            return Err(format!("page {number}'s boxes changed."));
        }
        let (old_streams, new_streams) = (streams(original, *old)?, streams(&reopened, *new)?);
        if !old_streams.is_empty() && !new_streams.windows(old_streams.len()).any(|window| window == old_streams.as_slice()) {
            return Err(format!("page {number}'s original content changed."));
        }
    }
    for layer in expected {
        let text = reopened.extract_text(&[layer.page]).map_err(|error| format!("page {}'s text could not be read ({error}).", layer.page))?;
        if !text.split_whitespace().collect::<Vec<_>>().join(" ").contains(&layer.words.join(" ")) {
            return Err(format!("page {}'s text layer doesn't give back the recognized words.", layer.page));
        }
    }
    Ok(())
}

fn render_ocr_page(renderer: &Path, input: &Path, workspace: &OcrWorkspace, page: OcrPage, area: &[&str], deadline: Instant, timed_out: &str) -> Result<PathBuf, ToolError> {
    let prefix = workspace.page_prefix(page.number);
    let output = prefix.with_extension("png");
    let mut command = Command::new(renderer);
    command.arg("-png").arg("-r").arg(OCR_RENDER_DPI.to_string()).arg("-f").arg(page.number.to_string()).arg("-l").arg(page.number.to_string()).arg("-singlefile").args(area).arg(input).arg(&prefix);
    let result = run_ocr_helper(command, deadline, timed_out)?;
    if result.status.success() && output.is_file() {
        let size = std::fs::metadata(&output).map_err(|error| ToolError::processing(format!("Could not inspect rendered OCR page: {error}")))?.len();
        if size > OCR_MAX_RENDERED_BYTES {
            return Err(ToolError::limit_exceeded(LimitKind::Decompression, "Rendered OCR page exceeds the 50 MiB file-size limit."));
        }
        Ok(output)
    } else {
        Err(ToolError::processing(stderr(result, "PDF renderer failed to render the selected OCR page.")))
    }
}

fn run_ocr_helper(command: Command, deadline: Instant, timed_out: &str) -> Result<std::process::Output, ToolError> {
    match crate::kit::pdf::run_bounded_helper(command, deadline, OCR_MAX_OUTPUT_BYTES, crate::kit::pdf::PDF_TEXT_MAX_ERROR_BYTES, timed_out) {
        Ok(output) => Ok(output),
        Err(message) if message == timed_out => Err(ToolError::limit_exceeded(LimitKind::Time, message)),
        Err(message) => Err(ToolError::processing(message)),
    }
}

fn find_pdf_renderer() -> Result<PathBuf, String> {
    find_pdf_renderer_with_override(std::env::var_os("TOOLBOX_PDFTOPPM_PATH").map(PathBuf::from), || {
        let root = crate::kit::resources::application_resource_root().unwrap_or_default();
        let executable = if cfg!(windows) { "pdftoppm.exe" } else { "pdftoppm" };
        [root.join("pdf-bin").join(executable), root.join("resources").join(executable), root.join(executable)]
            .into_iter()
            .find(|path| path.is_file())
            .or_else(|| Command::new(executable).arg("-h").output().ok().filter(|result| result.status.success()).map(|_| PathBuf::from(executable)))
            .ok_or_else(|| "PDF rasterizer is unavailable. Bundle pdftoppm or set TOOLBOX_PDFTOPPM_PATH.".to_string())
    })
}

fn find_pdf_renderer_with_override<F>(override_path: Option<PathBuf>, fallback: F) -> Result<PathBuf, String>
where
    F: FnOnce() -> Result<PathBuf, String>,
{
    if let Some(path) = override_path.filter(|path| !path.as_os_str().is_empty()) {
        if path.is_file() { return Ok(path); }
        return Err("TOOLBOX_PDFTOPPM_PATH does not point to a file.".to_string());
    }
    fallback()
}

fn normalize_ocr_text(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "OCR helper produced invalid UTF-8 text.".to_string())?
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(text)
}

pub fn blur_faces(request: &VisionRequest, input: PathBuf) -> JobOutcome {
    run_image_adapter(request, input, "face-blur", "faceBlur", "faceBlurModel", "TOOLBOX_FACE_BLUR_PATH", "TOOLBOX_FACE_BLUR_MODEL_PATH", "toolbox-face-blur", "-blurred", "Blurred faces", ImageAdapterValidation::FaceBlur)
}

pub fn remove_background(request: &VisionRequest, input: PathBuf) -> JobOutcome {
    run_image_adapter(request, input, "background-removal", "backgroundRemoval", "backgroundRemovalModel", "TOOLBOX_BACKGROUND_REMOVAL_PATH", "TOOLBOX_BACKGROUND_REMOVAL_MODEL_PATH", "toolbox-background-removal", "-cutout", "Background removed", ImageAdapterValidation::Cutout)
}

enum ImageAdapterValidation { FaceBlur, Cutout }

fn run_image_adapter(request: &VisionRequest, input: PathBuf, mode: &str, resource_name: &str, model_name: &str, variable: &str, model_variable: &str, command: &str, suffix: &str, detail: &str, validation: ImageAdapterValidation) -> JobOutcome {
    let output = match OutputNaming::reserve_destination(&input, &request.output_location, suffix, "png") {
        Ok(output) => output,
        Err(error) => return failure(input, format!("Could not reserve vision output: {error}")),
    };
    let engine = match find_engine(resource_name, variable, command) {
        Ok(engine) => engine,
        Err(error) => return unavailable(input, error),
    };
    let model = match find_model(model_name, model_variable) {
        Ok(model) => model,
        Err(error) => return unavailable(input, error),
    };
    match Command::new(engine).arg("--mode").arg(mode).arg("--input").arg(&input).arg("--output").arg(output.path()).arg("--model").arg(model).output_until_stopped() {
        Ok(result) if result.status.success() && output.path().is_file() => {
            let validation = match validation {
                ImageAdapterValidation::FaceBlur => validate_face_blur(&input, output.path()),
                ImageAdapterValidation::Cutout => validate_cutout(output.path()),
            };
            match validation {
                Ok(()) => match output.publish() {
                    Ok(path) => success(input, path, detail),
                    Err(error) => failure(input, format!("Could not publish vision output: {error}")),
                },
                Err(error) => failure(input, error),
            }
        }
        Ok(result) if result.status.success() => {
            failure(input, "Vision adapter reported success but produced no output.".to_string())
        }
        Ok(result) => failure(input, stderr(result, "Vision adapter failed.")),
        Err(error) => failure(input, format!("Could not run vision adapter: {error}")),
    }
}

fn find_engine(name: &str, variable: &str, command: &str) -> Result<PathBuf, String> {
    resources::resolve(name, &vision_resource_root(), variable, command).map(|resource| resource.path)
}

fn find_model(name: &str, variable: &str) -> Result<PathBuf, String> {
    let root = vision_resource_root();
    resources::resolve_bundled_or_override(name, &root, variable)
        .and_then(|resource| resource.map(|resource| resource.path).ok_or_else(|| format!("Resource {name} is unavailable.")))
}

fn find_resource(name: &str, variable: &str, root: &std::path::Path) -> Result<PathBuf, String> {
    resources::resolve_bundled_or_override(name, root, variable)
        .and_then(|resource| resource.map(|resource| resource.path).ok_or_else(|| format!("Resource {name} is unavailable.")))
}

fn vision_resource_root() -> std::path::PathBuf {
    let root = resources::application_resource_root().unwrap_or_default();
    let direct = root.join("vision");
    if direct.is_dir() || !root.join("resources").join("vision").is_dir() { direct } else { root.join("resources").join("vision") }
}

fn ocr_resource_root() -> std::path::PathBuf {
    let root = resources::application_resource_root().unwrap_or_default();
    let direct = root.join("ocr");
    if direct.is_dir() || !root.join("resources").join("ocr").is_dir() { direct } else { root.join("resources").join("ocr") }
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

fn success(input_path: PathBuf, output: PathBuf, detail: &str) -> JobOutcome { JobOutcome::succeeded(input_path, vec![output], detail.to_string()) }
fn failure(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::processing(error)) }
fn unavailable(input_path: PathBuf, error: String) -> JobOutcome { JobOutcome::failure(input_path, ToolError::unavailable(error)) }
fn too_large(noun: &str) -> ToolError { ToolError::limit_exceeded(LimitKind::FileSize, format!("{noun} exceeds the 100 MiB OCR input limit.")) }
fn text_too_long() -> ToolError { ToolError::limit_exceeded(LimitKind::FileSize, "OCR output exceeds the 10 MiB text limit.") }
fn stderr(result: std::process::Output, fallback: &str) -> String { String::from_utf8_lossy(&result.stderr).trim().lines().last().filter(|line| !line.is_empty()).unwrap_or(fallback).to_string() }

#[cfg(test)]
mod tests {
    use super::{normalize_ocr_text, ocr_pdf, ocr_pdf_within, validate_cutout, validate_face_blur, OcrRequest, OcrRun, OCR_DEADLINE};
    use crate::kit::common::limits::HelperDeadline;
    use crate::packs::uninstall::RunPacks;
    use crate::kit::common::OutputLocation;
    use crate::kit::contracts::{ErrorKind, LimitKind, OutcomePointer, PackNeed};
    use image::{Rgba, RgbaImage};
    use lopdf::{dictionary, Document, Object, Stream};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;


    fn path(name: &str) -> PathBuf { std::env::temp_dir().join(format!("toolbox_vision_{}_{}", std::process::id(), name)) }

    fn ocr_request(input: &Path, page_range: Option<&str>, output_root: &Path) -> OcrRequest {
        OcrRequest {
            paths: vec![input.to_path_buf()],
            page_range: page_range.map(str::to_string),
            mode: "text".to_string(),
            force: false,
            languages: vec!["eng".to_string()],
            output_location: OutputLocation::CustomFolder(output_root.to_path_buf()),
            language_overrides: Default::default(),
        }
    }

    /// The boundary check with no packs to find, so English is the only language it knows.
    fn validate_ocr_request(request: &OcrRequest) -> Result<(), Vec<crate::kit::common::JobOutcome>> {
        super::validate_ocr_request(request, &RunPacks::new(Err("No packs in this test.".into()), "1.0.14", Err("No catalog in this test.".into())))
    }

    /// A run of `request` with no packs to find, so it reads English only.
    fn run_of(request: OcrRequest) -> OcrRun {
        OcrRun { request, packs: RunPacks::new(Err("No packs in this test.".into()), "1.0.14", Err("No catalog in this test.".into())) }
    }

    #[cfg(unix)]
    fn executable(path: &Path, script: &str) {
        fs::write(path, script).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(unix)]
    /// Sets environment variables for one test and puts the old values back when dropped.
    struct Env(Vec<(&'static str, Option<std::ffi::OsString>)>);

    #[cfg(unix)]
    impl Env {
        fn set(values: &[(&'static str, Option<&Path>)]) -> Self {
            let old = values.iter().map(|(name, _)| (*name, std::env::var_os(name))).collect();
            for (name, value) in values {
                match value { Some(value) => std::env::set_var(name, value), None => std::env::remove_var(name) }
            }
            Self(old)
        }
    }

    #[cfg(unix)]
    impl Drop for Env {
        fn drop(&mut self) {
            for (name, value) in &self.0 {
                match value { Some(value) => std::env::set_var(name, value), None => std::env::remove_var(name) }
            }
        }
    }

    #[cfg(unix)]
    struct MixedBatch {
        root: PathBuf,
        english: PathBuf,
        hindi: PathBuf,
        output: PathBuf,
    }

    #[cfg(unix)]
    impl MixedBatch {
        /// One English and one Hindi PDF, an engine that prints the languages it was asked for and
        /// each language's file in that order, all from the one folder it is given, and a renderer
        /// that needs no PDF tools.
        fn new(name: &str) -> Self {
            let root = crate::packs::install::tests::sandbox(&format!("ocr-{name}"));
            let batch = Self { english: root.join("english.pdf"), hindi: root.join("hindi.pdf"), output: root.join("output"), root };
            make_pdf(&batch.english, 1);
            make_pdf(&batch.hindi, 1);
            fs::create_dir_all(&batch.output).unwrap();
            fs::create_dir_all(batch.root.join("eng")).unwrap();
            fs::write(batch.root.join("eng/eng.traineddata"), "english data").unwrap();
            executable(&batch.root.join("engine.sh"), "#!/bin/sh\ndata=''\nlanguage=''\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    --tessdata-dir) data=\"$2\"; shift ;;\n    -l) language=\"$2\"; shift ;;\n  esac\n  shift\ndone\nprintf '%s:' \"$language\"\nfor code in $(printf '%s' \"$language\" | tr '+' ' '); do\n  printf ' '\n  cat \"$data/$code.traineddata\" || exit 1\ndone\nprintf '\\n'\n");
            executable(&batch.root.join("renderer.sh"), "#!/bin/sh\n: > \"${10}.png\"\n");
            batch
        }

        /// Runs the English file in English and the Hindi file in Hindi as one batch, and checks
        /// the originals are untouched. The outcomes come back in input order.
        fn run(&self, app_data: &Path, app_version: &str, language_dir: Option<&Path>) -> Vec<crate::kit::common::JobOutcome> {
            self.run_reading(app_data, app_version, language_dir, &["hin"])
        }

        /// Runs the batch with the Hindi file read in `hindi_languages`, in that order.
        fn run_reading(&self, app_data: &Path, app_version: &str, language_dir: Option<&Path>, hindi_languages: &[&str]) -> Vec<crate::kit::common::JobOutcome> {
            let originals = [fs::read(&self.english).unwrap(), fs::read(&self.hindi).unwrap()];
            let outcomes = {
                let _guard = crate::kit::lock_process_env();
                let _env = Env::set(&[
                    ("TOOLBOX_TESSERACT_PATH", Some(&self.root.join("engine.sh"))),
                    ("TOOLBOX_PDFTOPPM_PATH", Some(&self.root.join("renderer.sh"))),
                    ("TOOLBOX_TESSDATA_PATH", Some(&self.root.join("eng/eng.traineddata"))),
                    (super::OCR_LANGUAGE_DIR_ENV, language_dir),
                ]);
                let request = OcrRequest {
                    paths: vec![self.english.clone(), self.hindi.clone()],
                    page_range: None,
                    mode: "text".into(),
                    force: false,
                    languages: vec!["eng".into()],
                    output_location: OutputLocation::CustomFolder(self.output.clone()),
                    language_overrides: [(self.hindi.clone(), hindi_languages.iter().map(|code| code.to_string()).collect())].into_iter().collect(),
                };
                let packs = RunPacks::new(Ok(app_data.to_path_buf()), app_version, Ok(crate::packs::install::tests::Fixture::new().catalog()));
                // One run for the whole batch, each file through the job the batch runner calls. The
                // runner itself refuses ocr_pdf while the tool is unavailable in this build.
                let run = OcrRun { request, packs };
                run.request.paths.iter().map(|input| ocr_pdf(&run, input.clone())).collect::<Vec<_>>()
            };
            assert_eq!([fs::read(&self.english).unwrap(), fs::read(&self.hindi).unwrap()], originals, "originals are never modified");
            assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), [self.english.clone(), self.hindi.clone()]);
            outcomes
        }

        fn assert_english_read(&self, outcome: &crate::kit::common::JobOutcome) {
            assert!(outcome.error().is_none(), "{outcome:?}");
            let text = fs::read_to_string(&outcome.output_paths()[0]).unwrap();
            assert_eq!(text, "eng: english data\n");
        }

        /// The Hindi file failed with the pointer and saved nothing.
        fn assert_needs_pack(&self, outcome: &crate::kit::common::JobOutcome, reason: PackNeed, message: &str) {
            let error = outcome.error().unwrap_or_else(|| panic!("{outcome:?}"));
            assert_eq!(error.kind, ErrorKind::Unavailable);
            assert_eq!(error.pointer, Some(OutcomePointer::Pack { pack_id: "ocr-hindi".into(), name: "Hindi".into(), reason }));
            assert_eq!(error.message, message);
            assert!(outcome.output_paths().is_empty());
            let saved: Vec<_> = fs::read_dir(&self.output).unwrap().flatten().map(|entry| entry.file_name()).collect();
            assert!(saved.iter().all(|name| name.to_string_lossy().starts_with("english")), "nothing was saved for the Hindi file: {saved:?}");
        }
    }

    #[cfg(unix)]
    /// Installs the fixture Hindi pack into a fresh app-data folder.
    fn install_hindi(root: &Path) -> PathBuf {
        use crate::packs::install::{install, tests::{Fixture, TestKey}, InstallContext, PackLimits};
        let (fixture, key) = (Fixture::new(), TestKey::new(1));
        let file = root.join("hindi.toolboxpack");
        fs::write(&file, fixture.file(&key)).unwrap();
        let app_data = root.join("app-data");
        let target = crate::packs::current_target();
        install(&file, &InstallContext { catalog: &fixture.catalog(), app_data: &app_data, target: &target, app_version: "1.0.14", public_key: &key.public_key(), limits: PackLimits::DEFAULT, cancel: None, changed: None }).unwrap();
        app_data
    }

    #[cfg(unix)]
    #[test]
    fn a_language_whose_pack_is_not_installed_fails_with_a_pointer_while_english_in_the_same_batch_succeeds() {
        let batch = MixedBatch::new("not-installed");
        let outcomes = batch.run(&batch.root.join("app-data"), "1.0.14", None);
        batch.assert_english_read(&outcomes[0]);
        batch.assert_needs_pack(&outcomes[1], PackNeed::NotInstalled, "Needs the Hindi text recognition pack, which isn't installed. Nothing was saved.");
    }

    #[cfg(unix)]
    #[test]
    fn a_language_list_with_one_pack_not_installed_fails_that_file_with_the_pack_named_while_the_others_succeed() {
        let batch = MixedBatch::new("list-not-installed");
        let outcomes = batch.run_reading(&batch.root.join("app-data"), "1.0.14", None, &["eng", "hin"]);
        batch.assert_english_read(&outcomes[0]);
        batch.assert_needs_pack(&outcomes[1], PackNeed::NotInstalled, "Needs the Hindi text recognition pack, which isn't installed. Nothing was saved.");
    }

    #[cfg(unix)]
    #[test]
    fn english_and_an_override_language_reach_the_engine_in_the_order_chosen_and_repeat_byte_for_byte() {
        let batch = MixedBatch::new("two-languages");
        let language_dir = batch.root.join("languages");
        fs::create_dir_all(&language_dir).unwrap();
        fs::write(language_dir.join("hin.traineddata"), "fixture hindi").unwrap();
        let app_data = batch.root.join("app-data");
        // English comes from the app and Hindi from the override folder, so both are gathered
        // into the one folder the engine reads.
        for (order, expected, detail) in [
            (["eng", "hin"], "eng+hin: english data fixture hindi\n", "1 page recognized. English and Hindi."),
            (["hin", "eng"], "hin+eng: fixture hindi english data\n", "1 page recognized. Hindi and English."),
        ] {
            let first = batch.run_reading(&app_data, "1.0.14", Some(&language_dir), &order);
            let second = batch.run_reading(&app_data, "1.0.14", Some(&language_dir), &order);
            for outcomes in [&first, &second] {
                batch.assert_english_read(&outcomes[0]);
                assert!(outcomes[1].error().is_none(), "{:?}", outcomes[1]);
                assert_eq!(fs::read_to_string(&outcomes[1].output_paths()[0]).unwrap(), expected);
                assert_eq!(outcomes[1].detail, detail);
            }
            assert_ne!(first[1].output_paths(), second[1].output_paths(), "a second run never overwrites the first");
            assert_eq!(fs::read(&first[1].output_paths()[0]).unwrap(), fs::read(&second[1].output_paths()[0]).unwrap(), "the same list gives the same output");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_development_override_stands_in_for_the_language_pack_and_its_language_is_read() {
        let batch = MixedBatch::new("override");
        let language_dir = batch.root.join("languages");
        fs::create_dir_all(&language_dir).unwrap();
        fs::write(language_dir.join("hin.traineddata"), "fixture hindi").unwrap();
        let outcomes = batch.run(&batch.root.join("app-data"), "1.0.14", Some(&language_dir));
        batch.assert_english_read(&outcomes[0]);
        assert!(outcomes[1].error().is_none(), "{:?}", outcomes[1]);
        assert_eq!(fs::read_to_string(&outcomes[1].output_paths()[0]).unwrap(), "hin: fixture hindi\n");
    }

    #[cfg(unix)]
    #[test]
    fn an_installed_language_pack_is_read_and_a_damaged_or_incompatible_one_names_its_own_reason() {
        let batch = MixedBatch::new("installed");
        let app_data = install_hindi(&batch.root);
        let outcomes = batch.run(&app_data, "1.0.14", None);
        assert_eq!(fs::read_to_string(&outcomes[1].output_paths()[0]).unwrap(), format!("hin: {}\n", "hindi model".repeat(100)));

        // The fixture pack fits apps from 1.0.0. A development override does not stand in for it.
        let language_dir = batch.root.join("languages");
        fs::create_dir_all(&language_dir).unwrap();
        fs::write(language_dir.join("hin.traineddata"), "fixture hindi").unwrap();
        let incompatible = MixedBatch::new("incompatible");
        let outcomes = incompatible.run(&install_hindi(&incompatible.root), "0.9.0", Some(&language_dir));
        incompatible.assert_english_read(&outcomes[0]);
        incompatible.assert_needs_pack(&outcomes[1], PackNeed::Incompatible, "Needs a newer Hindi pack. The installed one doesn't work with this version of Toolbox. Nothing was saved.");

        let damaged = MixedBatch::new("damaged");
        let app_data = install_hindi(&damaged.root);
        let model = crate::packs::installed_pack_root(&app_data, "ocr-hindi").unwrap().join("tessdata/hin.traineddata");
        fs::write(&model, "changed after install").unwrap();
        let outcomes = damaged.run(&app_data, "1.0.14", Some(&language_dir));
        damaged.assert_english_read(&outcomes[0]);
        damaged.assert_needs_pack(&outcomes[1], PackNeed::Damaged, "The Hindi pack is damaged, so it was not used. Nothing was saved.");
    }

    #[test]
    fn a_pack_pointer_names_the_catalog_pack_for_its_language_and_a_language_no_pack_reads_has_none() {
        // The development override is read from the environment.
        let _guard = crate::kit::lock_process_env();
        let root = std::env::temp_dir().join(format!("toolbox-ocr-pointer-{}", std::process::id()));
        let catalogs = [
            ("built-in", include_str!("../../../shared/pack-catalog.json").to_string()),
            ("fixture", fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/pack-catalog.json")).unwrap()),
        ];
        for (name, text) in catalogs {
            let catalog = crate::packs::parse_catalog(&text).unwrap();
            let packs = RunPacks::new(Ok(root.join(name)), "1.0.14", Ok(catalog.clone()));
            for pack in &catalog.packs {
                let Some(language) = &pack.ocr_language else { continue };
                let error = super::ocr_language(&packs, &language.code).unwrap_err();
                assert_eq!(error.kind, ErrorKind::Unavailable, "{name} {}", pack.id);
                assert_eq!(error.pointer, Some(OutcomePointer::Pack { pack_id: pack.id.clone(), name: language.name.clone(), reason: PackNeed::NotInstalled }), "{name} {}", pack.id);
            }
            let error = super::ocr_language(&packs, "xyz").unwrap_err();
            assert_eq!(error.kind, ErrorKind::InvalidInput);
            assert_eq!(error.pointer, None, "a language no {name} pack reads points nowhere");
        }
        assert!(!root.exists(), "looking for a pack creates nothing");
    }

    /// Scanned pages: something is drawn, but no text.
    fn make_pdf(path: &PathBuf, page_count: usize) {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let mut kids = Vec::new();
        for number in 0..page_count {
            let content_id = document.add_object(Stream::new(dictionary! {}, format!("q 0 0 0 rg 72 720 {} 10 re f Q", number + 1).into_bytes()));
            let page_id = document.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } }, "Contents" => content_id,
            });
            kids.push(Object::Reference(page_id));
        }
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => page_count as i64 }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(path).unwrap();
    }

    #[cfg(unix)]
    fn fake_ocr_engine(fail_on_page_two: bool) -> PathBuf {
        let engine = path("fake-ocr.sh");
        let page_two = if fail_on_page_two { "  *page-2.png) printf 'OCR failure\\n' >&2; exit 1 ;;" } else { "  *page-2.png) printf 'page two\\n' ;;" };
        fs::write(&engine, format!("#!/bin/sh\ncase \"$1\" in\n  *page-1.png) printf 'selected page one\\n' ;;\n{page_two}\n  *.pdf) printf 'selected page one\\npage two\\n' ;;\n  *) exit 2 ;;\nesac\n")).unwrap();
        let mut permissions = fs::metadata(&engine).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&engine, permissions).unwrap();
        engine
    }

    #[test]
    fn selected_ocr_pages_preserve_document_order_and_clamp_the_range_to_the_file() {
        let _guard = crate::kit::lock_process_env();
        let input = path("selected-pages-scope.pdf");
        make_pdf(&input, 2);

        let document = Document::load(&input).unwrap();
        let selected = |range| super::PagePlan::of(&document, Some(range), false).map(|plan| plan.recognized().iter().map(|page| page.number).collect::<Vec<_>>());
        assert!(matches!(selected("2, 1"), Ok(pages) if pages == vec![1, 2]));
        assert!(matches!(selected("2-"), Ok(pages) if pages == vec![2]));
        assert!(matches!(selected("2-9"), Ok(pages) if pages == vec![2]), "a range past the end is shortened");
        assert!(matches!(selected("3, 5-"), Err(super::NoPlan::Skipped(reason)) if reason.code == crate::kit::contracts::SkipCode::RangeEmptyAfterClamping && reason.message == "No pages left in 3, 5-. It has 2 pages, so nothing was made."));

        let _ = fs::remove_file(input);
    }

    #[test]
    fn selected_ocr_pages_reject_excessive_render_pixels_before_starting_helpers() {
        let _guard = crate::kit::lock_process_env();
        let input = path("oversized-page.pdf");
        make_pdf(&input, 1);
        let mut document = Document::load(&input).unwrap();
        let page = document.get_pages().values().next().copied().unwrap();
        let root = document.get_dictionary(page).unwrap().get(b"Parent").unwrap().as_reference().unwrap();
        document.get_dictionary_mut(page).unwrap().remove(b"MediaBox");
        document.get_dictionary_mut(root).unwrap().set("MediaBox", vec![0.into(), 0.into(), 14400.into(), 14400.into()]);
        document.save(&input).unwrap();

        let document = Document::load(&input).unwrap();
        let error = super::check_render_limits(&document, &super::PagePlan::of(&document, None, false).unwrap().recognized()).unwrap_err();
        assert_eq!(error.kind, ErrorKind::LimitExceeded { limit: LimitKind::Decompression });
        assert_eq!(error.message, "Selected PDF page exceeds the 40 million pixel OCR limit.");
        let _ = fs::remove_file(input);
    }

    #[cfg(unix)]
    #[test]
    fn oversized_rendered_page_is_rejected_before_ocr_runs() {
        // Holds the lock because sibling tests count private OCR workspaces.
        let _guard = crate::kit::lock_process_env();
        let renderer = path("oversized-renderer.sh");
        let engine = path("oversized-engine.sh");
        let input = path("oversized-render.pdf");
        make_pdf(&input, 1);
        fs::write(&renderer, format!("#!/bin/sh\ndd if=/dev/zero of=\"${{10}}.png\" bs=1 count=0 seek={} 2>/dev/null\n", super::OCR_MAX_RENDERED_BYTES + 1)).unwrap();
        fs::write(&engine, "#!/bin/sh\nprintf 'ocr should not run\\n'\n").unwrap();
        for helper in [&renderer, &engine] {
            let mut permissions = fs::metadata(helper).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(helper, permissions).unwrap();
        }
        let workspace = super::OcrWorkspace::new().unwrap();
        let result = super::render_ocr_page(&renderer, &input, &workspace, super::OcrPage { number: 1 }, &[], Instant::now() + Duration::from_secs(5), "timed out");
        let error = result.unwrap_err();
        assert_eq!(error.kind, ErrorKind::LimitExceeded { limit: LimitKind::Decompression });
        assert_eq!(error.message, "Rendered OCR page exceeds the 50 MiB file-size limit.");

        drop(workspace);
        let _ = fs::remove_file(renderer);
        let _ = fs::remove_file(engine);
        let _ = fs::remove_file(input);
    }

    #[cfg(unix)]
    #[test]
    fn selected_ocr_page_cannot_process_an_excluded_page() {
        let _guard = crate::kit::lock_process_env();
        let input = path("selected-pages.pdf");
        let output_root = path("selected-pages-output");
        make_pdf(&input, 2);
        let original = fs::read(&input).unwrap();
        fs::create_dir_all(&output_root).unwrap();
        let engine = fake_ocr_engine(false);
        let tessdata = path("selected-eng.traineddata");
        fs::write(&tessdata, b"fake tessdata").unwrap();
        let old_engine = std::env::var_os("TOOLBOX_TESSERACT_PATH");
        let old_tessdata = std::env::var_os("TOOLBOX_TESSDATA_PATH");
        std::env::set_var("TOOLBOX_TESSERACT_PATH", &engine);
        std::env::set_var("TOOLBOX_TESSDATA_PATH", &tessdata);

        let outcome = ocr_pdf(&run_of(ocr_request(&input, Some("1"), &output_root)), input.clone());
        let text = outcome.output_paths().first().and_then(|path| fs::read_to_string(path).ok());
        let output_path = outcome.output_paths().first().cloned();
        let input_after = fs::read(&input).unwrap();

        match old_engine {
            Some(value) => std::env::set_var("TOOLBOX_TESSERACT_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSERACT_PATH"),
        }
        match old_tessdata {
            Some(value) => std::env::set_var("TOOLBOX_TESSDATA_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSDATA_PATH"),
        }
        let _ = fs::remove_file(&engine);
        let _ = fs::remove_file(&tessdata);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_dir_all(&output_root);

        assert!(outcome.error().is_none(), "selected OCR failed: {:?}", outcome.error());
        assert_eq!(text.as_deref(), Some("selected page one\n"));
        assert_eq!(output_path.as_deref().and_then(Path::parent), Some(output_root.as_path()));
        assert_eq!(input_after, original);
    }

    #[cfg(unix)]
    #[test]
    fn failed_selected_ocr_cleans_private_workspace_and_reserved_output() {
        let _guard = crate::kit::lock_process_env();
        let input = path("failed-selected-pages.pdf");
        let output_root = path("failed-selected-pages-output");
        make_pdf(&input, 2);
        let original = fs::read(&input).unwrap();
        fs::create_dir_all(&output_root).unwrap();
        let engine = fake_ocr_engine(true);
        let tessdata = path("failed-eng.traineddata");
        fs::write(&tessdata, b"fake tessdata").unwrap();
        let before_workspaces = ocr_workspaces();
        let old_engine = std::env::var_os("TOOLBOX_TESSERACT_PATH");
        let old_tessdata = std::env::var_os("TOOLBOX_TESSDATA_PATH");
        std::env::set_var("TOOLBOX_TESSERACT_PATH", &engine);
        std::env::set_var("TOOLBOX_TESSDATA_PATH", &tessdata);

        let outcome = ocr_pdf(&run_of(ocr_request(&input, Some("2"), &output_root)), input.clone());
        let after_workspaces = ocr_workspaces();
        let input_after = fs::read(&input).unwrap();
        let output_entries = fs::read_dir(&output_root).unwrap().count();

        match old_engine {
            Some(value) => std::env::set_var("TOOLBOX_TESSERACT_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSERACT_PATH"),
        }
        match old_tessdata {
            Some(value) => std::env::set_var("TOOLBOX_TESSDATA_PATH", value),
            None => std::env::remove_var("TOOLBOX_TESSDATA_PATH"),
        }
        let _ = fs::remove_file(&engine);
        let _ = fs::remove_file(&tessdata);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_dir_all(&output_root);

        assert!(outcome.error().is_some_and(|error| matches!(error.kind, crate::kit::contracts::ErrorKind::Processing)));
        assert!(outcome.output_paths().is_empty());
        assert_eq!(before_workspaces, after_workspaces);
        assert_eq!(original, input_after);
        assert_eq!(output_entries, 0);
    }

    #[cfg(unix)]
    #[test]
    fn ocr_workspace_is_private_and_removed_on_drop() {
        let _guard = crate::kit::lock_process_env();
        let workspace = super::OcrWorkspace::new().unwrap();
        let path = workspace.0.clone();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o700);
        fs::write(path.join("secret.png"), b"private OCR input").unwrap();
        drop(workspace);
        assert!(!path.exists());
    }

    #[test]
    fn empty_pdf_renderer_override_uses_path_fallback() {
        let result = super::find_pdf_renderer_with_override(Some(PathBuf::new()), || Ok(PathBuf::from("pdftoppm")));
        assert_eq!(result.unwrap(), PathBuf::from("pdftoppm"));
    }

    #[cfg(unix)]
    fn ocr_workspaces() -> Vec<PathBuf> {
        let prefix = format!("toolbox_ocr_{}", std::process::id());
        let mut paths = fs::read_dir(std::env::temp_dir()).unwrap().filter_map(Result::ok).map(|entry| entry.path()).filter(|path| {
            path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(&prefix))
        }).collect::<Vec<_>>();
        paths.sort();
        paths
    }

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
        assert_eq!(normalize_ocr_text(b"  first   line\r\n\n second line  \n").unwrap(), "first line\nsecond line");
    }

    #[test]
    fn ocr_text_ends_lines_in_lf_and_separates_pages_with_a_form_feed() {
        let pages = [b"first  page\r\nline two\r\n".to_vec(), b" \n\n".to_vec(), b"third\n".to_vec()];
        assert_eq!(super::ocr_text(&pages).unwrap(), "first page\nline two\n\u{c}\u{c}third\n");
        assert_eq!(super::ocr_text(&[b"\n".to_vec(), b"  \x0c".to_vec()]).unwrap(), "");
    }

    #[test]
    fn page_lists_name_runs_of_pages_the_way_the_results_read() {
        assert_eq!(super::page_list(&[8]), "Page 8");
        assert_eq!(super::page_list(&[1, 2, 3, 4, 9]), "Pages 1-4 and 9");
        assert_eq!(super::page_list(&[1, 3, 4, 5, 9]), "Pages 1, 3-5 and 9");
        assert_eq!(super::page_list(&[5, 6, 7]), "Pages 5-7");
        assert_eq!(super::page_list(&[2, 3]), "Pages 2 and 3");
        assert_eq!(super::page_list(&[1, 2, 5, 6]), "Pages 1, 2, 5 and 6");
    }

    #[test]
    fn missing_ocr_components_name_themselves() {
        assert_eq!(super::OcrComponent::Engine.to_string(), "The text recognition engine is missing. Reinstall Toolbox.");
        assert_eq!(super::OcrComponent::Renderer.to_string(), "The PDF page renderer is missing. Reinstall Toolbox.");
        assert_eq!(super::OcrComponent::EnglishData.to_string(), "The English recognition data is missing. Reinstall Toolbox.");
        assert_eq!(super::OcrComponent::TextReader.to_string(), "The PDF text reader is missing. Reinstall Toolbox.");
    }

    #[test]
    fn rejects_invalid_utf8_ocr_output() {
        assert!(normalize_ocr_text(b"valid\xfftext").is_err());
    }

    #[test]
    fn ocr_deadline_is_thirty_seconds_per_file_plus_thirty_seconds_per_page() {
        assert_eq!(OCR_DEADLINE, HelperDeadline { per_file: Duration::from_secs(30), per_page: Duration::from_secs(30) });
        assert_eq!(OCR_DEADLINE.limit(1, 1), Duration::from_secs(60));
        assert_eq!(OCR_DEADLINE.limit(1, 3), Duration::from_secs(120));
        assert_eq!(super::OcrClock::start(Path::new("scan.pdf"), OCR_DEADLINE, 3).page_limit, Duration::from_secs(60), "no page may take longer than a one-page file");
    }

    #[test]
    fn ocr_requests_are_rejected_at_the_boundary_for_bad_languages_modes_and_ranges() {
        let input = PathBuf::from("scan.pdf");
        let output = PathBuf::from("out");
        let cases: [(&str, fn(&mut OcrRequest), &str); 5] = [
            ("empty languages", |request| request.languages.clear(), "at least one recognition language"),
            ("unknown language", |request| request.languages.push("klingon".into()), "\"klingon\" is not a recognition language"),
            ("unknown mode", |request| request.mode = "hocr".into(), "\"hocr\" is not an OCR output"),
            ("backwards range", |request| request.page_range = Some("3-1".into()), "runs backwards"),
            ("malformed range", |request| request.page_range = Some("1, x".into()), "isn't a page or range"),
        ];
        for (name, change, message) in cases {
            let mut request = ocr_request(&input, None, &output);
            change(&mut request);
            let outcomes = validate_ocr_request(&request).expect_err(name);
            assert_eq!(outcomes.len(), 1, "{name}");
            let error = outcomes[0].error().expect(name);
            assert_eq!(error.kind, ErrorKind::InvalidInput, "{name}");
            assert!(error.message.contains(message), "{name}: {}", error.message);
        }
        let mut searchable = ocr_request(&input, Some("1-3, 5, 8-"), &output);
        searchable.mode = "searchablePdf".into();
        searchable.force = true;
        assert!(validate_ocr_request(&searchable).is_ok());
    }

    #[test]
    fn word_boxes_keep_reading_order_and_take_line_heights_from_line_rows() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
1\t1\t0\t0\t0\t0\t0\t0\t1700\t2200\t-1\t\n\
4\t1\t1\t1\t1\t0\t100\t200\t500\t60\t-1\t\n\
5\t1\t1\t1\t1\t1\t100\t210\t200\t40\t96.1\tFirst\n\
5\t1\t1\t1\t1\t2\t320\t205\t280\t50\t12.0\tline\n\
5\t1\t1\t1\t1\t3\t620\t205\t0\t0\t-1\t \n\
5\t1\t2\t1\t1\t1\t100\t400\t240\t50\t91.0\tПривет\r\n";
        let layout = super::word_boxes(tsv.as_bytes()).unwrap();
        assert_eq!((layout.width, layout.height), (1700., 2200.));
        let lines = layout.lines.iter().map(|line| (line.top, line.bottom, line.words.iter().map(|word| (word.text.as_str(), word.left, word.width)).collect::<Vec<_>>())).collect::<Vec<_>>();
        assert_eq!(lines, vec![
            (200., 260., vec![("First", 100., 200.), ("line", 320., 280.)]),
            (400., 450., vec![("Привет", 100., 240.)]),
        ]);
        assert_eq!(super::word_boxes(b"level\tpage_num\n1\t1\t0\t0\t0\t0\t0\t0\t1700\t2200\t-1\t\n").unwrap().lines, Vec::new());
        assert!(super::word_boxes(b"5\t1\t1\t1\t1\t1\t1\t1\t9\t9\t90\tlost\n").is_err(), "words without a page size can't be placed");
        assert!(super::word_boxes(b"5\t1\t1\tnot a box\n").is_err());
    }

    #[test]
    fn the_shared_ocr_request_sample_parses_and_passes_the_boundary() {
        let request: OcrRequest = serde_json::from_str(include_str!("../../../shared/ocr-request-sample.json")).unwrap();
        assert_eq!(request.paths, vec![PathBuf::from("/scans/contract.pdf")]);
        assert_eq!(request.page_range.as_deref(), Some("1-3, 5, 8-"));
        assert_eq!(request.mode(), Ok(super::OcrMode::Text));
        assert!(!request.force);
        assert_eq!(request.languages, vec!["eng"]);
        assert!(matches!(request.output_location, OutputLocation::AlongsideInput));
        assert!(validate_ocr_request(&request).is_ok());
    }

    #[cfg(unix)]
    struct TimedOcrHelpers {
        renderer: PathBuf,
        engine: PathBuf,
        tessdata: PathBuf,
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    #[cfg(unix)]
    impl TimedOcrHelpers {
        fn install(name: &str, renderer_body: &str, engine_body: &str) -> Self {
            let renderer = path(&format!("{name}-renderer.sh"));
            let engine = path(&format!("{name}-engine.sh"));
            let tessdata = path(&format!("{name}-eng.traineddata"));
            fs::write(&renderer, format!("#!/bin/sh\n{renderer_body}\ntouch \"${{10}}.png\"\n")).unwrap();
            fs::write(&engine, format!("#!/bin/sh\n{engine_body}\n")).unwrap();
            fs::write(&tessdata, b"fake tessdata").unwrap();
            for helper in [&renderer, &engine] {
                fs::set_permissions(helper, fs::Permissions::from_mode(0o755)).unwrap();
            }
            let saved = ["TOOLBOX_PDFTOPPM_PATH", "TOOLBOX_TESSERACT_PATH", "TOOLBOX_TESSDATA_PATH"].into_iter().map(|key| (key, std::env::var_os(key))).collect();
            std::env::set_var("TOOLBOX_PDFTOPPM_PATH", &renderer);
            std::env::set_var("TOOLBOX_TESSERACT_PATH", &engine);
            std::env::set_var("TOOLBOX_TESSDATA_PATH", &tessdata);
            Self { renderer, engine, tessdata, saved }
        }
    }

    #[cfg(unix)]
    impl Drop for TimedOcrHelpers {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                match value { Some(value) => std::env::set_var(key, value), None => std::env::remove_var(key) }
            }
            for file in [&self.renderer, &self.engine, &self.tessdata] { let _ = fs::remove_file(file); }
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_multi_page_file_gets_a_budget_for_every_page() {
        let _guard = crate::kit::lock_process_env();
        let input = path("per-page-budget.pdf");
        let output_root = path("per-page-budget-output");
        make_pdf(&input, 3);
        fs::create_dir_all(&output_root).unwrap();
        let helpers = TimedOcrHelpers::install("per-page-budget", "sleep 0.6", "sleep 0.6\nprintf 'page\\n'");
        let deadline = HelperDeadline { per_file: Duration::from_millis(500), per_page: Duration::from_secs(2) };

        let started = Instant::now();
        let outcome = ocr_pdf_within(&run_of(ocr_request(&input, None, &output_root)), input.clone(), deadline);
        let elapsed = started.elapsed();
        let text = outcome.output_paths().first().and_then(|path| fs::read_to_string(path).ok());
        drop(helpers);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_dir_all(&output_root);

        assert!(outcome.error().is_none(), "budgeted OCR failed: {:?}", outcome.error());
        assert_eq!(text.as_deref(), Some("page\n\u{c}page\n\u{c}page\n"));
        assert!(elapsed > deadline.limit(1, 1), "three pages ran longer than one page's budget, so a single shared deadline would have stopped them: {elapsed:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_page_that_passes_its_budget_fails_the_file_with_a_time_limit() {
        let _guard = crate::kit::lock_process_env();
        let input = path("page-over-budget.pdf");
        let output_root = path("page-over-budget-output");
        make_pdf(&input, 2);
        let original = fs::read(&input).unwrap();
        fs::create_dir_all(&output_root).unwrap();
        let before_workspaces = ocr_workspaces();
        let helpers = TimedOcrHelpers::install("page-over-budget", ":", "case \"$1\" in *page-2.png) exec sleep 30 ;; esac\nprintf 'page one\\n'");

        let started = Instant::now();
        let outcome = ocr_pdf_within(&run_of(ocr_request(&input, None, &output_root)), input.clone(), HelperDeadline { per_file: Duration::from_secs(1), per_page: Duration::from_secs(3) });
        let elapsed = started.elapsed();
        let after_workspaces = ocr_workspaces();
        let output_entries = fs::read_dir(&output_root).unwrap().count();
        let input_after = fs::read(&input).unwrap();
        drop(helpers);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_dir_all(&output_root);

        let error = outcome.error().expect("the stalled page fails the file");
        assert_eq!(error.kind, ErrorKind::LimitExceeded { limit: LimitKind::Time });
        assert_eq!(error.message, "Stopped at page 2 of 2: that page took longer than 4 seconds. Nothing was saved.");
        assert!(elapsed < Duration::from_secs(10), "the engine is stopped at its deadline, not waited out: {elapsed:?}");
        assert!(outcome.output_paths().is_empty());
        assert_eq!(output_entries, 0);
        assert_eq!(before_workspaces, after_workspaces);
        assert_eq!(input_after, original);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_runs_out_of_its_shared_budget_says_which_page_it_reached() {
        let _guard = crate::kit::lock_process_env();
        let input = path("file-over-budget.pdf");
        let output_root = path("file-over-budget-output");
        make_pdf(&input, 3);
        let original = fs::read(&input).unwrap();
        fs::create_dir_all(&output_root).unwrap();
        // Pages one and two use most of each page's share, so the third has less left of the file's budget than a page may take.
        let helpers = TimedOcrHelpers::install("file-over-budget", ":", "case \"$1\" in *page-3.png) exec sleep 30 ;; esac\nsleep 1\nprintf 'page\\n'");
        let deadline = HelperDeadline { per_file: Duration::from_secs(3), per_page: Duration::from_millis(300) };

        let outcome = ocr_pdf_within(&run_of(ocr_request(&input, None, &output_root)), input.clone(), deadline);
        let output_entries = fs::read_dir(&output_root).unwrap().count();
        let input_after = fs::read(&input).unwrap();
        drop(helpers);
        let _ = fs::remove_file(&input);
        let _ = fs::remove_dir_all(&output_root);

        let error = outcome.error().expect("the file runs out of time");
        assert_eq!(error.kind, ErrorKind::LimitExceeded { limit: LimitKind::Time });
        assert_eq!(error.message, "Stopped at page 3 of 3: it reached the time limit for one file. Nothing was saved. Split it into smaller PDFs, then try again.");
        assert_eq!(output_entries, 0);
        assert_eq!(input_after, original);
    }
}
