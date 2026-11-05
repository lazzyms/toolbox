use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8, WINDOWS_1252};
use lopdf::{dictionary, Document, Object, Stream};
use serde::Deserialize;

use crate::kit::common::batch_runner::{BatchRunner, RunEvents};
use crate::kit::common::{JobOutcome, OutputLocation, OutputNaming};
use crate::kit::contracts::{native_capabilities, SkipCode, SkipReason, ToolError};
use crate::kit::pdf::font_resolver::{FontResolver, FontStyle, SystemFontLookup};
use crate::kit::pdf::unicode_font::UnicodeFont;

pub const COMMAND: &str = "convert_to_pdf";
const ALREADY_PDF: &str = "Already a PDF. Nothing was made.";
const FONT_SIZE: f32 = 10.;
const LEADING: f32 = 13.;
const TAB_WIDTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PageSize { #[default] A4, Letter, Legal }

impl PageSize {
    fn points(self) -> (f32, f32) {
        match self {
            Self::A4 => (595.28, 841.89),
            Self::Letter => (612., 792.),
            Self::Legal => (612., 1008.),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Margins { Narrow, #[default] Normal, Wide }

impl Margins {
    fn points(self) -> f32 {
        match self {
            Self::Narrow => 36.,
            Self::Normal => 72.,
            Self::Wide => 108.,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextFont { #[default] Monospace, Sans, Serif }

impl TextFont {
    fn family(self) -> &'static str {
        match self {
            Self::Monospace => "Courier New",
            Self::Sans => "Arial",
            Self::Serif => "Times New Roman",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextEncoding { #[default] Detect, Utf8, Utf16le, Utf16be, Windows1252 }

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageDefaults {
    #[serde(default)] pub page_size: PageSize,
    #[serde(default)] pub margins: Margins,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlainTextOptions {
    #[serde(default)] pub page_size: Option<PageSize>,
    #[serde(default)] pub margins: Option<Margins>,
    pub font: TextFont,
    pub encoding: TextEncoding,
    pub wrap_lines: bool,
}

impl Default for PlainTextOptions {
    fn default() -> Self { Self { page_size: None, margins: None, font: TextFont::default(), encoding: TextEncoding::default(), wrap_lines: true } }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConvertToPdfRequest {
    pub paths: Vec<PathBuf>,
    pub output_location: OutputLocation,
    #[serde(default)] pub page_defaults: PageDefaults,
    #[serde(default)] pub plain_text: Option<PlainTextOptions>,
}

pub struct ConvertJob<S: SystemFontLookup> {
    request: ConvertToPdfRequest,
    fonts: FontResolver<S>,
}

pub fn run(events: &RunEvents, request: serde_json::Value) -> Vec<JobOutcome> {
    run_with(events, request, FontResolver::installed)
}

pub fn run_with<S, F>(events: &RunEvents, request: serde_json::Value, fonts: F) -> Vec<JobOutcome>
where
    S: SystemFontLookup + Send + Sync + 'static,
    F: FnOnce() -> Result<FontResolver<S>, ToolError>,
{
    let paths = request_paths(&request);
    let fail_all = |error: ToolError| paths.iter().cloned().map(|path| JobOutcome::failure(path, error.clone())).collect();
    let request = match parse_request(request) { Ok(request) => request, Err(error) => return fail_all(error) };
    let fonts = match fonts() { Ok(fonts) => fonts, Err(error) => return fail_all(error) };
    let job = Arc::new(ConvertJob { request, fonts });
    let location = job.request.output_location.clone();
    BatchRunner::run(events, COMMAND, job.request.paths.clone(), &location, &job, convert)
}

fn request_paths(request: &serde_json::Value) -> Vec<PathBuf> {
    request.get("paths").and_then(serde_json::Value::as_array).map(|paths| {
        paths.iter().filter_map(serde_json::Value::as_str).map(PathBuf::from).collect()
    }).unwrap_or_default()
}

fn section_key(family_id: &str) -> String {
    let mut parts = family_id.split('-');
    let first = parts.next().unwrap_or_default().to_string();
    parts.fold(first, |mut key, part| {
        let mut chars = part.chars();
        if let Some(head) = chars.next() { key.extend(head.to_uppercase()); key.push_str(chars.as_str()); }
        key
    })
}

fn parse_request(request: serde_json::Value) -> Result<ConvertToPdfRequest, ToolError> {
    if let Some(fields) = request.as_object() {
        let families = native_capabilities().iter().find(|capability| capability.command == COMMAND).map(|capability| capability.format_families.as_slice()).unwrap_or_default();
        if let Some(family) = families.iter().find(|family| !family.native_available && fields.contains_key(&section_key(&family.id))) {
            let name = family.id.replace('-', " ");
            let mut chars = name.chars();
            let name = chars.next().map(|head| head.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default();
            return Err(ToolError::unavailable(format!("{name} options come in a later update.")));
        }
    }
    serde_json::from_value(request).map_err(|error| ToolError::invalid_input(format!("These conversion options can't be used: {error}")))
}

fn convert<S: SystemFontLookup>(job: &ConvertJob<S>, input: PathBuf) -> JobOutcome {
    let extension = input.extension().and_then(|extension| extension.to_str()).unwrap_or_default().to_ascii_lowercase();
    if extension == "pdf" {
        return JobOutcome::skipped(input, SkipReason::new(SkipCode::AlreadyInTargetFormat, ALREADY_PDF));
    }
    let options = job.request.plain_text.unwrap_or_default();
    match plain_text(job, &options, &input) {
        Ok(outcome) => outcome,
        Err(error) => JobOutcome::failure(input, error),
    }
}

fn plain_text<S: SystemFontLookup>(job: &ConvertJob<S>, options: &PlainTextOptions, input: &Path) -> Result<JobOutcome, ToolError> {
    let bytes = fs::read(input).map_err(|error| ToolError::processing(format!("Could not read the file: {error}")))?;
    let (text, encoding) = decode(&bytes, options.encoding)?;
    let resolved = job.fonts.resolve(options.font.family(), FontStyle::Regular, &[])?;
    let mut font = UnicodeFont::parse(resolved.data)?;
    let size = options.page_size.unwrap_or(job.request.page_defaults.page_size);
    let margin = options.margins.unwrap_or(job.request.page_defaults.margins).points();
    let (width, height) = size.points();
    let text_width = width - 2. * margin;
    let lines_per_page = (((height - 2. * margin) / LEADING).floor() as usize).max(1);

    let mut measure = Measure { font: &mut font, widths: HashMap::new() };
    let mut pages: Vec<Vec<String>> = vec![Vec::new()];
    for (page_index, page) in text.split('\u{c}').enumerate() {
        if page_index > 0 { pages.push(Vec::new()); }
        for line in page.lines() {
            let line = expand_tabs(line);
            let wrapped = if options.wrap_lines { measure.wrap(&line, text_width)? } else { vec![line] };
            for row in wrapped {
                if pages.last().is_some_and(|rows| rows.len() >= lines_per_page) { pages.push(Vec::new()); }
                pages.last_mut().expect("pages always has one page").push(row);
            }
        }
    }

    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let mut contents = Vec::with_capacity(pages.len());
    for rows in &pages {
        let mut content = String::new();
        for (index, row) in rows.iter().enumerate() {
            if row.trim().is_empty() { continue; }
            let run = font.show(row, FONT_SIZE)?;
            let baseline = height - margin - FONT_SIZE - index as f32 * LEADING;
            content.push_str(&format!("BT /F1 {FONT_SIZE} Tf {margin} {baseline:.2} Td {} ET\n", run.operators));
        }
        contents.push(content);
    }
    let font_id = font.embed(&mut document)?;
    let kids = contents.into_iter().map(|content| {
        let content_id = document.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        Object::Reference(document.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(width), Object::Real(height)],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
            "Contents" => content_id,
        }))
    }).collect::<Vec<_>>();
    let page_count = kids.len();
    document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => page_count as i64 }.into());
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    document.compress();

    let output = OutputNaming::reserve_destination(input, &job.request.output_location, "", "pdf")
        .map_err(|error| ToolError::processing(format!("Could not reserve the PDF output: {error}")))?;
    document.save(output.path()).map_err(|error| ToolError::processing(format!("Could not write the PDF: {error}")))?;
    let path = output.publish().map_err(|error| ToolError::processing(format!("Could not save the PDF: {error}")))?;
    let pages_label = if page_count == 1 { "1 page".to_string() } else { format!("{page_count} pages") };
    Ok(JobOutcome::succeeded(input.to_path_buf(), vec![path], format!("{pages_label}. Read as {encoding}.")).with_fonts(vec![resolved.report]))
}

fn decode(bytes: &[u8], encoding: TextEncoding) -> Result<(String, &'static str), ToolError> {
    let strict = |encoding: &'static Encoding, label: &'static str, bytes: &[u8]| {
        encoding.decode_without_bom_handling_and_without_replacement(bytes)
            .map(|text| (text.into_owned(), label))
            .ok_or_else(|| ToolError::invalid_input(format!("This file isn't valid {label}. Pick another encoding.")))
    };
    let without_bom = |target: &'static Encoding| match Encoding::for_bom(bytes) {
        Some((found, length)) if found == target => &bytes[length..],
        _ => bytes,
    };
    let (text, label) = match encoding {
        TextEncoding::Utf8 => strict(UTF_8, "UTF-8", without_bom(UTF_8))?,
        TextEncoding::Utf16le => strict(UTF_16LE, "UTF-16 LE", without_bom(UTF_16LE))?,
        TextEncoding::Utf16be => strict(UTF_16BE, "UTF-16 BE", without_bom(UTF_16BE))?,
        TextEncoding::Windows1252 => (WINDOWS_1252.decode_without_bom_handling(bytes).0.into_owned(), "Windows-1252"),
        TextEncoding::Detect => match Encoding::for_bom(bytes) {
            Some((found, length)) => {
                let label = if found == UTF_8 { "UTF-8" } else if found == UTF_16LE { "UTF-16 LE" } else { "UTF-16 BE" };
                strict(found, label, &bytes[length..])?
            }
            None if bytes.contains(&0) => return Err(ToolError::invalid_input("This file doesn't look like plain text, so no PDF was made.")),
            None => match std::str::from_utf8(bytes) {
                Ok(text) => (text.to_string(), "UTF-8"),
                Err(_) => (WINDOWS_1252.decode_without_bom_handling(bytes).0.into_owned(), "Windows-1252"),
            },
        },
    };
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    Ok((text.chars().filter(|character| !character.is_control() || matches!(character, '\n' | '\t' | '\u{c}')).collect(), label))
}

fn expand_tabs(line: &str) -> String {
    let mut expanded = String::with_capacity(line.len());
    let mut column = 0;
    for character in line.chars() {
        if character == '\t' {
            let spaces = TAB_WIDTH - column % TAB_WIDTH;
            expanded.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            expanded.push(character);
            column += 1;
        }
    }
    expanded
}

struct Measure<'a> {
    font: &'a mut UnicodeFont,
    widths: HashMap<char, f32>,
}

impl Measure<'_> {
    fn width(&mut self, character: char) -> Result<f32, ToolError> {
        if let Some(width) = self.widths.get(&character) { return Ok(*width); }
        let width = self.font.show(character.encode_utf8(&mut [0; 4]), FONT_SIZE)?.width;
        self.widths.insert(character, width);
        Ok(width)
    }

    fn wrap(&mut self, line: &str, limit: f32) -> Result<Vec<String>, ToolError> {
        let mut rows = Vec::new();
        let mut row = String::new();
        let mut row_width = 0.;
        for word in line.split_inclusive(' ') {
            let mut word_width = 0.;
            for character in word.chars() { word_width += self.width(character)?; }
            let visible = word_width - if word.ends_with(' ') { self.width(' ')? } else { 0. };
            if row_width + visible <= limit {
                row.push_str(word);
                row_width += word_width;
                continue;
            }
            if !row.is_empty() {
                rows.push(row.trim_end().to_string());
                row = String::new();
                row_width = 0.;
            }
            for character in word.chars() {
                let width = self.width(character)?;
                if row_width + width > limit && !row.is_empty() && character != ' ' {
                    rows.push(std::mem::take(&mut row));
                    row_width = 0.;
                }
                row.push(character);
                row_width += width;
            }
        }
        rows.push(row.trim_end().to_string());
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::contracts::{ErrorKind, OutcomeState};

    fn succeeded(outcome: &JobOutcome) -> bool { matches!(outcome.state, OutcomeState::Succeeded { .. }) }
    fn skip(outcome: &JobOutcome) -> &SkipReason { match &outcome.state { OutcomeState::Skipped { reason } => reason, other => panic!("not skipped: {other:?}") } }
    use crate::kit::pdf::font_resolver::{BundledFonts, FontSource, SystemFace};
    use serde_json::json;

    struct NoSystemFonts;
    impl SystemFontLookup for NoSystemFonts {
        fn faces(&self, _family: &str) -> Vec<SystemFace> { Vec::new() }
    }

    fn fonts() -> Result<FontResolver<NoSystemFonts>, ToolError> {
        Ok(FontResolver::new(BundledFonts::discover(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/fonts"))?, NoSystemFonts))
    }

    fn sandbox(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("toolbox_convert_to_pdf_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn convert_paths(paths: &[&Path], options: serde_json::Value) -> Vec<JobOutcome> {
        let mut request = json!({ "paths": paths, "outputLocation": "alongsideInput" });
        request.as_object_mut().unwrap().extend(options.as_object().unwrap().clone());
        run_with(&RunEvents::silent(), request, fonts)
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut names = fs::read_dir(folder).unwrap().flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        names.sort();
        names
    }

    fn plain(wrap: bool) -> serde_json::Value {
        json!({ "plainText": { "font": "monospace", "encoding": "detect", "wrapLines": wrap } })
    }

    #[test]
    fn non_utf8_text_with_a_long_line_becomes_a_wrapped_searchable_pdf() {
        let root = sandbox("long_line");
        let input = root.join("caf\u{e9}-log.txt");
        let long_line = (0..120).map(|index| format!("word{index}")).collect::<Vec<_>>().join(" ");
        let mut bytes = b"Caf\xe9 cr\xe8me na\xefve\n".to_vec();
        bytes.extend_from_slice(long_line.as_bytes());
        bytes.push(b'\n');
        let original = bytes.clone();
        fs::write(&input, &bytes).unwrap();

        let outcomes = convert_paths(&[&input], json!({ "plainText": { "pageSize": "letter", "font": "monospace", "encoding": "detect", "wrapLines": true } }));
        assert_eq!(outcomes.len(), 1);
        let outcome = &outcomes[0];
        assert!(succeeded(outcome), "{outcome:?}");
        assert_eq!(outcome.detail, "1 page. Read as Windows-1252.");
        let output = root.join("caf\u{e9}-log.pdf");
        assert_eq!(outcome.output_paths(), [output.clone()]);
        assert_eq!(outcome.fonts.len(), 1);
        assert_eq!(outcome.fonts[0].requested_family, "Courier New");
        assert_eq!(outcome.fonts[0].font_used, "Liberation Mono");
        assert_eq!(outcome.fonts[0].source, FontSource::BundledFallback);

        let document = Document::load(&output).unwrap();
        let pages = document.get_pages();
        assert_eq!(pages.len(), 1);
        let page = document.get_dictionary(pages[&1]).unwrap();
        let media_box = page.get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|value| value.as_float().unwrap()).collect::<Vec<_>>();
        assert_eq!(media_box, [0., 0., 612., 792.]);
        let text = document.extract_text(&[1]).unwrap();
        assert!(text.contains("Caf\u{e9} cr\u{e8}me na\u{ef}ve"), "{text}");
        assert!(text.contains("word0") && text.contains("word119"), "{text}");
        let content = document.get_page_content(pages[&1]);
        let rows = String::from_utf8_lossy(&content).matches("BT ").count();
        assert!(rows > 2, "the long line wraps onto several rows, found {rows}");
        assert_eq!(fs::read(&input).unwrap(), original);
        assert_eq!(names(&root), ["caf\u{e9}-log.pdf", "caf\u{e9}-log.txt"]);
    }

    #[test]
    fn many_lines_paginate_and_unwrapped_lines_stay_on_one_row() {
        let root = sandbox("paginate");
        let input = root.join("lines.txt");
        let text = (0..130).map(|index| format!("line {index}\tend")).collect::<Vec<_>>().join("\n");
        fs::write(&input, format!("{text}\n{}", "x".repeat(400))).unwrap();
        let outcomes = convert_paths(&[&input], json!({ "pageDefaults": { "pageSize": "a4", "margins": "normal" }, "plainText": { "font": "sans", "encoding": "utf8", "wrapLines": false } }));
        assert!(succeeded(&outcomes[0]), "{:?}", outcomes[0]);
        assert_eq!(outcomes[0].detail, "3 pages. Read as UTF-8.");
        assert_eq!(outcomes[0].fonts[0].font_used, "Liberation Sans");
        let document = Document::load(root.join("lines.pdf")).unwrap();
        assert_eq!(document.get_pages().len(), 3);
        assert!(document.extract_text(&[3]).unwrap().contains(&"x".repeat(400)));
    }

    #[test]
    fn pdf_inputs_are_skipped_and_a_damaged_file_fails_alone() {
        let root = sandbox("mixed");
        let pdf = root.join("already.PDF");
        let damaged = root.join("damaged.txt");
        let good = root.join("good.txt");
        fs::write(&pdf, b"%PDF-1.7 not really").unwrap();
        fs::write(&damaged, b"\x00\x01binary\x00junk").unwrap();
        fs::write(&good, "Hello").unwrap();
        let before = [fs::read(&pdf).unwrap(), fs::read(&damaged).unwrap(), fs::read(&good).unwrap()];

        let outcomes = convert_paths(&[&pdf, &damaged, &good], plain(true));
        assert_eq!(skip(&outcomes[0]), &SkipReason::new(SkipCode::AlreadyInTargetFormat, ALREADY_PDF));
        assert!(outcomes[0].output_paths().is_empty());
                assert_eq!(outcomes[1].error().unwrap().kind, ErrorKind::InvalidInput);
        assert!(succeeded(&outcomes[2]));
        assert_eq!(outcomes[2].detail, "1 page. Read as UTF-8.");
        assert_eq!([fs::read(&pdf).unwrap(), fs::read(&damaged).unwrap(), fs::read(&good).unwrap()], before);
        assert_eq!(names(&root), ["already.PDF", "damaged.txt", "good.pdf", "good.txt"]);
    }

    #[test]
    fn forced_encodings_decode_or_fail_without_leaving_files() {
        let root = sandbox("encodings");
        let utf16 = root.join("wide.txt");
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "\u{3b1}lpha".encode_utf16() { bytes.extend_from_slice(&unit.to_le_bytes()); }
        fs::write(&utf16, &bytes).unwrap();
        let detected = convert_paths(&[&utf16], plain(true));
        assert_eq!(detected[0].detail, "1 page. Read as UTF-16 LE.");
        assert!(Document::load(root.join("wide.pdf")).unwrap().extract_text(&[1]).unwrap().contains("\u{3b1}lpha"));

        let latin = root.join("latin.txt");
        fs::write(&latin, b"caf\xe9").unwrap();
        let forced = convert_paths(&[&latin], json!({ "plainText": { "font": "serif", "encoding": "utf8", "wrapLines": true } }));
        assert_eq!(forced[0].error().unwrap().kind, ErrorKind::InvalidInput);
        assert_eq!(forced[0].error().unwrap().message, "This file isn't valid UTF-8. Pick another encoding.");
        assert_eq!(names(&root), ["latin.txt", "wide.pdf", "wide.txt"]);
    }

    #[test]
    fn repeated_runs_never_overwrite_and_are_byte_identical() {
        let root = sandbox("repeat");
        let input = root.join("notes.txt");
        fs::write(&input, "Same text\nevery time\n").unwrap();
        for _ in 0..3 { assert!(succeeded(&convert_paths(&[&input], plain(true))[0])); }
        assert_eq!(names(&root), ["notes-1.pdf", "notes-2.pdf", "notes.pdf", "notes.txt"]);
        let first = fs::read(root.join("notes.pdf")).unwrap();
        assert_eq!(fs::read(root.join("notes-1.pdf")).unwrap(), first);
        assert_eq!(fs::read(root.join("notes-2.pdf")).unwrap(), first);
    }

    #[test]
    fn unknown_options_are_invalid_and_unshipped_families_are_unavailable() {
        let root = sandbox("options");
        let input = root.join("notes.txt");
        fs::write(&input, "text").unwrap();
        for options in [
            json!({ "plainText": { "font": "monospace", "encoding": "detect", "wrapLines": true, "fontSize": 12 } }),
            json!({ "plainText": { "font": "comic", "encoding": "detect", "wrapLines": true } }),
            json!({ "pageDefaults": { "pageSize": "a3" } }),
            json!({ "colour": true }),
        ] {
            let outcomes = convert_paths(&[&input], options.clone());
            assert_eq!(outcomes.len(), 1);
            assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::InvalidInput, "{options}");
        }
        let outcomes = convert_paths(&[&input], json!({ "markdown": { "toc": true } }));
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::Unavailable);
        assert_eq!(outcomes[0].error().unwrap().message, "Markdown options come in a later update.");
        let outcomes = convert_paths(&[&input], json!({ "webPages": {} }));
        assert_eq!(outcomes[0].error().unwrap().message, "Web pages options come in a later update.");
        let mobi = root.join("book.mobi");
        fs::write(&mobi, "mobi").unwrap();
        let outcomes = convert_paths(&[&mobi, &input], plain(true));
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::Unavailable);
        assert!(succeeded(&outcomes[1]));
        assert_eq!(names(&root), ["book.mobi", "notes.pdf", "notes.txt"]);
    }

    #[test]
    fn missing_glyphs_and_unavailable_fonts_fail_without_temporary_files() {
        let root = sandbox("missing_glyph");
        let input = root.join("emoji.txt");
        fs::write(&input, "rocket \u{1F680}").unwrap();
        let outcomes = convert_paths(&[&input], plain(true));
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::Processing);
        assert!(outcomes[0].error().unwrap().message.starts_with("The font has no glyph for"));
        assert_eq!(names(&root), ["emoji.txt"]);

        let request = json!({ "paths": [input], "outputLocation": "alongsideInput" });
        let outcomes = run_with::<NoSystemFonts, _>(&RunEvents::silent(), request, || Err(ToolError::unavailable("fonts missing")));
        assert_eq!(outcomes[0].error().unwrap().kind, ErrorKind::Unavailable);
        assert_eq!(names(&root), ["emoji.txt"]);
    }
}
