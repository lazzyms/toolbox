//! The hidden-content half of Privacy Check: what a PDF carries that its pages don't show. It only reads.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId};
use serde::{Deserialize, Serialize};

use super::document_info::{decode_text, parse_pdf_date, PdfDate};
use super::{inherited, resolve};

const MAX_DEPTH: usize = 32;
const MAX_ENTRIES: usize = 100_000;
const MAX_SAVES: usize = 10_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenContent {
    /// Saves appended after the first one. None when the save history couldn't be read.
    pub incremental_updates: Option<usize>,
    pub embedded_files: Vec<EmbeddedFile>,
    pub actions: Vec<AutomaticAction>,
    pub annotations: Vec<AnnotationGroup>,
    pub filled_form_fields: Vec<String>,
    pub optional_content: OptionalContent,
    pub links: Vec<LinkHost>,
    pub signatures: Vec<Signature>,
    pub leftovers: Leftovers,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedFile {
    pub name: String,
    pub size: Option<u64>,
    pub mime_type: Option<String>,
    /// The page of a file attached through a comment; None for a file attached to the document.
    pub page: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionTrigger {
    DocumentScript,
    Open,
    BeforeClose,
    BeforeSave,
    AfterSave,
    BeforePrint,
    AfterPrint,
    PageOpen,
    PageClose,
    Annotation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomaticAction {
    pub trigger: ActionTrigger,
    pub page: Option<u32>,
    /// The name of a document-level script.
    pub name: Option<String>,
    /// Action types in the order they run, such as JavaScript or Launch.
    pub kinds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationGroup {
    pub subtype: String,
    pub count: usize,
    pub authors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionalContent {
    pub layers: Vec<Layer>,
    pub hidden: Vec<HiddenItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    pub name: String,
    pub visible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HiddenReason {
    Hidden,
    OffPage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenItem {
    pub page: u32,
    pub subtype: String,
    pub reason: HiddenReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkHost {
    pub host: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signature {
    pub field: String,
    pub signer: Option<String>,
    pub signed_at: Option<PdfDate>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Leftovers {
    pub thumbnail_pages: Vec<u32>,
    pub page_pieces: Vec<PagePiece>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PagePiece {
    pub page: u32,
    pub applications: Vec<String>,
}

pub fn read(document: &Document, bytes: &[u8], unsupported: &mut Vec<String>) -> HiddenContent {
    let mut report = HiddenContent { incremental_updates: incremental_updates(document, bytes), ..HiddenContent::default() };
    if report.incremental_updates.is_none() {
        unsupported.push("The file's save history could not be read.".to_string());
    }
    let catalog = document.catalog().ok();
    let names = catalog.and_then(|catalog| get_dict(document, catalog, b"Names"));
    let mut attached = HashSet::new();
    if let Some(tree) = names.and_then(|names| names.get(b"EmbeddedFiles").ok()) {
        for (key, spec) in name_tree(document, tree, "embedded files", unsupported) {
            if let Object::Reference(id) = spec {
                attached.insert(*id);
            }
            report.embedded_files.push(embedded_file(document, spec, Some(&key), None));
        }
    }
    if let Some(tree) = names.and_then(|names| names.get(b"JavaScript").ok()) {
        for (key, action) in name_tree(document, tree, "document scripts", unsupported) {
            report.actions.push(AutomaticAction { trigger: ActionTrigger::DocumentScript, page: None, name: Some(key), kinds: action_kinds(document, action) });
        }
    }
    if let Some(catalog) = catalog {
        if let Some(open) = catalog.get(b"OpenAction").ok().and_then(|open| resolve(document, open).ok()) {
            let kinds = if open.as_dict().is_ok() { action_kinds(document, open) } else { Vec::new() };
            if kinds.iter().any(|kind| kind != "GoTo") {
                report.actions.push(AutomaticAction { trigger: ActionTrigger::Open, page: None, name: None, kinds });
            }
        }
        if let Some(events) = get_dict(document, catalog, b"AA") {
            for (key, trigger) in [(b"WC", ActionTrigger::BeforeClose), (b"WS", ActionTrigger::BeforeSave), (b"DS", ActionTrigger::AfterSave), (b"WP", ActionTrigger::BeforePrint), (b"DP", ActionTrigger::AfterPrint)] {
                if let Ok(action) = events.get(key) {
                    report.actions.push(AutomaticAction { trigger, page: None, name: None, kinds: action_kinds(document, action) });
                }
            }
        }
        report.optional_content.layers = layers(document, catalog, unsupported);
        if let Some(form) = get_dict(document, catalog, b"AcroForm") {
            if form.has(b"XFA") {
                unsupported.push("XFA form data could not be checked.".to_string());
            }
            read_fields(document, form, &mut report, unsupported);
        }
    }
    read_pages(document, &attached, &mut report);
    report
}

/// Saves found by following the cross-reference chain back from the end of the file.
fn incremental_updates(document: &Document, bytes: &[u8]) -> Option<usize> {
    let mut offset = last_startxref(bytes)?;
    let mut seen = HashSet::new();
    let mut saves = 0;
    loop {
        if !seen.insert(offset) || saves >= MAX_SAVES {
            return None;
        }
        saves += 1;
        let section = bytes.get(offset..)?;
        let section = &section[section.iter().position(|byte| !byte.is_ascii_whitespace())?..];
        let end = if section.starts_with(b"xref") { find(section, b"startxref")? } else { find(section, b"stream")? };
        match keyword_number(&section[..end], b"/Prev") {
            Some(previous) => offset = previous,
            None => break,
        }
    }
    // A linearized file keeps a separate first-page cross-reference section from its first save.
    let linearized = document.objects.values().any(|object| object.as_dict().is_ok_and(|dict| dict.has(b"Linearized")));
    Some(saves - 1 - usize::from(linearized && saves > 1))
}

fn last_startxref(bytes: &[u8]) -> Option<usize> {
    let start = bytes.windows(9).rposition(|window| window == b"startxref")?;
    number_after(&bytes[start + 9..])
}

fn keyword_number(bytes: &[u8], keyword: &[u8]) -> Option<usize> {
    let start = find(bytes, keyword)?;
    let rest = &bytes[start + keyword.len()..];
    rest.first().filter(|byte| byte.is_ascii_whitespace() || byte.is_ascii_digit())?;
    number_after(rest)
}

fn number_after(bytes: &[u8]) -> Option<usize> {
    let digits = bytes.iter().skip_while(|byte| byte.is_ascii_whitespace()).take_while(|byte| byte.is_ascii_digit()).map(|&byte| char::from(byte)).collect::<String>();
    digits.parse().ok()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn get_dict<'a>(document: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Dictionary> {
    resolve(document, dict.get(key).ok()?).ok()?.as_dict().ok()
}

fn get_array<'a>(document: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Vec<Object>> {
    resolve(document, dict.get(key).ok()?).ok()?.as_array().ok()
}

fn get_text(document: &Document, dict: &Dictionary, key: &[u8]) -> Option<String> {
    decode_text(resolve(document, dict.get(key).ok()?).ok()?)
}

fn get_name(document: &Document, dict: &Dictionary, key: &[u8]) -> Option<String> {
    resolve(document, dict.get(key).ok()?).ok()?.as_name().ok().map(|name| String::from_utf8_lossy(name).into_owned())
}

fn name_tree<'a>(document: &'a Document, root: &'a Object, what: &str, unsupported: &mut Vec<String>) -> Vec<(String, &'a Object)> {
    fn walk<'a>(document: &'a Document, node: &'a Object, depth: usize, seen: &mut HashSet<ObjectId>, entries: &mut Vec<(String, &'a Object)>) -> bool {
        if let Object::Reference(id) = node {
            if !seen.insert(*id) {
                return false;
            }
        }
        let Some(node) = resolve(document, node).ok().and_then(|node| node.as_dict().ok()) else { return false };
        if depth > MAX_DEPTH || entries.len() > MAX_ENTRIES {
            return false;
        }
        let mut complete = true;
        if let Some(pairs) = get_array(document, node, b"Names") {
            for pair in pairs.chunks(2) {
                match (pair.first().and_then(|key| resolve(document, key).ok()).and_then(decode_text), pair.get(1)) {
                    (Some(key), Some(value)) => entries.push((key, value)),
                    _ => complete = false,
                }
            }
        }
        if let Some(kids) = get_array(document, node, b"Kids") {
            for kid in kids {
                complete &= walk(document, kid, depth + 1, seen, entries);
            }
        }
        complete
    }
    let mut entries = Vec::new();
    if !walk(document, root, 0, &mut HashSet::new(), &mut entries) {
        unsupported.push(format!("Some {what} could not be read."));
    }
    entries
}

fn embedded_file(document: &Document, spec: &Object, fallback: Option<&str>, page: Option<u32>) -> EmbeddedFile {
    let spec = resolve(document, spec).ok().and_then(|spec| spec.as_dict().ok());
    let name = spec
        .and_then(|spec| get_text(document, spec, b"UF").or_else(|| get_text(document, spec, b"F")))
        .or_else(|| fallback.map(str::to_owned))
        .unwrap_or_else(|| "Unnamed file".to_string());
    let stream = spec
        .and_then(|spec| get_dict(document, spec, b"EF"))
        .and_then(|files| files.get(b"F").or_else(|_| files.get(b"UF")).ok())
        .and_then(|stream| resolve(document, stream).ok()?.as_stream().ok());
    let size = stream.and_then(|stream| {
        let declared = get_dict(document, &stream.dict, b"Params").and_then(|params| resolve(document, params.get(b"Size").ok()?).ok()?.as_i64().ok());
        declared.and_then(|size| u64::try_from(size).ok()).or_else(|| u64::try_from(stream.content.len()).ok())
    });
    let mime_type = stream.and_then(|stream| get_name(document, &stream.dict, b"Subtype"));
    EmbeddedFile { name, size, mime_type, page }
}

fn action_kinds(document: &Document, action: &Object) -> Vec<String> {
    let mut kinds = Vec::new();
    let mut seen = HashSet::new();
    let mut pending = vec![action];
    while let Some(action) = pending.pop() {
        if let Object::Reference(id) = action {
            if !seen.insert(*id) {
                continue;
            }
        }
        if kinds.len() > MAX_DEPTH {
            break;
        }
        let Some(action) = resolve(document, action).ok() else { continue };
        if let Ok(list) = action.as_array() {
            pending.extend(list.iter().rev());
            continue;
        }
        let Ok(action) = action.as_dict() else { continue };
        kinds.push(get_name(document, action, b"S").unwrap_or_else(|| "Unknown".to_string()));
        if let Ok(next) = action.get(b"Next") {
            pending.push(next);
        }
    }
    kinds
}

fn layers(document: &Document, catalog: &Dictionary, unsupported: &mut Vec<String>) -> Vec<Layer> {
    let Some(properties) = get_dict(document, catalog, b"OCProperties") else { return Vec::new() };
    let Some(groups) = get_array(document, properties, b"OCGs") else {
        unsupported.push("The hidden layers could not be read.".to_string());
        return Vec::new();
    };
    let defaults = get_dict(document, properties, b"D");
    let listed = |key: &[u8]| -> HashSet<ObjectId> {
        defaults.and_then(|defaults| get_array(document, defaults, key)).into_iter().flatten().filter_map(|group| group.as_reference().ok()).collect()
    };
    let (on, off) = (listed(b"ON"), listed(b"OFF"));
    let base_off = defaults.and_then(|defaults| get_name(document, defaults, b"BaseState")).as_deref() == Some("OFF");
    groups.iter().take(MAX_ENTRIES).map(|group| {
        let id = group.as_reference().ok();
        let name = resolve(document, group).ok().and_then(|group| group.as_dict().ok()).and_then(|group| get_text(document, group, b"Name")).unwrap_or_else(|| "Unnamed layer".to_string());
        let visible = match id {
            Some(id) if base_off => on.contains(&id),
            Some(id) => !off.contains(&id),
            None => !base_off,
        };
        Layer { name, visible }
    }).collect()
}

fn read_fields(document: &Document, form: &Dictionary, report: &mut HiddenContent, unsupported: &mut Vec<String>) {
    fn walk(document: &Document, field: &Object, parent: Option<&str>, inherited_type: Option<String>, depth: usize, seen: &mut HashSet<ObjectId>, report: &mut HiddenContent) -> bool {
        if let Object::Reference(id) = field {
            if !seen.insert(*id) {
                return false;
            }
        }
        if depth > MAX_DEPTH || seen.len() > MAX_ENTRIES {
            return false;
        }
        let Some(field) = resolve(document, field).ok().and_then(|field| field.as_dict().ok()) else { return false };
        let partial = get_text(document, field, b"T");
        let name = match (parent, partial) {
            (Some(parent), Some(partial)) => format!("{parent}.{partial}"),
            (None, Some(partial)) => partial,
            (Some(parent), None) => parent.to_owned(),
            (None, None) => "Unnamed field".to_string(),
        };
        let field_type = get_name(document, field, b"FT").or(inherited_type);
        let child_fields = get_array(document, field, b"Kids").into_iter().flatten()
            .filter(|kid| resolve(document, kid).ok().and_then(|kid| kid.as_dict().ok()).is_some_and(|kid| kid.has(b"T")))
            .collect::<Vec<_>>();
        if !child_fields.is_empty() {
            return child_fields.into_iter().fold(true, |complete, kid| walk(document, kid, Some(&name), field_type.clone(), depth + 1, seen, report) && complete);
        }
        let value = field.get(b"V").ok().and_then(|value| resolve(document, value).ok());
        match (field_type.as_deref(), value) {
            (Some("Sig"), Some(Object::Dictionary(signature))) => report.signatures.push(Signature {
                field: name,
                signer: get_text(document, signature, b"Name"),
                signed_at: get_text(document, signature, b"M").and_then(|date| parse_pdf_date(&date)),
            }),
            (Some("Sig"), _) | (_, None) => {}
            (_, Some(value)) if is_filled(document, value) => report.filled_form_fields.push(name),
            _ => {}
        }
        true
    }
    let complete = get_array(document, form, b"Fields").into_iter().flatten()
        .fold(true, |complete, field| walk(document, field, None, None, 0, &mut HashSet::new(), report) && complete);
    if !complete {
        unsupported.push("Some form fields could not be read.".to_string());
    }
}

fn is_filled(document: &Document, value: &Object) -> bool {
    match value {
        Object::String(text, _) => !text.is_empty() && decode_text(value).is_none_or(|text| !text.trim().is_empty()),
        Object::Name(name) => name.as_slice() != b"Off",
        Object::Array(values) => values.iter().any(|value| resolve(document, value).is_ok_and(|value| is_filled(document, value))),
        Object::Stream(stream) => !stream.content.is_empty(),
        Object::Integer(_) | Object::Real(_) => true,
        _ => false,
    }
}

fn read_pages(document: &Document, attached: &HashSet<ObjectId>, report: &mut HiddenContent) {
    let mut groups: BTreeMap<String, (usize, BTreeSet<String>)> = BTreeMap::new();
    let mut hosts: BTreeMap<String, usize> = BTreeMap::new();
    for (number, page_id) in document.get_pages() {
        let Ok(page) = document.get_dictionary(page_id) else { continue };
        if page.has(b"Thumb") {
            report.leftovers.thumbnail_pages.push(number);
        }
        if let Some(pieces) = get_dict(document, page, b"PieceInfo") {
            let applications = pieces.iter().map(|(key, _)| String::from_utf8_lossy(key).into_owned()).collect::<Vec<_>>();
            report.leftovers.page_pieces.push(PagePiece { page: number, applications });
        }
        if let Some(events) = get_dict(document, page, b"AA") {
            for (key, trigger) in [(b"O", ActionTrigger::PageOpen), (b"C", ActionTrigger::PageClose)] {
                if let Ok(action) = events.get(key) {
                    report.actions.push(AutomaticAction { trigger, page: Some(number), name: None, kinds: action_kinds(document, action) });
                }
            }
        }
        let page_box = page_box(document, page_id);
        for annotation in get_array(document, page, b"Annots").into_iter().flatten().take(MAX_ENTRIES) {
            let Some(annotation) = resolve(document, annotation).ok().and_then(|annotation| annotation.as_dict().ok()) else { continue };
            let subtype = get_name(document, annotation, b"Subtype").unwrap_or_else(|| "Unknown".to_string());
            if subtype == "Popup" {
                continue;
            }
            if let Some(reason) = hidden_reason(document, annotation, page_box) {
                report.optional_content.hidden.push(HiddenItem { page: number, subtype: subtype.clone(), reason });
            }
            if let Some(events) = get_dict(document, annotation, b"AA") {
                let kinds = events.iter().flat_map(|(_, action)| action_kinds(document, action)).collect::<Vec<_>>();
                if !kinds.is_empty() {
                    report.actions.push(AutomaticAction { trigger: ActionTrigger::Annotation, page: Some(number), name: None, kinds });
                }
            }
            match subtype.as_str() {
                "Widget" => {}
                "Link" => {
                    let uri = get_dict(document, annotation, b"A")
                        .filter(|action| get_name(document, action, b"S").as_deref() == Some("URI"))
                        .and_then(|action| resolve(document, action.get(b"URI").ok()?).ok()?.as_str().ok().map(|uri| String::from_utf8_lossy(uri).into_owned()));
                    if let Some(uri) = uri {
                        *hosts.entry(host(&uri)).or_default() += 1;
                    }
                }
                _ => {
                    if subtype == "FileAttachment" {
                        if let Ok(spec) = annotation.get(b"FS") {
                            if !matches!(spec, Object::Reference(id) if attached.contains(id)) {
                                report.embedded_files.push(embedded_file(document, spec, None, Some(number)));
                            }
                        }
                    }
                    let group = groups.entry(subtype).or_default();
                    group.0 += 1;
                    if let Some(author) = get_text(document, annotation, b"T").filter(|author| !author.trim().is_empty()) {
                        group.1.insert(author);
                    }
                }
            }
        }
    }
    report.annotations = groups.into_iter().map(|(subtype, (count, authors))| AnnotationGroup { subtype, count, authors: authors.into_iter().collect() }).collect();
    report.links = hosts.into_iter().map(|(host, count)| LinkHost { host, count }).collect();
}

fn page_box(document: &Document, page_id: ObjectId) -> Option<[f32; 4]> {
    let value = inherited(document, page_id, b"CropBox").ok().flatten().or_else(|| inherited(document, page_id, b"MediaBox").ok().flatten())?;
    rectangle(document, &value)
}

fn rectangle(document: &Document, value: &Object) -> Option<[f32; 4]> {
    let values = resolve(document, value).ok()?.as_array().ok()?;
    let numbers = values.iter().map(|value| resolve(document, value).ok()?.as_float().ok()).collect::<Option<Vec<_>>>()?;
    let [x0, y0, x1, y1] = numbers.try_into().ok()?;
    Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
}

fn hidden_reason(document: &Document, annotation: &Dictionary, page_box: Option<[f32; 4]>) -> Option<HiddenReason> {
    const HIDDEN: i64 = 1 << 1;
    const NO_VIEW: i64 = 1 << 5;
    let flags = annotation.get(b"F").ok().and_then(|flags| resolve(document, flags).ok()?.as_i64().ok()).unwrap_or(0);
    if flags & (HIDDEN | NO_VIEW) != 0 {
        return Some(HiddenReason::Hidden);
    }
    let [left, bottom, right, top] = rectangle(document, annotation.get(b"Rect").ok()?)?;
    let [page_left, page_bottom, page_right, page_top] = page_box?;
    (right <= page_left || left >= page_right || top <= page_bottom || bottom >= page_top).then_some(HiddenReason::OffPage)
}

/// The host a link points to: the domain for web and mail links, otherwise the link's scheme.
pub(crate) fn host(uri: &str) -> String {
    let uri = uri.trim();
    let scheme = uri.split_once(':').filter(|(scheme, _)| {
        !scheme.is_empty() && scheme.chars().all(|character| character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.'))
    });
    let authority = match scheme {
        Some((_, rest)) if rest.starts_with("//") => rest[2..].split(['/', '?', '#']).next().unwrap_or_default(),
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("mailto") => rest.split('?').next().unwrap_or_default().split(',').next().unwrap_or_default(),
        Some((scheme, _)) => return format!("{}:", scheme.to_ascii_lowercase()),
        None => uri.split(['/', '?', '#']).next().unwrap_or_default(),
    };
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = match host.strip_prefix('[') {
        Some(ipv6) => ipv6.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    };
    match host.trim_end_matches('.').to_ascii_lowercase() {
        host if host.is_empty() => scheme.map_or_else(|| "No host".to_string(), |(scheme, _)| format!("{}:", scheme.to_ascii_lowercase())),
        host => host,
    }
}
