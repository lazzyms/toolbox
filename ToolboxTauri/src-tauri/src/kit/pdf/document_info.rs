use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use lopdf::{Document, LoadOptions, Object};
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::{escape::unescape, NsReader, XmlVersion};
use serde::{Serialize, Serializer};

use crate::kit::contracts::ToolError;
use crate::kit::password::WRONG_PASSWORD;

const XMP_MAX_BYTES: usize = 4 * 1024 * 1024;
const RDF: &[u8] = b"http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const DC: &[u8] = b"http://purl.org/dc/elements/1.1/";
const XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/";
const PDF: &[u8] = b"http://ns.adobe.com/pdf/1.3/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DocumentProperty {
    Title,
    Author,
    Subject,
    Keywords,
    Creator,
    Producer,
    CreationDate,
    ModificationDate,
}

impl DocumentProperty {
    const ALL: [Self; 8] = [Self::Title, Self::Author, Self::Subject, Self::Keywords, Self::Creator, Self::Producer, Self::CreationDate, Self::ModificationDate];

    fn info_key(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Author => "Author",
            Self::Subject => "Subject",
            Self::Keywords => "Keywords",
            Self::Creator => "Creator",
            Self::Producer => "Producer",
            Self::CreationDate => "CreationDate",
            Self::ModificationDate => "ModDate",
        }
    }

    fn xmp_name(self) -> (&'static [u8], &'static [u8]) {
        match self {
            Self::Title => (DC, b"title"),
            Self::Author => (DC, b"creator"),
            Self::Subject => (DC, b"description"),
            Self::Keywords => (PDF, b"Keywords"),
            Self::Creator => (XMP, b"CreatorTool"),
            Self::Producer => (PDF, b"Producer"),
            Self::CreationDate => (XMP, b"CreateDate"),
            Self::ModificationDate => (XMP, b"ModifyDate"),
        }
    }

    fn from_xmp(namespace: &[u8], local: &[u8]) -> Option<Self> {
        Self::ALL.into_iter().find(|property| property.xmp_name() == (namespace, local))
    }

    fn is_date(self) -> bool {
        matches!(self, Self::CreationDate | Self::ModificationDate)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdfDate {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub offset_minutes: Option<i16>,
}

impl PdfDate {
    fn same_moment(&self, other: &Self) -> bool {
        match (self.offset_minutes, other.offset_minutes) {
            (Some(left), Some(right)) => self.local_seconds() - i64::from(left) * 60 == other.local_seconds() - i64::from(right) * 60,
            _ => self == other,
        }
    }

    fn local_seconds(&self) -> i64 {
        let (year, month) = (i64::from(self.year), i64::from(self.month));
        let shifted = if month <= 2 { year - 1 } else { year };
        let era = shifted.div_euclid(400);
        let year_of_era = shifted - era * 400;
        let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + i64::from(self.day) - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        let days = era * 146_097 + day_of_era;
        days * 86_400 + i64::from(self.hour) * 3_600 + i64::from(self.minute) * 60 + i64::from(self.second)
    }
}

impl fmt::Display for PdfDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", self.year, self.month, self.day, self.hour, self.minute, self.second)?;
        match self.offset_minutes {
            Some(0) => write!(formatter, "Z"),
            Some(offset) => write!(formatter, "{}{:02}:{:02}", if offset < 0 { '-' } else { '+' }, offset.unsigned_abs() / 60, offset.unsigned_abs() % 60),
            None => Ok(()),
        }
    }
}

impl Serialize for PdfDate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfProperties {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub creation_date: Option<PdfDate>,
    pub modification_date: Option<PdfDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PropertyValue {
    Text(String),
    Date(PdfDate),
}

impl PropertyValue {
    fn agrees_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Date(left), Self::Date(right)) => left.same_moment(right),
            _ => self == other,
        }
    }
}

impl fmt::Display for PropertyValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => formatter.write_str(text),
            Self::Date(date) => date.fmt(formatter),
        }
    }
}

impl PdfProperties {
    fn get(&self, property: DocumentProperty) -> Option<PropertyValue> {
        let text = |value: &Option<String>| value.clone().map(PropertyValue::Text);
        match property {
            DocumentProperty::Title => text(&self.title),
            DocumentProperty::Author => text(&self.author),
            DocumentProperty::Subject => text(&self.subject),
            DocumentProperty::Keywords => text(&self.keywords),
            DocumentProperty::Creator => text(&self.creator),
            DocumentProperty::Producer => text(&self.producer),
            DocumentProperty::CreationDate => self.creation_date.map(PropertyValue::Date),
            DocumentProperty::ModificationDate => self.modification_date.map(PropertyValue::Date),
        }
    }

    fn set(&mut self, property: DocumentProperty, value: PropertyValue) {
        match (property, value) {
            (DocumentProperty::Title, PropertyValue::Text(text)) => self.title = Some(text),
            (DocumentProperty::Author, PropertyValue::Text(text)) => self.author = Some(text),
            (DocumentProperty::Subject, PropertyValue::Text(text)) => self.subject = Some(text),
            (DocumentProperty::Keywords, PropertyValue::Text(text)) => self.keywords = Some(text),
            (DocumentProperty::Creator, PropertyValue::Text(text)) => self.creator = Some(text),
            (DocumentProperty::Producer, PropertyValue::Text(text)) => self.producer = Some(text),
            (DocumentProperty::CreationDate, PropertyValue::Date(date)) => self.creation_date = Some(date),
            (DocumentProperty::ModificationDate, PropertyValue::Date(date)) => self.modification_date = Some(date),
            _ => unreachable!("property value kind matches its property"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XmpSummary {
    #[serde(flatten)]
    pub values: PdfProperties,
    pub properties: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyConflict {
    pub property: DocumentProperty,
    pub info: String,
    pub xmp: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfDocumentInfo {
    pub info: PdfProperties,
    pub xmp: Option<XmpSummary>,
    pub conflicts: Vec<PropertyConflict>,
    pub unsupported: Vec<String>,
}

pub fn read(path: &Path, password: Option<&str>) -> Result<PdfDocumentInfo, ToolError> {
    load(&read_bytes(path)?, password).map(|document| from_document(&document))
}

pub(crate) fn read_bytes(path: &Path) -> Result<Vec<u8>, ToolError> {
    std::fs::read(path).map_err(|error| ToolError::processing(format!("Could not read the PDF: {error}")))
}

/// Opens a PDF held in memory, decrypting it with the open password when it needs one.
pub(crate) fn load(bytes: &[u8], password: Option<&str>) -> Result<Document, ToolError> {
    let options = LoadOptions { password: password.map(str::to_owned), ..LoadOptions::default() };
    let document = Document::load_mem_with_options(bytes, options).map_err(super::pdf_load_error)?;
    if document.is_encrypted() {
        return Err(match password {
            Some(_) => ToolError::wrong_password(WRONG_PASSWORD),
            None => ToolError::invalid_input("Enter the PDF password."),
        });
    }
    Ok(document)
}

pub(crate) fn from_document(document: &Document) -> PdfDocumentInfo {
    let mut unsupported = Vec::new();
    let info = read_info(document, &mut unsupported);
    let xmp = read_xmp(document, &mut unsupported);
    let conflicts = xmp.as_ref().map(|xmp| conflicts(&info, &xmp.values)).unwrap_or_default();
    PdfDocumentInfo { info, xmp, conflicts, unsupported }
}

fn conflicts(info: &PdfProperties, xmp: &PdfProperties) -> Vec<PropertyConflict> {
    DocumentProperty::ALL.into_iter().filter_map(|property| match (info.get(property), xmp.get(property)) {
        (Some(info), Some(xmp)) if !info.agrees_with(&xmp) => Some(PropertyConflict { property, info: info.to_string(), xmp: xmp.to_string() }),
        _ => None,
    }).collect()
}

fn read_info(document: &Document, unsupported: &mut Vec<String>) -> PdfProperties {
    let mut properties = PdfProperties::default();
    let Ok(info) = document.trailer.get(b"Info") else { return properties };
    let Ok(info) = super::resolve(document, info).and_then(|info| info.as_dict().map_err(|error| error.to_string())) else {
        unsupported.push("The document information dictionary could not be read.".to_string());
        return properties;
    };
    for property in DocumentProperty::ALL {
        let key = property.info_key();
        let Ok(value) = info.get(key.as_bytes()) else { continue };
        let text = super::resolve(document, value).ok().and_then(decode_text);
        let value = match text {
            Some(text) if property.is_date() => parse_pdf_date(&text).map(PropertyValue::Date),
            text => text.map(PropertyValue::Text),
        };
        match value {
            Some(value) => properties.set(property, value),
            None => unsupported.push(format!("Document information {key} could not be decoded.")),
        }
    }
    properties
}

pub(crate) fn decode_text(object: &Object) -> Option<String> {
    let bytes = object.as_str().ok()?;
    if let Some(utf16) = bytes.strip_prefix(b"\xFE\xFF") {
        if utf16.len() % 2 != 0 {
            return None;
        }
        return String::from_utf16(&utf16.chunks(2).map(|pair| u16::from_be_bytes([pair[0], pair[1]])).collect::<Vec<_>>()).ok();
    }
    if let Some(utf8) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8(utf8.to_vec()).ok();
    }
    bytes.iter().map(|&byte| match byte {
        b'\t' | b'\n' | b'\r' | 0x20..=0x7E => Some(char::from(byte)),
        _ => {
            let text = lopdf::decode_text_string(&Object::string_literal(vec![byte])).ok()?;
            let mut chars = text.chars();
            chars.next().filter(|_| chars.next().is_none())
        }
    }).collect()
}

fn read_xmp(document: &Document, unsupported: &mut Vec<String>) -> Option<XmpSummary> {
    let catalog = document.catalog().ok()?;
    let metadata = catalog.get(b"Metadata").ok()?;
    let packet = super::resolve(document, metadata).ok()
        .and_then(|metadata| metadata.as_stream().ok())
        .and_then(|stream| stream.decompressed_content_with_limit(XMP_MAX_BYTES).ok());
    let Some(packet) = packet else {
        unsupported.push("The XMP metadata stream could not be read.".to_string());
        return None;
    };
    match parse_xmp(&packet, unsupported) {
        Ok(summary) => Some(summary),
        Err(error) => {
            unsupported.push(format!("The XMP metadata is not valid XML: {error}"));
            None
        }
    }
}

struct OpenProperty {
    name: String,
    property: Option<DocumentProperty>,
    container: Option<Vec<u8>>,
    items: Vec<(Option<String>, String)>,
    in_item: bool,
    text: String,
}

fn parse_xmp(packet: &[u8], unsupported: &mut Vec<String>) -> Result<XmpSummary, String> {
    let mut reader = NsReader::from_reader(packet);
    let mut buffer = Vec::new();
    let mut stack: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut description_depth = None;
    let mut open: Option<OpenProperty> = None;
    let mut found: Vec<(String, Option<DocumentProperty>, String)> = Vec::new();
    loop {
        let (namespace, event) = reader.read_resolved_event_into(&mut buffer).map_err(|error| error.to_string())?;
        let namespace = match namespace {
            ResolveResult::Bound(namespace) => namespace.as_ref().to_vec(),
            _ => Vec::new(),
        };
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let local = element.local_name().as_ref().to_vec();
                let depth = stack.len();
                let is_rdf = |name: &[u8]| namespace == RDF && local == name;
                if is_rdf(b"Description") && stack.last().is_some_and(|(parent_ns, parent)| parent_ns == RDF && parent == b"RDF") {
                    found.extend(description_attributes(&reader, element)?);
                    if !empty {
                        description_depth = Some(depth);
                    }
                } else if description_depth.is_some_and(|description| depth == description + 1) {
                    let name = String::from_utf8_lossy(element.name().as_ref()).into_owned();
                    let property = DocumentProperty::from_xmp(&namespace, &local);
                    if empty {
                        found.push((name, property, String::new()));
                    } else {
                        open = Some(OpenProperty { name, property, container: None, items: Vec::new(), in_item: false, text: String::new() });
                    }
                } else if let Some(current) = open.as_mut() {
                    if namespace == RDF && matches!(local.as_slice(), b"Alt" | b"Seq" | b"Bag") {
                        current.container = Some(local.clone());
                    } else if is_rdf(b"li") {
                        current.items.push((attribute(element, b"xml:lang"), String::new()));
                        current.in_item = !empty;
                    }
                }
                if !empty {
                    stack.push((namespace, local));
                }
            }
            Event::End(_) => {
                let closed = stack.pop();
                let depth = stack.len();
                if description_depth == Some(depth) {
                    description_depth = None;
                } else if description_depth.is_some_and(|description| depth == description + 1) {
                    if let Some(current) = open.take() {
                        found.push((current.name.clone(), current.property, property_value(current)));
                    }
                } else if let Some(current) = open.as_mut().filter(|_| closed.as_ref().is_some_and(|(namespace, local)| namespace == RDF && local == b"li")) {
                    current.in_item = false;
                }
            }
            Event::Text(ref text) => append_text(&mut open, &text.xml_content(XmlVersion::Implicit1_0).map_err(|error| error.to_string())?),
            Event::CData(ref text) => append_text(&mut open, &text.decode().map_err(|error| error.to_string())?),
            Event::GeneralRef(ref reference) => {
                let name = reference.decode().map_err(|error| error.to_string())?;
                append_text(&mut open, &unescape(&format!("&{name};")).map_err(|error| error.to_string())?);
            }
            Event::Eof if stack.is_empty() => break,
            Event::Eof => return Err("the packet ends before its elements are closed".to_string()),
            _ => {}
        }
        buffer.clear();
    }
    let mut summary = XmpSummary::default();
    let mut names = BTreeSet::new();
    for (name, property, value) in found {
        names.insert(name.clone());
        let Some(property) = property else { continue };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let value = if property.is_date() { parse_xmp_date(value).map(PropertyValue::Date) } else { Some(PropertyValue::Text(value.to_string())) };
        match value {
            Some(value) => summary.values.set(property, value),
            None => unsupported.push(format!("XMP {name} could not be decoded.")),
        }
    }
    summary.properties = names.into_iter().collect();
    Ok(summary)
}

fn description_attributes(reader: &NsReader<&[u8]>, element: &BytesStart<'_>) -> Result<Vec<(String, Option<DocumentProperty>, String)>, String> {
    let mut values = Vec::new();
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| error.to_string())?;
        let (namespace, local) = reader.resolver().resolve_attribute(attribute.key);
        let ResolveResult::Bound(namespace) = namespace else { continue };
        if namespace.as_ref() == RDF || namespace.as_ref() == b"http://www.w3.org/XML/1998/namespace" || attribute.key.as_namespace_binding().is_some() {
            continue;
        }
        let property = DocumentProperty::from_xmp(namespace.as_ref(), local.as_ref());
        let value = attribute.normalized_value(XmlVersion::Implicit1_0).map_err(|error| error.to_string())?.into_owned();
        values.push((String::from_utf8_lossy(attribute.key.as_ref()).into_owned(), property, value));
    }
    Ok(values)
}

fn attribute(element: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    element.attributes().flatten().find(|attribute| attribute.key.as_ref() == name)
        .and_then(|attribute| attribute.normalized_value(XmlVersion::Implicit1_0).ok().map(|value| value.into_owned()))
}

fn append_text(open: &mut Option<OpenProperty>, text: &str) {
    let Some(current) = open.as_mut() else { return };
    match current.items.last_mut() {
        Some((_, item)) if current.in_item => item.push_str(text),
        _ => current.text.push_str(text),
    }
}

fn property_value(property: OpenProperty) -> String {
    if property.items.is_empty() {
        return property.text;
    }
    if property.container.as_deref() == Some(b"Alt") {
        return property.items.iter().find(|(language, _)| language.as_deref() == Some("x-default")).or(property.items.first()).map(|(_, text)| text.clone()).unwrap_or_default();
    }
    property.items.into_iter().map(|(_, text)| text.trim().to_string()).filter(|text| !text.is_empty()).collect::<Vec<_>>().join("; ")
}

struct Cursor<'a>(&'a [u8]);

impl Cursor<'_> {
    fn digits(&mut self, count: usize) -> Option<u32> {
        let digits = self.0.get(..count).filter(|digits| digits.iter().all(u8::is_ascii_digit))?;
        self.0 = &self.0[count..];
        Some(digits.iter().fold(0, |value, digit| value * 10 + u32::from(digit - b'0')))
    }

    fn optional_digits(&mut self, count: usize) -> Option<Option<u32>> {
        if self.0.is_empty() || !self.0[0].is_ascii_digit() { Some(None) } else { self.digits(count).map(Some) }
    }

    fn eat(&mut self, byte: u8) -> bool {
        let found = self.0.first() == Some(&byte);
        if found {
            self.0 = &self.0[1..];
        }
        found
    }
}

pub(crate) fn parse_pdf_date(text: &str) -> Option<PdfDate> {
    let text = text.trim();
    let mut cursor = Cursor(text.strip_prefix("D:").unwrap_or(text).as_bytes());
    let year = cursor.digits(4)?;
    let mut fields = [1, 1, 0, 0, 0];
    for field in &mut fields {
        match cursor.optional_digits(2)? {
            Some(value) => *field = value,
            None => break,
        }
    }
    let offset = match cursor.0.first() {
        None => None,
        Some(b'Z') => {
            cursor.eat(b'Z');
            let _ = pdf_offset(&mut cursor)?;
            Some(0)
        }
        Some(&sign @ (b'+' | b'-')) => {
            cursor.eat(sign);
            let minutes = pdf_offset(&mut cursor)?;
            Some(if sign == b'-' { -minutes } else { minutes })
        }
        Some(_) => return None,
    };
    if !cursor.0.is_empty() {
        return None;
    }
    date(year, fields, offset)
}

fn pdf_offset(cursor: &mut Cursor<'_>) -> Option<i32> {
    let Some(hours) = cursor.optional_digits(2)? else { return Some(0) };
    cursor.eat(b'\'');
    let minutes = cursor.optional_digits(2)?.unwrap_or(0);
    cursor.eat(b'\'');
    offset_minutes(hours, minutes)
}

fn parse_xmp_date(text: &str) -> Option<PdfDate> {
    let mut cursor = Cursor(text.as_bytes());
    let year = cursor.digits(4)?;
    let mut fields = [1, 1, 0, 0, 0];
    let mut parsed = 0;
    for (field, separator) in fields.iter_mut().zip([b'-', b'-', b'T', b':', b':']) {
        if !cursor.eat(separator) {
            break;
        }
        *field = cursor.digits(2)?;
        parsed += 1;
    }
    if parsed == 3 {
        return None;
    }
    if parsed == 5 && cursor.eat(b'.') {
        cursor.digits(1)?;
        while cursor.digits(1).is_some() {}
    }
    let offset = match cursor.0.first() {
        None => None,
        Some(_) if parsed < 4 => return None,
        Some(b'Z') => {
            cursor.eat(b'Z');
            Some(0)
        }
        Some(&sign @ (b'+' | b'-')) => {
            cursor.eat(sign);
            let hours = cursor.digits(2)?;
            if !cursor.eat(b':') {
                return None;
            }
            let minutes = offset_minutes(hours, cursor.digits(2)?)?;
            Some(if sign == b'-' { -minutes } else { minutes })
        }
        Some(_) => return None,
    };
    if !cursor.0.is_empty() {
        return None;
    }
    date(year, fields, offset)
}

fn offset_minutes(hours: u32, minutes: u32) -> Option<i32> {
    (hours < 24 && minutes < 60).then(|| (hours * 60 + minutes) as i32)
}

fn date(year: u32, [month, day, hour, minute, second]: [u32; 5], offset: Option<i32>) -> Option<PdfDate> {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if day == 0 || day > days || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some(PdfDate {
        year: year as u16,
        month: month as u8,
        day: day as u8,
        hour: hour as u8,
        minute: minute as u8,
        second: second as u8,
        offset_minutes: offset.map(|offset| offset as i16),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::contracts::ErrorKind;
    use lopdf::{dictionary, Dictionary, EncryptionState, EncryptionVersion, Permissions, Stream, StringFormat};
    use std::path::PathBuf;

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("toolbox-document-info-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn read(&self, bytes: &[u8], password: Option<&str>) -> Result<PdfDocumentInfo, ToolError> {
            let path = self.0.join("input.pdf");
            std::fs::write(&path, bytes).unwrap();
            let result = read(&path, password);
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "the original must stay byte-identical");
            assert_eq!(std::fs::read_dir(&self.0).unwrap().count(), 1, "the reader must create no files");
            result
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn literal(bytes: &[u8]) -> Object {
        Object::String(bytes.to_vec(), StringFormat::Literal)
    }

    fn utf16(text: &str) -> Object {
        let mut bytes = vec![0xFE, 0xFF];
        bytes.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        Object::String(bytes, StringFormat::Hexadecimal)
    }

    fn pdf(info: Option<Dictionary>, xmp: Option<&str>, encryption: Option<(&str, &str)>) -> Vec<u8> {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let content_id = document.add_object(Stream::new(dictionary! {}, b"BT ET".to_vec()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1 }.into());
        let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
        if let Some(xmp) = xmp {
            let mut stream = Stream::new(dictionary! { "Type" => "Metadata", "Subtype" => "XML" }, xmp.as_bytes().to_vec());
            stream.compress().unwrap();
            catalog.set("Metadata", document.add_object(stream));
        }
        let catalog_id = document.add_object(catalog);
        document.trailer.set("Root", catalog_id);
        if let Some(info) = info {
            let info_id = document.add_object(info);
            document.trailer.set("Info", info_id);
        }
        if let Some((owner_password, user_password)) = encryption {
            document.trailer.set("ID", Object::Array(vec![literal(b"toolbox-info-id-1"), literal(b"toolbox-info-id-2")]));
            let state = EncryptionState::try_from(EncryptionVersion::V2 { document: &document, owner_password, user_password, key_length: 128, permissions: Permissions::PRINTABLE }).unwrap();
            document.encrypt(&state).unwrap();
        }
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    fn xmp(description: &str) -> String {
        format!(r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">{description}</rdf:RDF></x:xmpmeta>
<?xpacket end="w"?>"#)
    }

    fn date(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8, offset_minutes: Option<i16>) -> PdfDate {
        PdfDate { year, month, day, hour, minute, second, offset_minutes }
    }

    #[test]
    fn reads_every_information_entry_without_xmp() {
        let sandbox = Sandbox::new("info");
        let info = dictionary! {
            "Title" => utf16("Quarterly report \u{2013} \u{65e5}\u{672c}"),
            "Author" => literal(b"Ada Lovelace"),
            "Subject" => literal(b"Line one\nline two \x80"),
            "Keywords" => literal(b"finance, q3"),
            "Creator" => literal(b"\xEF\xBB\xBFWriter \xE2\x9C\x93"),
            "Producer" => literal(b"Toolbox Fixture 1.0"),
            "CreationDate" => literal(b"D:20240102030405+05'30'"),
            "ModDate" => literal(b"D:20240607080910"),
        };

        let report = sandbox.read(&pdf(Some(info), None, None), None).unwrap();

        assert_eq!(report.info, PdfProperties {
            title: Some("Quarterly report \u{2013} \u{65e5}\u{672c}".into()),
            author: Some("Ada Lovelace".into()),
            subject: Some("Line one\nline two \u{2022}".into()),
            keywords: Some("finance, q3".into()),
            creator: Some("Writer \u{2713}".into()),
            producer: Some("Toolbox Fixture 1.0".into()),
            creation_date: Some(date(2024, 1, 2, 3, 4, 5, Some(330))),
            modification_date: Some(date(2024, 6, 7, 8, 9, 10, None)),
        });
        assert_eq!(report.xmp, None);
        assert!(report.conflicts.is_empty());
        assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["info"]["creationDate"], "2024-01-02T03:04:05+05:30");
        assert_eq!(json["info"]["modificationDate"], "2024-06-07T08:09:10");
    }

    #[test]
    fn a_pdf_without_information_or_xmp_reports_nothing() {
        let sandbox = Sandbox::new("empty");
        let report = sandbox.read(&pdf(None, None, None), None).unwrap();
        assert_eq!(report, PdfDocumentInfo::default());
    }

    #[test]
    fn summarizes_xmp_and_lists_values_that_disagree_with_the_information_dictionary() {
        let sandbox = Sandbox::new("conflict");
        let info = dictionary! {
            "Title" => literal(b"Info title"),
            "Author" => literal(b"Ada Lovelace; Charles Babbage"),
            "Producer" => literal(b"Info producer"),
            "CreationDate" => literal(b"D:20240102030405Z"),
            "ModDate" => literal(b"D:20240102030405Z"),
        };
        let packet = xmp(r#"
<rdf:Description rdf:about="" xmlns:pdf="http://ns.adobe.com/pdf/1.3/" xmlns:xmpMM="http://ns.adobe.com/xap/1.0/mm/" pdf:Producer="XMP producer" pdf:Keywords="a &amp; b" xmpMM:DocumentID="uuid:1234"/>
<rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:xap="http://ns.adobe.com/xap/1.0/">
  <dc:title><rdf:Alt><rdf:li xml:lang="fr">Titre</rdf:li><rdf:li xml:lang="x-default">XMP title</rdf:li></rdf:Alt></dc:title>
  <dc:creator><rdf:Seq><rdf:li>Ada Lovelace</rdf:li><rdf:li>Charles Babbage</rdf:li></rdf:Seq></dc:creator>
  <dc:description><rdf:Alt><rdf:li xml:lang="x-default"><![CDATA[Only in <XMP>]]></rdf:li></rdf:Alt></dc:description>
  <xap:CreatorTool>Fixture Writer</xap:CreatorTool>
  <xap:CreateDate>2024-01-02T08:34:05+05:30</xap:CreateDate>
  <xap:ModifyDate>2024-01-02T03:04:06Z</xap:ModifyDate>
</rdf:Description>"#);

        let report = sandbox.read(&pdf(Some(info), Some(&packet), None), None).unwrap();

        let xmp = report.xmp.unwrap();
        assert_eq!(xmp.values, PdfProperties {
            title: Some("XMP title".into()),
            author: Some("Ada Lovelace; Charles Babbage".into()),
            subject: Some("Only in <XMP>".into()),
            keywords: Some("a & b".into()),
            creator: Some("Fixture Writer".into()),
            producer: Some("XMP producer".into()),
            creation_date: Some(date(2024, 1, 2, 8, 34, 5, Some(330))),
            modification_date: Some(date(2024, 1, 2, 3, 4, 6, Some(0))),
        });
        assert_eq!(xmp.properties, ["dc:creator", "dc:description", "dc:title", "pdf:Keywords", "pdf:Producer", "xap:CreateDate", "xap:CreatorTool", "xap:ModifyDate", "xmpMM:DocumentID"]);
        assert_eq!(report.conflicts, vec![
            PropertyConflict { property: DocumentProperty::Title, info: "Info title".into(), xmp: "XMP title".into() },
            PropertyConflict { property: DocumentProperty::Producer, info: "Info producer".into(), xmp: "XMP producer".into() },
            PropertyConflict { property: DocumentProperty::ModificationDate, info: "2024-01-02T03:04:05Z".into(), xmp: "2024-01-02T03:04:06Z".into() },
        ]);
        assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
    }

    #[test]
    fn malformed_xmp_is_reported_as_unsupported_and_the_information_dictionary_is_still_read() {
        let sandbox = Sandbox::new("malformed");
        for packet in [xmp(r#"<rdf:Description><dc:title>Broken</rdf:Description>"#), r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF"#.to_string()] {
            let report = sandbox.read(&pdf(Some(dictionary! { "Title" => literal(b"Kept") }), Some(&packet), None), None).unwrap();

            assert_eq!(report.info.title.as_deref(), Some("Kept"));
            assert_eq!(report.xmp, None);
            assert_eq!(report.unsupported.len(), 1);
            assert!(report.unsupported[0].starts_with("The XMP metadata is not valid XML"), "{:?}", report.unsupported);
        }
    }

    #[test]
    fn undecodable_values_are_reported_as_unsupported() {
        let sandbox = Sandbox::new("undecodable");
        let info = dictionary! {
            "Title" => 42,
            "Author" => Object::String(vec![0xFE, 0xFF, 0x00, 0x41, 0x00], StringFormat::Hexadecimal),
            "Subject" => Object::String(vec![0xFE, 0xFF, 0xD8, 0x00], StringFormat::Hexadecimal),
            "Keywords" => literal(b"bad \x7F byte"),
            "Producer" => literal(b"Readable"),
            "CreationDate" => literal(b"D:20241345"),
            "ModDate" => literal(b"yesterday"),
        };
        let packet = xmp(r#"<rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:CreateDate="2024-02-30"/>"#);

        let report = sandbox.read(&pdf(Some(info), Some(&packet), None), None).unwrap();

        assert_eq!(report.info, PdfProperties { producer: Some("Readable".into()), ..PdfProperties::default() });
        assert_eq!(report.unsupported, [
            "Document information Title could not be decoded.",
            "Document information Author could not be decoded.",
            "Document information Subject could not be decoded.",
            "Document information Keywords could not be decoded.",
            "Document information CreationDate could not be decoded.",
            "Document information ModDate could not be decoded.",
            "XMP xmp:CreateDate could not be decoded.",
        ]);
        assert_eq!(report.xmp.unwrap().properties, ["xmp:CreateDate"]);
    }

    #[test]
    fn encrypted_pdfs_are_read_with_their_open_password() {
        let sandbox = Sandbox::new("encrypted");
        let protected = pdf(Some(dictionary! { "Title" => literal(b"Secret title") }), None, Some(("owner", crate::kit::TEST_PASSWORD)));

        let report = sandbox.read(&protected, Some(crate::kit::TEST_PASSWORD)).unwrap();
        assert_eq!(report.info.title.as_deref(), Some("Secret title"));
        assert!(matches!(sandbox.read(&protected, Some("wrong")).unwrap_err().kind, ErrorKind::WrongPassword));
        assert!(matches!(sandbox.read(&protected, None).unwrap_err().kind, ErrorKind::InvalidInput));

        let restricted = pdf(Some(dictionary! { "Title" => literal(b"Restricted title") }), None, Some(("owner", "")));
        assert_eq!(sandbox.read(&restricted, None).unwrap().info.title.as_deref(), Some("Restricted title"));
    }

    #[test]
    fn damaged_pdfs_fail_as_unreadable() {
        let sandbox = Sandbox::new("damaged");
        assert!(matches!(sandbox.read(b"%PDF-1.7\nnot a pdf", None).unwrap_err().kind, ErrorKind::Processing));
    }

    #[test]
    fn parses_pdf_and_xmp_dates_with_and_without_time_zones() {
        for (text, expected) in [
            ("D:2024", Some(date(2024, 1, 1, 0, 0, 0, None))),
            ("D:202402291530", Some(date(2024, 2, 29, 15, 30, 0, None))),
            ("20240102030405", Some(date(2024, 1, 2, 3, 4, 5, None))),
            ("D:20240102030405Z", Some(date(2024, 1, 2, 3, 4, 5, Some(0)))),
            ("D:20240102030405Z00'00'", Some(date(2024, 1, 2, 3, 4, 5, Some(0)))),
            ("D:20240102030405-08'00'", Some(date(2024, 1, 2, 3, 4, 5, Some(-480)))),
            ("D:20240102030405+0530", Some(date(2024, 1, 2, 3, 4, 5, Some(330)))),
            ("D:20230229", None),
            ("D:2024010203040", None),
            ("D:20240102030405+25'00'", None),
            ("D:20240102030405 junk", None),
        ] {
            assert_eq!(parse_pdf_date(text), expected, "{text}");
        }
        for (text, expected) in [
            ("2024", Some(date(2024, 1, 1, 0, 0, 0, None))),
            ("2024-03", Some(date(2024, 3, 1, 0, 0, 0, None))),
            ("2024-03-04T05:06", Some(date(2024, 3, 4, 5, 6, 0, None))),
            ("2024-03-04T05:06:07.123-04:00", Some(date(2024, 3, 4, 5, 6, 7, Some(-240)))),
            ("2024-03-04T05:06:07Z", Some(date(2024, 3, 4, 5, 6, 7, Some(0)))),
            ("2024-03-04T05", None),
            ("2024-03-04Z", None),
            ("2024-03-04T05:06:07+0400", None),
        ] {
            assert_eq!(parse_xmp_date(text), expected, "{text}");
        }
    }
}
