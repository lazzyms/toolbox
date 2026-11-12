use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use serde::Deserialize;
use crate::kit::contracts::ToolError;
use super::{effective_resource_entries, err, forms, next_resource_name, resolve, scene_resources};

const HIDDEN: i64 = 1 << 1;
const NO_VIEW: i64 = 1 << 5;
pub(super) const XFA_UNSUPPORTED: &str = "This form can't be flattened here. It was built with XFA, an older Adobe form format that Toolbox doesn't support.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlattenChoice { Forms, Annotations, Both }

impl FlattenChoice {
    fn forms(self) -> bool { matches!(self, Self::Forms | Self::Both) }
    fn annotations(self) -> bool { matches!(self, Self::Annotations | Self::Both) }
}

pub(super) struct FlattenPlan(FlattenChoice);

fn subtype(dict: &Dictionary) -> Option<&[u8]> { dict.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) }

/// Form widgets, or comments and markup: every annotation except links, which stay interactive.
fn targeted(dict: &Dictionary, choice: FlattenChoice) -> bool {
    match subtype(dict) {
        Some(b"Widget") => choice.forms(),
        Some(b"Link") => false,
        _ => choice.annotations(),
    }
}

fn annotations(doc: &Document, page: ObjectId) -> Result<Vec<Object>, String> {
    let Ok(annots) = doc.get_dictionary(page).map_err(err)?.get(b"Annots") else { return Ok(Vec::new()) };
    Ok(resolve(doc, annots)?.as_array().map_err(err)?.clone())
}

fn annotation<'a>(doc: &'a Document, entry: &'a Object) -> Option<&'a Dictionary> {
    match entry { Object::Reference(id) => doc.get_dictionary(*id).ok(), Object::Dictionary(dict) => Some(dict), _ => None }
}

/// Counts what the choice would flatten. Pop-up notes belong to their comment, so they are not counted.
pub(super) fn count(doc: &Document, choice: FlattenChoice) -> Result<usize, String> {
    let mut total = 0;
    for page in doc.get_pages().into_values() {
        total += annotations(doc, page)?.iter().filter_map(|entry| annotation(doc, entry))
            .filter(|dict| targeted(dict, choice) && subtype(dict) != Some(b"Popup")).count();
    }
    Ok(total)
}

pub(super) fn preflight(doc: &Document, choice: FlattenChoice) -> Result<FlattenPlan, ToolError> {
    if choice.forms() {
        let catalog = doc.catalog().map_err(|error| ToolError::processing(err(error)))?;
        let xfa = catalog.get(b"AcroForm").ok().and_then(|form| resolve(doc, form).ok()).and_then(|form| form.as_dict().ok()).is_some_and(|form| form.has(b"XFA"));
        if xfa { return Err(ToolError::invalid_input(XFA_UNSUPPORTED)); }
    }
    let fields = choice.forms() && count(doc, FlattenChoice::Forms).map_err(ToolError::processing)? > 0;
    let comments = choice.annotations() && count(doc, FlattenChoice::Annotations).map_err(ToolError::processing)? > 0;
    if !fields && !comments {
        return Err(ToolError::invalid_input(match choice {
            FlattenChoice::Forms => "This PDF has no form fields to flatten.",
            FlattenChoice::Annotations => "This PDF has no comments or markup to flatten.",
            FlattenChoice::Both => "This PDF has no form fields or comments to flatten.",
        }));
    }
    Ok(FlattenPlan(choice))
}

fn numbers(doc: &Document, value: &Object) -> Option<Vec<f32>> {
    resolve(doc, value).ok()?.as_array().ok()?.iter().map(|item| resolve(doc, item).ok().and_then(|item| super::super::metadata::number(item).ok())).collect()
}

/// The visible normal appearance of an annotation and the matrix that maps it onto the annotation's rectangle.
fn drawing(doc: &Document, dict: &Dictionary) -> Option<(ObjectId, [f32; 6])> {
    let flags = dict.get(b"F").ok().and_then(|value| resolve(doc, value).ok()).and_then(|value| value.as_i64().ok()).unwrap_or(0);
    if flags & (HIDDEN | NO_VIEW) != 0 { return None; }
    let normal = resolve(doc, dict.get(b"AP").ok()?).ok()?.as_dict().ok()?.get(b"N").ok()?;
    let id = match normal {
        Object::Reference(id) if doc.get_object(*id).ok()?.as_stream().is_ok() => *id,
        _ => resolve(doc, normal).ok()?.as_dict().ok()?.get(dict.get(b"AS").ok()?.as_name().ok()?).ok()?.as_reference().ok()?,
    };
    let stream = doc.get_object(id).ok()?.as_stream().ok()?;
    let rect = numbers(doc, dict.get(b"Rect").ok()?)?;
    let bbox = numbers(doc, stream.dict.get(b"BBox").ok()?)?;
    let m = stream.dict.get(b"Matrix").ok().and_then(|matrix| numbers(doc, matrix)).unwrap_or_else(|| vec![1., 0., 0., 1., 0., 0.]);
    if rect.len() != 4 || bbox.len() != 4 || m.len() != 6 { return None; }
    let corners = [(bbox[0], bbox[1]), (bbox[2], bbox[3]), (bbox[0], bbox[3]), (bbox[2], bbox[1])].map(|(x, y)| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]));
    let (x0, x1) = corners.iter().fold((f32::MAX, f32::MIN), |(a, b), (x, _)| (a.min(*x), b.max(*x)));
    let (y0, y1) = corners.iter().fold((f32::MAX, f32::MIN), |(a, b), (_, y)| (a.min(*y), b.max(*y)));
    let (rx0, ry0, rx1, ry1) = (rect[0].min(rect[2]), rect[1].min(rect[3]), rect[0].max(rect[2]), rect[1].max(rect[3]));
    if x1 - x0 <= 0. || y1 - y0 <= 0. || rx1 - rx0 <= 0. || ry1 - ry0 <= 0. { return None; }
    let (sx, sy) = ((rx1 - rx0) / (x1 - x0), (ry1 - ry0) / (y1 - y0));
    Some((id, [sx, 0., 0., sy, rx0 - sx * x0, ry0 - sy * y0]))
}

fn flatten_page(doc: &mut Document, page: ObjectId, choice: FlattenChoice) -> Result<(), String> {
    let entries = annotations(doc, page)?;
    let (mut kept, mut drawn) = (Vec::new(), Vec::new());
    for entry in &entries {
        match annotation(doc, entry) {
            Some(dict) if targeted(dict, choice) => drawn.extend(drawing(doc, dict)),
            _ => kept.push(entry.clone()),
        }
    }
    if kept.len() == entries.len() { return Ok(()); }
    if !drawn.is_empty() {
        let mut names = effective_resource_entries(doc, page, b"XObject")?;
        let mut xobjects = Dictionary::new();
        let mut content = String::from("Q\n");
        for (id, [a, b, c, d, e, f]) in drawn {
            let name = next_resource_name(&names, "XObject", "ToolboxFlat")?;
            names.set(name.as_str(), Object::Null);
            xobjects.set(name.as_str(), Object::Reference(id));
            content.push_str(&format!("q {a} {b} {c} {d} {e} {f} cm /{name} Do Q\n"));
            let form = doc.get_object_mut(id).and_then(Object::as_stream_mut).map_err(err)?;
            form.dict.set("Type", "XObject");
            form.dict.set("Subtype", "Form");
        }
        let resources = scene_resources(doc, page, xobjects, Dictionary::new(), Dictionary::new())?;
        // The page's own content is wrapped in q/Q so a graphics state it leaves open can't move the appearances.
        let open = doc.add_object(Stream::new(dictionary! {}, b"q\n".to_vec()));
        let close = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let original = match doc.get_dictionary(page).map_err(err)?.get(b"Contents") {
            Ok(contents) => match resolve(doc, contents)? { Object::Array(items) => items.clone(), _ => vec![contents.clone()] },
            Err(_) => Vec::new(),
        };
        let dict = doc.get_dictionary_mut(page).map_err(err)?;
        dict.set("Resources", resources);
        dict.set("Contents", [vec![Object::Reference(open)], original, vec![Object::Reference(close)]].concat());
    }
    let dict = doc.get_dictionary_mut(page).map_err(err)?;
    if kept.is_empty() { dict.remove(b"Annots"); } else { dict.set("Annots", kept); }
    Ok(())
}

pub(super) fn apply(doc: &mut Document, FlattenPlan(choice): FlattenPlan) -> Result<(), String> {
    if choice.forms() { forms::complete_appearances(doc)?; }
    for page in doc.get_pages().into_values() { flatten_page(doc, page, choice)?; }
    if choice.forms() { doc.catalog_mut().map_err(err)?.remove(b"AcroForm"); }
    doc.prune_objects();
    Ok(())
}
