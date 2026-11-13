use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use md5::{Digest, Md5};
use quick_xml::escape::escape;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::{NsReader, Writer};
use serde::{Deserialize, Serialize};

use super::document_info::{self, DocumentProperty, PdfDate, PdfProperties, DC, RDF, XMP, XMP_MAX_BYTES};
use super::qpdf;
use crate::kit::common::{OutputNaming, TempDir};
use crate::kit::contracts::{ErrorKind, JobOutcome, LimitKind, OpenPasswords, OutputLocation, SkipCode, SkipReason, ToolError};

const NO_CHANGE_ASKED: &str = "Every field is set to Keep, so nothing would change. Choose Set or Clear for a field.";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(tag = "action", content = "value", rename_all = "camelCase")]
pub enum FieldEdit {
    #[default]
    Keep,
    Set(String),
    Clear,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertiesPatch {
    #[serde(default)]
    pub title: FieldEdit,
    #[serde(default)]
    pub author: FieldEdit,
    #[serde(default)]
    pub subject: FieldEdit,
    #[serde(default)]
    pub creation_date: FieldEdit,
    #[serde(default)]
    pub modification_date: FieldEdit,
}

impl PropertiesPatch {
    /// The fields to change, with each set value trimmed and each date in one canonical form.
    fn edits(&self) -> Result<Vec<(DocumentProperty, FieldEdit)>, String> {
        let fields = [
            (DocumentProperty::Title, &self.title),
            (DocumentProperty::Author, &self.author),
            (DocumentProperty::Subject, &self.subject),
            (DocumentProperty::CreationDate, &self.creation_date),
            (DocumentProperty::ModificationDate, &self.modification_date),
        ];
        let mut edits = Vec::new();
        for (property, edit) in fields {
            match edit {
                FieldEdit::Keep => {}
                FieldEdit::Clear => edits.push((property, FieldEdit::Clear)),
                FieldEdit::Set(value) => {
                    let value = value.trim();
                    if value.is_empty() {
                        return Err(format!("Enter {} {}, or choose Keep or Clear.", if property == DocumentProperty::Author { "an" } else { "a" }, label(property)));
                    }
                    let value = if property.is_date() { zoned_date(value).ok_or_else(|| format!("The {} isn't a real date. Use YYYY-MM-DD HH:MM, like 2026-02-28 09:00.", label(property)))?.to_string() } else { value.to_string() };
                    edits.push((property, FieldEdit::Set(value)));
                }
            }
        }
        if edits.is_empty() {
            return Err(NO_CHANGE_ASKED.to_string());
        }
        Ok(edits)
    }
}

/// A date with a time and a time zone, as `2026-02-28T09:00:00+05:30`.
fn zoned_date(value: &str) -> Option<PdfDate> {
    document_info::parse_xmp_date(value).filter(|date| date.offset_minutes.is_some())
}

fn pdf_date(date: &PdfDate) -> String {
    let zone = match date.offset_minutes {
        None => String::new(),
        Some(0) => "Z".to_string(),
        Some(offset) => format!("{}{:02}'{:02}'", if offset < 0 { '-' } else { '+' }, offset.unsigned_abs() / 60, offset.unsigned_abs() % 60),
    };
    format!("D:{:04}{:02}{:02}{:02}{:02}{:02}{zone}", date.year, date.month, date.day, date.hour, date.minute, date.second)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditPropertiesRequest {
    pub paths: Vec<PathBuf>,
    pub patch: PropertiesPatch,
    pub output_location: OutputLocation,
    #[serde(flatten)]
    pub passwords: OpenPasswords,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadPropertiesRequest {
    pub paths: Vec<PathBuf>,
    #[serde(flatten)]
    pub passwords: OpenPasswords,
}

fn label(property: DocumentProperty) -> &'static str {
    match property {
        DocumentProperty::Title => "title",
        DocumentProperty::Author => "author",
        DocumentProperty::Subject => "subject",
        DocumentProperty::CreationDate => "creation date",
        DocumentProperty::ModificationDate => "modification date",
        _ => "property",
    }
}

fn text(properties: &PdfProperties, property: DocumentProperty) -> Option<String> {
    match property {
        DocumentProperty::Title => properties.title.clone(),
        DocumentProperty::Author => properties.author.clone(),
        DocumentProperty::Subject => properties.subject.clone(),
        DocumentProperty::CreationDate => properties.creation_date.map(|date| date.to_string()),
        DocumentProperty::ModificationDate => properties.modification_date.map(|date| date.to_string()),
        _ => None,
    }
}

/// Rejects a patch that could change nothing, or holds an empty value or an invalid date, before any file is opened.
pub fn validate(request: &EditPropertiesRequest) -> Result<(), Vec<JobOutcome>> {
    match request.patch.edits() {
        Ok(_) => Ok(()),
        Err(message) => Err(request.paths.iter().cloned().map(|path| JobOutcome::failure(path, ToolError::invalid_input(message.clone()))).collect()),
    }
}

/// Opens and decrypts a PDF, with the password that opened it: empty for a file that needs none
/// to open, which a file with only restrictions still opens with.
fn open<'a>(input: &Path, password: Option<&'a str>) -> Result<(Document, &'a str), ToolError> {
    let bytes = document_info::read_bytes(input)?;
    match (document_info::load(&bytes, Some("")), password) {
        (Err(error), Some(password)) if error.kind == ErrorKind::WrongPassword => Ok((document_info::load(&bytes, Some(password))?, password)),
        (opened, _) => Ok((opened?, "")),
    }
}

#[derive(Serialize)]
struct CurrentProperties {
    #[serde(flatten)]
    properties: PdfProperties,
    signed: bool,
}

/// Each field's current value: the document information value, or the XMP one when the dictionary has none.
pub fn read_job(request: &ReadPropertiesRequest, input: PathBuf) -> JobOutcome {
    let document = match open(&input, request.passwords.password_for(&input)) {
        Ok((document, _)) => document,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let merged = document_info::from_document(&document).merged();
    let signed = super::has_signed_signature_field(&document);
    match serde_json::to_string(&CurrentProperties { properties: merged, signed }) {
        Ok(detail) => JobOutcome::succeeded(input, Vec::new(), detail),
        Err(error) => JobOutcome::failure(input, ToolError::processing(format!("Could not read the properties: {error}"))),
    }
}

pub fn edit_job(request: &EditPropertiesRequest, input: PathBuf) -> JobOutcome {
    match edit(request, &input) {
        Ok(Edited::Saved(path, detail, signed)) => super::note_signature_removed(JobOutcome::succeeded(input, vec![path], detail), signed),
        Ok(Edited::Unchanged(message)) => JobOutcome::skipped(input, SkipReason::new(SkipCode::NoEffectiveChange, message)),
        Err(error) => JobOutcome::failure(input, error),
    }
}

enum Edited {
    Saved(PathBuf, String, bool),
    Unchanged(String),
}

fn edit(request: &EditPropertiesRequest, input: &Path) -> Result<Edited, ToolError> {
    let edits = request.patch.edits().map_err(ToolError::invalid_input)?;
    let (mut document, password) = open(input, request.passwords.password_for(input))?;
    let input_encryption = encryption(&document);
    let page_count = document.get_pages().len();
    let current = document_info::from_document(&document);
    let packet = xmp_packet(&document)?;
    if packet.is_some() && current.xmp.is_none() {
        return Err(ToolError::processing("The XMP metadata could not be read, so this PDF was left alone. Nothing was saved."));
    }
    let present_in_xmp = match &packet {
        Some((_, bytes)) => rewrite_xmp(bytes, &edits).map_err(unreadable_xmp)?.1,
        None => Vec::new(),
    };
    let info_keys = info_dictionary(&document).map(|info| edits.iter().filter(|(property, _)| info.has(property.info_key().as_bytes())).map(|(property, _)| *property).collect::<Vec<_>>()).unwrap_or_default();
    let xmp_values = current.xmp.as_ref().map(|xmp| &xmp.values);

    let mut effective = Vec::new();
    let mut unchanged = Vec::new();
    for (property, edit) in edits {
        let changes = match &edit {
            FieldEdit::Set(value) => text(&current.info, property).as_deref() != Some(value.as_str()) || xmp_values.is_some_and(|xmp| text(xmp, property).as_deref() != Some(value.as_str())),
            _ => info_keys.contains(&property) || present_in_xmp.contains(&property),
        };
        if changes {
            effective.push((property, edit));
        } else {
            unchanged.push(match &edit {
                FieldEdit::Set(value) => format!("the {} is already \"{value}\"", label(property)),
                _ => format!("there's no {}", label(property)),
            });
        }
    }
    if effective.is_empty() {
        return Ok(Edited::Unchanged(format!("No effective change: {}. No copy was made.", sentence_list(&unchanged))));
    }

    let signed = super::remove_signature_values(&mut document).map_err(ToolError::processing)?;
    apply_info(&mut document, &effective).map_err(ToolError::processing)?;
    match packet {
        Some((id, bytes)) => {
            let (rewritten, _) = rewrite_xmp(&bytes, &effective).map_err(unreadable_xmp)?;
            let Ok(Object::Stream(stream)) = document.get_object_mut(id) else { return Err(unreadable_xmp("the metadata stream moved".to_string())) };
            stream.set_plain_content(rewritten);
        }
        None if effective.iter().any(|(_, edit)| matches!(edit, FieldEdit::Set(_))) => create_xmp(&mut document).map_err(ToolError::processing)?,
        None => {}
    }
    let bytes = save_with_stable_id(&mut document).map_err(|error| ToolError::processing(format!("Could not write the edited PDF: {error}")))?;
    check_output(&Document::load_mem(&bytes).map_err(|_| not_checked("it could not be reopened"))?, page_count, &effective)?;

    let output = OutputNaming::reserve_destination(input, &request.output_location, "-edited", "pdf").map_err(|error| ToolError::processing(format!("Could not reserve the edited PDF: {error}")))?;
    let changes = effective.iter().map(|(property, edit)| format!("{} {}", label(*property), if matches!(edit, FieldEdit::Set(_)) { "set" } else { "cleared" })).collect::<Vec<_>>().join(", ");
    let mut detail = format!("{}{}.", changes[..1].to_uppercase(), &changes[1..]);
    match &input_encryption {
        None => std::fs::write(output.path(), &bytes).map_err(|error| ToolError::processing(format!("Could not write the edited PDF: {error}")))?,
        Some(expected) => {
            encrypt_like(&bytes, page_count, input, password, output.path())?;
            let saved = std::fs::read(output.path()).map_err(|error| ToolError::processing(format!("Could not write the edited PDF: {error}")))?;
            let saved = document_info::load(&saved, Some(password)).map_err(|_| not_checked("it no longer opens with the same password"))?;
            if encryption(&saved).as_ref() != Some(expected) {
                return Err(not_checked("its encryption changed"));
            }
            check_output(&saved, page_count, &effective)?;
            detail.push_str(if password.is_empty() { " It keeps its restrictions." } else { " It keeps its open password." });
        }
    }
    let path = output.publish().map_err(|error| ToolError::processing(format!("Could not publish the edited PDF: {error}")))?;
    Ok(Edited::Saved(path, detail, signed))
}

/// What decides who can open a PDF and what they may do with it: the same values mean the same
/// passwords and the same permissions.
#[derive(PartialEq)]
struct Encryption {
    version: i64,
    revision: i64,
    key_length: Option<usize>,
    encrypt_metadata: bool,
    permissions: u64,
    owner_value: Vec<u8>,
    user_value: Vec<u8>,
    file_encryption_key: Vec<u8>,
}

fn encryption(document: &Document) -> Option<Encryption> {
    document.encryption_state.as_ref().map(|state| Encryption {
        version: state.version(),
        revision: state.revision(),
        key_length: state.key_length(),
        encrypt_metadata: state.encrypt_metadata(),
        permissions: state.permissions().bits(),
        owner_value: state.owner_value().to_vec(),
        user_value: state.user_value().to_vec(),
        file_encryption_key: state.file_encryption_key().to_vec(),
    })
}

/// Writes `plain` to `output` encrypted exactly as `input` is. lopdf saves only plain PDFs, so
/// qpdf copies the input's encryption, reading the password that opens it from its standard
/// input. The plain copy lives only in a private temporary folder, removed when this returns.
fn encrypt_like(plain: &[u8], page_count: usize, input: &Path, password: &str, output: &Path) -> Result<(), ToolError> {
    if password.contains(['\n', '\r']) {
        return Err(ToolError::invalid_input(qpdf::PASSWORD_LINE_BREAK));
    }
    let helper = crate::kit::resources::resolve_helper(&crate::kit::resources::QPDF).map_err(ToolError::unavailable)?;
    let temp = TempDir::new().map_err(|error| ToolError::processing(format!("Could not prepare the edited PDF: {error}")))?;
    let plain_path = temp.0.join("edited.pdf");
    std::fs::write(&plain_path, plain).map_err(|error| ToolError::processing(format!("Could not prepare the edited PDF: {error}")))?;
    let mut copy_encryption = OsString::from("--copy-encryption=");
    copy_encryption.push(input);
    let mut command = Command::new(helper);
    command.arg(qpdf::PASSWORDS_FROM_STDIN).arg(&plain_path).arg(copy_encryption).arg(output);
    let limit = super::pdf_lock_time_limit(page_count);
    let timed_out = format!("Keeping the password took longer than {} seconds and was stopped. Nothing was saved.", limit.as_secs());
    let result = match qpdf::run_with_passwords(command, &[("--encryption-file-password", password)], Instant::now() + limit, &timed_out) {
        Ok(result) => result,
        Err(message) if message == timed_out => return Err(ToolError::limit_exceeded(LimitKind::Time, message)),
        Err(message) => return Err(ToolError::unavailable(message)),
    };
    if !matches!(result.status.code(), Some(0 | 3)) {
        let detail = String::from_utf8_lossy(&result.stderr).trim().lines().last().map(str::to_string).unwrap_or_else(|| "qpdf could not keep the password.".to_string());
        return Err(ToolError::processing(format!("{detail} Nothing was saved.")));
    }
    Ok(())
}

fn sentence_list(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn unreadable_xmp(error: String) -> ToolError {
    ToolError::processing(format!("The XMP metadata could not be updated ({error}). Nothing was saved."))
}

fn info_dictionary(document: &Document) -> Option<&Dictionary> {
    let info = document.trailer.get(b"Info").ok()?;
    super::resolve(document, info).ok()?.as_dict().ok()
}

fn xmp_packet(document: &Document) -> Result<Option<(ObjectId, Vec<u8>)>, ToolError> {
    let Ok(metadata) = document.catalog().and_then(|catalog| catalog.get(b"Metadata")) else { return Ok(None) };
    let Object::Reference(id) = metadata else { return Err(unreadable_xmp("the metadata is not a separate stream".to_string())) };
    let packet = document.get_object(*id).ok().and_then(|object| object.as_stream().ok()).and_then(|stream| stream.decompressed_content_with_limit(XMP_MAX_BYTES).ok());
    match packet {
        Some(packet) => Ok(Some((*id, packet))),
        None => Err(ToolError::processing("The XMP metadata could not be read, so this PDF was left alone. Nothing was saved.")),
    }
}

fn info_value(property: DocumentProperty, value: &str) -> Object {
    match document_info::parse_xmp_date(value) {
        Some(date) if property.is_date() => Object::String(pdf_date(&date).into_bytes(), StringFormat::Literal),
        _ => info_string(value),
    }
}

fn info_string(value: &str) -> Object {
    if value.bytes().all(|byte| (0x20..=0x7E).contains(&byte)) {
        return Object::String(value.as_bytes().to_vec(), StringFormat::Literal);
    }
    let mut bytes = vec![0xFE, 0xFF];
    bytes.extend(value.encode_utf16().flat_map(u16::to_be_bytes));
    Object::String(bytes, StringFormat::Hexadecimal)
}

fn apply_info(document: &mut Document, edits: &[(DocumentProperty, FieldEdit)]) -> Result<(), String> {
    let sets_value = edits.iter().any(|(_, edit)| matches!(edit, FieldEdit::Set(_)));
    let info_id = match document.trailer.get(b"Info").ok().cloned() {
        Some(Object::Reference(id)) => Some(id),
        Some(Object::Dictionary(_)) => None,
        Some(_) => return Err("The document information dictionary could not be read.".to_string()),
        None if sets_value => {
            let id = document.add_object(Dictionary::new());
            document.trailer.set("Info", id);
            Some(id)
        }
        None => return Ok(()),
    };
    let info = match info_id {
        Some(id) => document.get_object_mut(id).and_then(Object::as_dict_mut),
        None => document.trailer.get_mut(b"Info").and_then(Object::as_dict_mut),
    }
    .map_err(|_| "The document information dictionary could not be read.".to_string())?;
    let mut replaced = Vec::new();
    for (property, edit) in edits {
        let key = property.info_key().as_bytes();
        if let Some(Object::Reference(id)) = info.remove(key) {
            replaced.push(id);
        }
        if let FieldEdit::Set(value) = edit {
            info.set(key, info_value(*property, value));
        }
    }
    for id in replaced {
        let orphan = matches!(document.objects.get(&id), Some(Object::String(..)));
        if orphan && !document.objects.values().any(|object| refers_to(object, id)) && !document.trailer.iter().any(|(_, object)| refers_to(object, id)) {
            document.objects.remove(&id);
        }
    }
    Ok(())
}

fn refers_to(object: &Object, id: ObjectId) -> bool {
    match object {
        Object::Reference(reference) => *reference == id,
        Object::Array(items) => items.iter().any(|item| refers_to(item, id)),
        Object::Dictionary(dictionary) => dictionary.iter().any(|(_, value)| refers_to(value, id)),
        Object::Stream(stream) => stream.dict.iter().any(|(_, value)| refers_to(value, id)),
        _ => false,
    }
}

fn xmp_property(property: DocumentProperty, value: &str) -> String {
    if property.is_date() {
        let local = String::from_utf8_lossy(property.xmp_name().1);
        return format!("<xmp:{local} xmlns:xmp=\"{}\">{}</xmp:{local}>", String::from_utf8_lossy(XMP), escape(value));
    }
    let (local, container, language) = match property {
        DocumentProperty::Author => ("creator", "Seq", ""),
        DocumentProperty::Subject => ("description", "Alt", " xml:lang=\"x-default\""),
        _ => ("title", "Alt", " xml:lang=\"x-default\""),
    };
    format!(
        "<dc:{local} xmlns:dc=\"{}\" xmlns:rdf=\"{}\"><rdf:{container}><rdf:li{language}>{}</rdf:li></rdf:{container}></dc:{local}>",
        String::from_utf8_lossy(DC),
        String::from_utf8_lossy(RDF),
        escape(value)
    )
}

/// Drops every edited property from the top-level descriptions and appends the set values to the first one.
/// Returns the new packet and the edited properties the packet held.
fn rewrite_xmp(packet: &[u8], edits: &[(DocumentProperty, FieldEdit)]) -> Result<(Vec<u8>, Vec<DocumentProperty>), String> {
    let target = |namespace: &[u8], local: &[u8]| edits.iter().map(|(property, _)| *property).find(|property| property.xmp_name() == (namespace, local));
    let insertion: String = edits.iter().filter_map(|(property, edit)| match edit {
        FieldEdit::Set(value) => Some(xmp_property(*property, value)),
        _ => None,
    }).collect();
    let mut reader = NsReader::from_reader(packet);
    let mut writer = Writer::new(Vec::with_capacity(packet.len() + insertion.len()));
    let mut stack: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut description_depth = None;
    let mut insert_at = None;
    let mut inserted = insertion.is_empty();
    let mut skipping = 0usize;
    let mut present = Vec::new();
    loop {
        let (namespace, event) = reader.read_resolved_event().map_err(|error| error.to_string())?;
        let namespace = match namespace {
            ResolveResult::Bound(namespace) => namespace.as_ref().to_vec(),
            _ => Vec::new(),
        };
        if skipping > 0 {
            match event {
                Event::Start(_) => skipping += 1,
                Event::End(_) => skipping -= 1,
                Event::Eof => return Err("the packet ends before its elements are closed".to_string()),
                _ => {}
            }
            continue;
        }
        let write = |writer: &mut Writer<Vec<u8>>, event: Event<'_>| writer.write_event(event).map_err(|error| error.to_string());
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let local = element.local_name().as_ref().to_vec();
                let depth = stack.len();
                if namespace == RDF && local == b"Description" && stack.last().is_some_and(|(parent_namespace, parent)| parent_namespace == RDF && parent == b"RDF") {
                    let mut kept = BytesStart::new(String::from_utf8_lossy(element.name().as_ref()).into_owned());
                    let mut dropped = false;
                    for attribute in element.attributes() {
                        let attribute = attribute.map_err(|error| error.to_string())?;
                        let (attribute_namespace, attribute_local) = reader.resolver().resolve_attribute(attribute.key);
                        let property = match attribute_namespace {
                            ResolveResult::Bound(attribute_namespace) => target(attribute_namespace.as_ref(), attribute_local.as_ref()),
                            _ => None,
                        };
                        match property {
                            Some(property) => {
                                present.push(property);
                                dropped = true;
                            }
                            None => kept.push_attribute(attribute),
                        }
                    }
                    let element = if dropped { kept } else { element.borrow() };
                    if !inserted && empty {
                        write(&mut writer, Event::Start(element.borrow()))?;
                        writer.get_mut().extend_from_slice(insertion.as_bytes());
                        write(&mut writer, Event::End(element.to_end()))?;
                        inserted = true;
                    } else if !inserted {
                        write(&mut writer, Event::Start(element.borrow()))?;
                        insert_at = Some(depth);
                        inserted = true;
                    } else {
                        write(&mut writer, if empty { Event::Empty(element.borrow()) } else { Event::Start(element.borrow()) })?;
                    }
                    if !empty {
                        description_depth = Some(depth);
                        stack.push((namespace, local));
                    }
                    continue;
                }
                if description_depth.is_some_and(|description| depth == description + 1) {
                    if let Some(property) = target(&namespace, &local) {
                        present.push(property);
                        if !empty {
                            skipping = 1;
                        }
                        continue;
                    }
                }
                write(&mut writer, event.borrow())?;
                if !empty {
                    stack.push((namespace, local));
                }
            }
            Event::End(_) => {
                let closed = stack.pop();
                let depth = stack.len();
                if description_depth == Some(depth) {
                    description_depth = None;
                }
                if insert_at == Some(depth) {
                    writer.get_mut().extend_from_slice(insertion.as_bytes());
                    insert_at = None;
                } else if !inserted && closed.is_some_and(|(closed_namespace, closed_local)| closed_namespace == RDF && closed_local == b"RDF") {
                    let description = format!("<rdf:Description xmlns:rdf=\"{}\" rdf:about=\"\">{insertion}</rdf:Description>", String::from_utf8_lossy(RDF));
                    writer.get_mut().extend_from_slice(description.as_bytes());
                    inserted = true;
                }
                write(&mut writer, event)?;
            }
            Event::Eof if stack.is_empty() => break,
            Event::Eof => return Err("the packet ends before its elements are closed".to_string()),
            event => write(&mut writer, event)?,
        }
    }
    if !inserted {
        return Err("the packet has no RDF block".to_string());
    }
    Ok((writer.into_inner(), present))
}

fn create_xmp(document: &mut Document) -> Result<(), String> {
    let info = document_info::from_document(document).info;
    let edits = [DocumentProperty::Title, DocumentProperty::Author, DocumentProperty::Subject, DocumentProperty::CreationDate, DocumentProperty::ModificationDate]
        .into_iter()
        .filter_map(|property| text(&info, property).map(|value| (property, FieldEdit::Set(value))))
        .collect::<Vec<_>>();
    let empty = format!(
        "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"{}\"><rdf:Description rdf:about=\"\"></rdf:Description></rdf:RDF></x:xmpmeta>\n<?xpacket end=\"w\"?>",
        String::from_utf8_lossy(RDF)
    );
    let (packet, _) = rewrite_xmp(empty.as_bytes(), &edits)?;
    let mut dictionary = Dictionary::new();
    dictionary.set("Type", Object::Name(b"Metadata".to_vec()));
    dictionary.set("Subtype", Object::Name(b"XML".to_vec()));
    let id = document.add_object(Stream::new(dictionary, packet));
    document.catalog_mut().map_err(|error| error.to_string())?.set("Metadata", id);
    Ok(())
}

/// Keeps the first file identifier and derives the second from the saved bytes, so the same edit always writes the same file.
fn save_with_stable_id(document: &mut Document) -> Result<Vec<u8>, lopdf::Error> {
    let first = document.trailer.get(b"ID").ok().and_then(|id| id.as_array().ok()).and_then(|ids| ids.first()).and_then(|id| id.as_str().ok()).map(<[u8]>::to_vec);
    let identifier = |first: Vec<u8>, second: Vec<u8>| Object::Array(vec![Object::String(first, StringFormat::Hexadecimal), Object::String(second, StringFormat::Hexadecimal)]);
    document.trailer.set("ID", identifier(first.clone().unwrap_or_else(|| vec![0; 16]), vec![0; 16]));
    let mut bytes = Vec::new();
    document.save_to(&mut bytes)?;
    let digest = Md5::digest(&bytes).to_vec();
    document.trailer.set("ID", identifier(first.unwrap_or_else(|| digest.clone()), digest));
    bytes.clear();
    document.save_to(&mut bytes)?;
    Ok(bytes)
}

fn not_checked(reason: &str) -> ToolError {
    ToolError::processing(format!("The edited PDF did not check out ({reason}). Nothing was saved."))
}

fn check_output(document: &Document, page_count: usize, edits: &[(DocumentProperty, FieldEdit)]) -> Result<(), ToolError> {
    if document.get_pages().len() != page_count {
        return Err(not_checked("its page count changed"));
    }
    let saved = document_info::from_document(document);
    for (property, edit) in edits {
        let expected = match edit {
            FieldEdit::Set(value) => Some(value.as_str()),
            _ => None,
        };
        let xmp = saved.xmp.as_ref().map(|xmp| text(&xmp.values, *property));
        let xmp_matches = match (expected, xmp) {
            (Some(_), None) => false,
            (_, None) => true,
            (expected, Some(found)) => found.as_deref() == expected,
        };
        if text(&saved.info, *property).as_deref() != expected || !xmp_matches {
            return Err(not_checked(&format!("its {} did not change", label(*property))));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::common::batch_runner::{BatchRunner, RunEvents};
    use crate::kit::contracts::OutcomeState;
    use lopdf::{dictionary, EncryptionState, EncryptionVersion, Permissions};
    use std::sync::Arc;

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("toolbox-properties-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }

        fn names(&self) -> Vec<String> {
            let mut names = std::fs::read_dir(&self.0).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
            names.sort();
            names
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn literal(text: &str) -> Object {
        Object::String(text.as_bytes().to_vec(), StringFormat::Literal)
    }

    fn xmp(description: &str) -> String {
        format!(
            r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">{description}</rdf:RDF></x:xmpmeta>
<?xpacket end="w"?>"#
        )
    }

    const CONTENT: &[u8] = b"BT /F1 12 Tf 72 720 Td (Quarterly figures) Tj ET";

    fn pdf(info: Option<Dictionary>, xmp: Option<&str>, encryption: Option<(&str, &str)>) -> Vec<u8> {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let content_id = document.add_object(Stream::new(dictionary! {}, CONTENT.to_vec()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1 }.into());
        let attachment_id = document.add_object(Stream::new(dictionary! { "Type" => "EmbeddedFile" }, b"attached notes".to_vec()));
        let file_spec_id = document.add_object(dictionary! { "Type" => "Filespec", "F" => literal("notes.txt"), "EF" => dictionary! { "F" => attachment_id } });
        let mut catalog = dictionary! {
            "Type" => "Catalog", "Pages" => pages_id,
            "Names" => dictionary! { "EmbeddedFiles" => dictionary! { "Names" => vec![literal("notes.txt"), Object::Reference(file_spec_id)] } },
        };
        if let Some(xmp) = xmp {
            let mut stream = Stream::new(dictionary! { "Type" => "Metadata", "Subtype" => "XML" }, xmp.as_bytes().to_vec());
            stream.compress().unwrap();
            catalog.set("Metadata", document.add_object(stream));
        }
        let catalog_id = document.add_object(catalog);
        document.trailer.set("Root", catalog_id);
        document.trailer.set("ID", Object::Array(vec![literal("toolbox-first-id"), literal("toolbox-second-id")]));
        if let Some(info) = info {
            let info_id = document.add_object(info);
            document.trailer.set("Info", info_id);
        }
        if let Some((owner_password, user_password)) = encryption {
            let state = EncryptionState::try_from(EncryptionVersion::V2 { document: &document, owner_password, user_password, key_length: 128, permissions: Permissions::PRINTABLE }).unwrap();
            document.encrypt(&state).unwrap();
        }
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    fn patch(title: FieldEdit, author: FieldEdit, subject: FieldEdit) -> PropertiesPatch {
        PropertiesPatch { title, author, subject, ..Default::default() }
    }

    fn dates(creation_date: FieldEdit, modification_date: FieldEdit) -> PropertiesPatch {
        PropertiesPatch { creation_date, modification_date, ..Default::default() }
    }

    fn set(value: &str) -> FieldEdit {
        FieldEdit::Set(value.to_string())
    }

    fn run(paths: &[PathBuf], patch: PropertiesPatch) -> Vec<JobOutcome> {
        run_with(paths, patch, OpenPasswords::default())
    }

    fn passwords(batch: &str, overrides: &[(&Path, &str)]) -> OpenPasswords {
        OpenPasswords { password: Some(batch.to_string()), password_overrides: overrides.iter().map(|(path, password)| (path.to_path_buf(), password.to_string())).collect() }
    }

    fn run_with(paths: &[PathBuf], patch: PropertiesPatch, passwords: OpenPasswords) -> Vec<JobOutcome> {
        let request = Arc::new(EditPropertiesRequest { paths: paths.to_vec(), patch, output_location: OutputLocation::AlongsideInput, passwords });
        if let Err(outcomes) = validate(&request) {
            return outcomes;
        }
        BatchRunner::run(&RunEvents::silent(), "edit_pdf_properties", paths.to_vec(), &OutputLocation::AlongsideInput, &request, edit_job)
    }

    fn saved(outcome: &JobOutcome) -> (Document, document_info::PdfDocumentInfo) {
        let path = &outcome.output_paths()[0];
        let document = Document::load(path).unwrap();
        let info = document_info::from_document(&document);
        (document, info)
    }

    fn stream_content(document: &Document, predicate: impl Fn(&Stream) -> bool) -> Vec<u8> {
        document.objects.values().filter_map(|object| object.as_stream().ok()).find(|stream| predicate(stream)).map(|stream| stream.content.clone()).unwrap()
    }

    #[test]
    fn deserializes_the_wire_patch() {
        let request: EditPropertiesRequest = serde_json::from_str(r#"{"paths":["a.pdf"],"patch":{"title":{"action":"set","value":"Q3"},"author":{"action":"clear"}},"outputLocation":"alongsideInput"}"#).unwrap();
        assert_eq!(request.patch, patch(set("Q3"), FieldEdit::Clear, FieldEdit::Keep));
        let with_passwords: EditPropertiesRequest = serde_json::from_str(r#"{"paths":["a.pdf","b.pdf"],"patch":{"title":{"action":"clear"}},"outputLocation":"alongsideInput","password":"batch-secret","passwordOverrides":{"b.pdf":"own-secret"}}"#).unwrap();
        assert_eq!(with_passwords.passwords.password_for(Path::new("a.pdf")), Some("batch-secret"));
        assert_eq!(with_passwords.passwords.password_for(Path::new("b.pdf")), Some("own-secret"));
        let printed = format!("{with_passwords:?}");
        assert!(!printed.contains("batch-secret") && !printed.contains("own-secret"), "{printed}");
    }

    #[test]
    fn rejects_an_all_keep_patch_and_an_empty_set_value_before_opening_files() {
        let sandbox = Sandbox::new("validate");
        let input = sandbox.file("report.pdf", &pdf(None, None, None));
        let outcomes = run(&[input.clone()], PropertiesPatch::default());
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::InvalidInput);
        assert_eq!(outcomes[0].error().unwrap().message, NO_CHANGE_ASKED);
        let empty = run(&[input], patch(FieldEdit::Keep, set("   "), FieldEdit::Keep));
        assert_eq!(empty[0].error().unwrap().message, "Enter an author, or choose Keep or Clear.");
        assert_eq!(sandbox.names(), vec!["report.pdf"]);
    }

    #[test]
    fn sets_and_clears_info_and_xmp_while_keeping_each_files_own_values() {
        let sandbox = Sandbox::new("batch");
        let description = r#"<rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:CreatorTool="Writer"><dc:title><rdf:Alt><rdf:li xml:lang="x-default">Old title</rdf:li></rdf:Alt></dc:title><dc:creator><rdf:Seq><rdf:li>Ann</rdf:li></rdf:Seq></dc:creator><dc:description><rdf:Alt><rdf:li xml:lang="x-default">Kept subject</rdf:li></rdf:Alt></dc:description></rdf:Description>"#;
        let first = sandbox.file("first.pdf", &pdf(Some(dictionary! { "Title" => literal("Old title"), "Author" => literal("Ann"), "Subject" => literal("Kept subject") }), Some(&xmp(description)), None));
        let second = sandbox.file("second.pdf", &pdf(Some(dictionary! { "Author" => literal("Bo"), "Subject" => literal("Other subject") }), None, None));
        let originals = [std::fs::read(&first).unwrap(), std::fs::read(&second).unwrap()];

        let outcomes = run(&[first.clone(), second.clone()], patch(set("  Q3 report — final "), FieldEdit::Clear, FieldEdit::Keep));

        assert_eq!(std::fs::read(&first).unwrap(), originals[0]);
        assert_eq!(std::fs::read(&second).unwrap(), originals[1]);
        assert_eq!(outcomes[0].detail, "Title set, author cleared.");
        assert_eq!(outcomes[0].output_paths()[0], sandbox.0.join("first-edited.pdf"));
        let (document, info) = saved(&outcomes[0]);
        assert_eq!(info.info.title.as_deref(), Some("Q3 report — final"));
        assert_eq!(info.info.author, None);
        assert_eq!(info.info.subject.as_deref(), Some("Kept subject"));
        let xmp = info.xmp.unwrap();
        assert_eq!(xmp.values.title.as_deref(), Some("Q3 report — final"));
        assert_eq!(xmp.values.author, None);
        assert_eq!(xmp.values.subject.as_deref(), Some("Kept subject"));
        assert_eq!(xmp.values.creator.as_deref(), Some("Writer"));
        assert!(info.conflicts.is_empty());
        assert_eq!(stream_content(&document, |stream| stream.content.starts_with(b"BT")), CONTENT);
        assert_eq!(stream_content(&document, |stream| stream.dict.has_type(b"EmbeddedFile")), b"attached notes");

        let (_, second_info) = saved(&outcomes[1]);
        assert_eq!(second_info.info.title.as_deref(), Some("Q3 report — final"));
        assert_eq!(second_info.info.subject.as_deref(), Some("Other subject"));
        assert_eq!(second_info.info.author, None);
        assert_eq!(second_info.xmp.unwrap().values.title.as_deref(), Some("Q3 report — final"), "a set field creates XMP");
    }

    #[test]
    fn rewrites_xmp_properties_written_as_description_attributes() {
        let packet = xmp(r#"<rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" dc:title="Old" dc:description="Gone"/>"#);
        let (rewritten, present) = rewrite_xmp(packet.as_bytes(), &[(DocumentProperty::Title, set("New & better")), (DocumentProperty::Subject, FieldEdit::Clear)]).unwrap();
        assert_eq!(present, vec![DocumentProperty::Title, DocumentProperty::Subject]);
        let text = String::from_utf8(rewritten).unwrap();
        assert!(!text.contains("Old") && !text.contains("Gone"), "{text}");
        assert!(text.contains("New &amp; better"), "{text}");
        assert!(text.contains(r#"rdf:about="""#), "{text}");
    }

    #[test]
    fn clearing_without_xmp_creates_none_and_unchanged_files_are_skipped_without_output() {
        let sandbox = Sandbox::new("skip");
        let input = sandbox.file("plain.pdf", &pdf(Some(dictionary! { "Title" => literal("Q3") }), None, None));
        let outcomes = run(&[input.clone()], patch(set("Q3"), FieldEdit::Clear, FieldEdit::Keep));
        let OutcomeState::Skipped { reason } = &outcomes[0].state else { panic!("{:?}", outcomes[0]) };
        assert_eq!(reason.code, SkipCode::NoEffectiveChange);
        assert_eq!(reason.message, "No effective change: the title is already \"Q3\" and there's no author. No copy was made.");
        assert_eq!(sandbox.names(), vec!["plain.pdf"]);

        let cleared = run(&[input], patch(FieldEdit::Clear, FieldEdit::Keep, FieldEdit::Keep));
        let (document, info) = saved(&cleared[0]);
        assert_eq!(info.info.title, None);
        assert!(info.xmp.is_none(), "clearing never creates XMP");
        assert!(document.catalog().unwrap().get(b"Metadata").is_err());
        assert!(!document.objects.values().any(|object| matches!(object, Object::String(bytes, _) if bytes == b"Q3")));
    }

    #[test]
    fn keeps_the_first_identifier_and_writes_identical_numbered_copies() {
        let sandbox = Sandbox::new("repeat");
        let input = sandbox.file("deck.pdf", &pdf(Some(dictionary! { "Title" => literal("Draft") }), None, None));
        let first = run(&[input.clone()], patch(set("Final"), FieldEdit::Keep, FieldEdit::Keep));
        let second = run(&[input.clone()], patch(set("Final"), FieldEdit::Keep, FieldEdit::Keep));
        assert_eq!(first[0].output_paths()[0], sandbox.0.join("deck-edited.pdf"));
        assert_eq!(second[0].output_paths()[0], sandbox.0.join("deck-edited-1.pdf"));
        assert_eq!(std::fs::read(&first[0].output_paths()[0]).unwrap(), std::fs::read(&second[0].output_paths()[0]).unwrap());
        let (document, _) = saved(&first[0]);
        let ids = document.trailer.get(b"ID").unwrap().as_array().unwrap();
        assert_eq!(ids[0].as_str().unwrap(), b"toolbox-first-id");
        assert_ne!(ids[1].as_str().unwrap(), b"toolbox-second-id");
        assert_eq!(ids[1].as_str().unwrap().len(), 16);
    }

    fn real_qpdf() -> Option<PathBuf> {
        match crate::kit::resources::resolve_helper(&crate::kit::resources::QPDF) {
            Ok(qpdf) => Some(qpdf),
            Err(_) if std::env::var_os("TOOLBOX_REQUIRE_QPDF").is_some() => panic!("qpdf is required for this test run"),
            Err(_) => {
                eprintln!("skipping: qpdf not available");
                None
            }
        }
    }

    /// An AES-256 copy of `plain`, written by qpdf the way Protect writes one.
    fn aes_256(qpdf: &Path, plain: &Path, output: &Path, user: &str, owner: &str) {
        let mut command = Command::new(qpdf);
        command.arg(plain).arg(output).args(["--encrypt", qpdf::PASSWORDS_FROM_STDIN, "--bits=256", "--print=none", "--"]);
        let result = qpdf::run_with_passwords(command, &[("--user-password", user), ("--owner-password", owner)], Instant::now() + std::time::Duration::from_secs(30), "timed out").unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    }

    fn opened(path: &Path, password: &str) -> Result<Document, ToolError> {
        document_info::load(&std::fs::read(path).unwrap(), Some(password))
    }

    #[test]
    fn encrypted_pdfs_opened_with_their_password_give_edited_copies_with_the_same_encryption() {
        let Some(qpdf) = real_qpdf() else { return };
        let sandbox = Sandbox::new("encrypted");
        let info = || Some(dictionary! { "Title" => literal("Draft"), "Author" => literal("Ann") });
        let locked = sandbox.file("locked.pdf", &pdf(info(), None, Some(("owner-secret", crate::kit::TEST_PASSWORD))));
        let restricted = sandbox.file("restricted.pdf", &pdf(info(), None, Some(("owner-secret", ""))));
        let plain = sandbox.file("plain.pdf", &pdf(info(), None, None));
        let aes = sandbox.0.join("aes.pdf");
        aes_256(&qpdf, &plain, &aes, crate::kit::TEST_OVERRIDE_PASSWORD, "aes-owner-secret");
        let inputs = [locked.clone(), restricted.clone(), aes.clone(), plain.clone()];
        let originals = inputs.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();

        let outcomes = run_with(&inputs, patch(set("Final"), FieldEdit::Keep, FieldEdit::Keep), passwords(crate::kit::TEST_PASSWORD, &[(&aes, crate::kit::TEST_OVERRIDE_PASSWORD)]));

        assert_eq!(outcomes.iter().map(|outcome| outcome.detail.as_str()).collect::<Vec<_>>(), ["Title set. It keeps its open password.", "Title set. It keeps its restrictions.", "Title set. It keeps its open password.", "Title set."]);
        for (input, original) in inputs.iter().zip(&originals) {
            assert_eq!(&std::fs::read(input).unwrap(), original, "the original is never modified");
        }
        for (input, password) in [(&locked, crate::kit::TEST_PASSWORD), (&restricted, ""), (&aes, crate::kit::TEST_OVERRIDE_PASSWORD)] {
            let output = sandbox.0.join(format!("{}-edited.pdf", input.file_stem().unwrap().to_string_lossy()));
            let (before, after) = (opened(input, password).unwrap(), opened(&output, password).unwrap());
            assert!(after.was_encrypted(), "{output:?} is still encrypted");
            assert!(encryption(&before) == encryption(&after), "{output:?} keeps the same passwords and permissions");
            let info = document_info::from_document(&after);
            assert_eq!((info.info.title.as_deref(), info.info.author.as_deref()), (Some("Final"), Some("Ann")));
            assert!(after.get_page_content(*after.get_pages().values().next().unwrap()).starts_with(CONTENT), "the page is kept");
            if !password.is_empty() {
                assert_eq!(opened(&output, "").unwrap_err().kind, ErrorKind::WrongPassword, "{output:?} still needs its open password");
            }
        }
        let kept = opened(&sandbox.0.join("restricted-edited.pdf"), "").unwrap().encryption_state.unwrap().permissions();
        assert!(kept.contains(Permissions::PRINTABLE) && !kept.contains(Permissions::MODIFIABLE) && !kept.contains(Permissions::COPYABLE), "{kept:?}");
        assert_eq!(sandbox.names(), ["aes-edited.pdf", "aes.pdf", "locked-edited.pdf", "locked.pdf", "plain-edited.pdf", "plain.pdf", "restricted-edited.pdf", "restricted.pdf"]);
    }

    #[test]
    fn a_wrong_or_missing_password_fails_only_that_file_and_never_shows_the_password() {
        let sandbox = Sandbox::new("wrong-password");
        let locked = sandbox.file("locked.pdf", &pdf(None, None, Some(("owner-secret", crate::kit::TEST_PASSWORD))));
        let broken = sandbox.file("broken.pdf", b"not a pdf");
        let plain = sandbox.file("plain.pdf", &pdf(None, None, None));
        let secret = "wrong-secret-1234";

        let outcomes = run_with(&[locked.clone(), broken, plain.clone()], patch(FieldEdit::Keep, FieldEdit::Keep, set("Budget")), passwords(secret, &[]));
        let error = outcomes[0].error().unwrap();
        assert_eq!(error.kind, ErrorKind::WrongPassword);
        assert!(outcomes[1].error().is_some_and(|error| error.kind != ErrorKind::WrongPassword));
        assert_eq!(outcomes[2].detail, "Subject set.");
        for outcome in &outcomes {
            assert!(!serde_json::to_string(outcome).unwrap().contains(secret), "{outcome:?}");
        }
        let missing = run(&[locked.clone()], patch(FieldEdit::Keep, FieldEdit::Keep, set("Budget")));
        assert_eq!(missing[0].error().unwrap().kind, ErrorKind::WrongPassword);
        let other_file = run_with(&[locked], patch(FieldEdit::Keep, FieldEdit::Keep, set("Budget")), passwords(secret, &[(&plain, crate::kit::TEST_PASSWORD)]));
        assert_eq!(other_file[0].error().unwrap().kind, ErrorKind::WrongPassword, "an override for another file is never tried on this one");
        assert_eq!(sandbox.names(), ["broken.pdf", "locked.pdf", "plain-edited.pdf", "plain.pdf"]);
    }

    fn signed_pdf(encryption: Option<(&str, &str)>) -> Vec<u8> {
        let mut document = Document::load_mem(&pdf(Some(dictionary! { "Title" => literal("NDA draft") }), None, None)).unwrap();
        super::super::tests::sign(&mut document);
        if let Some((owner_password, user_password)) = encryption {
            let state = EncryptionState::try_from(EncryptionVersion::V2 { document: &document, owner_password, user_password, key_length: 128, permissions: Permissions::PRINTABLE }).unwrap();
            document.encrypt(&state).unwrap();
        }
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn signed_pdfs_are_saved_without_their_signature_and_only_their_rows_warn() {
        use crate::kit::contracts::{OutcomeWarning, WarningKind};
        let qpdf = real_qpdf();
        let sandbox = Sandbox::new("signed");
        let signed = sandbox.file("vendor-nda.pdf", &signed_pdf(None));
        let plain = sandbox.file("lease.pdf", &pdf(Some(dictionary! { "Title" => literal("Lease") }), None, None));
        let mut inputs = vec![signed.clone(), plain.clone()];
        if qpdf.is_some() {
            inputs.push(sandbox.file("signed-restricted.pdf", &signed_pdf(Some(("owner-secret", "")))));
        }
        let originals = inputs.iter().map(|path| std::fs::read(path).unwrap()).collect::<Vec<_>>();
        for input in [&signed, &inputs[inputs.len() - 1]] {
            assert!(super::super::tests::holds_a_signature(&opened(input, "").unwrap()), "{input:?} is signed");
        }

        let outcomes = run(&inputs, patch(set("Final"), FieldEdit::Keep, FieldEdit::Keep));

        let removed = vec![OutcomeWarning { kind: WarningKind::SignatureRemoved, message: super::super::SIGNATURE_REMOVED.to_string() }];
        assert_eq!(outcomes[0].warnings, removed);
        assert!(outcomes[1].warnings.is_empty(), "{:?}", outcomes[1].warnings);
        assert_eq!(outcomes[0].output_paths(), [sandbox.0.join("vendor-nda-edited.pdf")]);
        if qpdf.is_some() {
            assert_eq!(outcomes[2].warnings, removed);
            assert_eq!(outcomes[2].detail, "Title set. It keeps its restrictions.");
            assert_eq!(outcomes[2].output_paths(), [sandbox.0.join("signed-restricted-edited.pdf")]);
        }
        for outcome in &outcomes {
            let output = opened(&outcome.output_paths()[0], "").unwrap();
            assert!(!super::super::tests::holds_a_signature(&output), "{:?} still holds a signature", outcome.output_paths());
            assert_eq!(document_info::from_document(&output).info.title.as_deref(), Some("Final"));
            assert!(output.get_page_content(*output.get_pages().values().next().unwrap()).starts_with(CONTENT), "the page is kept");
        }
        for (input, original) in inputs.iter().zip(&originals) {
            assert_eq!(&std::fs::read(input).unwrap(), original, "the original is never modified");
        }
        assert!(super::super::tests::holds_a_signature(&opened(&signed, "").unwrap()), "the original keeps its signature");
    }

    #[test]
    fn reading_current_values_reports_whether_each_pdf_is_signed() {
        let sandbox = Sandbox::new("read-signed");
        let signed = sandbox.file("vendor-nda.pdf", &signed_pdf(None));
        let plain = sandbox.file("lease.pdf", &pdf(None, None, None));
        let request = Arc::new(ReadPropertiesRequest { paths: vec![signed.clone(), plain.clone()], passwords: OpenPasswords::default() });
        let outcomes = BatchRunner::inspect(&RunEvents::silent(), "edit_pdf_properties", vec![signed, plain], &request, read_job);
        let values = outcomes.iter().map(|outcome| serde_json::from_str::<serde_json::Value>(&outcome.detail).unwrap()).collect::<Vec<_>>();
        assert_eq!((values[0]["signed"].clone(), values[0]["title"].clone()), (serde_json::json!(true), serde_json::json!("NDA draft")));
        assert_eq!(values[1]["signed"], serde_json::json!(false));
        assert_eq!(sandbox.names(), ["lease.pdf", "vendor-nda.pdf"]);
    }

    #[cfg(unix)]
    #[test]
    fn the_qpdf_call_that_keeps_the_password_reads_it_from_standard_input_only() {
        use crate::kit::common::fake_helper::{secret_exposure, Exposure, FakeHelper};
        let _guard = crate::kit::lock_process_env();
        let Some(qpdf) = real_qpdf() else { return };
        let sandbox = Sandbox::new("qpdf-call");
        let helper = FakeHelper::install(&sandbox.0, "qpdf", Some(&qpdf));
        let locked = sandbox.file("locked.pdf", &pdf(None, None, Some(("owner-secret", crate::kit::TEST_PASSWORD))));
        let previous = std::env::var_os("TOOLBOX_QPDF_PATH");
        std::env::set_var("TOOLBOX_QPDF_PATH", &helper.script);
        let outcomes = run_with(&[locked.clone()], patch(set("Final"), FieldEdit::Keep, FieldEdit::Keep), passwords(crate::kit::TEST_PASSWORD, &[]));
        match previous {
            Some(value) => std::env::set_var("TOOLBOX_QPDF_PATH", value),
            None => std::env::remove_var("TOOLBOX_QPDF_PATH"),
        }

        assert!(outcomes[0].error().is_none(), "{:?}", outcomes[0]);
        let calls = helper.invocations().into_iter().filter(|call| call.arguments.iter().any(|argument| argument.contains(&locked.display().to_string()))).collect::<Vec<_>>();
        assert_eq!(calls.len(), 1, "{:?}", calls.iter().map(|call| &call.arguments).collect::<Vec<_>>());
        assert_eq!(calls[0].arguments.iter().filter(|argument| *argument == qpdf::PASSWORDS_FROM_STDIN).count(), 1);
        assert_eq!(secret_exposure(&calls, crate::kit::TEST_PASSWORD), Exposure::default(), "the password reached qpdf's arguments or environment");
    }

    #[test]
    fn reads_current_values_once_the_right_password_is_given() {
        let sandbox = Sandbox::new("read-locked");
        let locked = sandbox.file("locked.pdf", &pdf(Some(dictionary! { "Title" => literal("Offer letter") }), None, Some(("owner-secret", crate::kit::TEST_PASSWORD))));
        let read = |passwords: OpenPasswords| {
            let request = Arc::new(ReadPropertiesRequest { paths: vec![locked.clone()], passwords });
            BatchRunner::inspect(&RunEvents::silent(), "edit_pdf_properties", vec![locked.clone()], &request, read_job).remove(0)
        };
        assert_eq!(read(OpenPasswords::default()).error().unwrap().kind, ErrorKind::WrongPassword);
        assert_eq!(read(passwords("wrong-secret", &[])).error().unwrap().kind, ErrorKind::WrongPassword);
        let values: serde_json::Value = serde_json::from_str(&read(passwords("", &[(&locked, crate::kit::TEST_PASSWORD)])).detail).unwrap();
        assert_eq!(values["title"], "Offer letter");
        assert_eq!(sandbox.names(), ["locked.pdf"]);
    }

    fn info_entry(document: &Document, key: &[u8]) -> Option<Vec<u8>> {
        info_dictionary(document).and_then(|info| info.get(key).ok()).map(|value| super::super::resolve(document, value).unwrap().as_str().unwrap().to_vec())
    }

    #[test]
    fn deserializes_date_edits_from_the_wire() {
        let request: EditPropertiesRequest = serde_json::from_str(r#"{"paths":["a.pdf"],"patch":{"creationDate":{"action":"set","value":"2026-02-28T09:00:00+05:30"},"modificationDate":{"action":"clear"}},"outputLocation":"alongsideInput"}"#).unwrap();
        assert_eq!(request.patch, dates(set("2026-02-28T09:00:00+05:30"), FieldEdit::Clear));
    }

    #[test]
    fn rejects_invalid_dates_as_invalid_input_before_opening_files() {
        let sandbox = Sandbox::new("invalid-dates");
        let input = sandbox.file("report.pdf", &pdf(None, None, None));
        for (patch, message) in [
            (dates(set("2026-02-30T09:00:00+00:00"), FieldEdit::Keep), "The creation date isn't a real date. Use YYYY-MM-DD HH:MM, like 2026-02-28 09:00."),
            (dates(set("2026-02-28 09:00"), FieldEdit::Keep), "The creation date isn't a real date. Use YYYY-MM-DD HH:MM, like 2026-02-28 09:00."),
            (dates(FieldEdit::Keep, set("2026-02-28T09:00:00")), "The modification date isn't a real date. Use YYYY-MM-DD HH:MM, like 2026-02-28 09:00."),
            (dates(FieldEdit::Keep, set("2026-02-28")), "The modification date isn't a real date. Use YYYY-MM-DD HH:MM, like 2026-02-28 09:00."),
            (dates(FieldEdit::Keep, set("2026-02-28T24:00:00Z")), "The modification date isn't a real date. Use YYYY-MM-DD HH:MM, like 2026-02-28 09:00."),
            (dates(FieldEdit::Keep, set("yesterday")), "The modification date isn't a real date. Use YYYY-MM-DD HH:MM, like 2026-02-28 09:00."),
            (dates(set("  "), FieldEdit::Keep), "Enter a creation date, or choose Keep or Clear."),
        ] {
            let outcomes = run(&[input.clone()], patch.clone());
            let error = outcomes[0].error().unwrap_or_else(|| panic!("{patch:?}"));
            assert_eq!(error.kind, ErrorKind::InvalidInput);
            assert_eq!(error.message, message);
            let direct = edit_job(&EditPropertiesRequest { paths: vec![input.clone()], patch, output_location: OutputLocation::AlongsideInput, passwords: OpenPasswords::default() }, input.clone());
            assert_eq!(direct.error().unwrap().kind, ErrorKind::InvalidInput);
        }
        assert_eq!(sandbox.names(), vec!["report.pdf"]);
    }

    #[test]
    fn sets_dates_in_the_information_dictionary_and_xmp_with_their_time_zone() {
        let sandbox = Sandbox::new("set-dates");
        let description = r#"<rdf:Description rdf:about="" xmlns:xap="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/" xap:ModifyDate="2020-01-01T00:00:00Z"><xap:CreateDate>2019-05-02T10:00:00Z</xap:CreateDate><dc:title><rdf:Alt><rdf:li xml:lang="x-default">Deck</rdf:li></rdf:Alt></dc:title></rdf:Description>"#;
        let with_xmp = sandbox.file("with-xmp.pdf", &pdf(Some(dictionary! { "Title" => literal("Deck"), "CreationDate" => literal("D:20190502100000Z"), "ModDate" => literal("D:20200101000000Z") }), Some(&xmp(description)), None));
        let plain = sandbox.file("plain.pdf", &pdf(Some(dictionary! { "Title" => literal("Plain") }), None, None));
        let originals = [std::fs::read(&with_xmp).unwrap(), std::fs::read(&plain).unwrap()];

        let outcomes = run(&[with_xmp.clone(), plain.clone()], dates(set(" 2026-02-28T09:00+05:30 "), set("2026-03-01T17:45:00-08:00")));

        assert_eq!(std::fs::read(&with_xmp).unwrap(), originals[0]);
        assert_eq!(std::fs::read(&plain).unwrap(), originals[1]);
        for outcome in &outcomes {
            assert_eq!(outcome.detail, "Creation date set, modification date set.");
            let (document, info) = saved(outcome);
            assert_eq!(info_entry(&document, b"CreationDate").unwrap(), b"D:20260228090000+05'30'");
            assert_eq!(info_entry(&document, b"ModDate").unwrap(), b"D:20260301174500-08'00'");
            let xmp = info.xmp.unwrap();
            assert_eq!(xmp.values.creation_date.unwrap().to_string(), "2026-02-28T09:00:00+05:30");
            assert_eq!(xmp.values.modification_date.unwrap().to_string(), "2026-03-01T17:45:00-08:00");
            assert!(info.conflicts.is_empty(), "{:?}", info.conflicts);
        }
        let (_, first) = saved(&outcomes[0]);
        assert_eq!(first.xmp.unwrap().values.title.as_deref(), Some("Deck"));
        let (_, second) = saved(&outcomes[1]);
        assert_eq!(second.xmp.unwrap().values.title.as_deref(), Some("Plain"), "a new XMP packet copies the kept values");

        let utc = run(&[plain], dates(set("2026-02-28T09:00:00+00:00"), FieldEdit::Keep));
        let (document, info) = saved(&utc[0]);
        assert_eq!(info_entry(&document, b"CreationDate").unwrap(), b"D:20260228090000Z");
        assert_eq!(info.xmp.unwrap().values.creation_date.unwrap().to_string(), "2026-02-28T09:00:00Z");
    }

    #[test]
    fn clears_dates_from_both_places_and_keep_leaves_them_untouched() {
        let sandbox = Sandbox::new("clear-dates");
        let description = r#"<rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:CreateDate="2019-05-02T10:00:00Z"><xmp:ModifyDate>2020-01-01T00:00:00Z</xmp:ModifyDate><xmp:CreatorTool>Writer</xmp:CreatorTool></rdf:Description>"#;
        let input = sandbox.file("dated.pdf", &pdf(Some(dictionary! { "Title" => literal("Old"), "CreationDate" => literal("D:20190502100000Z"), "ModDate" => literal("D:20200101000000Z") }), Some(&xmp(description)), None));

        let cleared = run(&[input.clone()], dates(FieldEdit::Clear, FieldEdit::Clear));
        assert_eq!(cleared[0].detail, "Creation date cleared, modification date cleared.");
        let (document, info) = saved(&cleared[0]);
        assert_eq!(info_entry(&document, b"CreationDate"), None);
        assert_eq!(info_entry(&document, b"ModDate"), None);
        let xmp = info.xmp.unwrap();
        assert_eq!((xmp.values.creation_date, xmp.values.modification_date), (None, None));
        assert_eq!(xmp.values.creator.as_deref(), Some("Writer"));

        let retitled = run(&[input], patch(set("New"), FieldEdit::Keep, FieldEdit::Keep));
        let (document, info) = saved(&retitled[0]);
        assert_eq!(info_entry(&document, b"CreationDate").unwrap(), b"D:20190502100000Z");
        assert_eq!(info_entry(&document, b"ModDate").unwrap(), b"D:20200101000000Z", "the modification date changes only when it is set");
        let xmp = info.xmp.unwrap();
        assert_eq!(xmp.values.creation_date.unwrap().to_string(), "2019-05-02T10:00:00Z");
        assert_eq!(xmp.values.modification_date.unwrap().to_string(), "2020-01-01T00:00:00Z");
    }

    #[test]
    fn matching_dates_are_skipped_and_repeated_date_edits_write_identical_bytes() {
        let sandbox = Sandbox::new("repeat-dates");
        let input = sandbox.file("memo.pdf", &pdf(Some(dictionary! { "CreationDate" => literal("D:20260228090000+05'30'") }), None, None));
        let same = run(&[input.clone()], dates(set("2026-02-28T09:00:00+05:30"), FieldEdit::Clear));
        let OutcomeState::Skipped { reason } = &same[0].state else { panic!("{:?}", same[0]) };
        assert_eq!(reason.message, "No effective change: the creation date is already \"2026-02-28T09:00:00+05:30\" and there's no modification date. No copy was made.");

        let edit = || run(&[input.clone()], dates(set("2026-02-28T03:30:00Z"), set("2026-03-01T12:00:00+01:00")));
        let first = edit();
        let second = edit();
        assert_eq!(second[0].output_paths()[0], sandbox.0.join("memo-edited-1.pdf"));
        assert_eq!(std::fs::read(&first[0].output_paths()[0]).unwrap(), std::fs::read(&second[0].output_paths()[0]).unwrap());
    }

    #[test]
    fn reads_current_values_with_xmp_fallback() {
        let sandbox = Sandbox::new("read");
        let description = r#"<rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator><rdf:Seq><rdf:li>Ann</rdf:li></rdf:Seq></dc:creator></rdf:Description>"#;
        let input = sandbox.file("read.pdf", &pdf(Some(dictionary! { "Title" => literal("Info title") }), Some(&xmp(description)), None));
        let request = Arc::new(ReadPropertiesRequest { paths: vec![input.clone()], passwords: OpenPasswords::default() });
        let outcomes = BatchRunner::inspect(&RunEvents::silent(), "edit_pdf_properties", vec![input], &request, read_job);
        let values: serde_json::Value = serde_json::from_str(&outcomes[0].detail).unwrap();
        assert_eq!(values["title"], "Info title");
        assert_eq!(values["author"], "Ann");
        assert_eq!(values["subject"], serde_json::Value::Null);
        assert_eq!(sandbox.names(), vec!["read.pdf"]);
    }
}
