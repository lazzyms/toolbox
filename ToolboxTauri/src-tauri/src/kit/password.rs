use std::path::{Path, PathBuf};

use lopdf::Document;

use crate::kit::common::{JobOutcome, OutputLocation};
use crate::kit::contracts::{SkipCode, SkipReason, ToolError};
use crate::kit::office::{self, OfficeProcessor};
use crate::kit::pdf::PDFProcessor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtectionState {
    NotProtected,
    OpenPassword,
    PermissionsOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtectedFormat {
    Pdf,
    Docx,
    Xlsx,
    Pptx,
    Doc,
    Xls,
    Ppt,
}

impl ProtectedFormat {
    pub fn from_extension(extension: &str) -> Option<Self> {
        Some(match extension.to_ascii_lowercase().as_str() {
            "pdf" => Self::Pdf,
            "docx" => Self::Docx,
            "xlsx" => Self::Xlsx,
            "pptx" => Self::Pptx,
            "doc" => Self::Doc,
            "xls" => Self::Xls,
            "ppt" => Self::Ppt,
            _ => return None,
        })
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension().and_then(|extension| extension.to_str()).and_then(Self::from_extension)
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
            Self::Doc => "doc",
            Self::Xls => "xls",
            Self::Ppt => "ppt",
        }
    }
}

pub fn probe(bytes: &[u8], format: ProtectedFormat) -> Result<ProtectionState, ToolError> {
    if format == ProtectedFormat::Pdf {
        return probe_pdf(bytes);
    }
    Ok(if office::is_encrypted(bytes, format.extension())? {
        ProtectionState::OpenPassword
    } else {
        ProtectionState::NotProtected
    })
}

fn probe_pdf(bytes: &[u8]) -> Result<ProtectionState, ToolError> {
    let document = Document::load_mem(bytes)
        .map_err(|error| ToolError::invalid_input(format!("Unsupported or damaged PDF: {error}")))?;
    Ok(if document.is_encrypted() {
        ProtectionState::OpenPassword
    } else if document.was_encrypted() {
        ProtectionState::PermissionsOnly
    } else {
        ProtectionState::NotProtected
    })
}

pub const ALREADY_PASSWORD_PROTECTED: &str = "Already password-protected. Unlock it first, then lock it again.";
pub const NOT_PASSWORD_PROTECTED: &str = "Not password-protected. Nothing to unlock, so no copy was made.";
pub const PERMISSIONS_ONLY: &str = "Opens without a password. It only has print and copy restrictions. To clear them, use Remove restrictions in PDF Editor.";

pub const WRONG_PASSWORD: &str = "The password didn't open this file.";
pub const UNSUPPORTED_ENCRYPTION: &str = "Uses an encryption type Toolbox can't open. The password may be right.";
pub const UNSUPPORTED_LEGACY_ENCRYPTION: &str = "Uses an older encryption type Toolbox can't open. The password may be right.";
pub const LEGACY_PASSWORD_TOO_LONG: &str = "Older Office files only take passwords up to 15 characters, and this one is longer. Toolbox can't try it.";
pub const LEGACY_PPT_PICTURES: &str = "Legacy .ppt files with pictures can't be unlocked yet. The file isn't damaged.";
pub const UNLOCK_NOT_VERIFIED: &str = "The unlocked copy didn't pass verification, so nothing was saved.";

impl ProtectedFormat {
    fn noun(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Docx | Self::Doc => "Word document",
            Self::Xlsx | Self::Xls => "Excel workbook",
            Self::Pptx | Self::Ppt => "PowerPoint presentation",
        }
    }
}

pub fn unreadable(format: ProtectedFormat) -> ToolError {
    ToolError::processing(format!(
        "Not a readable {}. The file may be damaged or incomplete. Nothing was saved.",
        format.noun()
    ))
}

pub fn legacy_lock_unavailable(format: ProtectedFormat) -> ToolError {
    let (modern, app) = match format {
        ProtectedFormat::Doc => (ProtectedFormat::Docx, "Word"),
        ProtectedFormat::Xls => (ProtectedFormat::Xlsx, "Excel"),
        _ => (ProtectedFormat::Pptx, "PowerPoint"),
    };
    ToolError::unavailable(format!(
        "Legacy .{} files can't be locked. Save it as .{} in {app}, then add it again.",
        format.extension(),
        modern.extension()
    ))
}

pub fn lock_skip_reason(state: ProtectionState) -> Option<SkipReason> {
    match state {
        ProtectionState::OpenPassword => Some(SkipReason::new(SkipCode::AlreadyProtected, ALREADY_PASSWORD_PROTECTED)),
        ProtectionState::NotProtected | ProtectionState::PermissionsOnly => None,
    }
}

pub fn unlock_skip_reason(state: ProtectionState) -> Option<SkipReason> {
    match state {
        ProtectionState::OpenPassword => None,
        ProtectionState::NotProtected => Some(SkipReason::new(SkipCode::NotProtected, NOT_PASSWORD_PROTECTED)),
        ProtectionState::PermissionsOnly => Some(SkipReason::new(SkipCode::PermissionsPasswordOnly, PERMISSIONS_ONLY)),
    }
}

pub struct PasswordProcessor;

impl PasswordProcessor {
    pub fn supports_extension(extension: &str) -> bool {
        ProtectedFormat::from_extension(extension).is_some()
    }

    pub fn remove_password(input_path: PathBuf, password: &str, output_location: &OutputLocation) -> JobOutcome {
        match ProtectedFormat::from_path(&input_path) {
            Some(ProtectedFormat::Pdf) => PDFProcessor::remove_password(input_path, password, output_location),
            Some(_) => OfficeProcessor::remove_password(input_path, password, output_location),
            None => unsupported(input_path),
        }
    }

    pub fn protect(input_path: PathBuf, password: &str, output_location: &OutputLocation) -> JobOutcome {
        match ProtectedFormat::from_path(&input_path) {
            Some(ProtectedFormat::Pdf) => PDFProcessor::protect(input_path, password, output_location),
            Some(_) => OfficeProcessor::protect(input_path, password, output_location),
            None => JobOutcome::failure(input_path, ToolError::invalid_input("Only PDF, DOCX, XLSX and PPTX files can be locked.")),
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
pub(crate) mod tests {
    use super::{probe, PasswordProcessor, ProtectedFormat, ProtectionState};
    use crate::kit::common::OutputLocation;
    use crate::kit::contracts::ErrorKind;
    use crate::kit::office::tests::{cfb_with_streams, biff_record, minimal_ooxml_package, ppt_fixture, ppt_plain_fixture};
    use crate::kit::office_agile::encrypt_ooxml;
    use lopdf::{dictionary, Document, EncryptionState, EncryptionVersion, Object, Permissions, Stream, StringFormat};
    use std::path::PathBuf;

    #[test]
    fn recognizes_pdf_and_all_supported_office_extensions() {
        for extension in ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "PDF", "Docx"] {
            assert!(PasswordProcessor::supports_extension(extension), "{extension} should be supported");
        }
        for extension in ["zip", "odt", "txt", ""] {
            assert!(!PasswordProcessor::supports_extension(extension), "{extension} should not be supported");
        }
    }

    #[test]
    fn unsupported_file_types_fail_before_processing() {
        let input = PathBuf::from("archive.zip");
        let outcome = PasswordProcessor::remove_password(input, "secret", &OutputLocation::AlongsideInput);

        assert!(matches!(outcome.error().map(|error| &error.kind), Some(ErrorKind::InvalidInput)));
        assert!(outcome.output_paths().is_empty());
    }

    pub(crate) fn pdf(encryption: Option<(&str, &str)>) -> Vec<u8> {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let content_id = document.add_object(Stream::new(dictionary! {}, b"BT ET".to_vec()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1 }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        if let Some((owner_password, user_password)) = encryption {
            document.trailer.set("ID", Object::Array(vec![
                Object::String(b"toolbox-probe-id-1".to_vec(), StringFormat::Literal),
                Object::String(b"toolbox-probe-id-2".to_vec(), StringFormat::Literal),
            ]));
            let state = EncryptionState::try_from(EncryptionVersion::V2 {
                document: &document,
                owner_password,
                user_password,
                key_length: 128,
                permissions: Permissions::PRINTABLE,
            }).unwrap();
            document.encrypt(&state).unwrap();
        }
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    pub(crate) fn word_document(flags: u16) -> Vec<u8> {
        let mut word = vec![0u8; 100];
        word[10..12].copy_from_slice(&flags.to_le_bytes());
        cfb_with_streams(&[("/WordDocument", &word), ("/0Table", &[0x11; 16])])
    }

    pub(crate) fn workbook(encrypted: bool) -> Vec<u8> {
        let mut stream = biff_record(0x0809, &[0, 6, 5, 0]);
        if encrypted {
            stream.extend_from_slice(&biff_record(0x002F, &[0, 0, 0x34, 0x12]));
        }
        stream.extend_from_slice(&biff_record(0x000A, &[]));
        cfb_with_streams(&[("/Workbook", &stream)])
    }

    #[test]
    fn probe_reports_the_protection_state_of_every_format() {
        let cases: Vec<(&str, Vec<u8>, ProtectedFormat, ProtectionState)> = vec![
            ("plain pdf", pdf(None), ProtectedFormat::Pdf, ProtectionState::NotProtected),
            ("open-password pdf", pdf(Some(("owner", "user"))), ProtectedFormat::Pdf, ProtectionState::OpenPassword),
            ("permissions-only pdf", pdf(Some(("owner", ""))), ProtectedFormat::Pdf, ProtectionState::PermissionsOnly),
            ("plain docx", minimal_ooxml_package("docx"), ProtectedFormat::Docx, ProtectionState::NotProtected),
            ("encrypted docx", encrypt_ooxml(&minimal_ooxml_package("docx"), "secret").unwrap(), ProtectedFormat::Docx, ProtectionState::OpenPassword),
            ("plain xlsx", minimal_ooxml_package("xlsx"), ProtectedFormat::Xlsx, ProtectionState::NotProtected),
            ("encrypted xlsx", encrypt_ooxml(&minimal_ooxml_package("xlsx"), "secret").unwrap(), ProtectedFormat::Xlsx, ProtectionState::OpenPassword),
            ("plain pptx", minimal_ooxml_package("pptx"), ProtectedFormat::Pptx, ProtectionState::NotProtected),
            ("encrypted pptx", encrypt_ooxml(&minimal_ooxml_package("pptx"), "secret").unwrap(), ProtectedFormat::Pptx, ProtectionState::OpenPassword),
            ("plain doc", word_document(0), ProtectedFormat::Doc, ProtectionState::NotProtected),
            ("encrypted doc", word_document(0x0100), ProtectedFormat::Doc, ProtectionState::OpenPassword),
            ("obfuscated doc", word_document(0x8100), ProtectedFormat::Doc, ProtectionState::OpenPassword),
            ("plain xls", workbook(false), ProtectedFormat::Xls, ProtectionState::NotProtected),
            ("encrypted xls", workbook(true), ProtectedFormat::Xls, ProtectionState::OpenPassword),
            ("plain ppt", ppt_plain_fixture(), ProtectedFormat::Ppt, ProtectionState::NotProtected),
            ("encrypted ppt", ppt_fixture(true, false), ProtectedFormat::Ppt, ProtectionState::OpenPassword),
        ];
        for (name, bytes, format, expected) in cases {
            assert_eq!(probe(&bytes, format).unwrap_or_else(|error| panic!("{name}: {}", error.message)), expected, "{name}");
        }
    }

    #[test]
    fn probe_refuses_bytes_that_are_not_the_named_format() {
        let cases = [
            (b"not a pdf".to_vec(), ProtectedFormat::Pdf),
            (b"not an office file".to_vec(), ProtectedFormat::Docx),
            (minimal_ooxml_package("xlsx"), ProtectedFormat::Docx),
            (cfb_with_streams(&[("/Other", b"data")]), ProtectedFormat::Pptx),
            (cfb_with_streams(&[("/Other", b"data")]), ProtectedFormat::Doc),
            (cfb_with_streams(&[("/WordDocument", &[0u8; 8])]), ProtectedFormat::Doc),
            (cfb_with_streams(&[("/Workbook", &[0x09, 0x08, 0xFF])]), ProtectedFormat::Xls),
            (cfb_with_streams(&[("/Current User", &[0u8; 4])]), ProtectedFormat::Ppt),
            (b"PK\x03\x04truncated".to_vec(), ProtectedFormat::Xls),
        ];
        for (bytes, format) in cases {
            let error = probe(&bytes, format).expect_err("unreadable bytes have no protection state");
            assert_eq!(error.kind, ErrorKind::InvalidInput, "{format:?}: {}", error.message);
        }
    }
}
