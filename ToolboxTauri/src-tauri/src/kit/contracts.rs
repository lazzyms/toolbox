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

impl OutputLocation {
    pub fn directory_for<'a>(&'a self, input: &'a std::path::Path) -> &'a std::path::Path {
        match self {
            OutputLocation::AlongsideInput => input.parent().unwrap_or_else(|| std::path::Path::new(".")),
            OutputLocation::CustomFolder(folder) => folder,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolError {
    pub kind: ErrorKind,
    pub message: String,
    #[serde(default)]
    pub pointer: Option<OutcomePointer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    InvalidInput,
    Unavailable,
    LimitExceeded,
    Processing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum OutcomePointer {
    Tool { tool_id: String },
    Pack { pack_id: String },
    OutputFolder,
}

impl ToolError {
    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), pointer: None }
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, message)
    }

    pub fn processing(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Processing, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unavailable, message)
    }

    pub fn with_pointer(self, pointer: OutcomePointer) -> Self {
        Self { pointer: Some(pointer), ..self }
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
    pub output_suffix: String,
    pub many_to_one: bool,
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
        if !is_valid_output_suffix(&capability.output_suffix) {
            return Err(format!(
                "{} has an invalid output suffix {}.",
                capability.command, capability.output_suffix
            ));
        }
    }
    Ok(capabilities)
}

fn is_valid_output_suffix(suffix: &str) -> bool {
    suffix.is_empty()
        || suffix.strip_prefix('-').is_some_and(|rest| {
            rest.split('-').all(|segment| {
                !segment.is_empty()
                    && segment.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
            })
        })
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipCode {
    NotProtected,
    AlreadyProtected,
    PermissionsPasswordOnly,
    NoSavings,
    AlreadyInTargetFormat,
    RangeEmptyAfterClamping,
    TextAlreadyPresent,
    NoEffectiveChange,
    UnsupportedFormat,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkipReason {
    pub code: SkipCode,
    pub message: String,
    #[serde(default)]
    pub pointer: Option<OutcomePointer>,
}

impl SkipReason {
    pub fn new(code: SkipCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), pointer: None }
    }

    pub fn with_pointer(self, pointer: OutcomePointer) -> Self {
        Self { pointer: Some(pointer), ..self }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", content = "details", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WarningKind {
    SignatureRemoved,
    FontSubstituted { requested_family: String, actual_family: String },
    RenamedToAvoidReplacing { chosen_name: String, final_name: String },
    PrecisionReduced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeWarning {
    #[serde(flatten)]
    pub kind: WarningKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutcomeState {
    Succeeded { output_paths: Vec<PathBuf> },
    Skipped { reason: SkipReason },
    Failed { output_paths: Vec<PathBuf>, error: ToolError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "JobOutcomeWire", try_from = "JobOutcomeWire")]
pub struct JobOutcome {
    pub input_path: PathBuf,
    pub detail: String,
    pub warnings: Vec<OutcomeWarning>,
    pub state: OutcomeState,
}

impl JobOutcome {
    pub fn succeeded(input_path: PathBuf, output_paths: Vec<PathBuf>, detail: impl Into<String>) -> Self {
        Self { input_path, detail: detail.into(), warnings: Vec::new(), state: OutcomeState::Succeeded { output_paths } }
    }

    pub fn skipped(input_path: PathBuf, reason: SkipReason) -> Self {
        Self { input_path, detail: String::new(), warnings: Vec::new(), state: OutcomeState::Skipped { reason } }
    }

    pub fn failure(input_path: PathBuf, error: ToolError) -> Self {
        Self::failure_with_outputs(input_path, Vec::new(), error)
    }

    pub fn failure_with_outputs(input_path: PathBuf, output_paths: Vec<PathBuf>, error: ToolError) -> Self {
        Self { input_path, detail: String::new(), warnings: Vec::new(), state: OutcomeState::Failed { output_paths, error } }
    }

    pub fn with_warning(mut self, warning: OutcomeWarning) -> Self {
        self.warnings.push(warning);
        self
    }

    pub fn output_paths(&self) -> &[PathBuf] {
        match &self.state {
            OutcomeState::Succeeded { output_paths } | OutcomeState::Failed { output_paths, .. } => output_paths,
            OutcomeState::Skipped { .. } => &[],
        }
    }

    pub fn error(&self) -> Option<&ToolError> {
        match &self.state {
            OutcomeState::Failed { error, .. } => Some(error),
            OutcomeState::Succeeded { .. } | OutcomeState::Skipped { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum OutcomeStateTag {
    Succeeded,
    Skipped,
    Failed,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobOutcomeWire {
    input_path: PathBuf,
    state: OutcomeStateTag,
    output_paths: Vec<PathBuf>,
    detail: String,
    failure: Option<ToolError>,
    reason: Option<SkipReason>,
    warnings: Vec<OutcomeWarning>,
}

impl From<JobOutcome> for JobOutcomeWire {
    fn from(outcome: JobOutcome) -> Self {
        let (state, output_paths, failure, reason) = match outcome.state {
            OutcomeState::Succeeded { output_paths } => (OutcomeStateTag::Succeeded, output_paths, None, None),
            OutcomeState::Skipped { reason } => (OutcomeStateTag::Skipped, Vec::new(), None, Some(reason)),
            OutcomeState::Failed { output_paths, error } => (OutcomeStateTag::Failed, output_paths, Some(error), None),
        };
        Self { input_path: outcome.input_path, state, output_paths, detail: outcome.detail, failure, reason, warnings: outcome.warnings }
    }
}

impl TryFrom<JobOutcomeWire> for JobOutcome {
    type Error = String;

    fn try_from(wire: JobOutcomeWire) -> Result<Self, Self::Error> {
        let state = match (wire.state, wire.failure, wire.reason) {
            (OutcomeStateTag::Succeeded, None, None) => OutcomeState::Succeeded { output_paths: wire.output_paths },
            (OutcomeStateTag::Skipped, None, Some(reason)) if wire.output_paths.is_empty() => OutcomeState::Skipped { reason },
            (OutcomeStateTag::Failed, Some(error), None) => OutcomeState::Failed { output_paths: wire.output_paths, error },
            (state, _, _) => return Err(format!("Inconsistent {state:?} job outcome for {}.", wire.input_path.display())),
        };
        Ok(Self { input_path: wire.input_path, detail: wire.detail, warnings: wire.warnings, state })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::office::OfficeProcessor;

    #[test]
    fn ipc_result_uses_the_frontend_field_names() {
        let outcome = JobOutcome::succeeded(PathBuf::from("source.pdf"), vec![PathBuf::from("source-unlocked.pdf")], "PDF Unlocked");
        let value = serde_json::to_value(outcome).unwrap();
        assert_eq!(value["inputPath"], "source.pdf");
        assert_eq!(value["state"], "succeeded");
        assert_eq!(value["outputPaths"][0], "source-unlocked.pdf");
        assert_eq!(value["detail"], "PDF Unlocked");
        assert!(value["failure"].is_null() && value["reason"].is_null());
        assert_eq!(value["warnings"], serde_json::json!([]));
    }

    #[test]
    fn failures_are_structured_for_frontend_handling() {
        let outcome = JobOutcome::failure(
            PathBuf::from("missing.pdf"),
            ToolError::invalid_input("Input file does not exist."),
        );
        let value = serde_json::to_value(outcome).unwrap();
        assert_eq!(value["state"], "failed");
        assert_eq!(value["failure"]["kind"], "invalidInput");
        assert_eq!(value["failure"]["message"], "Input file does not exist.");
        assert!(value["failure"]["pointer"].is_null() && value["reason"].is_null());

        let pointed = JobOutcome::failure(
            PathBuf::from("arabic.pdf"),
            ToolError::unavailable("Install the script pack.").with_pointer(OutcomePointer::Pack { pack_id: "script-pack".into() }),
        );
        let value = serde_json::to_value(pointed).unwrap();
        assert_eq!(value["failure"]["kind"], "unavailable");
        assert_eq!(value["failure"]["pointer"], serde_json::json!({ "kind": "pack", "packId": "script-pack" }));

        let read_only = ToolError::processing("The output folder is read-only. Nothing was saved.").with_pointer(OutcomePointer::OutputFolder);
        assert_eq!(serde_json::to_value(read_only).unwrap()["pointer"], serde_json::json!({ "kind": "outputFolder" }));
    }

    #[test]
    fn partial_failures_keep_published_outputs_in_the_ipc_contract() {
        let output = PathBuf::from("source-frame-1.png");
        let outcome = JobOutcome::failure_with_outputs(
            PathBuf::from("source.gif"),
            vec![output.clone()],
            ToolError::processing("Could not publish the next frame."),
        ).with_warning(OutcomeWarning {
            kind: WarningKind::RenamedToAvoidReplacing { chosen_name: "source-frame-1.png".into(), final_name: "source-frame-1 2.png".into() },
            message: "Renamed to avoid replacing.".into(),
        });
        let value = serde_json::to_value(outcome).unwrap();

        assert_eq!(value["state"], "failed");
        assert_eq!(value["outputPaths"], serde_json::json!([output]));
        assert_eq!(value["failure"]["kind"], "processing");
        assert_eq!(value["failure"]["message"], "Could not publish the next frame.");
        assert_eq!(value["warnings"][0]["code"], "renamedToAvoidReplacing");
        assert_eq!(value["warnings"][0]["details"], serde_json::json!({ "chosenName": "source-frame-1.png", "finalName": "source-frame-1 2.png" }));
    }

    #[test]
    fn skipped_outcomes_carry_a_reason_and_no_outputs() {
        let reason = SkipReason::new(SkipCode::UnsupportedFormat, "Convert HEIC images first.")
            .with_pointer(OutcomePointer::Tool { tool_id: "heic-convert".into() });
        let outcome = JobOutcome::skipped(PathBuf::from("photo.heic"), reason)
            .with_warning(OutcomeWarning { kind: WarningKind::SignatureRemoved, message: "Signature removed.".into() });
        assert!(outcome.output_paths().is_empty() && outcome.error().is_none());

        let value = serde_json::to_value(outcome).unwrap();
        assert_eq!(value["state"], "skipped");
        assert_eq!(value["outputPaths"], serde_json::json!([]));
        assert_eq!(value["detail"], "");
        assert!(value["failure"].is_null());
        assert_eq!(value["reason"], serde_json::json!({
            "code": "unsupportedFormat",
            "message": "Convert HEIC images first.",
            "pointer": { "kind": "tool", "toolId": "heic-convert" },
        }));
        assert_eq!(value["warnings"], serde_json::json!([{ "code": "signatureRemoved", "message": "Signature removed." }]));
    }

    #[test]
    fn seeded_codes_serialize_as_stable_camel_case_strings() {
        let skips = [
            SkipCode::NotProtected, SkipCode::AlreadyProtected, SkipCode::PermissionsPasswordOnly, SkipCode::NoSavings,
            SkipCode::AlreadyInTargetFormat, SkipCode::RangeEmptyAfterClamping, SkipCode::TextAlreadyPresent,
            SkipCode::NoEffectiveChange, SkipCode::UnsupportedFormat, SkipCode::Cancelled,
        ];
        assert_eq!(serde_json::to_value(skips).unwrap(), serde_json::json!([
            "notProtected", "alreadyProtected", "permissionsPasswordOnly", "noSavings", "alreadyInTargetFormat",
            "rangeEmptyAfterClamping", "textAlreadyPresent", "noEffectiveChange", "unsupportedFormat", "cancelled",
        ]));
        let warnings = [
            WarningKind::SignatureRemoved,
            WarningKind::FontSubstituted { requested_family: "A".into(), actual_family: "B".into() },
            WarningKind::RenamedToAvoidReplacing { chosen_name: "a".into(), final_name: "b".into() },
            WarningKind::PrecisionReduced,
        ];
        let codes = warnings.into_iter().map(|kind| serde_json::to_value(kind).unwrap()["code"].clone()).collect::<Vec<_>>();
        assert_eq!(codes, ["signatureRemoved", "fontSubstituted", "renamedToAvoidReplacing", "precisionReduced"]);
    }

    #[test]
    fn job_outcome_samples_match_the_shared_contract_artifact() {
        let shared: serde_json::Value = serde_json::from_str(include_str!("../../../shared/job-outcome-samples.json")).unwrap();
        let samples = vec![
            JobOutcome::succeeded(PathBuf::from("signed.pdf"), vec![PathBuf::from("signed-edited.pdf")], "PDF saved")
                .with_warning(OutcomeWarning { kind: WarningKind::SignatureRemoved, message: "The digital signature is no longer valid.".into() })
                .with_warning(OutcomeWarning {
                    kind: WarningKind::FontSubstituted { requested_family: "Garamond".into(), actual_family: "Times New Roman".into() },
                    message: "Garamond was replaced with Times New Roman.".into(),
                }),
            JobOutcome::skipped(
                PathBuf::from("photo.heic"),
                SkipReason::new(SkipCode::UnsupportedFormat, "Convert HEIC images before compressing them.")
                    .with_pointer(OutcomePointer::Tool { tool_id: "heic-convert".into() }),
            ).with_warning(OutcomeWarning { kind: WarningKind::PrecisionReduced, message: "The 16-bit source would be reduced to 8-bit.".into() }),
            JobOutcome::skipped(PathBuf::from("plain.pdf"), SkipReason::new(SkipCode::NotProtected, "This PDF is not password protected.")),
            JobOutcome::failure_with_outputs(
                PathBuf::from("scan.pdf"),
                vec![PathBuf::from("scan-page-1.png")],
                ToolError::unavailable("Install the script pack to shape this text.")
                    .with_pointer(OutcomePointer::Pack { pack_id: "script-pack".into() }),
            ).with_warning(OutcomeWarning {
                kind: WarningKind::RenamedToAvoidReplacing { chosen_name: "scan-page-1.png".into(), final_name: "scan-page-1 2.png".into() },
                message: "Saved as scan-page-1 2.png so the existing file was not replaced.".into(),
            }),
        ];

        assert_eq!(serde_json::to_value(&samples).unwrap(), shared);
        assert_eq!(serde_json::from_value::<Vec<JobOutcome>>(shared).unwrap(), samples);
    }

    #[test]
    fn inconsistent_outcome_states_are_rejected_when_parsed() {
        let base = serde_json::json!({ "inputPath": "a.pdf", "outputPaths": [], "detail": "", "failure": null, "reason": null, "warnings": [] });
        let with = |patch: serde_json::Value| {
            let mut value = base.clone();
            value.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
            serde_json::from_value::<JobOutcome>(value)
        };
        let reason = serde_json::json!({ "code": "noSavings", "message": "Would not get smaller.", "pointer": null });
        let failure = serde_json::json!({ "kind": "processing", "message": "Broke.", "pointer": null });

        assert!(with(serde_json::json!({ "state": "succeeded" })).is_ok());
        assert!(with(serde_json::json!({ "state": "skipped", "reason": reason })).is_ok());
        assert!(with(serde_json::json!({ "state": "skipped", "reason": reason, "outputPaths": ["a-1.pdf"] })).is_err());
        assert!(with(serde_json::json!({ "state": "skipped" })).is_err());
        assert!(with(serde_json::json!({ "state": "failed" })).is_err());
        assert!(with(serde_json::json!({ "state": "succeeded", "failure": failure })).is_err());
        assert!(with(serde_json::json!({ "state": "failed", "failure": failure, "reason": reason })).is_err());
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
        assert!(outcomes.iter().all(|outcome| outcome.error().is_some_and(|error| matches!(error.kind, ErrorKind::InvalidInput))));
    }

    #[test]
    fn capability_registry_rejects_unsupported_extensions_at_the_boundary() {
        let outcomes = validate_command_inputs("convert_images", &[PathBuf::from("source.pdf")]).unwrap_err();

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].error().is_some_and(|error| error.message.contains("Accepted extensions")));
    }

    #[test]
    fn images_to_pdf_rejects_tiff_at_the_native_boundary() {
        for extension in ["tif", "tiff"] {
            let outcomes = validate_command_inputs("images_to_pdf", &[PathBuf::from(format!("source.{extension}"))]).unwrap_err();

            assert_eq!(outcomes.len(), 1);
            assert!(outcomes[0].error().is_some_and(|error| matches!(error.kind, ErrorKind::InvalidInput)));
            assert!(outcomes[0].error().is_some_and(|error| error.message.contains("Accepted extensions")));
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
        assert!(outcomes[0].error().is_some_and(|error| error.message.contains("at least one page")));
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
            assert_eq!(capability.output_suffix, row["outputSuffix"].as_str().unwrap());
            assert_eq!(capability.many_to_one, row["manyToOne"].as_bool().unwrap());
            assert_eq!(
                capability.many_to_one,
                capability.input_cardinality == InputCardinality::Ordered,
                "{command} many-to-one must match its ordered inputs"
            );
        }
        let mut many_to_one = native_capabilities()
            .iter()
            .filter(|capability| capability.many_to_one)
            .map(|capability| capability.command.as_str())
            .collect::<Vec<_>>();
        many_to_one.sort_unstable();
        assert_eq!(many_to_one, ["create_gif", "images_to_pdf", "merge_pdfs", "process_tiff_pages"]);

        let office = native_capability("remove_password").unwrap();
        assert_eq!(office.accepted_extensions, [".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx"]);
        let pdf_protection = native_capability("protect_pdf").unwrap();
        assert_eq!(pdf_protection.accepted_extensions, [".pdf"]);
        assert!(pdf_protection.native_available);
        let images = native_capability("images_to_pdf").unwrap();
        assert_eq!(images.input_cardinality, InputCardinality::Ordered);
        assert!(!images.accepted_extensions.contains(&".tif".to_string()));
        assert!(!images.accepted_extensions.contains(&".tiff".to_string()));
        assert!(!native_capability("extract_gif_frames").unwrap().supports_preview);
        let pdf = native_capability("pdf_to_text").unwrap();
        assert!(pdf.supports_page_selection && !pdf.supports_preview);
        assert!(!native_capability("ocr_pdf").unwrap().native_available);
        assert!(!native_capability("blur_faces").unwrap().native_available);
        assert!(!native_capability("remove_image_background").unwrap().native_available);
    }

    #[test]
    fn shared_capability_parser_rejects_duplicate_and_unknown_contract_data() {
        let row = r#"{"command":"test","acceptedExtensions":[".pdf"],"inputCardinality":"single","supportsPageSelection":false,"supportsPreview":false,"nativeAvailability":"available","outputSuffix":"-tested","manyToOne":false}"#;
        assert_eq!(parse_capability_registry(&format!("[{row}]")).unwrap()[0].output_suffix, "-tested");
        assert!(parse_capability_registry(&format!("[{row},{row}]")).unwrap_err().contains("Duplicate"));
        let unknown = format!("[{}]", row.replace('}', ",\"unknown\":true}"));
        assert!(parse_capability_registry(&unknown)
            .unwrap_err()
            .contains("unknown field"));
    }

    #[test]
    fn shared_capability_parser_requires_exactly_one_valid_output_suffix() {
        let row = r#"{"command":"test","acceptedExtensions":[".pdf"],"inputCardinality":"single","supportsPageSelection":false,"supportsPreview":false,"nativeAvailability":"available","outputSuffix":"-tested","manyToOne":false}"#;
        let missing_suffix = format!("[{}]", row.replace(",\"outputSuffix\":\"-tested\"", ""));
        assert!(parse_capability_registry(&missing_suffix).unwrap_err().contains("missing field `outputSuffix`"));
        let missing_flag = format!("[{}]", row.replace(",\"manyToOne\":false", ""));
        assert!(parse_capability_registry(&missing_flag).unwrap_err().contains("missing field `manyToOne`"));
        let duplicate_suffix = format!("[{}]", row.replace('}', ",\"outputSuffix\":\"-other\"}"));
        assert!(parse_capability_registry(&duplicate_suffix).unwrap_err().contains("duplicate field `outputSuffix`"));
        let list_suffix = format!("[{}]", row.replace("\"-tested\"", "[\"-a\",\"-b\"]"));
        assert!(parse_capability_registry(&list_suffix).is_err());
        for invalid in ["tested", "-", "-Tested", "--tested", "-tested-", "-tested.pdf"] {
            let raw = format!("[{}]", row.replace("-tested", invalid));
            assert!(parse_capability_registry(&raw).unwrap_err().contains("invalid output suffix"), "{invalid}");
        }
        let flag = format!("[{}]", row.replace("\"manyToOne\":false", "\"manyToOne\":\"yes\""));
        assert!(parse_capability_registry(&flag).is_err());
    }

    #[test]
    fn native_office_protection_accepts_docx_and_xlsx_only() {
        for extension in ["docx", "xlsx", "DOCX", "XLSX"] {
            assert!(OfficeProcessor::supports_protection_extension(extension));
        }
        for extension in ["doc", "xls", "ppt", "pptx"] {
            assert!(!OfficeProcessor::supports_protection_extension(extension));
        }
    }
}
