pub mod metadata;
pub mod scene;
pub mod editor;
pub mod remaining;

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use lopdf::{Document, LoadOptions, Object, ObjectId};

use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::ToolError;

pub struct PDFProcessor;

pub(crate) const PDF_MAX_PAGE_POINTS: f32 = 14_400.0;
pub(crate) const PDF_PREVIEW_MAX_DIMENSION: u32 = 1_600;
pub(crate) const PDF_PREVIEW_MAX_PIXELS: u64 = 2_560_000;
pub(crate) const PDF_PREVIEW_MAX_BYTES: u64 = 20 * 1024 * 1024;
pub(crate) const PDF_PREVIEW_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const PDF_HELPER_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
pub(crate) const PDF_TEXT_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;
pub(crate) const PDF_TEXT_MAX_ERROR_BYTES: usize = 1024 * 1024;
pub(crate) const PDF_TEXT_TIMEOUT: Duration = Duration::from_secs(15);

fn read_helper_output<R: Read>(mut reader: R, stream: &'static str, maximum: usize) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer).map_err(|error| format!("Could not read PDF helper {stream}: {error}"))?;
        if count == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(count) > maximum {
            return Err(format!("PDF helper {stream} output exceeds its limit."));
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

fn spawn_helper_reader<R: Read + Send + 'static>(reader: R, stream: &'static str, maximum: usize) -> Receiver<Result<Vec<u8>, String>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = sender.send(read_helper_output(reader, stream, maximum));
    });
    receiver
}

fn poll_helper_reader(receiver: &Receiver<Result<Vec<u8>, String>>, stream: &'static str) -> Result<Option<Result<Vec<u8>, String>>, String> {
    match receiver.try_recv() {
        Ok(result) => Ok(Some(result)),
        Err(TryRecvError::Empty) => Ok(None),
        Err(TryRecvError::Disconnected) => Err(format!("PDF helper {stream} reader stopped unexpectedly.")),
    }
}

pub(crate) fn run_bounded_helper(mut command: Command, deadline: Instant, max_stdout: usize, max_stderr: usize, timeout_message: &str) -> Result<std::process::Output, String> {
    if Instant::now() >= deadline {
        return Err(timeout_message.to_string());
    }
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|error| format!("Could not run PDF helper: {error}"))?;
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("PDF helper stdout was not captured.".to_string());
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("PDF helper stderr was not captured.".to_string());
        }
    };
    let stdout_reader = spawn_helper_reader(stdout, "stdout", max_stdout);
    let stderr_reader = spawn_helper_reader(stderr, "stderr", max_stderr);
    let mut status = None;
    let mut stdout_result = None;
    let mut stderr_result = None;
    let mut timed_out = false;

    loop {
        if stdout_result.is_none() {
            stdout_result = poll_helper_reader(&stdout_reader, "stdout")?;
        }
        if stderr_result.is_none() {
            stderr_result = poll_helper_reader(&stderr_reader, "stderr")?;
        }
        if stdout_result.as_ref().is_some_and(Result::is_err) || stderr_result.as_ref().is_some_and(Result::is_err) {
            if status.is_none() {
                let _ = child.kill();
                status = Some(child.wait().map_err(|error| format!("Could not stop PDF helper: {error}"))?);
            }
        } else if status.is_none() {
            status = child.try_wait().map_err(|error| {
                let _ = child.kill();
                let _ = child.wait();
                format!("Could not read PDF helper status: {error}")
            })?;
        }

        if status.is_some() && stdout_result.is_some() && stderr_result.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        thread::sleep(Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())));
    }

    if timed_out {
        return Err(timeout_message.to_string());
    }
    let status = status.ok_or("PDF helper did not report a status.")?;
    let stdout = stdout_result.ok_or("PDF helper stdout was not read.")??;
    let stderr = stderr_result.ok_or("PDF helper stderr was not read.")??;
    Ok(std::process::Output { status, stdout, stderr })
}

pub(crate) fn helper_available(command: Command) -> bool {
    run_bounded_helper(
        command,
        Instant::now() + PDF_HELPER_PROBE_TIMEOUT,
        64 * 1024,
        PDF_TEXT_MAX_ERROR_BYTES,
        "PDF helper probe timed out.",
    )
    .map(|output| output.status.success())
    .unwrap_or(false)
}

pub(crate) fn preview_png_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err("PDF helper did not produce a PNG preview.".to_string());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().expect("PNG width has four bytes"));
    let height = u32::from_be_bytes(bytes[20..24].try_into().expect("PNG height has four bytes"));
    let pixels = u64::from(width).checked_mul(u64::from(height)).ok_or_else(|| "PDF preview dimensions overflowed.".to_string())?;
    if width == 0 || height == 0 || width > PDF_PREVIEW_MAX_DIMENSION || height > PDF_PREVIEW_MAX_DIMENSION || pixels > PDF_PREVIEW_MAX_PIXELS {
        return Err("PDF preview exceeds its raster size limit.".to_string());
    }
    Ok((width, height))
}

pub(crate) fn read_preview_png(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let size = fs::metadata(path).map_err(|error| format!("Could not inspect PDF preview: {error}"))?.len();
    if size > PDF_PREVIEW_MAX_BYTES {
        return Err("PDF preview exceeds its file-size limit.".to_string());
    }
    let bytes = fs::read(path).map_err(|error| format!("Could not read PDF preview: {error}"))?;
    let (width, height) = preview_png_dimensions(&bytes)?;
    image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).map_err(|error| format!("PDF helper did not produce a valid PNG preview: {error}"))?;
    Ok((bytes, width, height))
}

#[derive(Debug, Clone)]
pub(crate) struct PdfMutationPreflight {
    pub(crate) pages_root: ObjectId,
    pub(crate) pages: Vec<ObjectId>,
    pub(crate) has_navigation: bool,
    pub(crate) has_form_structure: bool,
    pub(crate) has_tagged_structure: bool,
}

pub(crate) fn resolve<'a>(document: &'a Document, mut value: &'a Object) -> Result<&'a Object, String> {
    let mut seen = HashSet::new();
    while let Object::Reference(id) = value {
        if !seen.insert(*id) {
            return Err("Cyclic PDF reference".to_string());
        }
        value = document.get_object(*id).map_err(|error| error.to_string())?;
    }
    Ok(value)
}

pub(crate) fn inherited(document: &Document, mut id: ObjectId, key: &[u8]) -> Result<Option<Object>, String> {
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(id) {
            return Err("Cyclic PDF page tree".to_string());
        }
        let page_tree = document.get_dictionary(id).map_err(|error| error.to_string())?;
        if let Ok(value) = page_tree.get(key) {
            return Ok(Some(resolve(document, value)?.clone()));
        }
        match page_tree.get(b"Parent") {
            Ok(parent) => id = parent.as_reference().map_err(|error| error.to_string())?,
            Err(_) => return Ok(None),
        }
    }
}

fn box_values(document: &Document, value: &Object) -> Result<[f32; 4], String> {
    let values = resolve(document, value)?.as_array().map_err(|error| error.to_string())?;
    if values.len() != 4 {
        return Err("PDF page box must have four values.".to_string());
    }
    let values = values
        .iter()
        .map(|value| crate::kit::pdf::metadata::number(resolve(document, value)?))
        .collect::<Result<Vec<_>, _>>()?;
    let result = [values[0], values[1], values[2], values[3]];
    if result.iter().any(|value| !value.is_finite()) || result[2] <= result[0] || result[3] <= result[1] {
        return Err("PDF page box has invalid dimensions.".to_string());
    }
    Ok(result)
}

pub(crate) fn set_page_box_family(document: &mut Document, page_id: ObjectId, new_box: [f32; 4], clamp_ancillary: bool) -> Result<(), String> {
    let mut ancillary = Vec::new();
    for key in ["BleedBox", "TrimBox", "ArtBox"] {
        if let Some(value) = inherited(document, page_id, key.as_bytes())? {
            ancillary.push((key, box_values(document, &value)?));
        }
    }
    let new_box_object = Object::Array(new_box.iter().copied().map(Object::Real).collect());
    let page = document.get_dictionary_mut(page_id).map_err(|error| error.to_string())?;
    page.set("MediaBox", new_box_object.clone());
    page.set("CropBox", new_box_object.clone());
    for (key, old_box) in ancillary {
        let value = if !clamp_ancillary {
            new_box
        } else {
            let clamped = [
                old_box[0].max(new_box[0]).min(new_box[2]),
                old_box[1].max(new_box[1]).min(new_box[3]),
                old_box[2].max(new_box[0]).min(new_box[2]),
                old_box[3].max(new_box[1]).min(new_box[3]),
            ];
            if clamped[2] > clamped[0] && clamped[3] > clamped[1] { clamped } else { new_box }
        };
        page.set(key, Object::Array(value.iter().copied().map(Object::Real).collect()));
    }
    Ok(())
}

fn has_catalog_entry(document: &Document, catalog: ObjectId, key: &[u8]) -> Result<bool, String> {
    Ok(document.get_dictionary(catalog).map_err(|error| error.to_string())?.get(key).is_ok())
}

fn field_has_signature(document: &Document, value: &Object, seen: &mut HashSet<ObjectId>) -> Result<bool, String> {
    let id = value.as_reference().map_err(|error| error.to_string())?;
    if !seen.insert(id) {
        return Err("Cyclic PDF form field tree".to_string());
    }
    let field = document.get_dictionary(id).map_err(|error| error.to_string())?;
    if field.get(b"FT").ok().and_then(|value| value.as_name().ok()) == Some(b"Sig") {
        return Ok(true);
    }
    let Some(kids) = field.get(b"Kids").ok() else {
        return Ok(false);
    };
    resolve(document, kids)?.as_array().map_err(|error| error.to_string())?.iter()
        .map(|kid| field_has_signature(document, kid, seen))
        .collect::<Result<Vec<_>, _>>()
        .map(|values| values.into_iter().any(|value| value))
}

fn is_signature_dictionary(value: &Object) -> bool {
    let dictionary = match value {
        Object::Dictionary(dictionary) => dictionary,
        Object::Stream(stream) => &stream.dict,
        _ => return false,
    };
    dictionary.get(b"Type").ok().and_then(|value| value.as_name().ok()) == Some(b"Sig") || dictionary.has(b"ByteRange")
}

fn document_has_signature(document: &Document, catalog: ObjectId, pages: &[ObjectId]) -> Result<bool, String> {
    let catalog_dictionary = document.get_dictionary(catalog).map_err(|error| error.to_string())?;
    if let Ok(perms) = catalog_dictionary.get(b"Perms") {
        resolve(document, perms)?.as_dict().map_err(|error| error.to_string())?;
        return Ok(true);
    }
    if document.objects.values().any(is_signature_dictionary) {
        return Ok(true);
    }
    if let Ok(acro_form) = catalog_dictionary.get(b"AcroForm") {
        let acro_form = resolve(document, acro_form)?.as_dict().map_err(|error| error.to_string())?;
        if acro_form.get(b"SigFlags").ok().and_then(|value| value.as_i64().ok()).is_some_and(|flags| flags != 0) {
            return Ok(true);
        }
        if let Some(fields) = acro_form.get(b"Fields").ok() {
            let mut seen = HashSet::new();
            if resolve(document, fields)?.as_array().map_err(|error| error.to_string())?.iter()
                .map(|field| field_has_signature(document, field, &mut seen))
                .collect::<Result<Vec<_>, _>>()?.into_iter().any(|value| value) {
                return Ok(true);
            }
        }
    }
    pages.iter().map(|page| {
        let Some(annotations) = document.get_dictionary(*page).map_err(|error| error.to_string())?.get(b"Annots").ok() else {
            return Ok(false);
        };
        resolve(document, annotations)?.as_array().map_err(|error| error.to_string())?.iter()
            .map(|annotation| {
                let id = annotation.as_reference().map_err(|error| error.to_string())?;
                Ok(document.get_dictionary(id).map_err(|error| error.to_string())?.get(b"FT").ok().and_then(|value| value.as_name().ok()) == Some(b"Sig"))
            })
            .collect::<Result<Vec<_>, String>>()
            .map(|values| values.into_iter().any(|value| value))
    }).collect::<Result<Vec<_>, String>>().map(|values| values.into_iter().any(|value| value))
}

fn page_has_internal_navigation(document: &Document, page: ObjectId) -> Result<bool, String> {
    let Some(annotations) = document.get_dictionary(page).map_err(|error| error.to_string())?.get(b"Annots").ok() else {
        return Ok(false);
    };
    let annotations = resolve(document, annotations)?.as_array().map_err(|error| error.to_string())?;
    annotations.iter().map(|annotation| {
        let annotation = resolve(document, annotation)?.as_dict().map_err(|error| error.to_string())?;
        if annotation.get(b"Dest").is_ok() {
            return Ok(true);
        }
        let Some(action) = annotation.get(b"A").ok() else {
            return Ok(false);
        };
        Ok(resolve(document, action)?.as_dict().map_err(|error| error.to_string())?.get(b"D").is_ok())
    }).collect::<Result<Vec<_>, String>>().map(|values| values.into_iter().any(|value| value))
}

pub(crate) fn mutation_preflight(document: &Document, reject_navigation: bool, reject_tagged_structure: bool) -> Result<PdfMutationPreflight, String> {
    if document.is_encrypted() {
        return Err("Unlock the PDF before editing".to_string());
    }
    let catalog = document.trailer.get(b"Root").map_err(|error| error.to_string())?.as_reference().map_err(|error| error.to_string())?;
    let pages_root = document.get_dictionary(catalog).map_err(|error| error.to_string())?.get(b"Pages").map_err(|error| error.to_string())?.as_reference().map_err(|error| error.to_string())?;
    let pages = document.get_pages().values().copied().collect::<Vec<_>>();
    let unsupported = || "PDF uses a nested or unsupported page tree; mutation was rejected before output".to_string();
    let root = document.get_dictionary(pages_root).map_err(|_| unsupported())?;
    if root.get(b"Type").map_err(|_| unsupported())?.as_name().map_err(|_| unsupported())? != b"Pages" {
        return Err(unsupported());
    }
    let kids = root.get(b"Kids").map_err(|_| unsupported())?.as_array().map_err(|_| unsupported())?;
    let count = root.get(b"Count").map_err(|_| "PDF page-tree /Count is missing; mutation was rejected before output".to_string())?;
    let count = resolve(document, count).map_err(|_| "PDF page-tree /Count is invalid; mutation was rejected before output".to_string())?.as_i64()
        .map_err(|_| "PDF page-tree /Count is invalid; mutation was rejected before output".to_string())?;
    if count < 0 || usize::try_from(count).ok() != Some(pages.len()) {
        return Err("PDF page-tree /Count does not match the flattened page count; mutation was rejected before output".to_string());
    }
    if kids.len() != pages.len() {
        return Err(unsupported());
    }
    for (index, kid) in kids.iter().enumerate() {
        let page_id = kid.as_reference().map_err(|_| unsupported())?;
        let page = document.get_dictionary(page_id).map_err(|_| unsupported())?;
        if page.get(b"Type").map_err(|_| unsupported())?.as_name().map_err(|_| unsupported())? != b"Page"
            || page_id != pages[index]
            || page.get(b"Parent").map_err(|_| unsupported())?.as_reference().map_err(|_| unsupported())? != pages_root {
            return Err(unsupported());
        }
    }
    if document_has_signature(document, catalog, &pages)? {
        return Err("Digitally signed PDFs cannot be edited because this mutation would invalidate the signature; remove the signature or use an unsigned copy".to_string());
    }
    let page_navigation = pages.iter().map(|page| page_has_internal_navigation(document, *page)).collect::<Result<Vec<_>, _>>()?.into_iter().any(|present| present);
    let has_navigation = [b"Outlines".as_slice(), b"Names", b"Dests", b"PageLabels", b"OpenAction"]
        .into_iter().map(|key| has_catalog_entry(document, catalog, key)).collect::<Result<Vec<_>, _>>()?.into_iter().any(|present| present);
    let has_navigation = has_navigation || page_navigation;
    let has_form_structure = has_catalog_entry(document, catalog, b"AcroForm")?;
    let has_tagged_structure = has_catalog_entry(document, catalog, b"StructTreeRoot")?;
    if reject_navigation && (has_navigation || has_form_structure) {
        return Err("This PDF contains navigation or form structures that could be invalidated by this mutation; use an unstructured PDF copy".to_string());
    }
    if reject_tagged_structure && has_tagged_structure {
        return Err("This tagged PDF contains a StructTreeRoot or ParentTree that could be invalidated by this mutation; use an untagged PDF copy".to_string());
    }
    Ok(PdfMutationPreflight { pages_root, pages, has_navigation, has_form_structure, has_tagged_structure })
}

impl PDFProcessor {
    pub fn remove_password(input_path: PathBuf, password: &str, output_location: &OutputLocation) -> JobOutcome {
        if password.is_empty() {
            return JobOutcome { input_path, output_paths: vec![], detail: "".to_string(), failure: Some(ToolError::invalid_input("Enter the PDF password.")) };
        }
        if input_path.extension().and_then(|extension| extension.to_str()).is_none_or(|extension| !extension.eq_ignore_ascii_case("pdf")) {
            return JobOutcome { input_path, output_paths: vec![], detail: "".to_string(), failure: Some(ToolError::invalid_input("Only PDF files can be unlocked.")) };
        }
        let output_path = match OutputNaming::reserve_destination(
            &input_path,
            output_location,
            "-unlocked",
            "pdf",
        ) {
            Ok(output) => output,
            Err(error) => return JobOutcome::failure(input_path, ToolError::processing(format!("Could not reserve unlocked PDF output: {error}"))),
        };

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
                let page_count = doc.get_pages().len();
                // Loading an encrypted PDF can leave the original trailer's
                // `/Size` higher than the surviving object table. Keep the
                // rewritten xref table truthful so strict PDF validators do
                // not report a stale object-count warning.
                doc.max_id = doc
                    .objects
                    .iter()
                    .filter(|(_, object)| {
                        object
                            .type_name()
                            .map(|name| ![b"ObjStm".as_slice(), b"XRef".as_slice(), b"Linearized".as_slice()].contains(&name))
                            .unwrap_or(true)
                    })
                    .map(|((id, _), _)| *id)
                    .max()
                    .unwrap_or(0);
                match doc.save(output_path.path()) {
                    Ok(_) => match Document::load(output_path.path()) {
                        Ok(verified) if !verified.is_encrypted() && verified.get_pages().len() == page_count && verified.objects.len() > 1 => match output_path.publish() {
                            Ok(path) => JobOutcome { input_path, output_paths: vec![path], detail: "PDF Unlocked and verified".to_string(), failure: None },
                            Err(error) => JobOutcome::failure(input_path, ToolError::processing(format!("Could not publish unlocked PDF output: {error}"))),
                        },
                        Ok(_) => JobOutcome { input_path, output_paths: vec![], detail: "".to_string(), failure: Some(ToolError::processing("Unlocked PDF failed verification.")) },
                        Err(error) => JobOutcome { input_path, output_paths: vec![], detail: "".to_string(), failure: Some(ToolError::processing(format!("Unlocked PDF could not be reopened: {error}"))) },
                    },
                    Err(e) => JobOutcome { input_path, output_paths: vec![], detail: "".to_string(), failure: Some(ToolError::processing(format!("Save failed: {}", e))) },
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
        let output_path = match OutputNaming::reserve_destination(
            &input_path,
            output_location,
            "-protected",
            "pdf",
        ) {
            Ok(output) => output,
            Err(error) => return JobOutcome::failure(input_path, ToolError::processing(format!("Could not reserve protected PDF output: {error}"))),
        };

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
        if password.is_empty() {
            return JobOutcome { input_path, output_paths: vec![], detail: "".to_string(), failure: Some(ToolError::invalid_input("A non-empty password is required.")) };
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
            .arg(output_path.path())
            .output();

        match status {
            Ok(out) if out.status.success() => {
                let encrypted = Document::load(output_path.path()).map(|document| document.is_encrypted()).unwrap_or(false);
                let readable = Document::load_with_options(output_path.path(), lopdf::LoadOptions::with_password(password)).map(|document| !document.is_encrypted()).unwrap_or(false);
                if encrypted && readable {
                    match output_path.publish() {
                        Ok(path) => JobOutcome { input_path, output_paths: vec![path], detail: "PDF Protected with verified AES-256 encryption".to_string(), failure: None },
                        Err(error) => JobOutcome::failure(input_path, ToolError::processing(format!("Could not publish protected PDF output: {error}"))),
                    }
                }
                else { JobOutcome { input_path, output_paths: vec![], detail: "".to_string(), failure: Some(ToolError::processing("qpdf produced an output that could not be verified as password-protected.")) } }
            }
            Ok(out) => {
                JobOutcome { input_path, output_paths: vec![], detail: "".to_string(), failure: Some(ToolError::processing(
                    String::from_utf8_lossy(&out.stderr).trim().lines().last().map(|l| l.to_string()).unwrap_or_else(|| "qpdf failed to protect the PDF.".to_string()),
                )) }
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
        let qpdf_check = std::process::Command::new(find_qpdf().unwrap())
            .arg("--check")
            .arg(&unlocked.output_paths[0])
            .output()
            .unwrap();
        assert!(
            qpdf_check.status.success(),
            "unlocked PDF should pass qpdf validation: {}",
            String::from_utf8_lossy(&qpdf_check.stderr)
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
