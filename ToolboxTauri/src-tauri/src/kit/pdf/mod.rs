use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::encryption::{EncryptionState, EncryptionVersion, Permissions};
use lopdf::Document;

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};

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

        match Document::load(&input_path) {
            Ok(mut doc) => {
                if doc.is_encrypted() {
                    // Rejects a wrong password; successful decrypt strips the /Encrypt
                    // trailer entry so the resaved document is unencrypted.
                    if let Err(e) = doc.decrypt(password) {
                        return JobOutcome {
                            input_path,
                            output_paths: vec![],
                            detail: "".to_string(),
                            failure: Some(format!("Wrong password or unsupported encryption: {}", e)),
                        };
                    }
                }
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
                        failure: Some(format!("Save failed: {}", e)),
                    },
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
                let mut file_encryption_key = [0u8; 32];
                if let Err(e) = getrandom::fill(&mut file_encryption_key) {
                    return JobOutcome {
                        input_path,
                        output_paths: vec![],
                        detail: "".to_string(),
                        failure: Some(format!("Failed to generate encryption key: {}", e)),
                    };
                }

                let crypt_filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
                let version = EncryptionVersion::V5 {
                    encrypt_metadata: true,
                    crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), crypt_filter)]),
                    file_encryption_key: &file_encryption_key,
                    stream_filter: b"StdCF".to_vec(),
                    string_filter: b"StdCF".to_vec(),
                    owner_password: password,
                    user_password: password,
                    permissions: Permissions::all(),
                };

                match EncryptionState::try_from(version) {
                    Ok(state) => {
                        if let Err(e) = doc.encrypt(&state) {
                            return JobOutcome {
                                input_path,
                                output_paths: vec![],
                                detail: "".to_string(),
                                failure: Some(format!("Encrypt failed: {}", e)),
                            };
                        }
                        match doc.save(&output_path) {
                            Ok(_) => JobOutcome {
                                input_path,
                                output_paths: vec![output_path],
                                detail: "PDF Protected".to_string(),
                                failure: None,
                            },
                            Err(e) => JobOutcome {
                                input_path,
                                output_paths: vec![],
                                detail: "".to_string(),
                                failure: Some(format!("Save failed: {}", e)),
                            },
                        }
                    }
                    Err(e) => JobOutcome {
                        input_path,
                        output_paths: vec![],
                        detail: "".to_string(),
                        failure: Some(format!("Encryption setup failed: {}", e)),
                    },
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

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;
    use lopdf::Object;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("toolbox_{}_{}", std::process::id(), name))
    }

    fn make_pdf(path: &PathBuf) {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![Object::Real(0.0), Object::Real(0.0), Object::Real(612.0), Object::Real(792.0)],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
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
        let src = temp_path("roundtrip.pdf");
        make_pdf(&src);

        let protected = PDFProcessor::protect(src.clone(), "hunter2");
        assert!(protected.failure.is_none(), "{}", protected.failure.clone().unwrap_or_default());
        assert_eq!(protected.output_paths.len(), 1);

        // protects and saves an encrypted file distinct from the input
        assert_ne!(&protected.output_paths[0], &src);

        let unlocked = PDFProcessor::remove_password(protected.output_paths[0].clone(), "hunter2");
        assert!(unlocked.failure.is_none(), "{}", unlocked.failure.clone().unwrap_or_default());

        // the unlocked copy must load as a plain (non-encrypted) document
        let loaded = Document::load(&unlocked.output_paths[0]);
        assert!(loaded.is_ok());
        assert!(!loaded.unwrap().is_encrypted());

        let _ = std::fs::remove_file(&protected.output_paths[0]);
        let _ = std::fs::remove_file(&unlocked.output_paths[0]);
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn wrong_password_is_rejected() {
        let src = temp_path("wrongpw.pdf");
        make_pdf(&src);

        let protected = PDFProcessor::protect(src.clone(), "correct horse");
        assert!(protected.failure.is_none());

        let unlocked = PDFProcessor::remove_password(protected.output_paths[0].clone(), "battery staple");
        assert!(unlocked.failure.is_some(), "wrong password must fail");
        assert!(unlocked.output_paths.is_empty());

        let _ = std::fs::remove_file(&protected.output_paths[0]);
        let _ = std::fs::remove_file(&src);
    }
}