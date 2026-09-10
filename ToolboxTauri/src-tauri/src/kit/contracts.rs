use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputLocation {
    AlongsideInput,
    CustomFolder(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub completed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfRequest {
    pub paths: Vec<PathBuf>,
    pub password: String,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordRequest {
    pub paths: Vec<PathBuf>,
    pub password: String,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressImagesRequest {
    pub paths: Vec<PathBuf>,
    pub quality: u8,
    #[serde(default)]
    pub lossless: bool,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertImagesRequest {
    pub paths: Vec<PathBuf>,
    pub format: String,
    pub output_location: OutputLocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputCardinality {
    Single,
    Multiple { minimum: usize },
}

#[derive(Debug, Clone, Copy)]
pub struct Capability {
    pub command: &'static str,
    pub accepted_extensions: &'static [&'static str],
    pub input_cardinality: InputCardinality,
    pub supports_page_selection: bool,
    pub supports_preview: bool,
    pub native_available: bool,
}

const IMAGE_EXTENSIONS: &[&str] = &["bmp", "gif", "heic", "jpeg", "jpg", "png", "tif", "tiff", "webp"];
const DOCUMENT_EXTENSIONS: &[&str] = &["doc", "docx", "pdf", "ppt", "pptx", "xls", "xlsx"];
const PDF_EXTENSIONS: &[&str] = &["pdf"];

pub const CAPABILITIES: &[Capability] = &[
    Capability { command: "remove_password", accepted_extensions: DOCUMENT_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: false, native_available: true },
    Capability { command: "add_page_numbers", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    Capability { command: "merge_pdfs", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Multiple { minimum: 2 }, supports_page_selection: false, supports_preview: false, native_available: true },
    Capability { command: "watermark_pdf", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    Capability { command: "crop_pdf", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    Capability { command: "edit_pdf", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    Capability { command: "protect_pdf", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: false, native_available: true },
    Capability { command: "images_to_pdf", accepted_extensions: IMAGE_EXTENSIONS, input_cardinality: InputCardinality::Multiple { minimum: 1 }, supports_page_selection: false, supports_preview: true, native_available: true },
    Capability { command: "pdf_to_images", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    Capability { command: "pdf_to_text", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: true, native_available: true },
    Capability { command: "split_pdf", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: false, native_available: true },
    Capability { command: "extract_pdf_images", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: false, native_available: true },
    Capability { command: "sign_pdf", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    Capability { command: "ocr_pdf", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: false, native_available: false },
    Capability { command: "remove_pdf_pages", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    Capability { command: "extract_pdf_pages", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    Capability { command: "organize_pdf", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    Capability { command: "compress_pdf", accepted_extensions: PDF_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: false, native_available: true },
    Capability { command: "convert_images", accepted_extensions: IMAGE_EXTENSIONS, input_cardinality: InputCardinality::Multiple { minimum: 1 }, supports_page_selection: false, supports_preview: true, native_available: true },
    Capability { command: "compress_images", accepted_extensions: IMAGE_EXTENSIONS, input_cardinality: InputCardinality::Multiple { minimum: 1 }, supports_page_selection: false, supports_preview: true, native_available: true },
    Capability { command: "resize_images", accepted_extensions: IMAGE_EXTENSIONS, input_cardinality: InputCardinality::Multiple { minimum: 1 }, supports_page_selection: false, supports_preview: true, native_available: true },
    Capability { command: "rotate_images", accepted_extensions: IMAGE_EXTENSIONS, input_cardinality: InputCardinality::Multiple { minimum: 1 }, supports_page_selection: false, supports_preview: true, native_available: true },
    Capability { command: "crop_images", accepted_extensions: IMAGE_EXTENSIONS, input_cardinality: InputCardinality::Multiple { minimum: 1 }, supports_page_selection: false, supports_preview: true, native_available: true },
    Capability { command: "generate_icon_set", accepted_extensions: IMAGE_EXTENSIONS, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: true, native_available: true },
    Capability { command: "create_gif", accepted_extensions: IMAGE_EXTENSIONS, input_cardinality: InputCardinality::Multiple { minimum: 2 }, supports_page_selection: false, supports_preview: true, native_available: true },
    Capability { command: "extract_gif_frames", accepted_extensions: &["gif"], input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: true, native_available: true },
    Capability { command: "watermark_images", accepted_extensions: IMAGE_EXTENSIONS, input_cardinality: InputCardinality::Multiple { minimum: 1 }, supports_page_selection: false, supports_preview: true, native_available: true },
    Capability { command: "image_metadata", accepted_extensions: IMAGE_EXTENSIONS, input_cardinality: InputCardinality::Multiple { minimum: 1 }, supports_page_selection: false, supports_preview: false, native_available: true },
    Capability { command: "adjust_image_tone", accepted_extensions: IMAGE_EXTENSIONS, input_cardinality: InputCardinality::Multiple { minimum: 1 }, supports_page_selection: false, supports_preview: true, native_available: true },
    Capability { command: "process_tiff_pages", accepted_extensions: &["tif", "tiff"], input_cardinality: InputCardinality::Multiple { minimum: 1 }, supports_page_selection: false, supports_preview: true, native_available: true },
    Capability { command: "blur_faces", accepted_extensions: IMAGE_EXTENSIONS, input_cardinality: InputCardinality::Multiple { minimum: 1 }, supports_page_selection: false, supports_preview: false, native_available: false },
    Capability { command: "remove_image_background", accepted_extensions: IMAGE_EXTENSIONS, input_cardinality: InputCardinality::Multiple { minimum: 1 }, supports_page_selection: false, supports_preview: false, native_available: false },
];

pub fn capability(command: &str) -> Option<&'static Capability> {
    CAPABILITIES.iter().find(|capability| capability.command == command)
}

pub fn validate_cardinality(command: &str, paths: &[PathBuf]) -> Result<(), ToolError> {
    let capability = capability(command).ok_or_else(|| ToolError::invalid_input(format!("Unknown tool command: {command}.")))?;
    if !capability.native_available {
        return Err(ToolError::unavailable(format!("{command} is unavailable because its native resource is not bundled.")));
    }
    let valid = match capability.input_cardinality {
        InputCardinality::Single => paths.len() == 1,
        InputCardinality::Multiple { minimum } => paths.len() >= minimum,
    };
    if valid {
        Ok(())
    } else {
        let expectation = match capability.input_cardinality {
            InputCardinality::Single => "exactly one file".to_string(),
            InputCardinality::Multiple { minimum } => format!("at least {minimum} files"),
        };
        Err(ToolError::invalid_input(format!("{command} accepts {expectation}.")))
    }
}

pub fn validate_path(command: &str, path: &PathBuf) -> Result<(), ToolError> {
    let capability = capability(command).ok_or_else(|| ToolError::invalid_input(format!("Unknown tool command: {command}.")))?;
    if !capability.native_available {
        return Err(ToolError::unavailable(format!("{command} is unavailable because its native resource is not bundled.")));
    }
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return Err(ToolError::invalid_input(format!("{} has no supported file extension.", path.display())));
    };
    if capability.accepted_extensions.iter().any(|accepted| accepted.eq_ignore_ascii_case(extension)) {
        Ok(())
    } else {
        Err(ToolError::invalid_input(format!("{} is not a supported input for {command}.", path.display())))
    }
}

pub fn validate_request(command: &str, paths: &[PathBuf]) -> Result<(), ToolError> {
    validate_cardinality(command, paths)?;
    paths.iter().try_for_each(|path| validate_path(command, path))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolError {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    InvalidInput,
    Unavailable,
    LimitExceeded,
    Processing,
}

impl ToolError {
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self { kind: ErrorKind::InvalidInput, message: message.into() }
    }

    pub fn processing(message: impl Into<String>) -> Self {
        Self { kind: ErrorKind::Processing, message: message.into() }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self { kind: ErrorKind::Unavailable, message: message.into() }
    }
}

impl Default for ToolError {
    fn default() -> Self {
        Self::processing("")
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobOutcome {
    pub input_path: PathBuf,
    pub output_paths: Vec<PathBuf>,
    pub detail: String,
    pub failure: Option<ToolError>,
}

impl JobOutcome {
    pub fn failure(input_path: PathBuf, error: ToolError) -> Self {
        Self { input_path, output_paths: Vec::new(), detail: String::new(), failure: Some(error) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_result_uses_the_frontend_field_names() {
        let outcome = JobOutcome {
            input_path: PathBuf::from("source.pdf"),
            output_paths: vec![PathBuf::from("source-unlocked.pdf")],
            detail: "PDF Unlocked".to_string(),
            failure: None,
        };
        let value = serde_json::to_value(outcome).unwrap();
        assert_eq!(value["inputPath"], "source.pdf");
        assert_eq!(value["outputPaths"][0], "source-unlocked.pdf");
    }

    #[test]
    fn failures_are_structured_for_frontend_handling() {
        let outcome = JobOutcome::failure(
            PathBuf::from("missing.pdf"),
            ToolError::invalid_input("Input file does not exist."),
        );
        let value = serde_json::to_value(outcome).unwrap();
        assert_eq!(value["failure"]["kind"], "invalidInput");
        assert_eq!(value["failure"]["message"], "Input file does not exist.");
    }

    #[test]
    fn request_and_progress_are_serializable_contracts() {
        let request = ToolRequest {
            paths: vec![PathBuf::from("source.pdf")],
            output_location: OutputLocation::AlongsideInput,
        };
        let progress = Progress { completed: 1, total: 2 };
        assert_eq!(serde_json::to_value(request).unwrap()["outputLocation"], "alongsideInput");
        assert_eq!(serde_json::to_value(progress).unwrap()["completed"], 1);
    }

    #[test]
    fn custom_folder_is_part_of_the_request_shape() {
        let request = ToolRequest {
            paths: vec![PathBuf::from("source.pdf")],
            output_location: OutputLocation::CustomFolder(PathBuf::from("exports")),
        };
        assert_eq!(serde_json::to_value(request).unwrap()["outputLocation"]["customFolder"], "exports");
    }

    #[test]
    fn capability_registry_covers_every_native_action() {
        assert_eq!(CAPABILITIES.len(), 32);
        assert!(CAPABILITIES.iter().all(|capability| !capability.accepted_extensions.is_empty()));
        assert!(capability("pdf_to_images").unwrap().supports_page_selection);
        assert!(capability("pdf_to_images").unwrap().supports_preview);
        assert!(!capability("ocr_pdf").unwrap().native_available);
    }

    #[test]
    fn validates_cardinality_and_extensions_at_the_native_boundary() {
        let single = vec![PathBuf::from("one.png"), PathBuf::from("two.png")];
        assert_eq!(validate_cardinality("generate_icon_set", &single).unwrap_err().kind, ErrorKind::InvalidInput);
        assert!(validate_path("convert_images", &PathBuf::from("one.PNG")).is_ok());
        assert_eq!(validate_path("convert_images", &PathBuf::from("one.txt")).unwrap_err().kind, ErrorKind::InvalidInput);
        assert_eq!(validate_cardinality("ocr_pdf", &[PathBuf::from("one.pdf")]).unwrap_err().kind, ErrorKind::Unavailable);
    }
}
