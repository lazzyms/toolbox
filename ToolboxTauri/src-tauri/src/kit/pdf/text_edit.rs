use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;
use lopdf::{content::{Content, Operation}, dictionary, Dictionary, Document, Encoding, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};
use crate::kit::contracts::{OutcomeWarning, ToolError, WarningKind};
use super::{inherited_ref, resolve, scene};

const CONTENT_LIMIT: usize = 64 * 1024 * 1024;
const MAX_RUNS: usize = 10_000;
const MAX_REPLACEMENT_CHARS: usize = 2_000;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextEdit { pub page_index: usize, pub run: usize, pub original: String, pub replacement: String }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotEditable { Picture, Encoding, Tagged, Unsupported }

impl NotEditable {
    pub fn message(self) -> &'static str {
        match self {
            Self::Picture => "This text is part of a picture, so it can't be edited. Use OCR first, or cover it with Add text.",
            Self::Encoding => "This text uses a font with no character map, so it can't be edited. Cover it with Add text instead.",
            Self::Tagged => "This text is linked to the PDF's accessibility tags, and changing it would break them, so it can't be edited.",
            Self::Unsupported => "This page's content can't be rewritten safely, so its text can't be edited.",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextEditRun {
    pub index: usize, pub text: String, pub x: f32, pub y: f32, pub width: f32, pub height: f32,
    pub font_name: String, pub font_size: f32, pub reason: Option<NotEditable>, pub substitute: Option<FontSubstitute>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontSubstitute { pub family: String, pub characters: String }

struct Font<'a> { encoding: Option<Encoding<'a>>, widths: Option<(i64, Vec<f32>)>, missing_width: f32, embedded: bool, name: String, substitute: scene::TextFont }

impl Font<'_> {
    fn width(&self, code: u8) -> f32 {
        self.widths.as_ref().and_then(|(first, widths)| widths.get(usize::try_from(i64::from(code) - first).ok()?).copied()).unwrap_or(self.missing_width)
    }
    fn decode(&self, bytes: &[u8]) -> Option<String> {
        let encoding = self.encoding.as_ref()?;
        let text = encoding.bytes_to_string(bytes).ok()?;
        (encoding.string_to_bytes(&text) == bytes).then_some(text)
    }
    fn encode_char(&self, c: char) -> Option<u8> {
        let encoding = self.encoding.as_ref()?;
        let bytes = encoding.string_to_bytes(c.encode_utf8(&mut [0; 4]));
        (bytes.len() == 1 && encoding.bytes_to_string(&bytes).ok()? == c.to_string()).then(|| bytes[0])
    }
    fn available(&self, c: char, used: Option<&HashSet<u8>>) -> Option<u8> {
        self.encode_char(c).filter(|code| !self.embedded || used.is_some_and(|used| used.contains(code)))
    }
}

fn substitute_font(name: &str, descriptor: Option<&Dictionary>, flags: Option<i64>) -> scene::TextFont {
    let lower = name.to_ascii_lowercase();
    let named = |words: &[&str]| words.iter().any(|word| lower.contains(word));
    let flag = |bit: i64| flags.is_some_and(|flags| flags & bit != 0);
    let entry = |key: &[u8]| descriptor.and_then(|descriptor| descriptor.get(key).ok()).and_then(number);
    let family = if flag(1) || named(&["courier", "mono", "consol", "menlo"]) { "Noto Sans Mono" }
        else if !named(&["sans"]) && (flag(2) || named(&["times", "serif", "georgia", "garamond", "cambria", "palatino", "minion", "caslon", "baskerville", "bodoni", "didot", "bookman", "century", "charter"])) { "Noto Serif" }
        else { "Noto Sans" };
    let bold = flag(1 << 18) || entry(b"FontWeight").is_some_and(|weight| weight >= 600.) || named(&["bold", "black", "heavy", "semibold", "demi"]);
    let italic = flag(64) || entry(b"ItalicAngle").is_some_and(|angle| angle != 0.) || named(&["italic", "oblique"]);
    scene::bundled_font(family, bold, italic).expect("every bundled family has every style")
}

fn standard_encoding_font() -> &'static Dictionary {
    static FONT: OnceLock<Dictionary> = OnceLock::new();
    FONT.get_or_init(|| dictionary! { "Type" => "Font", "Encoding" => "StandardEncoding" })
}

fn number(value: &Object) -> Option<f32> { value.as_float().ok().filter(|value| value.is_finite()) }

fn load_font<'a>(doc: &'a Document, font: &'a Dictionary) -> Font<'a> {
    let name = font.get(b"BaseFont").and_then(Object::as_name).map(|name| String::from_utf8_lossy(name).to_string()).unwrap_or_default();
    let descriptor = font.get(b"FontDescriptor").ok().and_then(|value| resolve(doc, value).ok()).and_then(|value| value.as_dict().ok());
    let embedded = descriptor.is_some_and(|descriptor| [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"].iter().any(|key| descriptor.has(key)));
    let flags = descriptor.and_then(|descriptor| descriptor.get(b"Flags").ok()).and_then(|value| value.as_i64().ok());
    let symbolic = flags.map_or(matches!(name.as_str(), "Symbol" | "ZapfDingbats"), |flags| flags & 4 != 0);
    let simple = matches!(font.get(b"Subtype").and_then(Object::as_name).ok(), Some(b"Type1" | b"TrueType" | b"MMType1"));
    let encoding = if !simple { None } else {
        match font.get(b"Encoding").ok().map(|value| resolve(doc, value)) {
            None if !embedded && !symbolic => standard_encoding_font().get_font_encoding(doc).ok(),
            Some(Ok(Object::Name(name))) if matches!(name.as_slice(), b"StandardEncoding" | b"WinAnsiEncoding" | b"MacRomanEncoding") => font.get_font_encoding(doc).ok(),
            Some(Ok(Object::Dictionary(encoding))) if encoding.get(b"Type").and_then(Object::as_name).ok() == Some(b"Encoding") => font.get_font_encoding(doc).ok(),
            _ => None,
        }
    };
    let widths = (|| {
        let first = font.get(b"FirstChar").ok()?.as_i64().ok()?;
        let widths = resolve(doc, font.get(b"Widths").ok()?).ok()?.as_array().ok()?;
        Some((first, widths.iter().map(|value| resolve(doc, value).ok().and_then(number).unwrap_or(0.)).collect()))
    })();
    let missing_width = descriptor.and_then(|descriptor| descriptor.get(b"MissingWidth").ok()).and_then(number)
        .unwrap_or(if name.starts_with("Courier") { 600. } else { 500. });
    let substitute = substitute_font(&name, descriptor, flags);
    Font { encoding, widths, missing_width, embedded, name: name.split_once('+').map_or(name.clone(), |(_, base)| base.to_string()), substitute }
}

fn resource_entry<'a>(doc: &'a Document, resources: Option<&'a Dictionary>, category: &[u8], name: &[u8]) -> Option<&'a Object> {
    let entries = resolve(doc, resources?.get(category).ok()?).ok()?.as_dict().ok()?;
    resolve(doc, entries.get(name).ok()?).ok()
}

type Matrix = [f32; 6];
const IDENTITY: Matrix = [1., 0., 0., 1., 0., 0.];
fn multiply(a: Matrix, b: Matrix) -> Matrix {
    [a[0] * b[0] + a[1] * b[2], a[0] * b[1] + a[1] * b[3], a[2] * b[0] + a[3] * b[2], a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4], a[4] * b[1] + a[5] * b[3] + b[5]]
}
fn transform(m: Matrix, x: f32, y: f32) -> (f32, f32) { (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5]) }
fn matrix(operands: &[Object]) -> Option<Matrix> {
    let values: Vec<f32> = operands.iter().map(number).collect::<Option<_>>()?;
    values.try_into().ok()
}

#[derive(Clone)]
struct State { ctm: Matrix, font: Option<Vec<u8>>, size: f32, char_spacing: f32, word_spacing: f32, scale: f32, leading: f32, rise: f32 }

struct Run { op: usize, font: Option<Vec<u8>>, size: f32, info: TextEditRun }

pub struct PageText<'a> { operations: Vec<Operation>, runs: Vec<Run>, used_codes: HashMap<Vec<u8>, HashSet<u8>>, fonts: HashMap<Vec<u8>, Font<'a>> }

impl PageText<'_> {
    pub fn runs(&self) -> Vec<TextEditRun> { self.runs.iter().map(|run| run.info.clone()).collect() }
}

fn shown_strings(operation: &Operation) -> Vec<&Object> {
    match operation.operator.as_str() {
        "Tj" | "'" => operation.operands.last().into_iter().collect(),
        "\"" => operation.operands.get(2).into_iter().collect(),
        "TJ" => operation.operands.first().and_then(|value| value.as_array().ok()).map(|items| items.iter().collect()).unwrap_or_default(),
        _ => vec![],
    }
}

fn scene_rect(corners: [(f32, f32); 4], to_scene: Matrix) -> (f32, f32, f32, f32) {
    let points = corners.map(|(x, y)| transform(to_scene, x, y));
    let (xs, ys) = (points.map(|p| p.0), points.map(|p| p.1));
    let (left, top) = (xs.iter().copied().fold(f32::INFINITY, f32::min), ys.iter().copied().fold(f32::INFINITY, f32::min));
    (left, top, xs.iter().copied().fold(f32::NEG_INFINITY, f32::max) - left, ys.iter().copied().fold(f32::NEG_INFINITY, f32::max) - top)
}

pub fn page_text(doc: &Document, page: ObjectId, to_scene: Matrix) -> Result<PageText<'_>, String> {
    let bytes = doc.get_page_content_with_limit(page, CONTENT_LIMIT).map_err(|error| error.to_string())?;
    let (operations, unsupported) = match Content::decode_strict(&bytes) {
        Ok(content) => { let unsupported = content.operations.iter().any(|operation| operation.operator == "BI"); (content.operations, unsupported) }
        Err(_) => (Content::decode(&bytes).map(|content| content.operations).unwrap_or_default(), true),
    };
    let resources = inherited_ref(doc, page, b"Resources")?.and_then(|value| value.as_dict().ok());
    let mut fonts: HashMap<Vec<u8>, Font> = HashMap::new();
    let mut state = State { ctm: IDENTITY, font: None, size: 0., char_spacing: 0., word_spacing: 0., scale: 1., leading: 0., rise: 0. };
    let mut saved = Vec::new();
    let mut marked: Vec<bool> = Vec::new();
    let (mut text_matrix, mut line_matrix) = (IDENTITY, IDENTITY);
    let mut runs = Vec::new();
    let mut used_codes: HashMap<Vec<u8>, HashSet<u8>> = HashMap::new();
    for (op, operation) in operations.iter().enumerate() {
        let operands = &operation.operands;
        let value = |index: usize| operands.get(index).and_then(number);
        match operation.operator.as_str() {
            "q" => saved.push(state.clone()),
            "Q" => if let Some(previous) = saved.pop() { state = previous },
            "cm" => if let Some(m) = matrix(operands) { state.ctm = multiply(m, state.ctm) },
            "BT" => { text_matrix = IDENTITY; line_matrix = IDENTITY; }
            "Tc" => state.char_spacing = value(0).unwrap_or(0.),
            "Tw" => state.word_spacing = value(0).unwrap_or(0.),
            "Tz" => state.scale = value(0).unwrap_or(100.) / 100.,
            "TL" => state.leading = value(0).unwrap_or(0.),
            "Ts" => state.rise = value(0).unwrap_or(0.),
            "Tf" => {
                state.font = operands.first().and_then(|name| name.as_name().ok()).map(<[u8]>::to_vec);
                state.size = value(1).unwrap_or(0.);
                if let Some(name) = state.font.clone() {
                    if let Some(font) = resource_entry(doc, resources, b"Font", &name).and_then(|value| value.as_dict().ok()) { fonts.entry(name).or_insert_with(|| load_font(doc, font)); }
                }
            }
            "Td" | "TD" => {
                let (x, y) = (value(0).unwrap_or(0.), value(1).unwrap_or(0.));
                if operation.operator == "TD" { state.leading = -y; }
                line_matrix = multiply([1., 0., 0., 1., x, y], line_matrix); text_matrix = line_matrix;
            }
            "Tm" => if let Some(m) = matrix(operands) { line_matrix = m; text_matrix = m; },
            "BDC" | "BMC" => {
                let properties = operands.get(1).and_then(|value| match value {
                    Object::Name(name) => resource_entry(doc, resources, b"Properties", name),
                    value => Some(value),
                }).and_then(|value| value.as_dict().ok());
                let tagged = properties.is_some_and(|properties| properties.has(b"MCID") || properties.has(b"ActualText"));
                marked.push(tagged || marked.last().copied().unwrap_or(false));
            }
            "EMC" => { marked.pop(); }
            "Do" | "BI" => {
                let image = operation.operator == "BI" || operands.first().and_then(|name| name.as_name().ok())
                    .and_then(|name| resource_entry(doc, resources, b"XObject", name)).and_then(|value| value.as_stream().ok())
                    .is_some_and(|stream| stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image"));
                if image && runs.len() < MAX_RUNS {
                    let (x, y, width, height) = scene_rect([(0., 0.), (1., 0.), (0., 1.), (1., 1.)], multiply(state.ctm, to_scene));
                    runs.push(Run { op, font: None, size: 0., info: TextEditRun { index: runs.len(), text: String::new(), x, y, width, height, font_name: String::new(), font_size: 0., reason: Some(NotEditable::Picture), substitute: None } });
                }
            }
            "T*" | "'" | "\"" => {
                if operation.operator == "\"" { state.word_spacing = value(0).unwrap_or(0.); state.char_spacing = value(1).unwrap_or(0.); }
                line_matrix = multiply([1., 0., 0., 1., 0., -state.leading], line_matrix); text_matrix = line_matrix;
            }
            _ => {}
        }
        if !matches!(operation.operator.as_str(), "Tj" | "TJ" | "'" | "\"") { continue; }
        let font = state.font.as_ref().and_then(|name| fonts.get(name));
        let start = text_matrix;
        let mut bytes = Vec::new();
        let mut advance = 0.;
        for item in shown_strings(operation) {
            match item {
                Object::String(string, _) => for &code in string {
                    bytes.push(code);
                    let glyph = font.map_or(500., |font| font.width(code)) / 1000.;
                    advance += (glyph * state.size + state.char_spacing + if code == b' ' { state.word_spacing } else { 0. }) * state.scale;
                }
                value => if let Some(adjust) = number(value) { advance -= adjust / 1000. * state.size * state.scale },
            }
        }
        text_matrix = multiply([1., 0., 0., 1., advance, 0.], text_matrix);
        if let Some(name) = &state.font { used_codes.entry(name.clone()).or_default().extend(bytes.iter().copied()); }
        let text = font.and_then(|font| font.decode(&bytes));
        if bytes.is_empty() || text.as_deref().is_some_and(|text| text.trim().is_empty()) || runs.len() >= MAX_RUNS { continue; }
        let reason = if unsupported { Some(NotEditable::Unsupported) }
            else if text.is_none() { Some(NotEditable::Encoding) }
            else if marked.last().copied().unwrap_or(false) { Some(NotEditable::Tagged) }
            else { None };
        let to_page = multiply(multiply(start, state.ctm), to_scene);
        let (descent, ascent) = (state.rise - 0.2 * state.size, state.rise + 0.8 * state.size);
        let (x, y, width, height) = scene_rect([(0., descent), (advance, descent), (0., ascent), (advance, ascent)], to_page);
        let font_size = state.size * (to_page[2] * to_page[2] + to_page[3] * to_page[3]).sqrt();
        runs.push(Run { op, font: state.font.clone(), size: state.size, info: TextEditRun {
            index: runs.len(), text: text.unwrap_or_default(), x, y, width, height,
            font_name: font.map(|font| font.name.clone()).unwrap_or_default(), font_size, reason, substitute: None,
        } });
    }
    for run in &mut runs {
        let Some(name) = &run.font else { continue };
        let Some(font) = fonts.get(name).filter(|font| font.embedded && run.info.reason.is_none()) else { continue };
        let mut codes: Vec<u8> = used_codes.get(name).into_iter().flatten().copied().collect();
        codes.sort_unstable();
        let characters = codes.into_iter().filter_map(|code| font.decode(&[code])).collect();
        run.info.substitute = Some(FontSubstitute { family: font.substitute.family.to_string(), characters });
    }
    Ok(PageText { operations, runs, used_codes, fonts })
}

struct Substitution { op: usize, face: &'static str, gids: Vec<u16>, original: Vec<u8>, size: f32 }
struct PagePlan { page: ObjectId, operations: Vec<Operation>, substitutions: Vec<Substitution> }

pub struct TextEditPlan { pages: Vec<PagePlan>, faces: BTreeMap<&'static str, (scene::TextFont, BTreeMap<u16, char>)>, warnings: Vec<OutcomeWarning> }

impl TextEditPlan {
    pub fn warnings(&self) -> &[OutcomeWarning] { &self.warnings }
}

fn refuse(message: impl Into<String>) -> ToolError { ToolError::invalid_input(message) }

fn replace_operation(operation: &Operation, encoded: Vec<u8>) -> Operation {
    let format = shown_strings(operation).first().and_then(|value| match value { Object::String(_, format) => Some(format.clone()), _ => None }).unwrap_or(lopdf::StringFormat::Literal);
    let string = Object::String(encoded, format);
    let mut operands = operation.operands.clone();
    match operation.operator.as_str() {
        "TJ" => operands = vec![Object::Array(vec![string])],
        "\"" => operands[2] = string,
        _ => *operands.last_mut().unwrap() = string,
    }
    Operation { operator: operation.operator.clone(), operands }
}

fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn substitution_warnings(substituted: Vec<(String, &'static str, usize, String)>) -> Vec<OutcomeWarning> {
    let mut grouped: Vec<((String, &'static str), Vec<(usize, String)>)> = Vec::new();
    for (requested, actual, page, text) in substituted {
        match grouped.iter_mut().find(|(key, _)| key.0 == requested && key.1 == actual) {
            Some((_, runs)) => runs.push((page, text)),
            None => grouped.push(((requested, actual), vec![(page, text)])),
        }
    }
    grouped.into_iter().map(|((requested, actual), runs)| {
        let mut pages: Vec<usize> = runs.iter().map(|(page, _)| *page).collect();
        pages.dedup();
        let pages = list(&pages.iter().map(usize::to_string).collect::<Vec<_>>());
        let texts = list(&runs.iter().map(|(_, text)| format!("“{text}”")).collect::<Vec<_>>());
        let noun = if runs.len() == 1 { "line" } else { "lines" };
        OutcomeWarning {
            kind: WarningKind::FontSubstituted { requested_family: requested.clone(), actual_family: actual.to_string() },
            message: format!("Font substituted on page{} {pages}. {requested} in this PDF doesn't include every character typed, so the edited {noun} {texts} use{} {actual}. Untouched text keeps its font.",
                if pages.contains(' ') { "s" } else { "" }, if runs.len() == 1 { "s" } else { "" }),
        }
    }).collect()
}

pub fn preflight(doc: &Document, edits: &[TextEdit], to_scene: impl Fn(ObjectId) -> Result<Matrix, String>) -> Result<TextEditPlan, ToolError> {
    if edits.is_empty() { return Err(refuse("No text was changed, so there is nothing to export.")); }
    let pages: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    let mut by_page: Vec<(usize, Vec<&TextEdit>)> = Vec::new();
    for edit in edits {
        if edit.page_index >= pages.len() { return Err(refuse("A text edit points to a page that isn't in this PDF.")); }
        match by_page.iter_mut().find(|(page, _)| *page == edit.page_index) {
            Some((_, list)) => list.push(edit),
            None => by_page.push((edit.page_index, vec![edit])),
        }
    }
    let mut plan = TextEditPlan { pages: Vec::new(), faces: BTreeMap::new(), warnings: Vec::new() };
    let mut substituted = Vec::new();
    for (page_index, edits) in by_page {
        let page_number = page_index + 1;
        let page_id = pages[page_index];
        let text = page_text(doc, page_id, to_scene(page_id).map_err(ToolError::processing)?).map_err(ToolError::processing)?;
        let mut page = PagePlan { page: page_id, operations: text.operations.clone(), substitutions: Vec::new() };
        let mut seen = HashSet::new();
        for edit in edits {
            let stale = || refuse(format!("The text you edited on page {page_number} no longer matches the PDF. Reopen the file and edit it again."));
            let run = text.runs.get(edit.run).ok_or_else(stale)?;
            if run.info.text != edit.original { return Err(stale()); }
            if let Some(reason) = run.info.reason { return Err(refuse(reason.message())); }
            if !seen.insert(edit.run) { return Err(refuse(format!("The same text on page {page_number} was edited twice in one export."))); }
            if edit.replacement == edit.original { return Err(refuse(format!("A text edit on page {page_number} doesn't change the text."))); }
            if edit.replacement.chars().count() > MAX_REPLACEMENT_CHARS { return Err(refuse("Replacement text must be 2,000 characters or fewer.")); }
            if edit.replacement.chars().any(char::is_control) { return Err(refuse("Line breaks and control characters can't be added to existing text.")); }
            let font_name = run.font.as_ref().ok_or_else(stale)?;
            let font = text.fonts.get(font_name).ok_or_else(stale)?;
            let used = text.used_codes.get(font_name);
            if let Some(encoded) = edit.replacement.chars().map(|c| font.available(c, used)).collect::<Option<Vec<u8>>>() {
                page.operations[run.op] = replace_operation(&page.operations[run.op], encoded);
                continue;
            }
            if !font.embedded {
                let c = edit.replacement.chars().find(|c| font.encode_char(*c).is_none()).unwrap_or_default();
                return Err(refuse(format!("“{c}” isn't in the character set of this text's font, so it can't be typed here. Cover the text with Add text instead.")));
            }
            let face = scene::parse_face(font.substitute.bytes)?;
            let mut gids = Vec::new();
            let mut missing = Vec::new();
            for c in edit.replacement.chars() {
                match face.glyph_index(c) {
                    Some(id) if id.0 != 0 => gids.push((id.0, c)),
                    _ => if !missing.contains(&c) { missing.push(c) },
                }
            }
            if !missing.is_empty() {
                let named = missing.iter().take(10).map(|c| format!("“{c}”")).collect::<Vec<_>>();
                return Err(refuse(format!("Neither {} nor {} can show {} on page {page_number}. Cover the text with Add text instead.", font.name, font.substitute.family, list(&named))));
            }
            let glyphs = &mut plan.faces.entry(font.substitute.name).or_insert_with(|| (font.substitute, BTreeMap::new())).1;
            for (gid, c) in &gids { glyphs.entry(*gid).or_insert(*c); }
            page.substitutions.push(Substitution { op: run.op, face: font.substitute.name, gids: gids.into_iter().map(|(gid, _)| gid).collect(), original: font_name.clone(), size: run.size });
            substituted.push((font.name.clone(), font.substitute.family, page_number, edit.replacement.clone()));
        }
        plan.pages.push(page);
    }
    plan.warnings = substitution_warnings(substituted);
    Ok(plan)
}

fn operation(operator: &str, operands: Vec<Object>) -> Operation { Operation::new(operator, operands) }

fn substituted_operations(original: &Operation, resource: &str, remapper: &subsetter::GlyphRemapper, substitution: &Substitution) -> Vec<Operation> {
    let mut operations = match original.operator.as_str() {
        "'" => vec![operation("T*", vec![])],
        "\"" => vec![operation("Tw", vec![original.operands[0].clone()]), operation("Tc", vec![original.operands[1].clone()]), operation("T*", vec![])],
        _ => vec![],
    };
    let glyphs = substitution.gids.iter().flat_map(|gid| remapper.get(*gid).unwrap_or(0).to_be_bytes()).collect();
    operations.push(operation("Tf", vec![Object::Name(resource.as_bytes().to_vec()), Object::Real(substitution.size)]));
    operations.push(operation("Tj", vec![Object::String(glyphs, lopdf::StringFormat::Hexadecimal)]));
    operations.push(operation("Tf", vec![Object::Name(substitution.original.clone()), Object::Real(substitution.size)]));
    operations
}

pub fn apply(doc: &mut Document, plan: TextEditPlan) -> Result<(), String> {
    let mut faces = HashMap::new();
    for (name, (font, glyphs)) in &plan.faces { faces.insert(*name, scene::embed_font(doc, *font, glyphs)?); }
    for PagePlan { page, mut operations, mut substitutions } in plan.pages {
        if !substitutions.is_empty() {
            let mut effective = scene::effective_resource_entries(doc, page, b"Font")?;
            let mut added = Dictionary::new();
            let mut names = HashMap::new();
            for substitution in &substitutions {
                if names.contains_key(substitution.face) { continue; }
                let name = scene::next_resource_name(&effective, "Font", &format!("EditText{}", substitution.face.replace('-', "")))?;
                effective.set(name.as_str(), Object::Null);
                added.set(name.as_str(), Object::Reference(faces[substitution.face].0));
                names.insert(substitution.face, name);
            }
            let resources = scene::scene_resources(doc, page, Dictionary::new(), added, Dictionary::new())?;
            doc.get_dictionary_mut(page).map_err(|error| error.to_string())?.set("Resources", resources);
            substitutions.sort_by_key(|substitution| std::cmp::Reverse(substitution.op));
            for substitution in &substitutions {
                let replacement = substituted_operations(&operations[substitution.op], &names[substitution.face], &faces[substitution.face].1, substitution);
                operations.splice(substitution.op..=substitution.op, replacement);
            }
        }
        let bytes = Content { operations }.encode().map_err(|error| error.to_string())?;
        let mut stream = Stream::new(dictionary! {}, bytes);
        let _ = stream.compress();
        let content = doc.add_object(stream);
        doc.get_dictionary_mut(page).map_err(|error| error.to_string())?.set("Contents", content);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_follow_the_original_font_family_weight_and_slant() {
        let pick = |name: &str, descriptor: Option<Dictionary>| {
            let flags = descriptor.as_ref().and_then(|descriptor| descriptor.get(b"Flags").ok()).and_then(|value| value.as_i64().ok());
            substitute_font(name, descriptor.as_ref(), flags).name
        };
        assert_eq!(pick("Helvetica", None), "NotoSans-Regular");
        assert_eq!(pick("TimesNewRomanPS-BoldItalicMT", None), "NotoSerif-BoldItalic");
        assert_eq!(pick("MicrosoftSansSerif", None), "NotoSans-Regular");
        assert_eq!(pick("CourierNewPS-ItalicMT", None), "NotoSansMono-Regular");
        assert_eq!(pick("Consolas-Bold", None), "NotoSansMono-Bold");
        assert_eq!(pick("F1", Some(dictionary! { "Flags" => 2 | 32, "FontWeight" => 700, "ItalicAngle" => -12 })), "NotoSerif-BoldItalic");
        assert_eq!(pick("F2", Some(dictionary! { "Flags" => 1 })), "NotoSansMono-Regular");
        assert_eq!(pick("F3", Some(dictionary! { "Flags" => 32 | 64 })), "NotoSans-Italic");
    }
}
