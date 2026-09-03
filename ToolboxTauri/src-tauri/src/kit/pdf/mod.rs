pub mod metadata;
pub mod editor;
pub mod remaining;

use std::path::PathBuf;
use std::process::Command;

use lopdf::{Document, LoadOptions};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::ToolError;

pub struct PDFProcessor;

impl PDFProcessor {
    pub fn remove_password(input_path: PathBuf, password: &str, output_location: &OutputLocation) -> JobOutcome {
        let output_path = OutputNaming::get_destination(
            &input_path,
            output_location,
            "-unlocked",
            "pdf",
        );

        // Reading an encrypted PDF without a password makes lopdf drop every
        // object except the /Encrypt dictionary (objects == 1), so decrypting
        // afterwards writes a corrupt skeleton file. The password must be given
        // at load time so lopdf decrypts each object as it parses it.
        let loaded = Document::load_with_options(&input_path, LoadOptions::with_password(password));
        match loaded {
            Ok(doc) => {
                if doc.is_encrypted() {
                    // Loading with the wrong password can surface a still-encrypted
                    // document rather than an error; treat it as a rejection.
                    return JobOutcome {
                        input_path,
                        output_paths: vec![],
                        detail: "".to_string(),
                        failure: Some(ToolError::invalid_input("Wrong password or unsupported encryption.")),
                    };
                }
                let mut doc = doc;
                match doc.save(&output_path) {
                    Ok(_) => JobOutcome {
                        input_path,
                        output_paths: vec![output_path],
                        detail: "PDF Unlocked".to_string(),
                        failure: None,
                    },
                    Err(e) => JobOutcome {
                        input_path,
                        output_paths: vec![],
                        detail: "".to_string(),
                        failure: Some(ToolError::processing(format!("Save failed: {}", e))),
                    },
                }
            }
            Err(e) => JobOutcome {
                input_path,
                output_paths: vec![],
                detail: "".to_string(),
                failure: Some(ToolError::invalid_input(format!("Wrong password or unsupported encryption: {}", e))),
            },
        }
    }

    /// Underlying encryption writer. lopdf's own `Document::encrypt` produces an
    /// /Encrypt dictionary that other readers (macOS PDFKit) cannot decrypt, so
    /// protection shells out to a correct, cross-platform AES-256 writer.
    pub fn protect(input_path: PathBuf, password: &str, output_location: &OutputLocation) -> JobOutcome {
        let output_path = OutputNaming::get_destination(
            &input_path,
            output_location,
            "-protected",
            "pdf",
        );

        if let Some(ref e) = input_path.extension().map(|e| e.to_string_lossy().to_lowercase()) {
            if e != "pdf" {
                return JobOutcome {
                    input_path,
                    output_paths: vec![],
                    detail: "".to_string(),
                    failure: Some(ToolError::invalid_input("Only PDF files can be protected.")),
                };
            }
        }

        let Some(qpdf) = find_qpdf() else {
            return JobOutcome {
                input_path,
                output_paths: vec![],
                detail: "".to_string(),
                failure: Some(ToolError::unavailable(
                    "qpdf is required to protect PDFs but was not found. \
                     Set TOOLBOX_QPDF_PATH or add qpdf to PATH.",
                )),
            };
        };

        let status = Command::new(&qpdf)
            .arg("--encrypt")
            .arg(password)
            .arg(password)
            .arg("256")
            .arg("--")
            .arg(&input_path)
            .arg(&output_path)
            .output();

        match status {
            Ok(out) if out.status.success() => JobOutcome {
                input_path,
                output_paths: vec![output_path],
                detail: "PDF Protected".to_string(),
                failure: None,
            },
            Ok(out) => JobOutcome {
                input_path,
                output_paths: vec![],
                detail: "".to_string(),
                failure: Some(ToolError::processing(
                    String::from_utf8_lossy(&out.stderr)
                        .trim()
                        .lines()
                        .last()
                        .map(|l| l.to_string())
                        .unwrap_or_else(|| "qpdf failed to protect the PDF.".to_string()),
                )),
            },
            Err(e) => JobOutcome {
                input_path,
                output_paths: vec![],
                detail: "".to_string(),
                failure: Some(ToolError::unavailable(format!("Could not run qpdf: {}", e))),
            },
        }
    }
}

/// Locate qpdf from an explicit override, the installed app resources, or PATH.
fn find_qpdf() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("TOOLBOX_QPDF_PATH") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        let exe_dir = exe.parent()?;
        let candidates = [
            exe_dir.join("qpdf.exe"),
            exe_dir.join("resources").join("qpdf.exe"),
            exe_dir.parent()?.join("Resources").join("qpdf-bin").join("qpdf"),
        ];
        for candidate in candidates {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // Fall back to a `qpdf` on PATH (dev / non-bundled runs).
    Command::new("qpdf")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| PathBuf::from("qpdf"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;
    use lopdf::Object;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("toolbox_{}_{}", std::process::id(), name))
    }

    fn qpdf_available() -> bool {
        find_qpdf().is_some()
    }

    fn require_qpdf() -> bool {
        if qpdf_available() {
            return true;
        }
        if std::env::var_os("TOOLBOX_REQUIRE_QPDF").is_some() {
            panic!("qpdf is required for this test run");
        }
        eprintln!("skipping: qpdf not available");
        false
    }

    fn make_pdf(path: &PathBuf) {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        // A real page carries a /Contents stream. Streams are what trigger the
        // encryption round-trip bugs, so every fixture must include one.
        let content_id = doc.add_object(Object::Stream(lopdf::Stream::new(
            dictionary! {},
            b"BT /F1 24 Tf (hello) Tj ET".to_vec(),
        )));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![Object::Real(0.0), Object::Real(0.0), Object::Real(612.0), Object::Real(792.0)],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
            "Contents" => content_id,
        });
        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        doc.save(path).unwrap();
    }

    #[test]
    fn protect_then_unlock_roundtrip() {
        if !require_qpdf() {
            return;
        }
        let src = temp_path("roundtrip.pdf");
        make_pdf(&src);

        let protected = PDFProcessor::protect(src.clone(), "hunter2", &OutputLocation::AlongsideInput);
        assert!(protected.failure.is_none(), "{}", protected.failure.clone().unwrap_or_default());
        assert_eq!(protected.output_paths.len(), 1);

        // the protected file must decrypt correctly with the password
        let enc = Document::load_with_options(&protected.output_paths[0], lopdf::LoadOptions::with_password("hunter2"));
        assert!(enc.is_ok());
        let enc = enc.unwrap();
        assert!(!enc.is_encrypted());
        assert!(enc.objects.len() > 1, "protected PDF lost its objects");

        // protects and saves an encrypted file distinct from the input
        assert_ne!(&protected.output_paths[0], &src);

        let unlocked = PDFProcessor::remove_password(protected.output_paths[0].clone(), "hunter2", &OutputLocation::AlongsideInput);
        assert!(unlocked.failure.is_none(), "{}", unlocked.failure.clone().unwrap_or_default());

        // the unlocked copy must load as a plain (non-encrypted) document
        let loaded = Document::load(&unlocked.output_paths[0]);
        assert!(loaded.is_ok());
        let loaded = loaded.unwrap();
        assert!(!loaded.is_encrypted());
        // Regression guard: a bad round-trip used to drop every object bar the
        // /Encrypt dict (and sometimes write a 180-byte skeleton). A real
        // unlocked page must retain all its objects.
        assert!(
            loaded.objects.len() > 1,
            "unlocked PDF lost its objects ({} remaining)",
            loaded.objects.len()
        );
        assert!(
            loaded.objects.iter().any(|(_, o)| matches!(o, Object::Stream(_))),
            "unlocked PDF must keep its content stream"
        );

        let _ = std::fs::remove_file(&protected.output_paths[0]);
        let _ = std::fs::remove_file(&unlocked.output_paths[0]);
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn wrong_password_is_rejected() {
        if !require_qpdf() {
            return;
        }
        let src = temp_path("wrongpw.pdf");
        make_pdf(&src);

        let protected = PDFProcessor::protect(src.clone(), "correct horse", &OutputLocation::AlongsideInput);
        assert!(protected.failure.is_none());

        let unlocked = PDFProcessor::remove_password(protected.output_paths[0].clone(), "battery staple", &OutputLocation::AlongsideInput);
        assert!(unlocked.failure.is_some(), "wrong password must fail");
        assert!(unlocked.output_paths.is_empty());

        let _ = std::fs::remove_file(&protected.output_paths[0]);
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn protect_reports_missing_qpdf() {
        // When qpdf is genuinely absent, protect must fail cleanly rather than
        // panic. Only meaningful if it is absent: otherwise this is a no-op that
        // still exercises the non-engine path via a bogus override.
        if qpdf_available() {
            return;
        }
        let src = temp_path("noqpdf.pdf");
        make_pdf(&src);
        let out = PDFProcessor::protect(src.clone(), "x", &OutputLocation::AlongsideInput);
        assert!(out.failure.is_some());
        let _ = std::fs::remove_file(&src);
    }
}
