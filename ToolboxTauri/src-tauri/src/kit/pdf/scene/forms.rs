use std::collections::{HashMap, HashSet};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use serde::Serialize;
use crate::kit::contracts::ToolError;
use super::{compress_bytes, embed_text_fonts, err, geometry, layout_text_for, load, resolve, text_content, Kind, Layer, Rect, SceneObject, TextAlign, TextLayout, TEXT_LINE_HEIGHT};

const READ_ONLY: i64 = 1;
const REQUIRED: i64 = 1 << 1;
const MULTILINE: i64 = 1 << 12;
const PASSWORD: i64 = 1 << 13;
const RADIO: i64 = 1 << 15;
const PUSHBUTTON: i64 = 1 << 16;
const COMBO: i64 = 1 << 17;
const EDIT: i64 = 1 << 18;
const FILE_SELECT: i64 = 1 << 20;
const MULTI_SELECT: i64 = 1 << 21;
const FONT_RESOURCE: &str = "ToolboxFormFont";
const PADDING: f32 = 2.;

pub const XFA_UNSUPPORTED: &str = "This form can't be filled here. It was built with XFA, an older Adobe form format that Toolbox doesn't support.";
pub const SCRIPTED_UNSUPPORTED: &str = "This form can't be filled here. Its fields run scripts to format what you type, which Toolbox doesn't support.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind { Text, Checkbox, Radio, Dropdown, Listbox }

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormWidget {
    pub page: usize, pub x: f32, pub y: f32, pub width: f32, pub height: f32,
    #[serde(skip_serializing_if = "Option::is_none")] pub option: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub enum Format {
    Date { pattern: String },
    Number { decimals: usize, separator: u8, negative: u8, currency: String, currency_prepend: bool },
    Percent { decimals: usize, separator: u8 },
    Special { special: u8 },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormField {
    pub name: String,
    pub label: Option<String>,
    #[serde(rename = "type")] pub kind: FieldKind,
    pub value: serde_json::Value,
    pub options: Vec<String>,
    pub option_labels: Vec<String>,
    pub flags: i64,
    pub required: bool,
    pub read_only: bool,
    pub multiline: bool,
    pub multi_select: bool,
    pub editable: bool,
    pub max_len: Option<i64>,
    pub format: Option<Format>,
    pub widgets: Vec<FormWidget>,
    #[serde(skip)] id: ObjectId,
    #[serde(skip)] widget_ids: Vec<ObjectId>,
    #[serde(skip)] widget_states: Vec<Option<Vec<u8>>>,
    #[serde(skip)] on_state: Vec<u8>,
    #[serde(skip)] appearance: String,
    #[serde(skip)] align: TextAlign,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInspection { pub fields: Vec<FormField>, pub unsupported: Option<String>, pub comments: usize }

#[derive(Clone)]
struct Inherited {
    name: String, kind: Option<Vec<u8>>, flags: i64, appearance: Option<String>, quadding: Option<i64>, value: Option<Object>,
    max_len: Option<i64>, options: Option<Object>, format: Option<Format>, scripted: bool,
}

struct Terminal { id: ObjectId, inherited: Inherited, label: Option<String>, widgets: Vec<ObjectId> }

pub fn inspect_form(path: &std::path::Path) -> Result<FormInspection, String> {
    let doc = load(path)?;
    Ok(FormInspection { comments: super::flatten::count(&doc, super::flatten::FlattenChoice::Annotations)?, ..read(&doc)? })
}

fn text(doc: &Document, value: Option<&Object>) -> Option<String> {
    value.and_then(|value| resolve(doc, value).ok()).and_then(|value| lopdf::decode_text_string(value).ok())
}

fn integer(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<i64> {
    dict.get(key).ok().and_then(|value| resolve(doc, value).ok()).and_then(|value| value.as_i64().ok())
}

enum Arg { Number(f64), Text(String), Bool(bool) }

fn parse_call(js: &str) -> Option<(&str, Vec<Arg>)> {
    let js = js.trim().trim_end_matches(';').trim_end();
    let open = js.find('(')?;
    let name = js[..open].trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { return None; }
    let mut chars = js[open + 1..].strip_suffix(')')?.chars().peekable();
    let mut args = Vec::new();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else { break };
        if first == '"' || first == '\'' {
            chars.next();
            let mut text = String::new();
            loop {
                match chars.next()? {
                    '\\' => text.push(chars.next()?),
                    c if c == first => break,
                    c => text.push(c),
                }
            }
            args.push(Arg::Text(text));
        } else {
            let mut token = String::new();
            while let Some(c) = chars.next_if(|c| *c != ',') { token.push(c); }
            args.push(match token.trim() { "true" => Arg::Bool(true), "false" => Arg::Bool(false), token => Arg::Number(token.parse().ok().filter(|n: &f64| n.is_finite())?) });
        }
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        match chars.next() { None => break, Some(',') => {}, Some(_) => return None }
    }
    Some((name, args))
}

#[derive(Clone, Copy)]
enum DatePart { Day(usize), Month(usize), Year(usize), Literal(char) }

fn date_parts(pattern: &str) -> Option<Vec<DatePart>> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut parts = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        let run = chars[index..].iter().take_while(|next| **next == c).count();
        parts.push(match (c, run) {
            ('d', 1 | 2) => DatePart::Day(run),
            ('m', 1..=4) => DatePart::Month(run),
            ('y', 2 | 4) => DatePart::Year(run),
            (c, _) if c.is_alphanumeric() => return None,
            (c, _) => { index += 1; parts.push(DatePart::Literal(c)); continue; }
        });
        index += run;
    }
    parts.iter().any(|part| !matches!(part, DatePart::Literal(_))).then_some(parts)
}

const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];

fn is_real_date(pattern: &str, value: &str) -> bool {
    let Some(parts) = date_parts(pattern) else { return false };
    let value: Vec<char> = value.trim().chars().collect();
    let (mut at, mut day, mut month, mut year) = (0, 1, 1, 2000);
    let digits = |at: &mut usize, min: usize, max: usize| -> Option<u32> {
        let count = value[*at..].iter().take(max).take_while(|c| c.is_ascii_digit()).count();
        if count < min { return None; }
        let number = value[*at..*at + count].iter().collect::<String>().parse().ok();
        *at += count;
        number
    };
    for part in parts {
        let parsed = match part {
            DatePart::Day(width) => digits(&mut at, width, 2).map(|n| day = n),
            DatePart::Month(width @ (1 | 2)) => digits(&mut at, width, 2).map(|n| month = n),
            DatePart::Month(width) => {
                let word: String = value[at..].iter().take_while(|c| c.is_alphabetic()).collect::<String>().to_lowercase();
                at += word.chars().count();
                MONTHS.iter().position(|name| if width == 3 { name[..3] == word } else { *name == word }).map(|index| month = index as u32 + 1)
            }
            DatePart::Year(width) => digits(&mut at, width, width).map(|n| year = if width == 4 { n } else if n < 50 { 2000 + n } else { 1900 + n }),
            DatePart::Literal(c) => (value.get(at) == Some(&c)).then(|| at += 1),
        };
        if parsed.is_none() { return false; }
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    at == value.len() && (1..=12).contains(&month) && day >= 1 && day <= days[month as usize - 1]
}

fn script_format(js: &str) -> Option<Format> {
    let (name, args) = parse_call(js)?;
    let number = |index: usize, max: f64| match args.get(index) {
        None => Some(0.),
        Some(Arg::Number(n)) if n.fract() == 0. && (0. ..=max).contains(n) => Some(*n),
        _ => None,
    };
    match name {
        "AFDate_FormatEx" | "AFDate_KeystrokeEx" => match args.as_slice() {
            [Arg::Text(pattern)] => date_parts(pattern).map(|_| Format::Date { pattern: pattern.clone() }),
            _ => None,
        },
        "AFNumber_Format" | "AFNumber_Keystroke" if args.len() <= 6 && matches!(args.get(3), None | Some(Arg::Number(_))) => Some(Format::Number {
            decimals: number(0, 10.)? as usize, separator: number(1, 4.)? as u8, negative: number(2, 3.)? as u8,
            currency: match args.get(4) { None => String::new(), Some(Arg::Text(text)) => text.clone(), _ => return None },
            currency_prepend: match args.get(5) { None => false, Some(Arg::Bool(prepend)) => *prepend, Some(Arg::Number(n)) => *n != 0., _ => return None },
        }),
        "AFPercent_Format" | "AFPercent_Keystroke" if args.len() <= 2 => Some(Format::Percent { decimals: number(0, 10.)? as usize, separator: number(1, 4.)? as u8 }),
        "AFSpecial_Format" | "AFSpecial_Keystroke" if args.len() == 1 => Some(Format::Special { special: number(0, 3.)? as u8 }),
        _ => None,
    }
}

fn read_format(doc: &Document, dict: &Dictionary, inherited: &mut Inherited) {
    let Some(actions) = dict.get(b"AA").ok().and_then(|value| resolve(doc, value).ok()).and_then(|value| value.as_dict().ok()) else { return };
    let mut found: Option<Format> = None;
    for key in [&b"F"[..], b"K"] {
        let Ok(action) = actions.get(key) else { continue };
        let action = resolve(doc, action).ok().and_then(|action| action.as_dict().ok());
        let js = action.filter(|action| action.get(b"S").ok().and_then(|kind| kind.as_name().ok()) == Some(b"JavaScript"))
            .and_then(|action| action.get(b"JS").ok()).and_then(|js| resolve(doc, js).ok())
            .and_then(|js| match js {
                Object::Stream(stream) => stream.decompressed_content().ok().or_else(|| Some(stream.content.clone())).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
                js => lopdf::decode_text_string(js).ok(),
            });
        match js.as_deref().and_then(script_format) {
            Some(format) if found.as_ref().is_none_or(|found| std::mem::discriminant(found) == std::mem::discriminant(&format)) => { found.get_or_insert(format); }
            _ => inherited.scripted = true,
        }
    }
    if found.is_some() { inherited.format = found; }
}

fn decimal_entry(text: &str, separator: u8) -> Option<(bool, String)> {
    let mark = if matches!(separator, 2 | 3) { ',' } else { '.' };
    let text = text.trim();
    let (negative, text) = text.strip_prefix('-').map_or((false, text), |rest| (true, rest));
    let (whole, fraction) = text.split_once(mark).unwrap_or((text, ""));
    let digits = |part: &str| part.chars().all(|c| c.is_ascii_digit());
    if whole.is_empty() && fraction.is_empty() || !digits(whole) || !digits(fraction) { return None; }
    let canonical = format!("{}{}{}", if whole.is_empty() { "0" } else { whole }, if fraction.is_empty() { "" } else { "." }, fraction);
    Some((negative && canonical.chars().any(|c| c.is_ascii_digit() && c != '0'), canonical))
}

fn group_number(value: f64, decimals: usize, separator: u8) -> String {
    let fixed = format!("{:.*}", decimals, value.abs());
    let (whole, fraction) = fixed.split_once('.').unwrap_or((&fixed, ""));
    let group = match separator { 0 => Some(','), 2 => Some('.'), 4 => Some('\''), _ => None };
    let mut grouped = String::new();
    for (index, c) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 { if let Some(group) = group { grouped.push(group); } }
        grouped.push(c);
    }
    if !fraction.is_empty() { grouped.push(if matches!(separator, 2 | 3) { ',' } else { '.' }); grouped.push_str(fraction); }
    grouped
}

impl Format {
    /// Returns the stored value, the shown text, and whether it is drawn in red.
    fn entry(&self, text: &str) -> Option<(String, String, bool)> {
        match self {
            Format::Date { pattern } => is_real_date(pattern, text).then(|| (text.trim().to_string(), text.trim().to_string(), false)),
            Format::Number { decimals, separator, negative, currency, currency_prepend } => {
                let (minus, digits) = decimal_entry(text, *separator)?;
                let amount = group_number(digits.parse().ok()?, *decimals, *separator);
                let amount = if *currency_prepend { format!("{currency}{amount}") } else { format!("{amount}{currency}") };
                let shown = match (minus, negative) { (false, _) | (true, 1) => amount, (true, 0) => format!("-{amount}"), _ => format!("({amount})") };
                Some((format!("{}{digits}", if minus { "-" } else { "" }), shown, minus && matches!(negative, 1 | 3)))
            }
            Format::Percent { decimals, separator } => {
                let (minus, digits) = decimal_entry(text, *separator)?;
                let value: f64 = digits.parse().ok()?;
                Some((format!("{}{digits}", if minus { "-" } else { "" }), format!("{}{}%", if minus { "-" } else { "" }, group_number(value * 100., *decimals, *separator)), false))
            }
            Format::Special { special } => {
                if !text.chars().all(|c| c.is_ascii_digit() || " -.()".contains(c)) { return None; }
                let d: String = text.chars().filter(char::is_ascii_digit).collect();
                let shown = match (special, d.len()) {
                    (0, 5) => d.clone(),
                    (1, 9) => format!("{}-{}", &d[..5], &d[5..]),
                    (2, 10) => format!("({}) {}-{}", &d[..3], &d[3..6], &d[6..]),
                    (2, 7) => format!("{}-{}", &d[..3], &d[3..]),
                    (3, 9) => format!("{}-{}-{}", &d[..3], &d[3..5], &d[5..]),
                    _ => return None,
                };
                Some((d, shown, false))
            }
        }
    }

    fn refusal(&self, name: &str) -> String {
        match self {
            Format::Date { pattern } => format!("“{name}” is not a real date. Enter it as {pattern}."),
            Format::Number { .. } | Format::Percent { .. } => format!("“{name}” must be a number."),
            Format::Special { special: 0 } => format!("“{name}” must be a 5-digit ZIP code."),
            Format::Special { special: 1 } => format!("“{name}” must be a ZIP+4 code, like 12345-6789."),
            Format::Special { special: 2 } => format!("“{name}” must be a phone number, like (555) 123-4567."),
            Format::Special { .. } => format!("“{name}” must be a Social Security number, like 123-45-6789."),
        }
    }
}

fn is_widget(dict: &Dictionary) -> bool {
    dict.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) == Some(b"Widget")
}

fn walk(doc: &Document, id: ObjectId, parent: &Inherited, seen: &mut HashSet<ObjectId>, out: &mut Vec<Terminal>) -> Result<(), String> {
    let malformed = || "This PDF's form fields are malformed, so they can't be filled.".to_string();
    if seen.len() > 100_000 || !seen.insert(id) { return Err(malformed()); }
    let dict = doc.get_dictionary(id).map_err(|_| malformed())?;
    let mut inherited = parent.clone();
    if let Some(part) = text(doc, dict.get(b"T").ok()) {
        inherited.name = if inherited.name.is_empty() { part } else { format!("{}.{part}", inherited.name) };
    }
    if let Ok(kind) = dict.get(b"FT").and_then(Object::as_name) { inherited.kind = Some(kind.to_vec()); }
    if let Some(flags) = integer(doc, dict, b"Ff") { inherited.flags = flags; }
    if let Some(appearance) = text(doc, dict.get(b"DA").ok()) { inherited.appearance = Some(appearance); }
    if let Some(quadding) = integer(doc, dict, b"Q") { inherited.quadding = Some(quadding); }
    if let Ok(value) = dict.get(b"V") { inherited.value = Some(resolve(doc, value)?.clone()); }
    if let Some(max_len) = integer(doc, dict, b"MaxLen") { inherited.max_len = Some(max_len); }
    if let Ok(options) = dict.get(b"Opt") { inherited.options = Some(resolve(doc, options)?.clone()); }
    read_format(doc, dict, &mut inherited);
    let kids = match dict.get(b"Kids") {
        Ok(kids) => resolve(doc, kids)?.as_array().map_err(|_| malformed())?.iter().map(|kid| kid.as_reference().map_err(|_| malformed())).collect::<Result<Vec<_>, _>>()?,
        Err(_) => Vec::new(),
    };
    let mut widgets = Vec::new();
    for kid in kids {
        let child = doc.get_dictionary(kid).map_err(|_| malformed())?;
        if is_widget(child) && !child.has(b"T") {
            read_format(doc, child, &mut inherited);
            widgets.push(kid);
        } else {
            walk(doc, kid, &inherited, seen, out)?;
        }
    }
    if is_widget(dict) { widgets.insert(0, id); }
    if !widgets.is_empty() {
        out.push(Terminal { id, label: text(doc, dict.get(b"TU").ok()), inherited, widgets });
    }
    Ok(())
}

fn on_state(doc: &Document, widget: &Dictionary) -> Option<Vec<u8>> {
    let normal = widget.get(b"AP").ok().and_then(|value| resolve(doc, value).ok())?.as_dict().ok()?.get(b"N").ok().and_then(|value| resolve(doc, value).ok())?.as_dict().ok()?;
    normal.iter().map(|(key, _)| key.clone()).find(|key| key.as_slice() != b"Off")
}

fn read(doc: &Document) -> Result<FormInspection, String> {
    let catalog = doc.catalog().map_err(err)?;
    let Some(acro_form) = catalog.get(b"AcroForm").ok() else { return Ok(FormInspection::default()) };
    let acro_form = resolve(doc, acro_form)?.as_dict().map_err(err)?;
    if acro_form.has(b"XFA") || catalog.has(b"NeedsRendering") {
        return Ok(FormInspection { unsupported: Some(XFA_UNSUPPORTED.into()), ..FormInspection::default() });
    }
    let root = Inherited {
        name: String::new(), kind: None, flags: 0, appearance: text(doc, acro_form.get(b"DA").ok()),
        quadding: integer(doc, acro_form, b"Q"), value: None, max_len: None, options: None, format: None, scripted: false,
    };
    let mut terminals = Vec::new();
    let mut seen = HashSet::new();
    if let Ok(fields) = acro_form.get(b"Fields") {
        for field in resolve(doc, fields)?.as_array().map_err(err)? {
            walk(doc, field.as_reference().map_err(err)?, &root, &mut seen, &mut terminals)?;
        }
    }
    let mut placement: HashMap<ObjectId, (usize, usize, FormWidget)> = HashMap::new();
    for (page_index, page_id) in doc.get_pages().values().enumerate() {
        let page = doc.get_dictionary(*page_id).map_err(err)?;
        let Some(annotations) = page.get(b"Annots").ok() else { continue };
        let g = geometry(doc, *page_id)?;
        let mut placed = Vec::new();
        for annotation in resolve(doc, annotations)?.as_array().map_err(err)? {
            let Ok(id) = annotation.as_reference() else { continue };
            let Ok(widget) = doc.get_dictionary(id) else { continue };
            if !is_widget(widget) { continue; }
            let Ok(rect) = widget.get(b"Rect").and_then(Object::as_array) else { continue };
            let numbers = rect.iter().map(|value| resolve(doc, value).and_then(super::super::metadata::number)).collect::<Result<Vec<_>, _>>()?;
            if numbers.len() != 4 { continue; }
            let m = g.matrix;
            let corners = [(numbers[0], numbers[1]), (numbers[2], numbers[3]), (numbers[0], numbers[3]), (numbers[2], numbers[1])]
                .map(|(x, y)| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]));
            let (x0, x1) = corners.iter().fold((f32::MAX, f32::MIN), |(a, b), (x, _)| (a.min(*x), b.max(*x)));
            let (y0, y1) = corners.iter().fold((f32::MAX, f32::MIN), |(a, b), (_, y)| (a.min(*y), b.max(*y)));
            placed.push((id, FormWidget { page: page_index, x: x0, y: y0, width: x1 - x0, height: y1 - y0, option: None }));
        }
        match page.get(b"Tabs").ok().and_then(|value| value.as_name().ok()) {
            Some(b"R") => placed.sort_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.1.x.total_cmp(&b.1.x))),
            Some(b"C") => placed.sort_by(|a, b| a.1.x.total_cmp(&b.1.x).then(a.1.y.total_cmp(&b.1.y))),
            _ => {}
        }
        for (order, (id, widget)) in placed.into_iter().enumerate() { placement.entry(id).or_insert((page_index, order, widget)); }
    }
    let mut fields = Vec::new();
    let mut scripted = false;
    for terminal in terminals {
        let Inherited { name, kind, flags, appearance, quadding, value, max_len, options, format, scripted: has_script } = terminal.inherited;
        let kind = match kind.as_deref() {
            Some(b"Tx") if flags & (PASSWORD | FILE_SELECT) == 0 => FieldKind::Text,
            Some(b"Btn") if flags & PUSHBUTTON != 0 => continue,
            Some(b"Btn") if flags & RADIO != 0 => FieldKind::Radio,
            Some(b"Btn") => FieldKind::Checkbox,
            Some(b"Ch") if flags & COMBO != 0 => FieldKind::Dropdown,
            Some(b"Ch") => FieldKind::Listbox,
            _ => continue,
        };
        let mut widgets: Vec<_> = terminal.widgets.iter().filter_map(|id| placement.get(id).map(|placed| (*id, placed.clone()))).collect();
        if widgets.is_empty() || name.is_empty() { continue; }
        scripted |= has_script || (format.is_some() && kind != FieldKind::Text);
        widgets.sort_by_key(|(_, (page, order, _))| (*page, *order));
        let states: Vec<Option<Vec<u8>>> = widgets.iter().map(|(id, _)| doc.get_dictionary(*id).ok().and_then(|widget| on_state(doc, widget))).collect();
        let on = states.iter().flatten().next().cloned().unwrap_or_else(|| b"Yes".to_vec());
        let lossy = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let selected_state = || value.as_ref().and_then(|value| value.as_name().ok().map(<[u8]>::to_vec))
            .or_else(|| widgets.iter().find_map(|(id, _)| doc.get_dictionary(*id).ok()?.get(b"AS").ok()?.as_name().ok().filter(|state| *state != b"Off").map(<[u8]>::to_vec)))
            .filter(|state| state != b"Off");
        let (options, option_labels) = match kind {
            FieldKind::Text => (Vec::new(), Vec::new()),
            FieldKind::Checkbox => (vec![lossy(&on)], vec![lossy(&on)]),
            FieldKind::Radio => {
                let mut unique: Vec<String> = Vec::new();
                for state in states.iter().flatten().map(|state| lossy(state)) { if !unique.contains(&state) { unique.push(state); } }
                (unique.clone(), unique)
            }
            FieldKind::Dropdown | FieldKind::Listbox => options.as_ref().and_then(|options| options.as_array().ok()).into_iter().flatten().filter_map(|option| {
                let option = resolve(doc, option).ok()?;
                let (export, label) = match option.as_array() {
                    Ok(pair) if pair.len() == 2 => (text(doc, pair.first())?, text(doc, pair.get(1))?),
                    _ => { let value = lopdf::decode_text_string(option).ok()?; (value.clone(), value) }
                };
                Some((export, label.replace("\r\n", " ").replace(['\r', '\n'], " ")))
            }).unzip(),
        };
        let stored = || value.as_ref().and_then(|value| lopdf::decode_text_string(value).ok());
        let value = match kind {
            FieldKind::Text | FieldKind::Dropdown => serde_json::Value::String(stored().unwrap_or_default()),
            FieldKind::Checkbox => serde_json::Value::Bool(selected_state().is_some()),
            FieldKind::Radio => serde_json::Value::String(selected_state().map(|state| lossy(&state)).unwrap_or_default()),
            FieldKind::Listbox => serde_json::Value::Array(match value.as_ref().map(|value| value.as_array()) {
                Some(Ok(items)) => items.iter().filter_map(|item| text(doc, Some(item))).map(serde_json::Value::String).collect(),
                _ => stored().map(serde_json::Value::String).into_iter().collect(),
            }),
        };
        fields.push((widgets[0].1.0, widgets[0].1.1, FormField {
            name, label: terminal.label, kind, value, options, option_labels, flags,
            required: flags & REQUIRED != 0, read_only: flags & READ_ONLY != 0, multiline: kind == FieldKind::Text && flags & MULTILINE != 0,
            multi_select: kind == FieldKind::Listbox && flags & MULTI_SELECT != 0, editable: kind == FieldKind::Dropdown && flags & EDIT != 0,
            max_len: if kind == FieldKind::Text { max_len } else { None },
            format: if kind == FieldKind::Text { format } else { None },
            widgets: widgets.iter().zip(&states).map(|((_, (_, _, widget)), state)| FormWidget {
                option: if kind == FieldKind::Radio { state.as_deref().map(lossy) } else { None }, ..widget.clone()
            }).collect(),
            id: terminal.id, widget_ids: widgets.iter().map(|(id, _)| *id).collect(), widget_states: states, on_state: on,
            appearance: appearance.unwrap_or_default(),
            align: match quadding { Some(1) => TextAlign::Center, Some(2) => TextAlign::Right, _ => TextAlign::Left },
        }));
    }
    if scripted { return Ok(FormInspection { unsupported: Some(SCRIPTED_UNSUPPORTED.into()), ..FormInspection::default() }); }
    fields.sort_by_key(|(page, order, _)| (*page, *order));
    Ok(FormInspection { fields: fields.into_iter().map(|(_, _, field)| field).collect(), ..FormInspection::default() })
}

enum Fill { Text { value: Object, red: bool }, Button(Vec<u8>), List(Vec<usize>) }

pub(super) struct FormPlan { fills: Vec<(FormField, Fill)>, objects: HashMap<(usize, usize), SceneObject>, layouts: HashMap<(usize, usize), TextLayout> }
impl FormPlan {
    /// A plan without fills leaves the document as it was, so it does not name the export.
    pub(super) fn changes_document(&self) -> bool { !self.fills.is_empty() }
}

struct Appearance { size: f32, color: String }

fn parse_appearance(appearance: &str) -> Appearance {
    let tokens: Vec<&str> = appearance.split_whitespace().collect();
    let numbers = |end: usize, count: usize| -> Option<Vec<f32>> {
        (end >= count).then(|| tokens[end - count..end].iter().map(|token| token.parse::<f32>().ok().filter(|n| n.is_finite())).collect())?
    };
    let mut parsed = Appearance { size: 0., color: "0 g".into() };
    for (index, token) in tokens.iter().enumerate() {
        match *token {
            "Tf" => if let Some(size) = numbers(index, 1) { parsed.size = size[0]; },
            "g" | "rg" | "k" => {
                let count = match *token { "g" => 1, "rg" => 3, _ => 4 };
                if let Some(values) = numbers(index, count) {
                    parsed.color = format!("{} {token}", values.iter().map(f32::to_string).collect::<Vec<_>>().join(" "));
                }
            }
            _ => {}
        }
    }
    parsed
}

fn widget_size(doc: &Document, id: ObjectId) -> Result<(f32, f32), String> {
    let rect = doc.get_dictionary(id).map_err(err)?.get(b"Rect").map_err(err)?;
    let values = resolve(doc, rect)?.as_array().map_err(err)?.iter().map(|value| resolve(doc, value).and_then(super::super::metadata::number)).collect::<Result<Vec<_>, _>>()?;
    if values.len() != 4 { return Err("A form field has an invalid rectangle".into()); }
    Ok(((values[2] - values[0]).abs(), (values[3] - values[1]).abs()))
}

fn text_object(field: &FormField, text: &str, size: (f32, f32), multiline: bool) -> SceneObject {
    let parsed = parse_appearance(&field.appearance);
    let (width, height) = size;
    let fit = ((height - 2. * PADDING) / TEXT_LINE_HEIGHT).clamp(4., 12.);
    let font_size = if parsed.size > 0. { parsed.size.min(1000.) } else { fit };
    let rect = if multiline {
        Rect { x: PADDING, y: PADDING, width: (width - 2. * PADDING).max(1.), height: (height - 2. * PADDING).max(1.) }
    } else {
        let line = font_size * TEXT_LINE_HEIGHT;
        Rect { x: PADDING, y: (height - line) / 2., width: (width - 2. * PADDING).max(1.), height: line }
    };
    SceneObject {
        kind: Kind::Text, rect, text: text.into(), font_size, color: "#000000".into(), opacity: 1.,
        shape: None, highlight_mode: None, signature_mode: None, signature_path: None, font_family: None, watermark: None, layer: Layer::Front,
        bold: false, italic: false, align: field.align, page_range: None, page_numbers: None, header_footer: None,
    }
}

fn plan_text(doc: &Document, plan: &mut FormPlan, field: &FormField, text: &str, multiline: bool) -> Result<(), ToolError> {
    for (index, id) in field.widget_ids.iter().enumerate() {
        let size = widget_size(doc, *id).map_err(ToolError::processing)?;
        let object = text_object(field, text, size, multiline);
        let mut layout_object = object.clone();
        if !field.multiline { layout_object.rect.width = f32::MAX / 4.; }
        let layout = layout_text_for(&layout_object, &format!("the “{}” field", field.name))?;
        plan.objects.insert((plan.fills.len(), index), object);
        plan.layouts.insert((plan.fills.len(), index), layout);
    }
    Ok(())
}

pub(super) fn preflight(doc: &Document, values: &[serde_json::Value]) -> Result<FormPlan, ToolError> {
    let inspection = read(doc).map_err(ToolError::processing)?;
    if let Some(message) = inspection.unsupported { return Err(ToolError::invalid_input(message)); }
    if inspection.fields.is_empty() { return Err(ToolError::invalid_input("This PDF has no form fields to fill. Use Add text to type onto the page instead.")); }
    plan(doc, inspection.fields, values)
}

fn plan(doc: &Document, fields: Vec<FormField>, values: &[serde_json::Value]) -> Result<FormPlan, ToolError> {
    let mut fields: HashMap<String, FormField> = fields.into_iter().map(|field| (field.name.clone(), field)).collect();
    let mut plan = FormPlan { fills: Vec::new(), objects: HashMap::new(), layouts: HashMap::new() };
    for entry in values {
        let (Some(name), Some(value)) = (entry.get("name").and_then(|name| name.as_str()), entry.get("value")) else {
            return Err(ToolError::invalid_input("Each form value must name a field and give its value."));
        };
        let Some(field) = fields.remove(name) else {
            return Err(ToolError::invalid_input(if plan.fills.iter().any(|(field, _)| field.name == name) {
                format!("“{name}” was given more than one value.")
            } else {
                format!("This PDF has no fillable field named “{name}”.")
            }));
        };
        if field.read_only { return Err(ToolError::invalid_input(format!("“{name}” is read-only, so it can't be filled."))); }
        let invalid = |message: String| Err(ToolError::invalid_input(message));
        let no_option = |choice: &str| ToolError::invalid_input(format!("“{name}” has no option “{choice}”."));
        let single_line = |text: &str| text.replace("\r\n", " ").replace(['\r', '\n'], " ");
        let fill = match (field.kind, value) {
            (FieldKind::Text, serde_json::Value::String(text)) => {
                let text = if field.multiline { text.clone() } else { single_line(text) };
                if let Some(max) = field.max_len.filter(|max| text.chars().count() as i64 > (*max).max(0)) {
                    return invalid(format!("“{name}” can be at most {max} characters."));
                }
                let (stored, shown, red) = match &field.format {
                    Some(format) if !text.trim().is_empty() => format.entry(&text).ok_or_else(|| ToolError::invalid_input(format.refusal(name)))?,
                    _ => (text.clone(), text, false),
                };
                plan_text(doc, &mut plan, &field, &shown, field.multiline)?;
                Fill::Text { value: lopdf::text_string(&stored), red }
            }
            (FieldKind::Dropdown, serde_json::Value::String(choice)) => {
                let shown = match field.options.iter().position(|option| option == choice) {
                    Some(index) => field.option_labels[index].clone(),
                    None if choice.is_empty() || field.editable => single_line(choice),
                    None => return Err(no_option(choice)),
                };
                plan_text(doc, &mut plan, &field, &shown, false)?;
                Fill::Text { value: lopdf::text_string(choice), red: false }
            }
            (FieldKind::Listbox, serde_json::Value::Array(choices)) => {
                let mut indices = Vec::new();
                for choice in choices {
                    let Some(choice) = choice.as_str() else { return invalid(format!("“{name}” is a list box, so its value must be a list of its options.")) };
                    let index = field.options.iter().position(|option| option == choice).ok_or_else(|| no_option(choice))?;
                    if indices.contains(&index) { return invalid(format!("“{name}” lists “{choice}” more than once.")); }
                    indices.push(index);
                }
                if indices.len() > 1 && !field.multi_select { return invalid(format!("“{name}” allows only one choice.")); }
                indices.sort_unstable();
                plan_text(doc, &mut plan, &field, &field.option_labels.join("\n"), true)?;
                Fill::List(indices)
            }
            (FieldKind::Checkbox, serde_json::Value::Bool(checked)) => Fill::Button(if *checked { field.on_state.clone() } else { b"Off".to_vec() }),
            (FieldKind::Radio, serde_json::Value::String(choice)) if choice.is_empty() => Fill::Button(b"Off".to_vec()),
            (FieldKind::Radio, serde_json::Value::String(choice)) => Fill::Button(
                field.widget_states.iter().flatten().find(|state| String::from_utf8_lossy(state) == *choice).cloned().ok_or_else(|| no_option(choice))?,
            ),
            (FieldKind::Text, _) => return invalid(format!("“{name}” is a text field, so its value must be text.")),
            (FieldKind::Checkbox, _) => return invalid(format!("“{name}” is a check box, so its value must be checked or unchecked.")),
            (FieldKind::Radio | FieldKind::Dropdown, _) => return invalid(format!("“{name}” is a choice field, so its value must be one of its options.")),
            (FieldKind::Listbox, _) => return invalid(format!("“{name}” is a list box, so its value must be a list of its options.")),
        };
        plan.fills.push((field, fill));
    }
    Ok(plan)
}

fn form_xobject(doc: &mut Document, width: f32, height: f32, resources: Dictionary, content: &str) -> Result<ObjectId, String> {
    let dict = dictionary! {
        "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), width.into(), height.into()],
        "Resources" => resources, "Filter" => "FlateDecode",
    };
    Ok(doc.add_object(Stream::new(dict, compress_bytes(content.as_bytes())?)))
}

fn text_appearance(doc: &mut Document, plan: &FormPlan, fonts: &HashMap<&'static str, (ObjectId, subsetter::GlyphRemapper)>, key: (usize, usize), id: ObjectId, color: &str, under: &str) -> Result<(), String> {
    let (width, height) = widget_size(doc, id)?;
    let layout = &plan.layouts[&key];
    let (font_id, remapper) = &fonts[layout.font.name];
    let o = &plan.objects[&key];
    let content = format!("/Tx BMC\nq\n1 0 0 -1 0 {height} cm\n{under}{color}\n{}Q\nEMC\n", text_content(&o.rect, o.align, o.font_size, layout, FONT_RESOURCE, remapper));
    let resources = dictionary! { "Font" => dictionary! { FONT_RESOURCE => *font_id } };
    let appearance = form_xobject(doc, width, height, resources, &content)?;
    doc.get_dictionary_mut(id).map_err(err)?.set("AP", dictionary! { "N" => appearance });
    Ok(())
}

fn has_appearance(doc: &Document, widget: ObjectId) -> bool {
    doc.get_dictionary(widget).ok().and_then(|widget| widget.get(b"AP").ok()).and_then(|ap| resolve(doc, ap).ok())
        .and_then(|ap| ap.as_dict().ok()).is_some_and(|ap| ap.has(b"N"))
}

/// Draws fields that hold a value but have no appearance, so flattening keeps what a viewer shows.
/// A stored value the field can't draw is left as it is.
pub(super) fn complete_appearances(doc: &mut Document) -> Result<(), String> {
    let Ok(FormInspection { fields, unsupported: None, .. }) = read(doc) else { return Ok(()) };
    for mut field in fields {
        let blank = matches!(&field.value, serde_json::Value::String(text) if text.is_empty()) || field.value == serde_json::Value::Bool(false)
            || matches!(&field.value, serde_json::Value::Array(items) if items.is_empty());
        if blank || field.widget_ids.iter().all(|id| has_appearance(doc, *id)) { continue; }
        let value = serde_json::json!({ "name": field.name, "value": field.value });
        field.read_only = false;
        if let Ok(plan) = plan(doc, vec![field], &[value]) { apply(doc, plan)?; }
    }
    Ok(())
}

pub(super) fn apply(doc: &mut Document, plan: FormPlan) -> Result<(), String> {
    let fonts = embed_text_fonts(doc, &plan.layouts)?;
    for (fill_index, (field, fill)) in plan.fills.iter().enumerate() {
        let color = parse_appearance(&field.appearance).color;
        match fill {
            Fill::Text { value, red } => {
                let dict = doc.get_dictionary_mut(field.id).map_err(err)?;
                dict.set("V", value.clone());
                dict.remove(b"RV");
                dict.remove(b"I");
                let color = if *red { "1 0 0 rg".to_string() } else { color };
                for (index, id) in field.widget_ids.iter().enumerate() { text_appearance(doc, &plan, &fonts, (fill_index, index), *id, &color, "")?; }
            }
            Fill::List(indices) => {
                let dict = doc.get_dictionary_mut(field.id).map_err(err)?;
                let chosen: Vec<Object> = indices.iter().map(|index| lopdf::text_string(&field.options[*index])).collect();
                match chosen.len() {
                    0 => { dict.remove(b"V"); dict.remove(b"I"); }
                    1 => { dict.set("V", chosen[0].clone()); dict.set("I", vec![Object::Integer(indices[0] as i64)]); }
                    _ => { dict.set("V", chosen); dict.set("I", indices.iter().map(|index| Object::Integer(*index as i64)).collect::<Vec<_>>()); }
                }
                for (index, id) in field.widget_ids.iter().enumerate() {
                    let object = &plan.objects[&(fill_index, index)];
                    let line = object.font_size * TEXT_LINE_HEIGHT;
                    let (width, _) = widget_size(doc, *id)?;
                    let under: String = indices.iter().map(|row| format!("0.6 0.75 0.86 rg 1 {} {} {line} re f\n", object.rect.y + *row as f32 * line, (width - 2.).max(0.))).collect();
                    text_appearance(doc, &plan, &fonts, (fill_index, index), *id, &color, &under)?;
                }
            }
            Fill::Button(state) => {
                let selected = state.as_slice() != b"Off";
                doc.get_dictionary_mut(field.id).map_err(err)?.set("V", Object::Name(state.clone()));
                for id in &field.widget_ids {
                    let (width, height) = widget_size(doc, *id)?;
                    let widget = doc.get_dictionary(*id).map_err(err)?;
                    let mut appearances = widget.get(b"AP").ok().and_then(|value| resolve(doc, value).ok()).and_then(|value| value.as_dict().ok()).cloned().unwrap_or_default();
                    let mut normal = appearances.get(b"N").ok().and_then(|value| resolve(doc, value).ok()).and_then(|value| value.as_dict().ok()).cloned().unwrap_or_default();
                    let own_on = normal.iter().map(|(key, _)| key.clone()).find(|key| key.as_slice() != b"Off");
                    let on = own_on.clone().or_else(|| (field.kind == FieldKind::Checkbox).then(|| field.on_state.clone()));
                    if let (None, Some(on)) = (&own_on, &on) {
                        let check = format!("q 0 G {} w 1 J 1 j {} {} m {} {} l {} {} l S Q\n", (width.min(height) / 10.).max(0.5),
                            width * 0.22, height * 0.52, width * 0.42, height * 0.28, width * 0.8, height * 0.76);
                        normal.set(on.clone(), form_xobject(doc, width, height, Dictionary::new(), &check)?);
                    }
                    if !normal.has(b"Off") { normal.set("Off", form_xobject(doc, width, height, Dictionary::new(), "")?); }
                    let shown = on.filter(|on| selected && (field.kind == FieldKind::Checkbox || on == state)).unwrap_or_else(|| b"Off".to_vec());
                    appearances.set("N", normal);
                    let widget = doc.get_dictionary_mut(*id).map_err(err)?;
                    widget.set("AP", appearances);
                    widget.set("AS", Object::Name(shown));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, process::Command};
    use lopdf::{dictionary, Document, Object, Stream};
    use crate::kit::{common::OutputLocation, contracts::ErrorKind};
    use super::super::tests::export;
    use super::super::{text_extractor, ExportRequest, PdfScene, ScenePage, TempDir};
    use super::*;

    fn rect(values: [f32; 4]) -> Object { Object::Array(values.iter().map(|value| Object::Real(*value)).collect()) }

    fn form_fixture(path: &Path, extra: impl FnOnce(&mut Document, ObjectId, ObjectId)) {
        let mut d = Document::with_version("1.7");
        let pages = d.new_object_id();
        let page = d.new_object_id();
        let helv = d.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let name = d.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "T" => Object::string_literal("name"),
            "TU" => Object::string_literal("Full name"), "Ff" => REQUIRED, "DA" => Object::string_literal("/Helv 12 Tf 0 0 1 rg"), "Rect" => rect([72., 700., 300., 724.]), "P" => page });
        let address = d.new_object_id();
        let city = d.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Widget", "T" => Object::string_literal("city"), "Parent" => address,
            "Rect" => rect([72., 600., 300., 624.]), "Q" => 1, "P" => page });
        d.objects.insert(address, Object::Dictionary(dictionary! { "FT" => "Tx", "T" => Object::string_literal("address"), "Kids" => vec![Object::Reference(city)] }));
        let on = d.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0., 0., 18., 18.]) }, b"0 g 4 4 10 10 re f".to_vec()));
        let off = d.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0., 0., 18., 18.]) }, Vec::new()));
        let agree = d.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Widget", "FT" => "Btn", "T" => Object::string_literal("agree"),
            "V" => "Off", "AS" => "Off", "Rect" => rect([72., 650., 90., 668.]), "AP" => dictionary! { "N" => dictionary! { "Agreed" => on, "Off" => off } }, "P" => page });
        let subscribe = d.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Widget", "FT" => "Btn", "T" => Object::string_literal("subscribe"),
            "Rect" => rect([72., 560., 90., 578.]), "P" => page });
        let content = d.add_object(Stream::new(dictionary! {}, b"BT /Helv 10 Tf 72 730 Td (Application) Tj ET".to_vec()));
        d.objects.insert(page, Object::Dictionary(dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => rect([0., 0., 612., 792.]),
            "Resources" => dictionary! { "Font" => dictionary! { "Helv" => helv } }, "Contents" => content,
            "Annots" => vec![name, city, agree, subscribe].into_iter().map(Object::Reference).collect::<Vec<_>>() }));
        d.objects.insert(pages, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1 }));
        let acro_form = d.add_object(dictionary! { "Fields" => vec![name, address, agree, subscribe].into_iter().map(Object::Reference).collect::<Vec<_>>(),
            "DA" => Object::string_literal("/Helv 0 Tf 0 g"), "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } } });
        let catalog = d.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages, "AcroForm" => acro_form });
        d.trailer.set("Root", catalog);
        extra(&mut d, acro_form, city);
        d.save(path).unwrap();
    }

    fn button_states(d: &mut Document, state: &str) -> Object {
        let on = d.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0., 0., 14., 14.]) }, b"0 g 4 4 6 6 re f".to_vec()));
        let off = d.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => rect([0., 0., 14., 14.]) }, Vec::new()));
        Object::Dictionary(dictionary! { "N" => dictionary! { state => on, "Off" => off } })
    }

    fn every_type(d: &mut Document, acro_form: ObjectId, city: ObjectId) {
        let page = d.get_dictionary(city).unwrap().get(b"P").unwrap().as_reference().unwrap();
        let widget = |d: &mut Document, extra: Dictionary, area: [f32; 4]| {
            let mut dict = dictionary! { "Type" => "Annot", "Subtype" => "Widget", "Rect" => rect(area), "P" => page, "DA" => Object::string_literal("/Helv 0 Tf 0 g") };
            dict.extend(&extra);
            d.add_object(dict)
        };
        let script = |js: &str| dictionary! { "S" => "JavaScript", "JS" => Object::string_literal(js) };
        let pets = d.new_object_id();
        let (yes_ap, no_ap) = (button_states(d, "Yes"), button_states(d, "No"));
        let yes = widget(d, dictionary! { "Parent" => pets, "AS" => "Off", "AP" => yes_ap }, [72., 520., 86., 534.]);
        let no = widget(d, dictionary! { "Parent" => pets, "AS" => "Off", "AP" => no_ap }, [120., 520., 134., 534.]);
        d.objects.insert(pets, Object::Dictionary(dictionary! { "FT" => "Btn", "Ff" => RADIO | REQUIRED, "T" => Object::string_literal("pets"),
            "TU" => Object::string_literal("Pets at the property"), "Kids" => vec![Object::Reference(yes), Object::Reference(no)] }));
        let option = |export: &str, label: &str| Object::Array(vec![Object::string_literal(export), Object::string_literal(label)]);
        let status = widget(d, dictionary! { "FT" => "Ch", "Ff" => COMBO, "T" => Object::string_literal("status"),
            "Opt" => vec![Object::string_literal("Employed"), Object::string_literal("Retired"), option("self", "Self-employed")] }, [72., 480., 300., 500.]);
        let languages = widget(d, dictionary! { "FT" => "Ch", "Ff" => MULTI_SELECT, "T" => Object::string_literal("languages"),
            "Opt" => vec![Object::string_literal("English"), Object::string_literal("French"), Object::string_literal("German")] }, [72., 400., 300., 460.]);
        let shift = widget(d, dictionary! { "FT" => "Ch", "T" => Object::string_literal("shift"),
            "Opt" => vec![Object::string_literal("Day"), Object::string_literal("Night")] }, [320., 400., 500., 460.]);
        let dob = widget(d, dictionary! { "FT" => "Tx", "T" => Object::string_literal("dob"), "AA" => dictionary! {
            "F" => script("AFDate_FormatEx(\"dd/mm/yyyy\");"), "K" => script("AFDate_KeystrokeEx(\"dd/mm/yyyy\");") } }, [72., 360., 300., 380.]);
        let amount_js = d.add_object(Stream::new(dictionary! {}, b"AFNumber_Format(2, 0, 0, 0, \"$\", true);".to_vec()));
        let amount = widget(d, dictionary! { "FT" => "Tx", "T" => Object::string_literal("amount"), "AA" => dictionary! {
            "F" => dictionary! { "S" => "JavaScript", "JS" => amount_js }, "K" => script("AFNumber_Keystroke(2, 0, 0, 0, \"$\", true);") } }, [72., 330., 300., 350.]);
        let zip = widget(d, dictionary! { "FT" => "Tx", "T" => Object::string_literal("zip"), "MaxLen" => 5, "AA" => dictionary! {
            "F" => script("AFSpecial_Format(0)"), "K" => script("AFSpecial_Keystroke(0)") } }, [72., 300., 300., 320.]);
        let annots = [yes, no, status, languages, shift, dob, amount, zip].map(Object::Reference);
        d.get_dictionary_mut(page).unwrap().get_mut(b"Annots").unwrap().as_array_mut().unwrap().extend(annots);
        d.get_dictionary_mut(acro_form).unwrap().get_mut(b"Fields").unwrap().as_array_mut().unwrap()
            .extend([pets, status, languages, shift, dob, amount, zip].map(Object::Reference));
    }

    fn identity_scene() -> PdfScene {
        PdfScene { pages: vec![ScenePage { mirrored: false, source_index: Some(0), width: 612., height: 792., rotation: 0, crop: None, source_rotation: None, source_box: None, objects: vec![] }] }
    }

    fn fill(input: &Path, scene: PdfScene, values: serde_json::Value) -> crate::kit::contracts::JobOutcome {
        let sections = serde_json::from_value(serde_json::json!({ "formValues": values })).unwrap();
        export(&ExportRequest { paths: vec![input.to_path_buf()], scene, output_location: OutputLocation::AlongsideInput, output_path: None, sections }, input.to_path_buf())
    }

    fn values() -> serde_json::Value {
        serde_json::json!([
            { "name": "name", "value": "Ada Lovelace" },
            { "name": "address.city", "value": "Zürich" },
            { "name": "agree", "value": true },
            { "name": "subscribe", "value": true },
        ])
    }

    fn field<'a>(doc: &'a Document, name: &[u8]) -> &'a Dictionary {
        doc.objects.values().filter_map(|object| object.as_dict().ok()).find(|dict| dict.get(b"T").ok().and_then(|value| value.as_str().ok()) == Some(name)).unwrap()
    }

    #[test]
    fn inspection_lists_text_fields_and_check_boxes_in_page_order() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("form.pdf"); form_fixture(&input, |_, _, _| {});
        let inspection = inspect_form(&input).unwrap();
        assert!(inspection.unsupported.is_none());
        let summary: Vec<_> = inspection.fields.iter().map(|field| (field.name.as_str(), field.kind, field.value.clone(), field.required)).collect();
        assert_eq!(summary, [
            ("name", FieldKind::Text, serde_json::json!(""), true),
            ("address.city", FieldKind::Text, serde_json::json!(""), false),
            ("agree", FieldKind::Checkbox, serde_json::json!(false), false),
            ("subscribe", FieldKind::Checkbox, serde_json::json!(false), false),
        ]);
        assert_eq!(inspection.fields[0].label.as_deref(), Some("Full name"));
        let widget = &inspection.fields[0].widgets[0];
        assert_eq!((widget.page, widget.x, widget.y, widget.width, widget.height), (0, 72., 68., 228., 24.));
        let wire = serde_json::to_value(&inspection).unwrap();
        assert_eq!(wire["fields"][2]["type"], "checkbox");
        assert_eq!(wire["fields"][2]["options"], serde_json::json!(["Agreed"]));
        assert_eq!(wire["fields"][3]["options"], serde_json::json!(["Yes"]));
        assert_eq!(wire["fields"][0]["readOnly"], false);
    }

    #[test]
    fn filled_copy_keeps_values_appearances_and_an_interactive_form() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("form.pdf"); form_fixture(&input, |_, _, _| {});
        let original = fs::read(&input).unwrap();
        let outcome = fill(&input, identity_scene(), values());
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        assert_eq!(outcome.output_paths(), [temp.0.join("form-filled.pdf")]);
        let second = fill(&input, identity_scene(), values());
        assert_eq!(second.output_paths(), [temp.0.join("form-filled-1.pdf")]);
        assert_eq!(fs::read(&input).unwrap(), original);

        let doc = Document::load(&outcome.output_paths()[0]).unwrap();
        assert!(doc.catalog().unwrap().has(b"AcroForm"));
        let name = field(&doc, b"name");
        assert_eq!(lopdf::decode_text_string(name.get(b"V").unwrap()).unwrap(), "Ada Lovelace");
        assert_eq!(name.get(b"FT").unwrap().as_name().unwrap(), b"Tx");
        assert_eq!(lopdf::decode_text_string(field(&doc, b"city").get(b"V").unwrap()).unwrap(), "Zürich");
        let appearance = doc.get_object(name.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap().as_reference().unwrap()).unwrap().as_stream().unwrap();
        let content = String::from_utf8(appearance.decompressed_content().unwrap()).unwrap();
        assert!(content.starts_with("/Tx BMC") && content.contains("0 0 1 rg") && content.contains("/ToolboxFormFont 12 Tf"), "{content}");
        let agree = field(&doc, b"agree");
        assert_eq!((agree.get(b"V").unwrap().as_name().unwrap(), agree.get(b"AS").unwrap().as_name().unwrap()), (&b"Agreed"[..], &b"Agreed"[..]));
        let subscribe = field(&doc, b"subscribe");
        assert_eq!(subscribe.get(b"AS").unwrap().as_name().unwrap(), b"Yes");
        let normal = subscribe.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap().as_dict().unwrap();
        assert!(normal.has(b"Yes") && normal.has(b"Off"));

        let reread = inspect_form(&outcome.output_paths()[0]).unwrap();
        let filled: Vec<_> = reread.fields.iter().map(|field| field.value.clone()).collect();
        assert_eq!(filled, [serde_json::json!("Ada Lovelace"), serde_json::json!("Zürich"), serde_json::json!(true), serde_json::json!(true)]);
        if let Ok(check) = Command::new("qpdf").arg("--check").arg(&outcome.output_paths()[0]).output() {
            assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stdout));
        }

        let Some(extractor) = text_extractor() else {
            if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some() { panic!("pdftotext is unavailable") }
            eprintln!("pdftotext unavailable"); return;
        };
        let output = Command::new(extractor).args(["-enc", "UTF-8"]).arg(&outcome.output_paths()[0]).arg("-").output().unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("Ada Lovelace") && text.contains("Zürich"), "{text}");
    }

    #[test]
    fn every_field_type_fills_reads_back_and_repeats_byte_for_byte() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("form.pdf"); form_fixture(&input, every_type);
        let inspection = inspect_form(&input).unwrap();
        assert!(inspection.unsupported.is_none());
        let wire = serde_json::to_value(&inspection).unwrap();
        let by_name = |name: &str| wire["fields"].as_array().unwrap().iter().find(|field| field["name"] == name).unwrap().clone();
        let pets = by_name("pets");
        assert_eq!((pets["type"].clone(), pets["options"].clone(), pets["required"].clone(), pets["value"].clone()), (serde_json::json!("radio"), serde_json::json!(["Yes", "No"]), serde_json::json!(true), serde_json::json!("")));
        assert_eq!(pets["widgets"].as_array().unwrap().iter().map(|widget| widget["option"].clone()).collect::<Vec<_>>(), [serde_json::json!("Yes"), serde_json::json!("No")]);
        let status = by_name("status");
        assert_eq!((status["type"].clone(), status["options"].clone(), status["optionLabels"].clone(), status["editable"].clone()),
            (serde_json::json!("dropdown"), serde_json::json!(["Employed", "Retired", "self"]), serde_json::json!(["Employed", "Retired", "Self-employed"]), serde_json::json!(false)));
        let languages = by_name("languages");
        assert_eq!((languages["type"].clone(), languages["multiSelect"].clone(), languages["value"].clone()), (serde_json::json!("listbox"), serde_json::json!(true), serde_json::json!([])));
        assert_eq!(by_name("shift")["multiSelect"], false);
        assert_eq!(by_name("dob")["format"], serde_json::json!({ "kind": "date", "pattern": "dd/mm/yyyy" }));
        assert_eq!(by_name("amount")["format"], serde_json::json!({ "kind": "number", "decimals": 2, "separator": 0, "negative": 0, "currency": "$", "currencyPrepend": true }));
        assert_eq!((by_name("zip")["format"].clone(), by_name("zip")["maxLen"].clone()), (serde_json::json!({ "kind": "special", "special": 0 }), serde_json::json!(5)));

        let mut all = values();
        all.as_array_mut().unwrap().extend(serde_json::json!([
            { "name": "pets", "value": "No" },
            { "name": "status", "value": "self" },
            { "name": "languages", "value": ["German", "English"] },
            { "name": "shift", "value": ["Night"] },
            { "name": "dob", "value": "29/02/2024" },
            { "name": "amount", "value": "-1234.5" },
            { "name": "zip", "value": "12345" },
        ]).as_array().unwrap().iter().cloned());
        let original = fs::read(&input).unwrap();
        let first = fill(&input, identity_scene(), all.clone());
        assert!(first.error().is_none(), "{:?}", first.error());
        let second = fill(&input, identity_scene(), all);
        assert_eq!(first.output_paths(), [temp.0.join("form-filled.pdf")]);
        assert_eq!(second.output_paths(), [temp.0.join("form-filled-1.pdf")]);
        assert_eq!(fs::read(&first.output_paths()[0]).unwrap(), fs::read(&second.output_paths()[0]).unwrap());
        assert_eq!(fs::read(&input).unwrap(), original);

        let output = &first.output_paths()[0];
        let reread = inspect_form(output).unwrap();
        let filled: Vec<_> = reread.fields.iter().map(|field| (field.name.as_str(), field.value.clone())).collect();
        assert_eq!(filled, [
            ("name", serde_json::json!("Ada Lovelace")), ("address.city", serde_json::json!("Zürich")),
            ("agree", serde_json::json!(true)), ("subscribe", serde_json::json!(true)),
            ("pets", serde_json::json!("No")), ("status", serde_json::json!("self")),
            ("languages", serde_json::json!(["English", "German"])), ("shift", serde_json::json!(["Night"])),
            ("dob", serde_json::json!("29/02/2024")), ("amount", serde_json::json!("-1234.5")), ("zip", serde_json::json!("12345")),
        ]);
        let doc = Document::load(output).unwrap();
        let pets = field(&doc, b"pets");
        assert_eq!(pets.get(b"V").unwrap().as_name().unwrap(), b"No");
        let states: Vec<_> = pets.get(b"Kids").unwrap().as_array().unwrap().iter()
            .map(|kid| doc.get_dictionary(kid.as_reference().unwrap()).unwrap().get(b"AS").unwrap().as_name().unwrap().to_vec()).collect();
        assert_eq!(states, [b"Off".to_vec(), b"No".to_vec()]);
        let languages = field(&doc, b"languages");
        assert_eq!(languages.get(b"I").unwrap().as_array().unwrap().iter().map(|index| index.as_i64().unwrap()).collect::<Vec<_>>(), [0, 2]);
        let listing = doc.get_object(languages.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap().as_reference().unwrap()).unwrap().as_stream().unwrap();
        let listing = String::from_utf8(listing.decompressed_content().unwrap()).unwrap();
        assert_eq!(listing.matches("0.6 0.75 0.86 rg").count(), 2, "{listing}");
        if let Ok(check) = Command::new("qpdf").arg("--check").arg(output).output() {
            assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stdout));
        }
        let Some(extractor) = text_extractor() else {
            if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some() { panic!("pdftotext is unavailable") }
            eprintln!("pdftotext unavailable"); return;
        };
        let output = Command::new(extractor).args(["-enc", "UTF-8"]).arg(output).arg("-").output().unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        for shown in ["Self-employed", "German", "29/02/2024", "-$1,234.50", "12345"] { assert!(text.contains(shown), "{shown} missing from {text}"); }
    }

    #[test]
    fn evaluable_formats_check_and_display_entries() {
        assert!(is_real_date("dd/mm/yyyy", "29/02/2024") && !is_real_date("dd/mm/yyyy", "29/02/2023") && !is_real_date("dd/mm/yyyy", "1/02/2024"));
        assert!(is_real_date("d-mmm-yy", "3-Mar-99") && is_real_date("mmmm d, yyyy", "april 30, 2025") && !is_real_date("mmmm d, yyyy", "April 31, 2025"));
        assert!(script_format("AFDate_FormatEx(\"dd/mm/yyyy HH:MM\")").is_none() && script_format("AFTime_Format(0)").is_none() && script_format("event.value = 1").is_none());
        assert!(script_format("AFNumber_Keystroke(2, 0, 0, 0, \"$\", true); app.alert(1)").is_none());
        let euro = script_format("AFNumber_Format(2, 2, 2, 0, ' €', false)").unwrap();
        assert_eq!(euro.entry("-1234567,891"), Some(("-1234567.891".into(), "(1.234.567,89 €)".into(), false)));
        assert_eq!(euro.entry("1.5"), None);
        let red = script_format("AFNumber_Format(0, 1, 3, 0, \"\", false)").unwrap();
        assert_eq!(red.entry("-7"), Some(("-7".into(), "(7)".into(), true)));
        assert_eq!(script_format("AFPercent_Format(1, 0)").unwrap().entry(".256"), Some(("0.256".into(), "25.6%".into(), false)));
        let phone = script_format("AFSpecial_Keystroke(2)").unwrap();
        assert_eq!(phone.entry("555 123.4567"), Some(("5551234567".into(), "(555) 123-4567".into(), false)));
        assert_eq!(phone.entry("555-CALL"), None);
        assert_eq!(script_format("AFSpecial_Format(3)").unwrap().entry("123456789").map(|entry| entry.1), Some("123-45-6789".into()));
    }

    #[test]
    fn unchecking_and_chaining_with_overlays_keeps_both_changes() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("form.pdf");
        form_fixture(&input, |d, _, _| {
            let agree = d.objects.iter().find(|(_, object)| object.as_dict().is_ok_and(|dict| dict.get(b"T").ok().and_then(|v| v.as_str().ok()) == Some(b"agree"))).map(|(id, _)| *id).unwrap();
            let dict = d.get_dictionary_mut(agree).unwrap(); dict.set("V", "Agreed"); dict.set("AS", "Agreed");
        });
        let mut scene = identity_scene();
        let mut note = super::super::SceneObject { text: "Reviewed".into(), font_size: 14., ..text_object(&inspect_form(&input).unwrap().fields[0], "", (100., 20.), false) };
        note.rect = Rect { x: 300., y: 40., width: 200., height: 30. };
        scene.pages[0].objects.push(note);
        let outcome = fill(&input, scene, serde_json::json!([{ "name": "agree", "value": false }]));
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        assert_eq!(outcome.output_paths(), [temp.0.join("form-edited.pdf")]);
        let doc = Document::load(&outcome.output_paths()[0]).unwrap();
        let agree = field(&doc, b"agree");
        assert_eq!((agree.get(b"V").unwrap().as_name().unwrap(), agree.get(b"AS").unwrap().as_name().unwrap()), (&b"Off"[..], &b"Off"[..]));
        let page = doc.get_pages()[&1];
        assert_eq!(doc.get_dictionary(page).unwrap().get(b"Annots").unwrap().as_array().unwrap().len(), 4);
        assert!(doc.get_page_fonts(page).unwrap().values().any(|font| font.get(b"BaseFont").ok().and_then(|v| v.as_name().ok()).is_some_and(|name| name.ends_with(b"+NotoSans-Regular"))));
    }

    #[test]
    fn unfillable_requests_are_refused_before_output() {
        let temp = TempDir::new().unwrap();
        let plain = temp.0.join("plain.pdf");
        form_fixture(&plain, |d, _, _| { d.catalog_mut().unwrap().remove(b"AcroForm"); });
        let xfa = temp.0.join("xfa.pdf");
        form_fixture(&xfa, |d, acro_form, _| { let packet = d.add_object(Stream::new(dictionary! {}, b"<xdp:xdp/>".to_vec())); d.get_dictionary_mut(acro_form).unwrap().set("XFA", packet); });
        let scripted = temp.0.join("scripted.pdf");
        form_fixture(&scripted, |d, _, city| { d.get_dictionary_mut(city).unwrap().set("AA", dictionary! { "K" => dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("AFTime_Keystroke(0)") } }); });
        let every = temp.0.join("every.pdf");
        form_fixture(&every, every_type);
        let form = temp.0.join("form.pdf");
        form_fixture(&form, |d, _, city| { d.get_dictionary_mut(city).unwrap().set("Ff", READ_ONLY); });
        assert_eq!(inspect_form(&plain).unwrap().fields.len(), 0);
        assert_eq!(inspect_form(&xfa).unwrap().unsupported.as_deref(), Some(XFA_UNSUPPORTED));
        assert_eq!(inspect_form(&scripted).unwrap().unsupported.as_deref(), Some(SCRIPTED_UNSUPPORTED));
        assert!(inspect_form(&form).unwrap().fields[1].read_only);
        let cases = [
            (&plain, values(), "This PDF has no form fields to fill. Use Add text to type onto the page instead."),
            (&xfa, values(), XFA_UNSUPPORTED),
            (&scripted, values(), SCRIPTED_UNSUPPORTED),
            (&form, serde_json::json!([{ "name": "missing", "value": "x" }]), "This PDF has no fillable field named “missing”."),
            (&every, serde_json::json!([{ "name": "pets", "value": true }]), "“pets” is a choice field, so its value must be one of its options."),
            (&every, serde_json::json!([{ "name": "pets", "value": "Maybe" }]), "“pets” has no option “Maybe”."),
            (&every, serde_json::json!([{ "name": "status", "value": "Self-employed" }]), "“status” has no option “Self-employed”."),
            (&every, serde_json::json!([{ "name": "languages", "value": "English" }]), "“languages” is a list box, so its value must be a list of its options."),
            (&every, serde_json::json!([{ "name": "languages", "value": ["English", "English"] }]), "“languages” lists “English” more than once."),
            (&every, serde_json::json!([{ "name": "shift", "value": ["Day", "Night"] }]), "“shift” allows only one choice."),
            (&every, serde_json::json!([{ "name": "dob", "value": "31/02/2024" }]), "“dob” is not a real date. Enter it as dd/mm/yyyy."),
            (&every, serde_json::json!([{ "name": "amount", "value": "12a" }]), "“amount” must be a number."),
            (&every, serde_json::json!([{ "name": "zip", "value": "123456" }]), "“zip” can be at most 5 characters."),
            (&every, serde_json::json!([{ "name": "zip", "value": "1234" }]), "“zip” must be a 5-digit ZIP code."),
            (&form, serde_json::json!([{ "name": "name", "value": true }]), "“name” is a text field, so its value must be text."),
            (&form, serde_json::json!([{ "name": "agree", "value": "yes" }]), "“agree” is a check box, so its value must be checked or unchecked."),
            (&form, serde_json::json!([{ "name": "address.city", "value": "Oslo" }]), "“address.city” is read-only, so it can't be filled."),
            (&form, serde_json::json!([{ "name": "name", "value": "A" }, { "name": "name", "value": "B" }]), "“name” was given more than one value."),
            (&form, serde_json::json!([{ "name": "name", "value": "日本" }]), "Noto Sans can't show these characters: “日” (U+65E5), “本” (U+672C). Remove them to export the “name” field."),
        ];
        for (input, values, message) in cases {
            let original = fs::read(input).unwrap();
            let outcome = fill(input, identity_scene(), values);
            let error = outcome.error().unwrap();
            assert_eq!((error.kind.clone(), error.message.as_str()), (ErrorKind::InvalidInput, message));
            assert!(outcome.output_paths().is_empty());
            assert_eq!(fs::read(input).unwrap(), original);
        }
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 5);
    }

    /// A drawn comment with its pop-up note, a hidden note and a link, beside the form.
    fn with_comments(d: &mut Document, _: ObjectId, city: ObjectId) {
        let page = d.get_dictionary(city).unwrap().get(b"P").unwrap().as_reference().unwrap();
        let box_ap = d.add_object(Stream::new(dictionary! { "BBox" => rect([0., 0., 100., 50.]) }, b"1 0 0 RG 4 w 2 2 96 46 re S".to_vec()));
        let square = d.new_object_id();
        let popup = d.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Popup", "Parent" => square, "Rect" => rect([420., 300., 560., 380.]) });
        d.objects.insert(square, Object::Dictionary(dictionary! { "Type" => "Annot", "Subtype" => "Square", "Rect" => rect([340., 300., 440., 350.]),
            "Contents" => Object::string_literal("Check this"), "AP" => dictionary! { "N" => box_ap }, "Popup" => popup, "P" => page }));
        let hidden_ap = d.add_object(Stream::new(dictionary! { "BBox" => rect([0., 0., 20., 20.]) }, b"0 g 0 0 20 20 re f".to_vec()));
        let hidden = d.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Text", "F" => 2, "Rect" => rect([340., 200., 360., 220.]), "AP" => dictionary! { "N" => hidden_ap }, "P" => page });
        let link = d.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => rect([72., 100., 200., 120.]), "Border" => vec![0.into(), 0.into(), 0.into()],
            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com") } });
        d.get_dictionary_mut(page).unwrap().get_mut(b"Annots").unwrap().as_array_mut().unwrap().extend([square, popup, hidden, link].map(Object::Reference));
    }

    fn export_sections(input: &Path, sections: serde_json::Value) -> crate::kit::contracts::JobOutcome {
        let sections = serde_json::from_value(sections).unwrap();
        export(&ExportRequest { paths: vec![input.to_path_buf()], scene: identity_scene(), output_location: OutputLocation::AlongsideInput, output_path: None, sections }, input.to_path_buf())
    }

    fn subtypes(path: &Path) -> Vec<Vec<u8>> {
        let doc = Document::load(path).unwrap();
        let page = doc.get_pages()[&1];
        let Ok(annots) = doc.get_dictionary(page).unwrap().get(b"Annots") else { return Vec::new() };
        annots.as_array().unwrap().iter().map(|entry| doc.get_dictionary(entry.as_reference().unwrap()).unwrap().get(b"Subtype").unwrap().as_name().unwrap().to_vec()).collect()
    }

    /// Checks the output is valid and returns its page text and rendering, or None when the Poppler helpers are missing.
    fn check(path: &Path, temp: &TempDir) -> Option<(String, String)> {
        if let Ok(check) = Command::new("qpdf").arg("--check").arg(path).output() {
            assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stdout));
        }
        let (Some(extractor), Ok(renderer)) = (text_extractor(), super::super::renderer()) else {
            if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some() { panic!("pdftotext or pdftoppm is unavailable") }
            eprintln!("pdftotext or pdftoppm unavailable"); return None;
        };
        let text = String::from_utf8(Command::new(extractor).args(["-enc", "UTF-8"]).arg(path).arg("-").output().unwrap().stdout).unwrap();
        Some((text, super::super::render(path, 0, &renderer, temp).unwrap().data_url))
    }

    #[test]
    fn flattening_draws_the_chosen_kind_into_the_page_and_removes_it() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("form.pdf"); form_fixture(&input, with_comments);
        assert_eq!(inspect_form(&input).unwrap().comments, 2);
        let filled = fill(&input, identity_scene(), values());
        let filled = filled.output_paths()[0].clone();
        let original = fs::read(&filled).unwrap();
        let before = check(&filled, &temp);
        let cases: [(&str, &[&[u8]], bool); 3] = [
            ("forms", &[b"Square", b"Popup", b"Text", b"Link"], false),
            ("annotations", &[b"Widget", b"Widget", b"Widget", b"Widget", b"Link"], true),
            ("both", &[b"Link"], false),
        ];
        for (index, (choice, remaining, interactive)) in cases.into_iter().enumerate() {
            let outcome = export_sections(&filled, serde_json::json!({ "flatten": choice }));
            assert!(outcome.error().is_none(), "{:?}", outcome.error());
            let suffix = if index == 0 { String::new() } else { format!("-{index}") };
            assert_eq!(outcome.output_paths(), [temp.0.join(format!("form-filled-flattened{suffix}.pdf"))]);
            let output = &outcome.output_paths()[0];
            assert_eq!(subtypes(output), remaining.iter().map(|subtype| subtype.to_vec()).collect::<Vec<_>>(), "{choice}");
            assert_eq!(Document::load(output).unwrap().catalog().unwrap().has(b"AcroForm"), interactive, "{choice}");
            assert_eq!(inspect_form(output).unwrap().fields.is_empty(), !interactive, "{choice}");
            if let (Some((text, render)), Some((_, expected))) = (check(output, &temp), &before) {
                assert!(text.contains("Application") && text.contains("Ada Lovelace") && text.contains("Zürich"), "{text}");
                assert!(&render == expected, "{choice} renders differently after flattening");
            }
        }
        assert_eq!(fs::read(&filled).unwrap(), original);
    }

    #[test]
    fn filling_and_flattening_together_lock_the_values_into_page_content() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("form.pdf");
        // A stored value without an appearance is drawn before flattening, so it isn't lost.
        form_fixture(&input, |d, _, city| { d.get_dictionary_mut(city).unwrap().set("V", Object::string_literal("Bern")); });
        let original = fs::read(&input).unwrap();
        let filled_only = fill(&input, identity_scene(), serde_json::json!([{ "name": "name", "value": "Ada Lovelace" }, { "name": "agree", "value": true }]));
        let outcome = export_sections(&input, serde_json::json!({ "formValues": [{ "name": "name", "value": "Ada Lovelace" }, { "name": "agree", "value": true }], "flatten": "forms" }));
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let output = &outcome.output_paths()[0];
        assert_eq!(output, &temp.0.join("form-edited.pdf"));
        assert!(subtypes(output).is_empty());
        assert!(!Document::load(output).unwrap().catalog().unwrap().has(b"AcroForm"));
        assert_eq!(fs::read(&input).unwrap(), original);
        if let Some((text, render)) = check(output, &temp) {
            assert!(text.contains("Ada Lovelace") && text.contains("Bern"), "{text}");
            let (_, interactive) = check(&filled_only.output_paths()[0], &temp).unwrap();
            assert_eq!(subtypes(&filled_only.output_paths()[0]).len(), 4);
            // The interactive copy has no drawing for the stored city, so only the flattened copy shows it.
            assert!(render != interactive);
        }
    }

    #[test]
    fn nothing_to_flatten_is_refused_before_output() {
        let temp = TempDir::new().unwrap();
        let plain = temp.0.join("plain.pdf");
        form_fixture(&plain, |d, _, city| {
            let page = d.get_dictionary(city).unwrap().get(b"P").unwrap().as_reference().unwrap();
            d.catalog_mut().unwrap().remove(b"AcroForm");
            d.get_dictionary_mut(page).unwrap().remove(b"Annots");
        });
        let form = temp.0.join("form.pdf");
        form_fixture(&form, |_, _, _| {});
        let xfa = temp.0.join("xfa.pdf");
        form_fixture(&xfa, |d, acro_form, _| { let packet = d.add_object(Stream::new(dictionary! {}, b"<xdp:xdp/>".to_vec())); d.get_dictionary_mut(acro_form).unwrap().set("XFA", packet); });
        assert_eq!(inspect_form(&form).unwrap().comments, 0);
        let cases = [
            (&plain, "forms", "This PDF has no form fields to flatten."),
            (&plain, "both", "This PDF has no form fields or comments to flatten."),
            (&form, "annotations", "This PDF has no comments or markup to flatten."),
            (&xfa, "forms", super::super::flatten::XFA_UNSUPPORTED),
        ];
        for (input, choice, message) in cases {
            let original = fs::read(input).unwrap();
            let outcome = export_sections(input, serde_json::json!({ "flatten": choice }));
            let error = outcome.error().unwrap();
            assert_eq!((error.kind.clone(), error.message.as_str()), (ErrorKind::InvalidInput, message));
            assert!(outcome.output_paths().is_empty());
            assert_eq!(fs::read(input).unwrap(), original);
        }
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 3);
    }

    /// A second page with a plain line the Edit text stage can change.
    fn with_rent_page(d: &mut Document, _: ObjectId, city: ObjectId) {
        let first = d.get_dictionary(city).unwrap().get(b"P").unwrap().as_reference().unwrap();
        let pages = d.get_dictionary(first).unwrap().get(b"Parent").unwrap().as_reference().unwrap();
        let font = d.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
        let content = d.add_object(Stream::new(dictionary! {}, b"BT /Body 14 Tf 72 700 Td (Rent 1450 per month) Tj ET".to_vec()));
        let second = d.add_object(dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => rect([0., 0., 612., 792.]),
            "Resources" => dictionary! { "Font" => dictionary! { "Body" => font } }, "Contents" => content });
        let kids = d.get_dictionary_mut(pages).unwrap();
        kids.get_mut(b"Kids").unwrap().as_array_mut().unwrap().push(Object::Reference(second));
        kids.set("Count", 2);
    }

    #[test]
    fn one_export_chains_every_stage_in_the_fixed_order_and_redaction_removes_what_came_before() {
        let _guard = crate::kit::lock_process_env();
        let (Some(extractor), Ok(_)) = (text_extractor(), super::super::renderer()) else {
            if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some() { panic!("pdftotext or pdftoppm is unavailable") }
            return eprintln!("pdftotext or pdftoppm unavailable");
        };
        let temp = TempDir::new().unwrap(); let input = temp.0.join("lease.pdf"); form_fixture(&input, with_rent_page);
        let original = fs::read(&input).unwrap();
        let overlay = |text: &str| serde_json::json!({ "kind": "text", "rect": { "x": 72, "y": 200, "width": 360, "height": 40 }, "text": text, "fontSize": 18, "color": "#202020", "opacity": 1, "strokes": [] });
        let request: ExportRequest = serde_json::from_value(serde_json::json!({
            "paths": [input], "outputLocation": "alongsideInput",
            "scene": { "pages": [
                { "sourceIndex": 0, "width": 612, "height": 792, "rotation": 0, "crop": null, "objects": [overlay("Overlaysecret")] },
                { "sourceIndex": 1, "width": 612, "height": 792, "rotation": 0, "crop": null, "objects": [overlay("Visible note")] }
            ] },
            "formValues": [{ "name": "name", "value": "Zebulon Quill" }, { "name": "agree", "value": true }],
            "textEdits": [{ "pageIndex": 1, "run": 0, "original": "Rent 1450 per month", "replacement": "Rent 1475 per month" }],
            "flatten": "forms",
            "redactions": [{ "pageIndex": 0, "rect": { "x": 0, "y": 0, "width": 612, "height": 300 }, "color": "#000000" }],
        })).unwrap();

        let outcome = export(&request, input.clone());
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let output = outcome.output_paths()[0].clone();
        assert_eq!(output, temp.0.join("lease-edited.pdf"), "several stages share the combined name");
        assert_eq!(outcome.detail, "2 pages kept, 0 removed, 0 rotated, 0 mirrored. Changes applied in this fixed order: Fill form; Edit text; Add text, signatures and page marks; Flatten; Redact. \
Page 1 was redacted as a 300 dpi image. Document metadata, XMP data and attachments were removed.");
        assert_eq!(fs::read(&input).unwrap(), original, "the original is unchanged");

        let text = |page: usize| String::from_utf8(Command::new(&extractor).args(["-enc", "UTF-8", "-f", &page.to_string(), "-l", &page.to_string()]).arg(&output).arg("-").output().unwrap().stdout).unwrap();
        assert!(text(1).trim().is_empty(), "the redacted page keeps no text: {}", text(1));
        let kept = text(2);
        assert!(kept.contains("Rent 1475 per month") && !kept.contains("1450"), "the text edit reaches the unredacted page: {kept}");
        assert!(kept.contains("Visible note"), "overlays on unredacted pages stay: {kept}");
        let bytes = fs::read(&output).unwrap().to_ascii_lowercase();
        for secret in ["zebulon", "quill", "overlaysecret", "application"] {
            assert!(!bytes.windows(secret.len()).any(|window| window == secret.as_bytes()), "{secret} survived in the file bytes");
        }
        let document = Document::load(&output).unwrap();
        assert!(!document.catalog().unwrap().has(b"AcroForm"), "flattened fields leave no form behind");
        assert!(subtypes(&output).is_empty());

        let again = export(&request, input.clone());
        assert_eq!(again.output_paths(), [temp.0.join("lease-edited-1.pdf")], "never overwrites an earlier output");
        assert_eq!(fs::read(&again.output_paths()[0]).unwrap(), fs::read(&output).unwrap(), "repeated runs are byte-identical");
        assert_eq!(fs::read(&input).unwrap(), original);
    }
}
