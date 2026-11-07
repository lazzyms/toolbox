//! Reads a PowerPoint 97-2003 (PPT) presentation into the shared slide model: each text shape becomes a box at its place on the
//! slide. Pictures and embedded objects are counted as left out, and macros and objects are never run.

use std::collections::{HashMap, HashSet};

use crate::kit::contracts::ToolError;
use crate::kit::pdf::compound::{i16_at, i32_at, u16_at, u32_at, Compound};
use crate::kit::pdf::layout::{plain_text, Align, Block, Float, Frame, Omitted, Paragraph, Place, Position, Span, Style};
use crate::kit::pdf::presentation::{slide_side, Deck, Include, Slide, DEFAULT_SLIDE};

pub const DAMAGED: &str = "This PowerPoint 97-2003 presentation is damaged or incomplete, so no PDF was made.";
pub const TOO_OLD: &str = "This presentation was saved by PowerPoint 95 or earlier, which Convert to PDF doesn't read. Save it as a newer presentation, then convert that.";

const DOCUMENT_STREAM: &str = "/PowerPoint Document";
const CURRENT_USER: &str = "/Current User";
const MASTER_UNITS_PER_POINT: f32 = 8.;
const POWERPOINT_97: u16 = 0x03F4;
const MAX_DEPTH: usize = 32;
const MAX_EDITS: usize = 4096;
const TITLE_SIZE: f32 = 28.;
const BODY_SIZE: f32 = 18.;
const NOTES_SIZE: f32 = 12.;
const MARGIN: f32 = 36.;

const DOCUMENT: u16 = 0x03E8;
const DOCUMENT_ATOM: u16 = 0x03E9;
const SLIDE: u16 = 0x03EE;
const SLIDE_ATOM: u16 = 0x03EF;
const NOTES: u16 = 0x03F0;
const SLIDE_PERSIST: u16 = 0x03F3;
const SLIDE_SHOW_INFO: u16 = 0x03F9;
const DRAWING: u16 = 0x040C;
const EXTERNAL_OBJECT: u16 = 0x0BC1;
const OUTLINE_TEXT_REF: u16 = 0x0F9E;
const TEXT_HEADER: u16 = 0x0F9F;
const TEXT_CHARS: u16 = 0x0FA0;
const TEXT_BYTES: u16 = 0x0FA8;
const SLIDE_LIST: u16 = 0x0FF0;
const USER_EDIT: u16 = 0x0FF5;
const CURRENT_USER_ATOM: u16 = 0x0FF6;
const PERSIST_DIRECTORY: u16 = 0x1772;
const SHAPE: u16 = 0xF004;
const SHAPE_ATOM: u16 = 0xF00A;
const TEXTBOX: u16 = 0xF00D;
const ANCHOR: u16 = 0xF010;
const PICTURE_FRAME: u16 = 75;

fn damaged() -> ToolError { ToolError::invalid_input(DAMAGED) }

/// One record: its type, instance and body, read from a stream.
#[derive(Clone, Copy)]
struct Record<'a> { kind: u16, instance: u16, container: bool, body: &'a [u8] }

impl<'a> Record<'a> {
    fn at(stream: &'a [u8], offset: usize) -> Option<Self> {
        let header = u16_at(stream, offset)?;
        let kind = u16_at(stream, offset + 2)?;
        let length = u32_at(stream, offset + 4)? as usize;
        let body = stream.get(offset + 8..(offset + 8).checked_add(length)?)?;
        Some(Self { kind, instance: header >> 4, container: header & 0x000F == 0x000F, body })
    }

    /// The records directly inside this container; a damaged tail ends the list.
    fn children(&self) -> Vec<Record<'a>> {
        let mut children = Vec::new();
        if !self.container { return children; }
        let mut offset = 0;
        while let Some(child) = Record::at(self.body, offset) {
            offset += 8 + child.body.len();
            children.push(child);
        }
        children
    }

    fn child(&self, kind: u16) -> Option<Record<'a>> { self.children().into_iter().find(|child| child.kind == kind) }

    /// Every record below this one with the given type, not looking inside records of type `stop`.
    fn descendants(&self, kind: u16, stop: u16, found: &mut Vec<Record<'a>>, depth: usize) {
        if depth > MAX_DEPTH { return; }
        for child in self.children() {
            if child.kind == kind { found.push(child); }
            if child.kind != stop && child.container { child.descendants(kind, stop, found, depth + 1); }
        }
    }
}

/// A run of text and the kind of placeholder it fills (0 title, 1 body, 2 notes, 6 centred title, ...).
#[derive(Debug, Clone)]
struct Text { kind: u32, text: String }

impl Text {
    fn title(&self) -> bool { matches!(self.kind, 0 | 6) }
}

/// The texts in a list of records, each taking the kind of the text header before it.
fn texts(records: &[Record]) -> Vec<Text> {
    let mut texts = Vec::new();
    let mut kind = 1;
    for record in records {
        match record.kind {
            TEXT_HEADER => kind = u32_at(record.body, 0).unwrap_or(1),
            TEXT_CHARS => texts.push(Text { kind, text: String::from_utf16_lossy(&record.body.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect::<Vec<_>>()) }),
            TEXT_BYTES => texts.push(Text { kind, text: record.body.iter().map(|byte| char::from(*byte)).collect() }),
            _ => {}
        }
    }
    texts
}

fn blocks(text: &Text, size: f32) -> Vec<Block> {
    let title = text.title();
    let style = Style { bold: title, half_points: (size * 2.) as u16, ..Style::default() };
    text.text.split('\r').map(|line| {
        let line = line.chars().map(|character| match character { '\u{0B}' => '\n', '\t' => ' ', other => other }).filter(|character| *character == '\n' || !character.is_control()).collect::<String>();
        Block::Formatted(Paragraph {
            spans: if line.is_empty() { Vec::new() } else { vec![Span { text: line, style, link: None }] },
            align: if text.kind == 6 { Align::Center } else { Align::Left },
            indent: 0.,
            right_indent: 0.,
            before: 0.,
            after: size * 0.3,
            size,
            leading: 1.2,
        })
    }).collect()
}

fn float(rect: (f32, f32, f32, f32), blocks: Vec<Block>) -> Block {
    let (x, y, width, height) = rect;
    Block::Float(Float {
        x: Position { frame: Frame::Page, place: Place::Offset(x) },
        y: Position { frame: Frame::Page, place: Place::Offset(y) },
        width,
        height,
        fill: None,
        line: None,
        picture: None,
        blocks,
        wrap: false,
    })
}

/// A shape's place on the slide, in points, from its client anchor.
fn anchor(record: &Record) -> Option<(f32, f32, f32, f32)> {
    let (top, left, right, bottom) = match record.body.len() {
        8 => (i32::from(i16_at(record.body, 0)?), i32::from(i16_at(record.body, 2)?), i32::from(i16_at(record.body, 4)?), i32::from(i16_at(record.body, 6)?)),
        16 => (i32_at(record.body, 0)?, i32_at(record.body, 4)?, i32_at(record.body, 8)?, i32_at(record.body, 12)?),
        _ => return None,
    };
    let unit = |value: i32| value as f32 / MASTER_UNITS_PER_POINT;
    (right > left && bottom > top).then(|| (unit(left), unit(top), unit(right - left), unit(bottom - top)))
}

struct Reader<'a> { stream: &'a [u8], persist: HashMap<u32, u32> }

impl<'a> Reader<'a> {
    fn object(&self, id: u32, kind: u16) -> Option<Record<'a>> {
        Record::at(self.stream, *self.persist.get(&id)? as usize).filter(|record| record.kind == kind)
    }

    /// The text shapes in a slide's or notes page's drawing. Shapes that point at the slide list's texts mark them used.
    fn shapes(&self, container: &Record, outline: &[Text], used: &mut HashSet<usize>, size: f32, omitted: &mut Omitted) -> Vec<(Option<(f32, f32, f32, f32)>, Text, f32)> {
        let mut shapes = Vec::new();
        let Some(drawing) = container.child(DRAWING) else { return shapes };
        let mut found = Vec::new();
        drawing.descendants(SHAPE, u16::MAX, &mut found, 0);
        for shape in found {
            let children = shape.children();
            if children.iter().any(|child| child.kind == SHAPE_ATOM && child.instance == PICTURE_FRAME) { omitted.pictures += 1; }
            let mut objects = Vec::new();
            shape.descendants(EXTERNAL_OBJECT, SHAPE, &mut objects, 0);
            omitted.objects += objects.len();
            let rect = children.iter().find(|child| child.kind == ANCHOR).and_then(anchor);
            let Some(textbox) = children.iter().find(|child| child.kind == TEXTBOX) else { continue };
            let records = textbox.children();
            for record in records.iter().filter(|record| record.kind == OUTLINE_TEXT_REF) {
                let Some(index) = u32_at(record.body, 0).map(|index| index as usize) else { continue };
                if let Some(text) = outline.get(index) {
                    used.insert(index);
                    shapes.push((rect, text.clone(), if text.title() { TITLE_SIZE } else { size }));
                }
            }
            for text in texts(&records) { let size = if text.title() { TITLE_SIZE } else { size }; shapes.push((rect, text, size)); }
        }
        shapes
    }
}

/// The persist directory: each object's offset in the document stream, newest edit winning.
fn persist_directory(stream: &[u8], first_edit: u32) -> Option<(HashMap<u32, u32>, u32)> {
    let mut edits = Vec::new();
    let mut seen = HashSet::new();
    let mut offset = first_edit;
    loop {
        if !seen.insert(offset) || edits.len() > MAX_EDITS { return None; }
        let edit = Record::at(stream, offset as usize).filter(|record| record.kind == USER_EDIT && record.body.len() >= 0x1C)?;
        edits.push(edit);
        match u32_at(edit.body, 8)? {
            0 => break,
            previous => offset = previous,
        }
    }
    let document = u32_at(edits[0].body, 16)?;
    let mut persist = HashMap::new();
    for edit in edits.iter().rev() {
        let directory = Record::at(stream, u32_at(edit.body, 12)? as usize).filter(|record| record.kind == PERSIST_DIRECTORY)?;
        let mut cursor = 0;
        while cursor + 4 <= directory.body.len() {
            let packed = u32_at(directory.body, cursor)?;
            let (start, count) = (packed & 0x000F_FFFF, (packed >> 20) as usize);
            cursor += 4;
            for index in 0..count {
                persist.insert(start + index as u32, u32_at(directory.body, cursor)?);
                cursor += 4;
            }
        }
    }
    Some((persist, document))
}

/// Reads a PPT file. `budget` caps the bytes read from the compound file; going over returns `exceeded`.
/// Hidden slides and speaker notes are only read when `include` asks for them.
pub fn parse(bytes: &[u8], budget: u64, exceeded: ToolError, include: Include) -> Result<Deck, ToolError> {
    let mut file = Compound::open(bytes, budget, exceeded, DAMAGED)?;
    let Some(stream) = file.stream(DOCUMENT_STREAM)? else {
        return Err(if file.stream("/PP40")?.is_some() { ToolError::invalid_input(TOO_OLD) } else { damaged() });
    };
    let user = file.stream(CURRENT_USER)?.ok_or_else(damaged)?;
    if u16_at(&user, 2) != Some(CURRENT_USER_ATOM) { return Err(damaged()); }
    if u16_at(&user, 22).is_some_and(|version| version < POWERPOINT_97) { return Err(ToolError::invalid_input(TOO_OLD)); }
    let (title, author) = file.summary()?;

    let (persist, document_id) = u32_at(&user, 16).and_then(|offset| persist_directory(&stream, offset)).ok_or_else(damaged)?;
    let reader = Reader { stream: &stream, persist };
    let document = reader.object(document_id, DOCUMENT).filter(|record| record.container).ok_or_else(damaged)?;
    let size = document.child(DOCUMENT_ATOM)
        .and_then(|atom| Some((i32_at(atom.body, 0)?, i32_at(atom.body, 4)?)))
        .filter(|(width, height)| *width > 0 && *height > 0)
        .map_or(DEFAULT_SLIDE, |(width, height)| (slide_side(width as f32 / MASTER_UNITS_PER_POINT), slide_side(height as f32 / MASTER_UNITS_PER_POINT)));

    // Each slide in the list: its persist reference and the outline texts stored after it.
    let mut listed: Vec<(u32, Vec<Record>)> = Vec::new();
    if let Some(list) = document.children().into_iter().find(|record| record.kind == SLIDE_LIST && record.instance == 0) {
        for record in list.children() {
            if record.kind == SLIDE_PERSIST { listed.push((u32_at(record.body, 0).ok_or_else(damaged)?, Vec::new())); }
            else if let Some((_, records)) = listed.last_mut() { records.push(record); }
        }
    }

    let mut slides = Vec::new();
    let mut hidden_left_out = 0;
    for (index, (reference, records)) in listed.iter().enumerate() {
        let container = reader.object(*reference, SLIDE).filter(|record| record.container).ok_or_else(damaged)?;
        let hidden = container.child(SLIDE_SHOW_INFO).and_then(|info| u16_at(info.body, 10)).is_some_and(|flags| flags & 0x0004 != 0);
        if hidden && !include.hidden {
            hidden_left_out += 1;
            continue;
        }
        let outline = texts(records);
        let mut used = HashSet::new();
        let mut omitted = Omitted::default();
        let shapes = reader.shapes(&container, &outline, &mut used, BODY_SIZE, &mut omitted);
        let mut placed = Vec::new();
        let mut loose: Vec<(Text, f32)> = Vec::new();
        for (rect, text, size) in shapes {
            match rect {
                Some(rect) => placed.push(float(rect, blocks(&text, size))),
                None => loose.push((text, size)),
            }
        }
        loose.extend(outline.iter().enumerate().filter(|(index, _)| !used.contains(index)).map(|(_, text)| (text.clone(), if text.title() { TITLE_SIZE } else { BODY_SIZE })));
        if !loose.is_empty() {
            let flowing = loose.iter().flat_map(|(text, size)| blocks(text, *size)).collect::<Vec<_>>();
            placed.push(float((MARGIN, MARGIN, size.0 - 2. * MARGIN, size.1 - 2. * MARGIN), flowing));
        }
        let title = outline.iter().find(|text| text.title()).map(|text| plain_text(&[Span { text: text.text.replace(['\r', '\u{0B}'], " "), style: Style::default(), link: None }])).filter(|title| !title.is_empty());

        let mut notes = Vec::new();
        if include.notes {
            let page = container.child(SLIDE_ATOM).and_then(|atom| u32_at(atom.body, 16)).filter(|id| *id != 0).and_then(|id| reader.object(id, NOTES));
            if let Some(page) = page {
                let mut ignored = Omitted::default();
                for (_, text, _) in reader.shapes(&page, &[], &mut HashSet::new(), NOTES_SIZE, &mut ignored).into_iter().filter(|(_, text, _)| text.kind == 2) {
                    notes.extend(blocks(&text, NOTES_SIZE));
                }
            }
        }
        slides.push(Slide { number: index + 1, size, title, shapes: placed, notes, approximate: 0, omitted });
    }

    Ok(Deck { slides, hidden_left_out, pictures: Vec::new(), families: Vec::new(), body_family: None, embedded: Vec::new(), title, author })
}

#[cfg(test)]
pub(crate) mod fixtures {
    use crate::kit::office::tests::cfb_with_streams;
    use crate::kit::pdf::compound::fixtures::summary;

    pub(crate) struct Page<'a> { pub(crate) title: &'a str, pub(crate) body: &'a str, pub(crate) hidden: bool, pub(crate) notes: Option<&'a str>, pub(crate) picture: bool }

    fn record(kind: u16, instance: u16, container: bool, body: &[u8]) -> Vec<u8> {
        let mut record = ((instance << 4) | if container { 0x000F } else { 0 }).to_le_bytes().to_vec();
        record.extend(kind.to_le_bytes());
        record.extend((body.len() as u32).to_le_bytes());
        record.extend(body);
        record
    }

    fn atom(kind: u16, body: &[u8]) -> Vec<u8> { record(kind, 0, false, body) }

    fn container(kind: u16, children: &[Vec<u8>]) -> Vec<u8> { record(kind, 0, true, &children.concat()) }

    fn chars(text: &str) -> Vec<u8> { atom(0x0FA0, &text.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>()) }

    fn header(kind: u32) -> Vec<u8> { atom(0x0F9F, &kind.to_le_bytes()) }

    fn anchor(left: i16, top: i16, right: i16, bottom: i16) -> Vec<u8> { atom(0xF010, &[top, left, right, bottom].iter().flat_map(|value| value.to_le_bytes()).collect::<Vec<_>>()) }

    fn shape(kind: u16, children: Vec<Vec<u8>>) -> Vec<u8> {
        let mut all = vec![record(0xF00A, kind, false, &[0u8; 8])];
        all.extend(children);
        container(0xF004, &all)
    }

    fn drawing(shapes: Vec<Vec<u8>>) -> Vec<u8> { container(0x040C, &[container(0xF002, &[container(0xF003, &shapes)])]) }

    /// A PowerPoint 97-2003 file with one slide per page, 720 by 540 points. Each title is an outline text placed by its shape;
    /// each body is a text box stored in its own shape.
    pub(crate) fn deck(pages: &[Page]) -> Vec<u8> {
        let mut stream = Vec::new();
        let mut offsets = vec![0u32];
        let mut slide_list = Vec::new();
        let slide_id = |index: usize| (2 + index * 2) as u32;
        for (index, page) in pages.iter().enumerate() {
            let mut persist = vec![0u8; 20];
            persist[0..4].copy_from_slice(&slide_id(index).to_le_bytes());
            persist[12..16].copy_from_slice(&(256 + index as u32).to_le_bytes());
            slide_list.extend([atom(0x03F3, &persist), header(0), chars(page.title)]);
        }
        let mut size = vec![0u8; 40];
        size[0..4].copy_from_slice(&5760i32.to_le_bytes());
        size[4..8].copy_from_slice(&4320i32.to_le_bytes());
        let document = container(0x03E8, &[atom(0x03E9, &size), record(0x0FF0, 0, true, &slide_list.concat())]);
        stream.extend(document);
        for (index, page) in pages.iter().enumerate() {
            let mut slide_atom = vec![0u8; 24];
            if page.notes.is_some() { slide_atom[16..20].copy_from_slice(&(slide_id(index) + 1).to_le_bytes()); }
            let mut info = vec![0u8; 16];
            if page.hidden { info[10] = 0x04; }
            let mut shapes = vec![
                shape(1, vec![anchor(576, 288, 5184, 1152), container(0xF00D, &[atom(0x0F9E, &0u32.to_le_bytes())])]),
                shape(1, vec![anchor(576, 1440, 5184, 3888), container(0xF00D, &[header(1), chars(page.body)])]),
            ];
            if page.picture { shapes.push(shape(75, vec![anchor(4000, 3000, 5000, 4000)])); }
            offsets.push(stream.len() as u32);
            stream.extend(container(0x03EE, &[atom(0x03EF, &slide_atom), atom(0x03F9, &info), drawing(shapes)]));
            offsets.push(stream.len() as u32);
            if let Some(notes) = page.notes {
                stream.extend(container(0x03F0, &[atom(0x03F1, &[0u8; 8]), drawing(vec![shape(1, vec![anchor(0, 0, 100, 100), container(0xF00D, &[header(2), chars(notes)])])])]));
            }
        }
        let directory_at = stream.len() as u32;
        let mut directory = (1u32 | ((offsets.len() as u32) << 20)).to_le_bytes().to_vec();
        for offset in &offsets { directory.extend(offset.to_le_bytes()); }
        stream.extend(atom(0x1772, &directory));
        let edit_at = stream.len() as u32;
        let mut edit = vec![0u8; 0x1C];
        edit[12..16].copy_from_slice(&directory_at.to_le_bytes());
        edit[16..20].copy_from_slice(&1u32.to_le_bytes());
        stream.extend(atom(0x0FF5, &edit));

        let mut user = vec![0u8; 20];
        user[0..4].copy_from_slice(&0x14u32.to_le_bytes());
        user[4..8].copy_from_slice(&0xE391_C05Fu32.to_le_bytes());
        user[8..12].copy_from_slice(&edit_at.to_le_bytes());
        user[14..16].copy_from_slice(&0x03F4u16.to_le_bytes());
        user[16] = 3;
        cfb_with_streams(&[("/Current User", &atom(0x0FF6, &user)), ("/PowerPoint Document", &stream), ("/\u{5}SummaryInformation", &summary("Toolbox notes", "Ana"))])
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{deck, Page};
    use super::*;
    use crate::kit::office::tests::{cfb_with_streams, ppt_fixture};

    fn exceeded() -> ToolError { ToolError::invalid_input("too big") }

    fn page<'a>(title: &'a str, body: &'a str, hidden: bool) -> Page<'a> { Page { title, body, hidden, notes: None, picture: false } }

    fn shape_texts(slide: &Slide) -> Vec<String> {
        slide.shapes.iter().map(|shape| {
            let Block::Float(float) = shape else { panic!("slides hold floats") };
            float.blocks.iter().map(|block| match block { Block::Formatted(paragraph) => plain_text(&paragraph.spans), other => format!("{other:?}") }).collect::<Vec<_>>().join("/")
        }).collect()
    }

    #[test]
    fn slides_keep_their_text_places_and_titles() {
        let mut first = page("Plan", "Caf\u{e9} \u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}\rSecond line", false);
        first.picture = true;
        let parsed = parse(&deck(&[first, page("Next", "More", false)]), 1 << 20, exceeded(), Include::default()).unwrap();
        assert_eq!(parsed.slides.len(), 2);
        let slide = &parsed.slides[0];
        assert_eq!((slide.number, slide.size, slide.title.as_deref()), (1, (720., 540.), Some("Plan")));
        assert_eq!(shape_texts(slide), ["Plan", "Caf\u{e9} \u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}/Second line"]);
        let Block::Float(title) = &slide.shapes[0] else { panic!() };
        assert_eq!((title.x.place, title.y.place, title.width, title.height), (Place::Offset(72.), Place::Offset(36.), 576., 108.));
        let Block::Formatted(heading) = &title.blocks[0] else { panic!() };
        assert!(heading.spans[0].style.bold);
        assert_eq!(heading.size, TITLE_SIZE);
        assert_eq!(slide.omitted.pictures, 1);
        assert_eq!((parsed.title.as_deref(), parsed.author.as_deref()), (Some("Toolbox notes"), Some("Ana")));
    }

    #[test]
    fn hidden_slides_and_notes_follow_the_include_options() {
        let mut first = page("Shown", "a", false);
        first.notes = Some("Say hello");
        let bytes = deck(&[first, page("Hidden", "b", true), page("Last", "c", false)]);
        let parsed = parse(&bytes, 1 << 20, exceeded(), Include::default()).unwrap();
        assert_eq!(parsed.slides.iter().map(|slide| slide.number).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(parsed.hidden_left_out, 1);
        assert!(parsed.slides[0].notes.is_empty());

        let parsed = parse(&bytes, 1 << 20, exceeded(), Include { hidden: true, notes: true }).unwrap();
        assert_eq!(parsed.slides.iter().map(|slide| slide.number).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(parsed.hidden_left_out, 0);
        let Block::Formatted(note) = &parsed.slides[0].notes[0] else { panic!() };
        assert_eq!(plain_text(&note.spans), "Say hello");
    }

    #[test]
    fn old_damaged_and_oversized_files_are_named() {
        assert_eq!(parse(&cfb_with_streams(&[("/PP40", &[0u8; 8])]), 1 << 20, exceeded(), Include::default()).err().unwrap().message, TOO_OLD);
        assert_eq!(parse(&cfb_with_streams(&[("/PowerPoint Document", &[0u8; 8])]), 1 << 20, exceeded(), Include::default()).err().unwrap().message, DAMAGED);
        assert_eq!(parse(&deck(&[page("a", "b", false)]), 64, exceeded(), Include::default()).err().unwrap().message, "too big");
    }

    #[test]
    fn encrypted_presentations_are_detected_before_parsing() {
        assert!(crate::kit::office::is_encrypted(&ppt_fixture(true, false), "ppt").unwrap());
        assert!(!crate::kit::office::is_encrypted(&deck(&[page("a", "b", false)]), "ppt").unwrap());
    }
}
