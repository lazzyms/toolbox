use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;
use std::sync::OnceLock;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputCardinality {
    Single,
    Multiple,
    Ordered,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeCapability {
    pub command: String,
    pub accepted_extensions: Vec<String>,
    pub input_cardinality: InputCardinality,
    pub supports_page_selection: bool,
    pub supports_preview: bool,
    #[serde(rename = "nativeAvailability", deserialize_with = "deserialize_native_availability")]
    pub native_available: bool,
}

static CAPABILITY_REGISTRY: OnceLock<Vec<NativeCapability>> = OnceLock::new();

fn deserialize_native_availability<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match String::deserialize(deserializer)?.as_str() {
        "available" => Ok(true),
        "unavailable" => Ok(false),
        value => Err(serde::de::Error::custom(format!("unknown native availability {value}"))),
    }
}

fn parse_capability_registry(raw: &str) -> Result<Vec<NativeCapability>, String> {
    let capabilities: Vec<NativeCapability> = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    if capabilities.is_empty() {
        return Err("The shared capability registry is empty.".to_string());
    }
    let mut commands = HashSet::new();
    for capability in &capabilities {
        if capability.command.is_empty() || !commands.insert(capability.command.as_str()) {
            return Err(format!("Duplicate or empty shared capability command {}.", capability.command));
        }
        if capability.accepted_extensions.is_empty() {
            return Err(format!("{} has no accepted extensions.", capability.command));
        }
        let mut extensions = HashSet::new();
        for extension in &capability.accepted_extensions {
            let valid = extension.strip_prefix('.').is_some_and(|value| {
                !value.is_empty()
                    && value.chars().all(|character| {
                        character.is_ascii_lowercase() || character.is_ascii_digit()
                    })
            });
            if !valid || !extensions.insert(extension.as_str()) {
                return Err(format!(
                    "{} has an invalid accepted extension {extension}.",
                    capability.command
                ));
            }
        }
    }
    Ok(capabilities)
}

pub fn native_capabilities() -> &'static [NativeCapability] {
    CAPABILITY_REGISTRY
        .get_or_init(|| {
            parse_capability_registry(include_str!("../../../shared/tool-capabilities.json"))
                .unwrap_or_else(|error| panic!("Invalid shared capability registry: {error}"))
        })
        .as_slice()
}

pub fn native_capability(command: &str) -> Option<&'static NativeCapability> {
    native_capabilities()
        .iter()
        .find(|capability| capability.command == command)
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
    let accepted_extensions = capability.accepted_extensions.join(", ");
    let (accepted, rejected): (Vec<_>, Vec<_>) = paths.iter().cloned().enumerate().partition(|(_, path)| {
        path.extension().and_then(|extension| extension.to_str()).map(|extension| {
            capability.accepted_extensions.iter().any(|accepted| accepted.trim_start_matches('.').eq_ignore_ascii_case(extension))
        }).unwrap_or(false)
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
        Self::failure_with_outputs(input_path, Vec::new(), error)
    }

    pub fn failure_with_outputs(input_path: PathBuf, output_paths: Vec<PathBuf>, error: ToolError) -> Self {
        Self { input_path, output_paths, detail: String::new(), failure: Some(error) }
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
    fn partial_failures_keep_published_outputs_in_the_ipc_contract() {
        let output = PathBuf::from("source-frame-1.png");
        let outcome = JobOutcome::failure_with_outputs(
            PathBuf::from("source.gif"),
            vec![output.clone()],
            ToolError::processing("Could not publish the next frame."),
        );
        let value = serde_json::to_value(outcome).unwrap();

        assert_eq!(value["outputPaths"], serde_json::json!([output]));
        assert_eq!(value["failure"]["kind"], "processing");
        assert_eq!(value["failure"]["message"], "Could not publish the next frame.");
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
    fn images_to_pdf_rejects_tiff_at_the_native_boundary() {
        for extension in ["tif", "tiff"] {
            let outcomes = validate_command_inputs("images_to_pdf", &[PathBuf::from(format!("source.{extension}"))]).unwrap_err();

            assert_eq!(outcomes.len(), 1);
            assert!(outcomes[0].failure.as_ref().is_some_and(|error| matches!(error.kind, ErrorKind::InvalidInput)));
            assert!(outcomes[0].failure.as_ref().is_some_and(|error| error.message.contains("Accepted extensions")));
        }
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

    #[test]
    fn native_capabilities_match_the_shared_contract_artifact() {
        let shared: serde_json::Value = serde_json::from_str(include_str!("../../../shared/tool-capabilities.json")).unwrap();
        let rows = shared.as_array().unwrap();

        assert_eq!(rows.len(), native_capabilities().len());
        for row in rows {
            let command = row["command"].as_str().unwrap();
            let capability = native_capability(command).unwrap_or_else(|| panic!("Missing native capability for {command}"));
            let accepted_extensions = row["acceptedExtensions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|extension| extension.as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(capability.accepted_extensions, accepted_extensions);
            assert_eq!(
                capability.input_cardinality,
                match row["inputCardinality"].as_str().unwrap() {
                    "single" => InputCardinality::Single,
                    "multiple" => InputCardinality::Multiple,
                    "ordered" => InputCardinality::Ordered,
                    cardinality => panic!("Unknown input cardinality {cardinality}"),
                }
            );
            assert_eq!(capability.supports_page_selection, row["supportsPageSelection"].as_bool().unwrap());
            assert_eq!(capability.supports_preview, row["supportsPreview"].as_bool().unwrap());
            assert_eq!(capability.native_available, row["nativeAvailability"] == "available");
        }

        let office = native_capability("remove_password").unwrap();
        assert_eq!(office.accepted_extensions, [".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx"]);
        let images = native_capability("images_to_pdf").unwrap();
        assert_eq!(images.input_cardinality, InputCardinality::Ordered);
        assert!(!images.accepted_extensions.contains(&".tif".to_string()));
        assert!(!images.accepted_extensions.contains(&".tiff".to_string()));
        assert!(!native_capability("extract_gif_frames").unwrap().supports_preview);
        let pdf = native_capability("pdf_to_text").unwrap();
        assert!(pdf.supports_page_selection && pdf.supports_preview);
        assert!(!native_capability("ocr_pdf").unwrap().native_available);
        assert!(!native_capability("blur_faces").unwrap().native_available);
        assert!(!native_capability("remove_image_background").unwrap().native_available);
    }

    #[test]
    fn shared_capability_parser_rejects_duplicate_and_unknown_contract_data() {
        let row = r#"{"command":"test","acceptedExtensions":[".pdf"],"inputCardinality":"single","supportsPageSelection":false,"supportsPreview":false,"nativeAvailability":"available"}"#;
        assert!(parse_capability_registry(&format!("[{row},{row}]")).unwrap_err().contains("Duplicate"));
        let unknown = format!("[{}]", row.replace('}', ",\"unknown\":true}"));
        assert!(parse_capability_registry(&unknown)
            .unwrap_err()
            .contains("unknown field"));
    }
}
