use std::path::PathBuf;

use crate::kit::common::{JobOutcome, OutputLocation};
use crate::kit::contracts::ToolError;
use crate::kit::office::OfficeProcessor;
use crate::kit::pdf::PDFProcessor;

pub struct PasswordProcessor;

impl PasswordProcessor {
    pub fn supports_extension(extension: &str) -> bool {
        extension.eq_ignore_ascii_case("pdf") || OfficeProcessor::supports_extension(extension)
    }

    pub fn remove_password(input_path: PathBuf, password: &str, output_location: &OutputLocation) -> JobOutcome {
        let Some(extension) = input_path.extension().and_then(|extension| extension.to_str()) else {
            return unsupported(input_path);
        };

        if extension.eq_ignore_ascii_case("pdf") {
            PDFProcessor::remove_password(input_path, password, output_location)
        } else if OfficeProcessor::supports_extension(extension) {
            OfficeProcessor::remove_password(input_path, password, output_location)
        } else {
            unsupported(input_path)
        }
    }
}

fn unsupported(input_path: PathBuf) -> JobOutcome {
    JobOutcome::failure(
        input_path,
        ToolError::invalid_input("Only PDF, Word, Excel, and PowerPoint files can be unlocked."),
    )
}

#[cfg(test)]
mod tests {
    use super::PasswordProcessor;
    use crate::kit::common::OutputLocation;
    use crate::kit::contracts::ErrorKind;
    use std::path::PathBuf;

    #[test]
    fn recognizes_pdf_and_all_supported_office_extensions() {
        for extension in ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx"] {
            assert!(PasswordProcessor::supports_extension(extension), "{extension} should be supported");
        }
    }

    #[test]
    fn unsupported_file_types_fail_before_processing() {
        let input = PathBuf::from("archive.zip");
        let outcome = PasswordProcessor::remove_password(input, "secret", &OutputLocation::AlongsideInput);

        assert!(matches!(outcome.failure.as_ref().map(|error| &error.kind), Some(ErrorKind::InvalidInput)));
        assert!(outcome.output_paths.is_empty());
    }
}
