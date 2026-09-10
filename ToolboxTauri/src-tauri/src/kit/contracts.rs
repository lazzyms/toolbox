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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolError {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputCardinality {
    Single,
    Multiple,
    Ordered,
}

#[derive(Debug, Clone, Copy)]
pub struct NativeCapability {
    pub command: &'static str,
    pub accepted_extensions: &'static [&'static str],
    pub input_cardinality: InputCardinality,
    pub supports_page_selection: bool,
    pub supports_preview: bool,
    pub native_available: bool,
}

const PDF: &[&str] = &["pdf"];
const OFFICE: &[&str] = &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx"];
const IMAGE: &[&str] = &["png", "jpg", "jpeg", "webp", "heic", "heif"];
const IMAGE_SEQUENCE: &[&str] = &["png", "jpg", "jpeg", "webp", "heic", "tif", "tiff"];
const TIFF: &[&str] = &["tif", "tiff"];

pub const NATIVE_CAPABILITIES: &[NativeCapability] = &[
    NativeCapability { command: "remove_password", accepted_extensions: OFFICE, input_cardinality: InputCardinality::Multiple, supports_page_selection: false, supports_preview: false, native_available: true },
    NativeCapability { command: "protect_pdf", accepted_extensions: PDF, input_cardinality: InputCardinality::Multiple, supports_page_selection: false, supports_preview: false, native_available: true },
    NativeCapability { command: "compress_images", accepted_extensions: IMAGE, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: true, native_available: true },
    NativeCapability { command: "convert_images", accepted_extensions: IMAGE, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: true, native_available: true },
    NativeCapability { command: "export_pdf_scene", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    NativeCapability { command: "export_image_edit_plan", accepted_extensions: IMAGE, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: true, native_available: true },
    NativeCapability { command: "crop_pdf", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    NativeCapability { command: "sign_pdf", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    NativeCapability { command: "edit_pdf", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    NativeCapability { command: "edit_pdf_session", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    NativeCapability { command: "organize_pdf", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    NativeCapability { command: "add_pdf_pages", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    NativeCapability { command: "add_page_numbers", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    NativeCapability { command: "watermark_pdf", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    NativeCapability { command: "compress_pdf", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: false, native_available: true },
    NativeCapability { command: "remove_pdf_pages", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    NativeCapability { command: "extract_pdf_pages", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: true, native_available: true },
    NativeCapability { command: "merge_pdfs", accepted_extensions: PDF, input_cardinality: InputCardinality::Ordered, supports_page_selection: false, supports_preview: false, native_available: true },
    NativeCapability { command: "split_pdf", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: false, native_available: true },
    NativeCapability { command: "pdf_to_images", accepted_extensions: PDF, input_cardinality: InputCardinality::Multiple, supports_page_selection: true, supports_preview: true, native_available: true },
    NativeCapability { command: "pdf_to_text", accepted_extensions: PDF, input_cardinality: InputCardinality::Multiple, supports_page_selection: true, supports_preview: true, native_available: true },
    NativeCapability { command: "extract_pdf_images", accepted_extensions: PDF, input_cardinality: InputCardinality::Multiple, supports_page_selection: true, supports_preview: false, native_available: true },
    NativeCapability { command: "images_to_pdf", accepted_extensions: IMAGE_SEQUENCE, input_cardinality: InputCardinality::Ordered, supports_page_selection: false, supports_preview: false, native_available: true },
    NativeCapability { command: "ocr_pdf", accepted_extensions: PDF, input_cardinality: InputCardinality::Single, supports_page_selection: true, supports_preview: false, native_available: false },
    NativeCapability { command: "blur_faces", accepted_extensions: IMAGE, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: false, native_available: false },
    NativeCapability { command: "remove_image_background", accepted_extensions: IMAGE, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: false, native_available: false },
    NativeCapability { command: "resize_images", accepted_extensions: IMAGE, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: true, native_available: true },
    NativeCapability { command: "rotate_images", accepted_extensions: IMAGE, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: true, native_available: true },
    NativeCapability { command: "crop_images", accepted_extensions: IMAGE, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: true, native_available: true },
    NativeCapability { command: "adjust_image_tone", accepted_extensions: IMAGE, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: true, native_available: true },
    NativeCapability { command: "watermark_images", accepted_extensions: IMAGE, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: true, native_available: true },
    NativeCapability { command: "generate_icon_set", accepted_extensions: IMAGE, input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: true, native_available: true },
    NativeCapability { command: "create_gif", accepted_extensions: &["png", "jpg", "jpeg", "webp"], input_cardinality: InputCardinality::Ordered, supports_page_selection: false, supports_preview: false, native_available: true },
    NativeCapability { command: "extract_gif_frames", accepted_extensions: &["gif"], input_cardinality: InputCardinality::Single, supports_page_selection: false, supports_preview: true, native_available: true },
    NativeCapability { command: "process_tiff_pages", accepted_extensions: TIFF, input_cardinality: InputCardinality::Ordered, supports_page_selection: false, supports_preview: true, native_available: true },
    NativeCapability { command: "image_metadata", accepted_extensions: &["png", "jpg", "jpeg", "webp", "heic", "heif", "tif", "tiff"], input_cardinality: InputCardinality::Multiple, supports_page_selection: false, supports_preview: false, native_available: true },
];

pub fn native_capability(command: &str) -> Option<&'static NativeCapability> {
    NATIVE_CAPABILITIES.iter().find(|capability| capability.command == command)
}

pub struct ValidatedCommandInputs {
    pub accepted: Vec<(usize, PathBuf)>,
    pub rejected: Vec<(usize, JobOutcome)>,
}

pub fn validate_page_selection(command: &str, paths: &[PathBuf], pages: Option<&[usize]>) -> Result<(), Vec<JobOutcome>> {
    let Some(pages) = pages else { return Ok(()); };
    if paths.is_empty() {
        return Err(vec![JobOutcome::failure(PathBuf::new(), ToolError::invalid_input("Select at least one input file."))]);
    }
    let Some(capability) = native_capability(command) else {
        return Err(paths.iter().cloned().map(|path| JobOutcome::failure(path, ToolError::invalid_input(format!("Unknown native command: {command}.")))).collect());
    };
    if !capability.supports_page_selection {
        return Err(paths.iter().cloned().map(|path| JobOutcome::failure(path, ToolError::invalid_input(format!("{command} does not support page selections.")))).collect());
    }
    if pages.is_empty() {
        return Err(paths.iter().cloned().map(|path| JobOutcome::failure(path, ToolError::invalid_input("Select at least one page."))).collect());
    }
    Ok(())
}

pub fn command_supports_preview(command: &str) -> bool {
    native_capability(command).is_some_and(|capability| capability.supports_preview)
}

pub fn validate_command_inputs(command: &str, paths: &[PathBuf]) -> Result<(), Vec<JobOutcome>> {
    let validation = match partition_command_inputs(command, paths) {
        Ok(validation) => validation,
        Err(outcomes) => return Err(outcomes),
    };
    if validation.rejected.is_empty() { return Ok(()); }
    let mut outcomes = vec![None; paths.len()];
    for (index, outcome) in validation.rejected { outcomes[index] = Some(outcome); }
    for (index, path) in validation.accepted {
        outcomes[index] = Some(JobOutcome::failure(path, ToolError::invalid_input("The selection contains an invalid input, so no files were processed.")));
    }
    Err(outcomes.into_iter().map(Option::unwrap).collect())
}

pub fn partition_command_inputs(command: &str, paths: &[PathBuf]) -> Result<ValidatedCommandInputs, Vec<JobOutcome>> {
    let Some(capability) = native_capability(command) else {
        return Err(vec![JobOutcome::failure(PathBuf::new(), ToolError::invalid_input(format!("Unknown native command: {command}.")))]);
    };
    if paths.is_empty() {
        let error = if capability.native_available { ToolError::invalid_input("Select at least one input file.") } else { ToolError::unavailable(format!("Native command {command} is unavailable in this build.")) };
        return Err(vec![JobOutcome::failure(PathBuf::new(), error)]);
    }
    if !capability.native_available {
        return Ok(ValidatedCommandInputs {
            accepted: Vec::new(),
            rejected: paths.iter().cloned().enumerate().map(|(index, path)| (index, JobOutcome::failure(path, ToolError::unavailable(format!("Native command {command} is unavailable in this build."))))).collect(),
        });
    }
    if capability.input_cardinality == InputCardinality::Single && paths.len() != 1 {
        return Ok(ValidatedCommandInputs {
            accepted: Vec::new(),
            rejected: paths.iter().cloned().enumerate().map(|(index, path)| (index, JobOutcome::failure(path, ToolError::invalid_input("This action accepts exactly one input file.")))).collect(),
        });
    }
    let accepted_extensions = capability.accepted_extensions.iter().map(|extension| format!(".{extension}")).collect::<Vec<_>>().join(", ");
    let (accepted, rejected): (Vec<_>, Vec<_>) = paths.iter().cloned().enumerate().partition(|(_, path)| {
        path.extension().and_then(|extension| extension.to_str()).map(|extension| capability.accepted_extensions.iter().any(|accepted| accepted.eq_ignore_ascii_case(extension))).unwrap_or(false)
    });
    Ok(ValidatedCommandInputs {
        accepted,
        rejected: rejected.into_iter().map(|(index, path)| {
            (index, JobOutcome::failure(path, ToolError::invalid_input(format!("Unsupported input format. Accepted extensions: {accepted_extensions}."))))
        }).collect(),
    })
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
    fn capability_registry_rejects_single_input_truncation() {
        let paths = vec![PathBuf::from("one.png"), PathBuf::from("two.png")];
        let outcomes = validate_command_inputs("convert_images", &paths).unwrap_err();

        assert_eq!(outcomes.iter().map(|outcome| outcome.input_path.clone()).collect::<Vec<_>>(), paths);
        assert!(outcomes.iter().all(|outcome| outcome.failure.as_ref().is_some_and(|error| matches!(error.kind, ErrorKind::InvalidInput))));
    }

    #[test]
    fn capability_registry_rejects_unsupported_extensions_at_the_boundary() {
        let outcomes = validate_command_inputs("convert_images", &[PathBuf::from("source.pdf")]).unwrap_err();

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].failure.as_ref().is_some_and(|error| error.message.contains("Accepted extensions")));
    }

    #[test]
    fn capability_registry_keeps_valid_inputs_when_one_extension_is_rejected() {
        let paths = vec![PathBuf::from("source.png"), PathBuf::from("source.pdf")];
        let validation = partition_command_inputs("image_metadata", &paths).unwrap();

        assert_eq!(validation.accepted, vec![(0, paths[0].clone())]);
        assert_eq!(validation.rejected.len(), 1);
        assert_eq!(validation.rejected[0].0, 1);
    }

    #[test]
    fn page_capability_rejects_an_explicit_empty_selection() {
        let outcomes = validate_page_selection("pdf_to_text", &[PathBuf::from("source.pdf")], Some(&[])).unwrap_err();

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].failure.as_ref().is_some_and(|error| error.message.contains("at least one page")));
    }
}
