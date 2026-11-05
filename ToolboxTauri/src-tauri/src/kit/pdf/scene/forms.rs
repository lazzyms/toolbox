use std::collections::{HashMap, HashSet};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use serde::Serialize;
use crate::kit::contracts::ToolError;
use super::{compress_bytes, embed_text_fonts, err, geometry, layout_text_for, load, resolve, text_content, Kind, Rect, SceneObject, TextAlign, TextLayout, TEXT_LINE_HEIGHT};

const READ_ONLY: i64 = 1;
const REQUIRED: i64 = 1 << 1;
const MULTILINE: i64 = 1 << 12;
const PASSWORD: i64 = 1 << 13;
const RADIO: i64 = 1 << 15;
const PUSHBUTTON: i64 = 1 << 16;
const FILE_SELECT: i64 = 1 << 20;
const FONT_RESOURCE: &str = "ToolboxFormFont";
const PADDING: f32 = 2.;

pub const XFA_UNSUPPORTED: &str = "This form can't be filled here. It was built with XFA, an older Adobe form format that Toolbox doesn't support.";
pub const SCRIPTED_UNSUPPORTED: &str = "This form can't be filled here. Its fields run scripts to format what you type, which Toolbox doesn't support.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind { Text, Checkbox }

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormWidget { pub page: usize, pub x: f32, pub y: f32, pub width: f32, pub height: f32 }

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormField {
    pub name: String,
    pub label: Option<String>,
    #[serde(rename = "type")] pub kind: FieldKind,
    pub value: serde_json::Value,
    pub options: Vec<String>,
    pub flags: i64,
    pub required: bool,
    pub read_only: bool,
    pub multiline: bool,
    pub max_len: Option<i64>,
    pub widgets: Vec<FormWidget>,
    #[serde(skip)] id: ObjectId,
    #[serde(skip)] widget_ids: Vec<ObjectId>,
    #[serde(skip)] on_state: Vec<u8>,
    #[serde(skip)] appearance: String,
    #[serde(skip)] align: TextAlign,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInspection { pub fields: Vec<FormField>, pub unsupported: Option<String> }

#[derive(Clone)]
struct Inherited { name: String, kind: Option<Vec<u8>>, flags: i64, appearance: Option<String>, quadding: Option<i64>, value: Option<Object>, max_len: Option<i64>, scripted: bool }

struct Terminal { id: ObjectId, inherited: Inherited, label: Option<String>, widgets: Vec<ObjectId> }

pub fn inspect_form(path: &std::path::Path) -> Result<FormInspection, String> { read(&load(path)?) }

fn text(doc: &Document, value: Option<&Object>) -> Option<String> {
    value.and_then(|value| resolve(doc, value).ok()).and_then(|value| lopdf::decode_text_string(value).ok())
}

fn integer(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<i64> {
    dict.get(key).ok().and_then(|value| resolve(doc, value).ok()).and_then(|value| value.as_i64().ok())
}

fn formats_with_script(doc: &Document, dict: &Dictionary) -> bool {
    dict.get(b"AA").ok().and_then(|value| resolve(doc, value).ok()).and_then(|value| value.as_dict().ok())
        .is_some_and(|actions| actions.has(b"F") || actions.has(b"K"))
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
    inherited.scripted |= formats_with_script(doc, dict);
    let kids = match dict.get(b"Kids") {
        Ok(kids) => resolve(doc, kids)?.as_array().map_err(|_| malformed())?.iter().map(|kid| kid.as_reference().map_err(|_| malformed())).collect::<Result<Vec<_>, _>>()?,
        Err(_) => Vec::new(),
    };
    let mut widgets = Vec::new();
    for kid in kids {
        let child = doc.get_dictionary(kid).map_err(|_| malformed())?;
        if is_widget(child) && !child.has(b"T") {
            inherited.scripted |= formats_with_script(doc, child);
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
        return Ok(FormInspection { fields: Vec::new(), unsupported: Some(XFA_UNSUPPORTED.into()) });
    }
    let root = Inherited {
        name: String::new(), kind: None, flags: 0, appearance: text(doc, acro_form.get(b"DA").ok()),
        quadding: integer(doc, acro_form, b"Q"), value: None, max_len: None, scripted: false,
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
            placed.push((id, FormWidget { page: page_index, x: x0, y: y0, width: x1 - x0, height: y1 - y0 }));
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
        let Inherited { name, kind, flags, appearance, quadding, value, max_len, scripted: has_script } = terminal.inherited;
        let kind = match kind.as_deref() {
            Some(b"Tx") if flags & (PASSWORD | FILE_SELECT) == 0 => FieldKind::Text,
            Some(b"Btn") if flags & (RADIO | PUSHBUTTON) == 0 => FieldKind::Checkbox,
            _ => continue,
        };
        let mut widgets: Vec<_> = terminal.widgets.iter().filter_map(|id| placement.get(id).map(|placed| (*id, placed.clone()))).collect();
        if widgets.is_empty() || name.is_empty() { continue; }
        scripted |= has_script;
        widgets.sort_by_key(|(_, (page, order, _))| (*page, *order));
        let on = terminal.widgets.iter().find_map(|id| doc.get_dictionary(*id).ok().and_then(|widget| on_state(doc, widget))).unwrap_or_else(|| b"Yes".to_vec());
        let value = match kind {
            FieldKind::Text => serde_json::Value::String(value.as_ref().and_then(|value| lopdf::decode_text_string(value).ok()).unwrap_or_default()),
            FieldKind::Checkbox => {
                let state = value.as_ref().and_then(|value| value.as_name().ok().map(<[u8]>::to_vec))
                    .or_else(|| doc.get_dictionary(widgets[0].0).ok()?.get(b"AS").ok()?.as_name().ok().map(<[u8]>::to_vec));
                serde_json::Value::Bool(state.is_some_and(|state| state != b"Off"))
            }
        };
        fields.push((widgets[0].1.0, widgets[0].1.1, FormField {
            options: if kind == FieldKind::Checkbox { vec![String::from_utf8_lossy(&on).into_owned()] } else { Vec::new() },
            name, label: terminal.label, kind, value, flags,
            required: flags & REQUIRED != 0, read_only: flags & READ_ONLY != 0, multiline: kind == FieldKind::Text && flags & MULTILINE != 0,
            max_len: if kind == FieldKind::Text { max_len } else { None },
            widgets: widgets.iter().map(|(_, (_, _, widget))| widget.clone()).collect(),
            id: terminal.id, widget_ids: widgets.iter().map(|(id, _)| *id).collect(), on_state: on,
            appearance: appearance.unwrap_or_default(),
            align: match quadding { Some(1) => TextAlign::Center, Some(2) => TextAlign::Right, _ => TextAlign::Left },
        }));
    }
    if scripted { return Ok(FormInspection { fields: Vec::new(), unsupported: Some(SCRIPTED_UNSUPPORTED.into()) }); }
    fields.sort_by_key(|(page, order, _)| (*page, *order));
    Ok(FormInspection { fields: fields.into_iter().map(|(_, _, field)| field).collect(), unsupported: None })
}

enum Fill { Text(String), Check(bool) }

pub(super) struct FormPlan { fills: Vec<(FormField, Fill)>, objects: HashMap<(usize, usize), SceneObject>, layouts: HashMap<(usize, usize), TextLayout> }

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

fn text_object(field: &FormField, text: &str, size: (f32, f32)) -> SceneObject {
    let parsed = parse_appearance(&field.appearance);
    let (width, height) = size;
    let fit = ((height - 2. * PADDING) / TEXT_LINE_HEIGHT).clamp(4., 12.);
    let font_size = if parsed.size > 0. { parsed.size.min(1000.) } else { fit };
    let rect = if field.multiline {
        Rect { x: PADDING, y: PADDING, width: (width - 2. * PADDING).max(1.), height: (height - 2. * PADDING).max(1.) }
    } else {
        let line = font_size * TEXT_LINE_HEIGHT;
        Rect { x: PADDING, y: (height - line) / 2., width: (width - 2. * PADDING).max(1.), height: line }
    };
    SceneObject {
        kind: Kind::Text, rect, text: text.into(), font_size, color: "#000000".into(), opacity: 1., strokes: Vec::new(),
        shape: None, highlight_mode: None, signature_mode: None, signature_path: None, font_family: None, watermark_pattern: None,
        bold: false, italic: false, align: field.align, page_range: None,
    }
}

pub(super) fn preflight(doc: &Document, values: &[serde_json::Value]) -> Result<FormPlan, ToolError> {
    let inspection = read(doc).map_err(ToolError::processing)?;
    if let Some(message) = inspection.unsupported { return Err(ToolError::invalid_input(message)); }
    if inspection.fields.is_empty() { return Err(ToolError::invalid_input("This PDF has no form fields to fill. Use Add text to type onto the page instead.")); }
    let mut fields: HashMap<String, FormField> = inspection.fields.into_iter().map(|field| (field.name.clone(), field)).collect();
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
        let fill = match (field.kind, value) {
            (FieldKind::Text, serde_json::Value::String(text)) => {
                let text = if field.multiline { text.clone() } else { text.replace("\r\n", " ").replace(['\r', '\n'], " ") };
                for (index, id) in field.widget_ids.iter().enumerate() {
                    let size = widget_size(doc, *id).map_err(ToolError::processing)?;
                    let object = text_object(&field, &text, size);
                    let mut layout_object = object.clone();
                    if !field.multiline { layout_object.rect.width = f32::MAX / 4.; }
                    let layout = layout_text_for(&layout_object, &format!("the “{name}” field"))?;
                    plan.objects.insert((plan.fills.len(), index), object);
                    plan.layouts.insert((plan.fills.len(), index), layout);
                }
                Fill::Text(text)
            }
            (FieldKind::Checkbox, serde_json::Value::Bool(checked)) => Fill::Check(*checked),
            (FieldKind::Text, _) => return Err(ToolError::invalid_input(format!("“{name}” is a text field, so its value must be text."))),
            (FieldKind::Checkbox, _) => return Err(ToolError::invalid_input(format!("“{name}” is a check box, so its value must be checked or unchecked."))),
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

pub(super) fn apply(doc: &mut Document, plan: FormPlan) -> Result<(), String> {
    let fonts = embed_text_fonts(doc, &plan.layouts)?;
    for (fill_index, (field, fill)) in plan.fills.into_iter().enumerate() {
        match fill {
            Fill::Text(value) => {
                let dict = doc.get_dictionary_mut(field.id).map_err(err)?;
                dict.set("V", lopdf::text_string(&value));
                dict.remove(b"RV");
                let color = parse_appearance(&field.appearance).color;
                for (index, id) in field.widget_ids.iter().enumerate() {
                    let (width, height) = widget_size(doc, *id)?;
                    let layout = &plan.layouts[&(fill_index, index)];
                    let (font_id, remapper) = &fonts[layout.font.name];
                    let content = format!("/Tx BMC\nq\n1 0 0 -1 0 {height} cm\n{color}\n{}Q\nEMC\n", text_content(&plan.objects[&(fill_index, index)], layout, FONT_RESOURCE, remapper));
                    let resources = dictionary! { "Font" => dictionary! { FONT_RESOURCE => *font_id } };
                    let appearance = form_xobject(doc, width, height, resources, &content)?;
                    doc.get_dictionary_mut(*id).map_err(err)?.set("AP", dictionary! { "N" => appearance });
                }
            }
            Fill::Check(checked) => {
                let state = if checked { field.on_state.clone() } else { b"Off".to_vec() };
                doc.get_dictionary_mut(field.id).map_err(err)?.set("V", Object::Name(state));
                for id in &field.widget_ids {
                    let (width, height) = widget_size(doc, *id)?;
                    let widget = doc.get_dictionary(*id).map_err(err)?;
                    let mut appearances = widget.get(b"AP").ok().and_then(|value| resolve(doc, value).ok()).and_then(|value| value.as_dict().ok()).cloned().unwrap_or_default();
                    let mut normal = appearances.get(b"N").ok().and_then(|value| resolve(doc, value).ok()).and_then(|value| value.as_dict().ok()).cloned().unwrap_or_default();
                    let own_on = normal.iter().map(|(key, _)| key.clone()).find(|key| key.as_slice() != b"Off");
                    if own_on.is_none() {
                        let check = format!("q 0 G {} w 1 J 1 j {} {} m {} {} l {} {} l S Q\n", (width.min(height) / 10.).max(0.5),
                            width * 0.22, height * 0.52, width * 0.42, height * 0.28, width * 0.8, height * 0.76);
                        normal.set(field.on_state.clone(), form_xobject(doc, width, height, Dictionary::new(), &check)?);
                    }
                    if !normal.has(b"Off") { normal.set("Off", form_xobject(doc, width, height, Dictionary::new(), "")?); }
                    let on = own_on.unwrap_or_else(|| field.on_state.clone());
                    appearances.set("N", normal);
                    let widget = doc.get_dictionary_mut(*id).map_err(err)?;
                    widget.set("AP", appearances);
                    widget.set("AS", Object::Name(if checked { on } else { b"Off".to_vec() }));
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
    use super::super::{export, text_extractor, ExportRequest, PdfScene, ScenePage, TempDir};
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
        let choice = d.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Widget", "FT" => "Btn", "Ff" => RADIO, "T" => Object::string_literal("choice"),
            "Rect" => rect([200., 560., 218., 578.]), "P" => page });
        let content = d.add_object(Stream::new(dictionary! {}, b"BT /Helv 10 Tf 72 730 Td (Application) Tj ET".to_vec()));
        d.objects.insert(page, Object::Dictionary(dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => rect([0., 0., 612., 792.]),
            "Resources" => dictionary! { "Font" => dictionary! { "Helv" => helv } }, "Contents" => content,
            "Annots" => vec![name, city, agree, subscribe, choice].into_iter().map(Object::Reference).collect::<Vec<_>>() }));
        d.objects.insert(pages, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1 }));
        let acro_form = d.add_object(dictionary! { "Fields" => vec![name, address, agree, subscribe, choice].into_iter().map(Object::Reference).collect::<Vec<_>>(),
            "DA" => Object::string_literal("/Helv 0 Tf 0 g"), "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } } });
        let catalog = d.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages, "AcroForm" => acro_form });
        d.trailer.set("Root", catalog);
        extra(&mut d, acro_form, city);
        d.save(path).unwrap();
    }

    fn identity_scene() -> PdfScene {
        PdfScene { pages: vec![ScenePage { source_index: Some(0), width: 612., height: 792., rotation: 0, crop: None, source_rotation: None, source_box: None, objects: vec![] }] }
    }

    fn fill(input: &Path, scene: PdfScene, values: serde_json::Value) -> crate::kit::contracts::JobOutcome {
        let sections = serde_json::from_value(serde_json::json!({ "formValues": values })).unwrap();
        export(&ExportRequest { paths: vec![input.to_path_buf()], scene, output_location: OutputLocation::AlongsideInput, sections }, input.to_path_buf())
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
    fn unchecking_and_chaining_with_overlays_keeps_both_changes() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("form.pdf");
        form_fixture(&input, |d, _, _| {
            let agree = d.objects.iter().find(|(_, object)| object.as_dict().is_ok_and(|dict| dict.get(b"T").ok().and_then(|v| v.as_str().ok()) == Some(b"agree"))).map(|(id, _)| *id).unwrap();
            let dict = d.get_dictionary_mut(agree).unwrap(); dict.set("V", "Agreed"); dict.set("AS", "Agreed");
        });
        let mut scene = identity_scene();
        let mut note = super::super::SceneObject { text: "Reviewed".into(), font_size: 14., ..text_object(&inspect_form(&input).unwrap().fields[0], "", (100., 20.)) };
        note.rect = Rect { x: 300., y: 40., width: 200., height: 30. };
        scene.pages[0].objects.push(note);
        let outcome = fill(&input, scene, serde_json::json!([{ "name": "agree", "value": false }]));
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        assert_eq!(outcome.output_paths(), [temp.0.join("form-edited-1.pdf")]);
        let doc = Document::load(&outcome.output_paths()[0]).unwrap();
        let agree = field(&doc, b"agree");
        assert_eq!((agree.get(b"V").unwrap().as_name().unwrap(), agree.get(b"AS").unwrap().as_name().unwrap()), (&b"Off"[..], &b"Off"[..]));
        let page = doc.get_pages()[&1];
        assert_eq!(doc.get_dictionary(page).unwrap().get(b"Annots").unwrap().as_array().unwrap().len(), 5);
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
        form_fixture(&scripted, |d, _, city| { d.get_dictionary_mut(city).unwrap().set("AA", dictionary! { "K" => dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("AFNumber_Keystroke(2)") } }); });
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
            (&form, serde_json::json!([{ "name": "choice", "value": true }]), "This PDF has no fillable field named “choice”."),
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
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 4);
    }
}
