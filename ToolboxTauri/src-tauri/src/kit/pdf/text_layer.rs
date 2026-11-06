//! Finds the text a page already has and, when asked, takes away an earlier invisible
//! OCR layer. Text is anything a text-showing operator draws, on the page or in a form
//! it draws. Text drawn in rendering mode 3 is invisible, the way OCR layers are made;
//! text in any other mode is visible.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

/// Forms drawn inside forms are followed this deep, which no real page reaches.
const MAX_FORM_DEPTH: usize = 16;
/// A content stream larger than this once decompressed is not read.
const MAX_CONTENT_BYTES: usize = 64 * 1024 * 1024;
const INVISIBLE_MODE: i64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageText {
    /// Nothing on the page draws text.
    None,
    /// Every piece of text is drawn invisibly, as an earlier OCR run leaves it.
    Invisible,
    /// Some text is drawn visibly. One visible word is enough.
    Visible,
}

/// The page's resources as the page sees them, inherited or its own.
pub(crate) fn page_resources(document: &Document, page_id: ObjectId) -> Result<Dictionary, String> {
    match super::inherited(document, page_id, b"Resources")? {
        Some(Object::Dictionary(resources)) => Ok(resources),
        Some(_) => Err("PDF page resources are invalid.".to_string()),
        None => Ok(Dictionary::new()),
    }
}

pub(crate) fn page_text(document: &Document, page_id: ObjectId) -> Result<PageText, String> {
    let resources = page_resources(document, page_id)?;
    let mut state = TextState::default();
    let mut found = Found::default();
    for id in document.get_page_contents(page_id) {
        let operations = operations(document, id, false)?;
        find_text(document, &operations, &resources, &mut state, &mut found, &mut Vec::new())?;
        if found.visible { return Ok(PageText::Visible); }
    }
    Ok(if found.invisible { PageText::Invisible } else { PageText::None })
}

/// Takes the invisible text off a page whose only text is invisible. Only the
/// operators that draw text in mode 3 go; everything else is written back in order.
/// Streams and forms that change are written as new objects, and the page gets its
/// own copy of its resources, so pages that share them keep their text.
pub(crate) fn remove_invisible_text(document: &mut Document, page_id: ObjectId) -> Result<(), String> {
    if page_text(document, page_id)? != PageText::Invisible {
        return Err("only a page whose text is all invisible can lose its text layer.".to_string());
    }
    let resources = page_resources(document, page_id)?;
    let mut state = TextState::default();
    let mut forms = Vec::new();
    let mut contents = Vec::new();
    let mut changed = false;
    for id in document.get_page_contents(page_id) {
        let original = operations(document, id, true)?;
        let stripped = strip(document, original, &resources, &mut state, &mut vec![id])?;
        forms.extend(stripped.forms);
        contents.push(match stripped.operations {
            Some(operations) => {
                changed = true;
                document.add_object(content_stream(Dictionary::new(), &operations)?)
            }
            None => id,
        });
    }
    let page = document.get_dictionary_mut(page_id).map_err(|error| error.to_string())?;
    if changed { page.set("Contents", contents.into_iter().map(Object::Reference).collect::<Vec<_>>()); }
    if !forms.is_empty() {
        let resources = with_forms(document, resources, forms)?;
        document.get_dictionary_mut(page_id).map_err(|error| error.to_string())?.set("Resources", resources);
    }
    Ok(())
}

/// Rendering mode is part of the graphics state, so q saves it and Q restores it.
#[derive(Debug, Clone, Default)]
struct TextState { mode: i64, saved: Vec<i64> }

impl TextState {
    fn apply(&mut self, operation: &Operation) {
        match operation.operator.as_str() {
            "q" => self.saved.push(self.mode),
            "Q" => if let Some(mode) = self.saved.pop() { self.mode = mode },
            "Tr" => if let Some(mode) = operation.operands.first().and_then(|mode| match mode {
                Object::Integer(mode) => Some(*mode),
                Object::Real(mode) => Some(*mode as i64),
                _ => None,
            }) { self.mode = mode },
            _ => {}
        }
    }

    /// A form starts in the state it was drawn in, and anything it changes is put back after it.
    fn for_form(&self) -> Self { Self { mode: self.mode, saved: Vec::new() } }
}

#[derive(Debug, Default)]
struct Found { invisible: bool, visible: bool }

fn shows_text(operation: &Operation) -> bool {
    let text = |object: &Object| matches!(object, Object::String(bytes, _) if !bytes.is_empty());
    match operation.operator.as_str() {
        "Tj" | "'" | "\"" => operation.operands.last().is_some_and(text),
        "TJ" => operation.operands.first().and_then(|items| items.as_array().ok()).is_some_and(|items| items.iter().any(text)),
        _ => false,
    }
}

fn find_text(document: &Document, operations: &[Operation], resources: &Dictionary, state: &mut TextState, found: &mut Found, drawing: &mut Vec<ObjectId>) -> Result<(), String> {
    for operation in operations {
        state.apply(operation);
        if shows_text(operation) {
            if state.mode == INVISIBLE_MODE { found.invisible = true } else { found.visible = true; return Ok(()); }
        }
        if let Some((id, form)) = drawn_form(document, operation, resources, drawing)? {
            let form_resources = form_resources(document, &form, resources)?;
            drawing.push(id);
            find_text(document, &operations_of(&form, false)?, &form_resources, &mut state.for_form(), found, drawing)?;
            drawing.pop();
            if found.visible { return Ok(()); }
        }
    }
    Ok(())
}

struct Stripped {
    /// The stream's operators without its invisible text, or None when it had none.
    operations: Option<Vec<Operation>>,
    /// Forms this stream draws that were rewritten, by resource name.
    forms: Vec<(Vec<u8>, ObjectId)>,
}

fn strip(document: &mut Document, operations: Vec<Operation>, resources: &Dictionary, state: &mut TextState, drawing: &mut Vec<ObjectId>) -> Result<Stripped, String> {
    let mut kept = Vec::with_capacity(operations.len());
    let mut forms = Vec::new();
    let mut dropped = false;
    for operation in operations {
        state.apply(&operation);
        if shows_text(&operation) && state.mode == INVISIBLE_MODE {
            dropped = true;
            continue;
        }
        if let Some((id, form)) = drawn_form(document, &operation, resources, drawing)? {
            if let Some(new_form) = strip_form(document, id, form, resources, &state.for_form(), drawing)? {
                let name = operation.operands[0].as_name().map_err(|error| error.to_string())?.to_vec();
                forms.push((name, new_form));
            }
        }
        kept.push(operation);
    }
    if dropped && kept.iter().any(|operation| operation.operator == "BI") {
        return Err("a content stream with an inline image can't be rewritten safely.".to_string());
    }
    Ok(Stripped { operations: dropped.then_some(kept), forms })
}

/// Writes a copy of the form without its invisible text, or returns None when it has none.
fn strip_form(document: &mut Document, id: ObjectId, form: Stream, resources: &Dictionary, state: &TextState, drawing: &mut Vec<ObjectId>) -> Result<Option<ObjectId>, String> {
    let form_resources = form_resources(document, &form, resources)?;
    drawing.push(id);
    let stripped = strip(document, operations_of(&form, true)?, &form_resources, &mut state.clone(), drawing);
    drawing.pop();
    let stripped = stripped?;
    if stripped.operations.is_none() && stripped.forms.is_empty() { return Ok(None); }
    let mut dictionary = form.dict.clone();
    if !stripped.forms.is_empty() { dictionary.set("Resources", with_forms(document, form_resources, stripped.forms)?); }
    let copy = match stripped.operations {
        Some(operations) => {
            dictionary.remove(b"Filter");
            dictionary.remove(b"DecodeParms");
            content_stream(dictionary, &operations)?
        }
        None => Stream { dict: dictionary, ..form },
    };
    Ok(Some(document.add_object(copy)))
}

/// The form a Do operator draws, unless it is already being drawn or nested too deep.
fn drawn_form(document: &Document, operation: &Operation, resources: &Dictionary, drawing: &[ObjectId]) -> Result<Option<(ObjectId, Stream)>, String> {
    if operation.operator != "Do" { return Ok(None); }
    let Some(Ok(name)) = operation.operands.first().map(Object::as_name) else { return Ok(None) };
    let Ok(xobjects) = resources.get(b"XObject") else { return Ok(None) };
    let Ok(xobjects) = super::resolve(document, xobjects)?.as_dict() else { return Ok(None) };
    let Ok(Object::Reference(id)) = xobjects.get(name) else { return Ok(None) };
    let Ok(stream) = document.get_object(*id).and_then(Object::as_stream) else { return Ok(None) };
    if !stream.dict.get(b"Subtype").and_then(Object::as_name).is_ok_and(|subtype| subtype == b"Form") { return Ok(None); }
    if drawing.contains(id) || drawing.len() > MAX_FORM_DEPTH { return Ok(None); }
    Ok(Some((*id, stream.clone())))
}

/// A form uses its own resources, or the resources of what draws it when it has none.
fn form_resources(document: &Document, form: &Stream, inherited: &Dictionary) -> Result<Dictionary, String> {
    match form.dict.get(b"Resources") {
        Ok(resources) => super::resolve(document, resources)?.as_dict().cloned().map_err(|_| "PDF form resources are invalid.".to_string()),
        Err(_) => Ok(inherited.clone()),
    }
}

fn with_forms(document: &Document, mut resources: Dictionary, forms: Vec<(Vec<u8>, ObjectId)>) -> Result<Dictionary, String> {
    let mut xobjects = match resources.get(b"XObject") {
        Ok(xobjects) => super::resolve(document, xobjects)?.as_dict().cloned().map_err(|_| "PDF page resources are invalid.".to_string())?,
        Err(_) => Dictionary::new(),
    };
    for (name, id) in forms { xobjects.set(name, Object::Reference(id)); }
    resources.set("XObject", xobjects);
    Ok(resources)
}

fn operations(document: &Document, id: ObjectId, strict: bool) -> Result<Vec<Operation>, String> {
    let stream = document.get_object(id).and_then(Object::as_stream).map_err(|error| format!("its content could not be read ({error})."))?;
    operations_of(stream, strict)
}

/// Reading for a check takes what can be read. Reading to rewrite takes the whole
/// stream or nothing, so nothing that couldn't be read is left out of the copy.
fn operations_of(stream: &Stream, strict: bool) -> Result<Vec<Operation>, String> {
    let bytes = stream.get_plain_content_with_limit(MAX_CONTENT_BYTES).map_err(|error| format!("its content could not be read ({error})."))?;
    let content = if strict { Content::decode_strict(&bytes) } else { Content::decode(&bytes) };
    content.map(|content| content.operations).map_err(|error| format!("its content could not be read ({error})."))
}

fn content_stream(dictionary: Dictionary, operations: &[Operation]) -> Result<Stream, String> {
    let bytes = Content { operations: operations.to_vec() }.encode().map_err(|error| error.to_string())?;
    let mut stream = Stream::new(dictionary, bytes);
    stream.compress().map_err(|error| format!("could not compress the page content ({error})."))?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::{page_text, remove_invisible_text, PageText};
    use lopdf::{dictionary, Document, Object, ObjectId, Stream};

    /// One page per content string. `/Form` draws a form whose content is `form`.
    fn document(pages: &[&str], form: Option<&str>) -> (Document, Vec<ObjectId>) {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let font = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let mut resources = dictionary! { "Font" => dictionary! { "F1" => font } };
        if let Some(form) = form {
            let form = document.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 612.into(), 792.into()] }, form.as_bytes().to_vec()));
            resources.set("XObject", dictionary! { "Form" => form });
        }
        let resources = document.add_object(resources);
        let mut ids = Vec::new();
        for content in pages {
            let content = document.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
            ids.push(document.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Resources" => resources, "Contents" => content }));
        }
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(), "Count" => ids.len() as i64 }.into());
        let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog);
        (document, ids)
    }

    #[test]
    fn pages_are_classed_by_the_text_they_draw_and_how_they_draw_it() {
        let cases = [
            ("q 612 0 0 792 0 0 cm 0 0 1 1 re f Q", PageText::None),
            ("BT /F1 12 Tf () Tj [-200] TJ ET", PageText::None),
            ("BT 3 Tr /F1 12 Tf (hidden) Tj ET", PageText::Invisible),
            ("BT /F1 12 Tf 3 Tr [(hid) -20 (den)] TJ ET", PageText::Invisible),
            ("BT /F1 12 Tf (shown) Tj ET", PageText::Visible),
            ("BT 3 Tr /F1 12 Tf (hidden) Tj 0 Tr (shown) ' ET", PageText::Visible),
            ("q 3 Tr Q BT /F1 12 Tf (shown) Tj ET", PageText::Visible),
            ("BT 7 Tr /F1 12 Tf (clipped) Tj ET", PageText::Visible),
        ];
        for (content, expected) in cases {
            let (document, pages) = document(&[content], None);
            assert_eq!(page_text(&document, pages[0]).unwrap(), expected, "{content}");
        }
    }

    #[test]
    fn text_in_a_drawn_form_counts_and_the_form_starts_in_the_mode_it_was_drawn_in() {
        let (document, pages) = document(&["3 Tr /Form Do", "/Form Do", "q 1 0 0 1 0 0 cm Q"], Some("BT /F1 12 Tf (inside) Tj ET"));
        let classes = pages.iter().map(|page| page_text(&document, *page).unwrap()).collect::<Vec<_>>();
        assert_eq!(classes, [PageText::Invisible, PageText::Visible, PageText::None]);
    }

    #[test]
    fn a_form_that_draws_itself_is_read_once() {
        let mut document = Document::with_version("1.7");
        let form = document.new_object_id();
        let resources = dictionary! { "XObject" => dictionary! { "Loop" => form } };
        document.objects.insert(form, Stream::new(dictionary! { "Subtype" => "Form", "Resources" => resources.clone() }, b"/Loop Do".to_vec()).into());
        let pages_id = document.new_object_id();
        let content = document.add_object(Stream::new(dictionary! {}, b"/Loop Do".to_vec()));
        let page = document.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "Resources" => resources, "Contents" => content });
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }.into());
        assert_eq!(page_text(&document, page).unwrap(), PageText::None);
    }

    #[test]
    fn removing_the_invisible_layer_keeps_every_other_operator_and_leaves_shared_objects_alone() {
        let (mut document, pages) = document(&["q 1 0 0 1 0 0 cm 0 0 5 5 re f Q BT 3 Tr /F1 12 Tf 10 10 Td (old) Tj ET /Form Do", "BT 3 Tr /F1 12 Tf (other) Tj ET /Form Do"], Some("BT 3 Tr /F1 12 Tf (form) Tj ET"));
        let shared_streams = document.get_page_contents(pages[1]);
        let shared_resources = document.get_dictionary(pages[1]).unwrap().get(b"Resources").unwrap().clone();

        remove_invisible_text(&mut document, pages[0]).unwrap();

        assert_eq!(page_text(&document, pages[0]).unwrap(), PageText::None);
        let content = String::from_utf8(document.get_page_content(pages[0])).unwrap();
        for kept in ["q", "cm", "re", "f", "Q", "BT", "3 Tr", "Tf", "Td", "ET", "/Form Do"] { assert!(content.contains(kept), "{kept} is kept: {content}"); }
        assert!(!content.contains("old"), "{content}");
        assert_eq!(page_text(&document, pages[1]).unwrap(), PageText::Invisible, "the page that shares the form and resources keeps its layer");
        assert_eq!(document.get_page_contents(pages[1]), shared_streams);
        assert_eq!(document.get_dictionary(pages[1]).unwrap().get(b"Resources").unwrap(), &shared_resources);
    }

    #[test]
    fn a_page_with_visible_text_is_never_rewritten() {
        let (mut document, pages) = document(&["BT 3 Tr /F1 12 Tf (hidden) Tj 0 Tr (shown) Tj ET"], None);
        let before = document.get_page_content(pages[0]);
        assert!(remove_invisible_text(&mut document, pages[0]).is_err());
        assert_eq!(document.get_page_content(pages[0]), before);
    }
}
