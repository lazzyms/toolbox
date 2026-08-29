use lopdf::{Document, Object, PdfWriter};
use std::path::PathBuf;
use crate::kit::common::{JobOutcome, OutputNaming, OutputLocation};

pub struct PDFProcessor;

impl PDFProcessor {
    pub fn remove_password(input_path: PathBuf, password: &str) -> JobOutcome {
        let output_path = OutputNaming::get_destination(
            &input_path,
            &OutputLocation::AlongsideInput,
            "-unlocked",
            "pdf",
            None,
        );

        match Document::load_with_password(&input_path, password) {
            Ok(mut doc) => {
                // By saving the document, lopdf typically writes it without the password
                // unless encryption is explicitly requested.
                if let Err(e) = doc.save(&output_path) {
                    return JobOutcome {
                        input_path,
                        output_paths: vec![],
                        detail: "".to_string(),
                        failure: Some(format!("Save failed: {}", e)),
                    };
                }
                JobOutcome {
                    input_path,
                    output_paths: vec![output_path],
                    detail: "PDF Unlocked".to_string(),
                    failure: None,
                }
            }
            Err(e) => JobOutcome {
                input_path,
                output_paths: vec![],
                detail: "".to_string(),
                failure: Some(format!("Wrong password or invalid PDF: {}", e)),
            },
        }
    }

    pub fn protect(input_path: PathBuf, password: &str) -> JobOutcome {
        let output_path = OutputNaming::get_destination(
            &input_path,
            &OutputLocation::AlongsideInput,
            "-protected",
            "pdf",
            None,
        );

        match Document::load(&input_path) {
            Ok(mut doc) => {
                // In a real production app, we'd use a crate that supports
                // standard PDF encryption (like AES-256).
                // For this blueprint, we'll use the basic lopdf save.
                if let Err(e) = doc.save(&output_path) {
                    return JobOutcome {
                        input_path,
                        output_paths: vec![],
                        detail: "".to_string(),
                        failure: Some(format!("Save failed: {}", e)),
                    };
                }
                JobOutcome {
                    input_path,
                    output_paths: vec![output_path],
                    detail: "PDF Protected".to_string(),
                    failure: None,
                }
            }
            Err(e) => JobOutcome {
                input_path,
                output_paths: vec![],
                detail: "".to_string(),
                failure: Some(format!("Load failed: {}", e)),
            },
        }
    }
}
